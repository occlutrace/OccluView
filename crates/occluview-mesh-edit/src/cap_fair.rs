//! Cap shape: the surface that carries the scan across a hole.
//!
//! The refined cap from `cap_refine` is flat between its rim vertices. This
//! module moves its interior onto the thin plate that leaves the rim the way
//! the scan arrives at it (Liepa 2003, "Filling Holes in Meshes"): the surface
//! that bends least, where bending is measured at every edge of the cap and at
//! every rim edge, whose hinge has the cap on one side and the scan on the
//! other. A cut tooth is closed by a dome that rises along its walls, a socket
//! in the gum by a surface that goes on as the gum does, and neither gets a
//! crease at the seam or a flat lid.
//!
//! Two things make that solve converge, where a fixed number of sweeps over
//! the finished cap moves little more than the ring next to the rim:
//!
//! * Bending is the quadratic hinge energy of Bergou et al. 2006 ("A Quadratic
//!   Bending Model for Inextensible Surfaces"), with the angles of the flat cap
//!   and of the scan triangles at the rim. It is zero exactly where two
//!   triangles go straight on, whatever their shape. A lasso cut leaves a rim
//!   of teeth and notches, and on a coarse cap a rim vertex has cap neighbors
//!   many times further away than its scan neighbors; a Laplacian taken at
//!   the rim's vertices reads both as curvature and wrinkles the cap to match.
//! * The cap is shaped coarse to fine, on the levels its refinement went
//!   through. Each level starts from the one below, so a sweep only ever has
//!   to settle detail at the scale of its own edges, which is what a sweep is
//!   quick at.
//!
//! Every sweep moves each interior vertex to the minimum of the energy along
//! its own coordinates, so the energy never rises and no step size is tuned.
//! Order is fixed, so the result is deterministic.

use super::cap_refine::RefinedCap;
use crate::numeric::count_as_f32;
use glam::Vec3;

/// Hard bound on sweeps per level (safety valve; tolerance exits earlier).
const MAX_FAIR_SWEEPS: usize = 160;
/// Convergence: stop when no vertex moved more than this fraction of the
/// cap's bounding-sphere-ish scale during a sweep.
const FAIR_TOLERANCE_FACTOR: f32 = 1e-5;
/// Largest cotangent a corner contributes. A sliver's cotangent runs to
/// infinity as it flattens; past this it says nothing about the surface.
const COTANGENT_LIMIT: f32 = 1.0e3;

/// What the scan adds to the hinge at one rim edge: its side of the hinge,
/// fixed while the cap is shaped.
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct RimSupport {
    /// The scan triangle's conormal at the rim edge: in the triangle's plane,
    /// square to the edge, toward the triangle, as long as the edge.
    pub(super) conormal: Vec3,
    /// Area of the scan triangle; zero where the rim edge has none.
    pub(super) area: f32,
}

impl RimSupport {
    /// The support of the scan triangle `(from, to, apex)` across the rim edge
    /// `from..to`.
    pub(super) fn across(from: Vec3, to: Vec3, apex: Vec3) -> Self {
        Face::at([from, to, apex]).map_or_else(Self::default, |face| Self {
            conormal: face.cotangents[0] * (apex - to) + face.cotangents[1] * (apex - from),
            area: face.area,
        })
    }
}

/// Cotangents of a triangle's three angles and its area.
struct Face {
    cotangents: [f32; 3],
    area: f32,
}

impl Face {
    /// `None` for a triangle with no area or no finite corners: a rim can
    /// carry two vertices at one position, and such a triangle has no angles.
    fn at(points: [Vec3; 3]) -> Option<Self> {
        let doubled = (points[1] - points[0])
            .cross(points[2] - points[0])
            .length();
        if !(doubled.is_finite() && doubled > 0.0) {
            return None;
        }
        let cotangents = [0, 1, 2].map(|corner| {
            let (to_next, to_last) = (
                points[(corner + 1) % 3] - points[corner],
                points[(corner + 2) % 3] - points[corner],
            );
            (to_next.dot(to_last) / doubled).clamp(-COTANGENT_LIMIT, COTANGENT_LIMIT)
        });
        Some(Self {
            cotangents,
            area: doubled * 0.5,
        })
    }
}

/// How a cap meets the scan at its rim.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum Continuity {
    /// The cap leaves the rim in the direction the scan arrives: the rim's
    /// hinges count.
    Tangent,
    /// The cap only shares the rim, free to turn on it: a soap film. It stays
    /// inside the rim's hull, so it is the shape to fall back on where the
    /// tangent one would run into the scan.
    Position,
}

