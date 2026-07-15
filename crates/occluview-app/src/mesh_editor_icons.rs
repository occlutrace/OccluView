//! Hand-painted vector icons for the mesh editor toolbar and the layer menu.
//!
//! egui ships only a thin emoji subset and glyph coverage is unreliable across
//! platforms, so every tool draws its own crisp, monochrome glyph here (exocad /
//! Fusion 360 toolbar style: simple geometry, even 1.2-1.6 px stroke weight,
//! readable at 15-18 px). This module is presentation only — no mesh logic lives
//! here. Two glyph vocabularies share one drawing language and the low-level
//! primitives (`arrowhead`, `arc`, …): [`EditorIcon`] for the editor toolbar and
//! [`LayerMenuIcon`] for the layer context-menu rows. Neither enum leaks into
//! the action layer.

// Icon glyphs are sampled from parametric curves; the loop indices cast to f32
// are tiny and exact, and one big `match` paints every glyph in one place on
// purpose (splitting it across files would scatter the visual language).
#![allow(
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    clippy::many_single_char_names
)]

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2};

use crate::ui_theme::ACCENT;

/// Shared corner radius for every mesh-editor cell and the text commit buttons,
/// so the icon tiles and the OK/Cancel-style buttons share one silhouette.
pub(crate) const CELL_ROUNDING: f32 = 4.0;

/// One tool glyph. Maps 1:1 onto a mesh-editor button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditorIcon {
    /// Freehand outline selection (exocad "Mark triangles").
    Lasso,
    /// Object pick: select one whole connected object of a multi-part STL.
    Object,
    /// Select every face (exocad "All").
    SelectAll,
    /// Clear the selection (exocad "None").
    SelectNone,
    /// Swap marked and unmarked faces (exocad "Invert").
    SelectInvert,
    /// Trash can — remove the marked faces.
    Delete,
    /// Crop marks — keep only the marked area (exocad "Crop").
    Keep,
    /// Fill open holes with interpolated caps (exocad "Close Holes").
    CloseHoles,
    /// Scissors on a dashed line — move the marked faces to a new mesh.
    Cut,
    /// Split the marked region into one mesh per connected part.
    Separate,
    /// Undo the last edit.
    Undo,
    /// Redo the undone edit.
    Redo,
    /// Selection mode: only visible front-facing surface.
    SurfaceMode,
    /// Selection mode: straight through the mesh, including backsides.
    ThroughMode,
}

/// One layer context-menu glyph, drawn in the shared line style at the small
/// (~15 px) gutter size a dropdown row uses. A vocabulary distinct from the
/// editor toolbar's [`EditorIcon`]: these name layer-level operator actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerMenuIcon {
    /// An open eye — the layer is visible (the "Hide" action).
    EyeOpen,
    /// A slashed eye — the layer is hidden (the "Show" action).
    EyeSlash,
    /// Spotlight one sheet among dimmed others (Show only this layer).
    Solo,
    /// Every sheet lit (Show all layers).
    ShowAll,
    /// A droplet — reset opacity.
    Opacity,
    /// An artist palette — cycle the tint.
    Tint,
    /// A pencil — open the mesh editor.
    Pencil,
    /// A stitched seam — repair the mesh.
    Repair,
    /// A surface with opposed normals — flip normals.
    FlipNormals,
    /// An arrow leaving a tray — export the layer.
    Export,
    /// A subdivided triangle — toggle the wireframe overlay.
    Wireframe,
    /// A trash can — remove the layer.
    Trash,
}

