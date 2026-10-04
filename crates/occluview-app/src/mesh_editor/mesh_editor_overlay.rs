//! The mesh editor tool window (the dental CAD "3D Data Editor" workflow).
//!
//! A movable `egui::Window` with a custom top bar of two tabs — Edit Mesh
//! (selection / repair) and Sculpt (the brushes) — over a shared status +
//! commit bar. Presentation only: every button maps to one [`MeshEditorAction`]
//! the viewport applies.
//!
//! The per-section rendering lives in the sibling [`groups`] module (declared
//! below with an explicit path); this file owns only the window shell and the
//! action vocabulary.

use eframe::egui;

use crate::app::workspace::id::SceneKey;
use crate::ui::icons::AppIcon;

use crate::sculpt::sculpt_tool::{SculptTip, SculptToolKind};

#[path = "mesh_editor_groups.rs"]
mod groups;
#[path = "mesh_editor_session.rs"]
mod session_bar;

/// The two tabs of the editor window: selection/repair tools, or the sculpt
/// brushes. Exactly one is shown at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum EditorTab {
    #[default]
    EditMesh,
    Sculpt,
}

/// Actions the mesh editor window can request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MeshEditorAction {
    /// Switch the active tab.
    SwitchTab(EditorTab),
    SelectAll,
    InvertSelection,
    ClearSelection,
    /// Arm/disarm the freehand lasso capture (the dental CAD "Edit Mesh" lasso).
    ToggleLasso,
    /// Arm/disarm Object pick: click one whole object of a multi-object STL.
    ToggleObject,
    /// Switch between surface (front-facing) and through-mesh selection.
    ToggleThroughMesh,
    /// Arm/disarm one interactive sculpt tool (the dental CAD Freeforming
    /// workflow: the Add/Remove clay knife or the Smooth relaxer), dragged on
    /// the surface.
    ToggleSculpt(SculptToolKind),
    /// Confirm the edit session: keep edits, close the window.
    Done,
    /// Revert the whole edit session to the captured baseline.
    Cancel,
    Delete,
    Crop,
    Cut,
    Separate,
    CloseHoles,
    Undo,
    Redo,
}

/// Snapshot of the editor state the window renders from. Kept as a struct so
/// the viewport reads each field once (borrow discipline) and the signature
/// stays stable as the window gains richer state. Each bool is an independent
/// flag (tool mode + session phase), not a bitfield of one concept.
#[expect(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MeshEditorPanelState {
    /// Total selected faces across visible editable layers.
    pub(crate) selected_face_count: usize,
    /// Whether an edit operation can be undone right now.
    pub(crate) can_undo: bool,
    /// Whether an undone edit operation can be re-applied right now.
    pub(crate) can_redo: bool,
    /// Whether the freehand lasso owns the primary viewport drag.
    pub(crate) lasso_armed: bool,
    /// Whether Object pick is armed (a click selects a whole component).
    pub(crate) object_mode: bool,
    /// Lasso mode: false = surface/front-facing, true = through-mesh.
    pub(crate) through_mesh: bool,
    /// The armed sculpt tool, if any (owns the primary drag when set).
    pub(crate) sculpt_armed: Option<SculptToolKind>,
    /// Whether the session carries uncommitted edits (Done is meaningful).
    pub(crate) dirty: bool,
    /// Whether a mesh operation is running (all mutating buttons disabled).
    pub(crate) busy: bool,
    /// Whether Sculpt has an in-flight drag or worker output. Structural mesh
    /// operations must wait for it, while Done/Cancel remain available.
    pub(crate) sculpt_pending: bool,
    /// Which tab is showing.
    pub(crate) active_tab: EditorTab,
}

const WINDOW_WIDTH_MIN: f32 = 200.0;
const WINDOW_WIDTH_MAX: f32 = 320.0;

fn window_width(viewport: egui::Rect) -> f32 {
    let max_width = (viewport.width() - 28.0).max(180.0);
    (viewport.width() * 0.22)
        .clamp(WINDOW_WIDTH_MIN, WINDOW_WIDTH_MAX)
        .min(max_width)
}

