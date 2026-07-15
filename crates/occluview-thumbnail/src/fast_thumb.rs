//! Fast thumbnail-specific file readers.
//!
//! These readers are intentionally narrower than the full product loaders:
//! they optimize the Explorer thumbnail path for latency by extracting a
//! lightweight preview mesh or point cloud from common large formats without
//! paying the full parser cost. The app/CLI/open path keeps using the canonical
//! readers in `occluview-formats`.

#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]
#![cfg_attr(test, allow(clippy::expect_used))]

use std::cmp::Ordering;

mod obj;
mod ply;
mod stl;

use crate::thumbnail_format::infer_thumbnail_format;
use glam::Vec3;
use occluview_core::{Mesh, MeshBuilder, Vertex};
use occluview_formats::dispatch::read_file_bytes;
use occluview_formats::ply::header::{self, Format as PlyFormat, Property, ScalarType};
use occluview_formats::stl::ascii as stl_ascii;
use occluview_formats::FormatError;
use occluview_formats::FormatKind;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use obj::fast_obj_thumbnail_mesh;
use ply::fast_ply_thumbnail_mesh;
use stl::fast_binary_stl_thumbnail_mesh;

const FAST_POINT_TARGET: usize = 32_000;
/// Deterministic sample budget (in vertex positions) for [`RobustBoundsSampler`].
///
/// The clustering grid is sized from an outlier-robust box, not the raw min/max
/// of the whole file: a single vertex parked at 1e6 mm would otherwise stretch
/// one cell to thousands of millimeters, weld the entire real model into one or
/// two cells, and leave the fast path with zero surviving triangles (a fully
/// transparent tile). Sampling a bounded, strided subset keeps sizing O(1) in
/// allocation regardless of how many millions of triangles the file declares,
/// while staying large enough that the 1% trim is a meaningful percentile.
const ROBUST_BOUNDS_SAMPLE_LIMIT: usize = 16_384;
/// Grid resolution (cells per axis) for the contiguous surface fast path.
///
/// Sized to the thumbnail's native pixel resolution: once a decimated mesh is
/// framed to fill the ~256 px tile, each occupied cell projects to at most ~1
/// output pixel, so neighboring cells rasterize to adjacent pixels and leave no
/// gaps. This is what keeps a decimated surface reading as a SOLID surface
/// rather than the see-through sieve that per-Nth-triangle striding produced.
const FAST_CLUSTER_GRID: u32 = 256;
const STL_HEADER_SIZE: usize = 80;
const STL_COUNT_SIZE: usize = 4;
const STL_TRIANGLE_SIZE: usize = 50;
const STL_FIRST_TRIANGLE_OFFSET: usize = STL_HEADER_SIZE + STL_COUNT_SIZE;

/// Try the format-specific fast path for an in-memory thumbnail input.
pub fn try_read_fast_thumbnail_mesh(extension: Option<&str>, bytes: &[u8]) -> Option<Mesh> {
    let kind = infer_thumbnail_format(extension, bytes).ok()?;
    try_read_fast_thumbnail_mesh_for_kind(kind, bytes)
}

/// Try the format-specific fast path when the input kind is already known.
pub fn try_read_fast_thumbnail_mesh_for_kind(kind: FormatKind, bytes: &[u8]) -> Option<Mesh> {
    match kind {
        FormatKind::Stl => {
            if stl_ascii::looks_like_ascii(bytes) {
                None
            } else {
                fast_binary_stl_thumbnail_mesh(bytes).ok()
            }
        }
        FormatKind::Obj => fast_obj_thumbnail_mesh(bytes).ok(),
        // The PLY fast reader builds a decimated point cloud ONLY for genuine
        // faceless point-cloud PLYs; it declines (returns an error, hence `None`
        // here) for surface PLYs that declare faces, so the caller falls through
        // to the full `occluview-formats` reader and gets a real triangulated
        // surface instead of a cloud of dots. For the point-cloud case the
        // renderer's point pipeline plus the sparse-visibility splat
        // (see render_thumb::rendering::boost_sparse_thumbnail_visibility)
        // keep the preview faithful. Reject only truly empty output.
        FormatKind::Ply => fast_ply_thumbnail_mesh(bytes)
            .ok()
            .filter(|mesh| !mesh.vertices().is_empty()),
        _ => None,
    }
}

