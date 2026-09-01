use super::{
    center_square_on_canvas, com_entry, e_fail, e_notimpl, e_pointer, implement,
    own_pinned_dll_module, path_extension, pixels_to_hbitmap, placeholder_for_oversize_input,
    s_false, w, win32_preview_orbit_delta, BeginPaint, BitBlt, CoTaskMemFree, CreateCompatibleDC,
    CreateWindowExW, DeferredSource, DeleteDC, DeleteObject, DestroyWindow, EndPaint,
    GetKeyboardFocus, IClassFactory, IInitializeWithFile, IInitializeWithFile_Impl,
    IInitializeWithItem, IInitializeWithItem_Impl, IInitializeWithStream,
    IInitializeWithStream_Impl, IObjectWithSite, IObjectWithSite_Impl, IOleWindow, IOleWindow_Impl,
    IPreviewHandler, IPreviewHandlerFrame, IPreviewHandler_Impl, IShellItem, IStream, IUnknown,
    Interface, InvalidateRect, MoveWindow, Ordering, PathBuf, PreviewSceneState, Ref,
    RenderDeadline, SelectObject, SetKeyboardFocus, SetParent, ShellError, StreamRead,
    ThumbnailProvider, ThumbnailSpec, Vec2, ACTIVE_COM_OBJECTS, BOOL, GUID, GWLP_USERDATA, HBITMAP,
    HGDIOBJ, HINSTANCE, HRESULT, HWND, MAX_OFFSCREEN_EDGE, MSG, PAINTSTRUCT, PCWSTR, POINT,
    PREVIEW_WINDOW_CLASS_NAME, RECT, SIGDN_FILESYSPATH, SRCCOPY, S_OK, WINDOW_EX_STYLE, WS_CHILD,
    WS_CLIPSIBLINGS, WS_VISIBLE,
};
#[cfg(feature = "diagnostic-logs")]
use occluview_render::AdapterResult;
use std::time::Duration;

mod context_menu;
mod theme;
mod window;

use theme::preview_theme;
use window::ensure_preview_window_class;

