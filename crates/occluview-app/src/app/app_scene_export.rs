//! Whole-scene export: posing a layer's geometry and writing the scene out.
//!
//! Split from `app_mesh_export` because these answer a different question. That
//! module writes one chosen layer; this one bakes a layer's placement into its
//! geometry and writes the scene the operator is actually looking at.

use super::app_mesh_export::{
    default_layer_export_format, layer_export_file_dialog, mesh_export_format_from_path,
    mesh_write_extension, normalize_layer_export_path, sanitize_filename_stem,
};
use super::{AppErrorDialog, OccluViewApp, Scene};
use glam::{Affine3A, DAffine3, DMat3, DVec3};
use occluview_core::{Mesh, SceneMesh, SceneMeshId, Vertex};
use occluview_formats::write::{write_mesh_overwrite, MeshWriteFormat, MeshWriteOptions};

impl OccluViewApp {
    /// Write every visible layer, in its current pose, as one file.
    ///
    /// A merged file carries geometry and vertex colours; a texture belongs to
    /// one mesh and cannot survive the merge, so the status line says so
    /// instead of letting the operator discover it later.
    pub(super) fn save_scene_dialog(&mut self) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        let Some(mesh) = merged_scene_mesh(scene.as_ref()) else {
            self.status_message = Some("Nothing visible to save".into());
            return;
        };
        let dropped_texture = scene
            .meshes()
            .iter()
            .any(|entry| entry.visible && entry.mesh.texture().is_some());

        let Some(selected) = layer_export_file_dialog(MeshWriteFormat::PlyBinaryLittleEndian)
            .set_file_name("scene.ply")
            .save_file()
        else {
            return;
        };
        let path = normalize_layer_export_path(selected, MeshWriteFormat::PlyBinaryLittleEndian);
        let Ok(format) = mesh_export_format_from_path(&path) else {
            self.status_message = Some("Unsupported output format".into());
            return;
        };

        // Only the VISIBLE layers go into the file, so only their edits are on
        // disk afterwards. Clearing the whole flag also cleared it for a hidden
        // layer that was never written: the close guard then read clean and the
        // app shut without asking, taking that layer's edits with it.
        let written: Vec<SceneMeshId> = scene
            .meshes()
            .iter()
            .filter(|entry| entry.visible)
            .map(SceneMesh::id)
            .collect();
        match write_mesh_overwrite(&path, &mesh, format, MeshWriteOptions::default()) {
            Ok(_) => {
                let note = if dropped_texture {
                    " (textures are not merged)"
                } else {
                    ""
                };
                self.forget_unsaved_edits(&written);
                self.status_message = Some(format!("Scene saved{}: {}", note, path.display()));
            }
            Err(error) => {
                let summary = format!("Could not save the scene: {error}");
                self.status_message = Some(summary.clone());
                self.app_error = Some(AppErrorDialog {
                    title: "Could not save the scene".to_string(),
                    summary,
                    details: format!(
                        "Scene export failed\n\nPath:\n{}\n\nError:\n{error:#}",
                        path.display()
                    ),
                });
            }
        }
    }

    /// Write every visible layer to its own file in a chosen folder, each in
    /// its current pose.
    pub(super) fn save_each_layer_dialog(&mut self) {
        let Some(scene) = self.scene.clone() else {
            return;
        };
        if !scene.meshes().iter().any(|entry| entry.visible) {
            self.status_message = Some("Nothing visible to save".into());
            return;
        }
        let Some(directory) = rfd::FileDialog::new().pick_folder() else {
            return;
        };

        let paths = self.current_paths.clone();
        let mut written = 0usize;
        let mut failed = 0usize;
        for (index, entry) in scene.meshes().iter().enumerate() {
            if !entry.visible {
                continue;
            }
            let format = default_layer_export_format(&paths, index);
            let stem =
                sanitize_filename_stem(&crate::layers_overlay::layer_label(&paths, entry, index));
            let path = directory.join(format!("{stem}.{}", mesh_write_extension(format)));
            match write_mesh_overwrite(
                &path,
                &posed_mesh(entry),
                format,
                MeshWriteOptions::default(),
            ) {
                Ok(_) => written += 1,
                Err(_) => failed += 1,
            }
        }

        if failed == 0 {
            // Same rule as the whole-scene save: a hidden layer was not written,
            // so its edits are still only in memory.
            let written: Vec<SceneMeshId> = scene
                .meshes()
                .iter()
                .filter(|entry| entry.visible)
                .map(SceneMesh::id)
                .collect();
            self.forget_unsaved_edits(&written);
        }
        self.status_message = Some(if failed == 0 {
            format!("Saved {written} layers to {}", directory.display())
        } else {
            format!(
                "Saved {written} layers to {}; {failed} could not be written",
                directory.display()
            )
        });
    }
}

