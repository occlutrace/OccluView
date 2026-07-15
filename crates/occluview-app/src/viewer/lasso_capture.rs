//! Pure decision logic for the armed mesh-edit lasso (exocad-style outline).
//!
//! # Why this exists (the bug it fixes)
//!
//! egui only reports `Response::clicked()` when the pointer moved less than
//! `InputOptions::max_click_dist` (6.0 px in egui 0.29) between the button press
//! and its release — see `PointerState::begin_pass` /
//! `could_any_button_be_click` in `egui/src/input_state/mod.rs`. A fast hand or
//! a high-DPI mouse exceeds 6 px on almost every click, so egui reclassifies the
//! gesture as a *drag* and never emits a click. The previous lasso added a point
//! on `response.clicked_by(Primary)`, so those points were silently dropped and
//! the operator saw only the rare, sparse clicks that happened to stay under
//! 6 px — the reported "clicks do nothing, only stray angular segments appear".
//!
//! The fix is to capture on the primary **press** edge (which egui always
//! reports), and to additionally sample points while the button is held so a
//! drag draws a smooth freehand outline. Both gestures — discrete click-click
//! corners and press-and-drag freehand — are handled by the same machine and
//! can be mixed freely.
//!
//! This module is intentionally free of egui side effects: it takes a snapshot
//! of the current frame's pointer/keyboard facts plus the outline collected so
//! far, and returns a single [`LassoEvent`]. The viewport adapter feeds it real
//! egui input and applies the event. That keeps the decision logic exhaustively
//! unit-testable without a live egui context.

use eframe::egui::Pos2;

/// A closed outline needs at least a triangle's worth of points.
pub(crate) const MIN_LASSO_POINTS: usize = 3;

/// Pressing this close (screen px) to the first point closes the outline.
pub(crate) const CLOSE_FIRST_RADIUS_PX: f32 = 8.0;

/// Freehand sampling / coincident-press dedup distance (screen px). While the
/// button is held, a new sample is only taken once the cursor has travelled at
/// least this far from the last captured point; the same threshold rejects a
/// press that lands on top of the previous point (e.g. a double-click's second
/// press).
pub(crate) const FREEHAND_MIN_SPACING_PX: f32 = 4.0;

/// One frame's worth of the pointer/keyboard facts the machine needs, plus a
/// summary of the outline collected so far. All geometry is in screen points.
///
/// This is a per-frame input snapshot, so the several independent booleans are
/// inherent (each mirrors one egui edge/state), not a flags-struct smell.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LassoFrameInput {
    /// Primary button went down this frame (`i.pointer.button_pressed`). This is
    /// the always-reported edge the capture is built on.
    pub pressed: bool,
    /// Primary button is currently held (`i.pointer.button_down`).
    pub down: bool,
    /// egui classified a double-click this frame. Kept as a close trigger, but
    /// no longer the only reliable one (it shares egui's move-distance limit).
    pub double_clicked: bool,
    /// Enter was pressed this frame (explicit close).
    pub enter: bool,
    /// Esc was pressed this frame (abandon the outline).
    pub escape: bool,
    /// The pointer is over the viewport response and not covered by any egui
    /// window/panel. Guards against a press on the mesh-editor window adding a
    /// stray point.
    pub over_viewport: bool,
    /// Current pointer position (the press position on a press frame, otherwise
    /// the live cursor).
    pub pointer_pos: Option<Pos2>,
    /// First captured point, if an outline is in progress (the close handle).
    pub first_point: Option<Pos2>,
    /// Last captured point, if any (for spacing / dedup).
    pub last_point: Option<Pos2>,
    /// Number of points captured so far.
    pub point_count: usize,
}

/// What the viewport adapter should do with the current frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LassoEvent {
    /// Append a deliberate vertex at this position (first press or a click
    /// corner). Creates the outline if none exists yet.
    AddPoint(Pos2),
    /// Append a freehand sample point at this position (drag).
    Sample(Pos2),
    /// Close the outline and commit the selection.
    Close,
    /// Abandon the outline (Esc); the lasso stays armed.
    Drop,
    /// Nothing to change this frame.
    None,
}

