//! Neutral decoded surface data.

use crate::HpsError;

/// Decoded RGBA8 texture pixels independent of any renderer or mesh type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedTexture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl DecodedTexture {
    /// Construct a decoded texture after validating its dimensions and byte length.
    ///
    /// # Errors
    /// Returns [`HpsError::TextureMalformed`] when a dimension is zero or
    /// `rgba` is not exactly `width * height * 4` bytes.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, HpsError> {
        if width == 0 || height == 0 {
            return Err(texture_malformed("texture dimensions must be non-zero"));
        }
        let expected_len = usize::try_from(
            u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or(HpsError::ResourceLimit {
                    resource: "texture RGBA size",
                    limit: u64::MAX,
                })?,
        )
        .map_err(|_| HpsError::ResourceLimit {
            resource: "texture RGBA size",
            limit: usize::MAX as u64,
        })?;
        if rgba.len() != expected_len {
            return Err(texture_malformed(format!(
                "texture dimensions require {expected_len} RGBA bytes, got {}",
                rgba.len()
            )));
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// Texture width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Texture height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Row-major RGBA8 pixels.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Consume the texture into dimensions and owned RGBA8 pixels.
    #[must_use]
    pub fn into_parts(self) -> (u32, u32, Vec<u8>) {
        (self.width, self.height, self.rgba)
    }
}

/// Product-neutral triangle surface decoded from HPS data.
///
/// Positions and indices always describe the source CAD topology. Texture UVs
/// are kept separately as corner attributes because a single source vertex can
/// legitimately have different UVs on adjacent triangle corners. Renderers may
/// split those corners later; geometry exporters must not inherit that split.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedSurface {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    colors: Option<Vec<[u8; 4]>>,
    /// Optional legacy per-vertex UVs for formats whose UV model is indexed by
    /// position rather than by triangle corner.
    uvs: Option<Vec<[f32; 2]>>,
    /// Optional HPS UVs in triangle-corner order. Length equals `indices.len()`.
    corner_uvs: Option<Vec<Option<[f32; 2]>>>,
    normals: Option<Vec<[f32; 3]>>,
    texture: Option<DecodedTexture>,
}

/// Owned arrays produced by consuming a validated [`DecodedSurface`].
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedSurfaceParts {
    /// Finite XYZ vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Complete triangle indices into `positions`.
    pub indices: Vec<u32>,
    /// Optional per-vertex RGBA8 colors.
    pub colors: Option<Vec<[u8; 4]>>,
    /// Optional per-vertex UVs for legacy indexed UV data.
    pub uvs: Option<Vec<[f32; 2]>>,
    /// Optional HPS UVs in triangle-corner order.
    pub corner_uvs: Option<Vec<Option<[f32; 2]>>>,
    /// Optional finite per-source-vertex normals.
    pub normals: Option<Vec<[f32; 3]>>,
    /// Optional decoded RGBA8 texture.
    pub texture: Option<DecodedTexture>,
}

impl DecodedSurface {
    /// Construct a validated triangle surface.
    ///
    /// # Errors
    /// Returns a typed parser error for non-finite positions, malformed triangle
    /// indices, mismatched optional attributes, or non-finite UV coordinates.
    pub fn new(
        positions: Vec<[f32; 3]>,
        indices: Vec<u32>,
        colors: Option<Vec<[u8; 4]>>,
        uvs: Option<Vec<[f32; 2]>>,
        texture: Option<DecodedTexture>,
    ) -> Result<Self, HpsError> {
        if positions
            .iter()
            .flatten()
            .any(|component| !component.is_finite())
        {
            return Err(bad_container("surface positions must be finite"));
        }
        if !indices.len().is_multiple_of(3) {
            return Err(bad_container(
                "surface index count must contain complete triangles",
            ));
        }
        if indices
            .iter()
            .any(|index| usize::try_from(*index).map_or(true, |index| index >= positions.len()))
        {
            return Err(bad_container("surface triangle index is out of bounds"));
        }
        if colors
            .as_ref()
            .is_some_and(|colors| colors.len() != positions.len())
        {
            return Err(bad_container(
                "vertex color count does not match position count",
            ));
        }
        if let Some(uvs) = &uvs {
            if uvs.len() != positions.len() {
                return Err(texture_malformed("UV count does not match position count"));
            }
            if uvs.iter().flatten().any(|component| !component.is_finite()) {
                return Err(texture_malformed("UV coordinates must be finite"));
            }
        }

        Ok(Self {
            positions,
            indices,
            colors,
            uvs,
            corner_uvs: None,
            normals: None,
            texture,
        })
    }

    /// Attach finite per-vertex normals to the validated surface.
    ///
    /// # Errors
    /// Returns [`HpsError::BadContainer`] when the normal count differs
    /// from the position count or any component is non-finite.
    pub fn with_normals(mut self, normals: Vec<[f32; 3]>) -> Result<Self, HpsError> {
        if normals.len() != self.positions.len() {
            return Err(bad_container(
                "vertex normal count does not match position count",
            ));
        }
        if normals
            .iter()
            .flatten()
            .any(|component| !component.is_finite())
        {
            return Err(bad_container("surface normals must be finite"));
        }
        self.normals = Some(normals);
        Ok(self)
    }