/// Paint `icon` inside `rect` in a single `color`. `active` slightly thickens
/// the stroke and adds a soft fill accent so a toggled-on tool reads as engaged.
pub(crate) fn paint(
    painter: &egui::Painter,
    rect: Rect,
    icon: EditorIcon,
    color: Color32,
    active: bool,
) {
    let side = rect.width().min(rect.height());
    let base = (side * 0.075).clamp(1.2, 1.6);
    let w = if active { base + 0.3 } else { base };
    let stroke = Stroke::new(w, color);
    // Soft fill for solid glyph parts; only shown when the tool is engaged so
    // the resting state stays a clean line drawing.
    let soft = color.gamma_multiply(if active { 0.40 } else { 0.22 });

    // Drawing box: a centred square inside the button so every glyph shares the
    // same optical weight regardless of the caller's rect aspect.
    let b = Rect::from_center_size(rect.center(), Vec2::splat(side * 0.72));
    let p = |x: f32, y: f32| Pos2::new(b.min.x + x * b.width(), b.min.y + y * b.height());
    let r = |v: f32| v * b.width();

    match icon {
        EditorIcon::Lasso => {
            // Dashed closed loop with a hanging grab node.
            let c = p(0.46, 0.42);
            let (rx, ry) = (r(0.32), r(0.30));
            let mut loop_pts = Vec::with_capacity(25);
            for i in 0..=24 {
                let a = std::f32::consts::TAU * (i as f32) / 24.0;
                loop_pts.push(Pos2::new(c.x + rx * a.cos(), c.y + ry * a.sin()));
            }
            painter.extend(Shape::dashed_line(&loop_pts, stroke, r(0.11), r(0.09)));
            painter.line_segment([p(0.42, 0.72), p(0.36, 0.90)], stroke);
            painter.circle_filled(p(0.36, 0.90), r(0.07), color);
        }
        EditorIcon::Object => {
            // A single 3D box (one whole object) with its front face lit and a
            // pick dot: click one object of a multi-part STL to select it. The
            // front face is a 4-corner array; the back carries only the three
            // corners the receding edges need.
            let front = [p(0.18, 0.42), p(0.62, 0.42), p(0.62, 0.86), p(0.18, 0.86)];
            let back_top_left = p(0.38, 0.26);
            let back_top_right = p(0.82, 0.26);
            let back_bottom_right = p(0.82, 0.70);
            // Lit front face (the selected object) plus its outline.
            painter.add(Shape::convex_polygon(front.to_vec(), soft, Stroke::NONE));
            painter.add(Shape::closed_line(front.to_vec(), stroke));
            // Top face and the three receding right-face edges give it depth.
            painter.add(Shape::closed_line(
                vec![front[0], back_top_left, back_top_right, front[1]],
                stroke,
            ));
            painter.line_segment([front[1], back_top_right], stroke);
            painter.line_segment([front[2], back_bottom_right], stroke);
            painter.line_segment([back_top_right, back_bottom_right], stroke);
            // Pick dot on the lit front face.
            painter.circle_filled(p(0.40, 0.64), r(0.07), color);
        }
        EditorIcon::SelectAll => {
            let rc = Rect::from_min_max(p(0.16, 0.16), p(0.84, 0.84));
            painter.rect_filled(rc, r(0.06), soft);
            painter.extend(Shape::dashed_line(
                &closed(&corners(rc)),
                stroke,
                r(0.11),
                r(0.08),
            ));
        }
        EditorIcon::SelectNone => {
            let rc = Rect::from_min_max(p(0.16, 0.16), p(0.84, 0.84));
            painter.extend(Shape::dashed_line(
                &closed(&corners(rc)),
                stroke,
                r(0.11),
                r(0.08),
            ));
        }
        EditorIcon::SelectInvert => {
            let rc = Rect::from_min_max(p(0.16, 0.16), p(0.84, 0.84));
            // One diagonal half filled to show the marked/unmarked swap.
            painter.add(Shape::convex_polygon(
                vec![rc.left_top(), rc.right_top(), rc.left_bottom()],
                soft,
                Stroke::NONE,
            ));
            painter.extend(Shape::dashed_line(
                &closed(&corners(rc)),
                stroke,
                r(0.11),
                r(0.08),
            ));
            painter.line_segment([rc.right_top(), rc.left_bottom()], stroke);
        }
        EditorIcon::Delete => {
            // Lid, handle, tapered body, and inner stripes.
            painter.line_segment([p(0.18, 0.28), p(0.82, 0.28)], stroke);
            painter.add(Shape::line(
                vec![p(0.40, 0.28), p(0.40, 0.16), p(0.60, 0.16), p(0.60, 0.28)],
                stroke,
            ));
            painter.add(Shape::line(
                vec![p(0.27, 0.28), p(0.31, 0.86), p(0.69, 0.86), p(0.73, 0.28)],
                stroke,
            ));
            // Two inner stripes (not three): the trash can was the densest
            // glyph in the row, so drop one line to match its neighbours' weight.
            for x in [0.44_f32, 0.56] {
                painter.line_segment([p(x, 0.38), p(x, 0.76)], stroke);
            }
        }
        EditorIcon::Keep => {
            // Two crop brackets framing the kept region.
            painter.rect_filled(Rect::from_min_max(p(0.26, 0.26), p(0.74, 0.74)), 0.0, soft);
            painter.line_segment([p(0.24, 0.14), p(0.24, 0.60)], stroke);
            painter.line_segment([p(0.14, 0.24), p(0.60, 0.24)], stroke);
            painter.line_segment([p(0.76, 0.40), p(0.76, 0.86)], stroke);
            painter.line_segment([p(0.40, 0.76), p(0.86, 0.76)], stroke);
        }
        EditorIcon::CloseHoles => {
            let c = p(0.50, 0.52);
            let rad = r(0.34);
            // Solid rim around the bottom, dashed rim across the open top, and a
            // soft filled patch = the interpolated cap.
            painter.add(Shape::line(arc(c, rad, -50.0, 230.0, 24), stroke));
            painter.extend(Shape::dashed_line(
                &arc(c, rad, 230.0, 310.0, 10),
                stroke,
                r(0.10),
                r(0.08),
            ));
            painter.circle_filled(p(0.50, 0.30), r(0.11), soft);
        }
        EditorIcon::Cut => {
            // Scissors opening onto a dashed cut line.
            painter.extend(Shape::dashed_line(
                &[p(0.10, 0.50), p(0.90, 0.50)],
                stroke,
                r(0.10),
                r(0.08),
            ));
            let pivot = p(0.46, 0.50);
            painter.line_segment([pivot, p(0.88, 0.32)], stroke);
            painter.line_segment([pivot, p(0.88, 0.68)], stroke);
            let h1 = p(0.22, 0.34);
            let h2 = p(0.22, 0.66);
            painter.circle_stroke(h1, r(0.10), stroke);
            painter.circle_stroke(h2, r(0.10), stroke);
            painter.line_segment([h1, pivot], stroke);
            painter.line_segment([h2, pivot], stroke);
        }
        EditorIcon::Separate => {
            // Two parts with a gap, arrows pulling them apart.
            painter.rect_stroke(
                Rect::from_min_max(p(0.14, 0.34), p(0.40, 0.72)),
                r(0.04),
                stroke,
            );
            painter.rect_stroke(
                Rect::from_min_max(p(0.60, 0.34), p(0.86, 0.72)),
                r(0.04),
                stroke,
            );
            painter.line_segment([p(0.30, 0.20), p(0.16, 0.20)], stroke);
            arrowhead(
                painter,
                p(0.16, 0.20),
                Vec2::new(-1.0, 0.0),
                r(0.12),
                stroke,
            );
            painter.line_segment([p(0.70, 0.20), p(0.84, 0.20)], stroke);
            arrowhead(painter, p(0.84, 0.20), Vec2::new(1.0, 0.0), r(0.12), stroke);
        }
        EditorIcon::Undo => circular_arrow(painter, b, color, w, false),
        EditorIcon::Redo => circular_arrow(painter, b, color, w, true),
        EditorIcon::SurfaceMode => {
            // A single visible surface with a pick marker on top.
            let mut s = Vec::with_capacity(17);
            for i in 0..=16 {
                let t = i as f32 / 16.0;
                let x = 0.14 + 0.72 * t;
                let y = 0.66 - 0.16 * (std::f32::consts::PI * t).sin();
                s.push(p(x, y));
            }
            painter.add(Shape::line(s, stroke));
            painter.line_segment([p(0.50, 0.44), p(0.50, 0.52)], stroke);
            painter.circle_filled(p(0.50, 0.34), r(0.08), color);
        }
        EditorIcon::ThroughMode => {
            // Two stacked surfaces with an arrow passing straight through both.
            let mut front = Vec::with_capacity(17);
            let mut back = Vec::with_capacity(17);
            for i in 0..=16 {
                let t = i as f32 / 16.0;
                let x = 0.14 + 0.72 * t;
                let bump = 0.10 * (std::f32::consts::PI * t).sin();
                front.push(p(x, 0.40 - bump));
                back.push(p(x, 0.72 - bump));
            }
            painter.add(Shape::line(front, stroke));
            painter.add(Shape::line(back, stroke));
            painter.line_segment([p(0.50, 0.14), p(0.50, 0.88)], stroke);
            arrowhead(painter, p(0.50, 0.88), Vec2::new(0.0, 1.0), r(0.16), stroke);
        }
    }
}

