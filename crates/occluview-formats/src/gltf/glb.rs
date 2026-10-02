//! GLB container splitter.
//!
//! GLB layout: 12-byte header (magic `glTF`, version `u32`, length `u32`)
//! followed by chunks: each chunk = 4-byte length + 4-byte type + payload. The
//! first chunk is JSON (type `0x4E4F_534A`), the optional second is BIN
//! (`0x004E_4942`).

use super::error::malformed;
use crate::error::FormatError;

const MAGIC: &[u8; 4] = b"glTF";
/// GLB chunk type for the JSON chunk: ASCII `JSON`.
const JSON_CHUNK_TYPE: u32 = 0x4E4F_534A;
/// GLB chunk type for the BIN chunk: ASCII `BIN\0`.
const BIN_CHUNK_TYPE: u32 = 0x004E_4942;

/// Split a GLB into its JSON document bytes and the optional BIN chunk.
///
/// # Errors
/// - [`FormatError::BadSignature`] if the magic isn't `glTF`.
/// - [`FormatError::Malformed`] for a truncated header, wrong version, or
///   missing JSON chunk.
/// - [`FormatError::Truncated`] if the declared total length or a chunk length
///   exceeds the actual bytes.
pub fn split(bytes: &[u8]) -> Result<(Vec<u8>, &[u8]), FormatError> {
    if bytes.len() < 12 || &bytes[..4] != MAGIC {
        return Err(FormatError::BadSignature {
            format: "glTF (GLB)",
            offset: 0,
        });
    }
    let version = u32_from(&bytes[4..8]);
    if version != 2 {
        return Err(FormatError::Malformed {
            format: "glTF (GLB)",
            offset: 4,
            reason: format!("unsupported GLB version {version} (only 2)"),
        });
    }
    let total_len = u32_from(&bytes[8..12]) as usize;
    if total_len > bytes.len() {
        return Err(FormatError::Truncated {
            format: "glTF (GLB)",
            expected: total_len,
            got: bytes.len(),
        });
    }
    if total_len < 12 || total_len != bytes.len() {
        return Err(malformed("GLB length does not match the complete file"));
    }

    let mut cursor = 12usize;
    let mut json_chunk: Option<Vec<u8>> = None;
    let mut bin_chunk: Option<&[u8]> = None;
    let mut chunk_index = 0;
    while cursor < total_len {
        let header_end = cursor
            .checked_add(8)
            .ok_or_else(|| malformed("GLB chunk header offset overflows"))?;
        if header_end > total_len {
            return Err(FormatError::Truncated {
                format: "glTF (GLB)",
                expected: header_end,
                got: total_len,
            });
        }
        let chunk_len = u32_from(&bytes[cursor..cursor + 4]) as usize;
        let chunk_type = u32_from(&bytes[cursor + 4..cursor + 8]);
        if !chunk_len.is_multiple_of(4) {
            return Err(malformed("GLB chunk length is not aligned to four bytes"));
        }
        if chunk_index == 0 && chunk_type != JSON_CHUNK_TYPE {
            return Err(malformed("GLB must begin with its JSON chunk"));
        }
        let payload_start = header_end;
        let payload_end = payload_start
            .checked_add(chunk_len)
            .ok_or_else(|| malformed("GLB chunk byte range overflows"))?;
        if payload_end > total_len {
            return Err(FormatError::Truncated {
                format: "glTF (GLB)",
                expected: payload_end,
                got: total_len,
            });
        }
        match chunk_type {
            JSON_CHUNK_TYPE => {
                if json_chunk.is_some() || chunk_index != 0 {
                    return Err(malformed(
                        "GLB contains a duplicate or misplaced JSON chunk",
                    ));
                }
                json_chunk = Some(bytes[payload_start..payload_end].to_vec());
            }
            BIN_CHUNK_TYPE => {
                if bin_chunk.is_some() || chunk_index != 1 {
                    return Err(malformed("GLB contains a duplicate or misplaced BIN chunk"));
                }
                bin_chunk = Some(&bytes[payload_start..payload_end]);
            }
            _ => {
                // Unknown chunk: tolerated per spec
                // ("implementations SHOULD ignore unknown chunks").
            }
        }
        // Padding belongs to the declared chunk length.
        cursor = payload_end;
        chunk_index += 1;
    }

    let json = json_chunk.ok_or(FormatError::Malformed {
        format: "glTF (GLB)",
        offset: 0,
        reason: "no JSON chunk".to_string(),
    })?;
    Ok((json, bin_chunk.unwrap_or(&[])))
}

/// Read 4 little-endian bytes as `u32`. The caller has already bounds-checked.
fn u32_from(b: &[u8]) -> u32 {
    let arr: [u8; 4] = b.try_into().unwrap_or([0; 4]);
    u32::from_le_bytes(arr)
}

