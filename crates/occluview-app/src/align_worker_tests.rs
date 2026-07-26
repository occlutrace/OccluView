//! Tests for [`super`]: the worker's colouring and its reuse contract.
//!
//! A `#[path]` child module of `align_worker.rs`, split out to hold the
//! workspace's 800-line file budget.
#![allow(clippy::expect_used, clippy::float_cmp, clippy::items_after_statements)]

use super::{
    color_map, AlignSettings, MeasureKey, SurfaceKey, CLINICAL_CEILING_MM, CLINICAL_MAX_MM,
    CLINICAL_MIN_MM,
};
use occluview_align::{
    deviation_colors, DeviationMap, Orientation, RampMode, RampSettings, Validity,
};

/// A map with one of everything: a hard negative, nominal, a hard positive,
/// something past the scale, and an entry that was never measured.
fn map() -> DeviationMap {
    DeviationMap {
        signed_mm: vec![-0.9, -0.31, 0.0, 0.17, 0.42, 3.0, 0.0],
        validity: vec![
            Validity::Measured,
            Validity::Measured,
            Validity::Measured,
            Validity::Measured,
            Validity::Measured,
            Validity::Measured,
            Validity::OutOfReach,
        ],
    }
}

/// The worker colours in parallel to keep a re-colour instant. It must be
/// the SAME ramp the rest of the tool reads — the legend calls
/// `ramp_color`, and a map painted a shade off from its own legend is a
/// measurement nobody can trust.
#[test]
fn colouring_in_parallel_matches_the_library() {
    let map = map();
    for mode in [RampMode::Signed, RampMode::Magnitude] {
        for bands in [None, Some(6)] {
            let ramp = RampSettings {
                scale_mm: 0.5,
                tolerance_mm: 0.2,
                bands,
                mode,
            };
            assert_eq!(
                color_map(&map, &ramp),
                deviation_colors(&map, &ramp),
                "parallel colouring diverged at {mode:?} / {bands:?}"
            );
        }
    }
}

/// Vertices with nothing to measure against must stay grey, whatever the
/// ramp says — colouring them would invent a measurement.
#[test]
fn unmeasured_vertices_stay_grey() {
    let colors = color_map(
        &map(),
        &RampSettings {
            scale_mm: 0.5,
            tolerance_mm: 0.2,
            bands: None,
            mode: RampMode::Signed,
        },
    );
    assert_eq!(colors[6], occluview_align::NO_DATA_COLOR);
    assert_ne!(colors[0], occluview_align::NO_DATA_COLOR);
}

fn key() -> MeasureKey {
    MeasureKey {
        moving: (1, 2),
        fixed: SurfaceKey {
            geometry: 3,
            pose: 4,
            markings: 0,
        },
        mask: 0,
        influence_radius_bits: 5.0_f64.to_bits(),
        orientation: Orientation::Match,
    }
}

/// The reuse rule, stated as a test: a change that only repaints an
/// existing measurement must key the same way, and a change that alters
/// what is measured must not.
#[test]
fn only_the_settings_that_change_the_distances_change_the_key() {
    let base = key();
    assert_eq!(base, key(), "an unchanged measurement keys the same way");

    let mut moved = key();
    moved.moving.1 = 99;
    assert_ne!(base, moved, "moving the scan changes what is measured");

    let mut reached = key();
    reached.influence_radius_bits = 2.0_f64.to_bits();
    assert_ne!(base, reached, "the reach changes what is measured");

    let mut masked = key();
    masked.mask = 1;
    assert_ne!(base, masked, "the mask changes what is measured");

    let mut facing = key();
    facing.orientation = Orientation::Inverted;
    assert_ne!(base, facing, "facing changes the sign of every distance");
}

/// The window opens on the clinical range, and it opens there every time.
///
/// Dentistry works to a fifth of a millimetre. A map whose ends are five
/// millimetres apart cannot show a fit that is either good or bad in that
/// regime, and a range that FOLLOWS the measurement walks straight out of it
/// the moment two meshes are roughly placed — which is how an operator ended up
/// reading an arch in red and blue mosaic and calling it a thermal camera.
///
/// The magnitude ramp is right here because of the nominal band: everything
/// inside tolerance lands on one flat cold colour, which is the correct reading
/// of "these agree", and what is left burning is what genuinely differs. That
/// was not true before the band existed, and this test used to pin the opposite.
#[test]
fn the_window_opens_on_the_clinical_range() {
    let settings = AlignSettings::default();
    assert!(
        (settings.scale_mm - CLINICAL_MAX_MM).abs() < f64::EPSILON,
        "the display maximum must open at the clinical one, got {}",
        settings.scale_mm
    );
    assert!(
        (settings.tolerance_mm - CLINICAL_MIN_MM).abs() < f64::EPSILON,
        "the nominal band must open at the clinical one, got {}",
        settings.tolerance_mm
    );
    assert!(
        !settings.auto_scale,
        "a range that follows the measurement leaves the clinical one behind"
    );
    assert_eq!(settings.ramp_mode, RampMode::Magnitude);
    assert!(
        settings.scale_mm <= CLINICAL_CEILING_MM,
        "the range must stay inside what a clinical instrument can mean"
    );
}

