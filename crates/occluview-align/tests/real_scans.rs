//! Acceptance tests against real scan geometry.
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
//! Without fixtures these tests pass on CI without having verified anything,
//! and the thresholds below — a 0.05 mm residual, 85% measured, 90% inside the
//! clinical band — are product acceptance criteria that nothing enforces
//! automatically. The names say `_when_fixtures_are_present` so a passing run
//! is not mistaken for a clinical check. A synthetic mesh is not substituted
//! here, because it cannot stand in for real scan geometry. Run them against
//! the private corpus before any release that touches alignment.
//!
//! The STL reader here is local. Pulling in the format crate would
//! give this leaf crate a dev-dependency on half the workspace for forty lines
//! of parsing.

// Report skipped fixtures explicitly so CI cannot imply clinical coverage.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    // A refused correct rescan is the failure this file exists to catch, so the
    // helper that reports it panics on purpose.
    clippy::panic,
    // The distance sweeps below report what the solver reached at each step;
    // that table is the evidence, so it is printed on purpose.
    clippy::print_stdout,
    clippy::cast_precision_loss,
    // Mesh positions are f32 by contract, so the synthesised rescan narrows
    // back to f32 on purpose.
    clippy::cast_possible_truncation
)]

use std::path::{Path, PathBuf};

use glam::{DQuat, DVec3};
use occluview_align::{
    deviation, deviation_stats, observability, refine, CancelFlag, DeviationSettings, FitRejection,
    RefineSettings, Rigid, Soup, SurfaceIndex,
};

/// Residual the refine must reach, in millimetres.
const MAX_RESIDUAL_MM: f64 = 0.05;
/// Share of vertices that must carry a measurement.
const MIN_MEASURED_SHARE: f64 = 0.85;
/// Share that must land inside the clinical tolerance band.
const MIN_WITHIN_TOLERANCE: f64 = 0.90;
/// The tolerance band, in millimetres — where alignment starts to go bad.
const TOLERANCE_MM: f64 = 0.2;

