//! The colour laws: the stops, the gate, the feather, and the slider.
//!
//! Each assertion is a property the operator reads the map by, and each is a
//! mistake that is easy to make and hard to see afterwards: red at the far end
//! rings every mark, a wide paint band drowns the marks, a feather that turns
//! pale reads as a lighting artefact, and one slider that moves both the touch
//! gate and the load depth makes both numbers unattributable.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

mod support;

use occluview_contact::{
    ContactScale, CLINICAL, LOAD_MAX_MM, LOAD_MIN_MM, SEARCH_RADIUS_MM, TIGHTNESS,
};

/// Every named stop reproduces the colour the law was written with.
///
/// A typo in the conversion matrices shows up here: these hexes are the design
/// table, and the evaluator has to land on them exactly.
#[test]
fn every_stop_reads_the_colour_it_was_written_with() {
    for law in [&TIGHTNESS, &CLINICAL] {
        let scale = ContactScale::new(law, law.load_mm);
        for (index, (mm, srgb)) in law.stops.iter().enumerate() {
            let colour = scale.color_at(*mm);
            if *mm >= law.paint_far_mm {
                assert_eq!(
                    colour[3], 0,
                    "the far edge is where the paint has already reached nothing: {mm} mm"
                );
                continue;
            }
            assert_eq!(
                [colour[0], colour[1], colour[2]],
                *srgb,
                "{} stop {index} at {mm} mm reads {colour:?}, not {srgb:?}",
                law.id
            );
            assert_eq!(colour[3], 255, "an inside stop is fully opaque: {mm} mm");
        }
    }
}

/// The feather ends at zero, is solid one fade below, and is halfway between.
#[test]
fn the_far_feather_runs_between_solid_and_nothing() {
    let scale = ContactScale::new(&CLINICAL, CLINICAL.load_mm);
    assert_eq!(scale.paint_weight_at(CLINICAL.paint_far_mm), 0.0);
    assert_eq!(
        scale.paint_weight_at(CLINICAL.paint_far_mm - CLINICAL.far_fade_mm),
        1.0
    );
    assert!(
        (scale.paint_weight_at(CLINICAL.paint_far_mm - CLINICAL.far_fade_mm / 2.0) - 0.5).abs()
            < 1e-9,
        "the feather is a smoothstep, so its midpoint is half opaque"
    );
    assert_eq!(scale.paint_weight_at(CLINICAL.paint_far_mm + 0.001), 0.0);
    assert_eq!(scale.paint_weight_at(f64::NAN), 0.0);
    assert_eq!(scale.paint_weight_at(f64::INFINITY), 0.0);
}

/// The gate and the paint agree everywhere except the single point where the
/// feather has just reached zero.
///
/// `is_painted` is the reach of the map — what the hover readout is allowed to
/// report and what the panel counts — while alpha is what is visible. A value
/// exactly at the far edge is inside the map and invisible, which is what makes
/// a contact at the edge of the band readable rather than a one-pixel gap.
#[test]
fn the_gate_and_the_paint_agree_on_what_is_shown() {
    for (law, load) in [(&TIGHTNESS, TIGHTNESS.load_mm), (&CLINICAL, 0.2)] {
        let scale = ContactScale::new(law, load);
        let far = law.paint_far_mm;
        let deepest = scale.stop_mm(law.stops.len() - 1);
        let steps = 400;
        for step in 0..=steps {
            let value = far - (f64::from(step) / f64::from(steps)) * (far - deepest);
            // The direction that matters: nothing is ever painted outside the
            // map. The other direction cannot hold to the byte — the last
            // fraction of a micrometre of the feather rounds to an alpha of
            // zero — so what is asserted is that the paint is a subset of the
            // gate, not that the gate is fully visible.
            if scale.color_at(value)[3] > 0 {
                assert!(
                    scale.is_painted(value),
                    "{} painted {value} mm without covering it",
                    law.id
                );
            }
        }
        // Solid wherever the feather has not started, which is everything the
        // mark itself is.
        assert_eq!(scale.color_at(far - law.far_fade_mm)[3], 255);
        assert!(scale.color_at(far - law.far_fade_mm / 2.0)[3] > 0);
        // The far edge itself: covered by the map, invisible on the surface.
        // A readout that excluded the value it is standing on would be worse
        // than one that shows it, so the gate keeps the edge and the alpha does
        // not.
        assert!(scale.is_painted(far));
        assert_eq!(scale.color_at(far), [0, 0, 0, 0]);
        // Above the gate: nothing painted, nothing covered.
        assert_eq!(scale.color_at(far + 0.01), [0, 0, 0, 0]);
        assert!(!scale.is_painted(far + 0.01));
        assert!(!scale.is_painted(f64::NAN));
        assert!(!scale.is_painted(f64::NEG_INFINITY));
    }
}

