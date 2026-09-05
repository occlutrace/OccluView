//! Tool-owned state: edit, sculpt, align, cut, bridge-split, measure, and the
//! mesh-editor tab routing between them.
//!
//! Owned invariants:
//!
//! - Each tool owns its workflow state. Bridge Split owns the scene while
//!   armed (checked via [`ToolState::bridge_split_active`]); cut and measure
//!   arm independently and callers gate through the modal predicate.
//! - `edit_mode` (selection/undo) lives in [`DocumentState`](super::state_document::DocumentState);
//!   this owner coordinates exclusion and cancellation across controllers,
//!   it does not duplicate selection.
//! - Worker identity (sculpt session match, align generation) is checked at
//!   the tool boundary so stale background results are rejected, never
//!   applied.
//!
//! Permitted mutation entry points: [`ToolState::new`] for bootstrap, each
//! tool's panel/workflow for its own controller, root orchestration for
//! cross-tool arbitration. Cross-domain outputs: committed edits feed the
//! document, invalidation requests feed the renderer.

use crate::align_state::AlignState;
use crate::bridge_split::{BridgeSplitController, BridgeSplitMode};
use crate::cut_manipulator::CutManipulator;
use crate::cut_tool::CutTool;
use crate::measure_tool::MeasureTool;
use crate::mesh_editor_overlay::EditorTab;
use crate::sculpt_tool::SculptTool;
use crate::section_view::SectionView;

pub(super) struct ToolState {
    pub(super) cut_view: CutTool,
    /// Bridge-separator controller and its world-fixed placement disc. Kept
    /// separate from Cut View: one previews a structural mesh operation, the
    /// other only changes viewport clipping.
    pub(super) bridge_split: BridgeSplitController,
    pub(super) bridge_split_disc: CutManipulator,
    /// Passive Cut View panel driven by the Bridge Split disc. It owns no
    /// placement interaction, so the bridge tool remains the single pose owner.
    pub(super) bridge_split_section: SectionView,
    /// Viewport measurement tools (ruler + wall-thickness probe). Mutually
    /// exclusive with `cut_view`; anchors are world-space and re-project every
    /// frame.
    pub(super) measure: MeasureTool,
    /// Interactive sculpt-brush tool and active stroke state.
    pub(super) sculpt: SculptTool,
    /// The Align Scans tool's whole state: tool, worker, settings, deviation
    /// display, markings, drag, brush and session poses. One struct so the app
    /// carries a single `align` field instead of eighteen loose ones.
    pub(super) align: AlignState,
    /// Which mesh-editor tab is showing (selection/repair vs sculpt).
    pub(super) editor_tab: EditorTab,
}

impl ToolState {
    pub(super) fn new() -> Self {
        Self {
            cut_view: CutTool::default(),
            bridge_split: BridgeSplitController::default(),
            bridge_split_disc: CutManipulator::default(),
            bridge_split_section: SectionView::default(),
            measure: MeasureTool::default(),
            sculpt: SculptTool::default(),
            align: AlignState::default(),
            editor_tab: EditorTab::default(),
        }
    }

    pub(super) fn bridge_split_active(&self) -> bool {
        self.bridge_split.session().mode() != BridgeSplitMode::Off
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_start_idle_with_no_modal_tool_armed() {
        let tools = ToolState::new();

        assert!(!tools.bridge_split_active());
        assert!(tools.sculpt.armed.is_none());
    }
}
