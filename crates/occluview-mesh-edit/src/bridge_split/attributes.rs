use glam::{DVec2, DVec3};

use crate::EditVertex;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct VertexAttributeKey {
    normal: [u32; 3],
    color: [u8; 4],
    uv: [u32; 2],
}

impl From<EditVertex> for VertexAttributeKey {
    fn from(vertex: EditVertex) -> Self {
        Self {
            normal: vertex.normal.map(f32::to_bits),
            color: vertex.color,
            uv: vertex.uv.map(f32::to_bits),
        }
    }
}

pub(crate) fn interpolate_vertex(
    first: EditVertex,
    second: EditVertex,
    t: f64,
    position: DVec3,
) -> EditVertex {
    let t = t.clamp(0.0, 1.0);
    let first_normal = DVec3::from_array(first.normal.map(f64::from));
    let second_normal = DVec3::from_array(second.normal.map(f64::from));
    let blended_normal = first_normal.lerp(second_normal, t);
    let normal = if blended_normal.length_squared() > f64::EPSILON {
        blended_normal.normalize()
    } else {
        DVec3::ZERO
    };
    let first_uv = DVec2::from_array(first.uv.map(f64::from));
    let second_uv = DVec2::from_array(second.uv.map(f64::from));
    let uv = first_uv.lerp(second_uv, t);
    let mut color = [0_u8; 4];
    for (channel, output) in color.iter_mut().enumerate() {
        let value = f64::from(first.color[channel])
            + (f64::from(second.color[channel]) - f64::from(first.color[channel])) * t;
        *output = rounded_u8(value);
    }

    EditVertex {
        position: position.as_vec3().to_array(),
        normal: normal.as_vec3().to_array(),
        color,
        uv: uv.as_vec2().to_array(),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn rounded_u8(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}
