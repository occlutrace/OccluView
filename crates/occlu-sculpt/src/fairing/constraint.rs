//! Contact-aware continuation of the shared cotangent fairing system.

use super::*;

/// A feasible position and the independent normal directions blocked by its
/// active contacts. The owner supplies normals in the same frame as positions.
/// Tangential degrees of freedom remain in the coupled fairing system.
#[derive(Clone, Copy, Debug)]
pub struct FairingContact {
    /// The feasible position of this control.
    pub position: DVec3,
    normals: [DVec3; 3],
    rank: usize,
}

impl FairingContact {
    /// A control that blocks no normal direction.
    pub fn free(position: DVec3) -> Self {
        Self {
            position,
            normals: [DVec3::ZERO; 3],
            rank: 0,
        }
    }

    /// Block motion along `normal` for this control.
    pub fn block_normal(&mut self, normal: DVec3) {
        if self.rank == 3 {
            return;
        }
        let mut normal = normal;
        // Reorthogonalize: almost parallel contacts must not leak normal
        // motion through the projected preconditioner.
        for _ in 0..2 {
            normal = self.tangent(normal);
        }
        let length = normal.length();
        if length.is_finite() && length > 1e-6 {
            self.normals[self.rank] = normal * (1.0 / length);
            self.rank += 1;
        }
    }

    fn tangent(&self, direction: DVec3) -> DVec3 {
        let mut result = direction;
        for &normal in &self.normals[..self.rank] {
            result -= normal * normal.dot(direction);
        }
        result
    }
}

#[derive(Default, Debug)]
/// Counters for one constrained fairing solve.
pub struct FairingContactStats {
    /// Outer contact-projection iterations.
    pub outer_steps: usize,
    /// Inner linear-system iterations.
    pub linear_steps: usize,
    /// Line-search backtracks.
    pub backtracks: usize,
    /// Projected residual at exit, millimetres.
    pub projected_residual_mm: f64,
}

/// Fair one selection subject to a caller-owned position constraint.
/// Contact removes forbidden motion, not the vertex's remaining degrees of
/// freedom. Revisit every row after its neighbours move so contact may slide
/// or release. The matrix, mass, right-hand side and selection stay frozen;
/// continuation solves the same fairing problem without applying another dose.
// the constraint, its statistics and the solve controls are one call's contract.
#[allow(clippy::too_many_arguments)]
pub fn fair_selection_with_constraint<S, F>(
    surface: &S,
    selection: &[(u32, f64)],
    feature_size_mm: f64,
    scratch: &mut FairingScratch,
    out: &mut Vec<(u32, DVec3)>,
    mut constrain: F,
) -> FairingContactStats
where
    S: FairingSurface + ?Sized,
    F: FnMut(u32, DVec3, DVec3) -> FairingContact,
{
    let mut stats = FairingContactStats::default();
    out.clear();
    let Some(mut job) = FairingJob::new(surface, selection, feature_size_mm, scratch) else {
        return stats;
    };
    job.advance(MAX_SOLVE_STEPS);
    let rows = job
        .selection
        .iter()
        .enumerate()
        .filter_map(|(row, &(vertex, weight))| {
            job.system.free[row].then_some((vertex, weight, job.system.position[row]))
        })
        .collect::<Vec<_>>();
    let mut project = |row: usize, target: DVec3| {
        let (vertex, weight, original) = rows[row];
        let proposed = original + ((target - original) * weight);
        let accepted = constrain(vertex, original, proposed);
        if accepted.position == proposed {
            // Preserve the unconstrained solve exactly, including its f64
            // coordinate. Inverting an unchanged selection adds roundoff.
            Some(FairingContact {
                position: target,
                ..accepted
            })
        } else {
            let target = original + ((accepted.position - original) * (1.0 / weight));
            target.is_finite().then_some(FairingContact {
                position: target,
                ..accepted
            })
        }
    };
    let projected = job
        .x
        .iter()
        .enumerate()
        .map(|(row, &target)| project(row, target))
        .collect::<Option<Vec<_>>>();
    let Some(projected) = projected else {
        job.recycle(scratch);
        return stats;
    };
    if projected.iter().zip(&job.x).any(|(p, &x)| p.position != x) {
        for (x, p) in job.x.iter_mut().zip(projected) {
            *x = p.position;
        }
        // Contact has its own bounded continuation. Exhausting CG must not
        // turn this operation into an isolated projection of each vertex.
        stats = relax_contact_system(
            &job.operator,
            &job.diagonal,
            &job.rhs,
            &mut job.x,
            &mut project,
        );
    }
    job.targets(out);
    job.recycle(scratch);
    stats
}

