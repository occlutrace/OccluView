use std::collections::BTreeMap;

use glam::DVec3;
use manifold_csg::Manifold;

use crate::native_mesh::{extract_part, manifold_from_mesh};
use crate::{
    invalid_input, kernel_error, position_bounds, prepare_robust_solid, PreparedRobustSolid,
    RobustCsgError, RobustMesh, RobustMeshPart, RobustSplit, RobustSplitReport, SeparatorDisc,
};

const MIN_CYLINDER_SEGMENTS: i32 = 96;
const MAX_CYLINDER_SEGMENTS: i32 = 512;
const CUTTER_CHORD_TOLERANCE_MM: f64 = 0.005;

struct PhysicalComponent {
    part: RobustMeshPart,
    ordering_key: [f64; 6],
    cut_loops: usize,
}

/// Split a prepared local-space solid at one placement without rebuilding its
/// shell union.
///
/// # Errors
/// Returns [`RobustCsgError`] for invalid placement, missed or ambiguous cuts,
/// native failures, or a result that cannot be grouped into two logical sides.
pub fn split_prepared_with_separator_disc(
    prepared: &PreparedRobustSolid,
    transform: &[f64; 12],
    disc: SeparatorDisc,
) -> Result<RobustSplit, RobustCsgError> {
    validate_disc(disc)?;
    let sources = prepared.transformed_manifolds(transform)?;
    let cutter = separator_cylinder(disc);
    let mut positive = Vec::new();
    let mut negative = Vec::new();
    let mut removed_any = false;
    let mut cut_has_positive = false;
    let mut cut_has_negative = false;
    let mut component_index = 0;
    for source in sources {
        let overlap = source.intersection(&cutter);
        overlap.status().map_err(kernel_error)?;
        let overlap_volume = overlap.volume();
        if !overlap_volume.is_finite() {
            return Err(kernel_error("separator overlap volume is non-finite"));
        }
        let source_was_cut = !overlap.is_empty() && overlap_volume > 0.0;
        let result = if source_was_cut {
            removed_any = true;
            let difference = source.difference(&cutter);
            difference.status().map_err(kernel_error)?;
            difference
        } else {
            source
        };
        for manifold in result.decompose() {
            manifold.status().map_err(kernel_error)?;
            let part = extract_part(manifold)?;
            let (positive_loops, negative_loops, side) = if source_was_cut {
                let positive_loops = cap_loop_count(&part, disc, 1.0);
                let negative_loops = cap_loop_count(&part, disc, -1.0);
                let side = classify_cut_component(
                    &part,
                    disc,
                    positive_loops,
                    negative_loops,
                    component_index,
                )?;
                (positive_loops, negative_loops, side)
            } else {
                (0, 0, classify_untouched_component(&part, disc))
            };
            let physical = PhysicalComponent {
                ordering_key: component_ordering_key(&part),
                part,
                cut_loops: positive_loops + negative_loops,
            };
            match side {
                LogicalSide::Positive => {
                    cut_has_positive |= source_was_cut;
                    positive.push(physical);
                }
                LogicalSide::Negative => {
                    cut_has_negative |= source_was_cut;
                    negative.push(physical);
                }
            }
            component_index += 1;
        }
    }
    if !removed_any {
        return Err(RobustCsgError::SeparatorMiss);
    }
    if !cut_has_positive || !cut_has_negative || positive.is_empty() || negative.is_empty() {
        return Err(RobustCsgError::UnexpectedComponents {
            components: positive.len() + negative.len(),
        });
    }

    positive.sort_by(physical_ordering);
    negative.sort_by(physical_ordering);
    let report = RobustSplitReport {
        part_a_physical_components: positive.len(),
        part_b_physical_components: negative.len(),
        part_a_cut_loops: positive.iter().map(|component| component.cut_loops).sum(),
        part_b_cut_loops: negative.iter().map(|component| component.cut_loops).sum(),
    };
    Ok(RobustSplit {
        part_a: compose_physical_parts(&positive)?,
        part_b: compose_physical_parts(&negative)?,
        report,
    })
}

/// Split one raw mesh with an identity placement.
///
/// This compatibility wrapper performs preparation for each call. Interactive
/// callers should cache [`PreparedRobustSolid`] and call
/// [`split_prepared_with_separator_disc`].
///
/// # Errors
/// Returns [`RobustCsgError`] when preparation or splitting fails.
pub fn split_with_separator_disc(
    mesh: &RobustMesh,
    disc: SeparatorDisc,
) -> Result<RobustSplit, RobustCsgError> {
    let prepared = prepare_robust_solid(mesh)?;
    split_prepared_with_separator_disc(&prepared, &identity_transform(), disc)
}

/// Rebuild a closed mesh after storage-position quantization.
///
/// # Errors
/// Returns [`RobustCsgError`] when the quantized mesh is no longer a valid
/// closed manifold.
pub fn normalize_closed_mesh(mesh: &RobustMesh) -> Result<RobustMeshPart, RobustCsgError> {
    let manifold = manifold_from_mesh(mesh)?;
    extract_part(manifold)
}

