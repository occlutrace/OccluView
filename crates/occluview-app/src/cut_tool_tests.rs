#![allow(clippy::float_cmp, clippy::expect_used)]
use super::*;
use crate::cut_manipulator::SurfaceSample;
use crate::cut_ruler::{SectionDisplay, SliceCam, SliceMeasureMode};

fn bbox() -> Aabb {
    Aabb::from_min_max(Vec3::new(10.0, 20.0, 30.0), Vec3::new(50.0, 80.0, 90.0))
}

fn frame(hit: Option<SurfaceSample>, pressed: bool) -> CutFrameInput {
    CutFrameInput {
        pointer: Some(egui::pos2(100.0, 100.0)),
        over_viewport: true,
        primary_pressed: pressed,
        primary_down: pressed,
        ctrl: false,
        escape: false,
        flip: false,
        wheel_notches: 0.0,
        eye: Vec3::new(0.0, 0.0, 100.0),
        view_dir: Vec3::NEG_Z,
        camera_right: Vec3::X,
        camera_up: Vec3::Y,
        ray_origin: Vec3::new(0.0, 0.0, 100.0),
        surface_hit: hit,
        disc_center_screen: Some(egui::pos2(100.0, 100.0)),
        disc_radius_screen: 40.0,
    }
}

#[test]
fn inactive_cut_tool_returns_disabled_clip_plane() {
    let tool = CutTool::default();
    assert_eq!(tool.viewport_clip_plane(bbox()).enabled, 0);
}

#[test]
fn armed_without_a_hover_has_no_clip_and_no_spec() {
    let mut tool = CutTool::default();
    tool.enable();
    assert!(tool.is_active());
    assert_eq!(tool.viewport_clip_plane(bbox()).enabled, 0);
    assert!(tool.cut_view_spec(bbox()).is_none());
}

#[test]
fn hovering_the_mesh_derives_a_clip_and_a_spec() {
    let mut tool = CutTool::default();
    tool.enable();
    let hit = Some(SurfaceSample {
        point: Vec3::new(30.0, 50.0, 60.0),
        normal: Vec3::Y,
    });
    let out = tool.update(&frame(hit, false), Vec3::new(0.0, 0.0, 100.0));
    assert!(out.pose_changed);
    let plane = tool.viewport_clip_plane(bbox());
    assert_eq!(plane.enabled, 1);
    assert!(tool.cut_view_spec(bbox()).is_some());
    assert!(tool.section_plane().is_some());
}

#[test]
fn planting_freezes_the_pose_and_focus_frames_the_disc() {
    let mut tool = CutTool::default();
    tool.enable();
    let hit = Some(SurfaceSample {
        point: Vec3::new(30.0, 50.0, 60.0),
        normal: Vec3::Y,
    });
    let out = tool.update(&frame(hit, true), Vec3::new(0.0, 0.0, 100.0));
    assert!(out.planted);
    assert!(tool.is_planted());
    let (center, half_extent) = tool.cut_view_focus(bbox());
    assert_eq!(center, Vec3::new(30.0, 50.0, 60.0));
    assert!(half_extent > 0.0 && half_extent < bbox().half_diagonal());
}

#[test]
fn disable_clears_everything() {
    let mut tool = CutTool::default();
    tool.enable();
    tool.update(
        &frame(
            Some(SurfaceSample {
                point: Vec3::ZERO,
                normal: Vec3::Y,
            }),
            false,
        ),
        Vec3::new(0.0, 0.0, 100.0),
    );
    tool.disable();
    assert!(!tool.is_active());
    assert_eq!(tool.viewport_clip_plane(bbox()).enabled, 0);
}

fn hover(point: Vec3) -> CutFrameInput {
    frame(
        Some(SurfaceSample {
            point,
            normal: Vec3::Y,
        }),
        false,
    )
}

fn slice_image() -> egui::ColorImage {
    egui::ColorImage::new([4, 4], egui::Color32::WHITE)
}

