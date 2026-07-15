use std::collections::BTreeMap;
use std::sync::Arc;

use glam::{DMat3, DVec3};
use manifold_csg::Manifold;

use crate::disc_split::split_prepared_with_separator_disc;
use crate::native_mesh::{extract_part, manifold_from_closed_shell, validate_mesh};
use crate::solid_compose::{subtract_contained_voids, union_overlapping_clusters};
use crate::{
    invalid_input, kernel_error, position_bounds, RobustCsgError, RobustMesh, SeparatorDisc,
};

const FAN_PAIRING_TIE_TOLERANCE: f64 = 1.0e-10;
const MAX_EDGE_FAN_PAIRS: usize = 8;

type CanonicalMesh = (Vec<[f64; 3]>, Vec<[u64; 3]>);

/// Opaque local-space solid prepared for repeated separator placements.
#[derive(Clone)]
pub struct PreparedRobustSolid {
    inner: Arc<PreparedSolidInner>,
}

struct PreparedSolidInner {
    solids: Vec<Manifold>,
    local_meshes: Vec<RobustMesh>,
}

impl std::fmt::Debug for PreparedRobustSolid {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedRobustSolid")
            .field("local_volume", &self.local_volume())
            .finish_non_exhaustive()
    }
}

/// Recover closed shells and build a reusable local-space native solid set.
///
/// # Errors
/// Returns [`RobustCsgError`] for malformed topology, ambiguous winding, or a
/// native boolean failure.
pub fn prepare_robust_solid(mesh: &RobustMesh) -> Result<PreparedRobustSolid, RobustCsgError> {
    PreparedRobustSolid::new(mesh)
}

impl PreparedRobustSolid {
    /// Prepare one raw indexed mesh without discarding valid small shells.
    ///
    /// # Errors
    /// Returns [`RobustCsgError`] when the source cannot represent a closed,
    /// consistently oriented additive solid with unambiguous nested voids.
    pub fn new(mesh: &RobustMesh) -> Result<Self, RobustCsgError> {
        validate_mesh(mesh)?;
        let mut shells = recover_shells(mesh)?;
        shells.sort_by(shell_ordering);
        let solids = match build_oriented_solids(&shells) {
            Ok(solids) => solids,
            Err(original_error) => {
                for shell in &mut shells {
                    shell.reverse_winding();
                }
                build_oriented_solids(&shells).map_err(|_| original_error)?
            }
        };
        let mut local_meshes = Vec::with_capacity(solids.len());
        for solid in &solids {
            solid.status().map_err(kernel_error)?;
            let volume = solid.volume();
            if solid.is_empty() || !volume.is_finite() || volume <= 0.0 {
                return Err(invalid_input("prepared solid cluster is empty"));
            }
            let local = extract_part(solid.clone())?;
            if local.positions.is_empty() || local.indices.is_empty() {
                return Err(invalid_input(
                    "prepared solid cluster has no output triangles",
                ));
            }
            local_meshes.push(RobustMesh {
                positions: local.positions,
                indices: local.indices,
            });
        }

        Ok(Self {
            inner: Arc::new(PreparedSolidInner {
                solids,
                local_meshes,
            }),
        })
    }

    /// Return the local-space enclosed volume without exposing native state.
    #[must_use]
    pub fn local_volume(&self) -> f64 {
        self.inner.solids.iter().map(Manifold::volume).sum()
    }

    /// Split this prepared solid at one local-to-world placement.
    ///
    /// The transform is a column-major affine 4x3 matrix.
    ///
    /// # Errors
    /// Returns [`RobustCsgError`] for an invalid transform, separator, or CSG
    /// result.
    pub fn split_with_separator_disc(
        &self,
        transform: &[f64; 12],
        disc: SeparatorDisc,
    ) -> Result<crate::RobustSplit, RobustCsgError> {
        split_prepared_with_separator_disc(self, transform, disc)
    }

