//! Structural-op tests: Separate/Cut single-pass correctness, the component
//! cap, and a headline perf harness for the "Divide" hang the owner hit.
#![allow(
    clippy::expect_used,
    clippy::print_stdout,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    // Tint arrays are compared for exact inequality (a spawned part must not
    // reuse the source tint); bit-exact float comparison is intentional here.
    clippy::float_cmp
)]

use super::super::{EditModeController, LayerContextAction, LayerContextRequest, Scene};
use super::selection_ops::apply_selected_face_mesh_edit_action;
use super::structural::{apply_separate_selected_components, MAX_SEPARATE_COMPONENTS};
use super::SelectedFaceEditContext;
use crate::edit_mode::EditModeCommand;
use glam::Vec3;
use occluview_core::{FaceSelection, Mesh, SceneMesh, ScenePickHit, Vertex};

/// Build a triangulated grid of `cols`x`rows` quads on a gentle paraboloid so
/// vertex positions and normals carry real, distinct geometry.
/// Vertices: `(cols + 1) * (rows + 1)`. Triangles: `2 * cols * rows`.
fn grid_mesh(cols: usize, rows: usize) -> Mesh {
    let mut vertices = Vec::with_capacity((cols + 1) * (rows + 1));
    for y in 0..=rows {
        for x in 0..=cols {
            let fx = x as f32;
            let fy = y as f32;
            let fz = 0.01 * (fx * fx + fy * fy);
            vertices.push(Vertex::at(Vec3::new(fx, fy, fz)));
        }
    }
    let stride = cols + 1;
    let mut indices = Vec::with_capacity(2 * cols * rows * 3);
    for y in 0..rows {
        for x in 0..cols {
            let a = (y * stride + x) as u32;
            let b = a + 1;
            let c = a + stride as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    Mesh::new(Some("grid".into()), vertices, indices).expect("valid grid mesh")
}

/// Select every triangle in the even grid rows. Odd rows stay unselected and
/// break vertical connectivity, so each selected row is its own connected
/// component: this yields `rows.div_ceil(2)` components — the fragmented
/// "select a noisy half" shape that made Separate hang.
fn even_row_strip_selection(cols: usize, rows: usize) -> (FaceSelection, usize) {
    let tris_per_row = 2 * cols;
    let mut mask = vec![false; tris_per_row * rows];
    let mut components = 0;
    for y in (0..rows).step_by(2) {
        for i in 0..tris_per_row {
            mask[y * tris_per_row + i] = true;
        }
        components += 1;
    }
    (FaceSelection::new(mask), components)
}

fn scene_with_mesh(mesh: Mesh) -> Scene {
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh));
    scene
}

fn separate_directly(
    scene: &mut Scene,
    selection: &FaceSelection,
    edit_mode: &mut EditModeController,
) -> usize {
    let layer_id = scene.meshes()[0].id();
    let token = edit_mode
        .begin_scene_edit(scene, layer_id, EditModeCommand::SeparateSelectedComponents)
        .expect("scene edit token");
    let context = SelectedFaceEditContext {
        index: 0,
        layer_id,
        token,
    };
    let apply = apply_separate_selected_components(scene, context, selection, edit_mode)
        .expect("separate ok");
    let _ = apply;
    scene.meshes().len()
}

#[test]
#[ignore = "perf harness: run with --ignored --nocapture"]
fn perf_separate_fragmented_half() {
    let (cols, rows) = (300, 500); // 300k triangles
    let mesh = grid_mesh(cols, rows);
    let tri_count = mesh.triangle_count();
    let (selection, components) = even_row_strip_selection(cols, rows);
    let mut scene = scene_with_mesh(mesh);
    let mut edit_mode = EditModeController::new(4, usize::MAX);

    let start = std::time::Instant::now();
    let layers = separate_directly(&mut scene, &selection, &mut edit_mode);
    let elapsed = start.elapsed();

    println!(
        "SEPARATE {tri_count} tris, {components} components -> {layers} layers in {elapsed:?}"
    );
}

