use eframe::egui;
use glam::Vec3;
use occluview_core::scene::SceneSection;
use occluview_core::SceneMeshId;

use super::model::{CutRuler, SliceBasis, SliceCam, SlicePlaneMap};
use crate::cut_geometry::snap_to_contour;
use crate::{probe_section, ui_theme};

/// Docked Section-panel geometry (bottom-right; the axis gizmo lifts above it).
/// The image stays square so the orthographic mm-per-pixel is identical on both
/// axes and the ruler mapping remains exact.
const IMAGE_SIDE_PX: f32 = 340.0;
const MIN_IMAGE_SIDE_PX: f32 = 180.0;
const CHROME_GAP_PX: f32 = 8.0;
const PANEL_PAD_PX: f32 = 6.0;
const PANEL_HEADER_PX: f32 = 22.0;
const PANEL_FOOTER_PX: f32 = 15.0;
const PANEL_MARGIN_PX: f32 = 14.0;
const PANEL_BOTTOM_GAP_PX: f32 = 16.0;

/// Contour stroke width (logical px) for `Lines` mode.
const SECTION_LINE_PX: f32 = 1.5;
/// Magnet snap radius, in PANEL pixels (constant on screen regardless of zoom).
const SNAP_RADIUS_PX: f32 = 8.0;

/// The Section panel's display mode. `Lines` (the default) draws only the crisp
/// plane∩mesh contour polylines on the panel's clean background; `Mesh` shows
/// the shaded offscreen slice render.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum SectionDisplay {
    #[default]
    Lines,
    Mesh,
}

/// The Section panel's measuring mode. `Distance` (the default) places two points
/// and reports the straight-line distance; `Thickness` casts a one-click in-slice
/// wall-thickness ray from the contour, sharing the main-viewport probe's look.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum SliceMeasureMode {
    #[default]
    Distance,
    Thickness,
}

/// What the Section panel should render this frame. `texture` is consulted only
/// in `Mesh` mode; `section` (the cached world-space contour) drives `Lines`
/// drawing AND magnet snapping in BOTH modes; `color_for` tints each layer's
/// contour to match the main-viewport overlay.
pub(crate) struct SectionRender<'a, F> {
    pub(crate) mode: SectionDisplay,
    pub(crate) measure_mode: SliceMeasureMode,
    pub(crate) magnet: bool,
    pub(crate) texture: Option<&'a egui::TextureHandle>,
    pub(crate) section: Option<&'a SceneSection>,
    pub(crate) color_for: F,
}

/// The Section panel's per-frame outcome: the (possibly toggled) header state,
/// the in-plane pan to fold into the framing focus, and whether the pointer was
/// consumed (so the disc/camera do not also react).
#[derive(Clone, Copy, Debug)]
pub(crate) struct SectionPanelOut {
    pub(crate) consumed: bool,
    pub(crate) mode: SectionDisplay,
    pub(crate) measure_mode: SliceMeasureMode,
    pub(crate) magnet: bool,
    pub(crate) pan_delta: Vec3,
    pub(crate) panned: bool,
    pub(crate) command: SectionPanelCommand,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SectionPanelCommand {
    #[default]
    None,
    Close,
}

/// The result of one frame of Section-panel pointer interaction.
struct PanelGesture {
    pan_delta: Vec3,
    panned: bool,
    consumed: bool,
}

/// Inputs a click needs to place a measurement: the section cam (to anchor the
/// point + supply the plane normal), the measuring mode, whether the magnet is
/// on, and the contour to snap/probe against.
struct RulerPlacement<'a> {
    cam: SliceCam,
    measure_mode: SliceMeasureMode,
    magnet: bool,
    section: Option<&'a SceneSection>,
}