    pub(crate) fn transformed_manifolds(
        &self,
        transform: &[f64; 12],
    ) -> Result<Vec<Manifold>, RobustCsgError> {
        let determinant = validate_transform(transform)?;
        let transformed = if determinant < 0.0 {
            self.inner
                .local_meshes
                .iter()
                .map(|mesh| transform_mesh(mesh, transform, true))
                .map(|mesh| {
                    mesh.and_then(|mesh| {
                        let (minimum, maximum) = position_bounds(&mesh.positions);
                        manifold_from_closed_shell(&mesh, minimum, maximum)
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            self.inner
                .solids
                .iter()
                .map(|solid| solid.transform(transform))
                .collect()
        };
        for solid in &transformed {
            solid.status().map_err(kernel_error)?;
        }
        Ok(transformed)
    }
}

#[derive(Clone)]
struct Shell {
    positions: Vec<[f64; 3]>,
    indices: Vec<u64>,
    signed_volume: f64,
    minimum: [f64; 3],
    maximum: [f64; 3],
}

impl Shell {
    fn reverse_winding(&mut self) {
        for triangle in self.indices.chunks_exact_mut(3) {
            triangle.swap(1, 2);
        }
        self.signed_volume = -self.signed_volume;
    }

    fn reversed(&self) -> Self {
        let mut shell = self.clone();
        shell.reverse_winding();
        shell
    }

    fn manifold(&self) -> Result<Manifold, RobustCsgError> {
        manifold_from_closed_shell(
            &RobustMesh {
                positions: self.positions.clone(),
                indices: self.indices.clone(),
            },
            self.minimum,
            self.maximum,
        )
    }
}

#[derive(Clone, Copy)]
struct EdgeUse {
    triangle: usize,
    from: u64,
    to: u64,
    opposite: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PositionKey([u64; 3]);

fn recover_shells(mesh: &RobustMesh) -> Result<Vec<Shell>, RobustCsgError> {
    reject_geometrically_collapsed_faces(mesh)?;
    let raw_triangles = mesh
        .indices
        .chunks_exact(3)
        .map(|face| [face[0], face[1], face[2]])
        .collect::<Vec<_>>();
    if let Ok(shells) = recover_from_topology(&mesh.positions, &raw_triangles, false) {
        return Ok(shells);
    }

    let (positions, triangles) = canonicalize_mesh(mesh)?;
    recover_from_topology(&positions, &triangles, true)
}

fn reject_geometrically_collapsed_faces(mesh: &RobustMesh) -> Result<(), RobustCsgError> {
    for face in mesh.indices.chunks_exact(3) {
        let keys = face
            .iter()
            .map(|&index| {
                let position = mesh.positions[usize::try_from(index)
                    .map_err(|_| invalid_input("mesh index exceeds platform address space"))?];
                Ok(PositionKey(position.map(canonical_zero).map(f64::to_bits)))
            })
            .collect::<Result<Vec<_>, RobustCsgError>>()?;
        if keys[0] == keys[1] || keys[1] == keys[2] || keys[2] == keys[0] {
            return Err(invalid_input(
                "mesh contains a geometrically degenerate triangle",
            ));
        }
    }
    Ok(())
}

fn recover_from_topology(
    positions: &[[f64; 3]],
    triangles: &[[u64; 3]],
    allow_edge_fans: bool,
) -> Result<Vec<Shell>, RobustCsgError> {
    let mut edges: BTreeMap<(u64, u64), Vec<EdgeUse>> = BTreeMap::new();
    for (triangle, &[a, b, c]) in triangles.iter().enumerate() {
        for (from, to, opposite) in [(a, b, c), (b, c, a), (c, a, b)] {
            let key = ordered_edge(from, to);
            edges.entry(key).or_default().push(EdgeUse {
                triangle,
                from,
                to,
                opposite,
            });
        }
    }

    let mut components = DisjointSet::new(triangles.len());
    // Ordinary two-face edges first recover each shell away from geometric
    // contacts. Fan edges can then preserve that identity instead of guessing
    // solely from a locally symmetric angle arrangement.
    for uses in edges.values() {
        if let [left, right] = uses.as_slice() {
            pair_edge(&mut components, *left, *right)?;
        }
    }
    for (edge, uses) in &edges {
        match uses.as_slice() {
            [_, _] => {}
            _ if allow_edge_fans && uses.len() >= 4 && uses.len() % 2 == 0 => {
                for (left, right) in pair_edge_fan(*edge, uses, positions, &mut components)? {
                    components.union(left.triangle, right.triangle);
                }
            }
            _ => return Err(invalid_input("recovered shell is open or non-manifold")),
        }
    }

    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for triangle in 0..triangles.len() {
        let root = components.root(triangle);
        groups.entry(root).or_default().push(triangle);
    }
    let mut shells = groups
        .values()
        .map(|members| extract_shell(positions, triangles, members))
        .collect::<Result<Vec<_>, _>>()?;
    if shells.is_empty() {
        return Err(invalid_input("mesh contains no closed shell"));
    }
    shells.sort_by(shell_ordering);
    Ok(shells)
}

fn extract_shell(
    positions: &[[f64; 3]],
    triangles: &[[u64; 3]],
    members: &[usize],
) -> Result<Shell, RobustCsgError> {
    let mut remap = BTreeMap::new();
    let mut shell_positions = Vec::new();
    let mut shell_indices = Vec::with_capacity(members.len() * 3);
    for &triangle_id in members {
        for &global_index in &triangles[triangle_id] {
            let local_index = if let Some(&local_index) = remap.get(&global_index) {
                local_index
            } else {
                let local_index = u64::try_from(shell_positions.len())
                    .map_err(|_| invalid_input("shell vertex count exceeds u64::MAX"))?;
                let position = positions
                    .get(usize::try_from(global_index).map_err(|_| {
                        invalid_input("shell vertex index exceeds platform address space")
                    })?)
                    .copied()
                    .ok_or_else(|| invalid_input("shell vertex index is out of range"))?;
                remap.insert(global_index, local_index);
                shell_positions.push(position);
                local_index
            };
            shell_indices.push(local_index);
        }
    }
    validate_shell_topology(&shell_indices)?;
    let signed_volume = signed_volume(&shell_positions, &shell_indices)?;
    if !signed_volume.is_finite() || signed_volume == 0.0 {
        return Err(invalid_input("shell has no finite signed volume"));
    }
    let (minimum, maximum) = position_bounds(&shell_positions);
    Ok(Shell {
        positions: shell_positions,
        indices: shell_indices,
        signed_volume,
        minimum,
        maximum,
    })
}

fn canonicalize_mesh(mesh: &RobustMesh) -> Result<CanonicalMesh, RobustCsgError> {
    let mut positions = Vec::new();
    let mut position_ids = BTreeMap::new();
    let mut source_to_canonical = Vec::with_capacity(mesh.positions.len());
    for &position in &mesh.positions {
        let position = position.map(canonical_zero);
        let key = PositionKey(position.map(f64::to_bits));
        let canonical = if let Some(&existing) = position_ids.get(&key) {
            existing
        } else {
            let index = u64::try_from(positions.len())
                .map_err(|_| invalid_input("vertex count exceeds u64::MAX"))?;
            positions.push(position);
            position_ids.insert(key, index);
            index
        };
        source_to_canonical.push(canonical);
    }
    let canonical_indices = mesh
        .indices
        .iter()
        .map(|&index| {
            source_to_canonical
                .get(
                    usize::try_from(index)
                        .map_err(|_| invalid_input("mesh index exceeds platform address space"))?,
                )
                .copied()
                .ok_or_else(|| invalid_input("mesh index is out of range"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((
        positions,
        canonical_indices
            .chunks_exact(3)
            .map(|face| [face[0], face[1], face[2]])
            .collect(),
    ))
}

fn pair_edge(
    components: &mut DisjointSet,
    left: EdgeUse,
    right: EdgeUse,
) -> Result<(), RobustCsgError> {
    if left.from != right.to || left.to != right.from {
        return Err(invalid_input(
            "shell winding is inconsistent across an edge",
        ));
    }
    components.union(left.triangle, right.triangle);
    Ok(())
}

fn pair_edge_fan(
    edge: (u64, u64),
    uses: &[EdgeUse],
    positions: &[[f64; 3]],
    components: &mut DisjointSet,
) -> Result<Vec<(EdgeUse, EdgeUse)>, RobustCsgError> {
    let (forward, reverse): (Vec<_>, Vec<_>) = uses
        .iter()
        .copied()
        .partition(|edge_use| edge_use.from == edge.0 && edge_use.to == edge.1);
    if forward.len() != reverse.len()
        || forward.is_empty()
        || forward.len() > MAX_EDGE_FAN_PAIRS
        || reverse
            .iter()
            .any(|edge_use| edge_use.from != edge.1 || edge_use.to != edge.0)
    {
        return Err(invalid_input("non-manifold edge fan is too ambiguous"));
    }
    if let Some(pairs) = pair_fan_by_existing_components(&forward, &reverse, components) {
        return Ok(pairs);
    }
    let edge_start = indexed_position(positions, edge.0)?;
    let edge_end = indexed_position(positions, edge.1)?;
    let direction = edge_end - edge_start;
    if direction.length_squared() <= f64::EPSILON {
        return Err(invalid_input("edge fan has a zero-length edge"));
    }
    let forward_radials = forward
        .iter()
        .map(|edge_use| edge_radial(*edge_use, edge_start, direction, positions))
        .collect::<Result<Vec<_>, _>>()?;
    let reverse_radials = reverse
        .iter()
        .map(|edge_use| edge_radial(*edge_use, edge_start, direction, positions))
        .collect::<Result<Vec<_>, _>>()?;
    let costs = forward_radials
        .iter()
        .map(|left| {
            reverse_radials
                .iter()
                .map(|right| 1.0 - left.dot(*right))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut search = FanPairSearch::new(&costs);
    search.visit(0, 0.0);
    if search.ambiguous || search.best_selection.is_empty() {
        return Err(invalid_input("non-manifold edge fan pairing is ambiguous"));
    }
    Ok(search
        .best_selection
        .iter()
        .enumerate()
        .map(|(index, &reverse_index)| (forward[index], reverse[reverse_index]))
        .collect())
}

fn pair_fan_by_existing_components(
    forward: &[EdgeUse],
    reverse: &[EdgeUse],
    components: &mut DisjointSet,
) -> Option<Vec<(EdgeUse, EdgeUse)>> {
    let reverse_roots = reverse
        .iter()
        .map(|edge_use| components.root(edge_use.triangle))
        .collect::<Vec<_>>();
    let mut used = vec![false; reverse.len()];
    let mut pairs = Vec::with_capacity(forward.len());
    for &forward_use in forward {
        let root = components.root(forward_use.triangle);
        let mut matches = reverse_roots
            .iter()
            .enumerate()
            .filter(|(index, reverse_root)| !used[*index] && **reverse_root == root);
        let (reverse_index, _) = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        used[reverse_index] = true;
        pairs.push((forward_use, reverse[reverse_index]));
    }
    used.iter().all(|is_used| *is_used).then_some(pairs)
}

fn edge_radial(
    edge_use: EdgeUse,
    edge_start: DVec3,
    edge_direction: DVec3,
    positions: &[[f64; 3]],
) -> Result<DVec3, RobustCsgError> {
    let opposite = indexed_position(positions, edge_use.opposite)?;
    let unit_edge = edge_direction.normalize();
    let radial = opposite - (edge_start + edge_direction * 0.5);
    let radial = radial - unit_edge * radial.dot(unit_edge);
    if radial.length_squared() <= f64::EPSILON {
        return Err(invalid_input("edge fan has a degenerate adjacent face"));
    }
    Ok(radial.normalize())
}

fn indexed_position(positions: &[[f64; 3]], index: u64) -> Result<DVec3, RobustCsgError> {
    positions
        .get(
            usize::try_from(index)
                .map_err(|_| invalid_input("vertex index exceeds platform address space"))?,
        )
        .copied()
        .map(DVec3::from_array)
        .ok_or_else(|| invalid_input("vertex index is out of range"))
}

struct FanPairSearch<'a> {
    costs: &'a [Vec<f64>],
    used: Vec<bool>,
    selection: Vec<usize>,
    best_selection: Vec<usize>,
    best_cost: f64,
    ambiguous: bool,
}

impl<'a> FanPairSearch<'a> {
    fn new(costs: &'a [Vec<f64>]) -> Self {
        Self {
            costs,
            used: vec![false; costs.len()],
            selection: Vec::with_capacity(costs.len()),
            best_selection: Vec::new(),
            best_cost: f64::INFINITY,
            ambiguous: false,
        }
    }

    fn visit(&mut self, position: usize, cost: f64) {
        if cost > self.best_cost + FAN_PAIRING_TIE_TOLERANCE {
            return;
        }
        if position == self.costs.len() {
            if cost < self.best_cost - FAN_PAIRING_TIE_TOLERANCE {
                self.best_cost = cost;
                self.best_selection.clone_from(&self.selection);
                self.ambiguous = false;
            } else if (cost - self.best_cost).abs() <= FAN_PAIRING_TIE_TOLERANCE {
                self.ambiguous = true;
            }
            return;
        }
        for reverse in 0..self.costs.len() {
            if self.used[reverse] {
                continue;
            }
            self.used[reverse] = true;
            self.selection.push(reverse);
            self.visit(position + 1, cost + self.costs[position][reverse]);
            self.selection.pop();
            self.used[reverse] = false;
        }
    }
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
        if left != right {
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
}

fn build_oriented_solids(shells: &[Shell]) -> Result<Vec<Manifold>, RobustCsgError> {
    let mut additives = Vec::new();
    let mut voids = Vec::new();
    for (shell_index, shell) in shells.iter().enumerate() {
        if shell.signed_volume > 0.0 {
            additives.push(shell);
        } else if shell.signed_volume < 0.0 {
            voids.push((shell_index, shell));
        } else {
            return Err(RobustCsgError::AmbiguousShellWinding {
                shell: shell_index,
                reason: "shell has zero signed volume".to_string(),
            });
        }
    }
    if additives.is_empty() {
        return Err(invalid_input("mesh contains no outward additive shell"));
    }

    let additive_manifolds = additives
        .iter()
        .map(|shell| shell.manifold())
        .collect::<Result<Vec<_>, _>>()?;
    let additive_bounds = additives
        .iter()
        .map(|shell| (shell.minimum, shell.maximum))
        .collect::<Vec<_>>();
    let additive_solids = union_overlapping_clusters(&additive_manifolds, &additive_bounds)?;
    let voids = voids
        .into_iter()
        .map(|(shell_index, shell)| Ok((shell_index, shell.reversed().manifold()?)))
        .collect::<Result<Vec<_>, RobustCsgError>>()?;
    subtract_contained_voids(additive_solids, voids)
}

fn validate_shell_topology(indices: &[u64]) -> Result<(), RobustCsgError> {
    let mut edges: BTreeMap<(u64, u64), Vec<(u64, u64)>> = BTreeMap::new();
    for triangle in indices.chunks_exact(3) {
        for (from, to) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            edges
                .entry(ordered_edge(from, to))
                .or_default()
                .push((from, to));
        }
    }
    if edges
        .values()
        .any(|uses| uses.len() != 2 || uses[0].0 != uses[1].1 || uses[0].1 != uses[1].0)
    {
        return Err(invalid_input("recovered shell is open or non-manifold"));
    }
    Ok(())
}

fn transform_mesh(
    mesh: &RobustMesh,
    transform: &[f64; 12],
    reverse_winding: bool,
) -> Result<RobustMesh, RobustCsgError> {
    let positions = mesh
        .positions
        .iter()
        .map(|&position| {
            let transformed = [
                transform[0] * position[0]
                    + transform[3] * position[1]
                    + transform[6] * position[2]
                    + transform[9],
                transform[1] * position[0]
                    + transform[4] * position[1]
                    + transform[7] * position[2]
                    + transform[10],
                transform[2] * position[0]
                    + transform[5] * position[1]
                    + transform[8] * position[2]
                    + transform[11],
            ];
            DVec3::from_array(transformed)
                .is_finite()
                .then_some(transformed)
                .ok_or_else(|| RobustCsgError::InvalidTransform {
                    reason: "transformed position is non-finite".to_string(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut indices = mesh.indices.clone();
    if reverse_winding {
        for triangle in indices.chunks_exact_mut(3) {
            triangle.swap(1, 2);
        }
    }
    Ok(RobustMesh { positions, indices })
}

fn validate_transform(transform: &[f64; 12]) -> Result<f64, RobustCsgError> {
    if transform.iter().any(|value| !value.is_finite()) {
        return Err(invalid_transform("matrix contains non-finite values"));
    }
    let matrix = DMat3::from_cols(
        DVec3::new(transform[0], transform[1], transform[2]),
        DVec3::new(transform[3], transform[4], transform[5]),
        DVec3::new(transform[6], transform[7], transform[8]),
    );
    let determinant = matrix.determinant();
    if !determinant.is_finite() || determinant == 0.0 {
        return Err(invalid_transform("linear transform is singular"));
    }
    let inverse = matrix.inverse();
    let max_axis = matrix
        .x_axis
        .length()
        .max(matrix.y_axis.length())
        .max(matrix.z_axis.length());
    let max_inverse_axis = inverse
        .x_axis
        .length()
        .max(inverse.y_axis.length())
        .max(inverse.z_axis.length());
    if !inverse.is_finite() || max_axis * max_inverse_axis >= 1.0e10 {
        return Err(invalid_transform("linear transform is too ill-conditioned"));
    }
    Ok(determinant)
}

fn signed_volume(positions: &[[f64; 3]], indices: &[u64]) -> Result<f64, RobustCsgError> {
    let position = |index: u64| {
        positions
            .get(
                usize::try_from(index)
                    .map_err(|_| invalid_input("shell index exceeds platform address space"))?,
            )
            .copied()
            .map(DVec3::from_array)
            .ok_or_else(|| invalid_input("shell index is out of range"))
    };
    let origin = position(indices[0])?;
    indices.chunks_exact(3).try_fold(0.0, |volume, triangle| {
        let a = position(triangle[0])? - origin;
        let b = position(triangle[1])? - origin;
        let c = position(triangle[2])? - origin;
        Ok(volume + a.dot(b.cross(c)) / 6.0)
    })
}

fn shell_ordering(left: &Shell, right: &Shell) -> std::cmp::Ordering {
    left.minimum
        .iter()
        .chain(left.maximum.iter())
        .zip(right.minimum.iter().chain(right.maximum.iter()))
        .map(|(left, right)| left.total_cmp(right))
        .find(|ordering| *ordering != std::cmp::Ordering::Equal)
        .unwrap_or_else(|| left.indices.len().cmp(&right.indices.len()))
}

fn ordered_edge(from: u64, to: u64) -> (u64, u64) {
    if from < to {
        (from, to)
    } else {
        (to, from)
    }
}

fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

fn invalid_transform(reason: &str) -> RobustCsgError {
    RobustCsgError::InvalidTransform {
        reason: reason.to_string(),
    }
}
