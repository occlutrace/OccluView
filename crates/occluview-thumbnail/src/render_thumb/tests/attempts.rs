//! Verdict-split regression tests: transient failures must surface as
//! [`ThumbnailAttempt::TransientFailure`] while deterministic verdicts stay
//! bitmaps.
//!
//! Explorer's thumbcache permanently stores any bitmap a provider returns
//! with `S_OK`, keyed only by the file's modification time. These tests pin
//! the boundary that keeps "busy right now" out of that cache while broken
//! files still get their cacheable placeholder — the difference between a
//! folder that heals on the next browse and one stuck on placeholder cubes
//! until every file is touched.

use super::*;
use std::thread;

fn spec_64() -> ThumbnailSpec {
    ThumbnailSpec {
        size_px: 64,
        ..Default::default()
    }
}

#[test]
fn missing_file_is_a_transient_failure_not_a_cacheable_placeholder() {
    // A path that cannot be stat-ed (vanished mid-browse, or locked by the
    // scanner still writing it on Windows) says nothing about the content.
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
    // A truncated binary STL is broken the same way on every attempt.
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
fn contended_deadline_is_transient_for_the_shell_but_placeholder_for_the_cli() {
    // Same pipeline, two callers: under a hopeless deadline the try_* entry
    // point must report the failure (the shell answers Explorer with an error
    // HRESULT so the item stays re-extractable), while the *_or_placeholder
    // wrapper keeps the freedesktop thumbnailer contract of always producing
    // pixels.
    let spec = spec_64();
    // Distinct content per caller: identical bytes would share one content
    // cache key, and the shell attempt's background worker could then hand the
    // CLI attempt a real cached bitmap instead of exercising its fallback.
    let shell_bytes = fixtures::large_binary_stl_tessellated_plane(4 * 1024 * 1024);
    let cli_bytes = fixtures::large_binary_stl_tessellated_plane(5 * 1024 * 1024);
    let shell_path = write_verdict_fixture("verdict-contended-shell.stl", &shell_bytes);
    let cli_path = write_verdict_fixture("verdict-contended-cli.stl", &cli_bytes);

    // A zero deadline cannot even wait for a job slot, so the outcome is a
    // deterministic SetupTimedOut regardless of what the shared gate is doing.
    let shell = try_render_thumbnail_file(&shell_path, spec, Duration::ZERO);
    assert_eq!(shell, ThumbnailAttempt::TransientFailure);

    let cli = render_thumbnail_file_or_placeholder_with_timeout(&cli_path, spec, Duration::ZERO);
    assert_eq!(cli, placeholder_thumbnail(spec));

    let _ = fs::remove_file(shell_path);
    let _ = fs::remove_file(cli_path);
}

#[test]
fn transient_failure_still_heals_from_the_background_render_on_retry() {
    // The core of the retry story: attempt one fails transiently (budget too
    // small), but its background worker finishes and caches; a later attempt
    // for the same content must come back as a real bitmap, not fail forever.
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
    // A leader that could not produce a verdict must not let its coalesced
    // followers fabricate one: a follower turning "leader timed out" into a
    // bitmap would hand Explorer exactly the cacheable stand-in the
    // transient/deterministic split exists to keep out of thumbcache.
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

/// The Explorer failure scenario, reproduced at the verdict level: one folder,
/// several files, several formats, all extracted concurrently through both the
/// file- and stream-backed entry points. Every healthy source must resolve to
/// a real bitmap, every broken source to its deterministic placeholder — and
/// no request may deadlock, panic, or launder a transient miss into pixels.
#[test]
fn mixed_format_burst_resolves_every_verdict_concurrently() {
    let spec = ThumbnailSpec {
        size_px: 48,
        ..Default::default()
    };
    let budget = Duration::from_secs(30);

    let stl_path = write_verdict_fixture("burst-real.stl", &fixtures::binary_stl_cube());
    let obj_path = write_verdict_fixture("burst-real.obj", fixtures::colored_obj_cube().as_bytes());
    let corrupt_path = {
        let mut bytes = vec![0u8; 84];
        bytes[80..84].copy_from_slice(&5000u32.to_le_bytes());
        write_verdict_fixture("burst-corrupt.stl", &bytes)
    };
    let hps_zip = fixtures::hps_zip_triangle().expect("HPS ZIP fixture should build");

    enum Expect {
        RealBitmap,
        ExactPlaceholder(Vec<u8>),
    }
    let corrupt_placeholder = placeholder_thumbnail_kind(spec, PlaceholderKind::Corrupt);
    let plain_placeholder = placeholder_thumbnail(spec);

    let mut requests: Vec<(
        &'static str,
        Box<dyn FnOnce() -> ThumbnailAttempt + Send>,
        Expect,
    )> = Vec::new();
    for round in 0..2 {
        let stl = stl_path.clone();
        let obj = obj_path.clone();
        let corrupt = corrupt_path.clone();
        let ply_stream = Arc::<[u8]>::from(fixtures::colored_ply_cube());
        let stl_stream = Arc::<[u8]>::from(fixtures::binary_stl_cube());
        let hps_stream = Arc::<[u8]>::from(hps_zip.clone());
        let noise_stream = Arc::<[u8]>::from(&b"\x07\x03garbage that matches no reader"[..]);
        let _ = round;
        requests.push((
            "stl file",
            Box::new(move || try_render_thumbnail_file(&stl, spec, budget)),
            Expect::RealBitmap,
        ));
        requests.push((
            "obj file",
            Box::new(move || try_render_thumbnail_file(&obj, spec, budget)),
            Expect::RealBitmap,
        ));
        requests.push((
            "ply stream",
            Box::new(move || {
                try_render_thumbnail_shared(Some("ply".to_string()), ply_stream, spec, budget)
            }),
            Expect::RealBitmap,
        ));
        requests.push((
            "extensionless stl stream",
            Box::new(move || try_render_thumbnail_shared(None, stl_stream, spec, budget)),
            Expect::RealBitmap,
        ));
        requests.push((
            "hps zip stream",
            Box::new(move || {
                try_render_thumbnail_shared(Some("dcm".to_string()), hps_stream, spec, budget)
            }),
            Expect::RealBitmap,
        ));
        requests.push((
            "corrupt stl file",
            Box::new(move || try_render_thumbnail_file(&corrupt, spec, budget)),
            Expect::ExactPlaceholder(corrupt_placeholder.clone()),
        ));
        requests.push((
            "unrecognized stream",
            Box::new(move || try_render_thumbnail_shared(None, noise_stream, spec, budget)),
            Expect::ExactPlaceholder(plain_placeholder.clone()),
        ));
    }

    let handles: Vec<_> = requests
        .into_iter()
        .map(|(label, request, expect)| (label, thread::spawn(request), expect))
        .collect();
    for (label, handle, expect) in handles {
        let attempt = handle
            .join()
            .unwrap_or_else(|_| panic!("burst request panicked: {label}"));
        let ThumbnailAttempt::Bitmap(pixels) = attempt else {
            panic!("{label} reported a transient failure inside a generous budget");
        };
        match expect {
            Expect::RealBitmap => {
                assert_ne!(
                    pixels, plain_placeholder,
                    "{label} must render real geometry, not the plain placeholder"
                );
                assert_ne!(
                    pixels, corrupt_placeholder,
                    "{label} must render real geometry, not the corrupt placeholder"
                );
                assert!(
                    pixels.chunks_exact(4).any(|px| px[3] > 0),
                    "{label} rendered a fully transparent tile"
                );
            }
            Expect::ExactPlaceholder(expected) => {
                assert_eq!(pixels, expected, "{label} verdict drifted");
            }
        }
    }

    for path in [stl_path, obj_path, corrupt_path] {
        let _ = fs::remove_file(path);
    }
}