/// Positions for the generated vertices of `cap`, in their order. `rim` holds
/// the rim positions in ring order and `support[i]` what the scan adds at the
/// rim edge from vertex `i` to the next.
pub(super) fn shape_cap(
    rim: &[Vec3],
    cap: &RefinedCap,
    support: &[RimSupport],
    continuity: Continuity,
) -> Vec<Vec3> {
    let rim_len = rim.len();
    // Work around the rim's centroid, in units of the cap's size: the sums
    // below then neither cancel on a cap far from the origin nor overflow on
    // one of any size.
    let mut origin = Vec3::ZERO;
    for &point in rim {
        origin += point;
    }
    origin /= count_as_f32(rim_len.max(1));
    let centred: Vec<Vec3> = rim
        .iter()
        .copied()
        .chain(
            cap.generated
                .iter()
                .map(|vertex| Vec3::from_array(vertex.position)),
        )
        .map(|point| point - origin)
        .collect();
    let scale = cap_scale(&centred);
    let flat: Vec<Vec3> = centred.into_iter().map(|point| point / scale).collect();
    let support: Vec<RimSupport> = match continuity {
        Continuity::Tangent => support
            .iter()
            .map(|support| RimSupport {
                conormal: support.conormal / scale,
                area: support.area / (scale * scale),
            })
            .collect(),
        Continuity::Position => Vec::new(),
    };

    let mut positions = flat.clone();
    let mut placed = rim_len;
    for level in &cap.levels {
        let count = level.vertex_count.min(positions.len());
        if count <= rim_len {
            continue;
        }
        // A new vertex starts midway between its parents, where the level
        // below left them.
        for vertex in placed.max(rim_len)..count {
            if let Some(&[a, b]) = cap.parents.get(vertex - rim_len) {
                positions[vertex] = (positions[a] + positions[b]) * 0.5;
            }
        }
        placed = count;
        LevelHinges::build(&flat[..count], rim_len, &level.triangles, &support)
            .relax(&mut positions[..count]);
    }
    positions
        .into_iter()
        .skip(rim_len)
        .map(|point| point * scale + origin)
        .collect()
}

/// One side of a cap edge: the triangle on it and which of the triangle's
/// corners faces the edge.
#[derive(Copy, Clone)]
struct Side {
    edge: (usize, usize),
    face: usize,
    apex: usize,
}

/// The hinges of one level of a cap: for every edge with a triangle on both
/// sides, how far the two are from going straight on, as a weighted sum of
/// their four corners. Read off the flat cap once and fixed while the level
/// is shaped.
struct LevelHinges {
    rim_len: usize,
    /// What a hinge's bend counts for: three over the area of its triangles.
    weights: Vec<f32>,
    /// The scan's side of a rim hinge; zero for a hinge inside the cap.
    fixed: Vec<Vec3>,
    /// Start of each vertex's entries in `corners`; one past the end last.
    offsets: Vec<usize>,
    /// For every vertex, the hinges it is a corner of and its coefficient in
    /// each.
    corners: Vec<(usize, f32)>,
}

impl LevelHinges {
    fn build(
        flat: &[Vec3],
        rim_len: usize,
        triangles: &[[usize; 3]],
        support: &[RimSupport],
    ) -> Self {
        let count = flat.len();
        let faces: Vec<Option<Face>> = triangles
            .iter()
            .map(|triangle| {
                triangle
                    .iter()
                    .all(|&vertex| vertex < count)
                    .then(|| Face::at(triangle.map(|vertex| flat[vertex])))
                    .flatten()
            })
            .collect();
        // Every side of every triangle, sorted so the two sides of an edge
        // end up together.
        let mut sides: Vec<Side> = Vec::with_capacity(triangles.len() * 3);
        for (face, triangle) in triangles.iter().enumerate() {
            if faces[face].is_none() {
                continue;
            }
            for apex in 0..3 {
                let (u, v) = (triangle[(apex + 1) % 3], triangle[(apex + 2) % 3]);
                sides.push(Side {
                    edge: (u.min(v), u.max(v)),
                    face,
                    apex,
                });
            }
        }
        sides.sort_unstable_by_key(|side| (side.edge, side.face));

        let mut weights: Vec<f32> = Vec::new();
        let mut fixed: Vec<Vec3> = Vec::new();
        // (vertex, hinge, coefficient), gathered per hinge and sorted per
        // vertex below.
        let mut entries: Vec<(usize, usize, f32)> = Vec::new();
        for edge_sides in sides.chunk_by(|left, right| left.edge == right.edge) {
            let mut area = 0.0_f32;
            let mut scan = Vec3::ZERO;
            match *edge_sides {
                [_, _] => {}
                // A rim edge: the scan is the other side, where it gives one.
                [Side { edge: (u, v), .. }] if v < rim_len => {
                    let rim_edge = if v == u + 1 {
                        u
                    } else if u == 0 && v + 1 == rim_len {
                        v
                    } else {
                        continue;
                    };
                    match support.get(rim_edge) {
                        Some(support) if support.area > 0.0 => {
                            area = support.area;
                            scan = support.conormal;
                        }
                        _ => continue,
                    }
                }
                _ => continue,
            }
            let hinge = weights.len();
            for side in edge_sides {
                let Some(face) = faces[side.face].as_ref() else {
                    continue;
                };
                let triangle = triangles[side.face];
                let (u, v) = ((side.apex + 1) % 3, (side.apex + 2) % 3);
                // The triangle's conormal at the edge: each end of the edge
                // pulls the apex by the cotangent at the other end.
                entries.push((triangle[u], hinge, -face.cotangents[v]));
                entries.push((triangle[v], hinge, -face.cotangents[u]));
                entries.push((
                    triangle[side.apex],
                    hinge,
                    face.cotangents[u] + face.cotangents[v],
                ));
                area += face.area;
            }
            weights.push(3.0 / area);
            fixed.push(scan);
        }

        entries.sort_unstable_by_key(|&(vertex, hinge, _)| (vertex, hinge));
        let mut offsets = vec![0_usize; count + 1];
        for &(vertex, _, _) in &entries {
            offsets[vertex + 1] += 1;
        }
        for vertex in 0..count {
            offsets[vertex + 1] += offsets[vertex];
        }
        Self {
            rim_len,
            weights,
            fixed,
            offsets,
            corners: entries
                .into_iter()
                .map(|(_, hinge, coefficient)| (hinge, coefficient))
                .collect(),
        }
    }

