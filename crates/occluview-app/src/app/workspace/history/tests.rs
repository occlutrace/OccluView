use super::{
    AutoCreatedDestination, HistoryDirection, HistorySnapshot, HistoryStepKind, NavigationError,
    TransferRecord, TransferredLayer, WorkspaceHistory,
};
use crate::app::workspace::id::{PaneId, SceneKey};
use crate::app::workspace::layout::WorkspaceLayout;
use occluview_core::{Mesh, Scene, SceneMesh};
use std::path::PathBuf;

fn scene_key(id: u64, epoch: u64) -> SceneKey {
    match SceneKey::from_raw_for_test(id, epoch) {
        Some(key) => key,
        None => panic!("test scene identity must be nonzero"),
    }
}

fn pane_id(id: u64) -> PaneId {
    match PaneId::from_raw_for_test(id) {
        Some(pane) => pane,
        None => panic!("test pane identity must be nonzero"),
    }
}

fn layer_edit(history: &mut WorkspaceHistory, scope: SceneKey, snapshot: &str) -> u64 {
    let layer_id = SceneMesh::new(Mesh::empty()).id();
    let snapshot = snapshot.to_owned();
    let bytes = snapshot.len();
    let Some(id) = history.push_pending_edit(
        Some(scope),
        HistoryStepKind::LayerEdit { layer_id },
        snapshot,
        bytes,
    ) else {
        panic!("test edit must fit in its history budget");
    };
    assert!(history.commit_pending_edit(id));
    id
}

fn workspace_layout() -> WorkspaceLayout {
    match WorkspaceLayout::side_by_side(pane_id(1), pane_id(2), 0.5) {
        Ok(layout) => layout,
        Err(error) => panic!("test panes must be distinct: {error:?}"),
    }
}

fn transfer_record(source: SceneKey, destination: SceneKey, auto_created: bool) -> TransferRecord {
    let pane = pane_id(2);
    let layout = workspace_layout();
    TransferRecord {
        source,
        destination,
        layers: vec![TransferredLayer {
            layer_id: SceneMesh::new(Mesh::empty()).id(),
            source_index: 0,
            destination_index: 0,
            source_path: Some(PathBuf::from("scan.stl")),
            source_hidden_stack_position: None,
            destination_hidden_stack_position: None,
        }],
        auto_created_destination: auto_created.then_some(AutoCreatedDestination {
            pane_id: pane,
            name: "Scan 2".to_owned(),
            previous_layout: WorkspaceLayout::single(pane_id(1)),
            previous_saved_split: None,
            layout_with_destination: layout,
            preserve_on_undo: false,
            retained_after_undo: false,
        }),
        destination_was_uninitialized: true,
        source_focused_layer_before: None,
        destination_focused_layer_before: None,
        active_scene_before: source,
        active_scene_after: destination,
    }
}

fn commit_transfer(
    history: &mut WorkspaceHistory,
    source: SceneKey,
    destination: SceneKey,
    auto_created: bool,
) -> u64 {
    let id = match history.prepare_transfer(transfer_record(source, destination, auto_created), 1) {
        Ok(id) => id,
        Err(error) => panic!("test transfer must be accepted: {error:?}"),
    };
    assert_eq!(history.commit_transfer(id), Ok(()));
    id
}

#[test]
fn edit_checkpoint_limits_navigation_and_cancel_restores_prior_redo_only_in_its_scene() {
    let source = scene_key(1, 1);
    let neighbor = scene_key(2, 2);
    let mut history = WorkspaceHistory::new(16, 1024);
    let old_edit = layer_edit(&mut history, source, "before old edit");
    let _neighbor_old = layer_edit(&mut history, neighbor, "neighbor old");

    let Some(old_snapshot) = history.navigate_edit(
        Some(source),
        HistoryDirection::Undo,
        old_edit,
        HistorySnapshot {
            value: "after old edit".to_owned(),
            bytes: "after old edit".len(),
        },
    ) else {
        panic!("the committed edit should undo");
    };
    assert_eq!(old_snapshot, "before old edit");

    let Some(checkpoint) = history.begin_edit_checkpoint(Some(source), Scene::new(), 1) else {
        panic!("checkpoint ID should be available");
    };
    assert!(history
        .top_step(Some(source), HistoryDirection::Undo)
        .is_none());
    assert!(history
        .top_step(Some(source), HistoryDirection::Redo)
        .is_none());

    let session_edit = layer_edit(&mut history, source, "session baseline");
    let neighbor_edit = layer_edit(&mut history, neighbor, "neighbor later");
    assert_eq!(
        history
            .top_step(Some(source), HistoryDirection::Undo)
            .map(|step| step.command_id),
        Some(session_edit)
    );

    let baseline = history.cancel_edit_checkpoint(checkpoint);
    assert!(baseline.is_some());
    assert_eq!(
        history
            .top_step(Some(source), HistoryDirection::Redo)
            .map(|step| step.command_id),
        Some(old_edit)
    );
    assert_eq!(
        history
            .top_step(Some(neighbor), HistoryDirection::Undo)
            .map(|step| step.command_id),
        Some(neighbor_edit)
    );
    assert!(history.payload::<String>(session_edit).is_none());
}

