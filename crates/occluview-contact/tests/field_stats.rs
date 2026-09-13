//! What a reading counts: patches, contacts and area.
//!
//! These are the numbers the panel prints, so each one is asserted against a
//! geometry whose answer is known by construction rather than against whatever
//! the code happened to produce.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

mod support;

use occluview_contact::ContactSettings;
use support::{field, field_with, plate, quad, Mesh};

/// One connected contact reads as one contact, and its area is the area of the
/// surface that is touching.
#[test]
fn a_single_contact_reads_its_own_area_and_one_mark() {
    let subject = plate(5.0, 5.0, 10, 10, 0.0, false);
    // The plate covers the whole subject, ten micrometres below it: inside the
    // touch tolerance everywhere, so the whole subject surface is in contact.
    let antagonist = plate(6.0, 6.0, 12, 12, -0.01, false);
    let measured = field(&subject, &antagonist);

    assert_eq!(measured.stats.contacts, 1, "one patch, one contact");
    assert!(
        (measured.stats.contact_area_mm2 - 100.0).abs() < 1.0,
        "a 10 by 10 mm plate reads {} mm² in contact",
        measured.stats.contact_area_mm2
    );
    let split = measured.stats.minus_x_area_mm2 + measured.stats.plus_x_area_mm2;
    assert!(
        (split - measured.stats.contact_area_mm2).abs() < 1e-6,
        "the left and right halves must add up to the whole"
    );
    assert_eq!(
        measured.stats.deepest_mm, 0.0,
        "a gap that never closes has no depth to report"
    );
}

/// Two contacts four millimetres apart are two contacts, which is what makes
/// the count worth printing: a bite loading on two teeth is not one mark.
#[test]
fn two_contacts_four_millimetres_apart_stay_two() {
    // Half-millimetre cells, so the counted mark is the mark: a triangle that
    // straddles the edge of a contact still carries part of its area, which
    // spreads the cluster points a fraction of a cell past the boundary.
    let subject = plate(5.0, 5.0, 20, 20, 0.0, false);
    let left = quad(-5.0, -2.0, -5.0, 5.0, -0.01, false);
    let right = quad(2.0, 5.0, -5.0, 5.0, -0.01, false);
    let two = field(&subject, &support::combine(&[left.clone(), right]));
    let one = field(&subject, &left);

    assert_eq!(two.stats.contacts, 2, "four millimetres is two marks");
    assert_eq!(one.stats.contacts, 1, "one plate is one mark");
    assert!(
        two.stats.minus_x_area_mm2 > 0.0 && two.stats.plus_x_area_mm2 > 0.0,
        "the marks sit on both sides of the surface's own centre"
    );
    assert!(
        (two.stats.minus_x_area_mm2 - two.stats.plus_x_area_mm2).abs() < 1e-6,
        "the two marks are the same size and the split is at the surface's centre"
    );
    // A 3 by 10 mm mark is 30 mm²; the corner weighting adds half a cell of
    // credit along the open edge, which is the known artefact of counting whole
    // triangles rather than clipping them.
    assert!(
        (one.stats.contact_area_mm2 - 32.5).abs() < 0.5,
        "one 3 by 10 mm mark reads {} mm²",
        one.stats.contact_area_mm2
    );
    assert!(
        (two.stats.contact_area_mm2 - 65.0).abs() < 1.0,
        "two identical marks read {} mm²",
        two.stats.contact_area_mm2
    );
}

/// Flattening a patch collapses it to its peak depth, and leaves a lone vertex
/// alone.
#[test]
fn flattening_collapses_a_patch_to_its_peak() {
    // A subject grid with half-millimetre spacing, so adjacent vertices chain
    // under the 0.75 mm flatten radius and the whole surface is one patch.
    let subject = plate(1.0, 1.0, 4, 4, 0.0, false);
    // A stepped antagonist: fifty micrometres deep on one side, a hundred on
    // the other. One patch, two depths.
    let stepped = support::combine(&[
        quad(-1.0, 0.0, -1.0, 1.0, 0.05, false),
        quad(0.0, 1.0, -1.0, 1.0, 0.10, false),
    ]);

    let plain = field(&subject, &stepped);
    let depths = distinct(&plain.subject_signed_mm);
    assert_eq!(depths.len(), 2, "two depths before flattening: {depths:?}");
    assert!((depths[0] + 0.05).abs() < 1e-4 || (depths[0] + 0.1).abs() < 1e-4);
    assert!((depths[1] + 0.05).abs() < 1e-4 || (depths[1] + 0.1).abs() < 1e-4);

    let flattened = field_with(
        &subject,
        &stepped,
        ContactSettings {
            flatten_patches: true,
            ..ContactSettings::default()
        },
    );
    let peaks = distinct(&flattened.subject_signed_mm);
    assert_eq!(
        peaks.len(),
        1,
        "one patch must read one depth after flattening: {peaks:?}"
    );
    assert!(
        (peaks[0] + 0.1).abs() < 1e-4,
        "the patch collapsed to its deepest vertex, not to its shallowest: {}",
        peaks[0]
    );
    assert_eq!(
        plain.stats.deepest_mm, flattened.stats.deepest_mm,
        "the deepest penetration is the same number either way"
    );
}

/// A single penetrating vertex is not a patch, so flattening is a no-op.
#[test]
fn flattening_a_lone_vertex_changes_nothing() {
    // Three vertices five millimetres apart over one small, deep plate: only
    // the middle one is above it, so the patch has exactly one member.
    let subject = Mesh::new(
        &[[-5.0, 0.0, 0.0], [0.0, 0.0, 0.0], [5.0, 0.0, 0.0]],
        &[0, 1, 2],
    );
    let spike = quad(-0.2, 0.2, -0.2, 0.2, 0.05, false);

    let plain = field(&subject, &spike);
    let flattened = field_with(
        &subject,
        &spike,
        ContactSettings {
            flatten_patches: true,
            ..ContactSettings::default()
        },
    );
    assert_eq!(
        bits(&plain.subject_signed_mm),
        bits(&flattened.subject_signed_mm)
    );
    let measured: Vec<f32> = plain
        .subject_signed_mm
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    assert_eq!(measured.len(), 1, "one vertex was over the plate");
    assert!(
        (f64::from(measured[0]) + 0.05).abs() < 1e-4,
        "the lone penetrating vertex keeps its own depth"
    );
}

/// The distinct finite values in a field, sorted.
fn distinct(values: &[f32]) -> Vec<f64> {
    let mut seen: Vec<f64> = values
        .iter()
        .filter(|value| value.is_finite())
        .map(|value| f64::from(*value))
        .collect();
    seen.sort_by(|left, right| left.partial_cmp(right).unwrap());
    seen.dedup_by(|left, right| (*left - *right).abs() < 1e-6);
    seen
}

/// The field vectors, as their exact bit patterns.
fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|value| value.to_bits()).collect()
}
