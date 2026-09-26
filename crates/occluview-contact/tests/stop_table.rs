//! The compiled stop table, against the shader that will consume it.
//!
//! The renderer re-runs the ramp in WGSL — it interpolates the stop table in
//! Oklab and converts to display sRGB — while the panel, the hover readout and
//! the legend read [`ContactScale::color_at`] on the CPU. Nothing in the type
//! system connects the two, so this file re-runs the shader's algorithm in Rust
//! over the table and asserts it lands on the CPU's colour.
//!
//! The table is a compiled artifact so the two evaluations share their numbers
//! by construction; this test fails when either evaluation changes on its own.
//! The renderer's offscreen test checks the WGSL against the same expectation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::many_single_char_names,
    clippy::cast_sign_loss
)]

use occluview_contact::{
    ContactScale, StopTable, CLINICAL, LOAD_MAX_MM, MAX_CONTACT_STOPS, TIGHTNESS,
};

/// A sweep of field values, at every law's every setting, paints the CPU colour
/// the shader would compute from the same table.
#[test]
fn the_stop_table_reproduces_the_cpu_colour() {
    for law in [&TIGHTNESS, &CLINICAL] {
        for load in [occluview_contact::LOAD_MIN_MM, law.load_mm, LOAD_MAX_MM] {
            let scale = ContactScale::new(law, load);
            let table = scale.stop_table();
            assert_eq!(
                table.count as usize,
                law.stops.len(),
                "{} lost stops on the way to the GPU",
                law.id
            );
            assert!(
                table.count as usize <= MAX_CONTACT_STOPS,
                "the table is a fixed-size payload"
            );

            let far = f64::from(table.stops[0][0]);
            let deepest = f64::from(table.stops[table.count as usize - 1][0]);
            // The table is a 32-bit GPU payload, so it is compared at 32-bit
            // precision: the shader reads these as floats.
            assert!(
                (f64::from(table.ramp[0]) - far).abs() < 1e-5,
                "ramp far end: {} against {far}",
                table.ramp[0]
            );
            assert!(
                (f64::from(table.ramp[1]) - (far - deepest)).abs() < 1e-5,
                "ramp span: {} against {}",
                table.ramp[1],
                far - deepest
            );
            assert!((f64::from(table.ramp[2]) - law.paint_far_mm).abs() < 1e-6);
            assert!((f64::from(table.ramp[3]) - law.far_fade_mm).abs() < 1e-6);

            let steps = 512;
            for step in 0..=steps {
                let value = far - (f64::from(step) / f64::from(steps)) * (far - deepest);
                // Stop exactly at the far gate: the shader's weight is zero
                // there and the CPU paints nothing, so there is no colour to
                // compare. Just inside it, both must agree.
                if value >= law.paint_far_mm {
                    continue;
                }
                let shader = shader_color(&table, value);
                let cpu = scale.color_at(value);
                for channel in 0..3 {
                    let difference = i32::from(shader[channel]) - i32::from(cpu[channel]);
                    assert!(
                        difference.abs() <= 1,
                        "{} at {load} mm load: {value} mm paints {shader:?} in the shader and \
                         {cpu:?} on the CPU ({difference} on channel {channel})",
                        law.id
                    );
                }
            }
        }
    }
}

/// The GPU payload is the shader's shape, not a copy of the CPU evaluator.
#[test]
fn the_table_is_the_shape_the_shader_reads() {
    let scale = ContactScale::new(&TIGHTNESS, TIGHTNESS.load_mm);
    let table = scale.stop_table();
    assert_eq!(table.stops.len(), MAX_CONTACT_STOPS);
    assert_eq!(table.count, u32::try_from(TIGHTNESS.stops.len()).unwrap());
    for slot in table.count as usize..MAX_CONTACT_STOPS {
        assert_eq!(
            table.stops[slot], [0.0; 4],
            "unused slots are zeroed so a stale stop cannot be read"
        );
    }
    // Descending in millimetres, which is the order the shader's span search
    // relies on.
    for pair in table.stops[..table.count as usize].windows(2) {
        assert!(
            pair[0][0] > pair[1][0],
            "stops must descend: {} then {}",
            pair[0][0],
            pair[1][0]
        );
    }
    // The gap slot is the caller's: this crate cannot know the field texture's
    // row length, and a wrong length is a wrong vertex, so it is left empty
    // rather than guessed.
    assert_eq!(table.gap, [0.0, 0.0]);
}

/// The slider reaches the GPU: penetration stops move, gap stops do not.
#[test]
fn only_penetration_stops_move_with_the_slider() {
    let base = ContactScale::new(&TIGHTNESS, TIGHTNESS.load_mm).stop_table();
    let wide = ContactScale::new(&TIGHTNESS, LOAD_MAX_MM).stop_table();
    for index in 0..base.count as usize {
        let (base_mm, wide_mm) = (base.stops[index][0], wide.stops[index][0]);
        if TIGHTNESS.stops[index].0 >= 0.0 {
            assert_eq!(base_mm, wide_mm, "gap stop {index} moved");
            assert_eq!(
                base.stops[index][1..],
                wide.stops[index][1..],
                "gap stop {index} changed colour"
            );
        } else {
            assert!(
                wide_mm < base_mm,
                "penetration stop {index} did not scale: {wide_mm} against {base_mm}"
            );
            assert_eq!(
                base.stops[index][1..],
                wide.stops[index][1..],
                "the Oklab colour of a stop is the law's, not the slider's"
            );
        }
    }
}

/// The shader's algorithm, in Rust: clamp to the table's ends, find the
/// containing span, mix the Oklab row, convert to display sRGB.
fn shader_color(table: &StopTable, signed_mm: f64) -> [u8; 3] {
    let count = table.count as usize;
    let value = signed_mm.clamp(
        f64::from(table.stops[count - 1][0]),
        f64::from(table.stops[0][0]),
    );
    let mut colour = [
        table.stops[count - 1][1],
        table.stops[count - 1][2],
        table.stops[count - 1][3],
    ];
    for index in 0..count - 1 {
        let high = table.stops[index];
        let low = table.stops[index + 1];
        if value > f64::from(high[0]) || value < f64::from(low[0]) {
            continue;
        }
        let span = f64::from(high[0]) - f64::from(low[0]);
        let t = if span <= 0.0 {
            0.0
        } else {
            (f64::from(high[0]) - value) / span
        };
        for channel in 0..3 {
            colour[channel] = high[channel + 1] + (low[channel + 1] - high[channel + 1]) * t as f32;
        }
        break;
    }
    oklab_to_srgb(colour)
}

/// Oklab to display sRGB, the matrices the WGSL will carry.
fn oklab_to_srgb(oklab: [f32; 3]) -> [u8; 3] {
    let l = f64::from(oklab[0]);
    let a = f64::from(oklab[1]);
    let b = f64::from(oklab[2]);
    let long = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let medium = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let short = l - 0.089_484_177_5 * a - 1.291_485_548 * b;
    let (long, medium, short) = (long.powi(3), medium.powi(3), short.powi(3));
    let linear = [
        (4.076_741_662_1 * long - 3.307_711_591_3 * medium + 0.230_969_929_2 * short)
            .clamp(0.0, 1.0),
        (-1.268_438_004_6 * long + 2.609_757_401_1 * medium - 0.341_319_396_5 * short)
            .clamp(0.0, 1.0),
        (-0.004_196_086_3 * long - 0.703_418_614_7 * medium + 1.707_614_701 * short)
            .clamp(0.0, 1.0),
    ];
    linear.map(|value| {
        let srgb = if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (srgb.clamp(0.0, 1.0) * 255.0).round() as u8
    })
}
