//! State machine for the interactive cut disc.
//!
//! The cut tool shows a small disc that stands upright on the surface under the
//! cursor (exocad "follow cursor"): the disc plane blends continuously between
//! an axial camera alignment and the local surface direction, so facets cannot
//! snap the blade. A primary click *plants* the disc; drag anywhere on its body
//! to translate, Ctrl+drag anywhere to arcball-tilt, or use the outer rim halo
//! for depth push/pull. The wheel scales the disc radius.
//!
//! This module is deliberately free of egui/renderer side effects: the viewport
//! adapter samples the current frame's pointer/keyboard/camera facts into a
//! [`CutFrameInput`], calls [`CutManipulator::update`], and reacts to the
//! returned [`CutUpdate`]. The stateless geometry (orientation, smoothing,
//! handle hit-test, transforms) lives in [`crate::cut_geometry`]; both halves
//! are exhaustively unit-tested without a live context.

use crate::cut_geometry::{
    apply_drag, begin_drag, camera_keep_side, follow_plane_normal, hover_cursor, scale_radius,
    smooth_normal,
};
use eframe::egui::Pos2;
use glam::Vec3;

/// Disc radius when the tool first arms (mm).
pub(crate) const DEFAULT_DISC_RADIUS_MM: f32 = 8.0;
/// Smallest disc radius the wheel can reach (mm).
pub(crate) const MIN_DISC_RADIUS_MM: f32 = 2.0;
/// Largest disc radius the wheel can reach (mm).
pub(crate) const MAX_DISC_RADIUS_MM: f32 = 60.0;
/// Radius multiplier per wheel notch.
pub(crate) const RADIUS_WHEEL_STEP: f32 = 1.1;
/// Exponential blend toward the freshly sampled normal, per hover frame.
pub(crate) const NORMAL_SMOOTH_BLEND: f32 = 0.68;
/// Grab radius (screen px) for the center translate handle.
pub(crate) const CENTER_GRAB_RADIUS_PX: f32 = 14.0;
/// Grab tolerance (screen px) around the rim ring for push/pull handles.
pub(crate) const RIM_GRAB_RADIUS_PX: f32 = 11.0;

/// Pose of the cut disc in world space. The disc lies in the plane through
/// `center` with `plane_normal`; the same plane is the GPU clip / section plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DiscPose {
    /// Disc center (world).
    pub(crate) center: Vec3,
    /// Unit plane normal (world). Perpendicular to the surface normal in follow.
    pub(crate) plane_normal: Vec3,
    /// Disc radius (mm).
    pub(crate) radius_mm: f32,
}

/// A hit mesh's principal-axis frame, in WORLD space (see
/// [`occluview_core::Mesh::principal_frame_cached`]) — a STABLE signal,
/// constant for a given mesh regardless of cursor position, that the follow
/// disc derives its orientation from instead of the hit triangle's local
/// normal: the LOCAL direction from `centroid` to the hit point, projected
/// onto the `axis0`/`axis1` plane, rotates smoothly as the cursor moves
/// around a dental arch or bridge span — reducing to (roughly) `axis0` at
/// the arch's left/right extremes and adapting continuously in between,
/// instead of staying fixed for the whole mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ArchFrame {
    /// PCA centroid, world space.
    pub(crate) centroid: Vec3,
    /// Greatest-variance axis, world space, unit length.
    pub(crate) axis0: Vec3,
    /// Second-greatest-variance axis, world space, unit length.
    pub(crate) axis1: Vec3,
}

/// A freshly sampled hover point on the mesh, feeding the follow disc.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SurfaceSample {
    /// World hit position under the cursor.
    pub(crate) point: Vec3,
    /// Raw (unsmoothed) averaged world surface normal at the hit triangle.
    pub(crate) normal: Vec3,
    /// The hit mesh's own principal-axis frame, in world space. `None` for a
    /// point cloud or a mesh too small to have a well-defined frame, in which
    /// case the disc falls back to the local surface-normal-driven
    /// orientation.
    pub(crate) arch_frame: Option<ArchFrame>,
}

