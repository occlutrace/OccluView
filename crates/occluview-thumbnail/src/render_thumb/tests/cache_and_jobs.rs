use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

#[test]
fn file_thumbnail_cache_hits_exact_size_without_rerender() {
    let mut cache = cache::ThumbnailFileCache::default();
    let key = ThumbnailFileCacheKey {
        path: PathBuf::from("/tmp/a.stl"),
        byte_len: 123,
        modified_nanos: 456,
    };
    let pixels = vec![7_u8; 32 * 32 * 4];
    cache.insert(key.clone(), 32, &pixels);
    assert_eq!(cache.get(&key, 32), Some(pixels));
}

#[test]
fn file_thumbnail_cache_downscales_divisible_larger_size() {
    let mut cache = cache::ThumbnailFileCache::default();
    let key = ThumbnailFileCacheKey {
        path: PathBuf::from("/tmp/a.stl"),
        byte_len: 123,
        modified_nanos: 456,
    };
    let pixels = vec![200, 100, 50, 255, 0, 0, 0, 0, 200, 100, 50, 255, 0, 0, 0, 0];
    cache.insert(key.clone(), 2, &pixels);
    assert_eq!(cache.get(&key, 1), Some(vec![200, 100, 50, 127]));
}

#[test]
fn thumbnail_cache_keeps_background_variants_isolated() {
    let mut cache = cache::ThumbnailFileCache::default();
    let key = ThumbnailFileCacheKey {
        path: PathBuf::from("/tmp/background.stl"),
        byte_len: 123,
        modified_nanos: 456,
    };
    let dark = vec![8_u8; 16 * 16 * 4];
    let light = vec![240_u8; 16 * 16 * 4];

    cache.insert_with_background(key.clone(), 16, [0; 4], &dark);
    cache.insert_with_background(key.clone(), 16, [1, 0, 0, 0], &light);

    assert_eq!(cache.get_with_background(&key, 16, [0; 4]), Some(dark));
    assert_eq!(
        cache.get_with_background(&key, 16, [1, 0, 0, 0]),
        Some(light)
    );
}

#[test]
fn file_thumbnail_cache_evicts_oldest_files_to_stay_bounded() {
    let mut cache = cache::ThumbnailFileCache::new(1, 4 * 1024 * 1024);
    let first = ThumbnailFileCacheKey {
        path: PathBuf::from("/tmp/first.stl"),
        byte_len: 10,
        modified_nanos: 1,
    };
    let second = ThumbnailFileCacheKey {
        path: PathBuf::from("/tmp/second.stl"),
        byte_len: 11,
        modified_nanos: 2,
    };
    let pixels = vec![1_u8; 16 * 16 * 4];
    cache.insert(first.clone(), 16, &pixels);
    cache.insert(second.clone(), 16, &pixels);
    assert!(cache.get(&first, 16).is_none());
    assert_eq!(cache.get(&second, 16), Some(pixels));
}

#[test]
fn stream_thumbnail_cache_key_changes_when_kind_or_bytes_change() {
    use occluview_formats::FormatKind;

    let obj = cache::ThumbnailStreamCacheKey::new(FormatKind::Obj, b"v 0 0 0\nf 1 1 1\n");
    let obj_copy = cache::ThumbnailStreamCacheKey::new(FormatKind::Obj, b"v 0 0 0\nf 1 1 1\n");
    let stl = cache::ThumbnailStreamCacheKey::new(FormatKind::Stl, b"v 0 0 0\nf 1 1 1\n");
    let obj_variant = cache::ThumbnailStreamCacheKey::new(FormatKind::Obj, b"v 0 0 1\nf 1 1 1\n");

    assert_eq!(obj, obj_copy);
    assert_ne!(obj, stl);
    assert_ne!(obj, obj_variant);
}

