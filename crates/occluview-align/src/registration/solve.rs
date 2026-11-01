//! The small dense solves of the search: a rigid fit of paired points and the
//! point-to-plane step of Chen and Medioni (1992),
//! <https://doi.org/10.1016/0262-8856(92)90066-C>.

use crate::Rigid;
use glam::{DMat3, DQuat, DVec3};

/// Relative damping of the step, and how often it grows before giving up.
const DAMPING: f64 = 1e-9;
const DAMPING_GROWTH: f64 = 100.;
const DAMPING_TRIES: usize = 5;
/// Sweeps of the eigenvalue iteration; a small matrix settles in far fewer.
const SWEEPS: usize = 64;

/// Whether a value is a number above zero.
pub(super) fn positive(value: f64) -> bool {
    value > 0.
}

/// The proper rigid motion that best carries the weighted moving points onto
/// their fixed partners (Horn 1987, see [`crate::pairs`]). Absent for fewer
/// than three pairs or no weight.
pub(super) fn rigid_from_pairs(
    pairs: impl Iterator<Item = (DVec3, DVec3, f64)> + Clone,
) -> Option<Rigid> {
    let (mut moving, mut fixed, mut total, mut count) = (DVec3::ZERO, DVec3::ZERO, 0., 0usize);
    for (from, to, weight) in pairs.clone() {
        total += weight;
        moving += (from - moving) * (weight / total);
        fixed += (to - fixed) * (weight / total);
        count += 1;
    }
    if count < 3 || !positive(total) {
        return None;
    }
    let mut covariance = DMat3::ZERO;
    for (from, to, weight) in pairs {
        let (left, right) = ((from - moving) * weight, to - fixed);
        covariance += DMat3::from_cols(left * right.x, left * right.y, left * right.z);
    }
    let rotation = crate::pairs::horn_quaternion(&covariance);
    let pose = Rigid::new(rotation, fixed - rotation * moving);
    pose.is_finite().then_some(pose)
}

/// Weighted squared plane offsets of a set of points, linear in a small
/// rotation about `pivot` and a small shift.
#[derive(Clone, Copy)]
pub(super) struct PlaneSystem {
    pivot: DVec3,
    matrix: [[f64; 6]; 6],
    gradient: [f64; 6],
    weight: f64,
}

impl PlaneSystem {
    pub(super) fn new(pivot: DVec3) -> Self {
        Self {
            pivot,
            matrix: [[0.; 6]; 6],
            gradient: [0.; 6],
            weight: 0.,
        }
    }

    /// A point `offset` above the plane with unit `normal`.
    pub(super) fn add(&mut self, point: DVec3, normal: DVec3, offset: f64, weight: f64) {
        let torque = (point - self.pivot).cross(normal);
        let row = [torque.x, torque.y, torque.z, normal.x, normal.y, normal.z];
        for (i, line) in self.matrix.iter_mut().enumerate() {
            self.gradient[i] -= weight * row[i] * offset;
            for (k, value) in line.iter_mut().enumerate() {
                *value += weight * row[i] * row[k];
            }
        }
        self.weight += weight;
    }

    /// The motion that lowers the offsets most, to be applied after the pose
    /// the points were read in. Absent when the points do not determine one.
    pub(super) fn step(&self) -> Option<Rigid> {
        if !positive(self.weight) {
            return None;
        }
        let mut damping = DAMPING;
        for _ in 0..DAMPING_TRIES {
            let mut damped = self.matrix;
            for (index, row) in damped.iter_mut().enumerate() {
                row[index] += damping * row[index].max(self.weight * 1e-12);
            }
            if let Some(found) = solve_symmetric(&damped, &self.gradient) {
                let rotation = DQuat::from_scaled_axis(DVec3::new(found[0], found[1], found[2]));
                let shift = DVec3::new(found[3], found[4], found[5]);
                return Some(Rigid::new(
                    rotation,
                    self.pivot - rotation * self.pivot + shift,
                ));
            }
            damping *= DAMPING_GROWTH;
        }
        None
    }
}

/// The finite solution of a symmetric positive definite system, by Cholesky
/// factors. Absent when the matrix is not positive definite.
pub(super) fn solve_symmetric<const N: usize>(
    matrix: &[[f64; N]; N],
    right: &[f64; N],
) -> Option<[f64; N]> {
    let mut lower = [[0f64; N]; N];
    for row in 0..N {
        for column in 0..=row {
            let below: f64 = lower[row][..column]
                .iter()
                .zip(&lower[column][..column])
                .map(|(a, b)| a * b)
                .sum();
            let sum = matrix[row][column] - below;
            if row == column {
                if !positive(sum - f64::MIN_POSITIVE) {
                    return None;
                }
                lower[row][row] = sum.sqrt();
            } else {
                lower[row][column] = sum / lower[column][column];
            }
        }
    }
    let mut forward = [0f64; N];
    for row in 0..N {
        let mut sum = right[row];
        for inner in 0..row {
            sum -= lower[row][inner] * forward[inner];
        }
        forward[row] = sum / lower[row][row];
    }
    let mut out = [0f64; N];
    for row in (0..N).rev() {
        let mut sum = forward[row];
        for inner in row + 1..N {
            sum -= lower[inner][row] * out[inner];
        }
        out[row] = sum / lower[row][row];
    }
    out.iter().all(|value| value.is_finite()).then_some(out)
}

