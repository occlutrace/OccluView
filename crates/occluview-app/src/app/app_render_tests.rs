#![allow(clippy::expect_used, clippy::float_cmp, clippy::panic)]

/// Source contract for the destroyed-texture submit crash. The per-frame
/// render paths must update ONE persistent egui texture id in place
/// (`TextureHandle::set` / `CutTool::store_slice`), never allocate a fresh id
/// per render. A fresh id can free a texture after this frame has painted it.
#[test]
fn per_frame_render_paths_reuse_persistent_texture_ids() {
    let source = crate::primary_ui_tests::production_source(include_str!("app_render.rs"))
        .replace("\r\n", "\n");
    assert!(
        source.contains("frame.texture.set(color_image, egui::TextureOptions::LINEAR)"),
        "render_now must update the viewport texture in place, not reallocate it"
    );
    assert!(
        source.contains("self.tools.cut_view.store_slice(ctx, color_image, slice_cam)"),
        "render_cut_now must route the slice through CutTool::store_slice"
    );
    assert!(
        !source.contains("load_texture(\"occluview-cut\""),
        "the cut slice must not allocate a fresh egui texture id per render"
    );
}

/// A deviation map must reach the screen unlit and in its own measured colors.
#[test]
fn a_deviation_overlay_forces_unlit_vertex_colors() {
    use glam::Vec3;
    use occluview_core::scene::SceneMesh;
    use occluview_core::{Mesh, Vertex};

    let mesh = Mesh::new(
        None,
        vec![
            Vertex::at(Vec3::ZERO),
            Vertex::at(Vec3::new(1.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2],
    )
    .expect("valid mesh");

    let mut entry = SceneMesh::new(mesh);
    entry.show_vertex_colors = false;
    entry.show_texture = true;

    let plain = super::scene_mesh_uniform(&entry);
    assert_eq!(plain.measured_map, 0);
    assert_eq!(plain.show_vertex_colors, 0);

    let colors = std::sync::Arc::new(vec![[0u8, 0, 0, 255]; 3]);
    let mapped = super::scene_mesh_uniform(&entry.with_deviation(Some(colors)));
    assert_eq!(mapped.measured_map, 1, "a deviation map must draw unlit");
    assert_eq!(mapped.show_vertex_colors, 1);
    assert_eq!(mapped.show_texture, 0);
}

#[test]
fn the_offscreen_viewport_replays_overlay_vertices_after_scene_upload() {
    let source = crate::primary_ui_tests::production_source(include_str!("app_render.rs"));
    assert!(
        source.contains("fn push_deviation_colors_offscreen"),
        "the fallback viewport needs its own overlay upload path"
    );
    assert!(
        source.contains("prepared.write_entry_vertices(offscreen.renderer()"),
        "offscreen scene uploads must receive the measured colours"
    );
    // The body, not the rest of the file: the upload helper is *defined* below
    // this method, so "everything after the signature" was satisfied by the
    // definition even after the call was gone.
    let body = method_body(source, "pub(super) fn render_scene_pixels");
    assert!(
        !body.is_empty(),
        "render_scene_pixels must exist and end at the impl indentation"
    );
    assert!(
        body.contains("push_deviation_colors_offscreen()"),
        "render_scene_pixels must restore a map after rebuilding its prepared scene"
    );
}

/// One method's body: from its signature to the first line closing at the impl
/// indentation, so a call has to be inside the method and not merely later in
/// the file.
fn method_body<'a>(source: &'a str, signature: &str) -> &'a str {
    source
        .split_once(signature)
        .and_then(|(_, rest)| rest.split_once("\n    }"))
        .map(|(body, _)| body)
        .unwrap_or_default()
}
