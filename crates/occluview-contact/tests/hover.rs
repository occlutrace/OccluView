//! Reading a value off the surface: the number, its unit, and the corner blend.
//!
//! A colour map without a number is a picture, and the number is written the way
//! a technician writes it: micrometres below a millimetre, millimetres above it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use occluview_contact::{
    format_contact_value, interpolate_field_at_triangle, is_no_contact, ContactReading,
    ContactReadingKind, FIELD_FAR_SENTINEL_MM, NO_CONTACT_MM,
};

/// Micrometres below a millimetre, millimetres above it, always with its unit.
#[test]
fn a_readout_is_spelled_in_the_units_a_technician_writes() {
    assert_eq!(format_contact_value(0.082), "82 µm");
    assert_eq!(format_contact_value(0.0), "0 µm");
    assert_eq!(format_contact_value(0.999), "999 µm");
    assert_eq!(format_contact_value(1.25), "1.25 mm");
    assert_eq!(format_contact_value(2.0), "2.00 mm");
    assert_eq!(
        format_contact_value(f64::NAN),
        "-- µm",
        "an unmeasured value says so rather than printing a number it does not have"
    );
}

/// A reading knows which side of touch it is on, and an unmeasured vertex has no
/// reading at all.
#[test]
fn a_reading_carries_its_side_of_touch() {
    let penetration = ContactReading::from_signed_mm(-0.082).expect("a measured vertex");
    assert_eq!(penetration.kind, ContactReadingKind::Penetration);
    assert_eq!(format_contact_value(penetration.magnitude_mm), "82 µm");

    let gap = ContactReading::from_signed_mm(0.15).expect("a measured vertex");
    assert_eq!(gap.kind, ContactReadingKind::Gap);
    assert_eq!(format_contact_value(gap.magnitude_mm), "150 µm");

    // Below the sign dead-band a reading is touch, not a sub-micron
    // interference: the same decision the field makes, so the number and the
    // pixels agree about where contact begins.
    let touching = ContactReading::from_signed_mm(-0.000_5).expect("a measured vertex");
    assert_eq!(touching.kind, ContactReadingKind::Gap);
    assert!(
        (touching.magnitude_mm - 0.000_5).abs() < 1e-9,
        "a sub-micron interference reads as a half-micrometre gap, not as a depth"
    );

    assert!(ContactReading::from_signed_mm(NO_CONTACT_MM).is_none());
    assert!(ContactReading::from_signed_mm(f32::NAN).is_none());
    assert!(is_no_contact(NO_CONTACT_MM));
    assert!(is_no_contact(f32::NAN));
    assert!(!is_no_contact(0.0), "exact touch is a measurement");
}

/// The blend under the pointer is the blend the GPU paints, with a missing
/// corner entering as the finite far sentinel rather than poisoning the
/// triangle.
#[test]
fn a_triangle_blends_its_corners_the_way_the_shader_does() {
    let values = [-0.2_f32, -0.1, 0.0];
    let indices = [0_u32, 1, 2];
    let middle = interpolate_field_at_triangle(&values, &indices, 0, [1.0 / 3.0; 3])
        .expect("a corner is measured");
    assert!(
        (f64::from(middle) + 0.1).abs() < 1e-6,
        "the centre of the triangle is the mean of its corners: {middle}"
    );

    let corner = interpolate_field_at_triangle(&values, &indices, 0, [1.0, 0.0, 0.0])
        .expect("a corner is measured");
    assert_eq!(corner, -0.2);

    // One unmeasured corner: the blend stays finite and lands between the
    // measured value and the far sentinel, which is what the paint shows too.
    let mixed_values = [NO_CONTACT_MM, -0.1, 0.0];
    let blended = interpolate_field_at_triangle(&mixed_values, &indices, 0, [0.5, 0.5, 0.0])
        .expect("two corners are measured");
    let expected = (FIELD_FAR_SENTINEL_MM - 0.1) / 2.0;
    assert!(
        (blended - expected).abs() < 1e-6,
        "an unmeasured corner enters as the finite sentinel: {blended} against {expected}"
    );
    assert!(blended.is_finite(), "a NaN here would poison the readout");
}

/// A triangle whose corners were all unmeasured has no reading, not a sentinel
/// reported as a distance — while an out-of-range request is a caller bug that
/// returns nothing rather than panicking.
#[test]
fn a_triangle_with_nothing_measured_reports_nothing() {
    let values = [NO_CONTACT_MM, NO_CONTACT_MM, NO_CONTACT_MM];
    let indices = [0_u32, 1, 2];
    assert_eq!(
        interpolate_field_at_triangle(&values, &indices, 0, [1.0 / 3.0; 3]),
        None
    );
    // An out-of-range triangle or vertex is a caller bug, not a panic.
    assert_eq!(
        interpolate_field_at_triangle(&values, &indices, 7, [1.0; 3]),
        None
    );
    assert_eq!(
        interpolate_field_at_triangle(&values, &[9, 10, 11], 0, [1.0; 3]),
        None
    );
    assert_eq!(
        interpolate_field_at_triangle(&values, &indices, 0, [0.0, 0.0, 0.0]),
        None
    );
}
