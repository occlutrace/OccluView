use eframe::egui;
use glam::Vec3;
use occluview_core::FaceSelection;
use occluview_core::{Camera, Scene, SceneMesh, SceneMeshId, ScenePickHit};

/// Region selection request (freehand lasso or the marquee rectangle as a
/// 4-point polygon). exocad "Mark triangles" semantics: a triangle is taken iff
/// its projected footprint INTERSECTS the outline in screen space (any triangle
/// vertex inside the outline, any outline vertex inside the triangle, or any
/// edges crossing) — so a lasso smaller than a big flat triangle, or one whose
/// edge merely clips it, still marks it. When `through_mesh` is false the
/// triangle must ALSO face the camera (surface mode). Completed outlines
/// ACCUMULATE into the existing highlight; with `unmark` (SHIFT) the outline
/// un-marks instead.
#[derive(Clone, Debug)]
pub(crate) struct ScreenPolygonSelectionRequest<'a> {
    pub(crate) viewport_rect: egui::Rect,
    pub(crate) polygon_px: &'a [egui::Pos2],
    pub(crate) unmark: bool,
    pub(crate) through_mesh: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FaceSelectionState {
    layer_id: SceneMeshId,
    selected_faces: Vec<bool>,
}

impl FaceSelectionState {
    pub(super) fn empty_for_layer(layer_id: SceneMeshId, triangle_count: usize) -> Option<Self> {
        if triangle_count == 0 {
            return None;
        }
        Some(Self {
            layer_id,
            selected_faces: vec![false; triangle_count],
        })
    }

    /// Click selection, exocad convention: a click MARKS the face (accumulates
    /// into the highlight); with `unmark` (SHIFT) it un-marks instead.
    pub(super) fn select_scene_hit(
        &mut self,
        scene: &Scene,
        hit: ScenePickHit,
        unmark: bool,
    ) -> Option<()> {
        let entry = scene.meshes().get(hit.layer_index)?;
        if entry.id() != self.layer_id
            || hit.layer_id != self.layer_id
            || !entry.visible
            || entry.mesh.is_point_cloud()
        {
            return None;
        }
        if entry.mesh.triangle_count() != self.selected_faces.len() {
            return None;
        }
        self.set_face(hit.triangle_index, !unmark)
    }

    /// Object-mode click: MARK (or, with `unmark`/SHIFT, un-mark) every triangle
    /// of the connected component under the cursor. Same layer-guard contract as
    /// [`Self::select_scene_hit`]: `None` when the hit is off this selection's
    /// layer, hidden, a point cloud, or stale against the live triangle count.
    pub(super) fn select_scene_hit_component(
        &mut self,
        scene: &Scene,
        hit: ScenePickHit,
        unmark: bool,
    ) -> Option<()> {
        let entry = scene.meshes().get(hit.layer_index)?;
        if entry.id() != self.layer_id
            || hit.layer_id != self.layer_id
            || !entry.visible
            || entry.mesh.is_point_cloud()
        {
            return None;
        }
        if entry.mesh.triangle_count() != self.selected_faces.len() {
            return None;
        }
        let component = component_triangles(entry, hit.triangle_index)?;
        let mark = !unmark;
        for &triangle in &component {
            if let Some(slot) = self.selected_faces.get_mut(triangle) {
                *slot = mark;
            }
        }
        Some(())
    }

    pub(crate) fn selected_count(&self) -> usize {
        self.selected_faces
            .iter()
            .filter(|selected| **selected)
            .count()
    }

    pub(crate) fn triangle_count(&self) -> usize {
        self.selected_faces.len()
    }

    pub(super) fn clear_selection(&mut self) -> bool {
        let had_selection = self.selected_faces.iter().any(|selected| *selected);
        self.selected_faces.fill(false);
        had_selection
    }

