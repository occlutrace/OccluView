//! Windows Jump List item model.
//!
//! The platform adapter lives in `occluview-app`; this module keeps the
//! recent-scene to shell-link contract testable without Win32.

use crate::{RecentEntry, RecentFiles};
use std::path::Path;

/// A custom Jump List destination for one recent scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpListItem {
    /// Text shown in the Jump List.
    pub title: String,
    /// Multi-line full-path tooltip.
    pub tooltip: String,
    /// Windows command-line arguments that reopen the scene.
    pub arguments: String,
}

impl RecentFiles {
    /// Build custom Jump List items from the app-owned recent-scene list.
    #[must_use]
    pub fn jump_list_items(&self, limit: usize) -> Vec<JumpListItem> {
        self.entries()
            .iter()
            .take(limit)
            .filter_map(jump_list_item)
            .collect()
    }
}

fn jump_list_item(entry: &RecentEntry) -> Option<JumpListItem> {
    let primary = entry.primary_path()?;
    Some(JumpListItem {
        title: scene_title(entry, primary),
        tooltip: entry
            .paths()
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        arguments: windows_command_line_for_paths(entry.paths()),
    })
}

fn scene_title(entry: &RecentEntry, primary: &Path) -> String {
    let primary = display_file_name(primary);
    let extra_count = entry.paths().len().saturating_sub(1);
    if extra_count == 0 {
        primary
    } else {
        format!("{primary} +{extra_count}")
    }
}

fn display_file_name(path: &Path) -> String {
    let display = path.display().to_string();
    let name = display
        .rsplit(['\\', '/'])
        .next()
        .filter(|name| !name.is_empty());
    match name {
        Some(name) => name.to_owned(),
        None => display,
    }
}

fn windows_command_line_for_paths(paths: &[std::path::PathBuf]) -> String {
    paths
        .iter()
        .map(|path| quote_windows_arg(&path.as_os_str().to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_windows_arg(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0;
    for ch in arg.chars() {
        if ch == '\\' {
            backslashes += 1;
            continue;
        }
        if ch == '"' {
            push_backslashes(&mut out, backslashes * 2 + 1);
            out.push('"');
            backslashes = 0;
            continue;
        }
        push_backslashes(&mut out, backslashes);
        backslashes = 0;
        out.push(ch);
    }
    push_backslashes(&mut out, backslashes * 2);
    out.push('"');
    out
}

fn push_backslashes(out: &mut String, count: usize) {
    for _ in 0..count {
        out.push('\\');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn jump_list_items_keep_recent_order_and_grouped_scene_arguments() {
        let mut recent = RecentFiles::new(4);
        recent.push_paths(&[
            PathBuf::from(r"C:\cases\upper scan.stl"),
            PathBuf::from(r"C:\cases\lower scan.ply"),
        ]);
        recent.push(PathBuf::from(r"C:\cases\single.obj"));

        let items = recent.jump_list_items(4);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "single.obj");
        assert_eq!(items[0].arguments, r#""C:\cases\single.obj""#);
        assert_eq!(items[1].title, "upper scan.stl +1");
        assert_eq!(
            items[1].arguments,
            r#""C:\cases\upper scan.stl" "C:\cases\lower scan.ply""#
        );
        assert_eq!(
            items[1].tooltip,
            r"C:\cases\upper scan.stl
C:\cases\lower scan.ply"
        );
    }

    #[test]
    fn jump_list_items_respect_limit() {
        let mut recent = RecentFiles::new(8);
        recent.push("a.stl");
        recent.push("b.stl");
        recent.push("c.stl");

        let items = recent.jump_list_items(2);

        assert_eq!(
            items
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>(),
            vec!["c.stl", "b.stl"]
        );
    }

    #[test]
    fn windows_command_line_quote_escapes_quotes_and_trailing_backslashes() {
        let quoted = quote_windows_arg(r#"C:\cases\scan "A"\"#);

        assert_eq!(quoted, r#""C:\cases\scan \"A\"\\""#);
    }
}
