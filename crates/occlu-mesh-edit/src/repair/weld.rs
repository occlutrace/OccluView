//! Pass 1: weld duplicate vertices.
//!
//! STL-style soup stores each triangle's corners separately; welding restores
//! shared connectivity. The anti-weld doctrine still holds: dental formats
//! duplicate a position with different colors/UVs ON PURPOSE, so the weld key
//! includes color and UV bits — only full-attribute matches merge. The
//! representative is the lowest-original-index member and survivors adopt its
//! exact bits (no averaging, no vertex ever moves).

use glam::Vec3;

use super::RepairReport;
use crate::{EditVertex, MeshEditBuffers, MeshEditError};

/// Position + color + UV bit patterns. Equal keys weld.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum WeldKey {
    Exact([u32; 3], [u8; 4], [u32; 2]),
    Quantized([i32; 3], [u8; 4], [u32; 2]),
}

/// Weld vertices whose position and full attribute payload match.
///
/// Sort-based grouping (serial): key every vertex, sort, scan equal-key runs.
/// The default quantization step is `weld_epsilon` capped at `1e-3` of the
/// mesh bounding-box diagonal. `exact_only` instead requires bit-identical
/// positions (with signed zero normalized), avoiding speculative crack repair.
pub(super) fn weld_duplicate_vertices(
    mesh: &mut MeshEditBuffers,
    weld_epsilon: f32,
    exact_only: bool,
    report: &mut RepairReport,
) -> Result<(), MeshEditError> {
    let vertex_count = mesh.vertices.len();
    if vertex_count == 0 {
        return Ok(());
    }
    let epsilon = effective_epsilon(&mesh.vertices, weld_epsilon);

    let mut keyed: Vec<(WeldKey, usize)> = mesh
        .vertices
        .iter()
        .enumerate()
        .map(|(index, vertex)| (weld_key(vertex, epsilon, exact_only), index))
        .collect();
    keyed.sort_unstable();

    // Equal-key runs; the run head (lowest original index — the index is the
    // sort tiebreaker) is the representative every member maps onto.
    let mut representative_of: Vec<usize> = (0..vertex_count).collect();
    let mut run_start = 0;
    for scan in 1..=keyed.len() {
        if scan != keyed.len() && keyed[scan].0 == keyed[run_start].0 {
            continue;
        }
        if scan - run_start > 1 {
            let representative = keyed[run_start].1;
            for entry in &keyed[run_start..scan] {
                representative_of[entry.1] = representative;
            }
        }
        run_start = scan;
    }

    let welded = representative_of
        .iter()
        .enumerate()
        .filter(|&(index, &representative)| index != representative)
        .count();
    if welded == 0 {
        return Ok(());
    }

    // Compact: representatives keep their relative order; duplicates vanish.
    let mut new_index = vec![0_u32; vertex_count];
    let mut kept: Vec<EditVertex> = Vec::with_capacity(vertex_count - welded);
    for (index, &representative) in representative_of.iter().enumerate() {
        if representative == index {
            new_index[index] =
                u32::try_from(kept.len()).map_err(|_| MeshEditError::MalformedMesh {
                    reason: "welded vertex count exceeds u32::MAX".to_string(),
                })?;
            kept.push(mesh.vertices[index]);
        }
    }
    for index in &mut mesh.indices {
        *index = new_index[representative_of[*index as usize]];
    }
    mesh.vertices = kept;
    report.welded_vertices += welded;
    Ok(())
}

/// `weld_epsilon` capped at `1e-3` of the bounding-box diagonal, so a huge
/// caller epsilon cannot merge distinct anatomy on a small mesh.
fn effective_epsilon(vertices: &[EditVertex], weld_epsilon: f32) -> f32 {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for vertex in vertices {
        let position = Vec3::from_array(vertex.position);
        min = min.min(position);
        max = max.max(position);
    }
    let scaled = (max - min).length() * 1e-3;
    if scaled.is_finite() && scaled > 0.0 {
        weld_epsilon.min(scaled)
    } else {
        weld_epsilon
    }
}

fn weld_key(vertex: &EditVertex, epsilon: f32, exact_only: bool) -> WeldKey {
    let payload = (
        vertex.color,
        [vertex.uv[0].to_bits(), vertex.uv[1].to_bits()],
    );
    if exact_only {
        return WeldKey::Exact(
            vertex.position.map(exact_position_key),
            payload.0,
            payload.1,
        );
    }
    WeldKey::Quantized(
        [
            lane_key(vertex.position[0], epsilon),
            lane_key(vertex.position[1], epsilon),
            lane_key(vertex.position[2], epsilon),
        ],
        payload.0,
        payload.1,
    )
}

fn exact_position_key(value: f32) -> u32 {
    if value == 0.0 {
        0
    } else {
        value.to_bits()
    }
}

/// One quantized position lane (the `position_lane_key` scheme with a
/// caller-chosen step).
#[allow(clippy::cast_possible_truncation)]
fn lane_key(value: f32, epsilon: f32) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let scaled = (f64::from(value) / f64::from(epsilon)).round();
    if scaled <= f64::from(i32::MIN) {
        i32::MIN
    } else if scaled >= f64::from(i32::MAX) {
        i32::MAX
    } else {
        scaled as i32
    }
}
