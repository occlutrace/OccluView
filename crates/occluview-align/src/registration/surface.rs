//! One scan as the search reads it: even clouds of its eligible surface at
//! two spacings and, when they fit in memory, its triangles themselves.

use super::cloud::{Cloud, Gather};
use super::seat::{Exact, Probe};
use crate::sample::vertex_at;
use crate::{MeshInput, RegionPolicy, SurfaceIndex};
use glam::{DAffine3, DVec3};
use occluview_geometry::surface::{BuildOutcome, GeometryControl};

/// Where and how finely the scans of one search are read.
#[derive(Clone, Copy, Debug)]
pub(super) struct Layout {
    /// World point taken off every coordinate: the centre of the fixed scan.
    pub origin: DVec3,
    /// Spacing of the fine and of the coarse clouds, in millimetres.
    pub fine_mm: f64,
    pub coarse_mm: f64,
    pub regions: RegionPolicy,
}

/// Eligible triangles of a mesh in world coordinates with `origin` taken off.
/// `omitted` counts triangles that name a vertex the mesh does not have.
fn triangles<'a>(
    mesh: MeshInput<'a>,
    regions: RegionPolicy,
    origin: DVec3,
    omitted: &'a mut usize,
) -> impl Iterator<Item = [DVec3; 3]> + 'a {
    mesh.soup
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(move |ids| {
            if ids
                .iter()
                .any(|&id| id as usize >= mesh.soup.vertex_count())
            {
                *omitted += 1;
                return None;
            }
            crate::mask::eligible_region(mesh.soup, *ids, regions)?;
            let corner = |id: u32| {
                vertex_at(mesh.soup.positions, id as usize)
                    .map(|point| mesh.world_from_local.transform_point3(point) - origin)
                    .filter(|point| point.is_finite())
            };
            Some([corner(ids[0])?, corner(ids[1])?, corner(ids[2])?])
        })
}

/// Area and area-weighted centre of the eligible surface, in world.
pub(super) fn extent(mesh: MeshInput<'_>, regions: RegionPolicy) -> (f64, DVec3) {
    let (mut area, mut centre, mut omitted) = (0., DVec3::ZERO, 0);
    for [first, second, third] in triangles(mesh, regions, DVec3::ZERO, &mut omitted) {
        let piece = (second - first).cross(third - first).length() * 0.5;
        if piece.is_finite() && piece > 0. {
            area += piece;
            centre += ((first + second + third) / 3. - centre) * (piece / area);
        }
    }
    (area, centre)
}

/// One scan, read.
pub(super) struct Side {
    pub fine: Cloud,
    pub coarse: Cloud,
    pub area: f64,
    /// Triangles left out for naming a vertex the mesh does not have.
    pub omitted: usize,
    exact: Option<SurfaceIndex>,
    /// Lifetime and memory admission of `exact` and of queries on it.
    admission: GeometryControl,
}

impl Side {
    /// Read `mesh`. Its triangles are indexed when `admission` has room for
    /// them and the clock allows; otherwise the clouds stand for them.
    pub(super) fn read(mesh: MeshInput<'_>, layout: &Layout, admission: GeometryControl) -> Self {
        let mut omitted = 0;
        let mut gather = Gather::new(layout.fine_mm);
        for corners in triangles(mesh, layout.regions, layout.origin, &mut omitted) {
            gather.add_triangle(corners);
        }
        let area = gather.area();
        let fine = gather.finish();
        let coarse = fine.coarser(layout.coarse_mm);
        let frame = DAffine3 {
            matrix3: mesh.world_from_local.matrix3,
            translation: mesh.world_from_local.translation - layout.origin,
        };
        let exact = match SurfaceIndex::build_controlled(mesh.soup, frame, &admission) {
            BuildOutcome::Complete(index) => Some(index),
            BuildOutcome::Partial { .. } | BuildOutcome::Empty => None,
        };
        Self {
            fine,
            coarse,
            area,
            omitted,
            exact,
            admission,
        }
    }