/// The layer's mesh with its scene transform baked into positions and normals.
///
/// Export has to do this. A scan the operator aligned in the viewport carries
/// its new orientation in the layer transform, not in its vertices, so writing
/// the source mesh hands back a file in the *original* orientation and quietly
/// throws the alignment away.
///
/// The bake runs in `f64`: the pose is accumulated in double precision while
/// aligning, and narrowing it exactly once, here, is the point of keeping it
/// there.
pub(super) fn posed_mesh(entry: &SceneMesh) -> Mesh {
    if entry.transform == Affine3A::IDENTITY {
        return entry.mesh.clone();
    }
    let affine = double_affine(entry.transform);
    // Normals transform by the inverse transpose. For a rigid pose that is
    // just the rotation, but a placement can carry scale and the general form
    // costs nothing here.
    let normal_basis = affine.matrix3.inverse().transpose();
    let vertices: Vec<Vertex> = entry
        .mesh
        .vertices()
        .iter()
        .map(|vertex| {
            let mut posed = *vertex;
            posed.position = affine
                .transform_point3(double_vec(vertex.position))
                .as_vec3()
                .to_array();
            posed.normal = (normal_basis * double_vec(vertex.normal))
                .normalize_or_zero()
                .as_vec3()
                .to_array();
            posed
        })
        .collect();

    let name = entry.mesh.name().map(str::to_owned);
    if entry.mesh.is_point_cloud() {
        return Mesh::point_cloud(name, vertices);
    }
    // The vertex count and the indices are unchanged, so this cannot fail for a
    // mesh that already validated. If it ever does, the fallback must NOT be the
    // source mesh: that writes the scan in its original position and calls it a
    // success, which is the one outcome an export must never produce — the
    // operator's alignment thrown away silently. Keep the posed vertices and let
    // the point-cloud form carry them.
    let Ok(mut posed) = Mesh::new(
        name.clone(),
        vertices.clone(),
        entry.mesh.indices().to_vec(),
    ) else {
        return Mesh::point_cloud(name, vertices);
    };
    if let Some(texture) = entry.mesh.texture() {
        posed.set_texture(texture.clone());
    }
    posed
}

/// Every visible layer merged into one mesh, each in its own pose.
///
/// Returns `None` when nothing visible remains. Textures cannot be merged —
/// one mesh carries one texture — so the caller says so before writing.
fn merged_scene_mesh(scene: &Scene) -> Option<Mesh> {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for entry in scene.meshes().iter().filter(|entry| entry.visible) {
        let posed = posed_mesh(entry);
        let offset = u32::try_from(vertices.len()).ok()?;
        indices.extend(posed.indices().iter().map(|index| index + offset));
        vertices.extend_from_slice(posed.vertices());
    }
    if vertices.is_empty() {
        return None;
    }
    let name = Some("scene".to_owned());
    if indices.is_empty() {
        return Some(Mesh::point_cloud(name, vertices));
    }
    Mesh::new(name, vertices, indices).ok()
}

/// Promote a single-precision affine to double precision.
fn double_affine(transform: Affine3A) -> DAffine3 {
    let basis = transform.matrix3;
    DAffine3::from_mat3_translation(
        DMat3::from_cols(
            basis.x_axis.as_dvec3(),
            basis.y_axis.as_dvec3(),
            basis.z_axis.as_dvec3(),
        ),
        transform.translation.as_dvec3(),
    )
}

