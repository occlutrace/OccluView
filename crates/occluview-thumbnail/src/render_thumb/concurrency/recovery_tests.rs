//! Recovery tests for poisoned locks, renderer panics, and stalled work.

use super::*;

mod poison_recovery_tests {
    use super::*;

    fn wait_for_queued_jobs(gate: &ThumbnailJobGate, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if lock_recover(&gate.inner.state).waiters.len() == expected {
                return;
            }
            assert!(Instant::now() < deadline, "thumbnail job did not queue");
            std::thread::yield_now();
        }
    }

    /// Panic while holding `mutex`'s guard, poisoning it, then hand the
    /// (recovered) inner value back.
    fn poison<T>(mutex: &Mutex<T>) {
        let _ = panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = mutex.lock().expect("fresh mutex is not poisoned");
            panic!("intentionally poison the lock for the recovery test");
        }));
        assert!(mutex.is_poisoned(), "the lock should now be poisoned");
    }

    #[test]
    fn lock_recover_returns_the_guard_even_when_poisoned() {
        let mutex = Mutex::new(41u32);
        {
            let mut guard = lock_recover(&mutex);
            *guard += 1;
        }
        poison(&mutex);
        // Recovery still yields a usable guard with the last-written value.
        let mut guard = lock_recover(&mutex);
        assert_eq!(*guard, 42);
        *guard = 7;
        assert_eq!(*guard, 7);
    }

    #[test]
    fn shell_renderer_parallelism_stays_inside_the_process_budget() {
        // Bounds, not exact values: this is named for a budget, and a tuning
        // change from one renderer to two -- comfortably inside it -- should
        // not have to edit a test about the budget.
        let renderers = default_thumbnail_renderer_pool_size();
        assert!(
            (1..=2).contains(&renderers),
            "one wgpu device per surrogate process is the point; {renderers} is \
             a different design"
        );
        let lanes = default_thumbnail_job_capacity();
        assert!(
            (1..=6).contains(&lanes),
            "past six lanes the folder gains no throughput and every request \
             waits longer for the same answer; {lanes} is outside that"
        );
        assert!(
            lanes <= std::thread::available_parallelism().map_or(6, std::num::NonZeroUsize::get),
            "a lane needs a thread to run on; {lanes} is more than this machine has"
        );
    }

    #[test]
    fn poisoned_job_gate_still_acquires_and_releases() {
        let gate = ThumbnailJobGate::new(1);
        poison(&gate.inner.state);

        let permit = gate.acquire_timeout(Duration::from_millis(50));
        assert!(
            permit.is_some(),
            "a poisoned gate must still hand out permits"
        );
        // Dropping the permit must release it despite the poison, so the single
        // slot is reusable rather than leaked forever.
        drop(permit);
        let again = gate.acquire_timeout(Duration::from_millis(50));
        assert!(
            again.is_some(),
            "release must not skip on poison, or the gate leaks its only permit"
        );
    }

    #[test]
    fn queued_thumbnail_jobs_acquire_in_arrival_order() {
        let gate = Arc::new(ThumbnailJobGate::new(1));
        let held = gate
            .acquire_timeout(Duration::from_millis(10))
            .expect("initial permit");
        let (order_tx, order_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();

        let first_gate = gate.clone();
        let first_tx = order_tx.clone();
        let first = std::thread::spawn(move || {
            let _permit = first_gate
                .acquire_timeout(Duration::from_secs(1))
                .expect("first queued permit");
            first_tx.send(1_u8).expect("record first acquisition");
            release_first_rx.recv().expect("release first waiter");
        });
        wait_for_queued_jobs(&gate, 1);

        let second_gate = gate.clone();
        let second = std::thread::spawn(move || {
            let _permit = second_gate
                .acquire_timeout(Duration::from_secs(1))
                .expect("second queued permit");
            order_tx.send(2_u8).expect("record second acquisition");
        });
        wait_for_queued_jobs(&gate, 2);

        drop(held);
        assert_eq!(order_rx.recv_timeout(Duration::from_secs(1)), Ok(1));
        assert!(order_rx.recv_timeout(Duration::from_millis(20)).is_err());
        release_first_tx.send(()).expect("release first waiter");
        assert_eq!(order_rx.recv_timeout(Duration::from_secs(1)), Ok(2));
        first.join().expect("first waiter thread");
        second.join().expect("second waiter thread");
    }

    #[test]
    fn poisoned_renderer_pool_still_serves_and_returns_renderers() {
        let _guard = crate::acquire_render_test_guard();
        let pool = ThumbnailRendererPool::new(2);
        poison(&pool.state);

        // A poisoned pool must still create/serve a renderer instead of failing
        // every checkout for the rest of the process.
        let renderer = pool
            .checkout_renderer()
            .expect("a poisoned renderer pool must still serve a renderer");
        {
            let lease = ThumbnailRendererLease::new(&pool, renderer);
            drop(lease);
        }
        // The returned renderer landed back in the idle set (return_renderer did
        // not skip on poison), so it is reusable.
        let idle = lock_recover(&pool.state).idle.len();
        assert_eq!(
            idle, 1,
            "the returned renderer must be reusable, not leaked, after poison"
        );
    }

    #[test]
    fn discarded_renderer_releases_pool_capacity() {
        let pool = ThumbnailRendererPool::new(2);
        {
            let mut state = lock_recover(&pool.state);
            state.total_renderers = 1;
        }

        pool.discard_renderer();

        assert_eq!(lock_recover(&pool.state).total_renderers, 0);
    }
}

