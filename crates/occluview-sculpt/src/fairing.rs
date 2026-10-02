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
/// Maximum conjugate-gradient iterations per call. The cap bounds work when a
/// surface system converges slowly.
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
/// one by mesh vertex - and neither needs to rebuild its topology into
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

/// Reusable row map and solver state, owned by the caller's session.
///
/// General fairing jobs keep a same-scale initial correction for shared
/// vertices. Brush-preserving jobs reuse row storage but solve cold in input
/// order so their bounded iterations retain the brush's reference behavior.
#[derive(Default)]
pub struct FairingScratch {
    slot_of: Vec<u32>,
    written: Vec<u32>,
    warm_feature_size_mm: Option<f64>,
    warm_start: Vec<(u32, DVec3)>,
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

#[derive(Clone, Copy)]
enum FairingSolvePolicy {
    /// Reorder and warm-start long-running shared fairing jobs.
    General,
    /// Preserve brush selection order and start each bounded solve at the
    /// current surface, matching the sculpt brush's finite-iteration contract.
    Brush,
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
        out.resize(diagonal.len(), DVec3::ZERO);
        #[cfg(feature = "parallel")]
        if diagonal.len() >= crate::sculpt_session::PAR_FLOOR {
            use rayon::prelude::*;
            out.par_iter_mut().enumerate().for_each(|(row, value)| {
                *value = self.multiply_row(diagonal, x, row);
            });
            return;
        }
        for (row, value) in out.iter_mut().enumerate() {
            *value = self.multiply_row(diagonal, x, row);
        }
    }

    fn multiply_row(&self, diagonal: &[f64], x: &[DVec3], row: usize) -> DVec3 {
        let mut sum = DVec3::ZERO;
        for entry in self.row_start[row]..self.row_start[row + 1] {
            sum += x[self.column[entry]] * self.weight[entry];
        }
        (x[row] * diagonal[row]) - sum
    }
}

fn update_solution_row(x: &mut DVec3, r: &mut DVec3, p: DVec3, ap: DVec3, alpha: f64) {
    *x += p * alpha;
    *r -= ap * alpha;
}

