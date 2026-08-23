use crate::{
    base64, crypto, faces, malformed, texture, xml, DecodedSurface, HpsError, HpsKeyProvider,
    NoHpsKeyProvider, ReadError,
};
use std::io::{Cursor, Read};
use std::mem::size_of;
use std::path::Path;
use zeroize::Zeroizing;

const MAX_PACKAGE_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PACKAGE_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;

/// Decode raw HPS XML or a dental HPS package without a CE key.
///
/// # Errors
/// Returns a typed [`HpsError`] when detection, decoding, or validation fails.
pub fn read(bytes: &[u8]) -> Result<DecodedSurface, HpsError> {
    match read_with_key_provider(bytes, &NoHpsKeyProvider) {
        Ok(surface) => Ok(surface),
        Err(ReadError::Parser(error)) => Err(error),
        Err(ReadError::KeyProvider(error)) => match error {},
    }
}

/// Decode raw HPS XML or a dental HPS package with an explicit CE key provider.
///
/// Provider failures remain distinguishable from parser failures through
/// [`ReadError::KeyProvider`].
///
/// # Errors
/// Returns [`ReadError::Parser`] for format failures and
/// [`ReadError::KeyProvider`] when `key_provider` fails.
pub fn read_with_key_provider<P: HpsKeyProvider + ?Sized>(
    bytes: &[u8],
    key_provider: &P,
) -> Result<DecodedSurface, ReadError<P::Error>> {
    if bytes.len() >= 132 && bytes.get(128..132) == Some(b"DICM".as_slice()) {
        return Err(HpsError::MedicalDicom.into());
    }
    if bytes.starts_with(&[0x50, 0x4B, 0x03, 0x04]) {
        return read_package(bytes, key_provider);
    }

    let text = xml::text_from_bytes(bytes)?;
    if !xml::looks_like_hps_xml(text) {
        return Err(HpsError::BadSignature.into());
    }

    let schema = xml::find_element(text, "Schema")?.body.trim();
    match schema {
        "CA" | "CB" | "CC" | "CE" => read_hps_xml(text, schema, key_provider),
        other => Err(malformed(format!("schema {other:?} is not supported")).into()),
    }
}

fn read_package<P: HpsKeyProvider + ?Sized>(
    bytes: &[u8],
    key_provider: &P,
) -> Result<DecodedSurface, ReadError<P::Error>> {
    let cursor = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|error| malformed(format!("HPS package open failed: {error}")))?;

    let mut aggregate_size = 0_u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|error| malformed(format!("HPS package entry open failed: {error}")))?;
        if !entry.is_dir() {
            aggregate_size = checked_aggregate_uncompressed_size(aggregate_size, entry.size())?;
        }
    }

    for candidate_only in [true, false] {
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|error| malformed(format!("HPS package entry open failed: {error}")))?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().to_string();
            if candidate_only != package_hps_candidate(&name) {
                continue;
            }
            if entry.size() > MAX_PACKAGE_ENTRY_BYTES {
                return Err(HpsError::ResourceLimit {
                    resource: "package entry",
                    limit: MAX_PACKAGE_ENTRY_BYTES,
                }
                .into());
            }

            let mut entry_bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
            entry
                .by_ref()
                .take(MAX_PACKAGE_ENTRY_BYTES + 1)
                .read_to_end(&mut entry_bytes)
                .map_err(|error| malformed(format!("HPS package entry read failed: {error}")))?;
            if u64::try_from(entry_bytes.len()).unwrap_or(u64::MAX) > MAX_PACKAGE_ENTRY_BYTES {
                return Err(HpsError::ResourceLimit {
                    resource: "package entry",
                    limit: MAX_PACKAGE_ENTRY_BYTES,
                }
                .into());
            }

            let Ok(text) = xml::text_from_bytes(&entry_bytes) else {
                continue;
            };
            if !xml::looks_like_hps_xml(text) {
                continue;
            }
            let schema = xml::find_element(text, "Schema")?.body.trim();
            return match schema {
                "CA" | "CB" | "CC" | "CE" => read_hps_xml(text, schema, key_provider),
                other => Err(malformed(format!("schema {other:?} is not supported")).into()),
            };
        }
    }

    Err(malformed("HPS package does not contain HPS geometry XML").into())
}

