//! Right-click context menu for the Explorer preview pane.
//!
//! A stationary right-click inside the live 3D preview pops a native Win32 menu
//! (built from the platform-agnostic model in [`crate::preview_menu`]) with:
//! Open / Edit in OccluView, the Front / Top / Side / Isometric view presets and
//! Fit view, a checkable Wireframe toggle, and Copy image. Each item carries a
//! runtime-rasterised 16 px icon tinted to the system menu-text colour.
//!
//! A right-*drag* still orbits the camera (see `window.rs`): the menu only
//! appears when the right button is released without movement, so this adds a
//! capability without changing the existing mouse semantics.
//!
//! This whole module is `cfg(windows)` (it lives under `com`); the reusable
//! logic — the menu inventory, the icon raster, and the clipboard DIB packing —
//! is factored into `crate::preview_menu`, which is unit tested on any host.

use super::super::e_fail;
use super::PreviewHandler;
use crate::preview_menu::dib::pack_clipboard_dib;
use crate::preview_menu::icons::PreviewMenuIcon;
use crate::preview_menu::{PreviewMenuCommand, PreviewMenuEntry, PREVIEW_MENU_LAYOUT};
use crate::preview_scene::PreviewSceneState;
use std::mem::size_of;
use std::path::PathBuf;
use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{GlobalFree, BOOL, HANDLE, HMODULE, HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreateDIBSection, DeleteObject, GetSysColor, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, COLOR_MENUTEXT, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_DIB;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, InsertMenuItemW, SetMenuDefaultItem, TrackPopupMenuEx,
    MENUITEMINFOW, MENU_ITEM_STATE, MFS_CHECKED, MFT_SEPARATOR, MFT_STRING, MIIM_BITMAP,
    MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_STRING, SW_SHOWNORMAL, TPM_LEFTALIGN, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN,
};

/// Base icon edge at 96 DPI; scaled up for high-DPI displays.
const BASE_ICON_PX: u32 = 16;

/// Which app intent a launch item carries. The two currently produce the same
/// command line (see [`PreviewHandler::launch_in_app`]).
#[derive(Clone, Copy)]
enum LaunchIntent {
    Open,
    Edit,
}

impl PreviewHandler {
    /// Build and track the context menu at a client-space point, then run the
    /// selected command. Best-effort: any failure leaves the preview untouched.
    #[allow(clippy::too_many_lines)]
    pub(super) fn show_context_menu(&self, hwnd: HWND, client_point: POINT) {
        let wireframe_on = self
            .preview_scene
            .borrow()
            .as_ref()
            .is_some_and(PreviewSceneState::is_wireframe);
        let icon_px = menu_icon_size_px(hwnd);

        // SAFETY: creates a fresh, unowned popup menu we destroy below.
        let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
            return;
        };

        // Icon bitmaps are referenced (not copied) by the menu, so they must
        // outlive TrackPopupMenuEx; free them only after the menu is gone.
        let mut icons: Vec<HBITMAP> = Vec::new();
        let mut default_id: Option<u32> = None;
        for entry in PREVIEW_MENU_LAYOUT {
            match entry {
                PreviewMenuEntry::Separator => {
                    let info = MENUITEMINFOW {
                        cbSize: size_of::<MENUITEMINFOW>() as u32,
                        fMask: MIIM_FTYPE,
                        fType: MFT_SEPARATOR,
                        ..Default::default()
                    };
                    // SAFETY: `menu` is live and `info` is a valid separator descriptor.
                    let _ = unsafe { InsertMenuItemW(menu, u32::MAX, BOOL(1), &info) };
                }
                PreviewMenuEntry::Command(command) => {
                    let command = *command;
                    if command.is_default() {
                        default_id = Some(command.id());
                    }
                    let bitmap = menu_icon_hbitmap(command.icon(), icon_px);
                    if !bitmap.0.is_null() {
                        icons.push(bitmap);
                    }

                    let mut label: Vec<u16> = command
                        .label()
                        .encode_utf16()
                        .chain(std::iter::once(0))
                        .collect();
                    let mut mask = MIIM_ID | MIIM_STRING | MIIM_FTYPE | MIIM_BITMAP;
                    let mut state = MENU_ITEM_STATE::default();
                    if command.is_checkable() {
                        mask |= MIIM_STATE;
                        if wireframe_on {
                            state = MFS_CHECKED;
                        }
                    }
                    let info = MENUITEMINFOW {
                        cbSize: size_of::<MENUITEMINFOW>() as u32,
                        fMask: mask,
                        fType: MFT_STRING,
                        fState: state,
                        wID: command.id(),
                        hbmpItem: bitmap,
                        dwTypeData: PWSTR(label.as_mut_ptr()),
                        cch: (label.len() - 1) as u32,
                        ..Default::default()
                    };
                    // SAFETY: `menu` is live; `info`/`label` are valid for the
                    // duration of this call, which copies the string internally.
                    let _ = unsafe { InsertMenuItemW(menu, u32::MAX, BOOL(1), &info) };
                }
            }
        }