/// The docked Section-panel rectangle (bottom-RIGHT), or `None` if the viewport
/// is too small to host it without crowding the chrome. The axis gizmo lifts to
/// sit just above this rect while the cut tool is active.
///
/// The panel ADAPTS to the window instead of painting over the chrome: its
/// image side shrinks from the full [`IMAGE_SIDE_PX`] until it would drop
/// below [`MIN_IMAGE_SIDE_PX`], budgeting (vertically) the lifted axis gizmo
/// and (horizontally) the bottom-left status pill.
pub(crate) fn section_panel_rect(viewport_rect: egui::Rect) -> Option<egui::Rect> {
    let chrome_h = PANEL_HEADER_PX + PANEL_FOOTER_PX + PANEL_PAD_PX * 2.0;
    // Vertical budget: the gizmo lifts above the panel with a breathing gap.
    let top_reserve =
        viewport_rect.top() + CHROME_GAP_PX + crate::viewer::axis_gizmo::AXIS_GIZMO_LIFT_RESERVE_PX;
    let side_by_height = viewport_rect.bottom() - PANEL_BOTTOM_GAP_PX - chrome_h - top_reserve;
    // Horizontal budget: never cover the bottom-left status pill (transient
    // messages must stay readable while cutting).
    let pill_right = crate::app_chrome::status_overlay_rect(viewport_rect).right();
    let side_by_width =
        viewport_rect.right() - PANEL_MARGIN_PX - PANEL_PAD_PX * 2.0 - (pill_right + CHROME_GAP_PX);
    let side = IMAGE_SIDE_PX.min(side_by_height).min(side_by_width);
    if side < MIN_IMAGE_SIDE_PX {
        return None;
    }
    let width = side + PANEL_PAD_PX * 2.0;
    let height = chrome_h + side;
    Some(egui::Rect::from_min_size(
        egui::pos2(
            viewport_rect.right() - PANEL_MARGIN_PX - width,
            viewport_rect.bottom() - PANEL_BOTTOM_GAP_PX - height,
        ),
        egui::vec2(width, height),
    ))
}

/// The square section-image sub-rect for a viewport, or `None` when the panel
/// does not fit. Public so the in-panel zoom-at-cursor can map the pointer to a
/// slice point with the same rect the panel image is painted into.
pub(crate) fn section_image_rect_for(viewport_rect: egui::Rect) -> Option<egui::Rect> {
    section_panel_rect(viewport_rect).map(section_image_rect)
}

/// Whether the pointer is inside the docked Section panel (used by the adapter to
/// scope the disc-radius wheel and reserve the panel's footprint).
pub(crate) fn section_panel_contains(viewport_rect: egui::Rect, pos: egui::Pos2) -> bool {
    section_panel_rect(viewport_rect).is_some_and(|rect| rect.contains(pos))
}

/// The square image sub-rect inside a section panel (below the header). The
/// side derives from the panel rect so an adaptively shrunk panel keeps the
/// ruler's pixel↔mm mapping exact.
fn section_image_rect(panel_rect: egui::Rect) -> egui::Rect {
    let side = panel_rect.width() - PANEL_PAD_PX * 2.0;
    egui::Rect::from_min_size(
        egui::pos2(
            panel_rect.left() + PANEL_PAD_PX,
            panel_rect.top() + PANEL_PAD_PX + PANEL_HEADER_PX,
        ),
        egui::vec2(side, side),
    )
}

/// Draw the docked bottom-right "Section" panel: a header with the Lines/Mesh
/// display toggle and the magnet-snap toggle, the section content (crisp contour
/// polylines in `Lines` mode, the shaded offscreen slice in `Mesh` mode), and the
/// two-point measuring ruler. Inside the image: left-drag OR right-drag pans;
/// left-click places a measurement point (a third restarts), snapping to the
/// nearest contour point when the magnet is on; right-click clears. Markers are
/// anchored in section-plane millimeters and re-project as the disc scales, so
/// lines, ruler, zoom and pan stay exactly consistent.
#[cfg(test)]
pub(crate) fn show_section_panel<F>(
    ui: &mut egui::Ui,
    viewport_rect: egui::Rect,
    cam: SliceCam,
    ruler: &mut CutRuler,
    render: SectionRender<'_, F>,
) -> SectionPanelOut
where
    F: Fn(SceneMeshId) -> egui::Color32,
{
    show_section_panel_with_basis(
        ui,
        viewport_rect,
        cam,
        SliceBasis::from_normal(cam.normal),
        ruler,
        render,
    )
}

