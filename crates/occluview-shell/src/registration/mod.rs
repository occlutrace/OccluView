//! `DllRegisterServer` / `DllUnregisterServer` — COM self-registration.
//!
//! `regsvr32 occluview_shell.dll` runs `DllRegisterServer`, which writes the
//! registry entries that make Windows Explorer activate our thumbnail provider
//! for each supported extension, AND registers the "Open with" ProgID so the
//! shell offers OccluView in the context menu. `DllUnregisterServer` (via
//! `regsvr32 /u`) removes them.
//!
//! Registration mirrors the MSI layout: thumbnail provider CLSID, one
//! `ShellEx\{E357FCCD-A995-4576-B01F-234630154E96}` entry per supported
//! extension, and one per-format `MeshFile.<EXT>` ProgID with the shared
//! 3D file icon, `ThumbnailCutoff`, `TypeOverlay`, and `shell\open\command`.

// Registry FFI is `unsafe` by definition; this module shares com.rs's gate
// (the `cfg(windows)` lives on `pub mod registration;` in lib.rs). The
// pedantic lints below are inherent to FFI/registry glue (raw pointer
// derefs, casts across the ABI) and are relaxed here only.
#![allow(
    unsafe_code,
    clippy::missing_safety_doc,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::not_unsafe_ptr_arg_deref,
    clippy::ptr_as_ptr,
    clippy::borrow_as_ptr,
    clippy::unnecessary_cast,
    clippy::doc_markdown,
    missing_docs
)]

mod associations;
mod clsid;
mod paths;
mod registry;

use associations::{
    register_extension, register_extension_fallback, register_open_with,
    register_preview_extension, register_progid, register_system_extension,
    register_system_preview_extension, unregister_extension, unregister_extension_fallback,
    unregister_open_with, unregister_preview_extension, unregister_progid,
    unregister_system_extension, unregister_system_preview_extension, LEGACY_OCCLUVIEW_PROGID,
};
use clsid::{
    register_approved_shell_extension, register_clsid, register_preview_handler_clsid,
    register_preview_handlers_list, unregister_approved_shell_extension, unregister_clsid,
    unregister_preview_handler_clsid, unregister_preview_handlers_list,
};
use paths::app_exe_path;
use registry::delete_tree;

use crate::com::{OCCLUVIEW_PREVIEW_CLSID, OCCLUVIEW_THUMBNAIL_CLSID};
use crate::{APP_EXE_NAME, SUPPORTED_EXTENSIONS};
use windows::core::{HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

const S_OK: HRESULT = HRESULT(0);
const E_FAIL: HRESULT = HRESULT(-2_147_418_235);

/// `regsvr32 occluview_shell.dll` calls this. Creates all registry entries.
#[no_mangle]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    match register_all() {
        Ok(()) => S_OK,
        Err(e) => {
            tracing::error!(error = ?e, "DllRegisterServer failed");
            E_FAIL
        }
    }
}

/// `regsvr32 /u occluview_shell.dll` calls this. Removes all entries.
#[no_mangle]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    match unregister_all() {
        Ok(()) => S_OK,
        Err(e) => {
            tracing::error!(error = ?e, "DllUnregisterServer failed");
            E_FAIL
        }
    }
}

/// Write every registry entry. Idempotent — re-running over existing entries
/// is a no-op (RegCreateKeyW opens existing keys).
///
/// The "Open with" ProgID registration is only written when the app exe is
/// found next to this DLL (same directory); otherwise it's skipped silently
/// so the thumbnail registration still succeeds. The installer is expected
/// to place both binaries together.
fn register_all() -> windows::core::Result<()> {
    let dll_path = own_dll_path()?;
    register_clsid(&dll_path)?;
    register_preview_handler_clsid(&dll_path)?;
    register_approved_shell_extension()?;
    register_preview_handlers_list()?;
    let our_clsid = HSTRING::from(OCCLUVIEW_THUMBNAIL_CLSID);
    let preview_clsid = HSTRING::from(OCCLUVIEW_PREVIEW_CLSID);
    for &ext in SUPPORTED_EXTENSIONS {
        register_extension(ext, &our_clsid)?;
        register_preview_extension(ext, &preview_clsid)?;
        register_system_extension(ext, &our_clsid)?;
        register_system_preview_extension(ext, &preview_clsid)?;
    }
    if let Some(app_path) = app_exe_path(&dll_path) {
        for &ext in SUPPORTED_EXTENSIONS {
            register_progid(ext, &app_path)?;
            register_extension_fallback(ext, &app_path)?;
            register_open_with(ext)?;
        }
    } else {
        tracing::warn!(
            "app exe '{}' not found next to DLL; skipping Open-with ProgID registration",
            APP_EXE_NAME
        );
    }
    notify_shell_associations_changed();
    Ok(())
}

/// Remove every registry entry. Missing keys are not an error.
fn unregister_all() -> windows::core::Result<()> {
    let dll_path = own_dll_path().unwrap_or_default();
    let app_path = app_exe_path(&dll_path);
    unregister_clsid()?;
    unregister_preview_handler_clsid()?;
    let _ = unregister_approved_shell_extension();
    let _ = unregister_preview_handlers_list();
    for &ext in SUPPORTED_EXTENSIONS {
        // Missing entry is fine (user may have deleted it); ignore not-found.
        let _ = unregister_extension(ext);
        let _ = unregister_preview_extension(ext);
        let _ = unregister_system_extension(ext);
        let _ = unregister_system_preview_extension(ext);
        let _ = unregister_open_with(ext);
        let _ = unregister_extension_fallback(ext, app_path.as_ref());
        let _ = unregister_progid(ext);
    }
    let _ = delete_tree(&HSTRING::from(LEGACY_OCCLUVIEW_PROGID));
    notify_shell_associations_changed();
    Ok(())
}

/// Tell Explorer that file associations and shell handlers changed.
pub fn notify_shell_associations_changed() {
    // SAFETY: SHChangeNotify accepts null item pointers for SHCNE_ASSOCCHANGED.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// Path to this DLL on disk.
fn own_dll_path() -> windows::core::Result<HSTRING> {
    let module = own_module_handle()?;
    let mut buf = [0u16; 1024];
    // SAFETY: GetModuleFileNameW writes up to `buf.len()` wide chars into our
    // stack array. `module` is resolved from an address inside this DLL.
    let n = unsafe { GetModuleFileNameW(module, buf.as_mut_slice()) };
    if n == 0 {
        return Err(windows::core::Error::from_win32());
    }
    HSTRING::from_wide(&buf[..n as usize])
}

fn own_module_handle() -> windows::core::Result<HMODULE> {
    let mut module = HMODULE::default();
    let flags =
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    // SAFETY: With FROM_ADDRESS, lpmodulename is interpreted as an address
    // inside the module. `DllRegisterServer` is exported from this DLL, and
    // UNCHANGED_REFCOUNT avoids changing loader lifetime during registration.
    unsafe {
        GetModuleHandleExW(
            flags,
            PCWSTR(DllRegisterServer as *const () as *const u16),
            &mut module,
        )
    }?;
    Ok(module)
}
