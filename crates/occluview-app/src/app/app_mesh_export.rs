use super::app_scene_export::posed_mesh;
use super::{
    AppErrorDialog, LayerContextAction, LayerContextRequest, OccluViewApp, PathBuf, Scene,
};
use anyhow::{bail, Context, Result};
use occluview_formats::write::{
    write_mesh_overwrite, MeshWriteFormat, MeshWriteOptions, MeshWriteReport, MeshWriteWarning,
};
use std::path::Path;

/// How an interactive save-edited-layers pass ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SaveEditedLayersOutcome {
    /// Every layer with unsaved edits was exported.
    AllSaved,
    /// The operator cancelled a dialog or an export failed; unsaved edits
    /// remain.
    Aborted,
    /// Nothing carried unsaved edits.
    NothingToSave,
}

impl OccluViewApp {
    pub(super) fn save_layer_export_dialog(
        &mut self,
        scene: &Scene,
        paths: &[PathBuf],
        request: LayerContextRequest,
    ) -> bool {
        let default_format = default_layer_export_format(paths, request.index);
        let mut dialog = layer_export_file_dialog(default_format).set_file_name(
            default_layer_export_name(paths, scene, request.index, default_format),
        );
        if let Some(directory) =
            default_layer_export_directory(paths, request.index, self.last_export_dir.as_deref())
        {
            dialog = dialog.set_directory(directory);
        }

        let Some(selected_path) = dialog.save_file() else {
            return false;
        };
        let path = normalize_layer_export_path(selected_path, default_format);

        match write_layer_export_to_path(scene, request, &path) {
            Ok(report) => {
                // The layer on disk now matches the scene: it no longer
                // counts toward the unsaved-edits close guard.
                self.unsaved_edit_layer_ids.remove(&request.layer_id);
                self.remember_export_directory(&path);
                let warning_suffix = mesh_export_warning_summary(&report.warnings)
                    .map(|summary| format!(" (warnings: {summary})"))
                    .unwrap_or_default();
                // Named, and with the pose called out. An operator who aligns two
                // scans and exports one has no other way to check they exported
                // the arch they moved: a file written in its original position and
                // a file written in its aligned one both just said "Exported
                // layer". Whichever it is, it is now on the status line.
                let name = self
                    .layer_display_name(request.layer_id)
                    .unwrap_or_else(|| "layer".to_owned());
                let placement = if moved_from_source(scene, request) {
                    " in its aligned position"
                } else {
                    " (this scan has not been moved)"
                };
                self.status_message = Some(format!(
                    "Exported {name}{placement} as {}{}: {}",
                    mesh_export_format_label(report.format),
                    warning_suffix,
                    path.display()
                ));
                true
            }
            Err(error) => {
                let summary = format!("Could not export layer: {error}");
                self.status_message = Some(summary.clone());
                self.app_error = Some(AppErrorDialog {
                    title: "Could not export layer".to_string(),
                    summary,
                    details: format!(
                        "Layer export failed\n\nPath:\n{}\n\nError:\n{error:#}",
                        path.display()
                    ),
                });
                false
            }
        }
    }

    /// Remember where an export landed so the next save dialog for a layer
    /// with no file of its own starts there instead of guessing.
    pub(super) fn remember_export_directory(&mut self, written: &Path) {
        if let Some(parent) = written.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            self.last_export_dir = Some(parent.to_path_buf());
        }
    }

    /// Walk every layer with unsaved edits through the export dialog, one at
    /// a time. Stops at the first cancelled dialog or failed write so the
    /// operator is never told edits were saved when they were not.
    pub(super) fn save_edited_layers_flow(&mut self) -> SaveEditedLayersOutcome {
        let Some(scene) = self.scene.clone() else {
            return SaveEditedLayersOutcome::NothingToSave;
        };
        let paths = self.current_paths.clone();
        let pending: Vec<(usize, occluview_core::SceneMeshId)> = scene
            .meshes()
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.unsaved_edit_layer_ids.contains(&entry.id()))
            .map(|(index, entry)| (index, entry.id()))
            .collect();
        if pending.is_empty() {
            // Edited layers may have been removed from the scene since; the
            // guard has nothing actionable left.
            self.clear_unsaved_mesh_edits();
            return SaveEditedLayersOutcome::NothingToSave;
        }
        for (index, layer_id) in pending {
            let request = LayerContextRequest {
                index,
                layer_id,
                action: LayerContextAction::ExportLayer,
            };
            if !self.save_layer_export_dialog(scene.as_ref(), &paths, request) {
                return SaveEditedLayersOutcome::Aborted;
            }
        }
        if self.unsaved_edit_layer_ids.is_empty() {
            SaveEditedLayersOutcome::AllSaved
        } else {
            SaveEditedLayersOutcome::Aborted
        }
    }
}

