use glam::{DAffine3, DVec3};
use occlu_mesh_edit::{BridgeSplitReport, BridgeSplitRequest, MeshEditBuffers};
use occluview_robust_csg::{
    normalize_closed_mesh, prepare_robust_solid, split_prepared_with_separator_disc,
    validate_separator_clearance as validate_robust_separator_clearance, PreparedRobustSolid,
    RobustMesh, RobustMeshPart, SeparatorDisc,
};

use super::bridge_split_adapter::{
    conversion, part_name, validate_restored_finite_result, CoreBridgeSplitError,
    CoreBridgeSplitResult,
};
use super::{Mesh, Vertex};

pub(super) fn supports(source: &Mesh) -> bool {
    !source.has_vertex_colors() && !source.has_uvs() && source.texture().is_none()
}

pub(super) fn prepare(source: &Mesh) -> Result<PreparedRobustSolid, CoreBridgeSplitError> {
    prepare_robust_solid(&robust_local_mesh_from_source(source)).map_err(|error| {
        CoreBridgeSplitError::RobustCsg {
            reason: error.to_string(),
        }
    })
}

pub(super) fn split(
    prepared: &PreparedRobustSolid,
    source: &Mesh,
    affine: DAffine3,
    request: BridgeSplitRequest,
) -> Result<CoreBridgeSplitResult, CoreBridgeSplitError> {
    let robust_split = split_prepared_with_separator_disc(
        prepared,
        &affine_transform(affine),
        SeparatorDisc {
            center: request.center.as_dvec3().to_array(),
            normal: request.normal.as_dvec3().to_array(),
            kerf_mm: f64::from(request.kerf_mm),
            radius_mm: f64::from(request.disc_radius_mm),
        },
    )
    .map_err(|error| CoreBridgeSplitError::RobustCsg {
        reason: error.to_string(),
    })?;
    let direct = core_result_from_robust_parts(
        [&robust_split.part_a, &robust_split.part_b],
        [
            robust_split.report.part_a_cut_loops,
            robust_split.report.part_b_cut_loops,
        ],
        source,
        affine,
        request,
    );
    if !matches!(&direct, Err(CoreBridgeSplitError::Conversion { .. })) {
        return direct;
    }

    let inverse = affine.inverse();
    let part_a = stabilize_for_local_storage(robust_split.part_a, affine, inverse)?;
    let part_b = stabilize_for_local_storage(robust_split.part_b, affine, inverse)?;
    core_result_from_robust_parts(
        [&part_a, &part_b],
        [
            robust_split.report.part_a_cut_loops,
            robust_split.report.part_b_cut_loops,
        ],
        source,
        affine,
        request,
    )
}

pub(super) fn validate_stored_separator_clearance(
    parts: [&MeshEditBuffers; 2],
    transform: DAffine3,
    request: BridgeSplitRequest,
    tolerance_mm: f64,
) -> Result<(), CoreBridgeSplitError> {
    let disc = SeparatorDisc {
        center: request.center.as_dvec3().to_array(),
        normal: request.normal.as_dvec3().to_array(),
        kerf_mm: f64::from(request.kerf_mm),
        radius_mm: f64::from(request.disc_radius_mm),
    };
    for (label, part) in [("Part A", parts[0]), ("Part B", parts[1])] {
        let world_mesh = robust_world_mesh_from_buffers(part, transform)?;
        validate_robust_separator_clearance(&world_mesh, disc, tolerance_mm).map_err(|error| {
            conversion(format!(
                "restored {label} does not preserve finite separator clearance: {error}"
            ))
        })?;
    }
    Ok(())
}

