//! Which side of the surface a point lies on, and where the surface ends.
//!
//! The side is decided by the normal of the closest *feature*, not of whichever
//! triangle carries the closest point: inside a face it is the face normal,
//! along an edge the mean of the faces that share it, and at a vertex the
//! corner-angle-weighted mean of the faces around it (the angle-weighted
//! pseudonormal, Bærentzen & Aanæs, IEEE TVCG 2005). Signing with the face
//! normal alone gets a point outside a sharp edge wrong whenever the face
//! listed first in the file points away from it.
//!
//! The same adjacency marks the open border: an edge only one kept triangle
//! uses, and the vertices on such edges. Triangles excluded from the index by
//! a mask do not count, so the rim of a painted-out region is border as well.
//! A zero-area triangle still counts: it joins its neighbours, it only has no
//! normal to contribute.

use std::collections::HashMap;

use glam::{DVec3, Vec3};

use super::surface_geometry::Feature;

/// Per-slot and per-vertex topology of the triangles an index kept.
#[derive(Clone, Debug, Default)]
pub(super) struct Topology {
    /// Dense welded-vertex id of each corner, per slot.
    pub(super) corners: Vec<[u32; 3]>,
    /// Unit pseudonormal of each edge of each slot (edge `k` joins corner `k`
    /// to corner `k + 1`).
    pub(super) edge_normals: Vec<[Vec3; 3]>,
    /// Bit `k` set when edge `k` of the slot is on the open border.
    pub(super) edge_border: Vec<u8>,
    /// Unit angle-weighted pseudonormal per welded vertex.
    pub(super) vertex_normals: Vec<Vec3>,
    /// Whether a welded vertex lies on the open border.
    pub(super) vertex_border: Vec<bool>,
}

impl Topology {
    /// The outward normal to sign against at `feature` of `slot`, and whether
    /// that feature lies on the open border. Falls back to `face` wherever a
    /// pseudonormal is missing or collapsed (a vertex whose faces cancel).
    pub(super) fn at(&self, slot: usize, feature: Feature, face: DVec3) -> (DVec3, bool) {
        let usable = |normal: Vec3| {
            let normal = normal.as_dvec3();
            if normal.is_finite() && normal.length_squared() > 0.25 {
                normal
            } else {
                face
            }
        };
        match feature {
            Feature::Face => (face, false),
            Feature::Edge(edge) => {
                let edge = usize::from(edge) % 3;
                let normal = self
                    .edge_normals
                    .get(slot)
                    .map_or(face, |normals| usable(normals[edge]));
                let border = self
                    .edge_border
                    .get(slot)
                    .is_some_and(|bits| bits & (1 << edge) != 0);
                (normal, border)
            }
            Feature::Corner(corner) => {
                let Some(&vertex) = self
                    .corners
                    .get(slot)
                    .map(|ids| &ids[usize::from(corner) % 3])
                else {
                    return (face, false);
                };
                let vertex = vertex as usize;
                let normal = self
                    .vertex_normals
                    .get(vertex)
                    .map_or(face, |&normal| usable(normal));
                let border = self.vertex_border.get(vertex).copied().unwrap_or(false);
                (normal, border)
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct EdgeUse {
    count: u32,
    normal_sum: DVec3,
}

/// Collects triangles in source order and turns them into a [`Topology`].
///
/// Vertices are identified by the caller's welded key (equal positions share
/// one id even in a soup that repeats every corner), and remapped to dense ids
/// here so the per-vertex arrays are sized by distinct positions only.
#[derive(Default)]
pub(super) struct TopologyBuilder {
    dense: HashMap<usize, u32>,
    edges: HashMap<(u32, u32), EdgeUse>,
    vertex_sums: Vec<DVec3>,
    kept: Vec<[u32; 3]>,
}

impl TopologyBuilder {
    fn id(&mut self, welded: usize) -> u32 {
        let next = u32::try_from(self.vertex_sums.len()).unwrap_or(u32::MAX);
        let id = *self.dense.entry(welded).or_insert(next);
        if id == next {
            self.vertex_sums.push(DVec3::ZERO);
        }
        id
    }

    fn edge_key(ids: [u32; 3], edge: usize) -> (u32, u32) {
        let (a, b) = (ids[edge], ids[(edge + 1) % 3]);
        (a.min(b), a.max(b))
    }

    /// A triangle that is not indexed (zero area) but still joins its
    /// neighbours along its edges.
    pub(super) fn add_connector(&mut self, welded: [usize; 3]) {
        let ids = welded.map(|vertex| self.id(vertex));
        for edge in 0..3 {
            self.edges
                .entry(Self::edge_key(ids, edge))
                .or_default()
                .count += 1;
        }
    }

    /// An indexed triangle, in slot order before any reordering.
    pub(super) fn add_kept(
        &mut self,
        welded: [usize; 3],
        corners: &[DVec3; 3],
        unit_normal: DVec3,
    ) {
        let ids = welded.map(|vertex| self.id(vertex));
        for edge in 0..3 {
            let entry = self.edges.entry(Self::edge_key(ids, edge)).or_default();
            entry.count += 1;
            entry.normal_sum += unit_normal;
        }
        for corner in 0..3 {
            let to_next = corners[(corner + 1) % 3] - corners[corner];
            let to_previous = corners[(corner + 2) % 3] - corners[corner];
            let angle = to_next.angle_between(to_previous);
            if angle.is_finite() {
                if let Some(sum) = self.vertex_sums.get_mut(ids[corner] as usize) {
                    *sum += unit_normal * angle;
                }
            }
        }
        self.kept.push(ids);
    }

    pub(super) fn finish(self) -> Topology {
        let mut vertex_border = vec![false; self.vertex_sums.len()];
        let mut edge_normals = Vec::with_capacity(self.kept.len());
        let mut edge_border = Vec::with_capacity(self.kept.len());
        for &ids in &self.kept {
            let mut normals = [Vec3::ZERO; 3];
            let mut border = 0u8;
            for (edge, normal) in normals.iter_mut().enumerate() {
                let key = Self::edge_key(ids, edge);
                let Some(entry) = self.edges.get(&key) else {
                    continue;
                };
                *normal = entry.normal_sum.normalize_or_zero().as_vec3();
                if entry.count == 1 {
                    border |= 1 << edge;
                    for vertex in [key.0, key.1] {
                        if let Some(flag) = vertex_border.get_mut(vertex as usize) {
                            *flag = true;
                        }
                    }
                }
            }
            edge_normals.push(normals);
            edge_border.push(border);
        }
        Topology {
            corners: self.kept,
            edge_normals,
            edge_border,
            vertex_normals: self
                .vertex_sums
                .iter()
                .map(|sum| sum.normalize_or_zero().as_vec3())
                .collect(),
            vertex_border,
        }
    }
}