    pub(super) fn select_all(&mut self) -> bool {
        let needs_change = self.selected_faces.iter().any(|selected| !*selected);
        self.selected_faces.fill(true);
        needs_change
    }

    pub(super) fn invert_selection(&mut self) -> bool {
        if self.selected_faces.is_empty() {
            return false;
        }
        for selected in &mut self.selected_faces {
            *selected = !*selected;
        }
        true
    }

    /// Region selection (lasso outline or the marquee as a 4-point polygon). A
    /// triangle is taken iff its screen projection INTERSECTS the outline —
    /// exocad "Mark triangles" semantics — so a lasso smaller than a big flat
    /// triangle, or one whose edge merely crosses it, still marks it (the old
    /// "all three vertices inside" rule silently dropped sparse flat regions,
    /// whose triangles are much larger than a dense curved surface's). In
    /// surface mode the triangle must also face the camera. Outlines
    /// accumulate; `unmark` clears instead.
    pub(super) fn select_screen_polygon(
        &mut self,
        scene: &Scene,
        camera: &Camera,
        request: ScreenPolygonSelectionRequest<'_>,
    ) -> Option<bool> {
        let entry = scene
            .meshes()
            .iter()
            .find(|entry| entry.id() == self.layer_id)?;
        if !entry.visible || entry.mesh.is_point_cloud() {
            return None;
        }
        if entry.mesh.triangle_count() != self.selected_faces.len() {
            return None;
        }

        let polygon = request.polygon_px;
        if polygon.len() < 3 {
            return None;
        }
        let mut polygon_bbox = egui::Rect::NOTHING;
        for &point in polygon {
            polygon_bbox.extend_with(point);
        }
        let polygon_bbox = polygon_bbox.intersect(request.viewport_rect);
        if polygon_bbox.width() <= f32::EPSILON || polygon_bbox.height() <= f32::EPSILON {
            return None;
        }
        // Project the outline to f64 once; every predicate below is a robust
        // f64 orientation test, so the polygon is reused across all triangles
        // instead of re-converting per triangle.
        let polygon_pts: Vec<ScreenPt> = polygon.iter().copied().map(ScreenPt::new).collect();

        // Precompute the orthographic projection basis ONCE (see `OrthoProjector`
        // — `camera.eye()`/`view_direction()`/`view_up()` each resolve the
        // orientation quaternion, so routing every vertex through the shared
        // per-point projector would recompute this basis millions of times on a
        // large mesh). `toward_viewer` is the single constant view direction the
        // ortho render selects through; a triangle is front-facing iff its
        // geometric face normal has a positive component along it. Using this
        // constant direction (NOT a per-face `eye - centroid`, a perspective-
        // style vector that swings with lateral offset and wrongly culls whole
        // front-facing FLAT patches off the view axis) keeps surface mode
        // consistent with the projection, and recomputing the normal from
        // positions never trusts stored vertex normals (flat meshes leave unset).
        let projector = OrthoProjector::new(camera, request.viewport_rect)?;
        let toward_viewer = projector.toward_viewer();
        let mark = !request.unmark;
        let mut changed = false;

        let vertices = entry.mesh.vertices();
        for (triangle_index, triangle) in entry.mesh.indices().chunks_exact(3).enumerate() {
            let [a, b, c] = triangle_world_points(vertices, triangle, entry.transform)?;

            let Some((screen_a, depth_a)) = projector.project(a) else {
                continue;
            };
            let Some((screen_b, depth_b)) = projector.project(b) else {
                continue;
            };
            let Some((screen_c, depth_c)) = projector.project(c) else {
                continue;
            };
            if depth_a <= 0.0 || depth_b <= 0.0 || depth_c <= 0.0 {
                continue;
            }

            // Cheap reject: a triangle whose screen bounding box does not even
            // overlap the outline's box cannot intersect it. Prunes the vast
            // majority of triangles far from the outline before any predicate
            // runs — keeps the O(polygon) tests off the whole mesh.
            let mut triangle_bbox = egui::Rect::NOTHING;
            triangle_bbox.extend_with(screen_a);
            triangle_bbox.extend_with(screen_b);
            triangle_bbox.extend_with(screen_c);
            if !triangle_bbox.intersects(polygon_bbox) {
                continue;
            }

            // True polygon/triangle intersection (exocad "Mark triangles"): mark
            // on ANY screen-space overlap, not only full containment. This is
            // what makes a small lasso catch a large flat triangle.
            if !triangle_intersects_polygon(
                ScreenPt::new(screen_a),
                ScreenPt::new(screen_b),
                ScreenPt::new(screen_c),
                &polygon_pts,
            ) {
                continue;
            }

            // Surface mode: skip triangles that face away from the camera (no
            // through-mesh pick). Through-mesh mode takes every enclosed face.
            // Degeneracy must be RELATIVE to the triangle's own scale: an
            // absolute epsilon in mm^4 silently culled real micro-triangles
            // (hi-res scanner facets have ~15 um edges). A triangle is
            // degenerate only when its area is vanishing relative to its edge
            // lengths, at any absolute scale.
            if !request.through_mesh {
                let e1 = (b - a).as_dvec3();
                let e2 = (c - a).as_dvec3();
                let normal = e1.cross(e2);
                let edge_scale = e1.length_squared() * e2.length_squared();
                if normal.length_squared() <= edge_scale * 1e-24 {
                    continue;
                }
                if normal.dot(toward_viewer.as_dvec3()) <= 0.0 {
                    continue;
                }
            }

            if self.selected_faces[triangle_index] != mark {
                self.selected_faces[triangle_index] = mark;
                changed = true;
            }
        }

        Some(changed)
    }

