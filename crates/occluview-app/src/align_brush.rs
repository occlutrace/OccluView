//! The exclusion brush, built the way exocad's is.
//!
//! In exocad's Align Meshes the brush is not a mode of the manual tab. It is
//! reached from the automatic tab by ticking **Matching: Exclude selected
//! parts**, which opens a separate **Brush tool** window; painting marks the
//! surface that best-fit matching must *ignore*, and the marked surface goes
//! blue. That is the shape reproduced here, control for control:
//!
//! | exocad | here |
//! | --- | --- |
//! | Fit everywhere | clears every marking |
//! | Fit nowhere | marks the whole scan |
//! | Invert markings | swaps marked for unmarked |
//! | Mark automatic | keeps only a disc at each arrow end |
//! | Radius for automatic marking | that disc's radius |
//! | Brush size | the painting radius |
//! | Brush inverse (or hold SHIFT) | a stroke clears instead of marks |

/// Smallest usable brush, in millimetres.
const MIN_RADIUS_MM: f32 = 0.1;
/// Largest usable brush, in millimetres.
const MAX_RADIUS_MM: f32 = 20.0;
/// Starting brush size — about a cusp.
const DEFAULT_RADIUS_MM: f32 = 1.5;
/// Starting radius for automatic marking, about a landmark's worth of surface.
const DEFAULT_AUTO_RADIUS_MM: f32 = 3.0;
/// How much one wheel notch changes the radius.
const WHEEL_STEP_MM: f32 = 0.25;

/// Brush state: whether its window is open, how big it is, and which way a
/// stroke goes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AlignBrush {
    armed: bool,
    inverse: bool,
    radius_mm: f32,
    auto_radius_mm: f32,
}

impl Default for AlignBrush {
    fn default() -> Self {
        Self {
            armed: false,
            inverse: false,
            radius_mm: DEFAULT_RADIUS_MM,
            auto_radius_mm: DEFAULT_AUTO_RADIUS_MM,
        }
    }
}

impl AlignBrush {
    /// Whether the Brush tool window is open and pointer drags are painting.
    pub(crate) fn is_armed(self) -> bool {
        self.armed
    }

    /// Open or close the brush.
    pub(crate) fn set_armed(&mut self, armed: bool) {
        self.armed = armed;
    }

    /// Whether a plain stroke clears instead of marks.
    pub(crate) fn is_inverse(self) -> bool {
        self.inverse
    }

    /// Set the standing stroke direction.
    pub(crate) fn set_inverse(&mut self, inverse: bool) {
        self.inverse = inverse;
    }

    /// Whether a stroke clears, given whether Shift is held.
    ///
    /// exocad: "Brush inverse … You can also hold SHIFT while painting to
    /// inverse the brush." Held together they cancel, which is what "inverse"
    /// means and what an operator who has already set the toggle expects Shift
    /// to do.
    pub(crate) fn erases(self, shift: bool) -> bool {
        self.inverse != shift
    }

    /// Brush radius in millimetres.
    pub(crate) fn radius_mm(self) -> f32 {
        self.radius_mm
    }

    /// Set the radius, clamped to a size a hand can actually aim.
    pub(crate) fn set_radius_mm(&mut self, radius_mm: f32) {
        self.radius_mm = clamp_radius(radius_mm, DEFAULT_RADIUS_MM);
    }

    /// Radius of the disc "Mark automatic" keeps at each arrow end.
    pub(crate) fn auto_radius_mm(self) -> f32 {
        self.auto_radius_mm
    }

    /// Set the automatic-marking radius.
    pub(crate) fn set_auto_radius_mm(&mut self, radius_mm: f32) {
        self.auto_radius_mm = clamp_radius(radius_mm, DEFAULT_AUTO_RADIUS_MM);
    }

    /// Resize from a wheel notch. `notches` is signed: up grows the brush.
    pub(crate) fn nudge_radius(&mut self, notches: f32) {
        if !notches.is_finite() {
            return;
        }
        self.set_radius_mm(self.radius_mm + notches * WHEEL_STEP_MM);
    }
}

/// Keep a radius inside the range a hand can aim, falling back rather than
/// letting a broken number poison the brush.
fn clamp_radius(radius_mm: f32, fallback: f32) -> f32 {
    if radius_mm.is_finite() {
        radius_mm.clamp(MIN_RADIUS_MM, MAX_RADIUS_MM)
    } else {
        fallback
    }
}

