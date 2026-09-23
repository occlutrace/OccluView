//! The occlusal contact colour laws: what a signed field value looks like.
//!
//! # Two laws, one evaluator
//!
//! A law is data: band edges plus an ordered stop table. The interpolation, the
//! feather, the clamp and the conversion are written once, so
//! [`TIGHTNESS`] and [`CLINICAL`] differ in numbers and never in behaviour.
//!
//! * [`TIGHTNESS`] is digital articulating paper. It marks only where the arches
//!   actually meet and colours that by how deep the bite is there. This is the
//!   reading the case viewer ships and the one a technician makes.
//! * [`CLINICAL`] is the T-Scan convention: cool blues for safe proximity, warm
//!   reds only for load, painting the approach as well as the contact.
//!
//! # Three properties decide whether the map can be read at all
//!
//! Each was learned the expensive way upstream, and each is a mistake that is
//! easy to make and hard to see afterwards.
//!
//! **Red belongs on the load side.** Put red at the far end and every mark wears
//! a red ring, because a tooth curves away from a contact within half a
//! millimetre and the geometry then guarantees the ring.
//!
//! **Almost nothing is painted.** Paint the whole approach band and a case with
//! a handful of real contacts reads as a field of colour with the marks lost
//! inside it. Paper marks where it is squeezed and leaves the rest of the tooth
//! bare, so the far gate on [`TIGHTNESS`] is a ten-micrometre measurement
//! tolerance rather than a band.
//!
//! **The paint ends by alpha.** A ramp that fades toward white reads as a
//! lighting artefact rather than as data, so the far edge leaves by opacity and
//! the hue holds still underneath it.
//!
//! # Why Oklab
//!
//! The perceptual straight line between two stops has no neon band and no hue
//! overshoot, which a naive sRGB lerp across blue to cyan to green to yellow to
//! red always produces. The stops sit on the saturated path rather than on a
//! chord through it, and interpolating in Oklab keeps every intermediate colour
//! vivid where a long chord through sRGB passes through washed-out mud.
//!
//! The evaluator is exact rather than table-driven. A table would have to be
//! fine enough that every named stop lands on a sample or the stop colours
//! drift by interpolation error, and evaluating a dozen stops is free next to
//! the nearest-surface search that produced the field.

use std::sync::OnceLock;

/// Narrowest "heavy at" depth the slider offers, in millimetres.
///
/// Below this the whole ramp lives inside one scan's noise, and the map stops
/// reporting the bite and starts reporting the scanner.
pub const LOAD_MIN_MM: f64 = 0.05;

/// Widest "heavy at" depth the slider offers, in millimetres.
///
/// Past this nothing on the ramp reads as a contact any more.
pub const LOAD_MAX_MM: f64 = 0.60;

/// Most stops a compiled [`StopTable`] can carry.
///
/// The table is a fixed-size GPU payload, so the bound is part of the contract
/// rather than a dynamic length: a law that outgrew it would truncate silently
/// on the GPU and disagree with [`ContactScale::color_at`] on the CPU, which is
/// the one way the picture and the numbers can come apart.
pub const MAX_CONTACT_STOPS: usize = 16;

/// One colour law, as data.
///
/// Public so a caller can reason about the bands and the reach, and `static`
/// because the compiled Oklab form is memoised inside: a law built at runtime
/// would re-derive its stops on every colour.
#[derive(Debug)]
pub struct ContactLaw {
    /// Stable identifier: `"tightness"` or `"clinical"`.
    pub id: &'static str,
    /// Field values above this stay unpainted base surface.
    pub paint_far_mm: f64,
    /// Feather width at the far edge, where paint fades into the surface.
    pub far_fade_mm: f64,
    /// Penetration depth this law calls fully loaded — where the ramp turns red.
    /// This is the number the slider moves.
    pub load_mm: f64,
    /// Penetration past this clamps to the last stop, so one gross interference
    /// cannot flatten the useful part of the ramp.
    pub clamp_mm: f64,
    /// `(millimetres, display sRGB)` per stop, descending in millimetres from
    /// the far edge down to the deep clamp. The evaluator relies on that order.
    pub stops: &'static [(f64, [u8; 3])],
    /// Oklab form of `stops`, built once on first use.
    oklab: OnceLock<Vec<Oklab>>,
}

