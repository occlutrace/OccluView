//! Whole-scan search on million-triangle synthetic meshes: a correctness
//! regression and a separate opt-in manual benchmark.
//!
//! The correctness case reads a few hundred thousand triangles on their exact
//! surface and more than a million on an even triangle sample, and requires the
//! true pose with `Verified` confidence. It runs under a finite safety
//! allowance far above the production wall limit. The search checks that
//! allowance cooperatively at its own checkpoints, so a stage that does not
//! reach a checkpoint can still run past it; the allowance lowers the coupling
//! between runner speed and the geometry assertion without guaranteeing a
//! completion time. Elapsed time is not part of the correctness contract.
//!
//! `large_scan_search_benchmark` is the speed measurement. It is ignored by
//! default and fails closed until the operator declares the machine, build and
//! source identity the numbers are reported against, so a raw timing is never
//! attributed to an undeclared baseline. It asserts no speed budget.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::print_stdout
)]
mod support;
use glam::{DQuat, DVec3};
use occluview_align::{
    search_alignment, AlignmentInput, CancelFlag, Completion, Confidence, MeshInput, Rigid,
    SearchControl, SearchSettings,
};
use serde::Deserialize;
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
/// It is far above the production wall limit (10 s), so a slow shared runner is
/// less likely to fail a correct search on the clock. The deadline is checked
/// cooperatively, so it is not a hard bound on a stage that does not reach a
/// checkpoint; work can exceed it before the next check. It is finite rather
/// than infinite.
const TEST_SEARCH_SAFETY_ALLOWANCE: Duration = Duration::from_secs(600);

/// Warmup searches discarded before each corpus size is measured. The search is
/// deterministic, so warmup absorbs fixture and allocator first-touch cost.
const BENCH_WARMUP_RUNS: usize = 1;
/// Measured repetitions per corpus size; every repetition is printed raw.
const BENCH_REPETITIONS: usize = 5;
/// The declared benchmark context; see [`large_scan_search_benchmark`] for the
/// required JSON shape. The run fails closed while it is absent or malformed.
const BENCH_CONTEXT_VAR: &str = "OCCLUVIEW_BENCH_CONTEXT";

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

/// Length-prefixed update, so adjacent fields cannot be re-segmented into the
/// same digest.
fn update_len(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_be_bytes());
}

/// Tagged, length-prefixed bytes.
fn update_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    update_len(hasher, bytes.len() as u64);
    hasher.update(bytes);
}

/// Length-prefixed `f64` run.
fn update_f64(hasher: &mut Sha256, values: &[f64]) {
    update_len(hasher, values.len() as u64);
    for value in values {
        hasher.update(value.to_le_bytes());
    }
}

/// One mesh field of the solver request: authored pose, revision, mask, and the
/// actual positions and indices the search reads.
fn update_mesh(hasher: &mut Sha256, tag: &[u8], mesh: &MeshInput<'_>) {
    update_bytes(hasher, tag);
    update_f64(hasher, &mesh.world_from_local.to_cols_array());
    update_len(hasher, mesh.revision);
    match mesh.soup.mask {
        Some(mask) => {
            hasher.update([1_u8]);
            update_bytes(hasher, mask);
        }
        None => hasher.update([0_u8]),
    }
    update_len(hasher, mesh.soup.positions.len() as u64);
    for position in mesh.soup.positions {
        hasher.update(position.to_le_bytes());
    }
    update_len(hasher, mesh.soup.indices.len() as u64);
    for &index in mesh.soup.indices {
        hasher.update(index.to_le_bytes());
    }
}

