//! OFF reader (Object File Format).
//!
//! Standard Princeton OFF: `OFF BINARY\n` header + 3x LE i32 counts + LE f64
//! positions + LE i32 face indices; ASCII variant also supported. N-gon faces
//! are fan-triangulated. Note: some dental CAD software emits a non-standard
//! binary OFF variant (BE floats, compressed) that is NOT supported here.
//!
//! Index/count casts are allowed at module scope.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]

use crate::error::FormatError;
use glam::Vec3;
use occluview_core::{Mesh, MeshBuilder, Vertex};

/// Cap a file-declared element count before it is used to pre-reserve a `Vec`.
///
/// A corrupt or hostile header can claim billions of vertices/indices; a raw
/// `Vec::with_capacity(that)` reserves gigabytes and aborts the process on
/// Windows (where allocations are committed eagerly) — a hard crash on a bad
/// file.
///
/// The unit matters: an element of THIS vector is a `Vec3`, so bounding the
/// count by the remaining bytes reserved `12 x` the file. At the 1 GiB import
/// cap that is ~12.9 GB, twice over with two files in flight, which is exactly
/// the reservation the bound exists to prevent. A truthful file still gets its
/// exact count, because a truthful header cannot declare more elements than
/// the bytes could encode.
use std::mem::size_of;

fn bounded_capacity<T>(declared: usize, remaining_bytes: usize) -> usize {
    declared.min(remaining_bytes / size_of::<T>().max(1))
}

/// Read an OFF file from raw bytes.
///
/// # Errors
/// - [`FormatError::BadSignature`] if not an OFF file.
/// - [`FormatError::Malformed`] for truncated or structurally invalid data.
/// - [`FormatError::Core`] for index-out-of-range.
pub fn read(bytes: &[u8]) -> Result<Mesh, FormatError> {
    // Detect ASCII vs binary by the first line.
    if bytes.starts_with(b"OFF BINARY") {
        read_binary(bytes)
    } else if bytes.starts_with(b"OFF")
        || bytes.starts_with(b"OFF\n")
        || bytes.starts_with(b"OFF\r")
        || bytes.starts_with(b"OFFST")
    {
        // "OFF", "OFF\n", "OFF\r\n", or "OFF ST" (with normals) — ASCII.
        read_ascii(bytes)
    } else {
        Err(FormatError::BadSignature {
            format: "OFF",
            offset: 0,
        })
    }
}

fn read_binary(bytes: &[u8]) -> Result<Mesh, FormatError> {
    // Skip the "OFF BINARY\n" header (10 bytes).
    let header = b"OFF BINARY\n";
    let cursor_start = header.len();
    if bytes.len() < cursor_start + 12 {
        return Err(FormatError::Truncated {
            format: "OFF (binary)",
            expected: cursor_start + 12,
            got: bytes.len(),
        });
    }
    let mut cur = cursor_start;
    let read_i32 = |b: &[u8], off: &mut usize| -> Result<i32, FormatError> {
        // `.get()` (not `b[..]`) so truncated face/index data yields an honest
        // `Truncated` error instead of an out-of-range slice-index panic.
        let arr: [u8; 4] = b
            .get(*off..*off + 4)
            .and_then(|slice| slice.try_into().ok())
            .ok_or(FormatError::Truncated {
                format: "OFF (binary)",
                expected: *off + 4,
                got: b.len(),
            })?;
        *off += 4;
        Ok(i32::from_le_bytes(arr))
    };

    let v_count = read_i32(bytes, &mut cur)?.max(0) as usize;
    let f_count = read_i32(bytes, &mut cur)?.max(0) as usize;
    let _e_count = read_i32(bytes, &mut cur)? as usize; // edges: unused

    let mut positions: Vec<Vec3> = Vec::with_capacity(bounded_capacity::<Vec3>(
        v_count,
        bytes.len().saturating_sub(cur),
    ));
    for _ in 0..v_count {
        let x = read_f64_le(bytes, &mut cur)?;
        let y = read_f64_le(bytes, &mut cur)?;
        let z = read_f64_le(bytes, &mut cur)?;
        positions.push(Vec3::new(x as f32, y as f32, z as f32));
    }

    let mut builder = MeshBuilder::new()
        .with_name("OFF")
        .from_input_of(bytes.len());
    // Pre-push all vertices so face indices reference them by 0-based handle.
    for p in &positions {
        builder.push_vertex(Vertex::at(*p));
    }

    for _ in 0..f_count {
        let n = read_i32(bytes, &mut cur)?.max(0) as usize;
        if n < 3 {
            // Skip degenerate face's indices.
            for _ in 0..n {
                let _ = read_i32(bytes, &mut cur)?;
            }
            continue;
        }
        let mut idxs: Vec<u32> =
            Vec::with_capacity(bounded_capacity::<u32>(n, bytes.len().saturating_sub(cur)));
        for k in 0..n {
            let raw = read_i32(bytes, &mut cur)?;
            let idx = u32::try_from(raw.max(0)).map_err(|_| FormatError::Malformed {
                format: "OFF (binary)",
                offset: cur,
                reason: format!("negative vertex index {raw} in face"),
            })?;
            if idx as usize >= v_count {
                return Err(FormatError::Core(
                    occluview_core::CoreError::IndexOutOfRange {
                        at_index: k,
                        value: idx,
                        vertex_count: v_count as u32,
                    },
                ));
            }
            idxs.push(idx);
        }
        // Fan triangulate.
        let f0 = idxs[0];
        for w in idxs[1..].windows(2) {
            builder.push_triangle(f0, w[0], w[1]);
        }
    }

    builder.build().map_err(FormatError::Core)
}

