//! Tests for the contact worker's queue, generations, and refusals.
//!
//! The worker is where a wrong answer becomes a wrong map, so these tests are
//! about the rules rather than about the compute: a job that has been superseded
//! must not repaint the screen, a layer with no surface must be refused with a
//! reason, and a dropped worker must not take the process with it.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use occluview_contact::ContactSettings;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// A flat square, two triangles, all vertices on `z`, in world coordinates.
fn plate(z: f32, size: f32) -> (Arc<Vec<f32>>, Arc<Vec<u32>>) {
    let positions = Arc::new(vec![0.0, 0.0, z, size, 0.0, z, size, size, z, 0.0, size, z]);
    let indices = Arc::new(vec![0, 1, 2, 0, 2, 3]);
    (positions, indices)
}

fn keys(offset: u64, flatten: bool) -> ContactJobKeys {
    ContactJobKeys {
        subject: (1, 2),
        antagonist: (3, offset),
        flatten_patches: flatten,
    }
}

fn job(worker: &ContactWorker, offset: u64, z_gap: f32) -> ContactJob {
    let (subject_positions, subject_indices) = plate(0.0, 1.0);
    let (antagonist_positions, antagonist_indices) = plate(z_gap, 1.0);
    ContactJob {
        generation: worker.generation(),
        request_id: 0,
        keys: keys(offset, false),
        subject_positions,
        subject_indices,
        antagonist_positions,
        antagonist_indices,
        settings: ContactSettings {
            search_radius_mm: occluview_contact::SEARCH_RADIUS_MM,
            flatten_patches: false,
        },
    }
}

fn completion_for(worker: &ContactWorker, offset: u64, z_gap: f32) -> ContactCompletion {
    let (subject_positions, subject_indices) = plate(0.0, 1.0);
    let (antagonist_positions, antagonist_indices) = plate(z_gap, 1.0);
    let job = ContactJob {
        generation: worker.generation(),
        request_id: 0,
        keys: keys(offset, false),
        subject_positions,
        subject_indices,
        antagonist_positions,
        antagonist_indices,
        settings: ContactSettings::default(),
    };
    let outcome = execute(&job, &CancelFlag::new());
    ContactCompletion {
        generation: job.generation,
        request_id: job.request_id,
        keys: job.keys,
        outcome,
    }
}

/// Wait for one completion, without spinning a fixed sleep in the test body.
fn wait_for_completion(worker: &ContactWorker, deadline: Duration) -> Option<ContactCompletion> {
    let started = Instant::now();
    while started.elapsed() < deadline {
        let drained = worker.drain();
        if let Some(completion) = drained.into_iter().next() {
            return Some(completion);
        }
        thread::sleep(Duration::from_millis(5));
    }
    None
}

/// The worker's core contract: a measurement comes back with a value per
/// vertex on both sides, and the counters the panel reports.
#[test]
fn a_measurement_comes_back_for_both_sides() {
    let worker = ContactWorker::spawn();
    // The antagonist plate sits a twentieth of a millimetre above the
    // subject's plane: the subject is therefore inside the antagonist, which is
    // the case a contact reading exists to report.
    let _ = worker.submit(job(&worker, 0, 0.05));
    let completion = wait_for_completion(&worker, Duration::from_secs(20)).expect("a completion");
    assert_eq!(completion.keys, keys(0, false));
    match completion.outcome {
        ContactOutcome::Measured {
            subject_signed_mm,
            antagonist_signed_mm,
            stats,
            diagnostics,
        } => {
            assert_eq!(subject_signed_mm.len(), 4);
            assert_eq!(antagonist_signed_mm.len(), 4);
            assert_eq!(diagnostics.subject_verts, 4);
            assert_eq!(diagnostics.antagonist_verts, 4);
            // Every vertex has a measurement, because two parallel plates
            // leave nothing out of reach — and the two sides read with opposite
            // signs, which is what the field means: a value is measured along
            // the opposing surface's outward normal. The
            // antagonist's plane sits a twentieth of a millimetre above the
            // subject's, so the subject is inside it and reads a load, while the
            // antagonist reads the same overlap as clearance.
            for value in &subject_signed_mm {
                assert!(value.is_finite(), "no vertex may be out of reach here");
                assert!(
                    (value + 0.05).abs() < 0.005,
                    "a twentieth of a millimetre of interference reads as -0.05: {value}"
                );
            }
            for value in &antagonist_signed_mm {
                assert!(value.is_finite(), "no vertex may be out of reach here");
                assert!(
                    (value - 0.05).abs() < 0.005,
                    "the opposing side reads the same overlap as a clearance: {value}"
                );
            }
            // The plates are a millimetre square and every corner of both
            // triangles is loaded, so the contact area has a value that can be
            // worked out by hand: 1 mm squared.
            assert!(
                (stats.contact_area_mm2 - 1.0).abs() < 0.01,
                "a fully loaded 1 mm square reads 1 mm2 of contact: {}",
                stats.contact_area_mm2
            );
            assert_eq!(stats.contacts, 1, "one patch of contact, not two");
        }
        ContactOutcome::Failed(reason) => panic!("a plate pair must measure, got {reason:?}"),
    }
}

/// A layer with no triangle surface is refused with a reason rather than
/// measured into an empty map.
#[test]
fn a_layer_with_no_surface_is_refused() {
    let worker = ContactWorker::spawn();
    let (subject_positions, subject_indices) = plate(0.0, 1.0);
    let (antagonist_positions, _) = plate(-0.1, 1.0);
    let _ = worker.submit(ContactJob {
        generation: worker.generation(),
        request_id: 0,
        keys: keys(1, false),
        subject_positions,
        subject_indices,
        antagonist_positions,
        antagonist_indices: Arc::new(Vec::new()),
        settings: ContactSettings::default(),
    });
    let completion = wait_for_completion(&worker, Duration::from_secs(20)).expect("a completion");
    assert!(matches!(
        completion.outcome,
        ContactOutcome::Failed(ContactFailure::NoSurface)
    ));
}

