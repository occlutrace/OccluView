//! Layer context-menu icon vocabulary (split out of `mesh_editor_icons.rs`
//! to hold the file-size budget -- see its module doc for the shared drawing
//! language and the sibling `EditorIcon`/`MeasureIcon` vocabularies).

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, Vec2};

use super::{arc, arrowhead};

/// One layer context-menu glyph, drawn in the shared line style at the small
/// (~15 px) gutter size a dropdown row uses. A vocabulary distinct from the
/// editor toolbar vocabulary: these name layer-level operator actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
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
    /// A filled dot — scan colors are shown (the "Hide scan colors" action).
    ColorsOn,
    /// A slashed dot — scan colors are hidden, showing the neutral material
    /// (the "Show scan colors" action).
    ColorsOff,
    /// A checker tile — an attached texture is currently sampled.
    TextureOn,
    /// A crossed checker tile — texture sampling is disabled.
    TextureOff,
    /// A trash can — remove the layer.
    Trash,
}

/// Paint `icon` inside `rect` in a single `color`, for a layer context-menu row.
/// Uses the same crisp line language as the editor toolbar, tuned for the
/// smaller gutter glyph:
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
        LayerMenuIcon::ColorsOn => {
            // A filled material dot with a soft outer ring — distinct from the
            // Tint palette (this is on/off, not a color picker).
            painter.circle_stroke(p(0.50, 0.50), r(0.36), stroke);
            painter.circle_filled(p(0.50, 0.50), r(0.20), color);
        }
        LayerMenuIcon::ColorsOff => {
            // The same dot, hollow, slashed — mirrors the EyeSlash pairing.
            painter.circle_stroke(p(0.50, 0.50), r(0.36), stroke);
            painter.line_segment([p(0.16, 0.84), p(0.84, 0.16)], stroke);
        }
        LayerMenuIcon::TextureOn | LayerMenuIcon::TextureOff => {
            let tile = Rect::from_min_max(p(0.18, 0.18), p(0.82, 0.82));
            painter.rect_stroke(tile, r(0.05), stroke);
            for (x, y) in [(0.32, 0.32), (0.58, 0.32), (0.32, 0.58), (0.58, 0.58)] {
                painter.rect_filled(
                    Rect::from_min_max(p(x, y), p(x + 0.14, y + 0.14)),
                    r(0.01),
                    soft,
                );
            }
            if icon == LayerMenuIcon::TextureOff {
                painter.line_segment([p(0.16, 0.16), p(0.84, 0.84)], stroke);
            }
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
#[cfg(test)]
mod tests {
    use super::*;

    const ALL_MENU_ICONS: [LayerMenuIcon; 16] = [
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
        LayerMenuIcon::ColorsOn,
        LayerMenuIcon::ColorsOff,
        LayerMenuIcon::TextureOn,
        LayerMenuIcon::TextureOff,
        LayerMenuIcon::Trash,
    ];

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
}
