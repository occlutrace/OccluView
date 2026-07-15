use super::error::malformed;
use super::json;
use crate::error::FormatError;

/// Read a FLOAT VEC3 accessor as `Vec<[f32; 3]>`.
pub(super) fn read_f32_vec3(
    doc: &json::GltfDoc,
    acc_idx: usize,
    bin_chunk: &[u8],
) -> Result<Vec<[f32; 3]>, FormatError> {
    let acc = doc
        .accessors
        .get(acc_idx)
        .ok_or_else(|| malformed("accessor out of range"))?;
    if acc.component_type != 5126 {
        return Err(malformed(&format!(
            "FLOAT (5126) required, got component_type {}",
            acc.component_type
        )));
    }
    if acc.type_ != "VEC3" {
        return Err(malformed(&format!("VEC3 required, got type {}", acc.type_)));
    }
    let bytes = read_accessor_bytes(doc, acc_idx, 12, bin_chunk)?;
    let mut out = Vec::with_capacity(acc.count);
    for i in 0..acc.count {
        let off = i * 12;
        let x = f32_at(&bytes[off..off + 4]);
        let y = f32_at(&bytes[off + 4..off + 8]);
        let z = f32_at(&bytes[off + 8..off + 12]);
        out.push([x, y, z]);
    }
    Ok(out)
}

/// Read a `TEXCOORD_0` accessor as `Vec<[f32; 2]>`. Supports FLOAT (5126)
/// and normalized `UNSIGNED_BYTE` (5121) / `UNSIGNED_SHORT` (5123) VEC2.
pub(super) fn read_texcoord(
    doc: &json::GltfDoc,
    acc_idx: usize,
    bin_chunk: &[u8],
) -> Result<Vec<[f32; 2]>, FormatError> {
    let acc = doc
        .accessors
        .get(acc_idx)
        .ok_or_else(|| malformed("accessor out of range"))?;
    if acc.type_ != "VEC2" {
        return Err(malformed(&format!(
            "VEC2 TEXCOORD_0 required, got type {}",
            acc.type_
        )));
    }
    let normalized = acc.normalized.unwrap_or(false);
    match acc.component_type {
        5126 => {
            // FLOAT VEC2.
            let bytes = read_accessor_bytes(doc, acc_idx, 8, bin_chunk)?;
            let mut out = Vec::with_capacity(acc.count);
            for i in 0..acc.count {
                let off = i * 8;
                let u = f32_at(bytes.get(off..off + 4).unwrap_or(&[0; 4]));
                let v = f32_at(bytes.get(off + 4..off + 8).unwrap_or(&[0; 4]));
                out.push([u, v]);
            }
            Ok(out)
        }
        5121 => {
            // UNSIGNED_BYTE VEC2, optionally normalized.
            let bytes = read_accessor_bytes(doc, acc_idx, 2, bin_chunk)?;
            let mut out = Vec::with_capacity(acc.count);
            for i in 0..acc.count {
                let off = i * 2;
                let u = bytes.get(off).copied().unwrap_or(0);
                let v = bytes.get(off + 1).copied().unwrap_or(0);
                let uf = if normalized {
                    f32::from(u) / 255.0
                } else {
                    f32::from(u)
                };
                let vf = if normalized {
                    f32::from(v) / 255.0
                } else {
                    f32::from(v)
                };
                out.push([uf, vf]);
            }
            Ok(out)
        }
        5123 => {
            // UNSIGNED_SHORT VEC2, optionally normalized.
            let bytes = read_accessor_bytes(doc, acc_idx, 4, bin_chunk)?;
            let mut out = Vec::with_capacity(acc.count);
            for i in 0..acc.count {
                let off = i * 4;
                let u = u16_at(bytes.get(off..off + 2).unwrap_or(&[0; 2]));
                let v = u16_at(bytes.get(off + 2..off + 4).unwrap_or(&[0; 2]));
                let uf = if normalized {
                    f32::from(u) / 65535.0
                } else {
                    f32::from(u)
                };
                let vf = if normalized {
                    f32::from(v) / 65535.0
                } else {
                    f32::from(v)
                };
                out.push([uf, vf]);
            }
            Ok(out)
        }
        other => Err(malformed(&format!(
            "TEXCOORD_0 component_type {other} not supported (use FLOAT/UBYTE/USHORT)"
        ))),
    }
}

