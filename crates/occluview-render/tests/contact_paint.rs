//! End-to-end tests for the occlusal contact paint path.
//!
//! A contact reading is a false-colour measurement, and three things decide
//! whether it can be read at all: the ramp's colour reaches the screen at the
//! colour the law gives it, the surface outside the painted band is left
//! exactly as it was, and the edge between the two is a fade rather than a
//! contour line. Each is asserted against a colour written out by hand below,
//! derived from the law's own stop values rather than from the shader's
//! arithmetic — a test that re-derived the production formula could only ever
//! agree with it.
//!
//! The field is a signed distance per vertex, so the fixture is a quad whose
//! four corner values put all three zones on screen at once: a fully painted
//! left end (clamped to the load stop), a feather in the middle, and bare scan
//! at the right end.

#![allow(clippy::expect_used)]

mod common;

use glam::{Mat4, Vec3};
use occluview_core::{Mesh, MeshBuilder, Vertex};
use occluview_render::{
    ContactFieldTexels, ContactPaintSource, GpuCamera, GpuMeshUniform, Offscreen, PreparedScene,
    PreparedSceneSource, PreparedSceneTopology, PreparedSceneUpdate, RenderDeadline, ViewportSpec,
};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

const WIDTH: usize = 64;
const HEIGHT: usize = 64;
const SIZE_PX: u16 = 64;
const DARK_TEST_BACKGROUND: [f64; 4] = [0.039, 0.039, 0.039, 1.0];
/// The quad's half extent, inside the orthographic frame's +/-1 so every region
/// this test samples is covered by real fragments.
const QUAD_HALF: f32 = 0.9;
/// Signed field at the quad's left and right edge, in millimetres. Chosen so the
/// left end clamps to the load stop, the middle sits inside the gap feather, and
/// the right end is past the painted band entirely.
const FIELD_LEFT_MM: f32 = -0.30;
const FIELD_RIGHT_MM: f32 = 0.10;
/// The gap edges bound into the uniform. The articulating-paper law's own edges
/// (0.01/0.01 mm) put its feather inside a fraction of a pixel of this fixture;
/// the width is widened here so the fade spans about six pixels and can be
/// sampled. The shader's arithmetic does not care which law supplied the
/// numbers.
const PAINT_FAR_MM: f32 = 0.04;
const FAR_FADE_MM: f32 = 0.04;

/// Columns the assertions read. Each is checked against the field the quad's own
/// gradient produces there, so a fixture that drifts fails loudly instead of
/// sampling the wrong zone.
const PAINTED_PX: usize = 4;
const MID_RAMP_PX: usize = 26;
const FEATHER_PX: usize = 49;
const BARE_PX: usize = 59;
/// Row sampled. The field depends only on x, so any covered row would do.
const SAMPLE_ROW: usize = HEIGHT / 2;

// ---------------------------------------------------------------------------
// The ramp's stops, as the CPU law writes them.
//
// `occluview_contact::TIGHTNESS` carries these two stops; the Oklab triples are
// the sRGB hex values converted through the standard sRGB -> linear -> Oklab
// transform (the ported reference is `packages/viewer/src/contact/scale.ts`).
// They are written out here rather than imported: the renderer is a lower layer
// than the metrology crate and must not depend on it, and a literal computed by
// hand is the only thing that can catch a wrong matrix constant in the shader.
// ---------------------------------------------------------------------------

/// Touch line, `#1d4ed8` at 0.00 mm — the lightest contact there is.
const TOUCH_STOP_OKLAB: [f32; 4] = [0.0, 0.488_198_3, -0.021_281_0, -0.216_120_2];
/// Expected sRGB of `TOUCH_STOP_OKLAB`, which the shader must arrive at.
const TOUCH_STOP_SRGB: [u8; 3] = [29, 78, 216];

/// Load, `#ef3e36` at -0.22 mm — RED STARTS HERE, and nowhere earlier.
const LOAD_STOP_OKLAB: [f32; 4] = [-0.22, 0.629_783_1, 0.189_664_2, 0.100_125_4];
/// Expected sRGB of `LOAD_STOP_OKLAB`.
const LOAD_STOP_SRGB: [u8; 3] = [239, 62, 54];
/// The load stop with its depth moved — what dragging "heavy at" produces.
const TIGHTENED_LOAD_STOP_OKLAB: [f32; 4] = [-0.10, 0.629_783_1, 0.189_664_2, 0.100_125_4];

fn gpu_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    common::ensure_test_runtime_dir();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("contact-paint GPU test lock is not poisoned")
}

