//! The part of a mesh a mark puts in front of the hole filler.
//!
//! A mark is local, and so is everything Close Holes does with it: the rims it
//! may close run among the marked faces, a cap reads the scan triangles at its
//! rim, and the piercing guard looks two rings further. This module cuts the
//! scan inside the box of the marked faces out as a mesh of its own, so
//! welding, healing, the boundary walk and the caps cost what the marked area
//! costs, and puts the result back into the scan. What remains proportional to
//! the scan is a few plain passes over its buffers.
//!
//! Faces are neighbours when they share a corner position: a scan read from an
//! STL has one vertex per corner, and that is the only adjacency it carries.

use glam::Vec3;

use super::topology::canonical_position_key;
use super::{EditVertex, FaceSelection, MeshEditBuffers};

/// How far past the box of the marked faces a rim may run and still be the
/// mark's, in mean edge lengths of the marked faces: the faces a lasso drawn
/// a little short misses at its far edge.
///
/// The reach is measured in space, not along the surface. A surface lasso
/// marks only the faces that look at the camera, and on the rim it encloses
/// it leaves the ones that look away: the walls of the cut. Those lie under
/// the lasso, inside the box, however far along the surface the nearest
/// marked face is; on a real arch that was ten rings of faces and more.
const MARK_REACH_EDGES: f32 = 8.0;

/// Rings of faces at the region's edge that are there to be read. A rim face
/// has to lie further in: the faces around its rim vertices lie one ring out,
/// and the faces the piercing guard reads, around the vertices two rings from
/// the rim, three.
const EDGE_RINGS: u8 = 3;

/// What a mark says about every face of the region cut out around it.
pub(super) struct MarkedFaces {
    /// Whether the operator marked the face.
    marked: Vec<bool>,
    /// Rings of faces between the face and the region's edge, where the scan
    /// goes on unseen; zero for a face that reaches out of the box, and
    /// [`EDGE_RINGS`] for every face at least that far in.
    from_edge: Vec<u8>,
}

impl MarkedFaces {
    /// The marked faces as a selection.
    pub(super) fn marked(&self) -> FaceSelection {
        FaceSelection::new(self.marked.clone())
    }

    /// Whether the operator marked this face.
    pub(super) fn is_marked(&self, triangle: usize) -> bool {
        self.marked.get(triangle) == Some(&true)
    }

    /// Whether everything a cap on this face reads of the scan is in the
    /// region.
    pub(super) fn is_inside(&self, triangle: usize) -> bool {
        self.from_edge
            .get(triangle)
            .is_some_and(|&ring| ring >= EDGE_RINGS)
    }

    /// Whether this face reaches out of the region.
    pub(super) fn is_at_edge(&self, triangle: usize) -> bool {
        self.from_edge.get(triangle) == Some(&0)
    }

    /// Drop the faces a healing pass removed.
    pub(super) fn retain(&mut self, keep: &[bool]) {
        let mut kept = keep.iter();
        self.marked.retain(|_| kept.next().copied().unwrap_or(true));
        let mut kept = keep.iter();
        self.from_edge
            .retain(|_| kept.next().copied().unwrap_or(true));
    }

    /// Per vertex of `mesh`, whether it is a corner of a marked face.
    pub(super) fn marked_corners(&self, mesh: &MeshEditBuffers) -> Vec<bool> {
        corners_of(mesh, self.marked.iter().copied())
    }

    /// Per vertex of `mesh`, whether it is a corner of a face at the region's
    /// edge. The surface goes on past those faces, so around such a vertex
    /// faces may be missing and what looks like a boundary there says nothing.
    pub(super) fn edge_corners(&self, mesh: &MeshEditBuffers) -> Vec<bool> {
        corners_of(mesh, self.from_edge.iter().map(|&ring| ring == 0))
    }
}

/// Per vertex of `mesh`, whether it is a corner of one of the `wanted` faces.
fn corners_of(mesh: &MeshEditBuffers, wanted: impl Iterator<Item = bool>) -> Vec<bool> {
    let mut corners = vec![false; mesh.vertices.len()];
    for (triangle, _) in mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .zip(wanted)
        .filter(|&(_, wanted)| wanted)
    {
        for &vertex in triangle {
            if let Some(flag) = corners.get_mut(vertex as usize) {
                *flag = true;
            }
        }
    }
    corners
}

/// The scan inside the box of a mark, as a mesh of its own, with the way back.
pub(super) struct MarkedRegion {
    /// The region's faces and vertices, in the order the scan has them.
    pub(super) mesh: MeshEditBuffers,
    /// Scan triangle of every region triangle, ascending.
    triangles: Vec<usize>,
    /// Scan vertex of every region vertex, ascending.
    vertices: Vec<u32>,
    /// Per region triangle, whether it reaches out of the box. Those faces
    /// are there to be read: the scan keeps them exactly as they were.
    context: Vec<bool>,
}

