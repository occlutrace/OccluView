//! PLY header parser.
//!
//! Parses the textual header into a [`ParsedHeader`] that the data readers
//! (ASCII / binary LE / binary BE) consume. The header is small and ASCII, so
//! we parse it as UTF-8 (rejecting non-UTF-8 headers as malformed).

use crate::error::FormatError;
use std::str::FromStr;

/// The data encoding declared by the `format` line.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Format {
    /// `format ascii 1.0`
    Ascii,
    /// `format binary_little_endian 1.0`
    BinaryLittleEndian,
    /// `format binary_big_endian 1.0`
    BinaryBigEndian,
}

/// A scalar data type a property can have.
///
/// Only the types the PLY spec defines; anything else is rejected.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScalarType {
    /// `char` — 1-byte signed.
    Char,
    /// `uchar` — 1-byte unsigned. (Vertex colors use this.)
    Uchar,
    /// `short` — 2-byte signed.
    Short,
    /// `ushort` — 2-byte unsigned.
    Ushort,
    /// `int` — 4-byte signed.
    Int,
    /// `uint` — 4-byte unsigned.
    Uint,
    /// `float` — 4-byte IEEE.
    Float,
    /// `double` — 8-byte IEEE.
    Double,
}

impl ScalarType {
    /// Size in bytes when encoded in binary.
    #[must_use]
    pub const fn byte_size(self) -> usize {
        match self {
            ScalarType::Char | ScalarType::Uchar => 1,
            ScalarType::Short | ScalarType::Ushort => 2,
            ScalarType::Int | ScalarType::Uint | ScalarType::Float => 4,
            ScalarType::Double => 8,
        }
    }
}

impl FromStr for ScalarType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "char" | "int8" => Ok(Self::Char),
            "uchar" | "uint8" => Ok(Self::Uchar),
            "short" | "int16" => Ok(Self::Short),
            "ushort" | "uint16" => Ok(Self::Ushort),
            "int" | "int32" => Ok(Self::Int),
            "uint" | "uint32" => Ok(Self::Uint),
            "float" | "float32" => Ok(Self::Float),
            "double" | "float64" => Ok(Self::Double),
            _ => Err(()),
        }
    }
}

/// A single property declaration inside an element.
#[derive(Clone, Debug, PartialEq)]
pub enum Property {
    /// `property <type> <name>` — a single scalar.
    Scalar {
        /// The property's name (`x`, `red`, `nx`, …).
        name: String,
        /// Declared scalar type.
        ty: ScalarType,
    },
    /// `property list <count_ty> <elem_ty> <name>` — a variable-length list.
    /// Faces use this for vertex indices.
    List {
        /// The list's name (e.g. `vertex_indices`).
        name: String,
        /// Scalar type encoding the per-row element count.
        count_ty: ScalarType,
        /// Scalar type of each list element.
        elem_ty: ScalarType,
    },
}

impl Property {
    /// The property's name (`x`, `red`, `vertex_indices`, …).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Property::Scalar { name, .. } | Property::List { name, .. } => name,
        }
    }
}

/// One element block of the header (`element vertex N { properties }`).
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    /// Element name (`vertex`, `face`, `edge`, …).
    pub name: String,
    /// How many instances of this element are in the data section.
    pub count: usize,
    /// Declared properties, in order.
    pub properties: Vec<Property>,
}

/// A fully parsed PLY header plus a view onto the data that follows.
#[derive(Clone, Debug)]
pub struct ParsedHeader<'a> {
    /// Declared format.
    pub format: Format,
    /// Elements in declaration order (typically `vertex` then `face`).
    pub elements: Vec<Element>,
    /// The raw bytes after `end_header\n` — the data section.
    pub data: &'a [u8],
}

