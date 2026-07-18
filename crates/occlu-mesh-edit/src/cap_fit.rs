//! The interpolated cap's SURFACE model: a local planar frame plus a quadric
//! height field least-squares fitted to the rim and a distance-weighted band
//! of surface samples just outside it. Split out of `cap_refine.rs` (which
//! keeps the triangulation/refinement machinery) to hold the file-size budget.

use glam::{Vec2, Vec3};

/// Tikhonov ridge added to the (scale-normalized) quadric normal equations.
const QUADRIC_RIDGE: f32 = 1e-4;

/// A local orthonormal frame plus a quadric height field over it. Interior cap
/// vertices are lifted onto `centroid + a*u + b*v + height(a,b)*normal`.
pub(super) struct CapSurface {
    pub(super) centroid: Vec3,
    pub(super) u: Vec3,
    pub(super) v: Vec3,
    pub(super) normal: Vec3,
    /// Coefficients of `h = c0 + c1 a + c2 b + c3 a^2 + c4 a b + c5 b^2`.
    coeffs: [f32; 6],
}

impl CapSurface {
    /// Local `(a, b)` planar coordinates of a 3D point in this frame.
    pub(super) fn local_ab(&self, position: Vec3) -> Vec2 {
        let relative = position - self.centroid;
        Vec2::new(relative.dot(self.u), relative.dot(self.v))
    }

    /// Fitted height above the plane at planar coordinates `(a, b)`.
    pub(super) fn height(&self, ab: Vec2) -> f32 {
        let c = &self.coeffs;
        let (a, b) = (ab.x, ab.y);
        c[0] + c[1] * a + c[2] * b + c[3] * a * a + c[4] * a * b + c[5] * b * b
    }

    /// Lift planar coordinates onto the fitted surface in 3D.
    pub(super) fn lift(&self, ab: Vec2) -> Vec3 {
        self.centroid + ab.x * self.u + ab.y * self.v + self.height(ab) * self.normal
    }

    /// Lift with an extra height offset above the fitted surface (used to blend
    /// in the harmonically interpolated rim residual).
    pub(super) fn lift_with(&self, ab: Vec2, extra_height: f32) -> Vec3 {
        self.centroid
            + ab.x * self.u
            + ab.y * self.v
            + (self.height(ab) + extra_height) * self.normal
    }
}

/// Falloff scale for support-sample weights, in units of the mean rim edge
/// length: a sample two edge lengths from the rim still weighs ~1/2, one at
/// four edge lengths ~1/65. The band exists to pin LOCAL curvature at the
/// seam; a topological neighbor that is metrically far (a cone apex, the far
/// wall of a deep socket) is distant geometry, and letting it pull the quadric
/// used to sink the whole cap toward it — a funnel-shaped "cap" that follows
/// the socket instead of covering it.
const SUPPORT_FALLOFF_EDGES: f32 = 2.0;

/// The cap's local orthonormal frame used while assembling fit samples.
struct PlanarFrame {
    centroid: Vec3,
    tangent_u: Vec3,
    tangent_v: Vec3,
    normal: Vec3,
}

/// Project rim + support into the frame as `(a, b, height, weight)` rows for
/// the weighted quadric fit. Rim samples weigh 1. Support samples are
/// distance-weighted (see [`SUPPORT_FALLOFF_EDGES`]), and any support sample
/// whose projection falls INSIDE the rim polygon is excluded outright: that is
/// an overhanging wall (tooth socket, cone flank), not the outside curvature
/// the band exists to capture — on a round rim the fit's center height is pure
/// extrapolation, so even a tiny weight on such a sample used to sink the
/// whole cap into a funnel.
fn weighted_planar_samples(
    rim: &[Vec3],
    support: &[[f32; 3]],
    support_distances: &[f32],
    frame: &PlanarFrame,
) -> Vec<(f32, f32, f32, f32)> {
    let rim_len = rim.len();
    // Mean rim edge length: the metric yardstick for "local to the seam".
    let mut rim_edge_sum = 0.0_f32;
    for index in 0..rim_len {
        rim_edge_sum += rim[index].distance(rim[(index + 1) % rim_len]);
    }
    let rim_edge_scale = (rim_edge_sum / count_as_f32(rim_len.max(1))).max(f32::EPSILON);

    // Sixth-power falloff: flat inside the intended band, near-zero for
    // metrically distant geometry.
    let support_weight = |distance: f32| -> f32 {
        let relative = distance / (SUPPORT_FALLOFF_EDGES * rim_edge_scale);
        let sixth = (relative * relative * relative).powi(2);
        1.0 / (1.0 + sixth)
    };
    let project = |point: Vec3| -> (f32, f32, f32) {
        let relative = point - frame.centroid;
        (
            relative.dot(frame.tangent_u),
            relative.dot(frame.tangent_v),
            relative.dot(frame.normal),
        )
    };
    let rim_planar: Vec<(f32, f32, f32)> = rim.iter().map(|&point| project(point)).collect();
    let overhang = OverhangClassifier::from_rim(&rim_planar);
    rim_planar
        .iter()
        .map(|&(coord_u, coord_v, height)| (coord_u, coord_v, height, 1.0_f32))
        .chain(
            support
                .iter()
                .zip(support_distances)
                .map(|(point, &distance)| {
                    let (coord_u, coord_v, height) = project(Vec3::from_array(*point));
                    let weight = if overhang.is_inside_rim(coord_u, coord_v) {
                        0.0
                    } else {
                        support_weight(distance)
                    };
                    (coord_u, coord_v, height, weight)
                }),
        )
        .collect()
}

