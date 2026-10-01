//! The root coordinates independent scene owners through explicit borrowed contexts.

use super::information_dialog::InformationDialog;
use super::state_document::DocumentState;
use super::state_persistence::PersistenceState;
use super::state_platform::{PlatformState, StartupHandles};
use super::state_render::RenderState;
use super::state_tool::ToolState;
use super::state_ui::{SceneUiState, UiState};
use super::workspace::commands::WorkspaceCommand;
use super::workspace::id::{IdAllocator, PaneId, SceneEpoch, SceneKey};
use super::workspace::loading::LoadCoordinator;
use super::workspace::state::{SceneSummary, WorkspaceState};
use super::{egui, home_camera_for_scene, CutTool, PathBuf};
use crate::live_viewport::SharedLiveViewport;
use std::collections::VecDeque;

fn ui_scale_zoom_is_allowed(ctx: &egui::Context) -> bool {
    !ctx.input(|input| input.pointer.any_down())
}

pub(super) fn restore_sculpt_preferences(
    ctx: &egui::Context,
    scene_key: SceneKey,
    settings: &mut crate::app_settings::Settings,
) {
    if !settings.remember_sculpt_brush {
        settings.last_sculpt_tool = crate::sculpt::sculpt_tool::SculptToolKind::default();
        settings.last_sculpt_tip = crate::sculpt::sculpt_tool::SculptTip::default();
        super::mesh_editor_overlay::set_sculpt_tip(ctx, scene_key, settings.last_sculpt_tip);
        return;
    }

    let tip = settings.last_sculpt_tip;
    let tip_index = match tip {
        crate::sculpt::sculpt_tool::SculptTip::Ball => 0,
        crate::sculpt::sculpt_tool::SculptTip::Knife => 1,
        crate::sculpt::sculpt_tool::SculptTip::Cylinder => 2,
    };
    crate::mesh_editor::mesh_editor_overlay::set_sculpt_tip(ctx, scene_key, tip);
    if let Some(share) = settings.sculpt_radius_share {
        crate::mesh_editor::mesh_editor_overlay::set_sculpt_radius_share(ctx, scene_key, share);
    } else {
        // Older settings only stored rounded millimetres. Seed the shared size
        // from the active tip's saved radius without passing through Ball.
        crate::mesh_editor::mesh_editor_overlay::set_sculpt_radius_mm(
            ctx,
            scene_key,
            tip,
            settings.sculpt_radii_mm[tip_index],
        );
    }
    for (kind, strength) in [
        crate::sculpt::sculpt_tool::SculptToolKind::AddRemove,
        crate::sculpt::sculpt_tool::SculptToolKind::Smooth,
    ]
    .into_iter()
    .zip(settings.sculpt_strengths)
    {
        super::mesh_editor_overlay::set_sculpt_strength(ctx, scene_key, kind, strength);
    }
}

pub(crate) struct OccluViewApp {
    pub(super) workspace: WorkspaceState,
    pub(super) loader: LoadCoordinator,
    pub(super) ui: UiState,
    pub(super) persistence: PersistenceState,
    pub(super) platform: PlatformState,
}

/// A single scene's operations borrow their exact owner and shared services.
/// No scene is installed into an active global slot to render or process work.
pub(crate) struct SceneContext<'a> {
    pub(super) scene_key: SceneKey,
    pub(super) lifetime_epoch: &'a mut SceneEpoch,
    pub(super) pane_id: PaneId,
    pub(super) preserve_on_transfer_undo: &'a mut bool,
    pub(super) document: &'a mut DocumentState,
    pub(super) render: &'a mut RenderState,
    pub(super) tools: &'a mut ToolState,
    pub(super) scene_ui: &'a mut SceneUiState,
    pub(super) ui: &'a mut UiState,
    pub(super) persistence: &'a mut PersistenceState,
    pub(super) platform: &'a mut PlatformState,
    pub(super) loader: &'a mut LoadCoordinator,
    pub(super) ids: &'a mut IdAllocator,
    pub(super) commands: &'a mut VecDeque<WorkspaceCommand>,
    pub(super) layer_drag: &'a mut Option<super::workspace::commands::LayerDragPayload>,
    pub(super) scene_tab_rects: &'a mut Vec<(SceneKey, egui::Rect)>,
    pub(super) scene_create_rect: &'a mut Option<egui::Rect>,
    pub(super) workspace_rect: Option<egui::Rect>,
    pub(super) saved_split: Option<super::workspace::layout::WorkspaceLayout>,
    pub(super) scene_summaries: Vec<SceneSummary>,
    pub(super) retained_scene_bytes: usize,
    pub(super) append_blocked_scene_keys: Vec<SceneKey>,
    pub(super) active_layer_count: usize,
    pub(super) is_active: bool,
    pub(super) input_allowed: bool,
}

