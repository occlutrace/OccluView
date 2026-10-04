//! The lifetime of one search: the caller's cancellation and the wall clock.
//!
//! The search itself fixes how much work it does, so nothing here counts
//! work. A deadline or a cancellation can still end a search early; the
//! result then carries what was finished and says what was not.

use crate::{CancelFlag, Completion, SearchSettings};
use occluview_geometry::surface::{GeometryControl, GeometryLimits};
use std::time::{Duration, Instant};

/// Ceiling of the conservative resident estimate of one scan's exact
/// surface: about nine hundred thousand triangles. A larger scan is read on
/// an even cloud of its triangles instead, which costs a few micrometres of
/// reading accuracy and none of the evidence.
const SURFACE_MEMORY_BYTES: usize = 1 << 30;

/// Caller cancellation and wall allowance for one search invocation.
#[derive(Clone, Debug)]
pub struct SearchControl {
    cancel: CancelFlag,
    started: Instant,
    wall_limit: Duration,
}

impl SearchControl {
    /// Start a lifetime using the supplied shared flag and wall allowance.
    pub fn new(cancel: CancelFlag, wall_limit: Duration) -> Self {
        Self {
            cancel,
            started: Instant::now(),
            wall_limit,
        }
    }

    /// Request cancellation; poses already found remain reviewable.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Cancellation first, then the deadline: the lesser of this lifetime's
    /// allowance and `wall_limit`.
    pub fn checkpoint(&self, wall_limit: Duration) -> Option<Completion> {
        if self.cancel.is_cancelled() {
            Some(Completion::Cancelled)
        } else if self.started.elapsed() >= self.wall_limit.min(wall_limit) {
            Some(Completion::Deadline)
        } else {
            None
        }
    }

    /// Elapsed lifetime.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// The same start instant and cancellation flag for the exact surface of
    /// one scan: only the clock, the caller and the memory ceiling can end
    /// work on it. Each scan gets its own, so one scan that does not fit
    /// leaves the other readable.
    pub(crate) fn surface_control(&self, settings: &SearchSettings) -> GeometryControl {
        GeometryControl::from_start(
            self.cancel.clone(),
            self.started,
            self.wall_limit.min(settings.wall_limit),
            GeometryLimits {
                query_calls: u64::MAX,
                triangle_tests: u64::MAX,
                operations: u64::MAX,
                point_pair_tests: u64::MAX,
                memory_bytes: SURFACE_MEMORY_BYTES,
                input_triangles: usize::MAX,
                input_vertices: usize::MAX,
                ..GeometryLimits::default()
            },
        )
    }
}

impl Default for SearchControl {
    fn default() -> Self {
        Self::new(CancelFlag::new(), Duration::from_secs(10))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_reported_before_the_deadline() {
        let control = SearchControl::new(CancelFlag::new(), Duration::ZERO);
        assert_eq!(
            control.checkpoint(Duration::MAX),
            Some(Completion::Deadline)
        );
        control.cancel();
        assert_eq!(
            control.checkpoint(Duration::MAX),
            Some(Completion::Cancelled)
        );
    }

    #[test]
    fn the_lesser_allowance_decides() {
        let control = SearchControl::new(CancelFlag::new(), Duration::MAX);
        assert_eq!(control.checkpoint(Duration::MAX), None);
        assert_eq!(
            control.checkpoint(Duration::ZERO),
            Some(Completion::Deadline)
        );
    }
}
