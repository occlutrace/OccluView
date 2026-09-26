//! Best-fit matching settings cluster of the Align window.
//!
//! Presentation only — values edit `AlignSettings` in place and return through
//! the existing panel actions of the window body (`align_panel`).

use eframe::egui;
use occluview_align::Orientation;

use crate::align_worker::AlignSettings;
use crate::ui_theme;

/// The two sliders and the orientation rule that steer best-fit matching.
pub(crate) fn matching(
    ui: &mut egui::Ui,
    settings: &mut AlignSettings,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) {
    let slider_width = (ui.available_width() - 68.0).max(80.0);
    ui.label(
        egui::RichText::new(locale.tr("align-matching-parts"))
            .size(11.0)
            .color(ui_theme::text_muted()),
    )
    .on_hover_text(locale.tr("align-matching-parts-hint"));
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = slider_width;
        ui.add_enabled(
            enabled,
            egui::Slider::new(&mut settings.matching_ratio, 0.1..=1.0)
                .custom_formatter(|value, _| format!("{:.0}%", value * 100.0)),
        )
        .on_hover_text(locale.tr("align-matching-parts-hint"));
    });
    ui.label(
        egui::RichText::new(locale.tr("align-max-influence"))
            .size(11.0)
            .color(ui_theme::text_muted()),
    )
    .on_hover_text(locale.tr("align-max-influence-hint"));
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = slider_width;
        ui.add_enabled(
            enabled,
            egui::Slider::new(&mut settings.influence_radius_mm, 0.2..=10.0).suffix(" mm"),
        )
        .on_hover_text(locale.tr("align-max-influence-hint"));
    });
    ui.collapsing(locale.tr("align-orientation-title"), |ui| {
        facing(ui, &mut settings.orientation, enabled, locale);
    });
}

/// The surface-orientation rule. An inverted mesh flips the whole signed map,
/// so this is the escape hatch for a scan whose winding disagrees with itself.
///
/// Disabled while a fit is running: the job holds the settings snapshot it was
/// submitted with, so an edit made mid-flight would describe a different match
/// than the one that lands.
pub(crate) fn facing(
    ui: &mut egui::Ui,
    orientation: &mut Orientation,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) {
    let either_hint = locale.tr("align-orientation-either-hint");
    let facing_hint = locale.tr("align-orientation-facing-hint");
    ui.add_enabled_ui(enabled, |ui| {
        for (value, label) in [
            (
                Orientation::Match,
                locale.tr("align-orientation-match").as_str(),
            ),
            (
                Orientation::Inverted,
                locale.tr("align-orientation-inverted").as_str(),
            ),
            (
                Orientation::Ignored,
                locale.tr("align-orientation-ignored").as_str(),
            ),
        ] {
            if ui
                .radio(*orientation == value, label)
                .on_hover_text(if value == Orientation::Ignored {
                    either_hint.as_str()
                } else {
                    facing_hint.as_str()
                })
                .clicked()
            {
                *orientation = value;
            }
        }
    });
}

/// Toggle the exclusion brush used by matching.
pub(crate) fn exclude(
    ui: &mut egui::Ui,
    excluding: &mut bool,
    enabled: bool,
    locale: &crate::i18n::LocaleManager,
) {
    ui.add_enabled_ui(enabled, |ui| {
        ui.checkbox(excluding, locale.tr("align-exclude").as_str())
            .on_hover_text(locale.tr("align-exclude-hint"));
    });
}
