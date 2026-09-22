//! Contracts over the viewport tools: what a tool leaves behind when it
//! closes, what it promises the operator, and who owns the pointer.

use super::*;

/// The scene-editing calls that require `self.scene` to be the only handle.
const IN_PLACE_SCENE_EDITS: &[&str] = &[
    "self.document.live_scene_mut()",
    "self.attach_overlay_colors(",
    "self.patch_overlay_colors(",
    "self.repaint_region_preview(",
];

/// Find scene handles held across in-place edits.
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
    for (clone_at, _) in source.match_indices("self.document.scene.clone()") {
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
    // `live_scene_mut` asserts in debug that it holds the only Arc<Scene>: a
    // second handle means a reader that will not see the edit, and a container
    // copied per frame. The assertion is the runtime detector; this analyzer is
    // what stops the shape coming back in a module no test happens to drive,
    // and it checks the property (a handle alive past its block) rather than
    // the wording. A renamed binding or a reflowed argument list leaves it
    // intact; moving a clone out of its block does not.
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
