use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
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
    stopped: std::sync::Mutex<Option<GeometryStop>>,
}

/// Product-neutral lifetime, work counters and fallible allocation admission.
///
/// Clones share counters. Serial calls have deterministic work-limit prefixes;
/// cancellation and wall time may end at different prefixes under load.
#[derive(Clone, Debug)]
pub struct GeometryControl(Arc<ControlState>);

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
        Self(Arc::new(ControlState {
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
            stopped: std::sync::Mutex::new(None),
        }))
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

    /// Check cancellation, deadline and any previously exhausted allowance.
    pub fn checkpoint(&self) -> Option<GeometryStop> {
        if self.0.cancel.is_cancelled() {
            return Some(GeometryStop::Cancelled);
        }
        if self.0.started.elapsed() >= self.0.wall {
            return Some(GeometryStop::Deadline);
        }
        *self
            .0
            .stopped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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

    /// Charge before executing scalar/topology/cell work, checking every call.
    ///
    /// # Errors
    /// Returns the interruption reason without executing the rejected work.
    pub fn charge_operations(&self, count: u64) -> Result<(), GeometryStop> {
        self.charge(&self.0.operations, self.0.limits.operations, count)
    }

    /// Charge distance/descriptor tests before evaluating point pairs.
    ///
    /// # Errors
    /// Returns interruption without beginning rejected work.
    pub fn charge_point_pairs(&self, count: u64) -> Result<(), GeometryStop> {
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
        if let Some(stop) = self.checkpoint() {
            return Err(stop);
        }
        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                old.checked_add(count).filter(|&next| next <= limit)
            })
            .map(|_| ())
            .map_err(|_| self.stop(GeometryStop::WorkLimit))
    }
    pub(super) fn stop(&self, reason: GeometryStop) -> GeometryStop {
        let mut state = self
            .0
            .stopped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *state.get_or_insert(reason)
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
    use super::CancelFlag;

    #[test]
    fn cloned_flags_share_cancellation() {
        let flag = CancelFlag::new();
        let echo = flag.clone();
        assert!(!flag.is_cancelled());
        echo.cancel();
        assert!(flag.is_cancelled(), "cancellation reaches every clone");
    }
}
