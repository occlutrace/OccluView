use super::app_scene_export::posed_mesh;
use super::{
    AppErrorAction, AppErrorDialog, LayerContextAction, LayerContextRequest, OccluViewApp, PathBuf,
    Scene,
};
use anyhow::{bail, Context, Result};
use occluview_formats::write::{
    write_mesh_overwrite, MeshWriteFormat, MeshWriteOptions, MeshWriteReport, MeshWriteWarning,
};
use std::ffi::OsStr;
use std::path::Path;

pub(super) enum PendingLayerExports {
    Ready {
        scene: std::sync::Arc<Scene>,
        pending: Vec<(usize, occluview_core::SceneMeshId)>,
    },
    Nothing,
    StrokeInFlight,
}

/// How an interactive save-edited-layers pass ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveEditedLayersOutcome {
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
        // A layer export reads the same scene the other two paths do, so it
        // obeys the same rule. `pending_layer_exports` already commits the
        // stroke for the Save flow and keeps its guard open; this covers the
        // direct "Export layer" menu item, which has none.
        let ctx = self.ui.repaint_ctx.clone();
        if self.refuse_export_during_stroke(&ctx) {
            return false;
        }
        let fallback = fallback_mesh_write_format(self.persistence.settings.fallback_export_format);
        let default_format = match scene.meshes().get(request.index) {
            Some(entry) => representable_export_format(
                layer_export_format(
                    paths,
                    request.index,
                    fallback,
                    self.persistence.settings.keep_source_export_format,
                ),
                &entry.mesh,
            ),
            None => fallback,
        };
        let mut dialog = layer_export_file_dialog(default_format).set_file_name(
            default_layer_export_name(paths, scene, request.index, default_format),
        );
        if let Some(directory) = default_layer_export_directory(
            paths,
            request.index,
            self.persistence.last_export_dir.as_deref(),
        ) {
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
                self.document
                    .unsaved_edit_layer_ids
                    .remove(&request.layer_id);
                self.remember_export_directory(&path);
                let warnings = mesh_export_warning_summary(&report.warnings, &self.ui.locale);
                // Named, and with the pose called out. An operator who aligns two
                // scans and exports one has no other way to check they exported
                // the arch they moved: a file written in its original position and
                // a file written in its aligned one both just said "Exported
                // layer". Whichever it is, it is now on the status line.
                let name = self
                    .layer_display_name(request.layer_id)
                    .unwrap_or_else(|| {
                        let position = scene
                            .meshes()
                            .iter()
                            .position(|entry| entry.id() == request.layer_id)
                            .map_or(1, |index| index + 1);
                        self.ui
                            .locale
                            .tr_with("layer-unnamed", &[("n", &position.to_string())])
                    });
                let aligned = moved_from_source(scene, request);
                let format_label = mesh_export_format_label(report.format).to_owned();
                let path_text = path.display().to_string();
                let status_key = match (aligned, warnings.is_some()) {
                    (true, false) => "mesh-exported-aligned",
                    (true, true) => "mesh-exported-aligned-warnings",
                    (false, false) => "mesh-exported-unmoved",
                    (false, true) => "mesh-exported-unmoved-warnings",
                };
                self.ui.status_message = Some(self.ui.locale.tr_with(
                    status_key,
                    &[
                        ("name", name.as_str()),
                        ("format", format_label.as_str()),
                        ("warnings", warnings.as_deref().unwrap_or("")),
                        ("path", path_text.as_str()),
                    ],
                ));
                true
            }
            Err(error) => {
                let summary = self.ui.locale.tr_with(
                    "mesh-export-failed-summary",
                    &[("detail", &error.to_string())],
                );
                self.ui.status_message = Some(summary.clone());
                self.ui.app_error = Some(AppErrorDialog {
                    title: self.ui.locale.tr("mesh-export-failed-title"),
                    summary,
                    details: format!(
                        "Layer export failed\n\nPath:\n{}\n\nError:\n{error:#}",
                        path.display()
                    ),
                    action: AppErrorAction::None,
                });
                false
            }
        }
    }

    /// Remember where an export landed so the next save dialog for a layer
    /// with no file of its own starts there instead of guessing.
    pub(super) fn remember_export_directory(&mut self, written: &Path) {
        let Some(parent) = written.parent().filter(|dir| !dir.as_os_str().is_empty()) else {
            return;
        };
        self.persistence.last_export_dir = Some(parent.to_path_buf());
        if self.persistence.settings.remember_export_dir {
            self.persistence.settings.last_export_dir = parent.to_str().map(str::to_owned);
            self.persistence.settings_persistence.mark_dirty();
        }
    }

    /// Refuse an export while a Sculpt stroke is still being rebuilt.
    ///
    /// The scene only advances to a stroke's result when its worker lands, so
    /// an export started mid-stroke writes the geometry from BEFORE the stroke
    /// — or an intermediate rebuild — reports success, and the operator finds
    /// out by re-opening the file. `save_scene_dialog`, `save_each_layer_dialog`
    /// and `save_layer_export_dialog` are three separate entry points to the
    /// same scene, and the close/replace guard was the only one that knew this
    /// rule; every path asks here instead.
    ///
    /// Returns true when the caller must stop.
    pub(super) fn refuse_export_during_stroke(&mut self, ctx: &egui::Context) -> bool {
        if !self.document.unsaved_sculpt_stroke {
            return false;
        }
        // Ask the worker to finish, exactly as Save does, so the next attempt
        // writes the stroke instead of nothing.
        let _ = self.commit_sculpt_stroke(ctx);
        self.ui.status_message = Some(self.ui.locale.tr("edit-session-busy"));
        true
    }

    /// Collect edited layers for Save, committing a held Align drag first.
    /// Returns `Nothing` when no edited layer remains.
    pub(super) fn pending_layer_exports(&mut self) -> PendingLayerExports {
        let Some(scene) = self.document.scene.clone() else {
            return PendingLayerExports::Nothing;
        };
        // Commit a held drag before collecting the edited layers.
        self.finish_align_drag();
        // A live Sculpt stroke must finish before its layer can be exported.
        if self.document.unsaved_sculpt_stroke {
            let ctx = self.ui.repaint_ctx.clone();
            let _ = self.commit_sculpt_stroke(&ctx);
            self.ui.status_message = Some(self.ui.locale.tr("edit-session-busy"));
            return PendingLayerExports::StrokeInFlight;
        }
        let scene = self.document.scene.clone().unwrap_or(scene);
        let pending: Vec<(usize, occluview_core::SceneMeshId)> = scene
            .meshes()
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.document.unsaved_edit_layer_ids.contains(&entry.id()))
            .map(|(index, entry)| (index, entry.id()))
            .collect();
        if pending.is_empty() {
            // Edited layers may have been removed from the scene since; the
            // guard has nothing actionable left.
            self.document.clear_unsaved_mesh_edits();
            return PendingLayerExports::Nothing;
        }
        PendingLayerExports::Ready { scene, pending }
    }

    pub(super) fn save_edited_layers_flow(&mut self) -> SaveEditedLayersOutcome {
        let (scene, pending) = match self.pending_layer_exports() {
            PendingLayerExports::Ready { scene, pending } => (scene, pending),
            PendingLayerExports::Nothing => return SaveEditedLayersOutcome::NothingToSave,
            // The caller keeps the guard open until the stroke has landed.
            PendingLayerExports::StrokeInFlight => return SaveEditedLayersOutcome::Aborted,
        };
        let paths = self.persistence.current_paths.clone();
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
        if self.document.unsaved_edit_layer_ids.is_empty() {
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

/// The format a layer is saved in.
///
/// `keep_source` is the operator's preference: with it on, a scan keeps the
/// format of the file it came from, and the fallback only applies where there
/// is no source format (a part cut out of another layer) or where the source
/// format has no writer. With it off, every layer is written in the fallback
/// format, which is what a shop that feeds one kind of mill wants.
pub(super) fn layer_export_format(
    paths: &[PathBuf],
    index: usize,
    fallback: MeshWriteFormat,
    keep_source: bool,
) -> MeshWriteFormat {
    if !keep_source {
        return fallback;
    }
    default_layer_export_format(paths, index, fallback)
}

pub(super) fn default_layer_export_format(
    paths: &[PathBuf],
    index: usize,
    fallback: MeshWriteFormat,
) -> MeshWriteFormat {
    source_path_for_export_defaults(paths, index)
        .and_then(mesh_export_format_from_source_path)
        .unwrap_or(fallback)
}

/// The format actually offered for one layer.
///
/// Two things make the chosen format unwritable or lossy here, and both are
/// answered before a file name is proposed:
///
/// * A point cloud cannot be written as STL — the writer refuses a non-triangle
///   mesh — so a forced STL falls back to PLY for that layer rather than
///   proposing a file name whose write is guaranteed to fail into an error
///   dialog.
/// * STL carries geometry and nothing else. A colour scan written as STL loses
///   the colour it was captured with, and the operator only finds out from a
///   warning on the status line, after the name is already chosen. PLY holds
///   the atlas, the vertex colours and the mapping in one file, so a layer that
///   has any of them opens its save dialog on PLY instead.
///
/// The colour rule is what a `.dcm`/HPS scan needs: those formats have no
/// writer, so the save dialog came up on the operator's fallback format, and a
/// fallback of STL silently proposed a colourless file for a colour scan.
pub(super) fn representable_export_format(
    format: MeshWriteFormat,
    mesh: &occluview_core::Mesh,
) -> MeshWriteFormat {
    if format != MeshWriteFormat::StlBinary {
        return format;
    }
    if mesh.kind() != occluview_core::MeshKind::TriangleMesh
        || mesh.texture().is_some()
        || mesh.has_vertex_colors()
        || mesh.has_uvs()
    {
        return MeshWriteFormat::PlyBinaryLittleEndian;
    }
    format
}

pub(super) const fn fallback_mesh_write_format(
    format: crate::app_settings::FallbackExportFormat,
) -> MeshWriteFormat {
    match format {
        crate::app_settings::FallbackExportFormat::Ply => MeshWriteFormat::PlyBinaryLittleEndian,
        crate::app_settings::FallbackExportFormat::Stl => MeshWriteFormat::StlBinary,
        crate::app_settings::FallbackExportFormat::Obj => MeshWriteFormat::Obj,
    }
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

pub(super) fn default_layer_export_stem(
    paths: &[PathBuf],
    scene: &Scene,
    index: usize,
    format: MeshWriteFormat,
) -> String {
    // Deliberately prefer an ASCII-safe source/file stem, then the mesh name,
    // then a numbered fallback. This name is used by both single-layer and
    // batch exports, so they cannot drift into different naming rules.
    let source_stem = source_path_for_export_defaults(paths, index)
        .and_then(|path| path.file_stem())
        .and_then(|stem| stem.to_str());
    let raw = source_stem.or_else(|| {
        scene
            .meshes()
            .get(index)
            .and_then(|entry| entry.mesh.name())
    });
    let stem = raw
        .map(sanitize_filename_stem)
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| crate::layers_overlay::ascii_layer_stem(index));
    strip_repeated_export_suffix(stem, format)
}

/// Remove a source extension that would otherwise be emitted twice when the
/// selected export format is the same (`upper.stl` -> `upper`, then `upper.stl`).
fn strip_repeated_export_suffix(mut stem: String, format: MeshWriteFormat) -> String {
    let extension = mesh_write_extension(format);
    while let Some((prefix, suffix)) = stem.rsplit_once('.') {
        if !suffix.eq_ignore_ascii_case(extension) {
            break;
        }
        stem = prefix.to_owned();
    }
    stem
}
pub(super) fn normalize_layer_export_path(
    path: PathBuf,
    fallback_format: MeshWriteFormat,
) -> PathBuf {
    let path = if path.extension().is_none() {
        path.with_extension(mesh_write_extension(fallback_format))
    } else {
        path
    };
    collapse_repeated_terminal_extension(path)
}

/// Native save dialogs may append the active filter extension even when the
/// editable name already contains it (`scan.stl` -> `scan.stl.stl`). Collapse
/// only adjacent copies of the actual final extension. A name such as
/// `scan.stl.obj` remains intentional and continues to select OBJ.
fn collapse_repeated_terminal_extension(path: PathBuf) -> PathBuf {
    let Some(extension) = path.extension().map(OsStr::to_os_string) else {
        return path;
    };
    if extension.is_empty() {
        return path;
    }
    let Some(mut base) = path.file_stem().map(OsStr::to_os_string) else {
        return path;
    };

    let mut repeated = false;
    while let Some(nested_extension) = Path::new(base.as_os_str()).extension() {
        if !os_str_ascii_case_equal(nested_extension, extension.as_os_str()) {
            break;
        }
        let Some(nested_stem) = Path::new(base.as_os_str()).file_stem() else {
            break;
        };
        base = nested_stem.to_os_string();
        repeated = true;
    }
    if !repeated {
        return path;
    }

    let mut file_name = base;
    file_name.push(".");
    file_name.push(extension);
    path.with_file_name(file_name)
}

fn os_str_ascii_case_equal(left: &OsStr, right: &OsStr) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        left.as_bytes().eq_ignore_ascii_case(right.as_bytes())
    }
    #[cfg(not(unix))]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
}

