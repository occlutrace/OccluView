//! Pure geometry for one or two side-by-side scene panes.

use super::id::PaneId;
use eframe::egui::{pos2, vec2, Rect};

const MIN_RATIO: f32 = 0.05;
const MAX_RATIO: f32 = 0.95;
const DEFAULT_RATIO: f32 = 0.5;

/// Requested workspace arrangement. It is kept separately from the effective
/// frame so a narrow window can show one pane without losing the split ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WorkspaceLayout {
    Single {
        pane: PaneId,
    },
    SideBySide {
        left: PaneId,
        right: PaneId,
        ratio: f32,
    },
}

impl WorkspaceLayout {
    #[must_use]
    pub(crate) const fn single(pane: PaneId) -> Self {
        Self::Single { pane }
    }

    pub(crate) fn side_by_side(
        left: PaneId,
        right: PaneId,
        ratio: f32,
    ) -> Result<Self, LayoutError> {
        if left == right {
            return Err(LayoutError::DuplicatePane);
        }
        Ok(Self::SideBySide {
            left,
            right,
            ratio: normalized_ratio(ratio),
        })
    }

    #[must_use]
    pub(crate) fn with_ratio(self, ratio: f32) -> Self {
        match self {
            Self::Single { pane } => Self::Single { pane },
            Self::SideBySide { left, right, .. } => Self::SideBySide {
                left,
                right,
                ratio: normalized_ratio(ratio),
            },
        }
    }

    /// Compute this frame's pane rectangles. A narrow frame falls back to the
    /// active pane while leaving `self` untouched for restoration on resize.
    #[must_use]
    pub(crate) fn effective_rects(
        self,
        available: Rect,
        active_pane: PaneId,
        constraints: LayoutConstraints,
    ) -> EffectiveLayout {
        let available = finite_rect(available);
        match self {
            Self::Single { pane } => EffectiveLayout::Single {
                pane,
                rect: available,
                reason: SingleViewReason::Requested,
            },
            Self::SideBySide { left, right, ratio } => {
                let divider_width = finite_nonnegative(constraints.divider_width);
                let minimum = finite_nonnegative(constraints.minimum_pane_width);
                let width = available.width();
                if width < minimum * 2.0 + divider_width {
                    let pane = if active_pane == right { right } else { left };
                    return EffectiveLayout::Single {
                        pane,
                        rect: available,
                        reason: SingleViewReason::InsufficientWidth,
                    };
                }

                let content_width = (width - divider_width).max(0.0);
                let left_width = (content_width * normalized_ratio(ratio))
                    .clamp(minimum, content_width - minimum);
                let divider_min = pos2(available.left() + left_width, available.top());
                let divider_max = pos2(divider_min.x + divider_width, available.bottom());
                let right_min = pos2(divider_max.x, available.top());
                EffectiveLayout::SideBySide {
                    left: PaneRect {
                        pane: left,
                        rect: Rect::from_min_max(
                            available.min,
                            pos2(divider_min.x, available.bottom()),
                        ),
                    },
                    divider: Rect::from_min_max(divider_min, divider_max),
                    right: PaneRect {
                        pane: right,
                        rect: Rect::from_min_max(right_min, available.max),
                    },
                }
            }
        }
    }
}

/// UI-derived sizing constraints; tool-panel requirements stay with the UI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LayoutConstraints {
    pub(crate) divider_width: f32,
    pub(crate) minimum_pane_width: f32,
}

impl Default for LayoutConstraints {
    fn default() -> Self {
        Self {
            divider_width: 8.0,
            minimum_pane_width: 320.0,
        }
    }
}

/// Rect assigned to one stable pane identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaneRect {
    pub(crate) pane: PaneId,
    pub(crate) rect: Rect,
}

/// Effective geometry for a frame; differs from requested layout only when
/// the available width cannot preserve both panes' minimum usable width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EffectiveLayout {
    Single {
        pane: PaneId,
        rect: Rect,
        reason: SingleViewReason,
    },
    SideBySide {
        left: PaneRect,
        divider: Rect,
        right: PaneRect,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SingleViewReason {
    Requested,
    InsufficientWidth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayoutError {
    DuplicatePane,
}

fn normalized_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(MIN_RATIO, MAX_RATIO)
    } else {
        DEFAULT_RATIO
    }
}

fn finite_nonnegative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn finite_rect(rect: Rect) -> Rect {
    let min_x = if rect.min.x.is_finite() {
        rect.min.x
    } else {
        0.0
    };
    let min_y = if rect.min.y.is_finite() {
        rect.min.y
    } else {
        0.0
    };
    let width = finite_nonnegative(rect.width());
    let height = finite_nonnegative(rect.height());
    Rect::from_min_size(pos2(min_x, min_y), vec2(width, height))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::{
        EffectiveLayout, LayoutConstraints, LayoutError, SingleViewReason, WorkspaceLayout,
    };
    use crate::app::workspace::id::PaneId;
    use eframe::egui::{pos2, Rect};

    fn pane(id: u64) -> PaneId {
        PaneId::from_raw_for_test(id).unwrap()
    }

    #[test]
    fn split_respects_ratio_and_keeps_pane_order() {
        let layout = WorkspaceLayout::side_by_side(pane(10), pane(20), 0.25).unwrap();
        let frame = layout.effective_rects(
            Rect::from_min_max(pos2(10.0, 20.0), pos2(1010.0, 620.0)),
            pane(10),
            LayoutConstraints {
                divider_width: 8.0,
                minimum_pane_width: 100.0,
            },
        );

        let EffectiveLayout::SideBySide {
            left,
            divider,
            right,
        } = frame
        else {
            panic!("wide split should remain side by side");
        };
        assert_eq!(left.pane, pane(10));
        assert_eq!(right.pane, pane(20));
        assert_eq!(left.rect.left(), 10.0);
        assert_eq!(divider.left(), left.rect.right());
        assert_eq!(right.rect.right(), 1010.0);
        assert!((left.rect.width() / (left.rect.width() + right.rect.width()) - 0.25).abs() < 0.01);
    }

    #[test]
    fn narrow_frame_shows_active_pane_without_changing_requested_ratio() {
        let layout = WorkspaceLayout::side_by_side(pane(1), pane(2), 0.7).unwrap();
        let frame = layout.effective_rects(
            Rect::from_min_max(pos2(0.0, 0.0), pos2(500.0, 400.0)),
            pane(2),
            LayoutConstraints {
                divider_width: 8.0,
                minimum_pane_width: 300.0,
            },
        );

        assert!(matches!(
            frame,
            EffectiveLayout::Single {
                pane: actual,
                reason: SingleViewReason::InsufficientWidth,
                ..
            } if actual == pane(2)
        ));
        assert!(matches!(layout, WorkspaceLayout::SideBySide { ratio, .. } if ratio == 0.7));
    }

    #[test]
    fn invalid_ratio_is_safe_and_duplicate_panes_are_rejected() {
        assert_eq!(
            WorkspaceLayout::side_by_side(pane(1), pane(1), 0.5),
            Err(LayoutError::DuplicatePane)
        );
        assert!(matches!(
            WorkspaceLayout::side_by_side(pane(1), pane(2), f32::NAN).unwrap(),
            WorkspaceLayout::SideBySide { ratio: 0.5, .. }
        ));
    }
}