fn checked_aggregate_uncompressed_size(total: u64, size: u64) -> Result<u64, HpsError> {
    let total = total.checked_add(size).ok_or(HpsError::ResourceLimit {
        resource: "package aggregate uncompressed size",
        limit: MAX_PACKAGE_UNCOMPRESSED_BYTES,
    })?;
    if total > MAX_PACKAGE_UNCOMPRESSED_BYTES {
        return Err(HpsError::ResourceLimit {
            resource: "package aggregate uncompressed size",
            limit: MAX_PACKAGE_UNCOMPRESSED_BYTES,
        });
    }
    Ok(total)
}

fn package_hps_candidate(name: &str) -> bool {
    let filename = name.rsplit('/').next().unwrap_or(name);
    Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("hps") || extension.eq_ignore_ascii_case("xml")
        })
}

fn read_hps_xml<P: HpsKeyProvider + ?Sized>(
    text: &str,
    schema: &str,
    key_provider: &P,
) -> Result<DecodedSurface, ReadError<P::Error>> {
    let key = if schema == "CE" {
        Some(crypto::derive_encryption_key(
            key_provider,
            &xml::parse_properties(text)?,
        )?)
    } else {
        None
    };

    let vertices_element = xml::find_element(text, "Vertices")?;
    let facets_element = xml::find_element(text, "Facets")?;
    let vertex_count = xml::parse_usize_attr(
        xml::required_attr(vertices_element.open_tag, "vertex_count")?,
        "vertex_count",
    )?;
    let face_count = xml::parse_usize_attr(
        xml::required_attr(facets_element.open_tag, "facet_count")?,
        "facet_count",
    )?;

    let key_slice = key.as_ref().map(|key| key.as_slice());
    let vertex_bytes = decode_vertex_bytes(schema, vertices_element, key_slice)?;
    let positions = parse_position_bytes(&vertex_bytes, vertex_count)?;
    let face_bytes = base64::decode(facets_element.body)?;
    let indices = faces::parse(&face_bytes, face_count, vertex_count)?;
    let colors = parse_colors(ColorParseInput {
        text,
        schema,
        key: key_slice,
        vertices_tag: vertices_element.open_tag,
        facets_tag: facets_element.open_tag,
        vertex_count,
    })?;
    let texture = texture::parse_texture_data(text, schema, key_slice, vertex_count, &indices)?;

    build_surface(positions, indices, colors, texture).map_err(Into::into)
}

fn build_surface(
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    colors: Option<Vec<[u8; 4]>>,
    texture: Option<texture::SurfaceTexture>,
) -> Result<DecodedSurface, HpsError> {
    let Some(texture) = texture else {
        let normals = smooth_normals(&positions, &indices);
        return DecodedSurface::new(positions, indices, colors, None, None)?.with_normals(normals);
    };
    let Some(corner_uvs) = texture.corner_uvs else {
        let normals = smooth_normals(&positions, &indices);
        return DecodedSurface::new(positions, indices, colors, None, texture.texture)?
            .with_normals(normals);
    };
    if corner_uvs.len() != indices.len() {
        return Err(HpsError::TextureMalformed {
            reason: "texture coordinate count does not match triangle corners".to_string(),
        });
    }

    let normals = smooth_normals(&positions, &indices);
    DecodedSurface::new(positions, indices, colors, None, texture.texture)?
        .with_corner_uvs(corner_uvs)?
        .with_normals(normals)
}

