//! Platform-owned state: native handles, single-instance handoff, and shell
//! integration.
//!
//! Owned invariants:
//!
//! - `_single_instance` guards the process lifetime; dropping it releases
//!   the primary claim.
//! - `incoming_open_requests` carries second-instance file handoffs with
//!   their activation tokens; [`PlatformState::take_open_requests`] drains
//!   them without blocking the frame.
//! - `pending_raise_token` keeps the most recent activation provenance until
//!   a raise consumes it.
//!
//! Permitted mutation entry points: [`PlatformState::new`] for bootstrap,
//! the open-request poll for arrivals. Cross-domain outputs: drained open
//! requests feed the loading pipeline, the raise target feeds window
//! activation.

use super::single_instance;
use eframe::egui;

/// Everything the bootstrap hands the app about how this process was started:
/// the single-instance guard, the window raise handle, and the launcher's
/// activation token (focus provenance for the first load).
pub(crate) struct StartupHandles {
    pub(crate) single_instance: single_instance::SingleInstance,
    pub(crate) raise_target: single_instance::RaiseTarget,
    pub(crate) activation_token: Option<String>,
}

pub(super) struct PlatformState {
    /// The process-wide primary claim. `None` in a headless test, which must
    /// not take the operator's lock file.
    pub(super) _single_instance: Option<single_instance::SingleInstance>,
    pub(super) incoming_open_requests: single_instance::OpenRequestListener,
    /// Raises the window on an open-file handoff through the native compositor
    /// activation protocol. See activation.rs.
    pub(super) raise_target: single_instance::RaiseTarget,
    /// Latest window-activation token forwarded by a second instance, used as
    /// provenance for the raise. Cleared once the raise's attention pulse ends.
    pub(super) pending_raise_token: Option<String>,
}

impl PlatformState {
    pub(super) fn new(repaint_ctx: egui::Context, startup: StartupHandles) -> Self {
        Self {
            incoming_open_requests: single_instance::OpenRequestListener::spawn(repaint_ctx),
            _single_instance: Some(startup.single_instance),
            raise_target: startup.raise_target,
            pending_raise_token: startup.activation_token,
        }
    }

    /// A platform state for a headless test: no process-wide single-instance
    /// claim, no listener thread, no activation target. The open-request side is
    /// only read through `take_open_requests`, which stays empty.
    #[cfg(test)]
    pub(super) fn for_tests(repaint_ctx: egui::Context) -> Self {
        let _ = repaint_ctx;
        Self {
            incoming_open_requests: single_instance::OpenRequestListener::for_tests(),
            _single_instance: None,
            raise_target: single_instance::RaiseTarget::default(),
            pending_raise_token: None,
        }
    }

    /// Drain second-instance open requests without blocking the frame.
    pub(super) fn take_open_requests(&self) -> Vec<single_instance::OpenRequest> {
        self.incoming_open_requests.take_requests()
    }

    /// Keep the most recent forwarded activation token; it is the provenance
    /// the post-load raise uses.
    pub(super) fn remember_raise_token(&mut self, token: Option<String>) {
        keep_latest_provenance(&mut self.pending_raise_token, token);
    }

    /// Consume the pending activation provenance for a raise attempt.
    pub(super) fn take_raise_token(&mut self) -> Option<String> {
        self.pending_raise_token.take()
    }
}

/// Pure activation-provenance rule behind
/// [`PlatformState::remember_raise_token`]: a forwarded token replaces the
/// slot, an absent token leaves it untouched. Kept as a free function so the
/// rule is testable without a real single-instance guard or an egui context;
/// [`PlatformState::take_raise_token`] is [`Option::take`] on the same slot.
fn keep_latest_provenance(slot: &mut Option<String>, token: Option<String>) {
    if token.is_some() {
        *slot = token;
    }
}

#[cfg(test)]
mod tests {
    use super::keep_latest_provenance;

    #[test]
    fn raise_token_keeps_only_the_most_recent_provenance() {
        // Deterministic: exercises the exact remember/take sequence against a
        // plain slot, with no single-instance guard and no early return.
        let mut slot: Option<String> = None;
        assert!(slot.take().is_none());
        keep_latest_provenance(&mut slot, None);
        assert!(slot.take().is_none());
        keep_latest_provenance(&mut slot, Some("first".to_string()));
        keep_latest_provenance(&mut slot, Some("second".to_string()));
        assert_eq!(slot.take().as_deref(), Some("second"));
        assert!(slot.take().is_none());
    }
}
