//! Behaviour tests for the markings on both scans.
//!
//! These run the real code with plain arrays — no scene, no camera, no GPU —
//! which is the whole point of the type existing. Every case below is a defect
//! an operator reported by hand before there was anywhere to write it down.

#![allow(clippy::expect_used)]

use super::{AlignMarkings, AlignSide, AutoKeep, MarkedMesh, MarkedOn, MaskCommand};
use glam::DVec3;
use occluview_align::{MaskEdit, Rigid, EXCLUDED, INCLUDED};

/// The one mesh identity every test below paints on.
const SUBJECT: u64 = 7;

/// The identity of a mesh with this many vertices, as production builds it.
fn on(vertex_count: usize) -> MarkedOn {
    MarkedOn {
        geometry: SUBJECT,
        vertex_count,
    }
}

/// Whether one vertex is marked out of the match, read the way production reads
/// it — through the identity-checked mask, so a test cannot pass on a mask that
/// belongs to different geometry.
fn marked(markings: &AlignMarkings, side: AlignSide, vertices: usize, vertex: usize) -> bool {
    markings
        .mask_for(side, on(vertices))
        .and_then(|mask| mask.get(vertex).copied())
        .is_some_and(|slot| slot == EXCLUDED)
}

/// A flat grid of vertices one millimetre apart, centred on the origin.
fn grid(side_vertices: usize) -> Vec<f32> {
    let mut positions = Vec::with_capacity(side_vertices * side_vertices * 3);
    #[allow(clippy::cast_precision_loss)]
    for row in 0..side_vertices {
        for column in 0..side_vertices {
            positions.push(column as f32 - side_vertices as f32 / 2.0);
            positions.push(row as f32 - side_vertices as f32 / 2.0);
            positions.push(0.0);
        }
    }
    positions
}

/// The mesh handle the marking calls take.
fn mesh(positions: &[f32]) -> MarkedMesh<'_> {
    MarkedMesh {
        positions,
        pose: Rigid::IDENTITY,
        vertex_count: positions.len() / 3,
        geometry: SUBJECT,
    }
}

/// One dab at the origin.
fn dab_at(center: DVec3, radius_mm: f64, erase: bool) -> MaskEdit {
    MaskEdit {
        center,
        radius_mm,
        erase,
    }
}

#[test]
fn both_scans_can_be_marked_and_neither_has_to_be_chosen_first() {
    // The reported defect: "on one surface it marks, on the other it does not."
    // There was one mask for the pair, so whichever scan was not the moving one
    // could never hold a marking.
    let positions = grid(16);
    let mut markings = AlignMarkings::default();

    let on_moving = markings.dab(
        AlignSide::Moving,
        &mesh(&positions),
        &dab_at(DVec3::ZERO, 2.0, false),
    );
    let on_fixed = markings.dab(
        AlignSide::Fixed,
        &mesh(&positions),
        &dab_at(DVec3::ZERO, 2.0, false),
    );

    assert!(on_moving > 0, "the dab on the moving scan marked nothing");
    assert!(on_fixed > 0, "the dab on the fixed scan marked nothing");
    assert!(marked(&markings, AlignSide::Moving, 16 * 16, 8 * 16 + 8));
    assert!(marked(&markings, AlignSide::Fixed, 16 * 16, 8 * 16 + 8));
}

#[test]
fn marking_one_scan_leaves_the_other_untouched() {
    let positions = grid(16);
    let mut markings = AlignMarkings::default();
    markings.dab(
        AlignSide::Moving,
        &mesh(&positions),
        &dab_at(DVec3::ZERO, 2.0, false),
    );
    assert!(markings.mask_for(AlignSide::Moving, on(16 * 16)).is_some());
    assert!(
        markings.mask_for(AlignSide::Fixed, on(16 * 16)).is_none(),
        "a dab on one scan invented a mask on the other"
    );
}

