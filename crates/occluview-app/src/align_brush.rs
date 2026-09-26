//! The exclusion brush, built the way dental CAD software builds it.
//!
//! In the align-meshes workflow operators already use, the brush is not a
//! mode of the manual tab. It is reached from the automatic tab by ticking
//! **Matching: Exclude selected parts**, which opens a separate **Brush
//! tool** window; painting marks the surface that best-fit matching must
//! *ignore*, and the marked surface goes blue. That is the shape reproduced
//! here, control for control:
//!
//! | dental CAD software | here |
//! | --- | --- |
//! | Fit everywhere | clears every marking |
//! | Fit nowhere | marks the whole scan |
//! | Invert markings | swaps marked for unmarked |
//! | Mark automatic | keeps only a disc at each arrow end |
//! | Radius for automatic marking | that disc's radius |
//! | Brush size | the painting radius |
//! | Brush inverse (or hold Shift) | a stroke clears instead of marks |

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

use crate::align_markings::AlignSide;

/// Which scan(s) the Brush tool acts on.
///
/// The window opens on [`Self::Both`], because the marking decides what the
/// match ignores on **either** surface and an operator who presses Fit nowhere
/// with two scans on screen means the pair, not whichever one happened to be
/// selected. Exocad's explicit Mesh selection is kept for the case it exists
/// for: aiming one scan when both overlap and the wrong one keeps taking the
/// stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BrushTarget {
    /// Every scan of the pair: a stroke paints the surface under the cursor,
    /// and a whole-mesh command reaches both.
    #[default]
    Both,
    /// Only the scan being placed.
    Moving,
    /// Only the scan that stays put.
    Fixed,
}

impl BrushTarget {
    /// Every target, in the order the Brush window lists them.
    pub(crate) const ALL: [Self; 3] = [Self::Both, Self::Moving, Self::Fixed];

    /// The sides this target covers, in the order they are acted on.
    pub(crate) fn sides(self) -> &'static [AlignSide] {
        match self {
            Self::Both => &AlignSide::BOTH,
            Self::Moving => &[AlignSide::Moving],
            Self::Fixed => &[AlignSide::Fixed],
        }
    }

    /// The target that keeps aiming at the same physical mesh after the roles
    /// are traded. A marking belongs to a surface, so `Moving` follows the
    /// scan it named rather than the role it was called by.
    pub(crate) fn swapped(self) -> Self {
        match self {
            Self::Both => Self::Both,
            Self::Moving => Self::Fixed,
            Self::Fixed => Self::Moving,
        }
    }
}

/// Brush state: whether its window is open, how big it is, and which member
/// of the alignment pair is selected for painting.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AlignBrush {
    armed: bool,
    inverse: bool,
    radius_mm: f32,
    auto_radius_mm: f32,
    target: BrushTarget,
}

impl Default for AlignBrush {
    fn default() -> Self {
        Self {
            armed: false,
            inverse: false,
            radius_mm: DEFAULT_RADIUS_MM,
            auto_radius_mm: DEFAULT_AUTO_RADIUS_MM,
            target: BrushTarget::default(),
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

    /// The mesh selection the Brush tool is aiming.
    pub(crate) fn target(self) -> BrushTarget {
        self.target
    }

    /// Select which member of the aligned pair receives strokes and commands.
    pub(crate) fn set_target(&mut self, target: BrushTarget) {
        self.target = target;
    }

    /// Keep the physical mesh selected when the user swaps moving and fixed.
    pub(crate) fn swap_target(&mut self) {
        self.target = self.target.swapped();
    }

    /// Forget a selection that referred to a scene that was cleared.
    pub(crate) fn reset_target(&mut self) {
        self.target = BrushTarget::default();
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
    /// Dental CAD software documents this as: "Brush inverse …
    /// You can also hold SHIFT while painting to inverse the brush." Held
    /// together they cancel, which is what "inverse" means and what an
    /// operator who has already set the toggle expects Shift to do.
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

#[cfg(test)]
mod tests {
    use super::{
        AlignBrush, BrushTarget, DEFAULT_AUTO_RADIUS_MM, DEFAULT_RADIUS_MM, MAX_RADIUS_MM,
        MIN_RADIUS_MM,
    };
    use crate::align_markings::AlignSide;

    #[test]
    fn a_new_brush_is_closed_at_a_usable_size_and_aims_both_scans() {
        let brush = AlignBrush::default();
        assert!(!brush.is_armed());
        assert!(!brush.is_inverse());
        assert!((brush.radius_mm() - DEFAULT_RADIUS_MM).abs() < f32::EPSILON);
        assert!((brush.auto_radius_mm() - DEFAULT_AUTO_RADIUS_MM).abs() < f32::EPSILON);
        assert_eq!(
            brush.target(),
            BrushTarget::Both,
            "a fresh brush marks the pair, not one arbitrary scan of it"
        );
    }

    /// Both is the target that makes Fit nowhere mean the pair. The other two
    /// exist for the overlapping case, where aiming one surface explicitly is
    /// the only way to stop the wrong one taking the stroke.
    #[test]
    fn both_covers_the_pair_and_a_named_target_covers_one_side() {
        assert_eq!(BrushTarget::Both.sides(), &AlignSide::BOTH);
        assert_eq!(BrushTarget::Moving.sides(), &[AlignSide::Moving]);
        assert_eq!(BrushTarget::Fixed.sides(), &[AlignSide::Fixed]);
        assert_eq!(BrushTarget::ALL.len(), 3);
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

    /// Dental CAD software follows this rule, and it is the reason the toggle
    /// and the key are one control: an operator who has set Brush inverse
    /// expects Shift to inverse that, not to be a second way of saying the same
    /// thing.
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

    #[test]
    fn mesh_selection_is_explicit_and_survives_role_swaps_by_physical_mesh() {
        let mut brush = AlignBrush::default();
        brush.set_target(BrushTarget::Fixed);
        assert_eq!(brush.target(), BrushTarget::Fixed);
        brush.swap_target();
        assert_eq!(brush.target(), BrushTarget::Moving);
        // Both names no physical mesh, so a role swap has nothing to follow.
        brush.set_target(BrushTarget::Both);
        brush.swap_target();
        assert_eq!(brush.target(), BrushTarget::Both);
        brush.set_target(BrushTarget::Moving);
        brush.reset_target();
        assert_eq!(brush.target(), BrushTarget::Both);
    }
}
