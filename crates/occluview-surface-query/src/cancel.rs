use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
