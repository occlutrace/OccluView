use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::atomic::{AtomicU64, AtomicU8};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Reason a controlled operation stopped before establishing an exact answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryStop {
    /// Caller cancellation.
    Cancelled,
    /// Wall allowance exhausted.
    Deadline,
    /// Deterministic work allowance exhausted.
    WorkLimit,
    /// Allocation or representation limit reached.
    ResourceLimit,
    /// Finite source arithmetic exceeded the representable query frame.
    Numerical,
}

/// Shared ceilings for a serial, reproducible sequence of geometry operations.
#[derive(Clone, Copy, Debug)]
pub struct GeometryLimits {
    /// Total nearest calls.
    pub query_calls: u64,
    /// Total triangle distance tests.
    pub triangle_tests: u64,
    /// Triangle tests in one nearest call.
    pub single_query_tests: u64,
    /// Feature-neighbour and descriptor distance evaluations.
    pub point_pair_tests: u64,
    /// Validation, topology, bucket and cell traversal operations.
    pub operations: u64,
    /// Additional resident allocations, conservatively accounted.
    pub memory_bytes: usize,
    /// Maximum source triangle records to scan.
    pub input_triangles: usize,
    /// Maximum source vertex records to scan.
    pub input_vertices: usize,
}

impl Default for GeometryLimits {
    fn default() -> Self {
        Self {
            query_calls: 8_000_000,
            triangle_tests: 80_000_000,
            single_query_tests: 16_384,
            point_pair_tests: 32_000_000,
            operations: 80_000_000,
            memory_bytes: 256 * 1024 * 1024,
            input_triangles: 2_000_000,
            input_vertices: 6_000_000,
        }
    }
}

/// Snapshot of charged work and conservative resident allocation accounting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GeometryCounters {
    /// Nearest calls begun.
    pub query_calls: u64,
    /// Triangle distance tests begun.
    pub triangle_tests: u64,
    /// Feature-neighbour/descriptor point-pair tests begun.
    pub point_pair_tests: u64,
    /// Other bounded operations begun.
    pub operations: u64,
    /// Currently reserved allocation bytes.
    pub memory_bytes: u64,
    /// Largest simultaneous reservation.
    pub peak_memory_bytes: u64,
}

#[derive(Debug)]
struct ControlState {
    cancel: CancelFlag,
    started: Instant,
    wall: Duration,
    limits: GeometryLimits,
    queries: AtomicU64,
    triangles: AtomicU64,
    operations: AtomicU64,
    point_pairs: AtomicU64,
    memory: AtomicU64,
    peak: AtomicU64,
    clock_ticks: AtomicU64,
    stopped: AtomicU8,
}

/// Product-neutral lifetime, work counters and fallible allocation admission.
///
/// Clones share counters. Serial calls have deterministic work-limit prefixes;
/// cancellation and wall time may end at different prefixes under load.
#[derive(Clone, Debug)]
pub struct GeometryControl(
    Arc<ControlState>,
    Option<Arc<PointPairAllowance>>,
    Option<Arc<OperationAllowance>>,
);

#[derive(Debug)]
struct OperationAllowance {
    ceiling: u64,
    stopped: AtomicBool,
}

#[derive(Debug)]
struct PointPairAllowance {
    limit: u64,
    used: AtomicU64,
    stopped: AtomicBool,
}

impl GeometryControl {
    /// Start one shared lifetime with explicit wall and resource ceilings.
    pub fn new(cancel: CancelFlag, wall: Duration, limits: GeometryLimits) -> Self {
        Self::from_start(cancel, Instant::now(), wall, limits)
    }

    /// Adapt a caller's existing start instant so preprocessing counts toward its deadline.
    pub fn from_start(
        cancel: CancelFlag,
        started: Instant,
        wall: Duration,
        limits: GeometryLimits,
    ) -> Self {
        Self(
            Arc::new(ControlState {
                cancel,
                started,
                wall,
                limits,
                queries: AtomicU64::new(0),
                triangles: AtomicU64::new(0),
                operations: AtomicU64::new(0),
                point_pairs: AtomicU64::new(0),
                memory: AtomicU64::new(0),
                peak: AtomicU64::new(0),
                clock_ticks: AtomicU64::new(0),
                stopped: AtomicU8::new(0),
            }),
            None,
            None,
        )
    }