/// Promote a stored `[f32; 3]` attribute to double precision.
fn double_vec(value: [f32; 3]) -> DVec3 {
    DVec3::new(
        f64::from(value[0]),
        f64::from(value[1]),
        f64::from(value[2]),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::{merged_scene_mesh, posed_mesh};
    use anyhow::Result;
    use glam::Vec3;
    use occluview_core::{Mesh, Scene, SceneMesh, Vertex};

    fn v(x: f32, y: f32, z: f32) -> Vertex {
        Vertex::at(Vec3::new(x, y, z))
    }

    fn exportable_scene() -> Result<Scene> {
        let mesh = Mesh::new(
            Some("scan".into()),
            vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        )?;
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(mesh));
        Ok(scene)
    }

    #[test]
    fn export_bakes_the_layer_transform_into_positions() -> Result<()> {
        let mut scene = exportable_scene()?;
        let original = scene.meshes()[0].mesh.vertices()[1].position;
        scene.meshes_mut()[0].transform =
            glam::Affine3A::from_translation(Vec3::new(10.0, -3.0, 2.0));

        let posed = posed_mesh(&scene.meshes()[0]);

        assert!((posed.vertices()[1].position[0] - original[0] - 10.0).abs() < 1e-4);
        assert!((posed.vertices()[1].position[1] - original[1] + 3.0).abs() < 1e-4);
        assert!((posed.vertices()[1].position[2] - original[2] - 2.0).abs() < 1e-4);
        Ok(())
    }

    #[test]
    fn export_bakes_rotation_into_normals() -> Result<()> {
        let mut scene = exportable_scene()?;
        scene.meshes_mut()[0].transform =
            glam::Affine3A::from_rotation_x(std::f32::consts::FRAC_PI_2);

        let posed = posed_mesh(&scene.meshes()[0]);

        let normal = posed.vertices()[0].normal;
        let length = normal.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!(
            (length - 1.0).abs() < 1e-3,
            "a baked normal must stay unit length, got {length}"
        );
        let before = scene.meshes()[0].mesh.vertices()[0].normal;
        assert!(
            (normal[1] - before[1]).abs() > 0.5 || (normal[2] - before[2]).abs() > 0.5,
            "a quarter turn must actually move the normal"
        );
        Ok(())
    }

    #[test]
    fn an_identity_transform_leaves_the_mesh_byte_identical() -> Result<()> {
        let scene = exportable_scene()?;
        let posed = posed_mesh(&scene.meshes()[0]);
        assert_eq!(posed.vertices(), scene.meshes()[0].mesh.vertices());
        assert_eq!(posed.indices(), scene.meshes()[0].mesh.indices());
        Ok(())
    }

    #[test]
    fn a_scene_export_merges_visible_layers_in_their_poses() -> Result<()> {
        let mut scene = exportable_scene()?;
        let copy = scene.meshes()[0].mesh.clone();
        let single = scene.meshes()[0].mesh.vertices().len();
        scene.add(
            SceneMesh::new(copy)
                .with_transform(glam::Affine3A::from_translation(Vec3::new(100.0, 0.0, 0.0))),
        );

        let merged = merged_scene_mesh(&scene).expect("two visible layers merge");

        assert_eq!(merged.vertices().len(), single * 2);
        assert_eq!(
            merged.indices().len(),
            scene.meshes()[0].mesh.indices().len() * 2
        );
        let far = merged
            .vertices()
            .iter()
            .filter(|vertex| vertex.position[0] > 50.0)
            .count();
        assert_eq!(far, single, "the second layer must land in its own pose");
        Ok(())
    }

    #[test]
    fn merged_indices_are_offset_so_the_second_layer_keeps_its_own_triangles() -> Result<()> {
        let mut scene = exportable_scene()?;
        let copy = scene.meshes()[0].mesh.clone();
        let single = u32::try_from(scene.meshes()[0].mesh.vertices().len())?;
        scene.add(SceneMesh::new(copy));

        let merged = merged_scene_mesh(&scene).expect("two visible layers merge");

        let tail = &merged.indices()[3..];
        assert!(
            tail.iter().all(|index| *index >= single),
            "the second layer's triangles must not reference the first layer's vertices"
        );
        Ok(())
    }

    #[test]
    fn a_hidden_layer_is_left_out_of_the_scene_export() -> Result<()> {
        let mut scene = exportable_scene()?;
        let copy = scene.meshes()[0].mesh.clone();
        let single = scene.meshes()[0].mesh.vertices().len();
        scene.add(SceneMesh::new(copy));
        scene.meshes_mut()[1].visible = false;

        let merged = merged_scene_mesh(&scene).expect("one visible layer still merges");

        assert_eq!(merged.vertices().len(), single);
        Ok(())
    }

    #[test]
    fn an_empty_or_all_hidden_scene_merges_to_nothing() -> Result<()> {
        let mut scene = exportable_scene()?;
        scene.meshes_mut()[0].visible = false;
        assert!(merged_scene_mesh(&scene).is_none());
        assert!(merged_scene_mesh(&Scene::new()).is_none());
        Ok(())
    }
}
