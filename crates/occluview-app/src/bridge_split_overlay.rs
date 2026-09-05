//! Compact Bridge Split controls and separator-disc overlay.

use crate::bridge_split::{
    BridgeSplitMode, BridgeSplitToolError, MAX_BRIDGE_SPLIT_KERF_MM, MIN_BRIDGE_SPLIT_KERF_MM,
};
use crate::cut_manipulator::{DiscPose, MAX_DISC_RADIUS_MM, MIN_DISC_RADIUS_MM};
use crate::ui_theme;
use crate::viewer::project_world_to_viewport;
use eframe::egui;
use glam::Vec3;
use occluview_core::Camera;

const PANEL_WIDTH: f32 = 224.0;
const RIM_SEGMENTS: u16 = 72;
const HALO: egui::Color32 = egui::Color32::from_rgba_premultiplied(245, 247, 249, 180);
const READY: egui::Color32 = egui::Color32::from_rgb(38, 121, 92);
const PENDING: egui::Color32 = egui::Color32::from_rgb(177, 116, 24);
const FOLLOW: egui::Color32 = egui::Color32::from_rgb(49, 96, 165);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum BridgeSplitPanelAction {
    SetKerfMm(f32),
    SetDiscRadiusMm(f32),
    Apply,
    Cancel,
}

pub(crate) struct BridgeSplitPanelState<'a> {
    pub(crate) mode: BridgeSplitMode,
    pub(crate) kerf_mm: f32,
    pub(crate) disc_radius_mm: f32,
    pub(crate) can_apply: bool,
    pub(crate) failure: Option<&'a BridgeSplitToolError>,
}

#[derive(Clone, Copy)]
pub(crate) struct SeparatorDisc {
    pub(crate) pose: DiscPose,
    pub(crate) kerf_mm: f32,
    pub(crate) mode: BridgeSplitMode,
}

struct RimProjection {
    center: Vec3,
    u: Vec3,
    v: Vec3,
    radius_mm: f32,
}

pub(crate) fn show_panel(
    ctx: &egui::Context,
    viewport_rect: egui::Rect,
    state: BridgeSplitPanelState<'_>,
    locale: &crate::i18n::LocaleManager,
) -> Option<BridgeSplitPanelAction> {
    let default_pos = viewport_rect.right_top() + egui::vec2(-PANEL_WIDTH - 16.0, 16.0);
    let mut action = None;
    let mut open = true;
    egui::Window::new(locale.tr("bridge-panel-title"))
        .id(egui::Id::new("occluview_bridge_split"))
        .default_pos(default_pos)
        .constrain_to(viewport_rect)
        .resizable(false)
        .collapsible(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(PANEL_WIDTH - 22.0);
            ui.style_mut().animation_time = 0.05;
            ui.label(
                egui::RichText::new(status_label(state.mode, locale))
                    .weak()
                    .size(11.0),
            );
            if let Some(error) = state.failure {
                ui.label(
                    egui::RichText::new(error_label(error, locale))
                        .color(ui_theme::danger())
                        .size(11.0),
                );
            }
            ui.add_space(4.0);
            let mut kerf = state.kerf_mm;
            let response = ui.add_enabled(
                !matches!(state.mode, BridgeSplitMode::PlantedPending),
                egui::Slider::new(
                    &mut kerf,
                    MIN_BRIDGE_SPLIT_KERF_MM..=MAX_BRIDGE_SPLIT_KERF_MM,
                )
                .text(locale.tr("bridge-kerf").as_str())
                .suffix(" mm")
                .step_by(0.01),
            );
            if response.changed() {
                action = Some(BridgeSplitPanelAction::SetKerfMm(kerf));
            }
            let mut diameter_mm = state.disc_radius_mm * 2.0;
            let size_response = ui.add_enabled(
                !matches!(state.mode, BridgeSplitMode::PlantedPending),
                egui::Slider::new(
                    &mut diameter_mm,
                    (MIN_DISC_RADIUS_MM * 2.0)..=(MAX_DISC_RADIUS_MM * 2.0),
                )
                .text(locale.tr("bridge-disc-size").as_str())
                .suffix(" mm")
                .step_by(0.25),
            );
            if size_response.changed() {
                action = Some(BridgeSplitPanelAction::SetDiscRadiusMm(diameter_mm * 0.5));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(locale.tr("bridge-cancel")).clicked() {
                    action = Some(BridgeSplitPanelAction::Cancel);
                }
                let apply = ui.add_enabled(
                    state.can_apply,
                    egui::Button::new(locale.tr("bridge-apply")),
                );
                if apply.clicked() {
                    action = Some(BridgeSplitPanelAction::Apply);
                }
            });
        });
    if !open {
        action = Some(BridgeSplitPanelAction::Cancel);
    }
    action
}