/// Parse the PLY header from `bytes`. Returns the typed header and a slice of
/// the remaining data bytes.
///
/// # Errors
/// - [`FormatError::BadSignature`] if the file does not start with `ply`.
/// - [`FormatError::Malformed`] for an unparseable header line.
/// - [`FormatError::Truncated`] if `end_header` is never found.
pub fn parse(bytes: &[u8]) -> Result<ParsedHeader<'_>, FormatError> {
    // The header is ASCII; the binary data section that follows `end_header`
    // may contain arbitrary bytes (raw floats) and must NOT be validated as
    // UTF-8. So: find the `end_header` line at the byte level first, split, and
    // only then parse the header portion as UTF-8.

    if !bytes.starts_with(b"ply") {
        return Err(FormatError::BadSignature {
            format: "PLY",
            offset: 0,
        });
    }

    // Find `end_header` as a line (preceded by a newline, or at start, or with
    // `\r\n`). We scan for the byte sequence `end_header` then verify it sits
    // at a line boundary.
    let needle = b"\nend_header";
    let mut end_header_at: Option<usize> = None;
    let mut search_from = 0usize;
    while let Some(idx) = find_subslice(&bytes[search_from..], needle) {
        let abs = search_from + idx;
        // Verify the line really is `end_header` (followed by \n or \r\n or EOF).
        let line_start = abs + 1; // skip the leading \n we matched
        let after_kw = line_start + "end_header".len();
        let terminator_ok = matches!(bytes.get(after_kw).copied(), None | Some(b'\n' | b'\r'));
        if terminator_ok {
            end_header_at = Some(line_start);
            break;
        }
        search_from = abs + 1;
    }

    // Also accept the case where the file begins with `end_header` immediately
    // after `ply\n` (very small headers). The search above handles the common
    // `\nend_header` case; check `ply\nend_header` separately for robustness.
    let header_end_line = match end_header_at {
        Some(at) => at,
        None => {
            // Last attempt: maybe the file is `ply\nend_header\n` (no other lines).
            if bytes.starts_with(b"ply\nend_header") {
                "ply\n".len()
            } else {
                return Err(FormatError::Truncated {
                    format: "PLY",
                    expected: 0, // header terminator missing
                    got: bytes.len(),
                });
            }
        }
    };

    // Skip past "end_header\n" (or "end_header\r\n").
    let after = header_end_line + "end_header".len();
    let mut data_start = after;
    if bytes.get(data_start) == Some(&b'\r') {
        data_start += 1;
    }
    if bytes.get(data_start) == Some(&b'\n') {
        data_start += 1;
    }
    let data = &bytes[data_start..];

    // Header portion (everything up to the `end_header` line) is ASCII; validate.
    let header_bytes = &bytes[..header_end_line];
    let header_text = std::str::from_utf8(header_bytes).map_err(|_| FormatError::Malformed {
        format: "PLY",
        offset: 0,
        reason: "header is not valid UTF-8".to_string(),
    })?;

    let mut format: Option<Format> = None;
    let mut elements: Vec<Element> = Vec::new();

    for line in header_text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("ply") {
            continue;
        }
        if line.starts_with("comment") || line.starts_with("obj_info") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("format ") {
            format = Some(parse_format_kind(rest)?);
            continue;
        }
        if let Some(rest) = line.strip_prefix("element ") {
            elements.push(parse_element_line(rest)?);
            continue;
        }
        if let Some(rest) = line.strip_prefix("property ") {
            push_property(&mut elements, rest)?;
        }
        // Unknown header keyword — lenient: ignore rather than reject.
    }

    let format = format.ok_or(FormatError::Malformed {
        format: "PLY",
        offset: 0,
        reason: "header has no 'format' line".to_string(),
    })?;

    if elements.is_empty() {
        return Err(FormatError::Malformed {
            format: "PLY",
            offset: 0,
            reason: "header declares no elements".to_string(),
        });
    }

    Ok(ParsedHeader {
        format,
        elements,
        data,
    })
}

/// Parse the `format <kind> <version>` line's kind token.
fn parse_format_kind(rest: &str) -> Result<Format, FormatError> {
    let kind = rest
        .split_whitespace()
        .next()
        .ok_or(FormatError::Malformed {
            format: "PLY",
            offset: 0,
            reason: "empty format line".to_string(),
        })?;
    match kind {
        "ascii" => Ok(Format::Ascii),
        "binary_little_endian" => Ok(Format::BinaryLittleEndian),
        "binary_big_endian" => Ok(Format::BinaryBigEndian),
        other => Err(FormatError::Malformed {
            format: "PLY",
            offset: 0,
            reason: format!("unknown format variant: {other}"),
        }),
    }
}

/// Parse the `element <name> <count>` line.
fn parse_element_line(rest: &str) -> Result<Element, FormatError> {
    let mut parts = rest.split_whitespace();
    let name = parts
        .next()
        .ok_or(malformed("element without name", rest))?;
    let count_str = parts
        .next()
        .ok_or(malformed("element without count", rest))?;
    let count = count_str
        .parse::<usize>()
        .map_err(|_| malformed("bad element count", count_str))?;
    Ok(Element {
        name: name.to_string(),
        count,
        properties: Vec::new(),
    })
}

/// Attach a parsed property to the last-declared element.
fn push_property(elements: &mut [Element], rest: &str) -> Result<(), FormatError> {
    let elem = elements.last_mut().ok_or(FormatError::Malformed {
        format: "PLY",
        offset: 0,
        reason: "property before any element".to_string(),
    })?;
    elem.properties.push(parse_property(rest)?);
    Ok(())
}

fn malformed(reason: &str, ctx: &str) -> FormatError {
    FormatError::Malformed {
        format: "PLY",
        offset: 0,
        reason: format!("{reason}: {ctx:?}"),
    }
}

