//! Windows Jump List publisher.

// Shell COM and PROPVARIANT cleanup are `unsafe` by definition; this
// module-level allow is the crate gate's only relaxation here.
#![allow(unsafe_code)]

use crate::desktop::jump_list_model::JumpListItem;
use crate::desktop::recent_files::RecentFiles;
use crate::APP_USER_MODEL_ID;
use std::mem::ManuallyDrop;
use std::path::Path;
use windows::core::{Interface, BSTR, HSTRING};
use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
use windows::Win32::System::Com::StructuredStorage::{
    PropVariantClear, PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Variant::VT_BSTR;
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink,
};

const CATEGORY: &str = "Recent scans";

pub(crate) fn publish_recent_files(recent: &RecentFiles) -> windows::core::Result<()> {
    let items = recent.jump_list_items(recent.entries().len().max(1));
    let _apartment = ComApartment::init()?;

    // SAFETY: COM is initialized for this thread and the CLSIDs/IIDs are the
    // documented shell COM classes used to publish custom Jump List categories.
    unsafe {
        let destination_list: ICustomDestinationList =
            CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        if items.is_empty() {
            destination_list.DeleteList(&HSTRING::from(APP_USER_MODEL_ID))?;
            return Ok(());
        }
        let exe_path = std::env::current_exe().map_err(|error| {
            windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                format!("could not determine the OccluView executable path: {error}"),
            )
        })?;
        destination_list.SetAppID(&HSTRING::from(APP_USER_MODEL_ID))?;
        let mut min_slots = 0;
        let _removed: IObjectArray = destination_list.BeginList(&raw mut min_slots)?;

        let collection: IObjectCollection =
            CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        for item in items {
            let link = shell_link_for_item(&exe_path, &item)?;
            collection.AddObject(&link)?;
        }

        let object_array: IObjectArray = collection.cast()?;
        destination_list.AppendCategory(&HSTRING::from(CATEGORY), &object_array)?;
        destination_list.CommitList()?;
    }

    Ok(())
}

fn shell_link_for_item(exe_path: &Path, item: &JumpListItem) -> windows::core::Result<IShellLinkW> {
    // SAFETY: COM is initialized by publish_recent_files before this helper is
    // called; ShellLink is an in-process shell COM class.
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)? };
    let exe_path = hstring_from_path(exe_path);
    let arguments = HSTRING::from_wide(&item.arguments);
    // SAFETY: The HSTRING values live for the duration of each call and are
    // copied by IShellLink.
    unsafe {
        link.SetPath(&exe_path)?;
        link.SetArguments(&arguments)?;
        link.SetDescription(&HSTRING::from(&item.tooltip))?;
        link.SetIconLocation(&exe_path, 0)?;
    }

    let property_store: IPropertyStore = link.cast()?;
    let title = OwnedPropVariant::from_bstr(&item.title);
    // SAFETY: PKEY_Title is a stable shell property key; `title` owns the
    // PROPVARIANT allocation until this function returns.
    unsafe {
        property_store.SetValue(&PKEY_Title, title.as_raw())?;
        property_store.Commit()?;
    }

    Ok(link)
}

fn hstring_from_path(path: &Path) -> HSTRING {
    use std::os::windows::ffi::OsStrExt;

    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    HSTRING::from_wide(&wide)
}

struct OwnedPropVariant(PROPVARIANT);

impl OwnedPropVariant {
    fn from_bstr(value: &str) -> Self {
        Self(PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_BSTR,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: PROPVARIANT_0_0_0 {
                        bstrVal: ManuallyDrop::new(BSTR::from(value)),
                    },
                }),
            },
        })
    }

    fn as_raw(&self) -> &PROPVARIANT {
        &self.0
    }
}

impl Drop for OwnedPropVariant {
    fn drop(&mut self) {
        // SAFETY: this value was constructed with a BSTR in PROPVARIANT form.
        let _ = unsafe { PropVariantClear(&raw mut self.0) };
    }
}

struct ComApartment;

impl ComApartment {
    fn init() -> windows::core::Result<Self> {
        // SAFETY: We request STA for the current thread before using Shell COM.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: Paired with successful CoInitializeEx in ComApartment::init.
        unsafe { CoUninitialize() };
    }
}

#[cfg(test)]
mod tests {
    use super::hstring_from_path;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    #[test]
    fn executable_path_hstring_preserves_native_utf16_units() {
        let native_path = [
            0xD800,
            u16::from(b'.'),
            u16::from(b'e'),
            u16::from(b'x'),
            u16::from(b'e'),
        ];
        let path = PathBuf::from(OsString::from_wide(&native_path));

        let shell_path = hstring_from_path(&path);

        assert_eq!(&*shell_path, native_path.as_slice());
    }
}
