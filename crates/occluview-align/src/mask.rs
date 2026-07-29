//! The exclusion mask: which vertices stay out of matching.
//!
//! Every operation compares mesh vertices and brush points in the **same posed
//! frame**, so a mark lands where the operator sees it after the scan has
//! already been moved. Every operation is also bounds-safe against a mask that
//! does not match the mesh: a stale mask leaves the mesh alone instead of
//! painting the wrong vertices.

use glam::DVec3;
use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSlice};

use crate::sample::vertex_at;
use crate::Rigid;

/// The mask byte meaning "excluded from matching".
pub const EXCLUDED: u8 = 1;
/// The mask byte meaning "included in matching".
pub const INCLUDED: u8 = 0;

/// One brush dab in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskEdit {
    /// Brush centre in the posed (world) frame.
    pub center: DVec3,
    /// Brush radius in millimetres.
    pub radius_mm: f64,
    /// Erase instead of paint.
    pub erase: bool,
}

/// Vertices per parallel chunk. Large enough that scheduling disappears, small
/// enough that a dab on a million-vertex arch still spreads over every core.
const CHUNK: usize = 8_192;

/// Apply one dab, appending the vertices it changed to `touched`.
///
/// Returns how many mask bytes actually changed, which is what lets the caller
/// skip an upload when a dab landed on already-painted geometry.
///
/// `touched` is the reason this reports indices rather than a count: repainting
/// and re-uploading a whole arch per dab is tens of megabytes of traffic for a
/// brush the size of a cusp, and it is what made painting run at three frames a
/// second. With the indices in hand the caller rewrites and uploads only those
/// vertices.
///
/// Parallel, and deterministic with it: each chunk collects its own hits and
/// the chunks are applied in order, so the mask and `touched` come out
/// identical whatever the thread count.
pub fn apply_brush(
    mask: &mut [u8],
    positions: &[f32],
    pose: Rigid,
    edit: &MaskEdit,
    touched: &mut Vec<u32>,
) -> usize {
    if !edit.center.is_finite() || !edit.radius_mm.is_finite() || edit.radius_mm <= 0.0 {
        return 0;
    }
    let target = if edit.erase { INCLUDED } else { EXCLUDED };
    let limit = edit.radius_mm * edit.radius_mm;
    let hits: Vec<Vec<u32>> = mask
        .par_chunks(CHUNK)
        .enumerate()
        .map(|(chunk, slots)| {
            let base = chunk * CHUNK;
            let mut local = Vec::new();
            for (offset, slot) in slots.iter().enumerate() {
                if *slot == target {
                    continue;
                }
                let vertex = base + offset;
                let Some(position) = vertex_at(positions, vertex) else {
                    continue;
                };
                if (pose.apply(position) - edit.center).length_squared() <= limit {
                    if let Ok(index) = u32::try_from(vertex) {
                        local.push(index);
                    }
                }
            }
            local
        })
        .collect();

    let mut changed = 0usize;
    for chunk in &hits {
        for index in chunk {
            let Some(slot) = mask.get_mut(*index as usize) else {
                continue;
            };
            *slot = target;
            touched.push(*index);
            changed += 1;
        }
    }
    changed
}

/// Set every vertex at once — the "fit nowhere" and "fit everywhere" commands.
pub fn set_all(mask: &mut [u8], excluded: bool) {
    let value = if excluded { EXCLUDED } else { INCLUDED };
    for slot in mask.iter_mut() {
        *slot = value;
    }
}

