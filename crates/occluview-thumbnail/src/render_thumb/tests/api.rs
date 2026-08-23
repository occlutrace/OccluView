use super::*;
use occluview_formats::dispatch::{
    dispatch_by_kind_shaded, dispatch_by_kind_with_key_provider, read_file_with_key_provider,
};
use occluview_formats::{hps::RuntimeHpsKeyProvider, FormatError, FormatKind};

#[test]
fn unsupported_extension_is_a_shell_error() {
    let res = render_thumbnail("xyz", &[0u8; 4], ThumbnailSpec::default());
    assert!(matches!(res, Err(ThumbnailError::Format(_))));
}

#[test]
fn obj_stream_without_extension_reaches_obj_parser() {
    let res = render_thumbnail_bytes(None, b"v not-a-number\n", ThumbnailSpec::default());
    assert!(matches!(
        res,
        Err(ThumbnailError::Format(FormatError::Malformed {
            format,
            ..
        })) if format == "OBJ"
    ));
}

#[test]
fn threemf_stream_is_rejected_before_rendering() {
    let res = render_thumbnail_bytes(
        Some("3mf"),
        &[0x50, 0x4B, 0x03, 0x04],
        ThumbnailSpec::default(),
    );
    assert!(matches!(
        res,
        Err(ThumbnailError::Format(FormatError::Unsupported { .. }))
    ));
}

#[test]
fn invalid_stream_returns_placeholder_without_error() {
    let spec = ThumbnailSpec {
        size_px: 16,
        ..Default::default()
    };
    let pixels = render_thumbnail_or_placeholder(None, b"not a mesh", spec);
    assert_eq!(pixels, placeholder_thumbnail(spec));
}

#[test]
fn valid_stl_stream_renders_mesh_thumbnail_not_placeholder() {
    let spec = ThumbnailSpec {
        size_px: 64,
        ..Default::default()
    };
    let bytes = fixtures::binary_stl_cube();
    let pixels = render_thumbnail_or_placeholder(Some("stl"), &bytes, spec);

    assert_ne!(pixels, placeholder_thumbnail(spec));
    assert_transparent_thumbnail_with_mesh_pixels(&pixels, spec);
}

#[test]
fn extless_hps_zip_stream_renders_mesh_thumbnail_not_placeholder() {
    let spec = ThumbnailSpec {
        size_px: 64,
        ..Default::default()
    };
    let fixture = fixtures::hps_zip_triangle();
    assert!(fixture.is_ok(), "HPS ZIP fixture should build");
    let Ok(bytes) = fixture else {
        return;
    };
    let pixels = render_thumbnail_or_placeholder(None, &bytes, spec);

    assert_ne!(pixels, placeholder_thumbnail(spec));
    assert_transparent_thumbnail_with_mesh_pixels(&pixels, spec);
}

#[test]
fn supersampled_thumbnail_downsample_preserves_transparent_edge_color() {
    let pixels = vec![200, 100, 50, 255, 0, 0, 0, 0, 200, 100, 50, 255, 0, 0, 0, 0];
    let out = rendering::downsample_rgba_premultiplied(&pixels, 2, 1);
    assert_eq!(out, vec![200, 100, 50, 127]);
}

#[test]
fn valid_colored_ply_stream_renders_mesh_thumbnail_not_placeholder() {
    let spec = ThumbnailSpec {
        size_px: 64,
        ..Default::default()
    };
    let bytes = fixtures::colored_ply_cube();
    let pixels = render_thumbnail_or_placeholder(Some("ply"), bytes, spec);

    assert_ne!(pixels, placeholder_thumbnail(spec));
    assert_transparent_thumbnail_with_mesh_pixels(&pixels, spec);
}

#[test]
fn shell_cache_sizes_render_real_thumbnails_not_placeholders() {
    let stl = fixtures::binary_stl_cube();
    let hps = fixtures::hps_zip_triangle();
    assert!(hps.is_ok(), "HPS ZIP fixture should build");
    let Ok(hps) = hps else {
        return;
    };
    let obj = fixtures::colored_obj_cube().into_bytes();
    let glb = fixtures::one_triangle_glb();
    let cases: [(&str, Option<&str>, &[u8]); 6] = [
        ("stl", Some("stl"), &stl),
        ("ply", Some("ply"), fixtures::colored_ply_cube()),
        ("obj", Some("obj"), &obj),
        ("glb", Some("glb"), &glb),
        (
            "legacy-hps-extension",
            Some(occluview_formats::LEGACY_HPS_EXTENSION),
            &hps,
        ),
        ("hps-stream", None, &hps),
    ];

    for size_px in [16, 32, 96, 256, 1024] {
        let spec = ThumbnailSpec {
            size_px,
            ..Default::default()
        };
        for (label, extension, bytes) in cases {
            let pixels = render_thumbnail_or_placeholder(extension, bytes, spec);
            assert_ne!(
                pixels,
                placeholder_thumbnail(spec),
                "{label} {size_px}px thumbnail fell back to placeholder"
            );
            assert_transparent_thumbnail_with_mesh_pixels(&pixels, spec);
        }
    }
}

