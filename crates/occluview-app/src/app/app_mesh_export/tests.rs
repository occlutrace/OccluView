use super::*;
use crate::layer_actions::{LayerContextAction, LayerContextRequest};
use glam::{Affine3A, Vec3};
use occluview_core::{Mesh, Scene, SceneMesh, Vertex};
use occluview_edit::{delete_selected_faces_in_mesh, FaceSelection, MeshEditOptions};
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

    let format = automatic_export_format(&paths, 0, &scene.meshes()[0].mesh);
    let name = default_layer_export_name(&paths, &scene, 0, format);

    assert_eq!(format, MeshWriteFormat::StlBinary);
    assert_eq!(name, "very-long-scan-name-edited.stl");
    Ok(())
}

/// A scan keeps the format it was opened in when the viewer can write it,
/// and that is the only rule for a plain geometry scan: no switch, no
/// per-session choice.
#[test]
fn a_scan_keeps_its_own_writable_format() {
    let Ok(scene) = exportable_scene() else {
        panic!("required test setup or expected result was missing");
    };
    let plain = (*scene.meshes()[0].mesh).clone();

    for (path, expected) in [
        ("upper.stl", MeshWriteFormat::StlBinary),
        ("upper.ply", MeshWriteFormat::PlyBinaryLittleEndian),
        ("upper.obj", MeshWriteFormat::Obj),
    ] {
        let paths = vec![PathBuf::from(path)];
        assert_eq!(
            automatic_export_format(&paths, 0, &plain),
            expected,
            "a geometry-only scan opened as {path} saves as {path}"
        );
    }
}

/// A point cloud cannot be written as STL, so the format actually offered
/// has to be one the geometry can be written as. The writer refuses a
/// non-triangle mesh, and a forced STL would propose a name whose write is
/// guaranteed to fail into the error dialog.
#[test]
fn a_forced_stl_falls_back_to_ply_for_a_point_cloud() {
    let cloud = Mesh::point_cloud(Some("points".to_owned()), vec![Vertex::at(Vec3::ZERO)]);
    assert_eq!(
        representable_export_format(MeshWriteFormat::StlBinary, &cloud),
        MeshWriteFormat::PlyBinaryLittleEndian
    );
    // And the automatic choice never asks for STL in the first place.
    let paths = vec![PathBuf::from("points.stl")];
    assert_eq!(
        automatic_export_format(&paths, 0, &cloud),
        MeshWriteFormat::PlyBinaryLittleEndian,
        "a point cloud is never proposed as STL, whatever it was opened as"
    );

    let Ok(scene) = exportable_scene() else {
        panic!("required test setup or expected result was missing");
    };
    let Some(entry) = scene.meshes().first() else {
        panic!("required test setup or expected result was missing");
    };
    assert_eq!(
        representable_export_format(MeshWriteFormat::StlBinary, &entry.mesh),
        MeshWriteFormat::StlBinary,
        "a plain triangle mesh keeps STL"
    );
}

