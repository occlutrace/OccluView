//! Exact clipping contracts through the bounded synthetic diagnostic.
#![cfg(feature = "search-probe")]
#![allow(clippy::unwrap_used)]

use glam::DVec3;
use occluview_align::{probe_cell_clipping, CancelFlag};
use occluview_geometry::surface::{GeometryControl, GeometryLimits, GeometryStop};
use std::time::Duration;

#[test]
fn unchanged_halfspace_keeps_vertices_without_reconstructing() {
    let control = GeometryControl::unlimited();
    let points = [
        DVec3::new(0.2, 0.3, 0.4),
        DVec3::new(0.3, 0.7, 0.2),
        DVec3::new(0.8, 0.5, 0.6),
    ];
    for axis in 0..3 {
        let result = probe_cell_clipping(points, axis, 0, &control).unwrap();
        assert_eq!(result.len, 3);
        assert_eq!(result.points[..3], points);
        assert_eq!(result.barycentric[..3], [DVec3::X, DVec3::Y, DVec3::Z]);
        assert_eq!(result.operations, 3);
    }
    let result = probe_cell_clipping(
        [
            DVec3::new(-0.5, 0., 0.),
            DVec3::new(0.5, 0., 0.),
            DVec3::new(0.5, 1., 0.),
        ],
        0,
        0,
        &control,
    )
    .unwrap();
    assert_eq!(result.len, 4);
    assert_eq!(
        result.points[..4],
        [
            DVec3::new(0., 0.5, 0.),
            DVec3::ZERO,
            DVec3::new(0.5, 0., 0.),
            DVec3::new(0.5, 1., 0.),
        ]
    );
    assert_eq!(
        result.barycentric[..4],
        [
            DVec3::new(0.5, 0., 0.5),
            DVec3::new(0.5, 0.5, 0.),
            DVec3::Y,
            DVec3::Z,
        ]
    );
    assert_eq!(result.operations, 3);
}

#[test]
fn two_sided_cut_matches_analytic_polygon_and_closed_boundaries() {
    let control = GeometryControl::unlimited();
    let result = probe_cell_clipping(
        [
            DVec3::new(-0.5, 0., 0.),
            DVec3::new(1.5, 0., 0.),
            DVec3::new(1.5, 1., 0.),
        ],
        0,
        0,
        &control,
    )
    .unwrap();
    assert_eq!(result.len, 4);
    let expected = [
        (DVec3::new(1., 0.75, 0.), DVec3::new(0.25, 0., 0.75)),
        (DVec3::new(0., 0.25, 0.), DVec3::new(0.75, 0., 0.25)),
        (DVec3::ZERO, DVec3::new(0.75, 0.25, 0.)),
        (DVec3::new(1., 0., 0.), DVec3::new(0.25, 0.75, 0.)),
    ];
    for (i, (point, barycentric)) in expected.into_iter().enumerate() {
        assert!((result.points[i] - point).abs().max_element() <= 1e-14);
        assert!((result.barycentric[i] - barycentric).abs().max_element() <= 1e-14);
    }
    assert_eq!(result.operations, 7);
    for point in [DVec3::ZERO, DVec3::ONE] {
        let result = probe_cell_clipping([point; 3], 0, 0, &control).unwrap();
        assert_eq!(result.len, 3);
        assert_eq!(result.points[..3], [point; 3]);
        assert_eq!(result.operations, 3);
    }
    for point in [DVec3::splat(-0.1), DVec3::splat(1.1)] {
        assert_eq!(
            probe_cell_clipping([point; 3], 0, 0, &control).unwrap().len,
            0
        );
    }
}

#[test]
fn clipping_probe_rejects_bad_input_and_honors_control() {
    let control = GeometryControl::unlimited();
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            probe_cell_clipping([DVec3::splat(invalid); 3], 0, 0, &control),
            Err(GeometryStop::Numerical)
        );
    }
    assert_eq!(
        probe_cell_clipping([DVec3::ZERO; 3], usize::MAX, 0, &control),
        Err(GeometryStop::Numerical)
    );
    assert_eq!(
        probe_cell_clipping([DVec3::splat(f64::MAX); 3], 0, 0, &control),
        Err(GeometryStop::ResourceLimit)
    );
    for coordinate in [i64::MIN, i64::MAX] {
        assert_eq!(
            probe_cell_clipping([DVec3::ZERO; 3], 0, coordinate, &control),
            Err(GeometryStop::ResourceLimit)
        );
    }
    let cancel = CancelFlag::new();
    cancel.cancel();
    let cancelled = GeometryControl::new(cancel, Duration::MAX, GeometryLimits::default());
    assert_eq!(
        probe_cell_clipping([DVec3::ZERO; 3], 0, 0, &cancelled),
        Err(GeometryStop::Cancelled)
    );
    let bounded = GeometryControl::new(
        CancelFlag::new(),
        Duration::MAX,
        GeometryLimits {
            operations: 2,
            ..GeometryLimits::default()
        },
    );
    assert_eq!(
        probe_cell_clipping([DVec3::ZERO; 3], 0, 0, &bounded),
        Err(GeometryStop::WorkLimit)
    );
    assert_eq!(bounded.counters().operations, 0);
}
