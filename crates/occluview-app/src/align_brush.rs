//! The exclusion brush: painting a region out of the comparison.
//!
//! An artefact, a bubble, or a bite block drags a registration off. Painting it
//! out removes it from the fit and from the deviation map, so both the pose and
//! the numbers describe the surface the operator actually cares about.

/// Smallest usable brush, in millimetres.
const MIN_RADIUS_MM: f32 = 0.1;
/// Largest usable brush, in millimetres.
const MAX_RADIUS_MM: f32 = 20.0;
/// Starting brush size — about a cusp.
const DEFAULT_RADIUS_MM: f32 = 1.5;

/// Brush state: whether it is armed, how big, and which way it paints.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AlignBrush {
    armed: bool,
    radius_mm: f32,
    erase: bool,
}

impl Default for AlignBrush {
    fn default() -> Self {
        Self {
            armed: false,
            radius_mm: DEFAULT_RADIUS_MM,
            erase: false,
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

    /// Whether the brush erases instead of paints.
    pub(crate) fn erases(self) -> bool {
        self.erase
    }

    /// Choose paint or erase. Holding Shift flips this for one stroke.
    pub(crate) fn set_erase(&mut self, erase: bool) {
        self.erase = erase;
    }
}

/// A whole-mask command from the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MaskCommand {
    /// Include everything.
    Nowhere,
    /// Exclude everything.
    Everywhere,
    /// Flip every vertex.
    Invert,
    /// Paint a disc around each clicked point.
    AroundPoints,
}

impl MaskCommand {
    /// What to tell the operator afterwards.
    pub(crate) fn report(self) -> &'static str {
        match self {
            Self::Nowhere => "Mask cleared — the whole scan is compared",
            Self::Everywhere => "Whole scan masked out",
            Self::Invert => "Mask inverted",
            Self::AroundPoints => "Masked around the clicked points",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AlignBrush, DEFAULT_RADIUS_MM, MAX_RADIUS_MM, MIN_RADIUS_MM};

    #[test]
    fn a_new_brush_is_idle_at_a_usable_size() {
        let brush = AlignBrush::default();
        assert!(!brush.is_armed());
        assert!(!brush.erases());
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
    fn arming_and_the_erase_flag_are_independent() {
        let mut brush = AlignBrush::default();
        brush.set_erase(true);
        assert!(brush.erases());
        assert!(!brush.is_armed(), "choosing erase must not arm the brush");
        brush.set_armed(true);
        assert!(brush.is_armed());
        assert!(brush.erases());
    }
}
