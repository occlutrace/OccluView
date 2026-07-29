//! Tests for the Align Scans click model, split out of `align_tool.rs`.
#![allow(clippy::expect_used)]

use crate::align_tool::{AlignPoint, AlignTool, ClickOutcome};
use glam::Vec3;
use occluview_core::{Mesh, Scene, SceneMesh, SceneMeshId, Vertex};

/// Three distinct scene layer ids. Ids are issued by the scene, so this builds
/// a real scene rather than inventing them.
fn ids() -> [SceneMeshId; 3] {
    let mut scene = Scene::new();
    for _ in 0..3 {
        let mesh = Mesh::new(
            None,
            vec![
                Vertex::at(Vec3::ZERO),
                Vertex::at(Vec3::new(1.0, 0.0, 0.0)),
                Vertex::at(Vec3::new(0.0, 1.0, 0.0)),
            ],
            vec![0, 1, 2],
        )
        .expect("valid mesh");
        scene.add(SceneMesh::new(mesh));
    }
    let meshes = scene.meshes();
    [meshes[0].id(), meshes[1].id(), meshes[2].id()]
}

fn point(layer: SceneMeshId, local: Vec3) -> AlignPoint {
    AlignPoint {
        layer,
        local,
        normal: Vec3::Z,
    }
}

fn armed() -> AlignTool {
    let mut tool = AlignTool::default();
    tool.arm();
    tool
}

#[test]
fn a_disarmed_tool_ignores_clicks() {
    let [a, _, _] = ids();
    let mut tool = AlignTool::default();
    assert!(!tool.is_armed());
    assert_eq!(tool.click(point(a, Vec3::ZERO)), ClickOutcome::Ignored);
    assert_eq!(tool.moving_layer(), None);
}

#[test]
fn the_first_click_names_the_moving_scan() {
    let [a, _, _] = ids();
    let mut tool = armed();
    assert_eq!(tool.click(point(a, Vec3::ZERO)), ClickOutcome::StartedPair);
    assert_eq!(tool.moving_layer(), Some(a));
    assert_eq!(tool.fixed_layer(), None);
    assert!(tool.pairs().is_empty());
}

#[test]
fn a_click_on_another_layer_completes_the_first_pair() {
    let [a, b, _] = ids();
    let mut tool = armed();
    tool.click(point(a, Vec3::ZERO));
    assert_eq!(
        tool.click(point(b, Vec3::X)),
        ClickOutcome::CompletedPair(0)
    );
    assert_eq!(tool.fixed_layer(), Some(b));
    assert_eq!(tool.pairs().len(), 1);
}

#[test]
fn a_completed_pair_keeps_each_half_on_its_own_layer() {
    let [a, b, _] = ids();
    let mut tool = armed();
    // Click the fixed side first on the second pair: the pair must still be
    // stored moving-first, or a fit would run backwards.
    tool.click(point(a, Vec3::ZERO));
    tool.click(point(b, Vec3::X));
    tool.click(point(b, Vec3::Y));
    tool.click(point(a, Vec3::Z));

    let pair = tool.pairs()[1];
    assert_eq!(pair.moving.layer, a);
    assert_eq!(pair.fixed.layer, b);
    assert_eq!(pair.moving.local, Vec3::Z);
    assert_eq!(pair.fixed.local, Vec3::Y);
}

#[test]
fn a_repeat_click_on_the_pending_layer_relocates_that_point() {
    let [a, _, _] = ids();
    let mut tool = armed();
    tool.click(point(a, Vec3::ZERO));
    assert_eq!(tool.click(point(a, Vec3::Y)), ClickOutcome::MovedPending);
    assert!(tool.pairs().is_empty());
    assert_eq!(tool.pending().map(|pending| pending.local), Some(Vec3::Y));
}

#[test]
fn a_click_on_a_third_layer_is_refused_without_mutating_anything() {
    let [a, b, c] = ids();
    let mut tool = armed();
    tool.click(point(a, Vec3::ZERO));
    tool.click(point(b, Vec3::X));
    assert_eq!(
        tool.click(point(c, Vec3::Y)),
        ClickOutcome::RefusedThirdLayer
    );
    assert_eq!(tool.pairs().len(), 1);
    assert_eq!(tool.pending(), None);
}

