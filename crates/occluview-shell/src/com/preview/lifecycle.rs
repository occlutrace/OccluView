//! Preview-child lifecycle and source reset operations.

use super::window::ensure_preview_window_class;
use super::*;

impl PreviewHandler {
    pub(super) fn initialize_path(&self, path: PathBuf) {
        self.source
            .borrow_mut()
            .initialize_path(path.clone(), path_extension(&path));
        self.oversize_stream_len.set(None);
        self.preview_scene.borrow_mut().take();
    }

    pub(super) fn preview_size(&self) -> (u32, u32) {
        let rect = *self.rect.borrow();
        (
            (rect.right - rect.left).unsigned_abs().max(1),
            (rect.bottom - rect.top).unsigned_abs().max(1),
        )
    }

    pub(super) fn preview_size_u16(&self) -> [u16; 2] {
        let (width, height) = self.preview_size();
        [
            width.clamp(1, u32::from(u16::MAX)) as u16,
            height.clamp(1, u32::from(u16::MAX)) as u16,
        ]
    }

    pub(super) fn ensure_preview_window(&self) -> windows::core::Result<HWND> {
        let hwnd = self.preview_hwnd.get();
        if !hwnd.0.is_null() {
            return Ok(hwnd);
        }
        let parent = self.parent_hwnd.get();
        if parent.0.is_null() {
            return Err(e_fail());
        }
        ensure_preview_window_class()?;
        let rect = *self.rect.borrow();
        let (width, height) = self.preview_size();
        let style = WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS;
        let create_param = std::ptr::from_ref::<Self>(self) as *const std::ffi::c_void;
        let module = own_pinned_dll_module().map_err(|_| e_fail())?;
        // SAFETY: Explorer supplied `parent` via IPreviewHandler::SetWindow.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PREVIEW_WINDOW_CLASS_NAME,
                w!(""),
                style,
                rect.left,
                rect.top,
                width as i32,
                height as i32,
                Some(parent),
                None,
                Some(HINSTANCE(module.0)),
                Some(create_param),
            )
        }?;
        self.preview_hwnd.set(hwnd);
        Ok(hwnd)
    }

    pub(super) fn resize_preview_window(&self) -> windows::core::Result<()> {
        let hwnd = self.preview_hwnd.get();
        if hwnd.0.is_null() {
            return Ok(());
        }
        let rect = *self.rect.borrow();
        let (width, height) = self.preview_size();
        // SAFETY: `hwnd` is a child window created by this object.
        unsafe { MoveWindow(hwnd, rect.left, rect.top, width as i32, height as i32, true) }?;
        Ok(())
    }

    pub(super) fn destroy_preview_window(&self) {
        let hwnd = self.preview_hwnd.replace(HWND::default());
        if !hwnd.0.is_null() {
            // Clear the raw pointer before destruction. Destruction may fail on
            // the wrong thread, so leaving it behind would be a use-after-free.
            // SAFETY: `hwnd` is a child window created by this object.
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0)
            };
            // SAFETY: `hwnd` is a child window created by this object.
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        if let Some(previous) = self.preview_bitmap.borrow_mut().take() {
            // SAFETY: the bitmap was allocated by this module.
            let _ = unsafe { DeleteObject(HGDIOBJ(previous.0)) };
        }
    }

    pub(super) fn clear_loaded_content(&self) {
        self.source.borrow_mut().clear_all();
        self.oversize_stream_len.set(None);
        self.preview_scene.borrow_mut().take();
        self.drag_mode.set(PreviewDragMode::None);
        self.drag_moved.set(false);
    }
}