        if let Some(id) = default_id {
            // SAFETY: sets the bold default item by command id.
            let _ = unsafe { SetMenuDefaultItem(menu, id, 0) };
        }

        let mut screen_point = client_point;
        // SAFETY: `hwnd` is our preview child window; `screen_point` is a valid out-param.
        let _ = unsafe { ClientToScreen(hwnd, &mut screen_point) };

        let flags =
            TPM_RETURNCMD.0 | TPM_NONOTIFY.0 | TPM_LEFTALIGN.0 | TPM_TOPALIGN.0 | TPM_RIGHTBUTTON.0;
        // SAFETY: modal tracking on our own menu/window; TPM_RETURNCMD returns
        // the selected command id in the BOOL's numeric field.
        let selection =
            unsafe { TrackPopupMenuEx(menu, flags, screen_point.x, screen_point.y, hwnd, None) };

        // SAFETY: destroy the menu, then the app-owned icon bitmaps it referenced.
        let _ = unsafe { DestroyMenu(menu) };
        for bitmap in icons {
            // SAFETY: each bitmap was created by this module and is no longer in use.
            let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
        }

        // TPM_RETURNCMD packs the selected command id into the BOOL's i32; menu
        // ids are always positive, and 0 means "nothing selected".
        let selected_id = u32::try_from(selection.0).unwrap_or(0);
        if selected_id != 0 {
            if let Some(command) = PreviewMenuCommand::from_id(selected_id) {
                let _ = self.run_menu_command(hwnd, command);
            }
        }
    }

    /// Fit-view keyboard shortcut (`F`).
    pub(super) fn key_fit_view(&self, hwnd: HWND) {
        let _ = self.run_menu_command(hwnd, PreviewMenuCommand::FitView);
    }

    /// Wireframe-toggle keyboard shortcut (`W`).
    pub(super) fn key_toggle_wireframe(&self, hwnd: HWND) {
        let _ = self.run_menu_command(hwnd, PreviewMenuCommand::ToggleWireframe);
    }

    fn run_menu_command(
        &self,
        hwnd: HWND,
        command: PreviewMenuCommand,
    ) -> windows::core::Result<()> {
        match command {
            PreviewMenuCommand::Open => {
                self.launch_in_app(hwnd, LaunchIntent::Open);
                Ok(())
            }
            PreviewMenuCommand::Edit => {
                self.launch_in_app(hwnd, LaunchIntent::Edit);
                Ok(())
            }
            PreviewMenuCommand::CopyImage => self.copy_preview_to_clipboard(hwnd),
            PreviewMenuCommand::FitView => {
                self.mutate_scene_and_repaint(PreviewSceneState::fit_view)
            }
            PreviewMenuCommand::ToggleWireframe => self.mutate_scene_and_repaint(|scene| {
                let enabled = !scene.is_wireframe();
                scene.set_wireframe(enabled)
            }),
            other => {
                if let Some(preset) = other.view_preset() {
                    self.mutate_scene_and_repaint(move |scene| scene.apply_view_preset(preset))
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Apply a mutation to the loaded scene and repaint if it changed anything.
    fn mutate_scene_and_repaint(
        &self,
        change: impl FnOnce(&mut PreviewSceneState) -> bool,
    ) -> windows::core::Result<()> {
        self.ensure_preview_scene_loaded()
            .map_err(super::shell_error_to_hresult)?;
        let changed = {
            let mut scene = self.preview_scene.borrow_mut();
            scene.as_mut().is_some_and(change)
        };
        if changed {
            self.render_preview_now()?;
        }
        Ok(())
    }

    fn launch_in_app(&self, hwnd: HWND, intent: LaunchIntent) {
        let Some(path) = self.source.borrow().path().map(PathBuf::from) else {
            // Stream-only preview with no filesystem path: nothing to launch.
            return;
        };
        let Some(exe) = resolve_app_exe() else {
            tracing::warn!("could not resolve occluview.exe next to the shell DLL");
            return;
        };
        // occluview-app has no `--edit` verb yet (its argument parser treats any
        // unknown argument as a file path), so BOTH intents currently open the
        // viewer with just the file. When the app gains `--edit`, prepend it in
        // the Edit arm below — the only change needed here.
        let params = match intent {
            LaunchIntent::Open | LaunchIntent::Edit => {
                HSTRING::from(format!("\"{}\"", path.display()))
            }
        };
        // SAFETY: all string pointers stay alive across this synchronous call.
        let result = unsafe {
            ShellExecuteW(
                hwnd,
                w!("open"),
                PCWSTR(exe.as_ptr()),
                PCWSTR(params.as_ptr()),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if (result.0 as usize) <= 32 {
            tracing::warn!(
                code = result.0 as usize,
                "ShellExecuteW failed to launch OccluView"
            );
        }
    }

    fn copy_preview_to_clipboard(&self, hwnd: HWND) -> windows::core::Result<()> {
        self.ensure_preview_scene_loaded()
            .map_err(super::shell_error_to_hresult)?;
        let size = self.preview_size_u16();
        let theme = super::theme::preview_theme();
        let pixels = self
            .render_preview_pixels(size, theme.background_linear(), theme.canvas_rgba())
            .map_err(super::shell_error_to_hresult)?;
        copy_rgba_to_clipboard(hwnd, &pixels, u32::from(size[0]), u32::from(size[1]))
    }
}

/// Icon edge in pixels for the given window's DPI.
fn menu_icon_size_px(hwnd: HWND) -> u32 {
    // SAFETY: `hwnd` is our live preview window.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi == 0 { 96 } else { dpi };
    ((BASE_ICON_PX * dpi) / 96).clamp(BASE_ICON_PX, 48)
}

/// Rasterise a glyph, tint it with the system menu-text colour, premultiply it,
/// and hand back a 32bpp top-down `HBITMAP`. Returns a null handle on failure so
/// the item still shows (just without an icon).
fn menu_icon_hbitmap(icon: PreviewMenuIcon, size_px: u32) -> HBITMAP {
    let mask = icon.rasterize(size_px);
    // SAFETY: a pure system-colour query with no resources involved.
    let color = unsafe { GetSysColor(COLOR_MENUTEXT) };
    let (cr, cg, cb) = (color & 0xFF, (color >> 8) & 0xFF, (color >> 16) & 0xFF);

    let pixel_count = (size_px * size_px) as usize;
    if mask.len() != pixel_count * 4 {
        return HBITMAP::default();
    }
    let mut bgra = vec![0u8; pixel_count * 4];
    for i in 0..pixel_count {
        let a = u32::from(mask[i * 4 + 3]);
        bgra[i * 4] = ((cb * a) / 255) as u8; // premultiplied B
        bgra[i * 4 + 1] = ((cg * a) / 255) as u8; // premultiplied G
        bgra[i * 4 + 2] = ((cr * a) / 255) as u8; // premultiplied R
        bgra[i * 4 + 3] = a as u8; // straight alpha
    }
    create_premultiplied_dib(size_px, size_px, &bgra).unwrap_or_default()
}

/// Build a 32bpp top-down `HBITMAP` from premultiplied BGRA bytes.
fn create_premultiplied_dib(
    width: u32,
    height: u32,
    bgra: &[u8],
) -> windows::core::Result<HBITMAP> {
    if width == 0 || height == 0 || bgra.len() != (width * height * 4) as usize {
        return Err(e_fail());
    }
    let bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32), // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: width * height * 4,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    // SAFETY: valid 32bpp DIB descriptor; `bits` is written by GDI; caller owns the handle.
    let hbmp = unsafe {
        CreateDIBSection(
            HDC::default(),
            &bitmap_info,
            DIB_RGB_COLORS,
            &mut bits,
            HANDLE::default(),
            0,
        )
    }?;
    if bits.is_null() {
        // SAFETY: free the just-allocated bitmap on the defensive null-bits path.
        let _ = unsafe { DeleteObject(HGDIOBJ(hbmp.0)) };
        return Err(e_fail());
    }
    // SAFETY: GDI allocated width*height*4 bytes and `bgra` has exactly that many.
    unsafe { std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits.cast::<u8>(), bgra.len()) };
    Ok(hbmp)
}

/// Copy a top-down RGBA frame to the clipboard as a `CF_DIB` bitmap.
fn copy_rgba_to_clipboard(
    hwnd: HWND,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> windows::core::Result<()> {
    let Some(dib) = pack_clipboard_dib(rgba, width, height) else {
        return Err(e_fail());
    };
    // SAFETY: allocates a moveable global block of the exact DIB size.
    let hglobal = unsafe { GlobalAlloc(GMEM_MOVEABLE, dib.len()) }?;
    // SAFETY: `hglobal` was just allocated.
    let ptr = unsafe { GlobalLock(hglobal) };
    if ptr.is_null() {
        // SAFETY: releasing the block we failed to lock.
        let _ = unsafe { GlobalFree(hglobal) };
        return Err(e_fail());
    }
    // SAFETY: `ptr` addresses at least `dib.len()` writable bytes.
    unsafe { std::ptr::copy_nonoverlapping(dib.as_ptr(), ptr.cast::<u8>(), dib.len()) };
    // SAFETY: matching unlock; a 0 return (fully unlocked) is expected, not an error.
    let _ = unsafe { GlobalUnlock(hglobal) };

    // SAFETY: take ownership of the clipboard tied to our window.
    unsafe { OpenClipboard(hwnd) }?;
    // SAFETY: clipboard is open; empty it before publishing our format.
    let outcome = unsafe { EmptyClipboard() }.and_then(|()| {
        // SAFETY: on success the system takes ownership of `hglobal`.
        unsafe { SetClipboardData(u32::from(CF_DIB.0), HANDLE(hglobal.0)) }.map(|_| ())
    });
    // SAFETY: always release the clipboard we opened.
    let _ = unsafe { CloseClipboard() };

    if outcome.is_err() {
        // The system never took ownership; reclaim the block.
        // SAFETY: `hglobal` is still ours on the failure path.
        let _ = unsafe { GlobalFree(hglobal) };
    }
    outcome
}

/// Resolve `occluview.exe` sitting next to this shell DLL.
fn resolve_app_exe() -> Option<HSTRING> {
    // An address inside our own mapped image, used to find the DLL's module.
    // Typed as `u16` so it can become a `PCWSTR` (an opaque address here, never
    // dereferenced) without an alignment-widening pointer cast.
    static ANCHOR: u16 = 0;
    let mut module = HMODULE::default();
    let flags =
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    let address = PCWSTR(core::ptr::addr_of!(ANCHOR));
    // SAFETY: `address` lies within this DLL; `module` is a valid out-param.
    unsafe { GetModuleHandleExW(flags, address, &mut module) }.ok()?;

    let mut buffer = [0u16; 1024];
    // SAFETY: `module` is our DLL handle; `buffer` is a valid write region.
    let len = unsafe { GetModuleFileNameW(module, &mut buffer) } as usize;
    if len == 0 || len >= buffer.len() {
        return None;
    }
    let dll_path = HSTRING::from_wide(&buffer[..len]).ok()?;
    sibling_file(&dll_path, crate::APP_EXE_NAME)
}

/// Replace the file name of `path` with `filename`, keeping the directory.
fn sibling_file(path: &HSTRING, filename: &str) -> Option<HSTRING> {
    let wide = path.as_wide();
    let sep = wide.iter().rposition(|&c| c == u16::from(b'\\'))?;
    let mut full: Vec<u16> = wide[..=sep].to_vec();
    full.extend(filename.encode_utf16());
    HSTRING::from_wide(&full).ok()
}