fn read_ascii(bytes: &[u8]) -> Result<Mesh, FormatError> {
    let text = std::str::from_utf8(bytes).map_err(|_| FormatError::Malformed {
        format: "OFF (ascii)",
        offset: 0,
        reason: "file is not valid UTF-8".to_string(),
    })?;
    let mut lines = text.lines();
    // First line: OFF (optionally with normals/colors flags). A writer may put
    // the three counts on this same keyword line -- `OFF 3 1 0` is as valid as
    // `OFF` followed by `3 1 0` -- so the rest of the keyword line is the
    // counts when it carries anything, and only a bare keyword falls through
    // to the next non-comment line.
    let first = lines.next().unwrap_or_default();
    let keyword_tail = first
        .trim_start()
        .strip_prefix("OFF")
        .map(str::trim)
        .filter(|rest| !rest.is_empty());
    let counts_line = match keyword_tail {
        Some(tail) => tail.to_string(),
        None => lines
            .by_ref()
            .find(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
            .ok_or(FormatError::Truncated {
                format: "OFF (ascii)",
                expected: 0,
                got: 0,
            })?
            .to_string(),
    };
    let mut counts = counts_line.split_whitespace();
    let v_count: usize = counts
        .next()
        .ok_or_else(|| malformed("vertex count missing"))?
        .parse()
        .map_err(|_| malformed("bad vertex count"))?;
    let f_count: usize = counts
        .next()
        .ok_or_else(|| malformed("face count missing"))?
        .parse()
        .map_err(|_| malformed("bad face count"))?;

    // Bound the reservation by the remaining text: an ASCII vertex needs at
    // least a few bytes, so a header claiming billions of vertices in a tiny
    // file cannot force a gigabyte reservation (which aborts on Windows).
    let mut positions: Vec<Vec3> =
        Vec::with_capacity(bounded_capacity::<Vec3>(v_count, bytes.len()));
    let mut lexer = Lexer::new(lines);

    for _ in 0..v_count {
        let x = lexer.next_f32()?;
        let y = lexer.next_f32()?;
        let z = lexer.next_f32()?;
        positions.push(Vec3::new(x, y, z));
    }

    let mut builder = MeshBuilder::new()
        .with_name("OFF")
        .from_input_of(bytes.len());
    for p in &positions {
        builder.push_vertex(Vertex::at(*p));
    }

    for _ in 0..f_count {
        let n_tok = lexer.next_f32()?;
        let n = n_tok as usize;
        if n < 3 {
            for _ in 0..n {
                let _ = lexer.next_f32()?;
            }
            continue;
        }
        let mut idxs: Vec<u32> = Vec::with_capacity(bounded_capacity::<u32>(n, bytes.len()));
        for k in 0..n {
            let raw = lexer.next_f32()?;
            let idx = raw as u32;
            if idx as usize >= v_count {
                return Err(FormatError::Core(
                    occluview_core::CoreError::IndexOutOfRange {
                        at_index: k,
                        value: idx,
                        vertex_count: v_count as u32,
                    },
                ));
            }
            idxs.push(idx);
        }
        let f0 = idxs[0];
        for w in idxs[1..].windows(2) {
            builder.push_triangle(f0, w[0], w[1]);
        }
    }

    builder.build().map_err(FormatError::Core)
}

/// Token-stream lexer: yields whitespace-split f32 values, skipping comments
/// and blank lines. Replaces the closure-lifetime tangle above with a struct.
struct Lexer<'a> {
    lines: std::str::Lines<'a>,
    tokens: std::vec::IntoIter<&'a str>,
}