/// Where the window may sit: the viewport without the bottom strip that
/// holds the status row and the scale bar. The window opens at the bottom
/// left, and every refusal of its own buttons is written to that row, so a
/// window over it would hide the answer to the click it just took.
fn window_bounds(viewport: egui::Rect) -> egui::Rect {
    let floor = crate::ui::app_chrome::status_overlay_rect(viewport).top() - 6.0;
    egui::Rect::from_min_max(
        viewport.min,
        egui::pos2(viewport.max.x, floor.max(viewport.min.y)),
    )
}

fn default_pos(viewport: egui::Rect) -> egui::Pos2 {
    let width = window_width(viewport);
    let estimated_height = 380.0;
    let x = viewport.min.x + 16.0;
    let y = (window_bounds(viewport).max.y - estimated_height).max(viewport.min.y + 16.0);
    egui::pos2(x.min(viewport.max.x - width - 16.0), y)
}

/// Default and bounds for the optional Close Holes rim-perimeter restraint.
/// It is off by default: the kernel preserves scan borders and repairs every
/// safe interior hole, matching the normal dental workflow.
pub(crate) const CLOSE_HOLES_LIMIT_DEFAULT_MM: f32 = 15.0;
pub(crate) const CLOSE_HOLES_LIMIT_MIN_MM: f32 = 1.0;
pub(crate) const CLOSE_HOLES_LIMIT_MAX_MM: f32 = 100.0;

fn scoped_id(label: &'static str, scene_key: SceneKey) -> egui::Id {
    egui::Id::new((label, scene_key))
}

pub(crate) fn close_holes_limit_id(scene_key: SceneKey) -> egui::Id {
    scoped_id("occluview_close_holes_limit_mm", scene_key)
}

fn close_holes_limit_enabled_id(scene_key: SceneKey) -> egui::Id {
    scoped_id("occluview_close_holes_limit_enabled", scene_key)
}

/// Optional maximum rim perimeter for Close Holes. The value lives in egui
/// memory so it remains stable while the editor is open without becoming a
/// global application preference.
pub(crate) fn close_holes_limit_mm(ctx: &egui::Context, scene_key: SceneKey) -> Option<f32> {
    let enabled = ctx
        .data(|data| data.get_temp::<bool>(close_holes_limit_enabled_id(scene_key)))
        .unwrap_or(false);
    enabled.then(|| {
        ctx.data(|data| data.get_temp::<f32>(close_holes_limit_id(scene_key)))
            .unwrap_or(CLOSE_HOLES_LIMIT_DEFAULT_MM)
    })
}

pub(crate) fn set_close_holes_limit_enabled(
    ctx: &egui::Context,
    scene_key: SceneKey,
    enabled: bool,
) {
    ctx.data_mut(|data| data.insert_temp(close_holes_limit_enabled_id(scene_key), enabled));
}

pub(crate) fn close_holes_limit_enabled(ctx: &egui::Context, scene_key: SceneKey) -> bool {
    ctx.data(|data| data.get_temp::<bool>(close_holes_limit_enabled_id(scene_key)))
        .unwrap_or(false)
}

fn sculpt_tip_id(scene_key: SceneKey) -> egui::Id {
    scoped_id("occluview_sculpt_tip", scene_key)
}

/// The brush tip the Sculpt panel is offering.
pub(crate) fn sculpt_tip(ctx: &egui::Context, scene_key: SceneKey) -> SculptTip {
    ctx.data(|data| data.get_temp::<SculptTip>(scoped_id("occluview_sculpt_tip", scene_key)))
        .unwrap_or_default()
}

pub(crate) fn set_sculpt_tip(ctx: &egui::Context, scene_key: SceneKey, tip: SculptTip) {
    ctx.data_mut(|data| data.insert_temp(sculpt_tip_id(scene_key), tip));
}

fn sculpt_radius_share_id(scene_key: SceneKey) -> egui::Id {
    scoped_id("occluview_sculpt_radius_share", scene_key)
}

fn sculpt_strength_id(scene_key: SceneKey, kind: SculptToolKind) -> egui::Id {
    egui::Id::new(("occluview_sculpt_strength", scene_key, kind))
}

