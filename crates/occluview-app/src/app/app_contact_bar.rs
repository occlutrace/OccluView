//! Horizontal controls and legend for an open contact reading.

use eframe::egui;
use occluview_contact::{ContactScale, ContactStats, LOAD_MAX_MM, LOAD_MIN_MM};

use super::OccluViewApp;
use crate::contact::{ContactMode, ContactStatus};
use crate::icons::AppIcon;
use crate::ui_theme;

/// Height of the strip, in points. The legend and the controls share one row so
/// the bar stays a strip rather than a panel.
const BAR_HEIGHT: f32 = 42.0;
/// Gap between the viewport's top edge and the bar.
const BAR_TOP_INSET: f32 = 10.0;
/// Height of the legend bar, in points.
const LEGEND_HEIGHT: f32 = 10.0;
/// Room under the ramp for the two numbers that name its ends.
const LEGEND_LABEL_HEIGHT: f32 = 11.0;
/// Narrowest ramp that can show both end labels without them meeting.
const LEGEND_LABEL_MIN_WIDTH: f32 = 150.0;
/// Narrowest the legend is allowed to get before the bar drops it.
const LEGEND_MIN_WIDTH: f32 = 100.0;
/// Widest the legend grows: past this the ramp reads as a ruler, not a scale.
const LEGEND_MAX_WIDTH: f32 = 260.0;
/// How many samples the legend bar is drawn from. One per point of its width is
/// wasteful; this is enough that no band boundary lands visibly on a step.
const LEGEND_STEPS: usize = 96;
/// Width of the two reading buttons.
const MODE_BUTTON_WIDTH: f32 = 88.0;
/// Width of the load slider, including its label.
const SLIDER_WIDTH: f32 = 196.0;
/// Height of one control row. Matches `align_panel`'s chip so the bar reads as
/// the same family, and keeps the strip exactly `BAR_HEIGHT` tall.
const CHIP_HEIGHT: f32 = 26.0;
/// Width of the details button, reserved when the legend measures itself.
const DETAILS_BUTTON_WIDTH: f32 = 92.0;
/// Widest the bar grows, so it stays a strip on a wide monitor.
const BAR_MAX_WIDTH: f32 = 940.0;
/// Minimum room for the mode pair, load control, details and close actions.
const BAR_SIDE_MIN_WIDTH: f32 = 520.0;
const BAR_EDGE_INSET: f32 = 16.0;
const BAR_LAYERS_GAP: f32 = 12.0;
/// Width of the ✕ that closes the reading.
const CLOSE_BUTTON_WIDTH: f32 = 26.0;
/// One side of `ui_theme::overlay_frame()`'s inner margin. Kept here so the
/// strip can size itself against the space the frame actually leaves.
const FRAME_PADDING_X: f32 = 10.0;

/// What the bar asked for this frame, applied by the caller after the closure
/// so the bar never holds a second mutable borrow of the app.
#[derive(Default)]
struct ContactBarRequest {
    close: bool,
    mode: Option<ContactMode>,
    load_mm: Option<f64>,
    flatten: Option<bool>,
    retry: bool,
    open_details: bool,
}

