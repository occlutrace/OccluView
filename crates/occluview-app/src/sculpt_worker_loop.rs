#![cfg_attr(test, allow(clippy::panic))]

use super::{
    Arc, DabDispatchClock, DabFailure, DabOutcome, Ordering, SculptCommand, SculptCommandQueue,
    SculptCompletion, SculptFailure, SculptSession, WorkerState,
};
use std::time::Instant;

// One command loop: each arm is a complete command and they read in order.
#[allow(clippy::too_many_lines)]
pub(super) fn run_worker(
    mut session: SculptSession,
    queue: Arc<SculptCommandQueue>,
    state: Arc<WorkerState>,
    pool: rayon::ThreadPool,
) {
    let mut dose_clock = DabDispatchClock::default();
    while let Some(command) = queue.pop() {
        maybe_panic_for_tests(&state);
        if state.stopping.load(Ordering::Acquire) {
            queue.mark_idle();
            break;
        }
        match command {
            SculptCommand::RayStep { step, .. } => {
                state.begin_geometry_update();
                let elapsed_ms = dose_clock.next_elapsed_ms(Instant::now());
                #[cfg(test)]
                state.record_input(super::SculptWorkerInput::Ray {
                    mode: step.mode,
                    hold: step.hold,
                    preserve_skirt: step.preserve_skirt,
                    elapsed_ms,
                });
                let outcome = pool.install(|| {
                    session.apply_ray_step_cancellable(&step, elapsed_ms, &state.stopping)
                });
                if !publish_apply(outcome, &state, &queue) {
                    break;
                }
            }
            SculptCommand::BreakPath { .. } => {
                #[cfg(test)]
                state.record_input(super::SculptWorkerInput::BreakPath);
                session.session.break_ray_path();
            }
            SculptCommand::PrimeWallRegion {
                center,
                radius_mm,
                budget,
            } => {
                session.session.prime_wall_region(center, radius_mm, budget);
            }
            #[cfg(test)]
            SculptCommand::Apply {
                stroke_id: _,
                stroke,
                mode,
                tip,
                axis,
                dose,
            } => {
                state.begin_geometry_update();
                let Some(outcome) = pool.install(|| {
                    session.apply_dab_cancellable(stroke, mode, &state.stopping, tip, axis, dose)
                }) else {
                    state.finish_geometry_update();
                    queue.mark_idle();
                    break;
                };
                if state.stopping.load(Ordering::Acquire) {
                    state.finish_geometry_update();
                    queue.mark_idle();
                    break;
                }
                if let Some(failure) = outcome.failure {
                    let failure = match failure {
                        DabFailure::ShadowPoisoned => SculptFailure::ShadowPoisoned,
                        DabFailure::ShadowShapeMismatch {
                            shadow_count,
                            live_count,
                        } => {
                            tracing::error!(
                                shadow_count,
                                live_count,
                                "sculpt display shadow shape differs from kernel mesh"
                            );
                            SculptFailure::ShadowShapeMismatch
                        }
                        DabFailure::InvalidVertexIndex {
                            vertex_id,
                            vertex_count,
                        } => {
                            tracing::error!(
                                vertex_id,
                                vertex_count,
                                "sculpt kernel returned an invalid vertex id"
                            );
                            SculptFailure::InvalidVertexIndex
                        }
                    };
                    state.set_error(failure);
                    state.finish_geometry_update();
                    queue.mark_idle();
                    break;
                }
                if let Some(delta) = outcome.topology_delta {
                    state.record_topology(delta);
                } else {
                    state.record_touched(outcome.touched, outcome.dirty_triangles);
                }
                state.finish_geometry_update();
                if state.has_error() {
                    queue.mark_idle();
                    break;
                }
            }
            SculptCommand::Finish => {
                #[cfg(test)]
                state.record_input(super::SculptWorkerInput::Finish);
                state.begin_geometry_update();
                session.session.finish_stroke();
                let dirty = session.dirty_stroke;
                session.dirty_stroke = false;
                let topology_dirty = session.topology_dirty_stroke;
                session.topology_dirty_stroke = false;
                let start_mesh = session.stroke_start_mesh.take();
                if dirty {
                    let Some(before) = start_mesh else {
                        state.set_error(SculptFailure::MissingUndoBaseline);
                        state.finish_geometry_update();
                        queue.mark_idle();
                        // This is a terminal worker invariant failure. Do
                        // not consume later commands after publishing the
                        // error: their output would be ordered after a
                        // stroke whose undo boundary was lost.
                        break;
                    };
                    let Ok(shadow) = session.shadow.read() else {
                        state.set_error(SculptFailure::ShadowPoisoned);
                        state.finish_geometry_update();
                        queue.mark_idle();
                        break;
                    };
                    let vertices = shadow.clone();
                    let mesh = if topology_dirty {
                        occluview_core::mesh_from_sculpt_session_like(
                            &session.base_mesh,
                            &session.session,
                        )
                        .map(Arc::new)
                        .map_err(|error| SculptFailure::TopologyRebuild {
                            detail: error.to_string(),
                        })
                    } else {
                        session
                            .base_mesh
                            .with_sculpted_vertices(vertices)
                            .map(Arc::new)
                            .ok_or(SculptFailure::VertexCountChanged)
                    };
                    let mesh = match mesh {
                        Ok(mesh) => mesh,
                        Err(failure) => {
                            state.set_error(failure);
                            state.finish_geometry_update();
                            queue.mark_idle();
                            break;
                        }
                    };
                    mesh.warm_bvh();
                    session.base_mesh = Arc::clone(&mesh);
                    session.topology = occluview_render::PreparedSceneTopology::from_mesh(&mesh);
                    session.topology_id = mesh.topology_id();
                    state
                        .reset_pick_geometry(Arc::clone(&mesh), session.session.indices().to_vec());
                    state.finish_geometry_update();
                    if !state.push_completion(SculptCompletion { before, mesh }) {
                        queue.mark_idle();
                        break;
                    }
                } else {
                    state.finish_geometry_update();
                }
            }
        }
        queue.mark_idle();
    }
}

