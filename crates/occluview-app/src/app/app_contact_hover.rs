//! Pointer readout for values on the current contact fields.

use eframe::egui;
use occluview_contact::{
    format_contact_value_in, ContactLengthUnit, ContactReading, ContactReadingKind,
};

use super::OccluViewApp;
use crate::contact::{field_value_at, reading_of};
use crate::ui_theme;

/// The chip's gap from the cursor, in points.
const READOUT_OFFSET_PX: f32 = 14.0;

impl OccluViewApp {
    /// Show the measured contact value under the pointer.
    pub(super) fn show_contact_hover(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        ctx: &egui::Context,
    ) {
        if !self.tools.contacts.is_open() || self.tools.contacts.fields().is_empty() {
            return;
        }
        let Some(pair) = self.tools.contacts.pair() else {
            return;
        };
        let Some(pointer) = ctx
            .input(|input| input.pointer.hover_pos())
            .filter(|pos| response.rect.contains(*pos))
        else {
            return;
        };
        // Do not show a readout during another pointer gesture.
        if ctx.input(|input| input.pointer.any_down()) {
            return;
        }
        let (Some(camera), Some(scene)) = (self.render.camera, self.document.scene.as_ref()) else {
            return;
        };
        // Pick each measured layer independently so either side can provide a
        // readout.
        let mut read = None;
        for layer in [pair.subject, pair.antagonist] {
            let (Some(entry), Some(field)) = (
                scene.meshes().iter().find(|entry| entry.id() == layer),
                self.tools.contacts.field_for(layer),
            ) else {
                continue;
            };
            let Some(hit) =
                crate::viewer::pick_layer_hit(&camera, response.rect, pointer, scene, layer)
            else {
                continue;
            };
            read = field_value_at(&field.signed_mm, entry, hit.triangle_index, hit.point)
                .and_then(reading_of)
                .map(|reading| (layer, reading));
            if read.is_some() {
                break;
            }
        }
        let Some((_layer, reading)) = read else {
            return;
        };
        let scale = self.tools.contacts.scale();
        let signed = match reading.kind {
            ContactReadingKind::Gap => reading.magnitude_mm,
            ContactReadingKind::Penetration => -reading.magnitude_mm,
        };
        // Show a swatch only when the value is inside the painted range.
        let accent = scale.is_painted(signed).then(|| {
            let [channel_r, channel_g, channel_b, _] = scale.color_at(signed);
            egui::Color32::from_rgb(channel_r, channel_g, channel_b)
        });
        paint_readout(
            ui,
            Readout {
                pointer,
                viewport: response.rect,
                locale: &self.ui.locale,
                reading,
                accent,
                unit: match self.persistence.settings.unit_display {
                    crate::app_settings::UnitDisplay::Millimeters => ContactLengthUnit::Millimeters,
                    crate::app_settings::UnitDisplay::Inches => ContactLengthUnit::Inches,
                },
            },
        );
    }
}

/// Data needed to paint the pointer readout.
struct Readout<'a> {
    /// Where the pointer is; the chip sits beside it.
    pointer: egui::Pos2,
    /// The viewport the chip is clamped inside.
    viewport: egui::Rect,
    /// The locale, for the two words in front of the number.
    locale: &'a crate::i18n::LocaleManager,
    /// The reading itself.
    reading: ContactReading,
    /// The exact colour the surface wears at this reading, or `None` when the
    /// reading is outside the painted band and the surface wears nothing there.
    accent: Option<egui::Color32>,
    /// The operator's length unit, so this readout matches the ruler rather than
    /// being the one measurement that ignores the preference.
    unit: ContactLengthUnit,
}

fn paint_readout(ui: &mut egui::Ui, readout: Readout<'_>) {
    let Readout {
        pointer,
        viewport,
        locale,
        reading,
        accent,
        unit,
    } = readout;
    let sign = match reading.kind {
        ContactReadingKind::Penetration => "+",
        ContactReadingKind::Gap => "−",
    };
    let label = match reading.kind {
        ContactReadingKind::Penetration => locale.tr("contact-readout-load"),
        ContactReadingKind::Gap => locale.tr("contact-readout-gap"),
    };
    // The operator's length preference reaches this readout too: it used to be
    // the only measurement in the app that ignored it, while the ruler, the
    // thickness probe and the scale bar all follow it.
    let value = format!(
        "{label} {sign}{}",
        format_contact_value_in(reading.magnitude_mm, unit)
    );

    let font = egui::FontId::proportional(11.0);
    let galley = ui.painter().layout_no_wrap(value, font, ui_theme::text());
    let padding = egui::vec2(8.0, 5.0);
    let swatch_width = 3.0;
    let swatch_gap = 6.0;
    let size = egui::vec2(
        galley.size().x + padding.x * 2.0 + swatch_width + swatch_gap,
        galley.size().y + padding.y * 2.0,
    );
    // Place the chip right of the cursor if it fits, else left; clamp inside the
    // viewport so a reading near an edge never sits under the chrome.
    let mut origin = egui::pos2(pointer.x + READOUT_OFFSET_PX, pointer.y - size.y / 2.0);
    if origin.x + size.x > viewport.right() - 4.0 {
        origin.x = pointer.x - READOUT_OFFSET_PX - size.x;
    }
    let max_x = (viewport.right() - size.x - 4.0).max(viewport.left() + 4.0);
    origin.x = origin.x.clamp(viewport.left() + 4.0, max_x);
    let max_y = (viewport.bottom() - size.y - 4.0).max(viewport.top() + 4.0);
    origin.y = origin.y.clamp(viewport.top() + 4.0, max_y);
    let chip = egui::Rect::from_min_size(origin, size);

    let painter = ui.painter();
    painter.rect_filled(chip, 4.0, ui_theme::panel_fill().gamma_multiply(0.94));
    painter.rect_stroke(
        chip,
        4.0,
        egui::Stroke::new(1.0, ui_theme::hairline()),
        egui::StrokeKind::Inside,
    );
    let text_left = match accent {
        Some(accent) => {
            let swatch = egui::Rect::from_min_size(
                egui::pos2(chip.left() + padding.x, chip.top() + padding.y),
                egui::vec2(swatch_width, galley.size().y),
            );
            painter.rect_filled(swatch, 1.5, accent);
            chip.left() + padding.x + swatch_width + swatch_gap
        }
        // No swatch is shown outside the painted range.
        None => chip.left() + padding.x,
    };
    painter.galley(
        egui::pos2(text_left, chip.top() + padding.y),
        galley,
        ui_theme::text(),
    );
}
