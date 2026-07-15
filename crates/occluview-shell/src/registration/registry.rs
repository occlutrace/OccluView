use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyW, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW, HKEY, HKEY_CLASSES_ROOT, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_SZ,
    REG_VALUE_TYPE,
};

/// `ERROR_SUCCESS`.
pub(super) const ERROR_SUCCESS: u32 = 0;
/// `ERROR_FILE_NOT_FOUND` — already-deleted key, treat as success on unregister.
pub(super) const ERROR_FILE_NOT_FOUND: u32 = 2;
/// `ERROR_PATH_NOT_FOUND` — parent path already deleted, also success.
pub(super) const ERROR_PATH_NOT_FOUND: u32 = 3;

/// Create or open a registry key under HKCR, returning the handle. Errors
/// propagate as `windows::core::Error`.
pub(super) fn create_key(subkey: &HSTRING) -> windows::core::Result<HKEY> {
    create_key_at(HKEY_CLASSES_ROOT, subkey)
}

pub(super) fn create_key_at(root: HKEY, subkey: &HSTRING) -> windows::core::Result<HKEY> {
    let mut hkey = HKEY::default();
    // SAFETY: `hkey` is a stack out-param; `subkey` is a valid PCWSTR.
    let r = unsafe { RegCreateKeyW(root, PCWSTR(subkey.as_ptr()), &mut hkey) };
    if r.0 == ERROR_SUCCESS {
        Ok(hkey)
    } else {
        Err(windows::core::Error::from_win32())
    }
}

/// Open an existing registry key for value query/delete. Missing keys are a
/// successful no-op for unregister paths.
pub(super) fn open_key_for_value_update(subkey: &HSTRING) -> windows::core::Result<Option<HKEY>> {
    open_key_for_value_update_at(HKEY_CLASSES_ROOT, subkey)
}

pub(super) fn open_key_for_value_update_at(
    root: HKEY,
    subkey: &HSTRING,
) -> windows::core::Result<Option<HKEY>> {
    let mut hkey = HKEY::default();
    // SAFETY: `hkey` is a valid out-param; `subkey` is a valid PCWSTR.
    let r = unsafe {
        RegOpenKeyExW(
            root,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            &mut hkey,
        )
    };
    if r.0 == ERROR_SUCCESS {
        Ok(Some(hkey))
    } else if r.0 == ERROR_FILE_NOT_FOUND || r.0 == ERROR_PATH_NOT_FOUND {
        Ok(None)
    } else {
        Err(windows::core::Error::from_win32())
    }
}

/// Set a `REG_SZ` value. `name=None` sets the key's default value.
pub(super) fn set_string(
    hkey: HKEY,
    name: Option<&HSTRING>,
    value: &HSTRING,
) -> windows::core::Result<()> {
    let name_pcwstr = match name {
        Some(n) => PCWSTR(n.as_ptr()),
        None => PCWSTR::null(),
    };
    // Include the trailing NUL wide char in the byte length.
    let bytes = value.as_wide();
    let byte_len = (bytes.len() + 1) * 2;
    // SAFETY: `value`'s wide buffer is valid for byte_len bytes; the trailing
    // NUL is implicit in HSTRING's allocation.
    let r = unsafe {
        RegSetValueExW(
            hkey,
            name_pcwstr,
            0,
            REG_SZ,
            Some(std::slice::from_raw_parts(
                bytes.as_ptr() as *const u8,
                byte_len,
            )),
        )
    };
    if r.0 == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(windows::core::Error::from_win32())
    }
}

pub(super) fn set_dword(hkey: HKEY, name: &HSTRING, value: u32) -> windows::core::Result<()> {
    let bytes = value.to_le_bytes();
    // SAFETY: `bytes` is exactly four bytes, as required for REG_DWORD.
    let r = unsafe { RegSetValueExW(hkey, PCWSTR(name.as_ptr()), 0, REG_DWORD, Some(&bytes)) };
    if r.0 == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(windows::core::Error::from_win32())
    }
}

/// Delete a single registry value under `subkey`. `name=None` deletes the
/// default value; missing keys/values are successful no-ops.
pub(super) fn delete_value(subkey: &HSTRING, name: Option<&HSTRING>) -> windows::core::Result<()> {
    delete_value_at(HKEY_CLASSES_ROOT, subkey, name)
}

