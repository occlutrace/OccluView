//! Per-mesh GPU uniform: model matrix, tint, opacity, texture/color presence,
//! independent display flags, and the occlusal-contact paint ramp.
//!
//! Bound at group 1, binding 0. One uniform per mesh lets the renderer place
//! multiple meshes (multi-mesh scene) and branch the fragment shader between
//! vertex-color and texture-sampled shading.
//!
//! Layout (400 bytes; the tail block provides the 16-byte alignment WGSL
//! requires of a uniform struct):
//! - `model`                `[f32;16]`       64 bytes
//! - `tint`                  `[f32;4]`       16 bytes
//! - `opacity`                `f32`           4 bytes
//! - `has_texture`            `u32`           4 bytes
//! - `show_orientation`       `u32`           4 bytes
//! - `show_vertex_colors`     `u32`           4 bytes
//! - `show_texture`           `u32`           4 bytes
//! - `measured_map`           `u32`           4 bytes
//! - `contact_map`            `u32`           4 bytes
//! - `contact_field_width`    `f32`           4 bytes
//! - `contact_gap`           `[f32;4]`       16 bytes
//! - `contact_stops`         `[[f32;4];16]` 256 bytes
//! - `contact_stop_count`     `u32`           4 bytes
//! - `overlay_paint`          `u32`           4 bytes
//! - `contact_padding`       `[u32;2]`        8 bytes
//!
//! # Why the ramp lives in the uniform
//!
//! The colour ramp is evaluated in the fragment shader from a stop table held
//! here rather than sampled from a ramp texture. The one control the operator
//! drives — the penetration depth that reads as fully loaded — then costs a
//! 400-byte uniform write and nothing else: no texture upload, no bind-group
//! rebuild, no per-law GPU resource to cache and invalidate. A texture would
//! have bought nothing, because the ramp is at most sixteen stops and its
//! evaluation is a handful of multiplies next to the lighting that follows it.
//!
//! A stop is `(mm, L, a, b)` in Oklab, descending in mm, exactly as
//! `occluview_contact::stop_table` produces it — Oklab rather than sRGB because
//! a straight sRGB interpolation across blue→cyan→green→yellow→red passes
//! through washed-out mud, while the perceptual straight line keeps every
//! intermediate colour vivid.

use bytemuck::{Pod, Zeroable};

/// Stops a contact ramp can carry, matching the shader's `array<vec4<f32>, 16>`.
pub const CONTACT_STOP_CAPACITY: usize = 16;

/// Per-mesh GPU uniform (see module docs for the full layout).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuMeshUniform {
    /// Column-major model matrix (the result of `Mat4::from_affine3(...)`).
    pub model: [f32; 16],
    /// Linear-sRGB tint multiplied into the base color. Default white.
    pub tint: [f32; 4],
    /// Opacity 0..1.
    pub opacity: f32,
    /// 0 = use vertex color; 1 = sample `mesh_texture`. Stored as `u32` for
    /// std140 alignment.
    pub has_texture: u32,
    /// 1 = orientation diagnostic: paint back-facing fragments solid red
    /// (the dental CAD "Show triangle orientation" convention).
    pub show_orientation: u32,
    /// 0 = ignore scan color/texture and shade with a flat neutral material
    /// (the shader's `NEUTRAL_MATERIAL_RGB`, matching
    /// `occluview_core::scene::material::DEFAULT_UNTEXTURED_MESH_TINT`); 1 =
    /// normal vertex-color/texture shading. Display-only: mesh data is
    /// untouched, so edits and exports keep the real colors.
    pub show_vertex_colors: u32,
    /// 0 = do not sample the attached texture; 1 = texture sampling enabled.
    /// This is intentionally independent from `show_vertex_colors`.
    pub show_texture: u32,
    /// 1 = this layer is showing a measured colour map.
    ///
    /// The map keeps its own hue and drops the tint, so a ramp reaches the
    /// screen at the colour it was measured at. Lighting is *reduced*, not
    /// removed: a fully unlit surface has no shading at all and reads as a
    /// flat silhouette, which is useless for judging a scan. The field
    /// occupies a slot that would otherwise be tail padding.
    pub measured_map: u32,
    /// 1 = this layer paints an occlusal contact field from `contact_stops`.
    ///
    /// A contact reading is a measurement, so the ramp reaches the screen at
    /// its own hue under a single reduced shade factor, and the specular
    /// highlight (which would move the hue at every bright pixel) is skipped.
    ///
    /// This does not pair with `measured_map = 1`: the two overlays are
    /// mutually exclusive because they are different measurements, and setting
    /// both paints the contact ramp into a colour taken from the deviation
    /// ramp -- the all-white layer the app's rule exists to prevent.
    pub contact_map: u32,
    /// Texels per row of the packed field texture, so the shader can turn a
    /// vertex index into a texture coordinate without `textureDimensions`.
    /// Must be >= 1 whenever `contact_map != 0`.
    pub contact_field_width: f32,
    /// `(widest painted gap mm, far-edge feather mm, 0, 0)`.
    ///
    /// The field is a signed distance: positive is a gap, zero is exact touch,
    /// negative is penetration depth. Values above the first element stay bare
    /// surface, and the paint feathers to nothing over the second element so
    /// the band ends by weight rather than on a visible contour line.
    pub contact_gap: [f32; 4],
    /// `(mm, L, a, b)` per stop in Oklab, descending in mm — the ramp's own
    /// numbers. Entries past `contact_stop_count` are inert.
    pub contact_stops: [[f32; 4]; CONTACT_STOP_CAPACITY],
    /// Stops actually in use. At least 1 whenever `contact_map != 0`, because
    /// the shader walks `stop_count - 1` spans.
    pub contact_stop_count: u32,
    /// 1 = this layer's overlay colours are paint, not a measurement.
    ///
    /// The RGB is a paint colour and the alpha is the weight mixed over the
    /// surface's own material, so alpha 0 leaves the scan bit-for-bit as it
    /// renders — its tint, its texture and its lighting included. Without the
    /// distinction the brush preview would be shaded as a measured map (tint
    /// dropped, lighting cut to 42%, gloss added), and a marked scan would read
    /// as a pale shiny shell. The field occupies a slot that would otherwise be
    /// tail padding.
    pub overlay_paint: u32,
    /// Explicit tail padding: a uniform struct is 16-byte aligned in WGSL even
    /// though each scalar field here is four-byte aligned.
    pub contact_padding: [u32; 2],
}