/// Digital articulating paper: only where the arches meet, coloured by how hard.
///
/// Nothing is painted where the teeth do not touch. The far gate is a ten
/// micrometre measurement tolerance rather than a band — below what a scan pair
/// can resolve — so a vertex measured a hair short of touching still belongs to
/// its mark while any real gap stays bare surface. The tolerance carries the
/// touch colour flat, so inside it only the opacity changes and a vertex fades
/// out instead of drifting to some other colour.
pub static TIGHTNESS: ContactLaw = ContactLaw {
    id: "tightness",
    paint_far_mm: 0.01,
    far_fade_mm: 0.01,
    load_mm: 0.22,
    clamp_mm: 0.5,
    stops: &[
        (0.01, [29, 78, 216]), // #1d4ed8 the tolerance, carrying the touch colour flat
        (0.0, [29, 78, 216]),  // #1d4ed8 the touch line: the lightest contact there is
        (-0.03, [21, 149, 201]), // #1595c9
        (-0.06, [18, 171, 143]), // #12ab8f
        (-0.09, [64, 192, 87]), // #40c057 a normal working contact
        (-0.12, [183, 203, 39]), // #b7cb27 about one thickness of articulating paper
        (-0.15, [252, 196, 25]), // #fcc419 firm
        (-0.18, [251, 106, 26]), // #fb6a1a strong, on its way to red
        (-0.22, [239, 62, 54]), // #ef3e36 RED STARTS HERE, and nowhere earlier
        (-0.32, [193, 39, 45]), // #c1272d heavy
        (-0.5, [122, 18, 18]), // #7a1212 gross interference
    ],
    oklab: OnceLock::new(),
};

/// The T-Scan convention: blue is safe proximity, red is load, and the approach
/// is painted as well as the contact.
///
/// The reading for "how close is the antagonist" rather than "where does it
/// touch". Both questions get asked about the same bite, so both laws exist.
pub static CLINICAL: ContactLaw = ContactLaw {
    id: "clinical",
    paint_far_mm: 0.2,
    far_fade_mm: 0.06,
    load_mm: 0.12,
    clamp_mm: 0.35,
    stops: &[
        (0.2, [165, 216, 255]),   // #a5d8ff far fade edge, barely visible
        (0.12, [77, 171, 247]),   // #4dabf7 the calm zone
        (0.06, [59, 201, 219]),   // #3bc9db still calm, approaching the warm transition
        (0.025, [105, 219, 124]), // #69db7c light proximity, about to graze
        (0.0, [255, 212, 59]),    // #ffd43b exact touch
        (-0.025, [255, 146, 43]), // #ff922b initial penetration
        (-0.06, [250, 82, 82]),   // #fa5252 loaded contact
        (-0.12, [224, 49, 49]),   // #e03131 heavy load
        (-0.35, [140, 17, 17]),   // #8c1111 deep interference
    ],
    oklab: OnceLock::new(),
};

/// The compiled law, as the GPU renderer consumes it.
///
/// A fixed-size, `Copy` payload rather than a borrowed slice, because it crosses
/// into a uniform buffer: the shader re-runs exactly the interpolation the CPU
/// runs in [`ContactScale::color_at`], and the only thing that keeps the two
/// honest is that both read the same compiled table.
///
/// `Copy` rather than `Clone` deliberately: this is written per frame for a
/// layer showing contacts, and an allocation there is a per-frame cost for no
/// benefit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StopTable {
    /// `(mm, L, a, b)` per stop, descending in millimetres; unused slots zeroed.
    ///
    /// `mm` is already scaled by the slider, so the shader never has to know a
    /// penetration stop moves and a gap stop does not.
    pub stops: [[f32; 4]; MAX_CONTACT_STOPS],
    /// Number of live slots in `stops`.
    pub count: u32,
    /// `(far end mm, span mm, widest painted gap mm, far-edge feather mm)`.
    pub ramp: [f32; 4],
    /// `(field texture texels per row, 0)`.
    ///
    /// The crate cannot fill this: the field texture's shape belongs to
    /// [`crate::pack_field_texels`]'s caller, which chooses `max_width` for its
    /// own device limits. The caller writes the width it packed into slot 0.
    pub gap: [f32; 2],
}