    /// Move the interior to the minimum of the bending summed over the
    /// level's hinges.
    fn relax(&self, positions: &mut [Vec3]) {
        let count = positions.len();
        let mut bend = self.fixed.clone();
        for (vertex, &position) in positions.iter().enumerate() {
            for &(hinge, coefficient) in
                &self.corners[self.offsets[vertex]..self.offsets[vertex + 1]]
            {
                bend[hinge] += coefficient * position;
            }
        }
        // How fast the energy grows as an interior vertex moves.
        let curvature: Vec<f32> = (0..count)
            .map(|vertex| {
                self.corners[self.offsets[vertex]..self.offsets[vertex + 1]]
                    .iter()
                    .map(|&(hinge, coefficient)| self.weights[hinge] * coefficient * coefficient)
                    .sum()
            })
            .collect();

        for _ in 0..MAX_FAIR_SWEEPS {
            let mut max_move = 0.0_f32;
            for vertex in self.rim_len..count {
                if !(curvature[vertex].is_finite() && curvature[vertex] > 0.0) {
                    continue;
                }
                let corners = &self.corners[self.offsets[vertex]..self.offsets[vertex + 1]];
                let mut slope = Vec3::ZERO;
                for &(hinge, coefficient) in corners {
                    slope += self.weights[hinge] * coefficient * bend[hinge];
                }
                let step = -slope / curvature[vertex];
                if !step.is_finite() {
                    continue;
                }
                positions[vertex] += step;
                for &(hinge, coefficient) in corners {
                    bend[hinge] += coefficient * step;
                }
                max_move = max_move.max(step.length());
            }
            if max_move <= FAIR_TOLERANCE_FACTOR {
                break;
            }
        }
    }
}

