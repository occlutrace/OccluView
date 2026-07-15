//! Raising the running instance past desktop focus-stealing prevention.
//!
//! # Why the naive "focus the window" call is denied on Linux
//!
//! When a second instance hands a freshly opened file to the already-running
//! viewer, we want the viewer's window to come to the front. Modern Linux
//! desktops refuse a self-initiated raise unless the request carries proof of a
//! recent *user* interaction. Both display servers deny our best effort through
//! eframe 0.29 / winit 0.30:
//!
//! * **X11.** `egui::ViewportCommand::Focus` maps to winit's
//!   `Window::focus_window`, which sends `_NET_ACTIVE_WINDOW` with source
//!   indication `1` (application) and timestamp `CURRENT_TIME` (0). Mutter
//!   (GNOME) and `KWin` (KDE) treat an application-sourced request with no valid
//!   timestamp as a focus steal and, instead of raising, mark the window as
//!   "demanding attention" — the "window is ready" notification the user sees.
//!   winit exposes no way to inject a valid timestamp or startup-notification id
//!   into a raise of an *already-created* window.
//!
//! * **Wayland.** winit's Wayland `focus_window` is a no-op
//!   (`winit-0.30.13/src/platform_impl/linux/wayland/window/mod.rs:629`). The
//!   xdg-activation protocol *can* raise a window, but winit only applies an
//!   activation token at window *creation*
//!   (`WindowAttributesExtStartupNotify::with_activation_token`); there is no
//!   public API to re-activate a live window with a forwarded token, and eframe
//!   never surfaces the winit `Window`, the `wl_display`, or the `xdg_activation`
//!   global. So even though the second instance owns the activation token, there
//!   is no reachable sink for it. Full raise on Wayland is therefore impossible
//!   through this stack; the caller falls back to `RequestUserAttention`, which
//!   winit implements via xdg-activation urgency (the taskbar entry highlights).
//!
//! # What this module does
//!
//! On X11 we bypass winit and send `_NET_ACTIVE_WINDOW` ourselves with valid
//! provenance, using the window's XID (taken from the raw window handle eframe
//! *does* expose):
//!
//! * If the forwarded startup id carries an X server timestamp (the
//!   `..._TIME<n>` suffix of a `DESKTOP_STARTUP_ID`), send source `1` with that
//!   timestamp — a legitimate, recent user-interaction time Mutter honors.
//! * Otherwise send source `2` (pager). Mutter and `KWin` honor pager-sourced
//!   activation unconditionally, which is the mechanism panels/taskbars use.
//!
//! Either way the request goes to the root window on our own X connection, so it
//! is independent of winit's event loop. If anything fails we return `false` and
//! the caller uses the winit/eframe fallback (no worse than before).

use raw_window_handle::HasWindowHandle;

/// A handle to the running instance's top-level window, captured once at
/// startup, that knows how to raise itself when a file is handed off.
#[derive(Clone, Copy, Default)]
pub(crate) struct RaiseTarget {
    /// The X11 window id, when the session is X11 and the handle was readable.
    /// `None` on Wayland (raising a live window is WM-denied there) and on
    /// non-Linux platforms.
    #[cfg(target_os = "linux")]
    x11_window: Option<u32>,
}

impl RaiseTarget {
    /// Capture the raise target from eframe's creation context (or any window
    /// handle provider). Reads the raw window handle; on X11 it keeps the XID,
    /// otherwise it degrades to the fallback path.
    #[cfg(target_os = "linux")]
    pub(crate) fn from_window_handle<H: HasWindowHandle>(handle: &H) -> Self {
        use raw_window_handle::RawWindowHandle;

        let x11_window = handle
            .window_handle()
            .ok()
            .and_then(|handle| match handle.as_raw() {
                // An X11 window id is a 32-bit XID even though Xlib widens it to
                // `c_ulong`; `try_from` keeps that explicit and arch-portable.
                RawWindowHandle::Xlib(window) => u32::try_from(window.window).ok(),
                RawWindowHandle::Xcb(window) => Some(window.window.get()),
                _ => None,
            });
        if x11_window.is_none() {
            tracing::debug!(
                "no X11 window handle for open-handoff activation; \
                 relying on winit focus fallback (expected on Wayland)"
            );
        }
        Self { x11_window }
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) fn from_window_handle<H: HasWindowHandle>(_handle: &H) -> Self {
        Self::default()
    }

