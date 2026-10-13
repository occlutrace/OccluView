//! Whole-scan search on million-triangle synthetic meshes: a correctness
//! regression and a separate opt-in manual benchmark.
//!
//! The correctness case reads a few hundred thousand triangles on their exact
//! surface and more than a million on an even triangle sample, and requires the
//! true pose with `Verified` confidence. It runs under a finite safety
//! allowance far above the production wall limit, so shared-runner speed cannot
//! decide a geometry assertion; elapsed time is not part of that contract.
//!
//! `large_scan_search_benchmark` is the speed measurement. It is ignored by
//! default and fails closed until the operator declares the machine and source
//! identity the numbers are reported against, so a raw timing is never
//! attributed to an undeclared baseline. It asserts no speed budget.
#![allow(
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::print_stdout
)]
mod support;
use glam::{DQuat, DVec3};
use occluview_align::{
    search_alignment, CancelFlag, Completion, Confidence, Rigid, SearchControl, SearchSettings,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use support::{alignment_input, arch, metrics, operators, SyntheticMesh};

/// Corpus sizes for the large-mesh search, each with whether the search reads
/// the exact triangle surface. The exact surface is abandoned once its
/// conservative resident estimate passes the mesh ceiling.
const LARGE_SCAN_CASES: [(usize, bool); 2] = [(300_000, true), (1_200_000, false)];

/// Finite safety allowance for one test search, applied to both the settings
/// and the control; the search uses the lesser of the two.
///
/// This is a safety timeout, not a performance budget or a clinical tolerance.
/// It is far above the production wall limit (10 s) so a loaded shared runner
/// cannot fail a correct search on the clock, while a hang still ends within a
/// known time. The correctness assertions are completion, evidence and
/// geometry, never elapsed time.
const TEST_SEARCH_SAFETY_ALLOWANCE: Duration = Duration::from_secs(600);

/// Warmup searches discarded before each corpus size is measured. The search is
/// deterministic, so warmup absorbs fixture and allocator first-touch cost.
const BENCH_WARMUP_RUNS: usize = 1;
/// Measured repetitions per corpus size; every repetition is printed raw.
const BENCH_REPETITIONS: usize = 5;
/// Machine and source identity the operator must declare before the benchmark
/// reports a number. The run fails closed while either is absent.
const BENCH_CONTEXT_VARS: [&str; 2] = ["OCCLUVIEW_BENCH_MACHINE", "OCCLUVIEW_BENCH_SOURCE"];

/// The truth displacement of the moving scan; search never receives it.
fn truth_pose() -> Rigid {
    Rigid::new(
        DQuat::from_axis_angle(DVec3::new(0.4, 0.1, -0.9).normalize(), 2.6),
        DVec3::new(-17., 9., 12.),
    )
}

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

/// Subdivide `mesh` in place until it holds at least `least` triangles.
fn grow_to(mesh: &mut SyntheticMesh, least: usize) {
    while mesh.triangles.len() / 3 < least {
        let grown = quartered(mesh);
        *mesh = grown;
    }
}

/// Grow `fixed` to the requested corpus size and return the moving copy, its
/// truth, and the independent probes that both runs read quality through.
fn large_scan(least: usize, fixed: &mut SyntheticMesh) -> (SyntheticMesh, Rigid, Vec<DVec3>) {
    grow_to(fixed, least);
    let truth = truth_pose();
    let moving = operators::rigid_offset(fixed, truth);
    let probes = (0..moving.positions.len() / 3)
        .step_by(977)
        .map(|i| moving.point(i))
        .collect();
    (moving, truth, probes)
}

/// Settings carrying the test safety allowance.
fn safety_settings() -> SearchSettings {
    SearchSettings {
        wall_limit: TEST_SEARCH_SAFETY_ALLOWANCE,
        ..SearchSettings::default()
    }
}

/// A fresh control carrying the same test safety allowance.
fn safety_control() -> SearchControl {
    SearchControl::new(CancelFlag::new(), TEST_SEARCH_SAFETY_ALLOWANCE)
}

/// Scans of several hundred thousand triangles are read on their triangles
/// and scans of more than a million on an even cloud of them; either way the
/// search ends within its safety allowance at the true pose and says how it
/// read. Elapsed time is not asserted.
#[test]
fn large_scans_are_searched_whole() {
    let mut fixed = arch::dental_arch(&arch::ArchSpec::default());
    for (least, exact) in LARGE_SCAN_CASES {
        let (moving, truth, probes) = large_scan(least, &mut fixed);
        let result = search_alignment(
            &alignment_input(moving.soup(), fixed.soup()),
            &safety_settings(),
            &safety_control(),
        )
        .unwrap();
        let best = &result.candidates[0];
        let error = metrics::probe_error(best.pose, truth, &probes);
        println!(
            "triangles={} completion={:?} class={:?} reasons={:?} gap={:?} probe_rms_mm={}",
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

/// Content identity of the solver input, independent of any file or case name.
fn corpus_sha256(mesh: &SyntheticMesh) -> String {
    let mut hasher = Sha256::new();
    for position in &mesh.positions {
        hasher.update(position.to_le_bytes());
    }
    for &index in &mesh.triangles {
        hasher.update(index.to_le_bytes());
    }
    hex_digest(&hasher.finalize())
}

/// Lowercase hexadecimal of a digest.
fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(*byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(*byte & 0x0f)]));
    }
    encoded
}