#[test]
fn the_marked_count_matches_a_full_recount_after_every_kind_of_change() {
    // The count is maintained incrementally so the panel never walks the mask.
    // If it ever drifts from the truth the panel reports a confident wrong
    // number, which is worse than reporting none.
    let positions = grid(24);
    let handle = mesh(&positions);
    let vertices = handle.vertex_count;
    let mut markings = AlignMarkings::default();

    // Compared as fractions, not converted back to counts: a float round-trip
    // through a count would hide a one-vertex drift behind rounding, which is
    // exactly the drift this test exists to catch.
    #[allow(clippy::cast_precision_loss)]
    let truth = |markings: &AlignMarkings| -> f32 {
        let counted = (0..vertices)
            .filter(|vertex| marked(markings, AlignSide::Moving, vertices, *vertex))
            .count();
        counted as f32 / vertices as f32
    };
    let reported = |markings: &AlignMarkings| -> f32 {
        markings
            .marked_fraction(on(vertices), on(0))
            .expect("a mask that fits the mesh reports a fraction")
    };
    let agrees = |markings: &AlignMarkings, after: &str| {
        let (reported, truth) = (reported(markings), truth(markings));
        assert!(
            (reported - truth).abs() < f32::EPSILON,
            "{after}: the panel would report {reported} where the mask holds {truth}"
        );
    };

    markings.dab(AlignSide::Moving, &handle, &dab_at(DVec3::ZERO, 4.0, false));
    agrees(&markings, "after a dab");

    markings.dab(
        AlignSide::Moving,
        &handle,
        &dab_at(DVec3::new(1.0, 1.0, 0.0), 2.0, true),
    );
    agrees(&markings, "after an erase");

    markings.command(
        AlignSide::Moving,
        MaskCommand::InvertMarkings,
        &handle,
        &AutoKeep {
            centres: &[],
            radius_mm: 0.0,
        },
    );
    agrees(&markings, "after invert");

    markings.command(
        AlignSide::Moving,
        MaskCommand::FitNowhere,
        &handle,
        &AutoKeep {
            centres: &[],
            radius_mm: 0.0,
        },
    );
    agrees(&markings, "after fit nowhere");

    markings.command(
        AlignSide::Moving,
        MaskCommand::MarkAutomatic,
        &handle,
        &AutoKeep {
            centres: &[DVec3::ZERO],
            radius_mm: 3.0,
        },
    );
    agrees(&markings, "after mark automatic");

    markings.command(
        AlignSide::Moving,
        MaskCommand::FitEverywhere,
        &handle,
        &AutoKeep {
            centres: &[],
            radius_mm: 0.0,
        },
    );
    agrees(&markings, "after fit everywhere");
}

#[test]
fn fit_everywhere_leaves_nothing_marked_and_fit_nowhere_leaves_everything() {
    let positions = grid(12);
    let handle = mesh(&positions);
    let mut markings = AlignMarkings::default();

    markings.command(
        AlignSide::Moving,
        MaskCommand::FitNowhere,
        &handle,
        &AutoKeep {
            centres: &[],
            radius_mm: 0.0,
        },
    );
    let all = markings
        .mask_for(AlignSide::Moving, handle.identity())
        .expect("fit nowhere makes a mask");
    assert!(all.iter().all(|slot| *slot == EXCLUDED));

    markings.command(
        AlignSide::Moving,
        MaskCommand::FitEverywhere,
        &handle,
        &AutoKeep {
            centres: &[],
            radius_mm: 0.0,
        },
    );
    let none = markings
        .mask_for(AlignSide::Moving, handle.identity())
        .expect("fit everywhere keeps the mask, emptied");
    assert!(none.iter().all(|slot| *slot == INCLUDED));
}

#[test]
fn mark_automatic_refuses_without_a_single_arrow() {
    // Marking everything and clearing nothing would leave the match with no
    // surface at all, from a button the operator read as helpful.
    let positions = grid(8);
    let handle = mesh(&positions);
    let mut markings = AlignMarkings::default();

    let reached = markings.command(
        AlignSide::Moving,
        MaskCommand::MarkAutomatic,
        &handle,
        &AutoKeep {
            centres: &[],
            radius_mm: 3.0,
        },
    );

    assert!(!reached, "mark automatic ran with no arrows to keep");
    assert!(markings
        .mask_for(AlignSide::Moving, handle.identity())
        .is_none());
}

