//! Cooperative search lifetime and explicit interruption boundaries.
//!
//! Fixed work allowances define reproducible prefixes; wall deadlines and
//! caller cancellation can end at different prefixes under different loads.

use crate::{CancelFlag, Completion, SearchProfile, SearchSettings};
use occluview_geometry::surface::{GeometryControl, GeometryLimits};
use std::time::{Duration, Instant};

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

    /// Request cancellation; existing finite checkpoints remain reviewable.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Check cancellation before deadline, with an additional settings ceiling.
    pub fn checkpoint(&self, wall_limit: Duration) -> Option<Completion> {
        if self.cancel.is_cancelled() {
            Some(Completion::Cancelled)
        } else if self.started.elapsed() >= self.wall_limit.min(wall_limit) {
            Some(Completion::Deadline)
        } else {
            None
        }
    }

    /// Elapsed lifetime, independent of numeric ranking.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Adapt the same start instant and cancellation flag for surface work.
    /// Profile defaults are ceilings; caller budgets can lower them. Clones of
    /// the returned control share counters and allocation admission.
    pub fn geometry_control(&self, settings: &SearchSettings) -> GeometryControl {
        let (wall, queries, triangles, operations) = match settings.profile {
            SearchProfile::Standard => (10, 8_000_000, 80_000_000, 80_000_000),
            SearchProfile::Extended => (30, 20_000_000, 240_000_000, 240_000_000),
            SearchProfile::Local => (2, 1_000_000, 10_000_000, 10_000_000),
        };
        GeometryControl::from_start(
            self.cancel.clone(),
            self.started,
            self.wall_limit
                .min(settings.wall_limit)
                .min(Duration::from_secs(wall)),
            GeometryLimits {
                query_calls: settings.work_budget.query_calls.min(queries),
                triangle_tests: settings.work_budget.triangle_tests.min(triangles),
                operations,
                point_pair_tests: settings.work_budget.point_pair_tests.min(
                    if settings.profile == SearchProfile::Extended {
                        128_000_000
                    } else {
                        32_000_000
                    },
                ),
                memory_bytes: settings.work_budget.memory_bytes.min(256 * 1024 * 1024),
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