#[test]
fn a_superseded_request_cannot_answer_the_latest_one() {
    let worker = ContactWorker::spawn();
    assert_eq!(worker.latest_request(), None, "nothing submitted yet");
    let _ = worker.submit(job(&worker, 0, -0.1));
    let first = worker.latest_request().expect("the first submission");
    let _ = worker.submit(job(&worker, 7, -0.1));
    let second = worker.latest_request().expect("the second submission");
    assert_ne!(
        first, second,
        "submitting again retires the previous request"
    );
}

#[test]
fn a_completion_for_a_superseded_request_is_not_drained() {
    let worker = ContactWorker::spawn();
    let first_id = worker.submit(job(&worker, 1, -0.1)).expect("queued");
    let mut stale = completion_for(&worker, 1, 0.05);
    stale.request_id = first_id;
    worker.publish_for_tests(stale);
    let second_id = worker.submit(job(&worker, 2, -0.1)).expect("queued");
    assert_ne!(first_id, second_id);
    let drained = worker.drain();
    assert!(
        drained
            .iter()
            .all(|completion| completion.keys != keys(1, false)),
        "the superseded completion must not be delivered as the current answer"
    );
    let second = wait_for_completion(&worker, Duration::from_secs(20)).expect("a completion");
    assert_eq!(second.keys, keys(2, false));
    assert_eq!(second.request_id, second_id);
}

#[test]
fn a_worker_without_a_thread_refuses_to_accept_work() {
    let worker = ContactWorker::spawn_failing();
    assert!(worker.has_failed());
    assert!(
        worker.submit(job(&worker, 0, -0.1)).is_none(),
        "a job with no executor must not be queued"
    );
    assert!(!worker.is_busy());
}

/// The busy hold is what the operator sees as the spinner, so it must survive
/// a panic in the job body: a hand-written decrement is skipped by the unwind.
#[test]
fn a_busy_hold_is_released_when_the_job_body_panics() {
    let counter = Arc::new(AtomicU64::new(0));
    let panicked = std::panic::catch_unwind({
        let counter = Arc::clone(&counter);
        move || {
            let _busy = Busy::new(&counter);
            assert_eq!(counter.load(Ordering::SeqCst), 1);
            panic!("the job body panicked");
        }
    });

    assert!(panicked.is_err());
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "the counter must come back down even when the job unwinds"
    );
}

/// A worker whose thread body panicked must look failed and idle, not busy
/// forever: the bar's spinner is fed by those two signals, and a job that died
/// mid-run would otherwise leave the operator waiting for a measurement no
/// thread is computing.
#[test]
fn a_panicking_worker_latches_a_failure_and_stops_looking_busy() {
    let worker = ContactWorker::spawn_panicking();

    let mut waited = Duration::ZERO;
    while !worker.has_failed() && waited < Duration::from_secs(10) {
        thread::sleep(Duration::from_millis(5));
        waited += Duration::from_millis(5);
    }

    assert!(
        worker.has_failed(),
        "a panicked worker must latch a failure"
    );
    assert!(
        !worker.is_busy(),
        "a dead worker must not hold the busy counter"
    );
    assert!(
        worker.submit(job(&worker, 0, -0.1)).is_none(),
        "no new work may be queued for a dead worker"
    );
}

/// A bump discards what is in flight and clears the queue, so the map on screen
/// describes the pair the operator is looking at now.
#[test]
fn a_generation_bump_discards_in_flight_work() {
    let worker = ContactWorker::spawn();
    let _ = worker.submit(job(&worker, 0, -0.1));
    worker.bump_generation();
    // The cancelled job must not publish: the completion list is cleared and the
    // worker's own flag stops the compute at its next checkpoint.
    let mut waited = Duration::ZERO;
    while waited < Duration::from_millis(300) {
        assert!(
            worker.drain().is_empty(),
            "a superseded job must not republish a measurement"
        );
        thread::sleep(Duration::from_millis(10));
        waited += Duration::from_millis(10);
    }
    // The worker still works afterwards.
    let _ = worker.submit(job(&worker, 2, -0.1));
    let completion = wait_for_completion(&worker, Duration::from_secs(20)).expect("a completion");
    assert_eq!(completion.keys, keys(2, false));
}
/// Submitting again replaces the queued job: computing an older reading on the
/// way would only delay the answer the operator is waiting for.
#[test]
fn a_new_submission_replaces_the_queued_one() {
    let worker = ContactWorker::spawn();
    let _ = worker.submit(job(&worker, 0, -0.1));
    let _ = worker.submit(job(&worker, 5, -0.1));
    let completion = wait_for_completion(&worker, Duration::from_secs(20)).expect("a completion");
    assert_eq!(completion.keys, keys(5, false));
}
/// Dropping the worker stops the thread and does not panic, which matters
/// because a release build aborts on panic.
#[test]
fn dropping_the_worker_stops_it_cleanly() {
    let worker = ContactWorker::spawn();
    let _ = worker.submit(job(&worker, 0, -0.1));
    drop(worker);
}

/// The queue reports itself as busy while work is outstanding, which is what
/// disables the panel's patch toggle.
#[test]
fn the_worker_reports_itself_busy() {
    let worker = ContactWorker::spawn();
    let _ = worker.submit(job(&worker, 0, -0.1));
    assert!(worker.is_busy());
    let _ = wait_for_completion(&worker, Duration::from_secs(20));
}
