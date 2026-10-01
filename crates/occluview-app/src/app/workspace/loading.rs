//! One decoder queue for every live scene in the workspace.
//!
//! Requests and decoded results keep the exact scene lifetime they were
//! accepted for. A scene switch therefore cannot redirect an in-flight load,
//! and closing a scene only retires its queued work: the one active decoder
//! keeps its slot until its thread actually returns.

use crate::app::workspace::id::SceneKey;
use crate::scene_loading::{DecodedSceneLoad, PendingSceneLoad, SceneLoadMode, SceneLoadRequest};
use std::collections::{HashSet, VecDeque};
use std::sync::mpsc::TryRecvError;

/// Workspace-owned import scheduler. The decoder and a result parked behind
/// an edit session are mutually exclusive, so decoded geometry remains part of
/// the same global import reservation and cannot overlap another parse.
#[derive(Default)]
pub(crate) struct LoadCoordinator {
    pub(crate) active: Option<PendingSceneLoad>,
    pub(crate) decoded: Option<DecodedSceneLoad>,
    pub(crate) queued: VecDeque<SceneLoadRequest>,
}

impl LoadCoordinator {
    /// Queue an accepted request. Replace invalidates only earlier work for
    /// its own scene lifetime; requests belonging to the other scene retain
    /// their original order. Appends accepted after a Replace remain behind it
    /// so a successful Replace can move them to the new scene lifetime.
    pub(crate) fn enqueue(&mut self, request: SceneLoadRequest) {
        if request.mode == SceneLoadMode::Replace {
            self.supersede_before_replace(request.scene_key, request.requested_at);
        }
        let position = self
            .queued
            .iter()
            .position(|queued| queued.requested_at > request.requested_at)
            .unwrap_or(self.queued.len());
        self.queued.insert(position, request);
    }

    /// A delayed Replace may be confirmed after Appends were accepted while
    /// its guard was open. Drop work accepted before that Replace, but preserve
    /// later Appends in global request order so they can follow the new scene.
    fn supersede_before_replace(&mut self, scene_key: SceneKey, requested_at: std::time::Instant) {
        if let Some(active) = self.active.as_mut() {
            if active.scene_key == scene_key && active.requested_at <= requested_at {
                active.superseded = true;
            }
        }
        self.queued.retain(|queued| {
            queued.scene_key != scene_key
                || (queued.mode == SceneLoadMode::Append && queued.requested_at > requested_at)
        });
        if self.decoded.as_ref().is_some_and(|decoded| {
            decoded.pending.scene_key == scene_key && decoded.pending.requested_at <= requested_at
        }) {
            self.decoded = None;
        }
    }

    /// Rebind only later Appends after a Replace has committed and advanced the
    /// scene epoch. Older work remains retired; requests for other scenes keep
    /// their positions in the shared queue.
    pub(crate) fn advance_scene_lifetime_after_replace(
        &mut self,
        old_key: SceneKey,
        new_key: SceneKey,
        requested_at: std::time::Instant,
    ) {
        self.queued.retain_mut(|request| {
            if request.scene_key != old_key {
                return true;
            }
            if request.mode == SceneLoadMode::Append && request.requested_at > requested_at {
                request.scene_key = new_key;
                return true;
            }
            false
        });
    }

    /// Put a decoded Replace back ahead of later work when another global
    /// modal currently owns the workspace. This retry does not supersede
    /// requests accepted after that Replace.
    pub(crate) fn requeue_front(&mut self, request: SceneLoadRequest) {
        self.queued.push_front(request);
    }

    /// Invalidate pending work for one target, including a decoded result
    /// waiting behind an Edit checkpoint. The active worker remains in its slot
    /// until it exits, preserving the single-decoder guarantee.
    pub(crate) fn supersede_scene(&mut self, scene_key: SceneKey) {
        if let Some(active) = self.active.as_mut() {
            if active.scene_key == scene_key {
                active.superseded = true;
            }
        }
        self.queued.retain(|request| request.scene_key != scene_key);
        if self
            .decoded
            .as_ref()
            .is_some_and(|decoded| decoded.pending.scene_key == scene_key)
        {
            self.decoded = None;
        }
    }

