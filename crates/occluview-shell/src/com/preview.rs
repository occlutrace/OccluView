use super::{
    center_square_on_canvas, e_fail, e_notimpl, e_pointer, implement, path_extension,
    pixels_to_hbitmap, placeholder_for_oversize_input, s_false, w, win32_preview_orbit_delta,
    BeginPaint, BitBlt, CoTaskMemFree, CreateCompatibleDC, CreateWindowExW, DeferredSource,
    DeleteDC, DeleteObject, DestroyWindow, EndPaint, GetKeyboardFocus, GetModuleHandleW,
    IClassFactory, IInitializeWithFile, IInitializeWithFile_Impl, IInitializeWithItem,
    IInitializeWithItem_Impl, IInitializeWithStream, IInitializeWithStream_Impl, IObjectWithSite,
    IObjectWithSite_Impl, IOleWindow, IOleWindow_Impl, IPreviewHandler, IPreviewHandler_Impl,
    IShellItem, IStream, IUnknown, Interface, MoveWindow, Ordering, PathBuf, PreviewSceneState,
    RedrawWindow, SelectObject, SetKeyboardFocus, SetParent, ShellError, StreamRead,
    ThumbnailProvider, ThumbnailSpec, Vec2, ACTIVE_COM_OBJECTS, BOOL, GUID, HBITMAP, HGDIOBJ,
    HINSTANCE, HMENU, HRESULT, HWND, MAX_PREVIEW_EDGE, MSG, PAINTSTRUCT, PCWSTR, POINT,
    PREVIEW_WINDOW_CLASS_NAME, RDW_INVALIDATE, RDW_UPDATENOW, RECT, SIGDN_FILESYSPATH, SRCCOPY,
    WINDOW_EX_STYLE, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
};

mod context_menu;
mod theme;
mod window;

use theme::preview_theme;
use window::ensure_preview_window_class;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PreviewDragMode {
    #[default]
    None,
    Orbit,
    Pan,
}

/// Explorer Preview Pane handler.
///
/// Unlike thumbnails, the preview path keeps a GPU-prepared scene resident
/// after the first load. That allows resizes and pointer interaction to
/// re-render the same file without reparsing or re-uploading the mesh payload.
#[implement(
    IPreviewHandler,
    IOleWindow,
    IObjectWithSite,
    IInitializeWithFile,
    IInitializeWithItem,
    IInitializeWithStream,
    IClassFactory
)]
pub struct PreviewHandler {
    source: std::cell::RefCell<DeferredSource<IStream>>,
    oversize_stream_len: std::cell::Cell<Option<usize>>,
    parent_hwnd: std::cell::Cell<HWND>,
    preview_hwnd: std::cell::Cell<HWND>,
    preview_bitmap: std::cell::RefCell<Option<HBITMAP>>,
    preview_scene: std::cell::RefCell<Option<PreviewSceneState>>,
    rect: std::cell::RefCell<RECT>,
    site: std::cell::RefCell<Option<IUnknown>>,
    drag_mode: std::cell::Cell<PreviewDragMode>,
    last_pointer: std::cell::Cell<POINT>,
    drag_moved: std::cell::Cell<bool>,
}

impl PreviewHandler {
    pub fn new() -> Self {
        ACTIVE_COM_OBJECTS.fetch_add(1, Ordering::AcqRel);
        Self {
            source: std::cell::RefCell::new(DeferredSource::default()),
            oversize_stream_len: std::cell::Cell::new(None),
            parent_hwnd: std::cell::Cell::new(HWND::default()),
            preview_hwnd: std::cell::Cell::new(HWND::default()),
            preview_bitmap: std::cell::RefCell::new(None),
            preview_scene: std::cell::RefCell::new(None),
            rect: std::cell::RefCell::new(RECT::default()),
            site: std::cell::RefCell::new(None),
            drag_mode: std::cell::Cell::new(PreviewDragMode::None),
            last_pointer: std::cell::Cell::new(POINT::default()),
            drag_moved: std::cell::Cell::new(false),
        }
    }