/// A cursor affordance the overlay should show this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CutCursor {
    /// No special cursor.
    #[default]
    Default,
    /// Hovering a grabbable handle.
    Grab,
    /// Actively dragging a handle.
    Grabbing,
}

/// An in-progress drag on a planted disc, holding its press-time anchors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DiscDrag {
    /// Center handle: translate the disc within the screen plane.
    Translate { center0: Vec3, ray_origin0: Vec3 },
    /// Rim handle: push/pull the disc along its plane normal.
    PushPull { center0: Vec3, ray_origin0: Vec3 },
    /// Ctrl+drag: arcball-tilt the disc plane about its center.
    Tilt { normal0: Vec3, pointer0: Pos2 },
}

/// The cut tool's mode.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CutMode {
    /// Not armed.
    Off,
    /// Armed and following the cursor. `pose` is `None` when the cursor is off
    /// the mesh; `smoothed_normal` carries the temporal filter state.
    Follow {
        pose: Option<DiscPose>,
        smoothed_normal: Option<Vec3>,
    },
    /// Planted at a fixed WORLD pose. The disc stays put on the model while the
    /// main-viewport camera orbits freely — an orbit does NOT sweep the section
    /// (owner rule). `keep_positive` is the clip side chosen at plant time
    /// (flipped by F); `drag` is any in-progress handle drag. Re-aim the cut by
    /// dragging the disc handles (move / Ctrl-tilt / rim push), not the camera.
    Planted {
        pose: DiscPose,
        keep_positive: bool,
        drag: Option<DiscDrag>,
    },
}

/// One frame of pointer/keyboard/camera facts the machine consumes.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct CutFrameInput {
    /// Current pointer position (screen), if any.
    pub(crate) pointer: Option<Pos2>,
    /// The pointer is over the viewport and not over the cut strip/panel.
    pub(crate) over_viewport: bool,
    /// Primary button went down this frame.
    pub(crate) primary_pressed: bool,
    /// Primary button is currently held.
    pub(crate) primary_down: bool,
    /// Ctrl (command) is held.
    pub(crate) ctrl: bool,
    /// Esc was pressed this frame.
    pub(crate) escape: bool,
    /// F (flip) was pressed this frame.
    pub(crate) flip: bool,
    /// Wheel travel this frame in notches (`+` = away/scroll-up).
    pub(crate) wheel_notches: f32,
    /// Camera eye (world).
    pub(crate) eye: Vec3,
    /// Camera forward / view direction (world, unit).
    pub(crate) view_dir: Vec3,
    /// Camera right axis (world, unit). Used to orient the follow-disc plane
    /// and to map the Ctrl+drag arcball tilt into world space.
    pub(crate) camera_right: Vec3,
    /// Camera up axis (world, unit). Used by the arcball tilt handle.
    pub(crate) camera_up: Vec3,
    /// Pointer world-ray origin (for the orthographic viewer this tracks the
    /// cursor within the view plane).
    pub(crate) ray_origin: Vec3,
    /// Surface hover sample under the cursor, if the ray hit the mesh.
    pub(crate) surface_hit: Option<SurfaceSample>,
    /// Projected disc center (screen), for handle hit-testing.
    pub(crate) disc_center_screen: Option<Pos2>,
    /// Projected disc radius (screen px).
    pub(crate) disc_radius_screen: f32,
}

/// What changed this frame; the adapter reacts to these.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CutUpdate {
    /// The clip/section/slice plane changed and needs a refresh.
    pub(crate) pose_changed: bool,
    /// The cut consumed the pointer this frame (gate camera orbit/pan/retarget).
    pub(crate) consumed_pointer: bool,
    /// The disc was planted this frame.
    pub(crate) planted: bool,
    /// The disc unplanted back to follow this frame (first Esc).
    pub(crate) unplanted: bool,
    /// Cut mode exited this frame (Esc from follow).
    pub(crate) exited: bool,
    /// Cursor affordance to show.
    pub(crate) cursor: CutCursor,
}

