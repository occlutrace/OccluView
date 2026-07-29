//! Regression against real scan geometry.
//!
//! Synthetic domes prove the maths; they do not prove it survives a real
//! surface with its noise, its holes, and its uneven triangle sizes. This test
//! takes actual meshes, displaces each by a known rigid transform, and requires
//! the refine to bring it home.
//!
//! Fixtures live outside the repository — scan data does not belong in git.
//! Point `OCCLUVIEW_ALIGN_FIXTURES` at a directory of binary STL files to run
//! it; without that the test reports that it skipped and passes, so CI stays
//! green without shipping meshes.
//!
//! The STL reader here is deliberately local. Pulling in the format crate would
//! give this leaf crate a dev-dependency on half the workspace for forty lines
//! of parsing.

// A skipped run and a per-mesh result line are the whole point of this test:
// without them a green tick would not distinguish "verified on real geometry"
// from "no fixtures present".
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::cast_precision_loss
)]

use std::path::{Path, PathBuf};

use glam::{DQuat, DVec3};
use occluview_align::{
    deviation, deviation_stats, observability, refine, reverse_deviation, surface_agreement,
    CancelFlag, DeviationSettings, RefineSettings, Rigid, Soup, SurfaceIndex,
};

/// Residual the refine must reach, in millimetres.
const MAX_RESIDUAL_MM: f64 = 0.05;
/// Share of vertices that must carry a measurement.
const MIN_MEASURED_SHARE: f64 = 0.85;
/// Share that must land inside the clinical tolerance band.
const MIN_WITHIN_TOLERANCE: f64 = 0.90;
/// The tolerance band, in millimetres — the owner's "where it starts to go bad".
const TOLERANCE_MM: f64 = 0.2;

#[test]
fn a_real_scan_returns_to_a_known_pose_and_measures_clean() {
    let Some(files) = fixtures() else {
        eprintln!(
            "skipped: set OCCLUVIEW_ALIGN_FIXTURES to a directory of binary STL files to run this"
        );
        return;
    };
    assert!(
        !files.is_empty(),
        "OCCLUVIEW_ALIGN_FIXTURES holds no .stl files"
    );

    for path in files {
        let (positions, indices) = read_binary_stl(&path);
        let soup = Soup {
            positions: &positions,
            indices: &indices,
            mask: None,
        };
        assert!(
            soup.triangle_count() > 100,
            "{} has too little geometry to be a scan",
            path.display()
        );
        let index = SurfaceIndex::build(soup).expect("a real mesh must index");

        // A displacement a hand would leave behind: about a third of a
        // millimetre and a fraction of a degree.
        let start = Rigid::new(
            DQuat::from_axis_angle(DVec3::new(0.3, 0.5, 0.8).normalize(), 0.01),
            DVec3::new(0.20, -0.15, 0.12),
        );

        let report = refine(
            soup,
            &index,
            start,
            &RefineSettings::default(),
            &CancelFlag::new(),
        )
        .expect("a real scan against itself must refine");

        assert!(
            report.rms < MAX_RESIDUAL_MM,
            "{}: residual {:.4} mm exceeds {MAX_RESIDUAL_MM} mm",
            path.display(),
            report.rms
        );
        assert!(
            report.rigid.translation.length() < MAX_RESIDUAL_MM * 4.0,
            "{}: the pose did not come home, {:?} remains",
            path.display(),
            report.rigid.translation
        );

        let map = deviation(
            soup,
            &index,
            report.rigid,
            &DeviationSettings::default(),
            &CancelFlag::new(),
        );
        let stats = deviation_stats(&map, TOLERANCE_MM);
        let total = f64::from(stats.measured + stats.unmeasured.total()).max(1.0);
        let measured_share = f64::from(stats.measured) / total;
        let summary = stats
            .summary
            .expect("a real scan against itself measures far more than MIN_MEASURED vertices");

        assert!(
            measured_share > MIN_MEASURED_SHARE,
            "{}: only {:.0}% of the surface could be measured",
            path.display(),
            measured_share * 100.0
        );
        assert!(
            summary.within_tolerance > MIN_WITHIN_TOLERANCE,
            "{}: only {:.0}% landed within {TOLERANCE_MM} mm",
            path.display(),
            summary.within_tolerance * 100.0
        );

        eprintln!(
            "{}: {} triangles, rms {:.4} mm, {:.0}% measured, {:.0}% within {TOLERANCE_MM} mm",
            path.display(),
            soup.triangle_count(),
            report.rms,
            measured_share * 100.0,
            summary.within_tolerance * 100.0
        );
    }
}

