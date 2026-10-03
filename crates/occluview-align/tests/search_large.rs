//! Synthetic-only large representation regressions.
#![allow(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::panic,
    clippy::print_stdout
)]
use glam::{DAffine3, DVec3};
use occluview_align::{prepare_alignment_surface, MeshInput, RegionPolicy, Soup, SurfaceSide};
use occluview_geometry::surface::{GeometryControl, GeometryLimits, QueryOutcome};
use std::time::Duration;

/// D4: full area is accounted before a bounded distributed representation.
#[test]
fn large_representation_remains_useful() {
    for count in [250_000, 1_000_000] {
        // Independent triangles cover a fixed 10 x 10 mm planar region.
        let width = 500usize;
        let height = count / (2 * width);
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for y in 0..=height {
            for x in 0..=width {
                positions.extend([
                    10. * x as f32 / width as f32,
                    10. * y as f32 / height as f32,
                    0.,
                ]);
            }
        }
        for y in 0..height {
            for x in 0..width {
                let a = (y * (width + 1) + x) as u32;
                let b = a + 1;
                let c = a + (width + 1) as u32;
                indices.extend([a, b, c, b, c + 1, c]);
            }
        }
        let control = GeometryControl::new(
            occluview_align::CancelFlag::default(),
            Duration::from_secs(10),
            GeometryLimits::default(),
        );
        let clock = std::time::Instant::now();
        let mut surfaces = Vec::new();
        for side in [SurfaceSide::Moving, SurfaceSide::Fixed] {
            let prepared = prepare_alignment_surface(
                MeshInput {
                    soup: Soup {
                        positions: &positions,
                        indices: &indices,
                        mask: None,
                    },
                    world_from_local: DAffine3::IDENTITY,
                    revision: count as u64,
                },
                side,
                RegionPolicy::AllEligible,
                &control,
            )
            .unwrap();
            assert!(
                prepared.surface.is_some(),
                "{count}: {:?}",
                prepared.completion
            );
            let surface = prepared.surface.unwrap();
            assert!(surface.original_index.triangle_count() <= 128_000);
            assert!((surface.eligible_area_mm2 - 100.).abs() < 1e-6);
            assert_eq!(surface.revision, count as u64);
            assert!(!surface.exact_original);
            assert!(!surface.quality.orientation_coherent);
            assert!(surface
                .samples
                .iter()
                .all(|set| set.samples.iter().all(|sample| sample.normal.is_none())));
            let mut max_error = 0f64;
            for i in 0..8192 {
                let world = DVec3::new(
                    0.03 + 9.94 * (f64::from(i) + 0.5) / 8192.,
                    0.03 + 9.94 * (f64::from(i * 73 % 8192) + 0.5) / 8192.,
                    0.,
                );
                let point = world - surface.frame.center_world;
                let hit = match surface
                    .original_index
                    .nearest_controlled(point, 0.1, &control)
                {
                    QueryOutcome::Complete(Some(hit)) => hit,
                    other => panic!("incomplete approximation: {other:?}"),
                };
                max_error = max_error.max(point.distance(hit.point));
            }
            println!("triangles={count} side={side:?} proxy={} original_area={} proxy_area={} max_error_mm={max_error} peak_bytes={}", surface.original_index.triangle_count(), surface.eligible_area_mm2, surface.represented_area_mm2, control.counters().peak_memory_bytes);
            assert!(max_error <= 0.02);
            surfaces.push(surface);
        }
        assert!(clock.elapsed() <= Duration::from_millis(10_100));
        assert!(control.counters().peak_memory_bytes <= 256 * 1024 * 1024);
    }
}
