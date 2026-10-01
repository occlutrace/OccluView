//! Display-only GPU state for the live Sculpt brush cursor.
//!
//! The cursor has two independent pieces: a colored footprint projected
//! onto the picked surface and a translucent tool volume hovering just above
//! it. Neither piece participates in picking, dab scheduling, or mesh writes.
//! The app still owns the one authoritative surface hit; this module only
//! carries finite GPU inputs and the unit tool geometry.

use bytemuck::{Pod, Zeroable};
use glam::DVec3;
use occluview_geometry_math::{
    stamp_weight, CYLINDER_PLATEAU, KNIFE_AXIS_MIN_LENGTH, KNIFE_CROSS_RADIUS_SHARE,
};
use std::f32::consts::TAU;

pub use occluview_geometry_math::TipStamp as SculptTipStamp;

/// Cylinder plateau quantized for the GPU's f32 field inputs.
#[allow(clippy::cast_possible_truncation)]
pub const SCULPT_CYLINDER_PLATEAU: f32 = CYLINDER_PLATEAU as f32;
/// Knife transverse reach quantized for the GPU's f32 field inputs.
#[allow(clippy::cast_possible_truncation)]
pub const SCULPT_KNIFE_CROSS_SHARE: f32 = KNIFE_CROSS_RADIUS_SHARE as f32;

pub(crate) const SCULPT_FEEDBACK_SHADER_SRC: &str = concat!(
    include_str!("../shaders/sculpt_field.wgsl"),
    include_str!("../shaders/sculpt_feedback.wgsl")
);

/// The three tool-volume shapes used by the native Sculpt editor.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SculptToolShape {
    /// Ball uses the full-radius profile.
    Cone = 0,
    /// Cylinder uses the full-radius straight profile.
    Cylinder = 1,
    /// Knife uses the narrow profile.
    Knife = 2,
}

/// Outline treatment at the footprint edge.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SculptFeedbackStyle {
    /// One continuous hairline.
    Solid = 0,
    /// A broken hairline for a non-depositing pass.
    Dashed = 1,
}

/// Surface-tint input. The layout is shared by Rust and
/// `sculpt_feedback.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SculptBrushUniform {
    /// World-space center of the current surface hit.
    pub center: [f32; 3],
    /// World-space brush radius.
    pub radius: f32,
    /// World-space surface normal, oriented toward the camera.
    pub normal: [f32; 3],
    /// Surface tint intensity, including a non-zero idle footprint.
    pub intensity: f32,
    /// World-space stroke bearing the knife footprint elongates along. Zero
    /// leaves the knife a narrow radial footprint.
    pub axis: [f32; 3],
    /// Brush tip stamp: `0` ball, `1` knife, `2` cylinder. Matches
    /// `occluview_sculpt::TipStamp`.
    pub tip: u32,
    /// Linear-sRGB display color. Alpha is reserved for the tool volume.
    pub color: [f32; 4],
    /// `1` while the cursor is valid; `0` is a no-op.
    pub visible: u32,
    /// Transverse knife reach from the shared sculpt field contract.
    pub knife_cross_share: f32,
    /// Cylinder plateau width from the shared sculpt field contract.
    pub cylinder_plateau: f32,
    /// Minimum stroke-axis length from the shared sculpt field contract.
    pub knife_axis_min_length: f32,
    /// Footprint outline treatment.
    pub edge_style: u32,
    /// Explicit scalar tail padding.
    pub padding_0: u32,
    /// Explicit scalar tail padding.
    pub padding_1: u32,
    /// Explicit scalar tail padding.
    pub padding_2: u32,
}

impl SculptBrushUniform {
    /// A safe no-op value used whenever the pointer has no valid surface hit.
    // Shared field math stays f64; only the display uniform is quantized.
    #[allow(clippy::cast_possible_truncation)]
    #[must_use]
    pub const fn hidden() -> Self {
        Self {
            center: [0.0; 3],
            radius: 1.0,
            normal: [0.0, 0.0, 1.0],
            intensity: 0.0,
            axis: [0.0; 3],
            tip: SculptTipStamp::Ball as u32,
            color: [0.0; 4],
            visible: 0,
            knife_cross_share: SCULPT_KNIFE_CROSS_SHARE,
            cylinder_plateau: SCULPT_CYLINDER_PLATEAU,
            knife_axis_min_length: KNIFE_AXIS_MIN_LENGTH as f32,
            edge_style: SculptFeedbackStyle::Solid as u32,
            padding_0: 0,
            padding_1: 0,
            padding_2: 0,
        }
    }
}

