//! PLY reader.
//!
//! PLY (Polygon File Format) is the dental format for **color / NIR scans**:
//! unlike STL it supports per-vertex properties — most importantly `red green
//! blue` vertex colors. Both ASCII and binary (little- or big-endian) variants
//! exist; the header declares which.
//!
//! ## Layout
//!
//! ```text
//! ply
//! format ascii 1.0          # or: binary_little_endian 1.0 / binary_big_endian 1.0
//! comment ...
//! element vertex <N>
//!   property float x        # property <type> <name>
//!   property float y
//!   property float z
//!   property uchar red      # colors are usually 8-bit
//!   property uchar green
//!   property uchar blue
//!   ...
//! element face <M>
//!   property list uchar int vertex_indices
//! end_header
//! <data ...>
//! ```
//!
//! ## Dental-scanner quirks tolerated
//!
//! - Property order/format varies wildly across scanners — parse strictly from
//!   the header, never hard-code.
//! - Mixed endianness; honor the declared binary variant.
//! - Some scanners add non-standard properties (`confidence`, `nx ny nz`,
//!   `alpha`) — read and ignore unknown ones gracefully.
//! - Units sometimes declared in a `comment obj_info` line.

pub mod ascii;
pub mod binary;
pub(crate) mod embed;
pub mod header;

use crate::error::FormatError;
use occluview_core::{Mesh, MeshBuilder, MeshTexture};

/// Texture coordinates gathered from a face element's `texcoord` list.
///
/// PLY's working texture convention stores `u,v` per face corner (`CloudCompare`,
/// Agisoft, `MeshLab` and the intraoral scanners all write it). The list arrives
/// with the faces, after the vertices, so it is gathered here in the order the
/// reader pushes triangles and applied to the builder before the mesh is
/// finalized.
///
/// A vertex whose corners carry different coordinates sits on a texture seam.
/// The builder copies such a vertex once per coordinate, which is how every
/// OBJ and glTF importer carries a seam: the surface keeps its exact shape and
/// only the vertex table grows. A scanner's atlas is cut into many charts, so
/// that growth is large (an intraoral arch grows by half), and it only pays
/// when there is a picture to draw. The coordinates are therefore kept only
/// when one exists; without it the file reads as the vertex-coloured scan it
/// also is.
#[derive(Default)]
pub(crate) struct FaceUvs {
    /// Whether a picture exists for the coordinates to address.
    keep: bool,
    /// One coordinate per triangle corner, in the order triangles were pushed.
    corners: Vec<[f32; 2]>,
}

impl FaceUvs {
    pub(crate) fn new(keep: bool) -> Self {
        Self {
            keep,
            corners: Vec::new(),
        }
    }

    /// Record the coordinates of one polygon's corners.
    ///
    /// The polygon is fanned from its first corner, exactly as the readers
    /// triangulate it, so the list stays in step with the builder's indices.
    pub(crate) fn face(&mut self, corners: &[u32], coords: &[f32]) -> Result<(), FormatError> {
        if corners.len().checked_mul(2) != Some(coords.len()) {
            return Err(FormatError::Malformed {
                format: "PLY",
                offset: 0,
                reason: "face texcoord list must contain exactly two coordinates per corner"
                    .to_string(),
            });
        }
        if !self.keep || corners.len() < 3 {
            return Ok(());
        }
        if let Some((first, rest)) = coords.as_chunks::<2>().0.split_first() {
            for pair in rest.windows(2) {
                self.corners.extend_from_slice(&[*first, pair[0], pair[1]]);
            }
        }
        Ok(())
    }

    /// Hand the gathered coordinates to the builder.
    pub(crate) fn apply(self, builder: &mut MeshBuilder) {
        if self.keep {
            builder.set_corner_uvs(&self.corners);
        }
    }
}