fn update_direction_row(p: &mut DVec3, z: DVec3, beta: f64) {
    *p = z + (*p * beta);
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
mod preserving;
pub use preserving::fair_selection_preserving;

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

/// Solve one fairing selection to a relative residual tolerance and bounded
/// iteration count. Brush calls preserve input order and start cold so the
/// two bounded shape-preserving solves match the sculpt kernel's contract.
#[derive(Clone, Copy)]
pub(crate) struct FairingSolveBounds {
    pub(crate) tolerance: f64,
    pub(crate) steps: usize,
}

pub(crate) fn fair_selection_within<S: FairingSurface + ?Sized>(
    surface: &S,
    selection: &[(u32, f64)],
    feature_size_mm: f64,
    scratch: &mut FairingScratch,
    bounds: FairingSolveBounds,
) -> Vec<(u32, DVec3)> {
    let mut out = Vec::new();
    let Some(mut job) = FairingJob::new_with_policy(
        surface,
        selection,
        feature_size_mm,
        scratch,
        FairingSolvePolicy::Brush,
    ) else {
        return out;
    };
    job.target = dot(&job.r, &job.r) * bounds.tolerance * bounds.tolerance;
    job.advance(bounds.steps.min(MAX_SOLVE_STEPS));
    job.targets(&mut out);
    job.recycle(scratch);
    out
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
    feature_size_mm: f64,
    persist_warm_start: bool,
}

fn selection_is_valid<S: FairingSurface + ?Sized>(surface: &S, selection: &[(u32, f64)]) -> bool {
    !selection.is_empty()
        && selection.iter().all(|&(vertex, weight)| {
            (vertex as usize) < surface.vertex_count()
                && weight.is_finite()
                && (0.0..=1.0).contains(&weight)
        })
}

impl FairingJob {
    /// Create a reusable general-purpose fairing job.
    pub fn new<S: FairingSurface + ?Sized>(
        surface: &S,
        selection: &[(u32, f64)],
        feature_size_mm: f64,
        scratch: &mut FairingScratch,
    ) -> Option<Self> {
        Self::new_with_policy(
            surface,
            selection,
            feature_size_mm,
            scratch,
            FairingSolvePolicy::General,
        )
    }

    fn new_with_policy<S: FairingSurface + ?Sized>(
        surface: &S,
        selection: &[(u32, f64)],
        feature_size_mm: f64,
        scratch: &mut FairingScratch,
        policy: FairingSolvePolicy,
    ) -> Option<Self> {
        if !feature_size_mm.is_finite()
            || feature_size_mm <= 0.0
            || !selection_is_valid(surface, selection)
        {
            return None;
        }
        let (ordered_selection, persist_warm_start) = match policy {
            FairingSolvePolicy::General => (bandwidth_order(surface, selection, scratch), true),
            FairingSolvePolicy::Brush => (selection.to_vec(), false),
        };
        let system = build(surface, &ordered_selection, scratch)?;
        let time = TIME_PER_SCALE_SQUARED * feature_size_mm * feature_size_mm;
        let (b, diagonal) = system.right_hand_side(time);
        let operator = FreeOperator::new(&system, time);
        let preconditioner = IncompleteCholesky::new(&operator, &diagonal);
        let mut x: Vec<DVec3> = system
            .position
            .iter()
            .zip(&system.free)
            .filter_map(|(&p, &free)| free.then_some(p))
            .collect();
        if persist_warm_start
            && scratch
                .warm_feature_size_mm
                .is_some_and(|previous| previous.to_bits() == feature_size_mm.to_bits())
        {
            for (row, &(vertex, _)) in ordered_selection.iter().enumerate() {
                if !system.free[row] {
                    continue;
                }
                if let Ok(index) = scratch
                    .warm_start
                    .binary_search_by_key(&vertex, |&(id, _)| id)
                {
                    let free_row = system.free_row[row] as usize;
                    x[free_row] += scratch.warm_start[index].1;
                }
            }
        }
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
            selection: ordered_selection,
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
            feature_size_mm,
            persist_warm_start,
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
            #[cfg(feature = "parallel")]
            if self.x.len() >= crate::sculpt_session::PAR_FLOOR {
                use rayon::prelude::*;
                self.x
                    .par_iter_mut()
                    .zip(self.p.par_iter())
                    .zip(self.r.par_iter_mut())
                    .zip(self.ap.par_iter())
                    .for_each(|(((x, &p), r), &ap)| {
                        update_solution_row(x, r, p, ap, alpha);
                    });
            } else {
                for (((x, &p), r), &ap) in self
                    .x
                    .iter_mut()
                    .zip(&self.p)
                    .zip(&mut self.r)
                    .zip(&self.ap)
                {
                    update_solution_row(x, r, p, ap, alpha);
                }
            }
            #[cfg(not(feature = "parallel"))]
            for i in 0..self.x.len() {
                update_solution_row(&mut self.x[i], &mut self.r[i], self.p[i], self.ap[i], alpha);
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
            #[cfg(feature = "parallel")]
            if self.x.len() >= crate::sculpt_session::PAR_FLOOR {
                use rayon::prelude::*;
                self.p
                    .par_iter_mut()
                    .zip(self.z.par_iter())
                    .for_each(|(p, &z)| update_direction_row(p, z, beta));
            } else {
                for (p, &z) in self.p.iter_mut().zip(&self.z) {
                    update_direction_row(p, z, beta);
                }
            }
            #[cfg(not(feature = "parallel"))]
            for i in 0..self.x.len() {
                update_direction_row(&mut self.p[i], self.z[i], beta);
            }
            self.rz = next;
            self.steps += 1;
        }
        self.done |= self.steps >= MAX_SOLVE_STEPS;
        self.done
    }

    /// Recycle row storage. General jobs also retain the correction as a warm
    /// start; brush jobs invalidate it and rebuild from the live surface cold.
    pub fn recycle(self, scratch: &mut FairingScratch) {
        if self.persist_warm_start {
            scratch.warm_start.clear();
            for (row, &(vertex, _)) in self.selection.iter().enumerate() {
                if self.system.free_row[row] == u32::MAX {
                    continue;
                }
                let free_row = self.system.free_row[row] as usize;
                scratch
                    .warm_start
                    .push((vertex, self.x[free_row] - self.system.position[row]));
            }
            scratch
                .warm_start
                .sort_unstable_by_key(|&(vertex, _)| vertex);
            scratch.warm_feature_size_mm = Some(self.feature_size_mm);
        } else {
            scratch.warm_start.clear();
            scratch.warm_feature_size_mm = None;
        }
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

/// Order free rows by the selected graph so sparse factors and solver vectors
/// retain locality. Held boundary rows do not enter the solve and follow them.
fn bandwidth_order<S: FairingSurface + ?Sized>(
    surface: &S,
    selection: &[(u32, f64)],
    scratch: &mut FairingScratch,
) -> Vec<(u32, f64)> {
    for &vertex in &scratch.written {
        if let Some(slot) = scratch.slot_of.get_mut(vertex as usize) {
            *slot = u32::MAX;
        }
    }
    scratch.slot_of.resize(surface.vertex_count(), u32::MAX);
    scratch.written.clear();
    for (row, &(vertex, _)) in selection.iter().enumerate() {
        scratch.slot_of[vertex as usize] = row as u32;
        scratch.written.push(vertex);
    }

    let mut degree = vec![0usize; selection.len()];
    for (row, &(vertex, weight)) in selection.iter().enumerate() {
        if weight <= 0.0 {
            continue;
        }
        degree[row] = surface
            .neighbors(vertex)
            .iter()
            .filter(|&&neighbor| {
                let slot = scratch.slot_of[neighbor as usize];
                slot != u32::MAX && selection[slot as usize].1 > 0.0
            })
            .count();
    }

    let mut visited = vec![false; selection.len()];
    let mut queue = Vec::with_capacity(selection.len());
    let mut adjacent = Vec::with_capacity(8);
    let mut rows = Vec::with_capacity(selection.len());
    loop {
        let start = (0..selection.len())
            .filter(|&row| selection[row].1 > 0.0 && !visited[row])
            .min_by_key(|&row| (degree[row], selection[row].0));
        let Some(start) = start else {
            break;
        };
        let component_start = rows.len();
        visited[start] = true;
        queue.push(start);
        let mut cursor = 0;
        while cursor < queue.len() {
            let row = queue[cursor];
            cursor += 1;
            rows.push(row);
            adjacent.clear();
            for &neighbor in surface.neighbors(selection[row].0) {
                let slot = scratch.slot_of[neighbor as usize];
                if slot != u32::MAX {
                    let neighbor_row = slot as usize;
                    if selection[neighbor_row].1 > 0.0 && !visited[neighbor_row] {
                        adjacent.push(neighbor_row);
                    }
                }
            }
            adjacent.sort_unstable_by_key(|&row| (degree[row], selection[row].0));
            for &neighbor_row in &adjacent {
                if !visited[neighbor_row] {
                    visited[neighbor_row] = true;
                    queue.push(neighbor_row);
                }
            }
        }
        rows[component_start..].reverse();
        queue.clear();
    }

    let mut ordered = Vec::with_capacity(selection.len());
    ordered.extend(rows.into_iter().map(|row| selection[row]));
    ordered.extend(
        selection
            .iter()
            .copied()
            .filter(|&(_, weight)| weight <= 0.0),
    );
    ordered
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
        if let Some(slot) = scratch.slot_of.get_mut(vertex as usize) {
            *slot = u32::MAX;
        }
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
            // Equal positive weights keep adjacency coefficients symmetric and
            // independent of live coordinates. Area mass scales the physical
            // response; uniform adjacency defines the smoothing operator.
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

#[cfg(all(test, feature = "parallel"))]
mod tests {
    use super::*;

    struct GridSurface {
        positions: Vec<DVec3>,
        neighbors: Vec<Vec<u32>>,
    }

    impl FairingSurface for GridSurface {
        fn vertex_count(&self) -> usize {
            self.positions.len()
        }

        fn position(&self, vertex: u32) -> DVec3 {
            self.positions[vertex as usize]
        }

        fn vertex_area(&self, _vertex: u32) -> f64 {
            1.0
        }

        fn neighbors(&self, vertex: u32) -> &[u32] {
            &self.neighbors[vertex as usize]
        }
    }

    fn grid_surface(side: usize) -> GridSurface {
        let side_u32 = u32::try_from(side).expect("grid side fits the vertex index");
        let mut positions = Vec::with_capacity(side * side);
        let mut neighbors = Vec::with_capacity(side * side);
        for y in 0..side_u32 {
            for x in 0..side_u32 {
                let id = y * side_u32 + x;
                positions.push(DVec3::new(
                    f64::from(x),
                    f64::from(y),
                    f64::from((x * 17 + y * 31) % 23) / 23.0,
                ));
                let mut row = Vec::with_capacity(4);
                if y > 0 {
                    row.push(id - side_u32);
                }
                if x > 0 {
                    row.push(id - 1);
                }
                if x + 1 < side_u32 {
                    row.push(id + 1);
                }
                if y + 1 < side_u32 {
                    row.push(id + side_u32);
                }
                neighbors.push(row);
            }
        }
        GridSurface {
            positions,
            neighbors,
        }
    }

    #[test]
    fn sparse_fairing_is_bit_identical_across_worker_counts_and_selection_order() {
        let surface = grid_surface(93);
        let vertex_count =
            u32::try_from(surface.vertex_count()).expect("grid fits the vertex index");
        let selection: Vec<(u32, f64)> = (0..vertex_count).map(|vertex| (vertex, 1.0)).collect();
        let reversed: Vec<(u32, f64)> = selection.iter().copied().rev().collect();
        let one_worker = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("one-worker pool builds");
        let four_workers = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .expect("four-worker pool builds");
        let mut one_scratch = FairingScratch::default();
        let mut four_scratch = FairingScratch::default();
        let mut first = Vec::new();
        let mut second = Vec::new();
        for _ in 0..2 {
            one_worker.install(|| {
                fair_selection(&surface, &reversed, 1.0, &mut one_scratch, &mut first);
            });
            four_workers.install(|| {
                fair_selection(&surface, &selection, 1.0, &mut four_scratch, &mut second);
            });
            assert!(!first.is_empty());
            assert_eq!(first, second);
        }
    }
}
