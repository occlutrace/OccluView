//! Manual load-path performance baselines.
//!
//! These tests are ignored because shared-runner timing is not stable enough
//! for a useful CI gate. Run them before and after load-path changes:
//!
//! ```text
//! cargo test -p occluview-formats --release -- --ignored --nocapture load_
//! ```
//!
//! Reference timings from the original release-mode, eight-thread run:
//!
//! ```text
//! load_binary_stl_500k_triangles   0.24 s
//! load_binary_stl_2m_triangles     0.89 s
//! ```

// A perf harness whose whole output is a printed number.
#![allow(clippy::print_stdout, clippy::expect_used)]

use std::time::{Duration, Instant};

/// A binary STL of `triangles` facets, in the shape a scanner writes: separate
/// vertices per facet, so the loader does the welding work a real scan costs.
fn binary_stl(triangles: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(84 + triangles * 50);
    bytes.extend_from_slice(&[0_u8; 80]);
    bytes.extend_from_slice(&u32::try_from(triangles).unwrap_or(u32::MAX).to_le_bytes());
    for index in 0..triangles {
        let step = index as f32 * 0.01;
        let corners = [
            [step, 0.0, 0.0],
            [step + 0.01, 0.0, 0.0],
            [step, 0.01, (index % 97) as f32 * 0.001],
        ];
        bytes.extend_from_slice(&0.0_f32.to_le_bytes());
        bytes.extend_from_slice(&0.0_f32.to_le_bytes());
        bytes.extend_from_slice(&1.0_f32.to_le_bytes());
        for corner in corners {
            for value in corner {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&0_u16.to_le_bytes());
    }
    bytes
}

fn time_parse(triangles: usize) -> Duration {
    let bytes = binary_stl(triangles);
    let started = Instant::now();
    let mesh = crate::dispatch::dispatch_by_extension("stl", &bytes);
    let elapsed = started.elapsed();
    let mesh = mesh.expect("the fixture is a valid binary STL");
    assert_eq!(mesh.triangle_count(), triangles);
    elapsed
}

/// Compare a release-mode run against a relaxed multiple of the baseline.
/// Debug builds report timing without enforcing the release threshold.
fn assert_within_baseline(what: &str, elapsed: Duration, ceiling: Duration) {
    if cfg!(debug_assertions) {
        println!(
            "  (debug build: {elapsed:?} is not judged against {ceiling:?}; \
             re-run with --release)"
        );
        return;
    }
    assert!(
        elapsed < ceiling,
        "parsing {what} took {elapsed:?} against a {ceiling:?} ceiling \
         (five times the recorded baseline); the load path has regressed"
    );
}

#[test]
#[ignore = "perf baseline: run with --release --ignored --nocapture"]
fn load_binary_stl_500k_triangles() {
    let elapsed = time_parse(500_000);
    println!("LOAD 500k triangles -> {elapsed:?}");
    assert_within_baseline("500k triangles", elapsed, Duration::from_millis(1_200));
}

#[test]
#[ignore = "perf baseline: run with --release --ignored --nocapture"]
fn load_binary_stl_2m_triangles() {
    let elapsed = time_parse(2_000_000);
    println!("LOAD 2M triangles -> {elapsed:?}");
    assert_within_baseline("2M triangles", elapsed, Duration::from_millis(4_500));
}
