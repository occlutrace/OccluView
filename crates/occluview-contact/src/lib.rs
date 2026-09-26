//! Occlusal contacts: where two dental surfaces meet, how hard, and in what
//! colour.
//!
//! # What the field is
//!
//! For every vertex of one surface this crate measures the distance to the
//! nearest point of the surface it bites against, signed by that surface's
//! outward triangle normal:
//!
//! * **positive** — a gap between the two surfaces, in millimetres;
//! * **zero** — exact touch;
//! * **negative** — penetration: the vertex sits behind the opposing surface,
//!   and the magnitude is how deep.
//!
//! No opposing surface inside the reach is not a zero: it is
//! [`NO_CONTACT_MM`], and it is never painted and never guessed at. A vertex
//! reads "met, 0.4 mm across" or "nothing within reach", and the panel can tell
//! those apart.
//!
//! # What it is not
//!
//! This is nearest-surface distance, not material overlap. A scan pair whose
//! two arches sit 0.1 mm apart reads 0.1 mm on both sides; a vertex 0.7 mm
//! inside the antagonist reads nothing at all, because [`SEARCH_RADIUS_MM`] is
//! 0.6 mm: the probe pays for its own radius on every vertex, so the reach is
//! kept short. It is above every law's saturation depth, so the only thing it
//! hides is overclosure no bite poses produce.
//!
//! The sign follows the *opposing* surface's winding, so an antagonist with
//! inverted normals turns gaps into penetrations and back. This is a property
//! of the measurement, not a defect, and callers that flip normals (`Flip
//! normals` in the layer menu) change which reading they get.
//!
//! # Geometry contract
//!
//! Inputs are [`occluview_align::Soup`]: indexed triangles in world space. The
//! caller bakes layer poses in before calling, as the align job does,
//! and the search index derives triangle normals from winding rather than from
//! imported vertex normals — so the sign uses the geometry that is on screen
//! rather than whatever the file claimed.
//!
//! # Why the colour law lives here too
//!
//! The field and the colour are one reading. A map whose colours come from a
//! second, separately tuned scale is a picture rather than a measurement, and
//! the two drift the first time either is touched: the surface paints one range
//! while the legend describes another. So the field, the two laws
//! ([`TIGHTNESS`], [`CLINICAL`]), the paint gate and the GPU tables all travel
//! together.
//!
//! There are three gates over the same signed field, and they have different
//! widths: [`ContactScale::is_painted`] (the hover readout and its swatch), the
//! shader's own painted weight (what reaches the screen), and the area/contact
//! counters' touch dead-band (see `stats`). They are not one shared predicate:
//! the measurement gate is wider so the counters do not fall short by a
//! feather's width.
//!
//! Painting happens per fragment from the table [`ContactScale::stop_table`]
//! compiles, never per vertex: the field is linear across a triangle and the
//! ramp is not, so a per-vertex colour would smear the ramp across whole
//! triangles and put the edge of the painted band wherever the tessellation
//! happened to fall. The shader re-runs the same interpolation over the same
//! numbers, so a CPU readout and a GPU pixel agree.

mod components;
mod field;
mod hover;
mod law;
mod lut;
mod stats;

pub use field::{compute_contact_field, ContactDiagnostics, ContactField, ContactSettings};
pub use hover::{
    format_contact_value, format_contact_value_in, interpolate_field_at_triangle, is_no_contact,
    ContactLengthUnit, ContactReading, ContactReadingKind,
};
pub use law::{
    ContactLaw, ContactScale, StopTable, CLINICAL, LOAD_MAX_MM, LOAD_MIN_MM, MAX_CONTACT_STOPS,
    TIGHTNESS,
};
pub use lut::{pack_field_texels, FieldTexels, FIELD_FAR_SENTINEL_MM};
pub use stats::ContactStats;

/// A vertex carries no measurement: no opposing surface inside the search
/// radius.
///
/// Infinite rather than zero because zero is a real reading — exact touch — and
/// because every consumer has to answer "was this measured?" before it can
/// answer "how much?". Consumers that hand a value to the GPU must substitute
/// a finite stand-in first; see [`FIELD_FAR_SENTINEL_MM`].
pub const NO_CONTACT_MM: f32 = f32::INFINITY;

/// Farthest a vertex looks for the opposing surface, in millimetres.
///
/// A vertex deeper inside the antagonist than this reach finds no surface and
/// falls back to [`NO_CONTACT_MM`], which paints as clean, bare tooth in the
/// middle of a strong interference mark — a "donut hole". The reach is therefore
/// also the deepest depth the field can report, and `ContactScale::stop_mm`
/// clamps the scaled ramp at it so no colour is drawn at a depth the probe
/// cannot deliver.
///
/// The number is a compromise: 0.6 mm is 1.2 x the widest `clamp_mm` (0.5 mm),
/// above every law's saturation depth so an interference inside the ramp's
/// usable range is never sentinelled, and low enough that a vertex in the
/// occlusal band does not pay a long probe time. Deep overclosure past this
/// reach is treated as a garbage-pose reading.
pub const SEARCH_RADIUS_MM: f64 = 0.6;

/// Sign dead-band, in millimetres.
///
/// A sub-micron interference reports the positive distance instead of a
/// negative depth. Below one micrometre the sign is decided by scanner noise
/// rather than by the bite, and a measurement that flips between "gap" and
/// "penetration" across a 0.5 µm wobble is worse than one that reads as touch.
pub(crate) const PENETRATION_EPS_MM: f32 = 0.001;

/// How often a long probe checks the caller's cancellation flag.
pub(crate) const CANCEL_CHECK_STRIDE: usize = 4096;