#[test]
fn repeated_file_backed_thumbnail_requests_stay_real_across_shell_sizes() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let path = std::env::temp_dir().join(format!("occluview-shell-repeat-{unique}.stl"));
    let write_result = fs::write(&path, fixtures::binary_stl_cube());
    assert!(write_result.is_ok(), "failed to write temp STL fixture");
    let Ok(()) = write_result else {
        return;
    };

    for size_px in [16, 32, 96, 256, 32, 16] {
        let spec = ThumbnailSpec {
            size_px,
            ..Default::default()
        };
        let pixels = render_thumbnail_file_or_placeholder(&path, spec);
        assert_ne!(
            pixels,
            placeholder_thumbnail(spec),
            "file-backed shell thumbnail should stay real for repeated size {size_px}px"
        );
        assert_transparent_thumbnail_with_mesh_pixels(&pixels, spec);
    }

    let _ = fs::remove_file(path);
}

#[test]
fn moderate_surface_files_match_full_fidelity_file_parse() {
    let spec = ThumbnailSpec {
        size_px: 256,
        ..Default::default()
    };
    let cases = [
        ("stl", fixtures::dense_binary_stl_strip(10_000)),
        ("ply", fixtures::colored_ply_cube().to_vec()),
    ];

    for (extension, bytes) in cases {
        let path = fixtures::write_temp_fixture(extension, &bytes);
        let direct_pixels =
            render_thumbnail_file(&path, spec).expect("file-backed thumbnail should render");
        let full_mesh = read_file_with_key_provider(&path, &RuntimeHpsKeyProvider)
            .expect("full parser should load moderate surface fixture");
        let full_pixels = rendering::render_mesh_thumbnail(full_mesh, spec)
            .expect("full parsed mesh should render");
        assert_eq!(
            direct_pixels, full_pixels,
            "moderate .{extension} thumbnails should prefer the full-fidelity parser instead of a lossy fast-path surrogate"
        );
        let _ = fs::remove_file(path);
    }
}

#[test]
fn valid_obj_file_inside_full_fidelity_budget_uses_canonical_mesh_not_sparse_surrogate() {
    let bytes = fixtures::large_colored_obj_tiles(2 * 1024 * 1024);
    let path = fixtures::write_temp_fixture("obj", &bytes);
    let metadata = cache::thumbnail_file_metadata(&path).expect("temp OBJ metadata");
    let full_mesh = read_file_with_key_provider(&path, &RuntimeHpsKeyProvider)
        .expect("canonical OBJ parser should load valid fixture");
    let thumb_mesh = load_thumbnail_mesh_from_file(&path, metadata).expect("OBJ thumbnail mesh");

    assert_eq!(
        thumb_mesh.triangle_count(),
        full_mesh.triangle_count(),
        "valid OBJ thumbnails inside the fidelity budget should not use a lossy fast surrogate that makes the mesh look speckled"
    );
    let _ = fs::remove_file(path);
}

#[test]
fn valid_obj_stream_inside_full_fidelity_budget_uses_canonical_mesh_not_sparse_surrogate() {
    let bytes = fixtures::large_colored_obj_tiles(2 * 1024 * 1024);
    let full_mesh =
        dispatch_by_kind_with_key_provider(FormatKind::Obj, &bytes, &RuntimeHpsKeyProvider)
            .expect("canonical OBJ parser should load valid stream fixture");
    let thumb_mesh = load_thumbnail_mesh_from_bytes_kind(FormatKind::Obj, &bytes)
        .expect("OBJ stream thumbnail mesh");

    assert_eq!(
        thumb_mesh.triangle_count(),
        full_mesh.triangle_count(),
        "valid OBJ stream thumbnails inside the fidelity budget should not use a lossy fast surrogate that makes the mesh look speckled"
    );
}

