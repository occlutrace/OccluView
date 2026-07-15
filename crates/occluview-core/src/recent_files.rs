//! Recent-file list logic for the desktop app.

use std::path::{Path, PathBuf};

/// One recent scene entry: one or more paths opened together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentEntry {
    paths: Vec<PathBuf>,
}

impl RecentEntry {
    #[must_use]
    fn from_paths(paths: &[PathBuf]) -> Option<Self> {
        let paths: Vec<PathBuf> = paths
            .iter()
            .filter(|path| !path.as_os_str().is_empty())
            .cloned()
            .collect();
        if paths.is_empty() {
            None
        } else {
            Some(Self { paths })
        }
    }

    /// Borrow paths in their scene order.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// The first path in the scene, used as the compact menu label anchor.
    #[must_use]
    pub fn primary_path(&self) -> Option<&Path> {
        self.paths.first().map(PathBuf::as_path)
    }
}

/// Most-recent-first list of mesh files opened by the desktop app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentFiles {
    entries: Vec<RecentEntry>,
    limit: usize,
}

impl RecentFiles {
    /// Default number of entries shown in the app.
    pub const DEFAULT_LIMIT: usize = 8;

    /// Construct an empty recent-file list.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Restore a recent-file list from the app storage string.
    ///
    /// Lines are ordered most-recent-first. Duplicate later lines are ignored
    /// so corrupt or manually edited storage cannot shuffle the list.
    #[must_use]
    pub fn deserialize(limit: usize, stored: &str) -> Self {
        let mut recent = Self::new(limit);
        for line in stored.lines() {
            if recent.entries.len() >= recent.limit {
                break;
            }
            let Some(entry) = decode_entry(line) else {
                continue;
            };
            if !recent.entries.iter().any(|existing| existing == &entry) {
                recent.entries.push(entry);
            }
        }
        recent
    }

    /// Add a path as the most recent entry.
    pub fn push<P: Into<PathBuf>>(&mut self, path: P) {
        self.push_paths(&[path.into()]);
    }

    /// Add a scene entry as the most recent item.
    pub fn push_paths(&mut self, paths: &[PathBuf]) {
        let Some(entry) = RecentEntry::from_paths(paths) else {
            return;
        };

        self.entries.retain(|existing| existing != &entry);
        self.entries.insert(0, entry);
        self.entries.truncate(self.limit);
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Borrow scene entries in most-recent-first order.
    #[must_use]
    pub fn entries(&self) -> &[RecentEntry] {
        &self.entries
    }

    /// Whether the list has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Convert to the compact app-storage format.
    #[must_use]
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for (i, entry) in self.entries.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            let mut first = true;
            for path in entry.paths() {
                if !first {
                    out.push('\t');
                }
                out.push_str(&encode_path(path));
                first = false;
            }
        }
        out
    }
}

fn encode_path(path: &Path) -> String {
    let raw = path.as_os_str().to_string_lossy();
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            '\t' => out.push_str("%09"),
            _ => out.push(ch),
        }
    }
    out
}

fn decode_entry(line: &str) -> Option<RecentEntry> {
    let paths: Vec<PathBuf> = line.split('\t').filter_map(decode_path).collect();
    RecentEntry::from_paths(&paths)
}

fn decode_path(line: &str) -> Option<PathBuf> {
    if line.is_empty() {
        return None;
    }

    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }

        let Some(first) = chars.next() else {
            out.push('%');
            break;
        };
        let Some(second) = chars.next() else {
            out.push('%');
            out.push(first);
            break;
        };

        if let Some(decoded) = decode_escape(first, second) {
            out.push(decoded);
        } else {
            out.push('%');
            out.push(first);
            out.push(second);
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(PathBuf::from(out))
    }
}