    fn initialize_path(&self, path: PathBuf) {
        self.source
            .borrow_mut()
            .initialize_path(path.clone(), path_extension(&path));
        self.oversize_stream_len.set(None);
        self.preview_scene.borrow_mut().take();
    }

    fn preview_size(&self) -> (u32, u32) {
        let rect = *self.rect.borrow();
        (
            (rect.right - rect.left).unsigned_abs().max(1),
            (rect.bottom - rect.top).unsigned_abs().max(1),
        )
    }

    fn preview_size_u16(&self) -> [u16; 2] {
        let (width, height) = self.preview_size();
        [
            width.clamp(1, u32::from(u16::MAX)) as u16,
            height.clamp(1, u32::from(u16::MAX)) as u16,
        ]
    }

    fn preview_render_to_hbitmap(&self, width: u32, height: u32) -> windows::core::Result<HBITMAP> {
        let width = width.clamp(1, MAX_PREVIEW_EDGE);
        let height = height.clamp(1, MAX_PREVIEW_EDGE);
        let theme = preview_theme();
        let pixels = match self.render_preview_pixels(
            [width as u16, height as u16],
            theme.background_linear(),
            theme.canvas_rgba(),
        ) {
            Ok(pixels) => pixels,
            Err(error) => {
                tracing::warn!(?error, "preview render failed; returning placeholder");
                let preview_edge_px = width.min(height).clamp(1, MAX_PREVIEW_EDGE) as u16;
                let spec = ThumbnailSpec {
                    size_px: preview_edge_px,
                    background: [0.0, 0.0, 0.0, 0.0],
                };
                let square = if let Some(byte_len) = self.oversize_stream_len.get() {
                    placeholder_for_oversize_input(spec, byte_len)
                } else {
                    crate::placeholder::placeholder_thumbnail(spec)
                };
                center_square_on_canvas(
                    &square,
                    preview_edge_px,
                    width,
                    height,
                    theme.canvas_rgba(),
                )
            }
        };
        pixels_to_hbitmap(&pixels, width, height)
    }

    fn render_preview_pixels(
        &self,
        size_px: [u16; 2],
        background_linear: [f64; 4],
        canvas_rgba: [u8; 4],
    ) -> Result<Vec<u8>, ShellError> {
        if let Some(byte_len) = self.oversize_stream_len.get() {
            let preview_edge_px = u32::from(size_px[0]).min(u32::from(size_px[1])) as u16;
            let spec = ThumbnailSpec {
                size_px: preview_edge_px.max(1),
                background: [0.0, 0.0, 0.0, 0.0],
            };
            let square = placeholder_for_oversize_input(spec, byte_len);
            return Ok(center_square_on_canvas(
                &square,
                spec.size_px,
                u32::from(size_px[0]),
                u32::from(size_px[1]),
                canvas_rgba,
            ));
        }

        self.ensure_preview_scene_loaded()?;
        let preview = self.preview_scene.borrow();
        let state = preview
            .as_ref()
            .ok_or_else(|| ShellError::Win32("preview scene unavailable".to_string()))?;
        state.render_rgba_with_background(size_px, background_linear)
    }

    fn ensure_preview_scene_loaded(&self) -> Result<(), ShellError> {
        if self.preview_scene.borrow().is_some() || self.oversize_stream_len.get().is_some() {
            return Ok(());
        }

        let source_path = self.source.borrow().path().map(PathBuf::from);
        let state = if let Some(path) = source_path {
            PreviewSceneState::from_file(&path)?
        } else if let Some(stream_result) =
            self.source
                .borrow_mut()
                .consume_pending_stream(|stream, extension| {
                    ThumbnailProvider::rewind_stream(&stream).map_err(|_| {
                        ShellError::Win32("rewinding preview stream failed".to_string())
                    })?;
                    let read = ThumbnailProvider::read_stream(&stream).map_err(|_| {
                        ShellError::Win32("reading preview stream failed".to_string())
                    })?;
                    Ok::<_, ShellError>((read, extension.map(str::to_owned)))
                })
        {
            match stream_result? {
                (StreamRead::Complete(bytes), extension) => {
                    PreviewSceneState::from_bytes(extension.as_deref(), &bytes)?
                }
                (StreamRead::OverCap { byte_len }, _extension) => {
                    self.oversize_stream_len.set(Some(byte_len));
                    return Ok(());
                }
                (StreamRead::ReadFailed, _extension) => {
                    return Err(ShellError::Win32(
                        "reading preview stream failed".to_string(),
                    ));
                }
            }
        } else {
            return Err(ShellError::Win32(
                "preview handler has no file or stream source".to_string(),
            ));
        };
        *self.preview_scene.borrow_mut() = Some(state);
        Ok(())
    }