/// Content identity of the complete solver request: both meshes with their
/// authored poses, revisions and masks, plus landmarks and seeds. It is
/// independent of any file or case name.
fn request_sha256(input: &AlignmentInput<'_>) -> String {
    let mut hasher = Sha256::new();
    update_mesh(&mut hasher, b"moving", &input.moving);
    update_mesh(&mut hasher, b"fixed", &input.fixed);
    update_len(&mut hasher, input.landmarks.len() as u64);
    for pair in input.landmarks {
        update_f64(&mut hasher, &pair.moving_local.to_array());
        update_f64(&mut hasher, &pair.fixed_local.to_array());
        match pair.normals_local {
            Some(normals) => {
                hasher.update([1_u8]);
                update_f64(&mut hasher, &normals[0].to_array());
                update_f64(&mut hasher, &normals[1].to_array());
            }
            None => hasher.update([0_u8]),
        }
    }
    update_len(&mut hasher, input.seeds.len() as u64);
    for seed in input.seeds {
        update_f64(&mut hasher, &seed.rotation.to_array());
        update_f64(&mut hasher, &seed.translation.to_array());
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

/// Caller-declared host identity. CPU and RAM are operator-declared rather than
/// probed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BenchMachine {
    cpu: String,
    ram_bytes: u64,
}

/// Caller-declared build identity: the profile actually built, the toolchain,
/// and the enabled features. `debug_assertions` is a separate derived fact.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BenchBuild {
    profile: String,
    toolchain: String,
    features: Vec<String>,
}

/// Caller-declared target; checked against the actual compile target.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BenchTarget {
    arch: String,
    os: String,
}

/// The complete declared context; see [`large_scan_search_benchmark`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BenchContext {
    machine: BenchMachine,
    build: BenchBuild,
    source_sha: String,
    target: BenchTarget,
}

/// A full lowercase 40-character git revision, which rejects placeholders and
/// abbreviated revisions. The process cannot re-derive git identity.
fn is_full_source_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Read and validate the declared benchmark context, failing closed with a
/// named reason before any fixture or measurement when a field is missing,
/// malformed, or contradicts the actual compile target.
fn read_context() -> BenchContext {
    let raw = std::env::var(BENCH_CONTEXT_VAR).unwrap_or_default();
    assert!(
        !raw.trim().is_empty(),
        "manual benchmark context is not declared: set {BENCH_CONTEXT_VAR} to the JSON \
         object documented on large_scan_search_benchmark"
    );
    let context: BenchContext = serde_json::from_str(&raw)
        .expect("manual benchmark context is not valid JSON for the documented shape");
    assert!(
        !context.machine.cpu.trim().is_empty(),
        "manual benchmark context.machine.cpu is empty"
    );
    assert!(
        context.machine.ram_bytes > 0,
        "manual benchmark context.machine.ram_bytes must be positive"
    );
    assert!(
        !context.build.profile.trim().is_empty(),
        "manual benchmark context.build.profile is empty"
    );
    assert!(
        !context.build.toolchain.trim().is_empty(),
        "manual benchmark context.build.toolchain is empty"
    );
    assert!(
        is_full_source_sha(&context.source_sha),
        "manual benchmark context.source_sha must be a full lowercase 40-character revision"
    );
    assert_eq!(
        context.target.arch,
        std::env::consts::ARCH,
        "manual benchmark context.target.arch does not match the actual compile target"
    );
    assert_eq!(
        context.target.os,
        std::env::consts::OS,
        "manual benchmark context.target.os does not match the actual compile target"
    );
    context
}

