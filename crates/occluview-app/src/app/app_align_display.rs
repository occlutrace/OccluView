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
    pub(super) fn apply_deviation_colors(&mut self, colors: Vec<[u8; 4]>) {
        let Some(layer) = self.align_mapped_layer() else {
            return;
        };
        if self.attach_overlay_colors(layer, colors, AlignOverlay::Map) {
            self.ghost_other_layer();
        }
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
        let Some(live) = self.live_scene_mut() else {
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
        entry.set_deviation(Some(Arc::clone(&shared)));
        self.set_overlay_colors(layer, shared);
        self.align_overlay = kind;
        // Change only the material data; preserve the prepared scene.
        self.mark_scene_materials_changed();
        self.deviation_push_pending = true;
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
        let (Some(scene), Some(live_viewport)) = (self.scene.clone(), self.live_viewport.clone())
        else {
            return false;
        };
        let Some(entry) = layer_of(&scene, layer) else {
            return false;
        };
        let count = entry.mesh.vertices().len();
        // The stored colours and scratch buffer must match this mesh.
        let Some(slot) = self
            .align_overlay_colors
            .iter_mut()
            .find(|(id, colors)| *id == layer && colors.len() == count)
        else {
            return false;
        };
        if !self.align_painted.holds(&entry.mesh, count) {
            return false;
        }

        // Require one replacement colour per touched vertex.
        if touched.len() != patched.len() {
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
            .align_painted
            .patch(&entry.mesh, &shared, touched)
            .map(<[_]>::to_vec);
        drop(scene);

        // Retain the full array so a later GPU rebuild can restore it.
        if let Some(live) = self.live_scene_mut() {
            if let Some(entry) = live
                .meshes_mut()
                .iter_mut()
                .find(|entry| entry.id() == layer)
            {
                entry.set_deviation(Some(Arc::clone(&shared)));
            }
        }

        if let Some(painted) = painted {
            let indices: Vec<usize> = touched.iter().map(|index| *index as usize).collect();
            if let Ok(viewport) = live_viewport.lock() {
                viewport.write_scene_vertices_sparse(&topology, &painted, &indices);
            }
        }
        self.needs_render = true;
        true
    }

    /// Remember one layer's colours, replacing whatever it had.
    fn set_overlay_colors(&mut self, layer: SceneMeshId, colors: Arc<Vec<[u8; 4]>>) {
        match self
            .align_overlay_colors
            .iter_mut()
            .find(|(id, _)| *id == layer)
        {
            Some(slot) => slot.1 = colors,
            None => self.align_overlay_colors.push((layer, colors)),
        }
    }

    /// Replace every overlaid layer's uploaded vertex colours. The CPU meshes
    /// are never touched, so a scan keeps its own colours and an export is
    /// unaffected by what happens to be on screen.
    ///
    /// Returns whether every pending layer reached the GPU. They do not when
    /// there is no prepared scene to write into yet; the caller re-pushes after
    /// the viewport has built one.
    pub(super) fn push_deviation_colors(&mut self) -> bool {
        let (Some(scene), Some(live_viewport)) = (self.scene.clone(), self.live_viewport.clone())
        else {
            return false;
        };
        if self.align_overlay_colors.is_empty() {
            return false;
        }
        let pending = self.align_overlay_colors.clone();
        let mut wrote = true;
        for (layer, colors) in pending {
            let Some(entry) = layer_of(&scene, layer) else {
                continue;
            };
            let topology = PreparedSceneTopology::from_mesh(&entry.mesh);
            // Reuse the existing buffer; only vertex colours changed.
            let Some(painted) = self.align_painted.repaint(&entry.mesh, &colors) else {
                wrote = false;
                continue;
            };
            let Ok(viewport) = live_viewport.lock() else {
                return false;
            };
            wrote &= viewport.write_scene_vertices(&topology, painted);
        }
        self.needs_render = true;
        wrote
    }

    /// Drop every overlay and restore the meshes' own colours.
    pub(super) fn clear_deviation_overlay(&mut self) {
        self.align_overlay = AlignOverlay::Nothing;
        // Restore the other layer even when no colour array remains.
        self.unghost_layers();
        if self.align_overlay_colors.is_empty() {
            return;
        }
        self.align_overlay_colors.clear();
        // A push still standing would chase colours that no longer exist.
        self.deviation_push_pending = false;
        self.align_painted.clear();
        let Some(live) = self.live_scene_mut() else {
            return;
        };
        let overlaid: Vec<SceneMeshId> = live
            .meshes_mut()
            .iter_mut()
            .filter(|entry| entry.deviation_colors().is_some())
            .map(|entry| {
                entry.set_deviation(None);
                entry.id()
            })
            .collect();
        self.mark_scene_materials_changed();
        self.restore_layer_colors(&overlaid);
        self.align_stats = None;
        self.needs_render = true;
    }

    /// Whether anything is currently overlaid.
    pub(super) fn align_overlay_is_up(&self) -> bool {
        !self.align_overlay_colors.is_empty()
    }

    /// Put the meshes' own vertex colours back on the GPU, for the layers that
    /// were carrying an overlay. Only those: re-uploading the whole scene to
    /// undo a change to one layer moves tens of megabytes for nothing.
    fn restore_layer_colors(&mut self, layers: &[SceneMeshId]) {
        if layers.is_empty() {
            return;
        }
        let (Some(scene), Some(live_viewport)) = (self.scene.clone(), self.live_viewport.clone())
        else {
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
        self.align.moving_layer()
    }

    /// The fixed layer, which is ghosted while the map is shown.
    fn align_other_layer(&self) -> Option<SceneMeshId> {
        self.align.fixed_layer()
    }

    /// Fade the other scan while the map is up.
    ///
    /// Two solid surfaces a fraction of a millimetre apart interpenetrate, and
    /// the coloured one is then only visible in patches. Lab software shows one
    /// clean coloured surface; this is how.
    pub(super) fn ghost_other_layer(&mut self) {
        if !self.align_ghosted.is_empty() {
            return;
        }
        let Some(other) = self.align_other_layer() else {
            return;
        };
        // Opacity is a material change; preserve the scene structure.
        let Some(live) = self.live_scene_mut() else {
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
        self.align_ghosted = remembered;
        self.mark_scene_materials_changed();
    }

    /// Bring the faded scan back.
    pub(super) fn unghost_layers(&mut self) {
        if self.align_ghosted.is_empty() {
            return;
        }
        let restore = std::mem::take(&mut self.align_ghosted);
        let Some(live) = self.live_scene_mut() else {
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

    /// Source before the test module.
    fn production() -> &'static str {
        let source =
            crate::primary_ui_tests::production_source(include_str!("app_align_display.rs"));
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    /// An overlay must not mutate CPU mesh colours or exports.
    #[test]
    fn an_overlay_never_touches_the_cpu_mesh() {
        let source = production();
        assert!(
            !source.contains("mesh.vertices_mut"),
            "an overlay must not be written into mesh data"
        );
        assert!(
            source.contains("write_scene_vertices(&topology, painted)"),
            "an overlay reaches the GPU through the vertex upload path"
        );
    }

    /// Re-colouring must not replace the scene.
    #[test]
    fn showing_and_hiding_an_overlay_never_replaces_the_scene() {
        let source = production();
        assert!(
            !source.contains("self.set_scene("),
            "an overlay must go through the material path, not set_scene"
        );
        assert!(
            source.contains("self.live_scene_mut()"),
            "the live scene is mutated in place, not cloned per re-colour"
        );
        assert!(
            !source.contains("Arc::make_mut(scene)"),
            "in-place scene edits go through live_scene_mut, which asserts the \
             handle is sole; bypassing it reintroduces a silent full copy"
        );
    }

    /// Re-colouring reuses the upload buffer.
    #[test]
    fn the_upload_buffer_is_repainted_not_rebuilt() {
        let source = production();
        assert!(
            source.contains("self.align_painted.repaint("),
            "the vertex upload must reuse its buffer"
        );
        assert!(
            source.contains("self.align_painted.clear()"),
            "dropping the overlay must drop the scratch buffer with it"
        );
    }

    /// The brush's hot path. A dab that went through the full attach repainted
    /// and re-uploaded a whole arch for a few hundred vertices, which is the
    /// three frames a second the operator reported.
    #[test]
    fn a_dab_uploads_only_the_vertices_it_touched() {
        let patch = production()
            .split_once("fn patch_overlay_colors(")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split_once("\n    /// Remember one layer"))
            .map(|(body, _)| body)
            .expect("a sparse patch path");
        assert!(patch.contains(".align_painted") && patch.contains(".patch("));
        assert!(patch.contains("write_scene_vertices_sparse("));
        // Same rule as above: release the cloned handle before the in-place
        // edit.
        assert!(
            crate::primary_ui_tests::appears_before(patch, "drop(scene);", "self.live_scene_mut()",),
            "the cloned scene handle must be dropped before the in-place edit"
        );
        assert!(
            !patch.contains("self.align_painted.repaint("),
            "the sparse path must not fall back to a full repaint silently"
        );
    }

    /// Two things want the one colour channel. Attaching either without
    /// recording which it was is what let a stale-map drop wipe the brush's own
    /// preview out from under the operator's hand.
    #[test]
    fn every_attached_overlay_says_what_it_is() {
        let source = production();
        let attach = source
            .split_once("fn attach_overlay_colors(")
            .map(|(_, rest)| rest)
            .expect("one place that attaches colours");
        assert!(
            attach.contains("self.align_overlay = kind"),
            "attaching colours must record what they mean"
        );
        let clear = source
            .split_once("fn clear_deviation_overlay(")
            .map(|(_, rest)| rest)
            .expect("one place that drops colours");
        assert!(
            clear.contains("self.align_overlay = AlignOverlay::Nothing"),
            "dropping colours must clear what they meant"
        );
    }
}
