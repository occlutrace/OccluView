//! Adapter between the app's sculpt contract and the `occlu-sculpt` kernel.
//!
//! Production input is a viewport ray step; the kernel owns raycast, path
//! continuity, dose, and remeshing. Point dabs remain only as test fixtures.
//!
//! The kernel carries positions and normals only, so this adapter owns the
//! attribute mirror: it keeps every vertex's colour and texture coordinate, and
//! blends a minted vertex's attributes from the two corners of the edge the
//! kernel split.

use crate::sculpt_tool::SculptTip;
use glam::DVec3;
#[cfg(test)]
use occlu_sculpt::Dab;
use occlu_sculpt::{BrushMode as KernelMode, SculptRayConstraints, SculptSession, TipStamp};
use occluview_core::{EditVertex, MeshEditBuffers, MeshEditError, MeshTopology, Vertex};
use occluview_render::{SculptFaceUpdate, SculptTopologyDelta, SculptVertexUpdate};
use std::sync::atomic::{AtomicBool, Ordering};

/// Which sculpting operation a dab performs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum BrushMode {
    /// Relax the surface: iron out grain and even the tessellation.
    Smooth,
    /// Add material along the sampled dental surface sheet.
    Add,
    /// Remove material from the sampled dental surface sheet.
    Remove,
    /// Even small surface detail while preserving the broad form.
    Relax,
}

/// One brush dab in the layer's mesh-local space.
#[cfg(test)]
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

/// One pointer sample in the active viewport, already transformed into the
/// sculpt layer's local millimetre space. The kernel owns the raycast and the
/// swept path between consecutive samples.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BrushRayStep {
    pub(crate) origin: [f32; 3],
    pub(crate) direction: [f32; 3],
    pub(crate) near_mm: f32,
    pub(crate) far_mm: f32,
    /// The renderer has one active section plane. Its local-space halfspace is
    /// represented as n·p + d >= 0, matching the kernel's visible-side test.
    pub(crate) clip_plane: Option<[f64; 4]>,
    pub(crate) radius_mm: f32,
    pub(crate) strength: f32,
    pub(crate) mode: BrushMode,
    pub(crate) tip: SculptTip,
    pub(crate) axis: Option<[f32; 3]>,
    /// Button remains down without travel; this controls path stamping, while
    /// the worker supplies elapsed time independently at dispatch.
    pub(crate) hold: bool,
    /// Ctrl preserves the opposite wall for every non-Relax mode.
    pub(crate) preserve_skirt: bool,
}

/// Test-only dose input for legacy point-dab fixtures.
#[cfg(test)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct DabDose {
    /// Whether this dab repeats the previous position while the button is
    /// held. A travelled dab is never a hold.
    pub(crate) hold: bool,
    /// Dwell this dab stands for, in milliseconds, when `hold`.
    pub(crate) elapsed_ms: f32,
}

#[cfg(test)]
impl DabDose {
    /// One full dose: a dab that travelled its own spacing, or the first dab of
    /// a stroke.
    pub(crate) const FULL: Self = Self {
        hold: false,
        elapsed_ms: 0.0,
    };

    /// `elapsed_ms` of stationary dwell at the current position.
    pub(crate) fn dwell(elapsed_ms: f32) -> Self {
        Self {
            hold: true,
            elapsed_ms,
        }
    }
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

struct KernelStepRows {
    touched: Vec<u32>,
    added: Vec<(u32, u32, u32)>,
    dirty: Vec<usize>,
    topology_changed: bool,
    base_vertex_count: usize,
    base_index_count: usize,
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
        let mut kernel = SculptSession::new(positions, mesh.indices.clone());
        // Erode uses the opposite-wall reserve. Build its immutable opening
        // probe on this already-background preparation path, never on a dab.
        kernel.prepare_wall_probe();
        Ok(Self {
            kernel,
            vertices: mesh.vertices.clone(),
            indices: mesh.indices.clone(),
            stroke_open: false,
        })
    }

