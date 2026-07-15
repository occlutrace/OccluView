use super::{
    header, sample_stride, FormatError, Mesh, MeshBuilder, PlyFormat, Property, ScalarType, Vertex,
    FAST_POINT_TARGET,
};

pub(super) fn fast_ply_thumbnail_mesh(bytes: &[u8]) -> Result<Mesh, FormatError> {
    let parsed = header::parse(bytes)?;
    let Some(vertex_element) = parsed.elements.first() else {
        return Err(FormatError::Malformed {
            format: "PLY",
            offset: 0,
            reason: "header declares no elements".to_string(),
        });
    };
    if vertex_element.name != "vertex" {
        return Err(FormatError::Deferred {
            format: "PLY",
            reason: "fast thumbnail path requires vertex element first".to_string(),
        });
    }
    // A PLY that declares a `face` element is a SURFACE mesh. This fast reader
    // only ever emits a point cloud (it never walks the face list), so building
    // it here would splat a solid surface as a cloud of dots - exactly the
    // "half my thumbnails are just points" bug on real dental PLY scans, which
    // are routinely well above the fidelity gate. Decline surface PLYs so the
    // caller falls through to the full `occluview-formats` reader, which
    // triangulates the faces into a real surface. This is affordable: the full
    // reader parses a 40 MB / 2M-triangle binary PLY in ~0.49 s and the whole
    // thumbnail (parse + offscreen render) lands in ~0.59 s - an order of
    // magnitude under the 6 s thumbnail budget - so there is no latency reason
    // to trade a surface for a point splat. Genuine point clouds (no `face`
    // element) keep the O(target) decimated fast path below.
    if parsed.elements.iter().any(|element| element.name == "face") {
        return Err(FormatError::Deferred {
            format: "PLY",
            reason: "surface PLY (declares faces); defer to the full reader for a real surface \
                     thumbnail instead of a point splat"
                .to_string(),
        });
    }
    let layout = PlyVertexLayout::from_properties(&vertex_element.properties)?;
    let stride = sample_stride(vertex_element.count, FAST_POINT_TARGET);
    let mut builder = MeshBuilder::new()
        .with_name("PLY")
        .as_point_cloud()
        .reserve(vertex_element.count.div_ceil(stride), 0);
    match parsed.format {
        PlyFormat::Ascii => fast_ply_ascii_vertices(&parsed, &layout, stride, &mut builder)?,
        PlyFormat::BinaryLittleEndian => {
            fast_ply_binary_vertices(&parsed, &layout, stride, &mut builder, Endian::Little)?;
        }
        PlyFormat::BinaryBigEndian => {
            fast_ply_binary_vertices(&parsed, &layout, stride, &mut builder, Endian::Big)?;
        }
    }
    builder.build().map_err(FormatError::Core)
}

#[derive(Clone, Copy)]
struct PlyVertexLayout {
    row_size: usize,
    x: PlyScalarField,
    y: PlyScalarField,
    z: PlyScalarField,
    red: Option<PlyScalarField>,
    green: Option<PlyScalarField>,
    blue: Option<PlyScalarField>,
}

#[derive(Clone, Copy)]
struct PlyScalarField {
    offset: usize,
    property_index: usize,
    ty: ScalarType,
}

impl PlyVertexLayout {
    fn from_properties(properties: &[Property]) -> Result<Self, FormatError> {
        let mut row_size = 0usize;
        let mut x = None;
        let mut y = None;
        let mut z = None;
        let mut red = None;
        let mut green = None;
        let mut blue = None;
        for (property_index, property) in properties.iter().enumerate() {
            let Property::Scalar { name, ty } = property else {
                return Err(FormatError::Deferred {
                    format: "PLY",
                    reason: "fast thumbnail path requires scalar vertex properties".to_string(),
                });
            };
            let field = PlyScalarField {
                offset: row_size,
                property_index,
                ty: *ty,
            };
            match name.as_str() {
                "x" => x = Some(field),
                "y" => y = Some(field),
                "z" => z = Some(field),
                "red" => red = Some(field),
                "green" => green = Some(field),
                "blue" => blue = Some(field),
                _ => {}
            }
            row_size = row_size
                .checked_add(ty.byte_size())
                .ok_or(FormatError::Malformed {
                    format: "PLY",
                    offset: 0,
                    reason: "vertex row size overflowed".to_string(),
                })?;
        }
        Ok(Self {
            row_size,
            x: x.ok_or(FormatError::Malformed {
                format: "PLY",
                offset: 0,
                reason: "vertex properties missing x".to_string(),
            })?,
            y: y.ok_or(FormatError::Malformed {
                format: "PLY",
                offset: 0,
                reason: "vertex properties missing y".to_string(),
            })?,
            z: z.ok_or(FormatError::Malformed {
                format: "PLY",
                offset: 0,
                reason: "vertex properties missing z".to_string(),
            })?,
            red,
            green,
            blue,
        })
    }
}

fn fast_ply_ascii_vertices(
    parsed: &header::ParsedHeader<'_>,
    layout: &PlyVertexLayout,
    stride: usize,
    builder: &mut MeshBuilder,
) -> Result<(), FormatError> {
    let text = std::str::from_utf8(parsed.data).map_err(|_| FormatError::Malformed {
        format: "PLY",
        offset: 0,
        reason: "ASCII body is not valid UTF-8".to_string(),
    })?;
    let vertex_count = parsed.elements[0].count;
    for (index, line) in text.lines().take(vertex_count).enumerate() {
        if index % stride != 0 {
            continue;
        }
        let tokens: Vec<&str> = line.split_ascii_whitespace().collect();
        let mut vertex = Vertex::at(glam::Vec3::new(
            parse_ascii_scalar(tokens.get(layout.x.property_index).copied())?,
            parse_ascii_scalar(tokens.get(layout.y.property_index).copied())?,
            parse_ascii_scalar(tokens.get(layout.z.property_index).copied())?,
        ));
        if let Some(color) = parse_ply_ascii_color(tokens.as_slice(), layout)? {
            vertex = vertex.with_color(color);
        }
        let _ = builder.push_vertex(vertex);
    }
    Ok(())
}