/// Build a minimal valid GLB from a JSON document and optional BIN chunk.
/// Available under `cfg(test)` so the glTF reader's own tests can synthesize
/// fixtures without re-implementing the chunk layout.
#[cfg(test)]
pub fn build_glb(json: &[u8], bin: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&2u32.to_le_bytes());
    let json_pad = (4 - (json.len() % 4)) % 4;
    let bin_pad = (4 - (bin.len() % 4)) % 4;
    let total = 12
        + 8
        + json.len()
        + json_pad
        + if bin.is_empty() {
            0
        } else {
            8 + bin.len() + bin_pad
        };
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&((json.len() + json_pad) as u32).to_le_bytes());
    out.extend_from_slice(&JSON_CHUNK_TYPE.to_le_bytes());
    out.extend_from_slice(json);
    out.extend(std::iter::repeat_n(b' ', json_pad));
    if !bin.is_empty() {
        out.extend_from_slice(&((bin.len() + bin_pad) as u32).to_le_bytes());
        out.extend_from_slice(&BIN_CHUNK_TYPE.to_le_bytes());
        out.extend_from_slice(bin);
        out.extend(std::iter::repeat_n(0u8, bin_pad));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_an_incomplete_chunk_header() {
        let mut bytes = build_glb(b"{}  ", &[0; 4]);
        bytes.extend_from_slice(&[0; 4]);
        let total = u32::try_from(bytes.len()).expect("length");
        bytes[8..12].copy_from_slice(&total.to_le_bytes());
        assert!(split(&bytes).is_err());
    }

    #[test]
    fn rejects_bytes_beyond_the_declared_length() {
        let mut bytes = build_glb(b"{}  ", &[]);
        bytes.extend_from_slice(&[0; 4]);
        assert!(split(&bytes).is_err());
    }

    #[test]
    fn rejects_a_binary_chunk_before_json() {
        let bytes = build_glb(b"{}  ", &[0; 4]);
        let mut reordered = bytes[..12].to_vec();
        reordered.extend_from_slice(&bytes[24..]);
        reordered.extend_from_slice(&bytes[12..24]);
        assert!(split(&reordered).is_err());
    }

    #[test]
    fn rejects_duplicate_json_chunks() {
        let mut bytes = build_glb(b"{}  ", &[]);
        let duplicate = bytes[12..].to_vec();
        bytes.extend_from_slice(&duplicate);
        let total = u32::try_from(bytes.len()).expect("length");
        bytes[8..12].copy_from_slice(&total.to_le_bytes());
        assert!(split(&bytes).is_err());
    }

    #[test]
    fn rejects_unaligned_chunk_lengths() {
        let mut bytes = build_glb(b"{}  ", &[]);
        bytes[12..16].copy_from_slice(&2u32.to_le_bytes());
        assert!(split(&bytes).is_err());
    }

    #[test]
    fn splits_json_only_glb() {
        let glb = build_glb(b"{\"scenes\":[]}", &[]);
        let (json, bin) = split(&glb).expect("valid");
        assert_eq!(json, b"{\"scenes\":[]}   ");
        assert!(bin.is_empty());
    }

    #[test]
    fn splits_json_and_bin() {
        let glb = build_glb(b"{}", &[1, 2, 3, 4, 5, 6, 7, 8]);
        let (json, bin) = split(&glb).expect("valid");
        assert_eq!(json, b"{}  ");
        assert_eq!(bin, &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn rejects_bad_magic() {
        assert!(split(b"XXXX....").is_err());
    }

    #[test]
    fn rejects_wrong_version() {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&12u32.to_le_bytes());
        assert!(split(&bytes).is_err());
    }

    #[test]
    fn rejects_truncated_header() {
        assert!(split(b"gl").is_err());
    }

    #[test]
    fn ignores_unknown_chunk() {
        // Manually assemble: header + JSON chunk + unknown chunk.
        let json = b"{}";
        let json_pad = (4 - (json.len() % 4)) % 4;
        let unknown = [9u8; 4];
        let unknown_pad = (4 - (unknown.len() % 4)) % 4;
        let total = 12 + 8 + json.len() + json_pad + 8 + unknown.len() + unknown_pad;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&(total as u32).to_le_bytes());
        bytes.extend_from_slice(&((json.len() + json_pad) as u32).to_le_bytes());
        bytes.extend_from_slice(&JSON_CHUNK_TYPE.to_le_bytes());
        bytes.extend_from_slice(json);
        bytes.extend(std::iter::repeat_n(b' ', json_pad));
        bytes.extend_from_slice(&(unknown.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        bytes.extend_from_slice(&unknown);
        bytes.extend(std::iter::repeat_n(0u8, unknown_pad));
        let (parsed_json, bin) = split(&bytes).expect("valid despite unknown chunk");
        assert_eq!(parsed_json, b"{}  ");
        assert!(bin.is_empty());
    }
}
