//! Cooperative search lifetime and explicit interruption boundaries.
//!
//! Fixed work allowances define reproducible prefixes; wall deadlines and
//! caller cancellation can end at different prefixes under different loads.
//!
//! Ordinary-operation calibration (2026-10-03): release build on an AMD Ryzen
//! 5 3600 (6 cores / 12 threads, 15.6 GiB RAM), `CARGO_BUILD_JOBS=2`. A synthetic
//! independently remeshed arch processed 255,999,965 ordinary units in 4.983 s
//! after conservative pruning and bounded accounting batches. With streamed
//! quadrature, cached feature neighborhoods and exact miss bounds it processed
//! 511,999,985 units in 8.866 s (about 58 million units/s). Rounded cooperative
//! ceilings are 576M / 1728M / 96M for 10 / 30 / 2 s; Local stays conservative.
//! These are work ceilings, not timing promises. Shared load can reduce
//! throughput; wall deadlines and independent query, triangle, point-pair and
//! allocation limits still stop slower/pathological work. Interrupted evidence
//! and counters remain visible.

use crate::{CancelFlag, Completion, SearchProfile, SearchSettings};
use occluview_geometry::surface::{GeometryControl, GeometryLimits, GeometryStop};
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
            // Bucket-cache probes are ordinary operations as well as the
            // independent distance-test allowance. Keep both bounded without
            // spending the distance allowance on reused arithmetic.
            SearchProfile::Standard => (10, 8_000_000, 80_000_000, 576_000_000),
            SearchProfile::Extended => (30, 20_000_000, 240_000_000, 1_728_000_000),
            SearchProfile::Local => (2, 1_000_000, 10_000_000, 96_000_000),
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

/// Pre-admit at most 128 elements of a known-length serial pass.
///
/// Call for every index in order, starting at zero, with the same total and
/// control. The successful pass charges exactly its length. An interrupted
/// chunk may charge its unvisited tail; no element precedes its admission.
/// Cancellation and time are checked at most 128 elements apart.
///
/// # Errors
/// Returns `Numerical` for an index outside the pass, or the control stop.
#[inline]
pub(crate) fn charge_linear_element(
    control: &GeometryControl,
    index: usize,
    total: usize,
) -> Result<(), GeometryStop> {
    if index >= total {
        return Err(GeometryStop::Numerical);
    }
    if index.is_multiple_of(128) {
        control.charge_operations(
            u64::try_from((total - index).min(128)).map_err(|_| GeometryStop::ResourceLimit)?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn fixed_linear_admission_preserves_work_and_bounds_counter_updates() {
        let control = GeometryControl::unlimited();
        let total = 10_000usize;
        let mut previous = 0;
        let mut updates = 0;
        let mut sum = 0usize;
        for index in 0..total {
            charge_linear_element(&control, index, total).unwrap();
            let charged = control.counters().operations;
            if charged != previous {
                updates += 1;
                assert!(charged - previous <= 128);
            }
            assert!(charged >= u64::try_from(index + 1).unwrap());
            sum += index;
            previous = charged;
        }
        assert_eq!(sum, total * (total - 1) / 2);
        assert_eq!(control.counters().operations, 10_000);
        assert!(updates <= total.div_ceil(128), "counter updates={updates}");
    }

    #[test]
    fn fixed_linear_admission_keeps_typed_caps_and_parent_work() {
        let limited = GeometryControl::new(
            CancelFlag::new(),
            Duration::MAX,
            GeometryLimits {
                operations: 127,
                ..GeometryLimits::default()
            },
        );
        assert_eq!(
            charge_linear_element(&limited, 0, usize::MAX),
            Err(GeometryStop::WorkLimit)
        );
        assert_eq!(limited.counters().operations, 0);
        let parent = GeometryControl::unlimited();
        let local = parent.with_operation_allowance(4);
        assert_eq!(
            charge_linear_element(&local, 0, 256),
            Err(GeometryStop::WorkLimit)
        );
        assert_eq!(parent.counters().operations, 0);
        assert_eq!(parent.checkpoint(), None);
        parent.charge_operations(5).unwrap();
        for (index, total) in [(0, 0), (128, 128), (usize::MAX, usize::MAX)] {
            assert_eq!(
                charge_linear_element(&parent, index, total),
                Err(GeometryStop::Numerical)
            );
        }
        assert_eq!(parent.counters().operations, 5);
    }

    #[test]
    fn fixed_linear_admission_observes_cancellation_within_one_chunk() {
        let flag = CancelFlag::new();
        let control = GeometryControl::new(flag.clone(), Duration::MAX, GeometryLimits::default());
        charge_linear_element(&control, 0, 256).unwrap();
        flag.cancel();
        let stopped = (1..256)
            .find(|&index| charge_linear_element(&control, index, 256).is_err())
            .unwrap();
        assert!(stopped <= 128);
        assert!(control.counters().operations <= 128);
        assert_eq!(control.checkpoint(), Some(GeometryStop::Cancelled));
    }
}
