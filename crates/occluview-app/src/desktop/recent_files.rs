//! Recent-file list logic for the desktop app.

#[cfg(any(unix, windows))]
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const NATIVE_V2_HEADER: &str = "\0occluview-recent-v2:";
#[cfg(unix)]
const NATIVE_PATH_TAG: u8 = 1;
#[cfg(windows)]
const NATIVE_PATH_TAG: u8 = 2;
#[cfg(not(any(unix, windows)))]
const NATIVE_PATH_TAG: u8 = 3;

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
    #[must_use]
    pub fn serialize(&self) -> String {
        if self
            .entries
            .iter()
            .flat_map(RecentEntry::paths)
            .any(|path| path.to_str().is_none())
        {
            return self.serialize_native_v2();
        }

        self.serialize_legacy()
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

    fn serialize_native_v2(&self) -> String {
        // V1 is kept byte-identical whenever every path is Unicode. V2 is a
        // single-line base64url envelope so arbitrary native path units cannot
        // become separators or control characters in the legacy text format.
        // Its leading NUL cannot occur in a valid filesystem path; older
        // binaries cannot restore this history and can recover by explicitly
        // clearing it. They cannot mistake the envelope for an openable path.
        let mut bytes = vec![native_path_tag()];
        write_count(&mut bytes, self.entries.len());
        for entry in &self.entries {
            write_count(&mut bytes, entry.paths.len());
            for path in entry.paths() {
                let units = native_path_units(path);
                write_count(&mut bytes, native_unit_count(&units));
                bytes.extend_from_slice(&units);
            }
        }

        let mut out = String::with_capacity(NATIVE_V2_HEADER.len() + bytes.len() * 4 / 3 + 4);
        out.push_str(NATIVE_V2_HEADER);
        out.push_str(&encode_base64url(&bytes));
        out
    }
}

fn write_count(bytes: &mut Vec<u8>, count: usize) {
    bytes.extend_from_slice(&(count as u64).to_le_bytes());
}

fn native_unit_count(units: &[u8]) -> usize {
    #[cfg(windows)]
    {
        units.len() / 2
    }
    #[cfg(not(windows))]
    {
        units.len()
    }
}

#[cfg(unix)]
const fn native_path_tag() -> u8 {
    NATIVE_PATH_TAG
}

#[cfg(windows)]
const fn native_path_tag() -> u8 {
    NATIVE_PATH_TAG
}

#[cfg(not(any(unix, windows)))]
const fn native_path_tag() -> u8 {
    NATIVE_PATH_TAG
}

#[cfg(unix)]
fn native_path_units(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn native_path_units(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(not(any(unix, windows)))]
fn native_path_units(path: &Path) -> Vec<u8> {
    path.as_os_str().as_encoded_bytes().to_vec()
}

fn decode_native_v2(limit: usize, payload: &str) -> Option<RecentFiles> {
    let bytes = decode_base64url(payload)?;
    let mut reader = NativeReader::new(&bytes);
    if reader.read_byte()? != native_path_tag() {
        return None;
    }

    let entry_count = reader.read_count()?;
    if entry_count == 0 {
        return None;
    }
    let mut recent = RecentFiles::new(limit);
    for _ in 0..entry_count {
        let path_count = reader.read_count()?;
        if path_count == 0 {
            return None;
        }
        let mut paths = Vec::new();
        for _ in 0..path_count {
            let unit_count = reader.read_count()?;
            if unit_count == 0 {
                return None;
            }
            let byte_count = unit_count.checked_mul(native_unit_width())?;
            let units = reader.read_bytes(byte_count)?;
            paths.push(native_path_from_units(units)?);
        }
        let entry = RecentEntry::from_paths(&paths)?;
        if recent.entries.len() < recent.limit
            && !recent.entries.iter().any(|existing| existing == &entry)
        {
            recent.entries.push(entry);
        }
    }
    if !reader.is_finished() {
        return None;
    }
    Some(recent)
}

fn encode_base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut encoded = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for chunk in bytes.chunks(3) {
        let first = u32::from(chunk[0]);
        let second = chunk.get(1).copied().map_or(0, u32::from);
        let third = chunk.get(2).copied().map_or(0, u32::from);
        encoded.push(char::from(ALPHABET[((first >> 2) & 0x3f) as usize]));
        encoded.push(char::from(
            ALPHABET[(((first << 4) | (second >> 4)) & 0x3f) as usize],
        ));
        if chunk.len() > 1 {
            encoded.push(char::from(
                ALPHABET[(((second << 2) | (third >> 6)) & 0x3f) as usize],
            ));
        }
        if chunk.len() > 2 {
            encoded.push(char::from(ALPHABET[(third & 0x3f) as usize]));
        }
    }
    encoded
}

