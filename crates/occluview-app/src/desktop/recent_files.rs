//! Recent-file list logic for the desktop app.

use serde::{Deserialize, Serialize};
#[cfg(any(unix, windows))]
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const NATIVE_V2_HEADER: &str = "\0occluview-recent-v2:";
const NATIVE_V2_VERSION: u8 = 2;

#[derive(Serialize, Deserialize)]
#[serde(tag = "platform", deny_unknown_fields)]
enum NativeRecentFilesV2 {
    #[cfg(unix)]
    #[serde(rename = "unix_bytes")]
    UnixBytes {
        version: u8,
        entries: Vec<Vec<Vec<u8>>>,
    },
    #[cfg(windows)]
    #[serde(rename = "windows_utf16_le")]
    WindowsUtf16Le {
        version: u8,
        entries: Vec<Vec<Vec<u16>>>,
    },
    #[cfg(not(any(unix, windows)))]
    #[serde(rename = "other_encoded")]
    OtherEncoded {
        version: u8,
        entries: Vec<Vec<Vec<u8>>>,
    },
}

/// A stored list can be newer than this binary. Callers must leave that file
/// untouched until the operator explicitly clears recent files.
pub(crate) enum RecentFilesDecode {
    Supported(RecentFiles),
    Unsupported,
}

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
    /// Construct an empty recent-file list.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Decode the supported legacy and native formats without treating an
    /// unknown NUL-prefixed envelope as an empty list that may be saved over.
    pub(crate) fn deserialize_storage(limit: usize, stored: &str) -> RecentFilesDecode {
        if stored.starts_with('\0') {
            let Some(payload) = stored.strip_prefix(NATIVE_V2_HEADER) else {
                return RecentFilesDecode::Unsupported;
            };
            return decode_native_v2(limit, payload)
                .map_or(RecentFilesDecode::Unsupported, RecentFilesDecode::Supported);
        }

        RecentFilesDecode::Supported(Self::deserialize_legacy(limit, stored))
    }

    fn deserialize_legacy(limit: usize, stored: &str) -> Self {
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
    ///
    /// The single-path convenience over [`RecentFiles::push_paths`]. The app
    /// opens scenes, which are lists, so only tests reach for this one.
    #[cfg(test)]
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
    pub fn serialize(&self) -> Result<String, serde_json::Error> {
        if self
            .entries
            .iter()
            .flat_map(RecentEntry::paths)
            .any(|path| path.to_str().is_none())
        {
            return self.serialize_native_v2();
        }

        Ok(self.serialize_legacy())
    }

    fn serialize_legacy(&self) -> String {
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

    fn serialize_native_v2(&self) -> Result<String, serde_json::Error> {
        // V1 remains byte-identical for all-Unicode paths. The NUL-prefixed
        // V2 envelope begins with a byte no real filesystem path can contain.
        // Older readers cannot restore it as a path, but may overwrite it on
        // a later save; use Clear Recent before rolling back to an older build.
        #[cfg(unix)]
        let envelope = NativeRecentFilesV2::UnixBytes {
            version: NATIVE_V2_VERSION,
            entries: self
                .entries
                .iter()
                .map(|entry| {
                    entry
                        .paths()
                        .iter()
                        .map(|path| {
                            use std::os::unix::ffi::OsStrExt;
                            path.as_os_str().as_bytes().to_vec()
                        })
                        .collect()
                })
                .collect(),
        };
        #[cfg(windows)]
        let envelope = NativeRecentFilesV2::WindowsUtf16Le {
            version: NATIVE_V2_VERSION,
            entries: self
                .entries
                .iter()
                .map(|entry| {
                    entry
                        .paths()
                        .iter()
                        .map(|path| {
                            use std::os::windows::ffi::OsStrExt;
                            path.as_os_str().encode_wide().collect()
                        })
                        .collect()
                })
                .collect(),
        };
        #[cfg(not(any(unix, windows)))]
        let envelope = NativeRecentFilesV2::OtherEncoded {
            version: NATIVE_V2_VERSION,
            entries: self
                .entries
                .iter()
                .map(|entry| {
                    entry
                        .paths()
                        .iter()
                        .map(|path| path.as_os_str().as_encoded_bytes().to_vec())
                        .collect()
                })
                .collect(),
        };

        let mut out = String::with_capacity(NATIVE_V2_HEADER.len() + self.entries.len() * 32);
        out.push_str(NATIVE_V2_HEADER);
        out.push_str(&serde_json::to_string(&envelope)?);
        Ok(out)
    }
}

fn decode_native_v2(limit: usize, payload: &str) -> Option<RecentFiles> {
    let envelope: NativeRecentFilesV2 = serde_json::from_str(payload).ok()?;
    match envelope {
        #[cfg(unix)]
        NativeRecentFilesV2::UnixBytes { version, entries }
            if version == NATIVE_V2_VERSION =>
        {
            decode_native_groups(limit, entries, |units| {
                use std::os::unix::ffi::OsStringExt;
                Some(PathBuf::from(OsString::from_vec(units.to_vec())))
            })
        }
        #[cfg(windows)]
        NativeRecentFilesV2::WindowsUtf16Le { version, entries }
            if version == NATIVE_V2_VERSION =>
        {
            decode_native_groups(limit, entries, |units| {
                use std::os::windows::ffi::OsStringExt;
                Some(PathBuf::from(OsString::from_wide(units)))
            })
        }
        #[cfg(not(any(unix, windows)))]
        NativeRecentFilesV2::OtherEncoded { version, entries }
            if version == NATIVE_V2_VERSION =>
        {
            decode_native_groups(limit, entries, |units| {
                Some(PathBuf::from(std::str::from_utf8(units).ok()?))
            })
        }
        _ => None,
    }
}

fn decode_native_groups<T>(
    limit: usize,
    entries: Vec<Vec<Vec<T>>>,
    decode_path: impl Fn(&[T]) -> Option<PathBuf>,
) -> Option<RecentFiles> {
    if entries.is_empty() {
        return None;
    }

    let mut recent = RecentFiles::new(limit);
    for encoded_paths in entries {
        if encoded_paths.is_empty() {
            return None;
        }
        let mut paths = Vec::with_capacity(encoded_paths.len());
        for units in encoded_paths {
            if units.is_empty() {
                return None;
            }
            paths.push(decode_path(&units)?);
        }
        let entry = RecentEntry::from_paths(&paths)?;
        if recent.entries.len() < recent.limit
            && !recent.entries.iter().any(|existing| existing == &entry)
        {
            recent.entries.push(entry);
        }
    }
    Some(recent)
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
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::{RecentFiles, RecentFilesDecode};
    use std::path::PathBuf;

    fn entry_paths(recent: &RecentFiles) -> Vec<Vec<PathBuf>> {
        recent
            .entries()
            .iter()
            .map(|entry| entry.paths().to_vec())
            .collect()
    }

    fn deserialize_supported(limit: usize, stored: &str) -> RecentFiles {
        match RecentFiles::deserialize_storage(limit, stored) {
            RecentFilesDecode::Supported(recent) => recent,
            RecentFilesDecode::Unsupported => panic!("test input must use a supported format"),
        }
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

        let serialized = recent.serialize().expect("legacy serialization is infallible");
        assert_eq!(
            serialized.as_bytes(),
            b"with%0Anewline.obj\nC:\\cases\\lower%25scan.stl"
        );
        assert_eq!(
            serialized,
            "with%0Anewline.obj\nC:\\cases\\lower%25scan.stl"
        );
        assert!(!serialized.contains("with\nnewline"));

        let restored = deserialize_supported(8, &serialized);
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

        let serialized = recent.serialize().expect("legacy serialization is infallible");
        assert!(serialized.contains('\t'));

        let restored = deserialize_supported(8, &serialized);
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
        let restored = deserialize_supported(3, "a.stl\n\nb.stl\na.stl\nc.stl\nd.stl\n");

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
    fn deserialize_accepts_single_path_lines_as_single_entry_scenes() {
        let restored = deserialize_supported(3, "a.stl\nb.stl\nc.stl\n");
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
        assert_eq!(recent.serialize().expect("legacy serialization is infallible"), "");
    }

    #[test]
    fn ordinary_filename_resembling_the_v2_header_stays_legacy_v1() {
        let mut recent = RecentFiles::new(2);
        recent.push("occluview-recent-v2");

        let serialized = recent.serialize().expect("legacy serialization is infallible");
        assert_eq!(serialized, "occluview-recent-v2");
        let restored = RecentFiles::deserialize_storage(2, &serialized);
        let RecentFilesDecode::Supported(restored) = restored else {
            panic!("legacy filename was mistaken for a native envelope");
        };
        assert_eq!(entry_paths(&restored), vec![vec![PathBuf::from("occluview-recent-v2")]]);
    }

    #[test]
    fn unknown_native_version_is_not_decoded_as_an_empty_writable_list() {
        let decoded = RecentFiles::deserialize_storage(8, "\0occluview-recent-v3:abcd");
        assert!(matches!(decoded, RecentFilesDecode::Unsupported));
    }

    #[test]
    fn future_v2_version_and_malformed_envelopes_remain_unsupported() {
        let platform = if cfg!(unix) {
            "unix_bytes"
        } else if cfg!(windows) {
            "windows_utf16_le"
        } else {
            "other_encoded"
        };
        let future_version = format!(
            "{NATIVE_V2_HEADER}{{\"platform\":\"{platform}\",\"version\":3,\"entries\":[[[97]]]}}"
        );
        assert!(matches!(
            RecentFiles::deserialize_storage(8, &future_version),
            RecentFilesDecode::Unsupported
        ));

        for entries in ["[]", "[[]]", "[[[]]]"] {
            let malformed = format!(
                "{NATIVE_V2_HEADER}{{\"platform\":\"{platform}\",\"version\":2,\"entries\":{entries}}}"
            );
            assert!(matches!(
                RecentFiles::deserialize_storage(8, &malformed),
                RecentFilesDecode::Unsupported
            ));
        }

        let unknown_field = format!(
            "{NATIVE_V2_HEADER}{{\"platform\":\"{platform}\",\"version\":2,\"entries\":[[[97]]],\"future\":true}}"
        );
        assert!(matches!(
            RecentFiles::deserialize_storage(8, &unknown_field),
            RecentFilesDecode::Unsupported
        ));

        let trailing_content = format!(
            "{NATIVE_V2_HEADER}{{\"platform\":\"{platform}\",\"version\":2,\"entries\":[[[97]]]}}trailing"
        );
        assert!(matches!(
            RecentFiles::deserialize_storage(8, &trailing_content),
            RecentFilesDecode::Unsupported
        ));
    }

    #[cfg(unix)]
    #[test]
    fn native_unix_bytes_round_trip_without_splitting_a_scene_group() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let native_path = PathBuf::from(OsString::from_vec(vec![
            b'u', b'p', b'p', b'e', b'r', b'-', 0xff, b'.', b's', b't', b'l',
        ]));
        let unicode_path = PathBuf::from("lower.stl");
        let mut recent = RecentFiles::new(3);
        recent.push_paths(&[native_path.clone(), unicode_path.clone()]);
        recent.push("single.obj");

        let serialized = recent.serialize().expect("native serialization should succeed");
        assert!(serialized.starts_with("\0occluview-recent-v2:"));
        assert!(serialized.contains("\"platform\":\"unix_bytes\""));
        assert!(serialized.contains("\"version\":2"));
        assert!(!serialized.contains('\n'));
        let RecentFilesDecode::Supported(restored) =
            RecentFiles::deserialize_storage(3, &serialized)
        else {
            panic!("native Unix path was not accepted by the v2 decoder");
        };
        assert_eq!(
            entry_paths(&restored),
            vec![
                vec![PathBuf::from("single.obj")],
                vec![native_path, unicode_path]
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn native_windows_utf16_round_trip_without_splitting_a_scene_group() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;

        let native_path = PathBuf::from(OsString::from_wide(&[
            b'u' as u16,
            b'p' as u16,
            0xd800,
            b'.' as u16,
            b's' as u16,
            b't' as u16,
            b'l' as u16,
        ]));
        let unicode_path = PathBuf::from(r"C:\cases\lower.stl");
        let mut recent = RecentFiles::new(3);
        recent.push_paths(&[native_path.clone(), unicode_path.clone()]);
        recent.push("single.obj");

        let serialized = recent.serialize().expect("native serialization should succeed");
        assert!(serialized.starts_with("\0occluview-recent-v2:"));
        assert!(serialized.contains("\"platform\":\"windows_utf16_le\""));
        assert!(serialized.contains("\"version\":2"));
        assert!(!serialized.contains('\n'));
        let RecentFilesDecode::Supported(restored) =
            RecentFiles::deserialize_storage(3, &serialized)
        else {
            panic!("native Windows path was not accepted by the v2 decoder");
        };
        assert_eq!(
            entry_paths(&restored),
            vec![
                vec![PathBuf::from("single.obj")],
                vec![native_path, unicode_path]
            ]
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn native_serialization_keeps_scene_pairs_whole_at_the_entry_limit() {
        #[cfg(unix)]
        let native_path = {
            use std::ffi::OsString;
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(OsString::from_vec(b"scene-\xff.stl".to_vec()))
        };
        #[cfg(windows)]
        let native_path = {
            use std::ffi::OsString;
            use std::os::windows::ffi::OsStringExt;
            PathBuf::from(OsString::from_wide(&[
                b's' as u16,
                b'c' as u16,
                0xd800,
                b'.' as u16,
                b's' as u16,
                b't' as u16,
                b'l' as u16,
            ]))
        };
        let scene = vec![native_path, PathBuf::from("lower.stl")];
        let mut recent = RecentFiles::new(1);
        recent.push_paths(&scene);
        recent.push_paths(&scene);

        let serialized = recent.serialize().expect("native serialization should succeed");
        let restored = deserialize_supported(1, &serialized);

        assert_eq!(entry_paths(&restored), vec![scene]);
    }
}