/// A whole-mesh command from the Brush tool window, named as exocad names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MaskCommand {
    /// Clear every marking — the whole scan takes part in the match.
    FitEverywhere,
    /// Mark the whole scan, so best-fit matching has no effect.
    FitNowhere,
    /// Swap marked for unmarked.
    InvertMarkings,
    /// Keep only a disc of surface at each arrow end as the matching region.
    MarkAutomatic,
}

impl MaskCommand {
    /// The label on the button, verbatim from exocad.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::FitEverywhere => "Fit everywhere",
            Self::FitNowhere => "Fit nowhere",
            Self::InvertMarkings => "Invert markings",
            Self::MarkAutomatic => "Mark automatic",
        }
    }

    /// What the button does, in one line.
    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::FitEverywhere => "Clear all existing markings",
            Self::FitNowhere => "Mark the complete mesh — best-fit matching will have no effect",
            Self::InvertMarkings => "Mark unmarked areas and vice versa",
            Self::MarkAutomatic => "Match only on a small area around each arrow end",
        }
    }

    /// What to tell the operator afterwards.
    pub(crate) fn report(self) -> &'static str {
        match self {
            Self::FitEverywhere => "Markings cleared — matching on the whole scan",
            Self::FitNowhere => "Whole mesh marked — best-fit matching will have no effect",
            Self::InvertMarkings => "Markings inverted",
            Self::MarkAutomatic => "Matching only around the arrow ends",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AlignBrush, MaskCommand, DEFAULT_AUTO_RADIUS_MM, DEFAULT_RADIUS_MM, MAX_RADIUS_MM,
        MIN_RADIUS_MM,
    };

    #[test]
    fn a_new_brush_is_closed_at_a_usable_size() {
        let brush = AlignBrush::default();
        assert!(!brush.is_armed());
        assert!(!brush.is_inverse());
        assert!((brush.radius_mm() - DEFAULT_RADIUS_MM).abs() < f32::EPSILON);
        assert!((brush.auto_radius_mm() - DEFAULT_AUTO_RADIUS_MM).abs() < f32::EPSILON);
    }

    #[test]
    fn every_radius_is_clamped_to_a_size_a_hand_can_aim() {
        let mut brush = AlignBrush::default();
        brush.set_radius_mm(0.0);
        assert!((brush.radius_mm() - MIN_RADIUS_MM).abs() < f32::EPSILON);
        brush.set_radius_mm(1_000.0);
        assert!((brush.radius_mm() - MAX_RADIUS_MM).abs() < f32::EPSILON);
        brush.set_auto_radius_mm(0.0);
        assert!((brush.auto_radius_mm() - MIN_RADIUS_MM).abs() < f32::EPSILON);
        brush.set_auto_radius_mm(1_000.0);
        assert!((brush.auto_radius_mm() - MAX_RADIUS_MM).abs() < f32::EPSILON);
    }

    #[test]
    fn a_broken_radius_falls_back_instead_of_poisoning_the_brush() {
        let mut brush = AlignBrush::default();
        brush.set_radius_mm(f32::NAN);
        brush.set_auto_radius_mm(f32::NAN);
        assert!((brush.radius_mm() - DEFAULT_RADIUS_MM).abs() < f32::EPSILON);
        assert!((brush.auto_radius_mm() - DEFAULT_AUTO_RADIUS_MM).abs() < f32::EPSILON);
    }

    /// exocad's rule, and the reason the toggle and the key are one control: an
    /// operator who has set Brush inverse expects Shift to inverse THAT, not to
    /// be a second way of saying the same thing.
    #[test]
    fn shift_inverses_the_brush_whichever_way_it_is_already_set() {
        let mut brush = AlignBrush::default();
        assert!(!brush.erases(false), "a plain stroke marks");
        assert!(brush.erases(true), "Shift clears");
        brush.set_inverse(true);
        assert!(brush.erases(false), "inverse makes a plain stroke clear");
        assert!(!brush.erases(true), "Shift inverses the inverse");
    }

    /// Shift+wheel resizes. A notch that walked past the clamp would let the
    /// wheel poison the brush.
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

    /// The labels are exocad's, verbatim. An operator who knows that dialog
    /// must not have to work out which of our words means which of theirs.
    #[test]
    fn the_commands_carry_exocads_own_labels() {
        for (command, label) in [
            (MaskCommand::FitEverywhere, "Fit everywhere"),
            (MaskCommand::FitNowhere, "Fit nowhere"),
            (MaskCommand::InvertMarkings, "Invert markings"),
            (MaskCommand::MarkAutomatic, "Mark automatic"),
        ] {
            assert_eq!(command.label(), label);
            assert!(!command.hint().is_empty());
            assert!(!command.report().is_empty());
        }
    }
}