#[test]
fn done_keeps_session_edits_undoable_and_releases_the_checkpoint_boundary() {
    let scene = scene_key(3, 1);
    let mut history = WorkspaceHistory::new(16, 1024);
    let previous = layer_edit(&mut history, scene, "previous");
    let Some(checkpoint) = history.begin_edit_checkpoint(Some(scene), Scene::new(), 1) else {
        panic!("checkpoint ID should be available");
    };
    let during_session = layer_edit(&mut history, scene, "during session");
    assert!(history.finish_edit_checkpoint(checkpoint));

    assert_eq!(
        history
            .top_step(Some(scene), HistoryDirection::Undo)
            .map(|step| step.command_id),
        Some(during_session)
    );
    let Some(_) = history.navigate_edit(
        Some(scene),
        HistoryDirection::Undo,
        during_session,
        HistorySnapshot {
            value: "after session edit".to_owned(),
            bytes: "after session edit".len(),
        },
    ) else {
        panic!("Done should leave the session edit undoable");
    };
    assert_eq!(
        history
            .top_step(Some(scene), HistoryDirection::Undo)
            .map(|step| step.command_id),
        Some(previous)
    );
}

#[test]
fn transfer_requires_both_timeline_heads_and_new_work_invalidates_both_redo_branches() {
    let source = scene_key(4, 1);
    let destination = scene_key(5, 1);
    let mut history = WorkspaceHistory::new(16, 1024);
    let _source_old = layer_edit(&mut history, source, "source before");
    let transfer = commit_transfer(&mut history, source, destination, false);
    assert_eq!(
        history
            .transfer_payload(transfer)
            .map(|record| record.destination_was_uninitialized),
        Some(true)
    );

    let destination_edit = layer_edit(&mut history, destination, "destination before");
    assert_eq!(
        history.validate_navigation(transfer, HistoryDirection::Undo),
        Err(NavigationError::CommandNotAtHead(transfer))
    );
    let Some(_) = history.navigate_edit(
        Some(destination),
        HistoryDirection::Undo,
        destination_edit,
        HistorySnapshot {
            value: "destination after".to_owned(),
            bytes: "destination after".len(),
        },
    ) else {
        panic!("the later destination edit should undo first");
    };
    assert_eq!(
        history.validate_navigation(transfer, HistoryDirection::Undo),
        Ok(())
    );
    assert_eq!(
        history.commit_navigation(transfer, HistoryDirection::Undo),
        Ok(())
    );
    for participant in [source, destination] {
        assert_eq!(
            history
                .top_step(Some(participant), HistoryDirection::Redo)
                .map(|step| step.command_id),
            Some(transfer)
        );
    }

    let replacement = layer_edit(&mut history, source, "new source edit");
    assert_eq!(
        history
            .top_step(Some(source), HistoryDirection::Undo)
            .map(|step| step.command_id),
        Some(replacement)
    );
    assert!(history
        .top_step(Some(source), HistoryDirection::Redo)
        .is_none());
    assert!(history
        .top_step(Some(destination), HistoryDirection::Redo)
        .is_none());
    assert!(history.payload::<TransferRecord>(transfer).is_none());
}

#[test]
fn successive_transfers_evict_oldest_once_and_keep_both_heads_coherent() {
    let source = scene_key(12, 1);
    let destination = scene_key(13, 1);
    let mut history = WorkspaceHistory::new(2, 16);

    let first = commit_transfer(&mut history, source, destination, false);
    let second = commit_transfer(&mut history, destination, source, false);
    let third = commit_transfer(&mut history, source, destination, false);

    assert!(history.transfer_payload(first).is_none());
    assert!(history.transfer_payload(second).is_some());
    assert!(history.transfer_payload(third).is_some());
    assert_eq!(history.entries.len(), 2);
    assert!(history.used_bytes() <= 16);
    for participant in [source, destination] {
        assert_eq!(
            history
                .top_step(Some(participant), HistoryDirection::Undo)
                .map(|step| step.command_id),
            Some(third)
        );
    }
    assert_eq!(
        history.validate_navigation(third, HistoryDirection::Undo),
        Ok(())
    );

    assert_eq!(
        history.commit_navigation(third, HistoryDirection::Undo),
        Ok(())
    );
    for participant in [source, destination] {
        assert_eq!(
            history
                .top_step(Some(participant), HistoryDirection::Redo)
                .map(|step| step.command_id),
            Some(third)
        );
        assert_eq!(
            history
                .top_step(Some(participant), HistoryDirection::Undo)
                .map(|step| step.command_id),
            Some(second)
        );
    }
    assert_eq!(
        history.validate_navigation(second, HistoryDirection::Undo),
        Ok(())
    );
}