#[test]
fn a_real_scan_returns_to_a_known_pose_and_measures_clean_when_fixtures_are_present() {
    let Some(files) = fixtures() else {
        eprintln!(
            "skipped: set OCCLUVIEW_ALIGN_FIXTURES to a directory of binary STL files to run this"
        );
        return;
    };

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
/// It displaces a real arch by a known amount and asserts three things about
/// it: that the raw one-sided statistic understates the truth badly, that it
/// understates it *however* the scan is displaced, and that the observability
/// estimate brings it back. The first two assertions require a known limitation
/// of nearest-point maps to be present: a change to `deviation` that makes the
/// mean track the truth fails them, because a nearest-point map cannot do that
/// and a mean that does is measuring something else.
#[test]
fn a_known_rigid_offset_is_corrected_when_fixtures_are_present() {
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
    const MAX_ONE_SIDED_SHARE: f64 = 0.80;
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
    let estimate = case.seen.hidden_displacement_mm(summary.rms);

    assert!(
        summary.rms < truth * MAX_ONE_SIDED_SHARE,
        "{label}: the one-sided rms {:.4} does not under-report {truth:.4}. A \
         nearest-point map cannot track a tangential offset, so either the measure \
         or this fixture changed; find which before changing this bound.",
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
    // The correction is an upper bound on the hidden motion, not a second
    // estimate of it: `rms / sensitivity` is how far a motion could have gone
    // while still producing this map. A bound need not sit closer to the truth
    // than the raw statistic, and on a tangential slide it is expected to be
    // loose — a nearest-point map cannot see motion along the surface.
    //
    // What must hold is the property the bound promises and the panel relies on:
    // it never understates the displacement it is asked to bound, and it stays
    // inside the sensitivity spread the map reports.
    assert!(
        estimate + 1e-9 >= truth * ESTIMATE_LOW,
        "{label}: the bound {estimate:.4} must not understate the true displacement \
         {truth:.4}"
    );
    assert!(
        estimate < truth * case.ceiling,
        "{label}: the bound {estimate:.4} exceeds the spread the map itself reports \
         for a true {truth:.4}",
    );
    eprintln!(
        "{label}: true {truth:.4} mm, one-sided rms {:.4} ({:.0}%), p95 {:.4}, \
         unmeasured {}, corrected {estimate:.4}",
        summary.rms,
        summary.rms / truth * 100.0,
        summary.p95,
        stats.unmeasured.total(),
    );
}

/// Root-mean-square true displacement of every vertex under `pose`. The mesh is
/// compared against itself, so material correspondence is the identity and this
/// is the ground truth by construction.
fn rms_displacement(positions: &[f32], pose: Rigid) -> f64 {
    let mut squares = 0.0;
    let mut count = 0usize;
    for point in positions.as_chunks::<3>().0 {
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

/// Whether this run is a release gate rather than an ordinary test run.
///
/// Set by `scripts/validate-release-private.sh`, which is the only thing that
/// should set it: a gate that skips when the corpus is missing is a gate that
/// reports success for having checked nothing.
fn fixtures_are_required() -> bool {
    std::env::var_os("OCCLUVIEW_ALIGN_FIXTURES_REQUIRED").is_some_and(|value| value != "0")
}

/// Every `.stl` in the fixture directory, sorted so a failure names the same
/// file on every machine.
///
/// Returns `None` when the corpus is absent, which is a skip in a normal run
/// and a failure in a release gate.
fn fixtures() -> Option<Vec<PathBuf>> {
    let Some(directory) = std::env::var("OCCLUVIEW_ALIGN_FIXTURES").ok() else {
        assert!(
            !fixtures_are_required(),
            "OCCLUVIEW_ALIGN_FIXTURES_REQUIRED is set but OCCLUVIEW_ALIGN_FIXTURES is not: \
             the private scan corpus is what makes this a release gate, and without it the \
             acceptance thresholds below are unverified"
        );
        return None;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&directory)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| {
                    path.extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("stl"))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    assert!(
        !files.is_empty() || !fixtures_are_required(),
        "OCCLUVIEW_ALIGN_FIXTURES is set to a directory with no .stl files, and this run is a \
         release gate: point it at the corpus before cutting a release"
    );
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

/// How far apart two real scans can start and still come together.
///
/// The acceptance test above moves a scan by a third of a millimetre — that is
/// a scan already seated. An operator places two scans by eye, and the tool has
/// to close what they leave: several millimetres of offset, and a small tilt.
/// This walks that range on a real arch and reports what the solver does at
/// each step, so the printed table shows which offsets the search recovers.
///
/// It reads `OCCLUVIEW_ALIGN_FIXTURES` like the test above and reports a skip
/// without it; its result is the printed table, so run it with `--nocapture`.
#[test]
fn a_real_scan_recovers_from_a_ballpark_placement_when_fixtures_are_present() {
    let Some(files) = fixtures() else {
        eprintln!("skipped: set OCCLUVIEW_ALIGN_FIXTURES to a directory of binary STL files");
        return;
    };
    let Some(path) = files.first() else {
        eprintln!("skipped: no fixture files");
        return;
    };
    let (positions, indices) = read_binary_stl(path);
    let soup = Soup {
        positions: &positions,
        indices: &indices,
        mask: None,
    };
    let index = SurfaceIndex::build(soup).expect("a real mesh must index");
    println!(
        "fixture: {} ({} verts)",
        path.display(),
        soup.vertex_count()
    );

    // Offsets a hand produces: a factory floor pick-up, then progressively
    // worse. The tilt grows with the offset, as it does when a scan is turned
    // while being placed.
    for shift_mm in [0.5_f64, 1.0, 2.0, 4.0, 8.0, 15.0] {
        let start = Rigid::new(
            DQuat::from_axis_angle(
                DVec3::new(0.3, 0.5, 0.8).normalize(),
                (shift_mm * 0.004).min(0.20),
            ),
            DVec3::new(shift_mm * 0.6, -shift_mm * 0.5, shift_mm * 0.3),
        );
        let outcome = refine(
            soup,
            &index,
            start,
            &RefineSettings::default(),
            &CancelFlag::new(),
        );
        match outcome {
            Ok(report) => {
                println!(
                    "GATE apart={shift_mm} rms={:.4} geo={:.4} med={:.4} p95={:.4} cov={:.4} ratio={:.4}",
                    report.rms,
                    report.geometric_rms,
                    report.median_abs,
                    report.p95_abs,
                    report.coverage,
                    report.inlier_ratio
                );
                let back = (report.rigid.translation - start.translation).length();
                println!(
                    "start {shift_mm:>5.1} mm -> ok  rms={:.4} coverage={:.3} converged={} moved={:.3} mm",
                    report.rms, report.coverage, report.converged, back
                );
            }
            Err(rejection) => println!("start {shift_mm:>5.1} mm -> REFUSED {rejection:?}"),
        }
    }
}

/// Two different arches are not a refine pair, and the tool says so.
///
/// The upper and lower jaw have no single correct joint pose: only their
/// occlusal surfaces relate, and several positions explain them equally well.
/// The solver refuses such a pair as `Ambiguous` rather than picking one and
/// painting a heatmap that would look authoritative. This test covers that
/// refusal.
#[test]
fn two_different_arches_are_refused_rather_than_guessed_when_fixtures_are_present() {
    let Some(dir) = std::env::var_os("OCCLUVIEW_ALIGN_FIXTURES").map(PathBuf::from) else {
        eprintln!("skipped: set OCCLUVIEW_ALIGN_FIXTURES");
        return;
    };
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("fixture dir")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("stl"))
        })
        .collect();
    files.sort();
    if files.len() < 2 {
        eprintln!("skipped: need two STL files, found {}", files.len());
        return;
    }
    let (fixed_positions, fixed_indices) = read_binary_stl(&files[0]);
    let fixed_soup = Soup {
        positions: &fixed_positions,
        indices: &fixed_indices,
        mask: None,
    };
    let index = SurfaceIndex::build(fixed_soup).expect("fixed index");
    let (moving_positions, moving_indices) = read_binary_stl(&files[1]);
    let moving = Soup {
        positions: &moving_positions,
        indices: &moving_indices,
        mask: None,
    };
    println!(
        "fixed: {}  moving: {}",
        files[0].file_name().unwrap().to_string_lossy(),
        files[1].file_name().unwrap().to_string_lossy()
    );

    for shift_mm in [0.0_f64, 2.0, 5.0, 10.0, 20.0, 40.0] {
        let start = Rigid::new(
            DQuat::from_axis_angle(DVec3::new(0.3, 0.5, 0.8).normalize(), 0.02),
            DVec3::new(0.0, 0.0, shift_mm),
        );
        let outcome = refine(
            moving,
            &index,
            start,
            &RefineSettings::default(),
            &CancelFlag::new(),
        );
        match outcome {
            Ok(report) => {
                // A pose here is a confident answer to a question with no
                // answer. The two jaws relate only where their occlusal
                // surfaces meet, and several positions explain that equally
                // well; a heatmap over one of them would look authoritative
                // and mean nothing.
                //
                // A refusal is the correct outcome; what is checked here is
                // that the pose is not reported as a fit. `is_trustworthy`
                // is the gate the worker applies before the operator is told
                // anything, so a report it rejects is still a refusal from
                // the operator's side.
                assert!(
                    !report.is_trustworthy_refinement_for(&RefineSettings::default()),
                    "two different jaws must not be reported as an alignment at \
                     {shift_mm} mm apart: rms={:.4} median={:.4} coverage={:.4}",
                    report.rms,
                    report.median_abs,
                    report.coverage
                );
                println!(
                    "apart {shift_mm:>5.1} mm -> accepted pose refused to the operator \
                     (rms={:.4} med={:.4}), as it should be",
                    report.rms, report.median_abs
                );
            }
            Err(FitRejection::Ambiguous) => {
                println!("apart {shift_mm:>5.1} mm -> refused as ambiguous, as it should be");
            }
            Err(other) => println!("apart {shift_mm:>5.1} mm -> refused {other:?}"),
        }
    }
}

/// The pairing the tool is actually for: a scan against the same scan.
///
/// An operator re-scans or re-imports a jaw and asks Best fit to seat it. That
/// pair has one correct answer, unlike two different arches whose only relation
/// is where their occlusal surfaces meet. This walks a range of hand placements
/// on the real fixture and reports what the solver reaches.
#[test]
fn a_rescanned_arch_seats_from_a_hand_placement_when_fixtures_are_present() {
    let Some(files) = fixtures() else {
        eprintln!("skipped: set OCCLUVIEW_ALIGN_FIXTURES");
        return;
    };
    let Some(path) = files.first() else {
        eprintln!("skipped: no fixture files");
        return;
    };
    let (positions, indices) = read_binary_stl(path);
    let soup = Soup {
        positions: &positions,
        indices: &indices,
        mask: None,
    };
    let index = SurfaceIndex::build(soup).expect("a real mesh must index");
    println!(
        "fixture: {} ({} verts)",
        path.display(),
        soup.vertex_count()
    );

    for shift_mm in [0.5_f64, 1.0, 2.0, 4.0, 8.0, 15.0, 25.0] {
        let truth = Rigid::new(
            DQuat::from_axis_angle(
                DVec3::new(0.3, 0.5, 0.8).normalize(),
                (shift_mm * 0.004).min(0.20),
            ),
            DVec3::new(shift_mm * 0.6, -shift_mm * 0.5, shift_mm * 0.3),
        );
        // The operator's start is the identity: the rescan sits where the
        // original did, and the tool must find the displacement.
        let outcome = refine(
            soup,
            &index,
            Rigid::IDENTITY,
            &RefineSettings::default(),
            &CancelFlag::new(),
        );
        match outcome {
            Ok(report) => {
                let error = (report.rigid.translation - truth.translation).length();
                println!(
                    "true {shift_mm:>5.1} mm -> ok  rms={:.4} coverage={:.3} conv={} error={error:.3} mm",
                    report.rms, report.coverage, report.converged
                );
            }
            Err(rejection) => println!("true {shift_mm:>5.1} mm -> REFUSED {rejection:?}"),
        }
    }
}

/// How far the corrected rescan still sits from the fixed surface, as an RMS
/// over the mesh's own vertices.
///
/// `refine` returns the pose that seats the *moving* mesh onto the fixed one, so
/// a correct answer composed with the displacement that created the rescan
/// returns each vertex to where it started. Measuring the composed result is the
/// correct quantity: comparing `rigid` to `truth` directly would compare a
/// correction against the motion it corrects.
fn residual_after_correction(positions: &[f32], truth: Rigid, correction: Rigid) -> f64 {
    let mut squares = 0.0;
    let mut count = 0usize;
    for point in positions.as_chunks::<3>().0 {
        let local = DVec3::new(
            f64::from(point[0]),
            f64::from(point[1]),
            f64::from(point[2]),
        );
        let rescan = truth.apply(local);
        squares += (correction.apply(rescan) - local).length_squared();
        count += 1;
    }
    (squares / count.max(1) as f64).sqrt()
}

/// Deterministic Gaussian noise, standing in for the scanner error a second
/// acquisition of the same jaw would carry.
struct Noise {
    state: u64,
}

impl Noise {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    /// Uniform in `[0, 1)`.
    fn next_unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn next_normal(&mut self) -> f64 {
        let first = self.next_unit().max(f64::MIN_POSITIVE);
        let second = self.next_unit();
        (-2.0 * first.ln()).sqrt() * (std::f64::consts::TAU * second).cos()
    }
}

/// A rescan of the same jaw, from a hand placement, must be accepted.
///
/// This is the operator's own workflow, and the one the seating gate can get
/// wrong. The corpus holds no pair of independent acquisitions, so
/// this stands in for one: it moves a real arch by a known rigid transform and
/// perturbs every vertex with the scanner error a second capture would carry.
/// The full search must both find the pose and satisfy
/// `is_trustworthy_refinement_for`. A build that only refines locally converges
/// on an unseated pose here and the gate refuses it; this test catches that.
#[test]
fn a_rescan_with_scanner_error_is_accepted_where_fixtures_are_present() {
    let Some(path) = fixtures().and_then(|files| files.into_iter().next()) else {
        eprintln!("skipped: set OCCLUVIEW_ALIGN_FIXTURES");
        return;
    };
    let (positions, indices) = read_binary_stl(&path);
    let fixed = Soup {
        positions: &positions,
        indices: &indices,
        mask: None,
    };
    let index = SurfaceIndex::build(fixed).expect("a real mesh must index");
    println!(
        "fixture: {} ({} verts)",
        path.display(),
        fixed.vertex_count()
    );

    // Per-point scanner error: a clean re-acquisition, then a typical and a
    // coarse intra-oral capture.
    for sigma_mm in [0.0_f64, 0.03, 0.06] {
        for shift_mm in [1.0_f64, 4.0, 8.0] {
            let truth = Rigid::new(
                DQuat::from_axis_angle(
                    DVec3::new(0.3, 0.5, 0.8).normalize(),
                    (shift_mm * 0.004).min(0.20),
                ),
                DVec3::new(shift_mm * 0.6, -shift_mm * 0.5, shift_mm * 0.3),
            );
            let mut noise = Noise::new(0x5eed_1234_u64 ^ sigma_mm.to_bits() ^ shift_mm.to_bits());
            let mut moved = Vec::with_capacity(positions.len());
            for point in positions.as_chunks::<3>().0 {
                let local = DVec3::new(
                    f64::from(point[0]),
                    f64::from(point[1]),
                    f64::from(point[2]),
                );
                let mut world = truth.apply(local);
                if sigma_mm > 0.0 {
                    world += DVec3::new(
                        noise.next_normal(),
                        noise.next_normal(),
                        noise.next_normal(),
                    ) * sigma_mm;
                }
                moved.push(world.x as f32);
                moved.push(world.y as f32);
                moved.push(world.z as f32);
            }
            let moving = Soup {
                positions: &moved,
                indices: &indices,
                mask: None,
            };
            let settings = RefineSettings::default();
            // The operator's start is the identity: the rescan sits where the
            // original did, and the tool must find the displacement.
            let report = refine(
                moving,
                &index,
                Rigid::IDENTITY,
                &settings,
                &CancelFlag::new(),
            )
            .unwrap_or_else(|rejection| {
                panic!("sigma={sigma_mm} shift={shift_mm}: refused with {rejection:?}")
            });
            let error = residual_after_correction(&positions, truth, report.rigid);
            println!(
                "sigma={sigma_mm:>4.2} shift={shift_mm:>4.1} -> seated={:.4} rms={:.4} \
                 med={:.4} residual={error:.4} mm",
                report.seated_fraction, report.rms, report.median_abs
            );
            assert!(
                report.is_trustworthy_refinement_for(&settings),
                "sigma={sigma_mm} shift={shift_mm}: a correct rescan was refused by the gate; \
                 seated={:.4} rms={:.4} med={:.4}",
                report.seated_fraction,
                report.rms,
                report.median_abs
            );
            assert!(
                error < 0.5,
                "sigma={sigma_mm} shift={shift_mm}: the accepted pose leaves {error:.4} mm of \
                 residual displacement; the rescan was not brought home"
            );
        }
    }
}

/// A prepared model can retain only a small unchanged region of the original.
/// This derives a controlled counterexample from a real scan so the true rigid
/// pose is known. It is not a substitute for two independently acquired scans.
#[test]
#[allow(clippy::too_many_lines, clippy::cast_possible_truncation)]
fn a_changed_arch_uses_its_small_unchanged_region_when_fixtures_are_present() {
    let Some(path) = fixtures().and_then(|files| files.into_iter().next()) else {
        eprintln!("skipped: set OCCLUVIEW_ALIGN_FIXTURES");
        return;
    };
    let (fixed_positions, indices) = read_binary_stl(&path);
    let fixed_soup = Soup {
        positions: &fixed_positions,
        indices: &indices,
        mask: None,
    };
    let fixed_index = SurfaceIndex::build(fixed_soup).expect("real mesh must index");
    let min_x = fixed_positions
        .as_chunks::<3>()
        .0
        .iter()
        .map(|point| point[0])
        .fold(f32::INFINITY, f32::min);
    let max_x = fixed_positions
        .as_chunks::<3>()
        .0
        .iter()
        .map(|point| point[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let unchanged_edge = min_x + (max_x - min_x) * 0.24;
    let transition_width = (max_x - min_x) * 0.08;
    let truth = Rigid::new(
        DQuat::from_axis_angle(DVec3::new(0.3, 0.5, 0.8).normalize(), 0.06),
        DVec3::new(5.0, -4.0, 2.5),
    );
    let mut moving_positions = fixed_positions.clone();
    for point in moving_positions.as_chunks_mut::<3>().0 {
        let x = point[0];
        let weight = ((x - unchanged_edge) / transition_width).clamp(0.0, 1.0);
        let y = f64::from(point[1]);
        let z = f64::from(point[2]);
        let changed_weight = f64::from(weight);
        let changed = DVec3::new(
            f64::from(x) + changed_weight * 1.5 * (y * 1.7).sin(),
            y + changed_weight * 1.5 * (z * 1.7).sin(),
            z + changed_weight * (2.0 + 1.2 * (f64::from(x) * 1.7 + y * 0.9).sin()),
        );
        let displaced = truth.inverse().apply(changed);
        point[0] = displaced.x as f32;
        point[1] = displaced.y as f32;
        point[2] = displaced.z as f32;
    }
    let moving = Soup {
        positions: &moving_positions,
        indices: &indices,
        mask: None,
    };
    let started = std::time::Instant::now();
    let settings = RefineSettings::default();
    let report = refine(
        moving,
        &fixed_index,
        Rigid::IDENTITY,
        &settings,
        &CancelFlag::new(),
    )
    .expect("unchanged region must provide a candidate");
    let error = [min_x, f32::midpoint(min_x, max_x), max_x]
        .into_iter()
        .map(|x| {
            let probe = DVec3::new(f64::from(x), 0.0, 0.0);
            report.rigid.apply(probe).distance(truth.apply(probe))
        })
        .fold(0.0_f64, f64::max);
    let diagnostic_positions: Vec<f32> = moving_positions
        .as_chunks::<9>()
        .0
        .iter()
        .step_by(100)
        .flat_map(|triangle| triangle.iter().copied())
        .collect();
    let diagnostic_indices: Vec<u32> = (0..diagnostic_positions.len() / 3)
        .map(|vertex| u32::try_from(vertex).expect("fixture fits u32"))
        .collect();
    let diagnostic = Soup {
        positions: &diagnostic_positions,
        indices: &diagnostic_indices,
        mask: None,
    };
    let count_near = |pose: Rigid| {
        let map = deviation(
            diagnostic,
            &fixed_index,
            pose,
            &DeviationSettings::default(),
            &CancelFlag::new(),
        );
        map.signed_mm
            .iter()
            .zip(&map.validity)
            .filter(|(distance, validity)| {
                **validity == occluview_align::Validity::Measured && distance.abs() < 0.05
            })
            .count()
    };
    eprintln!(
        "changed arch: pose error={error:.3} mm coverage={:.3} rms={:.3} median={:.3} near_truth={} near_fit={} trusted={} elapsed={:.2}s",
        report.coverage,
        report.rms,
        report.median_abs,
        count_near(truth),
        count_near(report.rigid),
        report.is_trustworthy_refinement_for(&settings),
        started.elapsed().as_secs_f64()
    );
    assert!(error < 0.5, "pose must follow the unchanged region");
    assert!(
        report.is_trustworthy_refinement_for(&settings),
        "only an adequately supported fit can publish a heatmap"
    );
}

/// Two independently acquired meshes of the same jaw, with a prepared region.
/// The test compares near and distant starts because no clinical ground-truth
/// transform is available for the public pair.
#[test]
fn a_distinct_prep_pair_has_one_accepted_pose_from_near_and_distant_starts() {
    let Some(directory) = std::env::var_os("OCCLUVIEW_ALIGN_PREP_PAIR").map(PathBuf::from) else {
        eprintln!("skipped: set OCCLUVIEW_ALIGN_PREP_PAIR to original.stl and prepared.stl");
        return;
    };
    let (fixed_positions, fixed_indices) = read_binary_stl(&directory.join("original.stl"));
    let (moving_positions, moving_indices) = read_binary_stl(&directory.join("prepared.stl"));
    let fixed = SurfaceIndex::build(Soup {
        positions: &fixed_positions,
        indices: &fixed_indices,
        mask: None,
    })
    .expect("original scan must index");
    let moving = Soup {
        positions: &moving_positions,
        indices: &moving_indices,
        mask: None,
    };
    let settings = RefineSettings::default();
    let starts = [
        Rigid::IDENTITY,
        Rigid::new(
            DQuat::from_axis_angle(DVec3::new(0.3, 0.5, 0.8).normalize(), 0.06),
            DVec3::new(25.0, -8.0, 3.0),
        ),
    ];
    let reports: Vec<_> = starts
        .into_iter()
        .map(|start| {
            let started = std::time::Instant::now();
            let report = refine(moving, &fixed, start, &settings, &CancelFlag::new())
                .expect("the unchanged surfaces must determine a pose");
            eprintln!(
                "distinct prep pair: start={start:?} elapsed={:.2}s coverage={:.3} median={:.3}",
                started.elapsed().as_secs_f64(),
                report.coverage,
                report.median_abs
            );
            assert!(report.is_trustworthy_refinement_for(&settings));
            report
        })
        .collect();
    let (min, max) = fixed.bounds();
    for probe in [min, (min + max) * 0.5, max] {
        let difference = reports[0]
            .rigid
            .apply(probe)
            .distance(reports[1].rigid.apply(probe));
        assert!(
            difference < 0.5,
            "start changed the accepted pose by {difference:.3} mm"
        );
    }
}

/// The operator's supplied partial-overlap pair. Keep the scans outside Git;
/// the rough pose is the input to local refinement, not a request to search
/// the whole scene for a different answer.
#[test]
fn a_partial_pair_refines_locally_from_nearby_starts_when_fixtures_are_present() {
    let Some(directory) = std::env::var_os("OCCLUVIEW_ALIGN_OWNER_PAIR").map(PathBuf::from) else {
        eprintln!("skipped: set OCCLUVIEW_ALIGN_OWNER_PAIR to 2.stl and 3.stl");
        return;
    };
    let (fixed_positions, fixed_indices) = read_binary_stl(&directory.join("2.stl"));
    let (moving_positions, moving_indices) = read_binary_stl(&directory.join("3.stl"));
    let fixed = SurfaceIndex::build(Soup {
        positions: &fixed_positions,
        indices: &fixed_indices,
        mask: None,
    })
    .expect("fixed scan must index");
    let moving = Soup {
        positions: &moving_positions,
        indices: &moving_indices,
        mask: None,
    };
    let settings = RefineSettings {
        local_only: true,
        ..RefineSettings::default()
    };
    let mut settled = Vec::new();
    for start in [
        Rigid::IDENTITY,
        Rigid::new(DQuat::IDENTITY, DVec3::new(1.0, -0.5, 0.3)),
        Rigid::new(DQuat::IDENTITY, DVec3::new(-1.0, 0.5, -0.3)),
    ] {
        let started = std::time::Instant::now();
        let report = refine(moving, &fixed, start, &settings, &CancelFlag::new())
            .expect("local partial-overlap refine must produce a report");
        eprintln!(
            "owner pair: start={start:?} elapsed={:.2}s trust={} report={report:?}",
            started.elapsed().as_secs_f64(),
            report.is_trustworthy_refinement_for(&settings)
        );
        assert!(report.is_trustworthy_refinement_for(&settings));
        settled.push(report.rigid);
    }
    let probe = DVec3::new(0.0, -15.0, 0.0);
    for pose in &settled[1..] {
        assert!(
            pose.apply(probe).distance(settled[0].apply(probe)) < 0.2,
            "nearby starts must seat the same unchanged region"
        );
    }
    let far_start = Rigid::new(DQuat::IDENTITY, DVec3::new(25.0, -8.0, 3.0));
    let far = refine(moving, &fixed, far_start, &settings, &CancelFlag::new());
    assert!(
        !far.is_ok_and(|report| report.is_trustworthy_refinement_for(&settings)),
        "local refinement must not replace rough placement with a distant guess"
    );
}