    /// Unlimited admission for existing non-registration consumers.
    pub fn unlimited() -> Self {
        Self::new(
            CancelFlag::new(),
            Duration::MAX,
            GeometryLimits {
                query_calls: u64::MAX,
                triangle_tests: u64::MAX,
                single_query_tests: u64::MAX,
                operations: u64::MAX,
                point_pair_tests: u64::MAX,
                memory_bytes: usize::MAX,
                input_triangles: usize::MAX,
                input_vertices: usize::MAX,
            },
        )
    }

    /// The effective ceilings.
    pub fn limits(&self) -> GeometryLimits {
        self.0.limits
    }

    /// Effective wall allowance, including the caller's existing lifetime.
    pub fn wall_limit(&self) -> Duration {
        self.0.wall
    }

    /// Check cancellation, deadline and any previously exhausted allowance.
    pub fn checkpoint(&self) -> Option<GeometryStop> {
        if self.0.cancel.is_cancelled() {
            return Some(GeometryStop::Cancelled);
        }
        if self.0.started.elapsed() >= self.0.wall {
            return Some(GeometryStop::Deadline);
        }
        self.recorded_stop().or_else(|| self.local_stop())
    }

    /// Current charged work; no counter exceeds its allowance.
    pub fn counters(&self) -> GeometryCounters {
        GeometryCounters {
            query_calls: self.0.queries.load(Ordering::Relaxed),
            triangle_tests: self.0.triangles.load(Ordering::Relaxed),
            operations: self.0.operations.load(Ordering::Relaxed),
            point_pair_tests: self.0.point_pairs.load(Ordering::Relaxed),
            memory_bytes: self.0.memory.load(Ordering::Relaxed),
            peak_memory_bytes: self.0.peak.load(Ordering::Relaxed),
        }
    }

    /// Charge before executing scalar/topology/cell work. Cancellation and
    /// deterministic limits are checked on every call; the wall clock is
    /// checked at most 128 charged operations apart, including inside queries.
    ///
    /// # Errors
    /// Returns the interruption reason without executing the rejected work.
    pub fn charge_operations(&self, count: u64) -> Result<(), GeometryStop> {
        if let Some(local) = &self.2 {
            if self
                .0
                .operations
                .load(Ordering::Relaxed)
                .checked_add(count)
                .is_none_or(|next| next > local.ceiling)
            {
                if let Some(stop) = self.checkpoint() {
                    return Err(stop);
                }
                local.stopped.store(true, Ordering::Relaxed);
                return Err(GeometryStop::WorkLimit);
            }
        }
        self.charge(&self.0.operations, self.0.limits.operations, count)
    }

    /// Reserve an ordinary-work prefix for a serial producer without poisoning
    /// its parent's remaining work. Clones and nested point-pair allowances
    /// share this stop. The ceiling counts global operations since construction;
    /// other work on the same control also consumes that prefix. Cancellation,
    /// wall time and hard global limits always retain priority.
    #[must_use]
    pub fn with_operation_allowance(&self, count: u64) -> Self {
        let ceiling = self
            .0
            .operations
            .load(Ordering::Relaxed)
            .saturating_add(count)
            .min(self.0.limits.operations)
            .min(self.2.as_ref().map_or(u64::MAX, |local| local.ceiling));
        Self(
            self.0.clone(),
            self.1.clone(),
            Some(Arc::new(OperationAllowance {
                ceiling,
                stopped: AtomicBool::new(false),
            })),
        )
    }

    /// Create a local point-pair allowance while sharing global counters,
    /// cancellation, deadline and resident memory. Exhausting this allowance
    /// stops only its clones; the parent can continue independent work.
    /// Nested allowances are capped by the parent's remaining local allowance.
    #[must_use]
    pub fn with_point_pair_allowance(&self, count: u64) -> Self {
        let count = self.1.as_ref().map_or(count, |local| {
            count.min(
                local
                    .limit
                    .saturating_sub(local.used.load(Ordering::Relaxed)),
            )
        });
        Self(
            self.0.clone(),
            Some(Arc::new(PointPairAllowance {
                limit: count,
                used: AtomicU64::new(0),
                stopped: AtomicBool::new(false),
            })),
            self.2.clone(),
        )
    }