/// A rigid offset a real scan is displaced by, and what each measure says.
///
/// This is the test that would have caught the under-reporting. It displaces a
/// real arch by a known amount and asserts three things about it: that the raw
/// one-sided statistic understates the truth badly, that it understates it
/// *however* the scan is displaced, and that the observability estimate brings
/// it back. The first two assertions look odd for a passing test — they require
/// a known flaw to still be present — but that is exactly the point. If somebody
/// later "fixes" `deviation` so the mean tracks the truth, these fire and force
/// the reader to notice, because a nearest-point map cannot do that and a mean
/// that suddenly does is measuring something else.
#[test]
fn a_known_rigid_offset_is_under_reported_and_the_estimate_corrects_it() {
    /// Displacement applied, in millimetres.
    const OFFSET_MM: f64 = 0.30;
    /// Along the blind mode the estimate is required to be *tight*, which is
    /// what proves the correction is the right size and not merely large. Along
    /// any other direction it may overstate by the spread of the spectrum,
    /// because it corrects by the worst sensitivity.
    const ESTIMATE_HIGH: f64 = 1.35;

    let Some(files) = fixtures() else {
        eprintln!("skipped: set OCCLUVIEW_ALIGN_FIXTURES to run this");
        return;
    };

    for path in files {
        let (positions, indices) = read_binary_stl(&path);
        let soup = Soup {
            positions: &positions,
            indices: &indices,
            mask: None,
        };
        let index = SurfaceIndex::build(soup).expect("a real mesh must index");
        let cancel = CancelFlag::new();
        let settings = DeviationSettings::default();

        let seen = observability(soup, &index, Rigid::IDENTITY, &settings, &cancel)
            .expect("a real arch determines all six freedoms");
        assert!(
            seen.worst_sensitivity() > 0.05,
            "{}: a whole arch should have no fully blind direction, got {:?}",
            path.display(),
            seen.sensitivity
        );

        // Every axis, plus the direction the estimate itself calls blindest.
        let mut cases: Vec<(String, Rigid)> = ["x", "y", "z"]
            .iter()
            .enumerate()
            .map(|(axis, name)| {
                let mut direction = DVec3::ZERO;
                direction[axis] = OFFSET_MM;
                ((*name).to_string(), Rigid::new(DQuat::IDENTITY, direction))
            })
            .collect();
        let angle = seen.blind_rotation.length() * OFFSET_MM;
        let turn = if angle > 0.0 {
            DQuat::from_axis_angle(seen.blind_rotation.normalize(), angle)
        } else {
            DQuat::IDENTITY
        };
        cases.push((
            "blind mode".into(),
            Rigid::new(
                turn,
                seen.pivot + seen.blind_translation * OFFSET_MM - turn * seen.pivot,
            ),
        ));

        // Applied along any direction but the blindest, the estimate is an
        // upper bound rather than an equality: it corrects by the worst
        // sensitivity, so it overstates by at most the spread of the spectrum.
        let spread = seen.best_sensitivity() / seen.worst_sensitivity();

        for (name, pose) in cases {
            let ceiling = if name == "blind mode" {
                ESTIMATE_HIGH
            } else {
                spread * ESTIMATE_HIGH
            };
            check_offset(&Offset {
                label: &format!("{} {name}", path.display()),
                positions: &positions,
                soup,
                index: &index,
                seen: &seen,
                pose,
                ceiling,
            });
        }
    }
}

/// One displaced fixture and everything needed to judge it.
struct Offset<'a> {
    label: &'a str,
    positions: &'a [f32],
    soup: Soup<'a>,
    index: &'a SurfaceIndex,
    seen: &'a occluview_align::Observability,
    pose: Rigid,
    ceiling: f64,
}

