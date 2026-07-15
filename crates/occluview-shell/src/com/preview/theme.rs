use super::super::{
    w, RegCloseKey, RegOpenKeyExW, RegQueryValueExW, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, PCWSTR, PREVIEW_DARK_BACKGROUND_LINEAR,
    PREVIEW_DARK_CANVAS_RGBA, PREVIEW_LIGHT_BACKGROUND_LINEAR, PREVIEW_LIGHT_CANVAS_RGBA,
    REG_DWORD, REG_VALUE_TYPE,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PreviewTheme {
    Light,
    Dark,
}

impl PreviewTheme {
    pub(super) const fn background_linear(self) -> [f64; 4] {
        match self {
            Self::Light => PREVIEW_LIGHT_BACKGROUND_LINEAR,
            Self::Dark => PREVIEW_DARK_BACKGROUND_LINEAR,
        }
    }

    pub(super) const fn canvas_rgba(self) -> [u8; 4] {
        match self {
            Self::Light => PREVIEW_LIGHT_CANVAS_RGBA,
            Self::Dark => PREVIEW_DARK_CANVAS_RGBA,
        }
    }
}

pub(super) fn preview_theme() -> PreviewTheme {
    preview_theme_from_apps_use_light_theme(windows_apps_use_light_theme())
}

fn preview_theme_from_apps_use_light_theme(apps_use_light_theme: Option<bool>) -> PreviewTheme {
    match apps_use_light_theme {
        Some(false) => PreviewTheme::Dark,
        Some(true) | None => PreviewTheme::Light,
    }
}

fn windows_apps_use_light_theme() -> Option<bool> {
    let mut hkey = HKEY::default();
    // SAFETY: `hkey` is a stack out-param and both PCWSTR literals are
    // null-terminated for the duration of the registry calls.
    let open = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            0,
            KEY_QUERY_VALUE,
            &mut hkey,
        )
    };
    if open.0 != ERROR_SUCCESS {
        return None;
    }
    let value = query_registry_dword(hkey, w!("AppsUseLightTheme")).map(|value| value != 0);
    // SAFETY: `hkey` was opened successfully above.
    let _ = unsafe { RegCloseKey(hkey) };
    value
}

fn query_registry_dword(hkey: HKEY, value_name: PCWSTR) -> Option<u32> {
    let mut value_type = REG_VALUE_TYPE::default();
    let mut bytes = [0_u8; 4];
    let mut byte_len = bytes.len() as u32;
    // SAFETY: `bytes` is a four-byte output buffer and `byte_len` describes its
    // size in bytes. `value_name` points at a live null-terminated string.
    let query = unsafe {
        RegQueryValueExW(
            hkey,
            value_name,
            None,
            Some(&mut value_type),
            Some(bytes.as_mut_ptr()),
            Some(&mut byte_len),
        )
    };
    if query.0 == ERROR_FILE_NOT_FOUND {
        return None;
    }
    if query.0 == ERROR_SUCCESS && value_type == REG_DWORD && byte_len >= 4 {
        Some(u32::from_le_bytes(bytes))
    } else {
        None
    }
}
