//! AccessKit metadata for custom-painted controls.

use eframe::egui::{Response, WidgetInfo, WidgetType};

pub(crate) fn button(response: &Response, label: &str, enabled: bool, selected: Option<bool>) {
    response.widget_info(|| match selected {
        Some(selected) => WidgetInfo::selected(WidgetType::Button, enabled, selected, label),
        None => WidgetInfo::labeled(WidgetType::Button, enabled, label),
    });
}

pub(crate) fn link(response: &Response, label: &str, enabled: bool) {
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Link, enabled, label));
}

pub(crate) fn slider(response: &Response, label: &str, enabled: bool, value: f64) {
    response.widget_info(|| WidgetInfo::slider(enabled, value, label));
}

pub(crate) fn spin_button(response: &Response, label: &str, enabled: bool) {
    response.widget_info(|| WidgetInfo::labeled(WidgetType::DragValue, enabled, label));
}

pub(crate) fn read_only_text(response: &Response, label: &str, value: &str) {
    response.widget_info(|| {
        let mut info = WidgetInfo::text_edit(false, value, value, "");
        info.label = Some(label.to_owned());
        info
    });
}