/// Decide what a single frame of input means for the armed lasso.
///
/// Precedence: Esc (abandon) → explicit close (Enter / double-click) → primary
/// press (close-on-first-point or add a vertex) → freehand sampling while held.
pub(crate) fn decide(input: &LassoFrameInput) -> LassoEvent {
    let outline_active = input.point_count > 0;

    // Esc always abandons an in-progress outline and never leaks elsewhere.
    if input.escape {
        return if outline_active {
            LassoEvent::Drop
        } else {
            LassoEvent::None
        };
    }

    // Explicit close gestures. Both require a real polygon; if there are too few
    // points we refuse without discarding what the operator has drawn.
    if input.enter || input.double_clicked {
        if outline_active && input.point_count >= MIN_LASSO_POINTS {
            return LassoEvent::Close;
        }
        return LassoEvent::None;
    }

    // Primary PRESS: the exocad point-placement edge. egui always reports this,
    // so no input is lost regardless of how far the click travels.
    if input.pressed {
        if !input.over_viewport {
            // The press belongs to an egui window/panel, not the viewport.
            return LassoEvent::None;
        }
        let Some(pos) = input.pointer_pos else {
            return LassoEvent::None;
        };
        // Pressing back onto the first-point handle closes the loop.
        if outline_active
            && input.point_count >= MIN_LASSO_POINTS
            && input
                .first_point
                .is_some_and(|first| first.distance(pos) <= CLOSE_FIRST_RADIUS_PX)
        {
            return LassoEvent::Close;
        }
        // Otherwise place a vertex, skipping a press coincident with the last
        // point (also absorbs the second press of a double-click).
        if point_is_new(pos, input.last_point) {
            return LassoEvent::AddPoint(pos);
        }
        return LassoEvent::None;
    }

    // Freehand: while the button is held and the cursor travels past the min
    // spacing, drop sample points so a drag draws a smooth outline.
    if input.down && outline_active && input.over_viewport {
        if let Some(pos) = input.pointer_pos {
            if point_is_new(pos, input.last_point) {
                return LassoEvent::Sample(pos);
            }
        }
    }

    LassoEvent::None
}

