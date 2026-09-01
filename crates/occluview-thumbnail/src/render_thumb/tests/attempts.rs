//! Thumbnail attempt classification and retry behaviour.

use super::*;
use crate::placeholder::PlaceholderKind;
use std::thread;

fn spec_64() -> ThumbnailSpec {
    ThumbnailSpec {
        size_px: 64,
        ..Default::default()
    }
}

fn expired_request_start() -> Instant {
    Instant::now()
        .checked_sub(Duration::from_millis(10))
        .expect("the process lifetime exceeds ten milliseconds")
}

#[test]
fn shell_request_anchors_response_and_cache_warm_deadlines_at_entry() {
    let started_at = Instant::now();
    let request = ThumbnailRenderRequest::from_started_at(
        started_at,
        Duration::from_millis(20),
        AdapterPolicy::HardwareThenFallback,
    );

    assert_eq!(
        request.response_deadline(),
        started_at + Duration::from_millis(20),
        "queueing, decoding, adapter creation, and readback must share the Shell response deadline"
    );
    assert_eq!(
        request.cache_warm_deadline(),
        started_at + DEFAULT_THUMBNAIL_TIMEOUT,
        "a detached cache warmer is bounded from request entry, not from when rendering starts"
    );
    assert_eq!(
        request.adapter_policy(),
        AdapterPolicy::HardwareThenFallback
    );
}

#[test]
fn expired_shell_request_never_starts_a_fresh_file_render_budget() {
    let spec = spec_64();
    let mut bytes = fixtures::binary_stl_cube();
    bytes[..8].copy_from_slice(b"deadline");
    let path = write_verdict_fixture("deadline-file-request.stl", &bytes);
    let started_at = expired_request_start();
    let request = ThumbnailRenderRequest::from_started_at(
        started_at,
        Duration::from_millis(1),
        AdapterPolicy::HardwareThenFallback,
    );

    let attempt = try_render_thumbnail_file_with_request(&path, spec, request);

    assert_eq!(attempt, ThumbnailAttempt::TransientFailure);
    let _ = fs::remove_file(path);
}

#[test]
fn expired_shell_stream_request_cannot_reserve_a_new_lane() {
    let request = ThumbnailRenderRequest::from_started_at(
        expired_request_start(),
        Duration::from_millis(1),
        AdapterPolicy::HardwareThenFallback,
    );

    assert!(reserve_thumbnail_stream_job_for_request(request).is_none());
}

#[test]
fn expired_shell_stream_request_never_starts_a_fresh_render_budget() {
    let mut bytes = fixtures::binary_stl_cube();
    bytes[..8].copy_from_slice(b"deadline");
    let request = ThumbnailRenderRequest::from_started_at(
        expired_request_start(),
        Duration::from_millis(1),
        AdapterPolicy::HardwareThenFallback,
    );

    let attempt = try_render_thumbnail_shared_with_request(
        Some("stl".to_string()),
        Arc::<[u8]>::from(bytes),
        spec_64(),
        request,
    );

    assert_eq!(attempt, ThumbnailAttempt::TransientFailure);
}

#[test]
fn missing_file_is_a_transient_failure_not_a_cacheable_placeholder() {
    let attempt = try_render_thumbnail_file(
        Path::new("/nonexistent/occluview-transient-probe.stl"),
        spec_64(),
        Duration::from_secs(2),
    );
    assert_eq!(attempt, ThumbnailAttempt::TransientFailure);
}

#[test]
fn unsupported_extension_is_a_deterministic_placeholder_verdict() {
    let path = fixtures::write_temp_fixture("verdict-unsupported.xyz", b"not ours");
    let attempt = try_render_thumbnail_file(&path, spec_64(), Duration::from_secs(2));
    assert_eq!(
        attempt,
        ThumbnailAttempt::Bitmap(placeholder_thumbnail(spec_64())),
        "an extension we never registered reproduces on every retry; cache it"
    );
    let _ = fs::remove_file(path);
}

#[test]
fn corrupt_recognized_file_is_a_deterministic_placeholder_verdict() {
    let mut bytes = vec![0u8; 84];
    bytes[80..84].copy_from_slice(&1000u32.to_le_bytes());
    let path = fixtures::write_temp_fixture("verdict-corrupt.stl", &bytes);
    let attempt = try_render_thumbnail_file(&path, spec_64(), Duration::from_secs(10));
    assert_eq!(
        attempt,
        ThumbnailAttempt::Bitmap(placeholder_thumbnail_kind(
            spec_64(),
            PlaceholderKind::Corrupt
        )),
    );
    let _ = fs::remove_file(path);
}

#[test]
fn unrecognized_stream_bytes_are_a_deterministic_placeholder_verdict() {
    let attempt = try_render_thumbnail_shared(
        None,
        Arc::<[u8]>::from(&b"\x00\x01\x02\x03 nothing recognizable"[..]),
        spec_64(),
        Duration::from_secs(2),
    );
    assert_eq!(
        attempt,
        ThumbnailAttempt::Bitmap(placeholder_thumbnail(spec_64())),
    );
}