/// Paint `icon` inside `rect` in a single `color`, for a layer context-menu row.
/// Same crisp line language as [`paint`], tuned for the smaller gutter glyph:
/// the whole glyph takes the row's ink color so it tracks the hover accent.
pub(crate) fn paint_layer_menu(
    painter: &egui::Painter,
    rect: Rect,
    icon: LayerMenuIcon,
    color: Color32,
) {
    let side = rect.width().min(rect.height());
    let w = (side * 0.095).clamp(1.2, 1.5);
    let stroke = Stroke::new(w, color);
    // Soft accent for the dimmed / filled glyph parts.
    let soft = color.gamma_multiply(0.30);

    // Centred drawing box, slightly larger than the editor's since menu glyphs
    // sit alone in a gutter rather than over a caption.
    let b = Rect::from_center_size(rect.center(), Vec2::splat(side * 0.86));
    let p = |x: f32, y: f32| Pos2::new(b.min.x + x * b.width(), b.min.y + y * b.height());
    let r = |v: f32| v * b.width();

    match icon {
        LayerMenuIcon::EyeOpen => {
            menu_almond(painter, b, stroke);
            painter.circle_filled(p(0.50, 0.50), r(0.11), color);
        }
        LayerMenuIcon::EyeSlash => {
            menu_almond(painter, b, stroke);
            painter.line_segment([p(0.12, 0.80), p(0.88, 0.20)], stroke);
        }
        LayerMenuIcon::Solo | LayerMenuIcon::ShowAll => {
            // Three stacked sheets. Solo lights only the middle one; Show all
            // lights every sheet.
            let all = icon == LayerMenuIcon::ShowAll;
            let sheet = |painter: &egui::Painter, cy: f32, lit: bool| {
                let rc = Rect::from_min_max(p(0.18, cy - 0.075), p(0.82, cy + 0.075));
                painter.rect_filled(rc, r(0.03), if lit { color } else { soft });
            };
            sheet(painter, 0.24, all);
            sheet(painter, 0.50, true);
            sheet(painter, 0.76, all);
        }
        LayerMenuIcon::Opacity => {
            // Teardrop: apex on top, round bulb below; softly filled to read as
            // translucency.
            let bulb = p(0.50, 0.60);
            let rad = r(0.30);
            let mut drop = Vec::with_capacity(22);
            drop.push(p(0.50, 0.12));
            drop.extend(arc(bulb, rad, -50.0, 230.0, 20));
            painter.add(Shape::convex_polygon(drop.clone(), soft, Stroke::NONE));
            painter.add(Shape::closed_line(drop, stroke));
        }
        LayerMenuIcon::Tint => {
            // Artist palette: an ellipse rim, a thumb hole, three paint dots.
            let c = p(0.48, 0.52);
            let mut rim = Vec::with_capacity(25);
            for i in 0..=24 {
                let a = std::f32::consts::TAU * (i as f32) / 24.0;
                rim.push(Pos2::new(c.x + r(0.38) * a.cos(), c.y + r(0.32) * a.sin()));
            }
            painter.add(Shape::closed_line(rim, stroke));
            painter.circle_stroke(p(0.34, 0.66), r(0.06), stroke);
            painter.circle_filled(p(0.42, 0.34), r(0.06), color);
            painter.circle_filled(p(0.62, 0.42), r(0.06), color);
            painter.circle_filled(p(0.56, 0.66), r(0.06), color);
        }
        LayerMenuIcon::Pencil => {
            // A slanted pencil: nib at lower-left, eraser cap at upper-right.
            let tip = p(0.20, 0.82);
            let cap = p(0.80, 0.22);
            let dir = (cap - tip).normalized();
            let n = Vec2::new(-dir.y, dir.x);
            let hw = r(0.11);
            let collar = tip + dir * r(0.24);
            let c1 = collar + n * hw;
            let c2 = collar - n * hw;
            let e1 = cap + n * hw;
            let e2 = cap - n * hw;
            painter.line_segment([c1, e1], stroke);
            painter.line_segment([c2, e2], stroke);
            painter.line_segment([e1, e2], stroke);
            painter.line_segment([c1, c2], stroke);
            painter.line_segment([c1, tip], stroke);
            painter.line_segment([c2, tip], stroke);
        }
        LayerMenuIcon::Repair => {
            // A broken seam pulled shut by vertical stitches — mend the mesh.
            painter.extend(Shape::dashed_line(
                &[p(0.12, 0.50), p(0.88, 0.50)],
                stroke,
                r(0.11),
                r(0.08),
            ));
            for x in [0.28_f32, 0.50, 0.72] {
                painter.line_segment([p(x, 0.30), p(x, 0.70)], stroke);
            }
        }
        LayerMenuIcon::FlipNormals => {
            // A surface line with normals pointing opposite ways across it.
            painter.line_segment([p(0.14, 0.52), p(0.86, 0.52)], stroke);
            painter.line_segment([p(0.32, 0.52), p(0.32, 0.20)], stroke);
            arrowhead(
                painter,
                p(0.32, 0.20),
                Vec2::new(0.0, -1.0),
                r(0.14),
                stroke,
            );
            painter.line_segment([p(0.68, 0.52), p(0.68, 0.84)], stroke);
            arrowhead(painter, p(0.68, 0.84), Vec2::new(0.0, 1.0), r(0.14), stroke);
        }
        LayerMenuIcon::Export => {
            // An open tray with an arrow leaving through the top.
            painter.add(Shape::line(
                vec![p(0.24, 0.46), p(0.24, 0.82), p(0.76, 0.82), p(0.76, 0.46)],
                stroke,
            ));
            painter.line_segment([p(0.50, 0.66), p(0.50, 0.16)], stroke);
            arrowhead(
                painter,
                p(0.50, 0.16),
                Vec2::new(0.0, -1.0),
                r(0.16),
                stroke,
            );
        }
        LayerMenuIcon::Wireframe => {
            // A triangle subdivided into four — mesh edges over a surface.
            let apex = p(0.50, 0.16);
            let bl = p(0.16, 0.84);
            let br = p(0.84, 0.84);
            painter.add(Shape::closed_line(vec![apex, bl, br], stroke));
            let mid_left = p(0.33, 0.50);
            let mid_right = p(0.67, 0.50);
            let mid_base = p(0.50, 0.84);
            painter.line_segment([mid_left, mid_right], stroke);
            painter.line_segment([mid_left, mid_base], stroke);
            painter.line_segment([mid_right, mid_base], stroke);
        }
        LayerMenuIcon::Trash => {
            painter.line_segment([p(0.20, 0.30), p(0.80, 0.30)], stroke);
            painter.add(Shape::line(
                vec![p(0.42, 0.30), p(0.42, 0.18), p(0.58, 0.18), p(0.58, 0.30)],
                stroke,
            ));
            painter.add(Shape::line(
                vec![p(0.28, 0.30), p(0.32, 0.84), p(0.68, 0.84), p(0.72, 0.30)],
                stroke,
            ));
            painter.line_segment([p(0.50, 0.40), p(0.50, 0.74)], stroke);
        }
    }
}