/// Draw the existing Section panel using the primary viewport's in-plane
/// orientation. The same basis is used for drawing, picking, measuring, pan and
/// zoom, so the panel never displays one orientation while interacting in another.
#[allow(clippy::too_many_arguments)]
pub(crate) fn show_section_panel_with_basis<F>(
    ui: &mut egui::Ui,
    viewport_rect: egui::Rect,
    cam: SliceCam,
    basis: SliceBasis,
    ruler: &mut CutRuler,
    render: SectionRender<'_, F>,
) -> SectionPanelOut
where
    F: Fn(SceneMeshId) -> egui::Color32,
{
    let mut out = SectionPanelOut {
        consumed: false,
        mode: render.mode,
        measure_mode: render.measure_mode,
        magnet: render.magnet,
        pan_delta: Vec3::ZERO,
        panned: false,
        command: SectionPanelCommand::None,
    };
    let Some(panel_rect) = section_panel_rect(viewport_rect) else {
        return out;
    };
    let image_rect = section_image_rect(panel_rect);
    draw_panel_frame(ui.painter(), panel_rect);
    let header = draw_section_header(
        ui,
        panel_rect,
        render.mode,
        render.measure_mode,
        render.magnet,
    );
    out.mode = header.mode;
    out.measure_mode = header.measure_mode;
    out.magnet = header.magnet;
    out.command = header.command;

    // Interaction first, so a pan can shift the drawn view the same frame.
    let placement = RulerPlacement {
        cam,
        measure_mode: render.measure_mode,
        magnet: render.magnet,
        section: render.section,
    };
    let gesture = handle_panel_gesture(ui, image_rect, ruler, placement, basis);
    out.pan_delta = gesture.pan_delta;
    out.panned = gesture.panned;

    // In `Lines` mode the pan shifts the (vector) view immediately; in `Mesh`
    // mode the texture is a raster that re-renders next frame, so keep the ruler
    // pinned to the displayed texture by NOT shifting the draw map there.
    let draw_cam = if matches!(render.mode, SectionDisplay::Lines) {
        SliceCam {
            focus: cam.focus + gesture.pan_delta,
            ..cam
        }
    } else {
        cam
    };
    let draw_map = SlicePlaneMap::new_with_basis(draw_cam, image_rect, basis);
    let has_content = match render.mode {
        SectionDisplay::Mesh => {
            draw_slice_texture(&ui.painter_at(image_rect), image_rect, render.texture)
        }
        SectionDisplay::Lines => draw_section_lines(
            &ui.painter_at(image_rect),
            &draw_map,
            render.section,
            &render.color_for,
        ),
    };
    if !has_content {
        draw_empty_state(ui.painter(), image_rect);
    }
    ruler.draw(&ui.painter_at(image_rect), &draw_map);
    draw_section_footer(ui.painter(), panel_rect, render.measure_mode);

    out.consumed =
        header.consumed || gesture.consumed || out.panned || ui.rect_contains_pointer(panel_rect);
    out
}

/// Paint the rounded panel background and the header hairline. The live distance
/// is drawn on the ruler line itself ([`CutRuler::draw`]), not in the header.
fn draw_panel_frame(painter: &egui::Painter, panel_rect: egui::Rect) {
    painter.rect_filled(
        panel_rect,
        egui::Rounding::same(8.0),
        ui_theme::panel_fill(),
    );
    painter.rect_stroke(
        panel_rect,
        egui::Rounding::same(8.0),
        egui::Stroke::new(1.0, ui_theme::panel_stroke()),
    );
    let header_baseline = panel_rect.top() + PANEL_PAD_PX + PANEL_HEADER_PX;
    painter.line_segment(
        [
            egui::pos2(panel_rect.left() + 6.0, header_baseline),
            egui::pos2(panel_rect.right() - 6.0, header_baseline),
        ],
        egui::Stroke::new(1.0, ui_theme::hairline()),
    );
}

/// The header toggles' resolved state for one frame.
struct HeaderOut {
    mode: SectionDisplay,
    measure_mode: SliceMeasureMode,
    magnet: bool,
    consumed: bool,
    command: SectionPanelCommand,
}