/// Measure one known offset three ways and hold each to what it promises.
fn check_offset(case: &Offset<'_>) {
    /// The one-sided statistic must come in below this share of the truth.
    const MAX_HONEST_SHARE: f64 = 0.80;
    /// The corrected estimate must never fall below this share of the truth.
    const ESTIMATE_LOW: f64 = 0.85;

    let label = case.label;
    let cancel = CancelFlag::new();
    let settings = DeviationSettings::default();
    let truth = rms_displacement(case.positions, case.pose);
    let map = deviation(case.soup, case.index, case.pose, &settings, &cancel);
    let stats = deviation_stats(&map, TOLERANCE_MM);
    let summary = stats
        .summary
        .expect("a real arch scan has far more than MIN_MEASURED vertices in reach");
    let reverse = reverse_deviation(case.soup, case.index, case.pose, &settings, &cancel);
    let agreement = surface_agreement(&map, &reverse, TOLERANCE_MM);
    let estimate = case.seen.hidden_displacement_mm(summary.rms);

    assert!(
        summary.rms < truth * MAX_HONEST_SHARE,
        "{label}: the one-sided rms {:.4} no longer under-reports {truth:.4}. A \
         nearest-point map cannot track a tangential offset, so either the measure \
         changed or this fixture did — do not relax this, work out which.",
        summary.rms
    );
    assert!(
        estimate > truth * ESTIMATE_LOW,
        "{label}: the corrected estimate {estimate:.4} understated the true \
         displacement {truth:.4}"
    );
    assert!(
        estimate < truth * case.ceiling,
        "{label}: the corrected estimate {estimate:.4} is looser than the sensitivity \
         spread allows against a true {truth:.4}"
    );
    assert!(
        (estimate - truth).abs() < (summary.rms - truth).abs(),
        "{label}: the correction must land closer to the truth than the raw statistic \
         did — estimate {estimate:.4}, raw {:.4}, truth {truth:.4}",
        summary.rms
    );
    let pooled = agreement
        .summary
        .expect("a real arch scan against itself measures plenty in both directions");
    assert!(
        pooled.rms > 0.0 && agreement.measured > stats.measured,
        "{label}: the symmetric measure must pool both directions"
    );

    let balanced_mean_abs = agreement
        .balanced_mean_abs()
        .expect("a real arch scan clears MIN_MEASURED on both directions of the symmetric measure");
    eprintln!(
        "{label}: true {truth:.4} mm, one-sided rms {:.4} ({:.0}%), symmetric rms \
         {:.4}, balanced {:.4}, HD95 {:.4}, corrected {estimate:.4}",
        summary.rms,
        summary.rms / truth * 100.0,
        pooled.rms,
        balanced_mean_abs,
        pooled.hausdorff_p95,
    );
}

/// Root-mean-square true displacement of every vertex under `pose`. The mesh is
/// compared against itself, so material correspondence is the identity and this
/// is the ground truth by construction.
fn rms_displacement(positions: &[f32], pose: Rigid) -> f64 {
    let mut squares = 0.0;
    let mut count = 0usize;
    for point in positions.chunks_exact(3) {
        let local = DVec3::new(
            f64::from(point[0]),
            f64::from(point[1]),
            f64::from(point[2]),
        );
        squares += (pose.apply(local) - local).length_squared();
        count += 1;
    }
    (squares / count.max(1) as f64).sqrt()
}

/// Every `.stl` in the fixture directory, sorted so a failure names the same
/// file on every machine.
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

/// Minimal binary STL reader: an 80-byte header, a triangle count, then 50
/// bytes per facet. Vertices are emitted as soup, which is what an STL is.
fn read_binary_stl(path: &Path) -> (Vec<f32>, Vec<u32>) {
    let bytes = std::fs::read(path).expect("fixture must be readable");
    assert!(
        bytes.len() > 84,
        "{} is too short to be an STL",
        path.display()
    );
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;

    let mut positions = Vec::with_capacity(count * 9);
    let mut indices = Vec::with_capacity(count * 3);
    for triangle in 0..count {
        let base = 84 + triangle * 50;
        if base + 50 > bytes.len() {
            break;
        }
        // Skip the facet normal: it is computed from the winding anyway.
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
        let first = u32::try_from(triangle * 3).expect("triangle index fits");
        indices.extend_from_slice(&[first, first + 1, first + 2]);
    }
    (positions, indices)
}