/// Try the fast path for a file-backed thumbnail input.
pub fn try_read_fast_thumbnail_mesh_from_file(path: &Path) -> Option<Mesh> {
    let file_bytes = read_file_bytes(path).ok()?;
    try_read_fast_thumbnail_mesh(Some(file_bytes.extension()), file_bytes.as_slice())
}

fn sample_stride(total: usize, target: usize) -> usize {
    if total <= target {
        1
    } else {
        total.div_ceil(target)
    }
}

/// Per-axis outlier-robust bounding box for sizing the clustering grid.
///
/// Accumulates a bounded, deterministic sample of *finite* vertex positions and
/// reduces it to a box that trims ~1% off each axis end, mirroring the
/// renderer's own framing (`render_thumb::rendering::robust_range`). A handful
/// of far outliers therefore cannot stretch the grid; positions outside the
/// robust box simply clamp into the border cells of [`SurfaceGridCluster`]
/// (isolated outliers weld together out there, harmless because the renderer's
/// robust framing ignores them). Non-finite positions are never sampled.
#[derive(Default)]
struct RobustBoundsSampler {
    xs: Vec<f32>,
    ys: Vec<f32>,
    zs: Vec<f32>,
}

impl RobustBoundsSampler {
    fn with_capacity(sample_limit: usize) -> Self {
        Self {
            xs: Vec::with_capacity(sample_limit),
            ys: Vec::with_capacity(sample_limit),
            zs: Vec::with_capacity(sample_limit),
        }
    }

    /// Record one candidate position; non-finite positions are ignored so they
    /// can never bias the grid bounds.
    fn push(&mut self, position: [f32; 3]) {
        let v = Vec3::from_array(position);
        if v.is_finite() {
            self.xs.push(v.x);
            self.ys.push(v.y);
            self.zs.push(v.z);
        }
    }

    /// Reduce the sample to a robust `(min, max)` box, or `None` if no finite
    /// position was ever recorded (fully non-finite / empty input).
    fn finish(mut self) -> Option<(Vec3, Vec3)> {
        let (min_x, max_x) = robust_axis_range(&mut self.xs)?;
        let (min_y, max_y) = robust_axis_range(&mut self.ys)?;
        let (min_z, max_z) = robust_axis_range(&mut self.zs)?;
        Some((
            Vec3::new(min_x, min_y, min_z),
            Vec3::new(max_x, max_y, max_z),
        ))
    }
}

/// Sort a finite axis sample and trim ~1% off each end so a few far outliers
/// cannot set the axis bounds. Matches the trim policy the renderer uses for
/// framing (`render_thumb::rendering::robust_range`): no trim below 128
/// samples, otherwise `len/100` clamped to `[1, len/10]`.
fn robust_axis_range(values: &mut [f32]) -> Option<(f32, f32)> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f32::total_cmp);
    let len = values.len();
    let trim = if len < 128 {
        0
    } else {
        (len / 100).clamp(1, len / 10)
    };
    Some((values[trim], values[len - 1 - trim]))
}

