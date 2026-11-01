//! Arming and switching sculpt tools.

use super::super::{egui, mesh_editor_overlay, SceneContext};
use crate::sculpt::sculpt_tool::SculptToolKind;

impl SceneContext<'_> {
    /// Arm/disarm a sculpt tool (toggling the armed one disarms).
    pub(in crate::app) fn toggle_sculpt_tool(&mut self, kind: SculptToolKind, ctx: &egui::Context) {
        // Finish a live stroke before switching brush modes. Other context
        // switches use their existing abort paths.
        if !self.commit_sculpt_stroke(ctx) {
            return;
        }
        self.tools.sculpt.toggle(kind);
        if self.tools.sculpt.armed.is_some() {
            if self.persistence.settings.last_sculpt_tool != kind {
                self.persistence.settings.last_sculpt_tool = kind;
                if self.persistence.settings.remember_sculpt_brush {
                    self.persistence.settings_persistence.mark_dirty();
                }
            }
            // Arming a brush means the Sculpt tab: show it and drop selection.
            self.tools.editor_tab = mesh_editor_overlay::EditorTab::Sculpt;
            self.document.mesh_selection_drag = None;
            // Prepare the target off the UI thread. Sculpt remains the owner of
            // the primary button while armed.
            self.prepare_armed_sculpt_session();
        } else if !self.tools.sculpt.worker_has_pending_work() {
            // The worker owns a queued Finish until the next poll.
            self.tools.sculpt.disarm();
        }
        self.scene_ui.status_message = Some(match self.tools.sculpt.armed {
            Some(SculptToolKind::AddRemove) if self.tools.sculpt.worker.is_some() => self
                .ui
                .locale
                .tr(crate::i18n::message_id!("sculpt-armed-addremove")),
            Some(SculptToolKind::Smooth) if self.tools.sculpt.worker.is_some() => self
                .ui
                .locale
                .tr(crate::i18n::message_id!("sculpt-armed-smooth")),
            Some(_) => self
                .ui
                .locale
                .tr(crate::i18n::message_id!("sculpt-preparing")),
            None => self.ui.locale.tr(crate::i18n::message_id!("sculpt-off")),
        });
        // Rebuild selection display data when Sculpt changes the visible mesh.
        self.render.invalidation.selection_changed();
        ctx.request_repaint();
    }

    /// Switch the editor tab: Sculpt arms a brush, Edit Mesh drops it.
    pub(in crate::app) fn switch_editor_tab(
        &mut self,
        tab: mesh_editor_overlay::EditorTab,
        ctx: &egui::Context,
    ) {
        use mesh_editor_overlay::EditorTab;
        if self.tools.editor_tab == tab {
            return;
        }
        self.tools.editor_tab = tab;
        match tab {
            EditorTab::EditMesh => {
                self.abort_sculpt_stroke();
                self.tools.sculpt.disarm();
            }
            EditorTab::Sculpt if self.tools.sculpt.armed.is_none() => {
                self.toggle_sculpt_tool(self.persistence.settings.last_sculpt_tool, ctx);
            }
            EditorTab::Sculpt => {}
        }
        self.render.invalidation.selection_changed();
        ctx.request_repaint();
    }

    /// In Mesh Editor, `1` opens Sculpt with Add/Remove and `2` with Smooth.
    /// Text fields retain digit keys.
    pub(in crate::app) fn handle_sculpt_hotkeys(&mut self, ctx: &egui::Context) -> bool {
        if !self.document.edit_mode.has_active_session() || ctx.egui_wants_keyboard_input() {
            return false;
        }
        if ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, egui::Key::Num1)
                // Do not let Shift change the meaning of the mode switch.
                || input.consume_key(egui::Modifiers::SHIFT, egui::Key::Num1)
        }) {
            self.arm_sculpt_tool(SculptToolKind::AddRemove, ctx);
            return true;
        }
        if ctx.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, egui::Key::Num2)
                || input.consume_key(egui::Modifiers::SHIFT, egui::Key::Num2)
        }) {
            self.arm_sculpt_tool(SculptToolKind::Smooth, ctx);
            return true;
        }
        false
    }

    /// Arm a sculpt tool idempotently — the hotkey only turns a tool on.
    fn arm_sculpt_tool(&mut self, kind: SculptToolKind, ctx: &egui::Context) {
        if self.tools.sculpt.armed != Some(kind) {
            self.toggle_sculpt_tool(kind, ctx);
        }
    }
}
