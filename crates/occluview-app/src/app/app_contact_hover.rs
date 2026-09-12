//! The pointer readout: the contact value on the surface under the cursor.
//!
//! A ray hit, not a projection trick. This viewer already picks the nearest
//! triangle under the pointer through the mesh's own BVH, and the field is a
//! per-vertex value on that triangle, so the number the readout prints is the
//! number the surface carries where the operator is pointing. A vertex with no
//! opposing surface inside the search radius answers nothing at all, which is
//! why the readout simply does not appear over a hollow of the arch that nothing
//! opposes.
//!
//! BOTH ARCHES ANSWER. The reading paints both surfaces because the operator
//! reads the bite from whichever side is facing them, so a readout that worked
//! on only one of the two painted arches would be dead over half of what the
//! feature draws — and which half is "the" arch is an implementation fact
//! (whichever was right-clicked) that nothing on screen shows. Each layer is
//! asked for its own field: the antagonist's field is that same surface measured
//! against the subject, so pointing at either side prints the value that side
//! carries.

use eframe::egui;
use occluview_contact::{format_contact_value, ContactReading, ContactReadingKind};

use super::OccluViewApp;
use crate::contact::{field_value_at, reading_of};
use crate::ui_theme;

/// The chip's gap from the cursor, in points.
const READOUT_OFFSET_PX: f32 = 14.0;

impl OccluViewApp {
    // ------------------------------------------------------------------- hover

    /// The readout under the cursor: the contact value at the point on the
    /// subject surface the pointer is over.
    ///
    /// A ray hit, not a projection trick. This viewer already picks the nearest
    /// triangle under the pointer through the mesh's own BVH, and the field is a
    /// per-vertex value on that triangle, so the number the operator reads is
    /// the number the surface carries where they are pointing. A vertex with no
    /// opposing surface inside the search radius answers nothing at all, which
    /// is why the readout simply does not appear over a hollow of the arch
    /// nothing opposes.
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
        // A pointer that is orbiting, panning or dragging is not asking for a
        // number; a readout that follows a drag is noise.
        if ctx.input(|input| input.pointer.any_down()) {
            return;
        }
        let (Some(camera), Some(scene)) = (self.render.camera, self.document.scene.as_ref()) else {
            return;
        };
        // The subject first, then the surface it is measured against: a click on
        // either painted arch reports that arch's own reading. Each pick is
        // layer-scoped, so the nearer of the two never answers for the other.
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
        // The swatch is the colour the surface wears at this reading, so it is
        // read through the ONE predicate the paint path uses. Outside the
        // painted band the surface shows nothing there, and a swatch drawn from
        // `color_at` would be a solid black chip claiming a colour that is not
        // on the scan at all.
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
            },
        );
    }
}

/// The pointer readout: a chip carrying the value, its unit, and a swatch of the
/// exact colour the surface wears there.
///
/// The swatch and the number come from the same scale evaluation, so the readout
/// can never describe a colour the surface is not painting.
/// Everything the chip says, gathered so the painter takes one value rather than
/// a growing argument list.
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
}

fn paint_readout(ui: &mut egui::Ui, readout: Readout<'_>) {
    let Readout {
        pointer,
        viewport,
        locale,
        reading,
        accent,
    } = readout;
    let sign = match reading.kind {
        ContactReadingKind::Penetration => "+",
        ContactReadingKind::Gap => "−",
    };
    let label = match reading.kind {
        ContactReadingKind::Penetration => locale.tr("contact-readout-load"),
        ContactReadingKind::Gap => locale.tr("contact-readout-gap"),
    };
    let value = format!(
        "{label} {sign}{}",
        format_contact_value(reading.magnitude_mm)
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
        // Not painted here: no swatch, because there is no colour to show.
        None => chip.left() + padding.x,
    };
    painter.galley(
        egui::pos2(text_left, chip.top() + padding.y),
        galley,
        ui_theme::text(),
    );
}
