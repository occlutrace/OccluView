use super::*;

/// P1 lost-update regression: contention on the frame path must not drop
/// the newest CPU state. A drained-but-unapplied update is restored, so a
/// later poll applies authoritative latest state. Non-blocking throughout.
#[test]
fn drained_update_is_not_lost_on_shadow_contention() {
    let worker = test_worker();
    worker.state.record_touched(vec![0, 1, 2], Vec::new());
    let write_guard = worker
        .state
        .shadow
        .write()
        .expect("test holds the shadow write lock");
    let drained = worker.take_update().expect("pending update must drain");
    assert!(
        worker.state.shadow.try_read().is_err(),
        "held write lock must make the flush unavailable"
    );
    // Fixed `flush_sculpt_update` path: restore instead of dropping.
    worker.restore_update(drained);
    drop(write_guard);
    let retry = worker.take_update();
    assert!(
        retry.is_some(),
        "lost update: a drained-but-unapplied sculpt update vanished; \
         contention must leave a pending retry or authoritative full-sync"
    );
    let retry = retry.expect("checked above");
    assert!(
        retry.full_sync || !retry.touched.is_empty(),
        "retry must carry authoritative latest state"
    );
}

/// Quiescence must count undrained worker output, not just the command
/// queue: Done/undo gate on it, and a false quiet lets the session
/// invalidate out from under unflushed deltas.
#[test]
fn quiescence_counts_undrained_deltas_not_just_the_queue() {
    let worker = test_worker();
    // No commands queued; a restored (drained-but-unapplied) update is
    // still live work the next poll owes a flush.
    worker.restore_update(SculptUpdate {
        touched: vec![0],
        full_sync: false,
    });
    assert!(
        !worker.is_quiescent(),
        "a restored update must read as pending work"
    );
    let _ = worker.take_update();
    assert!(worker.is_quiescent(), "a drained worker must read quiet");
}

/// Same for the full-sync flag: an authoritative resync owed to the GPU
/// is pending work even with an empty queue and no touched ids.
#[test]
fn quiescence_counts_a_pending_full_sync() {
    let worker = test_worker();
    worker.request_full_sync();
    assert!(
        !worker.is_quiescent(),
        "an owed full sync must read as pending work"
    );
    let update = worker.take_update().expect("the flag must drain");
    assert!(update.full_sync);
    assert!(worker.is_quiescent());
}

/// Same for a densify rebuild: no Finish, no drain, yet the new topology
/// (and its touches) is still owed to the UI.
#[test]
fn quiescence_counts_a_pending_layer_rebuild() {
    let worker = worker_for(&coarse_ridge_mesh());
    let dab = BrushStroke {
        center: [0.0, 0.0, 0.0],
        radius_mm: 3.5,
        strength: 1.0,
        view_dir: [0.0, 0.0, -1.0],
    };
    assert!(worker.try_apply(dab, BrushMode::Smooth));
    thread::sleep(Duration::from_millis(200));
    assert!(
        !worker.is_quiescent(),
        "an undrained densify rebuild must read as pending work"
    );
    worker
        .try_take_rebuild()
        .expect("the densifying dab must have produced a layer rebuild");
}

/// Coalescing keeps only the newest state: overflow escalates to one
/// authoritative full sync instead of an unbounded delta backlog.
#[test]
fn touched_overflow_escalates_to_full_sync() {
    let worker = test_worker();
    worker
        .state
        .record_touched(vec![0; MAX_PENDING_TOUCHES + 1], Vec::new());
    let update = worker.take_update().expect("overflow must stay visible");
    assert!(
        update.full_sync,
        "pending-touch overflow must become an authoritative full sync"
    );
}

/// A contended backlog defers the drain instead of blocking the frame: the
/// pending ids and the full-sync flag stay queued for the next poll.
#[test]
fn take_update_defers_when_backlog_locked() {
    let worker = test_worker();
    worker.state.record_touched(vec![0, 1, 2], Vec::new());
    worker.state.request_full_sync();
    let held = worker
        .state
        .pending_touched
        .lock()
        .expect("test holds the backlog lock");
    assert!(
        worker.take_update().is_none(),
        "a contended drain must defer, not block or half-drain"
    );
    drop(held);
    let retry = worker.take_update().expect("deferred update must redrain");
    assert!(
        retry.full_sync,
        "the full-sync flag must survive the deferred drain"
    );
}

