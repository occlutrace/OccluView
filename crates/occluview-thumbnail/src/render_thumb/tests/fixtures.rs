use super::*;
use glam::Vec3;
use occluview_core::{CoreError, Mesh, MeshBuilder, Vertex};

pub(super) fn binary_stl_cube() -> Vec<u8> {
    let vertices: [[f32; 3]; 8] = [
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
    ];
    let triangles: [([f32; 3], [usize; 3]); 12] = [
        ([0.0, 0.0, -1.0], [0, 2, 1]),
        ([0.0, 0.0, -1.0], [0, 3, 2]),
        ([0.0, 0.0, 1.0], [4, 5, 6]),
        ([0.0, 0.0, 1.0], [4, 6, 7]),
        ([0.0, -1.0, 0.0], [0, 1, 5]),
        ([0.0, -1.0, 0.0], [0, 5, 4]),
        ([1.0, 0.0, 0.0], [1, 2, 6]),
        ([1.0, 0.0, 0.0], [1, 6, 5]),
        ([0.0, 1.0, 0.0], [2, 3, 7]),
        ([0.0, 1.0, 0.0], [2, 7, 6]),
        ([-1.0, 0.0, 0.0], [3, 0, 4]),
        ([-1.0, 0.0, 0.0], [3, 4, 7]),
    ];

    let mut out = vec![0u8; 84];
    out[..18].copy_from_slice(b"OccluView cube STL");
    out[80..84].copy_from_slice(&12u32.to_le_bytes());
    for (normal, idx) in triangles {
        for value in normal {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for vertex_index in idx {
            for value in vertices[vertex_index] {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out.extend_from_slice(&[0, 0]);
    }
    out
}

pub(super) fn dense_binary_stl_strip(triangle_count: u32) -> Vec<u8> {
    let mut out = vec![0u8; 84];
    let header = b"OccluView dense strip STL";
    out[..header.len()].copy_from_slice(header);
    out[80..84].copy_from_slice(&triangle_count.to_le_bytes());
    for index in 0..triangle_count {
        let x = (index % 64) as f32 * 0.08;
        let z = (index / 64) as f32 * 0.08;
        let floats = [0.0, 1.0, 0.0, x, 0.0, z, x + 0.06, 0.0, z, x, 0.0, z + 0.06];
        for value in floats {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out
}

/// A binary STL of a continuously tessellated, roughly square 30mm x 30mm
/// plane with at least `min_bytes` worth of triangles.
///
/// Unlike `dense_binary_stl_strip` (which packs isolated, gapped triangles
/// into a fixed-width strip and becomes an unusably thin sliver at large
/// triangle counts), every triangle here shares its edges with its
/// neighbors, like a real dense scan surface. That keeps the projected
/// thumbnail solidly filled - with real interior opaque coverage - even
/// after fast-path decimation and 2x supersampling, at any size.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn large_binary_stl_tessellated_plane(min_bytes: usize) -> Vec<u8> {
    const SPAN_MM: f32 = 30.0;
    const TRIANGLE_BYTES: usize = 50;

    let triangle_target = (min_bytes.saturating_sub(84) / TRIANGLE_BYTES).max(2);
    let side_cells = ((triangle_target / 2) as f64).sqrt().ceil().max(1.0) as u32;
    let cell_size = SPAN_MM / side_cells as f32;

    let mut out = vec![0u8; 84];
    let header = b"OccluView dense tessellated plane STL";
    out[..header.len()].copy_from_slice(header);

    let mut triangle_count = 0u32;
    for row in 0..side_cells {
        for col in 0..side_cells {
            let x0 = col as f32 * cell_size;
            let x1 = x0 + cell_size;
            let z0 = row as f32 * cell_size;
            let z1 = z0 + cell_size;
            push_stl_triangle(
                &mut out,
                [0.0, 1.0, 0.0],
                [x0, 0.0, z0],
                [x1, 0.0, z0],
                [x1, 0.0, z1],
            );
            push_stl_triangle(
                &mut out,
                [0.0, 1.0, 0.0],
                [x0, 0.0, z0],
                [x1, 0.0, z1],
                [x0, 0.0, z1],
            );
            triangle_count += 2;
        }
    }
    out[80..84].copy_from_slice(&triangle_count.to_le_bytes());
    out
}

/// A dense binary-STL UV sphere of at least `min_bytes`. A sphere is the sharp
/// speckle probe: its thumbnail silhouette is a filled disc, so ANY transparent
/// pixel strictly inside the disc is a see-through hole — exactly the speckled
/// artifact that per-Nth-triangle striding produced on dense scans.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn dense_binary_stl_sphere(min_bytes: usize) -> Vec<u8> {
    let triangle_target = (min_bytes.saturating_sub(84) / 50).max(8);
    let stacks = ((triangle_target as f64 / 4.0).sqrt().ceil() as usize).max(4);
    let slices = stacks * 2;
    let radius = 10.0_f32;
    let point = |i: usize, j: usize| -> [f32; 3] {
        let phi = std::f32::consts::PI * i as f32 / stacks as f32;
        let theta = 2.0 * std::f32::consts::PI * j as f32 / slices as f32;
        [
            radius * phi.sin() * theta.cos(),
            radius * phi.cos(),
            radius * phi.sin() * theta.sin(),
        ]
    };

    let mut out = vec![0u8; 84];
    let header = b"OccluView dense sphere STL";
    out[..header.len()].copy_from_slice(header);
    let mut triangle_count = 0u32;
    for i in 0..stacks {
        for j in 0..slices {
            let a = point(i, j);
            let b = point(i + 1, j);
            let c = point(i + 1, j + 1);
            let d = point(i, j + 1);
            let normal = [0.0, 0.0, 1.0];
            push_stl_triangle(&mut out, normal, a, b, c);
            push_stl_triangle(&mut out, normal, a, c, d);
            triangle_count += 2;
        }
    }
    out[80..84].copy_from_slice(&triangle_count.to_le_bytes());
    out
}

fn push_stl_triangle(out: &mut Vec<u8>, normal: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) {
    for value in normal.into_iter().chain(a).chain(b).chain(c) {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
}

/// Append one triangle to a binary STL byte buffer and bump its count field.
fn append_binary_stl_triangle(out: &mut Vec<u8>, a: [f32; 3], b: [f32; 3], c: [f32; 3]) {
    let count = u32::from_le_bytes(out[80..84].try_into().expect("stl count field")) + 1;
    out[80..84].copy_from_slice(&count.to_le_bytes());
    push_stl_triangle(out, [0.0, 0.0, 1.0], a, b, c);
}

/// A dense binary-STL sphere plus ONE far outlier triangle parked at 1e6 mm on
/// every axis. Pre-fix, that lone outlier stretched the clustering grid so the
/// whole real sphere welded into one or two cells, every triangle was culled as
/// degenerate, and the fast path emitted 0 triangles -> a fully transparent
/// tile. Robust grid bounds must trim the outlier and keep the sphere solid.
pub(super) fn dense_binary_stl_sphere_with_far_outlier(min_bytes: usize) -> Vec<u8> {
    let mut out = dense_binary_stl_sphere(min_bytes);
    append_binary_stl_triangle(
        &mut out,
        [1.0e6, 1.0e6, 1.0e6],
        [1.0e6 + 1.0, 1.0e6, 1.0e6],
        [1.0e6, 1.0e6 + 1.0, 1.0e6],
    );
    out
}

/// A dense binary-STL sphere with ~0.2% of its triangle corners overwritten with
/// NaN / +Inf / -Inf. The bounds pass skipped non-finite corners, but the
/// clusterer still inserted them as cell representatives (a non-finite coord
/// casts to cell 0 / the border cell), poisoning the output mesh bbox -> dead
/// camera framing -> a blank tile for a 99.8%-valid scan. A triangle with any
/// non-finite corner must be dropped whole.
#[allow(clippy::cast_possible_truncation)]
pub(super) fn dense_binary_stl_sphere_with_nonfinite(min_bytes: usize) -> Vec<u8> {
    const HEADER: usize = 84;
    const RECORD: usize = 50;
    const NORMAL_BYTES: usize = 12;
    let mut out = dense_binary_stl_sphere(min_bytes);
    let count = u32::from_le_bytes(out[80..84].try_into().expect("stl count field")) as usize;
    let poison = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
    // Poison one corner of ~1 in 500 triangles (~0.2%).
    for (nth, triangle_index) in (0..count).step_by(500).enumerate() {
        let corner_x = HEADER + triangle_index * RECORD + NORMAL_BYTES;
        out[corner_x..corner_x + 4].copy_from_slice(&poison[nth % poison.len()].to_le_bytes());
    }
    out
}

/// A dense binary-STL sphere (mm-scale bulk) plus a few micro-scale (1e-3 mm)
/// triangles and one far (1e6 mm) outlier, so the coordinate range spans
/// 1e-3..1e6. Robust bounds must trim BOTH extremes and frame the bulk.
pub(super) fn dense_binary_stl_huge_coordinate_range(min_bytes: usize) -> Vec<u8> {
    let mut out = dense_binary_stl_sphere(min_bytes);
    for _ in 0..8 {
        append_binary_stl_triangle(
            &mut out,
            [1.0e-3, 1.0e-3, 1.0e-3],
            [2.0e-3, 1.0e-3, 1.0e-3],
            [1.0e-3, 2.0e-3, 1.0e-3],
        );
    }
    append_binary_stl_triangle(
        &mut out,
        [1.0e6, 1.0e6, 1.0e6],
        [1.0e6 + 1.0, 1.0e6, 1.0e6],
        [1.0e6, 1.0e6 + 1.0, 1.0e6],
    );
    out
}

/// A wholly non-drawable binary STL: zero-area triangles (three coincident
/// corners) scattered across a plane, plus some fully non-finite triangles.
/// Nothing here rasterizes; the fast path must decline and the loader must
/// surface a placeholder, never a transparent tile, and never panic.
#[allow(clippy::cast_precision_loss)]
pub(super) fn all_degenerate_binary_stl() -> Vec<u8> {
    let mut out = vec![0u8; 84];
    let header = b"OccluView all-degenerate STL";
    out[..header.len()].copy_from_slice(header);
    let mut count = 0u32;
    for i in 0..64u32 {
        let point = [i as f32 * 0.1, 0.0, i as f32 * 0.2];
        push_stl_triangle(&mut out, [0.0, 0.0, 1.0], point, point, point);
        count += 1;
    }
    for _ in 0..8 {
        let nan = [f32::NAN, f32::NAN, f32::NAN];
        push_stl_triangle(&mut out, [0.0, 0.0, 1.0], nan, nan, nan);
        count += 1;
    }
    out[80..84].copy_from_slice(&count.to_le_bytes());
    out
}

/// A tessellated `side x side` OBJ grid plane (mm-scale) plus one far outlier
/// triangle at 1e6 mm. Exercises the OBJ fast path's robust bounds and its
/// non-finite / degenerate handling.
pub(super) fn obj_grid_surface_with_far_outlier(side: usize) -> Vec<u8> {
    let mut out = String::with_capacity(side * side * 24);
    let step = 20.0 / side as f32;
    for row in 0..side {
        for col in 0..side {
            let _ = writeln!(out, "v {} 0 {}", col as f32 * step, row as f32 * step);
        }
    }
    let base = side * side;
    let _ = writeln!(out, "v 1000000 1000000 1000000");
    let _ = writeln!(out, "v 1000001 1000000 1000000");
    let _ = writeln!(out, "v 1000000 1000001 1000000");
    let vid = |row: usize, col: usize| row * side + col + 1;
    for row in 0..side - 1 {
        for col in 0..side - 1 {
            let _ = writeln!(
                out,
                "f {} {} {}",
                vid(row, col),
                vid(row, col + 1),
                vid(row + 1, col + 1)
            );
            let _ = writeln!(
                out,
                "f {} {} {}",
                vid(row, col),
                vid(row + 1, col + 1),
                vid(row + 1, col)
            );
        }
    }
    let _ = writeln!(out, "f {} {} {}", base + 1, base + 2, base + 3);
    out.into_bytes()
}

pub(super) fn colored_obj_cube() -> String {
    [
        "v -1 -1 -1 220 180 105",
        "v 1 -1 -1 215 170 96",
        "v 1 1 -1 205 160 88",
        "v -1 1 -1 210 166 92",
        "v -1 -1 1 235 196 122",
        "v 1 -1 1 226 184 112",
        "v 1 1 1 218 176 104",
        "v -1 1 1 230 190 118",
        "f 1 3 2",
        "f 1 4 3",
        "f 5 6 7",
        "f 5 7 8",
        "f 1 2 6",
        "f 1 6 5",
        "f 2 3 7",
        "f 2 7 6",
        "f 3 4 8",
        "f 3 8 7",
        "f 4 1 5",
        "f 4 5 8",
        "",
    ]
    .join("\n")
}

pub(super) fn one_triangle_glb() -> Vec<u8> {
    let json = br#"{"asset":{"version":"2.0"},
"scenes":[{"nodes":[0]}],
"nodes":[{"mesh":0}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
"accessors":[{"bufferView":0,"count":3,"type":"VEC3","componentType":5126},
             {"bufferView":1,"count":3,"type":"SCALAR","componentType":5125}],
"bufferViews":[{"buffer":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":12}],
"buffers":[{"byteLength":48}]}"#;
    let mut bin = Vec::new();
    for f in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        bin.extend_from_slice(&f.to_le_bytes());
    }
    for i in 0u32..3 {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    build_test_glb(json, &bin)
}

fn build_test_glb(json: &[u8], bin: &[u8]) -> Vec<u8> {
    let json_padded_len = align4(json.len());
    let bin_padded_len = align4(bin.len());
    let total_len = 12
        + 8
        + json_padded_len
        + if bin.is_empty() {
            0
        } else {
            8 + bin_padded_len
        };
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(
        &u32::try_from(total_len)
            .expect("test GLB length fits u32")
            .to_le_bytes(),
    );
    out.extend_from_slice(
        &u32::try_from(json_padded_len)
            .expect("test GLB JSON length fits u32")
            .to_le_bytes(),
    );
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(json);
    out.resize(out.len() + (json_padded_len - json.len()), b' ');
    if !bin.is_empty() {
        out.extend_from_slice(
            &u32::try_from(bin_padded_len)
                .expect("test GLB BIN length fits u32")
                .to_le_bytes(),
        );
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(bin);
        out.resize(out.len() + (bin_padded_len - bin.len()), 0);
    }
    out
}

const fn align4(n: usize) -> usize {
    (n + 3) & !3
}

pub(super) fn large_colored_obj_tiles(min_bytes: usize) -> Vec<u8> {
    let mut out = String::with_capacity(min_bytes + (min_bytes / 8).max(1024));
    let mut base_index = 1u32;
    let mut tile = 0u32;
    while out.len() <= min_bytes {
        let x = (tile % 256) as i32 * 3;
        let y = (tile / 256) as i32 * 3;
        let z = (tile % 11) as i32;
        for (dx, dy, dz, r, g, b) in [
            (0, 0, 0, 220, 180, 105),
            (1, 0, 0, 215, 170, 96),
            (1, 1, 0, 205, 160, 88),
            (0, 1, 0, 210, 166, 92),
            (0, 0, 1, 235, 196, 122),
            (1, 0, 1, 226, 184, 112),
            (1, 1, 1, 218, 176, 104),
            (0, 1, 1, 230, 190, 118),
        ] {
            let _ = writeln!(out, "v {} {} {} {} {} {}", x + dx, y + dy, z + dz, r, g, b);
        }
        for (a, b, c) in [
            (0, 2, 1),
            (0, 3, 2),
            (4, 5, 6),
            (4, 6, 7),
            (0, 1, 5),
            (0, 5, 4),
            (1, 2, 6),
            (1, 6, 5),
            (2, 3, 7),
            (2, 7, 6),
            (3, 0, 4),
            (3, 4, 7),
        ] {
            let _ = writeln!(
                out,
                "f {} {} {}",
                base_index + a,
                base_index + b,
                base_index + c
            );
        }
        base_index += 8;
        tile += 1;
    }
    out.into_bytes()
}

pub(super) fn obj_with_early_faces(min_bytes: usize) -> Vec<u8> {
    let mut out = String::with_capacity(min_bytes + (min_bytes / 8).max(1024));
    out.push_str("# OccluView OBJ thumbnail stress fixture\n");
    out.push_str("mtllib missing-materials.mtl\nusemtl scan\n");
    let mut base_index = 1u32;
    let mut tile = 0u32;
    while out.len() < min_bytes {
        let _ = writeln!(
            out,
            "f {}/{} {}/{} {}/{}",
            base_index,
            base_index,
            base_index + 1,
            base_index + 1,
            base_index + 2,
            base_index + 2
        );
        let x = (tile % 192) as f32 * 0.25;
        let y = (tile / 192) as f32 * 0.25;
        let z = (tile % 17) as f32 * 0.02;
        for (dx, dy, r, g, b) in [
            (0.0, 0.0, 220, 180, 105),
            (0.18, 0.0, 215, 170, 96),
            (0.0, 0.18, 235, 196, 122),
        ] {
            let _ = writeln!(out, "vt {dx} {dy}");
            let _ = writeln!(out, "v {} {} {} {} {} {}", x + dx, y + dy, z, r, g, b);
        }
        base_index += 3;
        tile += 1;
    }
    out.into_bytes()
}

pub(super) fn noisy_obj_with_early_faces(min_bytes: usize) -> Vec<u8> {
    let mut out = Vec::from(&b"\xef\xbb\xbf# scanner metadata "[..]);
    out.extend_from_slice(&[0xff, 0xfe, b'\n']);
    let body_target = min_bytes.saturating_sub(out.len());
    out.extend_from_slice(&obj_with_early_faces(body_target));
    while out.len() < min_bytes {
        out.extend_from_slice(b"# thumbnail padding\n");
    }
    out
}

pub(super) fn write_temp_fixture(extension: &str, bytes: &[u8]) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let path = std::env::temp_dir().join(format!(
        "occluview-shell-full-fidelity-{unique}.{extension}"
    ));
    fs::write(&path, bytes).expect("write temp fixture");
    path
}

pub(super) fn hps_zip_triangle() -> Result<Vec<u8>, String> {
    let hps = br#"<?xml version="1.0" encoding="UTF-8"?>
<HPS>
  <Packed_geometry>
    <Schema>CC</Schema>
    <Binary_data>
      <CC version="1.0">
        <Facets facet_count="1" base64_encoded_bytes="1">BA==</Facets>
        <Vertices vertex_count="3" base64_encoded_bytes="36">AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA</Vertices>
      </CC>
    </Binary_data>
  </Packed_geometry>
</HPS>"#;
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut archive = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        archive
            .start_file("scan/geometry.hps", options)
            .map_err(|err| err.to_string())?;
        archive.write_all(hps).map_err(|err| err.to_string())?;
        archive.finish().map_err(|err| err.to_string())?;
    }
    Ok(cursor.into_inner())
}

pub(super) fn colored_ply_cube() -> &'static [u8] {
    br"ply
format ascii 1.0
element vertex 8
property float x
property float y
property float z
property uchar red
property uchar green
property uchar blue
element face 12
property list uchar int vertex_indices
end_header
-1 -1 -1 220 180 105
1 -1 -1 215 170 96
1 1 -1 205 160 88
-1 1 -1 210 166 92
-1 -1 1 235 196 122
1 -1 1 226 184 112
1 1 1 218 176 104
-1 1 1 230 190 118
3 0 2 1
3 0 3 2
3 4 5 6
3 4 6 7
3 0 1 5
3 0 5 4
3 1 2 6
3 1 6 5
3 2 3 7
3 2 7 6
3 3 0 4
3 3 4 7
"
}

/// A binary `little_endian` PLY point cloud (vertex element only, no faces)
/// sized to at least `min_bytes`. Used to exercise the fast PLY thumbnail
/// surrogate on inputs well above the fidelity cutoff without paying ASCII's
/// text-formatting overhead for multi-megabyte fixtures.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn large_binary_ply_point_grid(min_bytes: usize) -> Vec<u8> {
    const ROW_BYTES: usize = (4 * 3) + 3; // x,y,z f32 + r,g,b uchar
    const HEADER_RESERVE: usize = 256;

    // A little headroom on top of the exact row count needed so the header
    // bytes never make the total undershoot `min_bytes`.
    let vertex_count = min_bytes.div_ceil(ROW_BYTES).saturating_add(4_096).max(1);
    let mut out = Vec::with_capacity(HEADER_RESERVE + vertex_count * ROW_BYTES);
    out.extend_from_slice(b"ply\n");
    out.extend_from_slice(b"format binary_little_endian 1.0\n");
    let _ = writeln!(out, "element vertex {vertex_count}");
    out.extend_from_slice(b"property float x\n");
    out.extend_from_slice(b"property float y\n");
    out.extend_from_slice(b"property float z\n");
    out.extend_from_slice(b"property uchar red\n");
    out.extend_from_slice(b"property uchar green\n");
    out.extend_from_slice(b"property uchar blue\n");
    out.extend_from_slice(b"end_header\n");

    let side = (vertex_count as f64).sqrt().ceil().max(1.0) as usize;
    for index in 0..vertex_count {
        let row = (index / side) as f32;
        let col = (index % side) as f32;
        let x = col * 0.05;
        let z = row * 0.05;
        out.extend_from_slice(&x.to_le_bytes());
        out.extend_from_slice(&0.0_f32.to_le_bytes());
        out.extend_from_slice(&z.to_le_bytes());
        out.extend_from_slice(&[210, 180, 120]);
    }
    out
}

/// A binary `little_endian` PLY *surface* (a tessellated plane with a real
/// `face` element) sized to at least `min_bytes` for surface thumbnail tests.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn large_binary_ply_surface_grid(min_bytes: usize) -> Vec<u8> {
    const VERTEX_ROW_BYTES: usize = (4 * 3) + 3; // x,y,z f32 + r,g,b uchar
    const FACE_ROW_BYTES: usize = 1 + (4 * 3); // count uchar + 3 int indices
    const HEADER_RESERVE: usize = 256;

    // Two triangles per interior quad; grow the grid until the packed vertex +
    // face bytes clear `min_bytes`.
    let mut side = 64usize;
    loop {
        let vertex_count = side * side;
        let quads = (side - 1) * (side - 1);
        let face_count = quads * 2;
        let packed = vertex_count * VERTEX_ROW_BYTES + face_count * FACE_ROW_BYTES;
        if packed >= min_bytes {
            let mut out = Vec::with_capacity(HEADER_RESERVE + packed);
            out.extend_from_slice(b"ply\n");
            out.extend_from_slice(b"format binary_little_endian 1.0\n");
            let _ = writeln!(out, "element vertex {vertex_count}");
            out.extend_from_slice(b"property float x\n");
            out.extend_from_slice(b"property float y\n");
            out.extend_from_slice(b"property float z\n");
            out.extend_from_slice(b"property uchar red\n");
            out.extend_from_slice(b"property uchar green\n");
            out.extend_from_slice(b"property uchar blue\n");
            let _ = writeln!(out, "element face {face_count}");
            out.extend_from_slice(b"property list uchar int vertex_indices\n");
            out.extend_from_slice(b"end_header\n");

            for index in 0..vertex_count {
                let row = (index / side) as f32;
                let col = (index % side) as f32;
                out.extend_from_slice(&(col * 0.05).to_le_bytes());
                out.extend_from_slice(&0.0_f32.to_le_bytes());
                out.extend_from_slice(&(row * 0.05).to_le_bytes());
                out.extend_from_slice(&[210, 180, 120]);
            }
            for r in 0..side - 1 {
                for c in 0..side - 1 {
                    let a = (r * side + c) as u32;
                    let b = a + 1;
                    let d = ((r + 1) * side + c) as u32;
                    let e = d + 1;
                    out.push(3);
                    out.extend_from_slice(&a.to_le_bytes());
                    out.extend_from_slice(&b.to_le_bytes());
                    out.extend_from_slice(&e.to_le_bytes());
                    out.push(3);
                    out.extend_from_slice(&a.to_le_bytes());
                    out.extend_from_slice(&e.to_le_bytes());
                    out.extend_from_slice(&d.to_le_bytes());
                }
            }
            return out;
        }
        side += 64;
    }
}

pub(super) fn point_cluster_with_outlier() -> Result<Mesh, CoreError> {
    let mut builder = MeshBuilder::new().with_name("outlier-cluster");
    for row in 0_u16..10 {
        for column in 0_u16..16 {
            let cluster_x = f32::from(column) * 0.12;
            let cluster_z = f32::from(row) * 0.12;
            let vertex_a = builder
                .push_vertex(Vertex::at(Vec3::new(cluster_x, 0.0, cluster_z)).with_normal(Vec3::Y));
            let vertex_b = builder.push_vertex(
                Vertex::at(Vec3::new(cluster_x + 0.08, 0.0, cluster_z)).with_normal(Vec3::Y),
            );
            let vertex_c = builder.push_vertex(
                Vertex::at(Vec3::new(cluster_x, 0.0, cluster_z + 0.08)).with_normal(Vec3::Y),
            );
            builder.push_triangle(vertex_a, vertex_b, vertex_c);
        }
    }

    let outlier = builder.push_vertex(Vertex::at(Vec3::new(80.0, 0.0, 80.0)).with_normal(Vec3::Y));
    let near_outlier_a =
        builder.push_vertex(Vertex::at(Vec3::new(80.1, 0.0, 80.0)).with_normal(Vec3::Y));
    let near_outlier_b =
        builder.push_vertex(Vertex::at(Vec3::new(80.0, 0.0, 80.1)).with_normal(Vec3::Y));
    builder.push_triangle(outlier, near_outlier_a, near_outlier_b);

    builder.build()
}
