use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

fn file_content_key_from_path(path: &Path) -> ThumbnailFileContentKey {
    let bytes = fs::read(path).expect("read file fixture bytes");
    thumbnail_file_content_key(path, &bytes)
}

#[test]
fn missing_file_errors_do_not_include_case_paths() {
    let path = fixtures::write_temp_fixture("case-identifier.stl", b"");
    fs::remove_file(&path).expect("remove fixture before metadata lookup");
    let error = cache::thumbnail_file_metadata_checked(&path).expect_err("missing file");
    let diagnostic = format!("{error:?}");
    assert!(!diagnostic.contains("case-identifier"));
    assert!(!diagnostic.contains(&path.to_string_lossy().to_string()));
}

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
fn large_streams_with_different_interior_bytes_have_distinct_cache_keys() {
    let length =
        usize::try_from(cache::LARGE_CONTENT_FIXTURE_BYTES).expect("hash budget fits usize") + 1;
    let mut bytes = vec![0u8; length];
    let first = cache::ThumbnailStreamCacheKey::new(occluview_formats::FormatKind::Obj, &bytes);
    bytes[length / 4] = 1;
    let second = cache::ThumbnailStreamCacheKey::new(occluview_formats::FormatKind::Obj, &bytes);
    assert_ne!(
        first, second,
        "different stream payloads must not share pixels"
    );
}

#[test]
fn large_file_keys_include_interior_bytes_even_when_metadata_matches() {
    use std::io::{Seek, SeekFrom, Write};
    let path = fixtures::write_temp_fixture("large-content-key.obj", b"");
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("file");
    let length = cache::LARGE_CONTENT_FIXTURE_BYTES + 1;
    file.set_len(length).expect("sparse file");
    let first = file_content_key_from_path(&path);
    file.seek(SeekFrom::Start(length / 4)).expect("interior");
    file.write_all(&[1]).expect("changed geometry byte");
    let second = file_content_key_from_path(&path);
    assert_ne!(first, second, "different scans must not share pixels");
    drop(file);
    let _ = fs::remove_file(path);
}

#[test]
fn file_content_cache_key_reuses_identical_copies_and_changes_for_content() {
    let bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let first = fixtures::write_temp_fixture("content-key-a.obj", bytes);
    let second = fixtures::write_temp_fixture("content-key-b.obj", bytes);
    let changed =
        fixtures::write_temp_fixture("content-key-c.obj", b"v 0 0 0\nv 1 0 0\nv 0 2 0\nf 1 2 3\n");

    let first_key = file_content_key_from_path(&first);
    let second_key = file_content_key_from_path(&second);
    let changed_key = file_content_key_from_path(&changed);

    assert_eq!(first_key, second_key);
    assert_ne!(first_key, changed_key);

    let _ = fs::remove_file(first);
    let _ = fs::remove_file(second);
    let _ = fs::remove_file(changed);
}

