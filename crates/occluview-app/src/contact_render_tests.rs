//! The end-to-end acceptance render: a real scan pair measured and painted.
//!
//! Everything else about the contact reading is asserted on fixtures small
//! enough to reason about — two plates, a quad, a stop table. This module is the
//! opposite end: it takes an actual articulated jaw pair from the local corpus,
//! measures it with the real compute, packs the field, prepares a real GPU scene
//! through the same path the viewport uses, and writes the frame to a PNG a
//! human can look at.
//!
//! It exists because unit tests cannot answer the question that matters most
//! about a colour map — whether a technician can read it. The numbers here are
//! therefore loose clinical sanity checks rather than exact values, and the
//! render is written to `target/contact-verify/` for inspection.
//!
//! The fixtures are the local scan corpus. A checkout without them skips the
//! test with a log line instead of failing: an absent corpus is not a defect in
//! the viewer.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use glam::{Mat4, Vec3};
use occluview_align::Soup;
use occluview_contact::{
    compute_contact_field, ContactReading, ContactReadingKind, ContactScale, ContactSettings,
    TIGHTNESS,
};
use occluview_core::{Mesh, MeshBuilder, SceneMesh, Vertex};
use occluview_render::{
    AdapterPolicy, ContactFieldTexels, ContactPaintSource, GpuCamera, GpuMeshUniform, Offscreen,
    PreparedScene, PreparedSceneSource, RenderDeadline, ViewportSpec,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Widest packed field row, matching the app's own choice.
const FIELD_TEXTURE_WIDTH: u32 = 1024;
/// Frame size of the acceptance render.
const SIZE_PX: u16 = 640;
/// Where the renders are written, relative to the workspace target directory.
const OUTPUT_DIR: &str = "contact-verify";

/// Environment variable naming the directory the acceptance corpus lives in.
///
/// The corpus is local scans, not a repository asset: a checkout has no
/// articulated pair in it, and shipping one would be shipping a patient's case.
/// So the directory is chosen at run time and the test skips with a warning when
/// it is unset.
///
/// The directory holds either a single pair (`upper.*` and `lower.*`) or
/// subdirectories each holding one, named after the case.
const FIXTURE_DIR_ENV: &str = "OCCLUVIEW_CONTACT_FIXTURES";

/// Whether this run is a release gate rather than an ordinary test run.
///
/// This module is the only end-to-end check of a real reading — the numbers the
/// panel shows as a clinical measurement. The tests that assert
/// `subject_measured > 0`, `contact_area_mm2 > 0` and "only measured vertices
/// may be painted" return early without a corpus, including on CI, so a release
/// gate sets this to turn an absent corpus into a failure.
fn fixtures_are_required() -> bool {
    std::env::var_os("OCCLUVIEW_CONTACT_FIXTURES_REQUIRED").is_some_and(|value| value != "0")
}

/// Every available `(case, upper, lower)` triple, found in the fixture
/// directory at run time.
fn available_fixtures() -> Vec<(String, PathBuf, PathBuf)> {
    let Some(dir) = std::env::var_os(FIXTURE_DIR_ENV).map(PathBuf::from) else {
        return Vec::new();
    };
    if !dir.is_dir() {
        return Vec::new();
    }

    // A directory holding the two meshes directly is the one-pair case.
    if let Some(pair) = pair_in(&dir, "case") {
        return vec![pair];
    }
    // Otherwise every subdirectory holding them is a case.
    let mut cases = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut directories: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    directories.sort();
    for path in directories {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(pair) = pair_in(&path, &name) {
            cases.push(pair);
        }
    }
    cases
}

/// The `upper`/`lower` pair inside `dir`, if both meshes are there.
///
/// Extension-agnostic on purpose: a corpus mixes STL, PLY and OBJ, and the
/// loader probes the format from the file itself rather than from its name.
fn pair_in(dir: &Path, name: &str) -> Option<(String, PathBuf, PathBuf)> {
    let read = |stem: &str| -> Option<PathBuf> {
        let entries = std::fs::read_dir(dir).ok()?;
        let mut matches: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_stem()
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(stem))
            })
            .filter(|path| path.is_file())
            .collect();
        matches.sort();
        matches.into_iter().next()
    };
    Some((name.to_owned(), read("upper")?, read("lower")?))
}