impl OccluViewApp {
    /// The contact bar. Returns whether it took the pointer.
    pub(super) fn show_contact_bar(
        &mut self,
        ui: &mut egui::Ui,
        viewport_rect: egui::Rect,
        ctx: &egui::Context,
    ) -> bool {
        if !self.tools.contacts.is_open() {
            return false;
        }

        let mut request = ContactBarRequest::default();
        let mut load_mm = self.tools.contacts.load_mm();
        let mode = self.tools.contacts.mode();
        let scale = self.tools.contacts.scale();
        let flatten = self.tools.contacts.flatten_patches();
        let busy = self.tools.contacts.is_busy();
        let status = self.tools.contacts.status();
        let numbers = self.tools.contacts.stats();
        let refused = self.tools.contacts.refused();
        let title = self.contact_bar_title();
        let details_open = self.tools.contacts.details_open();

        let locale = &self.ui.locale;
        let layer_count = self
            .document
            .scene
            .as_ref()
            .map_or(0, |scene| scene.meshes().len());
        let rect = contact_bar_rect(viewport_rect, layer_count);
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                // Account for frame padding when laying out the strip.
                let inner = rect.shrink2(egui::vec2(FRAME_PADDING_X, 0.0));
                ui.set_width(inner.width());
                ui.set_height(rect.height());
                ui.set_max_width(inner.width());
                ui_theme::overlay_frame().show(ui, |ui| {
                    ui.set_max_width(inner.width());
                    paint_strip(
                        ui,
                        rect,
                        StripView {
                            locale,
                            title: &title,
                            mode,
                            scale,
                            load_mm: &mut load_mm,
                            details_open,
                            busy,
                            refused,
                            status,
                        },
                        &mut request,
                    );
                });
            })
            .response;

        self.apply_contact_bar_request(ctx, request, load_mm, flatten);

        // Consume pointer input while the bar is visible.
        let hovered = response.hovered();

        if self.tools.contacts.details_open() {
            self.show_contact_details(
                ui,
                rect,
                DetailsContent {
                    numbers,
                    sentence: status,
                },
            );
        }
        hovered
    }

    fn apply_contact_bar_request(
        &mut self,
        ctx: &egui::Context,
        request: ContactBarRequest,
        load_mm: f64,
        flatten: bool,
    ) {
        if request.close {
            self.close_contacts(ctx);
            return;
        }
        if request.open_details {
            self.tools.contacts.toggle_details();
        }
        // Display-only changes update materials without re-measuring.
        let mode_changed = request
            .mode
            .is_some_and(|mode| self.tools.contacts.set_mode(mode));
        let load_changed = request
            .load_mm
            .is_some_and(|_| self.tools.contacts.set_load_mm(load_mm));
        if mode_changed || load_changed {
            self.mark_scene_materials_changed();
        }
        // Changing the patch rule requires a new measurement.
        if let Some(next) = request.flatten {
            if next != flatten && self.tools.contacts.set_flatten_patches(next) {
                self.tools.contacts.drop_fields();
                self.mark_scene_materials_changed();
                self.submit_contacts_job();
            }
        }
        if request.retry {
            self.tools.contacts.forget_failure();
            self.submit_contacts_job();
        }
    }

    /// What the bar calls the reading it is showing.
    fn contact_bar_title(&self) -> String {
        let Some(pair) = self.tools.contacts.pair() else {
            return self.ui.locale.tr("contact-title");
        };
        let subject = self
            .layer_display_name(pair.subject)
            .unwrap_or_else(|| self.ui.locale.tr("contact-unknown-layer"));
        let against = self
            .layer_display_name(pair.antagonist)
            .unwrap_or_else(|| self.ui.locale.tr("contact-unknown-layer"));
        self.ui.locale.tr_with(
            "contact-against",
            &[("subject", &subject), ("antagonist", &against)],
        )
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn contact_bar_stays_clear_of_layers_and_inside_the_viewport() {
        for width in [600.0, 1024.0, 1600.0] {
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 768.0));
            let layers = crate::layers_overlay::layer_overlay_rect(viewport, 4);
            let bar = contact_bar_rect(viewport, 4);
            assert!(
                viewport.contains_rect(bar),
                "bar outside {width}px viewport: {bar:?}"
            );
            assert!(
                !layers.intersects(bar),
                "bar overlaps layers at {width}px: {bar:?}"
            );
        }
    }
}

/// Values needed to paint the strip.
struct StripView<'a> {
    locale: &'a crate::i18n::LocaleManager,
    /// What the reading is called: which scan, against which.
    title: &'a str,
    mode: ContactMode,
    scale: ContactScale,
    load_mm: &'a mut f64,
    details_open: bool,
    busy: bool,
    refused: bool,
    status: Option<ContactStatus>,
}

