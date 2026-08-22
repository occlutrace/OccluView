use crate::error::FormatError;
use glam::Vec3;
use occluview_core::{Mesh, MeshTexture, Vertex};
use occluview_hps::{DecodedSurface, DecodedSurfaceParts};
use std::collections::HashMap;

pub(super) fn build_mesh(surface: DecodedSurface) -> Result<Mesh, FormatError> {
    let DecodedSurfaceParts {
        positions,
        indices,
        colors,
        uvs,
        corner_uvs,
        normals,
        texture,
    } = surface.into_parts();
    let positions: Vec<Vec3> = positions.into_iter().map(Vec3::from_array).collect();
    let normals = resolve_normals(&positions, &indices, normals.as_deref());

    let (vertices, render_indices) = if let Some(corner_uvs) = corner_uvs.as_deref() {
        build_corner_split_vertices(
            &positions,
            &indices,
            colors.as_deref(),
            &normals,
            corner_uvs,
        )?
    } else {
        let vertices =
            build_source_vertices(&positions, colors.as_deref(), &normals, uvs.as_deref())?;
        (vertices, indices)
    };

    mesh_from_parts(vertices, render_indices, texture)
}

/// Build the source-topology mesh used by CAD exports.
///
/// This deliberately ignores render-only corner UVs and texture images. The
/// returned indices point at the original source positions, so downstream
/// editing and geometry writers see the connected surface from the HPS file.
pub(super) fn build_geometry_mesh(surface: &DecodedSurface) -> Result<Mesh, FormatError> {
    let positions: Vec<Vec3> = surface
        .positions()
        .iter()
        .copied()
        .map(Vec3::from_array)
        .collect();
    let indices = surface.indices().to_vec();
    let normals = resolve_normals(&positions, &indices, surface.normals());
    let vertices = build_source_vertices(&positions, surface.colors(), &normals, None)?;
    Mesh::new(Some("HPS".to_string()), vertices, indices).map_err(FormatError::Core)
}

fn build_source_vertices(
    positions: &[Vec3],
    colors: Option<&[[u8; 4]]>,
    normals: &[Vec3],
    uvs: Option<&[[f32; 2]]>,
) -> Result<Vec<Vertex>, FormatError> {
    let mut vertices = Vec::with_capacity(positions.len());
    for (index, &position) in positions.iter().enumerate() {
        let normal = normals
            .get(index)
            .ok_or_else(|| invalid_surface("normal index"))?;
        let mut vertex = Vertex::at(position).with_normal(*normal);
        if let Some(colors) = colors {
            vertex = vertex.with_color(
                *colors
                    .get(index)
                    .ok_or_else(|| invalid_surface("color index"))?,
            );
        }
        if let Some(uvs) = uvs {
            vertex = vertex.with_uv(*uvs.get(index).ok_or_else(|| invalid_surface("UV index"))?);
        }
        vertices.push(vertex);
    }
    Ok(vertices)
}

fn build_corner_split_vertices(
    positions: &[Vec3],
    indices: &[u32],
    colors: Option<&[[u8; 4]]>,
    normals: &[Vec3],
    corner_uvs: &[Option<[f32; 2]>],
) -> Result<(Vec<Vertex>, Vec<u32>), FormatError> {
    if corner_uvs.len() != indices.len() {
        return Err(invalid_surface("corner UV count"));
    }

    let mut vertices = Vec::with_capacity(positions.len());
    let mut render_indices = Vec::with_capacity(indices.len());
    let mut lookup = HashMap::<(u32, u32, u32), u32>::new();

    for (corner, &source_index) in indices.iter().enumerate() {
        let source_index_usize =
            usize::try_from(source_index).map_err(|_| invalid_surface("corner vertex index"))?;
        let position = *positions
            .get(source_index_usize)
            .ok_or_else(|| invalid_surface("corner vertex index"))?;
        let normal = *normals
            .get(source_index_usize)
            .ok_or_else(|| invalid_surface("corner normal index"))?;
        let uv = corner_uvs[corner].unwrap_or([0.0, 0.0]);
        let key = (
            source_index,
            canonical_float_bits(uv[0]),
            canonical_float_bits(uv[1]),
        );

        let render_index = if let Some(&existing) = lookup.get(&key) {
            existing
        } else {
            let mut vertex = Vertex::at(position).with_normal(normal).with_uv(uv);
            if let Some(colors) = colors {
                vertex = vertex.with_color(
                    *colors
                        .get(source_index_usize)
                        .ok_or_else(|| invalid_surface("corner color index"))?,
                );
            }
            let render_index = u32::try_from(vertices.len())
                .map_err(|_| invalid_surface("render vertex count"))?;
            vertices.push(vertex);
            lookup.insert(key, render_index);
            render_index
        };
        render_indices.push(render_index);
    }

    Ok((vertices, render_indices))
}

fn mesh_from_parts(
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    texture: Option<occluview_hps::DecodedTexture>,
) -> Result<Mesh, FormatError> {
    let mut mesh =
        Mesh::new(Some("HPS".to_string()), vertices, indices).map_err(FormatError::Core)?;
    if let Some(texture) = texture {
        let (width, height, rgba) = texture.into_parts();
        mesh.set_texture(MeshTexture::new(width, height, rgba));
    }
    Ok(mesh)
}

