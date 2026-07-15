//! Timeout wrapper for shell thumbnail requests.
//!
//! The heavyweight concurrency gate already lives in `render_thumb.rs` via the
//! shared offscreen renderer pool. Adding a second bounded worker queue here
//! makes bursty Explorer folders spend timeout budget before a render even
//! starts. We instead run each request on its own lightweight helper thread and
//! let the renderer pool remain the single throughput bottleneck.

use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc;
use std::time::Duration;

/// Run `work` on a background thread and wait up to `timeout`.
///
/// # Errors
/// This function encodes failures as `None`: timeout, worker panic, thread
/// spawn failure, or channel disconnect all produce no result. The caller
/// decides the fallback behavior.
#[must_use]
pub fn run_with_timeout<T, F>(timeout: Duration, work: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = mpsc::sync_channel(1);
    let spawn = std::thread::Builder::new()
        .name("occluview-thumbnail-timeout".to_string())
        .spawn(move || {
            let result = panic::catch_unwind(AssertUnwindSafe(work));
            if let Ok(value) = result {
                let _ = tx.send(value);
            }
        });
    if spawn.is_err() {
        return None;
    }

    rx.recv_timeout(timeout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;
    use std::time::Instant;

    #[test]
    fn returns_worker_value_before_deadline() {
        let result = run_with_timeout(Duration::from_millis(500), || 42);
        assert_eq!(result, Some(42));
    }

    #[test]
    fn returns_none_after_deadline() {
        let started = Instant::now();
        let result = run_with_timeout(Duration::from_millis(10), || {
            thread::sleep(Duration::from_millis(200));
            42
        });
        assert_eq!(result, None);
        assert!(started.elapsed() < Duration::from_millis(120));
        thread::sleep(Duration::from_millis(250));
    }

    #[test]
    fn returns_none_when_worker_panics() {
        let result = run_with_timeout(Duration::from_millis(100), || -> u8 {
            panic::resume_unwind(Box::new("worker failed"));
        });
        assert_eq!(result, None);
    }

    #[test]
    fn parallel_thumbnail_bursts_start_without_shared_timeout_queue() {
        let request_count = 12usize;
        let entered = Arc::new(Barrier::new(request_count + 1));
        let release = Arc::new(Barrier::new(request_count + 1));
        let worker_ids = Arc::new(Mutex::new(Vec::with_capacity(request_count)));
        let mut callers = Vec::with_capacity(request_count);

        for _ in 0..request_count {
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            let worker_ids = Arc::clone(&worker_ids);
            callers.push(thread::spawn(move || {
                let worker = run_with_timeout(Duration::from_secs(2), move || {
                    worker_ids
                        .lock()
                        .expect("worker id list")
                        .push(thread::current().id());
                    entered.wait();
                    release.wait();
                    7_u8
                });
                assert_eq!(worker, Some(7_u8));
            }));
        }

        entered.wait();
        release.wait();

        for caller in callers {
            caller.join().expect("caller thread");
        }

        assert_eq!(
            worker_ids.lock().expect("worker id list").len(),
            request_count,
            "each burst request should begin execution instead of waiting behind a second timeout queue"
        );
    }

    #[test]
    fn running_jobs_do_not_block_new_quick_job_from_starting() {
        let request_count = 12usize;
        let entered = Arc::new(Barrier::new(request_count + 1));
        let release = Arc::new(Barrier::new(request_count + 1));
        let mut blocking_calls = Vec::with_capacity(request_count);

        for _ in 0..request_count {
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            blocking_calls.push(thread::spawn(move || {
                run_with_timeout(Duration::from_millis(500), move || {
                    entered.wait();
                    release.wait();
                    7_u8
                })
            }));
        }

        entered.wait();
        let queued = thread::spawn(|| run_with_timeout(Duration::from_millis(100), || 42_u8));
        assert_eq!(
            queued.join().expect("quick caller thread"),
            Some(42_u8),
            "a small thumbnail request should still start while other longer jobs are in flight"
        );
        release.wait();

        for call in blocking_calls {
            assert_eq!(call.join().expect("blocking caller thread"), Some(7_u8));
        }
    }
}
