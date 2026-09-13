//! Provenance across real history transitions.
//!
//! `app_scene_commit.rs` unit-tests `reconcile_scene_paths` directly, which
//! shows the mapping rule but not that the app keeps a layer's source path
//! through the transitions an operator actually performs. These tests run the
//! real history path — `apply_history_navigation_now`, which goes through
//! `commit_scene_draft` → `commit_structural_scene` — and then ask the export
//! defaults what directory, filename, and format a layer would get.

#![allow(
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use super::app_layer_edits;
use super::app_mesh_export::{
    default_layer_export_directory, default_layer_export_format, default_layer_export_stem,
};
use super::app_test_support::{named_scene, push_named_layer, scene_names, test_app};
use super::layers_overlay::LayerOverlayChanges;
use super::*;
use crate::edit_mode::EditModeCommand;
use occluview_core::{Mesh, SceneMesh, Vertex};
use occluview_formats::write::MeshWriteFormat;

/// The layer ids in scene order.
fn layer_ids(app: &OccluViewApp) -> Vec<occluview_core::SceneMeshId> {
    app.document
        .scene
        .as_ref()
        .expect("scene")
        .meshes()
        .iter()
        .map(SceneMesh::id)
        .collect()
}

#[test]
fn split_then_undo_redo_keeps_source_paths_and_export_defaults() {
    // The operator starts with one imported scan, cuts a new layer off it
    // (a structural op with its own history step), then steps Undo and Redo.
    // Every step must leave the layers' source paths and the export defaults a
    // save dialog would offer consistent with the scene.
    let mut app = test_app("provenance-history");
    let cases = std::env::temp_dir().join(format!("occluview-provenance-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&cases);
    let source_file = cases.join("lower.stl");
    app.document.scene = Some(Arc::new(named_scene("lower", 0.0)));
    app.persistence.current_paths = vec![source_file.clone()];
    let source_id = layer_ids(&app)[0];

    // The structural op: duplicate the layer as a derived one, which is the
    // shape Cut/Separate produce (a new id carrying the source's provenance).
    let token = app
        .document
        .edit_mode
        .begin_scene_edit(
            app.document.scene.as_ref().expect("scene"),
            source_id,
            EditModeCommand::CutSelectionToNewLayer,
        )
        .expect("scene edit token");
    let mut draft = app.document.scene.as_ref().expect("scene").as_ref().clone();
    let derived = SceneMesh::new(
        Mesh::new(
            Some("derived".to_string()),
            vec![
                Vertex::at(glam::Vec3::new(0.0, 0.0, 0.0)),
                Vertex::at(glam::Vec3::new(1.0, 0.0, 0.0)),
                Vertex::at(glam::Vec3::new(0.0, 1.0, 0.0)),
            ],
            vec![0, 1, 2],
        )
        .expect("test mesh"),
    )
    .with_source_layer_id(source_id);
    let derived_id = derived.id();
    draft.insert(1, derived);
    assert_eq!(
        app.document
            .edit_mode
            .finish_scene_edit_success(token, &draft),
        crate::edit_mode::BusyFinish::Applied
    );
    let previous = app.document.scene.clone();
    app.commit_structural_scene(previous.as_deref(), draft, &egui::Context::default());
    app.document.mark_mesh_edits_unsaved(source_id);
    app.document.mark_mesh_edits_unsaved(derived_id);

    let after_split = app.persistence.current_paths.clone();
    assert_eq!(
        after_split,
        vec![source_file.clone(), source_file.clone()],
        "a derived layer inherits the source path"
    );

    // Undo: the derived layer goes away, the source keeps its path.
    app.apply_history_navigation_now(false, &egui::Context::default());
    assert_eq!(
        scene_names(&app),
        vec!["lower".to_string()],
        "undo removes the derived layer"
    );
    assert_eq!(
        app.persistence.current_paths.len(),
        app.document.scene.as_ref().expect("scene").meshes().len(),
        "paths stay index-aligned with the scene"
    );
    assert_eq!(app.persistence.current_paths[0], source_file);

    // Redo: the derived layer returns with the source path again.
    app.apply_history_navigation_now(true, &egui::Context::default());
    assert_eq!(
        scene_names(&app),
        vec!["lower".to_string(), "derived".to_string()],
        "redo restores the derived layer"
    );
    let redone_paths = app.persistence.current_paths.clone();
    assert_eq!(
        redone_paths.len(),
        app.document.scene.as_ref().expect("scene").meshes().len()
    );

    // What the export dialog would actually offer, per layer: the source
    // directory, the source stem, and the source format.
    let scene = app.document.scene.as_ref().expect("scene");
    for index in 0..scene.meshes().len() {
        let directory = default_layer_export_directory(&redone_paths, index, None);
        assert_eq!(
            directory,
            Some(cases.clone()),
            "layer {index} should default to the folder it came from"
        );
        assert_eq!(
            default_layer_export_stem(&redone_paths, scene, index, MeshWriteFormat::StlBinary),
            "lower",
            "layer {index} should default to the source file's name"
        );
        assert_eq!(
            default_layer_export_format(&redone_paths, index, MeshWriteFormat::Obj),
            MeshWriteFormat::StlBinary,
            "layer {index} should keep the source file's format"
        );
    }
}

#[test]
fn removing_a_layer_keeps_the_survivor_path_aligned() {
    // Removing a whole layer from the menu is a structural scene commit with no
    // history step of its own (Ctrl+Z after a Remove reports "nothing to undo"
    // for that layer — the operation is not recorded). What must hold is that
    // the surviving layer keeps its own file: an index-shifted path list would
    // send Upper's next export to Lower's folder and name.
    let mut app = test_app("provenance-remove");
    let mut scene = named_scene("lower", 0.0);
    let upper_id = push_named_layer(&mut scene, "upper", 5.0);
    app.document.scene = Some(Arc::new(scene));
    let cases =
        std::env::temp_dir().join(format!("occluview-provenance-rm-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&cases);
    let lower_file = cases.join("lower.stl");
    let upper_file = cases.join("upper.stl");
    app.persistence.current_paths = vec![lower_file.clone(), upper_file.clone()];

    // The overlay's Remove path commits the draft through the structural
    // helper; paths are reconciled by stable layer id, not by position.
    let mut draft = app.document.scene.as_ref().expect("scene").as_ref().clone();
    let removed = draft.remove(0);
    assert!(removed.is_some(), "the first layer is the one removed");
    let previous = app.document.scene.clone();
    app.commit_structural_scene(previous.as_deref(), draft, &egui::Context::default());

    assert_eq!(scene_names(&app), vec!["upper".to_string()]);
    assert_eq!(
        app.persistence.current_paths,
        vec![upper_file.clone()],
        "the survivor keeps its own path after a neighbour is removed"
    );

    let scene = app.document.scene.as_ref().expect("scene");
    assert_eq!(scene.meshes()[0].id(), upper_id);
    assert_eq!(
        default_layer_export_stem(
            &app.persistence.current_paths,
            scene,
            0,
            MeshWriteFormat::StlBinary
        ),
        "upper",
        "the survivor exports under its own name, not the removed layer's"
    );
    assert_eq!(
        default_layer_export_directory(&app.persistence.current_paths, 0, None),
        Some(cases),
        "and into its own folder"
    );
}

/// Append a second grid mesh, so a part can be cut out of a part.
fn append_cuttable_layer(scene: &mut Scene, name: &str, x_offset: f32) {
    let mut other = cuttable_scene(name);
    let mut entry = other.remove(0).expect("the grid scene holds one layer");
    entry.transform = glam::Affine3A::from_translation(glam::Vec3::new(x_offset, 0.0, 0.0));
    scene.add(entry);
}

/// A grid mesh whose faces can actually be cut, unlike `named_scene`'s single
/// triangle (a whole-mesh selection is refused by design). Partial statements
/// are skipped by the shared test config, so a cut is neither empty nor whole.
fn cuttable_scene(name: &str) -> Scene {
    let cols = 4;
    let rows = 4;
    let mut vertices = Vec::new();
    for y in 0..=rows {
        for x in 0..=cols {
            vertices.push(Vertex::at(glam::Vec3::new(x as f32, y as f32, 0.0)));
        }
    }
    let stride = cols + 1;
    let mut indices = Vec::new();
    for y in 0..rows {
        for x in 0..cols {
            let a = u32::try_from(y * stride + x).expect("grid index fits u32");
            let b = a + 1;
            let c = a + u32::try_from(stride).expect("stride fits u32");
            let d = c + 1;
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    let mesh = Mesh::new(Some(name.to_string()), vertices, indices).expect("grid mesh");
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh));
    scene
}

/// Run the operator's Cut Selection to New Layer through the same executor the
/// Layer menu and the Mesh Editor buttons use: arm face selection on `index`,
/// mark one triangle, apply the action, commit the draft.
fn cut_one_triangle(app: &mut OccluViewApp, index: usize) {
    let scene = app.document.scene.as_ref().expect("scene").clone();
    let entry = scene.meshes()[index].clone();
    let layer_id = entry.id();
    let total = entry.mesh.triangle_count();
    assert!(
        total >= 8,
        "a whole-mesh selection is refused by design, and the part must be \
         large enough to cut again, so the source needs several triangles"
    );
    assert!(
        app.document
            .edit_mode
            .begin_face_selection(&entry, scene.as_ref()),
        "the edit session must open"
    );
    // Mark a quarter of the faces: not empty, and not the whole mesh.
    let marked = total / 4;
    for triangle_index in 0..marked {
        assert!(
            app.document.edit_mode.select_face_hit(
                scene.as_ref(),
                occluview_core::ScenePickHit {
                    layer_index: index,
                    layer_id,
                    triangle_index,
                    point: glam::Vec3::ZERO,
                    distance: 1.0,
                },
            ),
            "face {triangle_index} must be markable"
        );
    }
    let mut draft = scene.as_ref().clone();
    let apply = app_layer_edits::apply_visible_selected_face_mesh_edit_action(
        &mut draft,
        &mut app.document.edit_mode,
        LayerContextAction::CutSelectionToNewLayer,
    )
    .expect("cut ok");
    assert!(apply.scene_changed, "the cut must change the scene");
    let ids_before: Vec<_> = scene.meshes().iter().map(SceneMesh::id).collect();
    for id in draft
        .meshes()
        .iter()
        .map(SceneMesh::id)
        .filter(|id| !ids_before.contains(id))
    {
        app.document.mark_mesh_edits_unsaved(id);
    }
    let previous = app.document.scene.clone();
    app.commit_structural_scene(previous.as_deref(), draft, &egui::Context::default());
}

/// Remove a layer through the real overlay commit path.
fn remove_layer(app: &mut OccluViewApp, index: usize) {
    let scene = app.document.scene.as_ref().expect("scene").clone();
    let layer_id = scene.meshes()[index].id();
    let paths = app.persistence.current_paths.clone();
    app.apply_layer_overlay_changes(
        scene,
        &paths,
        LayerOverlayChanges {
            context_request: Some(LayerContextRequest {
                index,
                layer_id,
                action: LayerContextAction::Remove,
            }),
            layer_edits: Vec::new(),
        },
        &egui::Context::default(),
    );
}

#[test]
fn a_second_generation_part_keeps_its_ancestor_file() {
    // Two unrelated scans are open. A part is cut out of the first scan, that
    // scan is removed, a part is cut out of the part, and the part is removed.
    // What is left still descends from lower.stl, so its export defaults must
    // say so -- and must not fall through to the other case that happens to sit
    // next to it in the scene.
    let mut app = test_app("provenance-generations");
    let cases = std::env::temp_dir().join(format!("occluview-gen-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&cases);
    let lower_file = cases.join("lower.stl");
    let bite_file = cases.join("bite.obj");
    let mut scene = cuttable_scene("lower");
    append_cuttable_layer(&mut scene, "bite", 20.0);
    app.document.scene = Some(Arc::new(scene));
    app.persistence.current_paths = vec![lower_file.clone(), bite_file.clone()];

    // Generation 1: cut a part out of the imported scan.
    cut_one_triangle(&mut app, 0);
    assert_eq!(
        scene_names(&app),
        vec!["lower".to_string(), "lower".to_string(), "bite".to_string()],
        "the cut inserts the part right after its source"
    );
    assert_eq!(
        app.persistence.current_paths[1], lower_file,
        "the first-generation part inherits its source file"
    );

    // The imported scan leaves; the part carries its file forward.
    remove_layer(&mut app, 0);
    assert_eq!(
        app.persistence.current_paths,
        vec![lower_file.clone(), bite_file.clone()],
        "the part keeps the inherited path after its source is removed"
    );

    // Generation 2: cut a part out of the part. Its ancestor is no longer in
    // the scene, which is where the provenance chain was being dropped.
    cut_one_triangle(&mut app, 0);

    // The intermediate part leaves too, so nothing left in the scene sits
    // between generation 2 and the unrelated scan.
    remove_layer(&mut app, 0);

    let scene = app.document.scene.as_ref().expect("scene");
    let paths = app.persistence.current_paths.clone();
    assert_eq!(paths.len(), scene.meshes().len(), "paths stay aligned");
    assert_eq!(
        scene_names(&app),
        vec!["lower".to_string(), "bite".to_string()],
        "what remains is generation 2 and the unrelated scan"
    );
    assert_eq!(
        default_layer_export_directory(&paths, 0, None),
        Some(cases.clone()),
        "a second-generation part must still be offered its ancestor's folder"
    );
    assert_eq!(
        default_layer_export_stem(&paths, scene, 0, MeshWriteFormat::StlBinary),
        "lower",
        "and its ancestor's file name, not the neighbouring scan's"
    );
    assert_eq!(
        default_layer_export_format(&paths, 0, MeshWriteFormat::Obj),
        MeshWriteFormat::StlBinary,
        "and its ancestor's format"
    );
}
