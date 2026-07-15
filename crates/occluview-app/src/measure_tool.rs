// File-size exception (>500): the probe geometry keeps its hostile test meshes
// (nested-cube shell, NaN-poisoned, degenerate) next to the math it guards.

//! Viewport measurement tools: the two-point ruler and the wall-thickness probe.
//!
//! State machine + geometry only, pure and unit-tested. Painting lives in
//! [`crate::measure_overlay`], the input adapter in `app::app_viewport`, the
//! toolbar toggles in `app::app_dialogs`. Anchors are WORLD-SPACE points on the
//! mesh surface: they re-project through the live camera every frame, so the
//! drawn segment orbits/zooms/pans with the model (exocad ruler behavior).
//!
//! The thickness probe is honest, not a proxy: from the picked surface point it
//! casts a ray INWARD (opposite the barycentric-interpolated surface normal at
//! the hit) against the SAME layer's triangles; the nearest exit intersection is
//! the local wall thickness (the normal chord). No exit means an open scan, and
//! the reading says so instead of inventing a number.

use glam::{Vec3, Vec3A};
use occluview_core::SceneMesh;

/// Ignore intersections closer than this to the probe origin (mm), so the probe
/// never reports the entry triangle's edge-neighbors as an "exit".
const SELF_HIT_EPS_MM: f32 = 1.0e-3;

/// Which measurement tool is armed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MeasureMode {
    /// Two clicks on the surface; distance in millimeters between them.
    Ruler,
    /// One click on a shell; local wall thickness along the inward normal.
    Thickness,
}