fn test_render_deadline() -> RenderDeadline {
    RenderDeadline::after(Duration::from_secs(5))
}

fn viewport_spec() -> ViewportSpec {
    ViewportSpec {
        size_px: [SIZE_PX, SIZE_PX],
        background: DARK_TEST_BACKGROUND,
    }
}

fn orthographic_camera() -> GpuCamera {
    GpuCamera::new(
        Mat4::look_at_rh(Vec3::new(0.0, 0.0, 2.0), Vec3::ZERO, Vec3::Y),
        Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 0.1, 10.0),
        Vec3::Z,
        Vec3::new(0.0, 0.0, 2.0),
    )
}

/// A quad facing the camera, carrying a field that runs from `FIELD_LEFT_MM` to
/// `FIELD_RIGHT_MM` across its width.
///
/// The field lives on the four corners only: the shader interpolates it, and
/// that linearity is what the whole per-fragment ramp rests on.
fn field_quad() -> Mesh {
    let mut builder = MeshBuilder::new();
    let mut corner =
        |x: f32, y: f32| builder.push_vertex(Vertex::at(Vec3::new(x, y, 0.0)).with_normal(Vec3::Z));
    let bottom_left = corner(-QUAD_HALF, -QUAD_HALF);
    let bottom_right = corner(QUAD_HALF, -QUAD_HALF);
    let top_right = corner(QUAD_HALF, QUAD_HALF);
    let top_left = corner(-QUAD_HALF, QUAD_HALF);
    builder.push_triangle(bottom_left, bottom_right, top_right);
    builder.push_triangle(bottom_left, top_right, top_left);
    builder.build().expect("a quad is a mesh")
}

/// The field, in vertex-buffer order (`field_quad`).
fn field_values() -> [f32; 4] {
    [FIELD_LEFT_MM, FIELD_RIGHT_MM, FIELD_RIGHT_MM, FIELD_LEFT_MM]
}

fn packed_field(values: &[f32]) -> Arc<ContactFieldTexels> {
    Arc::new(ContactFieldTexels::from_values(values, 4).expect("four values pack into one row"))
}

/// The signed field a fragment at column `x` interpolates to.
///
/// Computed from the quad's own gradient — the orthographic frame maps world x
/// straight onto NDC x, and the varying is linear in world x — never from the
/// shader.
fn field_at_column(x: usize) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let ndc = (x as f32 + 0.5) / WIDTH as f32 * 2.0 - 1.0;
    let across = (ndc + QUAD_HALF) / (2.0 * QUAD_HALF);
    FIELD_LEFT_MM + (FIELD_RIGHT_MM - FIELD_LEFT_MM) * across
}

/// A uniform that paints `stops`.
///
/// A reading does NOT set the measured-map flag any more. That flag makes a
/// layer skip its tint and its lighting so a ramp keeps its own hue, and a
/// contact reading uses it for nothing: the paint is mixed over the finished
/// surface instead, so a scan keeps the treatment the operator gave it and only
/// the marks change. The fixture now matches the app, which is also what exposed
/// the earlier mismatch: the flag used to return before the paint ran, so this
/// test had been asking for a path the viewer no longer took.
fn contact_uniform(stops: &[[f32; 4]]) -> GpuMeshUniform {
    let mut uniform = GpuMeshUniform::identity();
    let copied = uniform.set_contact_paint(4, PAINT_FAR_MM, FAR_FADE_MM, stops);
    assert_eq!(copied, stops.len(), "the fixture's ramp must fit the table");
    uniform
}

/// The same layer with no contact field painted.
fn bare_uniform() -> GpuMeshUniform {
    GpuMeshUniform::identity()
}

fn prepare(
    offscreen: &Offscreen,
    mesh: &Mesh,
    uniform: &GpuMeshUniform,
    contact: Option<ContactPaintSource>,
) -> PreparedScene {
    offscreen.prepare_scene(&[PreparedSceneSource {
        mesh,
        uniform: *uniform,
        visible: true,
        wireframe: false,
        contact,
    }])
}