/// Fit a local frame (Newell normal over the rim) plus a quadric height field
/// least-squares fitted to the rim AND a band of surface samples just outside
/// it. A clean circular rim is nearly planar and carries no curvature on its
/// own, so the outside band is what lets the fit recover the local shape (a
/// sphere/saddle exactly, a gentle blend otherwise). Support samples are
/// distance-weighted (see [`SUPPORT_FALLOFF_EDGES`]); rim samples weigh 1.
pub(super) fn fit_cap_surface(
    rim: &[Vec3],
    support: &[[f32; 3]],
    support_distances: &[f32],
) -> CapSurface {
    let rim_len = rim.len();
    let mut centroid = Vec3::ZERO;
    for &p in rim {
        centroid += p;
    }
    centroid /= count_as_f32(rim_len.max(1));

    // Newell's method: robust polygon normal for a non-planar rim. Vertices
    // are taken RELATIVE to the centroid: Newell is translation-invariant in
    // exact arithmetic, and centering avoids the catastrophic f32 cancellation
    // a small far-from-origin rim would otherwise hit.
    let mut normal = Vec3::ZERO;
    for index in 0..rim_len {
        let current = rim[index] - centroid;
        let next = rim[(index + 1) % rim_len] - centroid;
        normal.x += (current.y - next.y) * (current.z + next.z);
        normal.y += (current.z - next.z) * (current.x + next.x);
        normal.z += (current.x - next.x) * (current.y + next.y);
    }
    let normal = if normal.is_finite() && normal.length_squared() > f32::EPSILON {
        normal.normalize()
    } else {
        Vec3::Z
    };
    let (tangent_u, tangent_v) = basis_from_normal(normal);

    // Least-squares quadric h = c0 + c1 a + c2 b + c3 a^2 + c4 a b + c5 b^2,
    // solving the 6x6 normal equations (A^T A + ridge) c = A^T h. The rim pins
    // the fit at the seam; the outside support band supplies the curvature.
    //
    // The fit runs in SCALE-NORMALIZED coordinates (divided by the RMS planar
    // radius): the fixed ridge is then meaningful for every hole size, where
    // in raw mm a sub-millimeter hole was flattened (ridge dominated its tiny
    // quadratic terms) and a very large one was effectively unregularized.
    let frame = PlanarFrame {
        centroid,
        tangent_u,
        tangent_v,
        normal,
    };
    let planar = weighted_planar_samples(rim, support, support_distances, &frame);
    let mut radius_sq_sum = 0.0_f64;
    let mut weight_sum = 0.0_f64;
    for &(coord_u, coord_v, _, weight) in &planar {
        radius_sq_sum += f64::from(weight)
            * (f64::from(coord_u) * f64::from(coord_u) + f64::from(coord_v) * f64::from(coord_v));
        weight_sum += f64::from(weight);
    }
    // f64 -> f32: a weighted RMS of finite f32 radii; well within f32 range.
    #[allow(clippy::cast_possible_truncation)]
    let rms_radius = ((radius_sq_sum / weight_sum.max(f64::MIN_POSITIVE)).sqrt()) as f32;
    if !(rms_radius.is_finite() && rms_radius > f32::EPSILON) {
        return CapSurface {
            centroid,
            u: tangent_u,
            v: tangent_v,
            normal,
            coeffs: [0.0; 6],
        };
    }
    let inv_radius = 1.0 / rms_radius;

    let mut normal_matrix = [[0.0f32; 6]; 6];
    let mut normal_rhs = [0.0f32; 6];
    for &(coord_u, coord_v, height, weight) in &planar {
        let (coord_u, coord_v, height) = (
            coord_u * inv_radius,
            coord_v * inv_radius,
            height * inv_radius,
        );
        let basis = [
            1.0,
            coord_u,
            coord_v,
            coord_u * coord_u,
            coord_u * coord_v,
            coord_v * coord_v,
        ];
        for (i, &bi) in basis.iter().enumerate() {
            for (j, &bj) in basis.iter().enumerate() {
                normal_matrix[i][j] += weight * bi * bj;
            }
            normal_rhs[i] += weight * bi * height;
        }
    }
    for (i, row) in normal_matrix.iter_mut().enumerate() {
        row[i] += QUADRIC_RIDGE;
    }
    let scaled = solve6(normal_matrix, normal_rhs).unwrap_or([0.0; 6]);
    // Undo the normalization: h = s*c0' + c1'*a + c2'*b + (c3'/s)*a^2 + ...
    let coeffs = [
        scaled[0] * rms_radius,
        scaled[1],
        scaled[2],
        scaled[3] * inv_radius,
        scaled[4] * inv_radius,
        scaled[5] * inv_radius,
    ];

    CapSurface {
        centroid,
        u: tangent_u,
        v: tangent_v,
        normal,
        coeffs,
    }
}