/// STL carries geometry only, so proposing it for a colour scan discards
/// the colour. This is the `.dcm` case: the format has no writer, so the
/// proposed format comes from what the scan holds, never from a fallback
/// that could be a colourless STL.
#[test]
fn a_forced_stl_falls_back_to_ply_so_colour_is_not_thrown_away() {
    use occluview_core::MeshTexture;

    // A textured scan: the atlas and its mapping both live only in PLY.
    let Ok(scene) = exportable_scene() else {
        panic!("required test setup or expected result was missing");
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

/// A `.dcm` opened and saved.
///
/// A `.dcm`/HPS has no writer, so there is no source format to keep. The
/// format follows what the scan holds: a colour scan is offered as PLY,
/// and a geometry-only scan as STL. There is no preference that can turn a
/// colour scan into a colourless `.stl`.
#[test]
fn a_dcm_is_offered_the_format_that_carries_what_it_holds() {
    use occluview_core::MeshTexture;

    let paths = vec![PathBuf::from("/scans/upper.dcm")];

    // A textured scan must come out PLY.
    let Ok(scene) = exportable_scene() else {
        panic!("required test setup or expected result was missing");
    };
    let mut textured = (*scene.meshes()[0].mesh).clone();
    textured.set_texture(MeshTexture::new(1, 1, vec![1, 2, 3, 255]));
    let proposed =
        representable_export_format(automatic_export_format(&paths, 0, &textured), &textured);
    assert_eq!(
        proposed,
        MeshWriteFormat::PlyBinaryLittleEndian,
        "a textured .dcm must be offered PLY, not a colourless STL"
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
        automatic_export_format(&paths, 0, &coloured),
        MeshWriteFormat::PlyBinaryLittleEndian
    );

    // A scan with a mapping but no image: STL would lose the mapping too.
    let mapped = Mesh::new(
        Some("upper".to_owned()),
        vec![
            Vertex::at(Vec3::ZERO).with_uv([0.0, 1.0]),
            Vertex::at(Vec3::X).with_uv([1.0, 1.0]),
            Vertex::at(Vec3::Y).with_uv([0.0, 0.0]),
        ],
        vec![0, 1, 2],
    );
    let Ok(mapped) = mapped else { return };
    assert_eq!(
        automatic_export_format(&paths, 0, &mapped),
        MeshWriteFormat::PlyBinaryLittleEndian
    );

    // A geometry-only .dcm loses nothing as STL, so STL is what it gets.
    let Ok(plain) = exportable_scene().map(|scene| (*scene.meshes()[0].mesh).clone()) else {
        panic!("required test setup or expected result was missing");
    };
    assert!(!plain.has_vertex_colors() && !plain.has_uvs() && plain.texture().is_none());
    let proposed = representable_export_format(automatic_export_format(&paths, 0, &plain), &plain);
    assert_eq!(
        proposed,
        MeshWriteFormat::StlBinary,
        "a geometry-only scan written as STL loses nothing, so STL is right"
    );
}

/// A writable source format is kept, unless it cannot carry what the scan
/// holds: OBJ has no image, STL has no colour at all.
#[test]
fn a_source_format_that_cannot_carry_the_payload_yields_to_ply() {
    use occluview_core::MeshTexture;

    let Ok(scene) = exportable_scene() else {
        panic!("required test setup or expected result was missing");
    };
    let mut textured = (*scene.meshes()[0].mesh).clone();
    textured.set_texture(MeshTexture::new(1, 1, vec![1, 2, 3, 255]));

    assert_eq!(
        automatic_export_format(&[PathBuf::from("upper.obj")], 0, &textured),
        MeshWriteFormat::PlyBinaryLittleEndian,
        "OBJ cannot hold an atlas, so a textured scan is saved as PLY"
    );
    assert_eq!(
        automatic_export_format(&[PathBuf::from("upper.stl")], 0, &textured),
        MeshWriteFormat::PlyBinaryLittleEndian,
        "STL cannot hold an atlas either"
    );
    assert_eq!(
        automatic_export_format(&[PathBuf::from("upper.ply")], 0, &textured),
        MeshWriteFormat::PlyBinaryLittleEndian,
        "PLY already is the format that holds it"
    );

    // A vertex-coloured scan: STL yields, OBJ keeps (it carries RGB).
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
        automatic_export_format(&[PathBuf::from("upper.stl")], 0, &coloured),
        MeshWriteFormat::PlyBinaryLittleEndian
    );
    assert_eq!(
        automatic_export_format(&[PathBuf::from("upper.obj")], 0, &coloured),
        MeshWriteFormat::Obj,
        "OBJ carries vertex colour, so a colour-only scan keeps OBJ"
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
    let rendered =
        append_mesh_export_warnings("Scene saved".to_owned(), Some("UVs not included"), &locale);

    assert!(rendered.starts_with("Scene saved"));
    assert!(rendered.contains("UVs not included"));
}

#[test]
fn a_read_only_source_format_falls_to_what_the_scan_holds() -> Result<()> {
    let scene = exportable_scene()?;
    let plain = (*scene.meshes()[0].mesh).clone();
    let paths = vec![PathBuf::from("encrypted-scan.hps")];

    // A geometry-only scan from a format with no writer becomes STL.
    let format = automatic_export_format(&paths, 0, &plain);
    assert_eq!(format, MeshWriteFormat::StlBinary);
    assert_eq!(
        default_layer_export_name(&paths, &scene, 0, format),
        "encrypted-scan-edited.stl"
    );

    // The same scan with colour becomes PLY, under the same base name.
    let mut coloured = plain;
    coloured.set_texture(occluview_core::MeshTexture::new(1, 1, vec![1, 2, 3, 255]));
    let format = automatic_export_format(&paths, 0, &coloured);
    assert_eq!(format, MeshWriteFormat::PlyBinaryLittleEndian);
    assert_eq!(
        default_layer_export_name(&paths, &scene, 0, format),
        "encrypted-scan-edited.ply"
    );
    Ok(())
}

#[test]
fn derived_layer_uses_its_neighbour_for_folder_and_format() {
    let paths = vec![PathBuf::new(), PathBuf::from("/case/scans/upper.obj")];

    let Ok(scene) = exportable_scene() else {
        panic!("required test setup or expected result was missing");
    };
    let plain = (*scene.meshes()[0].mesh).clone();
    assert_eq!(
        automatic_export_format(&paths, 0, &plain),
        MeshWriteFormat::Obj,
        "a layer with no file of its own takes its neighbour's format"
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
    let Ok(scene) = exportable_scene() else {
        panic!("required test setup or expected result was missing");
    };
    let plain = (*scene.meshes()[0].mesh).clone();
    assert_eq!(
        automatic_export_format(&paths, 1, &plain),
        MeshWriteFormat::PlyBinaryLittleEndian,
        "the layer's own .ply is kept"
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
    let Ok(scene) = exportable_scene() else {
        panic!("required test setup or expected result was missing");
    };
    let plain = (*scene.meshes()[0].mesh).clone();
    assert_eq!(
        automatic_export_format(&paths, 2, &plain),
        MeshWriteFormat::StlBinary,
        "the split part takes the source it descends from"
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
        normalize_layer_export_path(PathBuf::from("edited.stl.stl"), MeshWriteFormat::StlBinary),
        PathBuf::from("edited.stl")
    );
    assert_eq!(
        normalize_layer_export_path(PathBuf::from("edited.STL.StL"), MeshWriteFormat::StlBinary),
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
        normalize_layer_export_path(PathBuf::from("edited.stl.obj"), MeshWriteFormat::StlBinary),
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
        // this is a turn and a shift, not just a translation.
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
/// operator who exported the arch they did not move can see that.
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
