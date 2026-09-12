//! The measurement itself: sign, reach, determinism, cancellation.
//!
//! Every assertion here is a property the reading depends on, not an
//! implementation detail: a gap is positive, an interference is negative, a
//! vertex with nothing opposite it says so, and two runs of the same input
//! agree bit for bit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::cast_precision_loss
)]

mod support;

use occluview_contact::{
    compute_contact_field, ContactSettings, NO_CONTACT_MM, SEARCH_RADIUS_MM, TIGHTNESS,
};
use support::{field, field_cancelled, field_with, plate, quad};

/// Two parallel plates far enough apart that every vertex has an answer, and
/// close enough that it is the same answer everywhere.
#[test]
fn a_gap_reads_positive_and_an_interference_reads_negative() {
    let subject = plate(2.0, 2.0, 4, 4, 0.0, false);

    // The antagonist sits below with its surface facing up, so the subject is
    // in front of it: a gap.
    let below = plate(5.0, 5.0, 10, 10, -0.1, false);
    let gap = field(&subject, &below);
    assert_eq!(
        gap.diagnostics.subject_measured,
        u32::try_from(subject.vertex_count()).unwrap(),
        "every vertex has an opposing surface within a tenth of a millimetre"
    );
    for (vertex, value) in gap.subject_signed_mm.iter().enumerate() {
        assert!(
            (f64::from(*value) - 0.1).abs() < 1e-4,
            "vertex {vertex} reads {value}, not a 0.1 mm gap"
        );
        assert!(*value > 0.0, "a gap is positive");
    }

    // Driven to overlap by the same tenth, the sign flips and the magnitude
    // stays the depth.
    let above = plate(5.0, 5.0, 10, 10, 0.1, false);
    let interference = field(&subject, &above);
    for (vertex, value) in interference.subject_signed_mm.iter().enumerate() {
        assert!(
            (f64::from(*value) + 0.1).abs() < 1e-4,
            "vertex {vertex} reads {value}, not a 0.1 mm interference"
        );
    }
    assert_eq!(
        interference.diagnostics.subject_penetrating,
        u32::try_from(subject.vertex_count()).unwrap(),
        "an interference is counted as penetration"
    );
}

/// The sign follows the OPPOSING surface's winding. An antagonist whose normals
/// point the other way turns the same geometry into the opposite reading, which
/// is a property of the measurement rather than a defect — and the reason the
/// layer menu's "Flip normals" changes what a contact reading says.
#[test]
fn the_sign_follows_the_antagonists_winding() {
    let subject = plate(2.0, 2.0, 4, 4, 0.0, false);
    let above = plate(5.0, 5.0, 10, 10, 0.1, false);
    let above_flipped = plate(5.0, 5.0, 10, 10, 0.1, true);

    let interference = field(&subject, &above);
    let gap = field(&subject, &above_flipped);
    for (vertex, value) in gap.subject_signed_mm.iter().enumerate() {
        assert!(
            (f64::from(*value) - 0.1).abs() < 1e-4,
            "a reversed antagonist normal turns the same overlap into a gap at vertex {vertex}: \
             {value} against {}",
            interference.subject_signed_mm[vertex]
        );
    }
}

/// Nothing opposite means nothing measured — and the value is the sentinel, not
/// zero, which is a real reading.
#[test]
fn a_vertex_with_no_opposing_surface_is_not_measured() {
    let subject = plate(2.0, 2.0, 4, 4, 0.0, false);
    let far = plate(5.0, 5.0, 10, 10, -5.0, false);
    let measured = field(&subject, &far);

    for value in &measured.subject_signed_mm {
        assert_eq!(*value, NO_CONTACT_MM, "five millimetres is out of reach");
    }
    assert_eq!(measured.diagnostics.subject_measured, 0);
    assert_eq!(measured.stats.contact_area_mm2, 0.0);
    assert_eq!(measured.stats.contacts, 0);
}

