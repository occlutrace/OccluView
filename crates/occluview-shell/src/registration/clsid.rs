use super::registry::{create_key, create_key_at, delete_tree, delete_value_at, set_string};
use crate::com::{OCCLUVIEW_PREVIEW_CLSID, OCCLUVIEW_THUMBNAIL_CLSID};
use windows::core::{h, HSTRING};
use windows::Win32::System::Registry::{RegCloseKey, HKEY_LOCAL_MACHINE};

/// Static HSTRINGs via the `h!` macro (the macro yields `&'static HSTRING`).
const FRIENDLY_NAME_H: &HSTRING = h!("OccluView Thumbnail Provider");
const PREVIEW_FRIENDLY_NAME_H: &HSTRING = h!("OccluView Preview Handler");
const THREADING_MODEL_H: &HSTRING = h!("Apartment");
const PREVHOST_APPID: &HSTRING = h!("{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}");
const APPROVED_SHELL_EXTENSIONS_KEY: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Shell Extensions\\Approved";
const PREVIEW_HANDLERS_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\PreviewHandlers";

/// Register `HKCR\CLSID\{clsid}\InprocServer32` with the DLL path.
pub(super) fn register_clsid(dll_path: &HSTRING) -> windows::core::Result<()> {
    let clsid_key = HSTRING::from(format!("CLSID\\{OCCLUVIEW_THUMBNAIL_CLSID}"));
    let inproc_key = HSTRING::from(format!(
        "CLSID\\{OCCLUVIEW_THUMBNAIL_CLSID}\\InprocServer32"
    ));

    // Top-level CLSID entry: friendly name.
    let hk = create_key(&clsid_key)?;
    set_string(hk, None, FRIENDLY_NAME_H)?;
    let _ = unsafe { RegCloseKey(hk) };

    // InprocServer32: DLL path + ThreadingModel.
    let hk_inproc = create_key(&inproc_key)?;
    set_string(hk_inproc, None, dll_path)?;
    set_string(hk_inproc, Some(h!("ThreadingModel")), THREADING_MODEL_H)?;
    let _ = unsafe { RegCloseKey(hk_inproc) };
    Ok(())
}

pub(super) fn register_preview_handler_clsid(dll_path: &HSTRING) -> windows::core::Result<()> {
    let clsid_key = HSTRING::from(format!("CLSID\\{OCCLUVIEW_PREVIEW_CLSID}"));
    let inproc_key = HSTRING::from(format!("CLSID\\{OCCLUVIEW_PREVIEW_CLSID}\\InprocServer32"));

    let hk = create_key(&clsid_key)?;
    set_string(hk, None, PREVIEW_FRIENDLY_NAME_H)?;
    set_string(hk, Some(h!("AppID")), PREVHOST_APPID)?;
    let _ = unsafe { RegCloseKey(hk) };

    let hk_inproc = create_key(&inproc_key)?;
    set_string(hk_inproc, None, dll_path)?;
    set_string(hk_inproc, Some(h!("ThreadingModel")), THREADING_MODEL_H)?;
    let _ = unsafe { RegCloseKey(hk_inproc) };
    Ok(())
}

/// Remove `HKCR\CLSID\{clsid}` entirely (cascades to InprocServer32).
pub(super) fn unregister_clsid() -> windows::core::Result<()> {
    let clsid_key = HSTRING::from(format!("CLSID\\{OCCLUVIEW_THUMBNAIL_CLSID}"));
    delete_tree(&clsid_key)
}

pub(super) fn unregister_preview_handler_clsid() -> windows::core::Result<()> {
    let clsid_key = HSTRING::from(format!("CLSID\\{OCCLUVIEW_PREVIEW_CLSID}"));
    delete_tree(&clsid_key)
}

pub(super) fn register_approved_shell_extension() -> windows::core::Result<()> {
    let hk = create_key_at(
        HKEY_LOCAL_MACHINE,
        &HSTRING::from(APPROVED_SHELL_EXTENSIONS_KEY),
    )?;
    set_string(
        hk,
        Some(&HSTRING::from(OCCLUVIEW_THUMBNAIL_CLSID)),
        FRIENDLY_NAME_H,
    )?;
    set_string(
        hk,
        Some(&HSTRING::from(OCCLUVIEW_PREVIEW_CLSID)),
        PREVIEW_FRIENDLY_NAME_H,
    )?;
    let _ = unsafe { RegCloseKey(hk) };
    Ok(())
}

pub(super) fn unregister_approved_shell_extension() -> windows::core::Result<()> {
    let _ = delete_value_at(
        HKEY_LOCAL_MACHINE,
        &HSTRING::from(APPROVED_SHELL_EXTENSIONS_KEY),
        Some(&HSTRING::from(OCCLUVIEW_THUMBNAIL_CLSID)),
    );
    delete_value_at(
        HKEY_LOCAL_MACHINE,
        &HSTRING::from(APPROVED_SHELL_EXTENSIONS_KEY),
        Some(&HSTRING::from(OCCLUVIEW_PREVIEW_CLSID)),
    )
}

pub(super) fn register_preview_handlers_list() -> windows::core::Result<()> {
    let hk = create_key_at(HKEY_LOCAL_MACHINE, &HSTRING::from(PREVIEW_HANDLERS_KEY))?;
    set_string(
        hk,
        Some(&HSTRING::from(OCCLUVIEW_PREVIEW_CLSID)),
        PREVIEW_FRIENDLY_NAME_H,
    )?;
    let _ = unsafe { RegCloseKey(hk) };
    Ok(())
}

pub(super) fn unregister_preview_handlers_list() -> windows::core::Result<()> {
    delete_value_at(
        HKEY_LOCAL_MACHINE,
        &HSTRING::from(PREVIEW_HANDLERS_KEY),
        Some(&HSTRING::from(OCCLUVIEW_PREVIEW_CLSID)),
    )
}
