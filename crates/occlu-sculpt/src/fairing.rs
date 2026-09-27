//! Implicit uniform fairing over an area-weighted selected surface.
//!
//! One sparse symmetric system computes shape smoothing; zero-weight vertices
//! hold the boundary. Uniform edge weights depend on tessellation, while the
//! area mass sets the physical scale. The live sculpt session regularizes
//! spacing separately against an immutable post-brush surface.

use glam::DVec3;

mod preconditioner;
use preconditioner::IncompleteCholesky;

/// Physical feature scale as a fraction of the brush radius. Radius chooses
/// which features are inside the solve; strength is the correction dose only.
const SMOOTH_SCALE_FRACTION: f64 = 0.35;
/// Diffusion time per call, as a multiple of the squared smoothing scale. A
/// stroke lays many dabs, so one dab is deliberately not the whole correction.
const TIME_PER_SCALE_SQUARED: f64 = 1.0;
/// Factor by which the residual must fall before the solve stops.
const SOLVE_TOLERANCE: f64 = 1e-4;
/// Diagonal-scaled residual threshold in mm, independent of the solver's
/// preconditioner. This local indicator is not a bound on the full solution error.
const SOLVE_SETTLED_MM: f64 = 1.0e-7;
/// Conjugate-gradient iterations one call may spend. The count needed grows
/// like `scale / edge`, so this only binds on the widest brush over the finest
/// scan, where it costs some of the requested scale and never a stalled
/// stroke.
const MAX_SOLVE_STEPS: usize = 192;
/// Floor on a vertex's own area, so a collapsed sliver cannot divide by zero.
const MIN_VERTEX_AREA_MM2: f64 = 1e-9;

/// The millimetre scale a dab levels at. Everything smaller than this is
/// removed and everything larger is retained; the caller applies strength to
/// the correction amount, so the slider cannot silently change both controls.
pub fn smoothing_scale_mm(radius_mm: f64, _strength: f64) -> f64 {
    radius_mm * SMOOTH_SCALE_FRACTION
}

/// What the operator needs to know about the surface under a selection.
///
/// It is a trait rather than a pair of buffers because the two sessions that
/// run this operator index their surfaces differently - one by welded group,
/// one by mesh vertex - and neither may be made to rebuild its topology into
/// the other's shape just to be smoothed.
pub trait FairingSurface {
    /// Highest addressable vertex id plus one; the row map is sized from it.
    fn vertex_count(&self) -> usize;
    /// Position of a vertex in the caller's own index space.
    fn position(&self, vertex: u32) -> DVec3;
    /// The vertex's own share of surface area, mm² - the mass matrix diagonal.
    fn vertex_area(&self, vertex: u32) -> f64;
    /// One-ring neighbours of a vertex in the caller's index space.
    fn neighbors(&self, vertex: u32) -> &[u32];
}

/// Reusable row map, owned by the caller's session.
///
/// Rebuilding it per call is a write over every vertex in the mesh to answer a
/// question about a few thousand of them, and it showed: a region-local dab
/// cost 46% more on a 194k-vertex mesh than on a 6.5k one purely through that
/// line. It is cleared in O(footprint) instead.
#[derive(Default)]
pub struct FairingScratch {
    slot_of: Vec<u32>,
    written: Vec<u32>,
    recycled: Option<Fairing>,
}

/// The selection as a sparse symmetric system: one row per vertex that is free
/// to move, one uniform weight per edge between them, and the fixed rim folded
/// into the right-hand side.
#[derive(Default)]
struct Fairing {
    row_start: Vec<u32>,
    /// Column index into the selection's rows (a fixed neighbour is one whose
    /// `free_row` is `u32::MAX`).
    column: Vec<u32>,
    weight: Vec<f64>,
    /// Positions of every slot, fixed ones included.
    position: Vec<DVec3>,
    /// `false` marks the held rim.
    free: Vec<bool>,
    mass: Vec<f64>,
    /// Row index among the free rows, or `u32::MAX`.
    free_row: Vec<u32>,
}

/// Resolve held rows, column indirection and time scaling once per solve,
/// rather than repeating them for every conjugate-gradient iteration.
struct FreeOperator {
    row_start: Vec<usize>,
    column: Vec<usize>,
    weight: Vec<f64>,
}