/// What the filler made of a surface: its vertices followed by the ones the
/// filler added, the triangles that survived followed by the caps.
pub(super) struct CappedSurface {
    pub(super) vertices: Vec<EditVertex>,
    pub(super) indices: Vec<u32>,
    /// Per triangle of the surface, whether it survived; `None` when all did.
    pub(super) kept: Option<Vec<bool>>,
}

impl MarkedRegion {
    /// Cut the region around the marked faces of `mesh`, and say what the mark
    /// has of each of its faces. The selection has one entry per triangle and
    /// marks at least one.
    pub(super) fn around(mesh: &MeshEditBuffers, selection: &FaceSelection) -> (Self, MarkedFaces) {
        let corners = mesh.indices.as_chunks::<3>().0;
        let marked = selection.as_slice();
        let position = |vertex: u32| {
            mesh.vertices
                .get(vertex as usize)
                .map_or(Vec3::NAN, |vertex| Vec3::from_array(vertex.position))
        };

        // The mark's box and the mean length of its edges.
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let (mut edge_sum, mut edge_count) = (0.0_f64, 0.0_f64);
        for (triangle, _) in corners.iter().zip(marked).filter(|(_, &marked)| marked) {
            let points = triangle.map(position);
            for (corner, point) in points.iter().enumerate() {
                lo = lo.min(*point);
                hi = hi.max(*point);
                let length = point.distance(points[(corner + 1) % 3]);
                if length.is_finite() {
                    edge_sum += f64::from(length);
                    edge_count += 1.0;
                }
            }
        }
        // f64 -> f32: a mean of finite f32 lengths.
        #[allow(clippy::cast_possible_truncation)]
        let mean_edge = (edge_sum / edge_count.max(1.0)) as f32;
        // The box grows by the reach, and by the rings a rim at the end of
        // the reach still needs around it.
        let mut margin = mean_edge * (MARK_REACH_EDGES + f32::from(EDGE_RINGS));
        if !(margin.is_finite() && margin > 0.0) {
            // No lengths to measure by: every face, where nothing is cut off.
            margin = f32::INFINITY;
        }
        let (box_lo, box_hi) = (lo - margin, hi + margin);
        let inside = |point: Vec3| point.cmpge(box_lo).all() && point.cmple(box_hi).all();

        // The region is every face with a corner in the box. A face with all
        // its corners in it has all its neighbours in the region; one that
        // reaches out of the box is where the region ends.
        let (mut triangles, mut region_marked, mut at_edge) = (Vec::new(), Vec::new(), Vec::new());
        for (index, triangle) in corners.iter().enumerate() {
            let is_marked = marked.get(index).copied().unwrap_or(false);
            let within = triangle.map(|vertex| inside(position(vertex)));
            if is_marked || within.contains(&true) {
                triangles.push(index);
                region_marked.push(is_marked);
                at_edge.push(within.contains(&false));
            }
        }
        let from_edge = rings_from_edge(mesh, &triangles, &at_edge);

        let mut in_region = vec![false; mesh.vertices.len()];
        for &index in &triangles {
            for &vertex in &corners[index] {
                if let Some(flag) = in_region.get_mut(vertex as usize) {
                    *flag = true;
                }
            }
        }
        // A region is part of a mesh whose vertices are counted in `u32`, so
        // neither count below can overflow one.
        let mut local_of = vec![u32::MAX; mesh.vertices.len()];
        let mut vertices: Vec<u32> = Vec::new();
        let mut region_vertices: Vec<EditVertex> = Vec::new();
        for (vertex, _) in in_region.iter().enumerate().filter(|(_, &used)| used) {
            local_of[vertex] = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
            vertices.push(u32::try_from(vertex).unwrap_or(u32::MAX));
            region_vertices.push(mesh.vertices[vertex]);
        }
        let indices = triangles
            .iter()
            .flat_map(|&index| corners[index])
            .map(|vertex| local_of.get(vertex as usize).copied().unwrap_or(u32::MAX))
            .collect();

        let region = Self {
            mesh: MeshEditBuffers {
                vertices: region_vertices,
                indices,
                topology: mesh.topology,
            },
            triangles,
            vertices,
            context: at_edge,
        };
        let faces = MarkedFaces {
            marked: region_marked,
            from_edge,
        };
        (region, faces)
    }