fn slice_cam() -> SliceCam {
    SliceCam {
        focus: Vec3::ZERO,
        normal: Vec3::X,
        half_extent: 8.0,
    }
}

/// Regression for the destroyed-texture submit crash: hovering the surface in
/// cut mode crashed at `Queue::submit` ("texture ... has been destroyed") the
/// moment the follow disc first re-rendered its slice. Root cause was a fresh
/// egui texture id per render, whose dropped predecessor egui-wgpu 0.29 frees
/// (destroys) before submit. The preview must instead reuse ONE texture id.
#[test]
fn slice_preview_reuses_one_texture_id_across_pose_changes() {
    let ctx = egui::Context::default();
    let eye = Vec3::new(0.0, 0.0, 100.0);
    let mut tool = CutTool::default();
    tool.enable();
    // This regression is about the Mesh-mode offscreen texture lifecycle;
    // Lines mode (the default) has no texture to keep alive.
    tool.section.set_display_mode(SectionDisplay::Mesh);

    // First hover + slice render.
    tool.update(&hover(Vec3::ZERO), eye);
    tool.store_slice(&ctx, slice_image(), slice_cam());
    let first_id = tool.section.texture_id().expect("slice texture");
    assert!(tool.slice_visible(), "a rendered slice is shown");

    // A new hover pose hides the stale slice but keeps the handle alive.
    tool.update(&hover(Vec3::new(1.0, 0.0, 0.0)), eye);
    assert!(
        !tool.slice_visible(),
        "the stale slice hides until the re-render lands"
    );
    assert!(
        tool.section.texture_id().is_some(),
        "the handle survives the pose change (stable id, no free)"
    );

    // Re-render the slice for the new pose: same id, so no texture-`free`.
    tool.store_slice(&ctx, slice_image(), slice_cam());
    let second_id = tool.section.texture_id().expect("slice texture");
    assert_eq!(
        first_id, second_id,
        "the slice preview must update ONE texture id in place"
    );
    assert!(tool.slice_visible());
}

/// Frame-boundary invariant that egui-wgpu turns into the crash: a texture id
/// a frame paints must NOT be in that frame's `textures_delta.free`. egui-wgpu
/// 0.29 destroys freed textures AFTER recording the frame's draws but BEFORE
/// `queue.submit`, so a painted-and-freed id is destroyed mid-flight. Driving
/// the persistent-handle pattern through a real egui frame, the painted slice
/// id is never freed.
#[test]
fn a_painted_slice_texture_is_never_freed_in_the_same_frame() {
    use egui::epaint::Primitive;
    use std::collections::BTreeSet;

    fn painted_ids(ctx: &egui::Context, out: &egui::FullOutput) -> BTreeSet<egui::TextureId> {
        ctx.tessellate(out.shapes.clone(), out.pixels_per_point)
            .into_iter()
            .filter_map(|clipped| match clipped.primitive {
                Primitive::Mesh(mesh) => Some(mesh.texture_id),
                Primitive::Callback(_) => None,
            })
            .collect()
    }

    let ctx = egui::Context::default();
    let mut handle: Option<egui::TextureHandle> = None;

    // Two frames, each mirroring the app's per-frame slice lifecycle: update
    // the persistent texture in place, paint it, then a second in-frame render
    // updates it again in place — never reallocating.
    for _ in 0..2 {
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                match handle.as_mut() {
                    Some(existing) => {
                        existing.set(slice_image(), egui::TextureOptions::LINEAR);
                    }
                    None => {
                        handle = Some(ctx.load_texture(
                            "slice",
                            slice_image(),
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
                let id = handle.as_ref().expect("handle").id();
                ui.image((id, egui::vec2(4.0, 4.0)));
                // Second render pass (mirrors the post-input render-pending
                // pass): update in place, do NOT reallocate.
                handle
                    .as_mut()
                    .expect("handle")
                    .set(slice_image(), egui::TextureOptions::LINEAR);
            });
        });

        let painted = painted_ids(&ctx, &out);
        let freed: BTreeSet<_> = out.textures_delta.free.iter().copied().collect();
        assert!(
            painted.is_disjoint(&freed),
            "a painted texture id was freed the same frame: painted={painted:?} freed={freed:?}"
        );
    }
}

