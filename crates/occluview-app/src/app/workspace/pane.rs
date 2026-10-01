//! Pane chrome: one header per visible scene and the split divider.

use eframe::egui::{pos2, vec2, Rect};

use super::pane_geometry::{default_split, pane_header_rect};
use super::pointer::VisiblePane;
use crate::app::workspace::commands::WorkspaceCommand;
use crate::app::workspace::input::{GestureKind, PaneTarget};
use crate::app::workspace::layout::{EffectiveLayout, WorkspaceLayout};
use crate::app::{egui, OccluViewApp};
use crate::ui::icons::AppIcon;

impl OccluViewApp {
    // One header binds its title and view controls to the same pane.
    #[allow(clippy::too_many_lines)]
    pub(super) fn show_pane_header(
        &mut self,
        ui: &mut egui::Ui,
        pane: &VisiblePane,
        layout: EffectiveLayout,
        modal_open: bool,
    ) {
        let rect = pane_header_rect(pane.frame);
        let is_active = self.workspace.input.active().scene == pane.key;
        let split = matches!(layout, EffectiveLayout::SideBySide { .. });
        ui.painter()
            .rect_filled(rect, 0.0, crate::ui::ui_theme::panel_fill());
        let separator_y = rect.bottom() - if is_active && split { 1.0 } else { 0.5 };
        ui.painter().line_segment(
            [
                pos2(rect.left(), separator_y),
                pos2(rect.right(), separator_y),
            ],
            if is_active && split {
                egui::Stroke::new(2.0, crate::ui::ui_theme::accent())
            } else {
                egui::Stroke::new(1.0, crate::ui::ui_theme::hairline())
            },
        );

        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("pane-header", pane.pane))
                .max_rect(rect.shrink2(vec2(8.0, 2.0))),
            |ui| {
                ui.horizontal_centered(|ui| {
                    let available_title_width = (ui.available_width() - 40.0).max(48.0);
                    let title = ui
                        .add_sized(
                            vec2(available_title_width, 25.0),
                            egui::Button::selectable(is_active, &pane.name)
                                .frame(false)
                                .truncate(),
                        )
                        .on_hover_text(&pane.name);
                    crate::ui::accessibility::button(&title, &pane.name, !modal_open, Some(is_active));
                    if title.has_focus() && !modal_open {
                        ui.painter().rect_stroke(
                            title.rect,
                            3.0,
                            egui::Stroke::new(1.0, crate::ui::ui_theme::accent()),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if title.clicked() && !modal_open && !is_active {
                        self.workspace
                            .commands
                            .push_back(WorkspaceCommand::Activate(PaneTarget {
                                scene: pane.key,
                                pane: pane.pane,
                            }));
                    }

                    if split {
                        let label = self
                            .ui
                            .locale
                            .tr(crate::i18n::message_id!("workspace-fullscreen"));
                        let (_, response) =
                            ui.allocate_exact_size(vec2(26.0, 24.0), egui::Sense::click());
                        paint_pane_action(
                            ui,
                            &response,
                            AppIcon::FitView,
                            if is_active {
                                crate::ui::ui_theme::accent()
                            } else {
                                crate::ui::ui_theme::text_weak()
                            },
                            !modal_open,
                        );
                        let response = response.on_hover_text(&label);
                        crate::ui::accessibility::button(&response, &label, !modal_open, None);
                        if response.clicked() && !modal_open {
                            self.workspace
                                .commands
                                .push_back(WorkspaceCommand::Activate(PaneTarget {
                                    scene: pane.key,
                                    pane: pane.pane,
                                }));
                            self.workspace
                                .commands
                                .push_back(WorkspaceCommand::SetLayout(WorkspaceLayout::single(
                                    pane.pane,
                                )));
                        }
                    } else if self.workspace.scenes.len() > 1 {
                        let label = self
                            .ui
                            .locale
                            .tr(crate::i18n::message_id!("workspace-two-views"));
                        let (_, response) =
                            ui.allocate_exact_size(vec2(26.0, 24.0), egui::Sense::click());
                        paint_pane_action(
                            ui,
                            &response,
                            AppIcon::SplitView,
                            crate::ui::ui_theme::text_weak(),
                            !modal_open,
                        );
                        let response = response.on_hover_text(&label);
                        crate::ui::accessibility::button(&response, &label, !modal_open, None);
                        if response.clicked() && !modal_open {
                            let layout = self
                                .workspace
                                .saved_split
                                .unwrap_or_else(|| default_split(&self.workspace.scenes));
                            self.workspace
                                .commands
                                .push_back(WorkspaceCommand::SetLayout(layout));
                        }
                    }
                });
            },
        );
    }
    pub(super) fn show_divider(
        &mut self,
        ui: &mut egui::Ui,
        layout: EffectiveLayout,
        ctx: &egui::Context,
        modal_open: bool,
    ) -> Option<Rect> {
        let EffectiveLayout::SideBySide {
            left,
            right,
            divider,
        } = layout
        else {
            return None;
        };
        let id = egui::Id::new("workspace-scene-divider");
        let response = ui
            .interact(divider, id, egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        let label = self
            .ui
            .locale
            .tr(crate::i18n::message_id!("workspace-divider"));
        let current_ratio = match self.workspace.layout {
            WorkspaceLayout::SideBySide { ratio, .. } => ratio,
            WorkspaceLayout::Single { .. } => return Some(divider),
        };
        if modal_open {
            crate::ui::accessibility::slider(&response, &label, false, f64::from(current_ratio));
            return Some(divider);
        }

        let mut ratio = current_ratio;
        crate::ui::accessibility::slider(&response, &label, true, f64::from(ratio));
        if response.double_clicked() {
            ratio = 0.5;
        } else if response.dragged_by(egui::PointerButton::Primary)
            && self
                .workspace
                .input
                .capture()
                .is_some_and(|owner| owner.kind == GestureKind::DividerResize)
        {
            if let Some(pointer) = ctx.input(|input| input.pointer.interact_pos()) {
                let content_width = (left.rect.width() + right.rect.width()).max(1.0);
                ratio = ((pointer.x - left.rect.left() - divider.width() * 0.5) / content_width)
                    .clamp(0.05, 0.95);
            }
        } else if response.has_focus() {
            let step = 0.05;
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft))
            {
                ratio -= step;
            }
            if ctx
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight))
            {
                ratio += step;
            }
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Home)) {
                ratio = 0.5;
            }
        }
        let next_layout = self.workspace.layout.with_ratio(ratio);
        if next_layout != self.workspace.layout {
            self.workspace
                .commands
                .push_back(WorkspaceCommand::SetLayout(next_layout));
        }
        if response.has_focus() {
            ui.painter().rect_filled(
                divider.shrink(2.0),
                2.0,
                crate::ui::ui_theme::accent().gamma_multiply(0.16),
            );
            ui.painter().rect_stroke(
                divider.shrink(1.0),
                2.0,
                egui::Stroke::new(1.0, crate::ui::ui_theme::accent()),
                egui::StrokeKind::Inside,
            );
        }
        Some(divider)
    }
}

fn paint_pane_action(
    ui: &egui::Ui,
    response: &egui::Response,
    icon: AppIcon,
    ink: egui::Color32,
    enabled: bool,
) {
    let rect = response.rect;
    if enabled && (response.hovered() || response.has_focus()) {
        ui.painter()
            .rect_filled(rect, 3.0, crate::ui::ui_theme::row_hover_fill());
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                3.0,
                egui::Stroke::new(1.0, crate::ui::ui_theme::accent()),
                egui::StrokeKind::Inside,
            );
        }
    }
    crate::ui::icons::paint(ui.painter(), rect.shrink(5.0), icon, ink);
}