    /// Charge distance/descriptor tests before evaluating point pairs.
    ///
    /// # Errors
    /// Returns interruption without beginning rejected work.
    pub fn charge_point_pairs(&self, count: u64) -> Result<(), GeometryStop> {
        if let Some(stop) = self.checkpoint() {
            return Err(stop);
        }
        if let Some(local) = &self.1 {
            if local
                .used
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                    old.checked_add(count).filter(|&next| next <= local.limit)
                })
                .is_err()
            {
                local.stopped.store(true, Ordering::Relaxed);
                return Err(GeometryStop::WorkLimit);
            }
        }
        self.charge(&self.0.point_pairs, self.0.limits.point_pair_tests, count)
    }

    pub(super) fn begin_query(&self) -> Result<(), GeometryStop> {
        self.charge(&self.0.queries, self.0.limits.query_calls, 1)
    }
    pub(super) fn triangle_test(&self, local: &mut u64) -> Result<(), GeometryStop> {
        if *local >= self.0.limits.single_query_tests {
            return Err(self.stop(GeometryStop::WorkLimit));
        }
        self.charge(&self.0.triangles, self.0.limits.triangle_tests, 1)?;
        *local += 1;
        Ok(())
    }
    fn charge(&self, counter: &AtomicU64, limit: u64, count: u64) -> Result<(), GeometryStop> {
        if self.0.cancel.is_cancelled() {
            return Err(GeometryStop::Cancelled);
        }
        if let Some(stop) = self.recorded_stop().or_else(|| self.local_stop()) {
            return Err(stop);
        }
        let ticks = self.0.clock_ticks.fetch_add(count, Ordering::Relaxed);
        if (ticks == 0 || count >= 128 || ticks % 128 >= 128 - count)
            && self.0.started.elapsed() >= self.0.wall
        {
            return Err(GeometryStop::Deadline);
        }
        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                old.checked_add(count).filter(|&next| next <= limit)
            })
            .map(|_| ())
            .map_err(|_| self.stop(GeometryStop::WorkLimit))
    }
    pub(super) fn stop(&self, reason: GeometryStop) -> GeometryStop {
        if reason == GeometryStop::WorkLimit && self.local_stop().is_some() {
            return reason;
        }
        let encoded = match reason {
            GeometryStop::Cancelled => 1,
            GeometryStop::Deadline => 2,
            GeometryStop::WorkLimit => 3,
            GeometryStop::ResourceLimit => 4,
            GeometryStop::Numerical => 5,
        };
        let _ = self
            .0
            .stopped
            .compare_exchange(0, encoded, Ordering::Relaxed, Ordering::Relaxed);
        self.recorded_stop().unwrap_or(reason)
    }
    fn recorded_stop(&self) -> Option<GeometryStop> {
        match self.0.stopped.load(Ordering::Relaxed) {
            1 => Some(GeometryStop::Cancelled),
            2 => Some(GeometryStop::Deadline),
            3 => Some(GeometryStop::WorkLimit),
            4 => Some(GeometryStop::ResourceLimit),
            5 => Some(GeometryStop::Numerical),
            _ => None,
        }
    }

    fn local_stop(&self) -> Option<GeometryStop> {
        (self
            .1
            .as_ref()
            .is_some_and(|local| local.stopped.load(Ordering::Relaxed))
            || self
                .2
                .as_ref()
                .is_some_and(|local| local.stopped.load(Ordering::Relaxed)))
        .then_some(GeometryStop::WorkLimit)
    }

    /// Reserve conservative resident bytes before allocating; release them on drop.
    ///
    /// # Errors
    /// Returns cancellation, deadline or resource exhaustion. No bytes are
    /// admitted beyond the shared cap.
    pub fn reserve(&self, bytes: usize) -> Result<GeometryMemory, GeometryStop> {
        if let Some(stop) = self.checkpoint() {
            return Err(stop);
        }
        let bytes = u64::try_from(bytes).map_err(|_| self.stop(GeometryStop::ResourceLimit))?;
        let limit = u64::try_from(self.0.limits.memory_bytes).unwrap_or(u64::MAX);
        let old = self
            .0
            .memory
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                old.checked_add(bytes).filter(|&next| next <= limit)
            })
            .map_err(|_| self.stop(GeometryStop::ResourceLimit))?;
        self.0.peak.fetch_max(old + bytes, Ordering::Relaxed);
        Ok(GeometryMemory {
            control: self.clone(),
            bytes,
        })
    }
}