/// Active-set solution of the original quadratic. Exact contact queries are
/// outside CG: each linear solve keeps only normal constraints, while all free
/// and tangential coordinates remain coupled. The owner's next projection can
/// change a normal or release a contact; no geometric state survives this call.
fn relax_contact_system<F>(
    operator: &FreeOperator,
    diagonal: &[f64],
    rhs: &[DVec3],
    positions: &mut [DVec3],
    project: &mut F,
) -> FairingContactStats
where
    F: FnMut(usize, DVec3) -> Option<FairingContact>,
{
    let mut stats = FairingContactStats::default();
    let mut ax = Vec::new();
    let mut residual = vec![DVec3::ZERO; positions.len()];
    let mut contacts = Vec::with_capacity(positions.len());
    let mut candidate = positions.to_vec();
    let mut trial = positions.to_vec();
    let mut direction = positions.to_vec();
    let mut ad = Vec::new();
    let mut work = ContactSolveScratch::default();
    for _ in 0..MAX_SOLVE_STEPS {
        stats.outer_steps += 1;
        operator.multiply(diagonal, positions, &mut ax);
        contacts.clear();
        let mut settled = 0.0f64;
        for row in 0..positions.len() {
            residual[row] = rhs[row] - ax[row];
            // The local quadratic minimizer decides the active set. Testing
            // the current feasible position alone would never release contact.
            let Some(contact) = project(
                row,
                positions[row] + (residual[row] * (1.0 / diagonal[row])),
            ) else {
                return stats;
            };
            settled = settled.max((contact.position - positions[row]).length());
            candidate[row] = contact.position;
            contacts.push(contact);
        }
        stats.projected_residual_mm = settled;
        if settled <= SOLVE_SETTLED_MM {
            return stats;
        }
        {
            let steps = work.solve(
                operator,
                diagonal,
                rhs,
                &contacts,
                &mut candidate,
                MAX_SOLVE_STEPS,
            );
            stats.linear_steps += steps;
        }
        // Curved obstacles invalidate a frozen tangent plane away from its
        // anchor. Reproject and accept only a decrease of the same quadratic.
        // Backtracking changes the step, never the smoothing scale or RHS.
        let mut accepted = false;
        let mut fraction = 1.0;
        while fraction >= 1.0 / 1024.0 {
            for row in 0..positions.len() {
                let target = positions[row] + ((candidate[row] - positions[row]) * fraction);
                let Some(contact) = project(row, target) else {
                    return stats;
                };
                trial[row] = contact.position;
                direction[row] = trial[row] - positions[row];
            }
            operator.multiply(diagonal, &direction, &mut ad);
            let decrease = dot(&residual, &direction) - 0.5 * dot(&direction, &ad);
            if decrease > 0.0 {
                accepted = true;
                break;
            }
            stats.backtracks += 1;
            fraction *= 0.5;
        }
        if !accepted {
            // A projected gradient step globalizes the active-set method at
            // changing contacts. It uses the same operator and exact projector.
            for (row, contact) in contacts.iter().enumerate() {
                trial[row] = contact.position;
                direction[row] = trial[row] - positions[row];
            }
            operator.multiply(diagonal, &direction, &mut ad);
            if dot(&residual, &direction) - 0.5 * dot(&direction, &ad) <= 0.0 {
                return stats;
            }
        }
        positions.copy_from_slice(&trial);
    }
    stats
}

#[derive(Default)]
struct ContactSolveScratch {
    r: Vec<DVec3>,
    z: Vec<DVec3>,
    p: Vec<DVec3>,
    ap: Vec<DVec3>,
}

