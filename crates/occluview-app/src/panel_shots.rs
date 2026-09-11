//! Headless wireframes for the high-risk operator panels.
//!
//! These are deliberately real production panels rendered through egui rather
//! than hand-built mock layouts. The PNGs land in `target/i18n-shots/` for a
//! human to inspect; this keeps screenshot review out of the runtime and out
//! of tracked binary assets.

#![allow(clippy::expect_used)]

use crate::align_brush::AlignBrush;
use crate::align_drag::DragConstraint;
use crate::align_panel::{AlignPanelView, AlignTab};
use crate::align_tool::{AlignPoint, AlignTool};
use crate::align_worker::AlignSettings;
use crate::mesh_editor_overlay::{EditorTab, MeshEditorPanelState};
use crate::sculpt_tool::SculptToolKind;
use eframe::egui;
use glam::Vec3;
use occluview_core::{Mesh, SceneMesh};

const SCREEN: egui::Rect = egui::Rect::from_min_max(
    egui::Pos2::ZERO,
    egui::Pos2 {
        x: 1024.0,
        y: 768.0,
    },
);

fn frame(name: &str, mut draw: impl FnMut(&egui::Context)) {
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|style| style.animation_time = 0.0);
    let input = || egui::RawInput {
        screen_rect: Some(SCREEN),
        ..Default::default()
    };
    // egui windows need one layout pass to resolve their default position and
    // measured size before the frame is useful for visual review.
    ctx.run_ui(input(), |ui| draw(ui.ctx()))
        .drop_without_applying_deltas();
    let output = ctx.run_ui(input(), |ui| draw(ui.ctx()));
    crate::i18n::shots::save_shot_with_texts(name, &ctx, output, 1024, 768);
}

fn populated_align_tool() -> AlignTool {
    // `SceneMeshId` is intentionally created by the scene entry, not by a
    // test-only constructor. The panel only needs stable identities here.
    let ids = [
        SceneMesh::new(Mesh::empty()).id(),
        SceneMesh::new(Mesh::empty()).id(),
    ];
    let mut tool = AlignTool::default();
    tool.arm();
    for x in [0.0, 1.0] {
        let point = AlignPoint {
            layer: ids[0],
            local: Vec3::new(x, 0.0, 0.0),
            normal: Vec3::Z,
        };
        let partner = AlignPoint {
            layer: ids[1],
            local: Vec3::new(x, 0.0, 0.0),
            normal: Vec3::Z,
        };
        let _ = tool.click(point);
        let _ = tool.click(partner);
    }
    tool
}

fn render_align(name: &str, tab: AlignTab, refined: bool, brush_open: bool) {
    frame(name, |ctx| {
        let tool = populated_align_tool();
        let mut settings = AlignSettings {
            show_deviation: refined,
            ..AlignSettings::default()
        };
        let mut constraint = DragConstraint::Free;
        let mut excluding = brush_open;
        let mut drop_pending = false;
        let mut open_tab = tab;
        let roles = Some(crate::align_panel_roles::AlignRoles {
            moving: "upper-arch-scan.stl".to_owned(),
            fixed: "lower-arch-scan.stl".to_owned(),
            implied: false,
        });
        let brush_roles = Some(crate::align_panel_roles::AlignRoles {
            moving: "upper-arch-scan.stl".to_owned(),
            fixed: "lower-arch-scan.stl".to_owned(),
            implied: false,
        });
        let _ = crate::align_panel::show(
            ctx,
            SCREEN,
            AlignPanelView {
                tool: &tool,
                settings: &mut settings,
                constraint: &mut constraint,
                excluding: &mut excluding,
                drop_pending: &mut drop_pending,
                status: Some("Best fit matching complete"),
                refined_match_ready: refined,
                roles,
                busy: false,
                worker_failed: false,
                moved: true,
                can_undo: true,
                can_redo: false,
                tab: &mut open_tab,
            },
            &crate::i18n::LocaleManager::for_tests(),
        );
        if brush_open {
            let mut brush = AlignBrush::default();
            brush.set_armed(true);
            let _ = crate::align_panel_brush::show(
                ctx,
                SCREEN,
                &mut brush,
                brush_roles.as_ref(),
                true,
                &crate::i18n::LocaleManager::for_tests(),
            );
        }
    });
}

#[test]
fn align_and_mesh_editor_wireframes_for_visual_review() {
    crate::ui_theme::set_active(crate::app_settings::ThemePreference::Light);
    render_align(
        "audit-align-automatic",
        AlignTab::Automatically,
        true,
        false,
    );
    render_align("audit-align-manual", AlignTab::Manually, false, false);
    render_align("audit-align-brush", AlignTab::Automatically, true, true);

    frame("audit-mesh-editor", |ctx| {
        let state = MeshEditorPanelState {
            selected_face_count: 12,
            can_undo: true,
            can_redo: true,
            dirty: true,
            active_tab: EditorTab::EditMesh,
            ..Default::default()
        };
        let locale = crate::i18n::LocaleManager::for_tests();
        let _ = crate::mesh_editor_overlay::show(ctx, SCREEN, state, &locale);
    });

    frame("audit-sculpt-editor", |ctx| {
        let state = MeshEditorPanelState {
            sculpt_armed: Some(SculptToolKind::Smooth),
            sculpt_pending: true,
            dirty: true,
            active_tab: EditorTab::Sculpt,
            ..Default::default()
        };
        let locale = crate::i18n::LocaleManager::for_tests();
        let _ = crate::mesh_editor_overlay::show(ctx, SCREEN, state, &locale);
    });
}
