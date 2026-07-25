//! Painting the exclusion mask while Align Scans is armed.
//!
//! A dab is a plain pass over the moving layer's vertices with a distance
//! test — no acceleration structure, no worker. That is deliberate: it costs a
//! few milliseconds even on a dense scan, and routing it through the worker
//! would mean the paint lagged the cursor. The expensive half — re-measuring
//! the deviation map — still goes to the worker, and only once the stroke ends
//! rather than on every dab.

use std::sync::Arc;

use eframe::egui;
use glam::DVec3;
use occluview_align::{MaskEdit, Rigid};

use super::OccluViewApp;
use crate::align_brush::MaskCommand;
use crate::viewer::pick_scene_hit;

impl OccluViewApp {
    /// Paint or erase under the pointer. Returns whether the brush owns this
    /// frame's pointer.
    pub(super) fn handle_align_brush(
        &mut self,
        response: &egui::Response,
        ctx: &egui::Context,
    ) -> bool {
        if !self.align_brush.is_armed() {
            return false;
        }
        let primary_down =
            ctx.input(|input| input.pointer.button_down(egui::PointerButton::Primary));
        if !primary_down {
            // The stroke ended. The mask changed what would be measured, so a
            // map drawn before it is stale — drop it rather than silently
            // recomputing behind the operator's hand.
            if self.align_mask_stroke_open {
                self.align_mask_stroke_open = false;
                self.invalidate_deviation_map("Mask changed");
            }
            return false;
        }
        let Some(pointer) = response
            .interact_pointer_pos()
            .or_else(|| ctx.input(|input| input.pointer.hover_pos()))
        else {
            return false;
        };
        let Some((camera, scene)) = self.camera.zip(self.scene.clone()) else {
            return false;
        };
        let Some(hit) = pick_scene_hit(&camera, response.rect, pointer, &scene) else {
            return true;
        };
        if Some(hit.layer_id) != self.align.moving_layer() {
            self.align_status = Some("The mask is painted on the scan that moves".into());
            return true;
        }
        let Some(entry) = scene.meshes().get(hit.layer_index) else {
            return true;
        };
        let Some(pose) = Rigid::from_affine(&entry.transform) else {
            return true;
        };

        let vertex_count = entry.mesh.vertices().len();
        let mut mask = match self.align_mask.as_ref() {
            Some(existing) if existing.len() == vertex_count => existing.as_ref().clone(),
            _ => vec![occluview_align::INCLUDED; vertex_count],
        };
        let positions: Vec<f32> = entry
            .mesh
            .vertices()
            .iter()
            .flat_map(|vertex| vertex.position)
            .collect();

        let erase = ctx.input(|input| input.modifiers.shift);
        let changed = occluview_align::apply_brush(
            &mut mask,
            &positions,
            pose,
            &MaskEdit {
                center: DVec3::new(
                    f64::from(hit.point.x),
                    f64::from(hit.point.y),
                    f64::from(hit.point.z),
                ),
                radius_mm: f64::from(self.align_brush.radius_mm()),
                erase,
            },
        );

        self.align_mask_stroke_open = true;
        if changed > 0 {
            self.align_mask = Some(Arc::new(mask));
            ctx.request_repaint();
        }
        true
    }

    /// Apply one whole-mask command from the panel.
    pub(super) fn apply_align_mask_command(&mut self, command: MaskCommand) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let Some(moving) = self
            .align
            .moving_layer()
            .and_then(|id| super::app_align::layer_of(&scene, id))
        else {
            return;
        };
        let vertex_count = moving.mesh.vertices().len();
        let mut mask = match self.align_mask.as_ref() {
            Some(existing) if existing.len() == vertex_count => existing.as_ref().clone(),
            _ => vec![occluview_align::INCLUDED; vertex_count],
        };
        match command {
            MaskCommand::Nowhere => occluview_align::set_all(&mut mask, false),
            MaskCommand::Everywhere => occluview_align::set_all(&mut mask, true),
            MaskCommand::Invert => occluview_align::invert(&mut mask),
            MaskCommand::AroundPoints => {
                let Some(pose) = Rigid::from_affine(&moving.transform) else {
                    return;
                };
                let positions: Vec<f32> = moving
                    .mesh
                    .vertices()
                    .iter()
                    .flat_map(|vertex| vertex.position)
                    .collect();
                let points: Vec<DVec3> = self
                    .align
                    .pairs()
                    .iter()
                    .map(|pair| {
                        let world = moving.transform.transform_point3(pair.moving.local);
                        DVec3::new(f64::from(world.x), f64::from(world.y), f64::from(world.z))
                    })
                    .collect();
                occluview_align::mark_around(
                    &mut mask,
                    &positions,
                    pose,
                    &points,
                    f64::from(self.align_brush.radius_mm()),
                );
            }
        }
        self.align_mask = Some(Arc::new(mask));
        self.align_status = Some(command.report().into());
        self.invalidate_deviation_map(command.report());
    }

    /// Drop the mask — done whenever the pair changes, since a mask is indexed
    /// by one layer's vertices.
    pub(super) fn clear_align_mask(&mut self) {
        self.align_mask = None;
        self.align_mask_stroke_open = false;
    }
}

#[cfg(test)]
mod tests {
    use crate::align_brush::MaskCommand;

    /// Painting changes what would be measured, so a map drawn before the
    /// stroke describes a comparison that no longer exists. Dropping it is
    /// honest; silently recomputing behind the operator's hand is not, and
    /// recomputing per dab would also be slow.
    #[test]
    fn a_stroke_drops_the_map_instead_of_recomputing_it() {
        let source = include_str!("app_align_brush.rs");
        let production = source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before);
        assert!(
            production.contains("self.invalidate_deviation_map("),
            "a mask change must invalidate the map"
        );
        assert!(
            !production.contains("measure_if_shown"),
            "the brush must never kick off a measurement"
        );
    }

    /// Every command must actually reach the mask, or a panel button is a lie.
    #[test]
    fn every_mask_command_is_handled() {
        let source = include_str!("app_align_brush.rs");
        for command in [
            MaskCommand::Nowhere,
            MaskCommand::Everywhere,
            MaskCommand::Invert,
            MaskCommand::AroundPoints,
        ] {
            let name = format!("MaskCommand::{command:?}");
            assert!(
                source.matches(name.as_str()).count() >= 2,
                "{name} is declared but never applied"
            );
        }
    }
}
