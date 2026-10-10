//! Tool-owned state: edit, sculpt, align, cut, bridge-split, measure, and the
//! mesh-editor tab routing between them.
//!
//! Owned invariants:
//!
//! - Each tool owns its workflow state. Viewport tool entry releases competing
//!   gestures through [`super::SceneContext::prepare_viewport_tool_entry`].
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

use super::{egui, SceneContext};
use crate::align::align_state::AlignState;
use crate::bridge_split::{BridgeSplitController, BridgeSplitMode};
use crate::cut::cut_manipulator::CutManipulator;
use crate::cut::cut_tool::CutTool;
use crate::cut::section_view::SectionView;
use crate::measure::measure_tool::MeasureTool;
use crate::mesh_editor::mesh_editor_overlay::EditorTab;
use crate::sculpt::sculpt_tool::SculptTool;
use occluview_core::SceneMeshId;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ViewportTool {
    Edit,
    Align,
    Cut,
    Measure,
}

impl SceneContext<'_> {
    /// Whether an open editing session, sculpt tool or alignment must stay as
    /// it is, which rules out Cut View until they are finished or cancelled.
    fn editing_blocks_cut_view(&self) -> bool {
        self.document.edit_mode.has_active_session()
            || self.tools.sculpt.armed.is_some()
            || self.align_active()
    }

    /// Release the previous viewport owner before a new tool is armed.
    pub(super) fn prepare_viewport_tool_entry(
        &mut self,
        next: ViewportTool,
        ctx: &egui::Context,
    ) -> bool {
        if self.document.edit_mode.is_busy() || self.tools.sculpt.is_busy() {
            self.scene_ui.status_message = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("repair-edit-busy")),
            );
            ctx.request_repaint();
            return false;
        }
        // Cut View cannot share the viewport with an open editing session, and
        // entering it must not end that session or drop its sculpt state behind
        // the operator's back. Refuse, say why, and leave the session as it was.
        if next == ViewportTool::Cut && self.editing_blocks_cut_view() {
            self.scene_ui.status_message = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("cut-blocked-by-edit")),
            );
            ctx.request_repaint();
            return false;
        }
        if next != ViewportTool::Edit && self.document.edit_mode.has_active_session() {
            self.finish_mesh_edit_session_now(ctx);
        }
        if next != ViewportTool::Align && self.tools.align.tool.is_armed() {
            if next == ViewportTool::Edit {
                // Like Bridge Split entry, editing keeps the current scan poses.
                self.finish_align_session(ctx);
            } else {
                self.cancel_align_session(ctx);
                if self.tools.align.tool.is_armed() {
                    return false;
                }
            }
        }
        if self.tools.bridge_split_active() {
            let reason = self
                .ui
                .locale
                .tr(crate::i18n::message_id!("bridge-canceled"));
            self.cancel_bridge_split(&reason);
        }
        self.abort_sculpt_stroke();
        self.tools.sculpt.disarm();
        if next != ViewportTool::Cut {
            self.tools.cut_view.disable();
        }
        if next != ViewportTool::Measure {
            self.tools.measure.disarm();
        }
        self.document.mesh_selection_drag = None;
        self.render.invalidation.overlay_tools_changed();
        ctx.request_repaint();
        true
    }

    /// Start or retarget the one selection session used by all Edit entries.
    pub(super) fn begin_mesh_edit_session(&mut self, layer_id: SceneMeshId) -> bool {
        let editable = self.document.scene.as_ref().is_some_and(|scene| {
            scene.meshes().iter().any(|entry| {
                entry.id() == layer_id
                    && entry.visible
                    && !entry.mesh.is_point_cloud()
                    && entry.mesh.triangle_count() > 0
            })
        });
        if !editable {
            return false;
        }
        let ctx = self.ui.repaint_ctx.clone();
        if !self.prepare_viewport_tool_entry(ViewportTool::Edit, &ctx) {
            return false;
        }
        // Teardown can remove alignment display state. Capture the settled
        // document so Cancel never restores the previous tool's heatmap.
        let Some(scene) = self.document.scene.as_deref().cloned() else {
            return false;
        };
        let Some(entry) = scene.meshes().iter().find(|entry| entry.id() == layer_id) else {
            return false;
        };
        let starting = !self.document.edit_mode.has_active_session();
        if starting {
            self.document.capture_edit_metadata();
        }
        if !self.document.edit_mode.begin_face_selection(entry, &scene) {
            if starting {
                self.document.discard_edit_metadata();
            }
            return false;
        }
        self.tools.editor_tab = EditorTab::EditMesh;
        self.render.invalidation.selection_changed();
        true
    }
}

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
    /// The occlusal contact reading: the pair it runs between, the law it is
    /// read under, the load depth, the packed fields the viewport paints, and
    /// its own worker. Independent of `align`: a reading runs over
    /// its own pair and takes its roles as arguments, so nothing here can pick
    /// up whichever scans the alignment happened to be looking at.
    pub(super) contacts: crate::contact::state::ContactState,
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
            contacts: crate::contact::state::ContactState::default(),
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