/// Keeps the invariant test above honest: the PRE-FIX pattern (a fresh
/// `load_texture` per render, dropping the just-painted handle) really does
/// put the painted id into this frame's `textures_delta.free` — exactly the
/// condition egui-wgpu destroys before submit. If this ever stops
/// reproducing, the guard test has gone vacuous.
#[test]
fn load_texture_per_render_frees_the_painted_id_in_frame() {
    use egui::epaint::Primitive;
    use std::collections::BTreeSet;

    let ctx = egui::Context::default();
    let mut handle: Option<egui::TextureHandle> = None;
    let out = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            // First render + paint.
            handle = Some(ctx.load_texture("slice", slice_image(), egui::TextureOptions::LINEAR));
            let painted_id = handle.as_ref().expect("handle").id();
            ui.image((painted_id, egui::vec2(4.0, 4.0)));
            // Second render REALLOCATES: drops the just-painted handle -> free.
            handle = Some(ctx.load_texture("slice", slice_image(), egui::TextureOptions::LINEAR));
        });
    });

    let painted: BTreeSet<_> = ctx
        .tessellate(out.shapes.clone(), out.pixels_per_point)
        .into_iter()
        .filter_map(|clipped| match clipped.primitive {
            Primitive::Mesh(mesh) => Some(mesh.texture_id),
            Primitive::Callback(_) => None,
        })
        .collect();
    let freed: BTreeSet<_> = out.textures_delta.free.iter().copied().collect();
    assert!(
        !painted.is_disjoint(&freed),
        "pre-fix load_texture-per-render must free the painted id in-frame (repro sanity)"
    );
}

#[test]
fn section_display_defaults_to_lines_and_resets_on_disable() {
    let mut tool = CutTool::default();
    assert_eq!(
        tool.section.display_mode(),
        SectionDisplay::Lines,
        "Lines is the default"
    );
    assert!(tool.section.magnet(), "magnet is on by default");

    tool.enable();
    tool.section.set_display_mode(SectionDisplay::Mesh);
    tool.section.set_magnet(false);
    // Prefs persist across pose changes within one cut session.
    let eye = Vec3::new(0.0, 0.0, 100.0);
    tool.update(&hover(Vec3::new(1.0, 0.0, 0.0)), eye);
    assert_eq!(
        tool.section.display_mode(),
        SectionDisplay::Mesh,
        "mode persists in session"
    );
    assert!(!tool.section.magnet(), "magnet persists in session");

    // Disabling the tool restores the defaults.
    tool.disable();
    assert_eq!(tool.section.display_mode(), SectionDisplay::Lines);
    assert!(tool.section.magnet());
}

#[test]
fn lines_mode_shows_the_panel_from_the_live_pose_without_a_render() {
    let mut tool = CutTool::default(); // Lines by default.
    tool.enable();
    assert!(!tool.slice_visible(), "no pose yet, nothing to show");
    assert!(!tool.wants_offscreen_slice(), "Lines skips the GPU slice");

    let eye = Vec3::new(0.0, 0.0, 100.0);
    tool.update(&hover(Vec3::ZERO), eye);
    assert!(
        tool.slice_visible(),
        "Lines panel shows straight from the live pose, no slice render needed"
    );
    assert!(
        tool.section.live_cam().is_some(),
        "a live cam is derivable without an offscreen render"
    );
}

#[test]
fn mesh_mode_wants_the_offscreen_slice() {
    let mut tool = CutTool::default();
    tool.enable();
    tool.section.set_display_mode(SectionDisplay::Mesh);
    assert!(tool.wants_offscreen_slice());
    // Entering Mesh hides any stale slice until a fresh render lands.
    assert!(!tool.section.slice_ready());
    assert!(tool.section.needs_render());
}

fn probe_pose(center: Vec3) -> DiscPose {
    DiscPose {
        center,
        plane_normal: Vec3::Z,
        radius_mm: 5.0,
    }
}

