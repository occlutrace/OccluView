//! The exclusion mask: which vertices stay out of matching.
//!
//! Every operation compares mesh vertices and brush points in the **same posed
//! frame**, so a mark lands where the operator sees it after the scan has
//! already been moved. Every operation is also bounds-safe against a mask that
//! does not match the mesh: a stale mask leaves the mesh alone instead of
//! painting the wrong vertices.

use glam::DVec3;

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

/// Apply one dab, returning how many mask bytes actually changed.
///
/// The count is what lets the caller skip an upload when a dab landed on
/// already-painted geometry.
pub fn apply_brush(mask: &mut [u8], positions: &[f32], pose: Rigid, edit: &MaskEdit) -> usize {
    if !edit.center.is_finite() || !edit.radius_mm.is_finite() || edit.radius_mm <= 0.0 {
        return 0;
    }
    let target = if edit.erase { INCLUDED } else { EXCLUDED };
    let limit = edit.radius_mm * edit.radius_mm;
    let mut changed = 0usize;
    for (vertex, slot) in mask.iter_mut().enumerate() {
        let Some(local) = vertex_at(positions, vertex) else {
            continue;
        };
        if (pose.apply(local) - edit.center).length_squared() > limit {
            continue;
        }
        if *slot != target {
            *slot = target;
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

/// Paint a disc around each supplied point — the "mark around points" command
/// that pulls the clicked pairs out of the fit.
pub fn mark_around(
    mask: &mut [u8],
    positions: &[f32],
    pose: Rigid,
    points: &[DVec3],
    radius_mm: f64,
) -> usize {
    points
        .iter()
        .map(|&center| {
            apply_brush(
                mask,
                positions,
                pose,
                &MaskEdit {
                    center,
                    radius_mm,
                    erase: false,
                },
            )
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::{apply_brush, invert, mark_around, set_all, MaskEdit, EXCLUDED, INCLUDED};
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
        let changed = apply_brush(
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
        apply_brush(
            &mut mask,
            &positions,
            Rigid::IDENTITY,
            &dab(5.0, 2.0, false),
        );
        let again = apply_brush(
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
        apply_brush(
            &mut mask,
            &positions,
            Rigid::IDENTITY,
            &dab(5.0, 2.0, false),
        );
        apply_brush(&mut mask, &positions, Rigid::IDENTITY, &dab(5.0, 2.0, true));
        assert!(mask.iter().all(|slot| *slot == INCLUDED));
    }

    #[test]
    fn the_brush_works_in_the_posed_frame() {
        let positions = line(10);
        let mut mask = vec![INCLUDED; 10];
        let pose = Rigid::new(DQuat::IDENTITY, DVec3::new(100.0, 0.0, 0.0));
        apply_brush(&mut mask, &positions, pose, &dab(102.0, 1.5, false));
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
    fn mark_around_covers_every_supplied_point() {
        let positions = line(10);
        let mut mask = vec![INCLUDED; 10];
        mark_around(
            &mut mask,
            &positions,
            Rigid::IDENTITY,
            &[DVec3::new(1.0, 0.0, 0.0), DVec3::new(8.0, 0.0, 0.0)],
            1.0,
        );
        assert_eq!(mask[1], EXCLUDED);
        assert_eq!(mask[8], EXCLUDED);
        assert_eq!(mask[4], INCLUDED);
    }

    #[test]
    fn a_short_mask_leaves_the_rest_of_the_mesh_alone() {
        let positions = line(10);
        let mut mask = vec![INCLUDED; 3];
        let changed = apply_brush(
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
        assert_eq!(apply_brush(&mut mask, &positions, Rigid::IDENTITY, &nan), 0);
        assert_eq!(
            apply_brush(
                &mut mask,
                &positions,
                Rigid::IDENTITY,
                &dab(5.0, 0.0, false)
            ),
            0
        );
        assert!(mask.iter().all(|slot| *slot == INCLUDED));
    }
}
