//! One pointer call raycasts the live surface and processes a bounded swept
//! segment. Nearby hits include the intervening path; a segment that exceeds
//! the travel limit or crosses a surface gap starts at the current hit. This
//! bounds the work and footprint of each call.

use super::*;

/// Travel one full dab stands for, as a share of the brush radius. It sets a
/// stroke's dose per millimetre: the swept step integrates the tip along its
/// path at this density. A knife is a narrow blade, so its trail is denser.
const KNIFE_SPACING_SHARE: f64 = 0.10;
const BALL_SPACING_SHARE: f64 = 0.15;
/// The closest two dab centres may ever be, millimetres.
const MIN_SPACING_MM: f64 = 0.03;
/// Path samples as a share of the radius. Between samples the step reads the
/// surface as a straight chord; half a radius keeps that chord under the
/// footprint on a curved surface.
const PATH_SAMPLE_SHARE: f64 = 0.5;
/// Travel one step may sweep, in radii. It keeps a stroke continuous at any
/// hand speed the step rate can follow and bounds the footprint of a call to
/// about two dabs' area, whatever the pointer did meanwhile.
const MAX_STEP_TRAVEL_SHARE: f64 = 2.0;
/// Bisection levels one sample interval may take before the path counts as
/// broken. Three levels follow a steep flank or a ridge at an eighth of the
/// sample spacing, finer than the continuity bound on any surface the
/// pointer can see, so only a real gap or a turn onto a facing sheet breaks.
const PATH_REFINE_DEPTH: u32 = 3;

/// How a traced path ended.
enum PathTrace {
    /// It reached this call's endpoint.
    Continuous,
    /// A sample missed the surface or crossed a gap; the prefix stands.
    Broken,
    /// The pointer travelled farther than one step may sweep.
    TooFar,
}

impl SculptSession {
    fn continuous_hit(
        previous: (DVec3, DVec3),
        hit: DVec3,
        normal: DVec3,
        radius: f64,
        spacing: f64,
    ) -> bool {
        let distance = (hit - previous.0).length();
        if !distance.is_finite()
            || distance > 0.6f64.max(radius.abs() * 0.8).max(spacing.abs() * 8.0)
        {
            return false;
        }
        let previous_normal = previous.1.normalize_or_zero();
        let next_normal = normal.normalize_or_zero();
        previous_normal.length() <= 1e-9
            || next_normal.length() <= 1e-9
            || previous_normal.dot(next_normal) >= -0.35
    }

    fn interpolated_ray(
        start: StrokePathState,
        origin: DVec3,
        dir: DVec3,
        amount: f64,
    ) -> (DVec3, DVec3) {
        let t = amount.clamp(0.0, 1.0);
        let ray_origin = start.origin + ((origin - start.origin) * t);
        let mixed = start.dir + ((dir - start.dir) * t);
        let ray_dir = if mixed.length() > 1e-12 {
            mixed.normalize_or_zero()
        } else {
            start.dir
        };
        (ray_origin, ray_dir)
    }

    /// Apply one pointer segment entirely in Rust: current-surface raycasts,
    /// the swept dose, continuity rejection, one deformation and one remesh.
    /// A `hold` is a separate physical command: one dab at the current screen
    /// ray, dosed by the hold time the caller sets.
    // a ray step is named by its ray, dose and mode.
    #[allow(clippy::too_many_arguments)]
    pub fn dab_at_ray(
        &mut self,
        origin: DVec3,
        dir: DVec3,
        radius: f64,
        strength: f64,
        mode: BrushMode,
        hold: bool,
    ) -> StrokePathResult {
        self.dab_at_ray_visible(
            origin,
            dir,
            radius,
            strength,
            mode,
            hold,
            SculptRayConstraints::default(),
        )
    }