impl OccluViewApp {
    pub(crate) fn new(
        repaint_ctx: egui::Context,
        startup_paths: Vec<PathBuf>,
        live_viewport: Option<SharedLiveViewport>,
        startup: StartupHandles,
    ) -> Self {
        repaint_ctx.options_mut(|options| options.zoom_with_keyboard = false);
        let state_dir = crate::desktop::app_paths::app_state_dir();
        let (locale, locale_snapshot) = crate::i18n::LocaleManager::startup(
            state_dir.as_deref(),
            &crate::i18n::os::SystemLocaleSource,
        );
        if let Some(diagnostic) = locale_snapshot.diagnostic {
            tracing::warn!(
                ?diagnostic,
                "language preference sidecar was unusable; using Auto/English"
            );
        }
        let first_name = locale.tr_with(
            crate::i18n::message_id!("workspace-scene-name"),
            &[("number", "1")],
        );
        let mut app = Self {
            workspace: WorkspaceState::new(live_viewport, first_name),
            loader: LoadCoordinator::default(),
            ui: UiState::new(repaint_ctx.clone(), locale),
            persistence: PersistenceState::new(),
            platform: PlatformState::new(repaint_ctx.clone(), startup),
        };
        restore_sculpt_preferences(
            &app.ui.repaint_ctx,
            app.workspace.scenes[0].key,
            &mut app.persistence.settings,
        );
        if !startup_paths.is_empty() {
            if let Some(mut scene) = app.active_context() {
                scene.replace_paths(&startup_paths, "startup");
            }
        }
        app
    }

    pub(super) fn scene_context(&mut self, key: SceneKey) -> Option<SceneContext<'_>> {
        let summaries = self.workspace.summaries();
        let append_blocked_scene_keys = self
            .workspace
            .scenes
            .iter()
            .filter(|scene| {
                scene.document.edit_mode.has_active_session()
                    || scene.document.edit_mode.is_busy()
                    || scene.document.unsaved_sculpt_stroke
            })
            .map(|scene| scene.key)
            .collect();
        let retained_scene_bytes = self.workspace.scenes.iter().fold(
            self.workspace.history.borrow().used_bytes(),
            |bytes, scene| {
                bytes.saturating_add(scene.document.scene.as_deref().map_or(0, |scene| {
                    usize::try_from(scene.estimated_memory_bytes()).unwrap_or(usize::MAX)
                }))
            },
        );
        let active_layer_count = self
            .workspace
            .scene(self.workspace.active_id())
            .and_then(|scene| scene.document.scene.as_ref())
            .map_or(0, |scene| scene.meshes().len());
        let is_active = self.workspace.active_id() == key.id;
        let window_focused = self.ui.repaint_ctx.input(|input| input.focused);
        let input_allowed = window_focused
            && is_active
            && match self.workspace.input.route_pointer(None) {
                super::workspace::input::PointerRoute::Target(target) => target.scene == key,
                super::workspace::input::PointerRoute::Captured(owner) => {
                    owner.target.scene == key && owner.kind.allows_viewport_input()
                }
                super::workspace::input::PointerRoute::Suppressed => false,
            };
        let scene = self
            .workspace
            .scenes
            .iter_mut()
            .find(|scene| scene.key == key)?;
        Some(SceneContext {
            scene_key: scene.key,
            lifetime_epoch: &mut scene.key.epoch,
            pane_id: scene.pane,
            preserve_on_transfer_undo: &mut scene.preserve_on_transfer_undo,
            document: &mut scene.document,
            render: &mut scene.render,
            tools: &mut scene.tools,
            scene_ui: &mut scene.presentation,
            ui: &mut self.ui,
            persistence: &mut self.persistence,
            platform: &mut self.platform,
            loader: &mut self.loader,
            ids: &mut self.workspace.ids,
            commands: &mut self.workspace.commands,
            layer_drag: &mut self.workspace.layer_drag,
            scene_tab_rects: &mut self.workspace.scene_tab_rects,
            scene_create_rect: &mut self.workspace.scene_create_rect,
            workspace_rect: None,
            saved_split: self.workspace.saved_split,
            scene_summaries: summaries,
            retained_scene_bytes,
            append_blocked_scene_keys,
            active_layer_count,
            is_active,
            input_allowed,
        })
    }

    pub(super) fn active_context(&mut self) -> Option<SceneContext<'_>> {
        let key = self.workspace.scene(self.workspace.active_id())?.key;
        self.scene_context(key)
    }

    #[cfg(test)]
    pub(crate) fn new_for_tests(repaint_ctx: egui::Context) -> Self {
        Self {
            workspace: WorkspaceState::new(None, "Scene 1".to_owned()),
            loader: LoadCoordinator::default(),
            ui: UiState::new(repaint_ctx.clone(), crate::i18n::LocaleManager::for_tests()),
            persistence: PersistenceState::for_tests(),
            platform: PlatformState::for_tests(repaint_ctx),
        }
    }
}