    /// Apply one dab with `tip` that stands for `dose` of dwell, oriented along
    /// `axis` when the tip is the knife. See [`DabDose`] and the module docs for
    /// the contract.
    // One dab is named by its ray, dose, mode, tip and bearing together.
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) fn apply_stroke_dosed(
        &mut self,
        stroke: BrushStroke,
        mode: BrushMode,
        tip: SculptTip,
        axis: Option<[f32; 3]>,
        dose: DabDose,
    ) -> BrushStrokeOutcome {
        self.apply(stroke, mode, tip, axis, dose)
    }

    /// Break the swept pointer path while keeping the current stroke and its
    /// undo record open across a viewport-owned UI overlay.
    pub(crate) fn break_ray_path(&mut self) {
        self.kernel.break_stroke_path();
    }

    /// Warm the local opposite-wall reserve around an idle hover point.
    pub(crate) fn prime_wall_region(
        &mut self,
        center: DVec3,
        radius_mm: f64,
        budget: usize,
    ) -> usize {
        self.kernel.prime_wall_region(center, radius_mm, budget)
    }

    /// Cancellable worker form of [`Self::apply_ray_step`]. A canceled owner
    /// discards the entire session and its display mirror together.
    pub(crate) fn apply_ray_step_cancellable(
        &mut self,
        step: &BrushRayStep,
        elapsed_ms: f64,
        cancel: &AtomicBool,
    ) -> Option<BrushStrokeOutcome> {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let outcome = self.apply_ray(step, elapsed_ms);
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        Some(outcome)
    }

    /// Cancellable form of [`Self::apply_stroke_dosed`]. A cancellation never
    /// returns an outcome: the owning worker is being torn down, so its
    /// partially updated session and mirror must be discarded together.
    // The cancellation flag rides beside the dab's own arguments.
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) fn apply_stroke_cancellable_dosed(
        &mut self,
        stroke: BrushStroke,
        mode: BrushMode,
        tip: SculptTip,
        axis: Option<[f32; 3]>,
        dose: DabDose,
        cancel: &AtomicBool,
    ) -> Option<BrushStrokeOutcome> {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let outcome = self.apply(stroke, mode, tip, axis, dose);
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

    // One dab is named by its ray, dose, mode, tip and bearing together.
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    fn apply(
        &mut self,
        stroke: BrushStroke,
        mode: BrushMode,
        tip: SculptTip,
        axis: Option<[f32; 3]>,
        dose: DabDose,
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
        let elapsed_ms = if dose.hold {
            f64::from(dose.elapsed_ms)
        } else {
            occlu_sculpt::DWELL_FULL_DOSE_MS
        };
        self.kernel.set_dab_elapsed_ms(elapsed_ms);
        self.kernel.set_preserve_skirt(false);
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
        self.finish_kernel_step(KernelStepRows {
            touched,
            added,
            dirty,
            topology_changed: self.kernel.topology_revision() != base_revision,
            base_vertex_count,
            base_index_count,
        })
    }

    fn apply_ray(&mut self, step: &BrushRayStep, elapsed_ms: f64) -> BrushStrokeOutcome {
        if !self.stroke_open {
            self.kernel.start_stroke();
            self.stroke_open = true;
        }
        let base_vertex_count = self.vertices.len();
        let base_index_count = self.indices.len();
        let clips = step.clip_plane.iter().copied().collect::<Vec<_>>();
        self.kernel
            .set_brush_tip(TipStamp::from_u32(step.tip.kernel_stamp()).unwrap_or(TipStamp::Ball));
        self.kernel.set_dab_elapsed_ms(elapsed_ms);
        self.kernel.set_preserve_skirt(step.preserve_skirt);
        self.kernel
            .set_dab_axis(step.axis.map(|axis| {
                DVec3::new(f64::from(axis[0]), f64::from(axis[1]), f64::from(axis[2]))
            }));
        let origin = DVec3::new(
            f64::from(step.origin[0]),
            f64::from(step.origin[1]),
            f64::from(step.origin[2]),
        );
        let direction = DVec3::new(
            f64::from(step.direction[0]),
            f64::from(step.direction[1]),
            f64::from(step.direction[2]),
        );
        let constraints = SculptRayConstraints {
            near: f64::from(step.near_mm),
            far: f64::from(step.far_mm),
            clip_planes: &clips,
        };
        let result = self.kernel.dab_at_ray_visible(
            origin,
            direction,
            f64::from(step.radius_mm),
            f64::from(step.strength),
            kernel_mode(step.mode),
            step.hold,
            constraints,
        );
        // One ray step traces an entire swept path and calls `dab` exactly
        // once, so these per-dab parent and dirty-face rows cover the whole
        // returned movement/topology slice.
        // `stroke_step` returns early on a true ray miss, before clearing the
        // previous dab's scratch rows. Its live record is the per-call source
        // of truth for whether a dab ran; never mirror stale parent/face rows.
        let applied = result.live.words[1] > 0;
        let added = if applied {
            self.kernel.dab_added_parents().to_vec()
        } else {
            Vec::new()
        };
        let dirty = if applied {
            self.kernel
                .dab_dirty_triangles()
                .iter()
                .map(|&face| face as usize)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        self.finish_kernel_step(KernelStepRows {
            touched: result.moved,
            added,
            dirty,
            topology_changed: applied && !result.topo.is_empty(),
            base_vertex_count,
            base_index_count,
        })
    }

    fn finish_kernel_step(&mut self, rows: KernelStepRows) -> BrushStrokeOutcome {
        let KernelStepRows {
            touched,
            added,
            dirty,
            topology_changed,
            base_vertex_count,
            base_index_count,
        } = rows;
        self.sync_mirror(&touched, &added);
        let topology_delta = topology_changed.then(|| {
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
        let outcome = BrushStrokeOutcome {
            touched_vertices: touched.iter().map(|&vertex| vertex as usize).collect(),
            dirty_triangles: dirty,
            topology_delta,
        };
        outcome
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
        BrushMode::Relax => KernelMode::Relax,
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