#[test]
fn transfer_evicts_oldest_entries_when_only_the_byte_budget_is_full() {
    let source = scene_key(14, 1);
    let destination = scene_key(15, 1);
    let mut history = WorkspaceHistory::new(16, 3);
    let oldest = layer_edit(&mut history, source, "a");
    let retained_source_edit = layer_edit(&mut history, source, "b");
    let retained_destination_edit = layer_edit(&mut history, destination, "c");
    assert_eq!(history.used_bytes(), 3);

    let transfer = commit_transfer(&mut history, source, destination, false);

    assert!(history.payload::<String>(oldest).is_none());
    assert!(history.payload::<String>(retained_source_edit).is_some());
    assert!(history
        .payload::<String>(retained_destination_edit)
        .is_some());
    assert_eq!(history.used_bytes(), 3);
    assert_eq!(history.entries.len(), 3);
    for participant in [source, destination] {
        assert_eq!(
            history
                .top_step(Some(participant), HistoryDirection::Undo)
                .map(|step| step.command_id),
            Some(transfer)
        );
    }
    assert_eq!(
        history.validate_navigation(transfer, HistoryDirection::Undo),
        Ok(())
    );
}

#[test]
fn discarding_a_prepared_transfer_does_not_apply_its_eviction_plan() {
    let source = scene_key(16, 1);
    let destination = scene_key(17, 1);
    let mut history = WorkspaceHistory::new(1, 4);
    let existing = layer_edit(&mut history, source, "a");
    let used_bytes = history.used_bytes();
    let transfer = match history.prepare_transfer(transfer_record(source, destination, false), 1) {
        Ok(id) => id,
        Err(error) => panic!("the pending transfer should fit after eviction: {error:?}"),
    };

    assert_eq!(history.used_bytes(), used_bytes + 1);
    assert_eq!(
        history
            .top_step(Some(source), HistoryDirection::Undo)
            .map(|step| step.command_id),
        Some(existing)
    );
    assert!(history.discard_pending(transfer));

    assert_eq!(history.used_bytes(), used_bytes);
    assert!(history.payload::<String>(existing).is_some());
    assert_eq!(
        history
            .top_step(Some(source), HistoryDirection::Undo)
            .map(|step| step.command_id),
        Some(existing)
    );
    assert!(history
        .top_step(Some(destination), HistoryDirection::Undo)
        .is_none());
}

#[test]
fn refused_checkpoint_and_transfer_leave_history_heads_and_accounting_unchanged() {
    let source = scene_key(10, 1);
    let destination = scene_key(11, 1);
    let mut history = WorkspaceHistory::new(1, 4);
    let existing = layer_edit(&mut history, source, "a");
    let used_bytes = history.used_bytes();

    assert!(history
        .begin_edit_checkpoint(Some(source), Scene::new(), 4)
        .is_none());
    assert_eq!(history.used_bytes(), used_bytes);
    assert_eq!(
        history
            .top_step(Some(source), HistoryDirection::Undo)
            .map(|step| step.command_id),
        Some(existing)
    );

    assert_eq!(
        history.prepare_transfer(transfer_record(source, destination, false), 5),
        Err(super::HistoryError::BudgetExceeded)
    );
    assert_eq!(history.used_bytes(), used_bytes);
    assert_eq!(
        history
            .top_step(Some(source), HistoryDirection::Undo)
            .map(|step| step.command_id),
        Some(existing)
    );
    assert!(history
        .top_step(Some(destination), HistoryDirection::Undo)
        .is_none());
}

#[test]
fn auto_created_destination_redo_rekeys_to_a_fresh_scene_epoch() {
    let source = scene_key(6, 1);
    let removed_destination = scene_key(7, 2);
    let recreated_destination = scene_key(7, 3);
    let mut history = WorkspaceHistory::new(16, 1024);
    let transfer = commit_transfer(&mut history, source, removed_destination, true);
    assert_eq!(
        history.commit_navigation(transfer, HistoryDirection::Undo),
        Ok(())
    );
    assert_eq!(
        history.validate_transfer_scene_key_replacement(
            transfer,
            removed_destination,
            recreated_destination
        ),
        Ok(())
    );
    assert_eq!(
        history.replace_transfer_scene_key(transfer, removed_destination, recreated_destination),
        Ok(())
    );
    let Some(record) = history.transfer_payload(transfer) else {
        panic!("transfer payload should still exist");
    };
    assert_eq!(record.destination, recreated_destination);
    assert_eq!(record.active_scene_after, recreated_destination);
    assert_eq!(
        history.validate_navigation(transfer, HistoryDirection::Redo),
        Ok(())
    );
    assert_eq!(
        history.commit_navigation(transfer, HistoryDirection::Redo),
        Ok(())
    );
}

#[test]
fn closing_a_participant_discards_transfer_prefix_but_keeps_later_neighbor_work() {
    let closed_scene = scene_key(8, 1);
    let survivor = scene_key(9, 1);
    let mut history = WorkspaceHistory::new(16, 1024);
    let transfer = commit_transfer(&mut history, closed_scene, survivor, false);
    let later_edit = layer_edit(&mut history, survivor, "independent later edit");

    history.close_scope(Some(closed_scene));
    assert!(matches!(
        history.validate_navigation(transfer, HistoryDirection::Undo),
        Err(NavigationError::UnknownCommand(_))
    ));
    assert_eq!(
        history
            .top_step(Some(survivor), HistoryDirection::Undo)
            .map(|step| step.command_id),
        Some(later_edit)
    );
}