/// Evaluate the shared brush field for cursor geometry and field assertions.
#[allow(clippy::cast_possible_truncation)]
#[must_use]
pub fn sculpt_footprint_field(
    tip: SculptTipStamp,
    offset: [f32; 3],
    radius: f32,
    axis: [f32; 3],
) -> f32 {
    if !radius.is_finite() || radius <= 0.0 {
        return 0.0;
    }
    let offset = DVec3::new(
        f64::from(offset[0]),
        f64::from(offset[1]),
        f64::from(offset[2]),
    );
    let axis = DVec3::new(f64::from(axis[0]), f64::from(axis[1]), f64::from(axis[2]));
    let axis = (axis.is_finite() && axis.length() > KNIFE_AXIS_MIN_LENGTH).then_some(axis);
    stamp_weight(tip, offset, offset.length(), axis, f64::from(radius)) as f32
}

/// Per-volume transform and material. The transform maps the unit shape
/// (radius 1, length 1, +Z axis) into world space.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SculptToolUniform {
    /// World transform for the unit tool body.
    pub model: [f32; 16],
    /// Linear-sRGB volume color.
    pub color: [f32; 4],
    /// Source alpha for the translucent volume.
    pub opacity: f32,
    /// `SculptToolShape` tag selecting the tip profile.
    pub shape: u32,
    /// Morph weights for pressing and flattening the tip profile.
    pub action: [f32; 2],
}

impl SculptToolUniform {
    /// A safe no-op value used when the surface cursor is hidden.
    #[must_use]
    pub const fn hidden() -> Self {
        Self {
            model: IDENTITY,
            color: [0.0; 4],
            opacity: 0.0,
            shape: SculptToolShape::Cone as u32,
            action: [0.0; 2],
        }
    }
}

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, // column 0
    0.0, 1.0, 0.0, 0.0, // column 1
    0.0, 0.0, 1.0, 0.0, // column 2
    0.0, 0.0, 0.0, 1.0, // column 3
];

/// Canonical vertex used by the translucent tool volume.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct SculptToolVertex {
    pub(crate) position: [f32; 3],
    pub(crate) normal: [f32; 3],
}

/// Vertex layout for the tool-volume shader.
pub(crate) fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: size_of::<SculptToolVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 12,
                shader_location: 1,
            },
        ],
    }
}

/// Build an open-sided unit cylinder; the shader shapes it for the active tip
/// and modifier state while its caps leave the contact patch visible.
pub(crate) fn cylinder_geometry() -> (Vec<SculptToolVertex>, Vec<u32>) {
    let mut vertices = Vec::with_capacity(SEGMENTS * 4);
    let mut indices = Vec::with_capacity(SEGMENTS * 6);
    for segment in 0..SEGMENTS {
        let next = (segment + 1) % SEGMENTS;
        let (x0, y0) = ring_point(segment);
        let (x1, y1) = ring_point(next);
        let start = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
        vertices.extend_from_slice(&[
            SculptToolVertex {
                position: [x0, y0, 0.0],
                normal: [x0, y0, 0.0],
            },
            SculptToolVertex {
                position: [x1, y1, 0.0],
                normal: [x1, y1, 0.0],
            },
            SculptToolVertex {
                position: [x1, y1, 1.0],
                normal: [x1, y1, 0.0],
            },
            SculptToolVertex {
                position: [x0, y0, 1.0],
                normal: [x0, y0, 0.0],
            },
        ]);
        indices.extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
    }
    (vertices, indices)
}

const SEGMENTS: usize = 32;

fn ring_point(segment: usize) -> (f32, f32) {
    let angle = TAU * segment as f32 / SEGMENTS as f32;
    (angle.cos(), angle.sin())
}

fn normalized_strength(strength: f32) -> f32 {
    if strength.is_nan() {
        0.0
    } else if strength.is_infinite() {
        if strength.is_sign_positive() {
            1.0
        } else {
            0.0
        }
    } else {
        strength.clamp(0.0, 1.0)
    }
}

/// Tool height, bounded against invalid UI input.
#[must_use]
pub fn sculpt_tool_length(strength: f32) -> f32 {
    const MIN: f32 = 0.65;
    const MAX: f32 = 2.60;
    MIN + (MAX - MIN) * normalized_strength(strength)
}

/// Low-power surface glow. The non-zero floor keeps the cursor
/// visible at low force while the volume still communicates the main shape.
#[must_use]
pub fn sculpt_surface_light_intensity(strength: f32) -> f32 {
    0.035 + 0.14 * normalized_strength(strength).powf(1.2)
}
