//! Sculpt viewport input and dab scheduling.

use super::{egui, mesh_editor_overlay};
use crate::sculpt_kernel::{BrushMode, BrushStroke};
use crate::sculpt_tool::{
    SculptTip, SculptToolKind, StrokeState, DAB_SPACING_FRACTION, HOLD_DAB_INTERVAL_SEC,
    MAX_DABS_PER_FRAME, SCULPT_INTENSITY_MAX, SCULPT_INTENSITY_MIN, SCULPT_SIZE_MAX,
    SCULPT_SIZE_MIN, SCULPT_WHEEL_STEP,
};
use crate::sculpt_worker::SculptWorker;
use glam::Vec3;

/// What the pointer/keyboard said this frame, resolved once so the dab loop
/// does not re-read input.
pub(super) struct DabInput {
    pub(super) kind: SculptToolKind,
    pub(super) shift: bool,
    pub(super) dt: f32,
    /// Whether the primary button was pressed this frame, i.e. a fresh edge.
    ///
    /// A stroke may only begin on an edge. Carried through the input rather than
    /// re-read inside the dab loop so the frame that decides it is the same one
    /// that observed it.
    pub(super) fresh_press: bool,
}

/// A frame's dab request in world space plus the resolved kernel mode/strength;
/// [`schedule_dabs`] converts to the layer's local space and spaces the dabs.
pub(super) struct DabParams {
    pub(super) hit_world: Vec3,
    pub(super) view_world: Vec3,
    pub(super) radius_world: f32,
    pub(super) strength: f32,
    pub(super) mode: BrushMode,
    pub(super) tip: SculptTip,
    pub(super) dt: f32,
}

/// The stroke bearing a knife dab cuts along: the travel between two dab
/// centres with its component along the view removed, so the blade stays on
/// the surface the operator sees.
fn stroke_bearing(travel: Vec3, view: Vec3) -> Option<Vec3> {
    let on_surface = travel - view * travel.dot(view);
    (on_surface.length() > 1e-4).then(|| on_surface.normalize_or_zero())
}

pub(super) fn apply_sculpt_wheel_settings(ctx: &egui::Context) -> bool {
    let raw_scroll = super::app_input::raw_wheel_delta(ctx);
    let (shift, ctrl) = ctx.input(|input| {
        (
            input.modifiers.shift,
            input.modifiers.ctrl || input.modifiers.command,
        )
    });
    // Shift+wheel may arrive on either scroll axis.
    let scroll = if raw_scroll.y.abs() >= raw_scroll.x.abs() {
        raw_scroll.y
    } else {
        raw_scroll.x
    };
    if scroll.abs() < f32::EPSILON || !(shift || ctrl) {
        return false;
    }
    let delta = scroll.signum() * SCULPT_WHEEL_STEP;
    if shift {
        let next =
            (mesh_editor_overlay::sculpt_size(ctx) + delta).clamp(SCULPT_SIZE_MIN, SCULPT_SIZE_MAX);
        mesh_editor_overlay::set_sculpt_size(ctx, next);
    } else {
        let next = (mesh_editor_overlay::sculpt_intensity(ctx) + delta)
            .clamp(SCULPT_INTENSITY_MIN, SCULPT_INTENSITY_MAX);
        mesh_editor_overlay::set_sculpt_intensity(ctx, next);
    }
    true
}

/// Lay this frame's dabs on `session`, updating `stroke`'s scheduler state, and
/// return the touched vertex ids. The spacing decision is the pure
/// [`plan_dab_centers`]; this only converts to local space and applies.
pub(super) fn schedule_dabs(
    worker: &SculptWorker,
    stroke: &mut StrokeState,
    params: &DabParams,
) -> usize {
    let radius_local = (params.radius_world * worker.local_per_world).max(1e-4);
    let center = worker.world_to_local.transform_point3(params.hit_world);
    let view_local = worker
        .world_to_local
        .transform_vector3(params.view_world)
        .normalize_or_zero();
    let spacing = (radius_local * DAB_SPACING_FRACTION).max(1e-4);

    let (centers, last_dab, hold_seconds) = plan_dab_centers(
        stroke.last_dab_local,
        center,
        spacing,
        stroke.hold_seconds,
        params.dt,
    );
    let mut previous = stroke.last_dab_local;
    stroke.last_dab_local = last_dab;
    stroke.hold_seconds = hold_seconds;

    let mut queued = 0;
    for at in centers {
        // A press with no travel yet keeps the previous bearing, so a knife
        // dab at the start of a stroke still cuts along the last gesture
        // instead of leaving a round dimple.
        if let Some(axis) = previous.and_then(|last| stroke_bearing(at - last, view_local)) {
            stroke.last_axis = Some(axis);
        }
        queued += usize::from(worker.try_apply_tipped(
            BrushStroke {
                center: at.to_array(),
                radius_mm: radius_local,
                strength: params.strength,
                view_dir: view_local.to_array(),
            },
            params.mode,
            params.tip,
            stroke.last_axis.map(|axis| axis.to_array()),
        ));
        previous = Some(at);
    }
    queued
}

/// Pure dab scheduler: given the previous dab, the cursor `center`, the
/// `spacing`, and the hold accumulator, returns this frame's dab centers and the
/// updated `(last_dab, hold_seconds)`. Dabs are spaced by arc length while
/// moving and by a time cadence while (near) stationary, at most
/// [`MAX_DABS_PER_FRAME`] per frame. If the cursor jumps farther than that
/// budget, the segment is sampled evenly and the scheduler advances all the
/// way to the current point; this keeps input latency bounded instead of
/// building an invisible backlog of expensive dabs.
#[allow(clippy::cast_precision_loss)]
pub(super) fn plan_dab_centers(
    last_dab: Option<Vec3>,
    center: Vec3,
    spacing: f32,
    hold_seconds: f32,
    dt: f32,
) -> (Vec<Vec3>, Option<Vec3>, f32) {
    let Some(last) = last_dab else {
        return (vec![center], Some(center), 0.0);
    };
    let segment = center - last;
    let distance = segment.length();
    if distance >= spacing {
        if distance > spacing * MAX_DABS_PER_FRAME as f32 {
            let count = MAX_DABS_PER_FRAME as f32;
            let centers = (1..=MAX_DABS_PER_FRAME)
                .map(|step| last + segment * (step as f32 / count))
                .collect();
            return (centers, Some(center), 0.0);
        }
        let direction = segment / distance;
        let mut cursor = last;
        let mut walked = 0.0;
        let mut centers = Vec::new();
        while walked + spacing <= distance && centers.len() < MAX_DABS_PER_FRAME {
            cursor += direction * spacing;
            walked += spacing;
            centers.push(cursor);
        }
        (centers, Some(cursor), 0.0)
    } else {
        let mut hold = hold_seconds + dt.clamp(0.0, HOLD_DAB_INTERVAL_SEC * 4.0);
        let mut centers = Vec::new();
        while hold >= HOLD_DAB_INTERVAL_SEC && centers.len() < MAX_DABS_PER_FRAME {
            hold -= HOLD_DAB_INTERVAL_SEC;
            centers.push(center);
        }
        (centers, Some(last), hold)
    }
}
