use super::sculpt_cursor::{
    cone_geometry, cylinder_geometry, sculpt_surface_light_intensity, sculpt_tool_length,
    SculptBrushUniform, SculptToolShape, SculptToolUniform,
};
use std::mem::size_of;

#[test]
fn cursor_uniforms_have_the_wgsl_alignment_their_bindings_require() {
    assert_eq!(size_of::<SculptBrushUniform>(), 64);
    assert_eq!(size_of::<SculptToolUniform>(), 96);
}

#[test]
fn reference_cursor_length_is_finite_and_bounded() {
    assert_eq!(sculpt_tool_length(f32::NEG_INFINITY), 0.65);
    assert_eq!(sculpt_tool_length(f32::NAN), 0.65);
    assert_eq!(sculpt_tool_length(0.0), 0.65);
    assert_eq!(sculpt_tool_length(1.0), 2.6);
    assert!(sculpt_tool_length(0.5) > 0.65);
    assert!(sculpt_tool_length(0.5) < 2.6);
}

#[test]
fn reference_cursor_light_is_monotonic_without_a_zero_strength_blackout() {
    let weak = sculpt_surface_light_intensity(0.0);
    let medium = sculpt_surface_light_intensity(0.5);
    let strong = sculpt_surface_light_intensity(1.0);

    assert!(weak > 0.0);
    assert!(weak < medium);
    assert!(medium < strong);
    assert!(sculpt_surface_light_intensity(f32::INFINITY).is_finite());
}

#[test]
fn tool_shapes_have_stable_gpu_tags() {
    assert_eq!(SculptToolShape::Cone as u32, 0);
    assert_eq!(SculptToolShape::Cylinder as u32, 1);
}

#[test]
fn tool_geometry_is_open_and_uses_bounded_static_buffers() {
    let (cone_vertices, cone_indices) = cone_geometry();
    let (cylinder_vertices, cylinder_indices) = cylinder_geometry();

    assert_eq!(cone_vertices.len(), 32 * 3);
    assert_eq!(cone_indices.len(), 32 * 3);
    assert_eq!(cylinder_vertices.len(), 32 * 4);
    assert_eq!(cylinder_indices.len(), 32 * 6);
    assert!(cone_vertices.iter().all(|vertex| {
        vertex
            .position
            .iter()
            .all(|component| component.is_finite())
            && vertex.normal.iter().all(|component| component.is_finite())
    }));
    assert!(cylinder_vertices.iter().all(|vertex| {
        vertex
            .position
            .iter()
            .all(|component| component.is_finite())
            && vertex.normal.iter().all(|component| component.is_finite())
    }));
}

#[test]
fn cursor_shaders_keep_surface_light_and_depth_independent_volume_separate() {
    let feedback_shader = include_str!("../shaders/sculpt_feedback.wgsl");
    let tool_shader = include_str!("../shaders/sculpt_tool.wgsl");

    assert!(feedback_shader.contains("@group(3) @binding(0) var<uniform> sculpt_brush"));
    assert!(feedback_shader.contains("fn fs_sculpt_feedback"));
    assert!(feedback_shader.contains("sculpt_brush.color.rgb * sculpt_brush.intensity"));
    assert!(tool_shader.contains("fn vs_main"));
    assert!(tool_shader.contains("if tool.visible == 0u"));
    assert!(tool_shader.contains("struct SculptToolUniform"));
}
