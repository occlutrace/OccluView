//! Getting per-vertex colours onto the screen, and off it again.
//!
//! Split from `app_align` because it answers a different question. That module
//! routes clicks and jobs; this one owns the display overlay — what is attached
//! to the scene, what is uploaded, and what is put back when it goes away.
//!
//! Every path here is on the operator's hand. An overlay is a colour change:
//! the mesh does not move, its topology does not change, and nothing about the
//! scan is edited. So none of it goes through `set_scene`, which treats the
//! scene as replaced — dropping the prepared GPU scene, clearing measurements,
//! and copying every mesh in the process. On a 945k-vertex arch that copy alone
//! was seventy milliseconds, and the rebuild that followed it sixty more, for a
//! change of four bytes per vertex.
//!
//! Two layers can carry an overlay at once, because both meshes in an alignment
//! can carry markings.

use std::sync::Arc;

use occluview_core::SceneMeshId;
use occluview_render::PreparedSceneTopology;

use super::app_align::layer_of;

/// How solid the un-mapped scan stays while the heatmap is up. Enough to keep the
/// shape readable, faint enough that it never covers the coloured surface.
const GHOST_OPACITY: f32 = 0.16;

use super::OccluViewApp;

/// What the per-vertex colours currently mean.
///
/// One colour channel, two things that want it: the measured map and the
/// markings. Without a name for which one is up, dropping a stale map also
/// wiped the brush's own preview mid-stroke.
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
        let Some(scene) = self.scene.as_mut() else {
            return false;
        };
        // In place: the app holds the only reference, so nothing is copied.
        let live = Arc::make_mut(scene);
        let Some(entry) = live
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == layer)
        else {
            return false;
        };
        // Colours of the wrong length are not this layer's. Stretching them
        // over whatever vertices are there would be colour presented as
        // measurement.
        if entry.mesh.vertices().len() != shared.len() {
            return false;
        }
        entry.set_deviation(Some(Arc::clone(&shared)));
        self.set_overlay_colors(layer, shared);
        self.align_overlay = kind;
        // A material change, not a scene replacement: the prepared GPU scene
        // survives and only the per-layer uniform is rewritten, which is what
        // turns the layer into a measured map in the shader.
        self.mark_scene_materials_changed();
        self.deviation_push_pending = true;
        true
    }

    /// Rewrite only the vertices the last dab touched, and upload only those.
    ///
    /// The hot path of the brush. A dab the size of a cusp touches a few
    /// hundred vertices out of a million; going through the full attach above
    /// would repaint and re-upload all of them, thirty-four megabytes each way,
    /// which is what made painting run at three frames a second.
    /// `recolor` is asked only for the vertices that changed, so a dab never
    /// walks a million-entry array to rewrite six hundred of them.
    pub(super) fn patch_overlay_colors(
        &mut self,
        layer: SceneMeshId,
        recolor: impl Fn(usize) -> [u8; 4],
    ) -> bool {
        let (Some(scene), Some(live_viewport)) = (self.scene.clone(), self.live_viewport.clone())
        else {
            return false;
        };
        let Some(entry) = layer_of(&scene, layer) else {
            return false;
        };
        let count = entry.mesh.vertices().len();
        // Both the stored colours and the scratch buffer have to already hold
        // this mesh, or there is nothing to patch INTO and the caller has to
        // take the full path instead.
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

        // In place through `Arc::make_mut`: the app holds the only reference
        // outside the scene's own, and the scene's is replaced below.
        //
        // The list belongs to the markings, which produced it. Borrowed rather
        // than stolen: a `mem::take` here left the markings holding an empty
        // list for the rest of the frame, so anything else that asked what the
        // last dab touched was told "nothing".
        let touched = self.align_markings.touched().to_vec();
        let colors = Arc::make_mut(&mut slot.1);
        for index in &touched {
            let at = *index as usize;
            if let Some(entry) = colors.get_mut(at) {
                *entry = recolor(at);
            }
        }
        let shared = Arc::clone(&slot.1);

        // The scene keeps the whole array so a GPU rebuild can restore it.
        if let Some(live) = self.scene.as_mut() {
            let live = Arc::make_mut(live);
            if let Some(entry) = live
                .meshes_mut()
                .iter_mut()
                .find(|entry| entry.id() == layer)
            {
                entry.set_deviation(Some(Arc::clone(&shared)));
            }
        }

        let topology = PreparedSceneTopology::from_mesh(&entry.mesh);
        if let Some(painted) = self.align_painted.patch(&entry.mesh, &shared, &touched) {
            let indices: Vec<usize> = touched.iter().map(|index| *index as usize).collect();
            if let Ok(viewport) = live_viewport.lock() {
                viewport.write_scene_vertices_sparse(&topology, painted, &indices);
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
            // Repainted into a buffer that is already the right shape, rather
            // than built afresh: only the colour of each vertex changed.
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
        if self.align_overlay_colors.is_empty() {
            return;
        }
        self.align_overlay_colors.clear();
        // A push still standing would chase colours that no longer exist.
        self.deviation_push_pending = false;
        self.align_painted.clear();
        let Some(scene) = self.scene.as_mut() else {
            return;
        };
        let live = Arc::make_mut(scene);
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
        self.unghost_layers();
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

    // The four items below moved here from the session module: every caller is
    // in this one, because they are display and not session.
    /// Which layer carries the map: the one that moved.
    ///
    /// There used to be a control that put the map on the other surface
    /// instead. It asked the operator a rendering question dressed up as a
    /// measurement one — the distances are the same either way — so it is gone,
    /// and the answer is now always "the scan you are placing".
    pub(super) fn align_mapped_layer(&self) -> Option<SceneMeshId> {
        self.align.moving_layer()
    }

    /// The layer the map is *not* on, which is the one that has to get out of
    /// the way.
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
        let Some(scene) = self.scene.as_mut() else {
            return;
        };
        // Opacity is a material, not a structure: mutate the live scene in
        // place rather than replacing it, or fading one layer would copy every
        // mesh in the scene and force a full GPU rebuild.
        let live = Arc::make_mut(scene);
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
        let Some(scene) = self.scene.as_mut() else {
            return;
        };
        let live = Arc::make_mut(scene);
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

    /// The production half of this file: a source-contract test that scanned
    /// its own assertions would pass or fail on its own text.
    fn production() -> &'static str {
        let source = include_str!("app_align_display.rs");
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    /// An overlay is a display layer. Writing it into the mesh would corrupt
    /// the scan's own colours and leak into every export.
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

    /// A colour change is not a scene replacement. `set_scene` copies every
    /// mesh, drops the prepared GPU scene, and clears the measure tool — none
    /// of which a re-colour has any business doing, and all of which the
    /// operator pays for on every dab.
    #[test]
    fn showing_and_hiding_an_overlay_never_replaces_the_scene() {
        let source = production();
        assert!(
            !source.contains("self.set_scene("),
            "an overlay must go through the material path, not set_scene"
        );
        assert!(
            source.contains("Arc::make_mut(scene)"),
            "the live scene is mutated in place, not cloned per re-colour"
        );
    }

    /// The upload buffer is kept between re-colours. Rebuilding it allocated
    /// and copied thirty-four megabytes every time anything changed.
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
        assert!(patch.contains("self.align_painted.patch("));
        assert!(patch.contains("write_scene_vertices_sparse("));
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
