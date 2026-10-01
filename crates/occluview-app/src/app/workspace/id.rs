//! Stable identities used by the workspace and its asynchronous operations.

use std::num::NonZeroU64;

macro_rules! define_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(crate) struct $name(NonZeroU64);

        impl $name {
            /// First identity in a workspace's lifetime.
            pub(crate) const INITIAL: Self = Self(NonZeroU64::MIN);

            /// Numeric representation for logs and compact serialization.
            #[must_use]
            pub(crate) const fn get(self) -> u64 {
                self.0.get()
            }

            #[cfg(test)]
            pub(crate) const fn from_raw_for_test(value: u64) -> Option<Self> {
                match NonZeroU64::new(value) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }
        }
    };
}

define_id!(
    SceneId,
    "Identity of a workspace scene; never derived from its name or position."
);
define_id!(
    PaneId,
    "Identity of one workspace view; never derived from its screen position."
);
define_id!(
    SceneEpoch,
    "Monotonic lifetime token that distinguishes reuse after undo or replacement."
);

/// Identity of one live lifetime of a scene.
///
/// An undo may restore a reserved `SceneId`, but must allocate a fresh epoch so
/// late worker and loader replies from its former lifetime remain stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SceneKey {
    pub(crate) id: SceneId,
    pub(crate) epoch: SceneEpoch,
}

impl SceneKey {
    pub(crate) const INITIAL: Self = Self::new(SceneId::INITIAL, SceneEpoch::INITIAL);

    #[must_use]
    pub(crate) const fn new(id: SceneId, epoch: SceneEpoch) -> Self {
        Self { id, epoch }
    }

    #[cfg(test)]
    pub(crate) const fn from_raw_for_test(id: u64, epoch: u64) -> Option<Self> {
        let Some(id) = SceneId::from_raw_for_test(id) else {
            return None;
        };
        let Some(epoch) = SceneEpoch::from_raw_for_test(epoch) else {
            return None;
        };
        Some(Self::new(id, epoch))
    }
}

/// Checked, non-reusing identity source owned by the workspace coordinator.
///
/// Counters fail closed at exhaustion; they never wrap and issue a stale id.
#[derive(Clone, Debug)]
pub(crate) struct IdAllocator {
    scene: Option<NonZeroU64>,
    pane: Option<NonZeroU64>,
    epoch: Option<NonZeroU64>,
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl IdAllocator {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            scene: NonZeroU64::new(1),
            pane: NonZeroU64::new(1),
            epoch: NonZeroU64::new(1),
        }
    }

    /// Start a workspace with its required first scene and view already
    /// allocated; subsequent IDs continue at two and cannot collide with them.
    #[must_use]
    pub(crate) const fn new_with_initial_scene() -> (Self, SceneKey, PaneId) {
        (
            Self {
                scene: NonZeroU64::new(2),
                pane: NonZeroU64::new(2),
                epoch: NonZeroU64::new(2),
            },
            SceneKey::INITIAL,
            PaneId::INITIAL,
        )
    }

    pub(crate) fn allocate_scene_id(&mut self) -> Result<SceneId, IdExhausted> {
        Self::take_next(&mut self.scene).map(SceneId)
    }

    pub(crate) fn allocate_pane_id(&mut self) -> Result<PaneId, IdExhausted> {
        Self::take_next(&mut self.pane).map(PaneId)
    }

    pub(crate) fn allocate_scene_epoch(&mut self) -> Result<SceneEpoch, IdExhausted> {
        Self::take_next(&mut self.epoch).map(SceneEpoch)
    }

    pub(crate) fn allocate_scene_key(&mut self, id: SceneId) -> Result<SceneKey, IdExhausted> {
        Ok(SceneKey::new(id, self.allocate_scene_epoch()?))
    }

    /// Allocate a fresh scene identity and its first lifetime token.
    pub(crate) fn allocate_scene(&mut self) -> Result<SceneKey, IdExhausted> {
        let id = self.allocate_scene_id()?;
        self.allocate_scene_key(id)
    }

    fn take_next(next: &mut Option<NonZeroU64>) -> Result<NonZeroU64, IdExhausted> {
        let value = next.take().ok_or(IdExhausted)?;
        *next = value.get().checked_add(1).and_then(NonZeroU64::new);
        Ok(value)
    }
}

/// The workspace exhausted a monotonic identity counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IdExhausted;

impl std::fmt::Display for IdExhausted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("workspace identity space exhausted")
    }
}

impl std::error::Error for IdExhausted {}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use std::num::NonZeroU64;

    use super::{IdAllocator, SceneId};

    #[test]
    fn each_identity_domain_is_monotonic_and_scene_epoch_is_fresh() {
        let mut ids = IdAllocator::new();
        let first_scene = ids.allocate_scene().unwrap();
        let first_pane = ids.allocate_pane_id().unwrap();
        let second_scene_id = ids.allocate_scene_id().unwrap();
        let second_scene = ids.allocate_scene_key(second_scene_id).unwrap();
        let restored = ids.allocate_scene_key(first_scene.id).unwrap();

        assert_eq!(first_scene.id.get(), 1);
        assert_eq!(first_pane.get(), 1);
        assert_eq!(second_scene.id.get(), 2);
        assert_eq!(first_scene.epoch.get(), 1);
        assert_eq!(second_scene.epoch.get(), 2);
        assert_eq!(restored.id, first_scene.id);
        assert_ne!(restored.epoch, first_scene.epoch);
    }

    #[test]
    fn test_constructor_rejects_zero() {
        assert_eq!(SceneId::from_raw_for_test(0), None);
    }

    #[test]
    fn exhausted_counter_does_not_wrap_or_reissue_an_identity() {
        let mut ids = IdAllocator {
            scene: NonZeroU64::new(u64::MAX),
            pane: NonZeroU64::new(1),
            epoch: NonZeroU64::new(1),
        };
        assert_eq!(ids.allocate_scene_id().unwrap().get(), u64::MAX);
        assert!(ids.allocate_scene_id().is_err());
    }
}