/// Load one file as a layer.
fn load(path: &Path) -> Option<SceneMesh> {
    let scene = occluview_formats::read_files(&[path.to_path_buf()]).ok()?;
    scene.meshes().first().cloned()
}

/// Flat positions and indices for one layer, the form the contact compute takes.
fn soup(mesh: &Mesh) -> (Vec<f32>, Vec<u32>) {
    let positions = mesh
        .vertices()
        .iter()
        .flat_map(|vertex| vertex.position)
        .collect();
    (positions, mesh.indices().to_vec())
}

/// A `Mesh` carrying the same triangles the field was measured over, with the
/// quad's own normals recomputed by the builder.
fn mesh_from(positions: &[f32], indices: &[u32]) -> Option<Mesh> {
    let mut builder = MeshBuilder::new();
    let vertex_count = positions.len() / 3;
    for index in 0..vertex_count {
        let p = positions.get(index * 3..index * 3 + 3)?;
        builder.push_vertex(Vertex::at(Vec3::new(p[0], p[1], p[2])));
    }
    for triangle in indices.as_chunks::<3>().0 {
        builder.push_triangle(triangle[0], triangle[1], triangle[2]);
    }
    builder.build().ok()
}

/// The camera the acceptance frame is drawn from.
///
/// An occlusal view of the bite, from the side the marks are on.
///
/// Contacts live on the occlusal surface of the upper arch, which faces down:
/// from above, the same case shows the palate and the crowns from behind and not
/// one contact. Slightly off-axis so the cusps still read as geometry rather
/// than as a flat field of colour.
fn occlusal_camera(center: Vec3, radius: f32) -> GpuCamera {
    let eye = center + Vec3::new(0.18, -0.14, -0.97).normalize() * radius * 2.4;
    let view = Mat4::look_at_rh(eye, center, Vec3::Y);
    let proj = Mat4::perspective_rh(45.0_f32.to_radians(), 1.0, radius * 0.05, radius * 40.0);
    GpuCamera::new(view, proj, Vec3::new(0.25, -0.5, -0.83), eye)
}

/// The frame the harness writes, or `None` when no GPU adapter is available.
///
/// Both arches are drawn, each wearing its own reading, because that is what the
/// viewer shows: the field is measured in both directions and the operator reads
/// the bite from whichever side faces them. A render of one arch alone would
/// hide half the feature.
fn render_frame(
    subject: &Mesh,
    subject_signed_mm: &[f32],
    antagonist: &Mesh,
    antagonist_signed_mm: &[f32],
) -> Option<Vec<u8>> {
    let subject_texels = packed(subject_signed_mm)?;
    let antagonist_texels = packed(antagonist_signed_mm)?;

    let offscreen = pollster::block_on(Offscreen::new_with_adapter_policy(
        AdapterPolicy::HardwareThenFallback,
        RenderDeadline::after(Duration::from_secs(60)),
    ))
    .ok()?;
    let renderer = offscreen.renderer();

    let scale = ContactScale::new(&TIGHTNESS, TIGHTNESS.load_mm);
    let table = scale.stop_table();
    let uniform = {
        let mut uniform = GpuMeshUniform::identity();
        uniform.set_contact_paint(
            subject_texels.width,
            table.ramp[2],
            table.ramp[3],
            &table.stops[..usize::try_from(table.count).unwrap_or(0)],
        );
        // The measured-map treatment is not used for a contact reading: that
        // branch skips the tint and the studio light and returns early, so the
        // layer would render as a flat white shell with no marks. The app
        // paints the ramp into the base colour and lets the light act on it,
        // and so does this fixture, so the render below matches the viewer.
        uniform
    };

    let prepared = PreparedScene::prepare(
        renderer,
        &[
            PreparedSceneSource {
                mesh: subject,
                uniform,
                visible: true,
                wireframe: false,
                contact: Some(ContactPaintSource::new(Arc::new(subject_texels), 1)),
            },
            PreparedSceneSource {
                mesh: antagonist,
                uniform,
                visible: true,
                wireframe: false,
                contact: Some(ContactPaintSource::new(Arc::new(antagonist_texels), 2)),
            },
        ],
    );

    // Frame the pair as one scene, the same way the viewer frames a case.
    let bbox = subject.bbox().enclose_box(antagonist.bbox());
    let camera = occlusal_camera(bbox.center(), (bbox.max - bbox.min).length() * 0.5);
    pollster::block_on(offscreen.render_prepared_viewport_with_deadline(
        &prepared,
        &camera,
        ViewportSpec {
            size_px: [SIZE_PX, SIZE_PX],
            background: [0.07, 0.07, 0.08, 1.0],
        },
        RenderDeadline::after(Duration::from_secs(60)),
    ))
    .ok()
}

