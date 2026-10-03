//! Release measurements for equivalent bounded ordinary-work admission.
#![cfg(feature = "search-probe")]
#![allow(clippy::unwrap_used, clippy::print_stdout)]

use occluview_geometry::surface::GeometryControl;
use std::{hint::black_box, time::Instant};

#[test]
fn fixed_linear_work_has_identical_accounting() {
    const COUNT: u64 = 4_194_304;
    let scalar = GeometryControl::unlimited();
    let started = Instant::now();
    let mut scalar_sum = 0u64;
    for i in 0..COUNT {
        scalar.charge_operations(1).unwrap();
        scalar_sum = scalar_sum.wrapping_add(black_box(i));
    }
    let scalar_seconds = started.elapsed().as_secs_f64();
    let grouped = GeometryControl::unlimited();
    let started = Instant::now();
    let mut grouped_sum = 0u64;
    for start in (0..COUNT).step_by(128) {
        let end = (start + 128).min(COUNT);
        grouped.charge_operations(end - start).unwrap();
        for i in start..end {
            grouped_sum = grouped_sum.wrapping_add(black_box(i));
        }
    }
    let grouped_seconds = started.elapsed().as_secs_f64();
    assert_eq!(scalar_sum, grouped_sum);
    assert_eq!(scalar.counters().operations, COUNT);
    assert_eq!(grouped.counters().operations, COUNT);
    println!("WORK_ADMISSION operations={COUNT} scalar_calls={COUNT} grouped_calls={} scalar_seconds={scalar_seconds:.6} grouped_seconds={grouped_seconds:.6}", COUNT.div_ceil(128));
}