#[test]
fn io_error_mid_decode_is_transient_not_a_cacheable_placeholder() {
    let outcome = ThumbnailJobOutcome::Finished(Err(ThumbnailError::Format(FormatError::Io(
        std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    ))));
    let attempt =
        thumbnail_attempt_for_job_outcome(outcome, spec_64(), Duration::from_secs(1), "file");
    assert_eq!(attempt, ThumbnailAttempt::TransientFailure);
}

#[test]
fn contended_deadline_is_transient_for_the_shell_but_placeholder_for_the_cli() {
    let spec = spec_64();
    let shell_bytes = fixtures::large_binary_stl_tessellated_plane(4 * 1024 * 1024);
    let cli_bytes = fixtures::large_binary_stl_tessellated_plane(5 * 1024 * 1024);
    let shell_path = write_verdict_fixture("verdict-contended-shell.stl", &shell_bytes);
    let cli_path = write_verdict_fixture("verdict-contended-cli.stl", &cli_bytes);

    let shell = try_render_thumbnail_file(&shell_path, spec, Duration::ZERO);
    assert_eq!(shell, ThumbnailAttempt::TransientFailure);

    let cli = render_thumbnail_file_or_placeholder_with_timeout(&cli_path, spec, Duration::ZERO);
    assert_eq!(cli, placeholder_thumbnail(spec));

    let _ = fs::remove_file(shell_path);
    let _ = fs::remove_file(cli_path);
}