    /// Put a capped region back into the scan it was cut from. Scan triangles
    /// keep their order; the region's surviving ones are rewritten in place,
    /// and their vertices take the normals the region has for them. The caps
    /// follow, and the vertices the filler added follow the scan's.
    pub(super) fn splice(&self, mesh: &MeshEditBuffers, capped: CappedSurface) -> CappedSurface {
        let scan_vertices = mesh.vertices.len();
        let region_vertices = self.vertices.len();
        let mut vertices = mesh.vertices.clone();
        vertices.extend(capped.vertices.iter().skip(region_vertices));
        let mut global = |local: u32| -> u32 {
            let local = local as usize;
            match self.vertices.get(local) {
                Some(&vertex) => {
                    vertices[vertex as usize].normal = capped.vertices[local].normal;
                    vertex
                }
                // Past the region's own vertices come the ones the filler
                // added, in the order they follow the scan's.
                None => {
                    u32::try_from(scan_vertices + (local - region_vertices)).unwrap_or(u32::MAX)
                }
            }
        };

        let scan_triangles = mesh.triangle_count();
        let mut kept = capped.kept.as_ref().map(|_| vec![true; scan_triangles]);
        let mut indices: Vec<u32> = Vec::with_capacity(mesh.indices.len() + capped.indices.len());
        let mut rewritten = capped.indices.as_chunks::<3>().0.iter();
        let mut region = self.triangles.iter().copied().enumerate().peekable();
        for (index, triangle) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
            let Some((slot, _)) = region.next_if(|&(_, scan)| scan == index) else {
                indices.extend_from_slice(triangle);
                continue;
            };
            let survived = capped.kept.as_ref().is_none_or(|kept| kept[slot]);
            if let Some(kept) = kept.as_mut() {
                kept[index] = survived;
            }
            if !survived {
                continue;
            }
            match rewritten.next() {
                Some(_) if self.context[slot] => indices.extend_from_slice(triangle),
                Some(triangle) => indices.extend(triangle.iter().map(|&local| global(local))),
                None => {}
            }
        }
        // Whatever the region has left is caps.
        indices.extend(rewritten.flatten().map(|&local| global(local)));

        CappedSurface {
            vertices,
            indices,
            kept,
        }
    }
}

/// Rings of faces between every face of a region and the region's edge: zero
/// for a face `at_edge`, one more for each step across a shared corner
/// position, and [`EDGE_RINGS`] for every face at least that far in.
fn rings_from_edge(mesh: &MeshEditBuffers, triangles: &[usize], at_edge: &[bool]) -> Vec<u8> {
    let mut rings: Vec<u8> = at_edge
        .iter()
        .map(|&at_edge| if at_edge { 0 } else { EDGE_RINGS })
        .collect();
    let mut frontier: Vec<usize> = (0..triangles.len())
        .filter(|&slot| rings[slot] == 0)
        .collect();
    if frontier.is_empty() {
        return rings;
    }

    let corners = mesh.indices.as_chunks::<3>().0;
    // Every corner of the region with its position, sorted so the corners at
    // one position are a run; each corner then knows its run.
    let mut by_position: Vec<([u32; 3], usize)> = triangles
        .iter()
        .enumerate()
        .flat_map(|(slot, &index)| {
            corners[index]
                .iter()
                .enumerate()
                .map(move |(corner, &vertex)| {
                    let key = mesh
                        .vertices
                        .get(vertex as usize)
                        .map_or([u32::MAX; 3], |vertex| {
                            canonical_position_key(vertex.position)
                        });
                    (key, slot * 3 + corner)
                })
        })
        .collect();
    by_position.sort_unstable();
    let mut run_of = vec![0_usize; by_position.len()];
    let mut run_starts: Vec<usize> = Vec::new();
    for (at, (key, corner)) in by_position.iter().enumerate() {
        if at == 0 || by_position[at - 1].0 != *key {
            run_starts.push(at);
        }
        run_of[*corner] = run_starts.len() - 1;
    }
    run_starts.push(by_position.len());

    // A position hands out its faces once: whoever reaches it first is one
    // ring closer to the edge than they are.
    let mut spent = vec![false; run_starts.len()];
    for depth in 1..EDGE_RINGS {
        let mut grown: Vec<usize> = Vec::new();
        for &slot in &frontier {
            for &run in &run_of[slot * 3..slot * 3 + 3] {
                if std::mem::replace(&mut spent[run], true) {
                    continue;
                }
                for &(_, neighbour) in &by_position[run_starts[run]..run_starts[run + 1]] {
                    let neighbour = neighbour / 3;
                    if rings[neighbour] > depth {
                        rings[neighbour] = depth;
                        grown.push(neighbour);
                    }
                }
            }
        }
        frontier = grown;
    }
    rings
}