/// A facet is degenerate when its area falls below this fraction of its own
/// longest edge squared.
///
/// Deliberately a copy: this crate depends on no OccluView crate by design, so
/// it cannot import `occluview_core::DEGENERATE_AREA_SIN` or the one in
/// `occlu-mesh-edit`. All three hold the same number and must move together.
/// They did not once: the 2026-07-25 fix reached the others four weeks later.
const DEGENERATE_AREA_SIN: f32 = 1e-10;

fn smooth_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut normals = vec![[0.0_f32; 3]; positions.len()];
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
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let face_normal = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        let length_squared = dot(face_normal, face_normal);
        let longest_edge_sq = dot(ab, ab).max(dot(ac, ac)).max({
            let bc = [c[0] - b[0], c[1] - b[1], c[2] - b[2]];
            dot(bc, bc)
        });
        if face_normal.iter().all(|component| component.is_finite())
            && length_squared > longest_edge_sq * longest_edge_sq * DEGENERATE_AREA_SIN
        {
            for index in [index_a, index_b, index_c] {
                normals[index][0] += face_normal[0];
                normals[index][1] += face_normal[1];
                normals[index][2] += face_normal[2];
            }
        }
    }
    for normal in &mut normals {
        let length_squared = dot(*normal, *normal);
        if length_squared > f32::EPSILON {
            let inverse_length = length_squared.sqrt().recip();
            normal[0] *= inverse_length;
            normal[1] *= inverse_length;
            normal[2] *= inverse_length;
        } else {
            *normal = [0.0, 0.0, 1.0];
        }
    }
    normals
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn decode_vertex_bytes(
    schema: &str,
    vertices_element: xml::XmlElement<'_>,
    key: Option<&[u8]>,
) -> Result<Zeroizing<Vec<u8>>, HpsError> {
    let encoded = base64::decode(vertices_element.body)?;
    if schema != "CE" {
        return Ok(Zeroizing::new(encoded));
    }

    let key = key.ok_or(HpsError::KeyMissing)?;
    let decrypted = crypto::decrypt_hps_data(
        &encoded,
        key,
        original_size_attr(vertices_element.open_tag)?,
        encrypted_element_uses_scrambled_key(vertices_element.open_tag)?,
    )?;
    if let Some(expected) = xml::attr_value(vertices_element.open_tag, "check_value")? {
        let expected = xml::parse_u32_attr(expected, "check_value")?;
        let actual = crypto::hps_adler32_check_value(&decrypted);
        if actual != expected {
            return Err(HpsError::IntegrityFailure {
                reason: "CE vertex data integrity check failed".to_string(),
            });
        }
    }
    Ok(decrypted)
}

fn parse_position_bytes(bytes: &[u8], vertex_count: usize) -> Result<Vec<[f32; 3]>, HpsError> {
    let expected_len = vertex_count
        .checked_mul(3)
        .and_then(|count| count.checked_mul(size_of::<f32>()))
        .ok_or_else(|| malformed("vertex buffer size overflow"))?;
    if bytes.len() != expected_len {
        return Err(malformed("vertex buffer size does not match vertex_count"));
    }

    let mut positions = Vec::with_capacity(vertex_count);
    for chunk in bytes.chunks_exact(12) {
        positions.push([
            f32::from_le_bytes(
                chunk[0..4]
                    .try_into()
                    .map_err(|_| malformed("bad x vertex"))?,
            ),
            f32::from_le_bytes(
                chunk[4..8]
                    .try_into()
                    .map_err(|_| malformed("bad y vertex"))?,
            ),
            f32::from_le_bytes(
                chunk[8..12]
                    .try_into()
                    .map_err(|_| malformed("bad z vertex"))?,
            ),
        ]);
    }
    Ok(positions)
}

struct ColorParseInput<'a> {
    text: &'a str,
    schema: &'a str,
    key: Option<&'a [u8]>,
    vertices_tag: &'a str,
    facets_tag: &'a str,
    vertex_count: usize,
}

