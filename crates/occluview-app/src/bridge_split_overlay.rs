//! Compact Bridge Split controls and separator-disc overlay.

use crate::bridge_split::{
    BridgeSplitMode, BridgeSplitToolError, MAX_BRIDGE_SPLIT_KERF_MM, MIN_BRIDGE_SPLIT_KERF_MM,
};
use crate::cut_manipulator::{DiscPose, MAX_DISC_RADIUS_MM, MIN_DISC_RADIUS_MM};
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
) -> Option<BridgeSplitPanelAction> {
    let default_pos = viewport_rect.right_top() + egui::vec2(-PANEL_WIDTH - 16.0, 16.0);
    let mut action = None;
    let mut open = true;
    egui::Window::new("Bridge split")
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
                egui::RichText::new(status_label(state.mode))
                    .weak()
                    .size(11.0),
            );
            if let Some(error) = state.failure {
                ui.label(
                    egui::RichText::new(error_label(error))
                        .color(egui::Color32::from_rgb(158, 58, 50))
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
                .text("Kerf")
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
                .text("Disc size")
                .suffix(" mm")
                .step_by(0.25),
            );
            if size_response.changed() {
                action = Some(BridgeSplitPanelAction::SetDiscRadiusMm(diameter_mm * 0.5));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    action = Some(BridgeSplitPanelAction::Cancel);
                }
                let apply = ui.add_enabled(state.can_apply, egui::Button::new("Split bridge"));
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
        painter.add(egui::Shape::line(rim.clone(), egui::Stroke::new(3.8, HALO)));
        painter.extend(egui::Shape::dashed_line(
            rim,
            egui::Stroke::new(1.5, color),
            6.0,
            4.0,
        ));
    }
    if let Some((center, depth)) = project_world_to_viewport(camera, viewport_rect, pose.center) {
        if depth > 0.0 {
            painter.circle_filled(center, 4.0, color.gamma_multiply(0.35));
            painter.circle_stroke(center, 4.0, egui::Stroke::new(1.2, color));
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

fn status_label(mode: BridgeSplitMode) -> &'static str {
    match mode {
        BridgeSplitMode::Following => "Place disc",
        BridgeSplitMode::PlantedPending => "Calculating",
        BridgeSplitMode::PlantedReady => "Ready",
        BridgeSplitMode::Failed => "Cannot split here",
        BridgeSplitMode::Off => "",
    }
}

fn error_label(error: &BridgeSplitToolError) -> String {
    match error {
        BridgeSplitToolError::Kernel(error) => match error {
            occluview_core::BridgeSplitError::NoIntersection => {
                "Disc misses the bridge. Move it into a connector.".to_string()
            }
            occluview_core::BridgeSplitError::TangentContact => {
                "Disc only touches the surface. Move it through the connector.".to_string()
            }
            occluview_core::BridgeSplitError::DiscTooSmall {
                disc_radius_mm,
                required_radius_mm,
            } => format!(
                "Disc diameter is {:.1} mm; at least {:.1} mm is needed here.",
                disc_radius_mm * 2.0,
                required_radius_mm * 2.0
            ),
            occluview_core::BridgeSplitError::DiscLimitExceeded {
                required_radius_mm,
                max_radius_mm,
            } => format!(
                "This cut needs a {:.1} mm disc, above the {:.1} mm safety limit.",
                required_radius_mm * 2.0,
                max_radius_mm * 2.0
            ),
            occluview_core::BridgeSplitError::OpenOrNonManifold { .. } => {
                "Bridge must be a closed, single-shell mesh before splitting.".to_string()
            }
            occluview_core::BridgeSplitError::DisconnectedInput { .. } => {
                "Choose one connected bridge mesh to split.".to_string()
            }
            occluview_core::BridgeSplitError::DegenerateInput { .. } => {
                "Bridge contains degenerate faces. Repair the mesh before splitting.".to_string()
            }
            occluview_core::BridgeSplitError::DamagedCutRim { .. }
            | occluview_core::BridgeSplitError::CapFailed { .. } => {
                "This placement cannot produce clean closed parts. Move or tilt the disc."
                    .to_string()
            }
            occluview_core::BridgeSplitError::InvalidOutput { side, .. } => {
                format!("This placement would leave {side} invalid. Move or tilt the disc.")
            }
            occluview_core::BridgeSplitError::SeparationViolation { .. } => {
                "The requested gap cannot be preserved at this scale. Reduce the kerf.".to_string()
            }
            occluview_core::BridgeSplitError::EmptyInput => {
                "The selected layer has no triangle mesh to split.".to_string()
            }
            occluview_core::BridgeSplitError::InvalidRequest { .. }
            | occluview_core::BridgeSplitError::Mesh(_) => {
                "Disc settings are invalid. Reset the tool and try again.".to_string()
            }
        },
        BridgeSplitToolError::InvalidTransform { .. }
        | BridgeSplitToolError::Conversion { .. }
        | BridgeSplitToolError::Core { .. }
        | BridgeSplitToolError::WorkerStopped => "Preview could not be calculated.".to_string(),
        BridgeSplitToolError::RobustCsg { .. } => {
            "Disc must pass through one connector and leave two closed parts.".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn disc_miss_explains_how_to_correct_the_placement() {
        assert_eq!(
            error_label(&BridgeSplitToolError::Kernel(
                occluview_core::BridgeSplitError::NoIntersection
            )),
            "Disc misses the bridge. Move it into a connector."
        );
    }
}
