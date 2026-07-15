use super::paths::{path_exists, sibling_path, utf16_to_string};
use super::registry::{
    create_key, delete_tree, delete_value, delete_value_if_matches, key_default_matches,
    open_key_for_value_update, query_string_value, set_dword, set_string, ERROR_FILE_NOT_FOUND,
    ERROR_SUCCESS,
};
use crate::com::{OCCLUVIEW_PREVIEW_CLSID, OCCLUVIEW_THUMBNAIL_CLSID};
use crate::{PREVIEW_HANDLER_CATEGORY, THUMBNAIL_PROVIDER_CATEGORY};
use windows::core::{h, HSTRING, PCWSTR};
use windows::Win32::System::Registry::{RegCloseKey, RegDeleteValueW};

/// Legacy ProgIDs. Kept only for unregister cleanup.
pub(super) const LEGACY_OCCLUVIEW_PROGID: &str = "OccluView.Mesh";

/// Register `HKCR\.{ext}\ShellEx\{category}` -> our CLSID.
pub(super) fn register_extension(ext: &str, our_clsid: &HSTRING) -> windows::core::Result<()> {
    let key_path = HSTRING::from(format!(".{ext}\\ShellEx\\{THUMBNAIL_PROVIDER_CATEGORY}"));
    let hk = create_key(&key_path)?;
    set_string(hk, None, our_clsid)?;
    let _ = unsafe { RegCloseKey(hk) };
    Ok(())
}

pub(super) fn register_preview_extension(
    ext: &str,
    preview_clsid: &HSTRING,
) -> windows::core::Result<()> {
    let key_path = HSTRING::from(format!(".{ext}\\ShellEx\\{PREVIEW_HANDLER_CATEGORY}"));
    let hk = create_key(&key_path)?;
    set_string(hk, None, preview_clsid)?;
    let _ = unsafe { RegCloseKey(hk) };
    Ok(())
}

pub(super) fn register_system_extension(
    ext: &str,
    our_clsid: &HSTRING,
) -> windows::core::Result<()> {
    let key_path = system_file_association_shell_ex_key(ext, THUMBNAIL_PROVIDER_CATEGORY);
    let hk = create_key(&key_path)?;
    set_string(hk, None, our_clsid)?;
    let _ = unsafe { RegCloseKey(hk) };
    Ok(())
}

pub(super) fn register_system_preview_extension(
    ext: &str,
    preview_clsid: &HSTRING,
) -> windows::core::Result<()> {
    let key_path = system_file_association_shell_ex_key(ext, PREVIEW_HANDLER_CATEGORY);
    let hk = create_key(&key_path)?;
    set_string(hk, None, preview_clsid)?;
    let _ = unsafe { RegCloseKey(hk) };
    Ok(())
}

/// Clear the thumbnail-handler default value only when it still points at us.
pub(super) fn unregister_extension(ext: &str) -> windows::core::Result<()> {
    let key_path = HSTRING::from(format!(".{ext}\\ShellEx\\{THUMBNAIL_PROVIDER_CATEGORY}"));
    let our_clsid = HSTRING::from(OCCLUVIEW_THUMBNAIL_CLSID);
    delete_value_if_matches(&key_path, None, &our_clsid)
}

pub(super) fn unregister_preview_extension(ext: &str) -> windows::core::Result<()> {
    let key_path = HSTRING::from(format!(".{ext}\\ShellEx\\{PREVIEW_HANDLER_CATEGORY}"));
    let preview_clsid = HSTRING::from(OCCLUVIEW_PREVIEW_CLSID);
    delete_value_if_matches(&key_path, None, &preview_clsid)
}

pub(super) fn unregister_system_extension(ext: &str) -> windows::core::Result<()> {
    let key_path = system_file_association_shell_ex_key(ext, THUMBNAIL_PROVIDER_CATEGORY);
    let our_clsid = HSTRING::from(OCCLUVIEW_THUMBNAIL_CLSID);
    delete_value_if_matches(&key_path, None, &our_clsid)
}

