//! Raise the viewer through the compositor's activation protocol.
//!
//! A plain `ViewportCommand::Focus` is deliberately insufficient for an
//! already-running Linux window: X11 window managers require a valid
//! `_NET_ACTIVE_WINDOW` request, while Wayland requires the launcher's
//! `xdg-activation` token. eframe 0.29 exposes raw handles but not winit's
//! `Window`, so this module uses the raw X11/Wayland surface only for the
//! platform-specific activation request. File delivery remains in the
//! single-instance IPC module.

use anyhow::Context;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

#[cfg(target_os = "linux")]
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

#[cfg(target_os = "linux")]
use std::{ffi::c_void, ptr::NonNull};

/// A handle to the running instance's top-level window, captured once at
/// startup, that knows how to raise itself when a file is handed off.
#[derive(Default)]
pub(crate) struct RaiseTarget {
    /// The X11 window id, when the session is X11 and the handle was readable.
    /// `None` when the raw X11 handle is unavailable and on non-Linux
    /// platforms.
    #[cfg(target_os = "linux")]
    x11_window: Option<u32>,
    /// The existing Wayland surface plus the compositor's activation object.
    /// It is initialized from eframe's raw handles while the window is live.
    #[cfg(target_os = "linux")]
    wayland: Option<WaylandActivator>,
}

