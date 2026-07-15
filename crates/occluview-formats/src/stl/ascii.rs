//! ASCII STL reader.
//!
//! Grammar (whitespace-tolerant):
//! ```text
//! solid <optional name>
//!   facet normal nx ny nz
//!     outer loop
//!       vertex x y z
//!       vertex x y z
//!       vertex x y z
//!     endloop
//!   endfacet
//!   ... more facets ...
//! endsolid [<optional name>]
//! ```
//!
//! We are deliberately lenient: case-insensitive keywords, any whitespace
//! separation, optional `solid`/`endsolid` name, and we stop at the first
//! token we can't make sense of, returning what we parsed so far on a hard EOF
//! (some scanners forget the trailing `endsolid`).

use crate::error::FormatError;
use glam::Vec3;
use occluview_core::{Mesh, MeshBuilder, Vertex};

/// Quick check: does `bytes` look like ASCII STL? Used by [`super::read`] to
/// disambiguate before committing to the binary path.
///
/// Returns true iff the file begins with `solid` followed by whitespace *and*
/// the first 512 bytes are printable ASCII / whitespace. Binary STL can also
/// start with the bytes for "solid" by coincidence, but binary STL's body is
/// raw floats — almost never all-printable — so this heuristic is reliable in
/// practice. (False positives here just route to the ASCII reader, which will
/// itself fail cleanly if the file is actually binary.)
#[must_use]
pub fn looks_like_ascii(bytes: &[u8]) -> bool {
    // Match `solid` case-insensitively — some writers emit `SOLID`.
    let prefix_ok = bytes.len() >= 5 && bytes[..5].eq_ignore_ascii_case(b"solid");
    if !prefix_ok {
        return false;
    }
    let after = bytes.get(5);
    let sep_ok = matches!(after, Some(b' ' | b'\n' | b'\r' | b'\t'));
    if !sep_ok {
        return false;
    }
    // Sample window; confirm it's printable text.
    let window = bytes.len().min(512);
    bytes[..window]
        .iter()
        .all(|&b| b.is_ascii_graphic() || b.is_ascii_whitespace())
}

/// Read an ASCII STL from `bytes`.
///
/// # Errors
/// - [`FormatError::BadSignature`] if `bytes` does not start with `solid`.
/// - [`FormatError::Malformed`] on a syntactically broken token sequence.
pub fn read(bytes: &[u8]) -> Result<Mesh, FormatError> {
    let text = std::str::from_utf8(bytes).map_err(|_| FormatError::Malformed {
        format: "STL (ascii)",
        offset: 0,
        reason: "file is not valid UTF-8".to_string(),
    })?;

    // Accept any-case `solid` prefix; reject otherwise.
    let starts_solid = text.len() >= 5 && text[..5].eq_ignore_ascii_case("solid");
    if !starts_solid {
        return Err(FormatError::BadSignature {
            format: "STL (ascii)",
            offset: 0,
        });
    }

    let mut builder = MeshBuilder::new().with_name("STL");
    let mut tokens = text.split_ascii_whitespace().peekable();
    let mut facets = 0_usize;

    // Expect: solid [name]
    expect_keyword(&mut tokens, "solid")?;
    // Optional name token (anything until the next `facet`).
    if let Some(&next) = tokens.peek() {
        if !next.eq_ignore_ascii_case("facet") {
            tokens.next();
        }
    }

    while let Some(&kw) = tokens.peek() {
        if kw.eq_ignore_ascii_case("endsolid") {
            break;
        }
        if !kw.eq_ignore_ascii_case("facet") {
            return Err(unexpected("facet or endsolid", kw, text));
        }
        tokens.next(); // consume `facet`
                       // `normal nx ny nz` — but some writers emit `facet` then skip the normal.
        let normal = if tokens
            .peek()
            .is_some_and(|t| t.eq_ignore_ascii_case("normal"))
        {
            tokens.next();
            // Some writers write `normal 0 0 0` for "unset"; keep as-is, the
            // viewer can recompute if needed.
            read_vec3(&mut tokens, text)?
        } else {
            Vec3::ZERO
        };

        expect_keyword(&mut tokens, "outer")?;
        expect_keyword(&mut tokens, "loop")?;

        // Three vertices.
        let mut verts = [Vec3::ZERO; 3];
        for v in &mut verts {
            expect_keyword(&mut tokens, "vertex")?;
            *v = read_vec3(&mut tokens, text)?;
        }

        expect_keyword(&mut tokens, "endloop")?;
        expect_keyword(&mut tokens, "endfacet")?;

        let a = builder.push_vertex(Vertex::at(verts[0]).with_normal(normal));
        let b = builder.push_vertex(Vertex::at(verts[1]).with_normal(normal));
        let c = builder.push_vertex(Vertex::at(verts[2]).with_normal(normal));
        builder.push_triangle(a, b, c);
        facets += 1;
    }

    if facets == 0 {
        return Err(FormatError::Malformed {
            format: "STL (ascii)",
            offset: 0,
            reason: "no facets parsed".to_string(),
        });
    }

    builder.build().map_err(FormatError::Core)
}