/// One row of controls, left to right, inside the bar's frame.
fn paint_strip(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    view: StripView<'_>,
    request: &mut ContactBarRequest,
) {
    let locale = view.locale;
    ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
    ui.horizontal_centered(|ui| {
        paint_identity(ui, view.title);
        ui.add_space(2.0);
        paint_mode_pair(ui, locale, view.mode, request);
        ui_theme::vertical_divider(ui, CHIP_HEIGHT - 8.0);

        // Keep the legend when the remaining width can show a usable ramp.
        let reserved = MODE_BUTTON_WIDTH * 2.0 + SLIDER_WIDTH + DETAILS_BUTTON_WIDTH + 78.0 + 24.0;
        let bar_width = rect.width() - FRAME_PADDING_X * 2.0;
        let legend_width = (bar_width - reserved).clamp(0.0, LEGEND_MAX_WIDTH);
        if legend_width >= LEGEND_MIN_WIDTH {
            paint_legend(ui, view.scale, legend_width, locale);
            ui.add_space(2.0);
        }

        // Reserve the action group at the right edge.
        let actions_width = DETAILS_BUTTON_WIDTH
            + CLOSE_BUTTON_WIDTH
            + ui.spacing().item_spacing.x * 2.0
            + if view.refused {
                78.0 + ui.spacing().item_spacing.x
            } else {
                0.0
            }
            + if view.busy { 22.0 } else { 0.0 };
        let used = (ui.min_rect().right() - rect.left() - FRAME_PADDING_X).max(0.0)
            + ui.spacing().item_spacing.x;
        let load_width = (bar_width - used - actions_width).clamp(120.0, SLIDER_WIDTH);
        paint_load(ui, locale, view.load_mm, request, load_width);
        if view.busy {
            ui.add(egui::Spinner::new().size(14.0));
        }
        // Show actionable status on the bar.
        if let Some(status) = view.status.filter(|status| {
            !matches!(
                status,
                ContactStatus::Measuring | ContactStatus::Remeasuring
            )
        }) {
            ui.label(
                egui::RichText::new(locale.tr(status.key()))
                    .size(11.0)
                    .color(ui_theme::text_weak()),
            )
            .on_hover_text(locale.tr(status_hint_key(status)));
        }

        // Pin actions to the right so the close control remains available.
        let actions_rect = egui::Rect::from_min_size(
            egui::pos2(
                rect.right() - FRAME_PADDING_X - actions_width,
                ui.min_rect().center().y - CHIP_HEIGHT * 0.5,
            ),
            egui::vec2(actions_width, CHIP_HEIGHT),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(actions_rect), |ui| {
            ui.set_width(actions_width);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (close_rect, close_response) = ui.allocate_exact_size(
                    egui::vec2(CLOSE_BUTTON_WIDTH, CHIP_HEIGHT),
                    egui::Sense::click(),
                );
                crate::icons::paint(
                    ui.painter(),
                    close_rect.shrink(5.0),
                    AppIcon::Close,
                    if close_response.hovered() {
                        ui_theme::accent()
                    } else {
                        ui_theme::text_weak()
                    },
                );
                if close_response
                    .on_hover_text(locale.tr("contact-close-hint"))
                    .clicked()
                {
                    request.close = true;
                }
                paint_details_toggle(ui, locale, view.details_open, request);
                if view.refused
                    && crate::align_panel::chip(
                        ui,
                        78.0,
                        None,
                        &locale.tr("contact-retry"),
                        !view.busy,
                        false,
                    )
                    .clicked()
                {
                    request.retry = true;
                }
            });
        });
    });
}

/// Keep the horizontal reading beside Layers when it fits, otherwise put it
/// below. Both overlays use this same Layers geometry, so a wider layer list
/// cannot slide under the contact controls on a large window.
pub(crate) fn contact_bar_rect(viewport_rect: egui::Rect, layer_count: usize) -> egui::Rect {
    let layers = (layer_count > 0)
        .then(|| crate::layers_overlay::layer_overlay_rect(viewport_rect, layer_count));
    let side_left = layers.map_or(viewport_rect.left() + BAR_EDGE_INSET, |rect| {
        rect.right() + BAR_LAYERS_GAP
    });
    let right = viewport_rect.right() - BAR_EDGE_INSET;
    let (left, top) = if right - side_left >= BAR_SIDE_MIN_WIDTH {
        (side_left, viewport_rect.top() + BAR_TOP_INSET)
    } else {
        (
            viewport_rect.left() + BAR_EDGE_INSET,
            layers.map_or(viewport_rect.top() + BAR_TOP_INSET, |rect| {
                rect.bottom() + 8.0
            }),
        )
    };
    let width = (right - left).clamp(0.0, BAR_MAX_WIDTH);
    egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(width, BAR_HEIGHT))
}