/// Draw the interactive header toggles — the Lines/Mesh display switch, the
/// Dist/Thick measuring switch, and the magnet snap — returning their resolved
/// state and whether the header owned the pointer.
fn draw_section_header(
    ui: &mut egui::Ui,
    panel_rect: egui::Rect,
    mode: SectionDisplay,
    measure_mode: SliceMeasureMode,
    magnet: bool,
) -> HeaderOut {
    let header_rect = egui::Rect::from_min_size(
        egui::pos2(
            panel_rect.left() + PANEL_PAD_PX,
            panel_rect.top() + PANEL_PAD_PX,
        ),
        egui::vec2(panel_rect.width() - PANEL_PAD_PX * 2.0, PANEL_HEADER_PX),
    );
    let mut out = HeaderOut {
        mode,
        measure_mode,
        magnet,
        consumed: false,
        command: SectionPanelCommand::None,
    };
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(header_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let lines = ui.selectable_label(matches!(mode, SectionDisplay::Lines), "Lines");
            if lines.clicked() {
                out.mode = SectionDisplay::Lines;
            }
            let mesh = ui.selectable_label(matches!(mode, SectionDisplay::Mesh), "Mesh");
            if mesh.clicked() {
                out.mode = SectionDisplay::Mesh;
            }
            ui.separator();
            let dist = ui
                .selectable_label(matches!(measure_mode, SliceMeasureMode::Distance), "Dist")
                .on_hover_text("Distance: click two points");
            if dist.clicked() {
                out.measure_mode = SliceMeasureMode::Distance;
            }
            let thick = ui
                .selectable_label(matches!(measure_mode, SliceMeasureMode::Thickness), "Thick")
                .on_hover_text("Wall thickness: click one point on the contour");
            if thick.clicked() {
                out.measure_mode = SliceMeasureMode::Thickness;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = ui
                    .add_sized(
                        [24.0, 20.0],
                        egui::Button::new(egui::RichText::new("×").size(16.0)).frame(false),
                    )
                    .on_hover_text("Close section");
                if close.clicked() {
                    out.command = SectionPanelCommand::Close;
                }
                let snap = ui
                    .selectable_label(magnet, "Snap")
                    .on_hover_text("Magnet: click points snap to the section contour");
                if snap.clicked() {
                    out.magnet = !magnet;
                }
                out.consumed |=
                    close.hovered() || close.clicked() || snap.hovered() || snap.clicked();
            });
            out.consumed |= lines.hovered()
                || mesh.hovered()
                || dist.hovered()
                || thick.hovered()
                || lines.clicked()
                || mesh.clicked()
                || dist.clicked()
                || thick.clicked();
        },
    );
    out
}

/// Handle one frame of pointer interaction inside the section image: pan on a
/// left OR right drag, place a (optionally snapped) measurement point on a left
/// click, clear on a right click.
fn handle_panel_gesture(
    ui: &mut egui::Ui,
    image_rect: egui::Rect,
    ruler: &mut CutRuler,
    placement: RulerPlacement<'_>,
    basis: SliceBasis,
) -> PanelGesture {
    let map = SlicePlaneMap::new_with_basis(placement.cam, image_rect, basis);
    let response = ui.interact(
        image_rect,
        ui.id().with("cut-section-ruler"),
        egui::Sense::click_and_drag(),
    );
    let mut pan_delta = Vec3::ZERO;
    let mut panned = false;
    if response.dragged() {
        if let Some(pointer) = response.interact_pointer_pos() {
            pan_delta = map.pan_delta_for_drag(pointer, response.drag_delta());
            panned = true;
        }
    } else if response.secondary_clicked() {
        ruler.clear();
    } else if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            place_measurement(pos, &map, ruler, &placement);
        }
    }
    PanelGesture {
        pan_delta,
        panned,
        consumed: response.hovered()
            || response.clicked()
            || response.secondary_clicked()
            || panned,
    }
}

/// Apply one panel click for the active measuring mode: place a distance point
/// (magnet-snapped when enabled), or cast a one-click in-slice wall-thickness ray
/// from the contour. A thickness click that hits no opposite edge is an honest
/// no-op — nothing is placed.
fn place_measurement(
    click: egui::Pos2,
    map: &SlicePlaneMap,
    ruler: &mut CutRuler,
    placement: &RulerPlacement<'_>,
) {
    match placement.measure_mode {
        SliceMeasureMode::Distance => {
            let world = place_point_world(click, placement.magnet, map, placement.section);
            ruler.place(world, placement.cam);
        }
        SliceMeasureMode::Thickness => {
            let Some(section) = placement.section else {
                return;
            };
            let click_world = map.panel_to_world(click);
            let segments = section_segments(section);
            if let Some(probe) =
                probe_section::slice_wall_thickness(click_world, placement.cam.normal, &segments)
            {
                ruler.set_thickness(probe.entry, probe.exit, probe.thickness_mm, placement.cam);
            }
        }
    }
}