    /// One call is one step, and it always ends where the pointer is, so no
    /// work is left for a later call and `complete` is always true.
    #[allow(clippy::too_many_arguments)]
    pub fn dab_at_ray_visible(
        &mut self,
        origin: DVec3,
        dir: DVec3,
        radius: f64,
        strength: f64,
        mode: BrushMode,
        hold: bool,
        constraints: SculptRayConstraints<'_>,
    ) -> StrokePathResult {
        if self.stroke_clip_planes != constraints.clip_planes {
            self.stroke_path = None;
            self.stroke_clip_planes.clear();
            self.stroke_clip_planes
                .extend_from_slice(constraints.clip_planes);
        }
        let dir = dir.normalize_or_zero();
        // Everything this call commits publishes as one journal slice, in
        // order.
        let (slice_mark, slice_base) = self.topo_slice_point();
        let diag_before = diag::peek();
        self.live_kin = LiveKinematics::default();
        let spacing = if self.brush_tip == TipStamp::Knife {
            (radius * KNIFE_SPACING_SHARE).max(MIN_SPACING_MM)
        } else {
            (radius * BALL_SPACING_SHARE).max(MIN_SPACING_MM)
        };
        let (mut moved, hit, applied) = self.stroke_step(
            origin,
            dir,
            radius,
            strength,
            mode,
            hold,
            spacing,
            constraints,
        );
        moved.sort_unstable();
        moved.dedup();
        let live = self.capture_live_trace(
            diag_before,
            u32::from(applied),
            &moved,
            true,
            hit.is_some(),
            self.topo_journal
                .added_verts
                .len()
                .saturating_sub(slice_mark.added_verts) as u32,
            self.topo_journal
                .added_tris
                .len()
                .saturating_sub(slice_mark.added_tris) as u32,
            self.topo_journal
                .rewired
                .len()
                .saturating_sub(slice_mark.rewired) as u32,
            self.topo_journal
                .collapsed
                .len()
                .saturating_sub(slice_mark.collapsed) as u32,
        );
        StrokePathResult {
            hit,
            moved,
            complete: true,
            topo: self.publish_topology_slice(&slice_mark, &slice_base),
            live,
        }
    }

    /// Raycast the pointer, sweep the short path from the previous step's end
    /// when it is close (or stamp once for a press, a hold or a long jump),
    /// then deform and remesh once.
    /// Returns the dirty vertices, where the ray lands on the edited surface,
    /// and whether a dab ran. A miss or a break in the path clears the path,
    /// so the next call starts fresh.
    #[allow(clippy::too_many_arguments)]
    // one pointer segment: trace, deform and remesh.
    #[allow(clippy::too_many_lines)]
    fn stroke_step(
        &mut self,
        origin: DVec3,
        dir: DVec3,
        radius: f64,
        strength: f64,
        mode: BrushMode,
        hold: bool,
        spacing: f64,
        constraints: SculptRayConstraints<'_>,
    ) -> (Vec<u32>, Option<(DVec3, DVec3)>, bool) {
        let Some((endpoint, endpoint_normal)) = self.raycast_visible(origin, dir, constraints)
        else {
            // A miss is "the pointer left the surface": the cursor must
            // clear, not stay on the last applied dab.
            self.stroke_path = None;
            self.dab_axis = None;
            return (Vec::new(), None, false);
        };
        let Some(endpoint_triangle) = self.hit_triangle else {
            self.stroke_path = None;
            return (Vec::new(), None, false);
        };
        // A camera interval that changed without both ends finite cannot be
        // interpolated across: the step restarts from this ray.
        if let Some(previous) = self.stroke_path {
            let both_finite = previous.far.is_finite() && constraints.far.is_finite();
            if !both_finite
                && (previous.near != constraints.near || previous.far != constraints.far)
            {
                self.stroke_path = None;
            }
        }
        let previous = if hold { None } else { self.stroke_path };
        let view_axis = self.stroke_path.map_or(dir, |state| state.view_axis);
        self.dab_path.clear();
        let traced = match previous {
            None => PathTrace::Continuous,
            Some(previous) => self.trace_path(
                previous,
                (origin, dir, constraints),
                (endpoint, endpoint_normal, endpoint_triangle),
                radius,
                spacing,
            ),
        };
        let (previous, continuous) = match traced {
            PathTrace::Continuous => (previous, true),
            PathTrace::Broken => (previous, false),
            // Travel too long to sweep restarts the stroke here, as a press.
            PathTrace::TooFar => {
                self.dab_path.clear();
                (None, true)
            }
        };
        let swept = self.dab_path.len() >= 2 && self.path_length() > 1e-9;
        if previous.is_some() && !swept {
            self.dab_path.clear();
            if !continuous {
                self.stroke_path = None;
                self.dab_axis = None;
                return (Vec::new(), None, false);
            }
            // The pointer did not move along the surface: nothing to sweep.
            // A still hand doses through the caller's hold command.
            self.stroke_path = Some(StrokePathState {
                origin,
                dir,
                view_axis,
                hit: endpoint,
                normal: endpoint_normal,
                triangle: endpoint_triangle,
                near: constraints.near,
                far: constraints.far,
            });
            return (Vec::new(), Some((endpoint, endpoint_normal)), false);
        }
        let (center, triangle) = match self.dab_path.last() {
            Some(sample) if swept => (sample.point, sample.triangle),
            _ => (endpoint, endpoint_triangle),
        };
        self.hit_triangle = Some(triangle);
        if swept {
            self.dab_path_spacing = spacing;
            // The knife reads each segment's own bearing inside the sweep; the
            // memory keeps the step's travel on the stable view plane for the
            // next press or hold.
            let travel = center - self.dab_path[0].point;
            let on_plane = travel - (view_axis * travel.dot(view_axis));
            if on_plane.length() > 1e-9 {
                self.dab_axis_memory = Some(on_plane.normalize_or_zero());
            }
            self.dab_axis = self.dab_axis_memory;
        } else {
            let along_view = dir - (endpoint_normal * dir.dot(endpoint_normal));
            self.dab_axis = self.dab_axis_memory.or_else(|| {
                (along_view.length() > 1e-9).then(|| along_view * (1.0 / along_view.length()))
            });
        }
        let moved = self.dab(&Dab {
            center,
            radius,
            strength,
            view: dir,
            mode,
        });
        self.dab_path.clear();
        if !continuous {
            // The swept prefix is applied; the break itself starts a new
            // stroke segment on the next call.
            self.stroke_path = None;
            self.dab_axis = None;
            return (moved, None, true);
        }
        let current_hit = self.raycast_visible(origin, dir, constraints);
        self.stroke_path = match (current_hit, self.hit_triangle) {
            (Some((hit, normal)), Some(triangle)) => Some(StrokePathState {
                origin,
                dir,
                view_axis,
                hit,
                normal,
                triangle,
                near: constraints.near,
                far: constraints.far,
            }),
            _ => None,
        };
        (
            moved,
            Some(current_hit.unwrap_or((endpoint, endpoint_normal))),
            true,
        )
    }