#[test]
fn file_backed_obj_around_800kb_falls_back_to_fast_surrogate_after_parser_failure() {
    let bytes = fixtures::obj_with_early_faces(800 * 1024);
    let path = fixtures::write_temp_fixture("obj", &bytes);
    assert!(read_file_with_key_provider(&path, &RuntimeHpsKeyProvider).is_err());
    let metadata = cache::thumbnail_file_metadata(&path).expect("temp OBJ metadata");
    let mesh = load_thumbnail_mesh_from_file(&path, metadata)
        .expect("file-backed OBJ thumbnail should recover through the fast surrogate");
    assert!(mesh.triangle_count() > 0);

    let spec = ThumbnailSpec {
        size_px: 256,
        ..Default::default()
    };
    let pixels = render_thumbnail_file_or_placeholder(&path, spec);
    assert_ne!(pixels, placeholder_thumbnail(spec));
    // This stress fixture scatters many sub-pixel triangles; at 256px it now
    // falls inside the supersampled size range (MAX_SUPERSAMPLED_THUMBNAIL_SIZE_PX),
    // so such thin geometry legitimately antialiases to partial edge alpha
    // rather than hard-opaque pixels.
    assert_visible_thumbnail_pixels(&pixels, spec);

    let _ = fs::remove_file(path);
}

#[test]
fn noisy_800kb_obj_stream_and_file_thumbnails_stay_real() {
    let bytes = fixtures::noisy_obj_with_early_faces(800 * 1024);
    let strict_path = fixtures::write_temp_fixture("obj", &bytes);
    assert!(read_file_with_key_provider(&strict_path, &RuntimeHpsKeyProvider).is_err());
    let _ = fs::remove_file(strict_path);
    let spec = ThumbnailSpec {
        size_px: 256,
        ..Default::default()
    };

    let stream_pixels = render_thumbnail_or_placeholder(None, &bytes, spec);
    assert_ne!(stream_pixels, placeholder_thumbnail(spec));
    // See the comment above: this scattered-sub-pixel-triangle stress fixture
    // now supersamples at 256px, so only "visible" (not hard-opaque) coverage
    // is guaranteed.
    assert_visible_thumbnail_pixels(&stream_pixels, spec);

    let path = fixtures::write_temp_fixture("obj", &bytes);
    let file_pixels = render_thumbnail_file_or_placeholder(&path, spec);
    assert_ne!(file_pixels, placeholder_thumbnail(spec));
    assert_visible_thumbnail_pixels(&file_pixels, spec);

    let _ = fs::remove_file(path);
}

#[test]
fn noisy_obj_thumbnail_at_small_shell_size_stays_visible_not_edge_on() {
    let bytes = fixtures::noisy_obj_with_early_faces(128 * 1024);
    let path = fixtures::write_temp_fixture("obj", &bytes);
    let spec = ThumbnailSpec {
        size_px: 96,
        ..Default::default()
    };

    let pixels = render_thumbnail_file_or_placeholder(&path, spec);

    assert_ne!(pixels, placeholder_thumbnail(spec));
    assert_transparent_thumbnail_with_mesh_pixels(&pixels, spec);
    let _ = fs::remove_file(path);
}

#[test]
fn a_thirty_two_megabyte_obj_is_read_in_full_not_decimated() {
    // The OBJ budget used to stop at 24 MB, which sent a scan this size to the
    // decimating reader -- measured at 345 ms against 218 ms for the canonical
    // one, for a third fewer triangles. Decimation is held for the sizes where
    // resident memory is the constraint, not the clock.
    let spec = ThumbnailSpec {
        size_px: 256,
        ..Default::default()
    };
    let bytes = fixtures::large_colored_obj_tiles(
        usize::try_from(32_u64 * 1024 * 1024).unwrap_or(usize::MAX),
    );
    let direct_pixels =
        render_thumbnail_bytes(Some("obj"), &bytes, spec).expect("large OBJ stream should render");
    let full_mesh = dispatch_by_kind_shaded(
        FormatKind::Obj,
        &bytes,
        &RuntimeHpsKeyProvider,
        occluview_formats::MeshShading::AsWritten,
    )
    .expect("the canonical reader should load the OBJ fixture");
    let full_pixels =
        rendering::render_mesh_thumbnail(full_mesh, spec).expect("full parsed OBJ should render");

    assert_eq!(
        direct_pixels, full_pixels,
        "an OBJ inside the fidelity budget must stay on the canonical reader"
    );
}

#[test]
fn dense_large_stl_files_still_prefer_full_fidelity_parse() {
    let spec = ThumbnailSpec {
        size_px: 256,
        ..Default::default()
    };
    // 100k triangles is ~5 MB, comfortably inside the 40 MB STL fidelity cutoff.
    let path = fixtures::write_temp_fixture("stl", &fixtures::dense_binary_stl_strip(100_000));
    let direct_pixels =
        render_thumbnail_file(&path, spec).expect("large STL thumbnail should render");
    let full_mesh = read_file_with_key_provider(&path, &RuntimeHpsKeyProvider)
        .expect("full parser should load dense large STL fixture");
    let full_pixels = rendering::render_mesh_thumbnail(full_mesh, spec)
        .expect("full parsed large STL should render");
    assert_eq!(
        direct_pixels, full_pixels,
        "large STL thumbnails inside the STL-specific fidelity budget should stay on the canonical parser"
    );
    let _ = fs::remove_file(path);
}