fn publish_apply(
    outcome: Option<DabOutcome>,
    state: &WorkerState,
    queue: &SculptCommandQueue,
) -> bool {
    let Some(outcome) = outcome else {
        state.finish_geometry_update();
        queue.mark_idle();
        return false;
    };
    if let Some(failure) = outcome.failure {
        let failure = match failure {
            DabFailure::ShadowPoisoned => SculptFailure::ShadowPoisoned,
            DabFailure::ShadowShapeMismatch {
                shadow_count,
                live_count,
            } => {
                tracing::error!(
                    shadow_count,
                    live_count,
                    "sculpt display shadow shape differs from kernel mesh"
                );
                SculptFailure::ShadowShapeMismatch
            }
            DabFailure::InvalidVertexIndex {
                vertex_id,
                vertex_count,
            } => {
                tracing::error!(
                    vertex_id,
                    vertex_count,
                    "sculpt kernel returned an invalid vertex id"
                );
                SculptFailure::InvalidVertexIndex
            }
        };
        state.set_error(failure);
        state.finish_geometry_update();
        queue.mark_idle();
        return false;
    }
    if let Some(delta) = outcome.topology_delta {
        state.record_topology(delta);
    } else {
        state.record_touched(outcome.touched, outcome.dirty_triangles);
    }
    state.finish_geometry_update();
    if state.has_error() {
        queue.mark_idle();
        return false;
    }
    true
}

/// Test-only: unwind at the worker's command boundary when a test armed the
/// panic trigger, so the `catch_unwind` in `SculptWorker::spawn` — the real
/// production guard — is what converts the dead thread into a typed failure.
#[cfg(test)]
#[allow(clippy::panic)]
fn maybe_panic_for_tests(state: &WorkerState) {
    assert!(
        !state.panic_on_next_command.swap(false, Ordering::AcqRel),
        "sculpt worker body panicked for the test"
    );
}

#[cfg(not(test))]
fn maybe_panic_for_tests(_state: &WorkerState) {}

pub(super) fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}
