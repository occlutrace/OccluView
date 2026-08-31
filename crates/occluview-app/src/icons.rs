//! Shared Lucide icon renderer.
//!
//! SVGs are vendored under the ISC license and rasterized per egui context.

use eframe::egui;
use std::collections::HashMap;

/// Icons used by application controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AppIcon {
    // Chrome / dialogs
    Open,
    Add,
    Settings,
    Close,
    Check,
    ChevronDown,
    Warn,
    Error,
    Github,
    Globe,
    Licenses,
    InstallUpdate,
    Export,
    // Layers / context menu
    Palette,
    ScanColors,
    ScanColorsOff,
    Texture,
    TextureOff,
    EditMesh,
    Repair,
    FlipNormals,
    BridgeSplit,
    Wireframe,
    Trash,
    Eye,
    EyeOff,
    FitView,
    // Measure toolbar
    Ruler,
    Thickness,
    Align,
    // Mesh editor cells
    Lasso,
    Object,
    SelectAll,
    SelectNone,
    SelectInvert,
    Delete,
    Keep,
    CloseHoles,
    Smooth,
    SculptAdd,
    Cut,
    Separate,
    Undo,
    Redo,
    SurfaceMode,
    ThroughMode,
    AlignFit,
    AlignRefine,
    Heatmap,
    MaskBrush,
    MoveLayer,
    MoveVertical,
    MovePlane,
}

impl AppIcon {
    /// The vendored Lucide SVG for this icon.
    fn svg(self) -> &'static [u8] {
        macro_rules! lucide {
            ($file:literal) => {
                include_bytes!(concat!("../assets/icons/", $file, ".svg"))
            };
        }
        match self {
            Self::Open => lucide!("folder-open"),
            Self::Add => lucide!("file-plus-2"),
            Self::Settings => lucide!("settings"),
            Self::Close => lucide!("x"),
            Self::Check => lucide!("check"),
            Self::ChevronDown => lucide!("chevron-down"),
            Self::Warn => lucide!("triangle-alert"),
            Self::Error => lucide!("circle-alert"),
            Self::Github => lucide!("github"),
            Self::Globe => lucide!("globe"),
            Self::Licenses => lucide!("scroll-text"),
            Self::InstallUpdate => lucide!("hard-drive-download"),
            Self::Export => lucide!("upload"),
            Self::Palette => lucide!("palette"),
            Self::ScanColors => lucide!("droplet"),
            Self::ScanColorsOff => lucide!("droplet-off"),
            Self::Texture => lucide!("image"),
            Self::TextureOff => lucide!("image-off"),
            Self::EditMesh => lucide!("pencil"),
            Self::Repair => lucide!("wrench"),
            Self::FlipNormals => lucide!("refresh-ccw"),
            Self::BridgeSplit => lucide!("unplug"),
            Self::Wireframe => lucide!("box"),
            Self::Trash | Self::Delete => lucide!("trash-2"),
            Self::Eye => lucide!("eye"),
            Self::EyeOff => lucide!("eye-off"),
            Self::FitView => lucide!("maximize-2"),
            Self::Ruler => lucide!("ruler"),
            Self::Thickness => lucide!("proportions"),
            Self::Align => lucide!("combine"),
            Self::Lasso => lucide!("lasso"),
            Self::Object => lucide!("square-mouse-pointer"),
            Self::SelectAll => lucide!("square-check-big"),
            Self::SelectNone => lucide!("square-dashed"),
            Self::SelectInvert => lucide!("contrast"),
            Self::Keep => lucide!("crop"),
            Self::CloseHoles => lucide!("circle-dashed"),
            Self::Smooth => lucide!("waves"),
            Self::SculptAdd => lucide!("circle-plus"),
            Self::Cut => lucide!("scissors"),
            Self::Separate => lucide!("split"),
            Self::Undo => lucide!("undo-2"),
            Self::Redo => lucide!("redo-2"),
            Self::SurfaceMode => lucide!("layers"),
            Self::ThroughMode => lucide!("layers-3"),
            Self::AlignFit => lucide!("spline"),
            Self::AlignRefine => lucide!("magnet"),
            Self::Heatmap => lucide!("thermometer"),
            Self::MaskBrush => lucide!("brush"),
            Self::MoveLayer => lucide!("move"),
            Self::MoveVertical => lucide!("move-vertical"),
            Self::MovePlane => lucide!("move-3d"),
        }
    }
}

/// Rasterize an icon to a straight-alpha RGBA image at `px` square, inked in
/// `color`. Lucide draws with `stroke="currentColor"`, so recoloring is a
/// text substitution before parsing.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn rasterize(icon: AppIcon, px: usize, color: egui::Color32) -> Option<egui::ColorImage> {
    let svg = std::str::from_utf8(icon.svg()).ok()?;
    let hex = format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b());
    let colored = svg.replace("currentColor", &hex);
    let tree = usvg_tree(&colored)?;
    let svg_size = tree.size();
    let scale = px as f32 / svg_size.width();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px as u32, px as u32)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia stores premultiplied alpha; egui wants straight alpha.
    let mut rgba = pixmap.data().to_vec();
    for p in rgba.chunks_exact_mut(4) {
        let a = u16::from(p[3]);
        if a > 0 {
            for c in &mut p[..3] {
                *c = ((u16::from(*c) * 255) / a) as u8;
            }
        }
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([px, px], &rgba))
}