/// Contiguous vertex-clustering decimator for triangle soups.
///
/// The old fast surface path kept every Nth triangle. On a dense scan that
/// punches the surface full of holes — the thumbnail renders as a see-through
/// sieve (owner report: "ВСЯ В КРАПИНКУ, я сквозь неё вижу"). This instead
/// snaps every vertex onto a coarse spatial grid and welds all vertices that
/// land in the same cell to one shared representative. Neighboring triangles go
/// on sharing edges through those representatives, so the reduced mesh stays a
/// closed, opaque surface — just at a lower, thumbnail-appropriate resolution.
/// Triangles whose three corners collapse into fewer than three cells are
/// dropped (they carry no area), and exact-duplicate triangles are coalesced so
/// pathological inputs (e.g. thousands of stacked coincident triangles) can't
/// bloat the output.
///
/// The grid is sized over an outlier-robust box (see [`RobustBoundsSampler`]),
/// so a stray far vertex cannot inflate the cell size and collapse the model.
/// A triangle with ANY non-finite corner is dropped whole: a NaN/Inf vertex is
/// never made a cell representative and never poisons the output mesh bbox.
struct SurfaceGridCluster {
    min: Vec3,
    /// Upper corner of the robust box. Representatives are clamped into
    /// `[min, max]` so a far outlier welds to the border at box scale.
    max: Vec3,
    /// `grid / extent` per axis; 0 on a degenerate (zero-extent) axis so a flat
    /// mesh clusters on the remaining two axes instead of collapsing to a line.
    scale: Vec3,
    last_cell: u32,
    grid: u64,
    cells: HashMap<u64, u32>,
    seen_triangles: HashSet<[u32; 3]>,
    builder: MeshBuilder,
    emitted: usize,
}

impl SurfaceGridCluster {
    fn new(name: &'static str, min: Vec3, max: Vec3, grid: u32) -> Self {
        let grid = grid.max(1);
        // Guard against an inverted/degenerate robust box before deriving scale.
        let max = max.max(min);
        let extent = max - min;
        let axis_scale = |e: f32| {
            if e > f32::EPSILON {
                grid as f32 / e
            } else {
                0.0
            }
        };
        Self {
            min,
            max,
            scale: Vec3::new(
                axis_scale(extent.x),
                axis_scale(extent.y),
                axis_scale(extent.z),
            ),
            last_cell: grid - 1,
            grid: u64::from(grid),
            cells: HashMap::new(),
            seen_triangles: HashSet::new(),
            builder: MeshBuilder::new().with_name(name),
            emitted: 0,
        }
    }

    fn cell_key(&self, position: [f32; 3]) -> u64 {
        // `f32 as u32` saturates negatives to 0; `.min(last_cell)` caps the
        // upper edge, so every finite coordinate lands in a valid cell.
        let axis = |value: f32, min: f32, scale: f32| -> u64 {
            u64::from((((value - min) * scale) as u32).min(self.last_cell))
        };
        let cx = axis(position[0], self.min.x, self.scale.x);
        let cy = axis(position[1], self.min.y, self.scale.y);
        let cz = axis(position[2], self.min.z, self.scale.z);
        cx + cy * self.grid + cz * self.grid * self.grid
    }

    /// Insert a triangle corner and return its clustered vertex index. The first
    /// vertex seen in a cell becomes that cell's representative (normal and color
    /// preserved from a real surface sample; position clamped into the robust
    /// box). Clamping keeps a far outlier at box scale instead of parking a
    /// 1e6 mm vertex in the mesh — which would inflate the output bbox and make
    /// the renderer rasterize an astronomically large off-frame primitive on a
    /// micro-scale mesh (radius ~1e-3 mm), stalling the thumbnail.
    fn corner(&mut self, mut vertex: Vertex) -> u32 {
        vertex.position = Vec3::from_array(vertex.position)
            .clamp(self.min, self.max)
            .to_array();
        let key = self.cell_key(vertex.position);
        if let Some(&index) = self.cells.get(&key) {
            return index;
        }
        let index = self.builder.push_vertex(vertex);
        self.cells.insert(key, index);
        index
    }

    fn push_triangle(&mut self, a: Vertex, b: Vertex, c: Vertex) {
        // Drop the whole triangle if any corner is non-finite (NaN/Inf). A
        // non-finite position would cast to cell 0 / the border cell, become a
        // representative, and poison the output mesh bbox (infinite bbox =>
        // dead camera framing => blank tile) — so it must never be inserted.
        if !Vec3::from_array(a.position).is_finite()
            || !Vec3::from_array(b.position).is_finite()
            || !Vec3::from_array(c.position).is_finite()
        {
            return;
        }
        let ia = self.corner(a);
        let ib = self.corner(b);
        let ic = self.corner(c);
        if ia == ib || ib == ic || ia == ic {
            return;
        }
        if self.seen_triangles.insert([ia, ib, ic]) {
            self.builder.push_triangle(ia, ib, ic);
            self.emitted += 1;
        }
    }

