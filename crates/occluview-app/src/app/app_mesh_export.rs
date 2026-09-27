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
        // The format is decided from the scan, not from a preference: its own
        // format when the viewer can write it, and otherwise PLY or STL by what
        // the scan holds. A layer that is not in the scene has nothing to
        // inspect, so the dialog opens on PLY, the format that never loses a
        // payload.
        let default_format = match scene.meshes().get(request.index) {
            Some(entry) if entry.id() == request.layer_id => representable_export_format(
                automatic_export_format(paths, request.index, &entry.mesh),
                &entry.mesh,
            ),
            _ => MeshWriteFormat::PlyBinaryLittleEndian,
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
                // the arch they moved: a bare "Exported layer" reads the same for
                // a file written in its original position and one written in its
                // aligned position.
                let name = self
                    .layer_display_name(request.layer_id)
                    .unwrap_or_else(|| {
                        let position = scene
                            .meshes()
                            .iter()
                            .position(|entry| entry.id() == request.layer_id)
                            .map_or(1, |index| index + 1);
                        self.ui.locale.tr_with(
                            crate::i18n::message_id!("layer-unnamed"),
                            &[("n", &position.to_string())],
                        )
                    });
                let aligned = moved_from_source(scene, request);
                let format_label = mesh_export_format_label(report.format).to_owned();
                let path_text = path.display().to_string();
                let status_key = match (aligned, warnings.is_some()) {
                    (true, false) => crate::i18n::message_id!("mesh-exported-aligned"),
                    (true, true) => crate::i18n::message_id!("mesh-exported-aligned-warnings"),
                    (false, false) => crate::i18n::message_id!("mesh-exported-unmoved"),
                    (false, true) => crate::i18n::message_id!("mesh-exported-unmoved-warnings"),
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
                    crate::i18n::message_id!("mesh-export-failed-summary"),
                    &[("detail", &error.to_string())],
                );
                self.ui.status_message = Some(summary.clone());
                self.ui.app_error = Some(AppErrorDialog {
                    title: self
                        .ui
                        .locale
                        .tr(crate::i18n::message_id!("mesh-export-failed-title")),
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
    /// an export started mid-stroke would write the geometry from before the
    /// stroke (or an intermediate rebuild) and report success.
    /// `save_scene_dialog`, `save_each_layer_dialog` and
    /// `save_layer_export_dialog` are three separate entry points to the same
    /// scene, so every path asks here, as the close/replace guard does.
    ///
    /// Returns true when the caller must stop.
    pub(super) fn refuse_export_during_stroke(&mut self, ctx: &egui::Context) -> bool {
        if !self.document.unsaved_sculpt_stroke {
            return false;
        }
        // Ask the worker to finish, as Save does, so the next attempt
        // writes the stroke instead of nothing.
        let _ = self.commit_sculpt_stroke(ctx);
        self.ui.status_message = Some(
            self.ui
                .locale
                .tr(crate::i18n::message_id!("edit-session-busy")),
        );
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
            self.ui.status_message = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("edit-session-busy")),
            );
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
/// The same test the export bake uses, so the sentence on the status line and
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
        // HPS/DCM and GLB are readable but do not have a matching writer in
        // the public export contract. Keep the fallback explicit.
        _ => None,
    }
}

/// The format a layer is saved in, decided from the scan itself.
///
/// A scan keeps the format of the file it came from when the viewer can write
/// that format. When it cannot — `.dcm`/HPS, GLB, OFF, or a layer with no file
/// of its own — the format follows what the scan actually holds: PLY carries a
/// texture atlas, vertex colours and a mapping in one file, so a scan that has
/// any of them is saved as PLY, and a scan that is geometry alone is saved as
/// STL. STL cannot carry colour, which is why it is only proposed when there is
/// no colour to lose.
pub(super) fn automatic_export_format(
    paths: &[PathBuf],
    index: usize,
    mesh: &occluview_core::Mesh,
) -> MeshWriteFormat {
    source_path_for_export_defaults(paths, index)
        .and_then(mesh_export_format_from_source_path)
        .map_or_else(
            || format_for_payload(mesh),
            |source| source_format_that_carries(source, mesh),
        )
}

/// Whether this scan carries anything STL cannot hold.
fn carries_colour_payload(mesh: &occluview_core::Mesh) -> bool {
    mesh.texture().is_some() || mesh.has_vertex_colors() || mesh.has_uvs()
}

/// PLY when the scan has colour to keep or is not a triangle mesh, STL when it
/// is plain geometry.
///
/// A point cloud cannot be written as STL at all — the writer refuses a
/// non-triangle mesh — so the geometry kind belongs in the same decision as the
/// payload, and a point cloud is never proposed as STL.
pub(super) fn format_for_payload(mesh: &occluview_core::Mesh) -> MeshWriteFormat {
    if mesh.kind() != occluview_core::MeshKind::TriangleMesh || carries_colour_payload(mesh) {
        MeshWriteFormat::PlyBinaryLittleEndian
    } else {
        MeshWriteFormat::StlBinary
    }
}

/// Keep the source format, unless it cannot carry what the scan holds.
///
/// STL carries geometry only, so a coloured scan written as STL loses the
/// colour. OBJ carries vertex colours and a mapping but no image, so a textured
/// scan written as OBJ loses the atlas. PLY carries all three, so it is the one
/// format that never has to be second-guessed.
fn source_format_that_carries(
    source: MeshWriteFormat,
    mesh: &occluview_core::Mesh,
) -> MeshWriteFormat {
    match source {
        MeshWriteFormat::StlBinary if format_for_payload(mesh) != MeshWriteFormat::StlBinary => {
            MeshWriteFormat::PlyBinaryLittleEndian
        }
        MeshWriteFormat::Obj if mesh.texture().is_some() => MeshWriteFormat::PlyBinaryLittleEndian,
        other => other,
    }
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
/// writer, so without it a fallback of STL would propose a colourless file for
/// a colour scan.
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
    // Prefer an ASCII-safe source/file stem, then the mesh name,
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
    let suffix = locale.tr_with(
        crate::i18n::message_id!("mesh-export-warnings"),
        &[("warnings", warnings)],
    );
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
            MeshWriteWarning::VertexColorsNotWritten => {
                locale.tr(crate::i18n::message_id!("mesh-warning-vertex-colors"))
            }
            MeshWriteWarning::UvsNotWritten => {
                locale.tr(crate::i18n::message_id!("mesh-warning-uvs"))
            }
            MeshWriteWarning::TextureImageNotWritten => {
                locale.tr(crate::i18n::message_id!("mesh-warning-texture-image"))
            }
            MeshWriteWarning::VertexAlphaNotWritten => {
                locale.tr(crate::i18n::message_id!("mesh-warning-vertex-alpha"))
            }
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
mod tests;