/// Pack one field into the texture the vertex stage decodes.
fn packed(signed_mm: &[f32]) -> Option<ContactFieldTexels> {
    let packed = occluview_contact::pack_field_texels(signed_mm, FIELD_TEXTURE_WIDTH);
    ContactFieldTexels::new(packed.rgba, packed.width, packed.height)
}

/// Measure one pair, check the reading clinically, and write the render.
fn run_case(id: &str, upper: PathBuf, lower: PathBuf) {
    let Some(upper_entry) = load(&upper) else {
        tracing::warn!(case = id, "upper scan failed to load; skipping");
        return;
    };
    let Some(lower_entry) = load(&lower) else {
        tracing::warn!(case = id, "lower scan failed to load; skipping");
        return;
    };
    let (subject_positions, subject_indices) = soup(&upper_entry.mesh);
    let (antagonist_positions, antagonist_indices) = soup(&lower_entry.mesh);
    assert!(
        subject_indices.len() >= 3 && antagonist_indices.len() >= 3,
        "{id}: both scans must carry triangles"
    );

    // Measured twice on purpose. A multi-hue ramp on the raw field makes every
    // mark wear a rim of the intermediate depths the surface passes through on
    // its way in — the geometry guarantees it, because a tooth curves away from
    // a contact within half a millimetre. That is the reading an operator gets
    // with the panel's patch toggle off, and it is also the picture that says
    // why the toggle exists. Both are measured and both are rendered, so the
    // trade-off is visible in the output.
    let cancel = occluview_align::CancelFlag::new();
    let measure = |flatten: bool| {
        compute_contact_field(
            Soup {
                positions: &subject_positions,
                indices: &subject_indices,
                mask: None,
            },
            Soup {
                positions: &antagonist_positions,
                indices: &antagonist_indices,
                mask: None,
            },
            ContactSettings {
                search_radius_mm: occluview_contact::SEARCH_RADIUS_MM,
                flatten_patches: flatten,
            },
            &cancel,
        )
    };
    let field = measure(false);
    let flattened = measure(true);

    check_clinical_sanity(id, &field, &subject_positions, &antagonist_positions);
    let (Some(subject_mesh), Some(antagonist_mesh)) = (
        mesh_from(&subject_positions, &subject_indices),
        mesh_from(&antagonist_positions, &antagonist_indices),
    ) else {
        tracing::warn!(case = id, "the meshes could not be rebuilt");
        return;
    };
    let Some(dir) = prepare_output_dir(id) else {
        return;
    };
    write_renders(
        &dir,
        id,
        &subject_mesh,
        &antagonist_mesh,
        &[("", &field), ("-flattened", &flattened)],
    );
}

/// The two acceptance renders for one case: the raw reading, and the same
/// reading with every patch flattened to its peak.
fn write_renders(
    dir: &Path,
    id: &str,
    subject: &Mesh,
    antagonist: &Mesh,
    readings: &[(&str, &occluview_contact::ContactField)],
) {
    for (suffix, measured) in readings {
        let Some(frame) = render_frame(
            subject,
            &measured.subject_signed_mm,
            antagonist,
            &measured.antagonist_signed_mm,
        ) else {
            tracing::warn!(case = id, "no GPU adapter available; render skipped");
            return;
        };
        let png = dir.join(format!("{id}{suffix}.png"));
        let Some(image) = image::RgbaImage::from_raw(u32::from(SIZE_PX), u32::from(SIZE_PX), frame)
        else {
            tracing::warn!(
                case = id,
                "the frame did not come back at the expected size"
            );
            return;
        };
        if image.save(&png).is_ok() {
            tracing::info!(case = id, path = %png.display(), "contact render written");
        }
    }
}

