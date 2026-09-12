//! The contact panel: the two readings, the one slider, and the legend that
//! makes the colours readable.
//!
//! Its controls are chosen so that the expensive thing never happens: the mode
//! switches which law re-reads a field already in hand, and the slider moves the
//! depth the ramp calls fully loaded, which is a uniform write. Only the patch
//! rule is an input to the measurement, and that is the one control that
//! re-runs the worker.

use eframe::egui;
use occluview_contact::{ContactScale, ContactStats, LOAD_MAX_MM, LOAD_MIN_MM};

use super::OccluViewApp;
use crate::contact::{ContactMode, ContactStatus};
use crate::icons::AppIcon;
use crate::ui_theme;

/// Width of the panel, in points. Matches the Align window so the two read as
/// one family.
const PANEL_WIDTH: f32 = 272.0;
/// Height of the legend bar, in points.
const LEGEND_HEIGHT: f32 = 12.0;
/// How many samples the legend bar is drawn from. One per point of its width is
/// wasteful; this is enough that no band boundary lands visibly on a step.
const LEGEND_STEPS: usize = 96;

impl OccluViewApp {
    // ------------------------------------------------------------------- panel

    /// The contact panel. Returns whether it took the pointer.
    pub(super) fn show_contact_panel(
        &mut self,
        ctx: &egui::Context,
        viewport_rect: egui::Rect,
    ) -> bool {
        if !self.tools.contacts.is_open() {
            return false;
        }
        let mut request = ContactPanelRequest::default();
        let default_pos = viewport_rect.right_top() + egui::vec2(-PANEL_WIDTH - 16.0, 16.0);
        let title = self.contact_title();
        let styling = PanelStyling {
            map_scale: self.tools.contacts.scale(),
            mode: self.tools.contacts.mode(),
            flatten: self.tools.contacts.flatten_patches(),
            busy: self.tools.contacts.is_busy(),
            status: self.tools.contacts.status(),
            stats: self.tools.contacts.stats(),
            refused: self.tools.contacts.refused(),
        };
        let locale = &self.ui.locale;
        let mut load_mm = self.tools.contacts.load_mm();

        let window = egui::Window::new(locale.text("contact-title"))
            .id(egui::Id::new("occluview_contacts_window"))
            .default_pos(default_pos)
            .constrain_to(viewport_rect)
            .resizable(false)
            .collapsible(false)
            .title_bar(false)
            .show(ctx, |ui| {
                ui.set_min_width(PANEL_WIDTH - 24.0);
                ui.set_width(PANEL_WIDTH - 24.0);
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                ui.style_mut().animation_time = 0.05;
                paint_header(ui, locale, &title, &mut request);
                paint_mode_strip(ui, locale, styling.mode, &mut request);
                paint_controls(
                    ui,
                    ControlsView {
                        scale: styling.map_scale,
                        flatten: styling.flatten,
                        busy: styling.busy,
                        stats: styling.stats,
                        status: styling.status,
                        refused: styling.refused,
                        locale,
                    },
                    &mut request,
                    &mut load_mm,
                );
            });

        let hovered = window
            .as_ref()
            .is_some_and(|inner| inner.response.hovered());

        if request.close {
            self.close_contacts(ctx);
            return true;
        }
        // A mode change re-reads the same field under the other law: it repaints
        // and never re-measures.
        let mode_changed = request
            .mode
            .is_some_and(|mode| self.tools.contacts.set_mode(mode));
        let load_changed = request
            .load_mm
            .is_some_and(|load_mm| self.tools.contacts.set_load_mm(load_mm));
        // The patch rule IS an input to the measurement, so this one really does
        // re-run the worker.
        let flatten_changed = request
            .flatten
            .is_some_and(|flatten| self.tools.contacts.set_flatten_patches(flatten));
        if mode_changed || load_changed {
            self.mark_scene_materials_changed();
        }
        if flatten_changed {
            self.tools.contacts.drop_fields();
            self.mark_scene_materials_changed();
            self.submit_contacts_job();
        }
        if request.retry {
            self.tools.contacts.forget_failure();
            self.submit_contacts_job();
        }
        hovered
    }

