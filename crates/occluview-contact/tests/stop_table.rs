//! The compiled stop table: the shape the shader reads, and the slider.
//!
//! `occluview-contact` compiles a law's stops into a fixed-size payload for the
//! GPU and evaluates the same payload on the CPU for the panel, the hover readout
//! and the legend. That the two evaluations agree is asserted in
//! `occluview-render`'s `contact_paint` test, which uploads this table, paints it
//! through the real shader, and reads the pixels back; a copy of the shader's
//! arithmetic written in Rust could only agree with itself.
//!
//! What is left here needs no GPU: the payload's shape and the slider's reach.

#![allow(clippy::unwrap_used, clippy::float_cmp)]

use occluview_contact::{ContactScale, LOAD_MAX_MM, MAX_CONTACT_STOPS, TIGHTNESS};

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
    // The ramp restates the stops' ends and the law's gate in the 32-bit payload
    // the shader reads. A payload that disagrees with itself here would misstate
    // the surface to whoever reads it next.
    let far = table.stops[0][0];
    let deepest = table.stops[table.count as usize - 1][0];
    assert!(
        (f64::from(table.ramp[0]) - f64::from(far)).abs() < 1e-5,
        "ramp far end: {} against {far}",
        table.ramp[0]
    );
    assert!(
        (f64::from(table.ramp[1]) - f64::from(far - deepest)).abs() < 1e-5,
        "ramp span: {} against {}",
        table.ramp[1],
        far - deepest
    );
    assert!((f64::from(table.ramp[2]) - TIGHTNESS.paint_far_mm).abs() < 1e-6);
    assert!((f64::from(table.ramp[3]) - TIGHTNESS.far_fade_mm).abs() < 1e-6);
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
