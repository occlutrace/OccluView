use crate::app_files::path_display_name;
use occluview_core::SceneMesh;
use std::path::PathBuf;

pub(crate) fn layer_hover(paths: &[PathBuf], entry: &SceneMesh, index: usize) -> String {
    if let Some(path) = paths.get(index).filter(|path| !path.as_os_str().is_empty()) {
        return path.display().to_string();
    }
    layer_label(paths, entry, index)
}

pub(crate) fn layer_label(paths: &[PathBuf], entry: &SceneMesh, index: usize) -> String {
    if let Some(path) = paths.get(index).filter(|path| !path.as_os_str().is_empty()) {
        return path_display_name(path).unwrap_or_else(|| path.display().to_string());
    }
    if let Some(name) = entry.mesh.name().filter(|name| !name.is_empty()) {
        return name.to_owned();
    }
    format!("Layer {}", index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_core::Mesh;

    #[test]
    fn layer_label_prefers_file_name() {
        let entry = SceneMesh::new(Mesh::empty());
        let paths = vec![PathBuf::from(r"C:\cases\lower_scan.glb")];

        assert_eq!(layer_label(&paths, &entry, 0), "lower_scan.glb");
    }

    #[test]
    fn layer_label_falls_back_to_mesh_name_then_index() {
        let named_mesh_result = Mesh::new(Some("Upper arch".into()), vec![], vec![]);
        assert!(named_mesh_result.is_ok(), "named mesh should construct");
        let Ok(named_mesh) = named_mesh_result else {
            return;
        };
        let named = SceneMesh::new(named_mesh);
        let unnamed = SceneMesh::new(Mesh::empty());

        assert_eq!(layer_label(&[], &named, 0), "Upper arch");
        assert_eq!(layer_label(&[], &unnamed, 1), "Layer 2");
    }

    #[test]
    fn empty_placeholder_path_falls_back_to_mesh_name_and_non_empty_hover() {
        let named_mesh_result = Mesh::new(Some("Part B".into()), vec![], vec![]);
        assert!(named_mesh_result.is_ok(), "named mesh should construct");
        let Ok(named_mesh) = named_mesh_result else {
            return;
        };
        let named = SceneMesh::new(named_mesh);
        let placeholder = vec![PathBuf::new()];

        assert_eq!(layer_label(&placeholder, &named, 0), "Part B");
        assert_eq!(layer_hover(&placeholder, &named, 0), "Part B");
    }
}