/// Find `needle` in `hay`. A tiny `memchr`-style helper to avoid pulling a
/// dependency for a single byte-substring search.
fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn parse_property(rest: &str) -> Result<Property, FormatError> {
    let mut tokens = rest.split_whitespace();
    let first = tokens.next().ok_or(FormatError::Malformed {
        format: "PLY",
        offset: 0,
        reason: format!("empty property: {rest:?}"),
    })?;

    if first == "list" {
        let count_ty = ScalarType::from_str(tokens.next().ok_or(FormatError::Malformed {
            format: "PLY",
            offset: 0,
            reason: "list property missing count type".to_string(),
        })?)
        .map_err(|()| FormatError::Malformed {
            format: "PLY",
            offset: 0,
            reason: "list property has bad count type".to_string(),
        })?;
        let elem_ty = ScalarType::from_str(tokens.next().ok_or(FormatError::Malformed {
            format: "PLY",
            offset: 0,
            reason: "list property missing element type".to_string(),
        })?)
        .map_err(|()| FormatError::Malformed {
            format: "PLY",
            offset: 0,
            reason: "list property has bad element type".to_string(),
        })?;
        let name = tokens.next().ok_or(FormatError::Malformed {
            format: "PLY",
            offset: 0,
            reason: "list property missing name".to_string(),
        })?;
        return Ok(Property::List {
            name: name.to_string(),
            count_ty,
            elem_ty,
        });
    }

    let ty = ScalarType::from_str(first).map_err(|()| FormatError::Malformed {
        format: "PLY",
        offset: 0,
        reason: format!("unknown property type: {first:?}"),
    })?;
    let name = tokens.next().ok_or(FormatError::Malformed {
        format: "PLY",
        offset: 0,
        reason: format!("property without name: {rest:?}"),
    })?;
    Ok(Property::Scalar {
        name: name.to_string(),
        ty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASCII_HEADER: &str = "ply
format ascii 1.0
comment generated by a dental scanner
element vertex 3
property float x
property float y
property float z
property uchar red
property uchar green
property uchar blue
element face 1
property list uchar int vertex_indices
end_header\n";

    #[test]
    fn parses_ascii_header_with_colors() {
        let parsed = parse(ASCII_HEADER.as_bytes()).expect("valid header");
        assert_eq!(parsed.format, Format::Ascii);
        assert_eq!(parsed.elements.len(), 2);

        let v = &parsed.elements[0];
        assert_eq!(v.name, "vertex");
        assert_eq!(v.count, 3);
        assert_eq!(v.properties.len(), 6);
        assert_eq!(v.properties[3].name(), "red");

        let f = &parsed.elements[1];
        assert_eq!(f.name, "face");
        assert!(matches!(f.properties[0], Property::List { .. }));
    }

    #[test]
    fn parses_binary_little_endian() {
        let header = "ply
format binary_little_endian 1.0
element vertex 0
end_header\n";
        let parsed = parse(header.as_bytes()).expect("valid");
        assert_eq!(parsed.format, Format::BinaryLittleEndian);
    }

    #[test]
    fn parses_binary_big_endian() {
        let header = "ply
format binary_big_endian 1.0
element vertex 0
end_header\n";
        let parsed = parse(header.as_bytes()).expect("valid");
        assert_eq!(parsed.format, Format::BinaryBigEndian);
    }

    #[test]
    fn data_slice_starts_after_end_header() {
        let header = "ply\nformat ascii 1.0\nelement vertex 0\nend_header\nDATA\n";
        let parsed = parse(header.as_bytes()).expect("valid");
        assert_eq!(parsed.data, b"DATA\n");
    }

    #[test]
    fn handles_crlf_line_endings() {
        let header = "ply\r\nformat ascii 1.0\r\nelement vertex 0\r\nend_header\r\nDATA\r\n";
        let parsed = parse(header.as_bytes()).expect("valid");
        assert_eq!(parsed.data, b"DATA\r\n");
    }

    #[test]
    fn rejects_missing_ply_signature() {
        let err = parse(b"not ply\n").unwrap_err();
        assert!(matches!(err, FormatError::BadSignature { .. }));
    }

    #[test]
    fn rejects_missing_format_line() {
        let header = "ply\nelement vertex 0\nend_header\n";
        assert!(parse(header.as_bytes()).is_err());
    }

    #[test]
    fn rejects_missing_end_header() {
        let header = "ply\nformat ascii 1.0\nelement vertex 0\n";
        assert!(parse(header.as_bytes()).is_err());
    }

    #[test]
    fn accepts_type_aliases() {
        let header = "ply
format ascii 1.0
element vertex 1
property float32 x
property uint8 red
end_header\n";
        let parsed = parse(header.as_bytes()).expect("valid");
        let v = &parsed.elements[0];
        assert_eq!(
            v.properties[0],
            Property::Scalar {
                name: "x".into(),
                ty: ScalarType::Float
            }
        );
        assert_eq!(
            v.properties[1],
            Property::Scalar {
                name: "red".into(),
                ty: ScalarType::Uchar
            }
        );
    }

    #[test]
    fn scalar_type_byte_sizes() {
        assert_eq!(ScalarType::Uchar.byte_size(), 1);
        assert_eq!(ScalarType::Short.byte_size(), 2);
        assert_eq!(ScalarType::Int.byte_size(), 4);
        assert_eq!(ScalarType::Double.byte_size(), 8);
    }
}