/// The diagonal of the box enclosing both position arrays, in millimetres.
fn extents(subject: &[f32], antagonist: &[f32]) -> f64 {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for positions in [subject, antagonist] {
        for point in positions.as_chunks::<3>().0 {
            for axis in 0..3 {
                let value = f64::from(point[axis]);
                min[axis] = min[axis].min(value);
                max[axis] = max[axis].max(value);
            }
        }
    }
    if !min[0].is_finite() || !max[0].is_finite() {
        return 0.0;
    }
    let span = [
        (max[0] - min[0]).max(0.0),
        (max[1] - min[1]).max(0.0),
        (max[2] - min[2]).max(0.0),
    ];
    (span[0] * span[0] + span[1] * span[1] + span[2] * span[2]).sqrt()
}

/// Create the render directory, or `None` when it cannot be created.
fn prepare_output_dir(case: &str) -> Option<PathBuf> {
    let dir = output_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        tracing::warn!(case, "cannot create the render directory; skipping write");
        return None;
    }
    Some(dir)
}

/// Where the renders go: the workspace target directory, never the repository.
fn output_dir() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.pop();
    dir.pop();
    dir.push("target");
    dir.push(OUTPUT_DIR);
    dir
}

/// A pair that cannot meet, rendered so the empty state is visible.
///
/// The reading is legitimate — two casts several millimetres apart simply do
/// not touch — and the question this answers is what the operator sees when
/// that happens: a bare tooth surface with no marks on it, which is the correct
/// answer, rather than noise or a false contact.
#[test]
fn a_pair_that_cannot_meet_paints_nothing() {
    let Some((id, upper, lower)) = available_fixtures().into_iter().next() else {
        // A skip is acceptable in a developer run but not in a release gate:
        // the gate proves the reading is right, and "no corpus" would make it
        // report success for having checked nothing.
        assert!(
            !fixtures_are_required(),
            "{} is set, so the corpus is required, but no pair was found in {}",
            "OCCLUVIEW_CONTACT_FIXTURES_REQUIRED",
            FIXTURE_DIR_ENV
        );
        tracing::warn!(
            env = FIXTURE_DIR_ENV,
            "no corpus; the empty-state render is skipped"
        );
        return;
    };
    let (Some(upper_entry), Some(lower_entry)) = (load(&upper), load(&lower)) else {
        tracing::warn!(case = id, "a scan failed to load; skipping");
        return;
    };
    let (subject_positions, subject_indices) = soup(&upper_entry.mesh);
    let (mut antagonist_positions, antagonist_indices) = soup(&lower_entry.mesh);
    // Move the antagonist far enough away that nothing can be measured. A fixed
    // offset is not enough on a full arch — twenty millimetres along x puts the
    // two casts side by side but still overlapping — so the shift is the pair's
    // own bounding-box diagonal plus a margin: nothing can be inside the search
    // radius of anything, by construction.
    let extent = extents(&subject_positions, &antagonist_positions);
    #[allow(clippy::cast_possible_truncation)]
    let shift = (extent + 5.0) as f32;
    for value in antagonist_positions.iter_mut().step_by(3) {
        *value += shift;
    }

    let cancel = occluview_align::CancelFlag::new();
    let field = compute_contact_field(
        Soup {
            positions: &subject_positions,
            indices: &subject_indices,
            mask: None,
        },
        Soup {
            positions: &antagonist_positions,
            indices: &antagonist_indices,
            mask: None,
        },
        ContactSettings {
            search_radius_mm: occluview_contact::SEARCH_RADIUS_MM,
            flatten_patches: false,
        },
        &cancel,
    );
    assert_eq!(
        field.diagnostics.subject_measured, 0,
        "{id}: a pair shifted past its own extent has nothing in reach"
    );
    assert_eq!(field.stats.contacts, 0, "{id}: and nothing touching");
    assert!(
        field
            .subject_signed_mm
            .iter()
            .all(|value| !value.is_finite()),
        "{id}: every value is the no-contact sentinel, which paints nothing"
    );

    let Some(subject_mesh) = mesh_from(&subject_positions, &subject_indices) else {
        return;
    };
    let Some(antagonist_mesh) = mesh_from(&antagonist_positions, &antagonist_indices) else {
        return;
    };
    let Some(dir) = prepare_output_dir("no-overlap") else {
        return;
    };
    write_renders(
        &dir,
        "no-overlap",
        &subject_mesh,
        &antagonist_mesh,
        &[("", &field)],
    );
}

