//! Cooperative search lifetime and explicit interruption boundaries.
//!
//! Fixed work allowances define reproducible prefixes; wall deadlines and
//! caller cancellation can end at different prefixes under different loads.

use crate::{CancelFlag, Completion};
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
}

impl Default for SearchControl {
    fn default() -> Self {
        Self::new(CancelFlag::new(), Duration::from_secs(10))
    }
}
