//! Explicit layer-unit decisions with the document's existing Undo boundary.

use super::{LayerContextAction, LayerContextApply, LayerContextRequest, Scene, SceneContext};
use crate::edit_mode::EditModeCommand;
use occluview_core::{SourceUnit, UnitConfidence};

impl SceneContext<'_> {
    pub(super) fn confirm_layer_units(
        &mut self,
        scene: &mut Scene,
        request: LayerContextRequest,
    ) -> LayerContextApply {
        let interpretation = match request.action {
            LayerContextAction::ConfirmMillimeters => SourceUnit::Millimeters,
            LayerContextAction::ConfirmMeters => SourceUnit::Meters,
            _ => return LayerContextApply::default(),
        };
        let Some(layer) = scene.meshes().get(request.index) else {
            return LayerContextApply::default();
        };
        if layer.id() != request.layer_id
            || layer.import_units().confidence != UnitConfidence::Ambiguous
        {
            return LayerContextApply::default();
        }
        let source = layer.export_source_layer_id();
        // An open editing or placement transaction owns its baseline. Resolve
        // it first so a unit decision cannot be undone by that transaction.
        if self.document.edit_mode.has_active_session()
            || self.tools.sculpt.is_busy()
            || self.align_active()
        {
            return super::app_layer_edits::refuse_busy_layer_edit(
                self.scene_ui,
                &self.ui.locale,
            );
        }
        // A split preserves one import interpretation. Plan every same-source
        // piece before taking history, so a bad scale cannot leave half the
        // source converted. Other documents retain their independent decision.
        let desired_scale: f32 = match interpretation {
            SourceUnit::Meters => 1000.0,
            SourceUnit::Millimeters => 1.0,
            SourceUnit::Unitless => return LayerContextApply::default(),
        };
        let mut updates = Vec::new();
        for (index, entry) in scene.meshes().iter().enumerate() {
            if entry.export_source_layer_id() != source {
                continue;
            }
            let units = entry.import_units();
            if units.confidence == UnitConfidence::ConfirmedByOperator
                && units.scale_to_mm.to_bits() == desired_scale.to_bits()
            {
                continue;
            }
            if !matches!(units.confidence,
                UnitConfidence::Ambiguous | UnitConfidence::ConfirmedByOperator)
            {
                continue;
            }
            let mut updated = entry.clone();
            if !updated.confirm_import_units(interpretation) {
                return LayerContextApply::default();
            }
            updates.push((index, updated));
        }
        let Some(token) = self.document.edit_mode.begin_scene_edit(
            scene,
            request.layer_id,
            EditModeCommand::MoveLayer,
        ) else {
            return LayerContextApply::default();
        };
        for (index, updated) in updates {
            self.document.mark_mesh_edits_unsaved(updated.id());
            scene.meshes_mut()[index] = updated;
        }
        let _ = self.document.edit_mode.finish_scene_edit_success(token, scene);
        self.scene_ui.status_message = Some(
            self.ui.locale.tr(crate::i18n::message_id!("layer-units-confirmed")),
        );
        LayerContextApply {
            scene_changed: true,
            structural_scene_change: true,
            removed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::app::app_test_support::test_app;
    use crate::edit_mode::undo_snapshot::StructuralHistoryStep;
    use glam::{Affine3A, Vec3};
    use occluview_core::{Mesh, SceneMesh, UnitInterpretation, Vertex};

    fn ambiguous_layer() -> SceneMesh {
        let mesh = Mesh::new(
            None,
            vec![Vertex::at(Vec3::ZERO), Vertex::at(Vec3::new(0.12, 0.0, 0.0)),
                Vertex::at(Vec3::new(0.0, 0.08, 0.0))],
            vec![0, 1, 2],
        ).expect("finite triangle");
        SceneMesh::new(mesh).with_import_units(UnitInterpretation::ambiguous_gltf())
    }

    #[test]
    fn explicit_units_preserve_placement_and_restore_through_history() {
        let mut app = test_app("explicit-layer-units");
        let mut scene = Scene::new();
        let layer = ambiguous_layer()
            .with_transform(Affine3A::from_translation(Vec3::new(7.0, 8.0, 9.0)));
        let id = layer.id();
        let original_pose = layer.transform;
        let original_bounds = layer.import_bbox();
        let sibling = super::super::app_layer_edits::clone_layer_with_mesh(
            &layer, layer.mesh.as_ref().clone());
        let sibling_id = sibling.id();
        scene.add(layer);
        scene.add(sibling);
        scene.add(ambiguous_layer());
        app.workspace.scenes[0].document.scene = Some(std::sync::Arc::new(scene.clone()));
        let mut context = app.active_context().expect("scene");
        let request = LayerContextRequest { index: 0, layer_id: id,
            action: LayerContextAction::ConfirmMeters };
        assert!(context.confirm_layer_units(&mut scene, request).scene_changed);
        let confirmed = &scene.meshes()[0];
        assert_eq!(confirmed.transform.translation, original_pose.translation);
        assert_eq!(confirmed.import_bbox(), original_bounds);
        assert_eq!(confirmed.import_units().confidence, UnitConfidence::ConfirmedByOperator);
        assert_eq!(scene.meshes()[1].transform, confirmed.transform);
        assert_eq!(scene.meshes()[1].import_units(), confirmed.import_units());
        assert_eq!(scene.meshes()[2].import_units().confidence, UnitConfidence::Ambiguous);
        let pose = confirmed.transform;
        assert!(!context.confirm_layer_units(&mut scene, request).scene_changed);
        assert_eq!(scene.meshes()[0].transform, pose);
        assert!(context.document.unsaved_edit_layer_ids.contains(&id));
        assert!(context.document.unsaved_edit_layer_ids.contains(&sibling_id));
        let StructuralHistoryStep::Restored(restored) =
            context.document.edit_mode.undo_last_scene_edit(&scene, id)
        else { panic!("unit decision must be undoable"); };
        assert_eq!(restored.meshes()[0].transform, original_pose);
        assert_eq!(restored.meshes()[0].import_units().confidence, UnitConfidence::Ambiguous);
        assert_eq!(restored.meshes()[1].transform, original_pose);
        assert_eq!(restored.meshes()[1].import_units().confidence, UnitConfidence::Ambiguous);
        let StructuralHistoryStep::Restored(redone) =
            context.document.edit_mode.redo_last_scene_edit(&restored, id)
        else { panic!("unit decision must be redoable"); };
        assert_eq!(redone.meshes()[0].transform, pose);
        assert_eq!(redone.meshes()[0].import_units().confidence, UnitConfidence::ConfirmedByOperator);
    }

    #[test]
    fn import_bounds_survive_other_layers_and_extracted_geometry() {
        let layer = ambiguous_layer();
        let original = layer.import_bbox();
        let mut mixed = Scene::new();
        mixed.add(layer.clone());
        mixed.add(SceneMesh::new(layer.mesh.clone())
            .with_transform(Affine3A::from_scale(Vec3::splat(1000.0))));
        assert!(mixed.bbox().size().max_element() > 10.0);
        assert_eq!(occluview_formats::units::recommend_glb_scale(
            mixed.meshes()[0].import_bbox().size().max_element()),
            occluview_formats::units::GlbScaleRecommendation::MetersToMillimeters);
        let extracted = super::super::app_layer_edits::clone_layer_with_mesh(
            &layer, Mesh::empty());
        assert_eq!(extracted.import_bbox(), original);
        assert_eq!(extracted.import_units(), layer.import_units());
    }
}