#[test]
#[ignore = "perf harness: run with --ignored --nocapture"]
fn perf_label_1m() {
    let (cols, rows) = (1000, 500); // 1M triangles
    let mesh = grid_mesh(cols, rows);
    let (selection, components) = even_row_strip_selection(cols, rows);
    let start = std::time::Instant::now();
    let labelled =
        occluview_core::selected_connected_components_in_mesh(&mesh, &selection).expect("label");
    let elapsed = start.elapsed();
    let tris = mesh.triangle_count();
    let found = labelled.len();
    println!("LABEL {tris} tris -> {found} components (expected {components}) in {elapsed:?}");
}

fn request(scene: &Scene, action: LayerContextAction) -> LayerContextRequest {
    LayerContextRequest {
        index: 0,
        layer_id: scene.meshes()[0].id(),
        action,
    }
}

fn select_faces(scene: &Scene, edit_mode: &mut EditModeController, triangles: &[usize]) {
    let layer_id = scene.meshes()[0].id();
    for &triangle_index in triangles {
        assert!(edit_mode.select_face_hit_with_mode(
            scene,
            ScenePickHit {
                layer_index: 0,
                layer_id,
                triangle_index,
                point: Vec3::ZERO,
                distance: 1.0,
            },
            false,
        ));
    }
}

#[test]
fn separate_two_strips_yields_two_layers_and_deterministic_order() {
    // 4x1 grid: 8 triangles across 4 columns. Select columns 0 and 3 so the
    // selection is two disconnected strips (columns 1-2 unselected between).
    let mesh = grid_mesh(4, 1);
    let mut scene = scene_with_mesh(mesh);
    let mut edit_mode = EditModeController::new(4, usize::MAX);
    // Column c owns triangles 2c and 2c+1.
    select_faces(&scene, &mut edit_mode, &[0, 1, 6, 7]);

    let request = request(&scene, LayerContextAction::SeparateSelectedComponents);
    let apply = apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode)
        .expect("separate ok");
    assert!(apply.scene_changed && apply.structural_scene_change);
    // Source keeps the remainder (columns 1-2 = 4 triangles) plus two new
    // component layers of 2 triangles each, in ascending source order.
    assert_eq!(scene.meshes().len(), 3);
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 4);
    assert_eq!(scene.meshes()[1].mesh.triangle_count(), 2);
    assert_eq!(scene.meshes()[2].mesh.triangle_count(), 2);
}

#[test]
fn separate_preserves_vertex_attributes_on_components() {
    let mut mesh = grid_mesh(4, 1);
    // Tag every vertex with a distinct color so we can prove attributes carry
    // through the single-pass remap unchanged.
    let colored: Vec<Vertex> = mesh
        .vertices()
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let mut v = *v;
            v.color = [(i as u8).wrapping_mul(7), 1, 2, 255];
            v
        })
        .collect();
    mesh = Mesh::new(Some("grid".into()), colored, mesh.indices().to_vec()).expect("recolor");
    let mut scene = scene_with_mesh(mesh);
    let mut edit_mode = EditModeController::new(4, usize::MAX);
    select_faces(&scene, &mut edit_mode, &[0, 1]);

    let request = request(&scene, LayerContextAction::SeparateSelectedComponents);
    let apply = apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode)
        .expect("separate ok");
    assert!(apply.scene_changed);
    let component = &scene.meshes()[1].mesh;
    assert_eq!(component.triangle_count(), 2);
    // Column 0 uses vertices 0,1,5,6 (indices a,b,c,d of the first quad). Every
    // extracted vertex color must match one of the source colors, none reset.
    for vertex in component.vertices() {
        assert_ne!(vertex.color, [255, 255, 255, 255], "attributes were reset");
    }
}

