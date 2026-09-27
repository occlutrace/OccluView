//! Adapter between the app's sculpt contract and the `occlu-sculpt` kernel.
//!
//! The worker, commit, undo and abort paths all speak in [`BrushStroke`] and
//! [`BrushStrokeOutcome`]: a dab is a mesh-local centre with a radius, a
//! strength, a view direction and a mode, and a dab reports the vertices and
//! faces it moved plus whether it changed the topology. This module answers
//! that contract over [`occlu_sculpt::SculptSession`].
//!
//! The kernel carries positions and normals only, so this adapter owns the
//! attribute mirror: it keeps every vertex's colour and texture coordinate, and
//! blends a minted vertex's attributes from the two corners of the edge the
//! kernel split.

use crate::sculpt_tool::SculptTip;
use glam::DVec3;
use occlu_sculpt::{BrushMode as KernelMode, Dab, SculptSession, TipStamp};
use occluview_core::{EditVertex, MeshEditBuffers, MeshEditError, MeshTopology, Vertex};
use occluview_render::{SculptFaceUpdate, SculptTopologyDelta, SculptVertexUpdate};
use std::sync::atomic::{AtomicBool, Ordering};

/// Which sculpting operation a dab performs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum BrushMode {
    /// Relax the surface: iron out grain and even the tessellation.
    Smooth,
    /// Build material along the surface normal under the brush.
    Add,
    /// Carve material away against the surface normal.
    Remove,
}

/// One brush dab in the layer's mesh-local space.
#[derive(Copy, Clone, Debug)]
pub(crate) struct BrushStroke {
    /// Mesh-local dab centre, on the surface the pointer hit.
    pub(crate) center: [f32; 3],
    /// Falloff radius in mesh-local millimetres.
    pub(crate) radius_mm: f32,
    /// Per-dab strength, 0..1.
    pub(crate) strength: f32,
    /// Unit view direction, from the camera into the scene.
    pub(crate) view_dir: [f32; 3],
}

/// A prepared freeform-sculpting session over one mesh.
pub(crate) struct BrushSession {
    kernel: SculptSession,
    /// Attribute mirror, one entry per kernel vertex, in the same order.
    vertices: Vec<EditVertex>,
    /// Live triangle indices, patched from the kernel's changed face rows.
    indices: Vec<u32>,
    /// Whether the kernel has an open stroke. The kernel gates live remeshing
    /// on it, so the first dab of a drag opens one and the commit closes it.
    stroke_open: bool,
}

/// One dab's result: sparse vertex ids, dirty pick faces, and an optional
/// append/patch delta when topology changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct BrushStrokeOutcome {
    /// Vertex ids whose position or normal changed, sorted and deduplicated.
    pub(crate) touched_vertices: Vec<usize>,
    /// Face ids whose corners may have moved or been rewired, sorted.
    pub(crate) dirty_triangles: Vec<usize>,
    /// Local geometry update for a topology-changing dab.
    pub(crate) topology_delta: Option<SculptTopologyDelta>,
}

impl BrushStrokeOutcome {
    /// Whether this dab changed the triangle list, so a sparse vertex update
    /// would leave the caller's uploaded geometry stale in size and content.
    #[must_use]
    pub(crate) fn topology_changed(&self) -> bool {
        self.topology_delta.is_some()
    }
}

impl BrushSession {
    /// Prepare a session over `mesh`'s vertices and indices.
    ///
    /// # Errors
    /// Returns [`MeshEditError::MalformedMesh`] for a point cloud, for indices
    /// that are not three-aligned, or for one that addresses a vertex the
    /// buffer does not hold.
    pub(crate) fn prepare(mesh: &MeshEditBuffers) -> Result<Self, MeshEditError> {
        if mesh.topology != MeshTopology::TriangleMesh {
            return Err(MeshEditError::MalformedMesh {
                reason: "sculpt requires a triangle mesh, not a point cloud".to_string(),
            });
        }
        if mesh.indices.is_empty() || !mesh.indices.len().is_multiple_of(3) {
            return Err(MeshEditError::MalformedMesh {
                reason: format!(
                    "sculpt requires whole triangles, found {} indices",
                    mesh.indices.len()
                ),
            });
        }
        let vertex_count = mesh.vertices.len();
        if let Some(&index) = mesh
            .indices
            .iter()
            .find(|&&index| index as usize >= vertex_count)
        {
            return Err(MeshEditError::MalformedMesh {
                reason: format!(
                    "triangle index {index} addresses past the {vertex_count} vertices"
                ),
            });
        }
        let mut positions = Vec::with_capacity(vertex_count * 3);
        for vertex in &mesh.vertices {
            positions.extend_from_slice(&vertex.position);
        }
        Ok(Self {
            kernel: SculptSession::new(positions, mesh.indices.clone()),
            vertices: mesh.vertices.clone(),
            indices: mesh.indices.clone(),
            stroke_open: false,
        })
    }