    /// Attach optional UV coordinates in triangle-corner order.
    ///
    /// The source HPS representation permits one position to have a different
    /// UV on different incident faces. Keeping this data indexed by corners
    /// preserves the source topology until a render-specific adapter chooses
    /// where seams must be split.
    ///
    /// # Errors
    /// Returns [`HpsError::TextureMalformed`] when the corner count differs
    /// from the index count or a provided UV is non-finite.
    pub fn with_corner_uvs(mut self, corner_uvs: Vec<Option<[f32; 2]>>) -> Result<Self, HpsError> {
        if corner_uvs.len() != self.indices.len() {
            return Err(texture_malformed(
                "corner UV count does not match triangle corner count",
            ));
        }
        if corner_uvs
            .iter()
            .flatten()
            .flatten()
            .any(|component| !component.is_finite())
        {
            return Err(texture_malformed("corner UV coordinates must be finite"));
        }
        self.corner_uvs = Some(corner_uvs);
        Ok(self)
    }

    /// Vertex positions as finite XYZ triples.
    #[must_use]
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.positions
    }

    /// Triangle indices into [`Self::positions`].
    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Optional per-vertex RGBA8 colors.
    #[must_use]
    pub fn colors(&self) -> Option<&[[u8; 4]]> {
        self.colors.as_deref()
    }

    /// Optional per-vertex UVs for legacy indexed UV data.
    #[must_use]
    pub fn uvs(&self) -> Option<&[[f32; 2]]> {
        self.uvs.as_deref()
    }

    /// Optional HPS UVs in triangle-corner order.
    #[must_use]
    pub fn corner_uvs(&self) -> Option<&[Option<[f32; 2]>]> {
        self.corner_uvs.as_deref()
    }

    /// Optional finite per-source-vertex normals.
    #[must_use]
    pub fn normals(&self) -> Option<&[[f32; 3]]> {
        self.normals.as_deref()
    }

    /// Optional decoded RGBA8 texture.
    #[must_use]
    pub fn texture(&self) -> Option<&DecodedTexture> {
        self.texture.as_ref()
    }

    /// Consume the surface into its owned neutral arrays.
    #[must_use]
    pub fn into_parts(self) -> DecodedSurfaceParts {
        DecodedSurfaceParts {
            positions: self.positions,
            indices: self.indices,
            colors: self.colors,
            uvs: self.uvs,
            corner_uvs: self.corner_uvs,
            normals: self.normals,
            texture: self.texture,
        }
    }
}

fn bad_container(reason: impl Into<String>) -> HpsError {
    HpsError::BadContainer {
        reason: reason.into(),
    }
}

fn texture_malformed(reason: impl Into<String>) -> HpsError {
    HpsError::TextureMalformed {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{DecodedSurface, DecodedTexture};

    fn triangle() -> Result<DecodedSurface, crate::HpsError> {
        DecodedSurface::new(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![0, 1, 2],
            None,
            None,
            None,
        )
    }

    #[test]
    fn accepts_a_valid_untextured_surface() {
        let surface = triangle();
        assert_eq!(
            surface.as_ref().map(|surface| surface.positions().len()),
            Ok(3)
        );
        assert_eq!(
            surface.as_ref().map(DecodedSurface::indices),
            Ok([0, 1, 2].as_slice())
        );
        assert_eq!(surface.as_ref().map(DecodedSurface::colors), Ok(None));
        assert_eq!(surface.as_ref().map(DecodedSurface::uvs), Ok(None));
        assert_eq!(surface.as_ref().map(DecodedSurface::texture), Ok(None));
    }

    #[test]
    fn rejects_non_finite_positions() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let result = DecodedSurface::new(
                vec![[bad, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                vec![0, 1, 2],
                None,
                None,
                None,
            );
            assert!(result.is_err(), "position component {bad:?} must fail");
        }
    }

    #[test]
    fn rejects_non_triangular_or_out_of_range_indices() {
        assert!(
            DecodedSurface::new(vec![[0.0, 0.0, 0.0]; 3], vec![0, 1], None, None, None,).is_err()
        );
        assert!(
            DecodedSurface::new(vec![[0.0, 0.0, 0.0]; 3], vec![0, 1, 3], None, None, None,)
                .is_err()
        );
    }

    #[test]
    fn optional_rgba_colors_match_vertex_count() {
        assert!(DecodedSurface::new(
            vec![[0.0, 0.0, 0.0]; 3],
            vec![0, 1, 2],
            Some(vec![[255, 0, 0, 255]; 3]),
            None,
            None,
        )
        .is_ok());
        assert!(DecodedSurface::new(
            vec![[0.0, 0.0, 0.0]; 3],
            vec![0, 1, 2],
            Some(vec![[255, 0, 0, 255]; 2]),
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn per_vertex_uvs_match_seam_split_vertex_count() {
        assert!(DecodedSurface::new(
            vec![[0.0, 0.0, 0.0]; 3],
            vec![0, 1, 2],
            None,
            Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
            None,
        )
        .is_ok());
        assert!(DecodedSurface::new(
            vec![[0.0, 0.0, 0.0]; 3],
            vec![0, 1, 2],
            None,
            Some(vec![[0.0, 0.0]; 2]),
            None,
        )
        .is_err());
    }

    #[test]
    fn texture_dimensions_match_rgba_length() {
        assert!(DecodedTexture::new(2, 2, vec![255; 16]).is_ok());
        assert!(DecodedTexture::new(2, 2, vec![255; 15]).is_err());
        assert!(DecodedTexture::new(0, 2, Vec::new()).is_err());
    }
}