/// Read a PLY from raw bytes.
///
/// Dispatches to ASCII or binary (LE/BE) based on the header's `format` line.
/// A picture the file carries in its own header keeps the face coordinates; for
/// one that sits beside the file, read it through [`crate::read_file`].
///
/// # Errors
/// See [`FormatError`]. Parsers never panic.
pub fn read(bytes: &[u8]) -> Result<Mesh, FormatError> {
    read_shaded(bytes, crate::MeshShading::Reconstructed)
}

/// As [`read`], choosing how vertex normals are produced.
///
/// # Errors
/// See [`read`].
pub fn read_shaded(bytes: &[u8], shading: crate::MeshShading) -> Result<Mesh, FormatError> {
    crate::memory::check_estimate(crate::memory::estimate_file_peak_bytes(
        crate::probe::FormatKind::Ply,
        bytes,
        0,
    )?)?;
    read_admitted(bytes, shading, false)
}

/// Read a PLY whose memory has been admitted.
///
/// `atlas_beside` says the caller found the picture the header names, so the
/// per-corner coordinates are worth keeping even though the file itself holds
/// no image.
pub(crate) fn read_admitted(
    bytes: &[u8],
    shading: crate::MeshShading,
    atlas_beside: bool,
) -> Result<Mesh, FormatError> {
    // A UTF-8 BOM in front of `ply` is metadata a Windows tool added; without
    // this the signature check fails on an otherwise valid file.
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let mut parsed = header::parse(bytes)?;
    // Whether the coordinates have a picture to address decides whether they
    // are read at all, so the header's own image is decoded first.
    let embedded = embedded_texture(&parsed.texture);
    parsed.keep_corner_uvs = atlas_beside || embedded.is_some();
    let mut mesh = match parsed.format {
        header::Format::Ascii => ascii::read_shaded(&parsed, shading)?,
        header::Format::BinaryLittleEndian => binary::read_le_shaded(&parsed, shading)?,
        header::Format::BinaryBigEndian => binary::read_be_shaded(&parsed, shading)?,
    };
    // Only a mesh with coordinates can apply an image; attaching one to a mesh
    // without them paints a single flat texel over the whole layer.
    if mesh.has_uvs() {
        if let Some(texture) = embedded {
            mesh.set_texture(texture);
        }
    }
    Ok(mesh)
}

pub(crate) fn estimate_declared_bytes(bytes: &[u8]) -> Result<u64, FormatError> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let parsed = header::parse(bytes)?;
    let empty_vertex_rows = parsed.elements.iter().any(|element| {
        element.name == "vertex"
            && element.count > 0
            && !element
                .properties
                .iter()
                .any(|property| matches!(property, header::Property::Scalar { .. }))
    });
    if empty_vertex_rows {
        // The reader rejects a row with no scalar input before it iterates or
        // allocates from the declared count, so that count adds no peak memory.
        return Ok(0);
    }
    Ok(declared_memory_bytes(&parsed))
}

pub(crate) fn may_have_uvs(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let Ok(parsed) = header::parse(bytes) else {
        return false;
    };
    parsed.elements.iter().any(|element| {
        element.name == "vertex" && element.properties.iter().any(|property| {
            matches!(
                property,
                header::Property::Scalar { name, .. }
                    if matches!(name.as_str(), "s" | "t" | "texture_u" | "texture_v" | "tu" | "tv")
            )
        }) || element.name == "face"
            && element.properties.iter().any(|property| {
                matches!(
                    property,
                    header::Property::List { name, .. } if name == "texcoord"
                )
            })
    })
}

