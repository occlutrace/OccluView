use occluview_core::{RecentEntry, RecentFiles};
use std::path::{Path, PathBuf};

const RECENT_FILES_FILE: &str = "recent-files.txt";

pub(crate) fn load_recent_files() -> RecentFiles {
    let Some(path) = recent_files_path() else {
        return RecentFiles::new(RecentFiles::DEFAULT_LIMIT);
    };
    match std::fs::read_to_string(path) {
        Ok(stored) => RecentFiles::deserialize(RecentFiles::DEFAULT_LIMIT, &stored),
        Err(_) => RecentFiles::new(RecentFiles::DEFAULT_LIMIT),
    }
}

pub(crate) fn recent_files_path() -> Option<PathBuf> {
    crate::app_paths::app_state_dir().map(|base| base.join(RECENT_FILES_FILE))
}

pub(crate) fn save_recent_files(recent_files: &RecentFiles) {
    let Some(path) = recent_files_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(error = ?e, "recent files directory create failed");
            return;
        }
    }
    if let Err(e) = std::fs::write(path, recent_files.serialize()) {
        tracing::warn!(error = ?e, "recent files save failed");
    }
    #[cfg(windows)]
    if let Err(e) = crate::jump_list::publish_recent_files(recent_files) {
        tracing::warn!(error = ?e, "jump list update failed");
    }
}

pub(crate) fn recent_path_label(path: &Path) -> String {
    path_display_name(path).unwrap_or_else(|| path.display().to_string())
}

pub(crate) fn recent_scene_label(entry: &RecentEntry) -> String {
    let Some(primary) = entry.primary_path() else {
        return String::from("Scene");
    };
    let primary = recent_path_label(primary);
    let extra_count = entry.paths().len().saturating_sub(1);
    if extra_count == 0 {
        primary
    } else {
        format!("{primary} +{extra_count}")
    }
}

pub(crate) fn recent_scene_hover(entry: &RecentEntry) -> String {
    entry
        .paths()
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn path_display_name(path: &Path) -> Option<String> {
    path.display()
        .to_string()
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_scene_label_uses_primary_file_and_extra_count() {
        let mut recent = RecentFiles::new(4);
        recent.push_paths(&[
            PathBuf::from(r"C:\cases\upper.stl"),
            PathBuf::from(r"C:\cases\lower.ply"),
        ]);

        let Some(entry) = recent.entries().first() else {
            return;
        };

        assert_eq!(recent_scene_label(entry), "upper.stl +1");
        assert_eq!(
            recent_scene_hover(entry),
            "C:\\cases\\upper.stl\nC:\\cases\\lower.ply"
        );
    }
}