impl RaiseTarget {
    /// Capture the raise target from eframe's creation context (or any pair of
    /// window/display handle providers).
    #[cfg(target_os = "linux")]
    pub(crate) fn from_handles<W, D>(window_handle: &W, display_handle: &D) -> Self
    where
        W: HasWindowHandle,
        D: HasDisplayHandle,
    {
        let raw_window = window_handle
            .window_handle()
            .ok()
            .map(|handle| handle.as_raw());
        let raw_display = display_handle
            .display_handle()
            .ok()
            .map(|handle| handle.as_raw());
        let x11_window = raw_window.as_ref().and_then(|raw| match raw {
            // An X11 window id is a 32-bit XID even though Xlib widens it
            // to `c_ulong`; `try_from` keeps that explicit and portable.
            RawWindowHandle::Xlib(window) => u32::try_from(window.window).ok(),
            RawWindowHandle::Xcb(window) => Some(window.window.get()),
            _ => None,
        });
        let wayland = match (raw_window, raw_display) {
            (Some(RawWindowHandle::Wayland(window)), Some(RawDisplayHandle::Wayland(display))) => {
                match WaylandActivator::new(display.display, window.surface) {
                    Ok(activator) => Some(activator),
                    Err(error) => {
                        tracing::warn!(?error, "Wayland window activation is unavailable");
                        None
                    }
                }
            }
            _ => None,
        };

        if x11_window.is_none() && wayland.is_none() {
            tracing::debug!(
                "no compositor activation target for open handoff; using the window fallback"
            );
        }
        Self {
            x11_window,
            wayland,
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) fn from_handles<W: HasWindowHandle, D: HasDisplayHandle>(
        _window_handle: &W,
        _display_handle: &D,
    ) -> Self {
        Self::default()
    }

    /// Try to raise the window using the forwarded activation token as
    /// provenance. Returns `true` only after the platform request was sent.
    #[cfg(target_os = "linux")]
    pub(crate) fn try_activate(&self, token: Option<&str>) -> bool {
        if let Some(window) = self.x11_window {
            match x11_activate(window, token) {
                Ok(()) => {
                    tracing::debug!(window, "issued X11 _NET_ACTIVE_WINDOW for open handoff");
                    return true;
                }
                Err(error) => {
                    tracing::warn!(?error, window, "X11 open-handoff activation failed");
                }
            }
        }

        let Some(wayland) = self.wayland.as_ref() else {
            return false;
        };
        let Some(token) = token.filter(|token| !token.is_empty()) else {
            tracing::debug!("cannot activate Wayland surface without a launch token");
            return false;
        };
        match wayland.activate(token) {
            Ok(()) => {
                tracing::debug!("issued Wayland xdg-activation request");
                true
            }
            Err(error) => {
                tracing::warn!(?error, "Wayland open-handoff activation failed");
                false
            }
        }
    }

    // Same signature as the Linux path so call sites stay platform-free; the
    // receiver is intentionally unused off Linux.
    #[cfg(not(target_os = "linux"))]
    #[allow(clippy::unused_self)]
    pub(crate) fn try_activate(&self, _token: Option<&str>) -> bool {
        false
    }
}

/// Complete a launcher's startup-notification sequence when the launch came
/// through X11. Wayland uses the xdg-activation token itself; it has no
/// separate remove message.
pub(crate) fn complete_startup_notification(token: Option<&str>) {
    #[cfg(target_os = "linux")]
    if let Some(token) = token.filter(|token| startup_id_timestamp(token).is_some()) {
        if let Err(error) = x11_remove_startup_notification(token) {
            tracing::debug!(?error, "could not complete X11 startup notification");
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn complete_startup_notification(_token: Option<&str>) {}

#[cfg(target_os = "linux")]
struct WaylandActivator {
    connection: wayland_client::Connection,
    // Keep the queue and registry alive for the lifetime of the bound global.
    _event_queue: wayland_client::EventQueue<WaylandActivationState>,
    _globals: wayland_client::globals::GlobalList,
    activation: wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1,
    surface: wayland_client::protocol::wl_surface::WlSurface,
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct WaylandActivationState;

#[cfg(target_os = "linux")]
impl
    wayland_client::Dispatch<
        wayland_client::protocol::wl_registry::WlRegistry,
        wayland_client::globals::GlobalListContents,
    > for WaylandActivationState
{
    fn event(
        _state: &mut Self,
        _proxy: &wayland_client::protocol::wl_registry::WlRegistry,
        _event: wayland_client::protocol::wl_registry::Event,
        _data: &wayland_client::globals::GlobalListContents,
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

#[cfg(target_os = "linux")]
impl
    wayland_client::Dispatch<
        wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1,
        (),
    > for WaylandActivationState
{
    fn event(
        _state: &mut Self,
        _proxy: &wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1,
        _event: <wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

#[cfg(target_os = "linux")]
impl WaylandActivator {
    fn new(display: NonNull<c_void>, surface: NonNull<c_void>) -> anyhow::Result<Self> {
        use wayland_client::backend::{Backend, ObjectId};
        use wayland_client::globals::registry_queue_init;
        use wayland_client::protocol::wl_surface::WlSurface;
        use wayland_client::Proxy;
        use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

        // eframe/winit owns this connection. The system backend's foreign
        // display mode borrows it and does not close or replace it on drop.
        let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
        let connection = wayland_client::Connection::from_backend(backend);
        let (globals, event_queue) = registry_queue_init::<WaylandActivationState>(&connection)
            .context("enumerating Wayland globals for window activation")?;
        let queue_handle = event_queue.handle();
        let activation = globals
            .bind::<XdgActivationV1, _, _>(&queue_handle, 1..=1, ())
            .context("binding xdg_activation_v1")?;

        let surface_id =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.as_ptr().cast()) }
                .context("importing the eframe Wayland surface")?;
        let surface = WlSurface::from_id(&connection, surface_id)
            .context("creating a Wayland surface proxy")?;

        Ok(Self {
            connection,
            _event_queue: event_queue,
            _globals: globals,
            activation,
            surface,
        })
    }

    fn activate(&self, token: &str) -> anyhow::Result<()> {
        self.activation.activate(token.to_owned(), &self.surface);
        self.connection
            .flush()
            .context("flushing Wayland xdg-activation request")?;
        Ok(())
    }
}

/// Capture the window-activation token from this process's environment. The
/// short-lived second instance inherits it from the launcher (file manager,
/// terminal, or `.desktop` activation), so it carries the user-interaction
/// provenance the running instance lacks. `XDG_ACTIVATION_TOKEN` is the Wayland
/// carrier; `DESKTOP_STARTUP_ID` is the X11 one (and encodes an X timestamp).
pub(crate) fn capture_activation_token() -> Option<String> {
    let names: &[&str] = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        &["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
    } else {
        &["DESKTOP_STARTUP_ID", "XDG_ACTIVATION_TOKEN"]
    };
    let mut token = None;
    for name in names {
        if let Some(value) = std::env::var_os(name) {
            if let Ok(text) = value.into_string() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    token = Some(trimmed.to_string());
                    break;
                }
            }
        }
    }
    // winit documents clearing both variables after reading them; otherwise a
    // child process can accidentally reuse a one-shot activation token.
    std::env::remove_var("XDG_ACTIVATION_TOKEN");
    std::env::remove_var("DESKTOP_STARTUP_ID");
    token
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

#[cfg(target_os = "linux")]
fn x11_remove_startup_notification(token: &str) -> anyhow::Result<()> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, WindowClass,
    };
    use x11rb::COPY_FROM_PARENT;

    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn
        .setup()
        .roots
        .get(screen_num)
        .ok_or_else(|| anyhow::anyhow!("X11 screen {screen_num} is missing"))?
        .root;
    let startup_info_begin = conn
        .intern_atom(false, b"_NET_STARTUP_INFO_BEGIN")?
        .reply()?
        .atom;
    let startup_info = conn.intern_atom(false, b"_NET_STARTUP_INFO")?.reply()?.atom;

    // Startup notification messages use a short-lived, unmapped X window as
    // their message identity. The protocol permits destroying it immediately
    // after the events are queued.
    let message_window = conn.generate_id()?;
    conn.create_window(
        u8::try_from(COPY_FROM_PARENT)?,
        message_window,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new(),
    )?;

    let message = format!("remove: ID={token}\0");
    let bytes = message.as_bytes();
    let mut offset = 0;
    let mut first = true;
    while offset < bytes.len() {
        let mut data = [0_u8; 20];
        let start = usize::from(first);
        let available = (bytes.len() - offset).min(data.len() - start);
        data[start..start + available].copy_from_slice(&bytes[offset..offset + available]);
        offset += available;
        let message_type = if first {
            startup_info_begin
        } else {
            startup_info
        };
        let event = ClientMessageEvent::new(8, message_window, message_type, data);
        conn.send_event(false, root, EventMask::PROPERTY_CHANGE, event)?;
        first = false;
    }
    conn.destroy_window(message_window)?;
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