/// A contended rebuild slot defers the drain the same way.
#[test]
fn take_rebuild_defers_when_slot_locked() {
    let worker = test_worker();
    let mesh = Mesh::new(
        Some("rebuild-defer".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("rebuild mesh");
    worker.state.record_rebuild(
        1,
        SculptRebuild {
            topology: PreparedSceneTopology::from_mesh(&mesh),
            mesh,
        },
    );
    let held = worker
        .state
        .rebuild
        .lock()
        .expect("test holds the rebuild lock");
    assert!(
        worker.try_take_rebuild().is_err(),
        "a contended rebuild drain must defer"
    );
    drop(held);
    assert!(
        worker.try_take_rebuild().expect("rebuild lock").is_some(),
        "the deferred rebuild must redrain"
    );
}

/// A densifying rebuild supersedes queued sparse ids, which index the
/// pre-rebuild array. Rebuilds themselves stay ordered so every topology
/// transition remains available to the UI-side completion chain.
#[test]
fn rebuild_supersedes_queued_sparse_updates() {
    let worker = test_worker();
    worker.state.record_touched(vec![0, 1, 2], Vec::new());
    let mesh = Mesh::new(
        Some("rebuild-supersede".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("rebuild mesh");
    worker.state.record_rebuild(
        1,
        SculptRebuild {
            topology: PreparedSceneTopology::from_mesh(&mesh),
            mesh,
        },
    );
    assert!(
        worker.take_update().is_none(),
        "stale sparse ids must not survive a topology rebuild"
    );
    assert!(
        worker.try_take_rebuild().expect("rebuild lock").is_some(),
        "the authoritative rebuild must remain queued"
    );
}

#[test]
fn rebuild_queue_preserves_every_topology_transition_in_order() {
    let worker = test_worker();
    let first = Mesh::new(
        Some("rebuild-first".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("first rebuild mesh");
    let second = Mesh::new(
        Some("rebuild-second".to_string()),
        vec![
            Vertex::at(Vec3::new(-2.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(2.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(2.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-2.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("second rebuild mesh");
    let first_id = first.topology_id();
    let second_id = second.topology_id();
    worker.state.record_rebuild(
        1,
        SculptRebuild {
            topology: PreparedSceneTopology::from_mesh(&first),
            mesh: first,
        },
    );
    worker.state.record_rebuild(
        2,
        SculptRebuild {
            topology: PreparedSceneTopology::from_mesh(&second),
            mesh: second,
        },
    );
    assert_eq!(
        worker
            .try_take_rebuild()
            .expect("first rebuild")
            .expect("first rebuild present")
            .mesh
            .topology_id(),
        first_id
    );
    assert_eq!(
        worker
            .try_take_rebuild()
            .expect("second rebuild")
            .expect("second rebuild present")
            .mesh
            .topology_id(),
        second_id
    );
    assert!(
        worker.try_take_rebuild().expect("rebuild lock").is_none(),
        "the FIFO must be fully drained"
    );
}

/// Restoring a drained update is a lossless round-trip: the next drain
/// returns the same sparse ids when no rebuild intervened.
#[test]
fn restored_update_redrains_identical_sparse_ids() {
    let worker = test_worker();
    worker.state.record_touched(vec![3, 1, 2, 1], Vec::new());
    let drained = worker.take_update().expect("pending update must drain");
    worker.restore_update(drained);
    let retry = worker.take_update().expect("restored update must redrain");
    assert!(!retry.full_sync, "sparse restore must not escalate");
    let mut ids = retry.touched;
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids, vec![1, 2, 3]);
}

/// A restore that would overflow the backlog escalates to a full sync at the
/// same site as the record path, not just in `record_touched`.
#[test]
fn restore_overflow_escalates_to_full_sync() {
    let worker = test_worker();
    worker.restore_update(SculptUpdate {
        touched: vec![0; MAX_PENDING_TOUCHES + 1],
        full_sync: false,
    });
    let update = worker.take_update().expect("overflow must stay visible");
    assert!(
        update.full_sync,
        "restore overflow must become an authoritative full sync"
    );
}

/// A drained sparse update restored after a rebuild queued must not come
/// back as stale ids: it escalates to a full sync of the latest shadow.
#[test]
fn restore_after_rebuild_escalates_to_full_sync() {
    let worker = test_worker();
    worker.state.record_touched(vec![0, 1, 2], Vec::new());
    let drained = worker.take_update().expect("pending update must drain");
    let mesh = Mesh::new(
        Some("restore-rebuild".to_string()),
        vec![
            Vertex::at(Vec3::new(-1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, -1.0, 0.0)),
            Vertex::at(Vec3::new(1.0, 1.0, 0.0)),
            Vertex::at(Vec3::new(-1.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .expect("rebuild mesh");
    worker.state.record_rebuild(
        1,
        SculptRebuild {
            topology: PreparedSceneTopology::from_mesh(&mesh),
            mesh,
        },
    );
    worker.restore_update(drained);
    assert!(
        worker.take_update().is_none(),
        "a stale sparse restore must stay behind the queued topology rebuild"
    );
    assert!(
        worker.try_take_rebuild().expect("rebuild lock").is_some(),
        "the authoritative rebuild must be drained before its full sync"
    );
    let retry = worker.take_update().expect("restore must stay visible");
    assert!(
        retry.full_sync,
        "post-rebuild restore must escalate, never resurrect stale ids"
    );
}
