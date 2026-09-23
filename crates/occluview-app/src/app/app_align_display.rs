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
    #![allow(clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use crate::app::app_test_support::{named_scene, push_named_layer, test_app};
    use occluview_core::OverlayKind;

    /// An app holding a named pair of scans, so the display helpers that speak
    /// about "the other layer" have one.
    fn app_with_a_pair(name: &str) -> (OccluViewApp, SceneMeshId, SceneMeshId) {
        let mut app = test_app(name);
        let mut scene = named_scene("lower", 0.0);
        let fixed = scene.meshes()[0].id();
        let moving = push_named_layer(&mut scene, "upper", 5.0);
        app.document.scene = Some(Arc::new(scene));
        app.tools.align.tool.arm();
        app.tools.align.tool.imply_pair(&[moving, fixed]);
        (app, moving, fixed)
    }

    /// One layer's entry in the document's scene.
    fn layer_entry(app: &OccluViewApp, layer: SceneMeshId) -> &occluview_core::SceneMesh {
        app.document
            .scene
            .as_ref()
            .expect("a scene")
            .meshes()
            .iter()
            .find(|entry| entry.id() == layer)
            .expect("the layer")
    }

    /// A measurement is a picture of a scan, not a change to it.
    ///
    /// The colours live on the entry's overlay channel; the mesh's own vertex
    /// colours are what an export writes and what the scan looks like with the
    /// map off. An overlay written into the mesh would be saved into the
    /// operator's file, and a measurement is not a mesh edit.
    #[test]
    fn an_overlay_never_touches_the_cpu_mesh() {
        let (mut app, moving, _fixed) = app_with_a_pair("align-overlay-cpu-mesh");
        let measured = [200u8, 40, 40, 255];

        assert!(
            app.attach_overlay_colors(moving, vec![measured; 3], AlignOverlay::Map),
            "the map attaches to a layer whose vertex count it matches"
        );

        assert_eq!(
            layer_entry(&app, moving)
                .overlay_colors()
                .map(|colors| colors.as_slice()),
            Some(&[measured; 3][..]),
            "the measurement belongs to the overlay channel"
        );
        assert_eq!(
            layer_entry(&app, moving).overlay_kind(),
            Some(OverlayKind::Measured),
            "and the channel says the colours are a reading, not paint"
        );
        assert!(
            layer_entry(&app, moving)
                .mesh
                .vertices()
                .iter()
                .all(|vertex| vertex.color == [255, 255, 255, 255]),
            "the scan's own vertex colours must be untouched: they are what an \
             export writes and what the scan shows with the map off"
        );

        // The upload and the teardown must leave the mesh alone as well.
        let _ = app.push_deviation_colors();
        app.clear_deviation_overlay();
        assert!(layer_entry(&app, moving).overlay_colors().is_none());
        assert!(
            layer_entry(&app, moving)
                .mesh
                .vertices()
                .iter()
                .all(|vertex| vertex.color == [255, 255, 255, 255]),
            "showing and hiding a map must not leave the measurement in the mesh"
        );
    }

    /// Showing and hiding a map re-colours the live scene in place.
    ///
    /// A re-colour that installed a new scene would leave every other holder of
    /// the old handle reading a scan that never took the measurement — and would
    /// copy the whole scene container, per layer, to change four bytes per
    /// vertex. The scene handle and the layers in it have to survive.
    #[test]
    fn showing_and_hiding_an_overlay_never_replaces_the_scene() {
        let (mut app, moving, _fixed) = app_with_a_pair("align-overlay-scene-identity");
        let scene_before = Arc::as_ptr(app.document.scene.as_ref().expect("a scene"));
        let ids_before: Vec<SceneMeshId> = app
            .document
            .scene
            .as_ref()
            .expect("a scene")
            .meshes()
            .iter()
            .map(occluview_core::SceneMesh::id)
            .collect();

        assert!(app.attach_overlay_colors(moving, vec![[9, 9, 9, 255]; 3], AlignOverlay::Map));
        assert_eq!(
            Arc::as_ptr(app.document.scene.as_ref().expect("a scene")),
            scene_before,
            "re-colouring must edit the live scene, not install a new one"
        );
        let ids_after: Vec<SceneMeshId> = app
            .document
            .scene
            .as_ref()
            .expect("a scene")
            .meshes()
            .iter()
            .map(occluview_core::SceneMesh::id)
            .collect();
        assert_eq!(ids_after, ids_before, "and it must not rebuild the layers");

        app.clear_deviation_overlay();
        assert_eq!(
            Arc::as_ptr(app.document.scene.as_ref().expect("a scene")),
            scene_before,
            "taking the map down is an in-place edit too"
        );
        assert!(
            app.document
                .scene
                .as_ref()
                .expect("a scene")
                .meshes()
                .iter()
                .all(|entry| entry.overlay_colors().is_none()),
            "every layer's colours are back"
        );
    }

    /// An attached overlay records what its colours mean, and the teardown
    /// paths act on that record.
    ///
    /// A measured map and the brush's own marking preview are both per-vertex
    /// colours on the same layers. Without the record, a stale-map drop takes
    /// the brush's preview down from under the operator's hand, and the screen
    /// stops showing the marks the mask still holds.
    #[test]
    fn every_attached_overlay_says_what_it_is() {
        let (mut app, moving, _fixed) = app_with_a_pair("align-overlay-kind");

        assert!(app.attach_overlay_colors(moving, vec![[1, 2, 3, 255]; 3], AlignOverlay::Region));
        assert_eq!(app.tools.align.overlay, AlignOverlay::Region);
        assert_eq!(
            layer_entry(&app, moving).overlay_kind(),
            Some(OverlayKind::Paint),
            "markings are paint over the scan, not a reading"
        );

        // A map drawn for an older pose is stale; the markings are not.
        app.invalidate_deviation_map("the scan moved");
        assert_eq!(
            app.tools.align.overlay,
            AlignOverlay::Region,
            "a stale-map drop must not take the brush's preview down"
        );
        assert!(
            app.align_overlay_is_up(),
            "the marking colours are still on screen"
        );

        assert!(app.attach_overlay_colors(moving, vec![[4, 5, 6, 255]; 3], AlignOverlay::Map));
        app.invalidate_deviation_map("the scan moved");
        assert_eq!(
            app.tools.align.overlay,
            AlignOverlay::Nothing,
            "a map is exactly what the drop is for"
        );
        assert!(!app.align_overlay_is_up());
        assert!(layer_entry(&app, moving).overlay_colors().is_none());
    }

    /// The vertex upload buffer is repainted, not rebuilt.
    ///
    /// A re-colour changes four bytes per vertex; rebuilding the array allocates
    /// and copies a whole arch each time, which is most of what the map cost.
    /// The scratch has to go when the overlay does: left behind, the next dab
    /// takes the sparse path and re-attaches the stale array.
    #[test]
    fn the_upload_buffer_is_repainted_not_rebuilt() {
        let (mut app, moving, _fixed) = app_with_a_pair("align-upload-buffer");
        let mesh = layer_entry(&app, moving).mesh.clone();

        let first = app
            .tools
            .align
            .painted
            .repaint(&mesh, &[[1, 2, 3, 255]; 3])
            .expect("the array matches the mesh");
        let first_at = first.as_ptr();
        assert!(app.tools.align.painted.holds(&mesh, 3));

        let second = app
            .tools
            .align
            .painted
            .repaint(&mesh, &[[4, 5, 6, 255]; 3])
            .expect("the array matches the mesh");
        assert_eq!(
            second.as_ptr(),
            first_at,
            "a re-colour must overwrite the array, not allocate another"
        );
        assert!(second.iter().all(|vertex| vertex.color == [4, 5, 6, 255]));

        // The brush's sparse path rewrites what it touched and leaves the rest
        // of the array as the last re-colour left it. A patch that rebuilt the
        // array would blank the rest of the scan's painted region.
        let patched = app
            .tools
            .align
            .painted
            .patch(&mesh, &[[9, 9, 9, 255]; 3], &[1])
            .expect("a sorted touched list inside the mesh");
        assert_eq!(
            patched.as_ptr(),
            first_at,
            "the sparse path reuses the array"
        );
        assert_eq!(patched[1].color, [9, 9, 9, 255]);
        assert_eq!(
            patched[0].color,
            [4, 5, 6, 255],
            "a vertex the dab did not touch keeps the colour the last repaint left"
        );

        app.clear_deviation_overlay();
        assert!(
            !app.tools.align.painted.holds(&mesh, 3),
            "dropping the overlay must drop the scratch buffer with it"
        );
    }

    /// Dropping an overlay repairs everything that described it.
    ///
    /// The faded companion scan, the last measurement's numbers, and the queued
    /// GPU write all belong to the map being dropped. Left behind, the companion
    /// stays half transparent with no map to justify it, the panel shows numbers
    /// for a picture that is gone, and the offscreen path never re-uploads the
    /// layers' own colours.
    #[test]
    fn clearing_an_overlay_also_repairs_stale_display_bookkeeping() {
        let (mut app, moving, fixed) = app_with_a_pair("align-overlay-clear-bookkeeping");
        let own_opacity = layer_entry(&app, fixed).opacity;

        // Nothing was up, so nothing has to reach the GPU.
        app.clear_deviation_overlay();
        assert!(
            !app.tools.align.deviation_push_pending,
            "a clear with no overlay must not queue an upload"
        );

        assert!(app.attach_overlay_colors(moving, vec![[7, 7, 7, 255]; 3], AlignOverlay::Map));
        app.ghost_other_layer();
        assert!(
            !app.tools.align.ghosted.is_empty(),
            "the companion scan is faded while the map is up"
        );

        app.clear_deviation_overlay();

        assert!(app.tools.align.overlay_colors.is_empty());
        assert!(
            app.tools.align.ghosted.is_empty(),
            "the faded companion must come back with the map"
        );
        assert_eq!(
            layer_entry(&app, fixed).opacity,
            own_opacity,
            "and at its own opacity, not the ghost's"
        );
        assert!(
            app.tools.align.stats.is_none(),
            "the panel must not keep showing the dropped measurement's numbers"
        );
        assert!(
            app.tools.align.deviation_push_pending,
            "the layers' own colours still have to reach the GPU"
        );
        assert!(
            app.document
                .scene
                .as_ref()
                .expect("a scene")
                .meshes()
                .iter()
                .all(|entry| entry.overlay_colors().is_none()),
            "no layer is left carrying the dropped colours"
        );
    }

    /// A sparse overlay write that the renderer refuses is not reported as
    /// success. The brush reads `true` as "the dab reached the screen", so a
    /// rejected upload that returned success leaves the operator painting into
    /// a buffer that never changed.
    ///
    /// The rejection is exercised at the seam that decides it: with no live
    /// viewport there is nothing to write into, and the call must say so.
    #[test]
    fn a_rejected_sparse_overlay_upload_is_not_reported_as_success() {
        let (mut app, moving, _fixed) = app_with_a_pair("align-sparse-reject");
        assert!(
            app.attach_overlay_colors(moving, vec![[1, 2, 3, 255]; 3], AlignOverlay::Region),
            "the preview has to be up before a sparse write can be attempted"
        );
        assert!(
            app.render.live_viewport.is_none(),
            "the fixture has no GPU viewport, so the write must be refused"
        );

        let applied = app.patch_overlay_colors(moving, &[0, 1, 2], &[[9, 9, 9, 255]; 3]);

        assert!(
            !applied,
            "a sparse overlay write with nowhere to land must report failure"
        );
        assert!(
            app.tools.align.deviation_push_pending,
            "and the full repaint stays owed, so the colours are not silently lost"
        );
    }

    /// A malformed sparse write is refused before it mutates the scratch
    /// buffer. A partial preview accepted as a successful dab would leave the
    /// painted array disagreeing with the mask on a later rebuild.
    #[test]
    fn a_malformed_sparse_overlay_write_does_not_mutate_the_scratch() {
        let (mut app, moving, _fixed) = app_with_a_pair("align-sparse-malformed");

        // Establish the scratch through the full repaint path.
        let mesh = layer_entry(&app, moving).mesh.clone();
        app.tools
            .align
            .painted
            .repaint(&mesh, &[[1, 2, 3, 255]; 3])
            .expect("the array matches the mesh");
        assert!(app.attach_overlay_colors(moving, vec![[1, 2, 3, 255]; 3], AlignOverlay::Region));

        // An unsorted touched list is malformed: the renderer's sparse writer
        // coalesces ordered runs, so it must be refused, not reordered.
        let applied = app.patch_overlay_colors(moving, &[2, 0], &[[9, 9, 9, 255]; 2]);

        assert!(!applied, "an unsorted touched list must be refused");
        let scratch = app
            .tools
            .align
            .painted
            .repaint(&mesh, &[[1, 2, 3, 255]; 3])
            .expect("the scratch is still addressable");
        assert!(
            scratch.iter().all(|vertex| vertex.color == [1, 2, 3, 255]),
            "the rejected write must leave the scratch exactly as it was"
        );
    }
}
