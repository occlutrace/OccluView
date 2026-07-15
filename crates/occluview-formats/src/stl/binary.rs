//! Binary STL reader.
//!
//! Layout: 80-byte header, `u32` little-endian triangle count, then N triangles
//! of 50 bytes each: `f32×3` normal, `f32×3`×3 vertex, `u16` attribute.
//! Total expected size = `84 + N × 50`.
//!
//! Dental-scanner quirks tolerated here:
//! - Header may contain non-ASCII bytes; ignored.
//! - Triangle count may be wrong; we read until the declared count *or* EOF,
//!   whichever comes first, and report truncation only if we run out mid-triangle.
//! - Files smaller than 84 bytes are rejected as truncated.

use crate::error::FormatError;
use glam::Vec3;
use occluview_core::{Mesh, Vertex};
use rayon::prelude::*;

/// Header size in bytes.
const HEADER_SIZE: usize = 80;
/// Size of the triangle-count field.
const COUNT_SIZE: usize = 4;
/// Bytes per triangle in binary STL.
const TRIANGLE_SIZE: usize = 50;
/// Offset of the first triangle record.
const FIRST_TRIANGLE_OFFSET: usize = HEADER_SIZE + COUNT_SIZE;

/// Read a binary STL from `bytes`.
///
/// # Errors
/// - [`FormatError::Truncated`] if `bytes` is shorter than the 84-byte header.
/// - [`FormatError::Truncated`] if a triangle record is cut off mid-way.
pub fn read(bytes: &[u8]) -> Result<Mesh, FormatError> {
    if bytes.len() < FIRST_TRIANGLE_OFFSET {
        return Err(FormatError::Truncated {
            format: "STL (binary)",
            expected: FIRST_TRIANGLE_OFFSET,
            got: bytes.len(),
        });
    }

    let count_bytes: [u8; 4] = bytes[HEADER_SIZE..FIRST_TRIANGLE_OFFSET]
        .try_into()
        .map_err(|_| FormatError::Malformed {
            format: "STL (binary)",
            offset: HEADER_SIZE,
            reason: "count field is not 4 bytes".to_string(),
        })?;
    let triangle_count = u32::from_le_bytes(count_bytes) as usize;

    // Upper bound on data we expect. If the file is short, we read what we can
    // (dental scanners sometimes lie about the count); if it's short *inside* a
    // triangle, that's a hard truncation.
    let declared_end = FIRST_TRIANGLE_OFFSET + triangle_count * TRIANGLE_SIZE;
    if bytes.len() < declared_end {
        // Maybe the count is wrong but the file is internally consistent at a
        // smaller count — recompute how many full triangles actually fit.
        let payload_len = bytes.len() - FIRST_TRIANGLE_OFFSET;
        let available = payload_len / TRIANGLE_SIZE;
        let trailing = payload_len % TRIANGLE_SIZE;
        if trailing != 0 {
            return Err(FormatError::Truncated {
                format: "STL (binary)",
                expected: FIRST_TRIANGLE_OFFSET + (available + 1) * TRIANGLE_SIZE,
                got: bytes.len(),
            });
        }
        if available == 0 {
            return Err(FormatError::Truncated {
                format: "STL (binary)",
                expected: declared_end,
                got: bytes.len(),
            });
        }
        // Fall through with the smaller count; tolerate the mismatch.
        return read_triangles(bytes, available);
    }

    read_triangles(bytes, triangle_count)
}

/// Decode one 50-byte triangle record (starting at absolute offset `start`
/// within the full file) into its 3 corner vertices plus shared normal.
///
/// Extracted so the per-record work can run either serially or via rayon
/// over independent chunks — the decode itself has no shared state.
fn decode_triangle_record(bytes: &[u8], start: usize) -> Result<(Vec3, [Vec3; 3]), FormatError> {
    let rec = bytes
        .get(start..start + TRIANGLE_SIZE)
        .ok_or(FormatError::Truncated {
            format: "STL (binary)",
            expected: start + TRIANGLE_SIZE,
            got: bytes.len(),
        })?;

    // Decode 12 little-endian f32: normal + 3 vertices. `chunks_exact(4)`
    // gives us exactly 12 4-byte slices (50 bytes includes a 2-byte
    // attribute trailer we ignore).
    let mut floats = [0.0_f32; 12];
    for (slot, chunk) in floats.iter_mut().zip(rec.chunks_exact(4)) {
        let arr: [u8; 4] = chunk.try_into().map_err(|_| FormatError::Malformed {
            format: "STL (binary)",
            offset: start,
            reason: "float field is not 4 bytes".to_string(),
        })?;
        *slot = f32::from_le_bytes(arr);
    }
    let normal = Vec3::from_array([floats[0], floats[1], floats[2]]);
    let a = Vec3::from_array([floats[3], floats[4], floats[5]]);
    let b = Vec3::from_array([floats[6], floats[7], floats[8]]);
    let c = Vec3::from_array([floats[9], floats[10], floats[11]]);
    Ok((normal, [a, b, c]))
}

