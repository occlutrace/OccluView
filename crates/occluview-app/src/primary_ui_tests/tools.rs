//! Contracts over the viewport tools: what a tool leaves behind when it
//! closes, what it promises the operator, and who owns the pointer.

use super::*;

#[test]
fn closing_the_align_tool_leaves_no_setting_behind() {
    // Seventeen of the eighteen align-session fields were reset by hand and one
    // was not. An axis lock — "Z only" — set on one case survived into the next
    // pair of scans, where the scan refuses to move sideways and reads as stuck
    // rather than as a setting that is still on. The field is listed here by
    // name so the next field added to the session has to be listed too.
    let source = repo_source_file("src/app/app_align.rs");
    let start = source.find("pub(super) fn disarm_align_tool(");
    assert!(start.is_some(), "the align teardown should exist");
    let Some(start) = start else {
        return;
    };
    let body = &source[start..];
    let end = body
        .find("\n    }\n")
        .map_or(body.len(), |offset| offset + 6);
    let disarm = &body[..end];

    for reset in [
        "self.finish_align_drag();",
        "self.align_drag = None;",
        "self.clear_deviation_overlay();",
        "self.clear_align_mask();",
        "self.align_geometry.clear();",
        "self.align.disarm();",
        "self.align_status = None;",
        "self.align_stats = None;",
        "self.align_rejected.clear();",
        "self.align_session_poses.clear();",
        "self.align_brush.set_armed(false);",
        "self.align_tab = crate::align_panel::AlignTab::default();",
        "self.align_constraint = crate::align_drag::DragConstraint::default();",
    ] {
        assert!(
            disarm.contains(reset),
            "closing the align tool must reset the session: missing `{reset}`"
        );
    }
}

#[test]
fn no_status_line_promises_an_undo_that_was_not_stored() {
    // `begin_layer_edit_with_snapshot` skips an oversized pre-op snapshot: the
    // edit applies and Ctrl+Z will not undo it. Every mesh-edit status routes
    // through `with_undoable_note` for that reason; the sculpt path printed
    // "(Ctrl+Z undoes)" unconditionally, so an operator learned the promise was
    // empty by pressing it, on work they had already moved past.
    let sculpt = repo_source_file("src/app/app_sculpt_worker.rs");
    let promise = "(Ctrl+Z undoes)";
    assert!(
        sculpt.contains(promise),
        "the sculpt status should still say how to undo when it can be undone"
    );
    assert!(
        sculpt.contains("last_edit_undoable()"),
        "the promise must be conditional on the snapshot actually being stored"
    );
    assert!(
        sculpt.contains("not undoable: snapshot too large"),
        "the other branch should say plainly that it cannot be undone"
    );
    assert!(
        appears_before(&sculpt, "last_edit_undoable()", promise),
        "the check has to come before the promise"
    );

    // The shared helper stays the single wording for every other path.
    let layer_edits = repo_source_file("src/app/app_layer_edits/mod.rs");
    assert!(layer_edits.contains("fn with_undoable_note("));
    assert!(layer_edits.contains("not undoable: snapshot too large"));
}

#[test]
fn both_disc_tools_ask_one_question_about_the_pointer() {
    // The bridge tool and the cut tool each decide whether a click landed on
    // chrome or on bare scene, and each used to decide it in its own copy.
    // Adding one overlay meant editing two functions; missing one made clicks
    // on that panel plant or drag a disc underneath it for exactly one of the
    // two tools -- no compile error, no failing test, and a bug report that
    // reads "sometimes clicking the panel moves the cut".
    let cut = repo_source_file("src/app/app_cut_measure.rs");
    let bridge = repo_source_file("src/app/app_bridge_split.rs");

    assert!(
        cut.contains("pub(super) fn viewport_pointer("),
        "the arbitration should live in exactly one named place"
    );
    for (name, source) in [("cut", &cut), ("bridge", &bridge)] {
        assert!(
            source.contains("= self.viewport_pointer("),
            "the {name} tool must ask the shared question"
        );
    }
    // The gizmo footprint has to come from the same call the painter uses; a
    // hand-rolled equivalent is how the hit box ends up far from the glyph.
    assert_eq!(
        count_occurrences(&cut, "axis_gizmo::axis_gizmo_footprint(")
            + count_occurrences(&bridge, "axis_gizmo::axis_gizmo_footprint("),
        1,
        "the gizmo footprint test must exist once, inside the shared arbitration"
    );
    assert!(
        !bridge.contains("crate::cut_ruler::section_panel_rect("),
        "the bridge tool must not compute its own avoid-rect for the gizmo"
    );
}