/// Whether this layer sits anywhere other than where its file put it.
///
/// Exactly the test the export bake uses, so the sentence on the status line and
/// the geometry in the file cannot disagree.
fn moved_from_source(scene: &Scene, request: LayerContextRequest) -> bool {
    scene
        .meshes()
        .get(request.index)
        .filter(|entry| entry.id() == request.layer_id)
        .is_some_and(|entry| entry.transform != glam::Affine3A::IDENTITY)
}

fn write_layer_export_to_path(
    scene: &Scene,
    request: LayerContextRequest,
    path: &Path,
) -> Result<MeshWriteReport> {
    if request.action != LayerContextAction::ExportLayer {
        bail!("layer export received a non-export action");
    }

    let Some(entry) = scene.meshes().get(request.index) else {
        bail!("layer index {} is no longer available", request.index + 1);
    };
    if entry.id() != request.layer_id {
        bail!("layer identity changed before export");
    }

    let format = mesh_export_format_from_path(path)?;
    write_mesh_overwrite(
        path,
        &posed_mesh(entry),
        format,
        MeshWriteOptions::default(),
    )
    .with_context(|| format!("writing {}", path.display()))
}

pub(super) fn mesh_export_format_from_path(path: &Path) -> Result<MeshWriteFormat> {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        bail!("unsupported export format; choose an output file ending in .ply, .stl, or .obj");
    };

    match extension.to_ascii_lowercase().as_str() {
        "ply" => Ok(MeshWriteFormat::PlyBinaryLittleEndian),
        "stl" => Ok(MeshWriteFormat::StlBinary),
        "obj" => Ok(MeshWriteFormat::Obj),
        other => bail!("unsupported export format .{other}; use .ply, .stl, or .obj"),
    }
}

fn mesh_export_format_label(format: MeshWriteFormat) -> &'static str {
    match format {
        MeshWriteFormat::StlBinary => "STL",
        MeshWriteFormat::PlyBinaryLittleEndian => "PLY",
        MeshWriteFormat::Obj => "OBJ",
    }
}

pub(super) fn layer_export_file_dialog(default_format: MeshWriteFormat) -> rfd::FileDialog {
    let formats = match default_format {
        MeshWriteFormat::StlBinary => [
            MeshWriteFormat::StlBinary,
            MeshWriteFormat::PlyBinaryLittleEndian,
            MeshWriteFormat::Obj,
        ],
        MeshWriteFormat::PlyBinaryLittleEndian => [
            MeshWriteFormat::PlyBinaryLittleEndian,
            MeshWriteFormat::StlBinary,
            MeshWriteFormat::Obj,
        ],
        MeshWriteFormat::Obj => [
            MeshWriteFormat::Obj,
            MeshWriteFormat::PlyBinaryLittleEndian,
            MeshWriteFormat::StlBinary,
        ],
    };

    formats
        .into_iter()
        .fold(rfd::FileDialog::new(), |dialog, format| match format {
            MeshWriteFormat::StlBinary => dialog.add_filter("STL mesh", &["stl"]),
            MeshWriteFormat::PlyBinaryLittleEndian => dialog.add_filter("PLY mesh", &["ply"]),
            MeshWriteFormat::Obj => dialog.add_filter("OBJ mesh", &["obj"]),
        })
}