/// Build a [`Mesh`] from the first `count` triangle records.
///
/// Records are decoded in parallel via rayon: each triangle record is a
/// fixed 50-byte slice completely independent of its neighbors, and each
/// writes into its own deterministic `[3*i, 3*i+3)` vertex slot and matching
/// index slot, so output order is bit-identical to the equivalent serial
/// loop regardless of thread scheduling.
fn read_triangles(bytes: &[u8], count: usize) -> Result<Mesh, FormatError> {
    let mut vertices = vec![Vertex::default(); count * 3];
    let mut indices = vec![0u32; count * 3];

    vertices
        .par_chunks_mut(3)
        .zip(indices.par_chunks_mut(3))
        .enumerate()
        .try_for_each(
            |(i, (vertex_chunk, index_chunk))| -> Result<(), FormatError> {
                let start = FIRST_TRIANGLE_OFFSET + i * TRIANGLE_SIZE;
                let (normal, [a, b, c]) = decode_triangle_record(bytes, start)?;

                // One vertex per corner (STL is a soup; deduplication is a
                // separate concern, not done at parse time). We attach the
                // per-triangle normal to each of its vertices — matches how
                // every STL viewer shades.
                vertex_chunk[0] = Vertex::at(a).with_normal(normal);
                vertex_chunk[1] = Vertex::at(b).with_normal(normal);
                vertex_chunk[2] = Vertex::at(c).with_normal(normal);

                #[allow(clippy::cast_possible_truncation)]
                let base = (i * 3) as u32;
                index_chunk[0] = base;
                index_chunk[1] = base + 1;
                index_chunk[2] = base + 2;

                Ok(())
            },
        )?;

    Mesh::new(Some("STL".to_string()), vertices, indices).map_err(FormatError::Core)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Build an in-memory binary STL from explicit triangles for tests.
    fn build_binary_stl(header: &[u8; 80], triangles: &[[f32; 12]]) -> Vec<u8> {
        assert!(header.len() == 80);
        let mut out = Vec::with_capacity(84 + triangles.len() * 50);
        out.extend_from_slice(header);
        out.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
        for t in triangles {
            for &f in t {
                out.extend_from_slice(&f.to_le_bytes());
            }
            out.extend_from_slice(&[0, 0]); // attribute byte count
        }
        out
    }

    /// Build an 80-byte header: ASCII `text` left-aligned, zero-padded.
    fn header_with_text(text: &str) -> [u8; 80] {
        let mut h = [0u8; 80];
        let bytes = text.as_bytes();
        let n = bytes.len().min(80);
        h[..n].copy_from_slice(&bytes[..n]);
        h
    }

    #[test]
    fn rejects_short_header() {
        let err = read(&[0u8; 10]).unwrap_err();
        assert!(matches!(err, FormatError::Truncated { got: 10, .. }));
    }

    #[test]
    fn reads_a_single_triangle() {
        // A unit triangle in the XY plane, normal +Z.
        let tri = [
            0.0, 0.0, 1.0, // normal
            0.0, 0.0, 0.0, // v0
            1.0, 0.0, 0.0, // v1
            0.0, 1.0, 0.0, // v2
        ];
        let header = header_with_text("binary stl unit triangle");
        let bytes = build_binary_stl(&header, &[tri]);
        let mesh = read(&bytes).expect("valid STL");
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.vertices().len(), 3);
        // Vertex positions round-trip.
        assert_eq!(mesh.vertices()[0].position, [0.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices()[1].position, [1.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices()[2].position, [0.0, 1.0, 0.0]);
        // Normal attached to all three corners.
        for v in mesh.vertices() {
            assert_eq!(v.normal, [0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn header_non_ascii_is_tolerated() {
        // Some scanners write binary junk in the 80-byte header.
        let mut header = [0xFFu8; 80];
        header[..5].copy_from_slice(b"solid"); // even "solid" prefix shouldn't trip us
        let tri = [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let bytes = build_binary_stl(&header, &[tri]);
        let mesh = read(&bytes).expect("valid despite non-ASCII header");
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn tolerates_wrong_triangle_count_that_overstates() {
        // Scanner declares 5 triangles but only emits 1. We must read the 1,
        // not error, because this is a known real-world quirk.
        let tri = [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let mut bytes = build_binary_stl(&header_with_text("lying scanner header"), &[tri]);
        // Overwrite the count field to claim 5 triangles.
        bytes[80..84].copy_from_slice(&5u32.to_le_bytes());
        let mesh = read(&bytes).expect("should tolerate overstated count");
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn errors_on_truncation_inside_a_triangle() {
        // Declare 1 triangle but provide only part of its bytes.
        let mut bytes = vec![0u8; 84 + 25]; // half a triangle
        bytes[80..84].copy_from_slice(&1u32.to_le_bytes());
        let err = read(&bytes).unwrap_err();
        assert!(matches!(err, FormatError::Truncated { .. }));
    }

    #[test]
    fn reads_many_triangles() {
        let tris: Vec<[f32; 12]> = (0..10)
            .map(|i| {
                let z = i as f32;
                [0.0, 0.0, 1.0, 0.0, 0.0, z, 1.0, 0.0, z, 0.0, 1.0, z]
            })
            .collect();
        let bytes = build_binary_stl(&header_with_text("ten triangles"), &tris);
        let mesh = read(&bytes).expect("valid");
        assert_eq!(mesh.triangle_count(), 10);
        assert_eq!(mesh.vertices().len(), 30);
    }

    /// Tiny serial reference matching the pre-parallelization decode loop,
    /// used to prove the rayon-based `read_triangles` produces identical
    /// output order and values.
    fn read_triangles_serial_reference(bytes: &[u8], count: usize) -> Result<Mesh, FormatError> {
        let mut vertices = Vec::with_capacity(count * 3);
        let mut indices = Vec::with_capacity(count * 3);
        for i in 0..count {
            let start = FIRST_TRIANGLE_OFFSET + i * TRIANGLE_SIZE;
            let (normal, [a, b, c]) = decode_triangle_record(bytes, start)?;
            let base = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
            vertices.push(Vertex::at(a).with_normal(normal));
            vertices.push(Vertex::at(b).with_normal(normal));
            vertices.push(Vertex::at(c).with_normal(normal));
            indices.extend_from_slice(&[base, base + 1, base + 2]);
        }
        Mesh::new(Some("STL".to_string()), vertices, indices).map_err(FormatError::Core)
    }

    #[test]
    fn parallel_decode_matches_serial_reference() {
        let tris: Vec<[f32; 12]> = (0..37)
            .map(|i| {
                let z = i as f32 * 0.1;
                let n = if i % 3 == 0 {
                    [0.0, 0.0, 1.0]
                } else if i % 3 == 1 {
                    [0.0, 1.0, 0.0]
                } else {
                    [1.0, 0.0, 0.0]
                };
                [
                    n[0],
                    n[1],
                    n[2],
                    0.0,
                    0.0,
                    z,
                    1.0,
                    0.0,
                    z + 0.3,
                    0.0,
                    1.0,
                    z - 0.2,
                ]
            })
            .collect();
        let bytes = build_binary_stl(&header_with_text("parallel parity"), &tris);

        let parallel = read(&bytes).expect("parallel decode");
        let serial =
            read_triangles_serial_reference(&bytes, tris.len()).expect("serial reference decode");

        assert_eq!(parallel.vertices(), serial.vertices());
        assert_eq!(parallel.indices(), serial.indices());
    }

    proptest! {
        #[test]
        fn rejects_declared_extra_triangle_when_tail_is_partial(
            full_count in 1usize..8,
            trailing_bytes in 1usize..TRIANGLE_SIZE,
        ) {
            let tri = [
                0.0, 0.0, 1.0,
                0.0, 0.0, 0.0,
                1.0, 0.0, 0.0,
                0.0, 1.0, 0.0,
            ];
            let triangles = vec![tri; full_count];
            let mut bytes = build_binary_stl(&header_with_text("partial tail"), &triangles);
            bytes[80..84].copy_from_slice(&((full_count + 1) as u32).to_le_bytes());
            bytes.extend(std::iter::repeat_n(0u8, trailing_bytes));

            let err = read(&bytes).unwrap_err();

            prop_assert!(
                matches!(err, FormatError::Truncated { .. }),
                "expected truncated partial tail"
            );
        }
    }
}