    /// Apply one dab with `tip`, oriented along `axis` when the tip is the
    /// knife. See the module docs for the contract.
    // One dab is named by its ray, dose, mode, tip and bearing together.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_stroke(
        &mut self,
        stroke: BrushStroke,
        mode: BrushMode,
        tip: SculptTip,
        axis: Option<[f32; 3]>,
    ) -> BrushStrokeOutcome {
        self.apply(stroke, mode, tip, axis)
    }

    /// Cancellable form. A cancellation never returns an outcome: the owning
    /// worker is being torn down, so its partially updated session and mirror
    /// must be discarded together.
    // The cancellation flag rides beside the dab's own arguments.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_stroke_cancellable(
        &mut self,
        stroke: BrushStroke,
        mode: BrushMode,
        tip: SculptTip,
        axis: Option<[f32; 3]>,
        cancel: &AtomicBool,
    ) -> Option<BrushStrokeOutcome> {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let outcome = self.apply(stroke, mode, tip, axis);
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        Some(outcome)
    }

    /// Live vertex attributes, one entry per kernel vertex.
    pub(crate) fn vertices(&self) -> &[EditVertex] {
        &self.vertices
    }

    /// Current live triangle rows, including append and swap-delete updates.
    pub(crate) fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Seal the current stroke, so the next dab opens a new one.
    pub(crate) fn finish_stroke(&mut self) {
        if self.stroke_open {
            let _ = self.kernel.end_stroke();
            self.stroke_open = false;
        }
    }

    fn apply(
        &mut self,
        stroke: BrushStroke,
        mode: BrushMode,
        tip: SculptTip,
        axis: Option<[f32; 3]>,
    ) -> BrushStrokeOutcome {
        if !self.stroke_open {
            self.kernel.start_stroke();
            self.stroke_open = true;
        }
        let base_vertex_count = self.vertices.len();
        let base_index_count = self.indices.len();
        let base_revision = self.kernel.topology_revision();
        let radius = f64::from(stroke.radius_mm);
        let view = DVec3::new(
            f64::from(stroke.view_dir[0]),
            f64::from(stroke.view_dir[1]),
            f64::from(stroke.view_dir[2]),
        );
        let mut center = DVec3::new(
            f64::from(stroke.center[0]),
            f64::from(stroke.center[1]),
            f64::from(stroke.center[2]),
        );
        // The contract says a dab centre is a surface point, and the kernel
        // floods around the centre rather than around the ray hit. Project the
        // centre onto the surface it points at so a caller whose hit drifted by
        // a fraction of the radius still sculpts where the pointer is.
        if view.length() > 1e-12 {
            let origin = center - view.normalize_or_zero() * (radius * 2.0 + 1.0);
            if let Some((hit, _)) = self.kernel.raycast(origin, view) {
                center = hit;
            }
        }
        let dab = Dab {
            center,
            radius,
            strength: f64::from(stroke.strength),
            view,
            mode: kernel_mode(mode),
        };
        self.kernel
            .set_brush_tip(TipStamp::from_u32(tip.kernel_stamp()).unwrap_or(TipStamp::Ball));
        self.kernel.set_dab_axis(
            axis.map(|axis| DVec3::new(f64::from(axis[0]), f64::from(axis[1]), f64::from(axis[2]))),
        );
        let touched = self.kernel.dab(&dab);
        let added = self.kernel.dab_added_parents().to_vec();
        let dirty = self
            .kernel
            .dab_dirty_triangles()
            .iter()
            .map(|&face| face as usize)
            .collect::<Vec<_>>();
        self.sync_mirror(&touched, &added);
        let topology_delta = (self.kernel.topology_revision() != base_revision).then(|| {
            let face_updates = self.sync_indices(base_index_count, &dirty);
            let mut updated_vertices = touched
                .iter()
                .copied()
                .filter(|&vertex| (vertex as usize) < base_vertex_count)
                .filter_map(|vertex| {
                    self.vertices
                        .get(vertex as usize)
                        .copied()
                        .map(|value| SculptVertexUpdate {
                            vertex,
                            value: vertex_from_edit_vertex(value),
                        })
                })
                .collect::<Vec<_>>();
            updated_vertices.sort_unstable_by_key(|update| update.vertex);
            updated_vertices.dedup_by_key(|update| update.vertex);
            SculptTopologyDelta {
                base_vertex_count,
                appended_vertices: self.vertices[base_vertex_count..]
                    .iter()
                    .copied()
                    .map(vertex_from_edit_vertex)
                    .collect(),
                updated_vertices,
                base_index_count,
                live_index_count: self.indices.len(),
                face_updates,
                dirty_triangles: dirty.clone(),
            }
        });
        BrushStrokeOutcome {
            touched_vertices: touched.iter().map(|&vertex| vertex as usize).collect(),
            dirty_triangles: dirty,
            topology_delta,
        }
    }

    /// Bring the attribute mirror back in line with the kernel: copy the moved
    /// positions and normals, then blend an attribute row for each minted
    /// vertex.
    fn sync_mirror(&mut self, touched: &[u32], added: &[(u32, u32, u32)]) {
        let live = &self.kernel.verts;
        let normals = self.kernel.normals();
        for &vertex in touched {
            let Some(row) = self.vertices.get_mut(vertex as usize) else {
                continue;
            };
            let offset = vertex as usize * 3;
            row.position = [live[offset], live[offset + 1], live[offset + 2]];
            row.normal = [normals[offset], normals[offset + 1], normals[offset + 2]];
        }
        if added.is_empty() {
            return;
        }
        for &(child, parent_a, parent_b) in added {
            let mut row = blend_parents(
                self.vertices.get(parent_a as usize),
                self.vertices.get(parent_b as usize),
            );
            let offset = child as usize * 3;
            if let (Some(position), Some(normal)) = (
                live.get(offset..offset + 3),
                normals.get(offset..offset + 3),
            ) {
                row.position = [position[0], position[1], position[2]];
                row.normal = [normal[0], normal[1], normal[2]];
            }
            if child as usize == self.vertices.len() {
                self.vertices.push(row);
            } else if let Some(slot) = self.vertices.get_mut(child as usize) {
                *slot = row;
            }
        }
    }

    fn sync_indices(&mut self, base_index_count: usize, dirty: &[usize]) -> Vec<SculptFaceUpdate> {
        let faces = self.kernel.faces();
        let previous_triangles = base_index_count / 3;
        let live_triangles = faces.len() / 3;
        if self.indices.len() > faces.len() {
            self.indices.truncate(faces.len());
        } else if self.indices.len() < faces.len() {
            let start = self.indices.len();
            self.indices.extend_from_slice(&faces[start..]);
        }
        let mut changed: Vec<usize> = dirty
            .iter()
            .copied()
            .filter(|&triangle| triangle < live_triangles)
            .collect();
        changed.extend(previous_triangles.min(live_triangles)..live_triangles);
        changed.sort_unstable();
        changed.dedup();
        let mut updates = Vec::with_capacity(changed.len());
        for triangle in changed {
            let offset = triangle * 3;
            let Some(corners) = faces.get(offset..offset + 3) else {
                continue;
            };
            let current = &self.indices[offset..offset + 3];
            if current != corners || triangle >= previous_triangles {
                self.indices[offset..offset + 3].copy_from_slice(corners);
                let Ok(triangle) = u32::try_from(triangle) else {
                    continue;
                };
                updates.push(SculptFaceUpdate {
                    triangle,
                    indices: [corners[0], corners[1], corners[2]],
                });
            }
        }
        updates
    }
}

