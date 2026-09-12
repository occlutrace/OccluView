//! Tests for the contact reading's state and geometry helpers.
//!
//! The state tests are about the rules an operator can feel: which scan a
//! reading picks up, what a display change is allowed to cost, and when a
//! reading stops describing the scene in front of them. The geometry tests are
//! about the number the pointer readout prints under the cursor.

// Float comparisons with exact literals are the point of these tests: a
// centroid of a uniform gradient IS the mean of its corners, and a sentinel IS
// the same number in two crates.
#![allow(
    clippy::expect_used,
    clippy::float_cmp,
    clippy::panic,
    clippy::unwrap_used
)]

use super::*;
use glam::{Affine3A, Vec3};
use occluview_core::{Mesh, Scene, SceneMesh, Vertex};
use occluview_render::ContactFieldTexels;

/// A small triangle in a plane, at `z`, offset in x.
fn slab(z: f32, x: f32) -> Mesh {
    Mesh::new(
        None,
        vec![
            Vertex::at(Vec3::new(x, 0.0, z)),
            Vertex::at(Vec3::new(x + 1.0, 0.0, z)),
            Vertex::at(Vec3::new(x, 1.0, z)),
        ],
        vec![0, 1, 2],
    )
    .expect("valid mesh")
}

fn scene_of(meshes: Vec<Mesh>) -> Scene {
    let mut scene = Scene::new();
    for mesh in meshes {
        scene.add(SceneMesh::new(mesh));
    }
    scene
}

/// The pair the menu opens on a two-scan case: the clicked layer against the
/// other one, whichever it is.
#[test]
fn the_antagonist_is_the_only_other_surface_when_there_is_one() {
    let scene = scene_of(vec![slab(0.0, 0.0), slab(1.0, 0.0)]);
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    assert_eq!(antagonist_for(&scene, ids[0]), Some(ids[1]));
    assert_eq!(antagonist_for(&scene, ids[1]), Some(ids[0]));
}

/// Nearest wins, and the rule is the one an operator can predict without being
/// told: with a waxup in the scene as well, the reading is about the scan the
/// clicked one is actually closest to.
#[test]
fn the_antagonist_is_the_nearest_surface_not_merely_another_one() {
    let scene = scene_of(vec![slab(0.0, 0.0), slab(0.2, 0.0), slab(9.0, 0.0)]);
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    assert_eq!(antagonist_for(&scene, ids[0]), Some(ids[1]));
}

/// A hidden scan is not a candidate. Its geometry would measure perfectly well
/// and the colours would land on a surface nobody can see.
#[test]
fn a_hidden_scan_is_never_the_antagonist() {
    let mut scene = scene_of(vec![slab(0.0, 0.0), slab(0.2, 0.0), slab(9.0, 0.0)]);
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    scene.meshes_mut()[1].visible = false;
    assert_eq!(antagonist_for(&scene, ids[0]), Some(ids[2]));
}

/// Neither is a point cloud: there is no surface to measure to, and a reading
/// against one would be a reading against nothing.
#[test]
fn a_point_cloud_is_never_the_antagonist() {
    let cloud = Mesh::point_cloud(None, vec![Vertex::at(Vec3::ZERO)]);
    let scene = scene_of(vec![slab(0.0, 0.0), slab(9.0, 0.0)]);
    let mut scene = scene;
    scene.add(SceneMesh::new(cloud));
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    assert_eq!(antagonist_for(&scene, ids[0]), Some(ids[1]));
}

/// One scan is not a case a reading can be opened on, and the menu has to know
/// that before it offers the entry.
#[test]
fn a_lone_scan_cannot_be_read() {
    let scene = scene_of(vec![slab(0.0, 0.0)]);
    let id = scene.meshes()[0].id();
    assert!(!can_read_contacts(&scene, id));
    assert_eq!(antagonist_for(&scene, id), None);
}

/// A hidden subject cannot be read either: the marks would land on an invisible
/// layer while the visible one stays as it was.
#[test]
fn a_hidden_subject_cannot_be_read() {
    let mut scene = scene_of(vec![slab(0.0, 0.0), slab(0.2, 0.0)]);
    let id = scene.meshes()[0].id();
    scene.meshes_mut()[0].visible = false;
    assert!(!can_read_contacts(&scene, id));
}