    fn set_face(&mut self, triangle_index: usize, selected: bool) -> Option<()> {
        if triangle_index >= self.selected_faces.len() {
            return None;
        }
        self.selected_faces[triangle_index] = selected;
        Some(())
    }

    pub(crate) fn to_face_selection(&self) -> FaceSelection {
        FaceSelection::new(self.selected_faces.clone())
    }
}

/// Triangle indices of the object (connected component) that owns
/// `triangle_index` on `entry`'s mesh. Delegates to the welded-topology kernel
/// so a soup STL resolves to whole objects, not per-facet confetti. `None` on a
/// point cloud / faceless mesh or an out-of-range index — an honest no-op, never
/// a panic.
fn component_triangles(entry: &SceneMesh, triangle_index: usize) -> Option<Vec<usize>> {
    occluview_core::component_at_triangle_in_mesh(&entry.mesh, triangle_index)
        .ok()
        .flatten()
}

fn triangle_world_points(
    vertices: &[occluview_core::Vertex],
    triangle: &[u32],
    transform: glam::Affine3A,
) -> Option<[Vec3; 3]> {
    let ia = usize::try_from(*triangle.first()?).ok()?;
    let ib = usize::try_from(*triangle.get(1)?).ok()?;
    let ic = usize::try_from(*triangle.get(2)?).ok()?;
    Some([
        transform.transform_point3(Vec3::from_array(vertices.get(ia)?.position)),
        transform.transform_point3(Vec3::from_array(vertices.get(ib)?.position)),
        transform.transform_point3(Vec3::from_array(vertices.get(ic)?.position)),
    ])
}

/// Precomputed orthographic projection basis. Built once per region-selection
/// call so each of a large mesh's vertices projects with three dot products
/// instead of re-resolving the camera orientation quaternion per point (the
/// dominant cost when the shared per-point projector is called in a hot loop).
struct OrthoProjector {
    eye: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    half_width: f32,
    half_height: f32,
    width: f32,
    height: f32,
    left: f32,
    top: f32,
}