#[test]
fn the_one_guarded_forwarder_is_not_named_like_the_others() {
    // Twenty-nine `foo` / `foo_impl` pairs in this crate are pure
    // pass-throughs, which teaches a reader that the two are interchangeable.
    // One is not: `handle_edit_shortcuts` refuses to run while a dialog is
    // open, because undoing a mesh edit behind the unsaved-changes prompt
    // silently changes what "Save" then writes. Under the shared naming that
    // callee was a trapdoor one plausible call wide.
    let state = repo_source_file("src/app/state.rs");
    let editor = repo_source_file("src/app/app_mesh_editor.rs");

    assert!(
        state.contains("self.handle_edit_shortcuts_unguarded(ctx);"),
        "the guard should call a callee whose name says it is unguarded"
    );
    assert!(
        editor.contains("pub(super) fn handle_edit_shortcuts_unguarded("),
        "the hotkey body should carry the unguarded name"
    );
    let by_habit = format!("handle_edit_shortcuts{}", "_impl");
    assert!(
        !editor.contains(&by_habit) && !state.contains(&format!("self.{by_habit}")),
        "the pass-through name invites a call that skips the dialog check"
    );
    // And the guard itself must still refuse every dialog.
    for dialog in [
        "self.close_guard_open",
        "self.pending_replace_open.is_some()",
        "self.app_error.is_some()",
        "self.about_window == AboutWindowState::Open",
        "self.third_party_window_open",
    ] {
        assert!(
            state.contains(dialog),
            "edit hotkeys must stay refused while {dialog} is up"
        );
    }
}

#[test]
fn the_two_cut_cap_colours_are_one_colour() {
    // The app and the renderer each name the cap's colour. They have to agree,
    // and both have to be in the space the render target actually is: a linear
    // conversion of #E84C4B reached the screen as (198, 46, 45) under a comment
    // that said (232, 76, 75).
    let app = repo_source_file("src/cut_tool.rs");
    let renderer = include_str!("../../../occluview-render/src/clipping.rs");
    let value = "[0.910, 0.298, 0.294, 1.0]";
    assert!(
        app.contains(value),
        "the app's cap colour should be the sRGB fractions of #E84C4B"
    );
    assert!(
        renderer.contains(value),
        "the renderer's default cap colour should be the same value"
    );
}

#[test]
fn escape_belongs_to_the_dialog_in_front_not_the_tool_behind() {
    // Five hand-written copies of "is a dialog open" had drifted to three,
    // four and six terms. The cut and align tools did not count the
    // replace-open guard, and none counted the third-party licences window --
    // so with either up, Escape tore the tool down behind the dialog, and for
    // align it also ran `cancel_align_session`, putting every scan back where
    // it started.
    // The predicate itself is tested for behaviour next to its definition, in
    // app::open_dialogs. What only a source guard can check is that the five
    // call sites still ask it instead of counting dialogs again themselves,
    // which is the shape the drift took.
    let state = repo_source_file("src/app/state.rs");
    assert!(
        state.contains("pub(super) fn modal_dialog_open(&self) -> bool"),
        "the predicate should exist once"
    );
    for dialog in [
        "self.close_guard_open",
        "self.pending_replace_open.is_some()",
        "self.app_error.is_some()",
        "self.about_window == AboutWindowState::Open",
        "self.third_party_window_open",
    ] {
        assert!(state.contains(dialog), "the predicate must count {dialog}");
    }

    for module in [
        "src/app/app_bridge_split.rs",
        "src/app/app_cut_measure.rs",
        "src/app/app_align.rs",
    ] {
        let source = repo_source_file(module);
        assert!(
            source.contains("self.modal_dialog_open()"),
            "{module} should ask the shared predicate"
        );
        assert!(
            !source.contains("let dialogs_open = self.close_guard_open"),
            "{module} must not keep its own copy, which is how they drifted"
        );
    }
}

