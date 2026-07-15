use super::{
    sample_stride, FormatError, Mesh, MeshBuilder, Ordering, RobustBoundsSampler,
    SurfaceGridCluster, Vec3, Vertex, FAST_CLUSTER_GRID, FAST_POINT_TARGET,
    ROBUST_BOUNDS_SAMPLE_LIMIT,
};

pub(super) fn fast_obj_thumbnail_mesh(bytes: &[u8]) -> Result<Mesh, FormatError> {
    let text = String::from_utf8_lossy(bytes);
    let (total_vertices, face_records) = obj_fast_counts(&text);
    if total_vertices == 0 {
        return Err(FormatError::Malformed {
            format: "OBJ",
            offset: 0,
            reason: "file declares no vertices".to_string(),
        });
    }

    let mut source_vertices = Vec::with_capacity(total_vertices);
    for line in text.lines() {
        let Some((tag, mut tokens)) = obj_line_tokens(line) else {
            continue;
        };
        if tag != "v" {
            continue;
        }
        let vertex = obj_vertex_from_tokens(&mut tokens).ok_or(FormatError::Malformed {
            format: "OBJ",
            offset: source_vertices.len(),
            reason: "malformed vertex line".to_string(),
        })?;
        source_vertices.push(vertex);
    }

    if face_records == 0 {
        return sampled_obj_point_cloud(&source_vertices);
    }

    // Contiguous decimation: triangulate EVERY face and weld its corners onto a
    // coarse grid so the reduced surface stays solid. Never stride faces — that
    // is the see-through speckle the old fast path produced on dense scans.
    let (min, max) = obj_robust_bounds(&source_vertices);
    let mut cluster = SurfaceGridCluster::new("OBJ", min, max, FAST_CLUSTER_GRID);
    let mut seen_faces = 0usize;

    for line in text.lines() {
        let Some((tag, mut tokens)) = obj_line_tokens(line) else {
            continue;
        };
        if tag != "f" {
            continue;
        }

        let indices =
            obj_face_indices(&mut tokens, source_vertices.len()).ok_or(FormatError::Malformed {
                format: "OBJ",
                offset: seen_faces,
                reason: "malformed face line".to_string(),
            })?;
        if indices.len() >= 3 {
            for triangle in 1..indices.len() - 1 {
                cluster.push_triangle(
                    source_vertices[indices[0]],
                    source_vertices[indices[triangle]],
                    source_vertices[indices[triangle + 1]],
                );
            }
        }
        seen_faces += 1;
    }

    // Zero-triangle safety net: the file declared faces (face_records > 0 to
    // reach here) but none survived clustering (all degenerate / non-finite).
    // Building would yield a triangle mesh with 0 triangles that renders as a
    // fully transparent tile; splatting the raw vertices as a point cloud would
    // misrepresent a surface file as dots. Decline instead so the loader falls
    // through to the full reader (mirroring how the surface-PLY fast path
    // declines); loading.rs's renderability guard covers the case where the
    // full reader is also blank.
    if cluster.triangle_count() == 0 {
        return Err(FormatError::Deferred {
            format: "OBJ",
            reason: "clustered fast surrogate produced no surface; defer to the full reader"
                .to_string(),
        });
    }

    cluster.build()
}

/// Outlier-robust bounds for grid sizing, over a deterministic strided sample of
/// the parsed source vertices. Non-finite positions are skipped and a few far
/// outliers are trimmed (see [`RobustBoundsSampler`]) so they cannot inflate the
/// grid and collapse the model into a couple of cells.
fn obj_robust_bounds(source_vertices: &[Vertex]) -> (Vec3, Vec3) {
    let stride = sample_stride(source_vertices.len(), ROBUST_BOUNDS_SAMPLE_LIMIT);
    let mut sampler = RobustBoundsSampler::with_capacity(ROBUST_BOUNDS_SAMPLE_LIMIT);
    for vertex in source_vertices.iter().step_by(stride) {
        sampler.push(vertex.position);
    }
    // `SurfaceGridCluster::new` tolerates an empty/degenerate box (zero-extent
    // axes get a zero scale), so fall back to a point box when every sampled
    // position was non-finite; such a mesh emits no triangles and declines above.
    sampler.finish().unwrap_or((Vec3::ZERO, Vec3::ZERO))
}

fn obj_fast_counts(text: &str) -> (usize, usize) {
    let mut vertices = 0usize;
    let mut faces = 0usize;
    for line in text.lines() {
        let Some((tag, _)) = obj_line_tokens(line) else {
            continue;
        };
        match tag {
            "v" => vertices += 1,
            "f" => faces += 1,
            _ => {}
        }
    }
    (vertices, faces)
}

fn obj_line_tokens(line: &str) -> Option<(&str, impl Iterator<Item = &str>)> {
    let line = line
        .trim_start_matches('\u{feff}')
        .split('#')
        .next()
        .unwrap_or(line)
        .trim();
    if line.is_empty() {
        return None;
    }
    let mut tokens = line.split_ascii_whitespace();
    let tag = tokens.next()?;
    Some((tag, tokens))
}

fn obj_face_indices<'a>(
    tokens: &mut impl Iterator<Item = &'a str>,
    vertex_count: usize,
) -> Option<Vec<usize>> {
    let mut indices = Vec::new();
    for token in tokens {
        let head = token.split('/').next()?;
        let raw = head.parse::<i32>().ok()?;
        let index = match raw.cmp(&0) {
            Ordering::Greater => usize::try_from(raw - 1).ok()?,
            Ordering::Less => {
                let from_end = usize::try_from(-raw).ok()?;
                vertex_count.checked_sub(from_end)?
            }
            Ordering::Equal => return None,
        };
        if index >= vertex_count {
            return None;
        }
        indices.push(index);
    }
    Some(indices)
}

fn sampled_obj_point_cloud(source_vertices: &[Vertex]) -> Result<Mesh, FormatError> {
    let stride = sample_stride(source_vertices.len(), FAST_POINT_TARGET);
    let mut builder = MeshBuilder::new()
        .with_name("OBJ")
        .as_point_cloud()
        .reserve(source_vertices.len().div_ceil(stride), 0);

    for (index, vertex) in source_vertices.iter().copied().enumerate() {
        if index % stride == 0 {
            let _ = builder.push_vertex(vertex);
        }
    }

    builder.build().map_err(FormatError::Core)
}

fn obj_vertex_from_tokens<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Option<Vertex> {
    let x = tokens.next()?.parse::<f32>().ok()?;
    let y = tokens.next()?.parse::<f32>().ok()?;
    let z = tokens.next()?.parse::<f32>().ok()?;
    let mut vertex = Vertex::at(Vec3::new(x, y, z));
    let r = tokens.next();
    let g = tokens.next();
    let b = tokens.next();
    if let (Some(r), Some(g), Some(b)) = (r, g, b) {
        vertex = vertex.with_color([
            parse_obj_color_channel(r)?,
            parse_obj_color_channel(g)?,
            parse_obj_color_channel(b)?,
            255,
        ]);
    }
    Some(vertex)
}

fn parse_obj_color_channel(token: &str) -> Option<u8> {
    if let Ok(value) = token.parse::<i32>() {
        return Some(value.clamp(0, 255) as u8);
    }
    let value = token.parse::<f32>().ok()?;
    Some((value.clamp(0.0, 1.0) * 255.0).round() as u8)
}
