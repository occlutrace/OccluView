//! Shared topology recovery: weld an STL-style triangle SOUP back to the shared
//! vertex topology its exporter actually authored.
//!
//! STL — the dental workhorse — stores every triangle's three corners as fresh,
//! independent vertices, so no two triangles share a vertex index. In index
//! space that makes EVERY edge a boundary half-edge and the whole model reads as
//! a cloud of disconnected needles. Any operation that reasons about *shared
//! topology* — boundary walks (hole filling), connected components (Separate) —
//! is meaningless on soup until the shared corners are merged. This module is the
//! one place that recovery lives, so every consumer gets identical, deterministic
//! behavior.
//!
//! This is NOT the repair pipeline's weld (`repair/weld.rs`). That one is
//! tolerance-based (an epsilon quantizer that fuses *near*-coincident vertices to
//! heal genuine cracks). This one is EXACT-BIT: only byte-identical payloads
//! (position bits + color + uv bits) merge, so it recovers precisely the topology
//! the exporter welded and can never fuse two distinct points or blur anatomy.
//! Keep the two separate — conflating them would either blur soup anatomy or fail
//! to recover exact duplicates.

use super::{EditVertex, MeshEditBuffers, MeshEditError};

/// Full-payload weld key for soup recovery: exact position bits + color + uv
/// bits. STL and other soup formats write byte-identical coordinates for a
/// shared corner, so an exact key merges exactly the true duplicates and can
/// never fuse two genuinely distinct points. Normals are deliberately excluded:
/// a shared corner carries a different per-FACE normal in each incident triangle,
/// yet is the same topological vertex.
type SoupWeldKey = ([u32; 3], [u8; 4], [u32; 2]);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum TopologyWeldPolicy {
    FullPayload,
    PositionOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum TopologyWeldKey {
    FullPayload(SoupWeldKey),
    PositionOnly([u32; 3]),
}

#[derive(Clone, Debug)]
pub(crate) struct CanonicalTopology {
    indices: Vec<u32>,
    merged_vertices: usize,
}

impl CanonicalTopology {
    pub(crate) fn indices(&self) -> &[u32] {
        &self.indices
    }

    fn merged_vertices(&self) -> usize {
        self.merged_vertices
    }
}

pub(crate) fn indexed_topology(mesh: &MeshEditBuffers) -> CanonicalTopology {
    CanonicalTopology {
        indices: mesh.indices.clone(),
        merged_vertices: 0,
    }
}

fn soup_weld_key(vertex: &EditVertex) -> SoupWeldKey {
    (
        [
            vertex.position[0].to_bits(),
            vertex.position[1].to_bits(),
            vertex.position[2].to_bits(),
        ],
        vertex.color,
        [vertex.uv[0].to_bits(), vertex.uv[1].to_bits()],
    )
}

pub(crate) fn canonical_position_key(position: [f32; 3]) -> [u32; 3] {
    position.map(|component| {
        if component == 0.0 {
            0.0_f32.to_bits()
        } else {
            component.to_bits()
        }
    })
}

fn topology_weld_key(vertex: &EditVertex, policy: TopologyWeldPolicy) -> TopologyWeldKey {
    match policy {
        TopologyWeldPolicy::FullPayload => TopologyWeldKey::FullPayload(soup_weld_key(vertex)),
        TopologyWeldPolicy::PositionOnly => {
            TopologyWeldKey::PositionOnly(canonical_position_key(vertex.position))
        }
    }
}

pub(crate) fn canonical_topology(
    mesh: &MeshEditBuffers,
    policy: TopologyWeldPolicy,
) -> Result<CanonicalTopology, MeshEditError> {
    let vertex_count = mesh.vertices.len();
    if vertex_count == 0 {
        return Ok(CanonicalTopology {
            indices: mesh.indices.clone(),
            merged_vertices: 0,
        });
    }

    let mut keyed: Vec<(TopologyWeldKey, usize)> = mesh
        .vertices
        .iter()
        .enumerate()
        .map(|(index, vertex)| (topology_weld_key(vertex, policy), index))
        .collect();
    keyed.sort_unstable();

    let mut representative_of: Vec<usize> = (0..vertex_count).collect();
    let mut merged_vertices = 0_usize;
    let mut run_start = 0;
    for scan in 1..=keyed.len() {
        if scan != keyed.len() && keyed[scan].0 == keyed[run_start].0 {
            continue;
        }
        let representative = keyed[run_start].1;
        for entry in &keyed[run_start..scan] {
            if entry.1 != representative {
                representative_of[entry.1] = representative;
                merged_vertices += 1;
            }
        }
        run_start = scan;
    }

    let representative_of = representative_of
        .into_iter()
        .map(|representative| {
            u32::try_from(representative).map_err(|_| MeshEditError::MalformedMesh {
                reason: "welded representative id exceeds u32::MAX".to_string(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut indices = Vec::with_capacity(mesh.indices.len());
    for (at_index, &index) in mesh.indices.iter().enumerate() {
        let representative = representative_of.get(index as usize).ok_or_else(|| {
            MeshEditError::MalformedMesh {
                reason: format!(
                    "index {index} at position {at_index} is out of range for vertex_count {vertex_count}"
                ),
            }
        })?;
        indices.push(*representative);
    }

    Ok(CanonicalTopology {
        indices,
        merged_vertices,
    })
}

/// Weld an STL-style triangle soup back to shared topology.
///
/// STL stores each triangle's corners as independent vertices — no two triangles
/// share a vertex index. In index space that makes every edge a boundary and the
/// whole model a cloud of disconnected needles, so boundary walks find no real
/// rims and connected-component analysis returns one component per triangle (the
/// "317k confetti parts" Separate failure and the "524560 nicks healed, none
/// closed" hole-fill failure are the same root cause). This merges the shared
/// corners so those analyses see the true topology.
///
/// The merge is EXACT (position bits + color + uv): a CAD exporter writes
/// byte-identical coordinates for a shared corner, so this recovers the real
/// topology the exporter welded, and — because only byte-identical payloads
/// merge — it can never fuse two distinct points or blur anatomy. Attributes are
/// part of the key, honoring the anti-weld doctrine: a position duplicated with
/// different colors/UVs (dental color scans) stays distinct.
///
/// Triangle order and count are preserved — only vertex ids are remapped — so a
/// face selection keyed by triangle index stays valid on the returned buffers.
/// Every original vertex is kept; the now-unreferenced duplicates are dropped
/// later by any caller's vertex compaction, which is where the soup shrinks.
///
/// Returns `Ok(None)` when nothing merges (an already-welded mesh), leaving the
/// caller byte-for-byte unchanged. Deterministic: sort-based grouping with the
/// lowest original id elected representative.
///
/// # Errors
/// Returns [`MeshEditError::MalformedMesh`] if a representative id cannot be
/// expressed as a `u32` (a representative is always at or below a referenced
/// index, so this never fires for a validated mesh).
pub(crate) fn weld_soup_topology(
    mesh: &MeshEditBuffers,
) -> Result<Option<MeshEditBuffers>, MeshEditError> {
    let canonical = canonical_topology(mesh, TopologyWeldPolicy::FullPayload)?;
    if canonical.merged_vertices() == 0 {
        return Ok(None);
    }
    Ok(Some(MeshEditBuffers {
        vertices: mesh.vertices.clone(),
        indices: canonical.indices,
        topology: mesh.topology,
    }))
}