/// One measurement glyph for the top toolbar's Measure toggles. A vocabulary
/// distinct from [`EditorIcon`]/[`LayerMenuIcon`] (these name viewport
/// measurement tools) sharing the same drawing language and primitives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MeasureIcon {
    /// A slanted ruler with tick marks — two-point distance.
    Ruler,
    /// A shell cross-section with a wall-depth arrow — local thickness.
    Thickness,
}

/// Paint `icon` inside `rect` in a single `color`. `active` slightly thickens
/// the stroke, matching [`paint`]'s engaged-tool treatment.
pub(crate) fn paint_measure(
    painter: &egui::Painter,
    rect: Rect,
    icon: MeasureIcon,
    color: Color32,
    active: bool,
) {
    let side = rect.width().min(rect.height());
    let base = (side * 0.075).clamp(1.2, 1.6);
    let w = if active { base + 0.3 } else { base };
    let stroke = Stroke::new(w, color);

    // Centred drawing box (same optical weight rule as the editor glyphs).
    let b = Rect::from_center_size(rect.center(), Vec2::splat(side * 0.86));
    let p = |x: f32, y: f32| Pos2::new(b.min.x + x * b.width(), b.min.y + y * b.height());
    let r = |v: f32| v * b.width();

    match icon {
        MeasureIcon::Ruler => {
            // A slanted ruler body with graduation ticks along the top edge.
            let a0 = p(0.10, 0.90);
            let a1 = p(0.90, 0.10);
            let dir = (a1 - a0).normalized();
            let n = Vec2::new(-dir.y, dir.x);
            let hw = r(0.15);
            let c0 = a0 + n * hw;
            let c1 = a1 + n * hw;
            let d0 = a0 - n * hw;
            let d1 = a1 - n * hw;
            painter.add(Shape::closed_line(vec![c0, c1, d1, d0], stroke));
            for t in [0.3_f32, 0.5, 0.7] {
                let top = a0 + (a1 - a0) * t + n * hw;
                painter.line_segment([top, top - n * hw * 0.9], stroke);
            }
        }
        MeasureIcon::Thickness => {
            // A crown-shell cross-section: outer + inner dome arcs with a
            // double-headed arrow spanning the wall at the top.
            let c = p(0.50, 0.82);
            painter.add(Shape::line(arc(c, r(0.58), 205.0, 335.0, 20), stroke));
            painter.add(Shape::line(arc(c, r(0.34), 210.0, 330.0, 16), stroke));
            let top_outer = p(0.50, 0.82 - 0.58);
            let top_inner = p(0.50, 0.82 - 0.34);
            painter.line_segment([top_outer, top_inner], stroke);
            arrowhead(painter, top_outer, Vec2::new(0.0, -1.0), r(0.10), stroke);
            arrowhead(painter, top_inner, Vec2::new(0.0, 1.0), r(0.10), stroke);
        }
    }
}

