//! The part of a mesh a mark puts in front of the hole filler.
//!
//! A mark is local, and so is everything Close Holes does with it: the rims it
//! may close run through marked faces, a cap reads the scan triangles at its
//! rim, and the piercing guard looks two rings further. This module
//! cuts that neighbourhood out as a mesh of its own, so welding, healing, the
//! boundary walk and the caps cost what the marked area costs, and puts the
//! result back into the scan. What remains proportional to the scan is a few
//! plain passes over its buffers.
//!
//! Faces are neighbours when they share a corner position: a scan read from an
//! STL has one vertex per corner, and that is the only adjacency it carries.

use glam::Vec3;

use super::topology::canonical_position_key;
use super::{EditVertex, FaceSelection, MeshEditBuffers};

/// Rings of faces around the mark in which a marked rim may still run. A rim
/// that strays further has left the marked area and stays open.
///
/// The reach is what separates the faces a mark misses from the faces it was
/// not meant for. Measured on real arches with lasso marks: where a surface
/// lasso encloses a hole, the rim faces it leaves unmarked, the ones looking
/// away from the camera, lie three to five rings from a marked face; where a
/// lasso covers a little more than half of a hole, the far side of the rim
/// lies twenty-five rings away and more.
pub(super) const MARK_REACH_RINGS: u8 = 8;

/// Rings of faces a region keeps around the mark. A rim face may lie
/// [`MARK_REACH_RINGS`] out; the faces around its rim vertices lie one ring
/// further, and the faces the piercing guard reads, around the vertices two
/// rings from the rim, three.
const REGION_RINGS: u8 = MARK_REACH_RINGS + 3;

/// Ring of a face that is not part of the region.
const OUTSIDE: u8 = u8::MAX;

/// How far every face of a region lies from the mark, in rings of faces; a
/// marked face is at zero.
pub(super) struct MarkRings(Vec<u8>);

impl MarkRings {
    /// The marked faces as a selection.
    pub(super) fn marked(&self) -> FaceSelection {
        FaceSelection::new(self.0.iter().map(|&ring| ring == 0).collect())
    }

    /// Whether the operator marked this face.
    pub(super) fn is_marked(&self, triangle: usize) -> bool {
        self.0.get(triangle) == Some(&0)
    }

    /// Whether this face is marked or within reach of the mark.
    pub(super) fn is_near(&self, triangle: usize) -> bool {
        self.0
            .get(triangle)
            .is_some_and(|&ring| ring <= MARK_REACH_RINGS)
    }

    /// Drop the faces a healing pass removed.
    pub(super) fn retain(&mut self, keep: &[bool]) {
        let mut kept = keep.iter();
        self.0.retain(|_| kept.next().copied().unwrap_or(true));
    }

    /// Per vertex of `mesh`, whether it is a corner of a marked face.
    pub(super) fn marked_corners(&self, mesh: &MeshEditBuffers) -> Vec<bool> {
        self.corners_of(mesh, 0)
    }

    /// Per vertex of `mesh`, whether it is a corner of a face of the outermost
    /// ring. The surface goes on past those faces, so around such a vertex
    /// faces may be missing and what looks like a boundary there says nothing.
    pub(super) fn outermost_corners(&self, mesh: &MeshEditBuffers) -> Vec<bool> {
        self.corners_of(mesh, REGION_RINGS)
    }

    fn corners_of(&self, mesh: &MeshEditBuffers, wanted: u8) -> Vec<bool> {
        let mut corners = vec![false; mesh.vertices.len()];
        for (triangle, &ring) in mesh.indices.as_chunks::<3>().0.iter().zip(&self.0) {
            if ring == wanted {
                for &vertex in triangle {
                    if let Some(flag) = corners.get_mut(vertex as usize) {
                        *flag = true;
                    }
                }
            }
        }
        corners
    }
}

