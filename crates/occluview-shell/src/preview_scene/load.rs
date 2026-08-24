use super::render::prepared_scene_sources;
use super::PreviewSceneState;
use crate::offscreen_factory::shared_shell_offscreen;
use crate::ShellError;
use occluview_core::{Camera, Mesh, Scene, SceneMesh};
use occluview_formats::dispatch::{
    dispatch_by_kind_with_key_provider, read_file_with_key_provider,
};
use occluview_formats::hps::RuntimeHpsKeyProvider;
use occluview_formats::FormatKind;
use occluview_thumbnail::fast_thumb::{
    try_read_fast_thumbnail_mesh_for_kind, try_read_fast_thumbnail_mesh_from_file,
};
use occluview_thumbnail::thumbnail_format::infer_thumbnail_format;
use std::path::Path;

const PREVIEW_FULL_FIDELITY_SURFACE_FILE_BYTES: u64 = 32 * 1024 * 1024;
const PREVIEW_FULL_FIDELITY_STL_FILE_BYTES: u64 = 128 * 1024 * 1024;
const PREVIEW_FOV_RADIANS: f32 = 45.0_f32.to_radians();

impl PreviewSceneState {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn from_file(path: &Path) -> Result<Self, ShellError> {
        let metadata = std::fs::metadata(path).map_err(|error| {
            ShellError::Win32(format!(
                "preview metadata read failed for {}: {error}",
                path.display()
            ))
        })?;
        let mesh = load_preview_mesh_from_file(path, metadata.len())?;
        Self::from_scene(single_mesh_scene(mesh))
    }

    pub(crate) fn from_bytes(extension: Option<&str>, bytes: &[u8]) -> Result<Self, ShellError> {
        let kind = infer_thumbnail_format(extension, bytes)?;
        let mesh = load_preview_mesh_from_bytes_kind(kind, bytes)?;
        Self::from_scene(single_mesh_scene(mesh))
    }

    fn from_scene(scene: Scene) -> Result<Self, ShellError> {
        #[cfg(test)]
        let _guard = crate::acquire_render_test_guard();

        let bbox = scene.bbox();
        let camera = Camera::default().frame_occlusal(bbox, PREVIEW_FOV_RADIANS);
        let offscreen = shared_shell_offscreen()?;
        let prepared_scene = offscreen.prepare_scene(&prepared_scene_sources(&scene));
        Ok(Self {
            scene,
            camera,
            offscreen,
            prepared_scene,
        })
    }
}

fn single_mesh_scene(mesh: Mesh) -> Scene {
    let mut scene = Scene::new();
    scene.add(SceneMesh::new(mesh));
    scene
}

fn load_preview_mesh_from_file(path: &Path, byte_len: u64) -> Result<Mesh, ShellError> {
    if preview_prefers_full_fidelity_parse(path, byte_len) {
        match read_file_with_key_provider(path, &RuntimeHpsKeyProvider) {
            Ok(mesh) => return Ok(mesh),
            Err(error) => {
                if let Some(mesh) = try_read_fast_thumbnail_mesh_from_file(path) {
                    return Ok(mesh);
                }
                return Err(error.into());
            }
        }
    }

    if let Some(mesh) = try_read_fast_thumbnail_mesh_from_file(path) {
        return Ok(mesh);
    }

    Ok(read_file_with_key_provider(path, &RuntimeHpsKeyProvider)?)
}

fn load_preview_mesh_from_bytes_kind(kind: FormatKind, bytes: &[u8]) -> Result<Mesh, ShellError> {
    if preview_prefers_full_fidelity_kind(kind, bytes.len() as u64) {
        match dispatch_by_kind_with_key_provider(kind, bytes, &RuntimeHpsKeyProvider) {
            Ok(mesh) => return Ok(mesh),
            Err(error) => {
                if let Some(mesh) = try_read_fast_thumbnail_mesh_for_kind(kind, bytes) {
                    return Ok(mesh);
                }
                return Err(error.into());
            }
        }
    }

    if let Some(mesh) = try_read_fast_thumbnail_mesh_for_kind(kind, bytes) {
        return Ok(mesh);
    }

    Ok(dispatch_by_kind_with_key_provider(
        kind,
        bytes,
        &RuntimeHpsKeyProvider,
    )?)
}

