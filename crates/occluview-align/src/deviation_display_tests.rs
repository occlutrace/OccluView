//! Tests for [`crate::deviation_display`].
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use super::display_map;
use crate::{DeviationMap, Soup, Validity};

const SIDE: usize = 24;

/// A `SIDE` x `SIDE` grid of 0.1 mm cells on z = 0, as an indexed mesh.
fn grid() -> (Vec<f32>, Vec<u32>) {
    let mut positions = Vec::new();
    for y in 0..SIDE {
        for x in 0..SIDE {
            positions.extend_from_slice(&[x as f32 * 0.1, y as f32 * 0.1, 0.0]);
        }
    }
    let mut indices = Vec::new();
    for y in 0..SIDE - 1 {
        for x in 0..SIDE - 1 {
            let a = (y * SIDE + x) as u32;
            let (b, c, d) = (a + 1, a + SIDE as u32, a + SIDE as u32 + 1);
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    (positions, indices)
}

/// The same grid as an STL holds it: every triangle owns its three corners.
fn soup_of(positions: &[f32], indices: &[u32]) -> (Vec<f32>, Vec<u32>) {
    let mut corners = Vec::new();
    for &index in indices {
        let at = index as usize * 3;
        corners.extend_from_slice(&positions[at..at + 3]);
    }
    let order = (0..indices.len() as u32).collect();
    (corners, order)
}

/// A reproducible value in `-1..=1` for each vertex.
fn jitter(vertex: usize) -> f32 {
    let mut state = (vertex as u32)
        .wrapping_mul(2_654_435_761)
        .wrapping_add(12_345);
    state ^= state >> 15;
    state = state.wrapping_mul(2_246_822_519);
    state ^= state >> 13;
    (state % 2001) as f32 / 1000.0 - 1.0
}

fn measured(values: Vec<f32>) -> DeviationMap {
    let validity = vec![Validity::Measured; values.len()];
    DeviationMap {
        signed_mm: values,
        validity,
    }
}

fn soup<'a>(positions: &'a [f32], indices: &'a [u32]) -> Soup<'a> {
    Soup {
        positions,
        indices,
        mask: None,
    }
}

fn rms(values: impl Iterator<Item = f32>) -> f32 {
    let (sum, count) = values.fold((0.0_f64, 0_usize), |(sum, count), value| {
        (sum + f64::from(value).powi(2), count + 1)
    });
    (sum / count as f64).sqrt() as f32
}

#[test]
fn single_vertex_noise_is_calmed_and_a_slope_survives() {
    let (positions, indices) = grid();
    // A systematic 0.002 mm per cell slope under 0.02 mm of noise.
    let truth = |vertex: usize| (vertex % SIDE) as f32 * 0.002;
    let raw = measured(
        (0..SIDE * SIDE)
            .map(|vertex| truth(vertex) + 0.02 * jitter(vertex))
            .collect(),
    );
    let shown = display_map(&raw, soup(&positions, &indices));

    // Away from the border, where every vertex has its full ring of neighbours.
    let interior = |vertex: &usize| {
        let (x, y) = (vertex % SIDE, vertex / SIDE);
        (4..SIDE - 4).contains(&x) && (4..SIDE - 4).contains(&y)
    };
    let error = |map: &DeviationMap| {
        rms((0..SIDE * SIDE)
            .filter(interior)
            .map(|vertex| map.signed_mm[vertex] - truth(vertex)))
    };
    assert!(
        error(&shown) < error(&raw) / 2.5,
        "noise {} was only calmed to {}",
        error(&raw),
        error(&shown)
    );
    assert_eq!(shown.validity, raw.validity);
}

#[test]
fn a_soup_is_smoothed_like_the_welded_mesh() {
    let (positions, indices) = grid();
    let (corner_positions, corner_indices) = soup_of(&positions, &indices);
    let by_vertex: Vec<f32> = (0..SIDE * SIDE).map(jitter).collect();
    let welded = display_map(&measured(by_vertex.clone()), soup(&positions, &indices));
    // Each corner carries the value of the grid vertex it was cut from.
    let by_corner: Vec<f32> = indices
        .iter()
        .map(|&index| by_vertex[index as usize])
        .collect();
    let exploded = display_map(
        &measured(by_corner),
        soup(&corner_positions, &corner_indices),
    );
    for (corner, &index) in indices.iter().enumerate() {
        assert!(
            (exploded.signed_mm[corner] - welded.signed_mm[index as usize]).abs() < 1e-6,
            "corner {corner} of vertex {index} disagrees with the welded mesh"
        );
    }
}

#[test]
fn what_was_not_measured_neither_changes_nor_leaks() {
    let (positions, indices) = grid();
    let mut raw = measured(vec![0.01; SIDE * SIDE]);
    // A hole in the measurement, with a wild number behind it.
    let hole =
        |vertex: usize| (8..12).contains(&(vertex % SIDE)) && (8..12).contains(&(vertex / SIDE));
    for vertex in (0..SIDE * SIDE).filter(|&vertex| hole(vertex)) {
        raw.validity[vertex] = Validity::OutOfReach;
        raw.signed_mm[vertex] = 50.0;
    }
    let shown = display_map(&raw, soup(&positions, &indices));
    for vertex in 0..SIDE * SIDE {
        if hole(vertex) {
            assert_eq!(
                shown.signed_mm[vertex], 50.0,
                "an unmeasured value is left alone"
            );
        } else {
            assert!(
                (shown.signed_mm[vertex] - 0.01).abs() < 1e-6,
                "vertex {vertex} was pulled toward the hole: {}",
                shown.signed_mm[vertex]
            );
        }
    }
}

#[test]
fn a_map_out_of_step_with_its_mesh_is_returned_as_it_is() {
    let (positions, indices) = grid();
    let raw = measured(vec![0.5; 3]);
    assert_eq!(display_map(&raw, soup(&positions, &indices)), raw);
}