/// Stateful, headlessly-testable manipulator. Owns the mode and the remembered
/// radius that carries across follow/planted transitions.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CutManipulator {
    mode: CutMode,
    radius_mm: f32,
}

impl Default for CutManipulator {
    fn default() -> Self {
        Self {
            mode: CutMode::Off,
            radius_mm: DEFAULT_DISC_RADIUS_MM,
        }
    }
}

impl CutManipulator {
    /// Arm the tool into follow mode, keeping the remembered radius.
    pub(crate) fn arm(&mut self) {
        self.mode = CutMode::Follow {
            pose: None,
            smoothed_normal: None,
        };
    }

    /// Disarm the tool.
    pub(crate) fn disarm(&mut self) {
        self.mode = CutMode::Off;
    }

    /// Plant the disc programmatically at a fixed WORLD `pose` (the thickness
    /// probe driving the Section view). This produces exactly the same
    /// world-fixed [`CutMode::Planted`] a manual click plant does — an orbit
    /// leaves it untouched — and adopts the pose's radius as the remembered one.
    pub(crate) fn plant_pose(&mut self, pose: DiscPose, keep_positive: bool) {
        self.radius_mm = pose.radius_mm;
        self.mode = CutMode::Planted {
            pose,
            keep_positive,
            drag: None,
        };
    }

    /// Set the operator-selected radius without synthesizing a pointer gesture.
    pub(crate) fn set_radius_mm(&mut self, radius_mm: f32) -> bool {
        if !radius_mm.is_finite()
            || radius_mm <= 0.0
            || self.radius_mm.to_bits() == radius_mm.to_bits()
        {
            return false;
        }
        let pose = match &mut self.mode {
            CutMode::Follow { pose, .. } => pose.as_mut(),
            CutMode::Planted { pose, .. } => Some(pose),
            CutMode::Off => None,
        };
        let Some(pose) = pose else {
            return false;
        };
        self.radius_mm = radius_mm;
        pose.radius_mm = radius_mm;
        true
    }

    /// Whether the tool is armed (follow or planted).
    pub(crate) fn is_active(&self) -> bool {
        !matches!(self.mode, CutMode::Off)
    }

    /// Whether a disc is currently planted.
    pub(crate) fn is_planted(&self) -> bool {
        matches!(self.mode, CutMode::Planted { .. })
    }

    /// The current disc pose (follow-with-hover or planted), if any.
    pub(crate) fn pose(&self) -> Option<DiscPose> {
        match &self.mode {
            CutMode::Planted { pose, .. } => Some(*pose),
            CutMode::Follow { pose, .. } => *pose,
            CutMode::Off => None,
        }
    }

    /// The clip plane derived from the current disc, as `(normal, distance)`
    /// with the normal pointing toward the kept side. `eye` decides the kept
    /// side while following (kept side = camera side); planted uses its frozen
    /// (F-flippable) choice.
    pub(crate) fn clip(&self, eye: Vec3) -> Option<(Vec3, f32)> {
        let (pose, keep_positive) = match &self.mode {
            CutMode::Planted {
                pose,
                keep_positive,
                ..
            } => (*pose, *keep_positive),
            CutMode::Follow {
                pose: Some(pose), ..
            } => (*pose, camera_keep_side(pose, eye)),
            _ => return None,
        };
        let sign = if keep_positive { 1.0 } else { -1.0 };
        let normal = (pose.plane_normal * sign).normalize_or_zero();
        if normal.length_squared() <= f32::EPSILON {
            return None;
        }
        Some((normal, normal.dot(pose.center)))
    }

