use super::cache_and_jobs::write_mixed_folder_fixture;
use super::*;
use std::thread;

#[test]
fn eight_concurrent_mixed_requests_each_yield_a_bitmap_never_nothing() {
    // Every request must return either a thumbnail or a complete placeholder.
    let spec = ThumbnailSpec {
        size_px: 96,
        ..Default::default()
    };
    let expected_len = usize::from(spec.size_px) * usize::from(spec.size_px) * 4;
    let timeout = Duration::from_secs(5);

    let hps = fixtures::hps_zip_triangle().unwrap_or_default();
    // Truncated STL input should use a placeholder.
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
    // Requests beyond the pool size must complete without serial timeout buildup.
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

    // Allow CI variance while detecting serial timeout buildup.
    assert!(
        started.elapsed() < Duration::from_secs(45),
        "24-thread burst took {:?}; that is the folder-blanking pile-up regressing",
        started.elapsed()
    );
}

#[test]
fn deadline_under_contention_yields_placeholders_never_errors_or_missing_bitmaps() {
    // A deadline miss still returns a complete placeholder.
    let spec = ThumbnailSpec {
        size_px: 64,
        ..Default::default()
    };
    let expected_len = usize::from(spec.size_px) * usize::from(spec.size_px) * 4;

    // Distinct contents ensure a cold burst.
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
    // A background render may populate the cache after the caller times out.
    let spec = ThumbnailSpec {
        size_px: 64,
        ..Default::default()
    };
    let bytes = fixtures::large_binary_stl_tessellated_plane(4 * 1024 * 1024);
    let path = write_mixed_folder_fixture("progressive.stl", &bytes);
    let metadata = cache::thumbnail_file_metadata(&path).expect("fixture metadata");
    let key = ThumbnailFileCacheKey::new(&path, &metadata);

    // Retry until one request acquires a worker and fills the cache.
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