#[test]
fn two_visible_layers_imply_the_pair_before_any_click() {
    let [a, b, _] = ids();
    let mut tool = armed();
    tool.imply_pair(&[a, b]);
    assert_eq!(tool.moving_layer(), Some(a));
    assert_eq!(tool.fixed_layer(), Some(b));
    assert!(
        tool.can_measure(),
        "an implied pair is enough to compare two files"
    );
}

#[test]
fn three_visible_layers_imply_nothing() {
    let [a, b, c] = ids();
    let mut tool = armed();
    tool.imply_pair(&[a, b, c]);
    assert_eq!(tool.moving_layer(), None);
}

#[test]
fn an_implied_pair_does_not_override_a_named_one() {
    let [a, b, c] = ids();
    let mut tool = armed();
    tool.click(point(c, Vec3::ZERO));
    tool.imply_pair(&[a, b]);
    assert_eq!(tool.moving_layer(), Some(c));
}

#[test]
fn back_walks_the_points_and_clear_resets_the_pair() {
    let [a, b, _] = ids();
    let mut tool = armed();
    tool.click(point(a, Vec3::ZERO));
    tool.click(point(b, Vec3::X));
    assert!(tool.back());
    assert!(tool.pairs().is_empty());
    assert_eq!(
        tool.moving_layer(),
        Some(a),
        "back drops a point, not the pair"
    );

    tool.clear();
    assert_eq!(tool.moving_layer(), None);
    assert_eq!(tool.fixed_layer(), None);
}

#[test]
fn back_on_an_empty_tool_reports_that_it_did_nothing() {
    let mut tool = armed();
    assert!(!tool.back());
}

#[test]
fn align_needs_two_pairs_and_a_measurement_needs_only_the_pair() {
    let [a, b, _] = ids();
    let mut tool = armed();
    tool.click(point(a, Vec3::ZERO));
    tool.click(point(b, Vec3::X));
    assert!(tool.can_measure());
    assert!(!tool.can_align());

    tool.click(point(a, Vec3::Y));
    tool.click(point(b, Vec3::Z));
    assert!(tool.can_align());
}

#[test]
fn clearing_lets_a_third_scan_start_a_new_pair() {
    let [a, b, c] = ids();
    let mut tool = armed();
    tool.click(point(a, Vec3::ZERO));
    tool.click(point(b, Vec3::X));
    tool.clear();
    assert_eq!(tool.click(point(c, Vec3::ZERO)), ClickOutcome::StartedPair);
    assert_eq!(tool.moving_layer(), Some(c));
    assert!(tool.is_armed(), "clearing must not disarm the tool");
}