mod renderer_pool_recovery_tests {
    use super::{ThumbnailError, ThumbnailRendererPool, RENDERER_WAIT_CEILING};
    use std::time::{Duration, Instant};

    fn refuses() -> Result<occluview_render::Offscreen, ThumbnailError> {
        Err(ThumbnailError::Render(
            occluview_render::RenderError::NoAdapter,
        ))
    }

    fn panics() -> Result<occluview_render::Offscreen, ThumbnailError> {
        panic!("a driver reset during device creation");
    }

    /// A create that fails gives its slot back -- the case that was already
    /// handled, kept here so the panicking case beside it has a control.
    #[test]
    fn a_refused_create_leaves_the_pool_able_to_try_again() {
        let pool = ThumbnailRendererPool::with_create(1, refuses);
        for _ in 0..3 {
            assert!(
                pool.checkout_renderer_within(Duration::from_millis(50))
                    .is_err(),
                "the create refuses, so the checkout must too"
            );
        }
    }

    /// A create that panics must give its slot back as well.
    ///
    /// It claims the slot before the device exists. Unwinding past the
    /// bookkeeping left the pool believing its one renderer was in use, and
    /// every later request then waited for a renderer that was never coming --
    /// each holding a decode lane, so twelve of them were the whole folder.
    #[test]
    #[allow(clippy::expect_used)]
    fn a_panicking_create_leaves_the_pool_able_to_try_again() {
        let pool = ThumbnailRendererPool::with_create(1, panics);
        for attempt in 0..3 {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pool.checkout_renderer_within(Duration::from_millis(50))
            }));
            assert!(
                outcome.is_err(),
                "attempt {attempt} must surface the panic, not a timeout: the \
                 slot was not given back"
            );
        }
    }

    /// The wait is bounded, so a renderer that never returns cannot park a
    /// request forever.
    #[test]
    fn waiting_for_a_renderer_gives_up_instead_of_parking_the_lane() {
        let pool = ThumbnailRendererPool::with_create(1, refuses);
        // Claim the only slot and never give it back.
        {
            let mut state = super::lock_recover(&pool.state);
            state.total_renderers = 1;
        }
        let started = Instant::now();
        let outcome = pool.checkout_renderer_within(Duration::from_millis(120));
        assert!(outcome.is_err(), "the wait must expire, not block");
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(100) && waited < Duration::from_secs(5),
            "expired at the budget, not early and not never: {waited:?}"
        );
        assert!(
            RENDERER_WAIT_CEILING >= Duration::from_secs(10),
            "the production ceiling is a backstop, not a request budget"
        );
    }

    /// A render that panics must not park its device back in the pool.
    ///
    /// Whatever the renderer was doing, it did not finish; its device may be
    /// lost or mid-command. Unwinding alone runs the lease's `Drop`, which
    /// hands exactly that device to the next file in the folder.
    #[test]
    #[allow(clippy::expect_used)]
    fn a_panicking_render_retires_its_device_instead_of_reusing_it() {
        let _guard = crate::acquire_render_test_guard();
        let pool = ThumbnailRendererPool::new(1);
        let Ok(renderer) = pool.checkout_renderer_within(Duration::from_secs(20)) else {
            // No adapter here; there is no device to retire.
            return;
        };
        drop(super::ThumbnailRendererLease::new(&pool, renderer));

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pool.with_renderer(|_| -> Result<(), ThumbnailError> {
                panic!("a device fault mid-render")
            })
        }));
        assert!(panicked.is_err(), "the panic must reach the caller");

        let state = super::lock_recover(&pool.state);
        assert!(
            state.idle.is_empty() && state.total_renderers == 0,
            "the suspect device must be retired, not parked: idle={} total={}",
            state.idle.len(),
            state.total_renderers
        );
    }
}

mod one_budget_tests {
    use super::super::{
        run_thumbnail_job_with_deadline, ThumbnailJobGate, ThumbnailJobOutcome,
        ThumbnailJobProgress,
    };
    use std::time::{Duration, Instant};

    /// Waiting for a slot and then rendering must spend one budget, not two.
    ///
    /// Each used to take the caller's full timeout of its own, so a request
    /// could take twice what was asked -- and under Explorer's Apartment
    /// hosting every extraction serialises through one thread, so those
    /// seconds are the whole folder's.
    #[test]
    fn a_wait_for_a_slot_comes_out_of_the_request_budget() {
        let budget = Duration::from_millis(600);
        let gate = ThumbnailJobGate::shared();
        // Fill every lane, and let them go part-way through the budget.
        let mut held = Vec::new();
        while let Some(permit) = gate.acquire_timeout(Duration::from_millis(50)) {
            held.push(permit);
            if held.len() > 64 {
                break;
            }
        }
        let release_at = Instant::now() + Duration::from_millis(400);
        std::thread::spawn(move || {
            while Instant::now() < release_at {
                std::thread::sleep(Duration::from_millis(10));
            }
            drop(held);
        });

        let started = Instant::now();
        let outcome: ThumbnailJobOutcome<()> =
            run_thumbnail_job_with_deadline(budget, |progress| {
                std::thread::sleep(Duration::from_millis(500));
                let _ = progress.send(ThumbnailJobProgress::Finished(()));
            });
        let elapsed = started.elapsed();
        assert!(
            matches!(outcome, ThumbnailJobOutcome::SetupTimedOut)
                || matches!(outcome, ThumbnailJobOutcome::RenderTimedOut),
            "the work outlasts the budget, so the caller must give up"
        );
        assert!(
            elapsed < budget + Duration::from_millis(250),
            "one budget of {budget:?} was spent twice: {elapsed:?}"
        );
    }
}