    /// What the panel calls the reading it is showing.
    fn contact_title(&self) -> String {
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

/// The panel's title row: the glyph, the name, the pair being measured, and
/// the close button.
fn paint_header(
    ui: &mut egui::Ui,
    locale: &crate::i18n::LocaleManager,
    title: &str,
    request: &mut ContactPanelRequest,
) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        crate::icons::paint(ui.painter(), rect, AppIcon::Contacts, ui_theme::accent());
        ui.label(
            egui::RichText::new(locale.tr("contact-title"))
                .strong()
                .size(14.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            request.close = ui
                .small_button("✕")
                .on_hover_text(locale.tr("contact-close-hint"))
                .clicked();
        });
    });
    ui.label(
        egui::RichText::new(title)
            .color(ui_theme::text_weak())
            .size(11.0),
    );
}

/// The two readings side by side, so switching costs one click and the operator
/// can see there is a second question to ask.
fn paint_mode_strip(
    ui: &mut egui::Ui,
    locale: &crate::i18n::LocaleManager,
    mode: ContactMode,
    request: &mut ContactPanelRequest,
) {
    ui.horizontal(|ui| {
        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        for choice in ContactMode::ALL {
            if crate::align_panel::chip(
                ui,
                width,
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
    });
}

/// The panel's controls, drawn in one place so the window body stays a window
/// body: the one slider that matters, the legend that makes its colours
/// readable, the patch rule, and the numbers the measurement found.
fn paint_controls(
    ui: &mut egui::Ui,
    view: ControlsView<'_>,
    request: &mut ContactPanelRequest,
    load_mm: &mut f64,
) {
    ui.add_space(4.0);

    // The one control that matters. Named for what it does to the picture
    // rather than for the field it sets, because the operator is choosing where
    // heavy starts, not editing a parameter.
    let slider = ui
        .add(
            egui::Slider::new(load_mm, LOAD_MIN_MM..=LOAD_MAX_MM)
                .text(view.locale.tr("contact-load-label"))
                .suffix(view.locale.tr("contact-load-suffix"))
                .fixed_decimals(2),
        )
        .on_hover_text(view.locale.tr("contact-load-hint"));
    if slider.changed() {
        request.load_mm = Some(*load_mm);
    }

    ui.add_space(2.0);
    paint_legend(ui, view.scale, view.locale);

    ui.add_space(4.0);
    if crate::align_panel::chip(
        ui,
        ui.available_width(),
        None,
        &view.locale.tr("contact-flatten"),
        !view.busy,
        view.flatten,
    )
    .on_hover_text(view.locale.tr("contact-flatten-hint"))
    .clicked()
    {
        request.flatten = Some(!view.flatten);
    }

    if let Some(stats) = view.stats {
        ui.add_space(4.0);
        paint_stats(ui, stats, view.locale);
    }
    if let Some(status) = view.status {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(view.locale.tr(status.key()))
                .color(ui_theme::text_weak())
                .size(11.0),
        )
        .on_hover_text(view.locale.tr(status_hint_key(status)));
    }
    // A refusal is not retried frame after frame (that would re-run a doomed
    // search for as long as the panel is open), so the operator gets the one
    // button that asks for another attempt.
    if view.refused
        && crate::align_panel::chip(
            ui,
            ui.available_width(),
            None,
            &view.locale.tr("contact-retry"),
            !view.busy,
            false,
        )
        .clicked()
    {
        request.retry = true;
    }
}

/// What the panel reads from the reading, gathered once per frame.
#[derive(Clone, Copy)]
struct PanelStyling {
    /// The scale the map is painted with, which the legend draws.
    map_scale: ContactScale,
    /// Which reading is shown.
    mode: ContactMode,
    /// Whether penetration patches are being collapsed.
    flatten: bool,
    /// Whether a measurement is in flight, which disables the patch rule.
    busy: bool,
    /// The sentence the panel is showing, if any.
    status: Option<ContactStatus>,
    /// The numbers the last measurement found, if any.
    stats: Option<ContactStats>,
    /// Whether the last attempt was refused, which is when a retry is offered.
    refused: bool,
}

/// What the controls read from the reading.
struct ControlsView<'a> {
    /// The scale the map is painted with, which the legend draws.
    scale: ContactScale,
    /// Whether penetration patches are being collapsed.
    flatten: bool,
    /// Whether a measurement is in flight, which disables the patch rule.
    busy: bool,
    /// The numbers the last measurement found, if any.
    stats: Option<ContactStats>,
    /// The sentence the panel is showing, if any.
    status: Option<ContactStatus>,
    /// Whether the last attempt was refused, which is when a retry is offered.
    refused: bool,
    /// The locale.
    locale: &'a crate::i18n::LocaleManager,
}