#[test]
fn mark_automatic_keeps_the_discs_and_marks_out_the_rest() {
    let positions = grid(24);
    let handle = mesh(&positions);
    let mut markings = AlignMarkings::default();

    markings.command(
        AlignSide::Moving,
        MaskCommand::MarkAutomatic,
        &handle,
        &AutoKeep {
            centres: &[DVec3::ZERO],
            radius_mm: 3.0,
        },
    );

    assert!(
        !marked(&markings, AlignSide::Moving, 24 * 24, 12 * 24 + 12),
        "the vertex under the arrow was marked out of the match"
    );
    assert!(
        marked(&markings, AlignSide::Moving, 24 * 24, 0),
        "the far corner was left in the match"
    );
}

#[test]
fn a_dab_that_changes_nothing_does_not_bump_the_revision() {
    // Every downstream cache keys on the revision. A dab in empty space that
    // still bumped it threw away a measurement of a full arch for nothing.
    let positions = grid(8);
    let handle = mesh(&positions);
    let mut markings = AlignMarkings::default();

    markings.dab(AlignSide::Moving, &handle, &dab_at(DVec3::ZERO, 2.0, false));
    let after_real_dab = markings.revision();

    let changed = markings.dab(
        AlignSide::Moving,
        &handle,
        &dab_at(DVec3::new(500.0, 500.0, 500.0), 1.0, false),
    );

    assert_eq!(changed, 0, "a dab far off the mesh marked something");
    assert_eq!(
        markings.revision(),
        after_real_dab,
        "a dab that changed nothing still invalidated every cache"
    );
}

#[test]
fn every_change_that_does_something_moves_the_revision() {
    let positions = grid(8);
    let handle = mesh(&positions);
    let mut markings = AlignMarkings::default();
    let mut seen = vec![markings.revision()];

    markings.dab(AlignSide::Moving, &handle, &dab_at(DVec3::ZERO, 2.0, false));
    seen.push(markings.revision());
    markings.command(
        AlignSide::Fixed,
        MaskCommand::FitNowhere,
        &handle,
        &AutoKeep {
            centres: &[],
            radius_mm: 0.0,
        },
    );
    seen.push(markings.revision());
    markings.clear();
    seen.push(markings.revision());

    let mut unique = seen.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        seen.len(),
        "two different marking states shared one revision: {seen:?}"
    );
}

#[test]
fn clearing_an_unmarked_pair_costs_no_re_measure() {
    let mut markings = AlignMarkings::default();
    let before = markings.revision();
    assert!(!markings.clear(), "clear reported work it did not do");
    assert_eq!(markings.revision(), before);
}

#[test]
fn a_mask_taken_on_other_geometry_is_not_a_reading_about_this_mesh() {
    // A layer swapped under the tool leaves a mask indexing a different mesh's
    // vertices. Handing it to a job excludes an arbitrary region of the new
    // scan with nothing on screen to say so.
    let positions = grid(8);
    let handle = mesh(&positions);
    let mut markings = AlignMarkings::default();
    markings.dab(AlignSide::Moving, &handle, &dab_at(DVec3::ZERO, 3.0, false));

    assert!(markings
        .mask_for(AlignSide::Moving, handle.identity())
        .is_some());
    assert!(
        markings.mask_for(AlignSide::Moving, on(999)).is_none(),
        "a mask from a mesh of another size was handed out for this one"
    );
    assert!(
        markings.marked_fraction(on(999), on(0)).is_none(),
        "a mask from other geometry was counted into the coverage"
    );

    // The nastier half, and the one a vertex count cannot catch: a repair or a
    // sculpt can hand back a DIFFERENT mesh with the SAME number of vertices.
    // Checked by length alone, the old marks passed and then excluded an
    // arbitrary region of a surface nobody had painted.
    let same_size_other_mesh = MarkedOn {
        geometry: SUBJECT + 1,
        vertex_count: handle.vertex_count,
    };
    assert!(
        markings
            .mask_for(AlignSide::Moving, same_size_other_mesh)
            .is_none(),
        "identical vertex counts are not identical meshes"
    );
    assert!(
        markings.stale_for(AlignSide::Moving, same_size_other_mesh),
        "the operator has to be told their markings no longer apply"
    );
    assert!(
        !markings.stale_for(AlignSide::Moving, handle.identity()),
        "marks that still fit their own mesh are not stale"
    );
    assert!(
        !markings.stale_for(AlignSide::Fixed, same_size_other_mesh),
        "a side that was never marked has nothing to go stale"
    );
}