#[test]
fn file_content_cache_key_reuses_identical_copies_and_changes_for_content() {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let first = fixtures::write_temp_fixture("content-key-a.obj", bytes);
    let second = fixtures::write_temp_fixture("content-key-b.obj", bytes);
    let changed =
        fixtures::write_temp_fixture("content-key-c.obj", b"v 0 0 0\nv 1 0 0\nv 0 2 0\nf 1 2 3\n");

    let first_metadata = cache::thumbnail_file_metadata(&first).expect("first metadata");
    let second_metadata = cache::thumbnail_file_metadata(&second).expect("second metadata");
    let changed_metadata = cache::thumbnail_file_metadata(&changed).expect("changed metadata");
    let first_key = thumbnail_file_content_key(&first, &first_metadata).expect("first content key");
    let second_key =
        thumbnail_file_content_key(&second, &second_metadata).expect("second content key");
    let changed_key =
        thumbnail_file_content_key(&changed, &changed_metadata).expect("changed content key");

    assert_eq!(first_key, second_key);
    assert_ne!(first_key, changed_key);

    let _ = fs::remove_file(first);
    let _ = fs::remove_file(second);
    let _ = fs::remove_file(changed);
}

#[test]
fn stream_thumbnail_cache_hits_exact_size_and_downscales_reuse() {
    use occluview_formats::FormatKind;

    let mut cache = cache::ThumbnailStreamCache::default();
    let key = cache::ThumbnailStreamCacheKey::new(FormatKind::Obj, b"v 0 0 0\nf 1 1 1\n");
    let pixels = vec![200, 100, 50, 255, 0, 0, 0, 0, 200, 100, 50, 255, 0, 0, 0, 0];
    cache.insert(key.clone(), 2, &pixels);
    assert_eq!(cache.get(&key, 2), Some(pixels.clone()));
    assert_eq!(cache.get(&key, 1), Some(vec![200, 100, 50, 127]));
}

#[test]
fn stream_thumbnail_cache_evicts_oldest_entries_to_stay_bounded() {
    use occluview_formats::FormatKind;

    let mut cache = cache::ThumbnailStreamCache::new(1, 4 * 1024 * 1024);
    let first = cache::ThumbnailStreamCacheKey::new(FormatKind::Obj, b"v 0 0 0\nf 1 1 1\n");
    let second = cache::ThumbnailStreamCacheKey::new(FormatKind::Obj, b"v 0 0 1\nf 1 1 1\n");
    let pixels = vec![1_u8; 16 * 16 * 4];
    cache.insert(first.clone(), 16, &pixels);
    cache.insert(second.clone(), 16, &pixels);
    assert!(cache.get(&first, 16).is_none());
    assert_eq!(cache.get(&second, 16), Some(pixels));
}

#[test]
fn thumbnail_job_timeout_allows_longer_setup_before_short_render_budget() {
    // Use a private gate so this staged-deadline assertion is isolated from
    // whatever concurrent tests are doing to the shared thumbnail job gate.
    let gate = concurrency::ThumbnailJobGate::new(1);
    let outcome = concurrency::run_thumbnail_job_with_gate_and_timeouts(
        &gate,
        Duration::from_millis(200),
        Duration::from_millis(10),
        move |progress| {
            thread::sleep(Duration::from_millis(60));
            let _ = progress.send(ThumbnailJobProgress::Prepared);
            let _ = progress.send(ThumbnailJobProgress::Finished(7_u8));
        },
    );
    assert!(matches!(outcome, ThumbnailJobOutcome::Finished(7_u8)));
}

#[test]
fn thumbnail_job_timeout_still_enforces_render_deadline_after_prepare_signal() {
    // Private gate, as above: the render-deadline behavior must not depend on
    // the shared gate being idle.
    let gate = concurrency::ThumbnailJobGate::new(1);
    let outcome = concurrency::run_thumbnail_job_with_gate_and_timeouts(
        &gate,
        Duration::from_millis(200),
        Duration::from_millis(10),
        move |progress| {
            let _ = progress.send(ThumbnailJobProgress::Prepared);
            thread::sleep(Duration::from_millis(60));
            let _ = progress.send(ThumbnailJobProgress::Finished(7_u8));
        },
    );
    assert!(matches!(outcome, ThumbnailJobOutcome::RenderTimedOut));
}