fn resolve_normals(positions: &[Vec3], indices: &[u32], normals: Option<&[[f32; 3]]>) -> Vec<Vec3> {
    normals.map_or_else(
        || smooth_normals(positions, indices),
        |normals| normals.iter().copied().map(Vec3::from_array).collect(),
    )
}

fn canonical_float_bits(value: f32) -> u32 {
    if value == 0.0 {
        0.0_f32.to_bits()
    } else {
        value.to_bits()
    }
}

fn invalid_surface(attribute: &'static str) -> FormatError {
    FormatError::Malformed {
        format: "HPS",
        offset: 0,
        reason: format!("validated surface has an invalid {attribute}"),
    }
}

const DEGENERATE_AREA_SIN: f32 = 1e-10;

fn smooth_normals(positions: &[Vec3], indices: &[u32]) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for triangle in indices.chunks_exact(3) {
        let index_a = triangle[0] as usize;
        let index_b = triangle[1] as usize;
        let index_c = triangle[2] as usize;
        let (Some(&a), Some(&b), Some(&c)) = (
            positions.get(index_a),
            positions.get(index_b),
            positions.get(index_c),
        ) else {
            continue;
        };
        let face_normal = (b - a).cross(c - a);
        let longest_edge_sq = (b - a)
            .length_squared()
            .max((c - b).length_squared())
            .max((a - c).length_squared());
        if face_normal.is_finite()
            && face_normal.length_squared()
                > longest_edge_sq * longest_edge_sq * DEGENERATE_AREA_SIN
        {
            normals[index_a] += face_normal;
            normals[index_b] += face_normal;
            normals[index_c] += face_normal;
        }
    }
    for normal in &mut normals {
        *normal = if normal.length_squared() > f32::EPSILON {
            normal.normalize()
        } else {
            Vec3::Z
        };
    }
    normals
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{build_geometry_mesh, build_mesh};
    use occluview_hps::{DecodedSurface, DecodedTexture};

    #[test]
    fn neutral_surface_adapter_preserves_geometry_attributes_and_texture() {
        let texture = DecodedTexture::new(1, 1, vec![9, 8, 7, 255]).expect("valid texture");
        let surface = DecodedSurface::new(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![0, 1, 2],
            Some(vec![[1, 2, 3, 255], [4, 5, 6, 255], [7, 8, 9, 255]]),
            Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
            Some(texture),
        )
        .expect("valid neutral surface")
        .with_normals(vec![[1.0, 0.0, 0.0]; 3])
        .expect("valid neutral normals");

        let mesh = build_mesh(surface).expect("surface should adapt");

        assert_eq!(mesh.indices(), &[0, 1, 2]);
        assert_eq!(mesh.vertices()[1].position, [1.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices()[2].color, [7, 8, 9, 255]);
        assert_eq!(mesh.vertices()[2].uv, [0.0, 1.0]);
        assert_eq!(mesh.vertices()[0].normal, [1.0, 0.0, 0.0]);
        let texture = mesh.texture().expect("mesh texture should be present");
        assert_eq!((texture.width, texture.height), (1, 1));
        assert_eq!(texture.rgba, [9, 8, 7, 255]);
    }

    #[test]
    fn corner_uv_render_split_does_not_change_source_geometry_topology() {
        let texture = DecodedTexture::new(1, 1, vec![255, 255, 255, 255]).expect("texture");
        let surface = DecodedSurface::new(
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            vec![0, 1, 2, 0, 2, 3],
            None,
            None,
            Some(texture),
        )
        .expect("surface")
        .with_corner_uvs(vec![
            Some([0.0, 0.0]),
            Some([1.0, 0.0]),
            Some([1.0, 1.0]),
            Some([0.0, 0.0]),
            Some([1.0, 1.0]),
            Some([0.0, 1.0]),
        ])
        .expect("corner UVs")
        .with_normals(vec![[0.0, 0.0, 1.0]; 4])
        .expect("normals");

        let geometry = build_geometry_mesh(&surface).expect("geometry mesh");
        assert_eq!(geometry.vertices().len(), 4);
        assert_eq!(geometry.indices(), &[0, 1, 2, 0, 2, 3]);

        let render = build_mesh(surface).expect("render mesh");
        assert_eq!(render.vertices().len(), 4);
        assert_eq!(render.indices(), &[0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn render_split_duplicates_only_a_real_uv_seam() {
        let surface = DecodedSurface::new(
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            vec![0, 1, 2, 0, 2, 3],
            None,
            None,
            None,
        )
        .expect("surface")
        .with_corner_uvs(vec![
            Some([0.0, 0.0]),
            Some([1.0, 0.0]),
            Some([1.0, 1.0]),
            Some([0.25, 0.0]),
            Some([1.0, 1.0]),
            Some([0.0, 1.0]),
        ])
        .expect("corner UVs");

        let render = build_mesh(surface).expect("render mesh");
        assert_eq!(render.vertices().len(), 5);
        assert_eq!(render.indices(), &[0, 1, 2, 3, 2, 4]);
    }
}