/// Read a `COLOR_0` accessor as `Vec<[u8; 4]>` (RGBA, normalized to `0..=255`).
///
/// Supports FLOAT (5126) VEC3/VEC4 (values `0.0..=1.0`) and `UNSIGNED_BYTE`
/// (5121) VEC3/VEC4 (values `0..=255`; `normalized` is irrelevant since the
/// byte is
/// already what we store).
pub(super) fn read_color_f32(
    doc: &json::GltfDoc,
    acc_idx: usize,
    bin_chunk: &[u8],
) -> Result<Vec<[u8; 4]>, FormatError> {
    let acc = doc
        .accessors
        .get(acc_idx)
        .ok_or_else(|| malformed("accessor out of range"))?;
    let comp_per_elem: usize = match acc.type_.as_str() {
        "VEC3" => 3,
        "VEC4" => 4,
        other => {
            return Err(malformed(&format!(
                "VEC3/VEC4 COLOR_0 required, got {other}"
            )))
        }
    };

    match acc.component_type {
        5126 => {
            // FLOAT.
            let bytes = read_accessor_bytes(doc, acc_idx, comp_per_elem * 4, bin_chunk)?;
            let to_u8 = |f: f32| (f.clamp(0.0, 1.0) * 255.0) as u8;
            let mut out = Vec::with_capacity(acc.count);
            for i in 0..acc.count {
                let off = i * comp_per_elem * 4;
                let r = to_u8(f32_at(bytes.get(off..off + 4).unwrap_or(&[0; 4])));
                let g = to_u8(f32_at(bytes.get(off + 4..off + 8).unwrap_or(&[0; 4])));
                let b = to_u8(f32_at(bytes.get(off + 8..off + 12).unwrap_or(&[0; 4])));
                let a = if comp_per_elem == 4 {
                    to_u8(f32_at(bytes.get(off + 12..off + 16).unwrap_or(&[0; 4])))
                } else {
                    255
                };
                out.push([r, g, b, a]);
            }
            Ok(out)
        }
        5121 => {
            // UNSIGNED_BYTE: the raw byte IS the 0..=255 channel we store.
            let bytes = read_accessor_bytes(doc, acc_idx, comp_per_elem, bin_chunk)?;
            let mut out = Vec::with_capacity(acc.count);
            for i in 0..acc.count {
                let off = i * comp_per_elem;
                let r = bytes[off];
                let g = bytes[off + 1];
                let b = bytes[off + 2];
                let a = if comp_per_elem == 4 {
                    bytes[off + 3]
                } else {
                    255
                };
                out.push([r, g, b, a]);
            }
            Ok(out)
        }
        other => Err(malformed(&format!(
            "COLOR_0 component_type {other} not supported (FLOAT or UNSIGNED_BYTE only)"
        ))),
    }
}

/// Read an index accessor as `Vec<u32>`. Component types: 5125 (UINT),
/// 5123 (USHORT), 5121 (UBYTE).
pub(super) fn read_indices(
    doc: &json::GltfDoc,
    acc_idx: usize,
    bin_chunk: &[u8],
) -> Result<Vec<u32>, FormatError> {
    let acc = doc
        .accessors
        .get(acc_idx)
        .ok_or_else(|| malformed("accessor out of range"))?;
    let bytes_per = match acc.component_type {
        5125 => 4,
        5123 => 2,
        5121 => 1,
        other => {
            return Err(malformed(&format!(
                "index component_type {other} not supported"
            )))
        }
    };
    let bytes = read_accessor_bytes(doc, acc_idx, bytes_per, bin_chunk)?;
    let mut out = Vec::with_capacity(acc.count);
    for i in 0..acc.count {
        let off = i * bytes_per;
        let v = match bytes_per {
            4 => u32_at(bytes.get(off..off + 4).unwrap_or(&[0; 4])),
            2 => u32::from(u16_at(bytes.get(off..off + 2).unwrap_or(&[0; 2]))),
            1 => u32::from(*bytes.get(off).unwrap_or(&0)),
            _ => unreachable!(),
        };
        out.push(v);
    }
    Ok(out)
}

