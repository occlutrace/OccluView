//! Attach and upload per-vertex alignment colours without replacing scene
//! geometry. Both layers may carry an overlay.

use std::sync::Arc;

use occluview_core::SceneMeshId;
use occluview_render::PreparedSceneTopology;

use super::app_align::layer_of;

/// How solid the un-mapped scan stays while the heatmap is up. Enough to keep the
/// shape readable, faint enough that it never covers the coloured surface.
const GHOST_OPACITY: f32 = 0.16;

use super::OccluViewApp;

/// Meaning of the current per-vertex colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AlignOverlay {
    /// The meshes show their own colours.
    #[default]
    Nothing,
    /// Measured distances to the other mesh.
    Map,
    /// Which surface takes part in the match.
    Region,
}

impl OccluViewApp {
    /// Attach the measured colours to the mapped layer.
    ///
    /// The upload is left to the viewport sync, which runs once per frame and
    /// knows whether the prepared scene it would write into still exists.
    pub(super) fn apply_deviation_colors(&mut self, colors: Vec<[u8; 4]>) -> bool {
        let Some(layer) = self.align_mapped_layer() else {
            return false;
        };
        if !self.attach_overlay_colors(layer, colors, AlignOverlay::Map) {
            return false;
        }
        self.ghost_other_layer();
        true
    }

