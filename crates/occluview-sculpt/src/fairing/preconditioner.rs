//! Row-sum-preserving modified incomplete Cholesky for the free-row system.
//! The factor accelerates the solve; it never changes the physical operator.

use super::{DVec3, FreeOperator};

pub(super) struct IncompleteCholesky {
    row_start: Vec<usize>,
    column: Vec<usize>,
    lower: Vec<f64>,
    diagonal: Vec<f64>,
}

impl IncompleteCholesky {
    pub(super) fn new(operator: &FreeOperator, diagonal: &[f64]) -> Self {
        let mut factor = Self {
            row_start: Vec::with_capacity(diagonal.len() + 1),
            column: Vec::with_capacity(operator.column.len() / 2),
            lower: Vec::with_capacity(operator.weight.len() / 2),
            diagonal: diagonal.to_vec(),
        };
        let mut row = Vec::new();
        factor.row_start.push(0);
        for i in 0..diagonal.len() {
            row.clear();
            for entry in operator.row_start[i]..operator.row_start[i + 1] {
                let j = operator.column[entry];
                if j < i && operator.weight[entry] > 0.0 {
                    row.push((j, -operator.weight[entry]));
                }
            }
            row.sort_unstable_by_key(|&(j, _)| j);
            for &(j, value) in &row {
                factor.column.push(j);
                factor.lower.push(value);
            }
            factor.row_start.push(factor.lower.len());
        }
        // A column view addresses the same lower entries, without a second
        // copy of numerical coefficients. It exists only during factorization.
        let mut column_start = vec![0usize; diagonal.len() + 1];
        for &j in &factor.column {
            column_start[j + 1] += 1;
        }
        for i in 0..diagonal.len() {
            column_start[i + 1] += column_start[i];
        }
        let mut cursor = column_start.clone();
        let mut entries = vec![(0usize, 0usize); factor.lower.len()];
        for i in 0..diagonal.len() {
            for entry in factor.row_start[i]..factor.row_start[i + 1] {
                let j = factor.column[entry];
                entries[cursor[j]] = (i, entry);
                cursor[j] += 1;
            }
        }
        for k in 0..diagonal.len() {
            // Strict diagonal dominance follows from positive masses and
            // nonnegative edge weights. Dropped negative Schur entries are
            // transferred to both diagonals, preserving this row-sum law.
            // A roundoff floor affects P only, never the physical matrix A.
            let pivot = factor.diagonal[k].max(diagonal[k] * 1e-12).sqrt();
            factor.diagonal[k] = pivot;
            let column = &entries[column_start[k]..column_start[k + 1]];
            for &(i, entry) in column {
                factor.lower[entry] /= pivot;
                factor.diagonal[i] -= factor.lower[entry] * factor.lower[entry];
            }
            for left in 0..column.len() {
                let (i, i_entry) = column[left];
                for &(j, j_entry) in &column[left + 1..] {
                    let product = factor.lower[i_entry] * factor.lower[j_entry];
                    let start = factor.row_start[j];
                    let end = factor.row_start[j + 1];
                    if let Ok(slot) = factor.column[start..end].binary_search(&i) {
                        factor.lower[start + slot] -= product;
                    } else {
                        factor.diagonal[i] -= product;
                        factor.diagonal[j] -= product;
                    }
                }
            }
        }
        factor
    }

    pub(super) fn apply(&self, residual: &[DVec3], out: &mut Vec<DVec3>) {
        out.clear();
        out.extend_from_slice(residual);
        for i in 0..out.len() {
            let mut value = out[i];
            for entry in self.row_start[i]..self.row_start[i + 1] {
                value -= out[self.column[entry]] * self.lower[entry];
            }
            out[i] = value * (1.0 / self.diagonal[i]);
        }
        for i in (0..out.len()).rev() {
            let value = out[i] * (1.0 / self.diagonal[i]);
            out[i] = value;
            for entry in self.row_start[i]..self.row_start[i + 1] {
                let j = self.column[entry];
                out[j] -= value * self.lower[entry];
            }
        }
    }
}