fn decode_base64url(encoded: &str) -> Option<Vec<u8>> {
    let source = encoded.as_bytes();
    if source.len() % 4 == 1 {
        return None;
    }
    let mut bytes = Vec::with_capacity(source.len() * 3 / 4 + 2);
    for chunk in source.chunks(4) {
        let first = u32::from(decode_base64url_digit(*chunk.first()?)?);
        let second = u32::from(decode_base64url_digit(*chunk.get(1)?)?);
        bytes.push(((first << 2) | (second >> 4)) as u8);
        if let Some(third) = chunk.get(2) {
            let third = u32::from(decode_base64url_digit(*third)?);
            bytes.push((((second & 0x0f) << 4) | (third >> 2)) as u8);
            if let Some(fourth) = chunk.get(3) {
                let fourth = u32::from(decode_base64url_digit(*fourth)?);
                bytes.push((((third & 0x03) << 6) | fourth) as u8);
            } else if third & 0x03 != 0 {
                return None;
            }
        } else if second & 0x0f != 0 {
            return None;
        }
    }
    Some(bytes)
}

fn decode_base64url_digit(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

struct NativeReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> NativeReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn read_byte(&mut self) -> Option<u8> {
        let byte = *self.bytes.get(self.offset)?;
        self.offset += 1;
        Some(byte)
    }

    fn read_count(&mut self) -> Option<usize> {
        let bytes = self.read_bytes(std::mem::size_of::<u64>())?;
        let count = u64::from_le_bytes(bytes.try_into().ok()?);
        usize::try_from(count).ok()
    }

    fn read_bytes(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.offset.checked_add(count)?;
        let bytes = self.bytes.get(self.offset..end)?;
        self.offset = end;
        Some(bytes)
    }

    fn is_finished(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[cfg(unix)]
const fn native_unit_width() -> usize {
    1
}

#[cfg(windows)]
const fn native_unit_width() -> usize {
    2
}

#[cfg(not(any(unix, windows)))]
const fn native_unit_width() -> usize {
    1
}

#[cfg(unix)]
fn native_path_from_units(units: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(OsString::from_vec(units.to_vec())))
}

#[cfg(windows)]
fn native_path_from_units(units: &[u8]) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let wide = units
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    Some(PathBuf::from(OsString::from_wide(&wide)))
}

#[cfg(not(any(unix, windows)))]
fn native_path_from_units(units: &[u8]) -> Option<PathBuf> {
    // The fallback target cannot reconstruct arbitrary native units without
    // unsafe code; keep its unrecognized native history read-only instead.
    Some(PathBuf::from(std::str::from_utf8(units).ok()?))
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

        let serialized = recent.serialize();
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

        let serialized = recent.serialize();
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
        assert_eq!(recent.serialize(), "");
    }

    #[test]
    fn ordinary_filename_resembling_the_v2_header_stays_legacy_v1() {
        let mut recent = RecentFiles::new(2);
        recent.push("occluview-recent-v2");

        let serialized = recent.serialize();
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

        let serialized = recent.serialize();
        assert!(serialized.starts_with("\0occluview-recent-v2:"));
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

        let serialized = recent.serialize();
        assert!(serialized.starts_with("\0occluview-recent-v2:"));
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
}