#[test]
fn points_are_stored_in_layer_local_coordinates() {
    let [a, b, _] = ids();
    let mut tool = armed();
    tool.click(point(a, Vec3::new(1.0, 2.0, 3.0)));
    tool.click(point(b, Vec3::new(4.0, 5.0, 6.0)));

    let pair = tool.pairs()[0];
    assert_eq!(pair.moving.local, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(pair.fixed.local, Vec3::new(4.0, 5.0, 6.0));
}

#[test]
fn a_removed_layer_resets_the_pair_instead_of_leaving_half_of_one() {
    let [a, b, c] = ids();
    let mut tool = armed();
    tool.click(point(a, Vec3::ZERO));
    tool.click(point(b, Vec3::X));

    tool.forget_layer(c);
    assert_eq!(
        tool.moving_layer(),
        Some(a),
        "an unrelated layer changes nothing"
    );

    tool.forget_layer(b);
    assert_eq!(tool.moving_layer(), None);
    assert!(tool.pairs().is_empty());
}

#[test]
fn disarming_drops_the_session() {
    let [a, b, _] = ids();
    let mut tool = armed();
    tool.click(point(a, Vec3::ZERO));
    tool.click(point(b, Vec3::X));

    tool.disarm();
    assert!(!tool.is_armed());
    assert!(tool.pairs().is_empty());
    assert_eq!(tool.moving_layer(), None);
}

/// The arm-time guess is a guess, and the first click overrides it.
///
/// This is the bug an operator reported as "it aligned the other way round". Two
/// scans in view, so the pair was implied from scene order — which is the order
/// the files were opened in. Clicking the scan they wanted moved then landed on
/// whichever role that scan had been handed, and half the time the alignment ran
/// backwards for reasons nothing on screen explained.
#[test]
fn the_first_clicked_scan_is_the_one_that_moves_even_after_a_guess() {
    let [a, b, _] = ids();
    let mut tool = armed();
    tool.imply_pair(&[a, b]);
    assert!(
        tool.roles_are_implied(),
        "a guess has to admit to being one"
    );

    // The operator clicks the scan the guess had put on the fixed side.
    tool.click(point(b, Vec3::ZERO));
    assert_eq!(
        tool.moving_layer(),
        Some(b),
        "the clicked scan is the mover"
    );
    assert_eq!(tool.fixed_layer(), Some(a));
    assert!(
        !tool.roles_are_implied(),
        "once the operator has clicked, the roles are theirs"
    );
}

/// Clicking the scan the guess already had right changes nothing.
#[test]
fn a_guess_the_operator_agrees_with_is_left_alone() {
    let [a, b, _] = ids();
    let mut tool = armed();
    tool.imply_pair(&[a, b]);
    tool.click(point(a, Vec3::ZERO));
    assert_eq!(tool.moving_layer(), Some(a));
    assert_eq!(tool.fixed_layer(), Some(b));
}

/// A second click never re-decides. The roles are settled by the first point,
/// and a pair is placed by clicking alternately — so re-deciding on every click
/// would flip the direction on the way to placing one arrow.
#[test]
fn only_the_first_click_decides_the_direction() {
    let [a, b, _] = ids();
    let mut tool = armed();
    tool.imply_pair(&[a, b]);
    tool.click(point(a, Vec3::ZERO));
    tool.click(point(b, Vec3::X));
    tool.click(point(b, Vec3::Y));
    assert_eq!(tool.moving_layer(), Some(a), "the direction held");
    assert_eq!(tool.pairs().len(), 1);
}

/// Swapping trades the roles and takes the placed arrows with them. A pair holds
/// "the moving point and its partner", so leaving the halves alone would fit the
/// scans with every correspondence reversed.
#[test]
fn swapping_the_pair_turns_the_arrows_round_too() {
    let [a, b, _] = ids();
    let mut tool = armed();
    tool.click(point(a, Vec3::ZERO));
    tool.click(point(b, Vec3::X));
    let before = tool.pairs()[0];

    assert!(tool.swap_roles());
    assert_eq!(tool.moving_layer(), Some(b));
    assert_eq!(tool.fixed_layer(), Some(a));

    let after = tool.pairs()[0];
    assert_eq!(
        after.moving, before.fixed,
        "the halves swapped with the roles"
    );
    assert_eq!(after.fixed, before.moving);
    assert_eq!(
        after.moving.layer,
        tool.moving_layer()
            .expect("a pair means both roles are named"),
        "every pair's moving half must belong to the moving layer"
    );
}

/// Swapping twice is where it started. Nothing accumulates.
#[test]
fn swapping_twice_puts_everything_back() {
    let [a, b, _] = ids();
    let mut tool = armed();
    tool.click(point(a, Vec3::ZERO));
    tool.click(point(b, Vec3::X));
    let before = tool.pairs()[0];
    assert!(tool.swap_roles());
    assert!(tool.swap_roles());
    assert_eq!(tool.moving_layer(), Some(a));
    assert_eq!(tool.pairs()[0], before);
}

/// There is nothing to swap before both scans are named.
#[test]
fn a_half_named_pair_cannot_be_turned_round() {
    let [a, _, _] = ids();
    let mut tool = armed();
    tool.click(point(a, Vec3::ZERO));
    assert!(!tool.swap_roles());
    assert_eq!(tool.moving_layer(), Some(a));
}

/// Clear takes the guess with it, so the next click decides from scratch. This
/// is the path to aligning a third file without closing the tool.
#[test]
fn clearing_a_guessed_pair_leaves_nothing_guessed() {
    let [a, b, c] = ids();
    let mut tool = armed();
    tool.imply_pair(&[a, b]);
    tool.clear();
    assert!(!tool.roles_are_implied());
    assert_eq!(tool.moving_layer(), None);

    tool.click(point(c, Vec3::ZERO));
    assert_eq!(
        tool.moving_layer(),
        Some(c),
        "a third scan can start a pair"
    );
}