/// A 10 x 10 sheet on z = 0 with its outward normal along +Z: the surface a
/// measurement is taken against.
fn fixed_sheet() -> (Vec<f32>, Vec<u32>) {
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

/// A sheet tilted so it stands off the fixed one by 0 mm at one edge and
/// `offset_mm` at the other — a real registration error, not a constant.
fn tilted_sheet(offset_mm: f32) -> (Vec<f32>, Vec<u32>) {
    const STEPS: usize = 20;
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for row in 0..=STEPS {
        for column in 0..=STEPS {
            #[allow(clippy::cast_precision_loss)]
            let across = column as f32 / STEPS as f32;
            #[allow(clippy::cast_precision_loss)]
            let along = row as f32 / STEPS as f32;
            positions.extend_from_slice(&[across * 10.0, along * 10.0, across * offset_mm]);
        }
    }
    let width = u32::try_from(STEPS + 1).unwrap_or(1);
    for row in 0..u32::try_from(STEPS).unwrap_or(0) {
        for column in 0..u32::try_from(STEPS).unwrap_or(0) {
            let corner = row * width + column;
            indices.extend_from_slice(&[corner, corner + 1, corner + width]);
            indices.extend_from_slice(&[corner + 1, corner + width + 1, corner + width]);
        }
    }
    (positions, indices)
}

/// The end of the chain, on geometry with a deviation a lab would care about:
/// a 0.30 mm standoff that closes to nothing across the surface.
///
/// Three separate bugs have already reached the screen here — a mirrored
/// legend, a ramp default that painted every good result one flat blue, and an
/// unlit draw that flattened the form. This one covers the first two at once:
/// the map has to show a TRANSITION rather than one colour, and every colour on
/// the surface has to be a colour the legend also shows, at the same distance.
#[test]
fn a_real_third_of_a_millimetre_shows_a_transition_the_legend_agrees_with() {
    use crate::align_overlay::legend_value_mm;
    use occluview_align::{
        deviation, deviation_stats, suggested_scale_mm, CancelFlag, DeviationSettings, Soup,
        SurfaceIndex,
    };

    let (fixed_positions, fixed_indices) = fixed_sheet();
    let index = SurfaceIndex::build(Soup {
        positions: &fixed_positions,
        indices: &fixed_indices,
        mask: None,
    })
    .expect("a fixed surface");
    let (moving_positions, moving_indices) = tilted_sheet(0.30);
    let map = deviation(
        Soup {
            positions: &moving_positions,
            indices: &moving_indices,
            mask: None,
        },
        &index,
        occluview_align::Rigid::default(),
        &DeviationSettings {
            influence_radius_mm: 5.0,
            orientation: Orientation::Match,
        },
        &CancelFlag::new(),
    );

    let stats = deviation_stats(&map, 0.2);
    assert_eq!(stats.skipped, 0, "every vertex had surface within reach");
    assert!(
        (stats.p95 - 0.30).abs() < 0.02,
        "the geometry does not carry the offset it was built with: p95 {:.3}",
        stats.p95
    );

    // The range the tool picks for itself, which is what the operator sees.
    // The nominal band is set well inside the 0.30 mm standoff on purpose: a
    // band as wide as the deviation is a legitimate way to get two colours, and
    // this test is about the ramp BETWEEN them.
    let ramp = RampSettings {
        scale_mm: suggested_scale_mm(&stats),
        tolerance_mm: 0.05,
        bands: None,
        mode: RampMode::Signed,
    };
    let colors = color_map(&map, &ramp);

    // A transition, not one flat colour: the nominal end has to read green and
    // the far end hot, with the two clearly different.
    let nominal = colors[0];
    let farthest = colors
        .iter()
        .zip(&map.signed_mm)
        .max_by(|left, right| {
            left.1
                .partial_cmp(right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(color, _)| *color)
        .expect("a farthest vertex");
    assert!(
        nominal[1] > nominal[0] && nominal[1] > nominal[2],
        "a vertex sitting on the surface must read nominal green, got {nominal:?}"
    );
    assert!(
        farthest[0] > farthest[1] && farthest[0] > farthest[2],
        "a 0.30 mm standoff must read hot, got {farthest:?}"
    );
    let distinct: std::collections::BTreeSet<[u8; 4]> = colors.iter().copied().collect();
    assert!(
        distinct.len() > 8,
        "the map came out in {} colours — this is the flat-blue bug",
        distinct.len()
    );

    // The legend has to describe the surface. For every measured vertex, the
    // step of the legend bar nearest its distance must carry its colour.
    const STEPS: usize = 64;
    for (color, value) in colors.iter().zip(&map.signed_mm) {
        let (_, legend) = (0..STEPS)
            .map(|step| {
                let at = legend_value_mm(step, STEPS, ramp.mode, ramp.scale_mm);
                (
                    (at - f64::from(*value)).abs(),
                    occluview_align::ramp_color(at, &ramp),
                )
            })
            .min_by(|left, right| {
                left.0
                    .partial_cmp(&right.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("a legend step");
        for channel in 0..3 {
            let surface = i32::from(color[channel]);
            let bar = i32::from(legend[channel]);
            assert!(
                (surface - bar).abs() <= 24,
                "a vertex measured at {value:.3} mm is painted {color:?} but the legend \
                 shows {legend:?} at that distance"
            );
        }
    }
}
