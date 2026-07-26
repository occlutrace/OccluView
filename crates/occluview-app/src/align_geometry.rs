//! The arrays Align Scans hands to its worker, kept between jobs.
//!
//! [`occluview_core::Mesh`] stores interleaved vertices, and
//! [`occluview_align`] takes flat `xyz` triples, so every job's positions have
//! to be built by hand. Building them per submit is what made the heatmap feel
//! slow: Measure is re-submitted on every settings change, and a 945k-vertex
//! arch costs eleven megabytes of copying each time — for geometry that has not
//! changed since the last press.
//!
//! Both caches here are keyed by what the arrays were built FROM (the mesh's
//! geometry identity, and for world positions its pose), never by a layer id.
//! A sculpt mints a fresh geometry id precisely so geometry-derived caches can
//! tell that the surface moved under them.

use std::sync::Arc;

use glam::{Affine3A, Vec3};
use occluview_core::{Mesh, SceneMesh, Vertex};
use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSlice, ParallelSliceMut};

/// How many arrays of each kind are remembered. Two layers, each possibly
/// wanted in local and in world coordinates, and Swap flips which is which.
const SLOTS: usize = 4;

/// A cheap identity for a transform, so a cached array is reused only while the
/// layer really has not moved.
pub(crate) fn transform_key(transform: Affine3A) -> u64 {
    let mut key = 0u64;
    for value in transform.to_cols_array() {
        key = key
            .rotate_left(7)
            .wrapping_add(u64::from(value.to_bits()))
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    key
}

/// One remembered array and what it was built from.
type Slot<K, T> = Vec<(K, Arc<T>)>;

/// Flat position and index arrays for the align worker, cached by the geometry
/// and pose they were built from.
#[derive(Default)]
pub(crate) struct AlignGeometry {
    /// Positions in a layer's own local frame, keyed by geometry identity.
    local: Slot<u64, Vec<f32>>,
    /// Positions posed into world, keyed by geometry identity and pose.
    world: Slot<(u64, u64), Vec<f32>>,
    /// Triangle indices, keyed by geometry identity.
    indices: Slot<u64, Vec<u32>>,
}

impl AlignGeometry {
    /// A layer's vertex positions in its own local frame.
    pub(crate) fn local_positions(&mut self, entry: &SceneMesh) -> Arc<Vec<f32>> {
        let key = entry.mesh.geometry_id();
        remember(&mut self.local, key, || {
            entry
                .mesh
                .vertices()
                .iter()
                .flat_map(|vertex| vertex.position)
                .collect()
        })
    }

    /// A layer's vertex positions posed into world.
    pub(crate) fn world_positions(&mut self, entry: &SceneMesh) -> Arc<Vec<f32>> {
        let key = (entry.mesh.geometry_id(), transform_key(entry.transform));
        remember(&mut self.world, key, || {
            entry
                .mesh
                .vertices()
                .iter()
                .flat_map(|vertex| {
                    entry
                        .transform
                        .transform_point3(Vec3::from_array(vertex.position))
                        .to_array()
                })
                .collect()
        })
    }

    /// A layer's triangle indices.
    pub(crate) fn indices(&mut self, entry: &SceneMesh) -> Arc<Vec<u32>> {
        let key = entry.mesh.geometry_id();
        remember(&mut self.indices, key, || entry.mesh.indices().to_vec())
    }

    /// Drop everything. Called when the tool closes, so a session's worth of
    /// arrays does not sit in memory behind an operator who has moved on.
    pub(crate) fn clear(&mut self) {
        self.local.clear();
        self.world.clear();
        self.indices.clear();
    }
}

/// Return the array already cached under `key`, or build and remember it.
///
/// Most-recently-used first, so the two layers of a pair keep their slots while
/// a third geometry passing through falls off the end.
fn remember<K: PartialEq + Copy, T, F: FnOnce() -> T>(
    slots: &mut Slot<K, T>,
    key: K,
    make: F,
) -> Arc<T> {
    if let Some(at) = slots.iter().position(|(cached, _)| *cached == key) {
        let hit = slots.remove(at);
        let shared = Arc::clone(&hit.1);
        slots.insert(0, hit);
        return shared;
    }
    let fresh = Arc::new(make());
    slots.insert(0, (key, Arc::clone(&fresh)));
    slots.truncate(SLOTS);
    fresh
}

/// One layer's vertices, held with whatever colours are currently on it.
#[derive(Default)]
struct PaintedSlot {
    /// Which geometry the positions, normals, and UVs below came from.
    geometry: Option<u64>,
    /// The layer's vertices, with the overlay colour written over each.
    vertices: Vec<Vertex>,
}

/// The vertex arrays an overlay is pushed to the GPU through.
///
/// The upload path takes whole vertices, but a re-colour only changes four
/// bytes of each. Rebuilding the array per re-colour allocates and copies
/// thirty-four megabytes on a full arch; keeping it and overwriting the colour
/// field in place costs a tenth of that and no allocation at all.
///
/// Two slots, because both meshes in an alignment can carry markings at once
/// and one slot would rebuild the whole array every time the brush crossed from
/// one to the other.
#[derive(Default)]
pub(crate) struct PaintedVertices {
    slots: [PaintedSlot; 2],
}

impl PaintedVertices {
    /// The slot holding `mesh`, made ready to carry `count` colours.
    ///
    /// Picks the slot that already holds this geometry, else the one that holds
    /// nothing, else the first — two meshes are all an alignment has.
    fn slot_for(&mut self, mesh: &Mesh, count: usize) -> &mut PaintedSlot {
        let id = mesh.geometry_id();
        let at = self
            .slots
            .iter()
            .position(|slot| slot.geometry == Some(id))
            .or_else(|| self.slots.iter().position(|slot| slot.geometry.is_none()))
            .unwrap_or(0);
        let slot = &mut self.slots[at];
        if slot.geometry != Some(id) || slot.vertices.len() != count {
            slot.vertices.clear();
            slot.vertices.extend_from_slice(mesh.vertices());
            slot.geometry = Some(id);
        }
        slot
    }

    /// The layer's vertices carrying `colors`, or `None` if the map does not
    /// describe this mesh.
    pub(crate) fn repaint(&mut self, mesh: &Mesh, colors: &[[u8; 4]]) -> Option<&[Vertex]> {
        // Chunked rather than zipped per element: a million independent writes
        // of four bytes is dominated by iterator overhead, and the chunk size
        // only has to be large enough that scheduling disappears.
        const CHUNK: usize = 16_384;

        if mesh.vertices().len() != colors.len() {
            return None;
        }
        let slot = self.slot_for(mesh, colors.len());
        slot.vertices
            .par_chunks_mut(CHUNK)
            .zip(colors.par_chunks(CHUNK))
            .for_each(|(vertices, colors)| {
                for (vertex, color) in vertices.iter_mut().zip(colors) {
                    vertex.color = *color;
                }
            });
        Some(&slot.vertices)
    }

    /// Rewrite only `touched`, leaving every other vertex as it was.
    ///
    /// The whole point of the brush being fast. A dab the size of a cusp
    /// touches a few hundred vertices out of a million; repainting the array
    /// for those is thirty-four megabytes of memory traffic, and the upload
    /// that follows it is thirty-four more. Together they are what made
    /// painting run at three frames a second.
    pub(crate) fn patch(
        &mut self,
        mesh: &Mesh,
        colors: &[[u8; 4]],
        touched: &[u32],
    ) -> Option<&[Vertex]> {
        if mesh.vertices().len() != colors.len() {
            return None;
        }
        let slot = self.slot_for(mesh, colors.len());
        for index in touched {
            let at = *index as usize;
            let (Some(vertex), Some(color)) = (slot.vertices.get_mut(at), colors.get(at)) else {
                continue;
            };
            vertex.color = *color;
        }
        Some(&slot.vertices)
    }

    /// Whether this mesh already has a slot ready to be patched. A patch onto a
    /// slot that has to be rebuilt first is not a saving, so the caller repaints
    /// instead.
    pub(crate) fn holds(&self, mesh: &Mesh, count: usize) -> bool {
        let id = mesh.geometry_id();
        self.slots
            .iter()
            .any(|slot| slot.geometry == Some(id) && slot.vertices.len() == count)
    }

    /// Drop the arrays. The overlay is gone, so the scratch should be too.
    pub(crate) fn clear(&mut self) {
        self.slots = Default::default();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::float_cmp, clippy::unwrap_used)]

    use super::{transform_key, AlignGeometry, PaintedVertices, SLOTS};
    use glam::{Affine3A, Vec3};
    use occluview_core::{Mesh, SceneMesh, Vertex};

    fn triangle() -> Mesh {
        Mesh::new(
            None,
            vec![
                Vertex::at(Vec3::ZERO),
                Vertex::at(Vec3::new(1.0, 0.0, 0.0)),
                Vertex::at(Vec3::new(0.0, 1.0, 0.0)),
            ],
            vec![0, 1, 2],
        )
        .expect("valid mesh")
    }

    /// The whole point: a second job over unchanged geometry must not copy the
    /// mesh again. Sharing the same allocation is what makes that observable.
    #[test]
    fn unchanged_geometry_is_handed_out_without_being_rebuilt() {
        let entry = SceneMesh::new(triangle());
        let mut cache = AlignGeometry::default();
        let first = cache.local_positions(&entry);
        let second = cache.local_positions(&entry);
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "a repeat submit must share the array, not build a second one"
        );
        assert!(std::sync::Arc::ptr_eq(
            &cache.indices(&entry),
            &cache.indices(&entry)
        ));
    }

    /// World positions are a function of the pose, so moving the layer has to
    /// invalidate them — a stale array would measure against where the scan
    /// used to be.
    #[test]
    fn moving_a_layer_rebuilds_its_world_positions() {
        let mut entry = SceneMesh::new(triangle());
        let mut cache = AlignGeometry::default();
        let before = cache.world_positions(&entry);
        entry.transform = Affine3A::from_translation(Vec3::new(0.0, 0.0, 5.0));
        let after = cache.world_positions(&entry);
        assert!(!std::sync::Arc::ptr_eq(&before, &after));
        assert_eq!(after[2], 5.0, "the array must carry the new pose");
        // The old pose is still remembered, so undoing a drag is free too.
        assert!(cache.world.len() <= SLOTS);
    }

    /// A pose that never changed must key the same way twice, or every job
    /// would miss the cache and the whole thing would be dead weight.
    #[test]
    fn the_same_pose_keys_the_same_way() {
        let pose = Affine3A::from_translation(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(transform_key(pose), transform_key(pose));
        assert_ne!(transform_key(pose), transform_key(Affine3A::IDENTITY));
    }

    /// The scratch buffer keeps the scan's own positions and normals and only
    /// takes the measured colour. A map that overwrote geometry would push a
    /// deformed scan to the screen.
    #[test]
    fn repainting_changes_only_the_colour() {
        let mesh = triangle();
        let mut painted = PaintedVertices::default();
        let colors = vec![[1u8, 2, 3, 255]; 3];
        let vertices = painted.repaint(&mesh, &colors).expect("a repaint");
        for (before, after) in mesh.vertices().iter().zip(vertices) {
            assert_eq!(before.position, after.position);
            assert_eq!(before.normal, after.normal);
            assert_eq!(before.uv, after.uv);
            assert_eq!(after.color, [1, 2, 3, 255]);
        }
    }

    /// A map that does not describe this mesh must be refused, not stretched
    /// over whatever vertices happen to be there.
    #[test]
    fn a_map_of_the_wrong_length_is_refused() {
        let mut painted = PaintedVertices::default();
        assert!(painted.repaint(&triangle(), &[[0, 0, 0, 255]; 2]).is_none());
    }
}