#[test]
fn a_stroke_closes_exactly_once() {
    // The measurement runs when the stroke closes. Reporting a close twice
    // re-measured a full arch for a button that was already up.
    let positions = grid(8);
    let handle = mesh(&positions);
    let mut markings = AlignMarkings::default();

    assert!(
        !markings.close_stroke(),
        "a stroke was open before anything was painted"
    );
    markings.dab(AlignSide::Moving, &handle, &dab_at(DVec3::ZERO, 2.0, false));
    assert!(
        markings.close_stroke(),
        "a dab did not open a stroke, so the release would never re-measure"
    );
    assert!(
        !markings.close_stroke(),
        "the stroke closed twice from one press"
    );
}

#[test]
fn clearing_drops_the_markings_on_both_scans_together() {
    // Half-cleared markings mean the fit ignores a region of one scan that the
    // operator can no longer see marked anywhere.
    let positions = grid(8);
    let handle = mesh(&positions);
    let mut markings = AlignMarkings::default();
    markings.dab(AlignSide::Moving, &handle, &dab_at(DVec3::ZERO, 2.0, false));
    markings.dab(AlignSide::Fixed, &handle, &dab_at(DVec3::ZERO, 2.0, false));

    assert!(markings.clear());

    assert!(markings
        .mask_for(AlignSide::Moving, handle.identity())
        .is_none());
    assert!(markings.mask_for(AlignSide::Fixed, on(16 * 16)).is_none());
    assert!(!markings.any());
    assert!(markings.marked_fraction(handle.identity(), on(0)).is_none());
}

#[test]
fn an_erase_reports_the_vertices_it_cleared() {
    let positions = grid(16);
    let handle = mesh(&positions);
    let mut markings = AlignMarkings::default();
    markings.dab(AlignSide::Moving, &handle, &dab_at(DVec3::ZERO, 4.0, false));

    let cleared = markings.dab(AlignSide::Moving, &handle, &dab_at(DVec3::ZERO, 2.0, true));

    assert!(cleared > 0, "the erase cleared nothing");
    assert_eq!(
        markings.touched().len(),
        cleared,
        "the touched list disagrees with the reported count"
    );
    assert!(
        !marked(&markings, AlignSide::Moving, 16 * 16, 8 * 16 + 8),
        "the vertex under the eraser is still marked"
    );
}

#[test]
fn the_commands_carry_exocads_own_labels() {
    // An operator who knows that dialog must not have to work out which of our
    // words means which of theirs.
    for (command, label) in [
        (MaskCommand::FitEverywhere, "Fit everywhere"),
        (MaskCommand::FitNowhere, "Fit nowhere"),
        (MaskCommand::InvertMarkings, "Invert markings"),
        (MaskCommand::MarkAutomatic, "Mark automatic"),
    ] {
        assert_eq!(command.label(), label);
        assert!(!command.hint().is_empty());
        assert!(!command.report().is_empty());
    }
}

#[test]
fn every_command_is_reachable_from_the_brush_window() {
    // The window builds its buttons from `ALL`. A command added to the enum and
    // forgotten there is a feature nobody can press.
    assert_eq!(MaskCommand::ALL.len(), 4);
    for command in MaskCommand::ALL {
        assert!(!command.label().is_empty());
    }
}