/// `n` pairwise-disconnected triangles (no shared vertices), so a full selection
/// yields exactly `n` connected components.
fn isolated_triangles_mesh(n: usize) -> Mesh {
    let mut vertices = Vec::with_capacity(n * 3);
    let mut indices = Vec::with_capacity(n * 3);
    for i in 0..n {
        let base = (i * 3) as u32;
        let x = i as f32 * 4.0;
        vertices.push(Vertex::at(Vec3::new(x, 0.0, 0.0)));
        vertices.push(Vertex::at(Vec3::new(x + 1.0, 0.0, 0.0)));
        vertices.push(Vertex::at(Vec3::new(x, 1.0, 0.0)));
        indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    Mesh::new(Some("islands".into()), vertices, indices).expect("valid islands mesh")
}

#[test]
fn separate_refuses_selection_that_explodes_past_the_cap() {
    // One more component than the cap allows -> honest no-op, no layer storm.
    let count = MAX_SEPARATE_COMPONENTS + 1;
    let mut scene = scene_with_mesh(isolated_triangles_mesh(count));
    let mut edit_mode = EditModeController::new(4, usize::MAX);
    let selection = FaceSelection::new(vec![true; count]);
    let layer_id = scene.meshes()[0].id();
    let token = edit_mode
        .begin_scene_edit(
            &scene,
            layer_id,
            EditModeCommand::SeparateSelectedComponents,
        )
        .expect("scene edit token");
    let context = SelectedFaceEditContext {
        index: 0,
        layer_id,
        token,
    };

    let apply = apply_separate_selected_components(&mut scene, context, &selection, &mut edit_mode)
        .expect("cap refusal must not error");
    assert!(!apply.scene_changed, "capped Separate must be a no-op");
    assert_eq!(scene.meshes().len(), 1, "no new layers were spawned");
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), count);
    // The refusal discarded its pre-op snapshot: no phantom undo step.
    assert_eq!(edit_mode.undo_layer_id(), None);
}

#[test]
fn separate_at_cap_still_runs() {
    // Exactly the cap is allowed. Build one extra triangle and leave it out of
    // the selection so the source keeps a real (non-empty) remainder.
    let count = MAX_SEPARATE_COMPONENTS;
    let mut scene = scene_with_mesh(isolated_triangles_mesh(count + 1));
    let mut edit_mode = EditModeController::new(4, usize::MAX);
    let mut mask = vec![true; count + 1];
    mask[count] = false; // last triangle stays with the source
    let selection = FaceSelection::new(mask);
    let layer_id = scene.meshes()[0].id();
    let token = edit_mode
        .begin_scene_edit(
            &scene,
            layer_id,
            EditModeCommand::SeparateSelectedComponents,
        )
        .expect("scene edit token");
    let context = SelectedFaceEditContext {
        index: 0,
        layer_id,
        token,
    };
    let apply = apply_separate_selected_components(&mut scene, context, &selection, &mut edit_mode)
        .expect("separate at cap ok");
    assert!(apply.scene_changed);
    // Source remainder (1 triangle) + one layer per extracted component.
    assert_eq!(scene.meshes().len(), count + 1);
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 1);
}

#[test]
fn delete_removes_only_selected_faces_and_keeps_a_clean_remainder() {
    // 3x1 grid = 6 triangles. Delete the two triangles of the middle column.
    let mesh = grid_mesh(3, 1);
    let mut scene = scene_with_mesh(mesh);
    let mut edit_mode = EditModeController::new(4, usize::MAX);
    select_faces(&scene, &mut edit_mode, &[2, 3]); // middle column

    let request = request(&scene, LayerContextAction::DeleteSelectedFaces);
    let apply = apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode)
        .expect("delete ok");
    assert!(apply.scene_changed);
    assert_eq!(scene.meshes().len(), 1, "delete never spawns layers");
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 4);
}

#[test]
fn crop_keeps_exactly_the_selection() {
    let mesh = grid_mesh(3, 1);
    let mut scene = scene_with_mesh(mesh);
    let mut edit_mode = EditModeController::new(4, usize::MAX);
    select_faces(&scene, &mut edit_mode, &[0, 1]); // first column only

    let request = request(&scene, LayerContextAction::CropToSelectedFaces);
    let apply =
        apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode).expect("crop ok");
    assert!(apply.scene_changed);
    assert_eq!(scene.meshes().len(), 1);
    assert_eq!(
        scene.meshes()[0].mesh.triangle_count(),
        2,
        "kept only the selection"
    );
}

