//! Small numeric helpers the cap solvers and the hole walker share.
//!
//! Both of these existed as line-identical copies across those modules — three
//! of [`count_as_f32`], two of [`basis_from_normal`] — so a fix to either
//! reached one path and left the others behind.

use glam::Vec3;

/// A vertex count as `f32`.
///
/// Outside support fans are not bounded by the cap's rim-size limit. Convert
/// every byte of the count so large fans use their full valence in averages.
pub(crate) fn count_as_f32(count: usize) -> f32 {
    count
        .to_le_bytes()
        .iter()
        .rev()
        .fold(0.0_f32, |value, &byte| {
            value.mul_add(256.0, f32::from(byte))
        })
}

/// Right-handed orthonormal tangent basis for a unit `normal`.
pub(crate) fn basis_from_normal(normal: Vec3) -> (Vec3, Vec3) {
    let axis = if normal.x.abs() > 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let u = axis.cross(normal).normalize();
    let v = normal.cross(u);
    (u, v)
}