/// The scale as the operator has it set: a law, and the load depth the slider
/// moved to.
///
/// One number, because the question a bite poses is a question about a
/// threshold — where does close stop being contact and start being pressure —
/// and the honest way to answer it is to move the threshold and watch the map,
/// not to type a value. The gap side does not move with it: that side is about
/// measurement noise, not about load.
#[derive(Clone, Copy, Debug)]
pub struct ContactScale {
    law: &'static ContactLaw,
    /// Multiplier on the penetration-side stop positions only.
    depth: f64,
}

impl ContactScale {
    /// The scale for `law` with the ramp reaching full load at `load_mm`.
    ///
    /// A non-finite request falls back to the law's own load depth rather than
    /// to a clamped NaN, and every request is clamped to
    /// [`LOAD_MIN_MM`]..=[`LOAD_MAX_MM`]: outside that window the slider stops
    /// describing a bite and starts describing the scanner or nothing at all.
    pub fn new(law: &'static ContactLaw, load_mm: f64) -> Self {
        let load = if load_mm.is_finite() {
            load_mm.clamp(LOAD_MIN_MM, LOAD_MAX_MM)
        } else {
            law.load_mm
        };
        Self {
            law,
            depth: load / law.load_mm,
        }
    }

    /// The law this scale paints with.
    pub fn law(&self) -> &'static ContactLaw {
        self.law
    }

    /// The load depth the ramp reaches full red at, in millimetres.
    pub fn load_mm(&self) -> f64 {
        self.law.load_mm * self.depth
    }

    /// Where stop `index` sits on the signed field once the slider has moved.
    ///
    /// Only the penetration side scales. Stretching the tolerance on the gap
    /// side with it would make one slider change what counts as touching at the
    /// same time as what counts as heavy, and then no single reading on screen
    /// could be attributed to either.
    /// The deepest stop is capped at the probe's reach. `depth` scales with the
    /// operator's "heavy at" slider, and at the top of its range it put the last
    /// stop at 1.36 mm (TIGHTNESS) or 1.75 mm (Approach) — depths the field
    /// cannot report, because a vertex deeper than [`SEARCH_RADIUS_MM`] inside
    /// the antagonist finds no surface at all. The legend then named depths no
    /// reading could reach.
    ///
    /// Collapsing the unreachable tail onto the reach makes the last stops
    /// coincide at high slider values, so the topmost colour is what a
    /// maximum-depth reading gets rather than a span nothing can occupy. The
    /// sentinel case itself is unchanged and cannot be fixed here: a
    /// penetration past the reach is still [`NO_CONTACT_MM`], which is why the
    /// reach is set above every law's saturation depth in the first place.
    pub fn stop_mm(&self, index: usize) -> f64 {
        self.law.stops.get(index).map_or(0.0, |(mm, _)| {
            if *mm < 0.0 {
                (mm * self.depth).max(-crate::SEARCH_RADIUS_MM)
            } else {
                *mm
            }
        })
    }

    /// How opaque the paint is at `signed_mm`: fully on across the scale, and
    /// feathered to nothing over the last part of the far band.
    ///
    /// Smoothstep rather than linear, so the feather has no visible crease
    /// where it begins. Zero means bare surface, which is the point of the
    /// tightness law rather than an edge case of it.
    pub fn paint_weight_at(&self, signed_mm: f64) -> f64 {
        self.law.paint_weight_at(signed_mm)
    }

    /// Whether this value is inside the painted range at all.
    ///
    /// Whether the hover SWATCH counts this value as painted.
    ///
    /// Not a shared predicate: the screen uses the shader's own weight and the
    /// panel's counters use `stats::TOUCH_MM`, which is deliberately wider so a
    /// measured area does not fall short by a feather's width. An earlier
    /// version of this comment called it "the ONE predicate the paint path, the
    /// hover readout and the panel share", which was never true of all three.
    ///
    /// It is the *reach* of the map: at exactly the far edge the feather has
    /// already reached zero, so a value there is inside the map and invisible on
    /// the surface. That single point is the only difference from
    /// [`Self::color_at`]'s alpha, and it is deliberate — a readout that
    /// excludes the value it is standing on is worse than one that shows it.
    pub fn is_painted(&self, signed_mm: f64) -> bool {
        signed_mm.is_finite() && signed_mm <= self.law.paint_far_mm
    }

    /// The colour at a signed field value: display sRGB, with the far feather
    /// carried in alpha.
    ///
    /// A non-finite value is a vertex that found no opposing surface. It is not
    /// painted, and it is not guessed at either.
    pub fn color_at(&self, signed_mm: f64) -> [u8; 4] {
        let weight = self.paint_weight_at(signed_mm);
        if weight <= 0.0 {
            return [0, 0, 0, 0];
        }
        let Some(colour) = self.interpolate_oklab(signed_mm) else {
            return [0, 0, 0, 0];
        };
        let [red, green, blue] = linear_to_srgb(oklab_to_linear(colour));
        [red, green, blue, to_u8(weight)]
    }

    /// The compiled stop table, with the slider already applied.
    ///
    /// Built on demand rather than cached: it is 280 bytes of arithmetic over
    /// at most sixteen stops, and a cache would have to be keyed on the scale
    /// (law pointer plus depth) to be correct — more state than it saves.
    pub fn stop_table(&self) -> StopTable {
        let mut table = StopTable {
            stops: [[0.0; 4]; MAX_CONTACT_STOPS],
            count: 0,
            ramp: [0.0, 0.0, 0.0, 0.0],
            gap: [0.0, 0.0],
        };
        let Some(oklab) = self.compiled_oklab() else {
            return table;
        };
        let count = self.law.stops.len().min(MAX_CONTACT_STOPS);
        if count == 0 || oklab.len() < count {
            return table;
        }
        for (index, colour) in oklab.iter().take(count).enumerate() {
            table.stops[index] = [
                to_f32(self.stop_mm(index)),
                to_f32(colour.lightness),
                to_f32(colour.green_red),
                to_f32(colour.blue_yellow),
            ];
        }
        let far = self.stop_mm(0);
        let deepest = self.stop_mm(count - 1);
        table.count = to_u32(count);
        table.ramp = [
            to_f32(far),
            to_f32(far - deepest),
            to_f32(self.law.paint_far_mm),
            to_f32(self.law.far_fade_mm),
        ];
        table
    }

    /// The law's stops in Oklab, built once per law.
    fn compiled_oklab(&self) -> Option<&[Oklab]> {
        let compiled = self.law.oklab.get_or_init(|| {
            self.law
                .stops
                .iter()
                .map(|(_, srgb)| linear_to_oklab(srgb_to_linear(*srgb)))
                .collect()
        });
        if compiled.len() == self.law.stops.len() {
            Some(compiled.as_slice())
        } else {
            // Unreachable while `oklab` is derived from `stops` in one place,
            // and kept as a guard because the alternative is indexing a shorter
            // vector and taking the process down inside a colour lookup.
            None
        }
    }

    /// Piecewise-linear in Oklab between the stops, clamped to the end stops.
    fn interpolate_oklab(&self, signed_mm: f64) -> Option<Oklab> {
        let oklab = self.compiled_oklab()?;
        let last = oklab.len().checked_sub(1)?;
        let value = signed_mm.clamp(self.stop_mm(last), self.stop_mm(0));
        for index in 0..last {
            let high_mm = self.stop_mm(index);
            let low_mm = self.stop_mm(index + 1);
            if value > high_mm || value < low_mm {
                continue;
            }
            let span = high_mm - low_mm;
            let t = if span <= 0.0 {
                0.0
            } else {
                (high_mm - value) / span
            };
            let high = *oklab.get(index)?;
            let low = *oklab.get(index + 1)?;
            return Some(high.mix(low, t));
        }
        oklab.get(last).copied()
    }
}