impl FreeOperator {
    fn new(system: &Fairing, time: f64) -> Self {
        let mut operator = Self {
            row_start: vec![0],
            column: Vec::with_capacity(system.column.len()),
            weight: Vec::with_capacity(system.weight.len()),
        };
        for row in 0..system.free.len() {
            if !system.free[row] {
                continue;
            }
            for entry in system.row_start[row] as usize..system.row_start[row + 1] as usize {
                let column = system.free_row[system.column[entry] as usize];
                if column != u32::MAX {
                    operator.column.push(column as usize);
                    operator.weight.push(time * system.weight[entry]);
                }
            }
            operator.row_start.push(operator.column.len());
        }
        operator
    }

    fn multiply(&self, diagonal: &[f64], x: &[DVec3], out: &mut Vec<DVec3>) {
        out.clear();
        for row in 0..diagonal.len() {
            let mut sum = DVec3::ZERO;
            for entry in self.row_start[row]..self.row_start[row + 1] {
                sum += x[self.column[entry]] * self.weight[entry];
            }
            out.push((x[row] * diagonal[row]) - sum);
        }
    }
}

impl Fairing {
    /// `A = M - t L` applied to `x`, over the free rows only.
    /// `b = M p` plus the fixed rim's contribution, and the matrix diagonal.
    fn right_hand_side(&self, time: f64) -> (Vec<DVec3>, Vec<f64>) {
        let mut b = Vec::new();
        let mut diagonal = Vec::new();
        for row in 0..self.free.len() {
            if !self.free[row] {
                continue;
            }
            let start = self.row_start[row] as usize;
            let end = self.row_start[row + 1] as usize;
            let mut value = self.position[row] * self.mass[row];
            let mut own = self.mass[row];
            for entry in start..end {
                let w = self.weight[entry];
                own += time * w;
                let column = self.column[entry] as usize;
                if self.free_row[column] == u32::MAX {
                    value += self.position[column] * (time * w);
                }
            }
            b.push(value);
            diagonal.push(own);
        }
        (b, diagonal)
    }
}

fn dot(a: &[DVec3], b: &[DVec3]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x.dot(*y)).sum()
}

mod constraint;
pub use constraint::{fair_selection_with_constraint, FairingContact, FairingContactStats};

/// Fair one selection and report where each vertex should land.
///
/// `selection` pairs a vertex id with its 0..1 share of the correction: zero
/// holds the vertex as the system's boundary, one takes it all the way to the
/// solved surface. `feature_size_mm` is the SIZE of the detail removed, not a
/// gain and not a pass count.
///
/// `out` is replaced with `(vertex, target position)` for every vertex that
/// actually moves. Writing them is the caller's: the undo record, the clinical
/// masks and the anti-inversion guards are each session's own, and a solved
/// position that violates one of them must be refused where that rule lives.
pub fn fair_selection<S: FairingSurface + ?Sized>(
    surface: &S,
    selection: &[(u32, f64)],
    feature_size_mm: f64,
    scratch: &mut FairingScratch,
    out: &mut Vec<(u32, DVec3)>,
) {
    out.clear();
    let Some(mut job) = FairingJob::new(surface, selection, feature_size_mm, scratch) else {
        return;
    };
    job.advance(MAX_SOLVE_STEPS);
    job.targets(out);
    job.recycle(scratch);
}

/// A single immutable fairing problem. Continuations retain CG directions and
/// residuals; yielding never changes the numerical result or spends strength twice.
pub struct FairingJob {
    system: Fairing,
    operator: FreeOperator,
    preconditioner: IncompleteCholesky,
    selection: Vec<(u32, f64)>,
    rhs: Vec<DVec3>,
    diagonal: Vec<f64>,
    x: Vec<DVec3>,
    r: Vec<DVec3>,
    z: Vec<DVec3>,
    p: Vec<DVec3>,
    ap: Vec<DVec3>,
    rz: f64,
    target: f64,
    settled: f64,
    steps: usize,
    done: bool,
}

