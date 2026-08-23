use super::super::{
    com_entry, e_fail, own_pinned_dll_module, DefWindowProcW, RegisterClassW, ReleaseCapture,
    SetCapture, SetKeyboardFocus, CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA,
    HINSTANCE, HWND, LPARAM, LRESULT, POINT, PREVIEW_WINDOW_CLASS, PREVIEW_WINDOW_CLASS_NAME,
    WM_CANCELMODE, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SIZE, WNDCLASSW, WPARAM,
};

/// Virtual-key code for the `F` (fit view) shortcut.
const VK_F: u32 = 0x46;
/// Virtual-key code for the `W` (wireframe toggle) shortcut.
const VK_W: u32 = 0x57;
use super::{PreviewDragMode, PreviewHandler};

pub(super) fn ensure_preview_window_class() -> windows::core::Result<()> {
    let init = PREVIEW_WINDOW_CLASS.get_or_init(|| {
        let module = own_pinned_dll_module().map_err(|_| e_fail())?;
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(preview_window_proc),
            hInstance: HINSTANCE(module.0),
            lpszClassName: PREVIEW_WINDOW_CLASS_NAME,
            ..Default::default()
        };
        // SAFETY: the class structure is fully initialized for registration.
        let atom = unsafe { RegisterClassW(&class) };
        if atom == 0 {
            return Err(e_fail().code());
        }
        Ok(())
    });
    init.map_err(windows::core::Error::from_hresult)
}

/// The preview child wndproc. Every pointer, paint, and resize interaction
/// funnels through here, so this is a COM-ABI-equivalent boundary: a panic
/// unwinding out of this `extern "system"` fn aborts the whole `prevhost`
/// surrogate. `com_entry` converts a panic into a no-op `LRESULT(0)` — a
/// degraded frame beats taking down every preview the host is serving.
unsafe extern "system" fn preview_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    com_entry(
        "preview_window_proc",
        || LRESULT(0),
        // SAFETY: the closure runs on the same thread inside the same call;
        // pointer parameters keep their validity contract.
        || unsafe { preview_window_proc_body(hwnd, message, wparam, lparam) },
    )
}

#[allow(clippy::too_many_lines)]
unsafe fn preview_window_proc_body(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            let create = lparam.0 as *const CREATESTRUCTW;
            if create.is_null() {
                return LRESULT(0);
            }
            let handler = unsafe { (*create).lpCreateParams };
            if handler.is_null() {
                return LRESULT(0);
            }
            // SAFETY: the create param is a stable pointer to PreviewHandler.
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
                    hwnd,
                    GWLP_USERDATA,
                    handler as isize,
                )
            };
            return LRESULT(1);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            if let Some(handler) = preview_handler_from_hwnd(hwnd) {
                handler.paint_preview(hwnd);
                return LRESULT(0);
            }
        }
        WM_RBUTTONDOWN => {
            if let Some(handler) = preview_handler_from_hwnd(hwnd) {
                let point = point_from_lparam(lparam);
                handler.begin_drag(PreviewDragMode::Orbit, point);
                let _ = unsafe { SetKeyboardFocus(hwnd) };
                unsafe { SetCapture(hwnd) };
                return LRESULT(0);
            }
        }
        WM_MBUTTONDOWN => {
            if let Some(handler) = preview_handler_from_hwnd(hwnd) {
                let point = point_from_lparam(lparam);
                handler.begin_drag(PreviewDragMode::Pan, point);
                let _ = unsafe { SetKeyboardFocus(hwnd) };
                unsafe { SetCapture(hwnd) };
                return LRESULT(0);
            }
        }
        WM_MOUSEMOVE => {
            if let Some(handler) = preview_handler_from_hwnd(hwnd) {
                if handler.drag_mode.get() != PreviewDragMode::None {
                    let _ = handler.update_drag(point_from_lparam(lparam));
                    return LRESULT(0);
                }
            }
        }
        WM_RBUTTONUP => {
            if let Some(handler) = preview_handler_from_hwnd(hwnd) {
                let point = point_from_lparam(lparam);
                // A right-*drag* was an orbit; only a stationary right-click
                // opens the context menu, so RMB orbit semantics are preserved.
                let dragged = handler.drag_moved.get();
                handler.end_drag();
                let _ = unsafe { ReleaseCapture() };
                if !dragged {
                    handler.show_context_menu(hwnd, point);
                }
                return LRESULT(0);
            }
        }
        WM_KEYDOWN => {
            if let Some(handler) = preview_handler_from_hwnd(hwnd) {
                match wparam.0 as u32 {
                    VK_F => {
                        handler.key_fit_view(hwnd);
                        return LRESULT(0);
                    }
                    VK_W => {
                        handler.key_toggle_wireframe(hwnd);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }
        }
        WM_MBUTTONUP => {
            if let Some(handler) = preview_handler_from_hwnd(hwnd) {
                let point = point_from_lparam(lparam);
                let dragged = handler.drag_moved.get();
                handler.end_drag();
                let _ = unsafe { ReleaseCapture() };
                if !dragged {
                    let _ = handler.focus_preview_point(point);
                }
                return LRESULT(0);
            }
        }
        WM_LBUTTONDBLCLK => {
            if let Some(handler) = preview_handler_from_hwnd(hwnd) {
                let _ = handler.focus_preview_point(point_from_lparam(lparam));
                return LRESULT(0);
            }
        }
        WM_MOUSEWHEEL => {
            if let Some(handler) = preview_handler_from_hwnd(hwnd) {
                let delta = wheel_delta_from_wparam(wparam);
                let _ = handler.zoom_preview(f32::from(delta));
                return LRESULT(0);
            }
        }
        WM_SIZE => {
            if let Some(handler) = preview_handler_from_hwnd(hwnd) {
                if handler.preview_bitmap.borrow().is_some() {
                    let _ = handler.render_preview_now();
                }
                return LRESULT(0);
            }
        }
        WM_CANCELMODE => {
            if let Some(handler) = preview_handler_from_hwnd(hwnd) {
                handler.end_drag();
                handler.drag_moved.set(false);
                let _ = unsafe { ReleaseCapture() };
                return LRESULT(0);
            }
        }
        WM_NCDESTROY => {
            // The other direction: Explorer destroyed the parent, so this child
            // died with it and the handler is still holding its HWND. Left
            // stale, a later `Unload` would call `DestroyWindow` on a handle
            // the system has since reused. Clear both ends here -- the
            // handler's cell, and the raw pointer this window holds -- so
            // neither side can reach the other again.
            if let Some(handler) = preview_handler_from_hwnd(hwnd) {
                if handler.preview_hwnd.get() == hwnd {
                    handler.preview_hwnd.set(HWND::default());
                }
            }
            // SAFETY: clearing the slot this wndproc set at WM_NCCREATE.
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0)
            };
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn preview_handler_from_hwnd(hwnd: HWND) -> Option<&'static PreviewHandler> {
    // SAFETY: GWLP_USERDATA stores the raw PreviewHandler pointer set at WM_NCCREATE time.
    let ptr = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, GWLP_USERDATA)
            as *const PreviewHandler
    };
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { &*ptr })
    }
}

#[allow(clippy::cast_sign_loss)]
fn point_from_lparam(lparam: LPARAM) -> POINT {
    let bits = lparam.0 as u32;
    POINT {
        x: i32::from((bits & 0xFFFF) as i16),
        y: i32::from(((bits >> 16) & 0xFFFF) as i16),
    }
}

fn wheel_delta_from_wparam(wparam: WPARAM) -> i16 {
    let bits = wparam.0 as u32;
    ((bits >> 16) & 0xFFFF) as i16
}