impl OrthoProjector {
    /// Resolve the basis from the camera and viewport, or `None` when the
    /// camera/viewport is degenerate — matching the guards in
    /// `project_world_to_viewport`.
    fn new(camera: &Camera, viewport: egui::Rect) -> Option<Self> {
        let width = viewport.width();
        let height = viewport.height();
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let eye = camera.eye();
        let forward = camera.view_direction();
        if forward.length_squared() <= f32::EPSILON {
            return None;
        }
        let up = camera.view_up();
        let right = forward.cross(up).normalize_or_zero();
        if right.length_squared() <= f32::EPSILON || up.length_squared() <= f32::EPSILON {
            return None;
        }
        let half_height = camera.orthographic_height * 0.5;
        let half_width = half_height * (width / height);
        if half_height <= f32::EPSILON || half_width <= f32::EPSILON {
            return None;
        }
        Some(Self {
            eye,
            forward,
            right,
            up,
            half_width,
            half_height,
            width,
            height,
            left: viewport.left(),
            top: viewport.top(),
        })
    }

    /// Project a world point to (screen pixel, depth). Bit-identical to
    /// `project_world_to_viewport`: `None` for a non-finite point or depth so
    /// those triangles are skipped exactly as the shared projector would.
    #[inline]
    fn project(&self, point: Vec3) -> Option<(egui::Pos2, f32)> {
        if !point.is_finite() {
            return None;
        }
        let offset = point - self.eye;
        let depth = offset.dot(self.forward);
        if !depth.is_finite() {
            return None;
        }
        let ndc_x = offset.dot(self.right) / self.half_width;
        let ndc_y = offset.dot(self.up) / self.half_height;
        let screen = egui::pos2(
            self.left + (ndc_x + 1.0) * 0.5 * self.width,
            self.top + (1.0 - (ndc_y + 1.0) * 0.5) * self.height,
        );
        Some((screen, depth))
    }

    /// The constant direction from the surface back toward the camera (ortho).
    #[inline]
    fn toward_viewer(&self) -> Vec3 {
        -self.forward
    }
}

/// 2D screen-space point in f64. The polygon/triangle intersection predicates
/// run in f64 so large flat triangles and near-degenerate projections classify
/// consistently (f32 orientation determinants lose sign near-collinearly).
#[derive(Clone, Copy)]
struct ScreenPt {
    x: f64,
    y: f64,
}

impl ScreenPt {
    fn new(point: egui::Pos2) -> Self {
        Self {
            x: f64::from(point.x),
            y: f64::from(point.y),
        }
    }
}

/// Twice the signed area of triangle (a, b, c): > 0 for one winding, < 0 for the
/// other, 0 when collinear. The single orientation predicate every 2D
/// intersection test below is built from.
#[inline]
fn orient(a: ScreenPt, b: ScreenPt, c: ScreenPt) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// Whether the projected triangle intersects the outline in screen space
/// (exocad "Mark triangles"): true if ANY of — a triangle vertex is inside the
/// outline, an outline vertex is inside the triangle, or an outline edge
/// crosses a triangle edge. The two vertex-inside tests catch the common
/// fully-inside / fully-containing cases cheaply; the edge test only settles
/// the straddling boundary triangles neither vertex test resolved.
fn triangle_intersects_polygon(
    a: ScreenPt,
    b: ScreenPt,
    c: ScreenPt,
    polygon: &[ScreenPt],
) -> bool {
    // (a) any triangle vertex inside the outline (triangle ⊆ / clipped by it).
    if point_in_polygon_pts(a, polygon)
        || point_in_polygon_pts(b, polygon)
        || point_in_polygon_pts(c, polygon)
    {
        return true;
    }
    // (b) any outline vertex inside the triangle (outline ⊆ a big triangle —
    // the flat-surface case: the lasso sits wholly within one large triangle).
    for &point in polygon {
        if point_in_triangle(point, a, b, c) {
            return true;
        }
    }
    // (c) any outline edge crosses a triangle edge (a straddle with no vertex of
    // either shape inside the other).
    let triangle_edges = [(a, b), (b, c), (c, a)];
    let vertex_count = polygon.len();
    let mut previous = vertex_count - 1;
    for current in 0..vertex_count {
        let edge_start = polygon[previous];
        let edge_end = polygon[current];
        for &(t0, t1) in &triangle_edges {
            if segments_intersect(edge_start, edge_end, t0, t1) {
                return true;
            }
        }
        previous = current;
    }
    false
}

