//! Windows Jump List publisher.

#![allow(unsafe_code)]

use super::APP_USER_MODEL_ID;
use occluview_core::{JumpListItem, RecentFiles};
use std::path::Path;
use windows::core::{Interface, HSTRING};
use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink,
};

const CATEGORY: &str = "Recent scans";

pub(crate) fn publish_recent_files(recent: &RecentFiles) -> windows::core::Result<()> {
    let items = recent.jump_list_items(RecentFiles::DEFAULT_LIMIT);
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
        let exe_path = std::env::current_exe().map_err(|_| windows::core::Error::from_win32())?;
        destination_list.SetAppID(&HSTRING::from(APP_USER_MODEL_ID))?;
        let mut min_slots = 0;
        let _removed: IObjectArray = destination_list.BeginList(&mut min_slots)?;

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
    let exe_path = HSTRING::from(exe_path.display().to_string());
    // SAFETY: The HSTRING values live for the duration of each call and are
    // copied by IShellLink.
    unsafe {
        link.SetPath(&exe_path)?;
        link.SetArguments(&HSTRING::from(&item.arguments))?;
        link.SetDescription(&HSTRING::from(&item.tooltip))?;
        link.SetIconLocation(&exe_path, 0)?;
    }

    let property_store: IPropertyStore = link.cast()?;
    let title = windows::core::PROPVARIANT::from(item.title.as_str());
    // SAFETY: PKEY_Title is a stable shell property key; the PROPVARIANT owns
    // its BSTR until SetValue returns.
    unsafe {
        property_store.SetValue(&PKEY_Title, &title)?;
        property_store.Commit()?;
    }

    Ok(link)
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
