//! Turning a finished contact reading into the per-mesh uniform the viewport
//! draws.
//!
//! `app_render.rs` owns the frame; this module owns one layer's material.

use super::app_render::scene_mesh_uniform;
use occluview_core::SceneMesh;
use occluview_render::GpuMeshUniform;

/// The per-mesh uniform for one layer, including the contact paint it wears.
///
/// The paint is two independent facts and both are needed before a layer draws a
/// contact map: the packed field (bound in group 2, which the prepared scene
/// owns) and the ramp the field is read through. The ramp travels in this
/// uniform as a stop table, so moving the "heavy at" slider rewrites sixteen
/// `vec4`s instead of re-measuring a million vertices, re-uploading a field, or
/// rebuilding a bind group. The stop table is rebuilt per frame from the live
/// scale rather than cached, because it is 16 `vec4`s and rebuilding it is
/// cheaper than tracking when it went stale.
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
        // The layer keeps the treatment the operator gave it. A reading paints
        // the marks and nothing else: the ramp is mixed over the finished
        // surface in the shader, so the scan must not switch to the
        // measured-map treatment, which drops the tint and flattens the
        // lighting across the whole layer and would turn every scan white the
        // moment a reading opens.
    }
    uniform
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::float_cmp)]

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

    /// A reading paints its marks and changes nothing else about the layer.
    ///
    /// The ramp is mixed over the finished surface in the shader, so the layer
    /// must not switch to the measured-map treatment: that drops the tint and
    /// flattens the lighting across the whole scan, turning every model white
    /// the moment a reading opens.
    #[test]
    fn a_reading_does_not_restyle_the_layer_it_measures() {
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
            painted.measured_map, plain.measured_map,
            "a reading must not change how the layer is shaded"
        );
        assert!(
            painted.tint.map(f32::to_bits) == plain.tint.map(f32::to_bits),
            "the operator's tint survives a reading"
        );
        assert_eq!(
            painted.show_texture, plain.show_texture,
            "the operator's texture toggle survives a reading"
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

    /// The width argument is the shader's row stride: it turns a vertex index
    /// into `(index % width, index / width)`. Dropping or hardcoding it here
    /// would decode every vertex past the first row against the wrong texel.
    #[test]
    fn the_uniform_carries_the_field_width_it_was_given() {
        use occluview_contact::ContactScale;

        let entry = triangle_entry();
        let scale = ContactScale::new(&occluview_contact::TIGHTNESS, 0.22);

        let uniform = super::scene_mesh_uniform_with_contacts(&entry, Some(&scale), 37);
        assert_eq!(
            uniform.contact_field_width, 37.0,
            "the row stride the field was packed with must reach the uniform"
        );
    }
}
