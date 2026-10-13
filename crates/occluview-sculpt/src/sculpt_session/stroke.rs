//! One pointer call raycasts the live surface and processes a bounded swept
//! segment. Nearby hits include the intervening path; a long jump sweeps only
//! its reachable tail, and a segment across a surface gap starts at the current
//! hit. This bounds the work and footprint of each call.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

/// A cooperative cancellation checkpoint on the ray/path work. The production
/// token is the worker's permanent stopping flag; tests can supply a
/// deterministic fixture without changing the production call path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CancelPoint {
    Entry,
    Raycast,
    PathRaycast,
    BeforeDab,
    AfterDab,
    ExactFallback,
    Finish,
}

trait Cancellation {
    type Error;

    fn is_cancelled(&self, point: CancelPoint) -> Option<Self::Error>;
}

/// The non-cancellable public API passes this token so its implementation is
/// the same shared path as the cancellable one. `Infallible` makes its
/// error-free result statically explicit, without a panic branch.
struct NeverCancel;

impl Cancellation for NeverCancel {
    type Error = std::convert::Infallible;

    fn is_cancelled(&self, _point: CancelPoint) -> Option<Self::Error> {
        None
    }
}

impl Cancellation for AtomicBool {
    type Error = Cancelled;

    fn is_cancelled(&self, _point: CancelPoint) -> Option<Self::Error> {
        self.load(AtomicOrdering::Relaxed).then_some(Cancelled)
    }
}

#[derive(Debug)]
struct Cancelled;

fn ensure_not_cancelled<C: Cancellation>(
    cancel: &C,
    point: CancelPoint,
) -> Result<(), C::Error> {
    match cancel.is_cancelled(point) {
        Some(cancelled) => Err(cancelled),
        None => Ok(()),
    }
}

fn consider_ray_triangle(
    triangle: u32,
    verts: &[f32],
    tris: &[u32],
    marks: &mut [u32],
    test_epoch: u32,
    orig: DVec3,
    dir: DVec3,
    constraints: SculptRayConstraints<'_>,
    best: &mut Option<(f64, u32)>,
) {
    let mark = &mut marks[triangle as usize];
    if *mark == test_epoch {
        return;
    }
    *mark = test_epoch;
    let offset = triangle as usize * 3;
    let vertex = |index: u32| {
        let offset = index as usize * 3;
        DVec3::new(
            verts[offset] as f64,
            verts[offset + 1] as f64,
            verts[offset + 2] as f64,
        )
    };
    let (a, b, c) = (
        vertex(tris[offset]),
        vertex(tris[offset + 1]),
        vertex(tris[offset + 2]),
    );
    let Some(hit_t) = ray_tri(orig, dir, a, b, c) else {
        return;
    };
    if hit_t < constraints.near
        || hit_t > constraints.far
        || !constraints.contains(orig + dir * hit_t)
    {
        return;
    }
    // Exact equal-distance hits are resolved by dense triangle id, regardless
    // of bucket visitation or topology-history insertion order.
    if best.is_none_or(|(best_t, best_triangle)| {
        hit_t < best_t || (hit_t == best_t && triangle < best_triangle)
    }) {
        *best = Some((hit_t, triangle));
    }
}

/// Travel one dab footprint stands for, as a share of the brush radius. It
/// sizes remesh work for the area crossed, not the brush's material dose. A
/// knife is a narrow blade, so its trail is denser.
const KNIFE_SPACING_SHARE: f64 = 0.10;
const BALL_SPACING_SHARE: f64 = 0.15;
/// The closest two dab centres may ever be, millimetres.
const MIN_SPACING_MM: f64 = 0.03;

/// Remesh budget spacing along the travel path for `tip` at `radius_mm`, mm.
/// The knife has a narrower footprint, so its swept path needs denser remesh
/// work. The floor keeps budget estimates finite for tiny brushes.
#[must_use]
pub fn tip_dab_spacing_mm(tip: TipStamp, radius_mm: f64) -> f64 {
    let share = match tip {
        TipStamp::Knife => KNIFE_SPACING_SHARE,
        TipStamp::Ball | TipStamp::Cylinder => BALL_SPACING_SHARE,
    };
    (radius_mm.abs() * share).max(MIN_SPACING_MM)
}