    fn render_preview_now(&self) -> windows::core::Result<()> {
        let hwnd = self.preview_hwnd.get();
        if hwnd.0.is_null() {
            return Err(e_fail());
        }
        let (width, height) = self.preview_size();
        let hbmp = self.preview_render_to_hbitmap(width, height)?;
        self.replace_preview_bitmap(hbmp);
        // SAFETY: `hwnd` is our live preview child window.
        if unsafe { RedrawWindow(hwnd, None, None, RDW_INVALIDATE | RDW_UPDATENOW) }.0 == 0 {
            return Err(e_fail());
        }
        Ok(())
    }

    fn replace_preview_bitmap(&self, hbmp: HBITMAP) {
        if let Some(previous) = self.preview_bitmap.borrow_mut().replace(hbmp) {
            // SAFETY: the previous bitmap was allocated by this module.
            let _ = unsafe { DeleteObject(HGDIOBJ(previous.0)) };
        }
    }

    fn ensure_preview_window(&self) -> windows::core::Result<HWND> {
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
        let module = unsafe { GetModuleHandleW(None) }.map_err(|_| e_fail())?;
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
                parent,
                HMENU::default(),
                HINSTANCE(module.0),
                Some(create_param),
            )
        }?;
        self.preview_hwnd.set(hwnd);
        Ok(hwnd)
    }

    fn resize_preview_window(&self) -> windows::core::Result<()> {
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

    fn destroy_preview_window(&self) {
        let hwnd = self.preview_hwnd.replace(HWND::default());
        if !hwnd.0.is_null() {
            // SAFETY: `hwnd` is a child window created by this object.
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        if let Some(previous) = self.preview_bitmap.borrow_mut().take() {
            // SAFETY: the bitmap was allocated by this module.
            let _ = unsafe { DeleteObject(HGDIOBJ(previous.0)) };
        }
    }

    fn clear_loaded_content(&self) {
        self.source.borrow_mut().clear_all();
        self.oversize_stream_len.set(None);
        self.preview_scene.borrow_mut().take();
        self.drag_mode.set(PreviewDragMode::None);
        self.drag_moved.set(false);
    }

    fn begin_drag(&self, mode: PreviewDragMode, pointer: POINT) {
        self.drag_mode.set(mode);
        self.last_pointer.set(pointer);
        self.drag_moved.set(false);
    }

    fn update_drag(&self, pointer: POINT) -> windows::core::Result<()> {
        let previous = self.last_pointer.replace(pointer);
        let delta = Vec2::new(
            (pointer.x - previous.x) as f32,
            (pointer.y - previous.y) as f32,
        );
        if delta.length_squared() <= f32::EPSILON {
            return Ok(());
        }
        self.drag_moved.set(true);
        self.ensure_preview_scene_loaded()
            .map_err(shell_error_to_hresult)?;
        let size_px = self.preview_size_u16();
        let changed = {
            let mut preview = self.preview_scene.borrow_mut();
            let Some(state) = preview.as_mut() else {
                return Ok(());
            };
            match self.drag_mode.get() {
                PreviewDragMode::Orbit => {
                    state.orbit_drag_delta(win32_preview_orbit_delta(delta), size_px)
                }
                PreviewDragMode::Pan => state.pan_drag(delta, size_px),
                PreviewDragMode::None => false,
            }
        };
        if changed {
            self.render_preview_now()?;
        }
        Ok(())
    }

    fn end_drag(&self) {
        self.drag_mode.set(PreviewDragMode::None);
    }

    fn zoom_preview(&self, scroll_y: f32) -> windows::core::Result<()> {
        self.ensure_preview_scene_loaded()
            .map_err(shell_error_to_hresult)?;
        let changed = {
            let mut preview = self.preview_scene.borrow_mut();
            preview
                .as_mut()
                .is_some_and(|state| state.zoom_scroll(scroll_y))
        };
        if changed {
            self.render_preview_now()?;
        }
        Ok(())
    }

    fn focus_preview_point(&self, pointer: POINT) -> windows::core::Result<()> {
        self.ensure_preview_scene_loaded()
            .map_err(shell_error_to_hresult)?;
        let changed = {
            let mut preview = self.preview_scene.borrow_mut();
            preview.as_mut().is_some_and(|state| {
                state.focus_pointer(
                    Vec2::new(pointer.x as f32, pointer.y as f32),
                    self.preview_size_u16(),
                )
            })
        };
        if changed {
            self.render_preview_now()?;
        }
        Ok(())
    }

    fn paint_preview(&self, hwnd: HWND) {
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: `hwnd` is our preview child window and `paint` is valid.
        let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
        if let Some(bitmap) = *self.preview_bitmap.borrow() {
            // SAFETY: `hdc` is valid for the active paint cycle.
            let memory_dc = unsafe { CreateCompatibleDC(hdc) };
            if !memory_dc.0.is_null() {
                // SAFETY: the bitmap handle is owned by this module.
                let previous = unsafe { SelectObject(memory_dc, HGDIOBJ(bitmap.0)) };
                let (width, height) = self.preview_size();
                // SAFETY: both DCs are valid for this paint cycle.
                let _ = unsafe {
                    BitBlt(
                        hdc,
                        0,
                        0,
                        width as i32,
                        height as i32,
                        memory_dc,
                        0,
                        0,
                        SRCCOPY,
                    )
                };
                // SAFETY: restore the previous selected object before deleting the DC.
                let _ = unsafe { SelectObject(memory_dc, previous) };
                // SAFETY: the temporary memory DC was created above.
                let _ = unsafe { DeleteDC(memory_dc) };
            }
        }
        // SAFETY: completes the paint cycle begun with BeginPaint above.
        let _ = unsafe { EndPaint(hwnd, &paint) };
    }
}

impl Default for PreviewHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PreviewHandler {
    fn drop(&mut self) {
        ACTIVE_COM_OBJECTS.fetch_sub(1, Ordering::AcqRel);
    }
}

impl IPreviewHandler_Impl for PreviewHandler_Impl {
    fn SetWindow(&self, hwnd: HWND, prc: *const RECT) -> windows::core::Result<()> {
        if prc.is_null() {
            return Err(e_pointer());
        }
        if hwnd.0.is_null() {
            return Err(e_fail());
        }
        let previous_parent = self.this.parent_hwnd.replace(hwnd);
        let preview = self.this.preview_hwnd.get();
        if !preview.0.is_null() && previous_parent != hwnd {
            // SAFETY: `preview` is our live child preview window.
            let _ = unsafe { SetParent(preview, hwnd)? };
        }
        // SAFETY: `prc` is a caller-owned RECT pointer valid for this call.
        *self.this.rect.borrow_mut() = unsafe { *prc };
        self.this.resize_preview_window()?;
        if !preview.0.is_null() && self.this.preview_bitmap.borrow().is_some() {
            self.this.render_preview_now()?;
        }
        Ok(())
    }

    fn SetRect(&self, prc: *const RECT) -> windows::core::Result<()> {
        if prc.is_null() {
            return Err(e_pointer());
        }
        // SAFETY: `prc` is a caller-owned RECT pointer valid for this call.
        *self.this.rect.borrow_mut() = unsafe { *prc };
        self.this.resize_preview_window()?;
        if self.this.preview_bitmap.borrow().is_some() {
            self.this.render_preview_now()?;
        }
        Ok(())
    }

    fn DoPreview(&self) -> windows::core::Result<()> {
        let _ = self.this.ensure_preview_window()?;
        self.this.render_preview_now()
    }

    fn Unload(&self) -> windows::core::Result<()> {
        self.this.destroy_preview_window();
        self.this.clear_loaded_content();
        Ok(())
    }

    fn SetFocus(&self) -> windows::core::Result<()> {
        let target = {
            let preview = self.this.preview_hwnd.get();
            if preview.0.is_null() {
                self.this.parent_hwnd.get()
            } else {
                preview
            }
        };
        if target.0.is_null() {
            return Err(e_fail());
        }
        // SAFETY: `target` is either our preview child or the host parent.
        let _ = unsafe { SetKeyboardFocus(target) };
        Ok(())
    }

    fn QueryFocus(&self) -> windows::core::Result<HWND> {
        // SAFETY: Win32 returns the HWND with focus for the current thread.
        Ok(unsafe { GetKeyboardFocus() })
    }

    fn TranslateAccelerator(&self, _pmsg: *const MSG) -> windows::core::Result<()> {
        Err(s_false())
    }
}

impl IOleWindow_Impl for PreviewHandler_Impl {
    fn GetWindow(&self) -> windows::core::Result<HWND> {
        let preview = self.this.preview_hwnd.get();
        if preview.0.is_null() {
            Err(e_fail())
        } else {
            Ok(preview)
        }
    }

    fn ContextSensitiveHelp(&self, _fentermode: BOOL) -> windows::core::Result<()> {
        Err(e_notimpl())
    }
}

impl IObjectWithSite_Impl for PreviewHandler_Impl {
    fn SetSite(&self, punksite: Option<&IUnknown>) -> windows::core::Result<()> {
        *self.this.site.borrow_mut() = punksite.cloned();
        Ok(())
    }

    fn GetSite(
        &self,
        riid: *const GUID,
        ppvsite: *mut *mut std::ffi::c_void,
    ) -> windows::core::Result<()> {
        if riid.is_null() || ppvsite.is_null() {
            return Err(e_pointer());
        }
        if let Some(site) = self.this.site.borrow().as_ref() {
            // SAFETY: COM supplied `riid`/`ppvsite`.
            let hr = unsafe { site.query(riid, ppvsite) };
            if hr.is_ok() {
                Ok(())
            } else {
                Err(windows::core::Error::from_hresult(hr))
            }
        } else {
            Err(e_fail())
        }
    }
}

impl IInitializeWithStream_Impl for PreviewHandler_Impl {
    fn Initialize(&self, pstream: Option<&IStream>, _grfmode: u32) -> windows::core::Result<()> {
        let stream = pstream.ok_or_else(e_pointer)?;
        self.this
            .source
            .borrow_mut()
            .initialize_stream(stream.clone());
        self.this.preview_scene.borrow_mut().take();
        self.this.oversize_stream_len.set(None);
        Ok(())
    }
}

impl IInitializeWithFile_Impl for PreviewHandler_Impl {
    fn Initialize(&self, pszfilepath: &PCWSTR, _grfmode: u32) -> windows::core::Result<()> {
        let path_string = unsafe { pszfilepath.to_string() }.map_err(|_| e_fail())?;
        self.this.initialize_path(PathBuf::from(path_string));
        Ok(())
    }
}

impl IInitializeWithItem_Impl for PreviewHandler_Impl {
    fn Initialize(&self, psi: Option<&IShellItem>, _grfmode: u32) -> windows::core::Result<()> {
        let item = psi.ok_or_else(e_pointer)?;
        // SAFETY: `GetDisplayName(SIGDN_FILESYSPATH)` returns a CoTaskMem path.
        let path_ptr = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
        let path_string = unsafe { path_ptr.to_string() }.map_err(|_| {
            // SAFETY: freeing the COM-owned pointer returned by GetDisplayName.
            unsafe { CoTaskMemFree(Some(path_ptr.as_ptr().cast())) };
            e_fail()
        })?;
        // SAFETY: freeing the COM-owned pointer returned by GetDisplayName.
        unsafe { CoTaskMemFree(Some(path_ptr.as_ptr().cast())) };
        self.this.initialize_path(PathBuf::from(path_string));
        Ok(())
    }
}

fn shell_error_to_hresult(error: ShellError) -> windows::core::Error {
    windows::core::Error::new(HRESULT(0x8000_4005_u32 as i32), format!("{error}"))
}