pub(crate) fn vertex_from_edit_vertex(vertex: EditVertex) -> Vertex {
    Vertex {
        position: vertex.position,
        normal: vertex.normal,
        color: vertex.color,
        uv: vertex.uv,
    }
}

impl occluview_core::SculptSessionBuffers for BrushSession {
    fn sculpt_vertices(&self) -> &[EditVertex] {
        &self.vertices
    }

    fn sculpt_indices(&self) -> &[u32] {
        &self.indices
    }
}

fn kernel_mode(mode: BrushMode) -> KernelMode {
    match mode {
        BrushMode::Smooth => KernelMode::Smooth,
        BrushMode::Add => KernelMode::Deposit,
        BrushMode::Remove => KernelMode::Erode,
    }
}

/// The midpoint attributes a split mints: normal averaged, colour and texture
/// coordinate blended 50/50 from the edge's two corners.
fn blend_parents(a: Option<&EditVertex>, b: Option<&EditVertex>) -> EditVertex {
    let (Some(a), Some(b)) = (a, b) else {
        return a.or(b).copied().unwrap_or_else(|| EditVertex::at([0.0; 3]));
    };
    EditVertex {
        position: [
            f32::midpoint(a.position[0], b.position[0]),
            f32::midpoint(a.position[1], b.position[1]),
            f32::midpoint(a.position[2], b.position[2]),
        ],
        normal: [0.0; 3],
        color: [
            blend_channel(a.color[0], b.color[0]),
            blend_channel(a.color[1], b.color[1]),
            blend_channel(a.color[2], b.color[2]),
            blend_channel(a.color[3], b.color[3]),
        ],
        uv: [
            f32::midpoint(a.uv[0], b.uv[0]),
            f32::midpoint(a.uv[1], b.uv[1]),
        ],
    }
}

// The mid-channel cannot exceed 255, so the narrowing is exact.
#[allow(clippy::cast_possible_truncation)]
fn blend_channel(a: u8, b: u8) -> u8 {
    u16::midpoint(u16::from(a), u16::from(b)) as u8
}