#[test]
fn timed_out_thumbnail_job_keeps_gate_until_the_worker_really_finishes() {
    let gate = concurrency::ThumbnailJobGate::new(1);
    let (release_worker, wait_for_release) = std::sync::mpsc::channel::<()>();
    let first = concurrency::run_thumbnail_job_with_gate_and_timeouts(
        &gate,
        Duration::from_secs(1),
        Duration::from_millis(30),
        move |progress| {
            let _ = progress.send(ThumbnailJobProgress::Prepared);
            let _ = wait_for_release.recv_timeout(Duration::from_secs(2));
            let _ = progress.send(ThumbnailJobProgress::Finished(7_u8));
        },
    );
    assert!(
        matches!(first, ThumbnailJobOutcome::RenderTimedOut),
        "prepared worker should be held past the render deadline"
    );

    // The caller timed out, but the worker still owns a decoded mesh and may be
    // blocked on a GPU renderer. It must continue counting against the process
    // budget; otherwise a large Explorer folder can spawn an unbounded tail of
    // surviving workers after their callers have returned.
    let second = concurrency::run_thumbnail_job_with_gate_and_timeouts(
        &gate,
        Duration::from_millis(30),
        Duration::from_secs(1),
        move |progress| {
            let _ = progress.send(ThumbnailJobProgress::Prepared);
            let _ = progress.send(ThumbnailJobProgress::Finished(9_u8));
        },
    );
    assert!(matches!(second, ThumbnailJobOutcome::SetupTimedOut));

    let _ = release_worker.send(());
    let third = concurrency::run_thumbnail_job_with_gate_and_timeouts(
        &gate,
        Duration::from_secs(1),
        Duration::from_secs(1),
        move |progress| {
            let _ = progress.send(ThumbnailJobProgress::Prepared);
            let _ = progress.send(ThumbnailJobProgress::Finished(11_u8));
        },
    );
    assert!(matches!(third, ThumbnailJobOutcome::Finished(11_u8)));
}

