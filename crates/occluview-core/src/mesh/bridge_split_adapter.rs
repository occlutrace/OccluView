use glam::{Affine3A, DAffine3, DMat3, DVec3, Vec3};
use occlu_mesh_edit::{
    fill_holes, repair_mesh, split_bridge, split_bridge_surface, validate_bridge_split,
    validate_bridge_split_part, validate_bridge_split_request, BridgeSplitError, BridgeSplitReport,
    BridgeSplitRequest, MeshEditBuffers, MeshEditOptions, RepairOptions, RepairReport,
};
#[cfg(feature = "robust-csg")]
use occluview_robust_csg::PreparedRobustSolid;
use std::sync::Arc;
use thiserror::Error;

use super::edit_adapter::{
    mesh_edit_buffers_from_mesh, mesh_from_edit_buffers_named_preserving_texture,
};
use super::Mesh;
use crate::CoreError;

const MAX_AUTOMATIC_IMPORT_RIM_EDGES: usize = 32;
const MAX_AUTOMATIC_IMPORT_RIM_PERIMETER_MM: f32 = 2.0;
const MIN_RESTORED_GAP_RELATIVE_TOLERANCE: f64 = 1.0e-3;
const MAX_RESTORED_GAP_RELATIVE_TOLERANCE: f64 = 1.0e-2;

/// Two local-space meshes produced by one world-space separator disc.
#[derive(Clone, Debug)]
pub struct CoreBridgeSplitResult {
    /// Positive-normal side, ready to replace the source layer.
    pub part_a: Mesh,
    /// Negative-normal side, ready to become the adjacent second layer.
    pub part_b: Mesh,
    /// Product-neutral operation statistics.
    pub report: BridgeSplitReport,
}

/// One cached Bridge Split source plus its optional native prepared solid.
///
/// Cloning this value is cheap: mesh data and native CSG state are shared by
/// `Arc`. Disc movement never rebuilds or mutates the source topology.
#[derive(Clone, Debug)]
pub struct PreparedBridgeSplitSource {
    source: Arc<Mesh>,
    #[cfg(feature = "robust-csg")]
    robust: Option<Arc<PreparedRobustSolid>>,
}

/// Core adaptation failures retain kernel causes for the app error surface.
#[derive(Debug, Error)]
pub enum CoreBridgeSplitError {
    /// Scene placement cannot be inverted or contains non-finite values.
    #[error("invalid layer transform: {reason}")]
    InvalidTransform {
        /// Stable explanation without mesh payload data.
        reason: String,
    },

    /// A coordinate could not be represented safely by the mesh buffers.
    #[error("bridge split transform conversion failed: {reason}")]
    Conversion {
        /// Stable explanation without mesh payload data.
        reason: String,
    },