/// Path samples as a share of the radius. Between samples the step reads the
/// surface as a straight chord; half a radius keeps that chord under the
/// footprint on a curved surface.
const PATH_SAMPLE_SHARE: f64 = 0.5;
/// Travel one step may sweep, with a fixed physical limit and radius-relative
/// bounds. Small brushes can follow ordinary hand motion; a broad brush keeps
/// one call's footprint bounded.
const STEP_TRAVEL_MM: f64 = 3.0;
const MIN_STEP_TRAVEL_SHARE: f64 = 2.0;
const MAX_STEP_TRAVEL_SHARE: f64 = 4.0;
/// Attempts to find the start of the last sweepable tail of a long pointer
/// jump. Interpolated rays do not move linearly over the surface, so retries
/// move the start toward the endpoint by the ratio the surface span overshot.
const STRETCH_START_TRIES: usize = 3;
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
    /// The pointer travelled farther than one step may sweep and no reachable
    /// tail led to its endpoint.
    TooFar,
}

impl SculptSession {
    /// Travel one pointer step may sweep at this brush radius, millimetres.
    pub(super) fn step_reach(radius: f64) -> f64 {
        STEP_TRAVEL_MM
            .min(radius * MAX_STEP_TRAVEL_SHARE)
            .max(radius * MIN_STEP_TRAVEL_SHARE)
    }

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
    /// time dose, continuity rejection, one deformation and one remesh. The
    /// caller sets elapsed brush time before each call; `hold` only requests a
    /// stationary stamp at the current ray.
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
        match self.dab_at_ray_visible_with_cancel(
            origin,
            dir,
            radius,
            strength,
            mode,
            hold,
            constraints,
            &NeverCancel,
        ) {
            Ok(result) => result,
            Err(never) => match never {},
        }
    }

    /// Cancellable form of [`Self::dab_at_ray_visible`]. `None` means the
    /// owner stopped before this ray call committed a publishable result; the
    /// caller must discard the session and its display mirror. A true ray
    /// miss remains `Some` with `hit == None`.
    #[allow(clippy::too_many_arguments)]
    pub fn dab_at_ray_visible_cancellable(
        &mut self,
        origin: DVec3,
        dir: DVec3,
        radius: f64,
        strength: f64,
        mode: BrushMode,
        hold: bool,
        constraints: SculptRayConstraints<'_>,
        cancel: &AtomicBool,
    ) -> Option<StrokePathResult> {
        self.dab_at_ray_visible_with_cancel(
            origin,
            dir,
            radius,
            strength,
            mode,
            hold,
            constraints,
            cancel,
        )
        .ok()
    }

    #[allow(clippy::too_many_arguments)]
    fn dab_at_ray_visible_with_cancel<C: Cancellation>(
        &mut self,
        origin: DVec3,
        dir: DVec3,
        radius: f64,
        strength: f64,
        mode: BrushMode,
        hold: bool,
        constraints: SculptRayConstraints<'_>,
        cancel: &C,
    ) -> Result<StrokePathResult, C::Error> {
        ensure_not_cancelled(cancel, CancelPoint::Entry)?;
        if self.stroke_clip_planes != constraints.clip_planes {
            self.stroke_path = None;
            self.stroke_clip_planes.clear();
            self.stroke_clip_planes
                .extend_from_slice(constraints.clip_planes);
        }
        let dir = dir.normalize_or_zero();
        ensure_not_cancelled(cancel, CancelPoint::Entry)?;
        // Everything this call commits publishes as one journal slice, in
        // order.
        let (slice_mark, slice_base) = self.topo_slice_point();
        let diag_before = diag::peek();
        self.live_kin = LiveKinematics::default();
        let spacing = tip_dab_spacing_mm(self.brush_tip, radius);
        let (mut moved, hit, applied) = self.stroke_step(
            origin,
            dir,
            radius,
            strength,
            mode,
            hold,
            spacing,
            constraints,
            cancel,
        )?;
        ensure_not_cancelled(cancel, CancelPoint::AfterDab)?;
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
        ensure_not_cancelled(cancel, CancelPoint::Finish)?;
        Ok(StrokePathResult {
            hit,
            moved,
            complete: true,
            topo: self.publish_topology_slice(&slice_mark, &slice_base),
            live,
        })
    }

    /// Break ray-path continuity without closing the active stroke or its
    /// undo record. Callers use this when pointer routing temporarily skips a
    /// sample, such as while a drag is over a UI overlay. The next ray starts
    /// with a fresh dab instead of sweeping across the skipped region.
    pub fn break_stroke_path(&mut self) {
        self.stroke_path = None;
        self.dab_path.clear();
        self.dab_path_spacing = 0.0;
        self.dab_axis = None;
    }

    /// Raycast the pointer, sweep the short path from the previous step's end
    /// or the reachable tail of a long jump (or stamp once for a press or hold),
    /// then deform and remesh once.
    /// Returns the dirty vertices, where the ray lands on the edited surface,
    /// and whether a dab ran. A miss or a break in the path clears the path,
    /// so the next call starts fresh.
    #[allow(clippy::too_many_arguments)]
    // one pointer segment: trace, deform and remesh.
    #[allow(clippy::too_many_lines)]
    fn stroke_step<C: Cancellation>(
        &mut self,
        origin: DVec3,
        dir: DVec3,
        radius: f64,
        strength: f64,
        mode: BrushMode,
        hold: bool,
        spacing: f64,
        constraints: SculptRayConstraints<'_>,
        cancel: &C,
    ) -> Result<(Vec<u32>, Option<(DVec3, DVec3)>, bool), C::Error> {
        ensure_not_cancelled(cancel, CancelPoint::Entry)?;
        let endpoint_hit =
            self.raycast_visible_for_brush_with_cancel(origin, dir, constraints, cancel)?;
        ensure_not_cancelled(cancel, CancelPoint::Entry)?;
        let Some((endpoint, endpoint_normal)) = endpoint_hit else {
            // A miss is "the pointer left the surface": the cursor must
            // clear, not stay on the last applied dab.
            self.stroke_path = None;
            self.dab_axis = None;
            return Ok((Vec::new(), None, false));
        };
        let Some(endpoint_triangle) = self.hit_triangle else {
            self.stroke_path = None;
            return Ok((Vec::new(), None, false));
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
                cancel,
            )?,
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
            ensure_not_cancelled(cancel, CancelPoint::Entry)?;
            self.dab_path.clear();
            if !continuous {
                self.stroke_path = None;
                self.dab_axis = None;
                return Ok((Vec::new(), None, false));
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
            return Ok((Vec::new(), Some((endpoint, endpoint_normal)), false));
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
        ensure_not_cancelled(cancel, CancelPoint::BeforeDab)?;
        let moved = self.dab(&Dab {
            center,
            radius,
            strength,
            view: dir,
            mode,
        });
        ensure_not_cancelled(cancel, CancelPoint::AfterDab)?;
        self.dab_path.clear();
        if !continuous {
            // The swept prefix is applied; the break itself starts a new
            // stroke segment on the next call.
            self.stroke_path = None;
            self.dab_axis = None;
            return Ok((moved, None, true));
        }
        let current_hit =
            self.raycast_visible_for_brush_with_cancel(origin, dir, constraints, cancel)?;
        ensure_not_cancelled(cancel, CancelPoint::Finish)?;
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
        Ok((
            moved,
            Some(current_hit.unwrap_or((endpoint, endpoint_normal))),
            true,
        ))
    }

    /// Sample the pointer's path on the current surface into `dab_path`, from
    /// the previous step's end to this ray's hit. Samples come from rays
    /// interpolated between the two pointer rays, so the path follows the
    /// surface the operator sees. Long jumps start at a bounded recent tail.
    // the segment's endpoints and sampling controls are one trace.
    #[allow(clippy::too_many_arguments)]
    fn trace_path<C: Cancellation>(
        &mut self,
        previous: StrokePathState,
        ray: (DVec3, DVec3, SculptRayConstraints<'_>),
        end: (DVec3, DVec3, u32),
        radius: f64,
        spacing: f64,
        cancel: &C,
    ) -> Result<PathTrace, C::Error> {
        ensure_not_cancelled(cancel, CancelPoint::Entry)?;
        let (endpoint, endpoint_normal, endpoint_triangle) = end;
        let sample_step = (radius * PATH_SAMPLE_SHARE).max(MIN_SPACING_MM);
        let reach = Self::step_reach(radius).max(sample_step);
        let travel = (endpoint - previous.hit).length();
        if !travel.is_finite() {
            return Ok(PathTrace::TooFar);
        }
        let mut last = (0.0, previous.hit, previous.normal);
        let mut start_triangle = previous.triangle;
        let mut stretch = travel;
        if travel > reach {
            let mut amount = 1.0 - reach / travel;
            let mut start = None;
            for _ in 0..STRETCH_START_TRIES {
                ensure_not_cancelled(cancel, CancelPoint::Entry)?;
                let Some((hit, normal, triangle)) =
                    self.path_ray_hit(previous, ray, amount, cancel)?
                else {
                    break;
                };
                let span = (endpoint - hit).length();
                if span <= reach {
                    start = Some(((amount, hit, normal), triangle, span));
                    break;
                }
                amount = 1.0 - (1.0 - amount) * reach / span;
            }
            let Some((from, triangle, span)) = start else {
                return Ok(PathTrace::TooFar);
            };
            last = from;
            start_triangle = triangle;
            stretch = span;
        }
        self.dab_path.push(PathSample {
            point: last.1,
            triangle: start_triangle,
        });
        let first = last.0;
        let count = ((stretch / sample_step).ceil() as usize).max(1);
        for index in 1..=count {
            ensure_not_cancelled(cancel, CancelPoint::Entry)?;
            let amount = first + (1.0 - first) * index as f64 / count as f64;
            let next = if index == count {
                (amount, endpoint, endpoint_normal, endpoint_triangle)
            } else {
                let Some((hit, normal, triangle)) =
                    self.path_ray_hit(previous, ray, amount, cancel)?
                else {
                    return Ok(PathTrace::Broken);
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
                cancel,
            )? {
                return Ok(PathTrace::Broken);
            }
            last = (next.0, next.1, next.2);
        }
        ensure_not_cancelled(cancel, CancelPoint::Finish)?;
        Ok(PathTrace::Continuous)
    }

    /// The surface under the pointer ray `amount` of the way from the
    /// previous step's ray to this call's, with the face it hit.
    fn path_ray_hit<C: Cancellation>(
        &mut self,
        previous: StrokePathState,
        ray: (DVec3, DVec3, SculptRayConstraints<'_>),
        amount: f64,
        cancel: &C,
    ) -> Result<Option<(DVec3, DVec3, u32)>, C::Error> {
        ensure_not_cancelled(cancel, CancelPoint::PathRaycast)?;
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
        let Some((hit, normal)) =
            self.raycast_visible_for_brush_with_cancel(ray_origin, ray_dir, used, cancel)?
        else {
            return Ok(None);
        };
        ensure_not_cancelled(cancel, CancelPoint::PathRaycast)?;
        let Some(triangle) = self.hit_triangle else {
            return Ok(None);
        };
        Ok(Some((hit, normal, triangle)))
    }

    /// Append `to` after `from`. While the two hits are too far apart for one
    /// continuous stretch, the ray interval between them is bisected, so a
    /// steep flank or a ridge crossing keeps its path instead of breaking it.
    // the segment, its bisection depth and the scale are one continuity test.
    #[allow(clippy::too_many_arguments)]
    fn join_path<C: Cancellation>(
        &mut self,
        previous: StrokePathState,
        ray: (DVec3, DVec3, SculptRayConstraints<'_>),
        from: (f64, DVec3, DVec3),
        to: (f64, DVec3, DVec3, u32),
        scale: (f64, f64),
        depth: u32,
        cancel: &C,
    ) -> Result<bool, C::Error> {
        ensure_not_cancelled(cancel, CancelPoint::Entry)?;
        let (radius, spacing) = scale;
        if Self::continuous_hit((from.1, from.2), to.1, to.2, radius, spacing) {
            self.dab_path.push(PathSample {
                point: to.1,
                triangle: to.3,
            });
            return Ok(true);
        }
        if depth == 0 {
            return Ok(false);
        }
        let amount = f64::midpoint(from.0, to.0);
        let Some((hit, normal, triangle)) = self.path_ray_hit(previous, ray, amount, cancel)? else {
            return Ok(false);
        };
        if !self.join_path(
            previous,
            ray,
            from,
            (amount, hit, normal, triangle),
            scale,
            depth - 1,
            cancel,
        )? {
            return Ok(false);
        }
        self.join_path(
            previous,
            ray,
            (amount, hit, normal),
            to,
            scale,
            depth - 1,
            cancel,
        )
    }

    /// Raycast the current deformed surface (3D-DDA through the triangle
    /// buckets). Returns (hit point, triangle normal). Both winding sides hit
    /// (the ray decides the brush anchor, frontface culling is per-vertex).
    /// Exact hit-distance ties select the lower live triangle id.
    pub fn raycast(&mut self, orig: DVec3, dir: DVec3) -> Option<(DVec3, DVec3)> {
        self.raycast_visible(orig, dir, SculptRayConstraints::default())
    }

    /// Like [`Self::raycast`], restricted to a camera interval and halfspaces.
    /// Its normal is the split display normal; brush path continuity uses the
    /// welded-normal picker below.
    pub fn raycast_visible(
        &mut self,
        orig: DVec3,
        dir: DVec3,
        constraints: SculptRayConstraints<'_>,
    ) -> Option<(DVec3, DVec3)> {
        self.raycast_visible_with_normal(orig, dir, constraints, false)
    }

    pub(super) fn raycast_visible_for_brush(
        &mut self,
        orig: DVec3,
        dir: DVec3,
        constraints: SculptRayConstraints<'_>,
    ) -> Option<(DVec3, DVec3)> {
        self.raycast_visible_with_normal(orig, dir, constraints, true)
    }

    fn raycast_visible_with_normal(
        &mut self,
        orig: DVec3,
        dir: DVec3,
        constraints: SculptRayConstraints<'_>,
        brush_normal: bool,
    ) -> Option<(DVec3, DVec3)> {
        match self.raycast_visible_with_normal_inner(
            orig,
            dir,
            constraints,
            brush_normal,
            &NeverCancel,
        ) {
            Ok(hit) => hit,
            Err(never) => match never {},
        }
    }

    fn raycast_visible_for_brush_with_cancel<C: Cancellation>(
        &mut self,
        orig: DVec3,
        dir: DVec3,
        constraints: SculptRayConstraints<'_>,
        cancel: &C,
    ) -> Result<Option<(DVec3, DVec3)>, C::Error> {
        self.raycast_visible_with_normal_inner(orig, dir, constraints, true, cancel)
    }

    // the 3D-DDA walk is one loop.
    #[allow(clippy::too_many_lines)]
    fn raycast_visible_with_normal_inner<C: Cancellation>(
        &mut self,
        orig: DVec3,
        dir: DVec3,
        constraints: SculptRayConstraints<'_>,
        brush_normal: bool,
        cancel: &C,
    ) -> Result<Option<(DVec3, DVec3)>, C::Error> {
        ensure_not_cancelled(cancel, CancelPoint::Entry)?;
        self.hit_triangle = None;
        if !constraints.valid(orig, dir) {
            return Ok(None);
        }
        let dir = dir.normalize_or_zero();
        let cell = self.rays.cell;
        let lo = self.rays.lo;
        let (min_cell, max_cell) = (self.rays.min_cell, self.rays.max_cell);
        let min_cell = (
            i64::from(min_cell.0),
            i64::from(min_cell.1),
            i64::from(min_cell.2),
        );
        let max_cell = (
            i64::from(max_cell.0),
            i64::from(max_cell.1),
            i64::from(max_cell.2),
        );
        self.ray_test_epoch = self.ray_test_epoch.wrapping_add(1);
        if self.ray_test_epoch == 0 {
            self.ray_test_marks.fill(0);
            self.ray_test_epoch = 1;
        }
        let test_epoch = self.ray_test_epoch;
        let verts = &self.verts;
        let tris = &self.tris;
        let mut best: Option<(f64, u32)> = None;
        // Broad and unrepresentable triangle spans are tested even when the
        // regular-grid slab misses. All tests share the same ray, camera and
        // clipping predicates and use a dense id tie-break below.
        for &triangle in &self.rays.spilled_triangles {
            ensure_not_cancelled(cancel, CancelPoint::Raycast)?;
            consider_ray_triangle(
                triangle,
                verts,
                tris,
                &mut self.ray_test_marks,
                test_epoch,
                orig,
                dir,
                constraints,
                &mut best,
            );
        }

        let mut exact_fallback = false;
        if !self.rays.map.is_empty() {
            // The grid bounds grow with the geometry while cell indices remain
            // relative to its fixed origin `lo`, also the bucket key base.
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
            let mut intersects_grid = true;
            for k in 0..3 {
                let (o, d, l, h) = match k {
                    0 => (orig.x, dir.x, box_lo.x, hi.x),
                    1 => (orig.y, dir.y, box_lo.y, hi.y),
                    _ => (orig.z, dir.z, box_lo.z, hi.z),
                };
                if d.abs() < 1e-12 {
                    if o < l || o > h {
                        intersects_grid = false;
                        break;
                    }
                } else {
                    let (a, b) = ((l - o) / d, (h - o) / d);
                    t0 = t0.max(a.min(b));
                    t1 = t1.min(a.max(b));
                }
            }
            if t0 > t1 {
                intersects_grid = false;
            }
            if intersects_grid {
                let start = orig + (dir * (t0 + 1e-9));
                let mut cx = ((start.x - lo.x) / cell).floor() as i64;
                let mut cy = ((start.y - lo.y) / cell).floor() as i64;
                let mut cz = ((start.z - lo.z) / cell).floor() as i64;
                let step = (
                    if dir.x > 0.0 { 1i64 } else { -1 },
                    if dir.y > 0.0 { 1i64 } else { -1 },
                    if dir.z > 0.0 { 1i64 } else { -1 },
                );
                let next_t = |c: i64, o: f64, d: f64, l: f64| -> f64 {
                    if d.abs() < 1e-12 {
                        return f64::INFINITY;
                    }
                    let edge = l + (c as f64 + f64::from(u8::from(d > 0.0))) * cell;
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
                // Preserve the existing grid-relative guard while ensuring
                // traversal is O(live faces). A cutover exact-tests every
                // unvisited live face, so the bound cannot become a ray miss.
                let cell_span = (max_cell.0 - min_cell.0)
                    .max(max_cell.1 - min_cell.1)
                    .max(max_cell.2 - min_cell.2);
                let cell_guard = cell_span.saturating_mul(3).saturating_add(16);
                let face_guard = (self.rays.triangle_cells.len() as i64)
                    .saturating_mul(3)
                    .saturating_add(16);
                let guard_limit = cell_guard.min(face_guard);
                let mut guard = 0i64;
                loop {
                    guard += 1;
                    ensure_not_cancelled(cancel, CancelPoint::Raycast)?;
                    if guard > guard_limit {
                        exact_fallback = true;
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
                    let list = match [cx, cy, cz].map(i32::try_from) {
                        [Ok(x), Ok(y), Ok(z)] => self.rays.map.get(&(x, y, z)),
                        _ => None,
                    };
                    if let Some(list) = list {
                        for &triangle in list {
                            ensure_not_cancelled(cancel, CancelPoint::Raycast)?;
                            consider_ray_triangle(
                                triangle,
                                verts,
                                tris,
                                &mut self.ray_test_marks,
                                test_epoch,
                                orig,
                                dir,
                                constraints,
                                &mut best,
                            );
                        }
                    }
                    // The first cell with a confirmed hit INSIDE it ends the
                    // walk; later cells cannot have a nearer regular hit.
                    if let Some((best_t, _)) = best {
                        let cell_exit = tmx.min(tmy).min(tmz);
                        if best_t < cell_exit {
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
            }
        }
        if exact_fallback {
            for triangle in 0..self.rays.triangle_cells.len() {
                ensure_not_cancelled(cancel, CancelPoint::ExactFallback)?;
                consider_ray_triangle(
                    triangle as u32,
                    verts,
                    tris,
                    &mut self.ray_test_marks,
                    test_epoch,
                    orig,
                    dir,
                    constraints,
                    &mut best,
                );
            }
        }
        let v = |i: u32| {
            let offset = i as usize * 3;
            DVec3::new(
                verts[offset] as f64,
                verts[offset + 1] as f64,
                verts[offset + 2] as f64,
            )
        };
        let Some((t, ti)) = best else {
            ensure_not_cancelled(cancel, CancelPoint::Finish)?;
            return Ok(None);
        };
        let k = ti as usize * 3;
        let (a, b, c) = (v(tris[k]), v(tris[k + 1]), v(tris[k + 2]));
        let face = (b - a).cross(c - a);
        let face_normal = face.normalize_or_zero();
        let hit = orig + dir * t;
        let normal = if face.length_squared() > 1e-24 {
            let wa = (c - b).cross(hit - b).dot(face) / face.length_squared();
            let wb = (a - c).cross(hit - c).dot(face) / face.length_squared();
            let wc = 1.0 - wa - wb;
            let corner_normal = |vertex| {
                if brush_normal {
                    let offset = vertex as usize * 3;
                    DVec3::new(
                        self.brush_normals[offset] as f64,
                        self.brush_normals[offset + 1] as f64,
                        self.brush_normals[offset + 2] as f64,
                    )
                } else {
                    self.display_n(vertex)
                }
            };
            let smooth = (corner_normal(tris[k]) * wa
                + corner_normal(tris[k + 1]) * wb
                + corner_normal(tris[k + 2]) * wc)
                .normalize_or_zero();
            if smooth.length() > 0.5 && smooth.dot(face_normal) > 0.2 {
                smooth
            } else {
                face_normal
            }
        } else {
            face_normal
        };
        ensure_not_cancelled(cancel, CancelPoint::Finish)?;
        self.hit_triangle = Some(ti);
        Ok(Some((hit, normal)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct CancelAfterPathRaycasts {
        seen: Cell<usize>,
        cancel_on: usize,
    }

    impl Cancellation for CancelAfterPathRaycasts {
        type Error = Cancelled;

        fn is_cancelled(&self, point: CancelPoint) -> Option<Self::Error> {
            if point != CancelPoint::PathRaycast {
                return None;
            }
            let seen = self.seen.get();
            self.seen.set(seen + 1);
            (seen >= self.cancel_on).then_some(Cancelled)
        }
    }

    fn cancellable_path_grid() -> SculptSession {
        let side = 5usize;
        let spacing = 1.0f64;
        let half = (side - 1) as f64 * spacing * 0.5;
        let mut verts = Vec::with_capacity(side * side * 3);
        for y in 0..side {
            for x in 0..side {
                verts.extend_from_slice(&[
                    (x as f64 * spacing - half) as f32,
                    (y as f64 * spacing - half) as f32,
                    0.0,
                ]);
            }
        }
        let index = |x: usize, y: usize| (y * side + x) as u32;
        let mut tris = Vec::with_capacity((side - 1) * (side - 1) * 6);
        for y in 0..side - 1 {
            for x in 0..side - 1 {
                let (a, b, c, d) = (
                    index(x, y),
                    index(x + 1, y),
                    index(x, y + 1),
                    index(x + 1, y + 1),
                );
                tris.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
        SculptSession::new(verts, tris).expect("valid grid fixture")
    }

    #[test]
    fn cancellable_path_cancels_between_path_raycasts_without_a_returned_journal() {
        let mut session = cancellable_path_grid();
        session.start_stroke();
        session.remesh_armed = false;
        session.set_dab_elapsed_ms(crate::DWELL_FULL_DOSE_MS);
        session
            .dab_at_ray_visible_with_cancel(
                DVec3::new(0.0, 0.0, 5.0),
                -DVec3::Z,
                1.0,
                0.0,
                BrushMode::Deposit,
                false,
                SculptRayConstraints::default(),
                &NeverCancel,
            )
            .expect("the first ray establishes a path");
        let journal_before = (
            session.topo_journal.added_verts.len(),
            session.topo_journal.added_tris.len(),
            session.topo_journal.rewired.len(),
            session.topo_journal.collapsed.len(),
        );
        let cancel = CancelAfterPathRaycasts {
            seen: Cell::new(0),
            cancel_on: 2,
        };
        let outcome = session.dab_at_ray_visible_with_cancel(
            DVec3::new(1.5, 0.0, 5.0),
            -DVec3::Z,
            1.0,
            0.5,
            BrushMode::Deposit,
            false,
            SculptRayConstraints::default(),
            &cancel,
        );
        assert!(
            outcome.is_err(),
            "cancellation between path raycasts must be reported, not a miss"
        );
        assert!(
            cancel.seen.get() >= 3,
            "the fixture must reach the second path raycast before cancellation"
        );
        let journal_after = (
            session.topo_journal.added_verts.len(),
            session.topo_journal.added_tris.len(),
            session.topo_journal.rewired.len(),
            session.topo_journal.collapsed.len(),
        );
        assert_eq!(
            journal_before, journal_after,
            "a cancelled path step must not publish a topology journal slice"
        );
    }

    #[test]
    fn cancellable_ray_call_separates_cancellation_from_a_true_miss() {
        let mut session = cancellable_path_grid();
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let outcome = session.dab_at_ray_visible_cancellable(
            DVec3::new(0.0, 0.0, 5.0),
            -DVec3::Z,
            1.0,
            0.5,
            BrushMode::Deposit,
            false,
            SculptRayConstraints::default(),
            &cancelled,
        );
        assert!(
            outcome.is_none(),
            "a cancellation must not look like a successful empty ray miss"
        );

        let mut session = cancellable_path_grid();
        let live = std::sync::atomic::AtomicBool::new(false);
        let outcome = session
            .dab_at_ray_visible_cancellable(
                DVec3::new(0.0, 0.0, 5.0),
                DVec3::Z,
                1.0,
                0.5,
                BrushMode::Deposit,
                false,
                SculptRayConstraints::default(),
                &live,
            )
            .expect("an uncancelled ray call returns a result");
        assert!(
            outcome.hit.is_none(),
            "a true miss stays a completed empty result"
        );
    }

    #[test]
    fn ray_grid_walk_does_not_overflow_at_the_cell_index_limit() {
        let mut session = SculptSession::new(
            vec![
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                0.0,
                0.0,
                1.0,
                -4_294_967_296.0,
                0.0,
                0.0,
            ],
            vec![0, 1, 2],
        )
        .expect("valid mesh fixture");
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.raycast(DVec3::new(1.0, 0.25, 0.25), -DVec3::X)
        }));
        assert!(
            outcome.is_ok(),
            "an isolated distant vertex must not overflow ray bounds"
        );
        let (hit, _) = outcome.expect("no panic").expect("visible triangle");
        assert!((hit - DVec3::new(0.0, 0.25, 0.25)).length() < 1e-9);
    }

    #[test]
    fn empty_surface_rays_return_no_hit_without_overflow() {
        for verts in [Vec::new(), vec![0.0, 0.0, 0.0]] {
            let mut session = SculptSession::new(verts, Vec::new()).expect("valid mesh fixture");
            for direction in [DVec3::X, DVec3::Z, DVec3::ONE] {
                assert!(session.raycast(DVec3::ZERO, direction).is_none());
            }
        }
    }

    #[test]
    fn spilled_broad_triangle_obeys_near_far_and_clip_constraints() {
        let mut session = SculptSession::new(
            vec![
                0.0, 0.0, 0.0, 20_000.0, 0.0, 0.0, 0.0, 20_000.0, 0.0,
                0.0, 0.0, 100.0, 1.0, 0.0, 100.0, 0.0, 1.0, 100.0,
            ],
            vec![0, 1, 2, 3, 4, 5],
        )
        .expect("finite broad face remains a valid sculpt surface");
        assert_eq!(session.rays.spilled_triangles, vec![0]);
        assert!(!session.rays.map.is_empty());
        let origin = DVec3::new(6_000.0, 6_000.0, 10.0);
        let direction = -DVec3::Z;

        let keep_x_from_five_thousand = [[1.0, 0.0, 0.0, -5_000.0]];
        let hit = session
            .raycast_visible(
                origin,
                direction,
                SculptRayConstraints {
                    near: 9.0,
                    far: 10.0,
                    clip_planes: &keep_x_from_five_thousand,
                },
            )
            .expect("the spill scan keeps a hit on the visible ray interval");
        assert!((hit.0 - DVec3::new(6_000.0, 6_000.0, 0.0)).length() < 1e-9);

        assert!(session
            .raycast_visible(
                origin,
                direction,
                SculptRayConstraints {
                    near: 0.0,
                    far: 9.0,
                    clip_planes: &[],
                },
            )
            .is_none());

        let clip_that_excludes_hit = [[1.0, 0.0, 0.0, -7_000.0]];
        assert!(session
            .raycast_visible(
                origin,
                direction,
                SculptRayConstraints {
                    near: 0.0,
                    far: 20.0,
                    clip_planes: &clip_that_excludes_hit,
                },
            )
            .is_none());
    }

    #[test]
    fn dda_cutover_exactly_checks_unvisited_regular_faces() {
        let mut session = SculptSession::new(
            vec![
                0.0, -1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 0.0, 1.0,
                1_000.0, -1.0, -1.0, 1_000.0, 1.0, -1.0, 1_000.0, 0.0, 1.0,
            ],
            vec![0, 1, 2, 3, 4, 5],
        )
        .expect("valid sparse mesh fixture");
        assert!(session.rays.spilled_triangles.is_empty());

        let (hit, _) = session
            .raycast(DVec3::ZERO, DVec3::X)
            .expect("the distant regular face is found after DDA cutover");
        assert!((hit - DVec3::new(1_000.0, 0.0, 0.0)).length() < 1e-9);
        assert_eq!(session.hit_triangle, Some(1));
    }

    #[test]
    fn opposite_normal_coincident_faces_choose_the_lower_triangle_id() {
        let mut session = SculptSession::new(
            vec![
                -1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0,
                -1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, 0.0,
            ],
            vec![0, 1, 2, 3, 4, 5],
        )
        .expect("opposite winding faces remain valid");
        for triangles in session.rays.map.values_mut() {
            triangles.reverse();
        }
        let (_, normal) = session
            .raycast(DVec3::Z, -DVec3::Z)
            .expect("the coincident faces are hit");

        assert_eq!(session.hit_triangle, Some(0));
        assert!(normal.z > 0.9, "lower-id face normal should win: {normal:?}");
    }
}