/// What the panel asked for this frame, applied after the closure so the panel
/// never holds a second mutable borrow of the app.
#[derive(Default)]
struct ContactPanelRequest {
    close: bool,
    mode: Option<ContactMode>,
    load_mm: Option<f64>,
    flatten: Option<bool>,
    /// The operator asked for another attempt after a refusal.
    retry: bool,
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

/// The ramp as a bar, with the numbers that make it readable.
///
/// Drawn from the scale itself rather than from a copy of its colours, so a
/// legend can never describe a ramp the surface is not wearing. The bar is
/// painted over the panel fill because the far end of the ramp is barely there
/// at all — the paint ends by alpha, so a swatch drawn over the viewport would
/// show the scan behind it rather than the colour the ramp gives.
fn paint_legend(ui: &mut egui::Ui, scale: ContactScale, locale: &crate::i18n::LocaleManager) {
    let law = scale.law();
    let far = law.paint_far_mm;
    // The clamp stop is the deepest value the ramp ever shows, scaled with the
    // slider exactly as the stops are.
    let deepest = -(scale.load_mm() * (law.clamp_mm / law.load_mm));
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), LEGEND_HEIGHT),
        egui::Sense::hover(),
    );
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
    ui.horizontal(|ui| {
        // The left end is the widest gap this law paints at all, named with its
        // number: under the approach law it is 200 um of clearance, and calling
        // that "touch" would tell the operator blue means contact.
        ui.label(
            egui::RichText::new(
                locale.tr_with("contact-legend-gap", &[("mm", &format!("{far:.2}"))]),
            )
            .color(ui_theme::text_weak())
            .size(10.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(locale.tr_with(
                    "contact-legend-deepest",
                    &[("mm", &format!("{:.2}", -deepest))],
                ))
                .color(ui_theme::text_weak())
                .size(10.0),
            );
        });
    });
}

/// The numbers the measurement found: contact area, patch count, deepest
/// penetration.
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
        // The balance reading: how the contact area divides either side of the
        // subject's own mid-line. It is what answers "is this bite loading
        // evenly", and it is a raw-coordinate split, so the tooltip says so —
        // an operator comparing the two numbers on one case is reading what the
        // number means, and one comparing a column across differently oriented
        // cases is not.
        (
            "contact-stats-balance",
            format!(
                "{:.1} / {:.1} mm²",
                stats.minus_x_area_mm2, stats.plus_x_area_mm2
            ),
        ),
    ];
    egui::Grid::new("occluview_contact_stats")
        .num_columns(2)
        .spacing(egui::vec2(8.0, 2.0))
        .show(ui, |ui| {
            for (key, value) in rows {
                ui.label(
                    egui::RichText::new(locale.tr(key))
                        .color(ui_theme::text_muted())
                        .size(10.5),
                );
                let value = ui
                    .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(value).size(10.5))
                    });
                if key == "contact-stats-balance" {
                    value
                        .inner
                        .on_hover_text(locale.tr("contact-stats-balance-hint"));
                }
                ui.end_row();
            }
        });
}