pub(super) fn unregister_system_preview_extension(ext: &str) -> windows::core::Result<()> {
    let key_path = system_file_association_shell_ex_key(ext, PREVIEW_HANDLER_CATEGORY);
    let preview_clsid = HSTRING::from(OCCLUVIEW_PREVIEW_CLSID);
    delete_value_if_matches(&key_path, None, &preview_clsid)
}

/// Register the per-format ProgID with `shell\open\command` and `DefaultIcon`.
pub(super) fn register_progid(ext: &str, app_path: &HSTRING) -> windows::core::Result<()> {
    let progid_string = format_progid(ext);
    let progid = HSTRING::from(&progid_string);
    // Top-level: friendly name.
    let hk = create_key(&progid)?;
    set_string(hk, None, &HSTRING::from(format_file_type_name(ext)))?;
    set_dword(hk, h!("ThumbnailCutoff"), 1)?;
    set_string(hk, Some(h!("TypeOverlay")), &HSTRING::new())?;
    let _ = unsafe { RegCloseKey(hk) };

    let our_clsid = HSTRING::from(OCCLUVIEW_THUMBNAIL_CLSID);
    let preview_clsid = HSTRING::from(OCCLUVIEW_PREVIEW_CLSID);
    register_progid_thumbnail_handler(&progid_string, &our_clsid)?;
    register_progid_preview_handler(&progid_string, &preview_clsid)?;

    // DefaultIcon: installed format icon when present; app icon fallback keeps
    // manual regsvr32 from a raw build directory usable.
    let icon_key = HSTRING::from(format!("{progid_string}\\DefaultIcon"));
    let hk = create_key(&icon_key)?;
    let icon_val = format_icon_value(app_path);
    set_string(hk, None, &icon_val)?;
    let _ = unsafe { RegCloseKey(hk) };

    // shell\open\command: "<app.exe>" "%1"
    let cmd_key = HSTRING::from(format!("{progid_string}\\shell\\open\\command"));
    let hk = create_key(&cmd_key)?;
    let cmd_val = HSTRING::from(format!("\"{}\" \"%1\"", utf16_to_string(app_path)));
    set_string(hk, None, &cmd_val)?;
    let _ = unsafe { RegCloseKey(hk) };
    Ok(())
}

/// Register direct extension fallback values used when Windows has no
/// protected per-user default for the extension. This is deliberately separate
/// from UserChoice; Windows owns that value.
pub(super) fn register_extension_fallback(
    ext: &str,
    app_path: &HSTRING,
) -> windows::core::Result<()> {
    let dot_ext = HSTRING::from(format!(".{ext}"));
    let progid = HSTRING::from(format_progid(ext));
    let hk = create_key(&dot_ext)?;
    set_string(hk, None, &progid)?;
    set_dword(hk, h!("ThumbnailCutoff"), 1)?;
    set_string(hk, Some(h!("TypeOverlay")), &HSTRING::new())?;
    let _ = unsafe { RegCloseKey(hk) };

    let icon_key = HSTRING::from(format!(".{ext}\\DefaultIcon"));
    let hk = create_key(&icon_key)?;
    let icon_val = format_icon_value(app_path);
    set_string(hk, None, &icon_val)?;
    let _ = unsafe { RegCloseKey(hk) };
    Ok(())
}

pub(super) fn register_progid_thumbnail_handler(
    progid: &str,
    our_clsid: &HSTRING,
) -> windows::core::Result<()> {
    let key_path = HSTRING::from(format!("{progid}\\ShellEx\\{THUMBNAIL_PROVIDER_CATEGORY}"));
    let hk = create_key(&key_path)?;
    set_string(hk, None, our_clsid)?;
    let _ = unsafe { RegCloseKey(hk) };
    Ok(())
}