/// The reading's name and the mark that says what it is.
fn paint_identity(ui: &mut egui::Ui, title: &str) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    crate::icons::paint(ui.painter(), rect, AppIcon::Contacts, ui_theme::accent());
    response.on_hover_text(title);
}

/// The two readings, side by side: switching costs one click and the operator
/// can see there is a second question to ask.
fn paint_mode_pair(
    ui: &mut egui::Ui,
    locale: &crate::i18n::LocaleManager,
    mode: ContactMode,
    request: &mut ContactBarRequest,
) {
    for choice in ContactMode::ALL {
        if crate::align_panel::chip(
            ui,
            MODE_BUTTON_WIDTH,
            None,
            &locale.tr(choice.label_key()),
            true,
            mode == choice,
        )
        .on_hover_text(locale.tr(choice.hint_key()))
        .clicked()
        {
            request.mode = Some(choice);
        }
    }
}

/// The one control that matters, named for what it does to the picture rather
/// than for the field it sets.
fn paint_load(
    ui: &mut egui::Ui,
    locale: &crate::i18n::LocaleManager,
    load_mm: &mut f64,
    request: &mut ContactBarRequest,
    width: f32,
) {
    let show_value = width >= 170.0;
    ui.allocate_ui_with_layout(
        egui::vec2(width, CHIP_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            // Slider ignores add_sized's requested track width. Reserve the
            // value editor explicitly, or it grows into the Details action.
            ui.spacing_mut().slider_width = if show_value {
                (width - 110.0).max(24.0)
            } else {
                (width - 50.0).max(24.0)
            };
            // Label first, then the control: the same order every settings row
            // uses, so "heavy at" reads as the name of the slider rather than a
            // caption that drifted to the wrong side of it.
            ui.label(
                egui::RichText::new(locale.tr("contact-load-label"))
                    .size(10.5)
                    .color(ui_theme::text_weak()),
            );
            let slider = ui
                .add_sized(
                    egui::vec2(width - 52.0, 18.0),
                    egui::Slider::new(load_mm, LOAD_MIN_MM..=LOAD_MAX_MM)
                        .suffix(locale.tr("contact-load-suffix"))
                        .fixed_decimals(2)
                        .show_value(show_value),
                )
                .on_hover_text(locale.tr("contact-load-hint"));
            if slider.changed() {
                request.load_mm = Some(*load_mm);
            }
        },
    );
}

fn paint_details_toggle(
    ui: &mut egui::Ui,
    locale: &crate::i18n::LocaleManager,
    open: bool,
    request: &mut ContactBarRequest,
) {
    if crate::align_panel::chip(ui, 92.0, None, &locale.tr("contact-details"), true, open)
        .on_hover_text(locale.tr("contact-details-hint"))
        .clicked()
    {
        request.open_details = true;
    }
}