/// A representative geometric scale for the cap: half the bounding-box
/// diagonal. The cap is shaped in units of it, which is also what makes the
/// convergence tolerance a fraction of it.
fn cap_scale(positions: &[Vec3]) -> f32 {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for &p in positions {
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let diagonal = (hi - lo).length();
    if diagonal.is_finite() && diagonal > 0.0 {
        diagonal * 0.5
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cap_minweight::TakenTriangles;
    use crate::cap_refine::{refine_cap, CapDomain, CapLevel};
    use crate::{EditVertex, GeneratedVertexPolicy};

    /// A round rim of `rim_len` vertices at height zero, its refined cap, and
    /// the support of a band of scan triangles from the rim out to the ring at
    /// `outside(angle)`.
    fn round_cap(
        rim_len: usize,
        radius: f32,
        outside: impl Fn(f32) -> Vec3,
    ) -> (Vec<Vec3>, RefinedCap, Vec<RimSupport>) {
        let angle =
            |index: usize| std::f32::consts::TAU * ((index % rim_len) as f32) / (rim_len as f32);
        let on_rim = |index: usize| {
            Vec3::new(
                radius * angle(index).cos(),
                radius * angle(index).sin(),
                0.0,
            )
        };
        let rim: Vec<Vec3> = (0..rim_len).map(on_rim).collect();
        let vertices: Vec<EditVertex> = rim
            .iter()
            .map(|point| EditVertex::at(point.to_array()))
            .collect();
        let fan: Vec<[usize; 3]> = (1..rim_len - 1).map(|i| [0, i, i + 1]).collect();
        let cap = refine_cap(
            &vertices,
            fan,
            CapDomain::Plane,
            &TakenTriangles::new(),
            GeneratedVertexPolicy::InterpolateBoundary,
        );
        let support = (0..rim_len)
            .map(|index| {
                RimSupport::across(on_rim(index), on_rim(index + 1), outside(angle(index + 1)))
            })
            .collect();
        (rim, cap, support)
    }

    /// A tube cut square across: the scan arrives at the rim straight up the
    /// wall. The cap has to go on up and close over, a dome, and its middle
    /// has to stand well above the rim. A fixed number of sweeps over the
    /// finished cap leaves this a lid with a rounded edge.
    #[test]
    fn a_cut_tube_is_closed_by_a_dome_that_continues_its_wall() {
        let (radius, edge) = (4.0_f32, 0.2_f32);
        let rim_len = 126; // edge ~0.2 mm, like the wall below it
        let (rim, cap, support) = round_cap(rim_len, radius, |angle| {
            Vec3::new(radius * angle.cos(), radius * angle.sin(), -edge)
        });
        assert!(cap.generated.len() > 1_000, "{}", cap.generated.len());

        let dome = shape_cap(&rim, &cap, &support, Continuity::Tangent);
        let top = dome.iter().map(|point| point.z).fold(f32::MIN, f32::max);
        assert!(
            top > radius * 0.3,
            "the cap rose {top:.2} mm over a {radius} mm tube: a lid, not a dome"
        );
        // It rises from the rim: next to it the cap goes up the wall's way.
        let mut near_rim = 0;
        for point in &dome {
            let from_axis = point.truncate().length();
            if from_axis > radius - 1.5 * edge {
                near_rim += 1;
                assert!(point.z > 0.0, "the cap dips at the rim: {point:?}");
                assert!(from_axis < radius + edge, "the cap bulges out: {point:?}");
            }
        }
        assert!(near_rim > rim_len / 2);

        let film = shape_cap(&rim, &cap, &support, Continuity::Position);
        assert!(
            film.iter().all(|point| point.z.abs() < 1e-3),
            "a soap film over a flat rim is flat"
        );
    }

    /// A flat sheet with a hole: the scan arrives level, and the cap stays in
    /// the sheet.
    #[test]
    fn a_hole_in_a_flat_sheet_is_closed_flat() {
        let radius = 3.0_f32;
        let (rim, cap, support) = round_cap(64, radius, |angle| {
            Vec3::new(
                (radius + 0.3) * angle.cos(),
                (radius + 0.3) * angle.sin(),
                0.0,
            )
        });
        let flat = shape_cap(&rim, &cap, &support, Continuity::Tangent);
        assert!(!flat.is_empty());
        assert!(flat.iter().all(|point| point.z.abs() < 1e-3));
        assert!(flat.iter().all(|point| point.truncate().length() < radius));
    }

    /// The shape does not depend on where the scan sits or how large it is.
    #[test]
    fn the_shape_is_the_same_far_from_the_origin_and_at_any_size() {
        let (rim, cap, support) = round_cap(48, 4.0, |angle| {
            Vec3::new(4.0 * angle.cos(), 4.0 * angle.sin(), -0.4)
        });
        let reference = shape_cap(&rim, &cap, &support, Continuity::Tangent);
        assert!(reference.iter().any(|point| point.z > 1.0));

        // The same cap, moved and resized as a whole.
        let placed = |scale: f32, offset: Vec3| {
            let place = |point: Vec3| point * scale + offset;
            let rim: Vec<Vec3> = rim.iter().map(|&point| place(point)).collect();
            let support: Vec<RimSupport> = support
                .iter()
                .map(|support| RimSupport {
                    conormal: support.conormal * scale,
                    area: support.area * scale * scale,
                })
                .collect();
            let cap = RefinedCap {
                generated: cap
                    .generated
                    .iter()
                    .map(|vertex| {
                        EditVertex::at(place(Vec3::from_array(vertex.position)).to_array())
                    })
                    .collect(),
                parents: cap.parents.clone(),
                levels: cap
                    .levels
                    .iter()
                    .map(|level| CapLevel {
                        vertex_count: level.vertex_count,
                        triangles: level.triangles.clone(),
                    })
                    .collect(),
            };
            shape_cap(&rim, &cap, &support, Continuity::Tangent)
        };
        for (scale, offset) in [
            (1.0_f32, Vec3::new(250.0, -180.0, 90.0)),
            (1.0e-6, Vec3::ZERO),
            (1.0e6, Vec3::ZERO),
        ] {
            let moved = placed(scale, offset);
            assert_eq!(reference.len(), moved.len());
            for (a, b) in reference.iter().zip(&moved) {
                let back = (*b - offset) / scale;
                assert!(
                    a.distance(back) < 2e-3,
                    "at scale {scale}, offset {offset:?}: {a:?} vs {back:?}"
                );
            }
        }
    }
}