fn core_result_from_robust_parts(
    parts: [&RobustMeshPart; 2],
    cut_loops: [usize; 2],
    source: &Mesh,
    affine: DAffine3,
    request: BridgeSplitRequest,
) -> Result<CoreBridgeSplitResult, CoreBridgeSplitError> {
    let inverse = affine.inverse();
    let part_a = mesh_from_robust_part(parts[0], source.name(), "Part A", inverse)?;
    let part_b = mesh_from_robust_part(parts[1], source.name(), "Part B", inverse)?;
    let positive_buffers = super::mesh_edit_buffers_from_mesh(&part_a);
    let negative_buffers = super::mesh_edit_buffers_from_mesh(&part_b);
    validate_restored_finite_result(&positive_buffers, &negative_buffers, affine, request)?;

    Ok(CoreBridgeSplitResult {
        report: BridgeSplitReport {
            input_triangles: source.triangle_count(),
            part_a_triangles: part_a.triangle_count(),
            part_b_triangles: part_b.triangle_count(),
            kerf_mm: request.kerf_mm,
            disc_radius_mm: request.disc_radius_mm,
            required_disc_radius_mm: request.disc_radius_mm,
            part_a_cut_loops: cut_loops[0],
            part_b_cut_loops: cut_loops[1],
        },
        part_a,
        part_b,
    })
}

fn stabilize_for_local_storage(
    part: RobustMeshPart,
    affine: DAffine3,
    inverse: DAffine3,
) -> Result<RobustMeshPart, CoreBridgeSplitError> {
    let positions = part
        .positions
        .into_iter()
        .map(|position| {
            let local = inverse.transform_point3(DVec3::from_array(position));
            let local = super::bridge_split_adapter::finite_vec3(
                local,
                "robust CSG local storage position",
            )?;
            let restored_world = affine.transform_point3(DVec3::from_array(local.map(f64::from)));
            if !restored_world.is_finite() {
                return Err(conversion(
                    "robust CSG local storage position became non-finite in world space"
                        .to_string(),
                ));
            }
            Ok(restored_world.to_array())
        })
        .collect::<Result<Vec<_>, CoreBridgeSplitError>>()?;
    normalize_closed_mesh(&RobustMesh {
        positions,
        indices: part.indices,
    })
    .map_err(|error| conversion(error.to_string()))
}

fn robust_local_mesh_from_source(source: &Mesh) -> RobustMesh {
    RobustMesh {
        positions: source
            .vertices()
            .iter()
            .map(|vertex| vertex.position.map(f64::from))
            .collect(),
        indices: source
            .indices()
            .iter()
            .map(|&index| u64::from(index))
            .collect(),
    }
}

fn robust_world_mesh_from_buffers(
    buffers: &MeshEditBuffers,
    transform: DAffine3,
) -> Result<RobustMesh, CoreBridgeSplitError> {
    let positions = buffers
        .vertices
        .iter()
        .map(|vertex| {
            let world =
                transform.transform_point3(DVec3::from_array(vertex.position.map(f64::from)));
            world
                .is_finite()
                .then_some(world.to_array())
                .ok_or_else(|| conversion("restored robust CSG position is non-finite".to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let indices = buffers
        .indices
        .iter()
        .map(|&index| u64::from(index))
        .collect();
    Ok(RobustMesh { positions, indices })
}

fn mesh_from_robust_part(
    part: &RobustMeshPart,
    source_name: Option<&str>,
    suffix: &str,
    inverse: DAffine3,
) -> Result<Mesh, CoreBridgeSplitError> {
    let vertices = part
        .positions
        .iter()
        .map(|&position| {
            let local_position = inverse.transform_point3(DVec3::from_array(position));
            Ok(Vertex {
                position: super::bridge_split_adapter::finite_vec3(
                    local_position,
                    "robust CSG local position",
                )?,
                normal: [0.0; 3],
                color: [255, 255, 255, 255],
                uv: [0.0, 0.0],
            })
        })
        .collect::<Result<Vec<_>, CoreBridgeSplitError>>()?;
    let indices = part
        .indices
        .iter()
        .map(|&index| {
            u32::try_from(index)
                .map_err(|_| conversion("robust CSG index exceeds u32::MAX".to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Mesh::new(Some(part_name(source_name, suffix)), vertices, indices)
        .map_err(CoreBridgeSplitError::from)
}

fn affine_transform(affine: DAffine3) -> [f64; 12] {
    let matrix = affine.matrix3;
    let translation = affine.translation;
    [
        matrix.x_axis.x,
        matrix.x_axis.y,
        matrix.x_axis.z,
        matrix.y_axis.x,
        matrix.y_axis.y,
        matrix.y_axis.z,
        matrix.z_axis.x,
        matrix.z_axis.y,
        matrix.z_axis.z,
        translation.x,
        translation.y,
        translation.z,
    ]
}
