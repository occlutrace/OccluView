//! Small numeric helpers the cap solvers and the hole walker share.
//!
//! Both of these existed as line-identical copies across those modules — three
//! of [`count_as_f32`], two of [`basis_from_normal`] — so a fix to either
//! reached one path and left the others behind.

use glam::Vec3;

/// A vertex count as `f32`.
///
/// The counts are fan and rim sizes, which never approach `u16::MAX`; the
/// saturation guards a pathological input rather than a reachable one.
pub(crate) fn count_as_f32(count: usize) -> f32 {
    f32::from(u16::try_from(count).unwrap_or(u16::MAX))
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