fn default_layer_export_name(
    paths: &[PathBuf],
    scene: &Scene,
    index: usize,
    format: MeshWriteFormat,
) -> String {
    let stem = default_layer_export_stem(paths, scene, index, format);

    format!("{stem}-edited.{}", mesh_write_extension(format))
}

pub(super) fn append_mesh_export_warnings(
    mut status: String,
    warnings: Option<&str>,
    locale: &crate::i18n::LocaleManager,
) -> String {
    let Some(warnings) = warnings else {
        return status;
    };
    let suffix = locale.tr_with("mesh-export-warnings", &[("warnings", warnings)]);
    status.push_str(" · ");
    status.push_str(&suffix);
    status
}

pub(super) fn mesh_export_warning_summary(
    warnings: &[MeshWriteWarning],
    locale: &crate::i18n::LocaleManager,
) -> Option<String> {
    let labels: Vec<String> = warnings
        .iter()
        .map(|warning| match warning {
            MeshWriteWarning::VertexColorsNotWritten => locale.tr("mesh-warning-vertex-colors"),
            MeshWriteWarning::UvsNotWritten => locale.tr("mesh-warning-uvs"),
            MeshWriteWarning::TextureImageNotWritten => locale.tr("mesh-warning-texture-image"),
            MeshWriteWarning::VertexAlphaNotWritten => locale.tr("mesh-warning-vertex-alpha"),
        })
        .collect();
    (!labels.is_empty()).then(|| labels.join(", "))
}