    /// Take the first eligible request for this scene. A blocked Append keeps
    /// its position relative to its own scene's later requests, but does not
    /// block eligible work for another scene. The single active/decoded slot
    /// still preserves the global decoder limit.
    pub(crate) fn take_next_for(
        &mut self,
        scene_key: SceneKey,
        append_blocked_scenes: &[SceneKey],
    ) -> Option<SceneLoadRequest> {
        if self.active.is_some() || self.decoded.is_some() {
            return None;
        }
        let mut earlier_scenes = HashSet::new();
        let eligible_position = self
            .queued
            .iter()
            .enumerate()
            .find_map(|(index, request)| {
                let first_for_scene = earlier_scenes.insert(request.scene_key);
                let append_is_blocked = request.mode == SceneLoadMode::Append
                    && append_blocked_scenes.contains(&request.scene_key);
                (first_for_scene && !append_is_blocked).then_some(index)
            })?;
        if self.queued[eligible_position].scene_key != scene_key {
            return None;
        }
        self.queued.remove(eligible_position)
    }

    pub(crate) fn active_for(&self, scene_key: SceneKey) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.scene_key == scene_key)
    }

    pub(crate) fn take_active_for(&mut self, scene_key: SceneKey) -> Option<PendingSceneLoad> {
        self.active
            .as_ref()
            .is_some_and(|active| active.scene_key == scene_key)
            .then(|| self.active.take())
            .flatten()
    }

    pub(crate) fn decoded_for(&self, scene_key: SceneKey) -> bool {
        self.decoded
            .as_ref()
            .is_some_and(|decoded| decoded.pending.scene_key == scene_key)
    }

    pub(crate) fn take_decoded_for(&mut self, scene_key: SceneKey) -> Option<DecodedSceneLoad> {
        self.decoded
            .as_ref()
            .is_some_and(|decoded| decoded.pending.scene_key == scene_key)
            .then(|| self.decoded.take())
            .flatten()
    }

    /// Remove work for closed or replaced scene lifetimes. A running decoder
    /// is only marked obsolete here and remains the sole active decoder until
    /// its receiver reports completion or disconnection.
    pub(crate) fn reap_retired(&mut self, live_keys: &[SceneKey]) {
        self.queued
            .retain(|request| live_keys.contains(&request.scene_key));
        if self
            .decoded
            .as_ref()
            .is_some_and(|decoded| !live_keys.contains(&decoded.pending.scene_key))
        {
            self.decoded = None;
        }

        let retired_active = self
            .active
            .as_ref()
            .is_some_and(|active| !live_keys.contains(&active.scene_key));
        if !retired_active {
            return;
        }
        if let Some(active) = self.active.as_mut() {
            active.superseded = true;
        }
        let received = self
            .active
            .as_ref()
            .map(|active| active.receiver.try_recv().map(|_| ())); // drop retired results
        match received {
            Some(Ok(()) | Err(TryRecvError::Disconnected)) => self.active = None,
            Some(Err(TryRecvError::Empty)) | None => {}
        }
    }

    /// A request can only start for the scene owning the earliest eligible item.
    /// Called after `take_next_for` succeeds and the single decoder slot is
    /// still empty.
    pub(crate) fn install_active(&mut self, active: PendingSceneLoad) {
        debug_assert!(self.active.is_none());
        debug_assert!(self.decoded.is_none());
        self.active = Some(active);
    }

    /// Retain a completed parse while its target is temporarily unable to
    /// accept it. There is deliberately only one parked result in the whole
    /// workspace; the next decoder stays blocked until this result is applied
    /// or retired.
    pub(crate) fn park_decoded(&mut self, decoded: DecodedSceneLoad) {
        debug_assert!(self.active.is_none());
        debug_assert!(self.decoded.is_none());
        self.decoded = Some(decoded);
    }

    /// Whether queued work exists for one exact scene lifetime.
    pub(crate) fn has_queued_for(&self, scene_key: SceneKey) -> bool {
        self.queued
            .iter()
            .any(|request| request.scene_key == scene_key)
    }

    pub(crate) fn has_work_for(&self, scene_key: SceneKey) -> bool {
        self.active_for(scene_key) || self.decoded_for(scene_key) || self.has_queued_for(scene_key)
    }

    /// Number of queued requests, useful for localized progress reporting.
    pub(crate) fn queued_len(&self) -> usize {
        self.queued.len()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::LoadCoordinator;
    use crate::app::workspace::id::SceneKey;
    use crate::scene_loading::{PendingSceneLoad, SceneLoadMode, SceneLoadRequest};
    use occluview_core::Scene;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Instant;

    #[test]
    fn retired_decoder_keeps_the_global_slot_until_its_late_result_is_drained() {
        let retired_key = SceneKey::from_raw_for_test(1, 1).expect("retired scene key");
        let replacement_key = SceneKey::from_raw_for_test(1, 2).expect("new scene epoch");
        let other_key = SceneKey::from_raw_for_test(2, 3).expect("other scene key");
        let (sender, receiver) = mpsc::channel();
        let mut coordinator = LoadCoordinator::default();
        coordinator.install_active(PendingSceneLoad {
            scene_key: retired_key,
            paths: vec![PathBuf::from("retired.stl")],
            source: "test",
            mode: SceneLoadMode::Replace,
            started_at: Instant::now(),
            receiver,
            superseded: false,
            content_revision_at_request: 0,
            dirty_at_request: false,
            requested_at: Instant::now(),
        });
        coordinator.enqueue(SceneLoadRequest {
            scene_key: other_key,
            paths: vec![PathBuf::from("other.stl")],
            source: "test",
            mode: SceneLoadMode::Append,
            content_revision_at_request: 0,
            dirty_at_request: false,
            requested_at: Instant::now(),
        });

        coordinator.reap_retired(&[replacement_key, other_key]);

        assert!(coordinator.active_for(retired_key));
        assert!(coordinator
            .active
            .as_ref()
            .is_some_and(|active| active.superseded));
        assert!(coordinator.take_next_for(other_key, &[]).is_none());

        assert!(sender.send(Ok(Scene::new())).is_ok());
        coordinator.reap_retired(&[replacement_key, other_key]);

        assert!(coordinator.active.is_none());
        assert_eq!(
            coordinator
                .take_next_for(other_key, &[])
                .map(|request| request.scene_key),
            Some(other_key),
            "the next scene may load after the obsolete decoder returns"
        );
    }

    #[test]
    fn blocked_append_does_not_stall_other_scenes_and_per_scene_order_holds() {
        let first = SceneKey::from_raw_for_test(1, 1).expect("first scene key");
        let second = SceneKey::from_raw_for_test(2, 2).expect("second scene key");
        let third = SceneKey::from_raw_for_test(3, 3).expect("third scene key");
        let requested_at = Instant::now();
        let mut coordinator = LoadCoordinator::default();
        for (scene_key, mode, path, offset) in [
            (first, SceneLoadMode::Append, "first-a.stl", 0),
            (first, SceneLoadMode::Append, "first-b.stl", 1),
            (second, SceneLoadMode::Append, "second.stl", 2),
            (third, SceneLoadMode::Append, "third.stl", 3),
        ] {
            coordinator.enqueue(SceneLoadRequest {
                scene_key,
                paths: vec![PathBuf::from(path)],
                source: "test",
                mode,
                content_revision_at_request: 0,
                dirty_at_request: false,
                requested_at: requested_at + std::time::Duration::from_millis(offset),
            });
        }

        let blocked = [first];
        assert!(coordinator.take_next_for(third, &blocked).is_none());
        assert_eq!(
            coordinator
                .take_next_for(second, &blocked)
                .map(|request| request.paths[0].clone()),
            Some(PathBuf::from("second.stl")),
            "the earliest eligible other-scene request can start"
        );
        assert_eq!(
            coordinator
                .take_next_for(third, &blocked)
                .map(|request| request.paths[0].clone()),
            Some(PathBuf::from("third.stl")),
            "eligible work retains its global order"
        );
        assert!(coordinator.take_next_for(first, &blocked).is_none());
        assert_eq!(
            coordinator
                .take_next_for(first, &[])
                .map(|request| request.paths[0].clone()),
            Some(PathBuf::from("first-a.stl")),
            "once unblocked, the first scene receives its oldest request first"
        );
        assert_eq!(
            coordinator
                .take_next_for(first, &[])
                .map(|request| request.paths[0].clone()),
            Some(PathBuf::from("first-b.stl")),
            "a later request for a scene never overtakes its earlier Append"
        );
    }

    #[test]
    fn delayed_replace_keeps_only_appends_accepted_after_its_request() {
        let first = SceneKey::from_raw_for_test(1, 1).expect("first scene key");
        let other = SceneKey::from_raw_for_test(2, 2).expect("other scene key");
        let requested_at = Instant::now();
        let mut coordinator = LoadCoordinator::default();
        for (scene_key, mode, path, at) in [
            (
                first,
                SceneLoadMode::Append,
                "before-guard.stl",
                requested_at
                    .checked_sub(std::time::Duration::from_millis(1))
                    .expect("earlier request time"),
            ),
            (
                other,
                SceneLoadMode::Append,
                "other.stl",
                requested_at + std::time::Duration::from_millis(1),
            ),
            (
                first,
                SceneLoadMode::Append,
                "after-guard.stl",
                requested_at + std::time::Duration::from_millis(2),
            ),
        ] {
            coordinator.enqueue(SceneLoadRequest {
                scene_key,
                paths: vec![PathBuf::from(path)],
                source: "test",
                mode,
                content_revision_at_request: 0,
                dirty_at_request: false,
                requested_at: at,
            });
        }

        coordinator.enqueue(SceneLoadRequest {
            scene_key: first,
            paths: vec![PathBuf::from("replace.stl")],
            source: "test",
            mode: SceneLoadMode::Replace,
            content_revision_at_request: 0,
            dirty_at_request: true,
            requested_at,
        });

        let queue = coordinator
            .queued
            .iter()
            .map(|request| (request.scene_key, request.paths[0].clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            queue,
            vec![
                (first, PathBuf::from("replace.stl")),
                (other, PathBuf::from("other.stl")),
                (first, PathBuf::from("after-guard.stl")),
            ],
            "confirmation keeps the original Replace position and later Append"
        );
    }

    #[test]
    fn lifetime_advance_drops_old_requests_and_rebinds_only_later_appends() {
        let old_key = SceneKey::from_raw_for_test(1, 1).expect("old scene key");
        let new_key = SceneKey::from_raw_for_test(1, 2).expect("new scene epoch");
        let other_key = SceneKey::from_raw_for_test(2, 3).expect("other scene key");
        let requested_at = Instant::now();
        let mut coordinator = LoadCoordinator::default();
        for (scene_key, path, at) in [
            (
                old_key,
                "superseded.stl",
                requested_at
                    .checked_sub(std::time::Duration::from_millis(1))
                    .expect("earlier request time"),
            ),
            (
                other_key,
                "other.stl",
                requested_at + std::time::Duration::from_millis(1),
            ),
            (
                old_key,
                "follow-up.stl",
                requested_at + std::time::Duration::from_millis(2),
            ),
        ] {
            coordinator.queued.push_back(SceneLoadRequest {
                scene_key,
                paths: vec![PathBuf::from(path)],
                source: "test",
                mode: SceneLoadMode::Append,
                content_revision_at_request: 0,
                dirty_at_request: false,
                requested_at: at,
            });
        }

        coordinator.advance_scene_lifetime_after_replace(old_key, new_key, requested_at);

        assert_eq!(
            coordinator
                .queued
                .iter()
                .map(|request| (request.scene_key, request.paths[0].clone()))
                .collect::<Vec<_>>(),
            vec![
                (other_key, PathBuf::from("other.stl")),
                (new_key, PathBuf::from("follow-up.stl")),
            ],
            "the Replace cannot revive older work or reorder another scene"
        );
    }
}