/// The donut hole: a strong interference must not paint as bare tooth in the
/// middle of its own mark.
///
/// A vertex deeper inside the antagonist than the search reach finds no surface
/// and falls back to the sentinel, which paints as clean tooth. The reach is
/// twice the deepest saturation depth for exactly that reason, so every
/// interference a bite pose can produce is measured — and only overclosure that
/// is already a garbage pose is not.
#[test]
fn deep_penetration_is_measured_instead_of_vanishing() {
    let subject = plate(2.0, 2.0, 4, 4, 0.0, false);
    // A small plate that only the centre vertex reaches, at a depth well past
    // every law's saturation point.
    let spike = quad(-0.2, 0.2, -0.2, 0.2, 0.45, false);
    let measured = field(&subject, &spike);

    let centre = measured
        .subject_signed_mm
        .iter()
        .enumerate()
        .find(|(vertex, _)| subject.vertex(*vertex)[0] == 0.0 && subject.vertex(*vertex)[1] == 0.0)
        .map(|(_, value)| *value)
        .expect("the grid holds a centre vertex");
    assert!(
        (f64::from(centre) + 0.45).abs() < 1e-3,
        "a 0.45 mm interference reads {centre}, so the mark has a hole in it"
    );
    assert!(
        measured.diagnostics.subject_penetrating >= 1,
        "the interference is counted"
    );

    let scale = occluview_contact::ContactScale::new(&TIGHTNESS, TIGHTNESS.load_mm);
    assert!(
        scale.is_painted(f64::from(centre)),
        "a saturated interference is inside the painted range"
    );
    assert_eq!(
        scale.color_at(f64::from(centre))[3],
        255,
        "and it is fully opaque, not a faint rim"
    );

    // Past the reach there is genuinely nothing to report, and the reading says
    // so rather than inventing a depth.
    let beyond = quad(-0.2, 0.2, -0.2, 0.2, 0.7, false);
    let unmeasured = field(&subject, &beyond);
    assert!(
        unmeasured
            .subject_signed_mm
            .iter()
            .all(|value| *value == NO_CONTACT_MM),
        "beyond {SEARCH_RADIUS_MM} mm the probe finds nothing, which is the sentinel's job"
    );
}

/// The same input twice is the same bytes, whatever the thread count.
///
/// A measurement that drifted with scheduling would drift between a print and a
/// screenshot of the same case, and an operator comparing them would have no way
/// to know which one moved.
#[test]
fn the_measurement_is_bit_identical_across_runs_and_thread_counts() {
    let subject = plate(2.0, 2.0, 6, 6, 0.0, false);
    let mut antagonist = plate(5.0, 5.0, 12, 12, 0.0, false);
    // A tilted surface, so the field is a spread of values rather than one
    // number repeated: a constant field would agree bit for bit by accident.
    for vertex in antagonist.positions.as_chunks_mut::<3>().0 {
        vertex[2] = -0.1 - 0.02 * vertex[0];
    }

    let first = field(&subject, &antagonist);
    let second = field(&subject, &antagonist);
    assert_eq!(
        bits(&first.subject_signed_mm),
        bits(&second.subject_signed_mm),
        "two runs disagree"
    );
    assert_eq!(
        bits(&first.antagonist_signed_mm),
        bits(&second.antagonist_signed_mm)
    );

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .expect("a one-thread pool");
    let single = pool.install(|| {
        compute_contact_field(
            subject.soup(),
            antagonist.soup(),
            ContactSettings::default(),
            &occluview_align::CancelFlag::new(),
        )
    });
    assert_eq!(
        bits(&first.subject_signed_mm),
        bits(&single.subject_signed_mm),
        "one thread produced a different field from many"
    );
    assert_eq!(
        bits(&first.antagonist_signed_mm),
        bits(&single.antagonist_signed_mm)
    );
}

/// A cancelled measurement returns what it had, and does not come apart.
#[test]
fn a_cancelled_measurement_returns_immediately() {
    let subject = plate(2.0, 2.0, 4, 4, 0.0, false);
    let antagonist = plate(5.0, 5.0, 10, 10, -0.1, false);
    let cancelled = field_cancelled(&subject, &antagonist);

    assert_eq!(cancelled.subject_signed_mm.len(), subject.vertex_count());
    assert!(
        cancelled
            .subject_signed_mm
            .iter()
            .all(|value| *value == NO_CONTACT_MM),
        "a job cancelled before it started measured nothing"
    );
    assert_eq!(cancelled.stats, occluview_contact::ContactStats::default());
    assert!(cancelled.diagnostics.worker_ms >= 0.0);
}

/// Flattening is a request, not a default, and it changes only penetration.
#[test]
fn the_flatten_toggle_does_not_move_a_measured_gap() {
    let subject = plate(2.0, 2.0, 4, 4, 0.0, false);
    let antagonist = plate(5.0, 5.0, 10, 10, -0.1, false);
    let plain = field_with(&subject, &antagonist, ContactSettings::default());
    let flattened = field_with(
        &subject,
        &antagonist,
        ContactSettings {
            flatten_patches: true,
            ..ContactSettings::default()
        },
    );
    assert_eq!(
        bits(&plain.subject_signed_mm),
        bits(&flattened.subject_signed_mm),
        "a gap is not a patch to collapse"
    );
}

/// The field vectors, as their exact bit patterns.
fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|value| value.to_bits()).collect()
}
