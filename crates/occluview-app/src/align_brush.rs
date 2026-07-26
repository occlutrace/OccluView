//! The region brush: choosing which part of a scan takes part in the match.
//!
//! An artefact, a bubble, or a bite block drags a registration off. Taking it
//! out of the region removes it from the fit and from the deviation map, so
//! both the pose and the numbers describe the surface the operator cares about.
//!
//! The brush works the way lab software does. There is no separate erase key:
//! one control says whether a stroke *adds* surface to the match or *takes it
//! out*, and Shift+wheel resizes the brush — the same gesture the sculpt brush
//! already uses, so there is one size gesture in the whole application rather
//! than one per tool.

/// Smallest usable brush, in millimetres.
const MIN_RADIUS_MM: f32 = 0.1;
/// Largest usable brush, in millimetres.
const MAX_RADIUS_MM: f32 = 20.0;
/// Starting brush size — about a cusp.
const DEFAULT_RADIUS_MM: f32 = 1.5;
/// How much one wheel notch changes the radius.
const WHEEL_STEP_MM: f32 = 0.25;

/// What a stroke does to the surface it passes over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BrushPaint {
    /// Take the painted surface out of the match — the common case, because
    /// what an operator points at is usually the thing that should not count.
    #[default]
    Ignore,
    /// Put the painted surface back into the match. With "None" first, this is
    /// how an operator says "match on this region and nothing else".
    Use,
}

impl BrushPaint {
    /// The label on the control.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Ignore => "Ignore",
            Self::Use => "Use",
        }
    }

    /// What a stroke means, in one line.
    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::Ignore => "Paint over surface that must not take part in the match",
            Self::Use => "Paint over surface that must take part in the match",
        }
    }

    /// Whether a dab clears the mask rather than setting it.
    pub(crate) fn erases(self) -> bool {
        self == Self::Use
    }
}

/// Brush state: whether it is painting, what it paints, and how big it is.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AlignBrush {
    armed: bool,
    paint: BrushPaint,
    radius_mm: f32,
}

impl Default for AlignBrush {
    fn default() -> Self {
        Self {
            armed: false,
            paint: BrushPaint::default(),
            radius_mm: DEFAULT_RADIUS_MM,
        }
    }
}

impl AlignBrush {
    /// Whether pointer drags are painting.
    pub(crate) fn is_armed(self) -> bool {
        self.armed
    }

    /// Turn painting on or off.
    pub(crate) fn set_armed(&mut self, armed: bool) {
        self.armed = armed;
    }

    /// What a stroke marks.
    pub(crate) fn paint(self) -> BrushPaint {
        self.paint
    }

    /// Choose what a stroke marks.
    pub(crate) fn set_paint(&mut self, paint: BrushPaint) {
        self.paint = paint;
    }

    /// Brush radius in millimetres.
    pub(crate) fn radius_mm(self) -> f32 {
        self.radius_mm
    }

    /// Set the radius, clamped to a size a hand can actually aim.
    pub(crate) fn set_radius_mm(&mut self, radius_mm: f32) {
        self.radius_mm = if radius_mm.is_finite() {
            radius_mm.clamp(MIN_RADIUS_MM, MAX_RADIUS_MM)
        } else {
            DEFAULT_RADIUS_MM
        };
    }

    /// Resize from a wheel notch. `notches` is signed: up grows the brush.
    pub(crate) fn nudge_radius(&mut self, notches: f32) {
        if !notches.is_finite() {
            return;
        }
        self.set_radius_mm(self.radius_mm + notches * WHEEL_STEP_MM);
    }
}

/// A whole-region command from the panel.
///
/// Named for what the operator gets, not for what the mask byte becomes. The
/// question in front of them is "what is being matched", and a button called
/// "mask everything" answers the opposite one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MaskCommand {
    /// Match the whole scan.
    Everything,
    /// Match nothing yet — the starting point for painting a region in.
    Nothing,
    /// Swap what is matched for what is not.
    Invert,
}