pub(super) fn register_progid_preview_handler(
    progid: &str,
    preview_clsid: &HSTRING,
) -> windows::core::Result<()> {
    let key_path = HSTRING::from(format!("{progid}\\ShellEx\\{PREVIEW_HANDLER_CATEGORY}"));
    let hk = create_key(&key_path)?;
    set_string(hk, None, preview_clsid)?;
    let _ = unsafe { RegCloseKey(hk) };
    Ok(())
}

/// Remove only extension fallback values that still point to OccluView.
pub(super) fn unregister_extension_fallback(
    ext: &str,
    app_path: Option<&HSTRING>,
) -> windows::core::Result<()> {
    let dot_ext = HSTRING::from(format!(".{ext}"));
    let progid = HSTRING::from(format_progid(ext));
    let owns_extension_fallback = key_default_matches(&dot_ext, &progid)?;
    if owns_extension_fallback {
        delete_value(&dot_ext, Some(h!("ThumbnailCutoff")))?;
        delete_value(&dot_ext, Some(h!("TypeOverlay")))?;
    }
    delete_value_if_matches(&dot_ext, None, &progid)?;
    let icon_key = HSTRING::from(format!(".{ext}\\DefaultIcon"));
    if let Some(app_path) = app_path {
        let icon_val = format_icon_value(app_path);
        delete_value_if_matches(&icon_key, None, &icon_val)?;
    } else {
        delete_default_icon_if_occluview(&icon_key)?;
    }
    Ok(())
}

/// Remove the ProgID entirely (cascades to DefaultIcon, shell\open\command).
pub(super) fn unregister_progid(ext: &str) -> windows::core::Result<()> {
    let progid = HSTRING::from(format_progid(ext));
    delete_tree(&progid)?;
    let legacy = HSTRING::from(format_legacy_progid(ext));
    delete_tree(&legacy)
}

/// Register `HKCR\.{ext}\OpenWithProgids` with the ProgID as a named value.
/// The value data is empty string; the shell keys on the value *name*.
pub(super) fn register_open_with(ext: &str) -> windows::core::Result<()> {
    let progid = HSTRING::from(format_progid(ext));
    let key_path = HSTRING::from(format!(".{ext}\\OpenWithProgids"));
    let hk = create_key(&key_path)?;
    let empty = HSTRING::new();
    set_string(hk, Some(&progid), &empty)?;
    let _ = unsafe { RegCloseKey(hk) };
    Ok(())
}

/// Remove only OccluView's named value from `HKCR\.{ext}\OpenWithProgids`.
pub(super) fn unregister_open_with(ext: &str) -> windows::core::Result<()> {
    let key_path = HSTRING::from(format!(".{ext}\\OpenWithProgids"));
    let progid = HSTRING::from(format_progid(ext));
    delete_value(&key_path, Some(&progid))?;
    let legacy_format = HSTRING::from(format_legacy_progid(ext));
    delete_value(&key_path, Some(&legacy_format))?;
    let legacy = HSTRING::from(LEGACY_OCCLUVIEW_PROGID);
    delete_value(&key_path, Some(&legacy))
}

fn format_progid(ext: &str) -> String {
    format!("MeshFile.{}", ext.to_ascii_uppercase())
}

fn format_legacy_progid(ext: &str) -> String {
    format!("{LEGACY_OCCLUVIEW_PROGID}.{}", ext.to_ascii_uppercase())
}

fn system_file_association_shell_ex_key(ext: &str, category: &str) -> HSTRING {
    HSTRING::from(format!(
        "SystemFileAssociations\\.{ext}\\ShellEx\\{category}"
    ))
}

fn format_file_type_name(ext: &str) -> String {
    format!("{} File", ext.to_ascii_uppercase())
}

fn format_icon_value(app_path: &HSTRING) -> HSTRING {
    HSTRING::from(
        sibling_path(app_path, "occluview-3d.ico")
            .filter(path_exists)
            .map_or_else(
                || format!("{},0", utf16_to_string(app_path)),
                |icon_path| utf16_to_string(&icon_path),
            ),
    )
}