    /// Advance one frame.
    pub(crate) fn update(&mut self, input: &CutFrameInput) -> CutUpdate {
        let mut out = CutUpdate::default();
        let mode = std::mem::replace(&mut self.mode, CutMode::Off);
        self.mode = match mode {
            CutMode::Off => CutMode::Off,
            CutMode::Follow {
                pose,
                smoothed_normal,
            } => self.step_follow(pose, smoothed_normal, input, &mut out),
            CutMode::Planted {
                pose,
                keep_positive,
                drag,
            } => self.step_planted(pose, keep_positive, drag, input, &mut out),
        };
        out
    }

    fn step_follow(
        &mut self,
        pose: Option<DiscPose>,
        smoothed_normal: Option<Vec3>,
        input: &CutFrameInput,
        out: &mut CutUpdate,
    ) -> CutMode {
        if input.escape {
            out.exited = true;
            out.pose_changed = pose.is_some();
            return CutMode::Off;
        }
        if input.wheel_notches != 0.0 {
            self.radius_mm = scale_radius(self.radius_mm, input.wheel_notches);
            out.pose_changed = true;
        }
        let Some(sample) = input.surface_hit else {
            // Cursor left the mesh: the follow disc disappears.
            out.pose_changed |= pose.is_some();
            return CutMode::Follow {
                pose: None,
                smoothed_normal,
            };
        };
        let raw = follow_plane_normal(
            sample.arch_frame,
            sample.point,
            sample.normal,
            input.view_dir,
            input.camera_right,
        );
        let smoothed = smooth_normal(smoothed_normal, raw, NORMAL_SMOOTH_BLEND);
        let new_pose = DiscPose {
            center: sample.point,
            plane_normal: smoothed,
            radius_mm: self.radius_mm,
        };
        if input.primary_pressed && input.over_viewport {
            let keep_positive = camera_keep_side(&new_pose, input.eye);
            out.planted = true;
            out.pose_changed = true;
            out.consumed_pointer = true;
            out.cursor = CutCursor::Grabbing;
            // Plant the pose in WORLD space: it stays fixed while the camera
            // orbits (owner rule — an orbit must not sweep the section).
            return CutMode::Planted {
                pose: new_pose,
                keep_positive,
                drag: None,
            };
        }
        out.pose_changed |= pose != Some(new_pose);
        CutMode::Follow {
            pose: Some(new_pose),
            smoothed_normal: Some(smoothed),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn step_planted(
        &mut self,
        prev_pose: DiscPose,
        mut keep_positive: bool,
        mut drag: Option<DiscDrag>,
        input: &CutFrameInput,
        out: &mut CutUpdate,
    ) -> CutMode {
        if input.escape {
            out.unplanted = true;
            out.pose_changed = true;
            return CutMode::Follow {
                pose: None,
                smoothed_normal: None,
            };
        }
        // World-fixed: the planted pose stays put on the model. The main-viewport
        // camera orbits freely without touching it (owner rule) — only the disc
        // handles, F, and Ctrl+wheel re-author the cut.
        let mut pose = prev_pose;
        if input.flip {
            keep_positive = !keep_positive;
            out.pose_changed = true;
        }
        if input.wheel_notches != 0.0 {
            // Ctrl+wheel over the panel resizes the disc.
            self.radius_mm = scale_radius(pose.radius_mm, input.wheel_notches);
            pose.radius_mm = self.radius_mm;
            out.pose_changed = true;
        }
        match drag {
            None => {
                if input.primary_pressed && input.over_viewport {
                    if let Some(begun) = begin_drag(&pose, input) {
                        drag = Some(begun);
                        out.consumed_pointer = true;
                        out.cursor = CutCursor::Grabbing;
                    }
                } else {
                    out.cursor = hover_cursor(&pose, input);
                }
            }
            Some(active) => {
                out.consumed_pointer = true;
                out.cursor = CutCursor::Grabbing;
                if input.primary_down {
                    apply_drag(&mut pose, active, input);
                    self.radius_mm = pose.radius_mm;
                    out.pose_changed = true;
                } else {
                    drag = None;
                }
            }
        }
        if pose_moved(&prev_pose, &pose) {
            out.pose_changed = true;
        }
        CutMode::Planted {
            pose,
            keep_positive,
            drag,
        }
    }
}

/// Whether two poses differ enough to warrant a re-render (guards against
/// spurious per-frame re-renders from float noise while the camera is idle).
///
/// pub(crate): the tool ALSO compares the live pose against the pose the
/// visible slice was rendered from — frame-to-frame deltas alone let a slow
/// sub-epsilon orbit sweep the section 180° without ever re-rendering.
pub(crate) fn pose_moved(a: &DiscPose, b: &DiscPose) -> bool {
    const POS_EPS: f32 = 1.0e-4;
    const NORMAL_EPS: f32 = 1.0e-5;
    a.center.distance_squared(b.center) > POS_EPS * POS_EPS
        || a.plane_normal.dot(b.plane_normal) < 1.0 - NORMAL_EPS
        || (a.radius_mm - b.radius_mm).abs() > POS_EPS
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::expect_used, clippy::unnecessary_wraps)]
    use super::*;
    use crate::cut_geometry::scale_radius;
    use eframe::egui::pos2;