fn decode_escape(first: char, second: char) -> Option<char> {
    match (first, second) {
        ('2', '5') => Some('%'),
        ('0', '9') => Some('\t'),
        ('0', 'A' | 'a') => Some('\n'),
        ('0', 'D' | 'd') => Some('\r'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::RecentFiles;
    use std::path::PathBuf;

    fn entry_paths(recent: &RecentFiles) -> Vec<Vec<PathBuf>> {
        recent
            .entries()
            .iter()
            .map(|entry| entry.paths().to_vec())
            .collect()
    }

    #[test]
    fn push_promotes_existing_path_and_caps_length() {
        let mut recent = RecentFiles::new(3);

        recent.push("a.stl");
        recent.push("b.stl");
        recent.push("c.stl");
        recent.push("a.stl");
        recent.push("d.stl");

        let paths = entry_paths(&recent);
        assert_eq!(
            paths,
            vec![
                vec![PathBuf::from("d.stl")],
                vec![PathBuf::from("a.stl")],
                vec![PathBuf::from("c.stl")]
            ]
        );
    }

    #[test]
    fn push_paths_keeps_scene_grouped_and_promotes_existing_entry() {
        let mut recent = RecentFiles::new(3);

        recent.push_paths(&[PathBuf::from("upper.stl"), PathBuf::from("lower.stl")]);
        recent.push("single.obj");
        recent.push_paths(&[PathBuf::from("upper.stl"), PathBuf::from("lower.stl")]);
        recent.push_paths(&[PathBuf::from("scan-a.ply"), PathBuf::from("scan-b.ply")]);

        let entries = entry_paths(&recent);
        assert_eq!(
            entries,
            vec![
                vec![PathBuf::from("scan-a.ply"), PathBuf::from("scan-b.ply")],
                vec![PathBuf::from("upper.stl"), PathBuf::from("lower.stl")],
                vec![PathBuf::from("single.obj")]
            ]
        );
    }

    #[test]
    fn serialized_round_trip_preserves_order_and_escaped_paths() {
        let mut recent = RecentFiles::new(8);
        recent.push(r"C:\cases\lower%scan.stl");
        recent.push("with\nnewline.obj");

        let serialized = recent.serialize();
        assert!(!serialized.contains("with\nnewline"));

        let restored = RecentFiles::deserialize(8, &serialized);
        assert_eq!(entry_paths(&restored), entry_paths(&recent));
    }

    #[test]
    fn serialized_round_trip_preserves_grouped_scene_entries() {
        let mut recent = RecentFiles::new(8);
        recent.push_paths(&[
            PathBuf::from(r"C:\cases\upper scan.stl"),
            PathBuf::from(r"C:\cases\lower\tab%scan.ply"),
        ]);
        recent.push("single.obj");

        let serialized = recent.serialize();
        assert!(serialized.contains('\t'));

        let restored = RecentFiles::deserialize(8, &serialized);
        let entries = entry_paths(&restored);
        assert_eq!(
            entries,
            vec![
                vec![PathBuf::from("single.obj")],
                vec![
                    PathBuf::from(r"C:\cases\upper scan.stl"),
                    PathBuf::from(r"C:\cases\lower\tab%scan.ply")
                ],
            ]
        );
    }

    #[test]
    fn deserialize_deduplicates_and_ignores_empty_lines() {
        let restored = RecentFiles::deserialize(3, "a.stl\n\nb.stl\na.stl\nc.stl\nd.stl\n");

        let paths = entry_paths(&restored);
        assert_eq!(
            paths,
            vec![
                vec![PathBuf::from("a.stl")],
                vec![PathBuf::from("b.stl")],
                vec![PathBuf::from("c.stl")]
            ]
        );
    }

    #[test]
    fn deserialize_accepts_legacy_single_path_lines_as_single_entry_scenes() {
        let restored = RecentFiles::deserialize(3, "a.stl\nb.stl\nc.stl\n");
        let entries = entry_paths(&restored);

        assert_eq!(
            entries,
            vec![
                vec![PathBuf::from("a.stl")],
                vec![PathBuf::from("b.stl")],
                vec![PathBuf::from("c.stl")]
            ]
        );
    }

    #[test]
    fn clear_removes_all_entries() {
        let mut recent = RecentFiles::new(3);
        recent.push("a.stl");
        recent.push("b.stl");

        recent.clear();

        assert!(recent.is_empty());
        assert_eq!(recent.serialize(), "");
    }
}
