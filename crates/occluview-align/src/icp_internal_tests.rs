//! Focused tests for ICP state transitions that are not part of the public API.

#![allow(clippy::expect_used, clippy::panic)]

use super::{forward_coverage_is_sufficient, run_level, sample_vertices, vertex_normals, Level};
use crate::{CancelFlag, FitRejection, RefineSettings, Soup, SurfaceIndex};
use glam::{DQuat, DVec3};

fn flat_sheet() -> (Vec<f32>, Vec<u32>) {
    let positions = vec![
        0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, // row 0
        0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 2.0, 1.0, 0.0, // row 1
        0.0, 2.0, 0.0, 1.0, 2.0, 0.0, 2.0, 2.0, 0.0, // row 2
    ];
    let indices = vec![
        0, 1, 3, 1, 4, 3, 1, 2, 4, 2, 5, 4, 3, 4, 6, 4, 7, 6, 4, 5, 7, 5, 8, 7,
    ];
    (positions, indices)
}

#[test]
fn rank_deficient_nonzero_residual_is_not_reported_as_refined() {
    let (positions, indices) = flat_sheet();
    let moving = Soup {
        positions: &positions,
        indices: &indices,
        mask: None,
    };
    let fixed = SurfaceIndex::build(moving).expect("flat sheet is usable");
    let normals = vertex_normals(moving);
    let fixed_samples = fixed.representative_samples(64);
    let samples = sample_vertices(moving, 64);
    let settings = RefineSettings::default();
    let cancel = CancelFlag::new();
    let level = Level {
        moving,
        normals: &normals,
        fixed: &fixed,
        moving_surface: Some(&fixed),
        fixed_samples: &fixed_samples,
        samples: &samples,
        settings: &settings,
        cancel: &cancel,
        start: crate::Rigid::new(DQuat::IDENTITY, DVec3::new(0.0, 0.0, 0.2)),
    };

    let outcome = run_level(&level);

    assert!(
        matches!(outcome, Err(FitRejection::NoImprovement)),
        "rank-deficient nonzero residual must not authorize a heatmap"
    );
}

#[test]
fn a_large_scan_cannot_be_registered_from_a_tiny_accidental_patch() {
    assert!(!forward_coverage_is_sufficient(6, 40_000));
    assert!(forward_coverage_is_sufficient(400, 40_000));
    assert!(forward_coverage_is_sufficient(6, 400));
}