/// Draw the current color scale as a horizontal ramp, with its two ends named.
///
/// A ramp without numbers cannot be read: the operator sees that one contact is
/// bluer than another but not by how much, and the bar's own readout only
/// answers for the point under the cursor. The align view's legend carries its
/// bounds for the same reason.
fn paint_legend(
    ui: &mut egui::Ui,
    scale: ContactScale,
    width: f32,
    locale: &crate::i18n::LocaleManager,
) {
    let law = scale.law();
    let far = law.paint_far_mm;
    // Read the ramp's own deepest stop instead of recomputing it. The two
    // disagreed: the ramp clamps its tail at the probe's reach, while this
    // label printed `load x clamp / load_mm` — 1.36 mm at the top of the
    // slider, against a field that cannot report past 0.6 mm.
    let deepest = scale.stop_mm(law.stops.len().saturating_sub(1)).min(0.0);
    let width = width.clamp(LEGEND_MIN_WIDTH, LEGEND_MAX_WIDTH);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, LEGEND_HEIGHT + LEGEND_LABEL_HEIGHT),
        egui::Sense::hover(),
    );
    let gap_label = locale.tr_with("contact-legend-gap", &[("mm", &format!("{far:.2}"))]);
    let bite_label = locale.tr_with(
        "contact-legend-deepest",
        &[("mm", &format!("{:.2}", -deepest))],
    );
    response.on_hover_text(format!("{gap_label}\n{bite_label}"));
    let ramp = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), LEGEND_HEIGHT));
    let rect = ramp;
    let painter = ui.painter();
    #[allow(clippy::cast_precision_loss)]
    let steps = LEGEND_STEPS as f32;
    for step in 0..LEGEND_STEPS {
        #[allow(clippy::cast_precision_loss)]
        let fraction = step as f32 / steps;
        let signed = f64::from(fraction).mul_add(deepest - far, far);
        let [red, green, blue, alpha] = scale.color_at(signed);
        let cell = egui::Rect::from_min_size(
            egui::pos2(rect.left() + fraction * rect.width(), rect.top()),
            egui::vec2(rect.width() / steps + 1.0, rect.height()),
        );
        painter.rect_filled(cell, 0.0, ui_theme::panel_fill());
        painter.rect_filled(
            cell,
            0.0,
            egui::Color32::from_rgba_unmultiplied(red, green, blue, alpha),
        );
    }
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, ui_theme::hairline()),
        egui::StrokeKind::Inside,
    );

    // The ends named under the ramp: the far end is the largest gap the ramp
    // still paints, the near end the deepest bite it paints. A ramp too narrow
    // for both numbers keeps the hover text and paints neither, rather than
    // overlapping two labels into one unreadable smear.
    if ramp.width() < LEGEND_LABEL_MIN_WIDTH {
        return;
    }
    let label_y = ramp.bottom() + 1.0;
    painter.text(
        egui::pos2(ramp.left(), label_y),
        egui::Align2::LEFT_TOP,
        gap_label,
        egui::FontId::proportional(9.0),
        ui_theme::text_muted(),
    );
    painter.text(
        egui::pos2(ramp.right(), label_y),
        egui::Align2::RIGHT_TOP,
        bite_label,
        egui::FontId::proportional(9.0),
        ui_theme::text_muted(),
    );
}

/// The numbers and the patch rule, behind one button on the bar.
impl OccluViewApp {
    fn show_contact_details(&mut self, ui: &mut egui::Ui, bar: egui::Rect, shown: DetailsContent) {
        let locale = &self.ui.locale;
        let width = 268.0;
        let rect = egui::Rect::from_min_size(
            egui::pos2(bar.right() - width, bar.bottom() + 6.0),
            egui::vec2(width, 0.0),
        );
        let flatten = self.tools.contacts.flatten_patches();
        let busy = self.tools.contacts.is_busy();
        let mut toggle: Option<bool> = None;
        let mut close = false;
        let mut pick: Option<occluview_core::SceneMeshId> = None;
        let pair = self.tools.contacts.pair();
        let candidates: Vec<(occluview_core::SceneMeshId, String)> =
            pair.map_or_else(Vec::new, |pair| {
                self.document.scene.as_ref().map_or_else(Vec::new, |scene| {
                    crate::contact::antagonist_candidates(scene, pair.subject)
                        .into_iter()
                        .filter(|id| *id != pair.antagonist)
                        .filter_map(|id| self.layer_display_name(id).map(|name| (id, name)))
                        .collect()
                })
            });

        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_width(width);
            ui_theme::overlay_frame().show(ui, |ui| {
                ui.set_width(width - 20.0);
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                if crate::align_panel::chip(
                    ui,
                    ui.available_width(),
                    None,
                    &locale.tr("contact-flatten"),
                    !busy,
                    flatten,
                )
                .on_hover_text(locale.tr("contact-flatten-hint"))
                .clicked()
                {
                    toggle = Some(!flatten);
                }
                pick = paint_antagonist_picker(ui, &candidates, busy, locale);
                if let Some(numbers) = shown.numbers {
                    paint_stats(ui, numbers, locale);
                }
                if let Some(sentence) = shown.sentence {
                    ui.label(
                        egui::RichText::new(locale.tr(sentence.key()))
                            .color(ui_theme::text_weak())
                            .size(11.0),
                    )
                    .on_hover_text(locale.tr(status_hint_key(sentence)));
                }
                let [red, green, blue, alpha] = self.tools.contacts.scale().color_at(0.0);
                let (swatch, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 2.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(
                    swatch,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(red, green, blue, alpha.max(90)),
                );
                if ui
                    .add(egui::Button::new(locale.tr("contact-details-close")).frame(false))
                    .clicked()
                {
                    close = true;
                }
            });
        });

