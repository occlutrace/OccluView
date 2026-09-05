// Test-only screenshot helpers (`#[cfg(test)] mod shots`): `expect`
// marks fixture/environment invariants, the same convention as the
// `#![allow(clippy::expect_used)]` test modules elsewhere.
#![allow(clippy::expect_used)]

//! Headless UI wireframe screenshots for locale layout review.
//!
//! Test-only. Renders real egui surfaces without a GPU: solid fills for
//! painted shapes, magenta outline boxes for text extents (real font
//! metrics from layout — glyph rasterization is out of scope, overflow
//! detection is the point). PNGs go to `target/i18n-shots/` (gitignored
//! build dir, never committed). A human looks at them during visual
//! review; the shape of German/Russian expansion and CJK wrapping is
//! what matters, not glyph art.

use resvg::tiny_skia;
use std::path::PathBuf;

use eframe::egui;

/// Screenshot output directory (inside the gitignored build tree).
pub(crate) fn shots_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("../../target/i18n-shots")
}

/// Tessellate a finished UI pass with the context's own fonts.
/// Drops the font-atlas delta (headless rasterizer never uploads it).
pub(crate) fn tessellate(
    ctx: &egui::Context,
    mut output: egui::FullOutput,
    pixels_per_point: f32,
) -> Vec<egui::ClippedPrimitive> {
    output.textures_delta.clear();
    ctx.tessellate(output.shapes, pixels_per_point)
}

fn to_color(color: egui::Color32) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(color.r(), color.g(), color.b(), color.a())
}

fn solid_paint(color: egui::Color32) -> tiny_skia::Paint<'static> {
    tiny_skia::Paint {
        shader: tiny_skia::Shader::SolidColor(to_color(color)),
        anti_alias: true,
        ..Default::default()
    }
}

fn triangle_path(a: egui::Pos2, b: egui::Pos2, c: egui::Pos2) -> tiny_skia::Path {
    let mut builder = tiny_skia::PathBuilder::new();
    builder.move_to(a.x, a.y);
    builder.line_to(b.x, b.y);
    builder.line_to(c.x, c.y);
    builder.close();
    builder.finish().expect("triangle path")
}

fn rect_path(rect: egui::Rect) -> tiny_skia::Path {
    let mut builder = tiny_skia::PathBuilder::new();
    builder.move_to(rect.left(), rect.top());
    builder.line_to(rect.right(), rect.top());
    builder.line_to(rect.right(), rect.bottom());
    builder.line_to(rect.left(), rect.bottom());
    builder.close();
    builder.finish().expect("rect path")
}

fn average_color(colors: &[egui::Color32; 3]) -> egui::Color32 {
    let (mut r, mut g, mut b, mut a) = (0_u32, 0_u32, 0_u32, 0_u32);
    for color in colors {
        r += u32::from(color.r());
        g += u32::from(color.g());
        b += u32::from(color.b());
        a += u32::from(color.a());
    }
    // Each channel sums three bytes (≤ 765); thirds always fit a byte.
    let third = |sum: u32| u8::try_from(sum / 3).unwrap_or(u8::MAX);
    egui::Color32::from_rgba_unmultiplied(third(r), third(g), third(b), third(a))
}
/// Paint tessellated primitives with per-run text boxes taken from
/// pre-tessellation shapes. Solid fills for vector shapes, magenta
/// outline boxes for text extents. The tessellator may merge text runs
/// into one mesh (losing per-run boxes), while shapes keep exact
/// per-run galley rects.
pub(crate) fn paint_wireframe_with_text(
    primitives: &[egui::ClippedPrimitive],
    texts: &[(egui::Rect, String)],
    width: u32,
    height: u32,
) -> tiny_skia::Pixmap {
    let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("screenshot pixmap");
    pixmap.fill(tiny_skia::Color::WHITE);
    let text_stroke = tiny_skia::Stroke {
        width: 1.0,
        ..Default::default()
    };
    let magenta = solid_paint(egui::Color32::from_rgb(200, 0, 200));
    for (rect, _) in texts {
        if rect.is_positive() {
            pixmap.stroke_path(
                &rect_path(*rect),
                &magenta,
                &text_stroke,
                tiny_skia::Transform::identity(),
                None,
            );
        }
    }
    for primitive in primitives {
        let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
            continue;
        };
        if mesh.texture_id == egui::TextureId::Managed(0) {
            continue;
        }
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let [a, b, c] = [
                mesh.vertices[triangle[0] as usize],
                mesh.vertices[triangle[1] as usize],
                mesh.vertices[triangle[2] as usize],
            ];
            let color = average_color(&[a.color, b.color, c.color]);
            if color.a() == 0 {
                continue;
            }
            pixmap.fill_path(
                &triangle_path(a.pos, b.pos, c.pos),
                &solid_paint(color),
                tiny_skia::FillRule::Winding,
                tiny_skia::Transform::identity(),
                None,
            );
        }
    }
    pixmap
}

/// Collect per-run text rects from pre-tessellation shapes.
pub(crate) fn collect_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(egui::Rect, String)> {
    fn walk(shape: &egui::epaint::Shape, out: &mut Vec<(egui::Rect, String)>) {
        match shape {
            egui::epaint::Shape::Text(text) => {
                out.push((text.visual_bounding_rect(), text.galley.text().to_owned()));
            }
            egui::epaint::Shape::Vec(shapes) => {
                for inner in shapes {
                    walk(inner, out);
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    for shaped in shapes {
        walk(&shaped.shape, &mut out);
    }
    out
}

/// Render one UI pass with per-run text boxes and save a wireframe PNG.
/// Headless tests run at 1 physical pixel per point.
pub(crate) fn save_shot_with_texts(
    name: &str,
    ctx: &egui::Context,
    output: egui::FullOutput,
    width: u32,
    height: u32,
) -> PathBuf {
    let texts = collect_texts(&output.shapes);
    let primitives = tessellate(ctx, output, 1.0);
    let pixmap = paint_wireframe_with_text(&primitives, &texts, width, height);
    let dir = shots_dir();
    std::fs::create_dir_all(&dir).expect("shots dir");
    let path = dir.join(format!("{name}.png"));
    pixmap.save_png(&path).expect("save shot");
    path
}