impl SceneContext<'_> {
    /// Edit hotkeys, refused while a dialog is up.
    ///
    /// The callee is named `_unguarded` rather than `_impl` because it skips
    /// the dialog check: a direct call from a neighbouring module could delete
    /// faces or replay an undo while the unsaved-changes prompt is open,
    /// changing what "Save" then writes. The name makes such a call a visible
    /// choice.
    pub(super) fn handle_edit_shortcuts(&mut self, ctx: &egui::Context) {
        // The bridge tool owns the scene while it is armed, which is not a
        // dialog and so is not part of the shared predicate.
        if !self.input_allowed
            || self.ui.modal_dialog_open()
            || ctx.egui_wants_keyboard_input()
            || self.tools.bridge_split_active()
        {
            return;
        }
        self.handle_edit_shortcuts_unguarded(ctx);
    }

    pub(super) fn reset_camera_to_home(&mut self) {
        let Some(scene) = self.document.scene.as_ref() else {
            self.render.camera = None;
            return;
        };
        self.render.camera = Some(home_camera_for_scene(scene));
        self.render.invalidation.request_redraw();
    }

    pub(super) fn request_camera_repaint(&mut self, ctx: &egui::Context) {
        self.render.invalidation.request_redraw();
        *self.preserve_on_transfer_undo = true;
        if self.loader.has_work_for(self.scene_key) {
            self.document.camera_modified_during_load = true;
        }
        ctx.request_repaint();
    }

    pub(super) fn can_render_cut_view(&self) -> bool {
        self.document
            .scene
            .as_ref()
            .is_some_and(|scene| CutTool::can_render_bbox(scene.bbox()))
    }

    /// Whether any layer can take a measurement pick (visible triangles).
    pub(super) fn has_measurable_layer(&self) -> bool {
        self.document.scene.as_ref().is_some_and(|scene| {
            scene
                .meshes()
                .iter()
                .any(|entry| entry.visible && !entry.mesh.is_point_cloud())
        })
    }

    /// Render the information route that was active at the start of this UI
    /// pass. A selection inside About therefore replaces it on the following
    /// frame instead of briefly stacking two modal backdrops.
    pub(super) fn show_information_dialog(&mut self, ctx: &egui::Context) {
        if self.ui.foreground_dialog_open() {
            return;
        }
        match self.ui.information_dialog {
            InformationDialog::None => {}
            InformationDialog::About => self.show_about_dialog(ctx),
            InformationDialog::ThirdPartyNotices => self.show_third_party_window(ctx),
            InformationDialog::KeyboardMouse => self.show_help_dialog(ctx),
        }
    }
}