#[cfg(feature = "diagnostic-logs")]
use crate::shell_diagnostics::{
    elapsed_ms_since, prepare_shell_diagnostics, record_shell_error, record_shell_event,
    record_shell_failure, ShellDiagnosticAdapter, ShellDiagnosticComponent,
    ShellDiagnosticErrorClass, ShellDiagnosticOutcome, ShellDiagnosticStage,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PreviewDragMode {
    #[default]
    None,
    Orbit,
    Pan,
}

const PREVIEW_FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(8);
const PREVIEW_INTERACTION_FRAME_TIMEOUT: Duration = Duration::from_secs(8);

#[cfg(feature = "diagnostic-logs")]
const fn diagnostic_adapter(adapter: AdapterResult) -> ShellDiagnosticAdapter {
    match adapter {
        AdapterResult::Hardware => ShellDiagnosticAdapter::Hardware,
        AdapterResult::Fallback => ShellDiagnosticAdapter::Fallback,
    }
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

    fn preview_render_to_hbitmap(
        &self,
        width: u32,
        height: u32,
        deadline: RenderDeadline,
        #[cfg(feature = "diagnostic-logs")] started: std::time::Instant,
    ) -> windows::core::Result<HBITMAP> {
        let width = width.clamp(1, MAX_OFFSCREEN_EDGE);
        let height = height.clamp(1, MAX_OFFSCREEN_EDGE);
        let theme = preview_theme();
        let pixels = match self.render_preview_pixels(
            [width as u16, height as u16],
            theme.background_linear(),
            theme.canvas_rgba(),
            deadline,
        ) {
            Ok(pixels) => pixels,
            Err(error) => {
                #[cfg(feature = "diagnostic-logs")]
                record_shell_error(
                    &error,
                    ShellDiagnosticComponent::Preview,
                    ShellDiagnosticStage::Render,
                    ShellDiagnosticAdapter::NotObserved,
                    elapsed_ms_since(started),
                );
                #[cfg(not(feature = "diagnostic-logs"))]
                let _ = error;
                tracing::warn!("preview render failed; returning placeholder");
                // The resident scene may reference a retired device (the
                // renderer discards itself on render errors). Drop it so the
                // next paint reloads against a fresh renderer instead of
                // replaying the same failure for the rest of this file view.
                self.preview_scene.borrow_mut().take();
                let preview_edge_px = width.min(height).clamp(1, MAX_OFFSCREEN_EDGE) as u16;
                let spec = ThumbnailSpec {
                    size_px: preview_edge_px,
                    background: [0.0, 0.0, 0.0, 0.0],
                };
                let square = if let Some(byte_len) = self.oversize_stream_len.get() {
                    placeholder_for_oversize_input(spec, byte_len)
                } else {
                    occluview_thumbnail::placeholder::placeholder_thumbnail(spec)
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
        match pixels_to_hbitmap(&pixels, width, height) {
            Ok(bitmap) => Ok(bitmap),
            Err(error) => {
                #[cfg(feature = "diagnostic-logs")]
                record_shell_failure(
                    ShellDiagnosticComponent::Preview,
                    ShellDiagnosticStage::BitmapPublish,
                    ShellDiagnosticAdapter::NotObserved,
                    ShellDiagnosticErrorClass::Windows,
                    elapsed_ms_since(started),
                );
                Err(error)
            }
        }
    }

    fn render_preview_pixels(
        &self,
        size_px: [u16; 2],
        background_linear: [f64; 4],
        canvas_rgba: [u8; 4],
        deadline: RenderDeadline,
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

        self.ensure_preview_scene_loaded(deadline)?;
        let preview = self.preview_scene.borrow();
        let state = preview
            .as_ref()
            .ok_or_else(|| ShellError::Win32("preview scene unavailable".to_string()))?;
        #[cfg(feature = "diagnostic-logs")]
        let adapter = diagnostic_adapter(state.adapter_result());
        #[cfg(feature = "diagnostic-logs")]
        let render_started = std::time::Instant::now();
        #[cfg(feature = "diagnostic-logs")]
        record_shell_event(
            ShellDiagnosticComponent::Preview,
            ShellDiagnosticStage::Adapter,
            ShellDiagnosticOutcome::Completed,
            adapter,
            0,
        );
        let pixels = state.render_rgba_with_background_with_deadline(
            size_px,
            background_linear,
            deadline,
        )?;
        #[cfg(feature = "diagnostic-logs")]
        record_shell_event(
            ShellDiagnosticComponent::Preview,
            ShellDiagnosticStage::Render,
            ShellDiagnosticOutcome::Completed,
            adapter,
            elapsed_ms_since(render_started),
        );
        Ok(pixels)
    }

    fn ensure_preview_scene_loaded(&self, deadline: RenderDeadline) -> Result<(), ShellError> {
        if self.preview_scene.borrow().is_some() || self.oversize_stream_len.get().is_some() {
            return Ok(());
        }

        #[cfg(feature = "diagnostic-logs")]
        let load_started = std::time::Instant::now();
        #[cfg(feature = "diagnostic-logs")]
        record_shell_event(
            ShellDiagnosticComponent::Preview,
            ShellDiagnosticStage::Source,
            ShellDiagnosticOutcome::Started,
            ShellDiagnosticAdapter::NotObserved,
            0,
        );
        #[cfg(feature = "diagnostic-logs")]
        record_shell_event(
            ShellDiagnosticComponent::Preview,
            ShellDiagnosticStage::SceneLoad,
            ShellDiagnosticOutcome::Started,
            ShellDiagnosticAdapter::NotObserved,
            0,
        );

        let source_path = self.source.borrow().path().map(PathBuf::from);
        let state = if let Some(path) = source_path {
            PreviewSceneState::from_file_with_deadline(&path, deadline)?
        } else if let Some(stream_result) =
            self.source
                .borrow_mut()
                .consume_pending_stream(|stream, extension| {
                    ThumbnailProvider::rewind_stream(&stream).map_err(|_| {
                        ShellError::Win32("rewinding preview stream failed".to_string())
                    })?;
                    let read = ThumbnailProvider::read_stream_until(&stream, deadline.expires_at())
                        .map_err(|_| {
                            ShellError::Win32("reading preview stream failed".to_string())
                        })?;
                    Ok::<_, ShellError>((read, extension.map(str::to_owned)))
                })
        {
            match stream_result? {
                (StreamRead::Complete(bytes), extension) => {
                    PreviewSceneState::from_bytes_with_deadline(
                        extension.as_deref(),
                        &bytes,
                        deadline,
                    )?
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
                (StreamRead::TimedOut, _extension) => {
                    return Err(deadline
                        .remaining()
                        .map(|_| ShellError::Win32("preview stream deadline elapsed".to_string()))
                        .unwrap_or_else(ShellError::from));
                }
            }
        } else {
            return Err(ShellError::Win32(
                "preview handler has no file or stream source".to_string(),
            ));
        };
        *self.preview_scene.borrow_mut() = Some(state);
        #[cfg(feature = "diagnostic-logs")]
        record_shell_event(
            ShellDiagnosticComponent::Preview,
            ShellDiagnosticStage::Source,
            ShellDiagnosticOutcome::Completed,
            ShellDiagnosticAdapter::NotObserved,
            elapsed_ms_since(load_started),
        );
        #[cfg(feature = "diagnostic-logs")]
        record_shell_event(
            ShellDiagnosticComponent::Preview,
            ShellDiagnosticStage::SceneLoad,
            ShellDiagnosticOutcome::Completed,
            ShellDiagnosticAdapter::NotObserved,
            elapsed_ms_since(load_started),
        );
        Ok(())
    }

    /// Render and publish the first bitmap required before `DoPreview` returns.
    fn render_first_preview_frame(
        &self,
        hwnd: HWND,
        deadline: RenderDeadline,
    ) -> windows::core::Result<()> {
        self.refresh_preview_bitmap(hwnd, deadline)
    }

    /// Render one updated scene state and publish its bitmap to the child.
    fn refresh_preview_bitmap(
        &self,
        hwnd: HWND,
        deadline: RenderDeadline,
    ) -> windows::core::Result<()> {
        #[cfg(feature = "diagnostic-logs")]
        let started = std::time::Instant::now();
        #[cfg(feature = "diagnostic-logs")]
        prepare_shell_diagnostics();
        #[cfg(feature = "diagnostic-logs")]
        record_shell_event(
            ShellDiagnosticComponent::Preview,
            ShellDiagnosticStage::Render,
            ShellDiagnosticOutcome::Started,
            ShellDiagnosticAdapter::NotObserved,
            0,
        );
        let (width, height) = self.preview_size();
        let hbmp = self.preview_render_to_hbitmap(
            width,
            height,
            deadline,
            #[cfg(feature = "diagnostic-logs")]
            started,
        )?;
        self.replace_preview_bitmap(hbmp);
        #[cfg(feature = "diagnostic-logs")]
        record_shell_event(
            ShellDiagnosticComponent::Preview,
            ShellDiagnosticStage::BitmapPublish,
            ShellDiagnosticOutcome::Completed,
            ShellDiagnosticAdapter::NotObserved,
            elapsed_ms_since(started),
        );
        // SAFETY: `hwnd` is the child window owned by this handler. The bitmap
        // is already installed, so normal painting cannot show a spinner.
        if unsafe { InvalidateRect(Some(hwnd), None, false) }.0 == 0 {
            tracing::warn!("could not invalidate preview frame");
        }
        Ok(())
    }

    /// Compatibility choke point for interactive scene mutations.
    fn render_preview_now(&self, deadline: RenderDeadline) -> windows::core::Result<()> {
        let hwnd = self.preview_hwnd.get();
        if hwnd.0.is_null() {
            return Err(e_fail());
        }
        self.refresh_preview_bitmap(hwnd, deadline)
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
        // The child window must carry the DLL's module identity, matching the
        // class registration (see `own_pinned_dll_module`).
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
            // Cut the window's link to this object BEFORE destroying it, and do
            // it unconditionally. The window holds a raw `&PreviewHandler` in
            // GWLP_USERDATA, and `DestroyWindow` only works from the thread
            // that created the window -- so when it does not, the window
            // survives with a pointer to memory that is about to be freed.
            // Clearing the slot is legal cross-thread and turns that window
            // into a plain `DefWindowProcW` shell instead of a use-after-free.
            //
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
        let deadline = RenderDeadline::after(PREVIEW_INTERACTION_FRAME_TIMEOUT);
        self.ensure_preview_scene_loaded(deadline)
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
            self.render_preview_now(deadline)?;
        }
        Ok(())
    }

    fn end_drag(&self) {
        self.drag_mode.set(PreviewDragMode::None);
    }

    fn zoom_preview(&self, scroll_y: f32) -> windows::core::Result<()> {
        let deadline = RenderDeadline::after(PREVIEW_INTERACTION_FRAME_TIMEOUT);
        self.ensure_preview_scene_loaded(deadline)
            .map_err(shell_error_to_hresult)?;
        let changed = {
            let mut preview = self.preview_scene.borrow_mut();
            preview
                .as_mut()
                .is_some_and(|state| state.zoom_scroll(scroll_y))
        };
        if changed {
            self.render_preview_now(deadline)?;
        }
        Ok(())
    }

    fn focus_preview_point(&self, pointer: POINT) -> windows::core::Result<()> {
        let deadline = RenderDeadline::after(PREVIEW_INTERACTION_FRAME_TIMEOUT);
        self.ensure_preview_scene_loaded(deadline)
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
            self.render_preview_now(deadline)?;
        }
        Ok(())
    }

    fn paint_preview(&self, hwnd: HWND) {
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: `hwnd` is our preview child window and `paint` is valid.
        let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
        if let Some(bitmap) = *self.preview_bitmap.borrow() {
            // SAFETY: `hdc` is valid for the active paint cycle.
            let memory_dc = unsafe { CreateCompatibleDC(Some(hdc)) };
            if !memory_dc.0.is_null() {
                // SAFETY: the bitmap handle is owned by this module.
                let previous = unsafe { SelectObject(memory_dc, HGDIOBJ(bitmap.0)) };
                // The DIB was rendered at the pane size clamped to
                // MAX_OFFSCREEN_EDGE; blitting the raw pane size on a >2048 px
                // pane would read past the bitmap and leave garbage rows. Blit
                // exactly the bitmap extent — the window class background
                // covers any remainder on those extreme panes.
                let (width, height) = self.preview_size();
                let blit_width = width.min(MAX_OFFSCREEN_EDGE);
                let blit_height = height.min(MAX_OFFSCREEN_EDGE);
                // SAFETY: both DCs are valid for this paint cycle.
                let _ = unsafe {
                    BitBlt(
                        hdc,
                        0,
                        0,
                        blit_width as i32,
                        blit_height as i32,
                        Some(memory_dc),
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
        // `Unload` is host etiquette, not a COM requirement, and the two paths
        // that skip it are ordinary: a host that releases after `DoPreview`
        // returned an error, and re-entrancy with no misbehaving host at all --
        // `show_context_menu` runs `TrackPopupMenuEx`, a modal loop that pumps
        // the STA, so a click on another file in Explorer can deliver Unload
        // and Release while that call is still on the stack.
        //
        // Whatever the route, this object must not be freed while a live window
        // still points at it, and the last rendered bitmap -- up to 2048x2048x4
        // of GDI memory -- must not be left behind.
        self.destroy_preview_window();
        ACTIVE_COM_OBJECTS.fetch_sub(1, Ordering::AcqRel);
    }
}

impl IPreviewHandler_Impl for PreviewHandler_Impl {
    fn SetWindow(&self, hwnd: HWND, prc: *const RECT) -> windows::core::Result<()> {
        com_entry(
            "IPreviewHandler::SetWindow",
            || Err(e_fail()),
            || {
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
                    let _ = unsafe { SetParent(preview, Some(hwnd))? };
                }
                // SAFETY: `prc` is a caller-owned RECT pointer valid for this call.
                *self.this.rect.borrow_mut() = unsafe { *prc };
                // One render per resize: `MoveWindow` synchronously delivers
                // `WM_SIZE` when the size actually changed, and that handler
                // re-renders. Adding a second explicit render here made every host
                // resize pay two full GPU renders + readbacks back to back.
                self.this.resize_preview_window()?;
                Ok(())
            },
        )
    }

    fn SetRect(&self, prc: *const RECT) -> windows::core::Result<()> {
        com_entry(
            "IPreviewHandler::SetRect",
            || Err(e_fail()),
            || {
                if prc.is_null() {
                    return Err(e_pointer());
                }
                // SAFETY: `prc` is a caller-owned RECT pointer valid for this call.
                *self.this.rect.borrow_mut() = unsafe { *prc };
                // See SetWindow: the WM_SIZE handler owns the re-render, so a
                // resize renders once, and a pure move (same size, no WM_SIZE)
                // keeps the already-correct bitmap without any render at all.
                self.this.resize_preview_window()?;
                Ok(())
            },
        )
    }

    fn DoPreview(&self) -> windows::core::Result<()> {
        com_entry(
            "IPreviewHandler::DoPreview",
            || Err(e_fail()),
            || {
                #[cfg(feature = "diagnostic-logs")]
                let started = std::time::Instant::now();
                #[cfg(feature = "diagnostic-logs")]
                prepare_shell_diagnostics();
                #[cfg(feature = "diagnostic-logs")]
                record_shell_event(
                    ShellDiagnosticComponent::Preview,
                    ShellDiagnosticStage::Activation,
                    ShellDiagnosticOutcome::Started,
                    ShellDiagnosticAdapter::NotObserved,
                    0,
                );
                let result = self.this.ensure_preview_window().and_then(|hwnd| {
                    self.this.render_first_preview_frame(
                        hwnd,
                        RenderDeadline::after(PREVIEW_FIRST_FRAME_TIMEOUT),
                    )
                });
                #[cfg(feature = "diagnostic-logs")]
                if result.is_ok() {
                    record_shell_event(
                        ShellDiagnosticComponent::Preview,
                        ShellDiagnosticStage::ComReturn,
                        ShellDiagnosticOutcome::Completed,
                        ShellDiagnosticAdapter::NotObserved,
                        elapsed_ms_since(started),
                    );
                } else {
                    record_shell_failure(
                        ShellDiagnosticComponent::Preview,
                        ShellDiagnosticStage::ComReturn,
                        ShellDiagnosticAdapter::NotObserved,
                        ShellDiagnosticErrorClass::Windows,
                        elapsed_ms_since(started),
                    );
                }
                result
            },
        )
    }

    fn Unload(&self) -> windows::core::Result<()> {
        com_entry(
            "IPreviewHandler::Unload",
            || Err(e_fail()),
            || {
                self.this.destroy_preview_window();
                self.this.clear_loaded_content();
                Ok(())
            },
        )
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
        let _ = unsafe { SetKeyboardFocus(Some(target)) };
        Ok(())
    }

    fn QueryFocus(&self) -> windows::core::Result<HWND> {
        // SAFETY: Win32 returns the HWND with focus for the current thread.
        Ok(unsafe { GetKeyboardFocus() })
    }

    fn TranslateAccelerator(&self, pmsg: *const MSG) -> windows::core::Result<()> {
        if pmsg.is_null() {
            return Err(e_pointer());
        }
        // Low-integrity preview handlers must send unhandled accelerators back
        // to their host frame. The smoke harness has no site, in which case
        // S_FALSE correctly tells its caller the key was not consumed.
        let Some(site) = self.this.site.borrow().as_ref().cloned() else {
            return Err(s_false());
        };
        let frame = site.cast::<IPreviewHandlerFrame>().map_err(|_| s_false())?;
        // Preserve the host frame's exact HRESULT. The generated windows-rs
        // convenience method maps every non-negative result to `Ok(())`, which
        // would accidentally turn a host S_FALSE into S_OK at this COM boundary.
        // SAFETY: `frame` is a live IPreviewHandlerFrame and `pmsg` was checked
        // non-null; its lifetime is the caller's IPreviewHandler contract.
        let hresult = unsafe {
            (Interface::vtable(&frame).TranslateAccelerator)(Interface::as_raw(&frame), pmsg)
        };
        if hresult == S_OK {
            Ok(())
        } else {
            Err(windows::core::Error::from_hresult(hresult))
        }
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
    fn SetSite(&self, punksite: Ref<'_, IUnknown>) -> windows::core::Result<()> {
        com_entry(
            "IObjectWithSite::SetSite",
            || Err(e_fail()),
            || {
                *self.this.site.borrow_mut() = punksite.as_ref().cloned();
                Ok(())
            },
        )
    }

    fn GetSite(
        &self,
        riid: *const GUID,
        ppvsite: *mut *mut std::ffi::c_void,
    ) -> windows::core::Result<()> {
        com_entry(
            "IObjectWithSite::GetSite",
            || Err(e_fail()),
            || {
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
            },
        )
    }
}

impl IInitializeWithStream_Impl for PreviewHandler_Impl {
    fn Initialize(&self, pstream: Ref<'_, IStream>, _grfmode: u32) -> windows::core::Result<()> {
        com_entry(
            "preview IInitializeWithStream",
            || Err(e_fail()),
            || {
                let stream = pstream.ok()?;
                self.this
                    .source
                    .borrow_mut()
                    .initialize_stream(stream.clone());
                self.this.preview_scene.borrow_mut().take();
                self.this.oversize_stream_len.set(None);
                Ok(())
            },
        )
    }
}

impl IInitializeWithFile_Impl for PreviewHandler_Impl {
    fn Initialize(&self, pszfilepath: &PCWSTR, _grfmode: u32) -> windows::core::Result<()> {
        com_entry(
            "preview IInitializeWithFile",
            || Err(e_fail()),
            || {
                let path_string = unsafe { pszfilepath.to_string() }.map_err(|_| e_fail())?;
                self.this.initialize_path(PathBuf::from(path_string));
                Ok(())
            },
        )
    }
}

impl IInitializeWithItem_Impl for PreviewHandler_Impl {
    fn Initialize(&self, psi: Ref<'_, IShellItem>, _grfmode: u32) -> windows::core::Result<()> {
        com_entry(
            "preview IInitializeWithItem",
            || Err(e_fail()),
            || {
                let item = psi.ok()?;
                // SAFETY: `GetDisplayName(SIGDN_FILESYSPATH)` returns a CoTaskMem
                // path.
                let path_ptr = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
                let path_string = unsafe { path_ptr.to_string() }.map_err(|_| {
                    // SAFETY: freeing the COM-owned pointer returned by
                    // GetDisplayName.
                    unsafe { CoTaskMemFree(Some(path_ptr.as_ptr().cast())) };
                    e_fail()
                })?;
                // SAFETY: freeing the COM-owned pointer returned by GetDisplayName.
                unsafe { CoTaskMemFree(Some(path_ptr.as_ptr().cast())) };
                self.this.initialize_path(PathBuf::from(path_string));
                Ok(())
            },
        )
    }
}

fn shell_error_to_hresult(error: ShellError) -> windows::core::Error {
    windows::core::Error::new(HRESULT(0x8000_4005_u32 as i32), format!("{error}"))
}
