#![allow(clippy::expect_used, clippy::float_cmp, clippy::panic)]

/// A deviation map must reach the screen unlit and in its own measured colors,
/// while the brush's paint must keep the scan's own material underneath.
#[test]
fn a_deviation_overlay_forces_unlit_vertex_colors() {
    use glam::Vec3;
    use occluview_core::scene::SceneMesh;
    use occluview_core::{Mesh, OverlayKind, Vertex};

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
    assert_eq!(plain.overlay_paint, 0);
    assert_eq!(plain.show_vertex_colors, 0);

    let colors = std::sync::Arc::new(vec![[0u8, 0, 0, 255]; 3]);
    let measured = entry
        .clone()
        .with_overlay(OverlayKind::Measured, Some(std::sync::Arc::clone(&colors)));
    let mapped = super::scene_mesh_uniform(&measured);
    assert_eq!(mapped.measured_map, 1, "a deviation map must draw unlit");
    assert_eq!(mapped.overlay_paint, 0);
    assert_eq!(mapped.show_vertex_colors, 1);
    assert_eq!(mapped.show_texture, 0);

    // Paint is not a measurement: the scan keeps its texture and tint, and the
    // marking is mixed over that material instead of replacing the whole scan.
    let paint = entry.with_overlay(OverlayKind::Paint, Some(colors));
    let painted = super::scene_mesh_uniform(&paint);
    assert_eq!(painted.measured_map, 0, "paint must stay lit");
    assert_eq!(painted.overlay_paint, 1);
    assert_eq!(painted.show_vertex_colors, 1);
    assert_eq!(
        painted.show_texture, 1,
        "paint must not hide the scan's texture"
    );
    assert_eq!(
        painted.tint, plain.tint,
        "paint must not drop the scan's tint"
    );
}