fn exact_layer_source_path(paths: &[PathBuf], index: usize) -> Option<&Path> {
    paths
        .get(index)
        .map(PathBuf::as_path)
        .filter(|path| !path.as_os_str().is_empty())
}

/// The path standing in for a layer that has none of its own — a part split
/// or cut out of another scan: the nearest sibling with a file, looking
/// backwards first. Splits insert the new part right after its source layer,
/// so the backwards neighbour is that source, not whichever unrelated case
/// happens to sit first in the scene.
fn nearest_layer_source_path(paths: &[PathBuf], index: usize) -> Option<&Path> {
    let non_empty = |path: &&PathBuf| !path.as_os_str().is_empty();
    paths
        .get(..index)
        .and_then(|before| before.iter().rev().find(non_empty))
        .or_else(|| {
            paths
                .get(index.saturating_add(1)..)
                .and_then(|after| after.iter().find(non_empty))
        })
        .map(PathBuf::as_path)
}

fn source_path_for_export_defaults(paths: &[PathBuf], index: usize) -> Option<&Path> {
    exact_layer_source_path(paths, index).or_else(|| nearest_layer_source_path(paths, index))
}

fn mesh_export_format_from_source_path(path: &Path) -> Option<MeshWriteFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "ply" => Some(MeshWriteFormat::PlyBinaryLittleEndian),
        "stl" => Some(MeshWriteFormat::StlBinary),
        "obj" => Some(MeshWriteFormat::Obj),
        // HPS/DCM and GLB are currently readable but do not have a matching
        // writer in the public export contract. Keep the fallback explicit.
        _ => None,
    }
}

pub(super) fn default_layer_export_format(paths: &[PathBuf], index: usize) -> MeshWriteFormat {
    source_path_for_export_defaults(paths, index)
        .and_then(mesh_export_format_from_source_path)
        .unwrap_or(MeshWriteFormat::PlyBinaryLittleEndian)
}

/// The folders a save dialog may open in, best first: the layer's own
/// folder, its nearest sibling's, then wherever the last export of this
/// session landed. The order is a pure fact here; whether a candidate still
/// exists on disk is judged by [`first_existing_directory`].
fn export_directory_candidates(
    paths: &[PathBuf],
    index: usize,
    last_export_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let parent_of = |path: Option<&Path>| {
        path.and_then(Path::parent)
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
    };
    let mut candidates: Vec<PathBuf> = [
        parent_of(exact_layer_source_path(paths, index)),
        parent_of(nearest_layer_source_path(paths, index)),
        last_export_dir
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(Path::to_path_buf),
    ]
    .into_iter()
    .flatten()
    .collect();
    candidates.dedup();
    candidates
}

/// The first candidate that is still a directory on disk. A folder that has
/// gone away — unplugged media, a deleted export target — falls through to
/// the next candidate, or to the platform's own dialog memory when none
/// survive.
fn first_existing_directory(candidates: Vec<PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|dir| dir.is_dir())
}

pub(super) fn default_layer_export_directory(
    paths: &[PathBuf],
    index: usize,
    last_export_dir: Option<&Path>,
) -> Option<PathBuf> {
    first_existing_directory(export_directory_candidates(paths, index, last_export_dir))
}

pub(super) fn mesh_write_extension(format: MeshWriteFormat) -> &'static str {
    match format {
        MeshWriteFormat::StlBinary => "stl",
        MeshWriteFormat::PlyBinaryLittleEndian => "ply",
        MeshWriteFormat::Obj => "obj",
    }
}

pub(super) fn normalize_layer_export_path(
    path: PathBuf,
    fallback_format: MeshWriteFormat,
) -> PathBuf {
    if path.extension().is_none() {
        path.with_extension(mesh_write_extension(fallback_format))
    } else {
        path
    }
}

