//! Pointer focus and gesture ownership for scene panes.

use super::id::{PaneId, SceneKey};
use occluview_core::SceneMeshId;

/// A live scene shown in a stable pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PaneTarget {
    pub(crate) scene: SceneKey,
    pub(crate) pane: PaneId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GestureKind {
    Click,
    CameraOrbit,
    CameraPan,
    SculptStroke,
    Lasso,
    Ruler,
    AlignDrag,
    UiControl,
    WorkspaceControl,
    DividerResize,
    LayerDrag,
}

impl GestureKind {
    #[must_use]
    pub(crate) const fn allows_viewport_input(self) -> bool {
        matches!(
            self,
            Self::Click
                | Self::CameraOrbit
                | Self::CameraPan
                | Self::SculptStroke
                | Self::Lasso
                | Self::Ruler
                | Self::AlignDrag
        )
    }
}

/// Stable owner of a gesture from its start until release or cancellation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GestureOwner {
    pub(crate) target: PaneTarget,
    pub(crate) kind: GestureKind,
    pub(crate) layer: Option<SceneMeshId>,
    start: GestureStart,
}

impl GestureOwner {
    #[must_use]
    pub(crate) fn divider_initial_ratio(self) -> Option<f32> {
        match self.start {
            GestureStart::DividerResize { initial_ratio_bits } => {
                Some(f32::from_bits(initial_ratio_bits))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GestureStart {
    PrimaryPress,
    DividerResize { initial_ratio_bits: u32 },
    SafeGesture,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PointerButtons {
    pub(crate) primary: bool,
    pub(crate) secondary: bool,
    pub(crate) middle: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActivationResult {
    Unchanged,
    Activated,
    DeferredUntilGestureEnds,
    IgnoredWhileGestureCaptured,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PressResult {
    Begin(GestureOwner),
    ActivateOnly(PaneTarget),
    RouteToCapture(GestureOwner),
    Suppressed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerRoute {
    Target(PaneTarget),
    Captured(GestureOwner),
    Suppressed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReleaseResult {
    Finished(GestureOwner),
    Suppressed,
    Unclaimed,
}

/// Owns the active target and fences pointer input across pane transitions.
#[derive(Clone, Debug)]
// Each physical button needs an independent release fence; focus is orthogonal.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct InputArbiter {
    active: PaneTarget,
    capture: Option<GestureOwner>,
    suppress_primary_until_release: bool,
    suppress_secondary_until_release: bool,
    suppress_middle_until_release: bool,
    pending_activation: Option<PaneTarget>,
    focused: bool,
}

impl InputArbiter {
    #[must_use]
    pub(crate) const fn new(active: PaneTarget) -> Self {
        Self {
            active,
            capture: None,
            suppress_primary_until_release: false,
            suppress_secondary_until_release: false,
            suppress_middle_until_release: false,
            pending_activation: None,
            focused: true,
        }
    }

    #[must_use]
    pub(crate) const fn active(&self) -> PaneTarget {
        self.active
    }

    #[must_use]
    pub(crate) const fn capture(&self) -> Option<GestureOwner> {
        self.capture
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn suppresses_primary(&self) -> bool {
        self.suppress_primary_until_release
    }

    /// Explicit activation (for scene controls) waits for an owned gesture to
    /// finish so an in-progress stroke is never retargeted midway through.
    pub(crate) fn request_activation(&mut self, target: PaneTarget) -> ActivationResult {
        if self.capture.is_some() || self.pointer_is_suppressed() {
            self.pending_activation = Some(target);
            return ActivationResult::DeferredUntilGestureEnds;
        }
        if self.active == target {
            ActivationResult::Unchanged
        } else {
            self.active = target;
            ActivationResult::Activated
        }
    }

    /// A primary press on an inactive pane activates it and consumes the full
    /// press/release sequence. The next press can begin its usual tool action.
    pub(crate) fn primary_pressed(
        &mut self,
        target: PaneTarget,
        kind: GestureKind,
        layer: Option<SceneMeshId>,
    ) -> PressResult {
        if !self.focused || self.pointer_is_suppressed() {
            return PressResult::Suppressed;
        }
        if let Some(owner) = self.capture {
            return PressResult::RouteToCapture(owner);
        }
        if self.active != target {
            self.active = target;
            self.suppress_primary_until_release = true;
            return PressResult::ActivateOnly(target);
        }
        let owner = GestureOwner {
            target,
            kind,
            layer,
            start: GestureStart::PrimaryPress,
        };
        self.capture = Some(owner);
        PressResult::Begin(owner)
    }

    /// Begin a divider drag with enough context to restore its prior split on
    /// Escape or focus loss.
    pub(crate) fn begin_divider_resize(
        &mut self,
        target: PaneTarget,
        initial_ratio: f32,
    ) -> PressResult {
        match self.primary_pressed(target, GestureKind::DividerResize, None) {
            PressResult::Begin(mut owner) => {
                owner.start = GestureStart::DividerResize {
                    initial_ratio_bits: initial_ratio.to_bits(),
                };
                self.capture = Some(owner);
                PressResult::Begin(owner)
            }
            other => other,
        }
    }

    /// Camera gestures are safe to start on the hovered pane, so they activate
    /// that pane and capture ownership in the same transition.
    pub(crate) fn begin_safe_gesture(
        &mut self,
        target: PaneTarget,
        kind: GestureKind,
    ) -> PressResult {
        if !self.focused || self.pointer_is_suppressed() {
            return PressResult::Suppressed;
        }
        if let Some(owner) = self.capture {
            return PressResult::RouteToCapture(owner);
        }
        self.active = target;
        let owner = GestureOwner {
            target,
            kind,
            layer: None,
            start: GestureStart::SafeGesture,
        };
        self.capture = Some(owner);
        PressResult::Begin(owner)
    }

    /// Discrete hover-driven input such as a wheel may activate the pane under
    /// the pointer, except while another gesture owns input.
    pub(crate) fn activate_under_pointer(&mut self, target: PaneTarget) -> ActivationResult {
        if self.capture.is_some() || self.pointer_is_suppressed() {
            return ActivationResult::IgnoredWhileGestureCaptured;
        }
        self.request_activation(target)
    }

    #[must_use]
    pub(crate) fn route_pointer(&self, hovered: Option<PaneTarget>) -> PointerRoute {
        if !self.focused || self.pointer_is_suppressed() {
            PointerRoute::Suppressed
        } else if let Some(owner) = self.capture {
            PointerRoute::Captured(owner)
        } else if let Some(target) = hovered {
            PointerRoute::Target(target)
        } else {
            PointerRoute::Target(self.active)
        }
    }

    /// End capture at the owning gesture's release. A deferred scene switch is
    /// then applied as one state transition.
    pub(crate) fn gesture_finished(&mut self) -> Option<GestureOwner> {
        let owner = self.capture.take()?;
        self.apply_pending_activation_if_idle();
        Some(owner)
    }

    /// Escape cancels the owner and consumes its eventual mouse release.
    pub(crate) fn escape(&mut self, buttons: PointerButtons) -> Option<GestureOwner> {
        let owner = self.capture.take();
        self.suppress_down_buttons(buttons);
        if let Some(owner) = owner {
            match owner.start {
                GestureStart::PrimaryPress | GestureStart::DividerResize { .. } => {
                    self.suppress_primary_until_release = true;
                }
                GestureStart::SafeGesture => match owner.kind {
                    GestureKind::CameraOrbit => self.suppress_secondary_until_release = true,
                    GestureKind::CameraPan => self.suppress_middle_until_release = true,
                    _ => {}
                },
            }
        }
        self.apply_pending_activation_if_idle();
        owner
    }

    /// Window focus loss cancels transient ownership. The caller supplies the
    /// platform's current button state to avoid leaking a held press on return.
    pub(crate) fn focus_lost(&mut self, buttons: PointerButtons) -> Option<GestureOwner> {
        self.focused = false;
        let owner = self.capture.take();
        self.suppress_down_buttons(buttons);
        self.apply_pending_activation_if_idle();
        owner
    }

    /// Mark the next frame as focused again. A button that is already down on
    /// return cannot begin a gesture in whichever pane happens to be active.
    pub(crate) fn focus_gained(&mut self, buttons: PointerButtons) {
        let regained_focus = !self.focused;
        self.focused = true;
        if regained_focus && self.capture.is_none() {
            self.suppress_down_buttons(buttons);
        }
    }

    /// Consume a primary release. Suppressed activation clicks never reach a
    /// tool; ordinary releases finish the captured gesture.
    pub(crate) fn primary_released(&mut self) -> ReleaseResult {
        if self.suppress_primary_until_release {
            self.suppress_primary_until_release = false;
            self.apply_pending_activation_if_idle();
            return ReleaseResult::Suppressed;
        }
        match self.capture {
            Some(owner)
                if matches!(
                    owner.start,
                    GestureStart::PrimaryPress | GestureStart::DividerResize { .. }
                ) =>
            {
                self.gesture_finished()
                    .map_or(ReleaseResult::Unclaimed, ReleaseResult::Finished)
            }
            _ => ReleaseResult::Unclaimed,
        }
    }

    /// End a safe camera gesture only when its own mouse button releases.
    /// Releasing the other button in a chord must not transfer a held orbit or
    /// pan to the pane now under the pointer.
    pub(crate) fn safe_gesture_released(&mut self, kind: GestureKind) -> ReleaseResult {
        if !matches!(kind, GestureKind::CameraOrbit | GestureKind::CameraPan) {
            return ReleaseResult::Unclaimed;
        }
        match self.capture {
            Some(owner) if owner.kind == kind && owner.start == GestureStart::SafeGesture => self
                .gesture_finished()
                .map_or(ReleaseResult::Unclaimed, ReleaseResult::Finished),
            _ => ReleaseResult::Unclaimed,
        }
    }

    /// Clear cancelled-button fences after egui reports their buttons released.
    /// An unowned primary press is not a viewport gesture: the workspace
    /// captures UI and divider presses explicitly before they cross panes.
    pub(crate) fn synchronize_pointer_state(&mut self, buttons: PointerButtons) {
        if !buttons.primary {
            self.suppress_primary_until_release = false;
        }
        if !buttons.secondary {
            self.suppress_secondary_until_release = false;
        }
        if !buttons.middle {
            self.suppress_middle_until_release = false;
        }
        self.apply_pending_activation_if_idle();
    }

    fn suppress_down_buttons(&mut self, buttons: PointerButtons) {
        self.suppress_primary_until_release |= buttons.primary;
        self.suppress_secondary_until_release |= buttons.secondary;
        self.suppress_middle_until_release |= buttons.middle;
    }

    /// A non-primary press that starts on workspace chrome cannot become a
    /// scene orbit or pan if the pointer later crosses into a viewport.
    pub(crate) fn suppress_non_primary_buttons_until_release(&mut self, buttons: PointerButtons) {
        self.suppress_secondary_until_release |= buttons.secondary;
        self.suppress_middle_until_release |= buttons.middle;
    }

    fn pointer_is_suppressed(&self) -> bool {
        self.suppress_primary_until_release
            || self.suppress_secondary_until_release
            || self.suppress_middle_until_release
    }

    fn apply_pending_activation_if_idle(&mut self) {
        if self.capture.is_none() && !self.pointer_is_suppressed() {
            self.apply_pending_activation();
        }
    }

    fn apply_pending_activation(&mut self) {
        if let Some(target) = self.pending_activation.take() {
            self.active = target;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::{
        ActivationResult, GestureKind, InputArbiter, PaneTarget, PointerButtons, PointerRoute,
        PressResult, ReleaseResult,
    };
    use crate::app::workspace::id::{PaneId, SceneKey};

    fn target(scene: u64, epoch: u64, pane: u64) -> PaneTarget {
        PaneTarget {
            scene: SceneKey::from_raw_for_test(scene, epoch).unwrap(),
            pane: PaneId::from_raw_for_test(pane).unwrap(),
        }
    }

    #[test]
    fn first_press_on_inactive_pane_consumes_press_motion_and_release() {
        let first = target(1, 1, 1);
        let second = target(2, 2, 2);
        let mut input = InputArbiter::new(first);

        assert_eq!(
            input.primary_pressed(second, GestureKind::SculptStroke, None),
            PressResult::ActivateOnly(second)
        );
        assert_eq!(input.active(), second);
        assert!(input.capture().is_none());
        assert_eq!(input.route_pointer(Some(first)), PointerRoute::Suppressed);
        input.synchronize_pointer_state(PointerButtons {
            primary: true,
            ..PointerButtons::default()
        });
        assert_eq!(input.route_pointer(Some(second)), PointerRoute::Suppressed);
        assert_eq!(input.primary_released(), ReleaseResult::Suppressed);
        input.synchronize_pointer_state(PointerButtons::default());
        assert!(!input.suppresses_primary());
        assert_eq!(
            input.route_pointer(Some(second)),
            PointerRoute::Target(second)
        );

        assert!(matches!(
            input.primary_pressed(second, GestureKind::SculptStroke, None),
            PressResult::Begin(_)
        ));
    }

    #[test]
    fn captured_gesture_keeps_owner_and_defers_scene_switch_until_release() {
        let first = target(1, 1, 1);
        let second = target(2, 2, 2);
        let mut input = InputArbiter::new(first);
        let PressResult::Begin(owner) = input.primary_pressed(first, GestureKind::Lasso, None)
        else {
            panic!("active pane should start the gesture");
        };

        assert_eq!(
            input.route_pointer(Some(second)),
            PointerRoute::Captured(owner)
        );
        assert_eq!(
            input.request_activation(second),
            ActivationResult::DeferredUntilGestureEnds
        );
        assert_eq!(input.active(), first);
        assert_eq!(input.primary_released(), ReleaseResult::Finished(owner));
        assert_eq!(input.active(), second);
    }

    #[test]
    fn escape_consumes_the_release_of_a_cancelled_press() {
        let first = target(1, 1, 1);
        let mut input = InputArbiter::new(first);
        assert!(matches!(
            input.primary_pressed(first, GestureKind::CameraOrbit, None),
            PressResult::Begin(_)
        ));
        assert!(input
            .escape(PointerButtons {
                primary: true,
                ..PointerButtons::default()
            })
            .is_some());
        assert_eq!(input.route_pointer(Some(first)), PointerRoute::Suppressed);
        assert_eq!(input.primary_released(), ReleaseResult::Suppressed);
        assert!(input.capture().is_none());
    }

    #[test]
    fn safe_camera_gesture_activates_and_starts_on_hovered_pane() {
        let first = target(1, 1, 1);
        let second = target(2, 2, 2);
        let mut input = InputArbiter::new(first);
        assert!(matches!(
            input.begin_safe_gesture(second, GestureKind::CameraPan),
            PressResult::Begin(owner) if owner.target == second
        ));
        assert_eq!(input.active(), second);
        assert!(input.focus_lost(PointerButtons::default()).is_some());
        assert_eq!(input.route_pointer(Some(first)), PointerRoute::Suppressed);
        input.focus_gained(PointerButtons::default());
        assert_eq!(
            input.activate_under_pointer(first),
            ActivationResult::Activated
        );
    }

    #[test]
    fn focus_loss_returns_owner_and_consumes_a_press_held_on_return() {
        let first = target(1, 1, 1);
        let mut input = InputArbiter::new(first);
        let PressResult::Begin(owner) =
            input.primary_pressed(first, GestureKind::SculptStroke, None)
        else {
            panic!("the focused active pane should own the stroke");
        };

        assert_eq!(
            input.focus_lost(PointerButtons {
                primary: true,
                ..PointerButtons::default()
            }),
            Some(owner)
        );
        assert_eq!(input.capture(), None);
        assert_eq!(input.route_pointer(Some(first)), PointerRoute::Suppressed);

        input.focus_gained(PointerButtons {
            primary: true,
            ..PointerButtons::default()
        });
        assert_eq!(
            input.primary_pressed(first, GestureKind::SculptStroke, None),
            PressResult::Suppressed
        );
        assert_eq!(input.primary_released(), ReleaseResult::Suppressed);
        input.synchronize_pointer_state(PointerButtons::default());
        assert_eq!(
            input.route_pointer(Some(first)),
            PointerRoute::Target(first)
        );
    }

    #[test]
    fn repeated_focused_frames_do_not_suppress_a_new_primary_press() {
        let first = target(1, 1, 1);
        let mut input = InputArbiter::new(first);

        // The workspace calls this before routing pointer events on every
        // focused frame, including the frame that reports a new press.
        input.focus_gained(PointerButtons {
            primary: true,
            ..PointerButtons::default()
        });
        assert_eq!(
            input.route_pointer(Some(first)),
            PointerRoute::Target(first)
        );
        assert!(matches!(
            input.primary_pressed(first, GestureKind::SculptStroke, None),
            PressResult::Begin(_)
        ));
    }

    #[test]
    fn ui_capture_blocks_viewport_input_without_suppressing_the_control_press() {
        let first = target(1, 1, 1);
        let mut input = InputArbiter::new(first);

        let PressResult::Begin(owner) = input.primary_pressed(first, GestureKind::UiControl, None)
        else {
            panic!("an active floating control should capture its press");
        };
        assert!(!owner.kind.allows_viewport_input());
        assert_eq!(
            input.route_pointer(Some(first)),
            PointerRoute::Captured(owner)
        );

        assert_eq!(
            input.primary_released(),
            ReleaseResult::Finished(owner),
            "the control owns the complete press/release sequence"
        );
        assert_eq!(
            input.route_pointer(Some(first)),
            PointerRoute::Target(first)
        );
    }

    #[test]
    fn escape_fences_held_camera_buttons_until_their_own_release() {
        let first = target(1, 1, 1);
        let second = target(2, 2, 2);

        for (kind, buttons) in [
            (
                GestureKind::CameraOrbit,
                PointerButtons {
                    secondary: true,
                    ..PointerButtons::default()
                },
            ),
            (
                GestureKind::CameraPan,
                PointerButtons {
                    middle: true,
                    ..PointerButtons::default()
                },
            ),
        ] {
            let mut input = InputArbiter::new(first);
            assert!(matches!(
                input.begin_safe_gesture(first, kind),
                PressResult::Begin(_)
            ));

            assert!(input.escape(buttons).is_some());
            assert_eq!(input.route_pointer(Some(second)), PointerRoute::Suppressed);

            input.synchronize_pointer_state(PointerButtons::default());
            assert_eq!(
                input.route_pointer(Some(second)),
                PointerRoute::Target(second),
                "after the canceled button releases, a later press can target another pane"
            );
        }
    }

    #[test]
    fn focus_loss_cancels_held_secondary_and_middle_gestures() {
        let first = target(1, 1, 1);
        let second = target(2, 2, 2);

        for (kind, buttons) in [
            (
                GestureKind::CameraOrbit,
                PointerButtons {
                    secondary: true,
                    ..PointerButtons::default()
                },
            ),
            (
                GestureKind::CameraPan,
                PointerButtons {
                    middle: true,
                    ..PointerButtons::default()
                },
            ),
        ] {
            let mut input = InputArbiter::new(first);
            assert!(matches!(
                input.begin_safe_gesture(first, kind),
                PressResult::Begin(_)
            ));

            assert!(input.focus_lost(buttons).is_some());
            input.focus_gained(buttons);
            assert_eq!(input.route_pointer(Some(second)), PointerRoute::Suppressed);

            input.synchronize_pointer_state(PointerButtons::default());
            assert_eq!(
                input.route_pointer(Some(second)),
                PointerRoute::Target(second)
            );
        }
    }

    #[test]
    fn camera_gesture_capture_ends_only_on_its_own_button_release() {
        let first = target(1, 1, 1);
        let second = target(2, 2, 2);

        for (kind, other_kind) in [
            (GestureKind::CameraOrbit, GestureKind::CameraPan),
            (GestureKind::CameraPan, GestureKind::CameraOrbit),
        ] {
            let mut input = InputArbiter::new(first);
            let PressResult::Begin(owner) = input.begin_safe_gesture(first, kind) else {
                panic!("the safe camera gesture should capture its pane");
            };
            assert_eq!(
                input.request_activation(second),
                ActivationResult::DeferredUntilGestureEnds
            );

            assert_eq!(
                input.safe_gesture_released(other_kind),
                ReleaseResult::Unclaimed,
                "a chord's unrelated button release must not finish the held gesture"
            );
            assert_eq!(input.capture(), Some(owner));
            assert_eq!(input.active(), first);

            assert_eq!(
                input.safe_gesture_released(kind),
                ReleaseResult::Finished(owner)
            );
            assert_eq!(input.active(), second);
        }
    }

    #[test]
    fn held_camera_button_started_on_chrome_stays_blocked_after_crossing_panes() {
        let first = target(1, 1, 1);
        let second = target(2, 2, 2);
        let mut input = InputArbiter::new(first);
        input.suppress_non_primary_buttons_until_release(PointerButtons {
            secondary: true,
            ..PointerButtons::default()
        });

        assert_eq!(input.route_pointer(Some(second)), PointerRoute::Suppressed);
        assert_eq!(
            input.begin_safe_gesture(second, GestureKind::CameraOrbit),
            PressResult::Suppressed,
            "a held right button begun on UI cannot start orbit in the pane"
        );

        input.synchronize_pointer_state(PointerButtons::default());
        assert!(matches!(
            input.begin_safe_gesture(second, GestureKind::CameraOrbit),
            PressResult::Begin(owner) if owner.target == second
        ));
    }

    #[test]
    fn wheel_over_other_pane_does_not_queue_focus_switch_during_capture() {
        let first = target(1, 1, 1);
        let second = target(2, 2, 2);
        let mut input = InputArbiter::new(first);
        assert!(matches!(
            input.begin_safe_gesture(first, GestureKind::CameraOrbit),
            PressResult::Begin(_)
        ));

        assert_eq!(
            input.activate_under_pointer(second),
            ActivationResult::IgnoredWhileGestureCaptured
        );
        assert_eq!(
            input.gesture_finished().map(|owner| owner.target),
            Some(first)
        );
        assert_eq!(input.active(), first);
    }

    #[test]
    fn unrelated_primary_release_does_not_end_safe_gesture_capture() {
        let first = target(1, 1, 1);
        let mut input = InputArbiter::new(first);
        let PressResult::Begin(owner) = input.begin_safe_gesture(first, GestureKind::CameraOrbit)
        else {
            panic!("safe gesture should capture its owner");
        };

        assert_eq!(input.primary_released(), ReleaseResult::Unclaimed);
        assert_eq!(input.capture(), Some(owner));
        assert_eq!(input.gesture_finished(), Some(owner));
    }
}