impl ContactLaw {
    /// How opaque the paint is at `signed_mm`. See
    /// [`ContactScale::paint_weight_at`] — the tolerance does not scale with
    /// the slider, so this is the law's own answer.
    pub fn paint_weight_at(&self, signed_mm: f64) -> f64 {
        if !signed_mm.is_finite() || signed_mm > self.paint_far_mm {
            return 0.0;
        }
        if self.far_fade_mm <= 0.0 || signed_mm <= self.paint_far_mm - self.far_fade_mm {
            return 1.0;
        }
        let t = ((self.paint_far_mm - signed_mm) / self.far_fade_mm).clamp(0.0, 1.0);
        // t*t*(3 - 2t), the smoothstep the shader mirrors.
        t * t * t.mul_add(-2.0, 3.0)
    }
}

/// A colour in Oklab, the space the stops are interpolated through.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Oklab {
    /// Perceptual lightness.
    lightness: f64,
    /// Green to red axis.
    green_red: f64,
    /// Blue to yellow axis.
    blue_yellow: f64,
}

impl Oklab {
    /// The straight perceptual line from `self` to `other` at `t`.
    fn mix(self, other: Self, t: f64) -> Self {
        Self {
            lightness: self.lightness + (other.lightness - self.lightness) * t,
            green_red: self.green_red + (other.green_red - self.green_red) * t,
            blue_yellow: self.blue_yellow + (other.blue_yellow - self.blue_yellow) * t,
        }
    }
}