impl<'a> Lexer<'a> {
    fn new(lines: std::str::Lines<'a>) -> Self {
        Self {
            lines,
            tokens: Vec::new().into_iter(),
        }
    }

    fn next_f32(&mut self) -> Result<f32, FormatError> {
        loop {
            if let Some(t) = self.tokens.next() {
                return t
                    .parse::<f32>()
                    .map_err(|_| malformed(&format!("bad number {t:?}")));
            }
            let line = self.lines.next().ok_or(FormatError::Truncated {
                format: "OFF (ascii)",
                expected: 0,
                got: 0,
            })?;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            self.tokens = trimmed.split_whitespace().collect::<Vec<_>>().into_iter();
        }
    }
}

fn read_f64_le(b: &[u8], off: &mut usize) -> Result<f64, FormatError> {
    // `.get()` (not `b[..]`) so a truncated binary OFF yields an honest
    // `Truncated` error instead of an out-of-range slice-index panic.
    let arr: [u8; 8] = b
        .get(*off..*off + 8)
        .and_then(|slice| slice.try_into().ok())
        .ok_or(FormatError::Truncated {
            format: "OFF (binary)",
            expected: *off + 8,
            got: b.len(),
        })?;
    *off += 8;
    Ok(f64::from_le_bytes(arr))
}

