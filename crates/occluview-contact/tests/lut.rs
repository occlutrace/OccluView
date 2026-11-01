//! Packing the field for the GPU: the bits have to come back out unchanged.
//!
//! The renderer's vertex stage decodes each texel with shifts and a `bitcast`,
//! so anything this crate does to a value on the way in is a reading the shader
//! will paint. That makes the round trip an acceptance test rather than a
//! formality.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::assertions_on_constants
)]

use occluview_contact::{pack_field_texels, FIELD_FAR_SENTINEL_MM, NO_CONTACT_MM};

/// A spread of real readings round-trips bit for bit, and a non-finite one
/// becomes the finite far sentinel.
#[test]
fn every_measured_value_survives_the_round_trip() {
    let values = [
        0.0_f32,
        -0.25,
        0.1,
        -0.6,
        FIELD_FAR_SENTINEL_MM,
        -0.001,
        1.5,
        NO_CONTACT_MM,
        f32::NAN,
        f32::NEG_INFINITY,
        -0.000_5,
    ];
    let packed = pack_field_texels(&values, 4);
    assert_eq!(packed.width, 4);
    assert_eq!(packed.height, 3);
    assert_eq!(packed.rgba.len(), 4 * 4 * 3);
    assert!(
        usize::try_from(packed.width * packed.height).unwrap() >= values.len(),
        "the texture has to hold every value"
    );

    for (texel, expected) in values.iter().enumerate() {
        let decoded = decode(&packed.rgba, texel);
        let want = if expected.is_finite() {
            *expected
        } else {
            FIELD_FAR_SENTINEL_MM
        };
        assert_eq!(
            decoded.to_bits(),
            want.to_bits(),
            "texel {texel} decoded as {decoded}, not {want}"
        );
    }

    // The last row is padding, and a padding texel has to be harmless: a huge
    // penetration depth there would paint a stray mark if the vertex stage ever
    // read past the end.
    for texel in values.len()..usize::try_from(packed.width * packed.height).unwrap() {
        assert_eq!(decode(&packed.rgba, texel), FIELD_FAR_SENTINEL_MM);
    }
}

/// The row length is the caller's device limit, narrowed to the data.
#[test]
fn the_row_length_never_widens_past_the_value_count() {
    let values = vec![0.0_f32; 6];
    let packed = pack_field_texels(&values, 4096);
    assert_eq!(packed.width, 6, "six values do not need a 4096-wide row");
    assert_eq!(packed.height, 1);

    let narrow = pack_field_texels(&values, 2);
    assert_eq!(narrow.width, 2);
    assert_eq!(narrow.height, 3);
    assert_eq!(narrow.rgba.len(), 2 * 3 * 4);

    let one = pack_field_texels(&values, 0);
    assert_eq!(one.width, 1, "a zero row length is still a valid texture");
    assert_eq!(one.height, 6);
}

/// An empty field is still a texture the shader can bind.
#[test]
fn an_empty_field_packs_one_sentinel_texel() {
    let packed = pack_field_texels(&[], 4096);
    assert_eq!(packed.width, 1);
    assert_eq!(packed.height, 1);
    assert_eq!(packed.rgba.len(), 4);
    assert_eq!(decode(&packed.rgba, 0), FIELD_FAR_SENTINEL_MM);
}

/// The sentinel is finite, past every law's painted range, and inside the
/// search radius — the three properties the packing depends on.
#[test]
fn the_sentinel_is_a_value_no_law_paints_and_no_probe_stops_at() {
    assert!(FIELD_FAR_SENTINEL_MM.is_finite());
    for law in [&occluview_contact::TIGHTNESS, &occluview_contact::CLINICAL] {
        let scale = occluview_contact::ContactScale::new(law, law.load_mm);
        assert_eq!(
            scale.color_at(f64::from(FIELD_FAR_SENTINEL_MM)),
            [0, 0, 0, 0],
            "{} must leave the sentinel unpainted",
            law.id
        );
    }
    assert!(FIELD_FAR_SENTINEL_MM > 0.0);
}

/// The little-endian byte order the shader's shift-and-or decode assumes.
#[test]
fn a_texel_is_the_little_endian_f32_bit_pattern() {
    let packed = pack_field_texels(&[-0.25_f32], 1);
    assert_eq!(packed.rgba.len(), 4);
    assert_eq!(packed.rgba, (-0.25_f32).to_le_bytes().to_vec());
}

/// One texel, decoded the way the shader decodes it.
fn decode(rgba: &[u8], texel: usize) -> f32 {
    let offset = texel * 4;
    let bytes = [
        rgba[offset],
        rgba[offset + 1],
        rgba[offset + 2],
        rgba[offset + 3],
    ];
    f32::from_le_bytes(bytes)
}
