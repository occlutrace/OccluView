mod job;
mod scene_apply;
mod state;

pub(crate) use job::{BridgeSplitJobInput, BridgeSplitJobOutput, BridgeSplitWorker};
pub(crate) use scene_apply::apply_preview_to_scene;
#[cfg(test)]
pub(crate) use scene_apply::{BridgeSplitSceneApplyError, BridgeSplitSceneResult};
#[cfg(test)]
pub(crate) use state::{
    clamp_bridge_split_kerf_mm, next_nonzero_session_id, BridgeSplitGuard,
    DEFAULT_BRIDGE_SPLIT_KERF_MM,
};
pub(crate) use state::{
    BridgeSplitMode, BridgeSplitPose, BridgeSplitSession, BridgeSplitTarget, BridgeSplitToolError,
    MAX_BRIDGE_SPLIT_KERF_MM, MIN_BRIDGE_SPLIT_KERF_MM,
};

use glam::Affine3A;
use occluview_core::{Mesh, SceneMesh};
use std::sync::Arc;

struct BridgeSplitSourceSnapshot {
    mesh: Arc<Mesh>,
    transform: Affine3A,
    target: BridgeSplitTarget,
}

#[derive(Default)]
pub(crate) struct BridgeSplitController {
    session: BridgeSplitSession,
    worker: BridgeSplitWorker,
    source: Option<BridgeSplitSourceSnapshot>,
}

impl BridgeSplitController {
    #[cfg(test)]
    fn with_worker(worker: BridgeSplitWorker) -> Self {
        Self {
            session: BridgeSplitSession::default(),
            worker,
            source: None,
        }
    }

    pub(crate) fn session(&self) -> &BridgeSplitSession {
        &self.session
    }

    pub(crate) fn session_mut(&mut self) -> &mut BridgeSplitSession {
        &mut self.session
    }

    pub(crate) fn start(&mut self, entry: &SceneMesh) {
        let target = BridgeSplitTarget::capture(entry);
        self.worker.clear_queued();
        self.source = Some(BridgeSplitSourceSnapshot {
            mesh: Arc::new(entry.mesh.clone()),
            transform: entry.transform,
            target,
        });
        self.session.start(target);
    }

    pub(crate) fn submit_current_request(&mut self, entry: &SceneMesh) -> bool {
        let live_target = BridgeSplitTarget::capture(entry);
        let Some(session_target) = self.session.target() else {
            return false;
        };
        let Some(source) = self.source.as_ref() else {
            return false;
        };
        if live_target != session_target || live_target != source.target {
            return false;
        }
        let Some(guard) = self.session.current_guard() else {
            return false;
        };
        if guard.target != source.target {
            return false;
        }
        let Some(request) = self.session.current_request() else {
            return false;
        };
        let input = BridgeSplitJobInput {
            mesh: Arc::clone(&source.mesh),
            transform: source.transform,
            request,
            guard,
        };
        match self.worker.submit(input) {
            Ok(()) => true,
            Err(error) => self.session.apply_job_output(
                self.session.target(),
                BridgeSplitJobOutput {
                    guard,
                    result: Err(error),
                },
            ),
        }
    }

    pub(crate) fn poll(&mut self, live_target: Option<BridgeSplitTarget>) -> bool {
        let mut changed = false;
        for output in self.worker.poll() {
            changed |= self.session.apply_job_output(live_target, output);
        }
        changed
    }

    pub(crate) fn cancel(&mut self) {
        self.source = None;
        self.session.cancel();
        self.worker.clear_queued();
    }
}

#[cfg(test)]
mod scene_apply_tests;
#[cfg(test)]
mod tests;