/// Flip every mask byte.
pub fn invert(mask: &mut [u8]) {
    for slot in mask.iter_mut() {
        *slot = if *slot == INCLUDED {
            EXCLUDED
        } else {
            INCLUDED
        };
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_brush, invert, set_all, MaskEdit, CHUNK, EXCLUDED, INCLUDED};

    /// A dab that throws its touched-index list away, for the tests that only
    /// care about the mask it leaves behind.
    fn dab_into(mask: &mut [u8], positions: &[f32], pose: Rigid, edit: &MaskEdit) -> usize {
        apply_brush(mask, positions, pose, edit, &mut Vec::new())
    }
    use crate::Rigid;
    use glam::{DQuat, DVec3};

    /// Ten vertices spaced one millimetre apart along x.
    fn line(count: usize) -> Vec<f32> {
        (0..count)
            .flat_map(|index| {
                #[allow(clippy::cast_precision_loss)]
                let x = index as f32;
                [x, 0.0, 0.0]
            })
            .collect()
    }

    fn dab(center: f64, radius_mm: f64, erase: bool) -> MaskEdit {
        MaskEdit {
            center: DVec3::new(center, 0.0, 0.0),
            radius_mm,
            erase,
        }
    }

    #[test]
    fn the_brush_marks_only_vertices_inside_its_radius() {
        let positions = line(10);
        let mut mask = vec![INCLUDED; 10];
        let changed = dab_into(
            &mut mask,
            &positions,
            Rigid::IDENTITY,
            &dab(2.0, 1.5, false),
        );
        assert_eq!(changed, 3);
        assert_eq!(&mask[..5], &[0, 1, 1, 1, 0]);
    }

    #[test]
    fn a_second_identical_dab_changes_nothing() {
        let positions = line(10);
        let mut mask = vec![INCLUDED; 10];
        dab_into(
            &mut mask,
            &positions,
            Rigid::IDENTITY,
            &dab(5.0, 2.0, false),
        );
        let again = dab_into(
            &mut mask,
            &positions,
            Rigid::IDENTITY,
            &dab(5.0, 2.0, false),
        );
        assert_eq!(again, 0, "an unchanged dab must not report work");
    }

    #[test]
    fn erasing_undoes_painting() {
        let positions = line(10);
        let mut mask = vec![INCLUDED; 10];
        dab_into(
            &mut mask,
            &positions,
            Rigid::IDENTITY,
            &dab(5.0, 2.0, false),
        );
        dab_into(&mut mask, &positions, Rigid::IDENTITY, &dab(5.0, 2.0, true));
        assert!(mask.iter().all(|slot| *slot == INCLUDED));
    }

    #[test]
    fn the_brush_works_in_the_posed_frame() {
        let positions = line(10);
        let mut mask = vec![INCLUDED; 10];
        let pose = Rigid::new(DQuat::IDENTITY, DVec3::new(100.0, 0.0, 0.0));
        dab_into(&mut mask, &positions, pose, &dab(102.0, 1.5, false));
        assert_eq!(
            &mask[..5],
            &[0, 1, 1, 1, 0],
            "the brush must follow the scan that was moved"
        );
    }

    #[test]
    fn set_all_and_invert_are_exact_opposites() {
        let mut mask = vec![INCLUDED; 6];
        set_all(&mut mask, true);
        assert!(mask.iter().all(|slot| *slot == EXCLUDED));
        invert(&mut mask);
        assert!(mask.iter().all(|slot| *slot == INCLUDED));
    }

    #[test]
    fn a_short_mask_leaves_the_rest_of_the_mesh_alone() {
        let positions = line(10);
        let mut mask = vec![INCLUDED; 3];
        let changed = dab_into(
            &mut mask,
            &positions,
            Rigid::IDENTITY,
            &dab(8.0, 2.0, false),
        );
        assert_eq!(changed, 0, "a stale mask must not paint the wrong vertices");
    }

    #[test]
    fn a_non_finite_or_empty_brush_is_ignored() {
        let positions = line(10);
        let mut mask = vec![INCLUDED; 10];
        let nan = MaskEdit {
            center: DVec3::new(f64::NAN, 0.0, 0.0),
            radius_mm: 2.0,
            erase: false,
        };
        assert_eq!(dab_into(&mut mask, &positions, Rigid::IDENTITY, &nan), 0);
        assert_eq!(
            dab_into(
                &mut mask,
                &positions,
                Rigid::IDENTITY,
                &dab(5.0, 0.0, false)
            ),
            0
        );
        assert!(mask.iter().all(|slot| *slot == INCLUDED));
    }
    /// The indices are what lets the caller upload only what changed. A dab
    /// that reported the wrong ones would leave the surface showing a marking
    /// the mask does not have, or hiding one it does.
    #[test]
    fn a_dab_reports_exactly_the_vertices_it_changed() {
        let positions = line(10);
        let mut mask = vec![INCLUDED; 10];
        let mut touched = Vec::new();
        let changed = apply_brush(
            &mut mask,
            &positions,
            Rigid::IDENTITY,
            &dab(4.0, 1.5, false),
            &mut touched,
        );
        assert_eq!(changed, touched.len());
        assert_eq!(touched, vec![3, 4, 5]);
        for (vertex, slot) in mask.iter().enumerate() {
            let reported = u32::try_from(vertex).is_ok_and(|index| touched.contains(&index));
            assert_eq!(
                *slot == EXCLUDED,
                reported,
                "vertex {vertex} disagrees with what the dab reported"
            );
        }

        // A second dab over the same place changes nothing and reports nothing,
        // which is what lets the caller skip the upload entirely.
        touched.clear();
        let again = apply_brush(
            &mut mask,
            &positions,
            Rigid::IDENTITY,
            &dab(4.0, 1.5, false),
            &mut touched,
        );
        assert_eq!(again, 0);
        assert!(touched.is_empty());
    }

    /// Parallel, so the order has to be pinned: a caller uploading in the order
    /// it was handed must get the same bytes on every machine.
    #[test]
    fn the_reported_indices_come_out_in_vertex_order() {
        let positions = line(20_000);
        let mut mask = vec![INCLUDED; 20_000];
        let mut touched = Vec::new();
        apply_brush(
            &mut mask,
            &positions,
            Rigid::IDENTITY,
            &dab(10_000.0, 9_000.0, false),
            &mut touched,
        );
        assert!(touched.len() > CHUNK, "the dab must span several chunks");
        assert!(
            touched.windows(2).all(|pair| pair[0] < pair[1]),
            "the indices came back out of order"
        );
    }
}