/// The world point a click places: the exact nearest contour point when the
/// magnet is on and one is within [`SNAP_RADIUS_PX`], else the raw plane point.
fn place_point_world(
    click: egui::Pos2,
    magnet: bool,
    map: &SlicePlaneMap,
    section: Option<&SceneSection>,
) -> Vec3 {
    if magnet {
        if let Some(section) = section {
            if let Some(snapped) = snap_to_contour(
                click,
                section_segments(section),
                |world| map.world_to_panel(world),
                SNAP_RADIUS_PX,
            ) {
                return snapped;
            }
        }
    }
    map.panel_to_world(click)
}

/// Flatten a section into world-space contour segments (including each closed
/// polyline's wrap-around edge) for magnet snapping.
fn section_segments(section: &SceneSection) -> Vec<(Vec3, Vec3)> {
    let mut segments = Vec::new();
    for layer in &section.per_layer {
        for polyline in &layer.polylines {
            let points = &polyline.points;
            for pair in points.windows(2) {
                segments.push((pair[0].as_vec3(), pair[1].as_vec3()));
            }
            if polyline.closed && points.len() >= 2 {
                if let (Some(last), Some(first)) = (points.last(), points.first()) {
                    segments.push((last.as_vec3(), first.as_vec3()));
                }
            }
        }
    }
    segments
}

/// Draw the shaded offscreen slice (crisp: rendered larger, downscaled here).
/// Returns whether a texture was actually painted.
fn draw_slice_texture(
    painter: &egui::Painter,
    image_rect: egui::Rect,
    texture: Option<&egui::TextureHandle>,
) -> bool {
    let Some(texture) = texture else {
        return false;
    };
    painter.image(
        texture.id(),
        image_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    true
}

/// Draw the section contour polylines as crisp 2D strokes through `map`, tinted
/// per layer. Returns whether any polyline was drawn.
fn draw_section_lines<F>(
    painter: &egui::Painter,
    map: &SlicePlaneMap,
    section: Option<&SceneSection>,
    color_for: &F,
) -> bool
where
    F: Fn(SceneMeshId) -> egui::Color32,
{
    let Some(section) = section else {
        return false;
    };
    let mut drawn = false;
    for layer in &section.per_layer {
        let color = color_for(layer.layer_id);
        for polyline in &layer.polylines {
            let mut points: Vec<egui::Pos2> = polyline
                .points
                .iter()
                .map(|p| map.world_to_panel(p.as_vec3()))
                .collect();
            if points.len() < 2 {
                continue;
            }
            if polyline.closed {
                points.push(points[0]);
            }
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(SECTION_LINE_PX, color),
            ));
            drawn = true;
        }
    }
    drawn
}

/// Honest empty state when the plane misses every mesh (or only point clouds are
/// visible): a centered note, never a stale picture.
fn draw_empty_state(painter: &egui::Painter, image_rect: egui::Rect) {
    painter.text(
        image_rect.center(),
        egui::Align2::CENTER_CENTER,
        "No intersection",
        egui::FontId::proportional(12.0),
        ui_theme::TEXT_MUTED,
    );
}

/// The one-line measuring/pan hint pinned to the panel's bottom edge, phrased for
/// the active measuring mode.
fn draw_section_footer(
    painter: &egui::Painter,
    panel_rect: egui::Rect,
    measure_mode: SliceMeasureMode,
) {
    let hint = match measure_mode {
        SliceMeasureMode::Distance => {
            "Drag = pan · click 2 pts = distance · right-click clears · scroll = zoom"
        }
        SliceMeasureMode::Thickness => {
            "Drag = pan · click contour = wall thickness · right-click clears · scroll = zoom"
        }
    };
    painter.text(
        egui::pos2(panel_rect.center().x, panel_rect.bottom() - PANEL_PAD_PX),
        egui::Align2::CENTER_BOTTOM,
        hint,
        egui::FontId::proportional(9.5),
        ui_theme::TEXT_MUTED,
    );
}

#[cfg(test)]
#[path = "mapping_tests.rs"]
mod mapping_tests;
#[cfg(test)]
#[path = "panel_tests.rs"]
mod panel_tests;