/// One completed two-point measurement (world-space anchors).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RulerMeasurement {
    pub(crate) a: Vec3,
    pub(crate) b: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RulerEndpoint {
    A,
    B,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RulerAnchorRef {
    pub(crate) ruler_index: usize,
    pub(crate) endpoint: RulerEndpoint,
}

impl RulerMeasurement {
    /// Straight-line distance in millimeters (`f64` accumulation).
    pub(crate) fn distance_mm(&self) -> f64 {
        let dx = f64::from(self.a.x) - f64::from(self.b.x);
        let dy = f64::from(self.a.y) - f64::from(self.b.y);
        let dz = f64::from(self.a.z) - f64::from(self.b.z);
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

/// The honest outcome of one thickness probe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ThicknessReading {
    /// The inward ray crossed the opposite wall: the normal chord.
    Wall { exit: Vec3, thickness_mm: f32 },
    /// No opposite wall along the inward normal (open scan).
    Open,
}

/// A placed thickness probe: the clicked entry point plus its reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ThicknessProbe {
    pub(crate) entry: Vec3,
    pub(crate) reading: ThicknessReading,
}

/// The measurement tool state: at most one armed mode, completed rulers, an
/// optional pending first anchor, and the latest thickness probe (a new click
/// re-probes, replacing the marker).
#[derive(Default)]
pub(crate) struct MeasureTool {
    mode: Option<MeasureMode>,
    pending: Option<Vec3>,
    rulers: Vec<RulerMeasurement>,
    probe: Option<ThicknessProbe>,
    dragged_anchor: Option<RulerAnchorRef>,
}

impl MeasureTool {
    pub(crate) fn is_active(&self) -> bool {
        self.mode.is_some()
    }

    pub(crate) fn mode(&self) -> Option<MeasureMode> {
        self.mode
    }

    /// Arm `mode`. Switching tools keeps completed overlays (RMB clears them)
    /// but drops a half-made ruler pair: a pending anchor must not bridge a
    /// tool switch.
    pub(crate) fn arm(&mut self, mode: MeasureMode) {
        self.mode = Some(mode);
        self.pending = None;
        self.dragged_anchor = None;
    }

    /// Exit the tool. The overlays go with it: once the tool is gone there is
    /// no owner left to clear stale labels over a changing scene.
    pub(crate) fn disarm(&mut self) {
        self.mode = None;
        self.pending = None;
        self.rulers.clear();
        self.probe = None;
        self.dragged_anchor = None;
    }

    /// Place a ruler anchor at a picked surface `point`. The first click of a
    /// pair returns `None`; the second completes a measurement and returns its
    /// distance in millimeters. Non-finite points are refused.
    pub(crate) fn place_ruler_point(&mut self, point: Vec3) -> Option<f64> {
        if !point.is_finite() {
            return None;
        }
        match self.pending.take() {
            None => {
                self.pending = Some(point);
                None
            }
            Some(first) => {
                let measurement = RulerMeasurement { a: first, b: point };
                let distance_mm = measurement.distance_mm();
                self.rulers.push(measurement);
                Some(distance_mm)
            }
        }
    }

    /// Replace the thickness probe (a repeated click re-probes).
    pub(crate) fn set_probe(&mut self, probe: ThicknessProbe) {
        self.probe = Some(probe);
    }

    pub(crate) fn clear_probe(&mut self) {
        self.probe = None;
    }

    pub(crate) fn begin_ruler_drag(&mut self, anchor: RulerAnchorRef) -> bool {
        if anchor.ruler_index >= self.rulers.len() {
            return false;
        }
        self.dragged_anchor = Some(anchor);
        true
    }

    pub(crate) fn dragged_ruler_anchor(&self) -> Option<RulerAnchorRef> {
        self.dragged_anchor
    }

    pub(crate) fn update_ruler_drag(&mut self, point: Vec3) -> Option<f64> {
        if !point.is_finite() {
            return None;
        }
        let anchor = self.dragged_anchor?;
        let ruler = self.rulers.get_mut(anchor.ruler_index)?;
        match anchor.endpoint {
            RulerEndpoint::A => ruler.a = point,
            RulerEndpoint::B => ruler.b = point,
        }
        Some(ruler.distance_mm())
    }

    pub(crate) fn end_ruler_drag(&mut self) {
        self.dragged_anchor = None;
    }

    /// Drop every measurement overlay, keeping the tool armed (the RMB
    /// gesture). Returns whether anything was actually cleared.
    pub(crate) fn clear_measurements(&mut self) -> bool {
        let had = self.pending.is_some() || !self.rulers.is_empty() || self.probe.is_some();
        self.pending = None;
        self.rulers.clear();
        self.probe = None;
        self.dragged_anchor = None;
        had
    }

    pub(crate) fn pending_anchor(&self) -> Option<Vec3> {
        self.pending
    }

    pub(crate) fn rulers(&self) -> &[RulerMeasurement] {
        &self.rulers
    }

    pub(crate) fn probe(&self) -> Option<&ThicknessProbe> {
        self.probe.as_ref()
    }
}

/// Outcome of clicking a Measure toolbar entry: the next armed mode (clicking
/// the active one toggles it off) and whether the cut tool must be disabled
/// first — the viewport-owning tools are mutually exclusive.
pub(crate) fn apply_menu_toggle(
    current: Option<MeasureMode>,
    cut_active: bool,
    clicked: MeasureMode,
) -> (Option<MeasureMode>, bool) {
    let next = if current == Some(clicked) {
        None
    } else {
        Some(clicked)
    };
    (next, next.is_some() && cut_active)
}

/// Whether the Measure toolbar entries are clickable: a pickable (visible,
/// triangle) layer must exist and no edit session may own the pointer.
pub(crate) fn measure_menu_enabled(has_pickable_layer: bool, edit_session_active: bool) -> bool {
    has_pickable_layer && !edit_session_active
}

/// `12.34 mm` labels (two decimals). Non-finite input (poisoned geometry)
/// reads as `n/a` — never a `NaN mm` label.
pub(crate) fn format_mm(mm: f64) -> String {
    if mm.is_finite() {
        format!("{mm:.2} mm")
    } else {
        "n/a".to_string()
    }
}

/// Local wall thickness at a picked surface point of `entry`.
///
/// Casts a ray from `point` opposite the interpolated surface normal of
/// triangle `triangle_index` (into the solid) against the same layer's
/// triangles; the nearest exit intersection past the self-hit epsilon is the
/// wall. Returns `None` when the surface normal is genuinely undeterminable
/// (degenerate geometry) — the probe refuses instead of guessing.
pub(crate) fn probe_wall_thickness(
    entry: &SceneMesh,
    triangle_index: usize,
    point: Vec3,
) -> Option<ThicknessProbe> {
    if !point.is_finite() {
        return None;
    }
    let base = triangle_index.checked_mul(3)?;
    let tri = entry.mesh.indices().get(base..base + 3)?;
    let world = world_triangle(entry, tri)?;
    let inward = -surface_normal_at(entry, tri, &world, point)?;
    let reading = match nearest_exit(entry, triangle_index, point, inward) {
        Some((thickness_mm, exit)) => ThicknessReading::Wall { exit, thickness_mm },
        None => ThicknessReading::Open,
    };
    Some(ThicknessProbe {
        entry: point,
        reading,
    })
}

/// The three world-space corners of an index triple, or `None` on a broken
/// index (defensive; scene meshes validate indices on construction).
fn world_triangle(entry: &SceneMesh, tri: &[u32]) -> Option<[Vec3; 3]> {
    let vertices = entry.mesh.vertices();
    let mut out = [Vec3::ZERO; 3];
    for (corner, &raw) in out.iter_mut().zip(tri) {
        let vertex = vertices.get(raw as usize)?;
        *corner = entry
            .transform
            .transform_point3(Vec3::from_array(vertex.position));
    }
    Some(out)
}

/// Interpolated world-space surface normal at `point` on the triangle, with an
/// honest fallback ladder: barycentric vertex normals, then the geometric face
/// normal, then `None` (degenerate — refuse rather than guess a direction).
fn surface_normal_at(
    entry: &SceneMesh,
    tri: &[u32],
    world: &[Vec3; 3],
    point: Vec3,
) -> Option<Vec3> {
    let vertices = entry.mesh.vertices();
    let weights = barycentric_weights(point, world[0], world[1], world[2]);
    let mut local = Vec3::ZERO;
    for (&raw, &weight) in tri.iter().zip(&weights) {
        local += Vec3::from_array(vertices.get(raw as usize)?.normal) * weight;
    }
    let normal_matrix = entry.transform.matrix3.inverse().transpose();
    let interpolated = Vec3::from(normal_matrix * Vec3A::from(local)).normalize_or_zero();
    if interpolated.is_finite() && interpolated.length_squared() > 0.5 {
        return Some(interpolated);
    }
    let face = (world[1] - world[0])
        .cross(world[2] - world[0])
        .normalize_or_zero();
    (face.is_finite() && face.length_squared() > 0.5).then_some(face)
}

/// Barycentric weights of `point` in a triangle, clamped to the triangle and
/// renormalized. A degenerate triangle yields equal thirds (the caller's
/// face-normal fallback then decides whether the probe is possible at all).
fn barycentric_weights(point: Vec3, corner_a: Vec3, corner_b: Vec3, corner_c: Vec3) -> [f32; 3] {
    let edge0 = corner_b - corner_a;
    let edge1 = corner_c - corner_a;
    let to_point = point - corner_a;
    let d00 = edge0.dot(edge0);
    let d01 = edge0.dot(edge1);
    let d11 = edge1.dot(edge1);
    let d20 = to_point.dot(edge0);
    let d21 = to_point.dot(edge1);
    let denom = d00 * d11 - d01 * d01;
    if !denom.is_finite() || denom.abs() <= f32::EPSILON {
        return [1.0 / 3.0; 3];
    }
    let weight_b = ((d11 * d20 - d01 * d21) / denom).clamp(0.0, 1.0);
    let weight_c = ((d00 * d21 - d01 * d20) / denom).clamp(0.0, 1.0);
    let weight_a = (1.0 - weight_b - weight_c).clamp(0.0, 1.0);
    let sum = weight_a + weight_b + weight_c;
    if !sum.is_finite() || sum <= f32::EPSILON {
        return [1.0 / 3.0; 3];
    }
    [weight_a / sum, weight_b / sum, weight_c / sum]
}

/// Nearest intersection along `direction` over the layer's own triangles,
/// skipping the origin triangle and near-zero self-hits. Returns the ray
/// distance (= thickness, `direction` is unit) and the exit point.
fn nearest_exit(
    entry: &SceneMesh,
    origin_triangle: usize,
    origin: Vec3,
    direction: Vec3,
) -> Option<(f32, Vec3)> {
    let mut nearest: Option<f32> = None;
    for (tri_idx, tri) in entry.mesh.indices().chunks_exact(3).enumerate() {
        if tri_idx == origin_triangle {
            continue;
        }
        let Some(world) = world_triangle(entry, tri) else {
            continue;
        };
        let Some(t) = ray_triangle_distance(origin, direction, &world) else {
            continue;
        };
        if t > SELF_HIT_EPS_MM && nearest.is_none_or(|best| t < best) {
            nearest = Some(t);
        }
    }
    nearest.map(|t| (t, origin + direction * t))
}

/// Moller-Trumbore, double-sided (an exit wall may face either way). Returns
/// the positive ray distance; `None` on miss, degenerate triangles, or
/// non-finite input (NaN fails every range check by construction).
fn ray_triangle_distance(origin: Vec3, direction: Vec3, tri: &[Vec3; 3]) -> Option<f32> {
    const EPSILON: f32 = 1.0e-7;
    let edge0 = tri[1] - tri[0];
    let edge1 = tri[2] - tri[0];
    let determinant_cross = direction.cross(edge1);
    let determinant = edge0.dot(determinant_cross);
    if determinant.abs() <= EPSILON || determinant.is_nan() {
        return None;
    }
    let inv_determinant = 1.0 / determinant;
    let origin_to_a = origin - tri[0];
    let bary_u = origin_to_a.dot(determinant_cross) * inv_determinant;
    if !(0.0..=1.0).contains(&bary_u) {
        return None;
    }
    let bary_cross = origin_to_a.cross(edge0);
    let bary_v = direction.dot(bary_cross) * inv_determinant;
    if !(0.0..=1.0).contains(&bary_v) || bary_u + bary_v > 1.0 {
        return None;
    }
    let distance = edge1.dot(bary_cross) * inv_determinant;
    (distance.is_finite() && distance > EPSILON).then_some(distance)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::float_cmp
    )]
    use super::*;
    use occluview_core::{Mesh, Scene, Vertex};

    #[test]
    fn ruler_pairs_complete_and_accumulate() {
        let mut tool = MeasureTool::default();
        tool.arm(MeasureMode::Ruler);
        assert!(tool.is_active());
        assert!(tool.place_ruler_point(Vec3::ZERO).is_none());
        assert_eq!(tool.pending_anchor(), Some(Vec3::ZERO));
        let first = tool.place_ruler_point(Vec3::new(3.0, 4.0, 0.0));
        assert_eq!(first, Some(5.0));
        assert!(tool.pending_anchor().is_none());
        // A second pair stacks on screen alongside the first.
        assert!(tool.place_ruler_point(Vec3::new(1.0, 0.0, 0.0)).is_none());
        assert!(tool.place_ruler_point(Vec3::new(1.0, 0.0, 2.0)).is_some());
        assert_eq!(tool.rulers().len(), 2);
    }

    #[test]
    fn clicking_the_same_point_twice_reads_zero_not_nan() {
        let mut tool = MeasureTool::default();
        tool.arm(MeasureMode::Ruler);
        let p = Vec3::new(7.5, -2.0, 1.0);
        tool.place_ruler_point(p);
        let distance = tool.place_ruler_point(p).expect("completed pair");
        assert_eq!(format_mm(distance), "0.00 mm");
    }

    #[test]
    fn non_finite_clicks_are_refused() {
        let mut tool = MeasureTool::default();
        tool.arm(MeasureMode::Ruler);
        assert!(tool.place_ruler_point(Vec3::NAN).is_none());
        assert!(tool.pending_anchor().is_none());
    }

    #[test]
    fn rmb_clear_keeps_the_tool_armed_and_esc_disarm_drops_overlays() {
        let mut tool = MeasureTool::default();
        tool.arm(MeasureMode::Ruler);
        tool.place_ruler_point(Vec3::ZERO);
        tool.place_ruler_point(Vec3::X);
        assert!(tool.clear_measurements(), "there was something to clear");
        assert!(tool.is_active(), "RMB clear keeps the tool armed");
        assert!(!tool.clear_measurements(), "second clear is a no-op");
        tool.place_ruler_point(Vec3::ZERO);
        tool.disarm();
        assert!(!tool.is_active());
        assert!(tool.rulers().is_empty() && tool.pending_anchor().is_none());
    }

    #[test]
    fn menu_toggle_arms_disarms_and_kills_the_cut_tool() {
        // Arming from idle with the cut tool active must disable the cut.
        assert_eq!(
            apply_menu_toggle(None, true, MeasureMode::Ruler),
            (Some(MeasureMode::Ruler), true)
        );
        // Clicking the armed entry toggles it off; nothing to disable.
        assert_eq!(
            apply_menu_toggle(Some(MeasureMode::Ruler), false, MeasureMode::Ruler),
            (None, false)
        );
        // Switching tools re-arms; a dormant cut stays untouched.
        assert_eq!(
            apply_menu_toggle(Some(MeasureMode::Ruler), false, MeasureMode::Thickness),
            (Some(MeasureMode::Thickness), false)
        );
    }

    #[test]
    fn completed_ruler_endpoints_can_be_dragged_without_rebuilding_the_pair() {
        let mut tool = MeasureTool::default();
        tool.arm(MeasureMode::Ruler);
        tool.place_ruler_point(Vec3::ZERO);
        tool.place_ruler_point(Vec3::X * 2.0);
        let anchor = RulerAnchorRef {
            ruler_index: 0,
            endpoint: RulerEndpoint::B,
        };
        assert!(tool.begin_ruler_drag(anchor));
        assert_eq!(tool.update_ruler_drag(Vec3::Y * 3.0), Some(3.0));
        assert_eq!(tool.rulers()[0].a, Vec3::ZERO);
        assert_eq!(tool.rulers()[0].b, Vec3::Y * 3.0);
        tool.end_ruler_drag();
        assert!(tool.dragged_ruler_anchor().is_none());
    }

    #[test]
    fn menu_entries_are_greyed_during_an_edit_session_or_empty_scene() {
        assert!(measure_menu_enabled(true, false));
        assert!(!measure_menu_enabled(true, true));
        assert!(!measure_menu_enabled(false, false));
    }

    #[test]
    fn format_mm_is_two_decimals_and_never_nan() {
        assert_eq!(format_mm(12.344), "12.34 mm");
        assert_eq!(format_mm(0.0), "0.00 mm");
        assert!(!format_mm(f64::NAN).contains("NaN"));
        assert!(!format_mm(f64::INFINITY).contains("inf"));
    }

    /// Cube corners + outward-wound faces spanning `[min, max]^3`, with smooth
    /// outward corner normals (so the interpolated normal at a face center is
    /// exactly the face axis).
    fn cube_parts(min: f32, max: f32, offset: u32) -> (Vec<Vertex>, Vec<u32>) {
        let corner = |x: f32, y: f32, z: f32| {
            Vertex::at(Vec3::new(x, y, z)).with_normal(Vec3::new(x, y, z).normalize())
        };
        let vertices = vec![
            corner(min, min, min),
            corner(max, min, min),
            corner(max, max, min),
            corner(min, max, min),
            corner(min, min, max),
            corner(max, min, max),
            corner(max, max, max),
            corner(min, max, max),
        ];
        let indices = [
            1, 2, 6, 1, 6, 5, // +X
            0, 4, 7, 0, 7, 3, // -X
            2, 3, 7, 2, 7, 6, // +Y
            0, 1, 5, 0, 5, 4, // -Y
            4, 5, 6, 4, 6, 7, // +Z
            0, 3, 2, 0, 2, 1, // -Z
        ]
        .iter()
        .map(|&i| i + offset)
        .collect();
        (vertices, indices)
    }

    /// A closed thick-walled shell: outer cube 10 mm, cavity 6 mm (2 mm wall).
    fn shell_scene() -> Scene {
        let (mut vertices, mut indices) = cube_parts(-5.0, 5.0, 0);
        let (inner_v, inner_i) = cube_parts(-3.0, 3.0, 8);
        vertices.extend(inner_v);
        indices.extend(inner_i);
        let mesh = Mesh::new(Some("shell".into()), vertices, indices).expect("shell mesh");
        let mut scene = Scene::new();
        scene.add(SceneMesh::new(mesh));
        scene
    }

    #[test]
    fn thickness_probe_reads_the_known_shell_wall() {
        let scene = shell_scene();
        // Pick the outer +X face dead center: the interpolated corner normals
        // cancel to a pure +X there, so the chord is the exact 2 mm wall.
        let hit = scene
            .pick_ray_hit(Vec3::new(20.0, 0.0, 0.0), -Vec3::X)
            .expect("outer face hit");
        assert!((hit.point.x - 5.0).abs() < 1.0e-4);
        let entry = &scene.meshes()[hit.layer_index];
        let probe = probe_wall_thickness(entry, hit.triangle_index, hit.point).expect("probe");
        match probe.reading {
            ThicknessReading::Wall { exit, thickness_mm } => {
                assert!(
                    (thickness_mm - 2.0).abs() < 1.0e-3,
                    "thickness {thickness_mm}"
                );
                assert!((exit.x - 3.0).abs() < 1.0e-3, "exit {exit}");
            }
            ThicknessReading::Open => panic!("closed shell must report a wall"),
        }
    }

    /// One open sheet in the x = 0 plane facing +X, spanning y,z in [-1, 1].
    fn sheet_mesh() -> Mesh {
        Mesh::new(
            Some("sheet".into()),
            vec![
                Vertex::at(Vec3::new(0.0, -1.0, -1.0)),
                Vertex::at(Vec3::new(0.0, 1.0, -1.0)),
                Vertex::at(Vec3::new(0.0, 1.0, 1.0)),
                Vertex::at(Vec3::new(0.0, -1.0, 1.0)),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
        .expect("sheet mesh")
    }

    #[test]
    fn open_surface_reports_open_not_a_fake_number() {
        let entry = SceneMesh::new(sheet_mesh());
        let probe = probe_wall_thickness(&entry, 0, Vec3::new(0.0, 0.2, 0.1)).expect("probe");
        assert_eq!(probe.reading, ThicknessReading::Open);
    }

    #[test]
    fn degenerate_triangle_refuses_to_probe() {
        // A zero-area triangle next to a real one: normal repair leaves its
        // vertex normals unusable and the face normal is zero, so the probe
        // must refuse (None), not guess a direction.
        let p = Vec3::new(4.0, 4.0, 4.0);
        let mesh = Mesh::new(
            Some("degenerate".into()),
            vec![
                Vertex::at(Vec3::new(0.0, -1.0, -1.0)).with_normal(Vec3::X),
                Vertex::at(Vec3::new(0.0, 1.0, -1.0)).with_normal(Vec3::X),
                Vertex::at(Vec3::new(0.0, 1.0, 1.0)).with_normal(Vec3::X),
                Vertex::at(p),
                Vertex::at(p),
                Vertex::at(p),
            ],
            vec![0, 1, 2, 3, 4, 5],
        )
        .expect("mesh");
        let entry = SceneMesh::new(mesh);
        assert!(probe_wall_thickness(&entry, 1, p).is_none());
    }

    #[test]
    fn nan_poisoned_exit_geometry_cannot_panic_or_poison_the_reading() {
        // Entry sheet is sane; a NaN triangle floats where the exit would be.
        let mut vertices = sheet_mesh().vertices().to_vec();
        let mut indices = sheet_mesh().indices().to_vec();
        let base = u32::try_from(vertices.len()).expect("small mesh");
        vertices.push(Vertex::at(Vec3::NAN));
        vertices.push(Vertex::at(Vec3::new(-2.0, 1.0, f32::NAN)));
        vertices.push(Vertex::at(Vec3::new(-2.0, f32::INFINITY, 1.0)));
        indices.extend([base, base + 1, base + 2]);
        let mesh = Mesh::new(Some("poisoned".into()), vertices, indices).expect("mesh");
        let entry = SceneMesh::new(mesh);
        let probe = probe_wall_thickness(&entry, 0, Vec3::new(0.0, 0.2, 0.1)).expect("probe");
        match probe.reading {
            ThicknessReading::Open => {}
            ThicknessReading::Wall { thickness_mm, .. } => {
                assert!(thickness_mm.is_finite(), "reading must never be NaN");
            }
        }
    }

    #[test]
    fn probe_out_of_range_triangle_is_refused() {
        let entry = SceneMesh::new(sheet_mesh());
        assert!(probe_wall_thickness(&entry, 99, Vec3::ZERO).is_none());
        assert!(probe_wall_thickness(&entry, usize::MAX, Vec3::ZERO).is_none());
    }
}