#[test]
fn transient_failure_still_heals_from_the_background_render_on_retry() {
    let spec = spec_64();
    let bytes = fixtures::large_binary_stl_tessellated_plane(4 * 1024 * 1024);
    let path = write_verdict_fixture("verdict-heals.stl", &bytes);

    let mut healed = None;
    'attempts: for _ in 0..40 {
        let _early = try_render_thumbnail_file(&path, spec, Duration::from_millis(20));
        for _ in 0..20 {
            if let ThumbnailAttempt::Bitmap(pixels) =
                try_render_thumbnail_file(&path, spec, Duration::from_millis(200))
            {
                healed = Some(pixels);
                break 'attempts;
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    let healed = healed.expect("a retried request must eventually serve the cached render");
    assert_ne!(
        healed,
        placeholder_thumbnail(spec),
        "the healed bitmap must be the real render, not a placeholder"
    );
    let _ = fs::remove_file(path);
}

#[test]
fn inflight_followers_inherit_the_leaders_transient_failure() {
    let bytes = b"v 0 0 0\nv 2 0 0\nv 0 2 0\nf 1 2 3\n";
    let key = ThumbnailRequestKey::Stream {
        cache_key: cache::ThumbnailStreamCacheKey::new(occluview_formats::FormatKind::Obj, bytes),
        size_px: 61,
        background: [0; 4],
    };
    let barrier = Arc::new(std::sync::Barrier::new(2));

    let leader_key = key.clone();
    let leader_barrier = barrier.clone();
    let leader = thread::spawn(move || {
        render_coalesced_thumbnail(leader_key, Duration::from_millis(250), move || {
            leader_barrier.wait();
            thread::sleep(Duration::from_millis(40));
            ThumbnailAttempt::TransientFailure
        })
    });

    barrier.wait();
    let follower = render_coalesced_thumbnail(key, Duration::from_millis(500), || {
        ThumbnailAttempt::Bitmap(vec![7, 7, 7, 7])
    });

    assert_eq!(
        leader.join().expect("leader should complete"),
        ThumbnailAttempt::TransientFailure
    );
    assert_eq!(follower, ThumbnailAttempt::TransientFailure);
}

fn write_verdict_fixture(name: &str, bytes: &[u8]) -> PathBuf {
    fixtures::write_temp_fixture(name, bytes)
}

/// One request in the mixed-format burst.
type BurstRequest = (
    &'static str,
    Box<dyn FnOnce() -> ThumbnailAttempt + Send>,
    BurstExpectation,
);

enum BurstExpectation {
    RealBitmap,
    ExactPlaceholder(Vec<u8>),
}

/// Mixed file- and stream-backed requests resolve concurrently.
#[test]
fn mixed_format_burst_resolves_every_verdict_concurrently() {
    let spec = ThumbnailSpec {
        size_px: 48,
        ..Default::default()
    };
    let stl_path = write_verdict_fixture("burst-real.stl", &fixtures::binary_stl_cube());
    let obj_path = write_verdict_fixture("burst-real.obj", fixtures::colored_obj_cube().as_bytes());
    let corrupt_path = {
        let mut bytes = vec![0u8; 84];
        bytes[80..84].copy_from_slice(&5000u32.to_le_bytes());
        write_verdict_fixture("burst-corrupt.stl", &bytes)
    };

    let mut requests = Vec::new();
    for _round in 0..2 {
        requests.extend(burst_round(spec, &stl_path, &obj_path, &corrupt_path));
    }

    let handles: Vec<_> = requests
        .into_iter()
        .map(|(label, request, expect)| (label, thread::spawn(request), expect))
        .collect();
    for (label, handle, expect) in handles {
        let attempt = handle.join().expect("burst request panicked");
        assert_burst_verdict(spec, label, attempt, &expect);
    }

    for path in [stl_path, obj_path, corrupt_path] {
        let _ = fs::remove_file(path);
    }
}

fn burst_round(
    spec: ThumbnailSpec,
    stl_path: &Path,
    obj_path: &Path,
    corrupt_path: &Path,
) -> Vec<BurstRequest> {
    let budget = Duration::from_secs(30);
    let stl = stl_path.to_path_buf();
    let obj = obj_path.to_path_buf();
    let corrupt = corrupt_path.to_path_buf();
    let ply_stream = Arc::<[u8]>::from(fixtures::colored_ply_cube());
    let stl_stream = Arc::<[u8]>::from(fixtures::binary_stl_cube());
    let hps_stream =
        Arc::<[u8]>::from(fixtures::hps_zip_triangle().expect("HPS ZIP fixture should build"));
    let noise_stream = Arc::<[u8]>::from(&b"\x07\x03garbage that matches no reader"[..]);

    vec![
        (
            "stl file",
            Box::new(move || try_render_thumbnail_file(&stl, spec, budget)),
            BurstExpectation::RealBitmap,
        ),
        (
            "obj file",
            Box::new(move || try_render_thumbnail_file(&obj, spec, budget)),
            BurstExpectation::RealBitmap,
        ),
        (
            "ply stream",
            Box::new(move || {
                try_render_thumbnail_shared(Some("ply".to_string()), ply_stream, spec, budget)
            }),
            BurstExpectation::RealBitmap,
        ),
        (
            "extensionless stl stream",
            Box::new(move || try_render_thumbnail_shared(None, stl_stream, spec, budget)),
            BurstExpectation::RealBitmap,
        ),
        (
            "hps zip stream",
            Box::new(move || {
                try_render_thumbnail_shared(Some("dcm".to_string()), hps_stream, spec, budget)
            }),
            BurstExpectation::RealBitmap,
        ),
        (
            "corrupt stl file",
            Box::new(move || try_render_thumbnail_file(&corrupt, spec, budget)),
            BurstExpectation::ExactPlaceholder(placeholder_thumbnail_kind(
                spec,
                PlaceholderKind::Corrupt,
            )),
        ),
        (
            "unrecognized stream",
            Box::new(move || try_render_thumbnail_shared(None, noise_stream, spec, budget)),
            BurstExpectation::ExactPlaceholder(placeholder_thumbnail(spec)),
        ),
    ]
}

fn assert_burst_verdict(
    spec: ThumbnailSpec,
    label: &str,
    attempt: ThumbnailAttempt,
    expect: &BurstExpectation,
) {
    assert_ne!(
        attempt,
        ThumbnailAttempt::TransientFailure,
        "{label} reported a transient failure inside a generous budget"
    );
    let ThumbnailAttempt::Bitmap(pixels) = attempt else {
        return;
    };
    match expect {
        BurstExpectation::RealBitmap => {
            assert_ne!(
                pixels,
                placeholder_thumbnail(spec),
                "{label} must render real geometry, not the plain placeholder"
            );
            assert_ne!(
                pixels,
                placeholder_thumbnail_kind(spec, PlaceholderKind::Corrupt),
                "{label} must render real geometry, not the corrupt placeholder"
            );
            assert!(
                pixels.as_chunks::<4>().0.iter().any(|px| px[3] > 0),
                "{label} rendered a fully transparent tile"
            );
        }
        BurstExpectation::ExactPlaceholder(expected) => {
            assert_eq!(&pixels, expected, "{label} verdict drifted");
        }
    }
}

/// An encrypted package with no key is a fact about the process, not the file.
///
/// The shell keeps whatever bitmap it is handed against the file's timestamp,
/// and a key appearing -- an official build replacing a build from source, an
/// environment variable set for the session -- does not move that timestamp.
/// A cacheable placeholder would therefore outlive the reason for it, and
/// every encrypted scan on the machine would keep the grey cube.
#[test]
fn a_package_that_needs_a_key_this_process_lacks_is_transient() {
    let spec = ThumbnailSpec {
        size_px: 32,
        ..Default::default()
    };
    let outcome =
        ThumbnailJobOutcome::Finished(Err(ThumbnailError::Format(FormatError::Deferred {
            format: "HPS",
            reason: "the package is encrypted and no decryption key is configured".to_string(),
        })));
    let attempt = thumbnail_attempt_for_job_outcome(outcome, spec, Duration::from_secs(6), "file");
    assert!(
        matches!(attempt, ThumbnailAttempt::TransientFailure),
        "a missing key must be asked about again, not cached as a verdict"
    );
}