pub(super) fn sanitize_filename_stem(raw: &str) -> String {
    let cleaned = raw
        .trim()
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect::<String>()
        .trim_matches(['.', ' '])
        .to_string();
    if is_windows_device_stem(&cleaned) {
        format!("_{cleaned}")
    } else {
        cleaned
    }
}

fn is_windows_device_stem(stem: &str) -> bool {
    let base = stem.split('.').next().unwrap_or_default();
    matches!(
        base.to_ascii_lowercase().as_str(),
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    )
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

        let name = default_layer_export_name(
            &paths,
            &scene,
            0,
            default_layer_export_format(&paths, 0, MeshWriteFormat::PlyBinaryLittleEndian),
        );

        assert_eq!(name, "very-long-scan-name-edited.stl");
        Ok(())
    }

    /// The operator's preference decides. "Keep each scan's own format" on is
    /// the default: a scan opened as STL saves as STL. Off, the chosen format
    /// wins for every layer, which is what a shop that feeds one kind of mill
    /// asks for.
    #[test]
    fn keeping_the_source_format_is_the_operator_choice() {
        let paths = vec![PathBuf::from("upper.stl")];

        assert_eq!(
            layer_export_format(&paths, 0, MeshWriteFormat::PlyBinaryLittleEndian, true),
            MeshWriteFormat::StlBinary,
            "the scan's own format wins while the switch is on"
        );
        assert_eq!(
            layer_export_format(&paths, 0, MeshWriteFormat::PlyBinaryLittleEndian, false),
            MeshWriteFormat::PlyBinaryLittleEndian,
            "the chosen format wins for every layer while the switch is off"
        );
    }

    /// A point cloud cannot be written as STL, so the format actually offered
    /// has to be one the geometry can be written as. The writer refuses a
    /// non-triangle mesh, and a forced STL used to propose a name whose write
    /// was guaranteed to fail into the error dialog.
    #[test]
    fn a_forced_stl_falls_back_to_ply_for_a_point_cloud() {
        let cloud = Mesh::point_cloud(Some("points".to_owned()), vec![Vertex::at(Vec3::ZERO)]);
        assert_eq!(
            representable_export_format(MeshWriteFormat::StlBinary, &cloud),
            MeshWriteFormat::PlyBinaryLittleEndian
        );

        let Ok(scene) = exportable_scene() else {
            return;
        };
        let Some(entry) = scene.meshes().first() else {
            return;
        };
        assert_eq!(
            representable_export_format(MeshWriteFormat::StlBinary, &entry.mesh),
            MeshWriteFormat::StlBinary,
            "a plain triangle mesh keeps the format the operator chose"
        );
    }

    /// STL carries geometry only, so proposing it for a colour scan silently
    /// throws the colour away. This is the `.dcm` case: the format has no
    /// writer, the dialog comes up on the fallback, and a fallback of STL used
    /// to propose a colourless file for a scan captured in colour.
    #[test]
    fn a_forced_stl_falls_back_to_ply_so_colour_is_not_thrown_away() {
        use occluview_core::MeshTexture;

        // A textured scan: the atlas and its mapping both live only in PLY.
        let Ok(scene) = exportable_scene() else {
            return;
        };
        let mut textured = (*scene.meshes()[0].mesh).clone();
        textured.set_texture(MeshTexture::new(1, 1, vec![1, 2, 3, 255]));
        assert_eq!(
            representable_export_format(MeshWriteFormat::StlBinary, &textured),
            MeshWriteFormat::PlyBinaryLittleEndian,
            "a textured scan must not be proposed as STL"
        );

        // A scan whose colour is per-vertex, with no atlas at all.
        let coloured = Mesh::new(
            Some("coloured".to_owned()),
            vec![
                Vertex::at(Vec3::ZERO).with_color([210, 180, 120, 255]),
                Vertex::at(Vec3::X).with_color([220, 170, 110, 255]),
                Vertex::at(Vec3::Y).with_color([230, 160, 100, 255]),
            ],
            vec![0, 1, 2],
        );
        let Ok(coloured) = coloured else { return };
        assert!(coloured.has_vertex_colors());
        assert_eq!(
            representable_export_format(MeshWriteFormat::StlBinary, &coloured),
            MeshWriteFormat::PlyBinaryLittleEndian,
            "a vertex-coloured scan must not be proposed as STL"
        );

        // A scan with a mapping but no image keeps it only in PLY too.
        let mapped = Mesh::new(
            Some("mapped".to_owned()),
            vec![
                Vertex::at(Vec3::ZERO).with_uv([0.0, 1.0]),
                Vertex::at(Vec3::X).with_uv([1.0, 1.0]),
                Vertex::at(Vec3::Y).with_uv([0.0, 0.0]),
            ],
            vec![0, 1, 2],
        );
        let Ok(mapped) = mapped else { return };
        assert!(mapped.has_uvs());
        assert_eq!(
            representable_export_format(MeshWriteFormat::StlBinary, &mapped),
            MeshWriteFormat::PlyBinaryLittleEndian,
            "a mapped scan must not be proposed as STL"
        );

        // PLY and OBJ are never second-guessed: only STL loses the payload.
        assert_eq!(
            representable_export_format(MeshWriteFormat::PlyBinaryLittleEndian, &coloured),
            MeshWriteFormat::PlyBinaryLittleEndian
        );
        assert_eq!(
            representable_export_format(MeshWriteFormat::Obj, &coloured),
            MeshWriteFormat::Obj
        );
    }

    /// The operator's exact case, end to end through the proposal chain.
    ///
    /// A `.dcm`/HPS has no writer, so "keep the source format" cannot apply and
    /// the stored fallback decides. With that fallback set to STL, a colour
    /// scan used to be offered as `.stl` — a file the export would then strip
    /// the colour out of. The proposal now comes out as PLY.
    #[test]
    fn a_colour_dcm_is_proposed_as_ply_even_when_the_fallback_is_stl() {
        use occluview_core::MeshTexture;

        let paths = vec![PathBuf::from("/scans/upper.dcm")];
        let fallback = MeshWriteFormat::StlBinary;

        // The source format of a .dcm has no writer, so the fallback is used.
        assert_eq!(
            layer_export_format(&paths, 0, fallback, true),
            MeshWriteFormat::StlBinary,
            "a .dcm has no writer, so the fallback is what the chain starts from"
        );

        // A texture scan: the proposal must not stay STL.
        let Ok(scene) = exportable_scene() else {
            return;
        };
        let mut textured = (*scene.meshes()[0].mesh).clone();
        textured.set_texture(MeshTexture::new(1, 1, vec![1, 2, 3, 255]));
        let proposed =
            representable_export_format(layer_export_format(&paths, 0, fallback, true), &textured);
        assert_eq!(
            proposed,
            MeshWriteFormat::PlyBinaryLittleEndian,
            "a textured .dcm must be proposed as PLY, not as a colourless STL"
        );
        assert_eq!(mesh_write_extension(proposed), "ply");

        // The same scan with colour on its vertices, no atlas.
        let coloured = Mesh::new(
            Some("upper".to_owned()),
            vec![
                Vertex::at(Vec3::ZERO).with_color([210, 180, 120, 255]),
                Vertex::at(Vec3::X).with_color([220, 170, 110, 255]),
                Vertex::at(Vec3::Y).with_color([230, 160, 100, 255]),
            ],
            vec![0, 1, 2],
        );
        let Ok(coloured) = coloured else { return };
        assert_eq!(
            representable_export_format(layer_export_format(&paths, 0, fallback, true), &coloured),
            MeshWriteFormat::PlyBinaryLittleEndian
        );

        // A plain geometry-only .dcm keeps the operator's STL: nothing is lost,
        // which is the other half of the rule.
        let Ok(plain) = exportable_scene().map(|scene| (*scene.meshes()[0].mesh).clone()) else {
            return;
        };
        assert!(!plain.has_vertex_colors() && !plain.has_uvs() && plain.texture().is_none());
        let proposed =
            representable_export_format(layer_export_format(&paths, 0, fallback, true), &plain);
        assert_eq!(
            proposed,
            MeshWriteFormat::StlBinary,
            "a geometry-only scan written as STL loses nothing, so STL stands"
        );
    }

    #[test]
    fn batch_export_stem_prefers_the_source_name_and_removes_its_format_suffix() -> Result<()> {
        let scene = exportable_scene()?;
        let paths = vec![PathBuf::from("upper.stl")];
        assert_eq!(
            default_layer_export_stem(&paths, &scene, 0, MeshWriteFormat::StlBinary),
            "upper"
        );
        let repeated = vec![PathBuf::from("upper.stl.stl")];
        assert_eq!(
            default_layer_export_stem(&repeated, &scene, 0, MeshWriteFormat::StlBinary),
            "upper"
        );
        Ok(())
    }

    #[test]
    fn derived_layer_stem_uses_the_nearest_source_when_it_has_no_path() -> Result<()> {
        let scene = exportable_scene()?;
        let paths = vec![PathBuf::new(), PathBuf::from("/case/upper.obj")];
        assert_eq!(
            default_layer_export_stem(&paths, &scene, 0, MeshWriteFormat::Obj),
            "upper"
        );
        Ok(())
    }

    #[test]
    fn generated_stems_avoid_windows_device_names() {
        assert_eq!(sanitize_filename_stem("CON"), "_CON");
        assert_eq!(sanitize_filename_stem("lpt1.final"), "_lpt1.final");
        assert_eq!(sanitize_filename_stem("COM10"), "COM10");
    }

    #[test]
    fn export_status_can_carry_writer_warnings_without_dropping_the_success() {
        let locale = crate::i18n::LocaleManager::for_tests();
        let rendered = append_mesh_export_warnings(
            "Scene saved".to_owned(),
            Some("UVs not included"),
            &locale,
        );

        assert!(rendered.starts_with("Scene saved"));
        assert!(rendered.contains("UVs not included"));
    }

    #[test]
    fn source_format_falls_back_to_ply_for_read_only_formats() -> Result<()> {
        let scene = exportable_scene()?;
        let paths = vec![PathBuf::from("encrypted-scan.hps")];

        assert_eq!(
            default_layer_export_format(&paths, 0, MeshWriteFormat::PlyBinaryLittleEndian),
            MeshWriteFormat::PlyBinaryLittleEndian
        );
        assert_eq!(
            default_layer_export_name(
                &paths,
                &scene,
                0,
                default_layer_export_format(&paths, 0, MeshWriteFormat::PlyBinaryLittleEndian),
            ),
            "encrypted-scan-edited.ply"
        );
        Ok(())
    }

    #[test]
    fn derived_layer_uses_its_neighbour_for_folder_and_format() {
        let paths = vec![PathBuf::new(), PathBuf::from("/case/scans/upper.obj")];

        assert_eq!(
            default_layer_export_format(&paths, 0, MeshWriteFormat::PlyBinaryLittleEndian),
            MeshWriteFormat::Obj
        );
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
            default_layer_export_format(&paths, 1, MeshWriteFormat::PlyBinaryLittleEndian),
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
            default_layer_export_format(&paths, 2, MeshWriteFormat::PlyBinaryLittleEndian),
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
    fn export_dialog_does_not_persist_repeated_terminal_extensions() {
        assert_eq!(
            normalize_layer_export_path(
                PathBuf::from("edited.stl.stl"),
                MeshWriteFormat::StlBinary
            ),
            PathBuf::from("edited.stl")
        );
        assert_eq!(
            normalize_layer_export_path(
                PathBuf::from("edited.STL.StL"),
                MeshWriteFormat::StlBinary
            ),
            PathBuf::from("edited.StL")
        );
        assert_eq!(
            normalize_layer_export_path(
                PathBuf::from("case/edited.ply.ply.ply"),
                MeshWriteFormat::PlyBinaryLittleEndian,
            ),
            PathBuf::from("case/edited.ply")
        );
        assert_eq!(
            normalize_layer_export_path(
                PathBuf::from("edited.stl.obj"),
                MeshWriteFormat::StlBinary
            ),
            PathBuf::from("edited.stl.obj"),
            "a different final format is an intentional filename, not a duplicate"
        );
    }

    #[cfg(unix)]
    #[test]
    fn export_dialog_collapses_repeated_extension_for_non_utf8_names() {
        use std::ffi::OsString;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let path = PathBuf::from(OsString::from_vec(b"edited\xff.stl.stl".to_vec()));
        let normalized = normalize_layer_export_path(path, MeshWriteFormat::StlBinary);

        assert_eq!(normalized.as_os_str().as_bytes(), b"edited\xff.stl");
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

    /// Export a transformed layer and verify the geometry after re-reading it.
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
