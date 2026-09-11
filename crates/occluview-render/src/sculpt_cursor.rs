//! Display-only GPU state for the live Sculpt brush cursor.
//!
//! The cursor has two independent pieces: a small emissive field projected
//! onto the picked surface and a translucent tool volume hovering just above
//! it. Neither piece participates in picking, dab scheduling, or mesh writes.
//! The app still owns the one authoritative surface hit; this module only
//! carries finite GPU inputs and the canonical cone/cylinder geometry.

use bytemuck::{Pod, Zeroable};
use std::f32::consts::TAU;

/// The two tool-volume shapes used by the native Sculpt editor.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SculptToolShape {
    /// Add/Remove uses the tapered Freeforming cone.
    Cone = 0,
    /// Smooth uses a short cylindrical plateau.
    Cylinder = 1,
}

/// Surface-light input. The layout is shared by Rust and
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
    /// Reference cursor light intensity, including a non-zero idle glow.
    pub intensity: f32,
    /// Linear-sRGB display color. Alpha is reserved for the tool volume.
    pub color: [f32; 4],
    /// `0` = cone field, `1` = cylinder/plateau field.
    pub tip: u32,
    /// `1` while the cursor is valid; `0` is a no-op.
    pub visible: u32,
    /// Explicit uniform tail padding.
    pub padding: [u32; 2],
}

impl SculptBrushUniform {
    /// A safe no-op value used whenever the pointer has no valid surface hit.
    #[must_use]
    pub const fn hidden() -> Self {
        Self {
            center: [0.0; 3],
            radius: 1.0,
            normal: [0.0, 0.0, 1.0],
            intensity: 0.0,
            color: [0.0; 4],
            tip: SculptToolShape::Cone as u32,
            visible: 0,
            padding: [0; 2],
        }
    }
}

/// Per-volume transform and material. The transform maps the unit shape
/// (radius 1, length 1, +Z axis) into world space.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SculptToolUniform {
    /// World transform for the canonical cone/cylinder.
    pub model: [f32; 16],
    /// Linear-sRGB volume color.
    pub color: [f32; 4],
    /// Source alpha for the translucent volume.
    pub opacity: f32,
    /// `SculptToolShape` tag selecting the geometry buffer.
    pub shape: u32,
    /// `1` while the volume is valid; `0` is a no-op.
    pub visible: u32,
    /// Explicit uniform tail padding.
    pub padding: u32,
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
            visible: 0,
            padding: 0,
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

/// Build an open-sided cone. Its base is z=0 and its tip is z=1, matching the
/// reference cursor's transparent `ConeGeometry(..., openEnded=true)`.
pub(crate) fn cone_geometry() -> (Vec<SculptToolVertex>, Vec<u32>) {
    let mut vertices = Vec::with_capacity(SEGMENTS * 3);
    let mut indices = Vec::with_capacity(SEGMENTS * 3);
    for segment in 0..SEGMENTS {
        let next = (segment + 1) % SEGMENTS;
        let (x0, y0) = ring_point(segment);
        let (x1, y1) = ring_point(next);
        let n0 = cone_normal(x0, y0);
        let n1 = cone_normal(x1, y1);
        let start = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
        vertices.extend_from_slice(&[
            SculptToolVertex {
                position: [x0, y0, 0.0],
                normal: n0,
            },
            SculptToolVertex {
                position: [x1, y1, 0.0],
                normal: n1,
            },
            SculptToolVertex {
                position: [0.0, 0.0, 1.0],
                normal: cone_normal(x0.midpoint(x1), y0.midpoint(y1)),
            },
        ]);
        indices.extend_from_slice(&[start, start + 1, start + 2]);
    }
    (vertices, indices)
}

/// Build an open-sided cylinder/plateau. Its caps stay open so the surface
/// light and the existing ring communicate the actual contact patch instead
/// of hiding it behind an opaque disc.
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

fn cone_normal(x: f32, y: f32) -> [f32; 3] {
    let length = (x * x + y * y + 1.0).sqrt().max(f32::EPSILON);
    [x / length, y / length, 1.0 / length]
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

/// Reference Freeforming cone length, bounded against invalid UI input.
#[must_use]
pub fn sculpt_tool_length(strength: f32) -> f32 {
    const MIN: f32 = 0.65;
    const MAX: f32 = 2.60;
    MIN + (MAX - MIN) * normalized_strength(strength)
}

/// Reference low-power surface glow. The non-zero floor keeps the cursor
/// visible at low force while the volume still communicates the main shape.
#[must_use]
pub fn sculpt_surface_light_intensity(strength: f32) -> f32 {
    0.035 + 0.14 * normalized_strength(strength).powf(1.2)
}