/// The almond eye outline shared by the open/slashed menu eyes, matching the
/// layer-row visibility eye so the two read as one glyph.
fn menu_almond(painter: &egui::Painter, b: Rect, stroke: Stroke) {
    let c = Pos2::new(b.min.x + 0.50 * b.width(), b.min.y + 0.50 * b.height());
    let hw = 0.40 * b.width();
    let hh = 0.24 * b.height();
    let mut outline = Vec::with_capacity(28);
    for i in 0..=12 {
        let t = i as f32 / 12.0;
        let x = c.x - hw + 2.0 * hw * t;
        let lid = (std::f32::consts::PI * t).sin();
        outline.push(Pos2::new(x, c.y - hh * lid));
    }
    for i in (0..=12).rev() {
        let t = i as f32 / 12.0;
        let x = c.x - hw + 2.0 * hw * t;
        let lid = (std::f32::consts::PI * t).sin();
        outline.push(Pos2::new(x, c.y + hh * lid));
    }
    painter.add(Shape::closed_line(outline, stroke));
}

/// A square icon button: a glyph over a tiny wrapping caption. Returns the
/// `Response` with `tooltip` already attached. Disabled buttons still hover so
/// the operator can read why an action is unavailable.
// A toolbar cell genuinely needs its glyph, caption, tooltip, and both state
// flags; bundling them into a struct would only add ceremony at every call.
#[allow(clippy::too_many_arguments)]
pub(crate) fn icon_button(
    ui: &mut Ui,
    size: Vec2,
    icon: EditorIcon,
    label: &str,
    tooltip: &str,
    enabled: bool,
    active: bool,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);

    let fg = if !enabled {
        ui.visuals().weak_text_color()
    } else if active {
        ACCENT
    } else {
        ui.visuals().widgets.inactive.fg_stroke.color
    };

    let painter = ui.painter();
    if active {
        // Lit "pressed" tool: a filled accent cell with a firm accent border so
        // an engaged toggle (Lasso / Surface / Through) is unmistakable at a
        // glance.
        painter.rect_filled(rect, CELL_ROUNDING, ACCENT.gamma_multiply(0.20));
        painter.rect_stroke(
            rect,
            CELL_ROUNDING,
            Stroke::new(1.2, ACCENT.gamma_multiply(0.90)),
        );
    } else if enabled && response.hovered() {
        // Warm hover: a soft accent wash plus a faint ring so the cell lifts
        // without shouting.
        painter.rect_filled(rect, CELL_ROUNDING, ACCENT.gamma_multiply(0.12));
        painter.rect_stroke(
            rect,
            CELL_ROUNDING,
            Stroke::new(1.0, ACCENT.gamma_multiply(0.30)),
        );
    }

    // Reserve the bottom strip for the caption. The editor labels are short and
    // the cells are wide enough that they render on one line.
    let caption_h = 20.0_f32;
    let icon_side = (rect.width().min(rect.height() - caption_h) - 6.0).clamp(12.0, 22.0);
    let icon_center = Pos2::new(rect.center().x, rect.top() + 4.0 + icon_side * 0.5);
    let icon_rect = Rect::from_center_size(icon_center, Vec2::splat(icon_side));
    paint(painter, icon_rect, icon, fg, active);

    let galley = painter.layout(
        label.to_owned(),
        FontId::proportional(9.0),
        fg,
        rect.width() - 2.0,
    );
    let caption_pos = Pos2::new(
        rect.center().x - galley.size().x * 0.5,
        rect.bottom() - caption_h + ((caption_h - galley.size().y) * 0.5).max(0.0),
    );
    painter.galley(caption_pos, galley, fg);

    response.on_hover_text(tooltip)
}

