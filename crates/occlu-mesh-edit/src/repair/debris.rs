//! Pass 7: debris removal.
//!
//! A component is debris only when it is BOTH relatively tiny in face count
//! (vs the largest component) AND relatively tiny in extent (vs the whole
//! mesh bounding-box diagonal). The largest component is never dropped, so a
//! two-jaw scan where both halves are large keeps both.

use glam::Vec3;

use super::{Components, RepairOptions, RepairReport};
use crate::MeshEditBuffers;

pub(super) fn remove_debris(
    mesh: &mut MeshEditBuffers,
    components: &Components,
    options: &RepairOptions,
    report: &mut RepairReport,
) {
    if components.members.len() <= 1 {
        return;
    }

    let mut largest = 0;
    for (component, members) in components.members.iter().enumerate().skip(1) {
        if members.len() > components.members[largest].len() {
            largest = component;
        }
    }
    let face_limit =
        f64::from(options.debris_face_fraction) * count_f64(components.members[largest].len());
    let diameter_limit =
        f64::from(options.debris_diameter_fraction) * f64::from(referenced_diagonal(mesh));

    let mut drop = vec![false; components.members.len()];
    let mut dropped_any = false;
    for (component, members) in components.members.iter().enumerate() {
        if component == largest {
            continue;
        }
        let small_count = count_f64(members.len()) < face_limit;
        let small_extent = f64::from(triangles_diagonal(mesh, members)) < diameter_limit;
        if small_count && small_extent {
            drop[component] = true;
            dropped_any = true;
            report.removed_debris_components += 1;
            report.removed_debris_triangles += members.len();
        }
    }
    if !dropped_any {
        return;
    }

    let mut kept = Vec::with_capacity(mesh.indices.len());
    for (triangle, tri) in mesh.indices.chunks_exact(3).enumerate() {
        if !drop[components.component_of[triangle]] {
            kept.extend_from_slice(tri);
        }
    }
    mesh.indices = kept;
}

/// Bounding-box diagonal over the vertices referenced by `triangles`.
fn triangles_diagonal(mesh: &MeshEditBuffers, triangles: &[usize]) -> f32 {
    diagonal_of(
        triangles
            .iter()
            .flat_map(|&triangle| &mesh.indices[triangle * 3..triangle * 3 + 3])
            .map(|&index| mesh.vertices[index as usize].position),
    )
}

/// Bounding-box diagonal over every vertex any triangle references.
fn referenced_diagonal(mesh: &MeshEditBuffers) -> f32 {
    diagonal_of(
        mesh.indices
            .iter()
            .map(|&index| mesh.vertices[index as usize].position),
    )
}

fn diagonal_of(positions: impl Iterator<Item = [f32; 3]>) -> f32 {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    for position in positions {
        let position = Vec3::from_array(position);
        min = min.min(position);
        max = max.max(position);
        any = true;
    }
    if any {
        (max - min).length()
    } else {
        0.0
    }
}

/// Face counts fit `f64` exactly for any real mesh; the cast is deliberate.
#[allow(clippy::cast_precision_loss)]
fn count_f64(count: usize) -> f64 {
    count as f64
}
