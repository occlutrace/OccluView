use super::sculpt_cursor::{
    cone_geometry, cylinder_geometry, sculpt_footprint_field, sculpt_surface_light_intensity,
    sculpt_tool_length, SculptBrushUniform, SculptTipStamp, SculptToolShape, SculptToolUniform,
    SCULPT_CYLINDER_PLATEAU, SCULPT_KNIFE_CROSS_SHARE,
};
use std::mem::size_of;

#[test]
fn cursor_uniforms_have_the_wgsl_alignment_their_bindings_require() {
    assert_eq!(size_of::<SculptBrushUniform>(), 80);
    assert_eq!(size_of::<SculptToolUniform>(), 96);
}

/// A ball dab is brightest at the centre, fades monotonically and stops at the
/// radius, so the mark cannot read as a hard-edged disc.
#[test]
fn ball_footprint_fades_from_the_centre_to_the_rim() {
    let field = |radius_share: f32| {
        sculpt_footprint_field(
            SculptTipStamp::Ball,
            [radius_share, 0.0, 0.0],
            1.0,
            [0.0; 3],
        )
    };
    assert_eq!(field(0.0), 1.0);
    assert!(field(0.4) > field(0.7));
    assert!(field(0.7) > field(0.99));
    assert!(field(0.99) > 0.0);
    assert_eq!(field(1.0), 0.0);
    assert_eq!(field(1.5), 0.0);
}

/// A knife reaches further along its bearing than across it: at 0.6 of the
/// radius it still cuts along the axis and has already ended across it.
#[test]
fn knife_footprint_is_longer_along_its_axis_than_across() {
    let axis = [1.0, 0.0, 0.0];
    let along = sculpt_footprint_field(SculptTipStamp::Knife, [0.6, 0.0, 0.0], 1.0, axis);
    let across = sculpt_footprint_field(SculptTipStamp::Knife, [0.0, 0.6, 0.0], 1.0, axis);
    assert!(along > 0.0, "the blade must still cut along its bearing");
    assert_eq!(across, 0.0, "the blade is narrower than 0.6 r across");
    assert!(across < along);
    // The transverse reach is the documented share of the along reach.
    assert!(sculpt_footprint_field(SculptTipStamp::Knife, [0.0, 0.5, 0.0], 1.0, axis) > 0.0);
    assert_eq!(
        sculpt_footprint_field(
            SculptTipStamp::Knife,
            [0.0, SCULPT_KNIFE_CROSS_SHARE + 0.01, 0.0],
            1.0,
            axis
        ),
        0.0
    );
    // A press with no bearing falls back to a narrow radial footprint: it
    // reaches less far than the ball in every direction and ends before 0.8 r.
    let no_axis = sculpt_footprint_field(SculptTipStamp::Knife, [0.0, 0.6, 0.0], 1.0, [0.0; 3]);
    let ball = sculpt_footprint_field(SculptTipStamp::Ball, [0.0, 0.6, 0.0], 1.0, [0.0; 3]);
    assert!(
        no_axis > 0.0 && no_axis < ball,
        "got {no_axis} against {ball}"
    );
    assert_eq!(
        sculpt_footprint_field(SculptTipStamp::Knife, [0.8, 0.0, 0.0], 1.0, [0.0; 3]),
        0.0
    );
    assert_eq!(
        sculpt_footprint_field(SculptTipStamp::Knife, [0.0, 0.2, 0.0], 1.0, [0.0; 3]),
        sculpt_footprint_field(SculptTipStamp::Knife, [0.2, 0.0, 0.0], 1.0, [0.0; 3])
    );
}

/// A cylinder is flat inside its plateau and only softens over the rim band,
/// which is what makes it level a face instead of raising a mound.
#[test]
fn cylinder_footprint_is_a_plateau_with_a_soft_rim() {
    let field = |radius_share: f32| {
        sculpt_footprint_field(
            SculptTipStamp::Cylinder,
            [radius_share, 0.0, 0.0],
            1.0,
            [0.0; 3],
        )
    };
    assert_eq!(field(0.0), 1.0);
    assert_eq!(field(SCULPT_CYLINDER_PLATEAU * 0.5), 1.0);
    assert_eq!(field(SCULPT_CYLINDER_PLATEAU), 1.0);
    let rim = field(SCULPT_CYLINDER_PLATEAU + (1.0 - SCULPT_CYLINDER_PLATEAU) * 0.5);
    assert!(rim > 0.0 && rim < 1.0, "the rim must blend, got {rim}");
    assert_eq!(field(1.0), 0.0);
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
