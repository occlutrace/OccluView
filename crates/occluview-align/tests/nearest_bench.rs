//! Temporary timing harness for the nearest-surface query. Not a permanent test.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::cast_precision_loss,
    missing_docs
)]

use std::path::{Path, PathBuf};
use std::time::Instant;

use occluview_align::{deviation, CancelFlag, DeviationSettings, Rigid, Soup, SurfaceIndex};

#[test]
fn bench_nearest() {
    let Some(files) = fixtures() else {
        eprintln!("skipped: set OCCLUVIEW_ALIGN_FIXTURES");
        return;
    };
    for path in &files {
        let (positions, indices) = read_binary_stl(path);
        let soup = Soup {
            positions: &positions,
            indices: &indices,
            mask: None,
        };
        let started = Instant::now();
        let index = SurfaceIndex::build(soup).expect("index");
        let build_ms = started.elapsed().as_secs_f64() * 1e3;
        eprintln!(
            "{}: {} tri, {} vtx, cell {:.4} mm, build {:.1} ms",
            path.file_name().unwrap().to_string_lossy(),
            soup.triangle_count(),
            soup.vertex_count(),
            index.cell_size(),
            build_ms
        );

        for radius in [2.0f64, 5.0] {
            // Self-comparison with a small offset: what the panel actually does
            // after a refine.
            let pose = Rigid::new(glam::DQuat::IDENTITY, glam::DVec3::new(0.1, 0.05, 0.03));
            let settings = DeviationSettings {
                influence_radius_mm: radius,
                ..DeviationSettings::default()
            };
            let started = Instant::now();
            let map = deviation(soup, &index, pose, &settings, &CancelFlag::new());
            let elapsed = started.elapsed().as_secs_f64();
            let measured = map
                .validity
                .iter()
                .filter(|v| matches!(v, occluview_align::Validity::Measured))
                .count();
            eprintln!(
                "  deviation r={radius} mm: {:.3} s  ({:.2} us/vertex, {:.0}% measured)",
                elapsed,
                elapsed * 1e6 / map.signed_mm.len() as f64,
                100.0 * measured as f64 / map.signed_mm.len() as f64
            );
        }

        // Single-thread query cost, split by whether the answer is in reach.
        let mut near = 0u64;
        let mut far = 0u64;
        let mut near_time = 0.0f64;
        let mut far_time = 0.0f64;
        let step = (soup.vertex_count() / 20_000).max(1);
        for vertex in (0..soup.vertex_count()).step_by(step) {
            let p = glam::DVec3::new(
                f64::from(positions[vertex * 3]) + 0.1,
                f64::from(positions[vertex * 3 + 1]),
                f64::from(positions[vertex * 3 + 2]),
            );
            let started = Instant::now();
            let hit = index.nearest(p, 5.0);
            let elapsed = started.elapsed().as_secs_f64();
            if hit.is_some() {
                near += 1;
                near_time += elapsed;
            } else {
                far += 1;
                far_time += elapsed;
            }
        }
        eprintln!(
            "  1-thread r=5: hit {near} at {:.1} us, miss {far} at {:.1} us",
            near_time * 1e6 / near.max(1) as f64,
            far_time * 1e6 / far.max(1) as f64
        );

        // Worst realistic case: a pose so far off that most vertices find
        // nothing, so every query must prove the whole window empty.
        for offset in [8.0f64] {
            let pose = Rigid::new(glam::DQuat::IDENTITY, glam::DVec3::new(offset, offset, 0.0));
            let settings = DeviationSettings {
                influence_radius_mm: 5.0,
                ..DeviationSettings::default()
            };
            let started = Instant::now();
            let map = deviation(soup, &index, pose, &settings, &CancelFlag::new());
            let elapsed = started.elapsed().as_secs_f64();
            let measured = map
                .validity
                .iter()
                .filter(|v| matches!(v, occluview_align::Validity::Measured))
                .count();
            eprintln!(
                "  deviation r=5 offset {offset} mm: {:.3} s ({:.0}% measured)",
                elapsed,
                100.0 * measured as f64 / map.signed_mm.len() as f64
            );
        }

        // Points inside the mesh box but away from any surface: the case the
        // shell walk cannot short-circuit.
        let lo = glam::DVec3::new(
            positions
                .chunks_exact(3)
                .map(|p| f64::from(p[0]))
                .fold(f64::INFINITY, f64::min),
            positions
                .chunks_exact(3)
                .map(|p| f64::from(p[1]))
                .fold(f64::INFINITY, f64::min),
            positions
                .chunks_exact(3)
                .map(|p| f64::from(p[2]))
                .fold(f64::INFINITY, f64::min),
        );
        let hi = glam::DVec3::new(
            positions
                .chunks_exact(3)
                .map(|p| f64::from(p[0]))
                .fold(f64::NEG_INFINITY, f64::max),
            positions
                .chunks_exact(3)
                .map(|p| f64::from(p[1]))
                .fold(f64::NEG_INFINITY, f64::max),
            positions
                .chunks_exact(3)
                .map(|p| f64::from(p[2]))
                .fold(f64::NEG_INFINITY, f64::max),
        );
        let mut inside_hit = (0u64, 0.0f64);
        let mut inside_miss = (0u64, 0.0f64);
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..5000 {
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 11) as f64 / (1u64 << 53) as f64
            };
            let p = lo + (hi - lo) * glam::DVec3::new(next(), next(), next());
            let started = Instant::now();
            let hit = index.nearest(p, 5.0);
            let elapsed = started.elapsed().as_secs_f64();
            if hit.is_some() {
                inside_hit.0 += 1;
                inside_hit.1 += elapsed;
            } else {
                inside_miss.0 += 1;
                inside_miss.1 += elapsed;
            }
        }
        eprintln!(
            "  1-thread r=5 inside box: hit {} at {:.1} us, miss {} at {:.1} us",
            inside_hit.0,
            inside_hit.1 * 1e6 / inside_hit.0.max(1) as f64,
            inside_miss.0,
            inside_miss.1 * 1e6 / inside_miss.0.max(1) as f64
        );

        // A deliberately empty region: how a miss costs when far from anything.
        let mut miss_time = 0.0f64;
        let mut count = 0u64;
        for k in 0..2000 {
            let p = glam::DVec3::new(
                f64::from(positions[0]) + 40.0 + f64::from(k % 17),
                f64::from(positions[1]) + f64::from(k % 23),
                f64::from(positions[2]) + f64::from(k % 13),
            );
            let started = Instant::now();
            let hit = index.nearest(p, 5.0);
            miss_time += started.elapsed().as_secs_f64();
            assert!(hit.is_none() || hit.is_some());
            count += 1;
        }
        eprintln!(
            "  1-thread r=5 far-outside miss: {:.1} us",
            miss_time * 1e6 / count as f64
        );
    }
}

fn fixtures() -> Option<Vec<PathBuf>> {
    let directory = std::env::var("OCCLUVIEW_ALIGN_FIXTURES").ok()?;
    let mut files: Vec<PathBuf> = std::fs::read_dir(directory)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("stl"))
        })
        .collect();
    files.sort();
    Some(files)
}

fn read_binary_stl(path: &Path) -> (Vec<f32>, Vec<u32>) {
    let bytes = std::fs::read(path).expect("fixture must be readable");
    assert!(bytes.len() > 84);
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    let mut positions = Vec::with_capacity(count * 9);
    let mut indices = Vec::with_capacity(count * 3);
    for triangle in 0..count {
        let base = 84 + triangle * 50;
        if base + 50 > bytes.len() {
            break;
        }
        for corner in 0..3 {
            for axis in 0..3 {
                let at = base + 12 + corner * 12 + axis * 4;
                positions.push(f32::from_le_bytes([
                    bytes[at],
                    bytes[at + 1],
                    bytes[at + 2],
                    bytes[at + 3],
                ]));
            }
        }
        let first = u32::try_from(triangle * 3).expect("fits");
        indices.extend_from_slice(&[first, first + 1, first + 2]);
    }
    (positions, indices)
}