        if let Some(next) = toggle {
            if self.tools.contacts.set_flatten_patches(next) {
                self.tools.contacts.drop_fields();
                self.mark_scene_materials_changed();
                self.submit_contacts_job();
            }
        }
        if let Some(antagonist) = pick {
            if self.tools.contacts.set_antagonist(antagonist) {
                self.mark_scene_materials_changed();
                self.submit_contacts_job();
            }
        }
        if close {
            self.tools.contacts.toggle_details();
        }
    }
}

/// The row that names what a reading is measured against.
///
/// It appears only when there is something to choose: with one obvious
/// antagonist the automatic pick stands, and a picker would be a question with
/// one answer. Returns the layer the operator picked, if any.
fn paint_antagonist_picker(
    ui: &mut egui::Ui,
    candidates: &[(occluview_core::SceneMeshId, String)],
    busy: bool,
    locale: &crate::i18n::LocaleManager,
) -> Option<occluview_core::SceneMeshId> {
    if candidates.is_empty() {
        return None;
    }
    let mut pick = None;
    ui.label(
        egui::RichText::new(locale.tr("contact-antagonist-pick"))
            .size(11.0)
            .color(ui_theme::text_muted()),
    )
    .on_hover_text(locale.tr("contact-antagonist-pick-hint"));
    for (id, name) in candidates {
        if crate::align_panel::chip(
            ui,
            ui.available_width(),
            None,
            &crate::align_panel_roles::shorten(name),
            !busy,
            false,
        )
        .on_hover_text(name)
        .clicked()
        {
            pick = Some(*id);
        }
    }
    pick
}

/// What the details popover reads from the reading.
#[derive(Clone, Copy)]
struct DetailsContent {
    /// The numbers the last measurement found, if any.
    numbers: Option<ContactStats>,
    /// The sentence the reading is showing, if any.
    sentence: Option<ContactStatus>,
}

/// The extra sentence a status earns.
fn status_hint_key(status: ContactStatus) -> &'static str {
    match status {
        ContactStatus::Measuring => "contact-status-measuring-hint",
        // A held drag defers measurement until the pose is stable.
        ContactStatus::Remeasuring => "contact-status-remeasuring-hint",
        ContactStatus::NeedsSecond => "contact-status-needs-second-hint",
        ContactStatus::SubjectUnusable => "contact-status-subject-unusable-hint",
        ContactStatus::AntagonistUnusable => "contact-status-antagonist-unusable-hint",
        ContactStatus::NoOverlap => "contact-status-no-overlap-hint",
        ContactStatus::Failed(_) => "contact-status-failed-hint",
    }
}

/// The numbers the measurement found: contact area, patch count, deepest
/// penetration, and the left/right balance.
fn paint_stats(ui: &mut egui::Ui, stats: ContactStats, locale: &crate::i18n::LocaleManager) {
    let rows = [
        (
            "contact-stats-area",
            format!("{:.1} mm²", stats.contact_area_mm2),
        ),
        ("contact-stats-contacts", stats.contacts.to_string()),
        (
            "contact-stats-deepest",
            occluview_contact::format_contact_value(stats.deepest_mm),
        ),
        (
            "contact-stats-balance",
            format!(
                "{:.1} / {:.1} mm²",
                stats.minus_x_area_mm2, stats.plus_x_area_mm2
            ),
        ),
    ];
    egui::Grid::new("contact_details_stats")
        .num_columns(2)
        .spacing(egui::vec2(10.0, 3.0))
        .show(ui, |ui| {
            for (key, value) in rows {
                let label = ui.label(
                    egui::RichText::new(locale.tr(key))
                        .size(11.0)
                        .color(ui_theme::text_weak()),
                );
                // "Area each side" is a mid-line split, not a contact count, and
                // the caveat that said so was a catalogue key nothing resolved —
                // it was deleted as dead, and this hover is where an operator can
                // still find the meaning.
                if key == "contact-stats-balance" {
                    label.on_hover_text(locale.tr("contact-stats-balance-hover"));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(value).size(11.5));
                });
                ui.end_row();
            }
        });
}
