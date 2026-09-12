//! The contact bar: one compact strip inside the viewport.
//!
//! It replaces the floating window this feature used to open. The window was
//! the shape of an inspector — a title bar, a pair name, a statistics block —
//! for a reading whose whole control set is two readings and one slider, and it
//! sat over the scan the operator was trying to look at.
//!
//! The bar keeps the three things that must stay reachable while a reading is
//! up: which reading is shown, where heavy starts, and how the colours read.
//! Everything else — the pair name, the numbers, the patch rule — is either
//! already on screen or belongs in the legend, so it moves to a popover that
//! opens from the bar itself.
//!
//! Nothing here measures. The law switches which field paints, the slider
//! rewrites the stop table in the per-mesh uniform, and the patch rule is the
//! one control that asks the worker for a new field.

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
/// Narrowest the legend is allowed to get before the bar drops it.
const LEGEND_MIN_WIDTH: f32 = 120.0;
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
const DETAILS_BUTTON_WIDTH: f32 = 78.0;
/// Widest the bar grows, so it stays a strip on a wide monitor.
const BAR_MAX_WIDTH: f32 = 940.0;
/// Space left for the layers overlay, which owns the top-left corner.
const LAYERS_OVERLAY_CLEARANCE: f32 = 300.0;

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
        let rect = contact_bar_rect(viewport_rect);
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.set_width(rect.width());
                ui.set_height(rect.height());
                ui_theme::overlay_frame().show(ui, |ui| {
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

        // A reading is a tool the operator is in the middle of: while the bar is
        // up, the pointer belongs to it, so a click on a button never falls
        // through to the camera underneath.
        let hovered = response.hovered();

        if self.tools.contacts.details_open() {
            self.show_contact_details(
                ui,
                viewport_rect,
                ctx,
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
        // Recolouring never re-measures: the law picks which field paints and
        // the slider rewrites the stop table.
        let mode_changed = request
            .mode
            .is_some_and(|mode| self.tools.contacts.set_mode(mode));
        let load_changed = request
            .load_mm
            .is_some_and(|_| self.tools.contacts.set_load_mm(load_mm));
        if mode_changed || load_changed {
            self.mark_scene_materials_changed();
        }
        // The patch rule IS an input to the measurement, so this one really does
        // re-run the worker.
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

/// What the strip reads from the reading, gathered once so the painter takes
/// one argument instead of eight.
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
        ui.add_space(2.0);

        // The legend is what makes the colours readable, so it is the last
        // thing to go: it takes whatever the controls leave, and only
        // disappears once that is genuinely too narrow to read a ramp.
        let reserved = MODE_BUTTON_WIDTH * 2.0 + SLIDER_WIDTH + DETAILS_BUTTON_WIDTH + 78.0 + 24.0;
        let legend_width = (rect.width() - reserved).clamp(0.0, LEGEND_MAX_WIDTH);
        if legend_width >= LEGEND_MIN_WIDTH {
            paint_legend(ui, view.scale, legend_width);
            ui.add_space(2.0);
        }

        // Everything from here to the right edge belongs to the action group;
        // reserving it keeps the slider's value from being painted over.
        let actions_width = DETAILS_BUTTON_WIDTH
            + 34.0
            + if view.refused { 78.0 + 8.0 } else { 0.0 }
            + if view.busy { 22.0 } else { 0.0 };
        let used = (ui.min_rect().right() - rect.left()).max(0.0) + 8.0;
        let load_width = (rect.width() - used - actions_width - 26.0).clamp(120.0, SLIDER_WIDTH);
        paint_load(ui, locale, view.load_mm, request, load_width);
        if view.busy {
            ui.add(egui::Spinner::new().size(14.0));
        }
        // A reading that cannot run says why on the bar itself. Behind the
        // Details button, "show the opposing scan first" only reached an
        // operator who had already decided to look for it.
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

        ui.allocate_ui_with_layout(
            egui::vec2(actions_width, CHIP_HEIGHT),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                if ui
                    .small_button("✕")
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
            },
        );
    });
}

/// Where the bar sits: along the top of the viewport, clear of the layer list.
///
/// The layers overlay owns the top-left corner, so the bar starts to its right
/// and grows toward the free space rather than centring over the list. The axis
/// triad owns the bottom-right, which the top row never reaches.
pub(crate) fn contact_bar_rect(viewport_rect: egui::Rect) -> egui::Rect {
    let left = viewport_rect.left() + LAYERS_OVERLAY_CLEARANCE;
    let available = (viewport_rect.right() - 16.0 - left).max(320.0);
    let width = available.min(BAR_MAX_WIDTH);
    egui::Rect::from_min_size(
        egui::pos2(left, viewport_rect.top() + BAR_TOP_INSET),
        egui::vec2(width, BAR_HEIGHT),
    )
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
    ui.allocate_ui_with_layout(
        egui::vec2(width, CHIP_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
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
                        .show_value(true),
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

/// The ramp as a bar, with the numbers that make it readable.
///
/// Drawn from the scale itself rather than from a copy of its colours, so a
/// legend can never describe a ramp the surface is not wearing. The bar is
/// painted over the strip's fill because the far end of the ramp is barely there
/// at all — the paint ends by alpha, so a swatch drawn over the viewport would
/// show the scan behind it rather than the colour the ramp gives.
fn paint_legend(ui: &mut egui::Ui, scale: ContactScale, width: f32) {
    let law = scale.law();
    let far = law.paint_far_mm;
    let deepest = -(scale.load_mm() * (law.clamp_mm / law.load_mm));
    let width = width.clamp(LEGEND_MIN_WIDTH, LEGEND_MAX_WIDTH);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, LEGEND_HEIGHT), egui::Sense::hover());
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
}

/// The numbers and the patch rule, behind one button on the bar.
impl OccluViewApp {
    fn show_contact_details(
        &mut self,
        ui: &mut egui::Ui,
        viewport_rect: egui::Rect,
        ctx: &egui::Context,
        shown: DetailsContent,
    ) {
        let locale = &self.ui.locale;
        let bar = contact_bar_rect(viewport_rect);
        let width = 268.0;
        let rect = egui::Rect::from_min_size(
            egui::pos2(bar.right() - width, bar.bottom() + 6.0),
            egui::vec2(width, 0.0),
        );
        let flatten = self.tools.contacts.flatten_patches();
        let busy = self.tools.contacts.is_busy();
        let mut toggle: Option<bool> = None;
        let mut close = false;

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
        if close {
            self.tools.contacts.toggle_details();
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Escape))
            && self.tools.contacts.details_open()
        {
            self.tools.contacts.toggle_details();
        }
    }
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
        // `Remeasuring` is the held state: a drag is rewriting a pose every
        // frame, so the reading waits for it to end. Saying "reading the two
        // surfaces" here described work that is deliberately not happening.
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
                ui.label(
                    egui::RichText::new(locale.tr(key))
                        .size(11.0)
                        .color(ui_theme::text_weak()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(value).size(11.5));
                });
                ui.end_row();
            }
        });
}