    fn base_input() -> CutFrameInput {
        CutFrameInput {
            pointer: Some(pos2(200.0, 200.0)),
            over_viewport: true,
            primary_pressed: false,
            primary_down: false,
            ctrl: false,
            escape: false,
            flip: false,
            wheel_notches: 0.0,
            eye: Vec3::new(0.0, 0.0, 100.0),
            view_dir: Vec3::NEG_Z,
            camera_right: Vec3::X,
            camera_up: Vec3::Y,
            ray_origin: Vec3::new(0.0, 0.0, 100.0),
            surface_hit: None,
            disc_center_screen: Some(pos2(200.0, 200.0)),
            disc_radius_screen: 40.0,
        }
    }

    fn sample(point: Vec3, normal: Vec3) -> Option<SurfaceSample> {
        Some(SurfaceSample {
            point,
            normal,
            arch_frame: None,
        })
    }

    fn plant(m: &mut CutManipulator) {
        m.arm();
        m.update(&CutFrameInput {
            surface_hit: sample(Vec3::ZERO, Vec3::Y),
            primary_pressed: true,
            primary_down: true,
            ..base_input()
        });
    }

    #[test]
    fn hover_follows_the_surface_and_updates_the_pose() {
        let mut m = CutManipulator::default();
        m.arm();
        let out = m.update(&CutFrameInput {
            surface_hit: sample(Vec3::new(1.0, 2.0, 3.0), Vec3::Y),
            ..base_input()
        });
        assert!(out.pose_changed);
        let pose = m.pose().expect("follow pose");
        assert_eq!(pose.center, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(pose.radius_mm, DEFAULT_DISC_RADIUS_MM);
    }

    #[test]
    fn leaving_the_mesh_drops_the_follow_disc() {
        let mut m = CutManipulator::default();
        m.arm();
        m.update(&CutFrameInput {
            surface_hit: sample(Vec3::ZERO, Vec3::Y),
            ..base_input()
        });
        assert!(m.pose().is_some());
        let out = m.update(&base_input());
        assert!(out.pose_changed);
        assert!(m.pose().is_none());
    }

    #[test]
    fn primary_press_on_the_mesh_plants_the_disc() {
        let mut m = CutManipulator::default();
        m.arm();
        let out = m.update(&CutFrameInput {
            surface_hit: sample(Vec3::ZERO, Vec3::Y),
            primary_pressed: true,
            primary_down: true,
            ..base_input()
        });
        assert!(out.planted);
        assert!(out.consumed_pointer);
        assert!(m.is_planted());
    }

    #[test]
    fn drag_on_a_planted_disc_consumes_the_pointer_and_moves_it() {
        let mut m = CutManipulator::default();
        plant(&mut m);
        let before = m.pose().expect("pose").center;
        // Press the center handle then drag the ray origin sideways.
        m.update(&CutFrameInput {
            primary_pressed: true,
            primary_down: true,
            ..base_input()
        });
        let out = m.update(&CutFrameInput {
            primary_down: true,
            ray_origin: Vec3::new(5.0, 0.0, 100.0),
            ..base_input()
        });
        assert!(out.consumed_pointer);
        assert!(out.pose_changed);
        assert!(m.pose().expect("pose").center.distance(before) > 1.0);
    }

    #[test]
    fn esc_ladder_planted_then_follow_then_off() {
        let mut m = CutManipulator::default();
        plant(&mut m);
        assert!(m.is_planted());
        let out = m.update(&CutFrameInput {
            escape: true,
            ..base_input()
        });
        assert!(out.unplanted && !out.exited);
        assert!(m.is_active() && !m.is_planted());
        let out = m.update(&CutFrameInput {
            escape: true,
            ..base_input()
        });
        assert!(out.exited);
        assert!(!m.is_active());
    }

    #[test]
    fn f_flips_the_kept_side_when_planted() {
        let mut m = CutManipulator::default();
        plant(&mut m);
        let before = m.clip(Vec3::new(0.0, 0.0, 100.0)).expect("clip");
        m.update(&CutFrameInput {
            flip: true,
            ..base_input()
        });
        let after = m.clip(Vec3::new(0.0, 0.0, 100.0)).expect("clip");
        assert_eq!(after.0, -before.0);
        assert_eq!(after.1, -before.1);
    }

    #[test]
    fn wheel_scales_the_planted_radius() {
        let mut m = CutManipulator::default();
        plant(&mut m);
        let r0 = m.pose().expect("pose").radius_mm;
        m.update(&CutFrameInput {
            wheel_notches: 2.0,
            ..base_input()
        });
        let r1 = m.pose().expect("pose").radius_mm;
        assert!(r1 > r0);
        assert!((r1 - scale_radius(r0, 2.0)).abs() < 1e-4);
    }

    #[test]
    fn explicit_radius_updates_a_planted_disc_without_rearming_or_clamping_to_wheel_minimum() {
        let mut m = CutManipulator::default();
        plant(&mut m);

        assert!(m.set_radius_mm(0.75));
        assert!(m.is_planted());
        assert_eq!(
            m.pose().expect("pose").radius_mm.to_bits(),
            0.75_f32.to_bits()
        );
        assert!(!m.set_radius_mm(f32::NAN));
        assert!(!m.set_radius_mm(0.0));
    }

    #[test]
    fn plant_pose_produces_a_world_fixed_planted_disc() {
        // The programmatic plant (thickness-probe path) must yield the same
        // world-fixed Planted invariants a manual plant does: planted, the exact
        // pose, and immune to a camera orbit.
        let mut m = CutManipulator::default();
        m.arm();
        let pose = DiscPose {
            center: Vec3::new(4.0, -1.0, 2.0),
            plane_normal: Vec3::new(0.0, 0.0, 1.0),
            radius_mm: 5.0,
        };
        m.plant_pose(pose, true);
        assert!(m.is_planted());
        assert_eq!(m.pose().expect("pose"), pose);
        // An orbit (new camera basis) must not move the planted pose.
        let out = m.update(&CutFrameInput {
            camera_right: Vec3::NEG_Z,
            view_dir: Vec3::NEG_X,
            ..base_input()
        });
        assert!(
            !out.pose_changed,
            "orbit must not sweep a planted probe disc"
        );
        assert_eq!(m.pose().expect("pose"), pose);
    }

    #[test]
    fn follow_keeps_the_camera_side() {
        let mut m = CutManipulator::default();
        m.arm();
        m.update(&CutFrameInput {
            surface_hit: sample(Vec3::ZERO, Vec3::Y),
            ..base_input()
        });
        // Disc normal is +X (surface +Y, view -Z). Eye on +X keeps +X.
        let clip = m.clip(Vec3::new(50.0, 0.0, 0.0)).expect("clip");
        assert!(
            clip.0.x > 0.0,
            "kept normal should face the camera: {}",
            clip.0
        );
    }
}