fn parse_colors(input: ColorParseInput<'_>) -> Result<Option<Vec<[u8; 4]>>, HpsError> {
    if let Some(color_element) = xml::find_optional_element(input.text, "VertexColorSet") {
        let mut color_bytes = Zeroizing::new(base64::decode(color_element.body)?);
        if input.schema == "CE" {
            let key = input.key.ok_or(HpsError::KeyMissing)?;
            color_bytes = crypto::decrypt_hps_data(
                &color_bytes,
                key,
                original_size_attr(color_element.open_tag)?,
                encrypted_element_uses_scrambled_key(color_element.open_tag)?,
            )?;
        }
        return parse_color_bytes(&color_bytes, input.vertex_count).map(Some);
    }

    let default_color = xml::attr_value(input.vertices_tag, "color")?
        .or(xml::attr_value(input.facets_tag, "color")?);
    default_color
        .map(xml::parse_color_attr)
        .transpose()
        .map(|maybe_color| {
            maybe_color.and_then(|color| {
                if is_neutral_default_color(color) {
                    None
                } else {
                    Some(vec![color; input.vertex_count])
                }
            })
        })
}

pub(crate) fn original_size_attr(open_tag: &str) -> Result<Option<usize>, HpsError> {
    let lower = xml::optional_usize_attr(open_tag, "base64_encoded_bytes")?;
    if lower.is_some() {
        return Ok(lower);
    }
    xml::optional_usize_attr(open_tag, "Base64EncodedBytes")
}

pub(crate) fn encrypted_element_uses_scrambled_key(open_tag: &str) -> Result<bool, HpsError> {
    Ok(xml::attr_value(open_tag, "Key")?.is_some_and(|value| !value.is_empty()))
}

fn is_neutral_default_color(color: [u8; 4]) -> bool {
    color == [128, 128, 128, 255]
}

fn parse_color_bytes(bytes: &[u8], vertex_count: usize) -> Result<Vec<[u8; 4]>, HpsError> {
    let packed_rgb_len = vertex_count
        .checked_mul(3)
        .ok_or_else(|| malformed("vertex color buffer size overflow"))?;
    let expanded_color_len = vertex_count
        .checked_mul(4)
        .ok_or_else(|| malformed("vertex color buffer size overflow"))?;

    if bytes.len() == packed_rgb_len {
        return Ok(bytes
            .chunks_exact(3)
            .map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
            .collect());
    }
    if bytes.len() == expanded_color_len {
        return Ok(bytes
            .chunks_exact(4)
            .map(|rgba| [rgba[0], rgba[1], rgba[2], rgba[3]])
            .collect());
    }

    Err(malformed(
        "vertex color buffer size does not match vertex_count",
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        build_surface, checked_aggregate_uncompressed_size, MAX_PACKAGE_UNCOMPRESSED_BYTES,
    };
    use crate::{texture::SurfaceTexture, HpsError};

    #[test]
    fn aggregate_zip_uncompressed_size_is_bounded() {
        assert_eq!(
            checked_aggregate_uncompressed_size(MAX_PACKAGE_UNCOMPRESSED_BYTES - 1, 1),
            Ok(MAX_PACKAGE_UNCOMPRESSED_BYTES)
        );
        assert!(matches!(
            checked_aggregate_uncompressed_size(MAX_PACKAGE_UNCOMPRESSED_BYTES, 1),
            Err(HpsError::ResourceLimit { .. })
        ));
    }

    #[test]
    fn corner_uvs_preserve_pre_split_smooth_normals() {
        let surface = build_surface(
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            vec![0, 1, 2, 0, 2, 3],
            None,
            Some(SurfaceTexture {
                corner_uvs: Some(vec![Some([0.0, 0.0]); 6]),
                texture: None,
            }),
        );

        let diagonal = std::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(
            surface.as_ref().map(|surface| surface.normals()),
            Ok(Some(
                [
                    [diagonal, 0.0, diagonal],
                    [0.0, 0.0, 1.0],
                    [diagonal, 0.0, diagonal],
                    [1.0, 0.0, 0.0],
                ]
                .as_slice()
            ))
        );
    }
}
