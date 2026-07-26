//! Getting the measured colours onto the screen, and off it again.
//!
//! Split from `app_align` because it answers a different question. That module
//! routes clicks and jobs; this one owns the display overlay — what is attached
//! to the scene, what is uploaded, and what is put back when the map goes away.
//!
//! Every path here is on the operator's slider. A deviation map is a colour
//! change: the mesh does not move, its topology does not change, and nothing
//! about the scan is edited. So none of it goes through `set_scene`, which
//! treats the scene as replaced — dropping the prepared GPU scene, clearing
//! measurements, and copying every mesh in the process. On a 945k-vertex arch
//! that copy alone was seventy milliseconds, and the rebuild that followed it
//! sixty more, for a change of four bytes per vertex.

use std::sync::Arc;

use occluview_core::SceneMeshId;
use occluview_render::PreparedSceneTopology;

use super::app_align::layer_of;
use super::OccluViewApp;

/// What the per-vertex overlay on the moving scan currently means.
///
/// One layer, one colour channel, two things that want it: the measured map and
/// the region tint. Without a name for which one is up, dropping a stale map
/// also wiped the brush's own preview mid-stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AlignOverlay {
    /// The scan shows its own colours.
    #[default]
    Nothing,
    /// Measured distances to the other scan.
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
        if self.attach_overlay_colors(colors, AlignOverlay::Map) {
            self.ghost_other_layer();
        }
    }

    /// Attach the region tint to the moving layer.
    ///
    /// No ghosting: while an operator is painting they are aiming at both
    /// surfaces, and fading one of them away is the opposite of what a
    /// measurement reading needs.
    pub(super) fn apply_region_colors(&mut self, colors: Vec<[u8; 4]>) {
        self.attach_overlay_colors(colors, AlignOverlay::Region);
    }

    /// Put per-vertex colours on the moving layer and record what they mean.
    ///
    /// Returns whether they were attached.
    fn attach_overlay_colors(&mut self, colors: Vec<[u8; 4]>, kind: AlignOverlay) -> bool {
        let Some(moving_id) = self.align_mapped_layer() else {
            return false;
        };
        let shared = Arc::new(colors);
        let Some(scene) = self.scene.as_mut() else {
            return false;
        };
        // In place: the app holds the only reference, so nothing is copied.
        let live = Arc::make_mut(scene);
        let Some(entry) = live
            .meshes_mut()
            .iter_mut()
            .find(|entry| entry.id() == moving_id)
        else {
            return false;
        };
        // A map of the wrong length is not this layer's map. Stretching it over
        // whatever vertices are there would be colour presented as measurement.
        if entry.mesh.vertices().len() != shared.len() {
            return false;
        }
        entry.set_deviation(Some(Arc::clone(&shared)));
        self.align_deviation = Some(shared);
        self.align_overlay = kind;
        // A material change, not a scene replacement: the prepared GPU scene
        // survives and only the per-layer uniform is rewritten, which is what
        // turns the layer into a measured map in the shader.
        self.mark_scene_materials_changed();
        self.deviation_push_pending = true;
        true
    }

    /// Replace the mapped layer's uploaded vertex colours with the measured
    /// map. The CPU mesh is never touched, so the scan keeps its own colours
    /// and an export is unaffected by what happens to be on screen.
    ///
    /// Returns whether the colours reached the GPU. They do not when there is
    /// no prepared scene to write into yet; the caller re-pushes after the
    /// viewport has built one.
    pub(super) fn push_deviation_colors(&mut self) -> bool {
        let (Some(scene), Some(colors), Some(moving_id), Some(live_viewport)) = (
            self.scene.clone(),
            self.align_deviation.clone(),
            self.align_mapped_layer(),
            self.live_viewport.clone(),
        ) else {
            return false;
        };
        let Some(entry) = layer_of(&scene, moving_id) else {
            return false;
        };
        let topology = PreparedSceneTopology::from_mesh(&entry.mesh);
        // Repainted into a buffer that is already the right shape, rather than
        // built afresh: only the colour of each vertex changed.
        let Some(painted) = self.align_painted.repaint(&entry.mesh, &colors) else {
            return false;
        };
        let Ok(viewport) = live_viewport.lock() else {
            return false;
        };
        let wrote = viewport.write_scene_vertices(&topology, painted);
        drop(viewport);
        self.needs_render = true;
        wrote
    }

    /// Drop the map and restore the scan's own colours.
    pub(super) fn clear_deviation_overlay(&mut self) {
        self.align_overlay = AlignOverlay::Nothing;
        if self.align_deviation.take().is_none() {
            return;
        }
        // A push still standing would chase a map that no longer exists.
        self.deviation_push_pending = false;
        self.align_painted.clear();
        let Some(scene) = self.scene.as_mut() else {
            return;
        };
        let live = Arc::make_mut(scene);
        let mapped: Vec<SceneMeshId> = live
            .meshes_mut()
            .iter_mut()
            .filter(|entry| entry.deviation_colors().is_some())
            .map(|entry| {
                entry.set_deviation(None);
                entry.id()
            })
            .collect();
        self.mark_scene_materials_changed();
        self.restore_layer_colors(&mapped);
        self.align_stats = None;
        self.needs_render = true;
        self.unghost_layers();
    }

    /// Put the scan's own vertex colours back on the GPU, for the layers that
    /// were carrying a map. Only those: re-uploading the whole scene to undo a
    /// change to one layer moves tens of megabytes for nothing.
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

    /// The map is a display overlay. Writing it into the mesh would corrupt the
    /// scan's own colours and leak into every export.
    #[test]
    fn the_deviation_map_never_touches_the_cpu_mesh() {
        let source = production();
        assert!(
            !source.contains("mesh.vertices_mut"),
            "the map must not be written into mesh data"
        );
        assert!(
            source.contains("write_scene_vertices(&topology, painted)"),
            "the map reaches the GPU through the vertex upload path"
        );
    }

    /// A colour change is not a scene replacement. `set_scene` copies every
    /// mesh, drops the prepared GPU scene, and clears the measure tool — none
    /// of which a re-colour has any business doing, and all of which the
    /// operator pays for on every nudge of the scale slider.
    #[test]
    fn showing_and_hiding_the_map_never_replaces_the_scene() {
        let source = production();
        assert!(
            !source.contains("self.set_scene("),
            "the deviation overlay must go through the material path, not set_scene"
        );
        assert!(
            source.contains("Arc::make_mut(scene)"),
            "the live scene is mutated in place, not cloned per re-colour"
        );
        assert_eq!(
            source
                .matches("self.mark_scene_materials_changed()")
                .count(),
            2,
            "attaching and clearing the map are both material changes"
        );
    }

    /// The upload buffer is kept between re-colours. Rebuilding it allocated
    /// and copied thirty-four megabytes every time the slider moved.
    #[test]
    fn the_upload_buffer_is_repainted_not_rebuilt() {
        let source = production();
        assert!(
            source.contains("self.align_painted.repaint("),
            "the vertex upload must reuse its buffer"
        );
        assert!(
            source.contains("self.align_painted.clear()"),
            "dropping the map must drop the scratch buffer with it"
        );
    }

    /// Two things want the one colour channel on the moving scan. Attaching
    /// either without recording which it was is what let a stale-map drop wipe
    /// the brush's own preview out from under the operator's hand.
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
        assert_eq!(
            source.matches("entry.set_deviation(Some(").count(),
            1,
            "colours must only be attached in one place"
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