/// Verify that a closed result mesh does not enter the finite separator body.
///
/// `tolerance_mm` contracts the tested body on every boundary so storage
/// quantization and exact cap contact are not mistaken for material overlap.
///
/// # Errors
/// Returns [`RobustCsgError`] for malformed input or measurable overlap with
/// the contracted separator volume.
pub fn validate_separator_clearance(
    mesh: &RobustMesh,
    disc: SeparatorDisc,
    tolerance_mm: f64,
) -> Result<(), RobustCsgError> {
    validate_disc(disc)?;
    if !tolerance_mm.is_finite()
        || tolerance_mm < 0.0
        || tolerance_mm * 2.0 >= disc.kerf_mm
        || tolerance_mm >= disc.radius_mm
    {
        return Err(invalid_input(
            "separator clearance tolerance must fit inside the disc",
        ));
    }

    let result = manifold_from_mesh(mesh)?;
    let contracted = separator_cylinder(SeparatorDisc {
        kerf_mm: disc.kerf_mm - tolerance_mm * 2.0,
        radius_mm: disc.radius_mm - tolerance_mm,
        ..disc
    });
    let overlap = result.intersection(&contracted);
    overlap.status().map_err(kernel_error)?;
    let overlap_volume = overlap.volume();
    if !overlap_volume.is_finite() || !overlap.is_empty() {
        return Err(RobustCsgError::SeparatorClearanceLost);
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum LogicalSide {
    Positive,
    Negative,
}

fn classify_cut_component(
    part: &RobustMeshPart,
    disc: SeparatorDisc,
    positive_loops: usize,
    negative_loops: usize,
    component_index: usize,
) -> Result<LogicalSide, RobustCsgError> {
    match (positive_loops > 0, negative_loops > 0) {
        (true, false) => return Ok(LogicalSide::Positive),
        (false, true) => return Ok(LogicalSide::Negative),
        (true, true) => {
            return Err(RobustCsgError::KerfSpanningComponent {
                component: component_index,
            });
        }
        (false, false) => {}
    }

    // A cap-free component from an affected native solid is accepted only
    // when it lies wholly on one side. Spanning the kerf after subtraction is
    // an invalid result, even if cap provenance could not be recovered.
    let (minimum, maximum) = projection_extent(part, disc);
    let half_kerf = disc.kerf_mm * 0.5;
    let tolerance = separator_extent_tolerance(disc);
    if minimum >= half_kerf - tolerance {
        Ok(LogicalSide::Positive)
    } else if maximum <= -half_kerf + tolerance {
        Ok(LogicalSide::Negative)
    } else {
        Err(RobustCsgError::KerfSpanningComponent {
            component: component_index,
        })
    }
}

fn classify_untouched_component(part: &RobustMeshPart, disc: SeparatorDisc) -> LogicalSide {
    let (minimum, maximum) = projection_extent(part, disc);
    // The finite cutter already proved zero intersection with this physical
    // component. Preserve it intact on the side containing the midpoint of
    // its normal-axis extent; an exact tie is stable and goes to Part A.
    if minimum + maximum >= 0.0 {
        LogicalSide::Positive
    } else {
        LogicalSide::Negative
    }
}

fn compose_physical_parts(
    components: &[PhysicalComponent],
) -> Result<RobustMeshPart, RobustCsgError> {
    let position_count = components.iter().try_fold(0_usize, |count, component| {
        count
            .checked_add(component.part.positions.len())
            .ok_or_else(|| invalid_input("logical part vertex count overflow"))
    })?;
    let index_count = components.iter().try_fold(0_usize, |count, component| {
        count
            .checked_add(component.part.indices.len())
            .ok_or_else(|| invalid_input("logical part index count overflow"))
    })?;
    let mut positions = Vec::with_capacity(position_count);
    let mut indices = Vec::with_capacity(index_count);
    for component in components {
        let base = u64::try_from(positions.len())
            .map_err(|_| invalid_input("logical part vertex count exceeds u64::MAX"))?;
        positions.extend_from_slice(&component.part.positions);
        for &index in &component.part.indices {
            indices.push(
                index
                    .checked_add(base)
                    .ok_or_else(|| invalid_input("logical part index overflow"))?,
            );
        }
    }
    Ok(RobustMeshPart { positions, indices })
}

fn cap_loop_count(part: &RobustMeshPart, disc: SeparatorDisc, side: f64) -> usize {
    let center = DVec3::from_array(disc.center);
    let normal = DVec3::from_array(disc.normal).normalize();
    let cap = side * disc.kerf_mm * 0.5;
    let tolerance = separator_extent_tolerance(disc);
    let mut cap_triangles = Vec::new();
    for (triangle_index, triangle) in part.indices.chunks_exact(3).enumerate() {
        let Some(vertices) = triangle
            .iter()
            .map(|&index| {
                usize::try_from(index)
                    .ok()
                    .and_then(|index| part.positions.get(index))
            })
            .collect::<Option<Vec<_>>>()
        else {
            return 0;
        };
        let on_cap = vertices.iter().all(|&&position| {
            let offset = DVec3::from_array(position) - center;
            let axial = offset.dot(normal);
            let radial = offset - normal * axial;
            (axial - cap).abs() <= tolerance && radial.length() <= disc.radius_mm + tolerance
        });
        if on_cap {
            cap_triangles.push(triangle_index);
        }
    }
    if cap_triangles.is_empty() {
        return 0;
    }

    let mut edge_counts: BTreeMap<(u64, u64), usize> = BTreeMap::new();
    for triangle_index in cap_triangles {
        let triangle = &part.indices[triangle_index * 3..triangle_index * 3 + 3];
        for (from, to) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            let edge = if from < to { (from, to) } else { (to, from) };
            *edge_counts.entry(edge).or_default() += 1;
        }
    }
    let mut adjacency: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for ((from, to), count) in edge_counts {
        if count == 1 {
            adjacency.entry(from).or_default().push(to);
            adjacency.entry(to).or_default().push(from);
        }
    }
    if adjacency.is_empty() {
        return 1;
    }

    let mut loops = 0;
    let mut visited = BTreeMap::new();
    for &vertex in adjacency.keys() {
        if visited.contains_key(&vertex) {
            continue;
        }
        loops += 1;
        let mut stack = vec![vertex];
        while let Some(current) = stack.pop() {
            if visited.insert(current, true).is_some() {
                continue;
            }
            if let Some(neighbors) = adjacency.get(&current) {
                stack.extend(neighbors.iter().copied());
            }
        }
    }
    loops
}

fn projection_extent(part: &RobustMeshPart, disc: SeparatorDisc) -> (f64, f64) {
    let center = DVec3::from_array(disc.center);
    let normal = DVec3::from_array(disc.normal).normalize();
    part.positions
        .iter()
        .map(|&position| (DVec3::from_array(position) - center).dot(normal))
        .fold(
            (f64::INFINITY, f64::NEG_INFINITY),
            |(minimum, maximum), value| (minimum.min(value), maximum.max(value)),
        )
}

fn component_ordering_key(part: &RobustMeshPart) -> [f64; 6] {
    let (minimum, maximum) = position_bounds(&part.positions);
    [
        minimum[0], minimum[1], minimum[2], maximum[0], maximum[1], maximum[2],
    ]
}

fn physical_ordering(left: &PhysicalComponent, right: &PhysicalComponent) -> std::cmp::Ordering {
    left.ordering_key
        .iter()
        .zip(right.ordering_key.iter())
        .map(|(left, right)| left.total_cmp(right))
        .find(|ordering| *ordering != std::cmp::Ordering::Equal)
        .unwrap_or_else(|| left.part.indices.len().cmp(&right.part.indices.len()))
}

fn validate_disc(disc: SeparatorDisc) -> Result<(), RobustCsgError> {
    let normal = DVec3::from_array(disc.normal);
    let normal_length = normal.length();
    if !DVec3::from_array(disc.center).is_finite()
        || !normal.is_finite()
        || !normal_length.is_finite()
        || normal_length <= f64::EPSILON
        || !disc.kerf_mm.is_finite()
        || disc.kerf_mm <= 0.0
        || !disc.radius_mm.is_finite()
        || disc.radius_mm <= 0.0
    {
        return Err(invalid_input(
            "separator geometry must be finite and non-zero",
        ));
    }
    Ok(())
}

fn separator_cylinder(disc: SeparatorDisc) -> Manifold {
    let normal = DVec3::from_array(disc.normal).normalize();
    let seed = if normal.x.abs() < 0.9 {
        DVec3::X
    } else {
        DVec3::Y
    };
    let tangent = (seed - normal * seed.dot(normal)).normalize();
    let bitangent = normal.cross(tangent);
    let center = DVec3::from_array(disc.center);
    Manifold::cylinder(
        disc.kerf_mm,
        disc.radius_mm,
        disc.radius_mm,
        cylinder_segments(disc.radius_mm),
        true,
    )
    .transform(&[
        tangent.x,
        tangent.y,
        tangent.z,
        bitangent.x,
        bitangent.y,
        bitangent.z,
        normal.x,
        normal.y,
        normal.z,
        center.x,
        center.y,
        center.z,
    ])
}

fn cylinder_segments(radius_mm: f64) -> i32 {
    let ratio = (1.0 - CUTTER_CHORD_TOLERANCE_MM / radius_mm).clamp(-1.0, 1.0);
    let segments = (std::f64::consts::PI / ratio.acos()).ceil();
    #[allow(clippy::cast_possible_truncation)]
    let segments = segments as i32;
    segments.clamp(MIN_CYLINDER_SEGMENTS, MAX_CYLINDER_SEGMENTS)
}

fn separator_extent_tolerance(disc: SeparatorDisc) -> f64 {
    let center = DVec3::from_array(disc.center);
    1.0e-8
        * disc
            .radius_mm
            .max(disc.kerf_mm)
            .max(1.0)
            .max(center.abs().max_element() * 1.0e-12)
}

pub(crate) fn identity_transform() -> [f64; 12] {
    [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]
}

#[cfg(test)]
#[path = "disc_split_tests.rs"]
mod tests;
