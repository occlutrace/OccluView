//! Turning a finished contact reading into the per-mesh uniform the viewport
//! draws.
//!
//! Two independent facts have to reach the GPU before a layer can paint a
//! contact map: the packed field (bound in group 2, which the prepared scene
//! owns) and the ramp the field is read through. The ramp travels in the
//! per-mesh uniform as a stop table, and that is the whole reason the "heavy at"
//! slider is free — moving it rewrites sixteen `vec4`s instead of re-measuring a
//! million vertices, re-uploading a field, or rebuilding a bind group.
//!
//! Kept out of `app_render.rs` on purpose: that module owns the frame, and this
//! is one layer's material.

use super::app_render::scene_mesh_uniform;
use occluview_core::SceneMesh;
use occluview_render::GpuMeshUniform;

/// The per-mesh uniform for one layer, including the contact paint it wears.
///
/// The paint is two independent facts and both are needed before a layer draws a
/// contact map: the packed field (bound in group 2, which the prepared scene
/// owns) and the ramp the field is read through (in this uniform, which is why
/// moving the slider costs a uniform write and nothing else). The stop table is
/// rebuilt per frame from the live scale rather than cached, because it is 16
/// `vec4`s and rebuilding it is cheaper than tracking when it went stale.
pub(super) fn scene_mesh_uniform_with_contacts(
    entry: &SceneMesh,
    contact: Option<&occluview_contact::ContactScale>,
    field_width: u32,
) -> GpuMeshUniform {
    let mut uniform = scene_mesh_uniform(entry);
    if let Some(scale) = contact {
        let table = scale.stop_table();
        uniform.set_contact_paint(
            field_width,
            table.ramp[2],
            table.ramp[3],
            &table.stops[..usize::try_from(table.count).unwrap_or(0)],
        );
        // A contact reading is a measurement, and a measurement keeps its own
        // hue: the layer draws under the measured-map treatment, which drops the
        // layer tint (a tint would shift the ramp away from the legend's
        // colours) and reduces the studio light so a saturated ramp still shows
        // a cusp. The operator's own colour and texture toggles are left alone —
        // outside the painted band the scan looks exactly as they set it up.
        uniform.measured_map = 1;
    }
    uniform
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use glam::Vec3;
    use occluview_core::scene::SceneMesh;
    use occluview_core::{Mesh, Vertex};

    fn triangle_entry() -> SceneMesh {
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
        SceneMesh::new(mesh)
    }

    /// A contact reading is a measurement too, and the same rule applies: it
    /// keeps its own hue. The tint is dropped (a tint would shift the ramp away
    /// from the legend) and the lighting is reduced, both of which ride on the
    /// measured-map flag.
    #[test]
    fn a_contact_map_draws_as_a_measured_map() {
        use occluview_contact::ContactScale;

        let entry = triangle_entry();
        let scale = ContactScale::new(&occluview_contact::TIGHTNESS, 0.22);

        let plain = super::scene_mesh_uniform(&entry);
        assert_eq!(
            plain.contact_map, 0,
            "a layer without a reading paints none"
        );

        let painted = super::scene_mesh_uniform_with_contacts(&entry, Some(&scale), 1024);
        assert_eq!(painted.contact_map, 1);
        assert_eq!(
            painted.measured_map, 1,
            "a contact map must draw as a measurement: its own hue, no tint"
        );
        assert!(
            painted.contact_stop_count > 1,
            "the ramp travels with the uniform, which is what makes the slider free"
        );
        assert!(
            (painted.contact_field_width - 1024.0).abs() < f32::EPSILON,
            "the shader divides by this, so it must be the exact texel count"
        );
    }
}