    /// Put per-vertex colours on one layer and record what they mean.
    ///
    /// Returns whether they were attached.
    pub(super) fn attach_overlay_colors(
        &mut self,
        layer: SceneMeshId,
        colors: Vec<[u8; 4]>,
        kind: AlignOverlay,
    ) -> bool {
        let shared = Arc::new(colors);
        let overlay_kind = match kind {
            // A measured map states the reading, so the marker shape never
            // touches it; a brush marking is paint over the scan.
            AlignOverlay::Map => occluview_core::OverlayKind::Measured,
            _ => occluview_core::OverlayKind::Paint,
        };
        let Some(live) = self.document.live_scene_mut() else {
            return false;
        };
        let Some(entry) = live
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == layer)
        else {
            return false;
        };
        // A colour array must match the layer's vertex count.
        if entry.mesh.vertices().len() != shared.len() {
            return false;
        }
        entry.set_overlay(overlay_kind, Some(Arc::clone(&shared)));
        self.set_overlay_colors(layer, shared);
        self.tools.align.overlay = kind;
        // Change only the material data; preserve the prepared scene.
        self.mark_scene_materials_changed();
        self.tools.align.deviation_push_pending = true;
        true
    }

    /// Rewrite only the vertices the last dab touched, and upload only those.
    ///
    /// Update only touched vertices and upload the sparse change.
    pub(super) fn patch_overlay_colors(
        &mut self,
        layer: SceneMeshId,
        touched: &[u32],
        patched: &[[u8; 4]],
    ) -> bool {
        let (Some(scene), Some(live_viewport)) = (
            self.document.scene.clone(),
            self.render.live_viewport.clone(),
        ) else {
            return false;
        };
        let Some(entry) = layer_of(&scene, layer) else {
            return false;
        };
        let count = entry.mesh.vertices().len();
        // The stored colours and scratch buffer must match this mesh.
        let Some(slot) = self
            .tools
            .align
            .overlay_colors
            .iter_mut()
            .find(|(id, colors)| *id == layer && colors.len() == count)
        else {
            return false;
        };
        if !self.tools.align.painted.holds(&entry.mesh, count) {
            return false;
        }

        // Require one replacement colour per touched vertex.
        if touched.len() != patched.len() {
            return false;
        }
        // The renderer coalesces sorted contiguous runs. Reject malformed
        // input before mutating the scratch buffer, so a partial preview can
        // never be mistaken for a successful brush stroke.
        if !touched.windows(2).all(|pair| pair[0] < pair[1])
            || touched
                .iter()
                .any(|index| usize::try_from(*index).map_or(true, |at| at >= count))
        {
            return false;
        }
        let colors = Arc::make_mut(&mut slot.1);
        for (index, colour) in touched.iter().zip(patched) {
            if let Some(entry) = colors.get_mut(*index as usize) {
                *entry = *colour;
            }
        }
        let shared = Arc::clone(&slot.1);

        // Finish reads through the cloned scene before editing the live scene.
        let topology = PreparedSceneTopology::from_mesh(&entry.mesh);
        let painted = self
            .tools
            .align
            .painted
            .patch(&entry.mesh, &shared, touched)
            .map(<[_]>::to_vec);
        drop(scene);

        // Retain the full array so a later GPU rebuild can restore it.
        if let Some(live) = self.document.live_scene_mut() {
            if let Some(entry) = live
                .meshes_mut()
                .iter_mut()
                .find(|entry| entry.id() == layer)
            {
                let kind = entry
                    .overlay_kind()
                    .unwrap_or(occluview_core::OverlayKind::Paint);
                entry.set_overlay(kind, Some(Arc::clone(&shared)));
            }
        }

        let Some(painted) = painted else {
            return false;
        };
        let indices: Vec<usize> = touched.iter().map(|index| *index as usize).collect();
        let Ok(viewport) = live_viewport.lock() else {
            return false;
        };
        if !viewport.write_scene_vertices_sparse(&topology, &painted, &indices) {
            return false;
        }
        self.render.invalidation.overlay_tools_changed();
        true
    }

    /// Remember one layer's colours, replacing whatever it had.
    fn set_overlay_colors(&mut self, layer: SceneMeshId, colors: Arc<Vec<[u8; 4]>>) {
        match self
            .tools
            .align
            .overlay_colors
            .iter_mut()
            .find(|(id, _)| *id == layer)
        {
            Some(slot) => slot.1 = colors,
            None => self.tools.align.overlay_colors.push((layer, colors)),
        }
    }

    /// Take one layer's overlay off and put its own vertex colours back.
    ///
    /// Used when the last mark on that side is cleared: an overlay that paints
    /// nothing is a full-array upload for a picture identical to the scan.
    pub(super) fn detach_region_preview(&mut self, layer: SceneMeshId) {
        self.tools
            .align
            .overlay_colors
            .retain(|(id, _)| *id != layer);
        let Some(live) = self.document.live_scene_mut() else {
            return;
        };
        let mut cleared = false;
        for entry in live.meshes_mut() {
            if entry.id() == layer && entry.overlay_colors().is_some() {
                entry.clear_overlay();
                cleared = true;
            }
        }
        if !cleared {
            return;
        }
        self.mark_scene_materials_changed();
        self.tools.align.deviation_push_pending = true;
        self.restore_layer_colors(&[layer]);
        self.render.invalidation.overlay_tools_changed();
    }

    /// Replace every overlaid layer's uploaded vertex colours. The CPU meshes
    /// are never touched, so a scan keeps its own colours and an export is
    /// unaffected by what happens to be on screen.
    ///
    /// Returns whether every pending layer reached the GPU. They do not when
    /// there is no prepared scene to write into yet; the caller re-pushes after
    /// the viewport has built one.
    pub(super) fn push_deviation_colors(&mut self) -> bool {
        let (Some(scene), Some(live_viewport)) = (
            self.document.scene.clone(),
            self.render.live_viewport.clone(),
        ) else {
            return false;
        };
        if self.tools.align.overlay_colors.is_empty() {
            // clear_deviation_overlay already restored the live viewport. A
            // queued no-op lets its caller retire the pending flag cleanly.
            return true;
        }
        let pending = self.tools.align.overlay_colors.clone();
        let mut wrote = true;
        for (layer, colors) in pending {
            let Some(entry) = layer_of(&scene, layer) else {
                wrote = false;
                continue;
            };
            let topology = PreparedSceneTopology::from_mesh(&entry.mesh);
            // Reuse the existing buffer; only vertex colours changed.
            let Some(painted) = self.tools.align.painted.repaint(&entry.mesh, &colors) else {
                wrote = false;
                continue;
            };
            let Ok(viewport) = live_viewport.lock() else {
                return false;
            };
            wrote &= viewport.write_scene_vertices(&topology, painted);
        }
        self.render.invalidation.overlay_tools_changed();
        wrote
    }

    /// Drop every overlay and restore the meshes' own colours.
    pub(super) fn clear_deviation_overlay(&mut self) {
        self.tools.align.overlay = AlignOverlay::Nothing;
        // Restore the other layer even when no colour array remains.
        self.unghost_layers();
        self.tools.align.stats = None;
        let had_overlay = !self.tools.align.overlay_colors.is_empty();
        self.tools.align.overlay_colors.clear();
        // The live path restores immediately; the offscreen path needs one
        // queued write because its prepared scene is intentionally retained.
        self.tools.align.painted.clear();
        let Some(live) = self.document.live_scene_mut() else {
            self.tools.align.deviation_push_pending = had_overlay;
            return;
        };
        let overlaid: Vec<SceneMeshId> = live
            .meshes_mut()
            .iter_mut()
            .filter(|entry| entry.overlay_colors().is_some())
            .map(|entry| {
                entry.clear_overlay();
                entry.id()
            })
            .collect();
        self.tools.align.deviation_push_pending = had_overlay || !overlaid.is_empty();
        if overlaid.is_empty() {
            return;
        }
        self.mark_scene_materials_changed();
        self.restore_layer_colors(&overlaid);
        self.render.invalidation.overlay_tools_changed();
    }

    /// Whether anything is currently overlaid.
    pub(super) fn align_overlay_is_up(&self) -> bool {
        !self.tools.align.overlay_colors.is_empty()
    }

    /// Put the meshes' own vertex colours back on the GPU, for the layers that
    /// were carrying an overlay. Only those: re-uploading the whole scene to
    /// undo a change to one layer moves tens of megabytes for nothing.
    fn restore_layer_colors(&mut self, layers: &[SceneMeshId]) {
        if layers.is_empty() {
            return;
        }
        let (Some(scene), Some(live_viewport)) = (
            self.document.scene.clone(),
            self.render.live_viewport.clone(),
        ) else {
            return;
        };
        let Ok(viewport) = live_viewport.lock() else {
            return;
        };
        for entry in scene
            .meshes()
            .iter()
            .filter(|entry| layers.contains(&entry.id()))
        {
            let topology = PreparedSceneTopology::from_mesh(&entry.mesh);
            let _ = viewport.write_scene_vertices(&topology, entry.mesh.vertices());
        }
    }

    // Display helpers.
    /// The moving layer carries the map.
    pub(super) fn align_mapped_layer(&self) -> Option<SceneMeshId> {
        self.tools.align.tool.moving_layer()
    }

    /// The fixed layer, which is ghosted while the map is shown.
    fn align_other_layer(&self) -> Option<SceneMeshId> {
        self.tools.align.tool.fixed_layer()
    }

    /// Fade the other scan while the map is up.
    ///
    /// Two solid surfaces a fraction of a millimetre apart interpenetrate, and
    /// the coloured one is then only visible in patches. Lab software shows one
    /// clean coloured surface; this is how.
    pub(super) fn ghost_other_layer(&mut self) {
        if !self.tools.align.ghosted.is_empty() {
            return;
        }
        let Some(other) = self.align_other_layer() else {
            return;
        };
        // Opacity is a material change; preserve the scene structure.
        let Some(live) = self.document.live_scene_mut() else {
            return;
        };
        let mut remembered = Vec::new();
        for entry in live.meshes_mut() {
            if entry.id() == other {
                remembered.push((entry.id(), entry.opacity));
                entry.opacity = GHOST_OPACITY;
            }
        }
        if remembered.is_empty() {
            return;
        }
        self.tools.align.ghosted = remembered;
        self.mark_scene_materials_changed();
    }

    /// Bring the faded scan back.
    pub(super) fn unghost_layers(&mut self) {
        if self.tools.align.ghosted.is_empty() {
            return;
        }
        let restore = std::mem::take(&mut self.tools.align.ghosted);
        let Some(live) = self.document.live_scene_mut() else {
            return;
        };
        for (id, opacity) in restore {
            if let Some(entry) = live.meshes_mut().iter_mut().find(|entry| entry.id() == id) {
                entry.opacity = opacity;
            }
        }
        self.mark_scene_materials_changed();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]
}
