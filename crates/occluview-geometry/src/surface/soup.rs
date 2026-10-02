/// A borrowed triangle mesh: xyz triples, triangle indices, and an optional
/// per-vertex exclusion mask where a non-zero byte means "excluded from
/// matching".
#[derive(Clone, Copy, Debug)]
pub struct Soup<'a> {
    /// Vertex positions as consecutive xyz triples.
    pub positions: &'a [f32],
    /// Triangle indices into `positions`, three per triangle.
    pub indices: &'a [u32],
    /// Optional per-vertex exclusion mask; `None` means nothing is excluded.
    pub mask: Option<&'a [u8]>,
}

impl Soup<'_> {
    /// Number of whole vertices, ignoring any trailing partial triple.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Number of whole triangles.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Whether `vertex` is excluded from matching by the mask.
    #[must_use]
    pub fn is_excluded(&self, vertex: usize) -> bool {
        self.mask
            .is_some_and(|mask| mask.get(vertex).copied().unwrap_or(0) != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::Soup;

    #[test]
    fn counts_complete_position_and_index_groups() {
        let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 9.0];
        let indices = [0, 1, 2, 0, 1];
        let soup = Soup {
            positions: &positions,
            indices: &indices,
            mask: None,
        };
        assert_eq!(soup.vertex_count(), 3);
        assert_eq!(soup.triangle_count(), 1);
    }

    #[test]
    fn missing_and_short_masks_exclude_no_unmarked_vertices() {
        let positions = [0.0; 9];
        let indices = [0, 1, 2];
        let short = [1u8];
        assert!(!Soup {
            positions: &positions,
            indices: &indices,
            mask: None
        }
        .is_excluded(0));
        let masked = Soup {
            positions: &positions,
            indices: &indices,
            mask: Some(&short),
        };
        assert!(masked.is_excluded(0));
        assert!(!masked.is_excluded(2));
    }
}
