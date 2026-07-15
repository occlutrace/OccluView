use glam::DVec3;
use manifold_csg::Manifold;

use crate::{
    flatten_positions, invalid_input, kernel_error, RobustCsgError, RobustMesh, RobustMeshPart,
};

pub(crate) fn validate_mesh(mesh: &RobustMesh) -> Result<(), RobustCsgError> {
    if mesh.positions.is_empty() || mesh.indices.is_empty() || mesh.indices.len() % 3 != 0 {
        return Err(invalid_input("mesh must contain indexed triangles"));
    }
    if mesh
        .positions
        .iter()
        .any(|&position| !DVec3::from_array(position).is_finite())
    {
        return Err(invalid_input("mesh positions must be finite"));
    }
    let vertex_count = u64::try_from(mesh.positions.len())
        .map_err(|_| invalid_input("vertex count exceeds u64::MAX"))?;
    if mesh.indices.iter().any(|&index| index >= vertex_count) {
        return Err(invalid_input("mesh index is out of range"));
    }
    Ok(())
}

pub(crate) fn manifold_from_mesh(mesh: &RobustMesh) -> Result<Manifold, RobustCsgError> {
    validate_mesh(mesh)?;
    Manifold::from_mesh_f64(&flatten_positions(&mesh.positions), 3, &mesh.indices)
        .map_err(kernel_error)
}

pub(crate) fn manifold_from_closed_shell(
    mesh: &RobustMesh,
    minimum: [f64; 3],
    maximum: [f64; 3],
) -> Result<Manifold, RobustCsgError> {
    let extent = DVec3::from_array(maximum) - DVec3::from_array(minimum);
    let scale = extent.max_element();
    if !scale.is_finite() || scale <= 0.0 {
        return Err(invalid_input("closed shell has invalid bounds"));
    }
    if scale >= 1.0e-6 {
        return manifold_from_mesh(mesh);
    }

    let center = (DVec3::from_array(minimum) + DVec3::from_array(maximum)) * 0.5;
    let normalized = RobustMesh {
        positions: mesh
            .positions
            .iter()
            .map(|&position| ((DVec3::from_array(position) - center) / scale).to_array())
            .collect(),
        indices: mesh.indices.clone(),
    };
    let manifold = manifold_from_mesh(&normalized)?.transform(&[
        scale, 0.0, 0.0, 0.0, scale, 0.0, 0.0, 0.0, scale, center.x, center.y, center.z,
    ]);
    manifold.status().map_err(kernel_error)?;
    Ok(manifold)
}

pub(crate) fn extract_part(manifold: Manifold) -> Result<RobustMeshPart, RobustCsgError> {
    let (properties, property_count, indices) = manifold.to_mesh_f64();
    if property_count < 3 || properties.len() % property_count != 0 {
        return Err(RobustCsgError::Kernel {
            reason: "native CSG returned malformed vertex properties".to_string(),
        });
    }
    let mut positions = Vec::with_capacity(properties.len() / property_count);
    for properties in properties.chunks_exact(property_count) {
        let position = [properties[0], properties[1], properties[2]];
        if !DVec3::from_array(position).is_finite() {
            return Err(RobustCsgError::Kernel {
                reason: "native CSG returned non-finite geometry".to_string(),
            });
        }
        positions.push(position);
    }
    Ok(RobustMeshPart { positions, indices })
}