impl ContactSolveScratch {
    /// Symmetric Gauss-Seidel on P*A*P, not P*A^-1*P. Restricting only the
    /// ends of an unconstrained inverse retains propagation through blocked
    /// normal coordinates and is a poor preconditioner near held margins.
    /// The triangular factors below stay in each row's tangent subspace.
    fn precondition(
        &mut self,
        operator: &FreeOperator,
        diagonal: &[f64],
        contacts: &[FairingContact],
    ) {
        self.z.resize(self.r.len(), DVec3::ZERO);
        for row in 0..self.r.len() {
            let mut value = self.r[row];
            for entry in operator.row_start[row]..operator.row_start[row + 1] {
                let column = operator.column[entry];
                if column < row {
                    value += self.z[column] * operator.weight[entry];
                }
            }
            self.z[row] = contacts[row].tangent(value) * (1.0 / diagonal[row]);
        }
        for row in (0..self.r.len()).rev() {
            let mut value = DVec3::ZERO;
            for entry in operator.row_start[row]..operator.row_start[row + 1] {
                let column = operator.column[entry];
                if column > row {
                    value += self.z[column] * operator.weight[entry];
                }
            }
            self.z[row] += contacts[row].tangent(value) * (1.0 / diagonal[row]);
        }
    }

    // the operator, diagonal, preconditioner and iteration counters are one solve state.
    #[allow(clippy::too_many_arguments)]
    fn solve(
        &mut self,
        operator: &FreeOperator,
        diagonal: &[f64],
        rhs: &[DVec3],
        contacts: &[FairingContact],
        positions: &mut [DVec3],
        budget: usize,
    ) -> usize {
        operator.multiply(diagonal, positions, &mut self.ap);
        self.r.clear();
        self.r.extend(
            rhs.iter()
                .zip(&self.ap)
                .zip(contacts)
                .map(|((&b, &ax), c)| c.tangent(b - ax)),
        );
        self.precondition(operator, diagonal, contacts);
        self.p.clone_from(&self.z);
        let mut rz = dot(&self.r, &self.z);
        let tolerance = dot(&self.r, &self.r) * SOLVE_TOLERANCE * SOLVE_TOLERANCE;
        let mut steps = 0;
        while steps < budget && rz > 0.0 && dot(&self.r, &self.r) > tolerance {
            if self
                .r
                .iter()
                .zip(diagonal)
                .all(|(r, d)| r.length() / d <= SOLVE_SETTLED_MM)
            {
                break;
            }
            operator.multiply(diagonal, &self.p, &mut self.ap);
            for (ap, c) in self.ap.iter_mut().zip(contacts) {
                *ap = c.tangent(*ap);
            }
            let denominator = dot(&self.p, &self.ap);
            if !denominator.is_finite() || denominator <= 1e-30 {
                break;
            }
            let alpha = rz / denominator;
            for (row, position) in positions.iter_mut().enumerate() {
                *position += self.p[row] * alpha;
                self.r[row] = self.r[row] - self.ap[row] * alpha;
            }
            self.precondition(operator, diagonal, contacts);
            let next = dot(&self.r, &self.z);
            for row in 0..self.p.len() {
                self.p[row] = self.z[row] + (self.p[row] * (next / rz));
            }
            rz = next;
            steps += 1;
        }
        steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> FreeOperator {
        FreeOperator {
            row_start: vec![0, 1, 2],
            column: vec![1, 0],
            weight: vec![1.0, 1.0],
        }
    }

    fn plane(p: DVec3, normal: DVec3) -> FairingContact {
        let side = p.dot(normal);
        let mut contact = FairingContact::free(p);
        if side < 0.0 {
            contact.position = p - normal * side;
            contact.block_normal(normal);
        }
        contact
    }

    fn solve(
        operator: &FreeOperator,
        rhs: &[DVec3],
        positions: &mut [DVec3],
        mut project: impl FnMut(usize, DVec3) -> FairingContact,
    ) {
        let diagonal = vec![2.0; positions.len()];
        relax_contact_system(operator, &diagonal, rhs, positions, &mut |row, p| {
            Some(project(row, p))
        });
    }

    #[test]
    fn contacting_rows_slide_along_different_obstacles() {
        // A = [[2,-1],[-1,2]]. Freezing the contact positions at the origin
        // leaves nonzero tangent forces and an artificial crease.
        let rhs = [DVec3::new(-2.0, 1.0, 0.0), DVec3::new(1.0, -2.0, 0.0)];
        let mut positions = [DVec3::ZERO; 2];
        solve(&pair(), &rhs, &mut positions, |row, p| {
            plane(
                p,
                if row == 0 {
                    DVec3::new(1., 0., 0.)
                } else {
                    DVec3::new(0., 1., 0.)
                },
            )
        });
        assert_eq!(positions[0].x, 0.0);
        assert_eq!(positions[1].y, 0.0);
        assert!((positions[0].y - 0.5).abs() < 1e-7);
        assert!((positions[1].x - 0.5).abs() < 1e-7);
    }

    #[test]
    fn contacting_row_can_release_when_neighbours_move() {
        let rhs = [DVec3::new(-1.0, 0.0, 0.0), DVec3::new(4.0, 0.0, 0.0)];
        let mut positions = [DVec3::ZERO; 2];
        solve(&pair(), &rhs, &mut positions, |_, p| {
            plane(p, DVec3::new(1., 0., 0.))
        });
        assert!((positions[0].x - 2.0 / 3.0).abs() < 1e-7);
        assert!((positions[1].x - 7.0 / 3.0).abs() < 1e-7);
    }

    #[test]
    fn multiple_contacts_leave_the_edge_tangent_free() {
        let rhs = [DVec3::new(-2., -3., 1.), DVec3::new(0., 0., 2.)];
        let mut positions = [DVec3::ZERO; 2];
        solve(&pair(), &rhs, &mut positions, |row, p| {
            if row == 1 {
                return FairingContact::free(p);
            }
            let mut c = plane(p, DVec3::new(1., 0., 0.));
            if c.position.y < 0. {
                c.position.y = 0.;
                c.block_normal(DVec3::new(0., 1., 0.));
            }
            c
        });
        assert!((positions[0] - DVec3::new(0., 0., 4. / 3.)).length() < 1e-7);
        assert!((positions[1] - DVec3::new(0., 0., 5. / 3.)).length() < 1e-7);
    }

    #[test]
    fn curved_contact_updates_normal_while_sliding() {
        let rhs = [DVec3::new(0.1, 0.2, 0.), DVec3::new(0., 0., 0.4)];
        let mut positions = [DVec3::new(1., 0., 0.), DVec3::new(0., 0., 0.2)];
        solve(&pair(), &rhs, &mut positions, |row, p| {
            let mut c = FairingContact::free(p);
            if row == 0 && p.length() < 1. {
                c.position = p * (1. / p.length());
                c.block_normal(c.position);
            }
            c
        });
        // Eliminating row 1 gives 1.5*x = (0.1,0.2,0.2). Outside the
        // unit sphere, its minimum is the radial point (1,2,2)/3.
        let expected = DVec3::new(1. / 3., 2. / 3., 2. / 3.);
        assert!((positions[0] - expected).length() < 1e-6, "{positions:?}");
        assert!((positions[1] - ((expected + rhs[1]) * 0.5)).length() < 1e-6);
        assert!(positions[0].length() >= 1. - 1e-12);
    }

    #[test]
    fn contact_search_is_outside_the_coupled_linear_iterations() {
        let n: usize = 96;
        let normal = DVec3::new(0.6, 0.8, 0.);
        let tangent = DVec3::new(-0.8, 0.6, 0.);
        let expected = (0..n)
            .map(|i| tangent * (i as f64 * 0.13).sin())
            .collect::<Vec<_>>();
        let mut operator = FreeOperator {
            row_start: vec![0],
            column: vec![],
            weight: vec![],
        };
        for i in 0..n {
            for j in [i.checked_sub(1), (i + 1 < n).then_some(i + 1)]
                .into_iter()
                .flatten()
            {
                operator.column.push(j);
                operator.weight.push(0.9);
            }
            operator.row_start.push(operator.column.len());
        }
        let mut rhs = Vec::new();
        operator.multiply(&vec![2.; n], &expected, &mut rhs);
        for b in &mut rhs {
            *b -= normal;
        }
        let mut positions = vec![DVec3::ZERO; n];
        let mut queries = 0;
        solve(&operator, &rhs, &mut positions, |_, p| {
            queries += 1;
            plane(p, normal)
        });
        assert!(positions
            .iter()
            .zip(expected)
            .all(|(a, b)| (a - b).length() < 1e-6));
        assert!(
            queries < 12 * n,
            "{queries} exact contact queries for {n} rows"
        );
    }
}