#[test]
fn file_backed_thumbnail_timeout_is_one_end_to_end_budget() {
    let timeout = Duration::from_millis(75);
    let path = fixtures::write_temp_fixture("obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
    let plan = prepare_file_thumbnail_render(&path, timeout)
        .expect("supported mesh file should produce a render plan");
    assert_eq!(
        plan.wait_timeout, timeout,
        "file thumbnail callers must receive one wall-clock budget"
    );
}

#[test]
fn stream_thumbnail_timeout_is_one_end_to_end_budget() {
    let timeout = Duration::from_millis(90);
    let plan = prepare_stream_thumbnail_render(
        Some("obj"),
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
        timeout,
    )
    .expect("supported thumbnail stream should produce a render plan");
    assert_eq!(
        plan.wait_timeout, timeout,
        "stream thumbnail callers must receive one wall-clock budget"
    );
}

#[test]
fn mixed_folder_noise_is_rejected_before_thumbnail_worker_startup() {
    assert!(matches!(
        prepare_file_thumbnail_render(
            Path::new("mixed-folder/readme.txt"),
            Duration::from_millis(50)
        ),
        Err(FileThumbnailPreflightError::UnsupportedExtension)
    ));
}

#[test]
fn mixed_folder_burst_renders_supported_file_thumbnails_despite_noise() {
    let spec = ThumbnailSpec {
        size_px: 96,
        ..Default::default()
    };
    let timeout = Duration::from_secs(5);
    let mut paths = Vec::new();

    for index in 0..24 {
        paths.push(write_mixed_folder_fixture(
            &format!("noise-{index}.txt"),
            b"not a mesh",
        ));
    }
    paths.push(write_mixed_folder_fixture(
        "scan-a.obj",
        fixtures::colored_obj_cube().as_bytes(),
    ));
    paths.push(write_mixed_folder_fixture(
        "scan-b.stl",
        &fixtures::binary_stl_cube(),
    ));
    paths.push(write_mixed_folder_fixture(
        "scan-c.ply",
        fixtures::colored_ply_cube(),
    ));

    let handles = paths
        .iter()
        .cloned()
        .map(|path| {
            thread::spawn(move || {
                let pixels =
                    render_thumbnail_file_or_placeholder_with_timeout(&path, spec, timeout);
                (path, pixels)
            })
        })
        .collect::<Vec<_>>();

    let mut supported_count = 0;
    for handle in handles {
        let (path, pixels) = handle
            .join()
            .expect("thumbnail worker thread should not panic");
        let extension = path.extension().and_then(|extension| extension.to_str());
        if matches!(extension, Some("obj" | "stl" | "ply")) {
            supported_count += 1;
            assert_ne!(
                pixels,
                placeholder_thumbnail(spec),
                "supported thumbnail fell back to placeholder in mixed folder burst: {}",
                path.display()
            );
            assert_burst_thumbnail_visible(&path, &pixels, spec);
        } else {
            assert_eq!(pixels, placeholder_thumbnail(spec));
        }
        let _ = fs::remove_file(path);
    }
    assert_eq!(supported_count, 3);
}

#[test]
fn eight_concurrent_mixed_requests_each_yield_a_bitmap_never_nothing() {
    // Bug B: a real folder with several large 3D files next to non-3D files and
    // broken files produced NO thumbnails at all. This fires more concurrent
    // requests than the renderer pool has slots, over a realistic mix (large
    // surfaces + a large point cloud + corrupt + encrypted + plain garbage), and
    // asserts EVERY request comes back with a full-size, non-empty bitmap - a
    // real surface, points, or a placeholder, but never an absent/empty buffer.
    // One bad or slow file must never blank its neighbors.
    let spec = ThumbnailSpec {
        size_px: 96,
        ..Default::default()
    };
    let expected_len = usize::from(spec.size_px) * usize::from(spec.size_px) * 4;
    let timeout = Duration::from_secs(5);

    let hps = fixtures::hps_zip_triangle().unwrap_or_default();
    // A truncated binary STL: recognized format, broken content -> placeholder.
    let mut corrupt_stl = fixtures::binary_stl_cube();
    corrupt_stl.truncate(120);

    let files: Vec<(&str, Vec<u8>)> = vec![
        (
            "surface-a.ply",
            fixtures::large_binary_ply_surface_grid(6 * 1024 * 1024),
        ),
        (
            "surface-b.ply",
            fixtures::large_binary_ply_surface_grid(5 * 1024 * 1024),
        ),
        (
            "cloud.ply",
            fixtures::large_binary_ply_point_grid(8 * 1024 * 1024),
        ),
        (
            "surface.stl",
            fixtures::large_binary_stl_tessellated_plane(12 * 1024 * 1024),
        ),
        (
            "surface.obj",
            fixtures::large_colored_obj_tiles(6 * 1024 * 1024),
        ),
        ("small.stl", fixtures::binary_stl_cube()),
        ("colored.ply", fixtures::colored_ply_cube().to_vec()),
        ("scan.hps", hps),
        ("corrupt.stl", corrupt_stl),
        ("notes.txt", b"this is not a mesh at all".to_vec()),
    ];

    let paths: Vec<PathBuf> = files
        .into_iter()
        .map(|(name, bytes)| write_mixed_folder_fixture(name, &bytes))
        .collect();

    let handles: Vec<_> = paths
        .iter()
        .cloned()
        .map(|path| {
            thread::spawn(move || {
                let pixels =
                    render_thumbnail_file_or_placeholder_with_timeout(&path, spec, timeout);
                (path, pixels)
            })
        })
        .collect();

    for handle in handles {
        let (path, pixels) = handle
            .join()
            .expect("no thumbnail request thread may panic across the concurrent burst");
        assert_eq!(
            pixels.len(),
            expected_len,
            "{} came back without a full-size bitmap (len {})",
            path.display(),
            pixels.len()
        );
        assert!(
            pixels.chunks_exact(4).any(|px| px[3] > 0),
            "{} produced an entirely empty bitmap (no visible pixels)",
            path.display()
        );
        let _ = fs::remove_file(path);
    }
}

#[test]
fn twenty_four_thread_mixed_burst_every_request_returns_a_bitmap_with_sane_walltime() {
    // Bug B: a folder with 20+ files (3D surfaces mixed with other formats and
    // noise) showed NO thumbnails at all. Fire more concurrent provider-level
    // requests than the render pool has slots, over a realistic mix, and assert
    // EVERY request returns a full-size, non-empty bitmap and the whole burst
    // finishes in a sane wall-clock (never the ~24 s-per-file pile-up that made
    // Explorer time out and blank the folder).
    let spec = ThumbnailSpec {
        size_px: 96,
        ..Default::default()
    };
    let expected_len = usize::from(spec.size_px) * usize::from(spec.size_px) * 4;

    let mut templates: Vec<(String, Vec<u8>)> = Vec::new();
    for index in 0..5 {
        templates.push((
            format!("surface-{index}.stl"),
            fixtures::large_binary_stl_tessellated_plane((5 + index) * 1024 * 1024),
        ));
    }
    for index in 0..4 {
        templates.push((
            format!("surface-{index}.obj"),
            fixtures::large_colored_obj_tiles((4 + index) * 1024 * 1024),
        ));
    }
    for index in 0..4 {
        templates.push((
            format!("cloud-{index}.ply"),
            fixtures::large_binary_ply_point_grid((5 + index) * 1024 * 1024),
        ));
    }
    for index in 0..4 {
        templates.push((format!("small-{index}.stl"), fixtures::binary_stl_cube()));
    }
    let mut corrupt = fixtures::binary_stl_cube();
    corrupt.truncate(120);
    for index in 0..4 {
        templates.push((format!("corrupt-{index}.stl"), corrupt.clone()));
    }
    for index in 0..3 {
        templates.push((
            format!("colored-{index}.ply"),
            fixtures::colored_ply_cube().to_vec(),
        ));
    }
    assert!(templates.len() >= 24, "need a 24+ file burst");

    let paths: Vec<PathBuf> = templates
        .iter()
        .map(|(name, bytes)| write_mixed_folder_fixture(name, bytes))
        .collect();

    let started = Instant::now();
    let handles: Vec<_> = paths
        .iter()
        .cloned()
        .map(|path| {
            thread::spawn(move || {
                let pixels = render_thumbnail_file_or_placeholder_with_timeout(
                    &path,
                    spec,
                    Duration::from_secs(6),
                );
                (path, pixels)
            })
        })
        .collect();

    for handle in handles {
        let (path, pixels) = handle.join().expect("no burst request thread may panic");
        assert_eq!(
            pixels.len(),
            expected_len,
            "{} returned without a full-size bitmap",
            path.display()
        );
        assert!(
            pixels.chunks_exact(4).any(|px| px[3] > 0),
            "{} produced an entirely empty bitmap",
            path.display()
        );
        let _ = fs::remove_file(path);
    }

    // Generous ceiling: the point is that the burst does NOT pile up into the
    // tens-of-seconds-per-file blocking that blanked large folders.
    assert!(
        started.elapsed() < Duration::from_secs(45),
        "24-thread burst took {:?}; that is the folder-blanking pile-up regressing",
        started.elapsed()
    );
}

#[test]
fn deadline_under_contention_yields_placeholders_never_errors_or_missing_bitmaps() {
    // The queue is deliberately longer than the deadline: many concurrent cold
    // requests with a punishing render budget. Every request must still come
    // back with a full-size bitmap (a placeholder when it cannot render in
    // time) — never an error, a panic, or a missing/short buffer that Explorer
    // would turn into a format icon.
    let spec = ThumbnailSpec {
        size_px: 64,
        ..Default::default()
    };
    let expected_len = usize::from(spec.size_px) * usize::from(spec.size_px) * 4;

    // Distinct content per file => distinct cache keys => a genuinely cold burst.
    let paths: Vec<PathBuf> = (0..24)
        .map(|index| {
            let bytes = fixtures::large_binary_stl_tessellated_plane((4 + index % 6) * 1024 * 1024);
            write_mixed_folder_fixture(&format!("contended-{index}.stl"), &bytes)
        })
        .collect();

    let handles: Vec<_> = paths
        .iter()
        .cloned()
        .map(|path| {
            thread::spawn(move || {
                let pixels = render_thumbnail_file_or_placeholder_with_timeout(
                    &path,
                    spec,
                    Duration::from_millis(20),
                );
                (path, pixels)
            })
        })
        .collect();

    for handle in handles {
        let (path, pixels) = handle.join().expect("no contended request may panic");
        assert_eq!(
            pixels.len(),
            expected_len,
            "{} returned a wrong-size buffer under a short deadline",
            path.display()
        );
        assert!(
            pixels.chunks_exact(4).any(|px| px[3] > 0),
            "{} returned an empty bitmap under a short deadline",
            path.display()
        );
        let _ = fs::remove_file(path);
    }
}

#[test]
fn render_that_outran_the_callers_deadline_still_populates_the_cache() {
    // Progressive fill: when a render outruns the caller's deadline the caller
    // gets a placeholder, but the background worker keeps going and caches the
    // real bitmap, so Explorer's next repaint of the same file pulls the real
    // thumbnail from the cache instead of a stuck placeholder.
    let spec = ThumbnailSpec {
        size_px: 64,
        ..Default::default()
    };
    let bytes = fixtures::large_binary_stl_tessellated_plane(4 * 1024 * 1024);
    let path = write_mixed_folder_fixture("progressive.stl", &bytes);
    let metadata = cache::thumbnail_file_metadata(&path).expect("fixture metadata");
    let key = ThumbnailFileCacheKey::new(&path, &metadata);

    // Fire with a tiny render budget so the caller bails to a placeholder while
    // the real render is still in flight, then watch the cache fill from the
    // background worker. Re-fire if a request could not even claim a job slot in
    // time (SetupTimedOut spawns no worker): under this suite's shared render
    // gate a concurrent burst test can be holding every permit, so we retry
    // until one attempt gets a worker through — whichever does caches the real
    // bitmap for the (Explorer) repaint even though its own caller bailed.
    let mut cached = None;
    'attempts: for _ in 0..40 {
        let _early = render_thumbnail_file_or_placeholder_with_timeout(
            &path,
            spec,
            Duration::from_millis(20),
        );
        for _ in 0..20 {
            if let Ok(mut cache) = thumbnail_file_cache().lock() {
                if let Some(pixels) = cache.get(&key, spec.size_px) {
                    cached = Some(pixels);
                    break 'attempts;
                }
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    let cached = cached.expect("a timed-out render must still populate the cache for the repaint");
    assert_eq!(
        cached.len(),
        usize::from(spec.size_px) * usize::from(spec.size_px) * 4
    );
    assert_ne!(
        cached,
        placeholder_thumbnail(spec),
        "the cache must hold the real render, not a placeholder"
    );
    let _ = fs::remove_file(path);
}

fn write_mixed_folder_fixture(name: &str, bytes: &[u8]) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let path = std::env::temp_dir().join(format!("occluview-mixed-folder-{unique}-{name}"));
    fs::write(&path, bytes).expect("write mixed folder fixture");
    path
}

fn assert_burst_thumbnail_visible(path: &Path, pixels: &[u8], spec: ThumbnailSpec) {
    let pixel_count = usize::from(spec.size_px) * usize::from(spec.size_px);
    assert_eq!(pixels.len(), pixel_count * 4);
    let transparent = pixels.chunks_exact(4).filter(|px| px[3] == 0).count();
    let opaque = pixels.chunks_exact(4).filter(|px| px[3] == 255).count();
    assert!(
        transparent > pixel_count / 16,
        "thumbnail should keep transparent background pixels for {} (transparent={transparent}, opaque={opaque})",
        path.display()
    );
    assert!(
        opaque > (pixel_count / 64).max(4),
        "thumbnail should contain a visible rendered mesh for {} (transparent={transparent}, opaque={opaque})",
        path.display()
    );
}

#[test]
fn stream_thumbnail_format_preflight_runs_before_worker_startup() {
    assert!(matches!(
        prepare_stream_thumbnail_render(None, b"not a mesh", Duration::from_millis(50)),
        Err(StreamThumbnailPreflightError::Format(_))
    ));

    let plan = prepare_stream_thumbnail_render(
        Some("obj"),
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
        Duration::from_millis(50),
    )
    .expect("supported OBJ bytes should infer before worker startup");
    assert_eq!(plan.kind, occluview_formats::FormatKind::Obj);
}

#[test]
fn thumbnail_job_gate_times_out_when_all_permits_are_busy() {
    let gate = concurrency::ThumbnailJobGate::new(1);
    let held_permit = gate
        .acquire_timeout(Duration::from_millis(10))
        .expect("first gate permit should be available");

    let start = Instant::now();
    let second = gate.acquire_timeout(Duration::from_millis(20));
    assert!(second.is_none());
    assert!(start.elapsed() >= Duration::from_millis(20));
    drop(held_permit);
}

#[test]
fn thumbnail_job_gate_releases_permits_after_drop() {
    let gate = concurrency::ThumbnailJobGate::new(1);
    {
        let permit = gate.acquire_timeout(Duration::from_millis(10));
        assert!(permit.is_some());
    }

    let reacquired = gate.acquire_timeout(Duration::from_millis(10));
    assert!(reacquired.is_some());
}

#[test]
fn inflight_thumbnail_coalesces_duplicate_requests() {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let key = ThumbnailRequestKey::Stream {
        cache_key: cache::ThumbnailStreamCacheKey::new(occluview_formats::FormatKind::Obj, bytes),
        size_px: 96,
        background: [0; 4],
    };
    let run_count = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(std::sync::Barrier::new(3));

    let make_worker = |run_count: Arc<AtomicUsize>, barrier: Arc<std::sync::Barrier>| {
        let key = key.clone();
        thread::spawn(move || {
            barrier.wait();
            render_coalesced_thumbnail(key, Duration::from_millis(250), move || {
                run_count.fetch_add(1, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(40));
                ThumbnailAttempt::Bitmap(vec![1, 2, 3, 4])
            })
        })
    };

    let left = make_worker(run_count.clone(), barrier.clone());
    let right = make_worker(run_count.clone(), barrier.clone());
    barrier.wait();

    let left = left.join().expect("left worker should complete");
    let right = right.join().expect("right worker should complete");
    assert_eq!(left, ThumbnailAttempt::Bitmap(vec![1, 2, 3, 4]));
    assert_eq!(right, ThumbnailAttempt::Bitmap(vec![1, 2, 3, 4]));
    assert_eq!(run_count.load(Ordering::SeqCst), 1);
}

#[test]
fn inflight_thumbnail_follower_timeout_reports_transient_failure_without_duplicate_render() {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let key = ThumbnailRequestKey::Stream {
        cache_key: cache::ThumbnailStreamCacheKey::new(occluview_formats::FormatKind::Obj, bytes),
        size_px: 128,
        background: [0; 4],
    };
    let run_count = Arc::new(AtomicUsize::new(0));

    // The leader signals the instant it enters its render closure. Because
    // `render_coalesced_thumbnail` registers the in-flight entry *before* it
    // calls the render closure, receiving this signal guarantees the follower
    // that starts next will observe the entry and take the follower path -
    // deterministically, without a racy fixed sleep.
    let (leader_entered_render, leader_registered) = std::sync::mpsc::channel::<()>();
    let leader_key = key.clone();
    let leader_count = run_count.clone();
    let leader = thread::spawn(move || {
        render_coalesced_thumbnail(leader_key, Duration::from_millis(250), move || {
            leader_count.fetch_add(1, Ordering::SeqCst);
            let _ = leader_entered_render.send(());
            thread::sleep(Duration::from_millis(90));
            ThumbnailAttempt::Bitmap(vec![9, 8, 7, 6])
        })
    });

    leader_registered
        .recv()
        .expect("leader should enter its render closure and register the in-flight entry");
    let follower_key = key.clone();
    let follower_count = run_count.clone();
    let follower = thread::spawn(move || {
        render_coalesced_thumbnail(follower_key, Duration::from_millis(10), move || {
            follower_count.fetch_add(1, Ordering::SeqCst);
            ThumbnailAttempt::Bitmap(vec![5, 4, 3, 2])
        })
    });
    let leader = leader.join().expect("leader should complete");
    let follower = follower.join().expect("follower should complete");

    assert_eq!(leader, ThumbnailAttempt::Bitmap(vec![9, 8, 7, 6]));
    // A follower that outwaits its budget reports the transient failure so the
    // COM layer can answer with an error HRESULT; inventing pixels here would
    // be cached by Explorer as the file's icon.
    assert_eq!(follower, ThumbnailAttempt::TransientFailure);
    assert_eq!(run_count.load(Ordering::SeqCst), 1);
}

#[test]
fn oversize_obj_stream_returns_placeholder_via_size_guard_before_parser() {
    let mut bytes = vec![b' '; MAX_THUMBNAIL_INPUT_BYTES + 1];
    bytes[..15].copy_from_slice(b"v not-a-number\n");

    let result = render_thumbnail_bytes(Some("obj"), &bytes, ThumbnailSpec::default());
    assert!(matches!(
        result,
        Err(ThumbnailError::Format(FormatError::Malformed { .. }))
    ));

    let spec = ThumbnailSpec {
        size_px: 16,
        ..Default::default()
    };
    let pixels = render_thumbnail_or_placeholder(Some("obj"), &bytes, spec);
    assert_eq!(pixels, placeholder_thumbnail(spec));
}

/// Two different scans of the same length must not share a thumbnail.
///
/// Above the exact-hash budget the content key is built from three 64 KiB
/// windows -- the head, the middle and the tail. A re-export whose changes are
/// interior falls between all three: the two files then key the same, and the
/// second one is served the first one's picture. That is the wrong patient's
/// arch on screen, so it is worth the cost of a 17 MB fixture to hold.
#[test]
fn a_scan_that_differs_only_inside_gets_its_own_thumbnail() {
    let triangles = 350_000_u32;
    let first = fixtures::dense_binary_stl_strip(triangles);

    // Move a run of triangles a quarter of the way in: away from the head,
    // the middle and the tail windows, and far enough to change the silhouette.
    let mut second = first.clone();
    let quarter = 84 + (usize::try_from(triangles).unwrap_or(0) / 4) * 50;
    for triangle in 0..20_000_usize {
        let base = quarter + triangle * 50 + 12;
        if base + 36 > second.len() {
            break;
        }
        for corner in 0..3 {
            let y = base + corner * 12 + 4;
            let lifted =
                f32::from_le_bytes([second[y], second[y + 1], second[y + 2], second[y + 3]]) + 9.0;
            second[y..y + 4].copy_from_slice(&lifted.to_le_bytes());
        }
    }
    assert_ne!(first, second, "the two fixtures must actually differ");

    let spec = ThumbnailSpec {
        size_px: 64,
        ..Default::default()
    };
    let first_path = fixtures::write_temp_fixture("stl", &first);
    let second_path = fixtures::write_temp_fixture("stl", &second);
    let first_meta = cache::thumbnail_file_metadata(&first_path).expect("first metadata");
    let second_meta = cache::thumbnail_file_metadata(&second_path).expect("second metadata");
    let first_key =
        thumbnail_file_content_key(&first_path, &first_meta).expect("first content key");
    let second_key =
        thumbnail_file_content_key(&second_path, &second_meta).expect("second content key");
    assert_ne!(
        first_key, second_key,
        "the two scans share a content key, so the second is served the first one's picture"
    );

    let rendered =
        |path: &Path| match try_render_thumbnail_file(path, spec, Duration::from_secs(20)) {
            ThumbnailAttempt::Bitmap(pixels) => pixels,
            // An empty answer fails the comparison below and names itself there.
            ThumbnailAttempt::TransientFailure => Vec::new(),
        };
    let first_pixels = rendered(&first_path);
    let second_pixels = rendered(&second_path);
    assert!(
        !first_pixels.is_empty() && !second_pixels.is_empty(),
        "both fixtures have to render before their pictures can be compared"
    );
    assert_ne!(
        first_pixels, second_pixels,
        "two different scans of the same length shared one picture"
    );
    let _ = fs::remove_file(first_path);
    let _ = fs::remove_file(second_path);
}

/// Past the exact budget the key is a sample, so it carries the timestamp too.
///
/// Sampling three windows can be fooled by a file that differs only between
/// them. Mixing the modification time in bounds what that costs: two files
/// then have to share a length, three windows and a timestamp before one can
/// be served the other's picture.
#[test]
fn a_sampled_content_key_is_not_shared_by_two_moments() {
    let path = fixtures::write_temp_fixture("stl", &fixtures::binary_stl_cube());
    let measured = cache::thumbnail_file_metadata(&path).expect("fixture metadata");
    // Claim a size past the exact budget so the sampled branch is taken; the
    // window reads stop at end of file.
    let sampled = |modified_nanos| ThumbnailFileMetadata {
        byte_len: cache::EXACT_CONTENT_HASH_BYTES + 1,
        modified_nanos,
    };
    let morning =
        thumbnail_file_content_key(&path, &sampled(1_000)).expect("a key for the earlier stamp");
    let evening =
        thumbnail_file_content_key(&path, &sampled(2_000)).expect("a key for the later stamp");
    assert_ne!(
        morning, evening,
        "a sampled key must distinguish two files that merely sample alike"
    );

    // And the exact branch keeps deduplicating copies, whatever their stamps.
    let exact = |modified_nanos| ThumbnailFileMetadata {
        byte_len: measured.byte_len,
        modified_nanos,
    };
    let copied = thumbnail_file_content_key(&path, &exact(1_000)).expect("a key");
    let original = thumbnail_file_content_key(&path, &exact(2_000)).expect("a key");
    assert_eq!(
        copied, original,
        "a scan copied into a folder must still share one decode with its twin"
    );
    let _ = fs::remove_file(path);
}