    /// Sample the pointer's path on the current surface into `dab_path`, from
    /// the previous step's end to this ray's hit. Samples come from rays
    /// interpolated between the two pointer rays, so the path follows the
    /// surface the operator sees.
    // the segment's endpoints and sampling controls are one trace.
    #[allow(clippy::too_many_arguments)]
    fn trace_path(
        &mut self,
        previous: StrokePathState,
        ray: (DVec3, DVec3, SculptRayConstraints<'_>),
        end: (DVec3, DVec3, u32),
        radius: f64,
        spacing: f64,
    ) -> PathTrace {
        let (endpoint, endpoint_normal, endpoint_triangle) = end;
        self.dab_path.push(PathSample {
            point: previous.hit,
            triangle: previous.triangle,
        });
        let sample_step = (radius * PATH_SAMPLE_SHARE).max(MIN_SPACING_MM);
        let travel = (endpoint - previous.hit).length();
        if !travel.is_finite() || travel > (radius * MAX_STEP_TRAVEL_SHARE).max(sample_step) {
            return PathTrace::TooFar;
        }
        let count = (travel / sample_step).ceil();
        let count = (count as usize).max(1);
        let mut last = (0.0, previous.hit, previous.normal);
        for index in 1..=count {
            let amount = index as f64 / count as f64;
            let next = if index == count {
                (amount, endpoint, endpoint_normal, endpoint_triangle)
            } else {
                let Some((hit, normal, triangle)) = self.path_ray_hit(previous, ray, amount) else {
                    return PathTrace::Broken;
                };
                (amount, hit, normal, triangle)
            };
            if !self.join_path(
                previous,
                ray,
                last,
                next,
                (radius, spacing),
                PATH_REFINE_DEPTH,
            ) {
                return PathTrace::Broken;
            }
            last = (next.0, next.1, next.2);
        }
        PathTrace::Continuous
    }

    /// The surface under the pointer ray `amount` of the way from the
    /// previous step's ray to this call's, with the face it hit.
    fn path_ray_hit(
        &mut self,
        previous: StrokePathState,
        ray: (DVec3, DVec3, SculptRayConstraints<'_>),
        amount: f64,
    ) -> Option<(DVec3, DVec3, u32)> {
        let (origin, dir, constraints) = ray;
        let (mut ray_origin, mut ray_dir) = Self::interpolated_ray(previous, origin, dir, amount);
        let mut used = constraints;
        if previous.far.is_finite() && constraints.far.is_finite() {
            let narrowed = visible_ray::interpolate_visible_segment(
                (previous.origin, previous.dir, previous.near, previous.far),
                (origin, dir, constraints.near, constraints.far),
                amount,
            );
            ray_origin = narrowed.0;
            ray_dir = narrowed.1;
            used.near = 0.0;
            used.far = narrowed.2;
        }
        let (hit, normal) = self.raycast_visible(ray_origin, ray_dir, used)?;
        Some((hit, normal, self.hit_triangle?))
    }

    /// Append `to` after `from`. While the two hits are too far apart for one
    /// continuous stretch, the ray interval between them is bisected, so a
    /// steep flank or a ridge crossing keeps its path instead of breaking it.
    // the segment, its bisection depth and the scale are one continuity test.
    #[allow(clippy::too_many_arguments)]
    fn join_path(
        &mut self,
        previous: StrokePathState,
        ray: (DVec3, DVec3, SculptRayConstraints<'_>),
        from: (f64, DVec3, DVec3),
        to: (f64, DVec3, DVec3, u32),
        scale: (f64, f64),
        depth: u32,
    ) -> bool {
        let (radius, spacing) = scale;
        if Self::continuous_hit((from.1, from.2), to.1, to.2, radius, spacing) {
            self.dab_path.push(PathSample {
                point: to.1,
                triangle: to.3,
            });
            return true;
        }
        if depth == 0 {
            return false;
        }
        let amount = f64::midpoint(from.0, to.0);
        let Some((hit, normal, triangle)) = self.path_ray_hit(previous, ray, amount) else {
            return false;
        };
        self.join_path(
            previous,
            ray,
            from,
            (amount, hit, normal, triangle),
            scale,
            depth - 1,
        ) && self.join_path(previous, ray, (amount, hit, normal), to, scale, depth - 1)
    }

    /// Raycast the current deformed surface (3D-DDA through the triangle
    /// buckets). Returns (hit point, triangle normal). Both winding sides hit
    /// (the ray decides the brush anchor, frontface culling is per-vertex).
    pub fn raycast(&mut self, orig: DVec3, dir: DVec3) -> Option<(DVec3, DVec3)> {
        self.raycast_visible(orig, dir, SculptRayConstraints::default())
    }

    /// Like [`Self::raycast`], restricted to a camera interval and halfspaces.
    // the 3D-DDA walk is one loop.
    #[allow(clippy::too_many_lines)]
    pub fn raycast_visible(
        &mut self,
        orig: DVec3,
        dir: DVec3,
        constraints: SculptRayConstraints<'_>,
    ) -> Option<(DVec3, DVec3)> {
        self.hit_triangle = None;
        if !constraints.valid(orig, dir) {
            return None;
        }
        let dir = dir.normalize_or_zero();
        let cell = self.rays.cell;
        let lo = self.rays.lo;
        // The grid bounds grow with the geometry while cell indices remain
        // relative to its fixed origin `lo`, which is also the bucket key base.
        let (min_cell, max_cell) = (self.rays.min_cell, self.rays.max_cell);
        let box_lo = DVec3::new(
            lo.x + min_cell.0 as f64 * cell,
            lo.y + min_cell.1 as f64 * cell,
            lo.z + min_cell.2 as f64 * cell,
        );
        let hi = DVec3::new(
            lo.x + (max_cell.0 + 1) as f64 * cell,
            lo.y + (max_cell.1 + 1) as f64 * cell,
            lo.z + (max_cell.2 + 1) as f64 * cell,
        );
        let mut t0 = constraints.near;
        let mut t1 = constraints.far;
        for k in 0..3 {
            let (o, d, l, h) = match k {
                0 => (orig.x, dir.x, box_lo.x, hi.x),
                1 => (orig.y, dir.y, box_lo.y, hi.y),
                _ => (orig.z, dir.z, box_lo.z, hi.z),
            };
            if d.abs() < 1e-12 {
                if o < l || o > h {
                    return None;
                }
            } else {
                let (a, b) = ((l - o) / d, (h - o) / d);
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
            }
        }
        if t0 > t1 {
            return None;
        }
        let start = orig + (dir * (t0 + 1e-9));
        let mut cx = ((start.x - lo.x) / cell).floor() as i32;
        let mut cy = ((start.y - lo.y) / cell).floor() as i32;
        let mut cz = ((start.z - lo.z) / cell).floor() as i32;
        let step = (
            if dir.x > 0.0 { 1i32 } else { -1 },
            if dir.y > 0.0 { 1i32 } else { -1 },
            if dir.z > 0.0 { 1i32 } else { -1 },
        );
        let next_t = |c: i32, o: f64, d: f64, l: f64| -> f64 {
            if d.abs() < 1e-12 {
                return f64::INFINITY;
            }
            let edge = l + (c + i32::from(d > 0.0)) as f64 * cell;
            (edge - o) / d
        };
        let mut tmx = next_t(cx, orig.x, dir.x, lo.x);
        let mut tmy = next_t(cy, orig.y, dir.y, lo.y);
        let mut tmz = next_t(cz, orig.z, dir.z, lo.z);
        let (tdx, tdy, tdz) = (
            (cell / dir.x.abs()).abs(),
            (cell / dir.y.abs()).abs(),
            (cell / dir.z.abs()).abs(),
        );
        self.ray_test_epoch = self.ray_test_epoch.wrapping_add(1);
        if self.ray_test_epoch == 0 {
            self.ray_test_marks.fill(0);
            self.ray_test_epoch = 1;
        }
        let test_epoch = self.ray_test_epoch;
        let verts = &self.verts;
        let tris = &self.tris;
        let v = |i: u32| {
            let offset = i as usize * 3;
            DVec3::new(
                verts[offset] as f64,
                verts[offset + 1] as f64,
                verts[offset + 2] as f64,
            )
        };
        let mut best: Option<(f64, u32)> = None;
        // The occupied bounds never move mid-raycast (only the per-triangle
        // test marks do), so the walk limit is loop-invariant.
        let cell_span = (max_cell.0 - min_cell.0)
            .max(max_cell.1 - min_cell.1)
            .max(max_cell.2 - min_cell.2);
        let guard_limit = cell_span as i64 * 3 + 16;
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > guard_limit {
                break;
            }
            if cx < min_cell.0 - 1
                || cy < min_cell.1 - 1
                || cz < min_cell.2 - 1
                || cx > max_cell.0 + 1
                || cy > max_cell.1 + 1
                || cz > max_cell.2 + 1
            {
                break;
            }
            if let Some(list) = self.rays.map.get(&(cx, cy, cz)) {
                for &ti in list {
                    let mark = &mut self.ray_test_marks[ti as usize];
                    if *mark == test_epoch {
                        continue;
                    }
                    *mark = test_epoch;
                    let t = ti as usize * 3;
                    let (a, b, c) = (v(tris[t]), v(tris[t + 1]), v(tris[t + 2]));
                    if let Some(hit_t) = ray_tri(orig, dir, a, b, c) {
                        if hit_t < constraints.near
                            || hit_t > constraints.far
                            || !constraints.contains(orig + dir * hit_t)
                        {
                            continue;
                        }
                        if best.is_none_or(|(bt, _)| hit_t < bt) {
                            best = Some((hit_t, ti));
                        }
                    }
                }
            }
            // The first cell with a confirmed hit INSIDE it ends the walk
            // (hits behind the current cell face can't be nearer).
            if let Some((bt, _)) = best {
                let cell_exit = tmx.min(tmy).min(tmz);
                if bt <= cell_exit {
                    break;
                }
            }
            if tmx <= tmy && tmx <= tmz {
                if tmx > t1 {
                    break;
                }
                cx += step.0;
                tmx += tdx;
            } else if tmy <= tmz {
                if tmy > t1 {
                    break;
                }
                cy += step.1;
                tmy += tdy;
            } else {
                if tmz > t1 {
                    break;
                }
                cz += step.2;
                tmz += tdz;
            }
        }
        let (t, ti) = best?;
        let k = ti as usize * 3;
        let (a, b, c) = (v(tris[k]), v(tris[k + 1]), v(tris[k + 2]));
        let face = (b - a).cross(c - a);
        let face_normal = face.normalize_or_zero();
        let hit = orig + dir * t;
        let normal = if face.length_squared() > 1e-24 {
            let wa = (c - b).cross(hit - b).dot(face) / face.length_squared();
            let wb = (a - c).cross(hit - c).dot(face) / face.length_squared();
            let wc = 1.0 - wa - wb;
            let smooth = (self.display_n(tris[k]) * wa
                + self.display_n(tris[k + 1]) * wb
                + self.display_n(tris[k + 2]) * wc)
                .normalize_or_zero();
            if smooth.length() > 0.5 && smooth.dot(face_normal) > 0.2 {
                smooth
            } else {
                face_normal
            }
        } else {
            face_normal
        };
        self.hit_triangle = Some(ti);
        Some((hit, normal))
    }
}