    /// Number of triangles emitted so far (after degenerate/duplicate culling).
    fn triangle_count(&self) -> usize {
        self.emitted
    }

    fn build(self) -> Result<Mesh, FormatError> {
        self.builder.build().map_err(FormatError::Core)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn binary_stl_fast_path_clusters_dense_surface_into_a_solid_reduced_mesh() {
        // A finely tessellated 10x10 plane with far more triangles than the
        // cluster grid can resolve. The fast path must WELD onto the grid (a
        // contiguous, hole-free surface) instead of dropping every Nth triangle
        // (the see-through speckle). So the output must stay a real triangle
        // SURFACE, be meaningfully reduced, and stay bounded by the grid.
        const CELLS: usize = 400; // 400x400 quads -> 320,000 triangles
        let span = 10.0_f32;
        let step = span / CELLS as f32;
        let triangle_count = (CELLS * CELLS * 2) as u32;

        let mut bytes = vec![0u8; STL_FIRST_TRIANGLE_OFFSET];
        bytes[STL_HEADER_SIZE..STL_FIRST_TRIANGLE_OFFSET]
            .copy_from_slice(&triangle_count.to_le_bytes());
        for row in 0..CELLS {
            for col in 0..CELLS {
                let x0 = col as f32 * step;
                let x1 = x0 + step;
                let z0 = row as f32 * step;
                let z1 = z0 + step;
                for tri in [
                    [x0, 0.0, z0, x1, 0.0, z0, x1, 0.0, z1],
                    [x0, 0.0, z0, x1, 0.0, z1, x0, 0.0, z1],
                ] {
                    let floats = [
                        0.0, 1.0, 0.0, tri[0], tri[1], tri[2], tri[3], tri[4], tri[5], tri[6],
                        tri[7], tri[8],
                    ];
                    for value in floats {
                        bytes.extend_from_slice(&value.to_le_bytes());
                    }
                    bytes.extend_from_slice(&[0, 0]);
                }
            }
        }

        let mesh = fast_binary_stl_thumbnail_mesh(&bytes).expect("fast STL thumbnail");

        assert!(!mesh.is_point_cloud(), "a dense STL must stay a surface");
        assert!(mesh.triangle_count() > 0);
        assert!(
            mesh.triangle_count() < triangle_count as usize,
            "clustering must reduce a dense surface (got {} of {triangle_count})",
            mesh.triangle_count()
        );
        // Grid-clustered: vertex count is bounded by the grid footprint, not by
        // the millions of input corners.
        let grid = FAST_CLUSTER_GRID as usize;
        assert!(
            mesh.vertices().len() <= grid * grid,
            "clustered vertex count {} exceeded grid bound {}",
            mesh.vertices().len(),
            grid * grid
        );
    }

    /// Build a minimal binary STL from `(a, b, c)` corner triples.
    fn binary_stl_from_triangles(triangles: &[([f32; 3], [f32; 3], [f32; 3])]) -> Vec<u8> {
        let mut bytes = vec![0u8; STL_FIRST_TRIANGLE_OFFSET];
        bytes[STL_HEADER_SIZE..STL_FIRST_TRIANGLE_OFFSET]
            .copy_from_slice(&(triangles.len() as u32).to_le_bytes());
        for (a, b, c) in triangles {
            let floats = [
                0.0, 0.0, 1.0, a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2],
            ];
            for value in floats {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes.extend_from_slice(&[0, 0]);
        }
        bytes
    }

    #[test]
    fn robust_axis_range_trims_a_lone_far_outlier() {
        // 400 tightly-clustered samples in [0, 1] plus one outlier at 1e6. The
        // 1% trim must discard the outlier so the range stays ~[0, 1], not
        // [0, 1e6] (which would blow up the grid cell size).
        let mut values: Vec<f32> = (0..400).map(|i| i as f32 / 400.0).collect();
        values.push(1.0e6);
        let (min, max) = robust_axis_range(&mut values).expect("non-empty");
        assert!(
            min >= 0.0 && max <= 1.0,
            "outlier not trimmed: ({min}, {max})"
        );
    }

    #[test]
    fn robust_bounds_sampler_ignores_non_finite_positions() {
        let mut sampler = RobustBoundsSampler::with_capacity(8);
        sampler.push([0.0, 0.0, 0.0]);
        sampler.push([f32::NAN, 1.0, 2.0]);
        sampler.push([f32::INFINITY, 0.0, 0.0]);
        sampler.push([2.0, 2.0, 2.0]);
        let (min, max) = sampler.finish().expect("two finite samples");
        assert!(min.is_finite() && max.is_finite());
        assert_eq!(min, Vec3::ZERO);
        assert_eq!(max, Vec3::splat(2.0));
    }

    #[test]
    fn binary_stl_fast_path_drops_non_finite_triangles() {
        // One valid triangle plus one all-NaN triangle: the NaN corners must
        // never become representatives, so the output stays finite with exactly
        // the one real triangle.
        let nan = [f32::NAN; 3];
        let bytes = binary_stl_from_triangles(&[
            ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            (nan, nan, nan),
        ]);
        let mesh = fast_binary_stl_thumbnail_mesh(&bytes).expect("valid triangle survives");
        assert_eq!(mesh.triangle_count(), 1);
        assert!(mesh
            .vertices()
            .iter()
            .all(|v| Vec3::from_array(v.position).is_finite()));
    }

    #[test]
    fn binary_stl_fast_path_declines_all_degenerate_soup() {
        // Every triangle is zero-area (three coincident corners): the clusterer
        // emits nothing, so the fast path declines (defers to the full reader)
        // instead of building a 0-triangle mesh that renders transparent.
        let triangles: Vec<_> = (0..16)
            .map(|i| {
                let p = [i as f32 * 0.1, 0.0, 0.0];
                (p, p, p)
            })
            .collect();
        let bytes = binary_stl_from_triangles(&triangles);
        assert!(matches!(
            fast_binary_stl_thumbnail_mesh(&bytes),
            Err(FormatError::Deferred { .. })
        ));
    }

    #[test]
    fn byte_backed_fast_path_detects_binary_stl_without_extension() {
        let mut bytes = vec![0u8; STL_FIRST_TRIANGLE_OFFSET];
        bytes[80..84].copy_from_slice(&1u32.to_le_bytes());
        // One real (non-degenerate) triangle: normal + three distinct corners.
        // A zero-area triangle would be correctly culled by the grid clusterer,
        // so use genuine geometry — this test is about format detection.
        let floats: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        for value in floats {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);

        let mesh = try_read_fast_thumbnail_mesh(None, &bytes).expect("fast stream STL path");

        assert_eq!(mesh.triangle_count(), 1);
        assert!(!mesh.is_point_cloud());
    }

    #[test]
    fn obj_fast_path_builds_triangle_mesh_with_vertex_colors_when_faces_exist() {
        let bytes = br"
v 0 0 0 255 200 100
v 1 0 0
v 0 1 0 0.5 0.25 0.0
f 1 2 3
";

        let mesh = fast_obj_thumbnail_mesh(bytes).expect("fast OBJ thumbnail");

        assert!(!mesh.is_point_cloud());
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.vertices().len(), 3);
        assert_eq!(mesh.vertices()[0].color, [255, 200, 100, 255]);
        assert_eq!(mesh.vertices()[2].color, [128, 64, 0, 255]);
    }