pub(crate) fn paint_separator_disc(
    painter: &egui::Painter,
    camera: &Camera,
    viewport_rect: egui::Rect,
    disc: SeparatorDisc,
) {
    let pose = disc.pose;
    let kerf_mm = disc.kerf_mm;
    let mode = disc.mode;
    let normal = pose.plane_normal.normalize_or_zero();
    if normal.length_squared() <= f32::EPSILON || !kerf_mm.is_finite() || kerf_mm <= 0.0 {
        return;
    }
    let (u, v) = plane_basis(normal);
    let half_kerf = kerf_mm * 0.5;
    let front_rim = RimProjection {
        center: pose.center + normal * half_kerf,
        u,
        v,
        radius_mm: pose.radius_mm,
    };
    let Some(front) = project_rim(camera, viewport_rect, front_rim) else {
        return;
    };
    let back_rim = RimProjection {
        center: pose.center - normal * half_kerf,
        u,
        v,
        radius_mm: pose.radius_mm,
    };
    let Some(back) = project_rim(camera, viewport_rect, back_rim) else {
        return;
    };
    let color = match mode {
        BridgeSplitMode::PlantedReady => READY,
        BridgeSplitMode::PlantedPending | BridgeSplitMode::Failed => PENDING,
        BridgeSplitMode::Following | BridgeSplitMode::Off => FOLLOW,
    };
    painter.add(egui::Shape::convex_polygon(
        front.clone(),
        color.gamma_multiply(0.12),
        egui::Stroke::NONE,
    ));
    for rim in [&front, &back] {
        painter.add(egui::Shape::line(
            rim.clone(),
            egui::Stroke::new(3.8_f32, HALO),
        ));
        painter.extend(egui::Shape::dashed_line(
            rim,
            egui::Stroke::new(1.5_f32, color),
            6.0,
            4.0,
        ));
    }
    if let Some((center, depth)) = project_world_to_viewport(camera, viewport_rect, pose.center) {
        if depth > 0.0 {
            painter.circle_filled(center, 4.0, color.gamma_multiply(0.35));
            painter.circle_stroke(center, 4.0, egui::Stroke::new(1.2_f32, color));
        }
    }
}

fn project_rim(
    camera: &Camera,
    viewport_rect: egui::Rect,
    rim_projection: RimProjection,
) -> Option<Vec<egui::Pos2>> {
    let RimProjection {
        center,
        u,
        v,
        radius_mm,
    } = rim_projection;
    if !radius_mm.is_finite() || radius_mm <= 0.0 {
        return None;
    }
    let mut rim = Vec::with_capacity(usize::from(RIM_SEGMENTS) + 1);
    for index in 0..RIM_SEGMENTS {
        let theta = std::f32::consts::TAU * f32::from(index) / f32::from(RIM_SEGMENTS);
        let point = center + (u * theta.cos() + v * theta.sin()) * radius_mm;
        let (screen, depth) = project_world_to_viewport(camera, viewport_rect, point)?;
        if depth <= 0.0 {
            return None;
        }
        rim.push(screen);
    }
    rim.push(rim[0]);
    Some(rim)
}

fn plane_basis(normal: Vec3) -> (Vec3, Vec3) {
    let seed = if normal.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let u = (seed - normal * seed.dot(normal)).normalize_or(Vec3::X);
    (u, normal.cross(u).normalize_or(Vec3::Z))
}

fn status_label(mode: BridgeSplitMode, locale: &crate::i18n::LocaleManager) -> String {
    let key = match mode {
        BridgeSplitMode::Following => "bridge-mode-place",
        BridgeSplitMode::PlantedPending => "bridge-mode-calculating",
        BridgeSplitMode::PlantedReady => "bridge-mode-ready",
        BridgeSplitMode::Failed => "bridge-mode-failed",
        BridgeSplitMode::Off => return String::new(),
    };
    locale.tr(key)
}