/// Read 2 little-endian bytes as `u16`. Caller guarantees `b.len() >= 2`.
fn u16_at(b: &[u8]) -> u16 {
    let arr: [u8; 2] = b.try_into().unwrap_or([0; 2]);
    u16::from_le_bytes(arr)
}

/// Read 4 little-endian bytes as `u32`. Caller guarantees `b.len() >= 4`.
fn u32_at(b: &[u8]) -> u32 {
    let arr: [u8; 4] = b.try_into().unwrap_or([0; 4]);
    u32::from_le_bytes(arr)
}

/// Read `acc.count` elements of `bytes_per_elem` from the BIN chunk into a
/// flat `Vec<u8>`, honoring buffer view offset/stride.
fn read_accessor_bytes(
    doc: &json::GltfDoc,
    acc_idx: usize,
    bytes_per_elem: usize,
    bin_chunk: &[u8],
) -> Result<Vec<u8>, FormatError> {
    let acc = doc
        .accessors
        .get(acc_idx)
        .ok_or_else(|| malformed("accessor out of range"))?;
    let view = doc
        .buffer_views
        .get(acc.buffer_view)
        .ok_or_else(|| malformed("buffer_view out of range"))?;
    let _buffer = doc
        .buffers
        .get(view.buffer)
        .ok_or_else(|| malformed("buffer out of range"))?;
    // v1: only embedded GLB BIN chunk (buffer 0, no URI). External buffers are
    // rejected upstream; if buffer.uri is Some, this is a .gltf we shouldn't
    // have reached (read() takes only GLB). Defensive check:
    if view.buffer != 0 || doc.buffers.first().is_some_and(|b| b.uri.is_some()) {
        return Err(malformed(
            "external buffer URIs not supported in v1 (GLB only)",
        ));
    }

    let stride = view.byte_stride.unwrap_or(bytes_per_elem);
    let start = view.byte_offset.unwrap_or(0) + acc.byte_offset.unwrap_or(0);
    // A hostile accessor `count` (attacker-controlled JSON) times the element
    // stride can overflow `usize` — a panic in debug and a silent wrap in
    // release that would defeat the bounds check below. Reject the overflow as
    // malformed instead of trusting the arithmetic.
    let span = acc
        .count
        .checked_mul(bytes_per_elem)
        .ok_or_else(|| malformed("accessor count times element size overflows"))?;
    let end = start
        .checked_add(span)
        .ok_or_else(|| malformed("accessor byte range overflows"))?;
    if end > bin_chunk.len() {
        return Err(FormatError::Truncated {
            format: "glTF",
            expected: end,
            got: bin_chunk.len(),
        });
    }
    let mut out = Vec::with_capacity(span);
    for i in 0..acc.count {
        let off = start + i * stride;
        out.extend_from_slice(bin_chunk.get(off..off + bytes_per_elem).ok_or_else(|| {
            FormatError::Truncated {
                format: "glTF",
                expected: off + bytes_per_elem,
                got: bin_chunk.len(),
            }
        })?);
    }
    Ok(out)
}

/// Read 4 little-endian bytes as `f32`. Caller guarantees `b.len() >= 4`.
fn f32_at(b: &[u8]) -> f32 {
    let arr: [u8; 4] = b.try_into().unwrap_or([0; 4]);
    f32::from_le_bytes(arr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gltf::json;

    #[test]
    fn hostile_accessor_count_errors_instead_of_aborting() {
        // A glTF whose accessor claims usize::MAX elements: `count * stride`
        // overflows. The reader must reject it as malformed, never panic (debug)
        // or wrap the bounds check and reserve gigabytes (release/Windows).
        let doc: json::GltfDoc = serde_json::from_str(
            r#"{"asset":{"version":"2.0"},
                "accessors":[{"bufferView":0,"count":18446744073709551615,"type":"VEC3","componentType":5126}],
                "bufferViews":[{"buffer":0,"byteLength":36}],
                "buffers":[{"byteLength":36}]}"#,
        )
        .expect("doc parses");
        let bin = [0u8; 36];
        assert!(
            read_f32_vec3(&doc, 0, &bin).is_err(),
            "overflowing accessor count must be an error, not a crash"
        );
    }
}