    /// The triangles themselves, when they were indexed.
    pub(super) fn exact(&self) -> Option<Exact<'_>> {
        self.exact.as_ref().map(|index| Exact {
            index,
            control: &self.admission,
        })
    }

    /// Whether the triangles face one way across shared edges; unknown when
    /// they were not indexed.
    pub(super) fn coherent(&self) -> Option<bool> {
        self.exact.as_ref().map(SurfaceIndex::orientation_coherent)
    }
}

/// At most `most` probes of a cloud at an even stride, each standing on the
/// source surface and for the area of the points the stride passes over.
pub(super) fn probes(cloud: &Cloud, most: usize) -> Vec<Probe> {
    let stride = stride(cloud, most);
    #[allow(clippy::cast_precision_loss)]
    let scale = stride as f64;
    cloud
        .points
        .iter()
        .step_by(stride)
        .map(|point| Probe {
            position: point.anchor,
            normal: point.normal,
            area: point.area * scale,
        })
        .collect()
}

/// The stride at which [`probes`] takes at most `most` points of a cloud.
pub(super) fn stride(cloud: &Cloud, most: usize) -> usize {
    cloud.points.len().div_ceil(most.max(1)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SearchControl, SearchSettings, Soup};

    fn layout() -> Layout {
        Layout {
            origin: DVec3::new(1., 2., 3.),
            fine_mm: 0.3,
            coarse_mm: 1.,
            regions: RegionPolicy::AllEligible,
        }
    }

    #[test]
    fn a_scan_is_read_in_the_work_frame_without_its_excluded_part() {
        // Two unit right triangles; the second is excluded by its own vertex.
        let positions = [
            0., 0., 0., 1., 0., 0., 0., 1., 0., //
            5., 5., 5., 6., 5., 5., 5., 6., 5., //
        ];
        let indices = [0, 1, 2, 3, 4, 5, 0, 1, 99];
        let mask = [0, 0, 0, 1, 0, 0];
        let mesh = MeshInput {
            soup: Soup {
                positions: &positions,
                indices: &indices,
                mask: Some(&mask),
            },
            world_from_local: DAffine3::from_translation(DVec3::new(10., 0., 0.)),
            revision: 0,
        };
        let (area, centre) = extent(mesh, RegionPolicy::AllEligible);
        assert!((area - 0.5).abs() < 1e-12);
        assert!(centre.distance(DVec3::new(10. + 1. / 3., 1. / 3., 0.)) < 1e-12);
        let admission = SearchControl::default().surface_control(&SearchSettings::default());
        let side = Side::read(mesh, &layout(), admission);
        assert!((side.area - 0.5).abs() < 1e-9);
        assert_eq!(side.omitted, 1);
        assert!(side.exact().is_some());
        for point in &side.fine.points {
            // World x 10..11 less the origin's 1, world y 0..1 less 2.
            assert!((9.0..=10.0).contains(&point.position.x));
            assert!((-2.0..=-1.0).contains(&point.position.y));
            assert!((point.position.z + 3.).abs() < 1e-12);
        }
    }

    #[test]
    fn probes_keep_the_area_of_the_points_they_pass_over() {
        let mut gather = Gather::new(0.5);
        for i in 0..40u32 {
            let x = f64::from(i) * 0.5;
            gather.add_triangle([
                DVec3::new(x, 0., 0.),
                DVec3::new(x + 0.5, 0., 0.),
                DVec3::new(x, 0.5, 0.),
            ]);
        }
        let cloud = gather.finish();
        let all = probes(&cloud, usize::MAX);
        let some = probes(&cloud, 10);
        assert_eq!(all.len(), cloud.points.len());
        assert!(some.len() <= 10);
        let total = |set: &[Probe]| set.iter().map(|probe| probe.area).sum::<f64>();
        assert!((total(&all) - total(&some)).abs() < 1e-9);
    }
}