fn parse_ascii_scalar(token: Option<&str>) -> Result<f32, FormatError> {
    let Some(token) = token else {
        return Err(FormatError::Malformed {
            format: "PLY",
            offset: 0,
            reason: "vertex line ended before required position fields".to_string(),
        });
    };
    token.parse::<f32>().map_err(|_| FormatError::Malformed {
        format: "PLY",
        offset: 0,
        reason: format!("bad numeric field {token:?}"),
    })
}

fn parse_ply_ascii_color(
    tokens: &[&str],
    layout: &PlyVertexLayout,
) -> Result<Option<[u8; 4]>, FormatError> {
    let Some(red) = layout.red else {
        return Ok(None);
    };
    let Some(green) = layout.green else {
        return Ok(None);
    };
    let Some(blue) = layout.blue else {
        return Ok(None);
    };
    let r = parse_ascii_color(tokens.get(red.property_index).copied())?;
    let g = parse_ascii_color(tokens.get(green.property_index).copied())?;
    let b = parse_ascii_color(tokens.get(blue.property_index).copied())?;
    Ok(Some([r, g, b, 255]))
}

fn parse_ascii_color(token: Option<&str>) -> Result<u8, FormatError> {
    let Some(token) = token else {
        return Err(FormatError::Malformed {
            format: "PLY",
            offset: 0,
            reason: "vertex line ended before color field".to_string(),
        });
    };
    if let Ok(value) = token.parse::<i32>() {
        return Ok(value.clamp(0, 255) as u8);
    }
    let value = token.parse::<f32>().map_err(|_| FormatError::Malformed {
        format: "PLY",
        offset: 0,
        reason: format!("bad color field {token:?}"),
    })?;
    Ok((value.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn fast_ply_binary_vertices(
    parsed: &header::ParsedHeader<'_>,
    layout: &PlyVertexLayout,
    stride: usize,
    builder: &mut MeshBuilder,
    endian: Endian,
) -> Result<(), FormatError> {
    let vertex_count = parsed.elements[0].count;
    let expected_len = layout
        .row_size
        .checked_mul(vertex_count)
        .ok_or(FormatError::Malformed {
            format: "PLY",
            offset: 0,
            reason: "vertex data length overflowed".to_string(),
        })?;
    if parsed.data.len() < expected_len {
        return Err(FormatError::Truncated {
            format: "PLY",
            expected: expected_len,
            got: parsed.data.len(),
        });
    }

    for index in (0..vertex_count).step_by(stride) {
        let start = index * layout.row_size;
        let row = &parsed.data[start..start + layout.row_size];
        let mut vertex = Vertex::at(glam::Vec3::new(
            read_scalar_f32(row, layout.x, endian)?,
            read_scalar_f32(row, layout.y, endian)?,
            read_scalar_f32(row, layout.z, endian)?,
        ));
        if let (Some(red), Some(green), Some(blue)) = (layout.red, layout.green, layout.blue) {
            vertex = vertex.with_color([
                read_scalar_u8(row, red, endian)?,
                read_scalar_u8(row, green, endian)?,
                read_scalar_u8(row, blue, endian)?,
                255,
            ]);
        }
        let _ = builder.push_vertex(vertex);
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Endian {
    Little,
    Big,
}

fn read_scalar_f32(row: &[u8], field: PlyScalarField, endian: Endian) -> Result<f32, FormatError> {
    let bytes = scalar_bytes(row, field)?;
    Ok(match field.ty {
        ScalarType::Char => i8::from_ne_bytes([bytes[0]]) as f32,
        ScalarType::Uchar => bytes[0] as f32,
        ScalarType::Short => match endian {
            Endian::Little => i16::from_le_bytes([bytes[0], bytes[1]]) as f32,
            Endian::Big => i16::from_be_bytes([bytes[0], bytes[1]]) as f32,
        },
        ScalarType::Ushort => match endian {
            Endian::Little => u16::from_le_bytes([bytes[0], bytes[1]]) as f32,
            Endian::Big => u16::from_be_bytes([bytes[0], bytes[1]]) as f32,
        },
        ScalarType::Int => match endian {
            Endian::Little => i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32,
            Endian::Big => i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32,
        },
        ScalarType::Uint => match endian {
            Endian::Little => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32,
            Endian::Big => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32,
        },
        ScalarType::Float => match endian {
            Endian::Little => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            Endian::Big => f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        },
        ScalarType::Double => match endian {
            Endian::Little => f64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]) as f32,
            Endian::Big => f64::from_be_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]) as f32,
        },
    })
}

fn read_scalar_u8(row: &[u8], field: PlyScalarField, endian: Endian) -> Result<u8, FormatError> {
    let value = read_scalar_f32(row, field, endian)?;
    Ok(value.round().clamp(0.0, 255.0) as u8)
}

fn scalar_bytes(row: &[u8], field: PlyScalarField) -> Result<&[u8], FormatError> {
    row.get(field.offset..field.offset + field.ty.byte_size())
        .ok_or(FormatError::Truncated {
            format: "PLY",
            expected: field.offset + field.ty.byte_size(),
            got: row.len(),
        })
}