/// Red must sit on the load side and nowhere earlier.
///
/// Red at the far end would put a red ring around every mark, because a tooth
/// curves away from a contact within half a millimetre and the geometry then
/// guarantees the ring. So warmth has to rise the whole way into the bite, from
/// the lightest contact there is to where the law says red has arrived.
#[test]
fn red_rises_into_the_bite_and_never_comes_back_out() {
    let scale = ContactScale::new(&TIGHTNESS, TIGHTNESS.load_mm);
    assert!(
        warmth(scale.color_at(0.0)) < 0,
        "the lightest contact there is must read cool, not warm"
    );
    assert!(
        warmth(scale.color_at(-TIGHTNESS.load_mm)) > 0,
        "the load depth is where red has arrived"
    );

    // Strictly at the named stops, which is where the law makes its claim.
    // Oklab takes the shortest perceptual path between two stops, not the
    // shortest path in this test's crude scalar, so a few units of wobble is
    // the mixing being right rather than the ramp being wrong.
    let mut previous: Option<i32> = None;
    for (mm, _) in TIGHTNESS.stops {
        if *mm > 0.0 || *mm < -TIGHTNESS.load_mm {
            continue;
        }
        let here = warmth(scale.color_at(*mm));
        if let Some(earlier) = previous {
            assert!(
                here >= earlier - 12,
                "warmth fell away between stops at {mm} mm: {here} after {earlier}"
            );
        }
        previous = Some(here);
    }
}

/// The paint ends by opacity, never by turning pale.
#[test]
fn the_paint_leaves_by_opacity_and_never_by_turning_pale() {
    let scale = ContactScale::new(&TIGHTNESS, TIGHTNESS.load_mm);
    let touch = scale.color_at(0.0);
    let mut previous = 255_u8;
    for step in 0..=10 {
        let gap = f64::from(step) * (TIGHTNESS.paint_far_mm / 10.0);
        let faded = scale.color_at(gap);
        if faded[3] == 0 {
            continue;
        }
        assert_eq!(
            [faded[0], faded[1], faded[2]],
            [touch[0], touch[1], touch[2]],
            "the tolerance drifted in hue at {gap} mm instead of only in alpha"
        );
        assert!(
            faded[3] <= previous,
            "the feather is not monotonic at {gap} mm"
        );
        previous = faded[3];
    }
    assert!(previous < 255, "the feather never actually faded");
}

/// Articulating paper leaves the rest of the tooth bare.
///
/// Paint the whole approach band and a case with a handful of real contacts
/// reads as a field of colour with the marks lost inside it.
#[test]
fn articulating_paper_leaves_the_rest_of_the_tooth_bare() {
    let scale = ContactScale::new(&TIGHTNESS, TIGHTNESS.load_mm);
    assert_eq!(scale.color_at(0.05)[3], 0, "a 50 um gap is not a contact");
    assert_eq!(scale.color_at(0.2)[3], 0, "a 200 um gap is not a contact");
    assert_eq!(scale.color_at(1.0)[3], 0, "a 1 mm gap is certainly not");
    assert_eq!(
        scale.color_at(f64::NAN)[3],
        0,
        "a vertex that found no opposing surface is not painted, or guessed at"
    );
    assert_eq!(scale.color_at(0.0)[3], 255, "the touch line is a contact");
    assert_eq!(
        scale.color_at(-0.1)[3],
        255,
        "and so is 100 um into the bite"
    );
}