/// THE core promise of the panel: a display change must not re-measure. Keys
/// are what a job is re-submitted on, so the load depth must not appear in one —
/// if it did, every nudge of the slider would re-run a million-vertex search.
#[test]
fn a_display_change_produces_no_new_job_keys() {
    let scene = scene_of(vec![slab(0.0, 0.0), slab(0.2, 0.0)]);
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    let pair = ContactPair {
        subject: ids[0],
        antagonist: ids[1],
    };
    let keys = contact_job_keys(&scene, pair, false).expect("a pair in the scene");
    let mut state = ContactState::default();
    state.open(pair);
    state.mark_submitted(keys, ContactStatus::Measuring);
    state.mark_measured(keys, ContactStats::default());
    assert!(state.set_load_mm(0.4), "the slider moved");
    assert!(
        state.set_mode(ContactMode::Approach),
        "the reading switched"
    );
    assert_eq!(
        state.measured_keys(),
        Some(keys),
        "neither the slider nor the reading may invalidate a measurement"
    );
    assert_eq!(state.load_mm(), ContactMode::Approach.law().load_mm);
}

/// Moving a scan changes every distance in the map, so the keys change and the
/// reading is re-measured rather than left stale under a live legend.
#[test]
fn moving_a_scan_produces_new_job_keys() {
    let mut scene = scene_of(vec![slab(0.0, 0.0), slab(0.2, 0.0)]);
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    let pair = ContactPair {
        subject: ids[0],
        antagonist: ids[1],
    };
    let before = contact_job_keys(&scene, pair, false).expect("a pair in the scene");
    scene.meshes_mut()[1].transform = Affine3A::from_translation(Vec3::new(0.0, 0.0, 0.5));
    let after = contact_job_keys(&scene, pair, false).expect("a pair in the scene");
    assert_ne!(before, after, "a pose change must re-measure");
}

/// The patch rule IS an input to the measurement, so it belongs in the key.
#[test]
fn the_patch_rule_is_part_of_the_measurement_identity() {
    let scene = scene_of(vec![slab(0.0, 0.0), slab(0.2, 0.0)]);
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    let pair = ContactPair {
        subject: ids[0],
        antagonist: ids[1],
    };
    assert_ne!(
        contact_job_keys(&scene, pair, false),
        contact_job_keys(&scene, pair, true)
    );
}

/// A reading whose scan leaves the scene stops existing, and says which layers
/// have to stop wearing marks.
#[test]
fn a_reading_is_forgotten_when_a_scan_leaves_the_scene() {
    let mut scene = scene_of(vec![slab(0.0, 0.0), slab(0.2, 0.0)]);
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    let mut state = ContactState::default();
    state.open(ContactPair {
        subject: ids[0],
        antagonist: ids[1],
    });
    assert!(state.forget_missing(&scene).is_empty(), "both are present");
    scene.remove(1);
    assert_eq!(state.forget_missing(&scene), vec![ids[0]]);
    assert!(!state.is_open());
}

/// The load depth is clamped to the range the panel offers, so a settings file
/// from a future version cannot put the ramp somewhere no legend describes.
#[test]
fn the_load_depth_is_clamped_to_the_panel_range() {
    let mut state = ContactState::default();
    state.set_load_mm(99.0);
    assert_eq!(state.load_mm(), LOAD_MAX_MM);
    state.set_load_mm(-1.0);
    assert_eq!(state.load_mm(), LOAD_MIN_MM);
    assert!(!state.set_load_mm(f64::NAN), "a NaN is not a depth");
    assert_eq!(state.load_mm(), LOAD_MIN_MM);
}

/// Opening a reading starts from the law's own load depth, not from wherever the
/// previous case left the slider.
#[test]
fn opening_a_reading_resets_the_load_depth_to_the_law() {
    let scene = scene_of(vec![slab(0.0, 0.0), slab(0.2, 0.0)]);
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    let mut state = ContactState::default();
    state.set_load_mm(0.55);
    state.open(ContactPair {
        subject: ids[0],
        antagonist: ids[1],
    });
    assert_eq!(state.load_mm(), TIGHTNESS.load_mm);
}

/// The scale follows the reading: the two laws call different depths "loaded",
/// and carrying a number across would silently re-scale the map.
#[test]
fn switching_the_reading_takes_its_own_load_depth() {
    let mut state = ContactState::default();
    assert!(state.set_mode(ContactMode::Approach));
    assert_eq!(state.load_mm(), CLINICAL.load_mm);
    assert_eq!(state.scale().law().id, "clinical");
}