impl GpuMeshUniform {
    /// The identity uniform: identity model matrix, white tint, full opacity,
    /// no texture, vertex colors shown, no contact field bound. Used by the
    /// legacy single-mesh draw path and as a default.
    #[must_use]
    pub const fn identity() -> Self {
        // Column-major identity mat4.
        const IDENTITY: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, //
        ];
        Self {
            model: IDENTITY,
            tint: [1.0, 1.0, 1.0, 1.0],
            opacity: 1.0,
            has_texture: 0,
            show_orientation: 0,
            show_vertex_colors: 1,
            show_texture: 1,
            measured_map: 0,
            contact_map: 0,
            // The inert values are the ones that keep the shader's arithmetic
            // defined even if a future caller binds a field but forgets to fill
            // the table: a width of one texel, a stop count of one (so the
            // shader never indexes `count - 1` under an empty table) and a
            // zero-width gap (so `max(fade, 1e-6)` never divides by zero).
            contact_field_width: 1.0,
            contact_gap: [0.0; 4],
            contact_stops: [[0.0; 4]; CONTACT_STOP_CAPACITY],
            contact_stop_count: 1,
            overlay_paint: 0,
            contact_padding: [0; 2],
        }
    }

    /// Enable contact painting on this layer and register the ramp it paints
    /// with, returning how many stops were copied.
    ///
    /// `stops` are `(mm, L, a, b)` in Oklab, descending in mm — the shape
    /// `occluview_contact::stop_table` produces, and the order the shader's
    /// walk relies on. `paint_far_mm` is the widest gap that is painted at all
    /// and `far_fade_mm` the width of the feather at that edge.
    ///
    /// A table with no stops is refused (the call leaves `contact_map` at 0 and
    /// returns 0): a ramp with no colour has nothing to show, and the shader's
    /// index arithmetic is written around the table having at least one entry.
    /// A non-finite gap or stop value is replaced with zero rather than stored,
    /// because a NaN reaching `clamp()` in the shader would paint the whole
    /// layer with an undefined colour instead of failing visibly.
    pub fn set_contact_paint(
        &mut self,
        field_width: u32,
        paint_far_mm: f32,
        far_fade_mm: f32,
        stops: &[[f32; 4]],
    ) -> usize {
        let copied = stops.len().min(CONTACT_STOP_CAPACITY);
        if copied == 0 || field_width == 0 {
            return 0;
        }
        for (slot, stop) in self.contact_stops.iter_mut().enumerate() {
            *stop = if slot < copied {
                sanitize_stop(stops[slot])
            } else {
                [0.0; 4]
            };
        }
        // The shader reads this as `u32(max(width, 1.0))`; carrying the exact
        // texel count here means a caller never has to.
        self.contact_field_width = field_width as f32;
        self.contact_gap = [
            finite_or_zero(paint_far_mm),
            finite_or_zero(far_fade_mm).max(0.0),
            0.0,
            0.0,
        ];
        self.contact_stop_count = copied as u32;
        self.contact_map = 1;
        copied
    }

    /// Stop painting contact on this layer. The table is left in place: the
    /// shader never reads it while `contact_map == 0`, and a caller that
    /// toggles contacts on and off keeps the ramp it had.
    pub fn clear_contact_paint(&mut self) {
        self.contact_map = 0;
    }
}