/// Read the declared machine and source identity, failing closed when either is
/// absent so no measurement is attributed to an undeclared baseline.
fn declared_context() -> [String; 2] {
    let mut context = [String::new(), String::new()];
    let mut missing = Vec::new();
    for (index, name) in BENCH_CONTEXT_VARS.iter().enumerate() {
        match std::env::var(name) {
            Ok(value) if !value.trim().is_empty() => context[index] = value.trim().to_owned(),
            _ => missing.push(*name),
        }
    }
    assert!(
        missing.is_empty(),
        "manual benchmark context is not declared: set {} to the machine and source \
         identity this run reports against; refusing to emit an unattributed baseline",
        missing.join(", ")
    );
    context
}

/// Manual, opt-in whole-scan benchmark on the same synthetic corpora as the
/// correctness case.
///
/// Run it deliberately on the target machine, in the profile being reported:
///
/// ```text
/// OCCLUVIEW_BENCH_MACHINE="<host/cpu/ram>" \
/// OCCLUVIEW_BENCH_SOURCE="<git sha>" \
/// cargo test -p occluview-align --release --test search_large \
///     -- --ignored --nocapture large_scan_search_benchmark
/// ```
///
/// The header declares the machine, source, build profile, target, backend and
/// corpus identity; [`BENCH_WARMUP_RUNS`] searches are discarded per corpus and
/// [`BENCH_REPETITIONS`] are printed raw. Each repetition records wall time,
/// work counters and the independent probe quality. No speed budget is
/// asserted: a threshold needs a recorded baseline, which this harness does not
/// claim to have. The run fails closed when the machine or source identity is
/// undeclared.
#[test]
#[ignore = "manual perf benchmark: declare OCCLUVIEW_BENCH_MACHINE and OCCLUVIEW_BENCH_SOURCE, then run with --ignored --nocapture"]
fn large_scan_search_benchmark() {
    let [machine, source_revision] = declared_context();
    let parallelism = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let build_profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let triangle_targets = LARGE_SCAN_CASES.map(|(least, _)| least);
    println!(
        "{}",
        json!({
            "benchmark": "search_large",
            "machine": machine,
            "source_revision": source_revision,
            "build_profile": build_profile,
            "target_arch": std::env::consts::ARCH,
            "target_os": std::env::consts::OS,
            "backend": "cpu-rayon",
            "parallelism": parallelism,
            "corpus": "synthetic-quartered-dental-arch",
            "warmup_runs": BENCH_WARMUP_RUNS,
            "repetitions": BENCH_REPETITIONS,
            "triangle_targets": triangle_targets,
        })
    );
    let mut fixed = arch::dental_arch(&arch::ArchSpec::default());
    for (least, _) in LARGE_SCAN_CASES {
        let (moving, truth, probes) = large_scan(least, &mut fixed);
        let corpus_digest = corpus_sha256(&fixed);
        let input = alignment_input(moving.soup(), fixed.soup());
        for _ in 0..BENCH_WARMUP_RUNS {
            drop(search_alignment(&input, &safety_settings(), &safety_control()).unwrap());
        }
        for repetition in 0..BENCH_REPETITIONS {
            let started = Instant::now();
            let result = search_alignment(&input, &safety_settings(), &safety_control()).unwrap();
            let wall = started.elapsed();
            let best = &result.candidates[0];
            let error = metrics::probe_error(best.pose, truth, &probes);
            println!(
                "{}",
                json!({
                    "benchmark": "search_large",
                    "corpus": "synthetic-quartered-dental-arch",
                    "corpus_sha256": corpus_digest,
                    "triangles": fixed.triangles.len() / 3,
                    "vertices": fixed.positions.len() / 3,
                    "repetition": repetition,
                    "wall_ms": wall.as_secs_f64() * 1e3,
                    "work": {
                        "elapsed_ms": result.work.elapsed.as_secs_f64() * 1e3,
                        "examined_poses": result.work.examined_poses,
                        "retained_poses": result.work.retained_poses,
                        "iterations": result.work.iterations,
                        "families": result.work.families.iter().map(|family| json!({
                            "origin": format!("{:?}", family.family),
                            "examined": family.examined,
                            "retained": family.retained,
                        })).collect::<Vec<_>>(),
                    },
                    "quality": {
                        "completion": format!("{:?}", result.completion),
                        "confidence": format!("{:?}", best.confidence),
                        "original_surface_exact": best.evidence.original_surface_exact,
                        "probe_rms_mm": error[0],
                        "probe_max_mm": error[1],
                        "rival_gap": format!("{:?}", best.evidence.rival_gap),
                    },
                })
            );
        }
    }
}