    #[test]
    fn obj_fast_path_tolerates_bom_and_non_utf8_comment_metadata() {
        let mut bytes = Vec::from(&b"\xef\xbb\xbf# scanner metadata "[..]);
        bytes.extend_from_slice(&[0xff, 0xfe, b'\n']);
        bytes.extend_from_slice(
            br"mtllib scan.mtl
usemtl scan
v 0 0 0 255 200 100
v 1 0 0 210 170 120
v 0 1 0 240 220 180
f 1/1 2/2 3/3
",
        );

        let mesh = fast_obj_thumbnail_mesh(&bytes).expect("fast OBJ thumbnail");

        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.vertices().len(), 3);
        assert_eq!(mesh.vertices()[0].color, [255, 200, 100, 255]);
    }

    #[test]
    fn obj_fast_path_keeps_vertex_only_files_as_point_cloud() {
        let bytes = br"
v 0 0 0 255 200 100
v 1 0 0
v 0 1 0 0.5 0.25 0.0
";

        let mesh = fast_obj_thumbnail_mesh(bytes).expect("fast OBJ thumbnail");

        assert!(mesh.is_point_cloud());
        assert_eq!(mesh.vertices().len(), 3);
    }

    #[test]
    fn obj_fast_path_declines_when_declared_faces_are_all_degenerate() {
        // Faces ARE declared but every one is zero-area (a repeated vertex
        // index), so clustering emits nothing. Decline to the full reader rather
        // than splatting the surface file as a cloud of points.
        let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 1 1\nf 2 2 2\n";
        assert!(matches!(
            fast_obj_thumbnail_mesh(bytes),
            Err(FormatError::Deferred { .. })
        ));
    }

    #[test]
    fn ply_ascii_fast_path_preserves_vertex_colors() {
        // A genuine faceless point cloud: the fast path handles this and keeps
        // its per-vertex colors.
        let bytes = br"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
property uchar red
property uchar green
property uchar blue
end_header
0 0 0 255 220 180
1 0 0 200 180 120
0 1 0 150 120 90
";

        let mesh = fast_ply_thumbnail_mesh(bytes).expect("fast PLY thumbnail");

        assert!(mesh.is_point_cloud());
        assert_eq!(mesh.vertices().len(), 3);
        assert_eq!(mesh.vertices()[1].color, [200, 180, 120, 255]);
    }

    #[test]
    fn ply_surface_fast_path_declines_so_full_reader_renders_a_surface() {
        // A PLY that declares faces is a SURFACE. The fast reader must NOT
        // return it as a point cloud (that is the "куча точек" bug); it declines
        // so `load_thumbnail_mesh_*` falls through to the full reader, which
        // triangulates the faces into a real surface.
        let bytes = br"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
property uchar red
property uchar green
property uchar blue
element face 1
property list uchar int vertex_indices
end_header
0 0 0 255 220 180
1 0 0 200 180 120
0 1 0 150 120 90
3 0 1 2
";

        // The narrow fast reader declines (surface PLY defers to the full reader).
        assert!(matches!(
            fast_ply_thumbnail_mesh(bytes),
            Err(FormatError::Deferred { .. })
        ));
        assert!(
            try_read_fast_thumbnail_mesh_for_kind(FormatKind::Ply, bytes).is_none(),
            "the fast PLY path must decline surface PLYs so the full reader renders a real surface"
        );

        // The full reader that the caller falls through to yields a triangle
        // SURFACE, never a point cloud, for this same surface PLY.
        let mesh = occluview_formats::ply::read(bytes).expect("full PLY reader");
        assert!(
            !mesh.is_point_cloud(),
            "a surface PLY must thumbnail as a surface, not a point splat"
        );
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn try_read_fast_thumbnail_mesh_from_file_handles_real_file_extensions() {
        let dir = std::env::temp_dir().join(format!(
            "occluview-fast-thumb-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("scan.obj");
        let mut file = std::fs::File::create(&path).expect("create temp obj");
        writeln!(file, "v 0 0 0").expect("write");
        writeln!(file, "v 1 0 0").expect("write");
        writeln!(file, "v 0 1 0").expect("write");

        let mesh = try_read_fast_thumbnail_mesh_from_file(&path).expect("fast path");

        assert!(mesh.is_point_cloud());
        assert_eq!(mesh.vertices().len(), 3);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