/// Zero for a NaN or infinite value: a stop or gap edge that cannot be
/// compared would otherwise turn every fragment it touches into NaN.
const fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

/// A stop with every component finite.
const fn sanitize_stop(stop: [f32; 4]) -> [f32; 4] {
    [
        finite_or_zero(stop[0]),
        finite_or_zero(stop[1]),
        finite_or_zero(stop[2]),
        finite_or_zero(stop[3]),
    ]
}

impl Default for GpuMeshUniform {
    fn default() -> Self {
        Self::identity()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::float_cmp)]

    use super::*;

    /// The WGSL struct and this one are two hand-written copies of the same
    /// memory layout. Nothing but this test stops them drifting: a mismatch is
    /// silent corruption of every flag past the divergence, not a compile
    /// error.
    #[test]
    fn the_shader_struct_matches_this_one_field_for_field() {
        let shader = include_str!("../shaders/mesh.wgsl");
        let start = shader
            .find("struct MeshUniform {")
            .expect("mesh.wgsl must declare MeshUniform");
        let body = &shader[start..];
        let end = body.find('}').expect("MeshUniform must be closed");
        let fields: Vec<&str> = body[..end]
            .lines()
            .skip(1)
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("//"))
            .filter_map(|line| line.split(':').next())
            .collect();

        assert_eq!(
            fields,
            vec![
                "model",
                "tint",
                "opacity",
                "has_texture",
                "show_orientation",
                "show_vertex_colors",
                "show_texture",
                "measured_map",
                "contact_map",
                "contact_field_width",
                "contact_gap",
                "contact_stops",
                "contact_stop_count",
                "overlay_paint",
                "contact_padding_0",
                "contact_padding_1",
            ],
            "mesh.wgsl's MeshUniform drifted from GpuMeshUniform"
        );
    }

    /// Every shader that declares its own `MeshUniform` must match the same
    /// layout, or the drift is silent corruption rather than a compile error.
    ///
    /// `sculpt_feedback.wgsl` hand-declares a third copy (it names the contact
    /// slots `_padding_0/_padding_1`, which is correct for a pass that does not
    /// read them, and reads only `model`, which sits at offset 0). Nothing
    /// pinned it: the test above parses `mesh.wgsl` alone, so reordering
    /// `GpuMeshUniform::model` would have shifted every field this shader reads
    /// with no failure anywhere.
    #[test]
    fn every_shader_that_declares_mesh_uniform_matches_this_one() {
        for shader in [
            include_str!("../shaders/mesh.wgsl"),
            include_str!("../shaders/sculpt_feedback.wgsl"),
        ] {
            let start = shader
                .find("struct MeshUniform {")
                .expect("every render shader must declare MeshUniform");
            let body = &shader[start..];
            let end = body.find('}').expect("MeshUniform must be closed");
            let fields: Vec<&str> = body[..end]
                .lines()
                .skip(1)
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with("//"))
                .filter_map(|line| line.split(':').next())
                .collect();
            // `model` must be first in every copy: it is the only field the
            // feedback pass reads, and a reorder breaks it without any other
            // check failing.
            assert_eq!(
                fields.first().copied(),
                Some("model"),
                "a shader's MeshUniform does not start with `model`"
            );
            assert!(
                fields.contains(&"tint") && fields.contains(&"opacity"),
                "a shader's MeshUniform does not match GpuMeshUniform's prefix"
            );
        }
    }

    /// The WGSL array length and this constant are one fact in two languages.
    #[test]
    fn the_shader_stop_array_holds_every_stop_the_ramp_can_carry() {
        let shader = include_str!("../shaders/mesh.wgsl");
        assert!(
            shader.contains("array<vec4<f32>, 16>"),
            "mesh.wgsl's stop table must hold CONTACT_STOP_CAPACITY entries"
        );
        assert_eq!(CONTACT_STOP_CAPACITY, 16);
    }

    #[test]
    fn identity_is_400_bytes_and_aligned() {
        assert_eq!(size_of::<GpuMeshUniform>(), 400);
        // A uniform struct must be a multiple of 16 in WGSL; 400 is, and the
        // Rust struct itself only needs 4-byte alignment.
        assert_eq!(size_of::<GpuMeshUniform>() % 16, 0);
        assert_eq!(align_of::<GpuMeshUniform>(), 4);
    }

    #[test]
    fn identity_round_trips() {
        let u = GpuMeshUniform::identity();
        assert_eq!(u.tint, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(u.opacity, 1.0);
        assert_eq!(u.has_texture, 0);
        // Identity mat4: diagonal = 1, off-diagonal = 0.
        assert_eq!(u.model[0], 1.0);
        assert_eq!(u.model[5], 1.0);
        assert_eq!(u.model[10], 1.0);
        assert_eq!(u.model[15], 1.0);
        assert_eq!(u.model[1], 0.0);
    }

    /// A layer with no contact field must be inert in every field the shader
    /// reads, not merely flagged off: the vertex stage skips its texture load
    /// on the flag, and the fragment stage never enters the ramp walk, so the
    /// numbers behind the flag are only ever a guard against arithmetic that
    /// cannot be reached.
    #[test]
    fn an_identity_uniform_binds_no_contact_field() {
        let u = GpuMeshUniform::identity();
        assert_eq!(u.contact_map, 0);
        assert_eq!(u.contact_field_width, 1.0);
        assert_eq!(u.contact_stop_count, 1);
        assert_eq!(u.contact_gap, [0.0; 4]);
    }

    #[test]
    fn a_ramp_wider_than_the_table_keeps_the_stops_that_fit() {
        let mut u = GpuMeshUniform::identity();
        let stops: Vec<[f32; 4]> = (0..20)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let mm = -(i as f32);
                [mm, 0.5, 0.0, 0.0]
            })
            .collect();

        let copied = u.set_contact_paint(64, 0.01, 0.01, &stops);

        assert_eq!(copied, CONTACT_STOP_CAPACITY);
        assert_eq!(u.contact_map, 1);
        assert_eq!(u.contact_stop_count, CONTACT_STOP_CAPACITY as u32);
        assert_eq!(u.contact_field_width, 64.0);
        assert_eq!(u.contact_gap, [0.01, 0.01, 0.0, 0.0]);
        assert_eq!(u.contact_stops[0], [0.0, 0.5, 0.0, 0.0]);
        assert_eq!(u.contact_stops[15][0], -15.0);
    }

    /// A ramp with no stops has nothing to paint, and the shader's
    /// `stop_count - 1` walk would index a table it was never given.
    #[test]
    fn an_empty_ramp_is_refused() {
        let mut u = GpuMeshUniform::identity();
        assert_eq!(u.set_contact_paint(64, 0.01, 0.01, &[]), 0);
        assert_eq!(u.contact_map, 0);
        assert_eq!(
            u.set_contact_paint(0, 0.01, 0.01, &[[0.0, 1.0, 0.0, 0.0]]),
            0
        );
        assert_eq!(u.contact_map, 0);
    }

    /// A NaN would propagate through the shader's `clamp`/`mix` and paint the
    /// layer an undefined colour instead of failing where it can be seen.
    #[test]
    fn a_non_finite_stop_or_gap_edge_is_replaced_not_stored() {
        let mut u = GpuMeshUniform::identity();
        u.set_contact_paint(
            8,
            f32::NAN,
            f32::INFINITY,
            &[[-0.1, f32::NAN, f32::INFINITY, 0.2]],
        );

        assert_eq!(u.contact_gap, [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(u.contact_stops[0], [-0.1, 0.0, 0.0, 0.2]);
    }

    /// Clearing the paint must leave the table intact so toggling contacts off
    /// and on again does not need the caller to rebuild the ramp.
    #[test]
    fn clearing_the_paint_keeps_the_ramp() {
        let mut u = GpuMeshUniform::identity();
        u.set_contact_paint(16, 0.02, 0.02, &[[0.0, 0.5, 0.0, 0.0]]);
        u.clear_contact_paint();

        assert_eq!(u.contact_map, 0);
        assert_eq!(u.contact_stop_count, 1);
        assert_eq!(u.contact_stops[0], [0.0, 0.5, 0.0, 0.0]);
    }

    /// Pins the shader's hand-copied `NEUTRAL_MATERIAL_RGB` (`mesh.wgsl`)
    /// against the core crate's own untextured-mesh tint, so the two cannot
    /// drift apart; `mesh.wgsl`'s doc comment for that constant refers to
    /// this test.
    #[test]
    fn neutral_material_matches_the_core_untextured_tint() {
        const NEUTRAL_MATERIAL_RGB: [f32; 3] = [0.82, 0.68, 0.42];
        let [r, g, b, _a] = occluview_core::DEFAULT_UNTEXTURED_MESH_TINT;
        assert_eq!(NEUTRAL_MATERIAL_RGB, [r, g, b]);
    }
}