impl MaskCommand {
    /// The label on the button.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Everything => "All",
            Self::Nothing => "None",
            Self::Invert => "Invert",
        }
    }

    /// What the button does, in one line.
    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::Everything => "Match on the whole scan",
            Self::Nothing => "Match on nothing — then paint in the part that counts",
            Self::Invert => "Swap what is matched for what is not",
        }
    }

    /// What to tell the operator afterwards.
    pub(crate) fn report(self) -> &'static str {
        match self {
            Self::Everything => "The whole scan takes part in the match",
            Self::Nothing => "Nothing takes part yet — paint in the part that counts",
            Self::Invert => "Region inverted",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AlignBrush, BrushPaint, MaskCommand, DEFAULT_RADIUS_MM, MAX_RADIUS_MM, MIN_RADIUS_MM,
    };

    #[test]
    fn a_new_brush_is_idle_at_a_usable_size() {
        let brush = AlignBrush::default();
        assert!(!brush.is_armed());
        assert!((brush.radius_mm() - DEFAULT_RADIUS_MM).abs() < f32::EPSILON);
    }

    #[test]
    fn the_radius_is_clamped_to_a_size_a_hand_can_aim() {
        let mut brush = AlignBrush::default();
        brush.set_radius_mm(0.0);
        assert!((brush.radius_mm() - MIN_RADIUS_MM).abs() < f32::EPSILON);
        brush.set_radius_mm(1_000.0);
        assert!((brush.radius_mm() - MAX_RADIUS_MM).abs() < f32::EPSILON);
    }

    #[test]
    fn a_broken_radius_falls_back_instead_of_poisoning_the_brush() {
        let mut brush = AlignBrush::default();
        brush.set_radius_mm(f32::NAN);
        assert!((brush.radius_mm() - DEFAULT_RADIUS_MM).abs() < f32::EPSILON);
        assert!(brush.radius_mm().is_finite());
    }

    #[test]
    fn arming_survives_a_radius_change() {
        let mut brush = AlignBrush::default();
        brush.set_armed(true);
        brush.set_radius_mm(4.0);
        assert!(brush.is_armed());
        assert!((brush.radius_mm() - 4.0).abs() < f32::EPSILON);
    }

    /// Shift+wheel is the size gesture in this application. A notch that walked
    /// straight past the clamp would let the wheel poison the brush.
    #[test]
    fn the_wheel_resizes_within_the_same_limits_as_the_slider() {
        let mut brush = AlignBrush::default();
        brush.nudge_radius(4.0);
        assert!(brush.radius_mm() > DEFAULT_RADIUS_MM);
        brush.nudge_radius(-1_000.0);
        assert!((brush.radius_mm() - MIN_RADIUS_MM).abs() < f32::EPSILON);
        brush.nudge_radius(10_000.0);
        assert!((brush.radius_mm() - MAX_RADIUS_MM).abs() < f32::EPSILON);
        brush.nudge_radius(f32::NAN);
        assert!(brush.radius_mm().is_finite());
    }

    /// The brush replaced a Shift-to-erase gesture, because Shift now resizes.
    /// The two directions have to come from this control or one of them is
    /// unreachable.
    #[test]
    fn the_paint_control_covers_both_directions() {
        let mut brush = AlignBrush::default();
        assert_eq!(brush.paint(), BrushPaint::Ignore);
        assert!(!brush.paint().erases());
        brush.set_paint(BrushPaint::Use);
        assert!(brush.paint().erases());
    }

    /// The buttons are named for what the operator gets. A label that named the
    /// mask byte instead would answer the opposite question to the one they are
    /// asking.
    #[test]
    fn every_region_command_says_what_it_matches() {
        for command in [
            MaskCommand::Everything,
            MaskCommand::Nothing,
            MaskCommand::Invert,
        ] {
            let hint = command.hint().to_lowercase();
            assert!(
                hint.contains("match"),
                "{command:?} says {hint:?}, which never mentions matching"
            );
            assert!(!command.label().is_empty());
            assert!(!command.report().is_empty());
        }
    }
}
