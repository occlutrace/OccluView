use crate::APP_EXE_NAME;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Storage::FileSystem::GetFileAttributesW;

/// Resolve the app exe path: same directory as `dll_path`, with
/// [`APP_EXE_NAME`]. Returns `None` if the file does not exist.
pub(super) fn app_exe_path(dll_path: &HSTRING) -> Option<HSTRING> {
    let wide = dll_path.as_wide();
    // Find the last path separator (backslash).
    let sep = wide.iter().rposition(|&c| c == u16::from(b'\\'))?;
    let dir = &wide[..=sep];
    let exe_name: Vec<u16> = APP_EXE_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut full = dir.to_vec();
    full.extend_from_slice(&exe_name);
    let full = HSTRING::from_wide(&full[..full.len() - 1]).ok()?;
    // Existence check via GetFileAttributesW (avoid pulling std::fs for cfg).
    // SAFETY: full is a valid NUL-terminated wide path.
    let attrs = unsafe { GetFileAttributesW(PCWSTR(full.as_ptr())) };
    // INVALID_FILE_ATTRIBUTES == 0xFFFFFFFF (-1 as u32).
    if attrs == u32::MAX {
        return None;
    }
    Some(full)
}

pub(super) fn sibling_path(path: &HSTRING, filename: &str) -> Option<HSTRING> {
    let wide = path.as_wide();
    let sep = wide.iter().rposition(|&c| c == u16::from(b'\\'))?;
    let dir = &wide[..=sep];
    let filename: Vec<u16> = filename.encode_utf16().collect();
    let mut full = dir.to_vec();
    full.extend_from_slice(&filename);
    HSTRING::from_wide(&full).ok()
}

pub(super) fn path_exists(path: &HSTRING) -> bool {
    // SAFETY: path is a valid NUL-terminated wide path.
    let attrs = unsafe { GetFileAttributesW(PCWSTR(path.as_ptr())) };
    attrs != u32::MAX
}

/// Best-effort HSTRING → String for assembling registry command lines. Lossy
/// on non-UTF-16 paths (uses replacement char), which is acceptable for the
/// diagnostics/assembly path; the actual registration uses the original
/// HSTRING bytes when it matters.
pub(super) fn utf16_to_string(s: &HSTRING) -> String {
    String::from_utf16_lossy(s.as_wide())
}
