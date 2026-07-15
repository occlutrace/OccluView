use std::collections::BTreeMap;

use manifold_csg::Manifold;

use crate::{invalid_input, kernel_error, RobustCsgError};

pub(crate) fn union_overlapping_clusters(
    manifolds: &[Manifold],
    bounds: &[([f64; 3], [f64; 3])],
) -> Result<Vec<Manifold>, RobustCsgError> {
    if manifolds.is_empty() || manifolds.len() != bounds.len() {
        return Err(invalid_input("additive shell inventory is inconsistent"));
    }

    let mut clusters = DisjointSet::new(manifolds.len());
    for left in 0..manifolds.len() {
        for right in left + 1..manifolds.len() {
            if !bounds_have_positive_overlap(bounds[left], bounds[right]) {
                continue;
            }
            let overlap = manifolds[left].intersection(&manifolds[right]);
            overlap.status().map_err(kernel_error)?;
            let overlap_volume = overlap.volume();
            if !overlap_volume.is_finite() {
                return Err(invalid_input("additive shell overlap volume is non-finite"));
            }
            if !overlap.is_empty() && overlap_volume > 0.0 {
                clusters.union(left, right);
            }
        }
    }

    let mut groups: BTreeMap<usize, Vec<Manifold>> = BTreeMap::new();
    for (index, manifold) in manifolds.iter().enumerate() {
        groups
            .entry(clusters.root(index))
            .or_default()
            .push(manifold.clone());
    }
    groups
        .into_values()
        .map(|group| {
            let solid = if group.len() == 1 {
                group[0].clone()
            } else {
                Manifold::batch_union(&group)
            };
            solid.status().map_err(kernel_error)?;
            Ok(solid)
        })
        .collect()
}

pub(crate) fn subtract_contained_voids(
    mut solids: Vec<Manifold>,
    voids: Vec<(usize, Manifold)>,
) -> Result<Vec<Manifold>, RobustCsgError> {
    for (shell_index, void) in voids {
        let container = find_void_container(shell_index, &void, &solids)?;
        let result = solids[container].difference(&void);
        result.status().map_err(kernel_error)?;
        if result.is_empty() || !result.volume().is_finite() || result.volume() <= 0.0 {
            return Err(RobustCsgError::AmbiguousShellWinding {
                shell: shell_index,
                reason: "inward shell removes its complete additive cluster".to_string(),
            });
        }
        solids[container] = result;
    }
    Ok(solids)
}

fn find_void_container(
    shell_index: usize,
    void: &Manifold,
    solids: &[Manifold],
) -> Result<usize, RobustCsgError> {
    let void_volume = void.volume();
    if !void_volume.is_finite() || void_volume <= 0.0 {
        return Err(ambiguous_void(
            shell_index,
            "inward shell has no positive volume",
        ));
    }
    let tolerance = void_volume * 1.0e-8;
    let mut container = None;
    for (index, solid) in solids.iter().enumerate() {
        let intersection = solid.intersection(void);
        intersection.status().map_err(kernel_error)?;
        let overlap = intersection.volume();
        if !overlap.is_finite() {
            return Err(ambiguous_void(
                shell_index,
                "inward shell overlap volume is non-finite",
            ));
        }
        if (void_volume - overlap).abs() <= tolerance {
            if container.replace(index).is_some() {
                return Err(ambiguous_void(
                    shell_index,
                    "inward shell has more than one additive container",
                ));
            }
        } else if !intersection.is_empty() && overlap > 0.0 {
            return Err(ambiguous_void(
                shell_index,
                "inward shell intersects an additive cluster partially",
            ));
        }
    }
    container.ok_or_else(|| {
        ambiguous_void(
            shell_index,
            "inward shell is not fully contained by an additive cluster",
        )
    })
}

fn ambiguous_void(shell: usize, reason: &str) -> RobustCsgError {
    RobustCsgError::AmbiguousShellWinding {
        shell,
        reason: reason.to_string(),
    }
}

fn bounds_have_positive_overlap(left: ([f64; 3], [f64; 3]), right: ([f64; 3], [f64; 3])) -> bool {
    (0..3).all(|axis| left.0[axis].max(right.0[axis]) < left.1[axis].min(right.1[axis]))
}

struct DisjointSet {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl DisjointSet {
    fn new(length: usize) -> Self {
        Self {
            parent: (0..length).collect(),
            rank: vec![0; length],
        }
    }

    fn root(&mut self, value: usize) -> usize {
        let mut root = value;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut current = value;
        while self.parent[current] != current {
            let next = self.parent[current];
            self.parent[current] = root;
            current = next;
        }
        root
    }

    fn union(&mut self, left: usize, right: usize) {
        let left = self.root(left);
        let right = self.root(right);
        if left == right {
            return;
        }
        match self.rank[left].cmp(&self.rank[right]) {
            std::cmp::Ordering::Less => self.parent[left] = right,
            std::cmp::Ordering::Greater => self.parent[right] = left,
            std::cmp::Ordering::Equal => {
                self.parent[right] = left;
                self.rank[left] = self.rank[left].saturating_add(1);
            }
        }
    }
}