fn radius_share(tip: SculptTip, radius_mm: f32) -> f32 {
    let (min, max) = tip.radius_range_mm();
    ((radius_mm.clamp(min, max) - min) / (max - min)).clamp(0.0, 1.0)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "Catalog arithmetic uses decimal f64; the UI stores the nearest representable f32 radius."
)]
fn radius_for_share(tip: SculptTip, share: f32) -> f32 {
    let (min, max) = tip.radius_range_mm();
    let catalog_decimal = |value: f32| (f64::from(value) * 1_000_000.0).round() / 1_000_000.0;
    let min = catalog_decimal(min);
    let max = catalog_decimal(max);
    let step = catalog_decimal(SculptTip::radius_step_mm());
    let share = f64::from(share.clamp(0.0, 1.0));
    let raw = min + share * (max - min);
    let steps = ((raw - min) / step).round();
    (min + steps * step).clamp(min, max) as f32
}

/// Normalized size shared by the catalog tips and persisted without converting
/// through another tip's coarser physical step grid.
pub(crate) fn sculpt_radius_share(ctx: &egui::Context, scene_key: SceneKey) -> f32 {
    ctx.data(|data| data.get_temp::<f32>(sculpt_radius_share_id(scene_key)))
        .filter(|share| share.is_finite())
        .map_or_else(
            || radius_share(SculptTip::Ball, SculptTip::Ball.default_radius_mm()),
            |share| share.clamp(0.0, 1.0),
        )
}

pub(crate) fn set_sculpt_radius_share(ctx: &egui::Context, scene_key: SceneKey, share: f32) {
    let share = if share.is_finite() {
        share.clamp(0.0, 1.0)
    } else {
        radius_share(SculptTip::Ball, SculptTip::Ball.default_radius_mm())
    };
    ctx.data_mut(|data| data.insert_temp(sculpt_radius_share_id(scene_key), share));
}

/// The radius is a normalized share of the selected tip's range. Switching
/// tips preserves the operator's size choice rather than reusing millimetres.
pub(crate) fn sculpt_radius_mm(ctx: &egui::Context, scene_key: SceneKey, tip: SculptTip) -> f32 {
    radius_for_share(tip, sculpt_radius_share(ctx, scene_key))
}

pub(crate) fn set_sculpt_radius_mm(
    ctx: &egui::Context,
    scene_key: SceneKey,
    tip: SculptTip,
    radius_mm: f32,
) {
    set_sculpt_radius_share(ctx, scene_key, radius_share(tip, radius_mm));
}

/// Per-mode kernel strength, with the donor's Add and Smooth defaults.
pub(crate) fn sculpt_strength(
    ctx: &egui::Context,
    scene_key: SceneKey,
    kind: SculptToolKind,
) -> f32 {
    let (min, max) = kind.strength_range();
    ctx.data(|data| data.get_temp::<f32>(sculpt_strength_id(scene_key, kind)))
        .filter(|strength| strength.is_finite())
        .unwrap_or_else(|| kind.default_strength())
        .clamp(min, max)
}

pub(crate) fn set_sculpt_strength(
    ctx: &egui::Context,
    scene_key: SceneKey,
    kind: SculptToolKind,
    strength: f32,
) {
    let (min, max) = kind.strength_range();
    let strength = if strength.is_finite() {
        strength.clamp(min, max)
    } else {
        kind.default_strength()
    };
    ctx.data_mut(|data| {
        data.insert_temp(sculpt_strength_id(scene_key, kind), strength);
    });
}

/// Show the movable mesh editor window; returns the requested action, if any.
pub(crate) fn show(
    ctx: &egui::Context,
    scene_key: SceneKey,
    viewport_rect: egui::Rect,
    state: MeshEditorPanelState,
    locale: &crate::i18n::LocaleManager,
) -> Option<MeshEditorAction> {
    let width = window_width(viewport_rect);
    let mut action = None;
    egui::Window::new(locale.text(crate::i18n::message_id!("meshedit-window-title")))
        .id(scoped_id("occluview_mesh_editor_window", scene_key))
        .default_pos(default_pos(viewport_rect))
        .constrain_to(window_bounds(viewport_rect))
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        .show(ctx, |ui| {
            ui.set_min_width(width - 24.0);
            ui.set_width(width - 24.0);
            action = window_action(ui, scene_key, state, locale);
        });
    action
}