impl FairingJob {
    pub fn new<S: FairingSurface + ?Sized>(
        surface: &S,
        selection: &[(u32, f64)],
        feature_size_mm: f64,
        scratch: &mut FairingScratch,
    ) -> Option<Self> {
        if !feature_size_mm.is_finite()
            || feature_size_mm <= 0.0
            || selection.is_empty()
            || selection.iter().any(|&(v, w)| {
                v as usize >= surface.vertex_count() || !w.is_finite() || !(0.0..=1.0).contains(&w)
            })
        {
            return None;
        }
        let system = build(surface, selection, scratch)?;
        let time = TIME_PER_SCALE_SQUARED * feature_size_mm * feature_size_mm;
        let (b, diagonal) = system.right_hand_side(time);
        let operator = FreeOperator::new(&system, time);
        let preconditioner = IncompleteCholesky::new(&operator, &diagonal);
        let x: Vec<DVec3> = system
            .position
            .iter()
            .zip(&system.free)
            .filter_map(|(&p, &free)| free.then_some(p))
            .collect();
        let mut ap = Vec::with_capacity(x.len());
        operator.multiply(&diagonal, &x, &mut ap);
        let r: Vec<DVec3> = b.iter().zip(&ap).map(|(b, a)| b - (*a)).collect();
        let settled = r
            .iter()
            .zip(&diagonal)
            .fold(0.0f64, |m, (r, d)| m.max(r.length() / d.max(1e-30)));
        let mut z = Vec::with_capacity(r.len());
        preconditioner.apply(&r, &mut z);
        let rz = dot(&r, &z);
        let target = dot(&r, &r) * SOLVE_TOLERANCE * SOLVE_TOLERANCE;
        let p = z.clone();
        Some(Self {
            system,
            operator,
            preconditioner,
            selection: selection.to_vec(),
            rhs: b,
            diagonal,
            x,
            r,
            z,
            p,
            ap,
            rz,
            target,
            settled,
            steps: 0,
            done: false,
        })
    }

    pub fn advance(&mut self, steps: usize) -> bool {
        for _ in 0..steps {
            if self.done {
                break;
            }
            if self.steps >= MAX_SOLVE_STEPS
                || self.rz <= 0.0
                || dot(&self.r, &self.r) <= self.target
                || self.settled <= SOLVE_SETTLED_MM
            {
                self.done = true;
                break;
            }
            self.operator
                .multiply(&self.diagonal, &self.p, &mut self.ap);
            let denominator = dot(&self.p, &self.ap);
            if !denominator.is_finite() || denominator <= 1e-30 {
                self.done = true;
                break;
            }
            let alpha = self.rz / denominator;
            for i in 0..self.x.len() {
                self.x[i] = self.x[i] + self.p[i] * alpha;
                self.r[i] = self.r[i] - self.ap[i] * alpha;
            }
            self.settled = 0.0;
            self.preconditioner.apply(&self.r, &mut self.z);
            for i in 0..self.x.len() {
                self.settled = self
                    .settled
                    .max(self.r[i].length() / self.diagonal[i].max(1e-30));
            }
            let next = dot(&self.r, &self.z);
            let beta = next / self.rz;
            for i in 0..self.x.len() {
                self.p[i] = self.z[i] + self.p[i] * beta;
            }
            self.rz = next;
            self.steps += 1;
        }
        self.done |= self.steps >= MAX_SOLVE_STEPS;
        self.done
    }

    /// Reuse allocation capacity, while rebuilding every geometric coefficient
    /// from the next dab's surface. No stale Laplacian survives a deformation.
    pub fn recycle(self, scratch: &mut FairingScratch) {
        scratch.recycled = Some(self.system);
    }

    /// Targets always refer to the original surface, never an intermediate iterate.
    pub fn targets(&self, out: &mut Vec<(u32, DVec3)>) {
        out.clear();
        for (row, &(vertex, weight)) in self.selection.iter().enumerate() {
            if !self.system.free[row] {
                continue;
            }
            let here = self.system.position[row];
            let target = here + ((self.x[self.system.free_row[row] as usize] - here) * weight);
            if target.x.is_finite()
                && target.y.is_finite()
                && target.z.is_finite()
                && (target - here).length() > 1e-15
            {
                out.push((vertex, target));
            }
        }
    }
}