    /// Try to raise the window using the forwarded activation token as
    /// provenance. Returns `true` when a real activation request was issued and
    /// the caller can skip the WM-denied focus fallback; `false` means the
    /// caller must fall back (Wayland, missing handle, or a failed request).
    #[cfg(target_os = "linux")]
    pub(crate) fn try_activate(self, token: Option<&str>) -> bool {
        let Some(window) = self.x11_window else {
            return false;
        };
        match x11_activate(window, token) {
            Ok(()) => {
                tracing::debug!(window, "issued X11 _NET_ACTIVE_WINDOW for open handoff");
                true
            }
            Err(error) => {
                tracing::warn!(?error, window, "X11 open-handoff activation failed");
                false
            }
        }
    }

    // Same signature as the Linux path so call sites stay platform-free; the
    // receiver is intentionally unused off Linux.
    #[cfg(not(target_os = "linux"))]
    #[allow(clippy::unused_self)]
    pub(crate) fn try_activate(self, _token: Option<&str>) -> bool {
        false
    }
}

/// Capture the window-activation token from this process's environment. The
/// short-lived second instance inherits it from the launcher (file manager,
/// terminal, or `.desktop` activation), so it carries the user-interaction
/// provenance the running instance lacks. `XDG_ACTIVATION_TOKEN` is the Wayland
/// carrier; `DESKTOP_STARTUP_ID` is the X11 one (and encodes an X timestamp).
pub(crate) fn capture_activation_token() -> Option<String> {
    for name in ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"] {
        if let Some(value) = std::env::var_os(name) {
            if let Ok(text) = value.into_string() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

/// Extract the X server timestamp encoded in a `DESKTOP_STARTUP_ID`. The
/// startup-notification spec appends `_TIME<timestamp>` to the id; that value is
/// the X server time of the user action that launched us, which Mutter accepts
/// as legitimate provenance for a source-`1` activation.
#[cfg(target_os = "linux")]
fn startup_id_timestamp(token: &str) -> Option<u32> {
    let index = token.rfind("_TIME")?;
    let digits = &token[index + "_TIME".len()..];
    let digits: String = digits.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    // Timestamps wrap at 32 bits (X server time is a CARD32); a value that does
    // not fit is malformed, so ignore it and fall back to a pager request.
    digits.parse::<u32>().ok()
}

#[cfg(target_os = "linux")]
fn x11_activate(window: u32, token: Option<&str>) -> anyhow::Result<()> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt, EventMask};

    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn
        .setup()
        .roots
        .get(screen_num)
        .ok_or_else(|| anyhow::anyhow!("X11 screen {screen_num} is missing"))?
        .root;
    let net_active_window = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")?
        .reply()?
        .atom;

    // Prefer a real user-interaction timestamp forwarded from the launcher
    // (source 1); Mutter honors that. Without one, use a pager request
    // (source 2), which Mutter/KWin honor unconditionally.
    let (source_indication, timestamp) = match token.and_then(startup_id_timestamp) {
        Some(timestamp) => (1u32, timestamp),
        None => (2u32, 0u32),
    };

    let event = ClientMessageEvent::new(
        32,
        window,
        net_active_window,
        [source_indication, timestamp, 0, 0, 0],
    );
    conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        event,
    )?;
    conn.flush()?;
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn startup_id_timestamp_parses_trailing_time() {
        assert_eq!(
            startup_id_timestamp("occluview/host/42-7-host_TIME1234567"),
            Some(1_234_567)
        );
    }

    #[test]
    fn startup_id_timestamp_stops_at_non_digits() {
        assert_eq!(
            startup_id_timestamp("prefix_TIME99trailer"),
            Some(99),
            "only the leading run of digits after _TIME is the timestamp"
        );
    }

    #[test]
    fn startup_id_timestamp_uses_last_time_marker() {
        // Only the final _TIME marker is the real timestamp per the spec.
        assert_eq!(startup_id_timestamp("_TIME5/thing_TIME8"), Some(8));
    }

    #[test]
    fn startup_id_timestamp_absent_or_malformed_is_none() {
        assert_eq!(startup_id_timestamp("no-time-here"), None);
        assert_eq!(startup_id_timestamp("ends_TIME"), None);
        assert_eq!(startup_id_timestamp("ends_TIMExyz"), None);
    }

    #[test]
    fn raise_target_without_handle_does_not_activate() {
        // The default target has no X11 window, so it must defer to the caller's
        // fallback rather than claim it raised the window.
        let target = RaiseTarget::default();
        assert!(!target.try_activate(Some("occluview_TIME1")));
        assert!(!target.try_activate(None));
    }
}