/// Reservation that lives as long as its associated geometry allocations.
#[derive(Debug)]
pub struct GeometryMemory {
    control: GeometryControl,
    bytes: u64,
}
impl GeometryMemory {
    /// Conservative bytes admitted by this reservation.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub(super) fn shrink_to(&mut self, bytes: usize) {
        let next = u64::try_from(bytes).unwrap_or(u64::MAX).min(self.bytes);
        self.control
            .0
            .memory
            .fetch_sub(self.bytes - next, Ordering::Relaxed);
        self.bytes = next;
    }
}
impl Drop for GeometryMemory {
    fn drop(&mut self) {
        self.control
            .0
            .memory
            .fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

/// Complete build, explicit interrupted build, or no usable triangles.
#[derive(Debug)]
pub enum BuildOutcome<T> {
    /// All eligible triangles are indexed.
    Complete(T),
    /// A usable partial view, if available, with the reason it is incomplete.
    Partial {
        /// Optional completed representative view.
        value: Option<T>,
        /// Interruption reason.
        reason: GeometryStop,
    },
    /// No usable eligible surface.
    Empty,
}

/// Nearest query completeness is separate from a best-so-far hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QueryOutcome<T> {
    /// Exact nearest answer in radius, including proven absence.
    Complete(Option<T>),
    /// Search ended early; the hit is only an upper bound, never exact evidence.
    Interrupted {
        /// Best point examined so far.
        best: Option<T>,
        /// Interruption reason.
        reason: GeometryStop,
    },
}

/// Cooperative cancellation shared with a long-running geometry job.
///
/// Stages check the flag at bounded intervals and return completed work at the
/// next checkpoint.
#[derive(Clone, Debug, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    /// A fresh, uncancelled flag.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask every holder of this flag to stop at its next checkpoint.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::{CancelFlag, GeometryControl, GeometryLimits, GeometryStop};

    #[test]
    fn local_operation_exhaustion_preserves_remaining_global_work() {
        let parent = GeometryControl::new(
            CancelFlag::new(),
            std::time::Duration::from_secs(10),
            GeometryLimits {
                operations: 10,
                ..GeometryLimits::default()
            },
        );
        let local = parent.with_operation_allowance(2);
        let nested = local.with_point_pair_allowance(1);
        assert!(nested.charge_operations(2).is_ok());
        assert_eq!(nested.charge_operations(1), Err(GeometryStop::WorkLimit));
        assert_eq!(local.checkpoint(), Some(GeometryStop::WorkLimit));
        assert_eq!(nested.begin_query(), Err(GeometryStop::WorkLimit));
        assert_eq!(
            nested.stop(GeometryStop::WorkLimit),
            GeometryStop::WorkLimit
        );
        assert_eq!(parent.checkpoint(), None);
        assert!(parent.charge_operations(8).is_ok());
        assert_eq!(parent.counters().operations, 10);
        assert_eq!(parent.charge_operations(1), Err(GeometryStop::WorkLimit));
    }

    #[test]
    fn local_point_pair_exhaustion_preserves_global_work() {
        let parent = GeometryControl::new(
            CancelFlag::new(),
            std::time::Duration::from_secs(10),
            GeometryLimits::default(),
        );
        let local = parent.with_point_pair_allowance(2);
        assert!(local.charge_point_pairs(2).is_ok());
        assert_eq!(local.charge_point_pairs(1), Err(GeometryStop::WorkLimit));
        assert_eq!(local.checkpoint(), Some(GeometryStop::WorkLimit));
        assert_eq!(parent.checkpoint(), None);
        assert!(parent.charge_point_pairs(1).is_ok());
        assert_eq!(parent.counters().point_pair_tests, 3);
        parent.0.cancel.cancel();
        assert_eq!(local.checkpoint(), Some(GeometryStop::Cancelled));
    }

    #[test]
    fn cloned_flags_share_cancellation() {
        let flag = CancelFlag::new();
        let echo = flag.clone();
        assert!(!flag.is_cancelled());
        echo.cancel();
        assert!(flag.is_cancelled(), "cancellation reaches every clone");
    }
}