/// The faces a mark reaches, as a mesh of their own, with the way back.
pub(super) struct MarkedRegion {
    /// The region's faces and vertices, in the order the scan has them.
    pub(super) mesh: MeshEditBuffers,
    /// Scan triangle of every region triangle, ascending.
    triangles: Vec<usize>,
    /// Scan vertex of every region vertex, ascending.
    vertices: Vec<u32>,
    /// Per region triangle, whether it is of the outermost ring. Those faces
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
    /// Cut the region around the marked faces of `mesh`, and say how far each
    /// of its faces lies from the mark. The selection has one entry per
    /// triangle and marks at least one.
    pub(super) fn around(mesh: &MeshEditBuffers, selection: &FaceSelection) -> (Self, MarkRings) {
        let corners = mesh.indices.as_chunks::<3>().0;
        let marked = selection.as_slice();
        let position = |vertex: u32| {
            mesh.vertices
                .get(vertex as usize)
                .map_or(Vec3::NAN, |vertex| Vec3::from_array(vertex.position))
        };

        // The mark's box and the mean length of its edges. The rings are
        // looked for among the faces that reach into the box grown by a
        // margin, and the first margin is a guess at how far the rings go.
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
        let mut margin = mean_edge * f32::from(REGION_RINGS) * 2.0;
        if !(margin.is_finite() && margin > 0.0) {
            margin = f32::INFINITY;
        }

        let mut widenings = 0;
        let (triangles, rings) = loop {
            let (box_lo, box_hi) = (lo - margin, hi + margin);
            let inside = |point: Vec3| point.cmpge(box_lo).all() && point.cmple(box_hi).all();
            let candidates: Vec<usize> = (0..corners.len())
                .filter(|&index| {
                    marked.get(index).copied().unwrap_or(false)
                        || corners[index]
                            .iter()
                            .any(|&vertex| inside(position(vertex)))
                })
                .collect();
            let rings = rings_from_mark(mesh, &candidates, marked);
            // A face outside the candidates has no corner in the box, so it
            // can only neighbour a face that has a corner outside it too. The
            // rings found are the true ones once no face short of the last
            // ring is such a face.
            let settled = candidates.iter().zip(&rings).all(|(&index, &ring)| {
                ring >= REGION_RINGS
                    || corners[index]
                        .iter()
                        .all(|&vertex| inside(position(vertex)))
            });
            if settled || margin.is_infinite() {
                break candidates
                    .into_iter()
                    .zip(rings)
                    .filter(|&(_, ring)| ring != OUTSIDE)
                    .unzip::<usize, u8, Vec<usize>, Vec<u8>>();
            }
            // A mesh whose edges grow away from the mark: widen and look
            // again, and after a few rounds take every face, where the rings
            // are exact whatever the coordinates.
            widenings += 1;
            margin = if widenings < 4 {
                margin * 4.0
            } else {
                f32::INFINITY
            };
        };

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
            context: rings.iter().map(|&ring| ring == REGION_RINGS).collect(),
        };
        let rings = MarkRings(rings);
        (region, rings)
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

/// Rings of faces from the mark for every candidate: zero for a marked face,
/// one more for each step across a shared corner position, [`OUTSIDE`] past
/// [`REGION_RINGS`].
fn rings_from_mark(mesh: &MeshEditBuffers, candidates: &[usize], marked: &[bool]) -> Vec<u8> {
    let corners = mesh.indices.as_chunks::<3>().0;
    // Every candidate corner with its position, sorted so the corners at one
    // position are a run; each corner then knows its run.
    let mut by_position: Vec<([u32; 3], usize)> = candidates
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

    let mut rings: Vec<u8> = candidates
        .iter()
        .map(|&index| {
            if marked.get(index).copied().unwrap_or(false) {
                0
            } else {
                OUTSIDE
            }
        })
        .collect();
    let mut frontier: Vec<usize> = (0..candidates.len())
        .filter(|&slot| rings[slot] == 0)
        .collect();
    // A position hands out its faces once: whoever reaches it first is one
    // ring closer than they are.
    let mut spent = vec![false; run_starts.len()];
    for depth in 1..=REGION_RINGS {
        let mut grown: Vec<usize> = Vec::new();
        for &slot in &frontier {
            for &run in &run_of[slot * 3..slot * 3 + 3] {
                if std::mem::replace(&mut spent[run], true) {
                    continue;
                }
                for &(_, neighbour) in &by_position[run_starts[run]..run_starts[run + 1]] {
                    let neighbour = neighbour / 3;
                    if rings[neighbour] == OUTSIDE {
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