/// The scene-editing calls that require `self.scene` to be the only handle.
const IN_PLACE_SCENE_EDITS: &[&str] = &[
    "self.live_scene_mut()",
    "self.attach_overlay_colors(",
    "self.patch_overlay_colors(",
    "self.repaint_region_preview(",
];

/// Every place a scene handle is still alive when an in-place edit runs.
///
/// A handle is alive from the `self.scene.clone()` that made it until the
/// scan leaves the brace depth it was made at, or until an explicit `drop`.
/// Anything in between that edits the scene in place will find a second
/// handle, and `Arc::make_mut` will copy the whole case.
///
/// This is a structural property rather than a snapshot of the current
/// wording: renaming a binding, reflowing an argument list or restructuring
/// a loop leaves it intact, and moving a clone out of its block does not.
fn scene_handles_alive_across_an_edit(source: &str) -> Vec<String> {
    let bytes: Vec<char> = source.chars().collect();
    let mut depth_at = vec![0i32; bytes.len() + 1];
    let mut depth = 0i32;
    for (index, character) in bytes.iter().enumerate() {
        match character {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        depth_at[index] = depth;
    }

    let mut offenders = Vec::new();
    for (clone_at, _) in source.match_indices("self.scene.clone()") {
        let clone_index = source[..clone_at].chars().count();
        let held_at = depth_at[clone_index];
        // The handle dies with its block, or where the code says so.
        let released = depth_at
            .iter()
            .enumerate()
            .skip(clone_index)
            .find(|(_, depth)| **depth < held_at)
            .map_or(bytes.len(), |(index, _)| index);
        let tail: String = bytes[clone_index..released].iter().collect();
        let tail = tail
            .split_once("drop(scene)")
            .map_or(tail.as_str(), |(a, _)| a);
        for edit in IN_PLACE_SCENE_EDITS {
            if tail.contains(edit) {
                let line = source[..clone_at].matches('\n').count() + 1;
                offenders.push(format!("line {line}: handle still alive at {edit}"));
            }
        }
    }
    offenders
}

#[test]
fn nothing_holds_a_second_scene_handle_across_an_in_place_edit() {
    // `live_scene_mut` asserts in debug that it holds the only Arc<Scene>,
    // because Arc::make_mut copies the whole case when it does not -- 52 ms
    // per frame, measured on two 945k-vertex arches, on paths that run every
    // frame. That assertion is the detector; this is what stops the shape
    // coming back, and it checks the property rather than the wording.
    for module in [
        "src/app/app_align_brush.rs",
        "src/app/app_align_display.rs",
        "src/app/app_align_drag.rs",
        "src/app/app_layer_interaction.rs",
        "src/app/app_bridge_split.rs",
        "src/app/app_sculpt.rs",
    ] {
        let source = repo_source_file(module);
        let offenders = scene_handles_alive_across_an_edit(&source);
        assert!(
            offenders.is_empty(),
            "{module} edits the scene in place while a second handle is \
             alive:\n{}",
            offenders.join("\n")
        );
    }

    // The two shapes that made the handle inevitable rather than incidental.
    let brush = repo_source_file("src/app/app_align_brush.rs");
    assert!(
        brush.contains("fn region_colors_for("),
        "the region colours must be computed as values; a closure over the \
         vertices keeps a handle alive by construction"
    );
    let display = repo_source_file("src/app/app_align_display.rs");
    assert!(
        display.contains("patched: &[[u8; 4]],"),
        "the patch writer takes colours, not a closure that can read the scene"
    );

    // A background thread outlives every block, so the rule above cannot see
    // it: what it takes has to be the mesh, not the case.
    for (module, taken) in [
        ("src/sculpt_tool.rs", "let mesh = entry.mesh.clone();"),
        ("src/app/app_bridge_split.rs", "let target_mesh = self"),
    ] {
        let source = repo_source_file(module);
        assert!(
            source.contains(taken),
            "{module} spawns a worker; it must take the mesh it needs, not \
             an Arc<Scene> that makes every scene edit copy the case"
        );
    }
}