fn preview_prefers_full_fidelity_parse(path: &Path, byte_len: u64) -> bool {
    let Some(extension) = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
    else {
        return false;
    };

    let byte_limit = match extension.as_str() {
        "stl" => PREVIEW_FULL_FIDELITY_STL_FILE_BYTES,
        "ply" | "obj" => PREVIEW_FULL_FIDELITY_SURFACE_FILE_BYTES,
        _ => return true,
    };

    byte_len <= byte_limit
}

fn preview_prefers_full_fidelity_kind(kind: FormatKind, byte_len: u64) -> bool {
    let byte_limit = match kind {
        FormatKind::Stl => PREVIEW_FULL_FIDELITY_STL_FILE_BYTES,
        FormatKind::Obj | FormatKind::Ply => PREVIEW_FULL_FIDELITY_SURFACE_FILE_BYTES,
        _ => return true,
    };

    byte_len <= byte_limit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview_scene::test_support::{
        noisy_obj_with_early_faces, obj_with_early_faces, valid_obj_tiles,
    };

    #[test]
    fn preview_scene_detects_stream_backed_obj_without_extension() {
        let obj = br"# stream-backed OBJ from Explorer
v 0 0 0 255 220 180
v 1 0 0 255 220 180
v 0 1 0 255 220 180
f 1 2 3
";

        let state = PreviewSceneState::from_bytes(None, obj);

        assert!(
            state.is_ok(),
            "preview streams should use shell format inference for OBJ text instead of falling back to placeholder"
        );
    }

    #[test]
    fn preview_scene_uses_canonical_obj_mesh_inside_fidelity_budget() {
        let obj = valid_obj_tiles(2 * 1024 * 1024);
        assert!(
            obj.len() as u64 <= PREVIEW_FULL_FIDELITY_SURFACE_FILE_BYTES,
            "fixture should stay inside preview's full-fidelity OBJ budget"
        );
        let full_mesh =
            dispatch_by_kind_with_key_provider(FormatKind::Obj, &obj, &RuntimeHpsKeyProvider)
                .expect("canonical OBJ parser should load valid preview fixture");

        let preview_mesh = load_preview_mesh_from_bytes_kind(FormatKind::Obj, &obj)
            .expect("preview OBJ mesh should load");

        assert_eq!(
            preview_mesh.triangle_count(),
            full_mesh.triangle_count(),
            "valid OBJ live preview should keep full mesh fidelity instead of showing a sparse placeholder-like surrogate"
        );
    }

    #[test]
    fn preview_scene_recovers_800kb_obj_through_fast_surrogate() {
        let obj = obj_with_early_faces(800 * 1024);
        assert!(
            obj.len() >= 800 * 1024,
            "fixture should cover the reported small OBJ size; got {} bytes",
            obj.len()
        );

        let state = PreviewSceneState::from_bytes(Some("obj"), &obj);

        assert!(
            state.is_ok(),
            "preview should recover OBJ geometry through the fast surrogate before falling back to placeholder"
        );
    }

    #[test]
    fn preview_scene_recovers_noisy_800kb_obj_stream_without_extension_hint() {
        let obj = noisy_obj_with_early_faces(800 * 1024);
        assert!(
            obj.len() >= 800 * 1024,
            "fixture should cover the reported small OBJ size; got {} bytes",
            obj.len()
        );

        let state = PreviewSceneState::from_bytes(None, &obj);

        assert!(
            state.is_ok(),
            "Explorer preview streams should recover noisy OBJ geometry without relying on a file extension hint"
        );
    }
}
