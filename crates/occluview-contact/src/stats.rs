//! What a contact reading counts: how much surface is in contact, how many
//! marks there are, and how deep the deepest one goes.
//!
//! # Why the area is weighted by corners
//!
//! A triangle either has contact or it does not, and the boundary between the
//! two runs through the middle of triangles. Clipping the exact contact region
//! would need a contour per triangle for a number that is read as "how much of
//! this arch is carrying", so a triangle contributes its own area scaled by how
//! many of its three corners are in contact. That is a partial-credit weight,
//! not a clipped area: it is exact when the contact boundary follows the mesh
//! and slightly under- or over-counts where it does not, which is a fraction of
//! a triangle out of square millimetres.
//!
//! # Why the split is x against the mean
//!
//! The left/right split is a balance reading: "is this bite loading evenly".
//! It is a raw-coordinate split on the subject surface's own vertex mean, not
//! an anatomical left and right, so a mirrored or rotated scan can swap the two
//! columns. An operator comparing the two numbers against each other on the
//! same case is reading what the number means; one comparing a left column
//! across differently oriented cases is not.

use glam::DVec3;

use crate::components::connected_components;
use crate::NO_CONTACT_MM;

/// A vertex this close to the antagonist counts as touching, in millimetres.
///
/// Twenty micrometres: below what a scan pair resolves, and wide enough that a
/// real contact is never missed by a rounding step. It is deliberately NOT the
/// tightness law's paint gate (10 um): the painted band is the reading a
/// technician takes one mark at a time, while this gate measures how much of the
/// arch is carrying, and a band that stopped exactly where the paint stops would
/// report an area short by the whole feather. The two numbers differ by one
/// feather's width and say two different things.
pub(crate) const TOUCH_MM: f32 = 0.02;

/// Chaining radius for counting separate contacts, in millimetres.
///
/// Not the flatten radius (0.75 mm, [`CONTACT_CLUSTER_RADIUS_MM`]'s sibling in
/// `components`): this one answers "how many marks does this bite have", and it
/// has to hold a whole occlusal patch together on a coarse scan while keeping
/// two teeth apart. Three millimetres does both on a dental arch.
///
/// An earlier version of this comment justified the number against a "hover
/// region radius (1.6 mm)" that exists nowhere in the tree; the hover readout
/// takes no radius at all.
pub(crate) const CONTACT_CLUSTER_RADIUS_MM: f64 = 3.0;

/// What one contact reading counted.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContactStats {
    /// Total contact area on the subject surface, in mm².
    pub contact_area_mm2: f64,
    /// Connected contact patches.
    pub contacts: u32,
    /// Deepest measured penetration on the subject, in millimetres (>= 0).
    pub deepest_mm: f64,
    /// Contact area where the subject triangle sits below the surface's own
    /// mean x, in mm².
    pub minus_x_area_mm2: f64,
    /// Contact area where the subject triangle sits at or above the mean x.
    pub plus_x_area_mm2: f64,
}

/// Measure the subject surface's own contact reading.
///
/// `values` is the signed field over `positions`, one entry per vertex. A shape
/// mismatch returns the default rather than a partial number: a count taken
/// over half a mesh reads like a small bite, which is worse than no reading at
/// all.
pub(crate) fn compute(positions: &[f32], indices: &[u32], values: &[f32]) -> ContactStats {
    let vertex_count = positions.len() / 3;
    if vertex_count < 3 || values.len() != vertex_count || indices.len() < 3 {
        return ContactStats::default();
    }
    let mut total_x = 0.0_f64;
    for vertex in 0..vertex_count {
        total_x += f64::from(positions[vertex * 3]);
    }
    let mean_x = total_x / to_f64(vertex_count);

    let mut stats = ContactStats::default();
    let mut contact_centroids: Vec<DVec3> = Vec::new();
    for triangle in indices.as_chunks::<3>().0 {
        let Some(first) = corner(positions, triangle[0], values) else {
            continue;
        };
        let Some(second) = corner(positions, triangle[1], values) else {
            continue;
        };
        let Some(third) = corner(positions, triangle[2], values) else {
            continue;
        };
        let touching = [first.1, second.1, third.1]
            .iter()
            .filter(|value| is_touching(**value))
            .count();
        if touching == 0 {
            continue;
        }
        let area = triangle_area(first.0, second.0, third.0);
        let contribution = area * (to_f64(touching) / 3.0);
        stats.contact_area_mm2 += contribution;
        let centroid = (first.0 + second.0 + third.0) / 3.0;
        if centroid.x < mean_x {
            stats.minus_x_area_mm2 += contribution;
        } else {
            stats.plus_x_area_mm2 += contribution;
        }
        contact_centroids.push(centroid);
    }

    stats.deepest_mm = deepest_penetration(values);
    stats.contacts = to_u32(connected_components(&contact_centroids, CONTACT_CLUSTER_RADIUS_MM).1);
    stats
}

/// One triangle corner: its position and its signed field value.
fn corner(positions: &[f32], index: u32, values: &[f32]) -> Option<(DVec3, f32)> {
    let vertex = usize::try_from(index).ok()?;
    let offset = vertex.checked_mul(3)?;
    let point = DVec3::new(
        f64::from(*positions.get(offset)?),
        f64::from(*positions.get(offset + 1)?),
        f64::from(*positions.get(offset + 2)?),
    );
    let value = *values.get(vertex)?;
    point.is_finite().then_some((point, value))
}

/// Whether a corner is part of a contact: measured, and at or inside the touch
/// tolerance. A vertex with no opposing surface is not in contact.
fn is_touching(value: f32) -> bool {
    value != NO_CONTACT_MM && value.is_finite() && value <= TOUCH_MM
}

/// The triangle's area in mm², or zero when it is degenerate.
fn triangle_area(first: DVec3, second: DVec3, third: DVec3) -> f64 {
    let cross = (second - first).cross(third - first);
    let doubled = cross.length();
    if doubled.is_finite() {
        0.5 * doubled
    } else {
        0.0
    }
}

/// The deepest measured penetration over the whole field, in millimetres.
fn deepest_penetration(values: &[f32]) -> f64 {
    let mut deepest = 0.0_f64;
    for value in values {
        if !value.is_finite() || *value >= 0.0 {
            continue;
        }
        deepest = deepest.max(-f64::from(*value));
    }
    deepest
}

/// A vertex count as a scalar for averaging.
#[allow(clippy::cast_precision_loss)]
fn to_f64(value: usize) -> f64 {
    value as f64
}

/// A count narrowed for the panel, saturating rather than wrapping.
#[allow(clippy::cast_possible_truncation)]
fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