/// Eigenvalues of a symmetric matrix in rising order, with their unit
/// vectors, by cyclic Jacobi rotations.
pub(super) fn eigen<const N: usize>(mut matrix: [[f64; N]; N]) -> ([f64; N], [[f64; N]; N]) {
    let mut vectors = [[0f64; N]; N];
    for (index, row) in vectors.iter_mut().enumerate() {
        row[index] = 1.;
    }
    for _ in 0..SWEEPS {
        let mut off = 0f64;
        for p in 0..N {
            for q in p + 1..N {
                off = off.max(matrix[p][q].abs());
                if matrix[p][q] == 0. {
                    continue;
                }
                let theta = (matrix[q][q] - matrix[p][p]) / (2. * matrix[p][q]);
                let tangent = theta.signum() / (theta.abs() + (1. + theta * theta).sqrt());
                let cosine = 1. / (1. + tangent * tangent).sqrt();
                let sine = tangent * cosine;
                for row in &mut matrix {
                    let (a, b) = (row[p], row[q]);
                    row[p] = cosine * a - sine * b;
                    row[q] = sine * a + cosine * b;
                }
                let (top, bottom) = (matrix[p], matrix[q]);
                for k in 0..N {
                    matrix[p][k] = cosine * top[k] - sine * bottom[k];
                    matrix[q][k] = sine * top[k] + cosine * bottom[k];
                }
                for row in &mut vectors {
                    let (a, b) = (row[p], row[q]);
                    row[p] = cosine * a - sine * b;
                    row[q] = sine * a + cosine * b;
                }
            }
        }
        if off <= 1e-15 {
            break;
        }
    }
    let mut order: [usize; N] = std::array::from_fn(|i| i);
    order.sort_by(|&a, &b| matrix[a][a].total_cmp(&matrix[b][b]).then(a.cmp(&b)));
    (
        order.map(|i| matrix[i][i]),
        order.map(|i| std::array::from_fn(|row| vectors[row][i])),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points() -> Vec<DVec3> {
        (0..40)
            .map(|i| {
                let t = f64::from(i);
                DVec3::new(
                    (t * 0.7).sin() * 9.,
                    (t * 1.3).cos() * 6.,
                    (t * 0.37).sin() * 4.,
                )
            })
            .collect()
    }

    #[test]
    fn a_rigid_fit_recovers_the_motion_of_exact_pairs() {
        let truth = Rigid::new(
            DQuat::from_axis_angle(DVec3::new(0.2, 0.9, -0.4).normalize(), 2.1),
            DVec3::new(12., -3., 7.),
        );
        let moving = points();
        let pairs = moving.iter().map(|&p| (p, truth.apply(p), 1.));
        let found = rigid_from_pairs(pairs).unwrap();
        for &p in &moving {
            assert!(found.apply(p).distance(truth.apply(p)) < 1e-9);
        }
        assert!(rigid_from_pairs(moving.iter().take(2).map(|&p| (p, p, 1.))).is_none());
    }

    #[test]
    fn a_plane_step_removes_a_small_motion() {
        // Points of a curved surface with their normals, moved a little.
        let surface: Vec<(DVec3, DVec3)> = points()
            .into_iter()
            .map(|p| (p, (p + DVec3::new(0., 0., 20.)).normalize()))
            .collect();
        let moved = Rigid::new(
            DQuat::from_axis_angle(DVec3::new(0.5, 0.2, 0.8).normalize(), 0.002),
            DVec3::new(0.01, -0.02, 0.015),
        );
        let mut system = PlaneSystem::new(DVec3::new(1., 2., 3.));
        for (point, normal) in &surface {
            let at = moved.apply(*point);
            system.add(at, *normal, (at - *point).dot(*normal), 1.);
        }
        let step = system.step().unwrap();
        for (point, normal) in &surface {
            let back = step.apply(moved.apply(*point));
            assert!((back - *point).dot(*normal).abs() < 1e-5);
        }
    }

    #[test]
    fn eigenvalues_rise_and_rebuild_the_matrix() {
        let mut matrix = [[0f64; 6]; 6];
        for (i, row) in matrix.iter_mut().enumerate() {
            for (k, value) in row.iter_mut().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let (a, b) = (i as f64, k as f64);
                *value = 1. / (1. + a + b) + if i == k { a } else { 0. };
            }
        }
        let (values, vectors) = eigen(matrix);
        assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
        for i in 0..6 {
            for k in 0..6 {
                let rebuilt: f64 = (0..6)
                    .map(|mode| values[mode] * vectors[mode][i] * vectors[mode][k])
                    .sum();
                assert!((rebuilt - matrix[i][k]).abs() < 1e-9, "{i} {k}");
            }
        }
    }
}