fn delete_default_icon_if_occluview(subkey: &HSTRING) -> windows::core::Result<()> {
    let Some(hkey) = open_key_for_value_update(subkey)? else {
        return Ok(());
    };
    let current = query_string_value(hkey, None)?;
    let result = if current
        .as_ref()
        .is_some_and(is_occluview_default_icon_value)
    {
        // SAFETY: `hkey` is open and a null value name deletes the default value.
        let r = unsafe { RegDeleteValueW(hkey, PCWSTR::null()) };
        if r.0 == ERROR_SUCCESS || r.0 == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(windows::core::Error::from_win32())
        }
    } else {
        Ok(())
    };
    let _ = unsafe { RegCloseKey(hkey) };
    result
}

fn is_occluview_default_icon_value(value: &HSTRING) -> bool {
    let normalized = utf16_to_string(value)
        .replace('/', "\\")
        .to_ascii_lowercase();
    let executable = normalized.strip_suffix(",0").unwrap_or(&normalized);
    normalized.ends_with("\\occluview\\occluview-3d.ico")
        || executable.ends_with("\\occluview\\occluview.exe")
}

#[cfg(test)]
mod tests {
    use super::{
        format_icon_value, is_occluview_default_icon_value, system_file_association_shell_ex_key,
        utf16_to_string, THUMBNAIL_PROVIDER_CATEGORY,
    };
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
    use windows::core::HSTRING;

    fn unique_temp_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("occluview-{name}-{}-{stamp}", std::process::id()))
    }

    #[test]
    fn format_icon_value_prefers_sibling_icon_when_present() {
        let temp_dir = unique_temp_dir("icon-present");
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let app_path = temp_dir.join("occluview.exe");
        let icon_path = temp_dir.join("occluview-3d.ico");
        fs::write(&icon_path, b"icon").expect("icon fixture should be written");

        let value = format_icon_value(&HSTRING::from(app_path.to_string_lossy().as_ref()));
        assert_eq!(
            value.as_wide(),
            HSTRING::from(icon_path.to_string_lossy().as_ref()).as_wide()
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn format_icon_value_falls_back_to_executable_when_sibling_icon_is_missing() {
        let app_path = HSTRING::from(r"C:\Program Files\OccluView\occluview.exe");
        assert_eq!(
            format_icon_value(&app_path).as_wide(),
            HSTRING::from(r"C:\Program Files\OccluView\occluview.exe,0").as_wide()
        );
    }

    #[test]
    fn system_file_association_shell_ex_key_uses_the_expected_registry_path() {
        assert_eq!(
            utf16_to_string(&system_file_association_shell_ex_key(
                "stl",
                THUMBNAIL_PROVIDER_CATEGORY
            )),
            "SystemFileAssociations\\.stl\\ShellEx\\{E357FCCD-A995-4576-B01F-234630154E96}"
        );
    }

    #[test]
    fn default_icon_cleanup_matches_only_occluview_owned_icons() {
        let owned_values = [
            r"C:\Program Files\OccluView\occluview-3d.ico",
            r"C:/Program Files/OccluView/occluview-3d.ico",
            r"C:\Program Files\OccluView\occluview.exe",
            r"C:\Program Files\OccluView\occluview.exe,0",
        ];
        for value in owned_values {
            assert!(
                is_occluview_default_icon_value(&HSTRING::from(value)),
                "expected OccluView-owned default icon value to be selected for cleanup: {value}"
            );
        }
    }

    #[test]
    fn default_icon_cleanup_rejects_foreign_icons_even_with_similar_names() {
        let foreign_values = [
            r"C:\Program Files\OtherApp\occluview-3d.ico",
            r"C:\Program Files\OtherApp\occluview.exe,1",
            r"C:\Program Files\OtherApp\mesh.ico",
            r"C:\Program Files\OccluView\custom.ico",
        ];
        for value in foreign_values {
            assert!(
                !is_occluview_default_icon_value(&HSTRING::from(value)),
                "foreign icon value should survive unregister cleanup: {value}"
            );
        }
    }
}