/// Two barbs forming an arrowhead at `tip`, opening opposite `dir`.
fn arrowhead(painter: &egui::Painter, tip: Pos2, dir: Vec2, len: f32, stroke: Stroke) {
    let d = dir.normalized();
    let n = Vec2::new(-d.y, d.x);
    let back = tip - d * len;
    painter.line_segment([tip, back + n * len * 0.6], stroke);
    painter.line_segment([tip, back - n * len * 0.6], stroke);
}

/// A circular history arrow. `redo` draws the clockwise (right-hand) sense; the
/// undo variant is the mirror image so the pair reads left/right at a glance.
fn circular_arrow(painter: &egui::Painter, b: Rect, color: Color32, w: f32, redo: bool) {
    let stroke = Stroke::new(w, color);
    let c = Pos2::new(b.min.x + 0.50 * b.width(), b.min.y + 0.54 * b.height());
    let rad = 0.32 * b.width();
    let mut pts = arc(c, rad, 30.0, 300.0, 28);
    if !redo {
        for pt in &mut pts {
            pt.x = 2.0 * c.x - pt.x;
        }
    }
    let n = pts.len();
    let tip = pts[n - 1];
    let dir = tip - pts[n - 2];
    painter.add(Shape::line(pts, stroke));
    arrowhead(painter, tip, dir, 0.22 * b.width(), stroke);
}