/// Lossless-enough count-to-float for averaging small vertex fans. Cap sizes
/// never approach `u16::MAX`, so the saturation only guards a pathological rim.
fn count_as_f32(count: usize) -> f32 {
    f32::from(u16::try_from(count).unwrap_or(u16::MAX))
}

/// Angular bins over the rim's planar radii, for a cheap "does this support
/// sample project INSIDE the rim polygon" verdict. Per bin the MINIMUM rim
/// radius is kept (conservative: near-rim outside samples are never dropped by
/// a wiggly bin); a sample clearly under its bin's minimum is an overhang.
/// Approximate for strongly non-star-shaped rims, which is acceptable — a
/// misclassified sample only shifts its fit weight, and the fold/pierce guards
/// still gate the final cap.
struct OverhangClassifier {
    min_radius_by_bin: Vec<f32>,
}

/// Bin count: ~5.6° per bin resolves real lasso rims without letting one
/// nearby rim vertex dominate a wide angular span.
const OVERHANG_BINS: usize = 64;
/// A sample must be clearly under the bin's minimum rim radius to be dropped.
const OVERHANG_MARGIN: f32 = 0.9;

impl OverhangClassifier {
    fn from_rim(rim_planar: &[(f32, f32, f32)]) -> Self {
        let mut min_radius_by_bin = vec![f32::MAX; OVERHANG_BINS];
        let mut global_min = f32::MAX;
        for &(coord_u, coord_v, _) in rim_planar {
            let radius = (coord_u * coord_u + coord_v * coord_v).sqrt();
            global_min = global_min.min(radius);
            let bin = Self::bin_of(coord_u, coord_v);
            min_radius_by_bin[bin] = min_radius_by_bin[bin].min(radius);
        }
        // Bins no rim vertex landed in fall back to the global minimum
        // (conservative: only clearly-interior samples are dropped there).
        if global_min < f32::MAX {
            for slot in &mut min_radius_by_bin {
                if *slot == f32::MAX {
                    *slot = global_min;
                }
            }
        }
        Self { min_radius_by_bin }
    }

    fn bin_of(coord_u: f32, coord_v: f32) -> usize {
        let angle = coord_v.atan2(coord_u); // [-pi, pi]
        let normalized = (angle + std::f32::consts::PI) / std::f32::consts::TAU;
        // Wrap-around safe: 1.0 maps back onto bin 0. Bin count is tiny, so
        // the count-to-f32 conversion is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bin = ((normalized * count_as_f32(OVERHANG_BINS)) as usize) % OVERHANG_BINS;
        bin
    }

    fn is_inside_rim(&self, coord_u: f32, coord_v: f32) -> bool {
        if self.min_radius_by_bin.is_empty() {
            return false;
        }
        let radius = (coord_u * coord_u + coord_v * coord_v).sqrt();
        let bin_minimum = self.min_radius_by_bin[Self::bin_of(coord_u, coord_v)];
        bin_minimum < f32::MAX && radius < bin_minimum * OVERHANG_MARGIN
    }
}

/// Right-handed orthonormal tangent basis for a unit `normal`.
fn basis_from_normal(normal: Vec3) -> (Vec3, Vec3) {
    let axis = if normal.x.abs() > 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let u = axis.cross(normal).normalize();
    let v = normal.cross(u);
    (u, v)
}

/// Solve a 6x6 linear system by Gaussian elimination with partial pivoting.
/// Returns `None` if the matrix is singular (caller falls back to a plane).
fn solve6(mut m: [[f32; 6]; 6], mut b: [f32; 6]) -> Option<[f32; 6]> {
    for col in 0..6 {
        // Partial pivot.
        let mut pivot = col;
        for row in (col + 1)..6 {
            if m[row][col].abs() > m[pivot][col].abs() {
                pivot = row;
            }
        }
        if m[pivot][col].abs() < 1e-12 {
            return None;
        }
        m.swap(col, pivot);
        b.swap(col, pivot);
        let inv = 1.0 / m[col][col];
        for row in (col + 1)..6 {
            let factor = m[row][col] * inv;
            if factor == 0.0 {
                continue;
            }
            for k in col..6 {
                m[row][k] -= factor * m[col][k];
            }
            b[row] -= factor * b[col];
        }
    }
    let mut x = [0.0f32; 6];
    for row in (0..6).rev() {
        let mut sum = b[row];
        for (col, &solved) in x.iter().enumerate().skip(row + 1) {
            sum -= m[row][col] * solved;
        }
        x[row] = sum / m[row][row];
    }
    Some(x)
}