fn expect_keyword<'a, I>(
    tokens: &mut std::iter::Peekable<I>,
    expected: &str,
) -> Result<(), FormatError>
where
    I: Iterator<Item = &'a str>,
{
    match tokens.next() {
        Some(kw) if kw.eq_ignore_ascii_case(expected) => Ok(()),
        Some(other) => Err(unexpected(expected, other, "")),
        None => Err(FormatError::Truncated {
            format: "STL (ascii)",
            expected: 0,
            got: 0,
        }),
    }
}

fn read_vec3<'a, I>(tokens: &mut std::iter::Peekable<I>, _text: &str) -> Result<Vec3, FormatError>
where
    I: Iterator<Item = &'a str>,
{
    let parse = |tokens: &mut std::iter::Peekable<I>| -> Result<f32, FormatError> {
        let tok = tokens.next().ok_or(FormatError::Truncated {
            format: "STL (ascii)",
            expected: 0,
            got: 0,
        })?;
        tok.parse::<f32>().map_err(|_| FormatError::Malformed {
            format: "STL (ascii)",
            offset: 0,
            reason: format!("not a number: {tok:?}"),
        })
    };
    let x = parse(tokens)?;
    let y = parse(tokens)?;
    let z = parse(tokens)?;
    Ok(Vec3::new(x, y, z))
}

fn unexpected(expected: &str, got: &str, _text: &str) -> FormatError {
    FormatError::Malformed {
        format: "STL (ascii)",
        offset: 0,
        reason: format!("expected {expected:?}, got {got:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINGLE_FACET: &str = "solid example
  facet normal 0 0 1
    outer loop
      vertex 0 0 0
      vertex 1 0 0
      vertex 0 1 0
    endloop
  endfacet
endsolid example\n";

    #[test]
    fn looks_like_ascii_detects_text() {
        assert!(looks_like_ascii(SINGLE_FACET.as_bytes()));
    }

    #[test]
    fn looks_like_ascii_rejects_binary_with_solid_prefix() {
        // Binary STL whose 80-byte header happens to begin with "solid", but
        // whose body contains raw (non-printable) float bytes.
        let mut bytes = vec![0u8; 200];
        bytes[..5].copy_from_slice(b"solid");
        bytes[5] = b' ';
        bytes[84] = 0xFF; // raw float junk inside the body
        bytes[85] = 0x00;
        bytes[86] = 0x80;
        bytes[87] = 0x3F;
        assert!(!looks_like_ascii(&bytes));
    }

    #[test]
    fn reads_a_single_facet() {
        let mesh = read(SINGLE_FACET.as_bytes()).expect("valid ASCII STL");
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.vertices().len(), 3);
        assert_eq!(mesh.vertices()[1].position, [1.0, 0.0, 0.0]);
        for v in mesh.vertices() {
            assert_eq!(v.normal, [0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn rejects_non_utf8() {
        let mut bytes = SINGLE_FACET.as_bytes().to_vec();
        bytes[20] = 0xFF;
        assert!(read(&bytes).is_err());
    }

    #[test]
    fn tolerates_missing_endsolid() {
        // Some scanners forget the trailing `endsolid`. We should still parse
        // the facets and return them.
        let no_end = "solid x
  facet normal 0 1 0
    outer loop
      vertex 0 0 0
      vertex 1 0 0
      vertex 0 0 1
    endloop
  endfacet\n";
        let mesh = read(no_end.as_bytes()).expect("missing endsolid is tolerated");
        assert_eq!(mesh.triangle_count(), 1);
    }

    #[test]
    fn errors_on_empty_facets() {
        assert!(read(b"solid\nendsolid\n").is_err());
    }

    #[test]
    fn case_insensitive_keywords() {
        let upper = "SOLID x
  FACET NORMAL 0 0 1
    OUTER LOOP
      VERTEX 0 0 0
      VERTEX 1 0 0
      VERTEX 0 1 0
    ENDLOOP
  ENDFACET
ENDSOLID\n";
        let mesh = read(upper.as_bytes()).expect("case-insensitive parse");
        assert_eq!(mesh.triangle_count(), 1);
    }
}