#[test]
fn probe_plant_is_world_fixed_seeds_the_thickness_and_keeps_the_main_view_whole() {
    let mut tool = CutTool::default();
    let pose = probe_pose(Vec3::new(2.0, 0.0, 0.0));
    tool.plant_from_probe(
        pose,
        true,
        SliceProbe {
            entry: Vec3::new(2.0, 0.0, 0.0),
            exit: Vec3::new(2.0, 2.0, 0.0),
            thickness_mm: 2.0,
        },
    );
    assert!(tool.is_active() && tool.is_planted() && tool.is_probe_linked());
    assert_eq!(tool.pose().expect("pose"), pose);
    // Additional view, not a slice of the main model: the viewport clip stays
    // OFF so the 3D model (and its thickness marker) stays whole.
    assert_eq!(tool.viewport_clip_plane(bbox()).enabled, 0);
    // The section panel shows straight from the live pose (Lines default) and
    // carries the SAME wall reading.
    assert!(tool.slice_visible());
    assert_eq!(tool.section.ruler().thickness_reading_mm(), Some(2.0));
    assert_eq!(tool.section.measure_mode(), SliceMeasureMode::Thickness);

    // An orbit (any camera basis) must not move the planted probe disc, and
    // the seeded measurement survives idle frames.
    let eye = Vec3::new(0.0, 0.0, 100.0);
    let out = tool.update(&frame(None, false), eye);
    assert!(!out.pose_changed, "orbit must not sweep the probe section");
    assert_eq!(tool.pose().expect("pose"), pose);
    assert_eq!(tool.section.ruler().thickness_reading_mm(), Some(2.0));

    tool.disable();
    assert!(!tool.is_probe_linked(), "disable clears the probe link");
}

#[test]
fn a_new_probe_re_aims_the_same_disc() {
    let mut tool = CutTool::default();
    tool.plant_from_probe(
        probe_pose(Vec3::new(2.0, 0.0, 0.0)),
        true,
        SliceProbe {
            entry: Vec3::new(2.0, 0.0, 0.0),
            exit: Vec3::new(2.0, 2.0, 0.0),
            thickness_mm: 2.0,
        },
    );
    // A second probe re-plants to the new plane and replaces the reading —
    // exactly what the owner asked for (prefer re-planting to the new probe).
    let repose = DiscPose {
        center: Vec3::new(-4.0, 1.0, 3.0),
        plane_normal: Vec3::X,
        radius_mm: 6.0,
    };
    tool.plant_from_probe(
        repose,
        false,
        SliceProbe {
            entry: Vec3::new(-4.0, 1.0, 3.0),
            exit: Vec3::new(-4.0, 1.0, 6.5),
            thickness_mm: 3.5,
        },
    );
    assert_eq!(tool.pose().expect("pose"), repose);
    assert_eq!(tool.section.ruler().thickness_reading_mm(), Some(3.5));
    assert!(tool.is_probe_linked());
}

#[test]
fn plane_change_resets_pan_and_clears_the_ruler() {
    let mut tool = CutTool::default();
    tool.enable();
    let eye = Vec3::new(0.0, 0.0, 100.0);
    tool.update(&hover(Vec3::ZERO), eye);

    // Measure on the current section plane, and pretend the user panned.
    let cam = tool.section.live_cam().expect("live cam on the first pose");
    tool.section.ruler_mut().place(cam.focus, cam);
    tool.section.ruler_mut().place(cam.focus + Vec3::Y, cam);
    assert!(tool.section.ruler().distance_mm().is_some());
    tool.section.set_pan(Vec3::new(2.0, -1.0, 0.5));

    // Move the follow disc to a different section plane.
    tool.update(&hover(Vec3::new(5.0, 3.0, 0.0)), eye);
    assert_eq!(
        tool.section.pan(),
        Vec3::ZERO,
        "plane change resets the pan"
    );
    assert!(
        tool.section.ruler().distance_mm().is_none(),
        "plane change clears the ruler in Lines mode too"
    );
}