#[test]
fn stl_files_above_the_fidelity_cutoff_use_fast_surrogate_policy() {
    let spec = ThumbnailSpec {
        size_px: 256,
        ..Default::default()
    };
    // 900k triangles is ~43 MB, comfortably above the 40 MB STL fidelity cutoff,
    // so the loader routes it through the fast (grid-clustered) surrogate.
    let path = fixtures::write_temp_fixture("stl", &fixtures::dense_binary_stl_strip(900_000));
    let direct_pixels =
        render_thumbnail_file(&path, spec).expect("large STL thumbnail should render");
    let bytes = fs::read(&path).expect("read back large STL fixture");
    let fast_mesh =
        crate::fast_thumb::try_read_fast_thumbnail_mesh_for_kind(FormatKind::Stl, &bytes)
            .expect("fast STL surrogate should parse");
    let fast_pixels = rendering::render_mesh_thumbnail(fast_mesh, spec)
        .expect("fast STL surrogate should render");
    assert_eq!(
        direct_pixels, fast_pixels,
        "file-backed large STL thumbnails should use the fast decimated surrogate once over the fidelity cutoff"
    );
    let _ = fs::remove_file(path);
}

#[test]
fn large_ply_streams_resurrect_fast_point_cloud_surrogate_and_render_non_black_pixels() {
    let spec = ThumbnailSpec {
        size_px: 256,
        ..Default::default()
    };
    // 33 MB+, comfortably above the 4 MB PLY fidelity cutoff.
    let bytes = fixtures::large_binary_ply_point_grid(33 * 1024 * 1024);
    assert!(bytes.len() > 33 * 1024 * 1024);

    let fast_mesh =
        crate::fast_thumb::try_read_fast_thumbnail_mesh_for_kind(FormatKind::Ply, &bytes)
            .expect("large PLY should resurrect the fast decimated point-cloud surrogate");
    assert!(
        fast_mesh.is_point_cloud(),
        "the PLY fast reader only ever produces a point cloud"
    );
    assert!(!fast_mesh.vertices().is_empty());

    let pixels = render_thumbnail_or_placeholder(Some("ply"), &bytes, spec);
    assert_ne!(pixels, placeholder_thumbnail(spec));
    let has_visible_non_black_pixel = pixels
        .chunks_exact(4)
        .any(|px| px[3] > 0 && (px[0] > 0 || px[1] > 0 || px[2] > 0));
    assert!(
        has_visible_non_black_pixel,
        "large PLY thumbnail should render visible, non-black pixels via the fast point-cloud path"
    );
}

#[test]
fn large_surface_ply_above_cutoff_thumbnails_as_a_surface_not_points() {
    // Regression for the "half my thumbnails are just points" bug: a PLY that
    // declares faces (a real surface mesh) sized well above the 4 MB PLY
    // fidelity cutoff must load as a triangle SURFACE, never as a decimated
    // point splat. The fast reader declines surface PLYs so the loader falls
    // through to the full `occluview-formats` reader.
    let bytes = fixtures::large_binary_ply_surface_grid(6 * 1024 * 1024);
    assert!(bytes.len() > 6 * 1024 * 1024);

    // The fast surrogate must decline (surface PLY), not return a point cloud.
    assert!(
        crate::fast_thumb::try_read_fast_thumbnail_mesh_for_kind(FormatKind::Ply, &bytes).is_none(),
        "fast PLY reader must decline a surface PLY so the full reader renders a surface"
    );

    // The thumbnail loader routes it to the full reader and gets a surface.
    let mesh = load_thumbnail_mesh_from_bytes_kind(FormatKind::Ply, &bytes)
        .expect("large surface PLY should load through the full reader");
    assert!(
        !mesh.is_point_cloud(),
        "a surface PLY above the fidelity cutoff must thumbnail as a surface, not points"
    );
    assert!(
        mesh.triangle_count() > 1000,
        "the full reader kept the faces"
    );

    // And it renders a visible thumbnail (a solid surface, not empty).
    let spec = ThumbnailSpec {
        size_px: 128,
        ..Default::default()
    };
    let pixels = render_thumbnail_or_placeholder(Some("ply"), &bytes, spec);
    assert_ne!(pixels, placeholder_thumbnail(spec));
    assert_visible_thumbnail_pixels(&pixels, spec);
}