fn default_layer_export_name(
    paths: &[PathBuf],
    scene: &Scene,
    index: usize,
    format: MeshWriteFormat,
) -> String {
    let stem = exact_layer_source_path(paths, index)
        .and_then(|path| path.file_stem())
        .and_then(|stem| stem.to_str())
        .or_else(|| {
            scene
                .meshes()
                .get(index)
                .and_then(|entry| entry.mesh.name())
        })
        .map(sanitize_filename_stem)
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| format!("layer-{}", index + 1));

    format!("{stem}-edited.{}", mesh_write_extension(format))
}

fn mesh_export_warning_summary(warnings: &[MeshWriteWarning]) -> Option<String> {
    let labels: Vec<&str> = warnings
        .iter()
        .map(|warning| match warning {
            MeshWriteWarning::PointCloudRejectedForStl => "point cloud omitted from STL",
            MeshWriteWarning::VertexColorsNotWritten => "vertex colors not included",
            MeshWriteWarning::UvsNotWritten => "UVs not included",
            MeshWriteWarning::TextureImageNotWritten => "texture image not included",
        })
        .collect();
    (!labels.is_empty()).then(|| labels.join(", "))
}

pub(super) fn sanitize_filename_stem(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect::<String>()
        .trim_matches(['.', ' '])
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer_actions::{LayerContextAction, LayerContextRequest};
    use glam::{Affine3A, Vec3};
    use occluview_core::{
        delete_selected_faces_in_mesh, FaceSelection, Mesh, MeshEditOptions, Scene, SceneMesh,
        Vertex,
    };
    use occluview_formats::write::MeshWriteFormat;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn layer_export_format_is_selected_from_output_extension() -> Result<()> {
        assert_eq!(
            mesh_export_format_from_path(Path::new("edited.stl")).ok(),
            Some(MeshWriteFormat::StlBinary)
        );
        assert_eq!(
            mesh_export_format_from_path(Path::new("edited.ply")).ok(),
            Some(MeshWriteFormat::PlyBinaryLittleEndian)
        );
        assert_eq!(
            mesh_export_format_from_path(Path::new("edited.obj")).ok(),
            Some(MeshWriteFormat::Obj)
        );
        let error = match mesh_export_format_from_path(Path::new("edited.glb")) {
            Ok(format) => {
                return Err(anyhow::anyhow!(
                    "glb should not be an edit export target, got {format:?}"
                ));
            }
            Err(error) => error,
        };
        assert!(error.to_string().contains("unsupported export format"));
        Ok(())
    }

    #[test]
    fn default_layer_export_name_uses_source_format() -> Result<()> {
        let scene = exportable_scene()?;
        let paths = vec![PathBuf::from("very-long-scan-name.stl")];

        let name =
            default_layer_export_name(&paths, &scene, 0, default_layer_export_format(&paths, 0));

        assert_eq!(name, "very-long-scan-name-edited.stl");
        Ok(())
    }

    #[test]
    fn source_format_falls_back_to_ply_for_read_only_formats() -> Result<()> {
        let scene = exportable_scene()?;
        let paths = vec![PathBuf::from("encrypted-scan.hps")];

        assert_eq!(
            default_layer_export_format(&paths, 0),
            MeshWriteFormat::PlyBinaryLittleEndian
        );
        assert_eq!(
            default_layer_export_name(&paths, &scene, 0, default_layer_export_format(&paths, 0),),
            "encrypted-scan-edited.ply"
        );
        Ok(())
    }

    #[test]
    fn derived_layer_uses_its_neighbour_for_folder_and_format() {
        let paths = vec![PathBuf::new(), PathBuf::from("/case/scans/upper.obj")];

        assert_eq!(default_layer_export_format(&paths, 0), MeshWriteFormat::Obj);
        assert_eq!(
            export_directory_candidates(&paths, 0, None),
            vec![PathBuf::from("/case/scans")]
        );
    }

    #[test]
    fn export_directory_prefers_the_exact_layer_source() {
        let paths = vec![
            PathBuf::from("/case/upper/upper.stl"),
            PathBuf::from("/case/lower/lower.ply"),
        ];

        assert_eq!(
            export_directory_candidates(&paths, 1, None),
            vec![PathBuf::from("/case/lower"), PathBuf::from("/case/upper")]
        );
        assert_eq!(
            default_layer_export_format(&paths, 1),
            MeshWriteFormat::PlyBinaryLittleEndian
        );
    }

    #[test]
    fn a_split_part_prefers_its_source_over_an_earlier_case() {
        // Splits insert the new part right after its source layer, so with a
        // second case open the backwards neighbour is the source scan, not
        // whichever case sits first in the scene.
        let paths = vec![
            PathBuf::from("/other-case/scan.stl"),
            PathBuf::from("/case/scans/bridge.stl"),
            PathBuf::new(),
        ];

        assert_eq!(
            export_directory_candidates(&paths, 2, None),
            vec![PathBuf::from("/case/scans")]
        );
        assert_eq!(
            default_layer_export_format(&paths, 2),
            MeshWriteFormat::StlBinary
        );
    }

    #[test]
    fn the_last_export_folder_backs_up_a_layer_with_no_sources_anywhere() {
        let paths = vec![PathBuf::new()];

        assert_eq!(
            export_directory_candidates(&paths, 0, Some(Path::new("/exports/today"))),
            vec![PathBuf::from("/exports/today")]
        );
        assert!(export_directory_candidates(&paths, 0, None).is_empty());
    }

    #[test]
    fn a_folder_that_no_longer_exists_is_passed_over() {
        let vanished = PathBuf::from("/occluview-test-this-folder-does-not-exist");
        let temp = std::env::temp_dir();

        assert_eq!(
            first_existing_directory(vec![vanished.clone(), temp.clone()]),
            Some(temp)
        );
        assert_eq!(first_existing_directory(vec![vanished]), None);
    }

    #[test]
    fn export_without_an_extension_uses_the_source_format() {
        assert_eq!(
            normalize_layer_export_path(PathBuf::from("edited"), MeshWriteFormat::StlBinary),
            PathBuf::from("edited.stl")
        );
        assert_eq!(
            normalize_layer_export_path(PathBuf::from("edited.obj"), MeshWriteFormat::StlBinary),
            PathBuf::from("edited.obj")
        );
    }

    #[test]
    fn write_layer_export_writes_requested_layer_without_mutating_scene() -> Result<()> {
        let scene = exportable_scene()?;
        let path = temp_file("ply");
        let request = LayerContextRequest {
            index: 0,
            layer_id: scene.meshes()[0].id(),
            action: LayerContextAction::ExportLayer,
        };

        let report = write_layer_export_to_path(&scene, request, &path)?;

        assert_eq!(report.format, MeshWriteFormat::PlyBinaryLittleEndian);
        assert!(path.exists());
        assert_eq!(scene.meshes()[0].mesh.triangle_count(), 1);
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn write_layer_export_rejects_stale_layer_identity() -> Result<()> {
        let scene = exportable_scene()?;
        let path = temp_file("ply");
        let request = LayerContextRequest {
            index: 0,
            layer_id: SceneMesh::new(Mesh::empty()).id(),
            action: LayerContextAction::ExportLayer,
        };

        let error = match write_layer_export_to_path(&scene, request, &path) {
            Ok(report) => {
                return Err(anyhow::anyhow!(
                    "stale layer export unexpectedly succeeded: {report:?}"
                ));
            }
            Err(error) => error,
        };

        assert!(error
            .to_string()
            .contains("layer identity changed before export"));
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn write_layer_export_uses_current_edited_mesh_snapshot() -> Result<()> {
        let mut scene = exportable_scene()?;
        let layer_id = scene.meshes()[0].id();
        let selection = FaceSelection::new(vec![true]);
        let edit = delete_selected_faces_in_mesh(
            &scene.meshes()[0].mesh,
            &selection,
            MeshEditOptions::default(),
        )?;
        scene.meshes_mut()[0].mesh = std::sync::Arc::new(edit.mesh);

        let path = temp_file("stl");
        let request = LayerContextRequest {
            index: 0,
            layer_id,
            action: LayerContextAction::ExportLayer,
        };

        let report = write_layer_export_to_path(&scene, request, &path)?;

        assert_eq!(report.format, MeshWriteFormat::StlBinary);
        assert!(path.exists());
        assert_eq!(scene.meshes()[0].mesh.triangle_count(), 0);
        let bytes = std::fs::read(&path)?;
        assert!(bytes.len() >= 84, "binary stl header should exist");
        let Ok(count_bytes) = <[u8; 4]>::try_from(&bytes[80..84]) else {
            return Err(anyhow::anyhow!("could not read binary stl triangle count"));
        };
        let triangle_count = u32::from_le_bytes(count_bytes);
        assert_eq!(triangle_count, 0);
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    /// The whole chain, end to end: a pose put on a layer the way a hand drag
    /// puts it there, exported, then READ BACK OFF DISK and checked.
    ///
    /// A manual move did not survive an export. Every
    /// step of this chain used to be covered only in pieces — the bake had a
    /// test, the writer had a test, the reader had a test — and none of them
    /// answered "does the file on disk hold the scan where the operator sees it".
    /// This one does, for all three formats, because the answer must not depend
    /// on which one they picked.
    #[test]
    fn an_exported_layer_lands_on_disk_in_the_pose_the_operator_sees() -> Result<()> {
        for extension in ["ply", "stl", "obj"] {
            let mut scene = exportable_scene()?;
            // A drag composes its steps onto whatever pose is already there, so
            // this is a turn AND a shift, not just a translation.
            let pose = Affine3A::from_rotation_z(std::f32::consts::FRAC_PI_2)
                * Affine3A::from_translation(Vec3::new(7.0, -3.0, 11.0));
            scene.meshes_mut()[0].transform = pose;
            let expected: Vec<Vec3> = scene.meshes()[0]
                .mesh
                .vertices()
                .iter()
                .map(|vertex| pose.transform_point3(Vec3::from_array(vertex.position)))
                .collect();

            let path = temp_file(extension);
            let request = LayerContextRequest {
                index: 0,
                layer_id: scene.meshes()[0].id(),
                action: LayerContextAction::ExportLayer,
            };
            write_layer_export_to_path(&scene, request, &path)?;

            let read_back = occluview_formats::read_file(&path)
                .map_err(|error| anyhow::anyhow!("reading back {extension}: {error}"))?;
            let _ = std::fs::remove_file(&path);

            assert_eq!(
                read_back.vertices().len(),
                expected.len(),
                "{extension}: vertex count changed on the way through disk"
            );
            for (slot, (vertex, want)) in read_back.vertices().iter().zip(&expected).enumerate() {
                let got = Vec3::from_array(vertex.position);
                assert!(
                    (got - *want).length() < 1e-3,
                    "{extension}: vertex {slot} came back at {got:?}, not the posed {want:?} — \
                     the operator's alignment was thrown away"
                );
            }
        }
        Ok(())
    }

    /// The status line names the layer and says whether it carries a pose, so an
    /// operator who exported the arch they did NOT move can see that.
    #[test]
    fn the_export_reports_whether_the_scan_had_been_moved() -> Result<()> {
        let mut scene = exportable_scene()?;
        let request = LayerContextRequest {
            index: 0,
            layer_id: scene.meshes()[0].id(),
            action: LayerContextAction::ExportLayer,
        };
        assert!(
            !moved_from_source(&scene, request),
            "a freshly loaded scan sits where its file put it"
        );

        scene.meshes_mut()[0].transform = Affine3A::from_translation(Vec3::new(0.0, 0.0, 4.0));
        assert!(moved_from_source(&scene, request));

        let stale = LayerContextRequest {
            index: 0,
            layer_id: scene.meshes()[0].id(),
            ..request
        };
        let mut renamed = stale;
        renamed.index = 9;
        assert!(
            !moved_from_source(&scene, renamed),
            "a layer that is not there cannot be reported as moved"
        );
        Ok(())
    }

    fn temp_file(extension: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!("occluview-layer-export-{unique}.{extension}"))
    }
}