    /// The product-neutral split kernel refused the operation.
    #[error(transparent)]
    Kernel(#[from] BridgeSplitError),

    /// Rebuilding a core mesh failed.
    #[error(transparent)]
    Core(#[from] CoreError),

    /// The optional robust CSG fallback could not produce two valid parts.
    #[error("bridge split robust CSG fallback failed: {reason}")]
    RobustCsg {
        /// Stable technical cause for diagnostics, without source geometry.
        reason: String,
    },
}

/// Prepare and cache all placement-independent Bridge Split work.
///
/// Plain CAD meshes first attempt lossless native multi-shell preparation, so
/// overlapping crown shells and valid detached solids are not mistaken for
/// disposable debris. Import normalization remains available for bounded
/// exporter residue and for attribute-preserving meshes.
///
/// # Errors
/// Returns [`CoreBridgeSplitError`] when neither lossless preparation nor the
/// bounded import-normalization policy can produce a valid source.
pub fn prepare_bridge_split_source(
    source: Arc<Mesh>,
) -> Result<PreparedBridgeSplitSource, CoreBridgeSplitError> {
    #[cfg(feature = "robust-csg")]
    if super::bridge_split_robust::supports(&source) {
        if let Ok(robust) = super::bridge_split_robust::prepare(&source) {
            return Ok(PreparedBridgeSplitSource {
                source,
                robust: Some(Arc::new(robust)),
            });
        }

        let source = match normalize_bridge_split_input_with_policy(&source, true) {
            Ok(normalized) => normalized.map_or(source, Arc::new),
            // An open surface is a valid renderable input for the explicit
            // surface-split fallback. Do not force it through the closed-solid
            // repair contract just to create a misleading preflight error.
            // Repair can also fail while inspecting importer residue. That is
            // still a reason to try the non-destructive surface path, not a
            // reason to make the whole tool unavailable.
            Err(CoreBridgeSplitError::Kernel(_) | CoreBridgeSplitError::Core(_)) => source,
            Err(error) => return Err(error),
        };
        let robust = super::bridge_split_robust::prepare(&source)
            .ok()
            .map(Arc::new);
        if robust.is_none() {
            let source = match normalized_source(Arc::clone(&source)) {
                Ok(normalized) => normalized,
                Err(CoreBridgeSplitError::Kernel(_) | CoreBridgeSplitError::Core(_)) => source,
                Err(error) => return Err(error),
            };
            return Ok(PreparedBridgeSplitSource {
                source,
                robust: None,
            });
        }
        return Ok(PreparedBridgeSplitSource { source, robust });
    }

    let source = match normalized_source(Arc::clone(&source)) {
        Ok(normalized) => normalized,
        Err(CoreBridgeSplitError::Kernel(_) | CoreBridgeSplitError::Core(_)) => source,
        Err(error) => return Err(error),
    };
    Ok(PreparedBridgeSplitSource {
        source,
        #[cfg(feature = "robust-csg")]
        robust: None,
    })
}

fn normalized_source(source: Arc<Mesh>) -> Result<Arc<Mesh>, CoreBridgeSplitError> {
    normalize_bridge_split_input(&source).map(|normalized| normalized.map_or(source, Arc::new))
}

/// Prepare an imported mesh for one Bridge Split session.
///
/// Healthy input returns `Ok(None)` and stays on the direct path. When the
/// shared repair pipeline can remove only bounded face-level importer residue,
/// this returns `Ok(Some(mesh))` containing a separate normalized copy. Closed
/// components are never removed by size. Structural surgery, unresolved rims,
/// and failed post-repair validation retain the original typed input failure
/// instead of silently changing the restoration.
///
/// # Errors
/// Returns [`CoreBridgeSplitError`] when the source is not a safely
/// normalizable closed bridge mesh.
pub fn normalize_bridge_split_input(source: &Mesh) -> Result<Option<Mesh>, CoreBridgeSplitError> {
    normalize_bridge_split_input_with_policy(source, false)
}

fn normalize_bridge_split_input_with_policy(
    source: &Mesh,
    allow_multiple_components: bool,
) -> Result<Option<Mesh>, CoreBridgeSplitError> {
    let buffers = mesh_edit_buffers_from_mesh(source);
    let request = input_preflight_request();
    let input_error = match validate_bridge_split(&buffers, request) {
        Ok(()) => return Ok(None),
        Err(error) if input_error_can_be_normalized(&error) => error,
        Err(error) => return Err(error.into()),
    };

    let repaired = repair_mesh(
        &buffers,
        RepairOptions {
            // Hole capping happens below with both an edge and mm limit.
            tiny_hole_max_edges: 1,
            exact_weld_only: true,
            // Bridge Split must never reinterpret a valid closed shell as
            // disposable debris. Explicit Repair remains the place for that
            // user-visible policy.
            debris_face_fraction: 0.0,
            debris_diameter_fraction: 0.0,
            ..RepairOptions::default()
        },
    )
    .map_err(|error| CoreBridgeSplitError::Core(CoreError::Geometry(error.to_string())))?;
    if !repair_report_is_safe_for_bridge_split(&repaired.report) {
        return Err(input_error.into());
    }
    let filled = fill_small_import_rims(&repaired.mesh)?;
    let valid = if allow_multiple_components {
        validate_bridge_split_request(request)
            .and_then(|()| validate_bridge_split_part(&filled).map(|_| ()))
    } else {
        validate_bridge_split(&filled, request)
    };
    if valid.is_err() {
        return Err(input_error.into());
    }

    mesh_from_edit_buffers_named_preserving_texture(
        source,
        filled,
        source.name().map(str::to_owned),
    )
    .map(Some)
    .map_err(Into::into)
}

/// Apply a circular world-space separator disc to one locally stored mesh.
///
/// Working coordinates are promoted to `f64`, transformed to world, and
/// recentered around the disc before narrowing to the kernel's `f32` buffers.
/// This preserves sub-millimetre kerfs for layers placed far from the origin.
/// Both results are transformed back to source-local coordinates, so callers
/// preserve the original [`Affine3A`] on their scene layers.
///
/// # Errors
/// Returns [`CoreBridgeSplitError`] for non-finite/singular transforms,
/// unrepresentable coordinates, kernel refusal, or invalid rebuilt meshes.
pub fn bridge_split_mesh_in_world(
    source: &Mesh,
    transform: Affine3A,
    request: BridgeSplitRequest,
) -> Result<CoreBridgeSplitResult, CoreBridgeSplitError> {
    validate_bridge_split_request(request)?;
    let prepared = prepare_bridge_split_source(Arc::new(source.clone()))?;
    bridge_split_prepared_mesh_in_world(&prepared, transform, request)
}

/// Apply one world-space separator placement to a cached prepared source.
///
/// # Errors
/// Returns [`CoreBridgeSplitError`] for invalid transforms, a missed or
/// ambiguous separator, invalid generated topology, or conversion loss.
pub fn bridge_split_prepared_mesh_in_world(
    prepared: &PreparedBridgeSplitSource,
    transform: Affine3A,
    request: BridgeSplitRequest,
) -> Result<CoreBridgeSplitResult, CoreBridgeSplitError> {
    let affine = validate_transform(transform)?;
    validate_bridge_split_request(request)?;
    let source = prepared.source.as_ref();

    #[cfg(feature = "robust-csg")]
    if let Some(robust) = prepared.robust.as_deref() {
        match super::bridge_split_robust::split(robust, source, affine, request) {
            Ok(result) => return Ok(result),
            Err(robust_error) => {
                // A successful direct result is finite-disc equivalent because
                // the clipper proves its complete slab radius fits the selected
                // disc before emitting geometry.
                return match bridge_split_mesh_fast(source, affine, request) {
                    Ok(result) => Ok(result),
                    Err(_) => {
                        bridge_split_mesh_surface(source, affine, request).map_err(|_| robust_error)
                    }
                };
            }
        }
    }

    match bridge_split_mesh_fast(source, affine, request) {
        Ok(result) => Ok(result),
        Err(fast_error) => {
            bridge_split_mesh_surface(source, affine, request).map_err(|_| fast_error)
        }
    }
}

fn bridge_split_mesh_fast(
    source: &Mesh,
    affine: DAffine3,
    request: BridgeSplitRequest,
) -> Result<CoreBridgeSplitResult, CoreBridgeSplitError> {
    let inverse = affine.inverse();
    let matrix = affine.matrix3;
    let reflected = matrix.determinant() < 0.0;
    let center = request.center.as_dvec3();
    let mut centered = mesh_edit_buffers_from_mesh(source);
    let normal_to_world = matrix.inverse().transpose();

    for vertex in &mut centered.vertices {
        let local_position = DVec3::from_array(vertex.position.map(f64::from));
        let relative_world = affine.transform_point3(local_position) - center;
        vertex.position = finite_vec3(relative_world, "world-relative position")?;
        vertex.normal = transform_normal(
            DVec3::from_array(vertex.normal.map(f64::from)),
            normal_to_world,
            "world normal",
        )?;
    }
    if reflected {
        reverse_triangle_winding(&mut centered.indices);
    }

    let centered_request = BridgeSplitRequest {
        center: Vec3::ZERO,
        normal: request.normal,
        kerf_mm: request.kerf_mm,
        disc_radius_mm: request.disc_radius_mm,
        max_disc_radius_mm: request.max_disc_radius_mm,
    };
    let split = split_bridge(&centered, centered_request)?;
    let mut part_a = restore_local_buffers(split.part_a, inverse, matrix, center)?;
    let mut part_b = restore_local_buffers(split.part_b, inverse, matrix, center)?;
    if reflected {
        reverse_triangle_winding(&mut part_a.indices);
        reverse_triangle_winding(&mut part_b.indices);
    }
    validate_restored_result(&part_a, &part_b, affine, request)?;

    let positive_name = part_name(source.name(), "Part A");
    let negative_name = part_name(source.name(), "Part B");
    Ok(CoreBridgeSplitResult {
        part_a: mesh_from_edit_buffers_named_preserving_texture(
            source,
            part_a,
            Some(positive_name),
        )?,
        part_b: mesh_from_edit_buffers_named_preserving_texture(
            source,
            part_b,
            Some(negative_name),
        )?,
        report: split.report,
    })
}

fn bridge_split_mesh_surface(
    source: &Mesh,
    affine: DAffine3,
    request: BridgeSplitRequest,
) -> Result<CoreBridgeSplitResult, CoreBridgeSplitError> {
    let inverse = affine.inverse();
    let matrix = affine.matrix3;
    let reflected = matrix.determinant() < 0.0;
    let center = request.center.as_dvec3();
    let mut centered = mesh_edit_buffers_from_mesh(source);
    let normal_to_world = matrix.inverse().transpose();

    for vertex in &mut centered.vertices {
        let local_position = DVec3::from_array(vertex.position.map(f64::from));
        let relative_world = affine.transform_point3(local_position) - center;
        vertex.position = finite_vec3(relative_world, "world-relative surface position")?;
        vertex.normal = transform_normal(
            DVec3::from_array(vertex.normal.map(f64::from)),
            normal_to_world,
            "world surface normal",
        )?;
    }
    if reflected {
        reverse_triangle_winding(&mut centered.indices);
    }

    let centered_request = BridgeSplitRequest {
        center: Vec3::ZERO,
        normal: request.normal,
        kerf_mm: request.kerf_mm,
        disc_radius_mm: request.disc_radius_mm,
        max_disc_radius_mm: request.max_disc_radius_mm,
    };
    let split = split_bridge_surface(&centered, centered_request)?;
    let mut part_a = restore_local_buffers(split.part_a, inverse, matrix, center)?;
    let mut part_b = restore_local_buffers(split.part_b, inverse, matrix, center)?;
    if reflected {
        reverse_triangle_winding(&mut part_a.indices);
        reverse_triangle_winding(&mut part_b.indices);
    }

    let center = request.center.as_dvec3();
    let normal = request.normal.as_dvec3().normalize();
    let positive_min = projected_world_min(&part_a, affine, center, normal);
    let negative_max = projected_world_max(&part_b, affine, center, normal);
    validate_restored_gap(
        [&part_a, &part_b],
        affine,
        normal,
        request,
        (positive_min, negative_max),
    )?;

    let positive_name = part_name(source.name(), "Part A");
    let negative_name = part_name(source.name(), "Part B");
    Ok(CoreBridgeSplitResult {
        part_a: mesh_from_edit_buffers_named_preserving_texture(
            source,
            part_a,
            Some(positive_name),
        )?,
        part_b: mesh_from_edit_buffers_named_preserving_texture(
            source,
            part_b,
            Some(negative_name),
        )?,
        report: split.report,
    })
}

fn input_error_can_be_normalized(error: &BridgeSplitError) -> bool {
    matches!(
        error,
        BridgeSplitError::DegenerateInput { .. }
            | BridgeSplitError::DisconnectedInput { .. }
            | BridgeSplitError::OpenOrNonManifold { .. }
    )
}

fn input_preflight_request() -> BridgeSplitRequest {
    BridgeSplitRequest {
        center: Vec3::ZERO,
        normal: Vec3::X,
        kerf_mm: 0.05,
        disc_radius_mm: 1.0,
        max_disc_radius_mm: 1.0,
    }
}

fn repair_report_is_safe_for_bridge_split(report: &RepairReport) -> bool {
    report.split_nonmanifold_edges == 0
        && report.split_bowtie_vertices == 0
        && report.removed_debris_components == 0
        && report.removed_debris_triangles == 0
        && report.warnings.is_empty()
}

fn fill_small_import_rims(mesh: &MeshEditBuffers) -> Result<MeshEditBuffers, CoreBridgeSplitError> {
    fill_holes(
        mesh,
        None,
        MeshEditOptions {
            max_boundary_loop: MAX_AUTOMATIC_IMPORT_RIM_EDGES,
            max_rim_perimeter_mm: Some(MAX_AUTOMATIC_IMPORT_RIM_PERIMETER_MM),
            protect_scan_border: false,
            ..MeshEditOptions::default()
        },
    )
    .map(|result| result.mesh)
    .map_err(|error| CoreBridgeSplitError::Core(CoreError::Geometry(error.to_string())))
}

fn validate_transform(transform: Affine3A) -> Result<DAffine3, CoreBridgeSplitError> {
    let matrix = DMat3::from_cols(
        transform.matrix3.x_axis.as_dvec3(),
        transform.matrix3.y_axis.as_dvec3(),
        transform.matrix3.z_axis.as_dvec3(),
    );
    let translation = transform.translation.as_dvec3();
    if !matrix.is_finite() || !translation.is_finite() {
        return Err(invalid_transform("transform contains non-finite values"));
    }
    let max_axis = matrix
        .x_axis
        .length()
        .max(matrix.y_axis.length())
        .max(matrix.z_axis.length());
    let determinant = matrix.determinant();
    if max_axis == 0.0 || !determinant.is_finite() || determinant == 0.0 {
        return Err(invalid_transform(
            "transform is singular or numerically degenerate",
        ));
    }
    let inverse_matrix = matrix.inverse();
    let max_inverse_axis = inverse_matrix
        .x_axis
        .length()
        .max(inverse_matrix.y_axis.length())
        .max(inverse_matrix.z_axis.length());
    let condition_estimate = max_axis * max_inverse_axis;
    if !inverse_matrix.is_finite() || !condition_estimate.is_finite() || condition_estimate >= 1.0e8
    {
        return Err(invalid_transform(
            "transform is too ill-conditioned for mesh editing",
        ));
    }
    Ok(DAffine3::from_mat3_translation(matrix, translation))
}

fn validate_restored_result(
    part_a: &MeshEditBuffers,
    part_b: &MeshEditBuffers,
    transform: DAffine3,
    request: BridgeSplitRequest,
) -> Result<(), CoreBridgeSplitError> {
    validate_restored_topology(part_a, part_b)?;

    let center = request.center.as_dvec3();
    let normal = request.normal.as_dvec3().normalize();
    let positive_min = projected_world_min(part_a, transform, center, normal);
    let negative_max = projected_world_max(part_b, transform, center, normal);
    validate_restored_gap(
        [part_a, part_b],
        transform,
        normal,
        request,
        (positive_min, negative_max),
    )
}

#[cfg(feature = "robust-csg")]
pub(super) fn validate_restored_finite_result(
    part_a: &MeshEditBuffers,
    part_b: &MeshEditBuffers,
    transform: DAffine3,
    request: BridgeSplitRequest,
) -> Result<(), CoreBridgeSplitError> {
    validate_restored_topology(part_a, part_b)?;
    let center = request.center.as_dvec3();
    let normal = request.normal.as_dvec3().normalize();
    let radial_margin =
        f64::from(request.disc_radius_mm) * 1.0e-5 + f64::from(request.kerf_mm) * 0.01;
    let positive_min = projected_world_extreme_in_disc(
        part_a,
        transform,
        center,
        normal,
        f64::from(request.disc_radius_mm) + radial_margin,
        f64::min,
        f64::INFINITY,
    );
    let negative_max = projected_world_extreme_in_disc(
        part_b,
        transform,
        center,
        normal,
        f64::from(request.disc_radius_mm) + radial_margin,
        f64::max,
        f64::NEG_INFINITY,
    );
    validate_restored_gap(
        [part_a, part_b],
        transform,
        normal,
        request,
        (positive_min, negative_max),
    )?;
    let tolerance = restored_gap_tolerance(
        part_a,
        part_b,
        transform.matrix3,
        normal,
        f64::from(request.kerf_mm),
    );
    super::bridge_split_robust::validate_stored_separator_clearance(
        [part_a, part_b],
        transform,
        request,
        tolerance,
    )
}

fn validate_restored_topology(
    part_a: &MeshEditBuffers,
    part_b: &MeshEditBuffers,
) -> Result<(), CoreBridgeSplitError> {
    validate_bridge_split_part(part_a).map_err(|error| {
        conversion(format!(
            "restored Part A is no longer a valid closed mesh: {error}"
        ))
    })?;
    validate_bridge_split_part(part_b).map_err(|error| {
        conversion(format!(
            "restored Part B is no longer a valid closed mesh: {error}"
        ))
    })?;
    Ok(())
}

fn validate_restored_gap(
    parts: [&MeshEditBuffers; 2],
    transform: DAffine3,
    normal: DVec3,
    request: BridgeSplitRequest,
    extents: (f64, f64),
) -> Result<(), CoreBridgeSplitError> {
    let [part_a, part_b] = parts;
    let (positive_min, negative_max) = extents;
    let observed_gap = positive_min - negative_max;
    let requested_gap = f64::from(request.kerf_mm);
    let tolerance =
        restored_gap_tolerance(part_a, part_b, transform.matrix3, normal, requested_gap);
    if !observed_gap.is_finite() || observed_gap + tolerance < requested_gap {
        return Err(conversion(
            "restoring local coordinates cannot preserve the requested separator gap".to_string(),
        ));
    }
    Ok(())
}

#[cfg(feature = "robust-csg")]
#[allow(clippy::too_many_arguments)]
fn projected_world_extreme_in_disc(
    mesh: &MeshEditBuffers,
    transform: DAffine3,
    center: DVec3,
    normal: DVec3,
    radius: f64,
    combine: fn(f64, f64) -> f64,
    initial: f64,
) -> f64 {
    let radius_sq = radius * radius;
    mesh.vertices
        .iter()
        .filter_map(|vertex| {
            let local = DVec3::from_array(vertex.position.map(f64::from));
            let relative = transform.transform_point3(local) - center;
            let projection = relative.dot(normal);
            let radial = relative - normal * projection;
            (radial.length_squared() <= radius_sq).then_some(projection)
        })
        .fold(initial, combine)
}

fn restored_gap_tolerance(
    part_a: &MeshEditBuffers,
    part_b: &MeshEditBuffers,
    world_from_local: DMat3,
    normal: DVec3,
    requested_gap: f64,
) -> f64 {
    let nominal = requested_gap * MIN_RESTORED_GAP_RELATIVE_TOLERANCE;
    let cap = requested_gap * MAX_RESTORED_GAP_RELATIVE_TOLERANCE;
    // `restore_local_buffers` narrows positions back to f32. Account for the
    // resulting world-space projection error, but cap the allowance so a gap
    // that is genuinely lost at huge local coordinates is still refused.
    nominal
        .max(restored_projection_resolution(
            part_a,
            part_b,
            world_from_local,
            normal,
        ))
        .min(cap)
}

fn restored_projection_resolution(
    part_a: &MeshEditBuffers,
    part_b: &MeshEditBuffers,
    world_from_local: DMat3,
    normal: DVec3,
) -> f64 {
    let projected_axes = [
        normal.dot(world_from_local.x_axis).abs(),
        normal.dot(world_from_local.y_axis).abs(),
        normal.dot(world_from_local.z_axis).abs(),
    ];
    let maximum_single_vertex_error = part_a
        .vertices
        .iter()
        .chain(&part_b.vertices)
        .map(|vertex| {
            vertex
                .position
                .into_iter()
                .zip(projected_axes)
                .map(|(coordinate, projected_axis)| {
                    let local_resolution =
                        f64::from(coordinate.abs().max(f32::MIN_POSITIVE) * f32::EPSILON);
                    local_resolution * projected_axis
                })
                .sum::<f64>()
        })
        .fold(0.0_f64, f64::max);
    // Each half of the separator can round towards the other.
    maximum_single_vertex_error * 2.0
}

fn projected_world_min(
    mesh: &MeshEditBuffers,
    transform: DAffine3,
    center: DVec3,
    normal: DVec3,
) -> f64 {
    mesh.vertices
        .iter()
        .map(|vertex| {
            let local = DVec3::from_array(vertex.position.map(f64::from));
            (transform.transform_point3(local) - center).dot(normal)
        })
        .fold(f64::INFINITY, f64::min)
}

fn projected_world_max(
    mesh: &MeshEditBuffers,
    transform: DAffine3,
    center: DVec3,
    normal: DVec3,
) -> f64 {
    mesh.vertices
        .iter()
        .map(|vertex| {
            let local = DVec3::from_array(vertex.position.map(f64::from));
            (transform.transform_point3(local) - center).dot(normal)
        })
        .fold(f64::NEG_INFINITY, f64::max)
}

fn restore_local_buffers(
    mut buffers: MeshEditBuffers,
    inverse: DAffine3,
    source_matrix: DMat3,
    center: DVec3,
) -> Result<MeshEditBuffers, CoreBridgeSplitError> {
    let normal_to_local = source_matrix.transpose();
    for vertex in &mut buffers.vertices {
        let relative_world = DVec3::from_array(vertex.position.map(f64::from));
        let local_position = inverse.transform_point3(relative_world + center);
        vertex.position = finite_vec3(local_position, "restored local position")?;
        vertex.normal = transform_normal(
            DVec3::from_array(vertex.normal.map(f64::from)),
            normal_to_local,
            "restored local normal",
        )?;
    }
    Ok(buffers)
}

fn transform_normal(
    normal: DVec3,
    matrix: DMat3,
    label: &str,
) -> Result<[f32; 3], CoreBridgeSplitError> {
    if normal.length_squared() <= f64::EPSILON {
        return Ok([0.0; 3]);
    }
    let transformed = matrix * normal;
    if !transformed.is_finite() || transformed.length_squared() == 0.0 {
        return Err(conversion(format!("{label} is degenerate")));
    }
    finite_vec3(transformed.normalize(), label)
}

pub(super) fn finite_vec3(value: DVec3, label: &str) -> Result<[f32; 3], CoreBridgeSplitError> {
    if !value.is_finite() || value.abs().max_element() > f64::from(f32::MAX) {
        return Err(conversion(format!("{label} is not representable as f32")));
    }
    Ok(value.as_vec3().to_array())
}

fn reverse_triangle_winding(indices: &mut [u32]) {
    for triangle in indices.chunks_exact_mut(3) {
        triangle.swap(1, 2);
    }
}

pub(super) fn part_name(source_name: Option<&str>, suffix: &str) -> String {
    source_name.map_or_else(|| suffix.to_string(), |name| format!("{name} - {suffix}"))
}

fn invalid_transform(reason: &str) -> CoreBridgeSplitError {
    CoreBridgeSplitError::InvalidTransform {
        reason: reason.to_string(),
    }
}

pub(super) fn conversion(reason: String) -> CoreBridgeSplitError {
    CoreBridgeSplitError::Conversion { reason }
}