/// Assemble the selection's cotangent system. `None` when nothing in it is
/// free to move.
fn build<S: FairingSurface + ?Sized>(
    surface: &S,
    selection: &[(u32, f64)],
    scratch: &mut FairingScratch,
) -> Option<Fairing> {
    let count = selection.len();
    let vertex_count = surface.vertex_count();
    // Remeshing changes the vertex count between dabs. Preserve the untouched
    // slots instead of clearing the whole mesh whenever one vertex is added.
    for &vertex in &scratch.written {
        scratch.slot_of[vertex as usize] = u32::MAX;
    }
    scratch.slot_of.resize(vertex_count, u32::MAX);
    scratch.written.clear();
    for (row, &(vertex, _)) in selection.iter().enumerate() {
        scratch.slot_of[vertex as usize] = row as u32;
        scratch.written.push(vertex);
    }
    let Fairing {
        mut row_start,
        mut column,
        mut weight,
        mut position,
        mut mass,
        mut free_row,
        mut free,
    } = scratch.recycled.take().unwrap_or_default();
    free_row.clear();
    free_row.resize(count, u32::MAX);
    free.clear();
    let mut free_count = 0u32;
    for &(_, weight) in selection {
        let movable = weight > 0.0;
        free.push(movable);
        free_count += u32::from(movable);
    }
    if free_count == 0 {
        return None;
    }
    // Free rows are numbered in place, so a solution vector indexes straight
    // from a row without a second map.
    let mut next_free = 0u32;
    for (row, movable) in free.iter().enumerate() {
        if *movable {
            free_row[row] = next_free;
            next_free += 1;
        }
    }

    row_start.clear();
    column.clear();
    weight.clear();
    position.clear();
    mass.clear();
    row_start.push(0u32);
    for &(vertex, _) in selection {
        let here = surface.position(vertex);
        position.push(here);
        mass.push(surface.vertex_area(vertex).max(MIN_VERTEX_AREA_MM2));
        for &neighbor in surface.neighbors(vertex) {
            let slot = scratch.slot_of[neighbor as usize];
            if slot == u32::MAX {
                continue;
            }
            column.push(slot);
            // uniform (umbrella) weight. Every neighbour counts the same, and
            // every weight is positive.
            //
            // This replaced `max(0.5 * (cot a + cot b), 0)`, and the clamp was
            // the defect. Cotangent weights have LINEAR PRECISION: on a planar
            // patch they satisfy `sum_j w_ij (p_j - p_i) = 0` exactly, which is
            // why the Laplacian of a flat surface vanishes and why the operator
            // does not read the tessellation. Clamping each negative weight
            // (which is what an obtuse triangle produces) destroys that identity,
            // so a FREE vertex of a perfectly flat IRREGULAR patch acquires a
            // nonzero operator and moves. Worse, whether a given edge is clamped
            // depends on the current positions, and `max` is not continuous: as
            // vertices move by a fraction of a micron the clamp set flips, so the
            // operator solved on dab N+1 is a different function from the one
            // solved on dab N. Repeated stationary dabs then do not converge to a
            // fixed point — they oscillate, which is the operator's "it twitches
            // where the triangles are bad and is fine where they are regular".
            // Regular patches have no negative cotangents, so they never hit the
            // clamp and never showed the symptom.
            //
            // Uniform weights cannot have that failure: they are symmetric,
            // strictly positive, independent of the positions, and trivially
            // positive definite, so the system is the same function of the
            // selection on every dab and the solve is monotone. This is also
            // exactly what the reference sculpting brush ships — its default
            // smooth is a plain umbrella average over the one-ring, with no
            // cotangent weight anywhere. The price is the one the cotangent
            // rewrite was trying to avoid (a uniform Laplacian reads the
            // tessellation), and it is the correct price to pay: a smoothing
            // brush that is stable on every mesh is worth more than one that is
            // mesh-independent in theory and oscillates in practice.
            weight.push(1.0);
        }
        row_start.push(column.len() as u32);
    }
    Some(Fairing {
        row_start,
        column,
        weight,
        position,
        free,
        mass,
        free_row,
    })
}