/// Two crates spell the "nothing measured here" sentinel, and they must agree:
/// the crate that measures owns the rule, the crate that uploads needs its own
/// copy of the constant, and the app is the only place both are visible.
#[test]
fn the_field_sentinel_is_one_number_in_both_crates() {
    assert_eq!(
        occluview_contact::FIELD_FAR_SENTINEL_MM,
        occluview_render::FIELD_FAR_SENTINEL_MM
    );
}

/// A value read off a surface is interpolated across the triangle, so the number
/// beside the pointer matches the colour under it instead of jumping at vertex
/// spacing.
#[test]
fn the_readout_interpolates_across_the_triangle() {
    let scene = scene_of(vec![slab(0.0, 0.0)]);
    let entry = &scene.meshes()[0];
    let field = vec![0.0_f32, 0.3, 0.6];
    let (a, b, c) = (
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    );
    let centroid = (a + b + c) / 3.0;
    let value = field_value_at(&field, entry, 0, centroid).expect("a measured triangle");
    assert!(
        (value - 0.3).abs() < 1.0e-5,
        "the centroid of a uniform gradient is the mean of its corners: {value}"
    );
    // At a corner the reading is that corner's own value.
    let at_a = field_value_at(&field, entry, 0, a).expect("a measured triangle");
    assert!(at_a.abs() < 1.0e-5, "corner a carries 0.0, got {at_a}");
}

/// A triangle whose corners were all unmeasured has no reading, and the readout
/// says nothing rather than reporting the sentinel as a distance.
#[test]
fn an_unmeasured_triangle_has_no_reading() {
    let scene = scene_of(vec![slab(0.0, 0.0)]);
    let entry = &scene.meshes()[0];
    let field = vec![
        occluview_contact::NO_CONTACT_MM,
        occluview_contact::NO_CONTACT_MM,
        occluview_contact::NO_CONTACT_MM,
    ];
    let centroid = Vec3::new(1.0, 1.0, 0.0) / 3.0;
    assert!(field_value_at(&field, entry, 0, centroid).is_none());
}

/// One unmeasured corner does not poison the triangle: near an edge of a contact
/// the honest answer is a value between "measured here" and "nothing found
/// nearby", which is what the paint shows too.
#[test]
fn one_unmeasured_corner_still_yields_a_reading() {
    let scene = scene_of(vec![slab(0.0, 0.0)]);
    let entry = &scene.meshes()[0];
    let field = vec![0.0_f32, 0.2, occluview_contact::NO_CONTACT_MM];
    let near_a = Vec3::new(0.05, 0.05, 0.0);
    let value = field_value_at(&field, entry, 0, near_a).expect("a reading");
    assert!(
        value.is_finite() && value < 0.2,
        "a corner with no measurement must not turn into a distance: {value}"
    );
}

/// The layer transform is part of the answer: the same triangle under a moved
/// layer must still read its own field, which means the barycentric weights are
/// computed in the layer's world pose.
#[test]
fn the_readout_follows_a_moved_layer() {
    let mut scene = scene_of(vec![slab(0.0, 0.0)]);
    scene.meshes_mut()[0].transform = Affine3A::from_translation(Vec3::new(10.0, 0.0, 0.0));
    let entry = &scene.meshes()[0];
    let field = vec![0.0_f32, 0.3, 0.6];
    let moved_centroid = Vec3::new(10.0 + 1.0 / 3.0, 1.0 / 3.0, 0.0);
    let value = field_value_at(&field, entry, 0, moved_centroid).expect("a measured triangle");
    assert!((value - 0.3).abs() < 1.0e-5, "got {value}");
}

/// Degenerate triangles have no barycentric coordinates, and the readout must
/// answer nothing rather than dividing by zero.
#[test]
fn a_degenerate_triangle_has_no_barycentric_coordinates() {
    assert!(barycentric(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO).is_none());
}

/// The reading's own sign rule comes from the contact crate, so the chip and the
/// paint cannot disagree about which side of touch a value is on.
#[test]
fn the_reading_splits_gap_from_penetration_at_touch() {
    use occluview_contact::ContactReadingKind;
    assert_eq!(
        reading_of(0.12).expect("a gap").kind,
        ContactReadingKind::Gap
    );
    assert_eq!(
        reading_of(-0.12).expect("a load").kind,
        ContactReadingKind::Penetration
    );
    assert!(
        reading_of(occluview_contact::NO_CONTACT_MM).is_none(),
        "no opposing surface is not a gap of any size"
    );
    assert!(reading_of(f32::NAN).is_none());
}

