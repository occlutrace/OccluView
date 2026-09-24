#![cfg_attr(test, allow(clippy::panic))]

use super::{
    Arc, DabFailure, Ordering, SculptCommand, SculptCommandQueue, SculptCompletion, SculptFailure,
    SculptSession, WorkerState,
};

pub(super) fn run_worker(
    mut session: SculptSession,
    queue: Arc<SculptCommandQueue>,
    state: Arc<WorkerState>,
    pool: rayon::ThreadPool,
) {
    while let Some(command) = queue.pop() {
        maybe_panic_for_tests(&state);
        if state.stopping.load(Ordering::Acquire) {
            queue.mark_idle();
            break;
        }
        match command {
            SculptCommand::Apply {
                stroke_id,
                stroke,
                mode,
            } => {
                let Some(outcome) =
                    pool.install(|| session.apply_dab_cancellable(stroke, mode, &state.stopping))
                else {
                    queue.mark_idle();
                    break;
                };
                if state.stopping.load(Ordering::Acquire) {
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
                        DabFailure::TopologyRebuild { detail } => {
                            SculptFailure::TopologyRebuild { detail }
                        }
                    };
                    state.set_error(failure);
                    queue.mark_idle();
                    break;
                }
                if let Some(rebuild) = outcome.rebuild {
                    state.record_rebuild(stroke_id, rebuild);
                } else {
                    state.record_touched(outcome.touched, outcome.dirty_triangles);
                }
                if state.has_error() {
                    queue.mark_idle();
                    break;
                }
            }
            SculptCommand::Finish {
                stroke_id: _stroke_id,
            } => {
                let dirty = session.dirty_stroke;
                session.dirty_stroke = false;
                let start_mesh = session.stroke_start_mesh.take();
                if dirty {
                    let Some(before) = start_mesh else {
                        state.set_error(SculptFailure::MissingUndoBaseline);
                        queue.mark_idle();
                        // This is a terminal worker invariant failure. Do
                        // not consume later commands after publishing the
                        // error: their output would be ordered after a
                        // stroke whose undo boundary was lost.
                        break;
                    };
                    let Ok(shadow) = session.shadow.read() else {
                        state.set_error(SculptFailure::ShadowPoisoned);
                        queue.mark_idle();
                        break;
                    };
                    let vertices = shadow.clone();
                    // `base_mesh` already tracks any mid-stroke rebuild, so the
                    // lengths match whether or not the stroke densified. Undo
                    // restores `before`, which still has the PRE-stroke
                    // topology — coarse triangles and all.
                    let mesh = session.base_mesh.with_sculpted_vertices(vertices);
                    if let Some(mesh) = mesh {
                        if !state.push_completion(SculptCompletion { before, mesh }) {
                            queue.mark_idle();
                            break;
                        }
                    } else {
                        state.set_error(SculptFailure::VertexCountChanged);
                        queue.mark_idle();
                        break;
                    }
                }
            }
        }
        queue.mark_idle();
    }
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