fn declared_memory_bytes(parsed: &header::ParsedHeader<'_>) -> u64 {
    parsed.elements.iter().fold(0_u64, |total, element| {
        let count = u64::try_from(element.count).unwrap_or(u64::MAX);
        let bytes_per_item = match element.name.as_str() {
            "vertex" => {
                size_of::<occluview_core::Vertex>().saturating_add(size_of::<Option<[f32; 2]>>())
            }
            "face" => {
                let indices = size_of::<u32>().saturating_mul(3);
                // A face that carries coordinates keeps one per corner until the
                // mesh is built, and a seam copies its vertex. One copy per face
                // is well above what a scanner's texture charts produce.
                if element.properties.iter().any(|property| {
                    matches!(
                        property,
                        header::Property::List { name, .. } if name == "texcoord"
                    )
                }) {
                    indices
                        .saturating_add(size_of::<[f32; 2]>().saturating_mul(3))
                        .saturating_add(size_of::<occluview_core::Vertex>())
                } else {
                    indices
                }
            }
            _ => 0,
        };
        total
            .saturating_add(count.saturating_mul(u64::try_from(bytes_per_item).unwrap_or(u64::MAX)))
    })
}

/// The most base64 this reader will decode from a header comment.
///
/// Four thirds of the companion-image cap: the same picture budget, applied
/// before the pixels exist, so a header cannot make the reader allocate from
/// input length alone while the file buffer is still held.
pub(crate) fn max_encoded_chars() -> usize {
    let bytes = usize::try_from(crate::companions::MAX_COMPANION_IMAGE_BYTES).unwrap_or(0);
    bytes / 3 * 4
}