/// Assemble the window body: the tab strip, then the active tab's tools, then
/// the shared status + commit bar. Every section renders in [`groups`]; this
/// function fixes the shared spacing and chains the optional actions.
fn window_action(
    ui: &mut egui::Ui,
    scene_key: SceneKey,
    state: MeshEditorPanelState,
    locale: &crate::i18n::LocaleManager,
) -> Option<MeshEditorAction> {
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 3.0);
    // Snappier hover/press for this dense tool palette than the global chrome.
    ui.style_mut().animation_time = 0.05;
    // While a mesh operation or Sculpt write runs, structural operations are
    // disabled. The session bar receives the narrower edit-mode flag below so
    // Done/Cancel can still resolve or abort the pending sculpt.
    let ops_enabled = !state.busy && !state.sculpt_pending;

    groups::header(
        ui,
        &locale.tr(crate::i18n::message_id!("meshedit-header-edit")),
        AppIcon::EditMesh,
    );
    let mut action = groups::tab_strip(ui, &state, locale);
    ui.add_space(4.0);
    match state.active_tab {
        EditorTab::EditMesh => {
            action = action.or(groups::selection(ui, &state, ops_enabled, locale));
            action = action.or(groups::edit_selection(ui, &state, ops_enabled, locale));
            action = action.or(groups::close_holes(
                ui,
                scene_key,
                &state,
                ops_enabled,
                locale,
            ));
        }
        EditorTab::Sculpt => {
            action = action.or(groups::sculpt(ui, scene_key, &state, ops_enabled, locale));
        }
    }
    session_bar::status(ui, &state, locale);
    action = action.or(session_bar::session(ui, &state, !state.busy, locale));
    action
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::{
        close_holes_limit_id, close_holes_limit_mm, scoped_id, sculpt_radius_mm, sculpt_strength,
        sculpt_tip, set_close_holes_limit_enabled, set_sculpt_radius_mm, set_sculpt_strength,
        set_sculpt_tip, CLOSE_HOLES_LIMIT_DEFAULT_MM,
    };
    use crate::app::workspace::id::SceneKey;
    use crate::sculpt::sculpt_tool::{SculptTip, SculptToolKind};
    use eframe::egui;

    fn panel_frame(
        ctx: &egui::Context,
        state: &super::MeshEditorPanelState,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<super::MeshEditorAction>) {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 1024.0));
        let mut action = None;
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(viewport),
                events,
                ..Default::default()
            },
            |ui| {
                ui.allocate_rect(viewport, egui::Sense::click_and_drag());
                action = super::show(
                    ui.ctx(),
                    SceneKey::INITIAL,
                    viewport,
                    state.clone(),
                    &crate::i18n::LocaleManager::for_tests(),
                );
            },
        );
        (output, action)
    }

    fn control_bounds(
        output: &egui::FullOutput,
        label: &str,
        role: egui::accesskit::Role,
    ) -> egui::Rect {
        let update = output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("control tree");
        let bounds = update
            .nodes
            .iter()
            .find_map(|(_, node)| {
                let name = node.label().map_or_else(
                    || {
                        node.labelled_by()
                            .iter()
                            .filter_map(|id| {
                                update
                                    .nodes
                                    .iter()
                                    .find(|(candidate, _)| candidate == id)
                                    .and_then(|(_, label)| label.value())
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    },
                    str::to_owned,
                );
                (node.role() == role && name == label)
                    .then(|| node.bounds())
                    .flatten()
            })
            .expect("named control bounds");
        let min = glam::DVec2::new(bounds.x0, bounds.y0).as_vec2();
        let max = glam::DVec2::new(bounds.x1, bounds.y1).as_vec2();
        egui::Rect::from_min_max(egui::pos2(min.x, min.y), egui::pos2(max.x, max.y))
    }

    fn click_panel_control(
        state: &super::MeshEditorPanelState,
        label: &str,
    ) -> Option<super::MeshEditorAction> {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        click_live_control(&ctx, state, label, egui::accesskit::Role::Button)
    }

    fn click_live_control(
        ctx: &egui::Context,
        state: &super::MeshEditorPanelState,
        label: &str,
        role: egui::accesskit::Role,
    ) -> Option<super::MeshEditorAction> {
        for _ in 0..2 {
            panel_frame(ctx, state, vec![])
                .0
                .drop_without_applying_deltas();
        }
        let (output, _) = panel_frame(ctx, state, vec![]);
        let point = control_bounds(&output, label, role).center();
        output.drop_without_applying_deltas();
        let mut action = None;
        for pressed in [true, false] {
            let (output, emitted) = panel_frame(
                ctx,
                state,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            output.drop_without_applying_deltas();
            action = action.or(emitted);
        }
        action
    }

    #[test]
    fn close_holes_requires_a_visible_face_selection() {
        assert_eq!(
            click_panel_control(&super::MeshEditorPanelState::default(), "Close holes"),
            None,
            "a repair needing selected rim faces must be disabled without marks",
        );
        assert_eq!(
            click_panel_control(
                &super::MeshEditorPanelState {
                    selected_face_count: 1,
                    ..Default::default()
                },
                "Close holes"
            ),
            Some(super::MeshEditorAction::CloseHoles),
        );
    }

    /// The window opens at the bottom left, which is where the status row is.
    /// It has to stop above that row: the row is where the window's own
    /// buttons say why they refused.
    #[test]
    fn the_window_opens_clear_of_the_status_row() {
        let ctx = egui::Context::default();
        let state = super::MeshEditorPanelState::default();
        for _ in 0..3 {
            let (mut output, _) = panel_frame(&ctx, &state, Vec::new());
            output.textures_delta.clear();
        }
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 1024.0));
        let window = ctx
            .memory(|memory| {
                memory.area_rect(scoped_id("occluview_mesh_editor_window", SceneKey::INITIAL))
            })
            .expect("the window was laid out");
        let status = crate::ui::app_chrome::status_overlay_rect(viewport);
        assert!(
            window.bottom() <= status.top(),
            "the window ({window:?}) covers the status row ({status:?})"
        );
    }

    #[test]
    fn edit_mesh_buttons_emit_their_actions_on_pointer_clicks() {
        use super::{EditorTab, MeshEditorAction, MeshEditorPanelState};
        let state = MeshEditorPanelState {
            selected_face_count: 1,
            can_undo: true,
            can_redo: true,
            through_mesh: true,
            ..Default::default()
        };
        for (label, expected) in [
            (
                "Mesh Editing",
                MeshEditorAction::SwitchTab(EditorTab::EditMesh),
            ),
            ("Sculpt", MeshEditorAction::SwitchTab(EditorTab::Sculpt)),
            (
                "Cancel the session (edits are reverted)",
                MeshEditorAction::Cancel,
            ),
            ("Lasso", MeshEditorAction::ToggleLasso),
            ("Object", MeshEditorAction::ToggleObject),
            ("Surface", MeshEditorAction::ToggleThroughMesh),
            ("All", MeshEditorAction::SelectAll),
            ("None", MeshEditorAction::ClearSelection),
            ("Invert", MeshEditorAction::InvertSelection),
            ("Delete", MeshEditorAction::Delete),
            ("Crop", MeshEditorAction::Crop),
            ("Cut", MeshEditorAction::Cut),
            ("Separate", MeshEditorAction::Separate),
            ("Close holes", MeshEditorAction::CloseHoles),
            ("Undo", MeshEditorAction::Undo),
            ("Redo", MeshEditorAction::Redo),
            ("Cancel", MeshEditorAction::Cancel),
            ("Done", MeshEditorAction::Done),
        ] {
            assert_eq!(
                click_panel_control(&state, label),
                Some(expected),
                "{label}"
            );
        }
        assert_eq!(click_panel_control(&state, "Through"), None);
        let surface_state = MeshEditorPanelState {
            through_mesh: false,
            ..state
        };
        assert_eq!(
            click_panel_control(&surface_state, "Through"),
            Some(MeshEditorAction::ToggleThroughMesh)
        );
        assert_eq!(click_panel_control(&surface_state, "Surface"), None);
    }

    #[test]
    fn edit_mesh_controls_respect_busy_and_sculpt_pending_gates() {
        use super::{MeshEditorAction, MeshEditorPanelState};
        for state in [
            MeshEditorPanelState {
                selected_face_count: 1,
                can_undo: true,
                can_redo: true,
                busy: true,
                ..Default::default()
            },
            MeshEditorPanelState {
                selected_face_count: 1,
                can_undo: true,
                can_redo: true,
                sculpt_pending: true,
                ..Default::default()
            },
        ] {
            for label in [
                "Lasso",
                "Object",
                "Surface",
                "Through",
                "All",
                "None",
                "Invert",
                "Delete",
                "Crop",
                "Cut",
                "Separate",
                "Close holes",
                "Undo",
                "Redo",
            ] {
                assert_eq!(
                    click_panel_control(&state, label),
                    None,
                    "{label}, {state:?}"
                );
            }
            for (label, action) in [
                ("Done", MeshEditorAction::Done),
                ("Cancel", MeshEditorAction::Cancel),
            ] {
                let expected = (!state.busy).then_some(action);
                assert_eq!(
                    click_panel_control(&state, label),
                    expected,
                    "{label}, {state:?}"
                );
            }
        }
        let state = MeshEditorPanelState::default();
        for label in ["Delete", "Crop", "Cut", "Separate", "Undo", "Redo"] {
            assert_eq!(
                click_panel_control(&state, label),
                None,
                "{label} without selection/history"
            );
        }
        let state = MeshEditorPanelState {
            object_mode: true,
            ..Default::default()
        };
        for label in ["Surface", "Through"] {
            assert_eq!(
                click_panel_control(&state, label),
                None,
                "{label} in Object mode"
            );
        }
    }

    #[test]
    fn sculpt_buttons_emit_their_actions_on_pointer_clicks() {
        use super::{EditorTab, MeshEditorAction, MeshEditorPanelState};
        let state = MeshEditorPanelState {
            active_tab: EditorTab::Sculpt,
            ..Default::default()
        };
        for (label, kind) in [
            ("Add / Remove  [1]", SculptToolKind::AddRemove),
            ("Smooth  [2]", SculptToolKind::Smooth),
        ] {
            assert_eq!(
                click_panel_control(&state, label),
                Some(MeshEditorAction::ToggleSculpt(kind))
            );
        }
    }

    #[test]
    fn sculpt_tip_clicks_change_live_settings_only_when_enabled() {
        for pending in [false, true] {
            for tip in SculptTip::ALL {
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                let previous = if tip == SculptTip::Ball {
                    SculptTip::Knife
                } else {
                    SculptTip::Ball
                };
                set_sculpt_tip(&ctx, SceneKey::INITIAL, previous);
                let state = super::MeshEditorPanelState {
                    active_tab: super::EditorTab::Sculpt,
                    sculpt_pending: pending,
                    ..Default::default()
                };
                let label = crate::i18n::LocaleManager::for_tests().tr(tip.label_key());
                assert_eq!(
                    click_live_control(&ctx, &state, &label, egui::accesskit::Role::Button),
                    None
                );
                assert_eq!(
                    sculpt_tip(&ctx, SceneKey::INITIAL),
                    if pending { previous } else { tip }
                );
            }
        }
    }

    #[test]
    fn close_holes_limit_clicks_toggle_the_live_option_only_when_enabled() {
        for busy in [false, true] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let state = super::MeshEditorPanelState {
                busy,
                ..Default::default()
            };
            assert_eq!(
                click_live_control(&ctx, &state, "limit", egui::accesskit::Role::CheckBox),
                None
            );
            assert_eq!(
                close_holes_limit_mm(&ctx, SceneKey::INITIAL),
                (!busy).then_some(CLOSE_HOLES_LIMIT_DEFAULT_MM)
            );
            assert_eq!(
                click_live_control(&ctx, &state, "limit", egui::accesskit::Role::CheckBox),
                None
            );
            assert_eq!(close_holes_limit_mm(&ctx, SceneKey::INITIAL), None);
        }
    }

    #[test]
    fn sculpt_panel_rendering_preserves_normalized_brush_size() {
        let ctx = egui::Context::default();
        let state = super::MeshEditorPanelState {
            active_tab: super::EditorTab::Sculpt,
            ..Default::default()
        };
        let share = 0.37;
        super::set_sculpt_radius_share(&ctx, SceneKey::INITIAL, share);
        for tip in SculptTip::ALL {
            set_sculpt_tip(&ctx, SceneKey::INITIAL, tip);
            panel_frame(&ctx, &state, vec![])
                .0
                .drop_without_applying_deltas();
            assert_eq!(
                super::sculpt_radius_share(&ctx, SceneKey::INITIAL),
                share,
                "rendering {tip:?} must preserve the user's size share"
            );
        }
    }

    #[test]
    fn sculpt_sliders_update_live_settings_only_on_enabled_pointer_input() {
        for pending in [false, true] {
            for label in ["size", "force"] {
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                let state = super::MeshEditorPanelState {
                    active_tab: super::EditorTab::Sculpt,
                    sculpt_armed: Some(SculptToolKind::AddRemove),
                    sculpt_pending: pending,
                    ..Default::default()
                };
                super::set_sculpt_radius_share(&ctx, SceneKey::INITIAL, 0.37);
                for _ in 0..2 {
                    panel_frame(&ctx, &state, vec![])
                        .0
                        .drop_without_applying_deltas();
                }
                let (output, _) = panel_frame(&ctx, &state, vec![]);
                let bounds = control_bounds(&output, label, egui::accesskit::Role::Slider);
                output.drop_without_applying_deltas();
                let point = egui::pos2(bounds.right() - 10.0, bounds.center().y);
                let size_before = super::sculpt_radius_share(&ctx, SceneKey::INITIAL);
                let force_before =
                    sculpt_strength(&ctx, SceneKey::INITIAL, SculptToolKind::AddRemove);
                for pressed in [true, false] {
                    let (output, action) = panel_frame(
                        &ctx,
                        &state,
                        vec![
                            egui::Event::PointerMoved(point),
                            egui::Event::PointerButton {
                                pos: point,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                    output.drop_without_applying_deltas();
                    assert_eq!(action, None);
                }
                assert_eq!(
                    super::sculpt_radius_share(&ctx, SceneKey::INITIAL) != size_before,
                    !pending && label == "size"
                );
                assert_eq!(
                    sculpt_strength(&ctx, SceneKey::INITIAL, SculptToolKind::AddRemove)
                        != force_before,
                    !pending && label == "force"
                );
            }
        }
    }

    #[test]
    fn sculpt_strength_keeps_live_settings_finite_and_in_range() {
        let ctx = egui::Context::default();
        for kind in [SculptToolKind::AddRemove, SculptToolKind::Smooth] {
            for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                set_sculpt_strength(&ctx, SceneKey::INITIAL, kind, invalid);
                assert_eq!(
                    sculpt_strength(&ctx, SceneKey::INITIAL, kind),
                    kind.default_strength()
                );
                ctx.data_mut(|data| {
                    data.insert_temp(super::sculpt_strength_id(SceneKey::INITIAL, kind), invalid)
                });
                assert_eq!(
                    sculpt_strength(&ctx, SceneKey::INITIAL, kind),
                    kind.default_strength()
                );
            }
            let (min, max) = kind.strength_range();
            for (input, expected) in [
                (-1.0, min),
                (2.0, max),
                (kind.default_strength(), kind.default_strength()),
            ] {
                set_sculpt_strength(&ctx, SceneKey::INITIAL, kind, input);
                assert_eq!(sculpt_strength(&ctx, SceneKey::INITIAL, kind), expected);
            }
        }
    }

    #[test]
    fn mesh_editor_live_settings_are_isolated_by_scene_lifetime() {
        let ctx = egui::Context::default();
        let first = SceneKey::INITIAL;
        let second = SceneKey::from_raw_for_test(2, 2).expect("nonzero scene identity");

        set_sculpt_radius_mm(&ctx, first, SculptTip::Ball, 1.25);
        set_sculpt_strength(&ctx, first, SculptToolKind::AddRemove, 0.6);
        set_sculpt_tip(&ctx, first, SculptTip::Knife);
        set_close_holes_limit_enabled(&ctx, first, true);
        ctx.data_mut(|data| data.insert_temp(close_holes_limit_id(first), 21.0_f32));

        assert_eq!(sculpt_radius_mm(&ctx, first, SculptTip::Ball), 1.25);
        assert_eq!(sculpt_strength(&ctx, first, SculptToolKind::AddRemove), 0.6);
        assert_eq!(sculpt_tip(&ctx, first), SculptTip::Knife);
        assert_eq!(close_holes_limit_mm(&ctx, first), Some(21.0));

        assert_eq!(
            sculpt_radius_mm(&ctx, second, SculptTip::Ball),
            SculptTip::Ball.default_radius_mm()
        );
        assert_eq!(
            sculpt_strength(&ctx, second, SculptToolKind::AddRemove),
            SculptToolKind::AddRemove.default_strength()
        );
        assert_eq!(sculpt_tip(&ctx, second), SculptTip::default());
        assert_eq!(close_holes_limit_mm(&ctx, second), None);
        set_close_holes_limit_enabled(&ctx, second, true);
        assert_eq!(
            close_holes_limit_mm(&ctx, second),
            Some(CLOSE_HOLES_LIMIT_DEFAULT_MM)
        );
        assert_ne!(
            scoped_id("mesh-editor", first),
            scoped_id("mesh-editor", second)
        );
    }
}