pub(super) fn delete_value_at(
    root: HKEY,
    subkey: &HSTRING,
    name: Option<&HSTRING>,
) -> windows::core::Result<()> {
    let Some(hkey) = open_key_for_value_update_at(root, subkey)? else {
        return Ok(());
    };
    let name_pcwstr = value_name_pcwstr(name);
    // SAFETY: `hkey` is open and `name_pcwstr` is either null for the default
    // value or points at a live HSTRING.
    let r = unsafe { RegDeleteValueW(hkey, name_pcwstr) };
    let _ = unsafe { RegCloseKey(hkey) };
    if r.0 == ERROR_SUCCESS || r.0 == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(windows::core::Error::from_win32())
    }
}

/// Delete a single registry value only if its current value exactly matches
/// `expected`. This prevents self-unregistration from clearing a handler that
/// another app has claimed after OccluView was registered.
pub(super) fn delete_value_if_matches(
    subkey: &HSTRING,
    name: Option<&HSTRING>,
    expected: &HSTRING,
) -> windows::core::Result<()> {
    let Some(hkey) = open_key_for_value_update(subkey)? else {
        return Ok(());
    };
    let current = query_string_value(hkey, name)?;
    let result = if current
        .as_ref()
        .is_some_and(|value| value.as_wide() == expected.as_wide())
    {
        let name_pcwstr = value_name_pcwstr(name);
        // SAFETY: `hkey` is open and `name_pcwstr` is either null or a live
        // HSTRING pointer.
        let r = unsafe { RegDeleteValueW(hkey, name_pcwstr) };
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

pub(super) fn key_default_matches(
    subkey: &HSTRING,
    expected: &HSTRING,
) -> windows::core::Result<bool> {
    let Some(hkey) = open_key_for_value_update(subkey)? else {
        return Ok(false);
    };
    let current = query_string_value(hkey, None)?;
    let _ = unsafe { RegCloseKey(hkey) };
    Ok(current
        .as_ref()
        .is_some_and(|value| value.as_wide() == expected.as_wide()))
}

pub(super) fn query_string_value(
    hkey: HKEY,
    name: Option<&HSTRING>,
) -> windows::core::Result<Option<HSTRING>> {
    let name_pcwstr = value_name_pcwstr(name);
    let mut value_type = REG_VALUE_TYPE::default();
    let mut byte_len = 0u32;
    // SAFETY: first query asks Windows for required buffer length.
    let r = unsafe {
        RegQueryValueExW(
            hkey,
            name_pcwstr,
            None,
            Some(&mut value_type),
            None,
            Some(&mut byte_len),
        )
    };
    if r.0 == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if r.0 != ERROR_SUCCESS {
        return Err(windows::core::Error::from_win32());
    }
    if value_type != REG_SZ || byte_len == 0 {
        return Ok(None);
    }

    let mut bytes = vec![0u8; byte_len as usize];
    // SAFETY: `bytes` is allocated to the byte length Windows just reported.
    let r = unsafe {
        RegQueryValueExW(
            hkey,
            name_pcwstr,
            None,
            Some(&mut value_type),
            Some(bytes.as_mut_ptr()),
            Some(&mut byte_len),
        )
    };
    if r.0 != ERROR_SUCCESS {
        return Err(windows::core::Error::from_win32());
    }
    let mut wide = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    while wide.last().copied() == Some(0) {
        let _ = wide.pop();
    }
    Ok(Some(HSTRING::from_wide(&wide).unwrap_or_default()))
}

fn value_name_pcwstr(name: Option<&HSTRING>) -> PCWSTR {
    match name {
        Some(value) => PCWSTR(value.as_ptr()),
        None => PCWSTR::null(),
    }
}

/// Recursively delete a key. `ERROR_FILE_NOT_FOUND` is treated as success.
pub(super) fn delete_tree(subkey: &HSTRING) -> windows::core::Result<()> {
    // SAFETY: `subkey` is a valid PCWSTR; RegDeleteTreeW recursively removes
    // the key and all subkeys.
    let r = unsafe { RegDeleteTreeW(HKEY_CLASSES_ROOT, PCWSTR(subkey.as_ptr())) };
    if r.0 == ERROR_SUCCESS || r.0 == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(windows::core::Error::from_win32())
    }
}