fn malformed(reason: &str) -> FormatError {
    FormatError::Malformed {
        format: "OFF",
        offset: 0,
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_minimal_binary_off() {
        // OFF BINARY, 3 verts, 1 face (triangle), little-endian.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"OFF BINARY\n");
        bytes.extend_from_slice(&3i32.to_le_bytes()); // verts
        bytes.extend_from_slice(&1i32.to_le_bytes()); // faces
        bytes.extend_from_slice(&0i32.to_le_bytes()); // edges
                                                      // 3 vertices (f64 LE): (0,0,0), (1,0,0), (0,1,0)
        for f in [0.0f64, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        // 1 triangle face: n=3, indices 0,1,2
        bytes.extend_from_slice(&3i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&2i32.to_le_bytes());

        let mesh = read(&bytes).expect("valid binary OFF");
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.vertices().len(), 3);
        assert_eq!(mesh.vertices()[1].position, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn reads_minimal_ascii_off() {
        let text = "OFF\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
        let mesh = read(text.as_bytes()).expect("valid ASCII OFF");
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.vertices().len(), 3);
    }

    #[test]
    fn fan_triangulates_quad() {
        let text = "OFF\n4 1 0\n0 0 0\n1 0 0\n1 1 0\n0 1 0\n4 0 1 2 3\n";
        let mesh = read(text.as_bytes()).expect("valid");
        assert_eq!(mesh.triangle_count(), 2);
    }

    #[test]
    fn rejects_bad_signature() {
        assert!(read(b"NOTOFF...").is_err());
    }

    #[test]
    fn rejects_truncated_header() {
        assert!(read(b"OFF BINARY\nshort").is_err());
    }

    #[test]
    fn rejects_out_of_range_index() {
        let text = "OFF\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 9 2\n";
        let err = read(text.as_bytes()).unwrap_err();
        assert!(matches!(
            err,
            FormatError::Core(occluview_core::CoreError::IndexOutOfRange { .. })
        ));
    }

    #[test]
    fn tolerates_comments_and_blanks() {
        let text = "OFF\n# a comment\n\n3 1 0\n# another\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
        let mesh = read(text.as_bytes()).expect("valid");
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn bounded_capacity_caps_liar_but_keeps_honest_count() {
        // Truthful file: reserve exactly what the header declares.
        assert_eq!(bounded_capacity::<Vec3>(3, 4096), 3);
        // Lie: a tiny file claiming billions is capped to what its bytes could
        // encode, so the reservation cannot abort the process.
        assert_eq!(bounded_capacity::<Vec3>(4_000_000_000, 24), 2);
    }

    /// The bound must be counted in BYTES, not elements.
    ///
    /// Bounding the element count by the remaining bytes reserved `size_of::<T>`
    /// times the file: a 1 GiB OFF whose header lies reserved ~12.9 GB of
    /// `Vec3` (and twice that with two files in flight), which is the eager
    /// commit that aborts on Windows. A smaller element must still get its own
    /// honest count, so an `u32` face row is not cut to a twelfth of what its
    /// bytes hold.
    #[test]
    fn the_reservation_is_bounded_by_bytes_not_elements() {
        assert_eq!(bounded_capacity::<Vec3>(1_000_000, 24_000), 2_000);
        assert_eq!(bounded_capacity::<u32>(1_000_000, 24_000), 6_000);
        // No element may reserve more than the file could hold.
        for bytes in [0usize, 1, 11, 12, 4096] {
            let reserved = bounded_capacity::<Vec3>(usize::MAX, bytes);
            assert!(reserved * size_of::<Vec3>() <= bytes);
        }
    }

    #[test]
    fn counts_on_the_keyword_line_are_read_instead_of_an_empty_mesh() {
        // Several writers put the three counts on the keyword line. Discarding
        // that line made the reader take the first vertex row as the counts and
        // return an empty mesh with no error.
        let text = "OFF 3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
        let mesh = read(text.as_bytes()).expect("keyword-line counts must parse");
        assert_eq!(mesh.vertices().len(), 3);
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn a_bare_keyword_line_still_takes_the_counts_from_the_next_line() {
        let text = "OFF\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
        let mesh = read(text.as_bytes()).expect("bare keyword must still parse");
        assert_eq!(mesh.vertices().len(), 3);
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn a_comment_between_the_keyword_and_the_counts_is_skipped() {
        let text = "OFF\n# written by a tool\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
        let mesh = read(text.as_bytes()).expect("comment must be skipped");
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn ascii_vertex_count_bomb_errors_without_aborting() {
        // A header claiming 4 billion vertices in a near-empty file must fail
        // as a truncation, never a multi-gigabyte reservation.
        let text = "OFF\n4000000000 4000000000 0\n0 0 0\n";
        assert!(read(text.as_bytes()).is_err());
    }

    #[test]
    fn ascii_negative_face_degree_is_rejected_not_reserved() {
        // A negative n-gon degree used to cast to a giant usize.
        let text = "OFF\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n-9 0 1 2\n";
        // Degenerate (n<3 after clamping) faces are skipped; the file still
        // parses to its 3 vertices with no faces rather than crashing.
        let mesh = read(text.as_bytes()).expect("negative degree is skipped, not fatal");
        assert_eq!(mesh.triangle_count(), 0);
    }

    #[test]
    fn binary_vertex_count_bomb_errors_without_aborting() {
        // "OFF BINARY\n" + three LE i32 counts claiming ~4B vertices, no data.
        let mut bytes = b"OFF BINARY\n".to_vec();
        bytes.extend_from_slice(&2_000_000_000i32.to_le_bytes()); // v_count
        bytes.extend_from_slice(&0i32.to_le_bytes()); // f_count
        bytes.extend_from_slice(&0i32.to_le_bytes()); // e_count
        assert!(read(&bytes).is_err());
    }
}