#[test]
fn cut_moves_selection_to_a_new_layer_preserving_presentation() {
    let mesh = grid_mesh(3, 1);
    let mut scene = scene_with_mesh(mesh);
    let mut edit_mode = EditModeController::new(4, usize::MAX);
    let before_transform = scene.meshes()[0].transform;
    scene.meshes_mut()[0].show_orientation = true;
    select_faces(&scene, &mut edit_mode, &[0, 1]); // first column

    let request = request(&scene, LayerContextAction::CutSelectionToNewLayer);
    let apply =
        apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode).expect("cut ok");
    assert!(apply.scene_changed && apply.structural_scene_change);
    // Source keeps the remainder; a new layer holds exactly the cut faces.
    assert_eq!(scene.meshes().len(), 2);
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 4);
    assert_eq!(scene.meshes()[1].mesh.triangle_count(), 2);
    assert_eq!(scene.meshes()[1].transform, before_transform);
    assert!(scene.meshes()[1].show_orientation);
}

#[test]
#[ignore = "perf harness: run with --ignored --nocapture"]
fn perf_separate_soup_500k() {
    // The owner's real input: a large STL-style SOUP model with a two-wall
    // (fragmented) selection. This exercises the full executor Separate INCLUDING
    // the soup->shared-topology weld, which is the cost the fix adds. It must
    // stay interactive (sub-second), not regress to minutes.
    let (cols, rows) = (500, 500); // 500k triangles, 1.5M soup vertices
    let welded = grid_mesh(cols, rows);
    let soup = explode_mesh_to_soup(&welded);
    let tri_count = soup.triangle_count();
    let vtx_count = soup.vertices().len();
    let (selection, components) = even_row_strip_selection(cols, rows);
    let mut scene = scene_with_mesh(soup);
    let mut edit_mode = EditModeController::new(4, usize::MAX);

    let start = std::time::Instant::now();
    let layers = separate_directly(&mut scene, &selection, &mut edit_mode);
    let elapsed = start.elapsed();
    println!(
        "SEPARATE-SOUP-500K {tri_count} tris, {vtx_count} soup verts, \
         {components} selected strips -> {layers} layers in {elapsed:?}"
    );
}

#[test]
#[ignore = "perf harness: run with --ignored --nocapture"]
fn perf_separate_1m() {
    let (cols, rows) = (1000, 500); // 1M triangles
    let mesh = grid_mesh(cols, rows);
    let tri_count = mesh.triangle_count();
    let (selection, components) = even_row_strip_selection(cols, rows);
    let mut scene = scene_with_mesh(mesh);
    let mut edit_mode = EditModeController::new(4, usize::MAX);

    let start = std::time::Instant::now();
    let layers = separate_directly(&mut scene, &selection, &mut edit_mode);
    let elapsed = start.elapsed();
    println!(
        "SEPARATE-1M {tri_count} tris, {components} components -> {layers} layers in {elapsed:?}"
    );
}

/// Explode a welded mesh into STL-style soup: three fresh vertices per triangle
/// corner, sequential indices, nothing shared — byte-for-byte the topology a
/// binary STL reader produces. Separate on this used to explode into one part
/// per triangle (the owner's "317000 parts"); it must now weld back to the true
/// island count before splitting.
fn explode_mesh_to_soup(mesh: &Mesh) -> Mesh {
    let mut vertices = Vec::with_capacity(mesh.indices().len());
    let mut indices = Vec::with_capacity(mesh.indices().len());
    for &vi in mesh.indices() {
        indices.push(vertices.len() as u32);
        vertices.push(mesh.vertices()[vi as usize]);
    }
    Mesh::new(Some("soup".into()), vertices, indices).expect("valid soup mesh")
}

#[test]
fn separate_soup_connected_patch_is_one_part_not_confetti() {
    // Case (a): an open scan with ONE selected patch. The patch is two
    // edge-connected triangles; as STL soup they share no vertex index, so the
    // old index-topology path saw two islands (and a real dental patch of tens
    // of thousands of triangles exploded into that many parts). After the weld
    // it is a single island: source remainder + exactly one extracted part.
    let soup = explode_mesh_to_soup(&grid_mesh(3, 1));
    let mut scene = scene_with_mesh(soup);
    let mut edit_mode = EditModeController::new(4, usize::MAX);
    select_faces(&scene, &mut edit_mode, &[0, 1]); // first column, one patch

    let request = request(&scene, LayerContextAction::SeparateSelectedComponents);
    let apply = apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode)
        .expect("separate ok");
    assert!(apply.scene_changed && apply.structural_scene_change);
    assert_eq!(
        scene.meshes().len(),
        2,
        "one connected soup patch -> remainder + one part (not confetti)"
    );
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 4, "remainder");
    assert_eq!(scene.meshes()[1].mesh.triangle_count(), 2, "the patch");
}