/// The slider moves the load depth and nothing else.
///
/// It must not move the gap side: that side is about measurement noise, not
/// about load, and a slider that stretched both would change what counts as
/// touching at the same time as what counts as heavy — after which no reading
/// on screen could be attributed to either.
#[test]
fn the_slider_scales_penetration_stops_only() {
    for law in [&TIGHTNESS, &CLINICAL] {
        let base = ContactScale::new(law, law.load_mm);
        let wide = ContactScale::new(law, LOAD_MAX_MM);
        assert_eq!(base.load_mm(), law.load_mm);
        assert_eq!(wide.load_mm(), LOAD_MAX_MM);
        let deepest = law.stops.len() - 1;
        assert!(
            wide.stop_table().stops[deepest][0] < base.stop_table().stops[deepest][0],
            "{}: a deeper load depth must stretch the penetration side",
            law.id
        );

        for index in 0..law.stops.len() {
            let (mm, _) = law.stops[index];
            if mm >= 0.0 {
                assert_eq!(
                    base.stop_mm(index),
                    wide.stop_mm(index),
                    "{}: the gap side moved with the slider at {mm} mm",
                    law.id
                );
                assert_eq!(
                    base.color_at(mm),
                    wide.color_at(mm),
                    "{}: the gap colour moved with the slider at {mm} mm",
                    law.id
                );
            } else {
                // The penetration side scales with the slider until the ramp
                // would ask for a depth the probe cannot reach; past that it
                // clamps. A stop deeper than the reach paints nothing, so the
                // clamp is what keeps every colour the ramp draws reachable.
                let scaled = mm * (LOAD_MAX_MM / law.load_mm);
                let expected = scaled.max(-SEARCH_RADIUS_MM);
                assert!(
                    (wide.stop_mm(index) - expected).abs() < 1e-9,
                    "{}: penetration stop {index} is {}, expected {expected}",
                    law.id,
                    wide.stop_mm(index)
                );
                assert!(
                    wide.stop_mm(index) <= base.stop_mm(index) + 1e-9,
                    "{}: the slider must not make a stop shallower",
                    law.id
                );
            }
        }
        // The touch line is a contact under both settings, at the same colour.
        assert_eq!(base.color_at(0.0), wide.color_at(0.0));
    }
}

/// Out-of-range slider requests are clamped, and a nonsense one falls back.
#[test]
fn the_load_depth_is_clamped_to_the_slider_range() {
    let law = &TIGHTNESS;
    assert_eq!(
        ContactScale::new(law, 0.001).load_mm(),
        LOAD_MIN_MM,
        "below the narrowest useful depth the ramp would report the scanner"
    );
    assert_eq!(ContactScale::new(law, 5.0).load_mm(), LOAD_MAX_MM);
    assert_eq!(
        ContactScale::new(law, f64::NAN).load_mm(),
        law.load_mm,
        "a non-finite request falls back to the law's own depth, not to a clamp of NaN"
    );
    assert_eq!(ContactScale::new(law, f64::INFINITY).load_mm(), law.load_mm);
}

/// Monotone in, monotone out: a deeper reading never reads cooler.
#[test]
fn a_deeper_reading_never_reads_cooler() {
    let scale = ContactScale::new(&TIGHTNESS, TIGHTNESS.load_mm);
    let mut previous: Option<i32> = None;
    for step in 0..=200 {
        let value = -(f64::from(step) / 100.0);
        let here = warmth(scale.color_at(value));
        if let Some(earlier) = previous {
            // Oklab takes the shortest perceptual path between two stops, not
            // the shortest path in this test's crude scalar, so a few units of
            // wobble is the mixing being right rather than the ramp going the
            // wrong way.
            assert!(
                here >= earlier - 12,
                "warmth reversed at {value} mm: {here} after {earlier}"
            );
        }
        previous = Some(here);
    }
}

/// How red a colour reads, measured against its loudest rival channel.
///
/// Not red-minus-blue: that calls orange warmer than red, because orange has
/// almost no blue in it. Measuring red against the strongest rival orders the
/// whole blue-cyan-green-yellow-orange-red path the way an eye does.
fn warmth(colour: [u8; 4]) -> i32 {
    i32::from(colour[0]) - i32::from(colour[1]).max(i32::from(colour[2]))
}

/// The ramp must never demand a depth the probe cannot reach.
///
/// The "heavy at" slider scales every penetration stop by `load / law.load_mm`,
/// so uncapped, the top of its range would put the deepest stop at 1.36 mm
/// (TIGHTNESS) or 1.75 mm (CLINICAL), while a vertex deeper than
/// `SEARCH_RADIUS_MM` inside the antagonist finds no surface at all. Such stops
/// would paint nothing — the field reports `NO_CONTACT_MM` and the mark has a
/// bare-tooth hole in it — and the legend would name depths no reading can show.
#[test]
fn no_ramp_stop_is_deeper_than_the_probe_can_reach() {
    for law in [&TIGHTNESS, &CLINICAL] {
        for load in [
            law.load_mm,
            f64::midpoint(law.load_mm, LOAD_MAX_MM),
            LOAD_MAX_MM,
        ] {
            let scale = ContactScale::new(law, load);
            for index in 0..law.stops.len() {
                let stop = scale.stop_mm(index);
                assert!(
                    stop >= -SEARCH_RADIUS_MM,
                    "stop {index} at load {load} is {stop} mm, past the {SEARCH_RADIUS_MM} mm reach"
                );
            }
        }
    }
}
