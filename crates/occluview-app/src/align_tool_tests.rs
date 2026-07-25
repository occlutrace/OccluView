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