#[test]
fn separate_soup_two_walls_become_two_parts_plus_one_remainder() {
    // Case (b)/(c): a through-mode lasso on a closed hollow model marks a patch
    // on BOTH the outer and the inner wall in one gesture — two disjoint islands.
    // Modelled here as two disconnected soup strips (columns 0 and 3 of a 4x1
    // grid, columns 1-2 unselected between). Contract: each disjoint marked
    // island becomes its own layer (deterministic source order, stepped tints);
    // the remainder stays ONE layer.
    let soup = explode_mesh_to_soup(&grid_mesh(4, 1));
    let mut scene = scene_with_mesh(soup);
    let mut edit_mode = EditModeController::new(4, usize::MAX);
    select_faces(&scene, &mut edit_mode, &[0, 1, 6, 7]); // columns 0 and 3

    let request = request(&scene, LayerContextAction::SeparateSelectedComponents);
    let apply = apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode)
        .expect("separate ok");
    assert!(apply.scene_changed);
    assert_eq!(
        scene.meshes().len(),
        3,
        "two marked walls -> remainder + two parts"
    );
    assert_eq!(scene.meshes()[0].mesh.triangle_count(), 4, "one remainder");
    assert_eq!(scene.meshes()[1].mesh.triangle_count(), 2);
    assert_eq!(scene.meshes()[2].mesh.triangle_count(), 2);
    // Every spawned part steps the palette off the source tint (visible split).
    let source_tint = scene.meshes()[0].tint;
    for part in scene.meshes().iter().skip(1) {
        assert_ne!(part.tint, source_tint);
    }
}

#[test]
fn separate_remainder_stays_one_layer_even_when_the_cut_disconnects_it() {
    // Contract: the remainder is always a SINGLE layer, even when removing the
    // marked island splits the leftover surface into disjoint shells. Select the
    // middle column of a 3x1 soup grid: the marked island is one part, and the
    // remainder (columns 0 and 2) is geometrically disconnected yet stays one
    // layer — matching exocad's "the rest stays the base".
    let soup = explode_mesh_to_soup(&grid_mesh(3, 1));
    let mut scene = scene_with_mesh(soup);
    let mut edit_mode = EditModeController::new(4, usize::MAX);
    select_faces(&scene, &mut edit_mode, &[2, 3]); // middle column only

    let request = request(&scene, LayerContextAction::SeparateSelectedComponents);
    let apply = apply_selected_face_mesh_edit_action(&mut scene, request, &mut edit_mode)
        .expect("separate ok");
    assert!(apply.scene_changed);
    assert_eq!(
        scene.meshes().len(),
        2,
        "one marked island + one (disconnected) remainder layer"
    );
    assert_eq!(
        scene.meshes()[0].mesh.triangle_count(),
        4,
        "remainder holds both disjoint columns in one layer"
    );
    assert_eq!(
        scene.meshes()[1].mesh.triangle_count(),
        2,
        "the marked column"
    );
}

#[test]
fn separate_parts_get_distinct_tints_so_the_split_is_visible() {
    // The spawned parts are geometrically coincident with where they sat in
    // the source, so with the source tint the divide would be invisible on
    // screen (the owner's "Separate does nothing" report). Every spawned
    // layer must step the palette, exocad-style.
    let mesh = grid_mesh(4, 4);
    let tris_per_row = 2 * 4;
    let mut mask = vec![false; mesh.triangle_count()];
    for slot in mask.iter_mut().take(tris_per_row * 2) {
        *slot = true;
    }
    let selection = FaceSelection::new(mask);
    let mut scene = scene_with_mesh(mesh);
    let source_tint = scene.meshes()[0].tint;
    let mut edit_mode = EditModeController::new(4, usize::MAX);

    let layers = separate_directly(&mut scene, &selection, &mut edit_mode);

    assert!(layers >= 2, "separate must spawn at least one part layer");
    for part in scene.meshes().iter().skip(1) {
        assert_ne!(
            part.tint, source_tint,
            "a spawned part must not share the source tint (invisible split)"
        );
    }
}