/// Whether `point` lies inside triangle (a, b, c) (edges inclusive). A
/// degenerate (zero-area) projected triangle contains nothing here — such a
/// triangle is instead caught, if at all, by the vertex-inside or edge-crossing
/// tests, so this never reports a spurious hit from a collapsed projection.
#[inline]
fn point_in_triangle(point: ScreenPt, a: ScreenPt, b: ScreenPt, c: ScreenPt) -> bool {
    if orient(a, b, c).abs() <= f64::EPSILON {
        return false;
    }
    let d1 = orient(a, b, point);
    let d2 = orient(b, c, point);
    let d3 = orient(c, a, point);
    let has_negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_negative && has_positive)
}

/// Whether the closed segments (p1, p2) and (p3, p4) intersect, including
/// collinear touching. Classic four-orientation test, evaluated in f64.
#[inline]
fn segments_intersect(p1: ScreenPt, p2: ScreenPt, p3: ScreenPt, p4: ScreenPt) -> bool {
    let d1 = orient(p3, p4, p1);
    let d2 = orient(p3, p4, p2);
    let d3 = orient(p1, p2, p3);
    let d4 = orient(p1, p2, p4);
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    (d1 == 0.0 && on_segment(p3, p4, p1))
        || (d2 == 0.0 && on_segment(p3, p4, p2))
        || (d3 == 0.0 && on_segment(p1, p2, p3))
        || (d4 == 0.0 && on_segment(p1, p2, p4))
}

/// Whether `point`, known to be collinear with segment (a, b), lies within it.
#[inline]
fn on_segment(a: ScreenPt, b: ScreenPt, point: ScreenPt) -> bool {
    point.x >= a.x.min(b.x)
        && point.x <= a.x.max(b.x)
        && point.y >= a.y.min(b.y)
        && point.y <= a.y.max(b.y)
}

/// Even-odd crossing-number point-in-polygon on a 2D closed outline (f64).
/// Orientation-agnostic, so it works in egui's y-down screen space unchanged.
/// The straddle guard rules out the horizontal-edge divide-by-zero, so no
/// epsilon fudge is needed. This runs O(polygon) per query on the whole mesh's
/// enclosed triangles, so the `above` flag is carried across edges (one
/// y-compare per vertex, not two per edge).
#[inline]
fn point_in_polygon_pts(point: ScreenPt, polygon: &[ScreenPt]) -> bool {
    let vertex_count = polygon.len();
    if vertex_count < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous_point = polygon[vertex_count - 1];
    let mut previous_above = previous_point.y > point.y;
    for &current_point in polygon {
        let current_above = current_point.y > point.y;
        if current_above != previous_above {
            let crossing_x = (previous_point.x - current_point.x) * (point.y - current_point.y)
                / (previous_point.y - current_point.y)
                + current_point.x;
            if point.x < crossing_x {
                inside = !inside;
            }
        }
        previous_point = current_point;
        previous_above = current_above;
    }
    inside
}

/// Even-odd point-in-polygon on screen pixels — a thin `egui::Pos2` wrapper over
/// [`point_in_polygon_pts`], kept for the outline unit test.
#[cfg(test)]
fn point_in_polygon(point: egui::Pos2, polygon: &[egui::Pos2]) -> bool {
    let polygon: Vec<ScreenPt> = polygon.iter().copied().map(ScreenPt::new).collect();
    point_in_polygon_pts(ScreenPt::new(point), &polygon)
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
