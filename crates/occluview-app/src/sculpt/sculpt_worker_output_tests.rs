use super::*;
use glam::Vec3;

fn append_triangle_delta(worker: &SculptWorker) -> SculptTopologyDelta {
    let appended = Vertex::at(Vec3::new(0.0, 0.0, 1.0));
    worker
        .shadow()
        .write()
        .expect("display shadow")
        .push(appended);
    let delta = SculptTopologyDelta {
        base_vertex_count: 4,
        appended_vertices: vec![appended],
        updated_vertices: Vec::new(),
        base_index_count: 6,
        live_index_count: 9,
        face_updates: vec![occluview_render::SculptFaceUpdate {
            triangle: 2,
            indices: [1, 4, 2],
        }],
        dirty_triangles: vec![2],
    };
    worker.queue_topology_delta_for_tests(delta.clone());
    delta
}

fn queue_face_delta(worker: &SculptWorker, triangle: u32, indices: [u32; 3]) {
    let pick = worker.state.pick.read().expect("pick state");
    let delta = SculptTopologyDelta {
        base_vertex_count: pick.shadow.read().expect("display shadow").len(),
        appended_vertices: Vec::new(),
        updated_vertices: Vec::new(),
        base_index_count: pick.indices.len(),
        live_index_count: pick.indices.len(),
        face_updates: vec![occluview_render::SculptFaceUpdate { triangle, indices }],
        dirty_triangles: vec![triangle as usize],
    };
    drop(pick);
    worker.queue_topology_delta_for_tests(delta);
}

/// Contention on the frame path must not drop the newest CPU state. A
/// drained-but-unapplied update is restored, so a later poll applies it.
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
    assert!(worker.state.shadow.try_read().is_err());
    worker.restore_update(drained);
    drop(write_guard);
    let retry = worker.take_update().expect("the drained update must retry");
    assert!(retry.full_sync || !retry.touched.is_empty());
}

#[test]
fn quiescence_counts_undrained_sparse_and_topology_outputs() {
    let worker = test_worker();
    worker.restore_update(SculptUpdate {
        touched: vec![0],
        full_sync: false,
    });
    assert!(!worker.is_quiescent());
    let _ = worker.take_update();
    assert!(worker.is_quiescent());

    let delta = append_triangle_delta(&worker);
    assert_eq!(delta.live_index_count, 9);
    assert!(!worker.is_quiescent());
    assert!(worker
        .try_take_topology_delta()
        .expect("delta lock")
        .is_some());
    assert!(worker.is_quiescent());
}

#[test]
fn quiescence_counts_a_pending_full_sync() {
    let worker = test_worker();
    worker.request_full_sync();
    assert!(!worker.is_quiescent());
    let update = worker.take_update().expect("the flag must drain");
    assert!(update.full_sync);
    assert!(worker.is_quiescent());
}

#[test]
fn touched_overflow_escalates_to_full_sync() {
    let worker = test_worker();
    worker
        .state
        .record_touched(vec![0; MAX_PENDING_TOUCHES + 1], Vec::new());
    let update = worker.take_update().expect("overflow must stay visible");
    assert!(update.full_sync);
}

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
    assert!(worker.take_update().is_none());
    drop(held);
    assert!(worker.take_update().expect("deferred update").full_sync);
}

#[test]
fn an_ordered_drain_defers_when_topology_slot_is_locked() {
    let worker = test_worker();
    let delta = append_triangle_delta(&worker);
    let held = worker
        .state
        .topology_deltas
        .lock()
        .expect("test holds the topology slot");
    assert!(worker.take_ordered_outputs().is_err());
    drop(held);
    let (deltas, completions, update) = worker
        .take_ordered_outputs()
        .expect("deferred outputs must redrain");
    assert_eq!(deltas, VecDeque::from([delta]));
    assert!(completions.is_empty());
    assert!(update.is_none());
}

#[test]
fn topology_deltas_keep_append_order_with_the_matching_completion() {
    let worker = test_worker();
    let first = append_triangle_delta(&worker);
    queue_face_delta(&worker, 2, [1, 2, 4]);
    let mesh = Arc::new(coarse_ridge_mesh().expect("ridge mesh"));
    assert!(worker.state.push_completion(SculptCompletion {
        before: Arc::clone(&mesh),
        mesh,
    }));

    let (deltas, completions, update) = worker
        .take_ordered_outputs()
        .expect("the ordered output boundary must be available");
    assert_eq!(deltas.len(), 2);
    assert_eq!(deltas[0], first);
    assert_eq!(deltas[0].base_vertex_count, 4);
    assert_eq!(deltas[1].base_vertex_count, 5);
    assert_eq!(deltas[1].base_index_count, 9);
    assert_eq!(completions.len(), 1);
    assert!(update.is_none());
    assert!(worker
        .take_ordered_outputs()
        .expect("the boundary remains usable")
        .0
        .is_empty());
}

#[test]
fn appended_topology_keeps_earlier_sparse_vertex_ids_valid() {
    let worker = test_worker();
    worker.state.record_touched(vec![1, 3], Vec::new());
    let drained = worker.take_update().expect("sparse update must drain");
    let delta = append_triangle_delta(&worker);
    worker.restore_update(drained);
    let (deltas, completions, update) = worker
        .take_ordered_outputs()
        .expect("stable vertex ids can share one ordered drain");
    assert_eq!(deltas, VecDeque::from([delta]));
    assert!(completions.is_empty());
    let update = update.expect("sparse update remains valid after append");
    assert!(!update.full_sync);
    assert_eq!(update.touched, vec![1, 3]);
}

#[test]
fn restored_sparse_updates_keep_their_vertex_ids() {
    let worker = test_worker();
    worker.state.record_touched(vec![3, 1, 2, 1], Vec::new());
    let drained = worker.take_update().expect("pending update must drain");
    worker.restore_update(drained);
    let retry = worker.take_update().expect("restored update must redrain");
    assert!(!retry.full_sync);
    let mut ids = retry.touched;
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[test]
fn restore_overflow_escalates_to_full_sync() {
    let worker = test_worker();
    worker.restore_update(SculptUpdate {
        touched: vec![0; MAX_PENDING_TOUCHES + 1],
        full_sync: false,
    });
    let update = worker.take_update().expect("overflow must stay visible");
    assert!(update.full_sync);
}