fn render(offscreen: &Offscreen, scene: &PreparedScene) -> Vec<u8> {
    pollster::block_on(offscreen.render_prepared_viewport_with_deadline(
        scene,
        &orthographic_camera(),
        viewport_spec(),
        test_render_deadline(),
    ))
    .expect("a prepared viewport renders")
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 3] {
    let at = (y * WIDTH + x) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

fn channel_gap(left: [u8; 3], right: [u8; 3]) -> i32 {
    (0..3)
        .map(|channel| i32::from(left[channel]) - i32::from(right[channel]))
        .map(i32::abs)
        .max()
        .expect("three channels")
}

/// The stop colour scaled by the one shade factor the shader is allowed to
/// apply on top of it.
///
/// A measured map is shaded by a single factor on all three channels — never
/// tinted and never given a specular highlight, because a false-colour map is
/// read by matching its hue against a legend. So the strict claim is: the pixel
/// IS the stop colour under one global shade, and that shade has to explain
/// every channel at once.
fn assert_stop_colour(actual: [u8; 3], expected: [u8; 3], what: &str) {
    let dominant = (0..3)
        .max_by_key(|channel| expected[*channel])
        .expect("three channels");
    let shade = f64::from(actual[dominant]) / f64::from(expected[dominant]);
    assert!(
        (0.90..=1.05).contains(&shade),
        "{what}: {actual:?} is not on the ramp at all (shade {shade:.3} of {expected:?})"
    );
    for channel in 0..3 {
        let want = f64::from(expected[channel]) * shade;
        assert!(
            (want - f64::from(actual[channel])).abs() <= 2.0,
            "{what}: channel {channel} of {actual:?} is not {expected:?} shaded by \
             {shade:.3} — the shader changed the ramp's colour"
        );
    }
}

/// The painted band takes the ramp's stop colour, the bare surface is untouched,
/// and the edge between them is a fade.
#[test]
fn a_contact_field_paints_the_stop_colour_and_leaves_the_rest_bare() {
    let _gpu = gpu_test_lock();

    // The fixture samples the zones it claims to.
    assert!(
        field_at_column(PAINTED_PX) <= LOAD_STOP_OKLAB[0],
        "the painted column must clamp to the load stop"
    );
    assert!(
        field_at_column(FEATHER_PX) > 0.0 && field_at_column(FEATHER_PX) < PAINT_FAR_MM,
        "the feather column must sit inside the far band"
    );
    assert!(
        field_at_column(BARE_PX) > PAINT_FAR_MM,
        "the bare column must be past the painted band"
    );

    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let mesh = field_quad();
    let stops = [TOUCH_STOP_OKLAB, LOAD_STOP_OKLAB];

    let painted = render(
        &offscreen,
        &prepare(
            &offscreen,
            &mesh,
            &contact_uniform(&stops),
            Some(ContactPaintSource::new(packed_field(&field_values()), 1)),
        ),
    );
    let bare = render(
        &offscreen,
        &prepare(&offscreen, &mesh, &bare_uniform(), None),
    );

    let deep = pixel(&painted, PAINTED_PX, SAMPLE_ROW);
    assert_stop_colour(deep, LOAD_STOP_SRGB, "the fully painted band");

    // The bare end is the same surface as before, byte for byte: a field must
    // not shade what it does not paint.
    let bare_with_field = pixel(&painted, BARE_PX, SAMPLE_ROW);
    let bare_without_field = pixel(&bare, BARE_PX, SAMPLE_ROW);
    assert_eq!(
        bare_with_field, bare_without_field,
        "a vertex outside the painted band must render exactly as it does without a field"
    );
    assert!(
        channel_gap(deep, bare_without_field) > 8,
        "the painted band must actually differ from the bare surface: {deep:?} vs \
         {bare_without_field:?}"
    );

    // The fade is a blend of the bare surface and the ramp's touch colour with
    // the weight the shader documents, so it is strictly between the two and it
    // is neither of them.
    let feather = pixel(&painted, FEATHER_PX, SAMPLE_ROW);
    // The touch stop under the same single shade factor everything else gets,
    // read off the deepest painted pixel.
    let shade = f64::from(deep[0]) / f64::from(LOAD_STOP_SRGB[0]);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let touch_shaded: [u8; 3] =
        TOUCH_STOP_SRGB.map(|channel| (f64::from(channel) * shade).round() as u8);
    assert!(
        channel_gap(feather, bare_without_field) > 8 && channel_gap(feather, deep) > 8,
        "the feather pixel {feather:?} must differ from both the bare surface and the \
         fully painted band"
    );
    for channel in 0..3 {
        let low = i32::from(touch_shaded[channel].min(bare_without_field[channel])) - 6;
        let high = i32::from(touch_shaded[channel].max(bare_without_field[channel])) + 6;
        assert!(
            (low..=high).contains(&i32::from(feather[channel])),
            "the feather pixel {feather:?} is not a blend of the bare surface \
             {bare_without_field:?} and the touch stop {touch_shaded:?}"
        );
    }
    assert!(
        i32::from(feather[1]) > i32::from(deep[1]) + 8
            && i32::from(feather[1]) < i32::from(bare_without_field[1]) - 8,
        "the feather's green channel must lie between the load stop and the bare surface"
    );
}

/// Moving the load stop repaints the surface from the uniform alone, and the
/// field is re-uploaded only when the caller says it changed.
///
/// This is the whole shape of the operator's slider: one number in the ramp
/// table, one uniform write, and the packed field — megabytes on a real scan —
/// untouched.
#[test]
fn moving_the_load_stop_repaints_without_re_uploading_the_field() {
    let _gpu = gpu_test_lock();

    let offscreen = pollster::block_on(Offscreen::new()).expect("offscreen init");
    let mesh = field_quad();
    let field = packed_field(&field_values());
    let topology = PreparedSceneTopology::from_mesh(&mesh);
    let mut scene = prepare(
        &offscreen,
        &mesh,
        &contact_uniform(&[TOUCH_STOP_OKLAB, LOAD_STOP_OKLAB]),
        Some(ContactPaintSource::new(Arc::clone(&field), 1)),
    );

    let before = render(&offscreen, &scene);
    let mid = pixel(&before, MID_RAMP_PX, SAMPLE_ROW);
    // An interpolated stop is not the stop itself: this is what makes the
    // tightening below observable at all.
    assert!(
        channel_gap(mid, LOAD_STOP_SRGB) > 8,
        "the mid-ramp column must interpolate between the stops, not clamp to the load stop: \
         {mid:?}"
    );

    // The slider moves: the load stop tightens to -0.10 mm and nothing else in
    // the scene changes.
    assert!(
        scene.update(
            offscreen.renderer(),
            &[PreparedSceneUpdate {
                topology,
                uniform: contact_uniform(&[TOUCH_STOP_OKLAB, TIGHTENED_LOAD_STOP_OKLAB]),
                visible: true,
                wireframe: false,
                contact: Some(ContactPaintSource::new(Arc::clone(&field), 1)),
            }],
        ),
        "a uniform-only update must reconcile onto the same topology"
    );
    let tightened = render(&offscreen, &scene);
    let tightened_mid = pixel(&tightened, MID_RAMP_PX, SAMPLE_ROW);
    assert!(
        channel_gap(tightened_mid, mid) > 8,
        "moving the load stop must change the ramp: {mid:?} -> {tightened_mid:?}"
    );
    assert_stop_colour(
        tightened_mid,
        LOAD_STOP_SRGB,
        "the tightened load stop clamps the mid-ramp depth",
    );

    // A different field under the SAME revision is ignored: the caller's token
    // is the promise about the bytes, and honouring it is what keeps a per-frame
    // re-derivation from re-uploading the field every frame.
    let cleared = packed_field(&[0.5; 4]);
    assert!(
        scene.update(
            offscreen.renderer(),
            &[PreparedSceneUpdate {
                topology,
                uniform: contact_uniform(&[TOUCH_STOP_OKLAB, TIGHTENED_LOAD_STOP_OKLAB]),
                visible: true,
                wireframe: false,
                contact: Some(ContactPaintSource::new(cleared, 1)),
            }],
        ),
        "an update that changes nothing it promised to change still reconciles"
    );
    let unchanged = render(&offscreen, &scene);
    assert_eq!(
        pixel(&unchanged, MID_RAMP_PX, SAMPLE_ROW),
        tightened_mid,
        "a field published under the revision already bound must not be re-uploaded"
    );

    // A new revision does upload, and the cleared field paints nothing.
    assert!(
        scene.update(
            offscreen.renderer(),
            &[PreparedSceneUpdate {
                topology,
                uniform: contact_uniform(&[TOUCH_STOP_OKLAB, TIGHTENED_LOAD_STOP_OKLAB]),
                visible: true,
                wireframe: false,
                contact: Some(ContactPaintSource::new(packed_field(&[0.5; 4]), 2)),
            }],
        ),
        "a new field revision reconciles onto the same topology"
    );
    let cleared_pixels = render(&offscreen, &scene);
    let bare = render(
        &offscreen,
        &prepare(&offscreen, &mesh, &bare_uniform(), None),
    );
    assert_eq!(
        pixel(&cleared_pixels, MID_RAMP_PX, SAMPLE_ROW),
        pixel(&bare, MID_RAMP_PX, SAMPLE_ROW),
        "a field of sentinels must paint nothing, exactly like no field at all"
    );
}