/// The acceptance run. Every pair in the local corpus, measured and painted.
#[test]
fn real_scan_pairs_measure_and_render_a_readable_contact_map() {
    let fixtures = available_fixtures();
    if fixtures.is_empty() {
        assert!(
            !fixtures_are_required(),
            "OCCLUVIEW_CONTACT_FIXTURES_REQUIRED is set, so this test is a release gate, but \
             no corpus was found in {FIXTURE_DIR_ENV}"
        );
        tracing::warn!(
            env = FIXTURE_DIR_ENV,
            "no articulated scan corpus; set the variable to a directory of upper/lower \
             pairs (or case subdirectories) and re-run this test"
        );
        return;
    }
    for (id, upper, lower) in fixtures {
        run_case(&id, upper, lower);
    }
}

/// The clinical checks a real pair has to satisfy, loose on purpose.
///
/// The numbers depend on the scan, and what is being checked is that an
/// articulated pair produces a real reading rather than an empty or saturated
/// one, that only measured vertices are painted, and that the deepest value on
/// the surface is the deepest value the panel would report.
fn check_clinical_sanity(
    id: &str,
    field: &occluview_contact::ContactField,
    subject_positions: &[f32],
    antagonist_positions: &[f32],
) {
    assert_eq!(
        field.subject_signed_mm.len(),
        subject_positions.len() / 3,
        "{id}: one value per subject vertex"
    );
    assert_eq!(
        field.antagonist_signed_mm.len(),
        antagonist_positions.len() / 3,
        "{id}: one value per antagonist vertex"
    );
    assert!(
        field.diagnostics.subject_measured > 0,
        "{id}: an articulated pair must find opposing surface somewhere"
    );
    assert!(
        field.stats.contact_area_mm2 > 0.0,
        "{id}: two casts in occlusion share contact area, and a corpus pair that shows \
         none is not a bite fixture"
    );
    tracing::info!(
        case = id,
        subject_verts = field.diagnostics.subject_verts,
        antagonist_verts = field.diagnostics.antagonist_verts,
        measured = field.diagnostics.subject_measured,
        penetrating = field.diagnostics.subject_penetrating,
        contacts = field.stats.contacts,
        contact_area_mm2 = field.stats.contact_area_mm2,
        deepest_mm = field.stats.deepest_mm,
        worker_ms = field.diagnostics.worker_ms,
        "contact field measured on a real pair"
    );

    // The two rules the feature rests on: only measured vertices are
    // painted, and the deepest reading on the surface is the deepest reading the
    // panel reports.
    let scale = ContactScale::new(&TIGHTNESS, TIGHTNESS.load_mm);
    let painted = field
        .subject_signed_mm
        .iter()
        .filter(|value| scale.is_painted(f64::from(**value)))
        .count();
    assert!(
        u32::try_from(painted).unwrap_or(u32::MAX) <= field.diagnostics.subject_measured,
        "{id}: only measured vertices may be painted"
    );
    let deepest = field
        .subject_signed_mm
        .iter()
        .filter_map(|value| ContactReading::from_signed_mm(*value))
        .filter(|reading| reading.kind == ContactReadingKind::Penetration)
        .map(|reading| reading.magnitude_mm)
        .fold(0.0_f64, f64::max);
    assert!(
        (deepest - field.stats.deepest_mm).abs() < 1.0e-4,
        "{id}: the panel's deepest reading ({}) must be the deepest value on the \
         surface ({deepest})",
        field.stats.deepest_mm
    );
}
