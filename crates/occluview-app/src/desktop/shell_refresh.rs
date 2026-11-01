//! Telling Explorer that this build's file associations changed.
//!
//! Type-name migration and `SHChangeNotify`, kept here rather than imported from
//! `occluview-shell`. That crate is the COM surface -- thumbnail provider,
//! preview handler, registration -- and the viewer needs only this one call;
//! importing it would link all of it into the GUI binary.

// `SHChangeNotify` is the one shell FFI call the viewer makes; this
// module-level allow is the crate gate's only relaxation here.
#![allow(unsafe_code)]

/// Tell Explorer that file associations and shell handlers changed.
#[cfg(windows)]
pub(crate) fn notify_shell_associations_changed() {
    use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

    refresh_owned_type_names();
    // SAFETY: SHChangeNotify accepts null item pointers for SHCNE_ASSOCCHANGED.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// Existing per-user descriptions override the installer's machine-wide
/// labels. Refresh owned keys in both views without changing UserChoice or
/// creating registrations during uninstall.
#[cfg(windows)]
fn refresh_owned_type_names() {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, REG_SZ, RegCloseKey,
        RegOpenKeyExW, RegSetValueExW,
    };
    use windows::core::{HSTRING, PCWSTR, h};
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let formats = ["STL", "PLY", "OBJ", "OFF", "GLB"];
        let mut labels = Vec::new();
        for format in formats {
            for prefix in ["MeshFile", "OccluView.Mesh"] {
                labels.push((format!("{prefix}.{format}"), format!("{format} File"), true));
            }
        }
        // Old releases used one shared ProgID for every format. It cannot
        // name an individual format; retire its branded description while
        // preserving the user's choice of application.
        labels.push(("OccluView.Mesh".into(), "3D Model".into(), true));
        labels.push((
            "Applications\\occluview.exe".into(),
            "3D Model".into(),
            false,
        ));
        for (progid, label, replace_default) in labels {
            let path = HSTRING::from(format!("Software\\Classes\\{progid}"));
            let mut key = HKEY::default();
            // SAFETY: valid path and output handle; existing owned keys only.
            let status = unsafe { RegOpenKeyExW(root, &path, None, KEY_SET_VALUE, &mut key) };
            if status.0 != 0 {
                continue;
            }
            let label: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: label remains live; the byte slice includes its NUL.
            let bytes =
                unsafe { std::slice::from_raw_parts(label.as_ptr().cast::<u8>(), label.len() * 2) };
            // SAFETY: key is open for writing, name and value are valid.
            let _ =
                unsafe { RegSetValueExW(key, h!("FriendlyTypeName"), None, REG_SZ, Some(bytes)) };
            if replace_default {
                // SAFETY: a null value name writes the default description.
                let _ = unsafe { RegSetValueExW(key, PCWSTR::null(), None, REG_SZ, Some(bytes)) };
            }
            // SAFETY: this function owns the opened handle.
            let _ = unsafe { RegCloseKey(key) };
        }
    }
}