/// A point is worth keeping if it is the first one, or if it is at least the
/// freehand spacing away from the previous point.
fn point_is_new(pos: Pos2, last: Option<Pos2>) -> bool {
    last.is_none_or(|last| last.distance(pos) >= FREEHAND_MIN_SPACING_PX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    /// Builder for a quiet frame with an optional in-progress outline.
    fn frame(points: &[Pos2]) -> LassoFrameInput {
        LassoFrameInput {
            over_viewport: true,
            first_point: points.first().copied(),
            last_point: points.last().copied(),
            point_count: points.len(),
            ..LassoFrameInput::default()
        }
    }

    #[test]
    fn fast_press_still_places_a_point_when_egui_would_drop_the_click() {
        // The core regression: even if egui never reports a click (the pointer
        // moved > max_click_dist), a primary PRESS over the viewport adds a point.
        let input = LassoFrameInput {
            pressed: true,
            down: true,
            pointer_pos: Some(pos2(120.0, 80.0)),
            ..frame(&[])
        };
        assert_eq!(decide(&input), LassoEvent::AddPoint(pos2(120.0, 80.0)));
    }

    #[test]
    fn click_click_makes_straight_segments() {
        // A second deliberate press, far from the first, adds another corner.
        let input = LassoFrameInput {
            pressed: true,
            down: true,
            pointer_pos: Some(pos2(200.0, 200.0)),
            ..frame(&[pos2(100.0, 100.0)])
        };
        assert_eq!(decide(&input), LassoEvent::AddPoint(pos2(200.0, 200.0)));
    }

    #[test]
    fn freehand_drag_samples_when_moved_far_enough() {
        let input = LassoFrameInput {
            pressed: false,
            down: true,
            pointer_pos: Some(pos2(110.0, 100.0)),
            ..frame(&[pos2(100.0, 100.0)])
        };
        assert_eq!(decide(&input), LassoEvent::Sample(pos2(110.0, 100.0)));
    }

    #[test]
    fn freehand_drag_ignores_tiny_movement() {
        // Below FREEHAND_MIN_SPACING_PX from the last point: no new sample.
        let input = LassoFrameInput {
            pressed: false,
            down: true,
            pointer_pos: Some(pos2(101.0, 100.0)),
            ..frame(&[pos2(100.0, 100.0)])
        };
        assert_eq!(decide(&input), LassoEvent::None);
    }

    #[test]
    fn coincident_press_is_deduped() {
        // A press on top of the previous point (double-click's second press).
        let input = LassoFrameInput {
            pressed: true,
            down: true,
            pointer_pos: Some(pos2(100.5, 100.0)),
            ..frame(&[pos2(0.0, 0.0), pos2(50.0, 0.0), pos2(100.0, 100.0)])
        };
        assert_eq!(decide(&input), LassoEvent::None);
    }

    #[test]
    fn press_on_first_point_closes_when_polygon_is_complete() {
        let points = [pos2(100.0, 100.0), pos2(200.0, 100.0), pos2(150.0, 200.0)];
        let input = LassoFrameInput {
            pressed: true,
            down: true,
            pointer_pos: Some(pos2(103.0, 101.0)), // within CLOSE_FIRST_RADIUS_PX
            ..frame(&points)
        };
        assert_eq!(decide(&input), LassoEvent::Close);
    }

    #[test]
    fn press_near_first_point_adds_when_too_few_points() {
        // Only two points so far: a press near the start extends, not closes.
        let points = [pos2(100.0, 100.0), pos2(200.0, 100.0)];
        let input = LassoFrameInput {
            pressed: true,
            down: true,
            pointer_pos: Some(pos2(102.0, 100.0)),
            ..frame(&points)
        };
        assert_eq!(decide(&input), LassoEvent::AddPoint(pos2(102.0, 100.0)));
    }

    #[test]
    fn double_click_closes_when_polygon_is_complete() {
        let points = [pos2(0.0, 0.0), pos2(50.0, 0.0), pos2(25.0, 40.0)];
        let input = LassoFrameInput {
            double_clicked: true,
            ..frame(&points)
        };
        assert_eq!(decide(&input), LassoEvent::Close);
    }

    #[test]
    fn enter_closes_when_polygon_is_complete() {
        let points = [pos2(0.0, 0.0), pos2(50.0, 0.0), pos2(25.0, 40.0)];
        let input = LassoFrameInput {
            enter: true,
            ..frame(&points)
        };
        assert_eq!(decide(&input), LassoEvent::Close);
    }

    #[test]
    fn close_gesture_refused_under_three_points() {
        let points = [pos2(0.0, 0.0), pos2(50.0, 0.0)];
        let by_double = LassoFrameInput {
            double_clicked: true,
            ..frame(&points)
        };
        let by_enter = LassoFrameInput {
            enter: true,
            ..frame(&points)
        };
        assert_eq!(decide(&by_double), LassoEvent::None);
        assert_eq!(decide(&by_enter), LassoEvent::None);
    }

    #[test]
    fn escape_drops_an_active_outline_but_is_inert_when_empty() {
        let active = LassoFrameInput {
            escape: true,
            ..frame(&[pos2(0.0, 0.0), pos2(1.0, 1.0)])
        };
        assert_eq!(decide(&active), LassoEvent::Drop);

        let empty = LassoFrameInput {
            escape: true,
            ..frame(&[])
        };
        assert_eq!(decide(&empty), LassoEvent::None);
    }

    #[test]
    fn press_over_an_egui_window_is_ignored() {
        // over_viewport = false: the mesh-editor window owns the click.
        let input = LassoFrameInput {
            pressed: true,
            down: true,
            over_viewport: false,
            pointer_pos: Some(pos2(10.0, 10.0)),
            ..frame(&[pos2(0.0, 0.0)])
        };
        assert_eq!(decide(&input), LassoEvent::None);
    }

    #[test]
    fn escape_wins_over_a_simultaneous_press() {
        let input = LassoFrameInput {
            pressed: true,
            down: true,
            escape: true,
            pointer_pos: Some(pos2(10.0, 10.0)),
            ..frame(&[pos2(0.0, 0.0), pos2(5.0, 5.0)])
        };
        assert_eq!(decide(&input), LassoEvent::Drop);
    }
}