/// A refusal is about the input, not about the moment.
///
/// The frame loop asks `needs_measurement` every frame. Without a memory of the
/// refusal, a pair the compute cannot measure would be re-submitted on every
/// one of those frames: the worker would run the same doomed search for as long
/// as the panel stayed open, and the operator would watch a status line flicker
/// between "measuring" and "failed" forever.
#[test]
fn a_refusal_is_not_retried_until_something_changes() {
    let scene = scene_of(vec![slab(0.0, 0.0), slab(0.2, 0.0)]);
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    let pair = ContactPair {
        subject: ids[0],
        antagonist: ids[1],
    };
    let keys = contact_job_keys(&scene, pair, false).expect("a pair in the scene");
    let mut state = ContactState::default();
    state.open(pair);
    assert!(
        state.needs_measurement(keys),
        "a fresh reading has to measure"
    );

    state.mark_submitted(keys, ContactStatus::Measuring);
    assert!(
        !state.needs_measurement(keys),
        "a job already in flight must not be queued twice"
    );

    state.mark_failed(keys, ContactFailure::NoSurface);
    assert!(
        !state.needs_measurement(keys),
        "a refusal must not be retried on the next frame"
    );

    // The patch rule IS an input, so turning it is a reason to try again.
    assert!(state.set_flatten_patches(true));
    let retry = contact_job_keys(&scene, pair, true).expect("a pair in the scene");
    assert!(state.needs_measurement(retry));

    // And a measurement that landed is not re-run either.
    state.mark_measured(retry, ContactStats::default());
    assert!(!state.needs_measurement(retry));
}

/// The chip's swatch is a claim about the surface, so it has to be the paint's
/// own answer.
///
/// Outside the painted band the surface wears nothing there. A swatch drawn
/// straight from `color_at` would be a solid black chip — `color_at` folds the
/// paint weight into the alpha and returns `[0,0,0,0]` — claiming a colour the
/// operator cannot find on the scan.
#[test]
fn a_reading_outside_the_painted_band_has_no_swatch_to_show() {
    let scale = ContactScale::new(&TIGHTNESS, TIGHTNESS.load_mm);
    // Inside the band: painted, and the colour is the touch blue.
    assert!(scale.is_painted(0.0));
    let [red, green, blue, alpha] = scale.color_at(0.0);
    assert_eq!([red, green, blue], [29, 78, 216], "the touch stop");
    assert_eq!(alpha, 255, "and it is fully painted there");
    // Outside it: the surface paints nothing, and `color_at` says so with a
    // zero alpha rather than with a colour.
    let gap = 0.3;
    assert!(
        !scale.is_painted(gap),
        "a 300 um gap is bare surface under the articulating-paper law"
    );
    assert_eq!(
        scale.color_at(gap),
        [0, 0, 0, 0],
        "an unpainted value carries no colour, which is why the chip must not \
         turn it into a black swatch"
    );
}

/// The panel distinguishes which of the two scans stopped being readable: a
/// sentence about the wrong one sends the operator to unhide the wrong layer.
#[test]
fn the_unreadable_scan_status_names_the_right_scan() {
    assert_ne!(
        ContactStatus::SubjectUnusable.key(),
        ContactStatus::AntagonistUnusable.key()
    );
}

/// Only a refusal earns a retry button. A hidden scan is fixed by showing it
/// again, and a pair that does not meet is a valid reading, not a failure.
#[test]
fn only_a_refusal_offers_a_retry() {
    let scene = scene_of(vec![slab(0.0, 0.0), slab(0.2, 0.0)]);
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    let pair = ContactPair {
        subject: ids[0],
        antagonist: ids[1],
    };
    let keys = contact_job_keys(&scene, pair, false).expect("a pair in the scene");
    let mut state = ContactState::default();
    state.open(pair);
    assert!(!state.refused(), "a fresh reading is not a refusal");
    state.mark_measured(keys, ContactStats::default());
    assert!(!state.refused(), "a reading that landed is not a refusal");
    state.status_override(ContactStatus::NoOverlap);
    assert!(
        !state.refused(),
        "two scans too far apart is an answer, not a failure"
    );
    state.status_override(ContactStatus::SubjectUnusable);
    assert!(
        !state.refused(),
        "a hidden scan is fixed by showing it, not by measuring again"
    );
    state.mark_failed(keys, ContactFailure::Worker);
    assert!(state.refused(), "a job that produced nothing earns a retry");
}

