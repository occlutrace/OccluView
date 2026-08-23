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
        sculpt.find("last_edit_undoable()") < sculpt.find(promise),
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

#[test]
fn nothing_holds_a_second_scene_handle_across_an_in_place_edit() {
    // `live_scene_mut` asserts in debug that it holds the only Arc<Scene>,
    // because Arc::make_mut copies the whole case when it does not -- 45 ms
    // per frame on paths that run per frame. The assertion is the real
    // detector and fires on the first brush dab of any debug build. What it
    // cannot do is describe the four shapes that tripped it, so those are
    // pinned here.
    //
    // A source guard cannot prove the general rule; a handle can be held
    // anywhere. It can keep these four from coming back.
    let brush = repo_source_file("src/app/app_align_brush.rs");
    assert!(
        brush.contains("let (layer_id, painting, changed) = {"),
        "the dab must read the scene inside a block that closes before it \
         patches the preview"
    );
    assert!(
        brush.contains("fn region_colors_for("),
        "the region colours must be computed as values; a closure over the \
         vertices keeps a handle alive by construction"
    );
    assert!(
        !brush.contains("let recolor = |vertex: usize|"),
        "the closure form is what held the handle"
    );
    assert!(
        brush.contains("let sides: Vec<(AlignSide, SceneMeshId, SceneMesh)> = {"),
        "a whole-mesh mask command must take both layers out of the scene \
         before it repaints either preview"
    );

    let display = repo_source_file("src/app/app_align_display.rs");
    assert!(
        display.contains("patched: &[[u8; 4]],"),
        "the patch writer takes colours, not a closure that can read the scene"
    );

    let sculpt = repo_source_file("src/sculpt_tool.rs");
    assert!(
        sculpt.contains("let mesh = entry.mesh.clone();") && sculpt.contains("drop(scene);"),
        "the sculpt worker takes the one mesh it prepares; an Arc<Scene> in a \
         background thread makes every scene edit on the UI thread copy the case"
    );
    let Some((_, worker)) = sculpt.split_once(".spawn(move || {") else {
        panic!("the preparation worker should still be spawned here");
    };
    let Some((worker, _)) = worker.split_once("\n            });") else {
        panic!("the preparation worker body should be delimited");
    };
    assert!(
        !worker.contains("scene"),
        "the preparation worker must not hold the scene:\n{worker}"
    );
}
