use eframe::egui;

/// Reconstruct the discrete wheel delta that egui 0.29 exposed on `InputState`.
pub(super) fn wheel_delta_from_events(
    events: &[egui::Event],
    line_scroll_speed: f32,
    page_height: f32,
) -> egui::Vec2 {
    events.iter().fold(egui::Vec2::ZERO, |sum, event| {
        let egui::Event::MouseWheel {
            unit,
            delta,
            modifiers,
            ..
        } = event
        else {
            return sum;
        };

        let mut delta = match unit {
            egui::MouseWheelUnit::Point => *delta,
            egui::MouseWheelUnit::Line => line_scroll_speed * *delta,
            egui::MouseWheelUnit::Page => page_height * *delta,
        };
        if modifiers.shift {
            delta = egui::vec2(delta.x + delta.y, 0.0);
        }
        sum + delta
    })
}

/// Read this pass's unsmoothed wheel events without consuming them.
pub(super) fn raw_wheel_delta(ctx: &egui::Context) -> egui::Vec2 {
    let line_scroll_speed = ctx.options(|options| options.input_options.line_scroll_speed);
    ctx.input(|input| {
        wheel_delta_from_events(
            &input.raw.events,
            line_scroll_speed,
            input.viewport_rect().height(),
        )
    })
}

/// Read and selectively drain wheel input before later viewport consumers run.
pub(super) fn take_raw_wheel_delta(ctx: &egui::Context) -> egui::Vec2 {
    let line_scroll_speed = ctx.options(|options| options.input_options.line_scroll_speed);
    ctx.input_mut(|input| {
        let delta = wheel_delta_from_events(
            &input.raw.events,
            line_scroll_speed,
            input.viewport_rect().height(),
        );
        input
            .raw
            .events
            .retain(|event| !matches!(event, egui::Event::MouseWheel { .. }));
        input.smooth_scroll_delta = egui::Vec2::ZERO;
        delta
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wheel(
        unit: egui::MouseWheelUnit,
        delta: egui::Vec2,
        modifiers: egui::Modifiers,
    ) -> egui::Event {
        egui::Event::MouseWheel {
            unit,
            delta,
            phase: egui::TouchPhase::Move,
            modifiers,
        }
    }

    #[test]
    fn wheel_delta_from_events_keeps_point_units() {
        let events = [wheel(
            egui::MouseWheelUnit::Point,
            egui::vec2(2.5, -7.0),
            egui::Modifiers::NONE,
        )];

        assert_eq!(
            wheel_delta_from_events(&events, 40.0, 600.0),
            egui::vec2(2.5, -7.0)
        );
    }

    #[test]
    fn wheel_delta_from_events_scales_line_units() {
        let events = [wheel(
            egui::MouseWheelUnit::Line,
            egui::vec2(-2.0, 3.0),
            egui::Modifiers::NONE,
        )];

        assert_eq!(
            wheel_delta_from_events(&events, 40.0, 600.0),
            egui::vec2(-80.0, 120.0)
        );
    }

    #[test]
    fn wheel_delta_from_events_scales_page_units_by_viewport_height() {
        let events = [wheel(
            egui::MouseWheelUnit::Page,
            egui::vec2(0.5, -1.0),
            egui::Modifiers::NONE,
        )];

        assert_eq!(
            wheel_delta_from_events(&events, 40.0, 720.0),
            egui::vec2(360.0, -720.0)
        );
    }

    #[test]
    fn wheel_delta_from_events_uses_each_events_shift_modifier() {
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        let events = [
            wheel(egui::MouseWheelUnit::Point, egui::vec2(2.0, 3.0), shift),
            wheel(
                egui::MouseWheelUnit::Point,
                egui::vec2(7.0, 11.0),
                egui::Modifiers::NONE,
            ),
        ];

        assert_eq!(
            wheel_delta_from_events(&events, 40.0, 600.0),
            egui::vec2(12.0, 11.0)
        );
    }

    #[test]
    fn wheel_delta_from_events_sums_multiple_events_in_arrival_order() {
        let events = [
            wheel(
                egui::MouseWheelUnit::Point,
                egui::vec2(0.0, 1.0e20),
                egui::Modifiers::NONE,
            ),
            wheel(
                egui::MouseWheelUnit::Point,
                egui::vec2(0.0, -1.0e20),
                egui::Modifiers::NONE,
            ),
            wheel(
                egui::MouseWheelUnit::Point,
                egui::vec2(4.0, 3.0),
                egui::Modifiers::NONE,
            ),
        ];

        assert_eq!(
            wheel_delta_from_events(&events, 40.0, 600.0),
            egui::vec2(4.0, 3.0)
        );
    }

    #[test]
    fn wheel_delta_from_events_take_preserves_non_wheel_events() {
        let ctx = egui::Context::default();
        let pointer = egui::pos2(12.0, 18.0);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events: vec![
                egui::Event::PointerMoved(pointer),
                wheel(
                    egui::MouseWheelUnit::Point,
                    egui::vec2(0.0, 50.0),
                    egui::Modifiers::NONE,
                ),
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        };
        let mut taken = None;
        let mut remaining = Vec::new();
        let mut smooth = None;

        ctx.run_ui(input, |ui| {
            taken = Some(take_raw_wheel_delta(ui.ctx()));
            remaining = ui.ctx().input(|input| input.raw.events.clone());
            smooth = Some(ui.ctx().input(|input| input.smooth_scroll_delta));
        })
        .drop_without_applying_deltas();

        assert_eq!(taken, Some(egui::vec2(0.0, 50.0)));
        assert_eq!(remaining.len(), 2);
        assert!(matches!(remaining[0], egui::Event::PointerMoved(pos) if pos == pointer));
        assert!(matches!(
            remaining[1],
            egui::Event::Key {
                key: egui::Key::A,
                pressed: true,
                ..
            }
        ));
        assert_eq!(smooth, Some(egui::Vec2::ZERO));
    }

    #[test]
    fn wheel_delta_from_events_preserves_trackpad_phase_independence() {
        let events = [egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 120.0),
            phase: egui::TouchPhase::End,
            modifiers: egui::Modifiers::NONE,
        }];

        assert_eq!(
            wheel_delta_from_events(&events, 40.0, 600.0),
            egui::vec2(0.0, 120.0),
            "the raw-wheel contract reads each native event regardless of trackpad phase"
        );
    }
}