#[test]
fn owned_primary_bytes_drive_both_digest_and_parse_after_path_changes() {
    let original = fixtures::binary_stl_cube();
    let mut replacement = original.clone();
    let triangle_count = u32::from_le_bytes(original[80..84].try_into().expect("STL count"));
    for triangle in 0..usize::try_from(triangle_count).expect("triangle count") {
        let triangle_start = 84 + triangle * 50;
        for corner in 0..3 {
            let y = triangle_start + 12 + corner * 12 + 4;
            let value = f32::from_le_bytes(
                replacement[y..y + 4].try_into().expect("vertex coordinate"),
            ) * 2.0;
            replacement[y..y + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    assert_eq!(original.len(), replacement.len());
    assert_ne!(original, replacement);

    let source_path = fixtures::write_temp_fixture("owned-source.stl", &original);
    let replacement_path = fixtures::write_temp_fixture("owned-replacement.stl", &replacement);
    let owned = occluview_formats::dispatch::read_opened_file_bytes_with_limit(
        &source_path,
        MAX_THUMBNAIL_FILE_BYTES as u64,
    )
    .expect("owned source bytes");
    let source_key = thumbnail_file_content_key(&source_path, owned.file_bytes().as_slice());
    let replacement_key = thumbnail_file_content_key(&replacement_path, &replacement);
    assert_ne!(source_key, replacement_key);

    let parsed_from_owned = load_thumbnail_mesh_from_opened_file(
        &replacement_path,
        &owned,
        occluview_formats::FormatKind::Stl,
    )
    .expect("parse the original owned bytes despite a changed path");
    let expected_source =
        load_thumbnail_mesh_from_bytes_kind(occluview_formats::FormatKind::Stl, &original)
            .expect("parse source fixture");
    let replacement_mesh =
        load_thumbnail_mesh_from_bytes_kind(occluview_formats::FormatKind::Stl, &replacement)
            .expect("parse replacement fixture");
    assert_eq!(
        parsed_from_owned.bbox_cached().size().to_array(),
        expected_source.bbox_cached().size().to_array(),
        "the parser must consume the bytes that were hashed, not reopen the changed path"
    );
    assert_ne!(
        parsed_from_owned.bbox_cached().size().to_array(),
        replacement_mesh.bbox_cached().size().to_array()
    );

    drop(owned);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(replacement_path);
}

#[test]
fn opened_file_size_recheck_rejects_a_stale_under_cap_preflight() {
    let path = fixtures::write_temp_fixture("grown-thumbnail.stl", b"old");
    let old_preflight = cache::thumbnail_file_metadata(&path).expect("old metadata");
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open for growth");
    file.set_len(MAX_THUMBNAIL_FILE_BYTES as u64 + 1)
        .expect("grow sparse fixture above the thumbnail cap");
    drop(file);

    assert!(old_preflight.byte_len < MAX_THUMBNAIL_FILE_BYTES as u64);
    match occluview_formats::dispatch::read_opened_file_bytes_with_limit(
        &path,
        MAX_THUMBNAIL_FILE_BYTES as u64,
    ) {
        Err(occluview_formats::FormatError::TooLarge { bytes, limit }) => {
            assert_eq!(bytes, MAX_THUMBNAIL_FILE_BYTES as u64 + 1);
            assert_eq!(limit, MAX_THUMBNAIL_FILE_BYTES as u64);
        }
        Err(error) => panic!("unexpected source error: {error:?}"),
        Ok(_) => panic!("fresh opened length must not be hidden by old preflight bytes"),
    }
    let _ = fs::remove_file(path);
}

#[test]
fn companion_only_changes_bypass_primary_content_and_path_pixel_reuse() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let root = std::env::temp_dir().join(format!("occluview-companion-cache-{unique}"));
    let obj_a = root.join("obj-a");
    let obj_b = root.join("obj-b");
    let ply_a = root.join("ply-a");
    let ply_b = root.join("ply-b");
    for directory in [&obj_a, &obj_b, &ply_a, &ply_b] {
        fs::create_dir_all(directory).expect("create companion fixture directory");
    }

    let obj_primary = b"mtllib material.mtl\nusemtl Dental\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n";
    let obj_a_path = obj_a.join("mesh.obj");
    let obj_b_path = obj_b.join("mesh.obj");
    fs::write(&obj_a_path, obj_primary).expect("write first OBJ primary");
    fs::write(&obj_b_path, obj_primary).expect("write second OBJ primary");
    fs::write(obj_a.join("material.mtl"), b"newmtl Dental\nmap_Kd image.png\n")
        .expect("write first OBJ material");
    fs::write(obj_b.join("material.mtl"), b"newmtl Dental\nmap_Kd image.png\n")
        .expect("write second OBJ material");
    fs::write(obj_a.join("image.png"), b"first companion image")
        .expect("write first OBJ image");
    fs::write(obj_b.join("image.png"), b"second companion image")
        .expect("write second OBJ image");

    let ply_primary = b"ply\nformat ascii 1.0\ncomment TextureFile atlas.png\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty float s\nproperty float t\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0 0 1\n1 0 0 1 1\n0 1 0 0 0\n3 0 1 2\n";
    let ply_a_path = ply_a.join("mesh.ply");
    let ply_b_path = ply_b.join("mesh.ply");
    fs::write(&ply_a_path, ply_primary).expect("write first PLY primary");
    fs::write(&ply_b_path, ply_primary).expect("write second PLY primary");
    fs::write(ply_a.join("atlas.png"), b"first companion atlas")
        .expect("write first PLY atlas");
    fs::write(ply_b.join("atlas.png"), b"second companion atlas")
        .expect("write second PLY atlas");

    assert_ne!(
        fs::read(obj_a.join("image.png")).expect("first OBJ image bytes"),
        fs::read(obj_b.join("image.png")).expect("second OBJ image bytes")
    );
    assert_ne!(
        fs::read(ply_a.join("atlas.png")).expect("first PLY atlas bytes"),
        fs::read(ply_b.join("atlas.png")).expect("second PLY atlas bytes")
    );
    let obj_path_key_before = ThumbnailFileCacheKey::new(
        &obj_a_path,
        &cache::thumbnail_file_metadata(&obj_a_path).expect("OBJ primary metadata"),
    );
    let obj_content_key_before = thumbnail_file_content_key(&obj_a_path, obj_primary);
    fs::write(obj_a.join("image.png"), b"updated companion image")
        .expect("change only the OBJ companion");
    assert_eq!(
        obj_path_key_before,
        ThumbnailFileCacheKey::new(
            &obj_a_path,
            &cache::thumbnail_file_metadata(&obj_a_path).expect("unchanged OBJ metadata"),
        )
    );
    assert_eq!(
        obj_content_key_before,
        thumbnail_file_content_key(&obj_a_path, obj_primary)
    );

    let ply_path_key_before = ThumbnailFileCacheKey::new(
        &ply_a_path,
        &cache::thumbnail_file_metadata(&ply_a_path).expect("PLY primary metadata"),
    );
    let ply_content_key_before = thumbnail_file_content_key(&ply_a_path, ply_primary);
    fs::write(ply_a.join("atlas.png"), b"updated companion atlas")
        .expect("change only the PLY companion");
    assert_eq!(
        ply_path_key_before,
        ThumbnailFileCacheKey::new(
            &ply_a_path,
            &cache::thumbnail_file_metadata(&ply_a_path).expect("unchanged PLY metadata"),
        )
    );
    assert_eq!(
        ply_content_key_before,
        thumbnail_file_content_key(&ply_a_path, ply_primary)
    );
    let obj_kind = occluview_formats::probe::probe(Some("obj"), obj_primary)
        .expect("probe actual OBJ kind");
    let ply_kind = occluview_formats::probe::probe(Some("ply"), ply_primary)
        .expect("probe actual PLY kind");
    assert_eq!(obj_kind, occluview_formats::FormatKind::Obj);
    assert_eq!(ply_kind, occluview_formats::FormatKind::Ply);
    assert_eq!(
        thumbnail_file_content_key(&obj_a_path, obj_primary),
        thumbnail_file_content_key(&obj_b_path, obj_primary),
        "primary OBJ bytes alone cannot identify its material texture"
    );
    assert_eq!(
        thumbnail_file_content_key(&ply_a_path, ply_primary),
        thumbnail_file_content_key(&ply_b_path, ply_primary),
        "primary PLY bytes alone cannot identify its atlas"
    );
    assert!(
        !file_thumbnail_can_share_primary_bytes(obj_kind)
            && !file_thumbnail_can_share_primary_bytes(ply_kind),
        "companion-capable formats must skip content/path pixel caches and inflight joins"
    );
    assert!(file_thumbnail_can_share_primary_bytes(occluview_formats::FormatKind::Stl));

    let _ = fs::remove_dir_all(root);
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
fn a_job_that_finishes_inside_its_budget_returns_its_value() {
    // A private gate, so the assertion does not depend on what the rest of the
    // suite is doing to the shared one -- but the runner is the production one.
    let gate = concurrency::ThumbnailJobGate::new(1);
    let permit = gate
        .acquire_timeout(Duration::from_millis(200))
        .expect("an idle private gate hands out its permit");
    let outcome = run_thumbnail_job_by(
        permit,
        Instant::now() + Duration::from_millis(400),
        move |progress| {
            let _ = progress.send(ThumbnailJobProgress::Prepared);
            let _ = progress.send(ThumbnailJobProgress::Finished(7_u8));
        },
    );
    assert!(matches!(outcome, ThumbnailJobOutcome::Finished(7_u8)));
}

#[test]
fn a_job_that_outruns_its_budget_after_preparing_is_a_render_timeout() {
    let gate = concurrency::ThumbnailJobGate::new(1);
    let permit = gate
        .acquire_timeout(Duration::from_millis(200))
        .expect("an idle private gate hands out its permit");
    let (release_worker, wait_for_release) = std::sync::mpsc::channel::<()>();
    let outcome = run_thumbnail_job_by(
        permit,
        // The worker waits for an explicit release after publishing
        // `Prepared`, so the test exercises the render-timeout contract
        // without depending on how quickly the runner schedules the worker; a
        // 30 ms deadline could expire as a setup timeout under a full
        // workspace run.
        Instant::now() + Duration::from_secs(2),
        move |progress| {
            let _ = progress.send(ThumbnailJobProgress::Prepared);
            let _ = wait_for_release.recv();
            let _ = progress.send(ThumbnailJobProgress::Finished(7_u8));
        },
    );
    let _ = release_worker.send(());
    assert!(matches!(outcome, ThumbnailJobOutcome::RenderTimedOut));
}

#[test]
fn a_timed_out_worker_keeps_its_lane_until_it_really_finishes() {
    // The caller gives up, but the worker still holds a decoded mesh and may be
    // waiting on the renderer. Releasing its lane early would let a large
    // folder build an unbounded tail of survivors behind the callers that have
    // already returned.
    let gate = concurrency::ThumbnailJobGate::new(1);
    let (release_worker, wait_for_release) = std::sync::mpsc::channel::<()>();
    let permit = gate
        .acquire_timeout(Duration::from_millis(200))
        .expect("an idle private gate hands out its permit");
    let first: ThumbnailJobOutcome<u8> = run_thumbnail_job_by(
        permit,
        Instant::now() + Duration::from_millis(30),
        move |progress| {
            let _ = progress.send(ThumbnailJobProgress::Prepared);
            let _ = wait_for_release.recv_timeout(Duration::from_secs(2));
            let _ = progress.send(ThumbnailJobProgress::Finished(7_u8));
        },
    );
    assert!(
        matches!(first, ThumbnailJobOutcome::RenderTimedOut),
        "a prepared worker is held past the render deadline"
    );
    assert!(
        gate.acquire_timeout(Duration::from_millis(30)).is_none(),
        "the lane is still the timed-out worker's"
    );

    let _ = release_worker.send(());
    let mut regained = None;
    for _ in 0..100 {
        if let Some(permit) = gate.acquire_timeout(Duration::from_millis(30)) {
            regained = Some(permit);
            break;
        }
    }
    assert!(
        regained.is_some(),
        "the lane comes back once the worker really finishes"
    );
}
#[test]
fn file_preflight_returns_only_file_metadata_and_cache_identity() {
    let path = fixtures::write_temp_fixture("obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
    let plan = prepare_file_thumbnail_render(&path)
        .expect("supported mesh file should produce a render plan");
    assert_eq!(
        plan.cache_key,
        ThumbnailFileCacheKey::new(&path, &plan.metadata),
        "preflight owns cache identity; the request object owns the deadline"
    );
}

#[test]
fn stream_preflight_returns_only_format_and_cache_identity() {
    let plan =
        prepare_stream_thumbnail_render(Some("obj"), b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")
            .expect("supported thumbnail stream should produce a render plan");
    assert_eq!(
        plan.kind,
        occluview_formats::FormatKind::Obj,
        "preflight owns format inference; the request object owns the deadline"
    );
}

#[test]
fn mixed_folder_noise_is_rejected_before_thumbnail_worker_startup() {
    assert!(matches!(
        prepare_file_thumbnail_render(Path::new("mixed-folder/readme.txt")),
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

pub(super) fn write_mixed_folder_fixture(name: &str, bytes: &[u8]) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let path = std::env::temp_dir().join(format!("occluview-mixed-folder-{unique}-{name}"));
    fs::write(&path, bytes).expect("write mixed folder fixture");
    path
}

pub(super) fn assert_burst_thumbnail_visible(path: &Path, pixels: &[u8], spec: ThumbnailSpec) {
    let pixel_count = usize::from(spec.size_px) * usize::from(spec.size_px);
    assert_eq!(pixels.len(), pixel_count * 4);
    let transparent = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[3] == 0)
        .count();
    let opaque = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[3] == 255)
        .count();
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
        prepare_stream_thumbnail_render(None, b"not a mesh"),
        Err(StreamThumbnailPreflightError::Format(_))
    ));

    let plan =
        prepare_stream_thumbnail_render(Some("obj"), b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")
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
    let (leader_entered_render, leader_registered) = std::sync::mpsc::channel::<()>();
    let (release_leader, wait_for_release) = std::sync::mpsc::channel::<()>();
    let leader_key = key.clone();
    let leader_count = run_count.clone();
    let leader = thread::spawn(move || {
        render_coalesced_thumbnail(leader_key, Duration::from_millis(250), move || {
            leader_count.fetch_add(1, Ordering::SeqCst);
            let _ = leader_entered_render.send(());
            let _ = wait_for_release.recv();
            ThumbnailAttempt::Bitmap(vec![1, 2, 3, 4])
        })
    });
    leader_registered
        .recv()
        .expect("leader should enter its render closure");

    let (follower_waiting, wait_for_follower) = std::sync::mpsc::channel::<()>();
    let follower_key = key;
    let follower_count = run_count.clone();
    let follower = thread::spawn(move || {
        concurrency::render_coalesced_thumbnail_with_follower_notice(
            follower_key,
            Duration::from_millis(250),
            move || {
                follower_count.fetch_add(1, Ordering::SeqCst);
                ThumbnailAttempt::Bitmap(vec![5, 6, 7, 8])
            },
            move || {
                let _ = follower_waiting.send(());
            },
        )
    });
    wait_for_follower
        .recv()
        .expect("duplicate request should join the in-flight render");
    let _ = release_leader.send(());

    let left = leader.join().expect("leader should complete");
    let right = follower.join().expect("follower should complete");
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
    // deterministically.
    let (leader_entered_render, leader_registered) = std::sync::mpsc::channel::<()>();
    let (release_leader, wait_for_release) = std::sync::mpsc::channel::<()>();
    let leader_key = key.clone();
    let leader_count = run_count.clone();
    let leader = thread::spawn(move || {
        render_coalesced_thumbnail(leader_key, Duration::from_millis(250), move || {
            leader_count.fetch_add(1, Ordering::SeqCst);
            let _ = leader_entered_render.send(());
            let _ = wait_for_release.recv();
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
    let follower = follower.join().expect("follower should complete");
    let _ = release_leader.send(());
    let leader = leader.join().expect("leader should complete");

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
    let first_key = file_content_key_from_path(&first_path);
    let second_key = file_content_key_from_path(&second_path);
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

/// Large copies share pixels, but changed interior geometry receives a new key.
#[test]
fn large_content_keys_track_geometry_across_moments_and_copies() {
    use std::io::{Seek, SeekFrom, Write};
    let path = fixtures::write_temp_fixture("large-moments.stl", &fixtures::binary_stl_cube());
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("file");
    let length = cache::LARGE_CONTENT_FIXTURE_BYTES + 1;
    file.set_len(length).expect("large file");
    let morning = file_content_key_from_path(&path);
    file.seek(SeekFrom::Start(length / 4)).expect("interior");
    file.write_all(&[1]).expect("new scan geometry");
    let evening = file_content_key_from_path(&path);
    assert_ne!(
        morning, evening,
        "a later scan must not share an earlier scan's pixels"
    );
    let copied = file_content_key_from_path(&path);
    assert_eq!(
        copied, evening,
        "identical large copies must share one decode"
    );
    drop(file);
    let _ = fs::remove_file(path);
}