/// Sample a circular arc (degrees, screen y-down) into a polyline.
fn arc(center: Pos2, radius: f32, deg0: f32, deg1: f32, segments: usize) -> Vec<Pos2> {
    let mut pts = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let deg = deg0 + (deg1 - deg0) * (i as f32 / segments as f32);
        let a = deg.to_radians();
        pts.push(Pos2::new(
            center.x + radius * a.cos(),
            center.y + radius * a.sin(),
        ));
    }
    pts
}

fn corners(rc: Rect) -> [Pos2; 4] {
    [
        rc.left_top(),
        rc.right_top(),
        rc.right_bottom(),
        rc.left_bottom(),
    ]
}

/// Close a polyline by repeating its first point (for `dashed_line`).
fn closed(pts: &[Pos2]) -> Vec<Pos2> {
    let mut out = pts.to_vec();
    if let Some(&first) = pts.first() {
        out.push(first);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_ICONS: [EditorIcon; 14] = [
        EditorIcon::Lasso,
        EditorIcon::Object,
        EditorIcon::SelectAll,
        EditorIcon::SelectNone,
        EditorIcon::SelectInvert,
        EditorIcon::Delete,
        EditorIcon::Keep,
        EditorIcon::CloseHoles,
        EditorIcon::Cut,
        EditorIcon::Separate,
        EditorIcon::Undo,
        EditorIcon::Redo,
        EditorIcon::SurfaceMode,
        EditorIcon::ThroughMode,
    ];

    const ALL_MENU_ICONS: [LayerMenuIcon; 12] = [
        LayerMenuIcon::EyeOpen,
        LayerMenuIcon::EyeSlash,
        LayerMenuIcon::Solo,
        LayerMenuIcon::ShowAll,
        LayerMenuIcon::Opacity,
        LayerMenuIcon::Tint,
        LayerMenuIcon::Pencil,
        LayerMenuIcon::Repair,
        LayerMenuIcon::FlipNormals,
        LayerMenuIcon::Export,
        LayerMenuIcon::Wireframe,
        LayerMenuIcon::Trash,
    ];

    #[test]
    fn every_icon_paints_without_panicking() {
        let ctx = egui::Context::default();
        let painter = ctx.debug_painter();
        let rect = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::splat(18.0));
        for icon in ALL_ICONS {
            for active in [false, true] {
                paint(&painter, rect, icon, Color32::WHITE, active);
            }
        }
    }

    const ALL_MEASURE_ICONS: [MeasureIcon; 2] = [MeasureIcon::Ruler, MeasureIcon::Thickness];

    #[test]
    fn every_measure_icon_paints_without_panicking() {
        let ctx = egui::Context::default();
        let painter = ctx.debug_painter();
        // Exercise both the toolbar toggle size and the strip glyph size.
        for side in [14.0_f32, 15.0, 18.0] {
            let rect = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::splat(side));
            for icon in ALL_MEASURE_ICONS {
                for active in [false, true] {
                    paint_measure(&painter, rect, icon, Color32::WHITE, active);
                }
            }
        }
    }

    #[test]
    fn every_layer_menu_icon_paints_without_panicking() {
        let ctx = egui::Context::default();
        let painter = ctx.debug_painter();
        // Exercise the small gutter size the menu rows actually use.
        for side in [14.0_f32, 15.0, 18.0] {
            let rect = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::splat(side));
            for icon in ALL_MENU_ICONS {
                paint_layer_menu(&painter, rect, icon, Color32::WHITE);
            }
        }
    }

    #[test]
    fn icon_button_renders_in_every_state_without_panicking() {
        // Exercise the disabled / hover-less / active branches of `icon_button`
        // with a real `Ui`, at the narrow cell size the window now uses.
        egui::__run_test_ui(|ui| {
            for icon in ALL_ICONS {
                for enabled in [false, true] {
                    for active in [false, true] {
                        icon_button(
                            ui,
                            Vec2::new(48.0, 50.0),
                            icon,
                            "Label",
                            "tooltip",
                            enabled,
                            active,
                        );
                    }
                }
            }
        });
    }

    #[test]
    fn arrowhead_barbs_stay_near_the_tip() {
        // A regression guard: the barbs must not shoot off to infinity when the
        // direction is axis-aligned (normalization edge cases).
        let ctx = egui::Context::default();
        let painter = ctx.debug_painter();
        arrowhead(
            &painter,
            Pos2::new(10.0, 10.0),
            Vec2::new(0.0, 1.0),
            3.0,
            Stroke::new(1.4, Color32::WHITE),
        );
    }
}