/// Manual, opt-in whole-scan benchmark on the same synthetic corpora as the
/// correctness case.
///
/// Run it deliberately on the target machine, in the profile being reported:
///
/// ```text
/// OCCLUVIEW_BENCH_CONTEXT='{"machine":{"cpu":"AMD Ryzen 9 5950X","ram_bytes":17179869184},"build":{"profile":"release","toolchain":"1.98.0","features":[]},"source_sha":"0123456789abcdef0123456789abcdef01234567","target":{"arch":"x86_64","os":"linux"}}' \
/// cargo test -p occluview-align --release --test search_large \
///     -- --ignored --nocapture large_scan_search_benchmark
/// ```
///
/// The context is caller-declared and reported under `declared`: the machine
/// CPU and RAM, the build profile, toolchain and features, the source SHA, and
/// the target arch/os. The run fails closed before building any fixture when a
/// field is missing, the source SHA is not a full lowercase 40-character
/// revision, or the target arch/os disagree with the actual compile target.
/// `debug_assertions`, the actual target, the backend, the available
/// parallelism and the actual Rayon pool size are reported separately under
/// `derived`; `debug_assertions` is not presented as the build profile.
///
/// The header also declares the corpus, the fixed [`BENCH_WARMUP_RUNS`] warmup
/// and [`BENCH_REPETITIONS`] repetitions. Each repetition records raw wall time,
/// work counters, the effective numerical settings and wall allowance, the
/// algorithm version, threshold set and input revisions, and the independent
/// probe quality. No speed budget is asserted: a threshold needs a recorded
/// baseline, which this harness does not claim to have.
#[test]
#[ignore = "manual perf benchmark: set OCCLUVIEW_BENCH_CONTEXT, then run with --ignored --nocapture"]
fn large_scan_search_benchmark() {
    let context = read_context();
    let available_parallelism =
        std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let rayon_current_num_threads = rayon::current_num_threads();
    let triangle_targets = LARGE_SCAN_CASES.map(|(least, _)| least);
    println!(
        "{}",
        json!({
            "benchmark": "search_large",
            "declared": {
                "machine": {
                    "cpu": context.machine.cpu,
                    "ram_bytes": context.machine.ram_bytes,
                },
                "build": {
                    "profile": context.build.profile,
                    "toolchain": context.build.toolchain,
                    "features": context.build.features,
                },
                "source_sha": context.source_sha,
                "target": {
                    "arch": context.target.arch,
                    "os": context.target.os,
                },
            },
            "derived": {
                "debug_assertions": cfg!(debug_assertions),
                "target_arch": std::env::consts::ARCH,
                "target_os": std::env::consts::OS,
                "backend": "cpu-rayon",
                "available_parallelism": available_parallelism,
                "rayon_current_num_threads": rayon_current_num_threads,
            },
            "method": {
                "corpus": "synthetic-quartered-dental-arch",
                "warmup_runs": BENCH_WARMUP_RUNS,
                "repetitions": BENCH_REPETITIONS,
                "triangle_targets": triangle_targets,
            },
        })
    );
    let mut fixed = arch::dental_arch(&arch::ArchSpec::default());
    for (least, _) in LARGE_SCAN_CASES {
        let (moving, truth, probes) = large_scan(least, &mut fixed);
        let input = alignment_input(moving.soup(), fixed.soup());
        let request_digest = request_sha256(&input);
        for _ in 0..BENCH_WARMUP_RUNS {
            drop(search_alignment(&input, &safety_settings(), &safety_control()).unwrap());
        }
        for repetition in 0..BENCH_REPETITIONS {
            let started = Instant::now();
            let result = search_alignment(&input, &safety_settings(), &safety_control()).unwrap();
            let wall = started.elapsed();
            let best = &result.candidates[0];
            let error = metrics::probe_error(best.pose, truth, &probes);
            let settings = &result.provenance.settings;
            println!(
                "{}",
                json!({
                    "benchmark": "search_large",
                    "corpus": "synthetic-quartered-dental-arch",
                    "request_sha256": request_digest,
                    "repetition": repetition,
                    "triangles": {
                        "moving": moving.triangles.len() / 3,
                        "fixed": fixed.triangles.len() / 3,
                    },
                    "vertices": {
                        "moving": moving.positions.len() / 3,
                        "fixed": fixed.positions.len() / 3,
                    },
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
                    "settings": {
                        "normal_policy": format!("{:?}", settings.normal_policy),
                        "profile": format!("{:?}", settings.profile),
                        "top_k": settings.top_k,
                        "reference_regions": format!("{:?}", settings.reference_regions),
                        "wall_limit_ms": settings.wall_limit.as_secs_f64() * 1e3,
                    },
                    "provenance": {
                        "threshold_set_id": result.provenance.threshold_set_id,
                        "algorithm_version": result.provenance.algorithm_version,
                        "input_revisions": result.provenance.input_revisions,
                        "masked": result.provenance.masked,
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