fn error_label(error: &BridgeSplitToolError, locale: &crate::i18n::LocaleManager) -> String {
    match error {
        BridgeSplitToolError::Kernel(error) => match error {
            occluview_core::BridgeSplitError::NoIntersection => locale.tr("bridge-err-miss"),
            occluview_core::BridgeSplitError::TangentContact => locale.tr("bridge-err-tangent"),
            occluview_core::BridgeSplitError::DiscTooSmall {
                disc_radius_mm,
                required_radius_mm,
            } => locale.tr_with(
                "bridge-err-small",
                &[
                    ("have", &format!("{:.1}", disc_radius_mm * 2.0)),
                    ("need", &format!("{:.1}", required_radius_mm * 2.0)),
                ],
            ),
            occluview_core::BridgeSplitError::DiscLimitExceeded {
                required_radius_mm,
                max_radius_mm,
            } => locale.tr_with(
                "bridge-err-limit",
                &[
                    ("need", &format!("{:.1}", required_radius_mm * 2.0)),
                    ("max", &format!("{:.1}", max_radius_mm * 2.0)),
                ],
            ),
            occluview_core::BridgeSplitError::OpenOrNonManifold { .. }
            | occluview_core::BridgeSplitError::DisconnectedInput { .. }
            | occluview_core::BridgeSplitError::DegenerateInput { .. } => {
                locale.tr("bridge-err-no-result")
            }
            occluview_core::BridgeSplitError::DamagedCutRim { .. }
            | occluview_core::BridgeSplitError::CapFailed { .. } => {
                locale.tr("bridge-err-invalid-cut")
            }
            occluview_core::BridgeSplitError::InvalidOutput { side, .. } => {
                locale.tr_with("bridge-err-invalid-side", &[("side", side)])
            }
            occluview_core::BridgeSplitError::SeparationViolation { .. } => {
                locale.tr("bridge-err-gap")
            }
            occluview_core::BridgeSplitError::EmptyInput => locale.tr("bridge-err-empty"),
            occluview_core::BridgeSplitError::InvalidRequest { .. }
            | occluview_core::BridgeSplitError::Mesh(_) => locale.tr("bridge-err-invalid"),
        },
        BridgeSplitToolError::InvalidTransform { .. }
        | BridgeSplitToolError::Conversion { .. }
        | BridgeSplitToolError::Core { .. }
        | BridgeSplitToolError::RobustCsg { .. }
        | BridgeSplitToolError::WorkerStopped => locale.tr("bridge-err-unusable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kept English mode labels render from the catalog verbatim.
    #[test]
    fn english_mode_labels_match_source_wording() {
        #![allow(clippy::expect_used)]
        let catalog = crate::i18n::catalog::Catalog::build("en").expect("en builds");
        let locale = crate::i18n::LocaleManager::for_tests();
        for (mode, key, label) in [
            (
                BridgeSplitMode::Following,
                "bridge-mode-place",
                "Place disc",
            ),
            (
                BridgeSplitMode::PlantedPending,
                "bridge-mode-calculating",
                "Calculating",
            ),
            (BridgeSplitMode::PlantedReady, "bridge-mode-ready", "Ready"),
            (
                BridgeSplitMode::Failed,
                "bridge-mode-failed",
                "Split attempt failed",
            ),
        ] {
            assert_eq!(status_label(mode, &locale), label);
            assert_eq!(catalog.text(key).as_deref(), Some(label));
        }
    }

    #[test]
    fn plane_basis_is_finite_and_orthogonal_to_disc_normal() {
        for normal in [
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            Vec3::new(1.0, 2.0, 3.0).normalize(),
        ] {
            let (u, v) = plane_basis(normal);
            assert!(u.is_finite() && v.is_finite());
            assert!(u.dot(normal).abs() < 1.0e-5);
            assert!(v.dot(normal).abs() < 1.0e-5);
            assert!(u.dot(v).abs() < 1.0e-5);
        }
    }

    fn english() -> crate::i18n::LocaleManager {
        crate::i18n::LocaleManager::for_tests()
    }

    #[test]
    fn disc_miss_explains_how_to_correct_the_placement() {
        assert_eq!(
            error_label(
                &BridgeSplitToolError::Kernel(occluview_core::BridgeSplitError::NoIntersection),
                &english(),
            ),
            "Disc misses the bridge. Move it into a connector."
        );
    }

    #[test]
    fn topology_failures_do_not_expose_repair_instructions() {
        let label = error_label(
            &BridgeSplitToolError::Kernel(occluview_core::BridgeSplitError::DegenerateInput {
                faces: 4,
            }),
            &english(),
        );
        assert!(!label.to_ascii_lowercase().contains("repair"));
        assert!(!label.to_ascii_lowercase().contains("degenerate"));
        assert!(label.contains("original mesh was kept"));
    }
}