impl eframe::App for OccluViewApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.workspace.retired_frame_textures.clear();
        self.ui.sync_native_title(ctx);
        if let Some(active) = self.workspace.scene(self.workspace.active_id()) {
            self.persistence.sync_sculpt_preferences(ctx, active.key);
        }
        self.persistence.persist_settings_if_due(ctx);
        let preference = self.ui.locale.snapshot().preference.clone();
        self.persistence.persist_language_if_due(ctx, &preference);
        SceneContext::schedule_linux_open_request_repaint(ctx);
        self.loader.reap_retired(&self.workspace.keys());
        for key in self.workspace.keys() {
            if let Some(mut scene) = self.scene_context(key) {
                scene.scene_ui.expire_status_message(ctx);
                scene.process_scene_loads(ctx);
                scene.poll_sculpt_preparation(ctx);
                scene.poll_sculpt_worker(ctx);
                scene.settle_sculpt_work_marker();
                scene.drain_align_worker(ctx);
                scene.drain_contacts_worker(ctx);
                scene.poll_bridge_split_worker(ctx);
            }
        }
        if let Some(active) = self.workspace.scene(self.workspace.active_id()) {
            if self.workspace.input.active().scene != active.key {
                self.workspace.input = super::workspace::input::InputArbiter::new(active.target());
            }
        }
        if let Some(mut scene) = self.active_context() {
            scene.handle_open_requests(ctx);
            scene.finish_foreground_pulse_if_due(ctx);
        }
        self.persistence.update_notice.poll(ctx);
        self.intercept_workspace_close(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        crate::ui::ui_theme::set_active(self.persistence.settings.theme);
        ctx.set_visuals(super::viewer_visuals(self.persistence.settings.theme));
        let target_ui_scale = self.persistence.settings.ui_scale();
        if ui_scale_zoom_is_allowed(&ctx) && (ctx.zoom_factor() - target_ui_scale).abs() > 1e-3 {
            ctx.set_zoom_factor(target_ui_scale);
        }
        self.ui.popup_open_at_frame_start = egui::Popup::is_any_open(&ctx);
        self.ui.scene_report_open = self
            .workspace
            .scenes
            .iter()
            .any(|scene| scene.presentation.repair_report.is_open());
        if let Some(mut scene) = self.active_context() {
            scene.release_viewport_orbit_cursor_if_inactive(&ctx);
            scene.show_toolbar(ui);
        }
        self.show_workspace(ui);
        self.apply_workspace_commands(&ctx);
        for key in self.visible_scene_keys(&ctx) {
            if let Some(mut scene) = self.scene_context(key) {
                scene.render_pending_frame(&ctx);
                if scene.poll_gpu_errors() {
                    ctx.request_repaint();
                }
            }
        }
        for key in self.workspace.keys() {
            if let Some(mut scene) = self.scene_context(key) {
                if scene.scene_ui.open_dialog_requested {
                    scene.scene_ui.open_dialog_requested = false;
                    scene.open_files_dialog();
                }
            }
        }
        if let Some(mut scene) = self.active_context() {
            scene.show_error_dialog(&ctx);
            scene.show_information_dialog(&ctx);
        }
        for scene in &mut self.workspace.scenes {
            scene.presentation.repair_report.ui(&ctx, &self.ui.locale);
        }
        self.persistence.update_notice.show(&ctx, &self.ui.locale);
        self.show_workspace_close_guard(&ctx);
        if let Some(key) = self
            .ui
            .pending_replace_open
            .as_ref()
            .map(|pending| pending.scene_key)
        {
            if let Some(mut scene) = self.scene_context(key) {
                scene.guard_pending_replace_open(&ctx);
            } else {
                self.ui.pending_replace_open = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "Persisted catalog values are exact slider values."
    )]
    fn sculpt_preferences_restore_selected_tip_radius_and_tool_values() {
        let ctx = egui::Context::default();
        let mut settings = crate::app_settings::Settings::default();
        settings.last_sculpt_tool = crate::sculpt::sculpt_tool::SculptToolKind::Smooth;
        settings.last_sculpt_tip = crate::sculpt::sculpt_tool::SculptTip::Knife;
        settings.sculpt_radii_mm = [0.75, 1.0, 0.7];
        settings.sculpt_radius_share = Some(0.155_555_56);
        settings.sculpt_strengths = [0.4, 0.3];

        restore_sculpt_preferences(&ctx, SceneKey::INITIAL, &mut settings);

        assert_eq!(
            crate::mesh_editor::mesh_editor_overlay::sculpt_tip(&ctx, SceneKey::INITIAL),
            crate::sculpt::sculpt_tool::SculptTip::Knife
        );
        assert_eq!(
            crate::mesh_editor::mesh_editor_overlay::sculpt_radius_mm(
                &ctx,
                SceneKey::INITIAL,
                crate::sculpt::sculpt_tool::SculptTip::Knife
            ),
            0.6,
            "the exact normalized share takes precedence over the rounded snapshots"
        );
        assert_eq!(
            crate::mesh_editor::mesh_editor_overlay::sculpt_radius_mm(
                &ctx,
                SceneKey::INITIAL,
                crate::sculpt::sculpt_tool::SculptTip::Ball
            ),
            0.85
        );
        assert_eq!(
            crate::mesh_editor::mesh_editor_overlay::sculpt_strength(
                &ctx,
                SceneKey::INITIAL,
                crate::sculpt::sculpt_tool::SculptToolKind::Smooth
            ),
            0.3
        );

        let legacy_ctx = egui::Context::default();
        let mut legacy_settings = crate::app_settings::Settings::default();
        legacy_settings.last_sculpt_tip = crate::sculpt::sculpt_tool::SculptTip::Knife;
        legacy_settings.sculpt_radii_mm = [0.75, 0.6, 0.5];
        restore_sculpt_preferences(&legacy_ctx, SceneKey::INITIAL, &mut legacy_settings);
        assert_eq!(
            crate::mesh_editor::mesh_editor_overlay::sculpt_radius_mm(
                &legacy_ctx,
                SceneKey::INITIAL,
                crate::sculpt::sculpt_tool::SculptTip::Knife
            ),
            0.6,
            "legacy millimetres seed the share through the remembered tip"
        );

        settings.remember_sculpt_brush = false;
        restore_sculpt_preferences(&ctx, SceneKey::INITIAL, &mut settings);
        assert_eq!(
            settings.last_sculpt_tool,
            crate::sculpt::sculpt_tool::SculptToolKind::AddRemove
        );
        assert_eq!(
            crate::mesh_editor::mesh_editor_overlay::sculpt_tip(&ctx, SceneKey::INITIAL),
            crate::sculpt::sculpt_tool::SculptTip::Ball
        );
    }
}
