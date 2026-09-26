use crate::app_files::path_display_name;
use occluview_core::SceneMesh;
use std::path::PathBuf;

pub(crate) fn layer_hover(
    paths: &[PathBuf],
    entry: &SceneMesh,
    index: usize,
    locale: &crate::i18n::LocaleManager,
) -> String {
    if let Some(path) = paths.get(index).filter(|path| !path.as_os_str().is_empty()) {
        return path.display().to_string();
    }
    layer_label(paths, entry, index, locale)
}

pub(crate) fn layer_label(
    paths: &[PathBuf],
    entry: &SceneMesh,
    index: usize,
    locale: &crate::i18n::LocaleManager,
) -> String {
    if let Some(path) = paths.get(index).filter(|path| !path.as_os_str().is_empty()) {
        return path_display_name(path).unwrap_or_else(|| path.display().to_string());
    }
    if let Some(name) = entry.mesh.name().filter(|name| !name.is_empty()) {
        return name.to_owned();
    }
    locale.tr_with("layer-unnamed", &[("n", &(index + 1).to_string())])
}

/// ASCII fallback stem for default export filenames. Not localized: a
/// default filename must survive any filesystem locale
/// (see the `layer-unnamed` catalog key for the operator-visible name).
pub(crate) fn ascii_layer_stem(index: usize) -> String {
    format!("layer-{}", index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use occluview_core::Mesh;

    fn english() -> crate::i18n::LocaleManager {
        crate::i18n::LocaleManager::for_tests()
    }

    #[test]
    fn layer_label_prefers_file_name() {
        let entry = SceneMesh::new(Mesh::empty());
        let paths = vec![PathBuf::from(r"C:\cases\lower_scan.glb")];

        assert_eq!(layer_label(&paths, &entry, 0, &english()), "lower_scan.glb");
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

        assert_eq!(layer_label(&[], &named, 0, &english()), "Upper arch");
        assert_eq!(
            layer_label(&[], &unnamed, 1, &english()),
            // Interpolated numbers carry Fluent bidi isolation marks.
            "layer \u{2068}2\u{2069}"
        );
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

        assert_eq!(layer_label(&placeholder, &named, 0, &english()), "Part B");
        assert_eq!(layer_hover(&placeholder, &named, 0, &english()), "Part B");
    }
}
