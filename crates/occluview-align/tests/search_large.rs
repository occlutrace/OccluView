//! Synthetic-only regression: scans of a million triangles are searched whole.
#![allow(
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::print_stdout
)]
mod support;
use glam::{DQuat, DVec3};
use occluview_align::{
    search_alignment, Completion, Confidence, Rigid, SearchControl, SearchSettings,
};
use std::collections::HashMap;
use support::{alignment_input, arch, metrics, operators, SyntheticMesh};

/// Every triangle cut into four at the middles of its edges, the middles
/// shared between neighbours so the surface stays one piece.
fn quartered(mesh: &SyntheticMesh) -> SyntheticMesh {
    let mut out = mesh.clone();
    out.triangles.clear();
    out.region_ids.clear();
    let mut middles: HashMap<(u32, u32), u32> = HashMap::new();
    let mut middle = |a: u32, b: u32, out: &mut SyntheticMesh| {
        *middles.entry((a.min(b), a.max(b))).or_insert_with(|| {
            let point = (mesh.point(a as usize) + mesh.point(b as usize)) * 0.5;
            out.positions
                .extend(point.to_array().map(|value| value as f32));
            out.parameters.push([0.; 2]);
            (out.positions.len() / 3 - 1) as u32
        })
    };
    for (t, &region) in mesh
        .triangles
        .as_chunks::<3>()
        .0
        .iter()
        .zip(&mesh.region_ids)
    {
        let [a, b, c] = *t;
        let (ab, bc, ca) = (
            middle(a, b, &mut out),
            middle(b, c, &mut out),
            middle(c, a, &mut out),
        );
        out.triangles
            .extend([a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
        out.region_ids.extend([region; 4]);
    }
    out
}

/// Scans of several hundred thousand triangles are read on their triangles
/// and scans of more than a million on an even cloud of them; either way the
/// search ends within its deadline at the true pose and says how it read.
#[test]
fn large_scans_are_searched_whole() {
    let mut fixed = arch::dental_arch(&arch::ArchSpec::default());
    for (least, exact) in [(300_000, true), (1_200_000, false)] {
        while fixed.triangles.len() / 3 < least {
            fixed = quartered(&fixed);
        }
        let truth = Rigid::new(
            DQuat::from_axis_angle(DVec3::new(0.4, 0.1, -0.9).normalize(), 2.6),
            DVec3::new(-17., 9., 12.),
        );
        let moving = operators::rigid_offset(&fixed, truth);
        let clock = std::time::Instant::now();
        let result = search_alignment(
            &alignment_input(moving.soup(), fixed.soup()),
            &SearchSettings::default(),
            &SearchControl::default(),
        )
        .unwrap();
        let elapsed = clock.elapsed();
        let probes: Vec<DVec3> = (0..moving.positions.len() / 3)
            .step_by(977)
            .map(|i| moving.point(i))
            .collect();
        let best = &result.candidates[0];
        let error = metrics::probe_error(best.pose, truth, &probes);
        println!(
            "triangles={} completion={:?} class={:?} reasons={:?} gap={:?} probe_rms_mm={} elapsed={elapsed:?}",
            fixed.triangles.len() / 3,
            result.completion,
            best.confidence,
            best.reasons,
            best.evidence.rival_gap,
            error[0]
        );
        assert_eq!(result.completion, Completion::Complete);
        assert_eq!(best.evidence.original_surface_exact, [exact; 2]);
        assert_eq!(best.confidence, Confidence::Verified);
        assert!(error[0] < 0.01, "{error:?}");
    }
}
