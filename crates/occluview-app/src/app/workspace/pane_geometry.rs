//! Pure pane rectangle arithmetic used by the workspace composition.

use eframe::egui::{pos2, Rect};

use super::state::SceneSession;
use crate::app::workspace::id::PaneId;
use crate::app::workspace::layout::{EffectiveLayout, WorkspaceLayout};

/// Height of the header strip at the top of every pane.
pub(super) const PANE_HEADER_HEIGHT: f32 = 30.0;

pub(super) fn pane_header_rect(rect: Rect) -> Rect {
    Rect::from_min_max(
        rect.min,
        pos2(
            rect.right(),
            (rect.top() + PANE_HEADER_HEIGHT).min(rect.bottom()),
        ),
    )
}
pub(super) fn pane_is_visible(layout: EffectiveLayout, pane: PaneId) -> bool {
    match layout {
        EffectiveLayout::Single { pane: visible, .. } => pane == visible,
        EffectiveLayout::SideBySide { left, right, .. } => pane == left.pane || pane == right.pane,
    }
}
pub(super) fn reconcile_single_layout(layout: &mut WorkspaceLayout, active_pane: PaneId) -> bool {
    let WorkspaceLayout::Single { pane } = layout else {
        return false;
    };
    if *pane == active_pane {
        return false;
    }
    *layout = WorkspaceLayout::single(active_pane);
    true
}
pub(super) fn default_split(scenes: &[SceneSession]) -> WorkspaceLayout {
    let Some(left) = scenes.first() else {
        return WorkspaceLayout::single(PaneId::INITIAL);
    };
    let Some(right) = scenes.get(1) else {
        return WorkspaceLayout::single(left.pane);
    };
    WorkspaceLayout::SideBySide {
        left: left.pane,
        right: right.pane,
        ratio: 0.5,
    }
}
pub(super) fn divider_rect(layout: EffectiveLayout) -> Option<Rect> {
    match layout {
        EffectiveLayout::Single { .. } => None,
        EffectiveLayout::SideBySide { divider, .. } => Some(divider),
    }
}
