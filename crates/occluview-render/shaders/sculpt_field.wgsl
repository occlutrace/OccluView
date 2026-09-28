fn sculpt_brush_field(
    tip: u32,
    offset: vec3<f32>,
    radius: f32,
    axis_input: vec3<f32>,
    knife_cross_share: f32,
    cylinder_plateau: f32,
    knife_axis_min_length: f32,
) -> f32 {
    if !(radius > 0.0) {
        return 0.0;
    }
    let distance = length(offset);
    if tip == 1u {
        let axis_length = length(axis_input);
        if axis_length > knife_axis_min_length {
            let axis = axis_input / axis_length;
            let along = dot(offset, axis);
            let across = length(offset - axis * along);
            let edge = length(vec2<f32>(along, across / knife_cross_share)) / radius;
            let share = max(0.0, 1.0 - edge);
            return share * share;
        }
        let press_radius = radius * sqrt(knife_cross_share);
        let share = max(0.0, 1.0 - distance / press_radius);
        return share * share;
    }
    if tip == 2u {
        return 1.0 - smoothstep(cylinder_plateau, 1.0, distance / radius);
    }
    let share = max(0.0, 1.0 - distance / radius);
    return share * share;
}
