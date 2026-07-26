//! PROOF tests for [`crate::deviation`].
//!
//! Split out under `#[path]` rather than left inline: the module they cover is
//! already at the file-size budget, and a proof suite that cannot grow is a
//! proof suite that stops being written.

use super::{
    deviation, deviation_colors, deviation_stats, ramp_color, suggested_scale_mm, DeviationMap,
    DeviationSettings, DeviationStats, RampMode, RampSettings, Validity, MAGNITUDE_RAMP,
    NO_DATA_COLOR,
};
use crate::icp::Orientation;
use crate::{CancelFlag, Rigid, Soup, SurfaceIndex};

/// Two triangles forming a 10 x 10 sheet on z = 0, outward normal +Z.
fn sheet() -> (Vec<f32>, Vec<u32>) {
    (
        vec![
            0.0, 0.0, 0.0, //
            10.0, 0.0, 0.0, //
            10.0, 10.0, 0.0, //
            0.0, 10.0, 0.0,
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

fn settings() -> DeviationSettings {
    DeviationSettings {
        influence_radius_mm: 3.0,
        orientation: Orientation::Match,
    }
}

fn lifted(positions: &[f32], by: f32) -> Vec<f32> {
    positions
        .chunks_exact(3)
        .flat_map(|point| [point[0], point[1], point[2] + by])
        .collect()
}

fn map_of(moving: &[f32], indices: &[u32], settings: &DeviationSettings) -> DeviationMap {
    let (fixed_positions, fixed_indices) = sheet();
    let index = SurfaceIndex::build(Soup {
        positions: &fixed_positions,
        indices: &fixed_indices,
        mask: None,
    })
    .unwrap();
    deviation(
        Soup {
            positions: moving,
            indices,
            mask: None,
        },
        &index,
        Rigid::IDENTITY,
        settings,
        &CancelFlag::new(),
    )
}

#[test]
fn a_surface_above_the_target_reads_positive() {
    let (positions, indices) = sheet();
    let map = map_of(&lifted(&positions, 0.4), &indices, &settings());
    for (value, state) in map.signed_mm.iter().zip(&map.validity) {
        assert_eq!(*state, Validity::Measured);
        assert!((*value - 0.4).abs() < 1e-4, "expected +0.4, got {value}");
    }
}

#[test]
fn a_surface_below_the_target_reads_negative() {
    let (positions, indices) = sheet();
    let map = map_of(&lifted(&positions, -0.25), &indices, &settings());
    assert!(map
        .signed_mm
        .iter()
        .all(|value| (*value + 0.25).abs() < 1e-4));
}

#[test]
fn inverted_orientation_flips_every_sign() {
    let (positions, indices) = sheet();
    let inverted = DeviationSettings {
        orientation: Orientation::Inverted,
        ..settings()
    };
    let map = map_of(&lifted(&positions, 0.4), &indices, &inverted);
    assert!(map
        .signed_mm
        .iter()
        .all(|value| (*value + 0.4).abs() < 1e-4));
}

#[test]
fn ignored_orientation_reports_magnitude_only() {
    let (positions, indices) = sheet();
    let ignored = DeviationSettings {
        orientation: Orientation::Ignored,
        ..settings()
    };
    let map = map_of(&lifted(&positions, -0.3), &indices, &ignored);
    assert!(map
        .signed_mm
        .iter()
        .all(|value| (*value - 0.3).abs() < 1e-4));
}

#[test]
fn vertices_beyond_the_radius_are_out_of_reach_and_grey() {
    let (positions, indices) = sheet();
    let map = map_of(&lifted(&positions, 50.0), &indices, &settings());
    assert!(map
        .validity
        .iter()
        .all(|state| *state == Validity::OutOfReach));

    let stats = deviation_stats(&map, 0.2);
    assert_eq!(stats.measured, 0);
    assert_eq!(stats.skipped as usize, map.validity.len());

    let colors = deviation_colors(&map, &RampSettings::default());
    assert!(colors.iter().all(|color| *color == NO_DATA_COLOR));
}

#[test]
fn statistics_count_only_measured_vertices() {
    let (positions, indices) = sheet();
    let mut moving = lifted(&positions, 0.1);
    moving[2] = 40.0;
    let map = map_of(&moving, &indices, &settings());

    let stats = deviation_stats(&map, 0.2);
    assert_eq!(stats.skipped, 1);
    assert_eq!(stats.measured, 3);
    assert!((stats.within_tolerance - 1.0).abs() < 1e-9);
    assert!((stats.mean_abs - 0.1).abs() < 1e-4);
}

#[test]
fn a_painted_out_vertex_is_excluded_from_the_map_and_the_numbers() {
    let (fixed_positions, fixed_indices) = sheet();
    let index = SurfaceIndex::build(Soup {
        positions: &fixed_positions,
        indices: &fixed_indices,
        mask: None,
    })
    .unwrap();
    let moving = lifted(&fixed_positions, 0.1);
    let mask = [0u8, 1, 0, 0];
    let map = deviation(
        Soup {
            positions: &moving,
            indices: &fixed_indices,
            mask: Some(&mask),
        },
        &index,
        Rigid::IDENTITY,
        &settings(),
        &CancelFlag::new(),
    );

    assert_eq!(map.validity[1], Validity::Excluded);
    assert_eq!(map.validity[0], Validity::Measured);

    let stats = deviation_stats(&map, 0.2);
    assert_eq!(stats.measured, 3, "a painted vertex must leave the numbers");
    let colors = deviation_colors(&map, &RampSettings::default());
    assert_eq!(colors[1], NO_DATA_COLOR);
}

#[test]
fn a_non_finite_vertex_is_named_as_such() {
    let (positions, indices) = sheet();
    let mut moving = lifted(&positions, 0.1);
    moving[0] = f32::NAN;
    let map = map_of(&moving, &indices, &settings());
    assert_eq!(map.validity[0], Validity::NonFinite);
}

#[test]
fn the_magnitude_ramp_is_cool_at_nothing_and_hot_at_the_scale() {
    let map = DeviationMap {
        signed_mm: vec![0.0, 0.5, -0.5],
        validity: vec![Validity::Measured; 3],
    };
    let colors = deviation_colors(
        &map,
        &RampSettings {
            mode: RampMode::Magnitude,
            ..RampSettings::default()
        },
    );
    assert!(colors[0][2] > colors[0][0], "no deviation must read cool");
    assert!(colors[1][0] > colors[1][2], "full scale must read hot");
    assert_eq!(
        colors[1], colors[2],
        "magnitude ignores which side the surface sits on"
    );
}

/// The two assertions the "always blue" report needed. A ramp that never
/// leaves its first stop looks exactly like a correct one at the origin,
/// so checking the ends is not enough: this walks the whole scale and
/// requires the hue to actually pass through cyan, green and yellow on its
/// way to red, and requires the two hot/cold channels to move
/// monotonically so no stop is skipped or visited twice.
#[test]
fn the_magnitude_ramp_walks_blue_cyan_green_yellow_red_across_the_scale() {
    let ramp = RampSettings {
        mode: RampMode::Magnitude,
        ..RampSettings::default()
    };
    let at = |position: f64| ramp_color(position * ramp.scale_mm, &ramp);

    // The channel bounds are loose because the stops are deliberately
    // pulled in from the pure extremes: a ramp built from 0 and 255 turns
    // to ink and scab the moment the shader multiplies it by any light.
    // What is being pinned here is the WALK, not a particular blue.
    let (mut cyan, mut green, mut yellow, mut red) = (false, false, false, false);
    let mut previous = at(0.0);
    assert!(
        previous[2] > 180 && previous[0] < 90,
        "nothing measured must read blue, got {previous:?}"
    );
    for step in 1..=100 {
        let color = at(f64::from(step) / 100.0);
        // Red only ever rises and blue only ever falls across a magnitude
        // ramp; a stop table walked in the wrong order would break this
        // long before the ends looked wrong.
        assert!(
            color[0] >= previous[0] && color[2] <= previous[2],
            "the ramp doubled back at step {step}: {previous:?} then {color:?}"
        );
        cyan |= color[0] < 90 && color[1] > 160 && color[2] > 160;
        green |= color[1] > 180 && color[0] < 120 && color[2] < 120;
        yellow |= color[0] > 200 && color[1] > 150 && color[2] < 80;
        red |= color[0] > 200 && color[1] < 90 && color[2] < 60;
        previous = color;
    }
    assert!(cyan, "the ramp never passed through cyan");
    assert!(green, "the ramp never passed through green");
    assert!(yellow, "the ramp never passed through yellow");
    assert!(red, "the ramp never reached red");
    let [hot_r, hot_g, hot_b] = MAGNITUDE_RAMP[MAGNITUDE_RAMP.len() - 1].1;
    assert_eq!(
        at(1.0),
        [hot_r, hot_g, hot_b, 255],
        "the display scale must land on the hot stop exactly"
    );
}

#[test]
fn the_signed_ramp_is_blue_below_and_red_above() {
    let map = DeviationMap {
        signed_mm: vec![-0.5, 0.0, 0.5],
        validity: vec![Validity::Measured; 3],
    };
    let colors = deviation_colors(&map, &RampSettings::default());
    assert!(colors[0][2] > colors[0][0], "the negative end must be blue");
    assert!(colors[2][0] > colors[2][2], "the positive end must be red");
    assert!(
        colors[1][1] > colors[1][0] && colors[1][1] > colors[1][2],
        "zero must be green"
    );
}

#[test]
fn banded_mode_quantizes_neighbouring_values_to_one_color() {
    // Outside the 0.20 mm nominal band on purpose: inside it every value is
    // one colour by design, which would make this test pass for the wrong
    // reason.
    let map = DeviationMap {
        signed_mm: vec![0.30, 0.315],
        validity: vec![Validity::Measured; 2],
    };
    let ramp = RampSettings {
        bands: Some(10),
        ..RampSettings::default()
    };
    let colors = deviation_colors(&map, &ramp);
    assert_eq!(colors[0], colors[1], "one band must be one colour");

    let continuous = deviation_colors(&map, &RampSettings::default());
    assert_ne!(
        continuous[0], continuous[1],
        "the continuous ramp must still separate them"
    );
}

/// The fix for the map an operator called a thermal camera.
///
/// Two arch scans that agree to within scan noise used to come out as
/// speckle, because every twenty-micron wobble got its own hue. Everything
/// inside the tolerance now lands on ONE nominal colour, so what is left
/// burning is what genuinely differs.
#[test]
fn everything_inside_the_tolerance_is_one_nominal_colour() {
    let ramp = RampSettings {
        scale_mm: 2.0,
        tolerance_mm: 0.2,
        bands: None,
        mode: RampMode::Signed,
    };
    let nominal = ramp_color(0.0, &ramp);
    for value in [-0.19, -0.1, -0.02, 0.0, 0.02, 0.1, 0.19] {
        assert_eq!(
            ramp_color(value, &ramp),
            nominal,
            "{value} mm is inside the tolerance and must read nominal"
        );
    }
    // And the ramp still has to move the moment it leaves the band, or the
    // band would just be a wider dead zone.
    assert_ne!(ramp_color(0.6, &ramp), nominal);
    assert_ne!(ramp_color(-0.6, &ramp), nominal);
    assert_ne!(ramp_color(0.6, &ramp), ramp_color(-0.6, &ramp));
}

/// The band is the operator's tolerance, and they can set one wider than
/// the range they are looking at. Left alone that paints the whole scan
/// nominal and answers every question with "fine".
#[test]
fn a_tolerance_wider_than_the_range_still_leaves_a_ramp() {
    let ramp = RampSettings {
        scale_mm: 0.2,
        tolerance_mm: 5.0,
        bands: None,
        mode: RampMode::Magnitude,
    };
    assert_ne!(
        ramp_color(0.2, &ramp),
        ramp_color(0.0, &ramp),
        "the display range must still separate its own ends"
    );
}

/// A rough two-point fit really does leave arches millimetres apart. A
/// suggestion capped at a clinical number pins every vertex to an end stop,
/// which is the saturated mosaic the operator reported.
#[test]
fn the_suggested_scale_follows_a_badly_aligned_pair_instead_of_capping() {
    let rough = DeviationStats {
        within_tolerance: 0.19,
        mean_abs: 1.1,
        rms: 1.378,
        median: 0.4,
        p95: 3.24,
        max_abs: 6.0,
        measured: 100_000,
        skipped: 10_855,
    };
    let scale = suggested_scale_mm(&rough);
    assert!(
        scale >= rough.p95,
        "a scale of {scale} mm saturates a p95 of {} mm",
        rough.p95
    );
}

#[test]
fn a_cancelled_run_marks_everything_unmeasured() {
    let (positions, indices) = sheet();
    let (fixed_positions, fixed_indices) = sheet();
    let index = SurfaceIndex::build(Soup {
        positions: &fixed_positions,
        indices: &fixed_indices,
        mask: None,
    })
    .unwrap();
    let cancel = CancelFlag::new();
    cancel.cancel();

    let map = deviation(
        Soup {
            positions: &positions,
            indices: &indices,
            mask: None,
        },
        &index,
        Rigid::IDENTITY,
        &settings(),
        &cancel,
    );

    assert!(map
        .validity
        .iter()
        .all(|state| *state != Validity::Measured));
}
