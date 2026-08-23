//! A measured baseline for the path that is the product.
//!
//! Opening a file runs mmap-or-read, parse, normal generation and mesh
//! construction, and every one of those has been changed this cycle. The 1600
//! correctness tests would all stay green if a dependency bump made a
//! four-hundred-megabyte scan thirty percent slower, because none of them look
//! at time.
//!
//! Deliberately `#[ignore]` and deliberately not in CI: shared runners vary by
//! more than any regression worth catching, and a gate that cries wolf is worse
//! than no gate. Run it by hand before and after anything that touches the load
//! path:
//!
//! ```text
//! cargo test -p occluview-formats --release -- --ignored --nocapture load_
//! ```
//!
//! Recorded on the machine this was written on (release, 8 threads), so a later
//! run has something to compare against rather than a bare number:
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

#[test]
#[ignore = "perf baseline: run with --release --ignored --nocapture"]
fn load_binary_stl_500k_triangles() {
    let elapsed = time_parse(500_000);
    println!("LOAD 500k triangles -> {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(20),
        "parsing 500k triangles took {elapsed:?}; the load path has regressed by an order of magnitude"
    );
}

#[test]
#[ignore = "perf baseline: run with --release --ignored --nocapture"]
fn load_binary_stl_2m_triangles() {
    let elapsed = time_parse(2_000_000);
    println!("LOAD 2M triangles -> {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(60),
        "parsing 2M triangles took {elapsed:?}; the load path has regressed by an order of magnitude"
    );
}