/// Each painted arch answers for itself.
///
/// The reading paints both surfaces, so a readout wired to one of them is dead
/// over half of what the feature draws. The two fields are measured in opposite
/// directions, so the same physical spot reads a gap from one side and a load
/// from the other — which is exactly what each side's own paint shows.
#[test]
fn both_painted_arches_carry_their_own_reading() {
    let scene = scene_of(vec![slab(0.0, 0.0), slab(0.05, 0.0)]);
    let entries: Vec<&SceneMesh> = scene.meshes().iter().collect();
    let centroid = Vec3::new(1.0 / 3.0, 1.0 / 3.0, 0.0);
    // The lower plate sits 50 um above the upper one, so the upper is inside it
    // and the lower is clear of it.
    let upper_field = vec![-0.05_f32; 3];
    let lower_field = vec![0.05_f32; 3];
    let Some(upper) = field_value_at(&upper_field, entries[0], 0, centroid) else {
        panic!("the upper surface carries a reading");
    };
    let Some(lower) = field_value_at(&lower_field, entries[1], 0, centroid) else {
        panic!("the lower surface carries a reading");
    };
    assert_eq!(
        reading_of(upper).expect("a load").kind,
        occluview_contact::ContactReadingKind::Penetration,
        "the surface inside the other one reads a load"
    );
    assert_eq!(
        reading_of(lower).expect("a gap").kind,
        occluview_contact::ContactReadingKind::Gap,
        "and the surface it is inside reads the matching clearance"
    );
}

/// A hand drag moves a scan every frame, so the measurement's keys change every
/// frame. Holding the reading back is what keeps a full surface-index build from
/// being restarted per frame and thrown away — and it takes the marks down,
/// because a map measured against where the scan used to be is not the map the
/// operator is looking at.
#[test]
fn a_hand_drag_holds_the_reading_back_and_takes_the_marks_down() {
    let scene = scene_of(vec![slab(0.0, 0.0), slab(0.2, 0.0)]);
    let ids: Vec<SceneMeshId> = scene.meshes().iter().map(SceneMesh::id).collect();
    let pair = ContactPair {
        subject: ids[0],
        antagonist: ids[1],
    };
    let keys = contact_job_keys(&scene, pair, false).expect("a pair in the scene");
    let mut state = ContactState::default();
    state.open(pair);
    state.mark_measured(keys, ContactStats::default());
    state.store_field(
        pair.subject,
        ContactLayerField {
            layer: pair.subject,
            signed_mm: Arc::new(vec![0.0; 3]),
            texels: Arc::new(ContactFieldTexels::new(vec![0, 0, 0, 0], 1, 1).expect("a 1x1 field")),
            revision: 1,
        },
    );
    assert_eq!(state.fields().len(), 1);

    state.hold_for_drag();
    assert!(
        state.fields().is_empty(),
        "the marks come down with the drag"
    );
    assert!(
        !state.needs_measurement(keys),
        "and nothing is measured while the scan is still moving"
    );
    assert_eq!(
        state.status(),
        Some(ContactStatus::Remeasuring),
        "the panel says why the map went away"
    );

    state.resume_after_drag();
    assert!(
        state.needs_measurement(keys),
        "when the drag ends the reading runs once against where the scan landed"
    );
}

/// Opening a reading takes the align heatmap down with it.
///
/// The deviation map and the contact map measure the same two scans and are
/// painted through the same measured-map treatment, so a layer can only wear
/// one of them: with both up, the panel's legend describes a ramp the surface
/// is not wearing, and the operator has no way to tell which measurement the
/// colours came from. The align status line has the same problem, so the
/// overlay goes too rather than only the flag.
#[test]
fn opening_a_reading_clears_the_align_heatmap() {
    let source = crate::primary_ui_tests::production_source(include_str!("app/app_contact.rs"));
    let body =
        crate::primary_ui_tests::method_body(source, "pub(super) fn begin_contacts_from_layer");
    assert!(!body.is_empty(), "begin_contacts_from_layer must exist");
    let (before_open, _) = body
        .split_once("self.tools.contacts.open(")
        .expect("the reading is opened in this function");
    assert!(
        before_open.contains("show_deviation = false"),
        "the deviation flag must be cleared before the reading opens"
    );
    assert!(
        before_open.contains("clear_deviation_overlay()"),
        "the overlay arrays must be dropped, not just the flag"
    );
}