/// Display sRGB bytes to linear light.
fn srgb_to_linear(srgb: [u8; 3]) -> [f64; 3] {
    srgb.map(|channel| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    })
}

/// Linear light back to display sRGB bytes.
fn linear_to_srgb(linear: [f64; 3]) -> [u8; 3] {
    linear.map(|value| {
        let clipped = value.clamp(0.0, 1.0);
        let srgb = if clipped <= 0.003_130_8 {
            clipped * 12.92
        } else {
            1.055_f64.mul_add(clipped.powf(1.0 / 2.4), -0.055)
        };
        to_u8(srgb)
    })
}

/// Linear light to Oklab.
fn linear_to_oklab(linear: [f64; 3]) -> Oklab {
    let [red, green, blue] = linear;
    let long = 0.412_221_470_8 * red + 0.536_332_536_3 * green + 0.051_445_992_9 * blue;
    let medium = 0.211_903_498_2 * red + 0.680_699_545_1 * green + 0.107_396_956_6 * blue;
    let short = 0.088_302_461_9 * red + 0.281_718_837_6 * green + 0.629_978_700_5 * blue;
    let (long, medium, short) = (long.cbrt(), medium.cbrt(), short.cbrt());
    Oklab {
        lightness: 0.210_454_255_3 * long + 0.793_617_785 * medium - 0.004_072_046_8 * short,
        green_red: 1.977_998_495_1 * long - 2.428_592_205 * medium + 0.450_593_709_9 * short,
        blue_yellow: 0.025_904_037_1 * long + 0.782_771_766_2 * medium - 0.808_675_766 * short,
    }
}

/// Oklab back to linear light, each channel clamped into the displayable gamut.
///
/// The clamp lives here rather than after the sRGB encode because an
/// out-of-gamut Oklab value has no sRGB representation at all: letting the
/// encode saturate instead would skew the hue of every saturated stop.
fn oklab_to_linear(colour: Oklab) -> [f64; 3] {
    let long = colour.lightness
        + 0.396_337_777_4 * colour.green_red
        + 0.215_803_757_3 * colour.blue_yellow;
    let medium = colour.lightness
        - 0.105_561_345_8 * colour.green_red
        - 0.063_854_172_8 * colour.blue_yellow;
    let short =
        colour.lightness - 0.089_484_177_5 * colour.green_red - 1.291_485_548 * colour.blue_yellow;
    let (long, medium, short) = (long.powi(3), medium.powi(3), short.powi(3));
    [
        (4.076_741_662_1 * long - 3.307_711_591_3 * medium + 0.230_969_929_2 * short)
            .clamp(0.0, 1.0),
        (-1.268_438_004_6 * long + 2.609_757_401_1 * medium - 0.341_319_396_5 * short)
            .clamp(0.0, 1.0),
        (-0.004_196_086_3 * long - 0.703_418_614_7 * medium + 1.707_614_701 * short)
            .clamp(0.0, 1.0),
    ]
}

/// A paint weight, clamped and rounded into a byte.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_u8(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A stop position or colour channel narrowed for the GPU payload.
#[allow(clippy::cast_possible_truncation)]
fn to_f32(value: f64) -> f32 {
    value as f32
}

/// A count narrowed for the GPU payload, saturating rather than wrapping.
#[allow(clippy::cast_possible_truncation)]
fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