/// Parse an SVG using the version of usvg re-exported by resvg.
fn usvg_tree(svg: &str) -> Option<resvg::usvg::Tree> {
    resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).ok()
}

/// Cache key: icon, physical pixel size, and ink color.
type TextureCacheKey = (AppIcon, u32, [u8; 3]);

#[derive(Clone, Default)]
struct TextureCache(HashMap<TextureCacheKey, egui::TextureHandle>);

const TEXTURE_CACHE_ID: &str = "occluview-icon-textures";

/// Texture for `icon` at `logical_size` points, inked in `color`, rasterized
/// at the display's physical resolution, so it stays crisp on `HiDPI`.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub(crate) fn texture(
    ctx: &egui::Context,
    icon: AppIcon,
    logical_size: f32,
    color: egui::Color32,
) -> egui::TextureHandle {
    let ppi = ctx.pixels_per_point();
    let px = ((logical_size * ppi).round() as usize).clamp(8, 256);
    let key = (icon, px as u32, [color.r(), color.g(), color.b()]);
    if let Some(texture) = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<TextureCache>(egui::Id::new(TEXTURE_CACHE_ID))
            .0
            .get(&key)
            .cloned()
    }) {
        return texture;
    }
    let image = rasterize(icon, px, color)
        .unwrap_or_else(|| egui::ColorImage::new([px, px], egui::Color32::TRANSPARENT));
    let name = format!("icon/{icon:?}/{px}");
    let tex = ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<TextureCache>(egui::Id::new(TEXTURE_CACHE_ID))
            .0
            .insert(key, tex.clone());
    });
    tex
}

/// Draw `icon` centered in `rect`, sized to the rect's smaller side, inked in
/// `color`. The painter-based twin of `image()`.
pub(crate) fn paint(
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: AppIcon,
    color: egui::Color32,
) {
    let side = rect.width().min(rect.height());
    let tex = texture(painter.ctx(), icon, side, color);
    let uv = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1.0, 1.0));
    painter.image(tex.id(), rect, uv, egui::Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_rasterizes_at_menu_and_cell_sizes() {
        let all = [
            AppIcon::Open,
            AppIcon::Add,
            AppIcon::Settings,
            AppIcon::Close,
            AppIcon::Check,
            AppIcon::ChevronDown,
            AppIcon::Warn,
            AppIcon::Error,
            AppIcon::Github,
            AppIcon::Globe,
            AppIcon::Licenses,
            AppIcon::InstallUpdate,
            AppIcon::Export,
            AppIcon::Palette,
            AppIcon::ScanColors,
            AppIcon::ScanColorsOff,
            AppIcon::Texture,
            AppIcon::TextureOff,
            AppIcon::EditMesh,
            AppIcon::Repair,
            AppIcon::FlipNormals,
            AppIcon::BridgeSplit,
            AppIcon::Wireframe,
            AppIcon::Trash,
            AppIcon::Eye,
            AppIcon::EyeOff,
            AppIcon::FitView,
            AppIcon::Ruler,
            AppIcon::Thickness,
            AppIcon::Align,
            AppIcon::Lasso,
            AppIcon::Object,
            AppIcon::SelectAll,
            AppIcon::SelectNone,
            AppIcon::SelectInvert,
            AppIcon::Delete,
            AppIcon::Keep,
            AppIcon::CloseHoles,
            AppIcon::Smooth,
            AppIcon::SculptAdd,
            AppIcon::Cut,
            AppIcon::Separate,
            AppIcon::Undo,
            AppIcon::Redo,
            AppIcon::SurfaceMode,
            AppIcon::ThroughMode,
            AppIcon::AlignFit,
            AppIcon::AlignRefine,
            AppIcon::Heatmap,
            AppIcon::MaskBrush,
            AppIcon::MoveLayer,
            AppIcon::MoveVertical,
            AppIcon::MovePlane,
        ];
        for icon in all {
            for px in [15, 17, 18, 34] {
                let img = rasterize(icon, px, egui::Color32::from_rgb(26, 32, 44));
                assert!(img.is_some(), "{icon:?} failed to rasterize at {px}px");
                if let Some(img) = img {
                    assert_eq!(img.width(), px);
                    assert_eq!(img.height(), px);
                }
            }
        }
    }

    #[test]
    fn recolor_changes_the_ink() {
        let dark = rasterize(AppIcon::Open, 24, egui::Color32::BLACK);
        let light = rasterize(AppIcon::Open, 24, egui::Color32::WHITE);
        assert!(dark.is_some() && light.is_some(), "rasterize failed");
        if let (Some(dark), Some(light)) = (dark, light) {
            let dark_alpha: u32 = dark.pixels.iter().map(|p| u32::from(p.a())).sum();
            let light_alpha: u32 = light.pixels.iter().map(|p| u32::from(p.a())).sum();
            assert_eq!(dark_alpha, light_alpha, "same coverage, different ink");
            assert!(dark_alpha > 0, "a folder glyph must actually paint pixels");
        }
    }

    #[test]
    fn each_egui_context_owns_its_icon_texture() {
        let first = egui::Context::default();
        let second = egui::Context::default();
        let color = egui::Color32::from_rgb(37, 91, 143);

        let _first_texture = texture(&first, AppIcon::Globe, 19.0, color);
        let before = second.tex_manager().read().num_allocated();
        let _second_texture = texture(&second, AppIcon::Globe, 19.0, color);
        let after = second.tex_manager().read().num_allocated();

        assert_eq!(
            after,
            before + 1,
            "the second context must allocate its own texture"
        );
    }
}