#[test]
fn large_stl_file_and_ply_stream_render_through_the_public_thumbnail_entry_points() {
    let spec = ThumbnailSpec {
        size_px: 256,
        ..Default::default()
    };

    // File-backed entry point: what IInitializeWithFile/IInitializeWithItem use.
    let stl_path = fixtures::write_temp_fixture(
        "stl",
        &fixtures::large_binary_stl_tessellated_plane(20 * 1024 * 1024),
    );
    let stl_pixels = render_thumbnail_file_or_placeholder(&stl_path, spec);
    assert_ne!(stl_pixels, placeholder_thumbnail(spec));
    // A 20 MB tessellated plane is inside the STL fidelity budget, so it renders
    // through the full reader as a solid surface with a hard-opaque interior —
    // no stride-decimation speckle.
    assert_transparent_thumbnail_with_mesh_pixels(&stl_pixels, spec);
    let _ = fs::remove_file(stl_path);

    // Stream-backed entry point: what IInitializeWithStream uses.
    let ply_bytes = fixtures::large_binary_ply_point_grid(33 * 1024 * 1024);
    let ply_pixels = render_thumbnail_or_placeholder(Some("ply"), &ply_bytes, spec);
    assert_ne!(ply_pixels, placeholder_thumbnail(spec));
    let has_opaque_pixel = ply_pixels.chunks_exact(4).any(|px| px[3] > 0);
    assert!(
        has_opaque_pixel,
        "large PLY stream thumbnail through the public entry point should not be fully transparent"
    );
}

#[test]
fn fast_path_dense_surface_thumbnail_has_no_see_through_holes() {
    // Bug A (speckled, see-through thumbnails): the old fast path kept every
    // Nth triangle, punching the surface full of holes so the thumbnail read as
    // a see-through sieve. The fast path now WELDS onto a grid, so a decimated
    // dense surface must render as a SOLID disc — zero interior holes at 256px
    // for every size class that stays on the fast path.
    let spec = ThumbnailSpec {
        size_px: 256,
        ..Default::default()
    };
    for min_bytes in [4 * 1024 * 1024, 12 * 1024 * 1024, 24 * 1024 * 1024] {
        let bytes = fixtures::dense_binary_stl_sphere(min_bytes);
        let fast_mesh =
            crate::fast_thumb::try_read_fast_thumbnail_mesh_for_kind(FormatKind::Stl, &bytes)
                .expect("dense STL sphere should parse through the fast path");
        assert!(!fast_mesh.is_point_cloud(), "a dense STL stays a surface");
        let pixels = rendering::render_mesh_thumbnail(fast_mesh, spec)
            .expect("clustered fast-path mesh should render");
        let holes = interior_hole_count(&pixels, usize::from(spec.size_px));
        assert_eq!(
            holes,
            0,
            "clustered fast path left {holes} see-through holes for a ~{} MB dense sphere",
            min_bytes / 1024 / 1024
        );
    }
}

#[test]
fn stl_above_the_fidelity_gate_thumbnails_solid_through_the_public_entry_point() {
    // End-to-end: a dense sphere large enough to route past the 40 MB STL gate
    // must still come back as a solid surface (no speckle) via the same
    // file-backed entry point IInitializeWithFile/Item use.
    let spec = ThumbnailSpec {
        size_px: 256,
        ..Default::default()
    };
    let path =
        fixtures::write_temp_fixture("stl", &fixtures::dense_binary_stl_sphere(44 * 1024 * 1024));
    let pixels = render_thumbnail_file_or_placeholder(&path, spec);
    assert_ne!(pixels, placeholder_thumbnail(spec));
    let holes = interior_hole_count(&pixels, usize::from(spec.size_px));
    assert_eq!(
        holes, 0,
        "an above-gate dense sphere left {holes} see-through holes at the public entry point"
    );
    let _ = fs::remove_file(path);
}

#[test]
fn timed_out_thumbnail_returns_placeholder() {
    let spec = ThumbnailSpec {
        size_px: 16,
        ..Default::default()
    };
    let pixels =
        render_thumbnail_or_placeholder_with_timeout(None, b"not a mesh", spec, Duration::ZERO);
    assert_eq!(pixels, placeholder_thumbnail(spec));
}

#[test]
fn malformed_stl_returns_format_error_without_panic() {
    let res = render_thumbnail("stl", &[0u8; 10], ThumbnailSpec::default());
    assert!(matches!(res, Err(ThumbnailError::Format(_))));
}