/// Decode the texture an export carried in its own header.
///
/// A header that names a file but carries nothing decodable yields `None`: the
/// image would be beside the file, and this reader deliberately takes bytes and
/// nothing else, so that a file another process is replacing mid-import cannot
/// change what was parsed. A corrupt embedded image is ignored rather than
/// raised — the geometry in the same file is still worth opening.
fn embedded_texture(comments: &header::TextureComments) -> Option<MeshTexture> {
    // The writer says what it put in the payload. A value this build does not
    // know is not decoded on a guess.
    if let Some(format) = comments.format.as_deref() {
        if format != "png" && format != "jpeg" {
            return None;
        }
    }
    // The header parser already refused to accumulate past the ceiling; a
    // payload that tripped it has no bytes to decode.
    if comments.encoded_too_long {
        return None;
    }
    let encoded = comments.encoded.as_deref()?;
    // Belt and braces for a caller that built `TextureComments` itself: the
    // payload is decoded before the raster decoder can bound it, so the base64
    // itself needs a ceiling. A header may carry as much text as the file
    // holds, and decoding all of it while the file buffer is still alive is the
    // one place in this reader that allocates straight from input length. Four
    // thirds of the companion-image cap is the same picture budget applied
    // before the pixels exist.
    if encoded.len() > max_encoded_chars() {
        return None;
    }
    let png = embed::decode(encoded)?;
    crate::texture_decode::decode_embedded_raster(&png, "PLY").ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn understated_face_counts_cannot_silently_discard_geometry() {
        for encoding in ["ascii", "binary_little_endian", "binary_big_endian"] {
            for declared in [0, 1] {
                let mut bytes = format!(
                    "ply\nformat {encoding} 1.0\nelement vertex 4\n\
                     property float x\nproperty float y\nproperty float z\n\
                     element face {declared}\nproperty list uchar int vertex_indices\nend_header\n"
                )
                .into_bytes();
                for position in [
                    [0.0_f32, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [1.0, 1.0, 0.0],
                ] {
                    if encoding == "ascii" {
                        bytes.extend_from_slice(
                            format!("{} {} {}\n", position[0], position[1], position[2]).as_bytes(),
                        );
                    } else {
                        for component in position {
                            bytes.extend_from_slice(&if encoding == "binary_little_endian" {
                                component.to_le_bytes()
                            } else {
                                component.to_be_bytes()
                            });
                        }
                    }
                }
                for triangle in [[0_i32, 1, 2], [2, 1, 3]] {
                    if encoding == "ascii" {
                        bytes.extend_from_slice(
                            format!("3 {} {} {}\n", triangle[0], triangle[1], triangle[2])
                                .as_bytes(),
                        );
                    } else {
                        bytes.push(3);
                        for index in triangle {
                            bytes.extend_from_slice(&if encoding == "binary_little_endian" {
                                index.to_le_bytes()
                            } else {
                                index.to_be_bytes()
                            });
                        }
                    }
                }
                let mesh = read(&bytes);
                assert!(
                    mesh.is_err() || mesh.as_ref().is_ok_and(|mesh| mesh.triangle_count() == 2),
                    "{encoding} count {declared} discarded a complete face"
                );
            }
        }
    }

    use super::*;

    fn face_uv_file(format: &str, faces: &[([u32; 3], Vec<f32>)], faces_first: bool) -> Vec<u8> {
        let vertices = "element vertex 4\nproperty float x\nproperty float y\nproperty float z\n";
        let face_header = format!("element face {}\nproperty list uchar int vertex_indices\nproperty list uchar float texcoord\n", faces.len());
        let mut bytes = format!(
            "ply\nformat {format} 1.0\n{}{}end_header\n",
            if faces_first {
                face_header.as_str()
            } else {
                vertices
            },
            if faces_first {
                vertices
            } else {
                face_header.as_str()
            }
        )
        .into_bytes();
        for face_rows in if faces_first {
            [true, false]
        } else {
            [false, true]
        } {
            if face_rows {
                for (corners, coords) in faces {
                    if format == "ascii" {
                        bytes.extend_from_slice(
                            format!(
                                "3 {} {} {} {}",
                                corners[0],
                                corners[1],
                                corners[2],
                                coords.len()
                            )
                            .as_bytes(),
                        );
                        for coord in coords {
                            bytes.extend_from_slice(format!(" {coord}").as_bytes());
                        }
                        bytes.push(b'\n');
                    } else {
                        bytes.push(3);
                        for corner in corners {
                            bytes.extend_from_slice(&if format == "binary_big_endian" {
                                corner.to_be_bytes()
                            } else {
                                corner.to_le_bytes()
                            });
                        }
                        bytes.push(u8::try_from(coords.len()).expect("small UV list"));
                        for coord in coords {
                            bytes.extend_from_slice(&if format == "binary_big_endian" {
                                coord.to_be_bytes()
                            } else {
                                coord.to_le_bytes()
                            });
                        }
                    }
                }
            } else if format == "ascii" {
                bytes.extend_from_slice(b"0 0 0\n1 0 0\n0 1 0\n1 1 0\n");
            } else {
                for value in [
                    0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0,
                ] {
                    bytes.extend_from_slice(&if format == "binary_big_endian" {
                        value.to_be_bytes()
                    } else {
                        value.to_le_bytes()
                    });
                }
            }
        }
        bytes
    }

    /// Read as the loader does once it has found the picture beside the file.
    fn with_atlas(bytes: &[u8]) -> Result<Mesh, FormatError> {
        read_admitted(bytes, crate::MeshShading::Reconstructed, true)
    }

    #[test]
    fn without_a_picture_the_seams_are_not_paid_for() {
        for format in ["ascii", "binary_little_endian", "binary_big_endian"] {
            let faces = [
                ([0, 1, 2], vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0]),
                ([0, 2, 3], vec![0.5, 0.5, 0.0, 1.0, 1.0, 1.0]),
            ];
            let mesh = read(&face_uv_file(format, &faces, false)).expect("a seamed file opens");
            assert_eq!(mesh.vertices().len(), 4, "{format}: no vertex was copied");
            assert_eq!(mesh.triangle_count(), 2, "{format}");
            assert!(
                !mesh.has_uvs(),
                "{format}: coordinates nothing draws are dropped"
            );
        }
    }

    #[test]
    fn face_uv_seams_copy_the_vertex_and_keep_the_surface() {
        for format in ["ascii", "binary_little_endian", "binary_big_endian"] {
            for faces_first in [false, true] {
                // Vertex 0 meets two different coordinates; vertex 2 meets the
                // same one twice. Only vertex 0 sits on a seam.
                let faces = [
                    ([0, 1, 2], vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0]),
                    ([0, 2, 3], vec![0.5, 0.5, 0.0, 1.0, 1.0, 1.0]),
                ];
                let mesh = with_atlas(&face_uv_file(format, &faces, faces_first))
                    .unwrap_or_else(|error| panic!("{format}, {faces_first}: {error}"));
                assert_eq!(mesh.triangle_count(), 2, "{format}, {faces_first}");
                assert_eq!(mesh.vertices().len(), 5, "{format}, {faces_first}");
                let corner_uv = |triangle: usize, corner: usize| {
                    let index = mesh.indices()[triangle * 3 + corner] as usize;
                    (mesh.vertices()[index].position, mesh.vertices()[index].uv)
                };
                assert_eq!(corner_uv(0, 0), ([0.0, 0.0, 0.0], [0.0, 0.0]));
                assert_eq!(corner_uv(1, 0), ([0.0, 0.0, 0.0], [0.5, 0.5]));
                assert_eq!(corner_uv(0, 2), ([0.0, 1.0, 0.0], [0.0, 1.0]));
                assert_eq!(corner_uv(1, 1), ([0.0, 1.0, 0.0], [0.0, 1.0]));
                assert_eq!(corner_uv(1, 2), ([1.0, 1.0, 0.0], [1.0, 1.0]));
                assert_eq!(
                    mesh.indices()[2],
                    mesh.indices()[4],
                    "vertex 2 agrees on its coordinate and is shared"
                );
                let matching = [
                    faces[0].clone(),
                    ([0, 2, 3], vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
                ];
                let mesh = with_atlas(&face_uv_file(format, &matching, faces_first))
                    .expect("consistent coordinates");
                assert_eq!(mesh.vertices().len(), 4, "no seam, no copy");
                assert_eq!(mesh.triangle_count(), 2);
            }
        }
    }

    #[test]
    fn a_polygon_fans_its_corner_coordinates_with_its_triangles() {
        let mut uvs = FaceUvs::new(true);
        uvs.face(&[0, 1, 2, 3], &[0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])
            .expect("quad");
        assert_eq!(
            uvs.corners,
            [
                [0.0, 0.0],
                [1.0, 0.0],
                [1.0, 1.0],
                [0.0, 0.0],
                [1.0, 1.0],
                [0.0, 1.0]
            ]
        );
        let mut uvs = FaceUvs::new(true);
        uvs.face(&[0, 1], &[0.0; 4])
            .expect("a line makes no triangle");
        assert!(uvs.corners.is_empty());
    }

    #[test]
    fn face_uv_lists_require_two_coordinates_per_corner() {
        for format in ["ascii", "binary_little_endian", "binary_big_endian"] {
            for count in [0, 1, 4, 5, 7, 8] {
                let faces = [([0, 1, 2], vec![0.0; count])];
                assert!(
                    read(&face_uv_file(format, &faces, false)).is_err(),
                    "{format}: {count}"
                );
            }
        }
    }

    #[test]
    fn integer_face_uvs_require_integer_values() {
        let faces = vec![([0, 1, 2], vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0])];
        let bytes = face_uv_file("ascii", &faces, false);
        let text = String::from_utf8(bytes)
            .expect("ASCII")
            .replace("uchar float texcoord", "uchar int texcoord");
        assert!(
            read(text.as_bytes()).is_ok(),
            "integer coordinates remain supported"
        );
        let fractional = text.replace("6 0 0 1 0 0 1", "6 0.5 0 1 0 0 1");
        assert_ne!(
            fractional, text,
            "fixture includes a fractional integer token"
        );
        assert!(
            read(fractional.as_bytes()).is_err(),
            "integer UV property accepted a fraction"
        );
    }

    #[test]
    fn integer_vertex_fields_are_preserved_in_every_encoding() {
        for format in ["ascii", "binary_little_endian", "binary_big_endian"] {
            let mut bytes = format!(
                "ply\nformat {format} 1.0\nelement vertex 1\n\
                 property int x\nproperty int y\nproperty int z\n\
                 property int nx\nproperty int ny\nproperty int nz\n\
                 property int s\nproperty int t\nend_header\n"
            )
            .into_bytes();
            let values = [2i32, -3, 4, 0, 0, 1, 1, -1];
            if format == "ascii" {
                bytes.extend_from_slice(b"2 -3 4 0 0 1 1 -1\n");
            } else {
                for value in values {
                    bytes.extend_from_slice(&if format == "binary_big_endian" {
                        value.to_be_bytes()
                    } else {
                        value.to_le_bytes()
                    });
                }
            }
            let mesh = read(&bytes).expect("integer vertex fields");
            assert_eq!(mesh.vertices()[0].position, [2.0, -3.0, 4.0], "{format}");
            assert_eq!(mesh.vertices()[0].normal, [0.0, 0.0, 1.0], "{format}");
            assert_eq!(mesh.vertices()[0].uv, [1.0, -1.0], "{format}");
        }
    }

    #[test]
    fn non_integer_ascii_vertex_properties_are_rejected() {
        for value in ["1.5", "NaN", "word"] {
            let bytes = format!(
                "ply\nformat ascii 1.0\nelement vertex 1\nproperty int x\nend_header\n{value}\n"
            );
            assert!(read(bytes.as_bytes()).is_err(), "accepted integer {value}");
        }
    }

    fn vertex_list_file(format: &str) -> Vec<u8> {
        let mut bytes = format!(
            "ply\nformat {format} 1.0\nelement vertex 2\n\
             property list uchar int neighbors\nproperty float x\n\
             property list uchar float weights\nproperty float y\nproperty float z\n\
             property list uchar uchar flags\nend_header\n"
        )
        .into_bytes();
        if format == "ascii" {
            bytes.extend_from_slice(b"2 8 9 1 1 0.5 2 3 2 10 11\n0 4 0 5 6 0\n");
        } else {
            let big = format == "binary_big_endian";
            bytes.push(2);
            for n in [8i32, 9] {
                bytes.extend_from_slice(&if big {
                    n.to_be_bytes()
                } else {
                    n.to_le_bytes()
                });
            }
            let float = |n: f32| {
                if big {
                    n.to_be_bytes()
                } else {
                    n.to_le_bytes()
                }
            };
            bytes.extend_from_slice(&float(1.0));
            bytes.push(1);
            bytes.extend_from_slice(&float(0.5));
            bytes.extend_from_slice(&float(2.0));
            bytes.extend_from_slice(&float(3.0));
            bytes.extend_from_slice(&[2, 10, 11, 0]);
            bytes.extend_from_slice(&float(4.0));
            bytes.push(0);
            bytes.extend_from_slice(&float(5.0));
            bytes.extend_from_slice(&float(6.0));
            bytes.push(0);
        }
        bytes
    }

    #[test]
    fn vertex_lists_are_consumed_in_every_encoding() {
        for format in ["ascii", "binary_little_endian", "binary_big_endian"] {
            let mesh = read(&vertex_list_file(format)).expect("valid vertex lists");
            assert_eq!(mesh.vertices()[0].position, [1.0, 2.0, 3.0], "{format}");
            assert_eq!(mesh.vertices()[1].position, [4.0, 5.0, 6.0], "{format}");
            assert!(mesh.is_point_cloud());
        }
    }

    #[test]
    fn truncated_vertex_lists_are_rejected_in_every_encoding() {
        for format in ["ascii", "binary_little_endian", "binary_big_endian"] {
            let mut bytes = vertex_list_file(format);
            bytes.truncate(bytes.len() - 2);
            assert!(read(&bytes).is_err(), "accepted truncated {format} list");
        }
    }
}
