use super::adjacency::vertex_index;
use super::cap_fair::{shape_cap, Continuity, RimSupport};
use super::cap_guard::{
    candidate_pierces, cap_doubles_surface, CapCandidate, VertexTriangleIncidence,
};
use super::cap_minweight::{min_weight_triangulation_any, rim_is_simple_3d, TakenTriangles};
use super::cap_refine::{refine_cap, CapDomain};
use super::cap_support::{
    build_vertex_adjacency, rim_neighbourhood, rim_outside_support, rim_taken_triangles,
};
use super::holes_cleanup::{hanging_leftovers, RimHealOutcome};
use super::holes_gate::{
    border_perimeter_threshold, collect_boundary_loops, refuse_unweldable_soup,
    rim_exceeds_size_cap, rim_hold, MarkedVertices, RimHold,
};
use super::holes_region::{CappedSurface, MarkedFaces, MarkedRegion};
use super::holes_walk::{
    build_boundary_maps, ear_clip_cap, push_cap_index, vertex_position, BoundaryOwners,
};
use super::{
    copy_surviving_vertices, recompute_all_normals, remap_triangle_indices,
    validate_face_edit_buffers, validate_mesh_edit_options,
    validate_selection_against_triangle_count, EditVertex, FaceSelection, MeshEditBuffers,
    MeshEditError, MeshEditOptions, MeshEditReport, MeshEditResult, MeshEditWarning, MeshTopology,
};
use glam::Vec3;
use std::collections::HashSet;

/// Loops at or above this many edges get a shaped cap (refined interior
/// vertices on the surface that continues the scan); smaller holes keep the
/// plain planar cap.
const MIN_SHAPED_LOOP: usize = 8;

/// Rims longer than this are not refined in their own plane and go straight to
/// the membrane as the base of their cap. Real lasso cuts
/// routinely produce 200–1000-edge rims, and the raw membrane on such a rim is
/// full of near-folded creases (sharp spike-like artifacts), so a shaped cap
/// must cover them. Cost is held by `cap_refine`'s interior-vertex budget (the
/// target edge scale coarsens for big rims), so the ceiling is only a safety
/// valve against pathological mega-rims.
const MAX_PLANAR_LOOP: usize = 4096;

/// The edge ceiling for closing a hole: how many boundary edges one rim may
/// have before the tool refuses it.
///
/// Bounded well under the ear-clip's `u16` rim limit and within its measured
/// cost, so a huge accidental selection cannot hang the tool. With a face
/// selection present the gate takes `max(options.max_boundary_loop, this)`,
/// because an explicit selection is the operator's intent and large marked
/// rims should still close.
///
/// Exported so every caller reads this one number: because of that `max`, a
/// caller-side copy larger than this constant would override any change to it.
pub const CLOSE_HOLES_EDGE_CEILING: usize = 20_000;

#[derive(Copy, Clone)]
struct FillInputCounts {
    vertices: usize,
    triangles: usize,
}

/// Per-run tallies that only become known once every boundary loop has been
/// walked: how many were closed and how many were skipped, broken down by
/// reason (scan border, size restraint, damage).
///
/// [`fill_holes`] folds every skip kind into one
/// [`MeshEditWarning::DegenerateGeometry`] per skipped loop (its public
/// contract) and mirrors the breakdown in the report's `skipped_*`
/// counters. The repair pipeline maps oversize skips to informational
/// `open_rims_left`; it disables the border guard, so `skipped_border` stays
/// zero there.
#[derive(Copy, Clone, Default)]
pub(crate) struct FillLoopStats {
    /// Boundary loops closed with a complete cap.
    pub(crate) filled: usize,
    /// Rims protected as the scan's natural outer boundary.
    pub(crate) skipped_border: usize,
    /// Rims skipped only because they exceed the effective size cap.
    pub(crate) skipped_oversize: usize,
    /// Rims skipped as non-simple, too short, degenerate, or with a cap that
    /// would pierce nearby surface.
    pub(crate) skipped_degenerate: usize,
    /// Rims a mark touches but does not hold.
    pub(crate) skipped_partial: usize,
}

/// Close boundary loops in a triangle mesh, matching the convention dental CAD
/// software uses: every interior hole closes by default; only the scan's
/// natural outer boundary is protected.
///
/// Border protection (`options.protect_scan_border`, default on, whole-mesh
/// path only): a rim whose perimeter is at least half of the largest rim's
/// perimeter and at least half of the mesh bounding-box diagonal is treated
/// as the open scan border and left alone. Everything else closes regardless
/// of size, bounded only by `options.max_boundary_loop` (sanity/perf ceiling)
/// and, when set, the optional `options.max_rim_perimeter_mm` restraint.
///
/// When `selection` is present, a rim is capped when the mark holds it: the
/// operator has marked at least half of its owning faces, and the rim lies
/// inside the box of the marked faces, give or take the faces a lasso misses
/// at its far edge (explicit intent). The selection loop ceiling is lifted and
/// the border guard is bypassed; an explicitly enabled mm perimeter restraint
/// still applies. A rim the mark touches without holding stays open and is
/// counted in `skipped_partial_rims`. The work is done on the faces in that
/// box: the rest of the mesh comes back as it went in.
///
/// What the cut left in a hole that closes goes with the cap: the teeth of its
/// rim, the rags of unmarked surface hanging in it by a thread (counted with
/// the healed defects), and the loose flakes floating in it.
///
/// Two rims that meet at a single vertex are pre-split during pinch handling so
/// both become independent simple loops that fill; an unsplit shared junction
/// dead-ends the walk and neither loop closes.
///
/// Caps use the "interpolated from surrounding edges" convention dental CAD
/// software uses: loops above the shaped-cap edge threshold are refined with
/// generated interior vertices (density matched to the rim, attributes per
/// `options.attribute_policy`) and laid on the surface that leaves the rim the
/// way the mesh arrives at it, so a cut tooth closes with a dome that follows
/// its walls. The faces that hang on such a rim by one edge, the teeth of a cut
/// line, are removed with it. Tiny holes keep a plain planar cap on rim
/// vertices only. Strongly curved rims whose planar projection self-overlaps
/// are capped from a projection-free minimum-weight triangulation. Every
/// candidate cap is refused (never emitted) if it would pierce itself or the
/// surface around its rim, or lie on a triangle the surface already has.
///
/// Guards, each surfaced as one [`MeshEditWarning::DegenerateGeometry`] per
/// skipped loop in the returned report, with per-reason counts in the
/// report's `skipped_border_rims` / `skipped_oversize_rims` /
/// `skipped_damaged_rims`:
/// - border-protected rims stay open (border counter),
/// - loops longer than the effective edge cap, or over the mm restraint, are
///   skipped (oversize counter),
/// - non-simple / numerically stalled loops, degenerate planar normals, and
///   caps that run into the surface are skipped (damaged counter).
///
/// Loops left open because a selection did not hold their rim are not warned
/// about — that is requested behavior, not degeneracy.
///
/// # Errors
/// Returns typed validation errors for unsupported point clouds, malformed
/// triangle data, invalid selection masks, invalid options, or invalid rebuilt
/// triangle indices.
pub fn fill_holes(
    mesh: &MeshEditBuffers,
    selection: Option<&FaceSelection>,
    options: MeshEditOptions,
) -> Result<MeshEditResult, MeshEditError> {
    fill_holes_with_outcome(mesh, selection, options).map(|(result, _)| result)
}

/// Close only holes whose rim an explicit face selection holds. This is the
/// user-facing Mesh Editor contract; an empty selection is a valid no-op and
/// never widens into a whole-mesh repair.
///
/// # Errors
/// Returns the same typed validation or cap errors as [`fill_holes`].
pub fn fill_selected_holes(
    mesh: &MeshEditBuffers,
    selection: &FaceSelection,
    options: MeshEditOptions,
) -> Result<MeshEditResult, MeshEditError> {
    fill_holes(mesh, Some(selection), options)
}

/// [`fill_holes`] plus the per-loop skip breakdown the repair pipeline needs.
///
/// The returned [`MeshEditResult`] is exactly what [`fill_holes`] returns
/// (same mesh, same report, same warnings); the stats only add resolution.
/// Validate the inputs and take the input counts the report is measured against.
fn accept_fill_inputs(
    mesh: &MeshEditBuffers,
    selection: Option<&FaceSelection>,
    options: MeshEditOptions,
) -> Result<(MeshEditOptions, FillInputCounts), MeshEditError> {
    let options = validate_mesh_edit_options(options)?;
    validate_face_edit_buffers(mesh.topology, &mesh.vertices, &mesh.indices)?;
    if let Some(selection) = selection {
        validate_selection_against_triangle_count(mesh.triangle_count(), selection)?;
    }
    Ok((
        options,
        FillInputCounts {
            vertices: mesh.vertices.len(),
            triangles: mesh.triangle_count(),
        },
    ))
}

pub(crate) fn fill_holes_with_outcome(
    mesh: &MeshEditBuffers,
    selection: Option<&FaceSelection>,
    options: MeshEditOptions,
) -> Result<(MeshEditResult, FillLoopStats), MeshEditError> {
    let (options, counts) = accept_fill_inputs(mesh, selection, options)?;

    if counts.triangles == 0 || selection.is_some_and(|mask| mask.selected_count() == 0) {
        let stats = FillLoopStats::default();
        return Ok((unchanged_fill_result(mesh, counts, stats), stats));
    }

    // A mark is local, so the filler works on the faces in the mark's box and
    // the result goes back into the scan. Without a mark the whole mesh is the
    // subject.
    let filled = match selection {
        Some(selection) => {
            let (region, mark) = MarkedRegion::around(mesh, selection);
            let mut filled = cap_rims(&region.mesh, Some(mark), options)?;
            // A mark that closed nothing and healed nothing leaves the scan as
            // it came, and the report says what kept its rims open.
            if filled.stats.filled == 0 && filled.healed_rims == 0 && filled.surface.kept.is_none()
            {
                let stats = filled.stats;
                return Ok((unchanged_fill_result(mesh, counts, stats), stats));
            }
            // The region changed, so its normals are recomputed, on the region
            // alone; the rest of the scan keeps the normals it came with.
            recompute_used_normals(&mut filled.surface.vertices, &filled.surface.indices)?;
            filled.surface = region.splice(mesh, filled.surface);
            filled
        }
        None => cap_rims(mesh, None, options)?,
    };
    let stats = filled.stats;
    let result = finalize_fill_result(
        mesh.topology,
        filled,
        options.compact_vertices,
        selection.is_none(),
        counts,
    )?;
    Ok((result, stats))
}

/// Recompute the normals of the vertices a surface's triangles use; a vertex
/// no triangle uses keeps the normal it has, and does not vote on the normals
/// of the vertices that share its position.
fn recompute_used_normals(
    vertices: &mut [EditVertex],
    indices: &[u32],
) -> Result<(), MeshEditError> {
    let used_indices = surviving_vertex_indices(indices, vertices.len())?;
    let (mut used, remap) = copy_surviving_vertices(vertices, &used_indices)?;
    let remapped = remap_triangle_indices(indices, &remap)?;
    recompute_all_normals(&mut used, &remapped)?;
    for (&vertex, recomputed) in used_indices.iter().zip(&used) {
        vertices[vertex].normal = recomputed.normal;
    }
    Ok(())
}

/// A surface with its qualifying rims capped, and what the run counted.
struct FilledRims {
    surface: CappedSurface,
    stats: FillLoopStats,
    healed_rims: usize,
}

/// Heal the cut line of `mesh`, walk its rims and cap the ones that qualify.
///
/// With `mark` the mesh is the region around an operator's mark and a rim
/// qualifies when the mark holds it; without, the mesh is the whole subject and
/// every rim but the scan border qualifies.
fn cap_rims(
    mesh: &MeshEditBuffers,
    mut mark: Option<MarkedFaces>,
    options: MeshEditOptions,
) -> Result<FilledRims, MeshEditError> {
    let triangles = mesh.triangle_count();
    // The weld runs first, and the soup refusal is decided on its result. The
    // `heal_boundary_rims` flag alone does not mean "welded": healing welds by
    // full payload, so a soup whose coincident corners differ in colour or UV
    // merges nothing, and the healing pass would delete every triangle as an
    // isolated nick.
    let welded = apply_soup_weld(mesh, options.heal_boundary_rims)?;
    let mesh: &MeshEditBuffers = welded.as_ref().unwrap_or(mesh);
    // Decided on the post-weld mesh, by whether its corners are shared — not by
    // the heal flag or the buffer lengths — so a welded surface and a soup are
    // told apart at any size.
    refuse_unweldable_soup(mesh, triangles)?;

    // Pre-clean the cut line (opt-in via `heal_boundary_rims`): drop dangling
    // needle/lone triangles and weld near-coincident boundary vertices so a
    // jagged lasso cut (a digitally extracted tooth) yields clean simple rims
    // that cap — instead of dozens of "damaged" nick loops around the one socket
    // the operator wanted closed. Off by default (repair path), so the buffers
    // below stay byte-for-byte unchanged there.
    let marked = mark.as_ref().map(MarkedFaces::marked);
    let healing = if options.heal_boundary_rims {
        accept_rim_healing(
            mesh,
            super::holes_cleanup::heal_boundary_rims(mesh, marked.as_ref()),
        )
    } else {
        None
    };
    // Rim healing deletes triangles (dangling needles, lone faces, faces
    // collapsed by a weld). They count as removed, so `input - output` agrees
    // with the report and the operator can see that geometry was dropped.
    let (healed, healed_rims, kept) = match healing {
        Some(outcome) => (Some(outcome.mesh), outcome.healed, Some(outcome.keep)),
        None => (None, 0, None),
    };
    let mesh: &MeshEditBuffers = healed.as_ref().unwrap_or(mesh);
    if let (Some(mark), Some(kept)) = (mark.as_mut(), kept.as_ref()) {
        mark.retain(kept);
    }
    let (mut kept, mut healed_rims) = (kept, healed_rims);

    let mut walked = walk_rims(mesh, mark.as_ref(), options)?;
    // What a surface lasso left hanging in the holes that are about to close
    // is part of the cut line too, and the rims run round it: it goes, each
    // piece one healed defect, and the rims are walked again without it.
    let cleaned = match mark.as_mut() {
        Some(mark) if options.heal_boundary_rims => {
            let walked_on = walked.split.as_ref().unwrap_or(mesh);
            let holes: Vec<(Vec3, Vec3)> = walked
                .admitted
                .iter()
                .map(|&rim| rim_box(walked_on, &walked.loops[rim].0))
                .collect();
            let leftovers = hanging_leftovers(walked_on, mark, &holes);
            (!leftovers.faces.is_empty()).then(|| {
                drop_triangles(&mut kept, triangles, &leftovers.faces);
                healed_rims += leftovers.pieces;
                without_faces(walked_on, mark, &leftovers.faces)
            })
        }
        _ => None,
    };
    if let Some(cleaned) = cleaned.as_ref() {
        walked = walk_rims(cleaned, mark.as_ref(), options)?;
    }
    let mesh = walked.split.as_ref().or(cleaned.as_ref()).unwrap_or(mesh);

    let mut surface = CappedSurface {
        vertices: mesh.vertices.clone(),
        indices: mesh.indices.clone(),
        kept,
    };
    let mut stats = walked.stats;
    let admitted: Vec<&[usize]> = walked
        .admitted
        .iter()
        .map(|&rim| walked.loops[rim].0.as_slice())
        .collect();
    if admitted.is_empty() {
        return Ok(FilledRims {
            surface,
            stats,
            healed_rims,
        });
    }

    // Phase 3: fill. A cut line is a saw: every other face on it hangs by one
    // edge and sticks into the hole. The rims that are about to close are
    // capped without those teeth.
    let trimmed = trim_rim_teeth(mesh, &admitted, &walked.owner_by_edge, mark.as_ref());
    let (caps, pulled) = cap_trimmed_rims(&trimmed, options, &mut stats)?;
    // A rim that closed lost its teeth to the cap; one left open keeps them.
    if !pulled.is_empty() {
        let mut next = pulled.iter().copied().peekable();
        surface.indices = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .enumerate()
            .filter(|&(index, _)| next.next_if_eq(&index).is_none())
            .flat_map(|(_, triangle)| *triangle)
            .collect();
        drop_triangles(&mut surface.kept, triangles, &pulled);
    }
    surface.indices.extend(caps.indices);
    surface.vertices.extend(caps.vertices);

    Ok(FilledRims {
        surface,
        stats,
        healed_rims,
    })
}

/// `mesh` without the `gone` faces (ascending), and `mark` brought in line
/// with it.
fn without_faces(
    mesh: &MeshEditBuffers,
    mark: &mut MarkedFaces,
    gone: &[usize],
) -> MeshEditBuffers {
    let mut gone = gone.iter().copied().peekable();
    let keep: Vec<bool> = (0..mesh.triangle_count())
        .map(|face| gone.next_if_eq(&face).is_none())
        .collect();
    mark.retain(&keep);
    MeshEditBuffers {
        vertices: mesh.vertices.clone(),
        indices: mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&keep)
            .filter(|&(_, &keep)| keep)
            .flat_map(|(triangle, _)| *triangle)
            .collect(),
        topology: mesh.topology,
    }
}

/// The rims of a mesh, and which of them are to be capped.
struct WalkedRims {
    /// The mesh the rims were walked on, where it is not the one given: the
    /// same faces with the boundary junctions split.
    split: Option<MeshEditBuffers>,
    owner_by_edge: BoundaryOwners,
    /// Every rim with its perimeter in mm.
    loops: Vec<(Vec<usize>, f64)>,
    /// The rims to cap, as places in `loops`.
    admitted: Vec<usize>,
    /// What the walk and the gate counted.
    stats: FillLoopStats,
}

/// Walk the rims of `mesh` and gate them: with `mark` a rim is admitted when
/// the mark holds it, without when it is not the scan border.
fn walk_rims(
    mesh: &MeshEditBuffers,
    mark: Option<&MarkedFaces>,
    options: MeshEditOptions,
) -> Result<WalkedRims, MeshEditError> {
    // Two rims that meet at a single vertex both dead-end the boundary walk (the
    // junction has no unique successor), so neither would close and the
    // operator would see "random" holes left open next to closed ones. Duplicate each
    // boundary-junction vertex per incident fan so every rim becomes a simple
    // loop that fills. A clean mesh (and repair's already bowtie-split input)
    // has no junctions, so this returns `None` and the path below runs on the
    // original buffers, byte-for-byte unchanged. Triangle count is preserved, so
    // what the mark says of every face still holds. Corners of the faces at a
    // region's edge are left alone: what looks like a junction there is where
    // the region was cut out.
    let at_edge = mark.map(|mark| mark.edge_corners(mesh));
    let split = super::pinch::split_boundary_pinch_vertices(mesh, at_edge.as_deref())?
        .map(|(split, _)| split);
    let mesh = split.as_ref().unwrap_or(mesh);

    let (next_boundary_vertex, owner_by_edge, boundary_starts) = build_boundary_maps(mesh)?;
    let mut stats = FillLoopStats::default();

    // Phase 1: walk every loop first. Border protection needs all rim
    // perimeters before any fill decision can be made.
    let marked_vertices = mark.map(|mark| MarkedVertices {
        marked_corners: mark.marked_corners(mesh),
        edge_corners: mark.edge_corners(mesh),
    });
    let loops = collect_boundary_loops(
        mesh,
        &next_boundary_vertex,
        &boundary_starts,
        marked_vertices.as_ref(),
        &mut stats,
    )?;

    // The scan-border guard applies only without a mark: an explicit mark is
    // operator intent and may close anything it holds.
    let border_guard = options.protect_scan_border && mark.is_none();
    let border_threshold = if border_guard {
        border_perimeter_threshold(mesh, &loops)
    } else {
        f64::INFINITY
    };

    // Phase 2: gate. A rim the mark does not hold staying open is requested
    // behavior, not degeneracy — no warning.
    let mut admitted: Vec<usize> = Vec::new();
    for (slot, (boundary_loop, perimeter)) in loops.iter().enumerate() {
        if let Some(mark) = mark {
            match rim_hold(boundary_loop, &owner_by_edge, mark) {
                RimHold::Held => {}
                RimHold::Partial => {
                    stats.skipped_partial += 1;
                    continue;
                }
                RimHold::Untouched => continue,
            }
        }

        if border_guard && *perimeter >= border_threshold {
            stats.skipped_border += 1;
            continue;
        }

        if rim_exceeds_size_cap(boundary_loop, *perimeter, mark.is_some(), options) {
            stats.skipped_oversize += 1;
            continue;
        }
        admitted.push(slot);
    }
    Ok(WalkedRims {
        split,
        owner_by_edge,
        loops,
        admitted,
        stats,
    })
}

/// The box of a rim's vertices.
fn rim_box(mesh: &MeshEditBuffers, rim: &[usize]) -> (Vec3, Vec3) {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for &vertex in rim {
        let point = mesh
            .vertices
            .get(vertex)
            .map_or(Vec3::NAN, |vertex| Vec3::from_array(vertex.position));
        (lo, hi) = (lo.min(point), hi.max(point));
    }
    (lo, hi)
}

/// Weld an STL-style triangle soup back to shared topology on the Close Holes
/// path (`heal_boundary_rims`), a no-op on the repair path.
///
/// STL stores each triangle's corners as independent vertices, so in index space
/// every edge reads as a boundary and the whole model looks like a cloud of
/// disconnected needles — without this the cut-line healing "heals" one phantom
/// nick per triangle and no real rim is ever found. The weld preserves triangle
/// order, so any face selection stays valid; the repair pipeline runs its own
/// weld and keeps healing off, so it is byte-for-byte untouched here.
///
/// Returns the welded buffers (owned, so the borrow in
/// [`fill_holes_with_outcome`] outlives it) or `None` when nothing merged.
fn apply_soup_weld(
    mesh: &MeshEditBuffers,
    heal_boundary_rims: bool,
) -> Result<Option<MeshEditBuffers>, MeshEditError> {
    if heal_boundary_rims {
        super::topology::weld_soup_topology(mesh)
    } else {
        Ok(None)
    }
}

/// The caps of one run: their triangles, and the vertices generated for them,
/// which take their places after the surface's own.
#[derive(Default)]
struct Caps {
    indices: Vec<u32>,
    vertices: Vec<EditVertex>,
}

/// Cap every trimmed rim that takes a cap. Returns the caps and, ascending,
/// the teeth of the rims that closed; the tallies go to `stats`.
fn cap_trimmed_rims(
    trimmed: &TrimmedRims,
    options: MeshEditOptions,
    stats: &mut FillLoopStats,
) -> Result<(Caps, Vec<usize>), MeshEditError> {
    // Vertex-vertex adjacency of the surrounding surface gives the guard the
    // neighbourhood a cap could run into, and the triangle incidence gives each
    // cap the scan triangles at its rim, so the cap continues the surface
    // there. Both are built once and shared by every loop.
    let adjacency = build_vertex_adjacency(&trimmed.mesh);
    let incidence = VertexTriangleIncidence::build(&trimmed.mesh);
    let context = LoopFillContext {
        mesh: &trimmed.mesh,
        adjacency: &adjacency,
        incidence: &incidence,
    };
    let mut caps = Caps::default();
    let mut pulled: Vec<usize> = Vec::new();
    for rim in &trimmed.rims {
        // Only a complete cap counts as a filled hole. A partial cap is never
        // emitted, so the caps only grow when the whole rim was triangulated.
        if triangulate_loop(&context, &rim.vertices, options, &mut caps)? {
            stats.filled += 1;
            pulled.extend(&rim.teeth);
        } else {
            // No cap that keeps clear of the surface around the rim: skipped.
            stats.skipped_degenerate += 1;
        }
    }
    pulled.sort_unstable();
    pulled.dedup();
    Ok((caps, pulled))
}

/// One rim about to be capped, with what goes with its cap taken off.
struct TrimmedRim {
    /// The rim's vertices in ring order, tooth tips left out.
    vertices: Vec<usize>,
    /// The rim's teeth, and the loose pieces in its hole, as triangles of the
    /// mesh they were taken from.
    teeth: Vec<usize>,
}

/// The rims about to be capped with their teeth off, and the mesh without
/// those teeth and without the loose pieces in the rims' holes.
struct TrimmedRims {
    mesh: MeshEditBuffers,
    rims: Vec<TrimmedRim>,
}

/// Take the teeth off the rims that are about to be capped, and the loose
/// pieces out of their holes.
///
/// A tooth is a face that owns two rim edges in a row: it hangs on the surface
/// by its third edge and sticks into the hole by its own height. A lasso cut
/// leaves one at every other face of its rim. The surface does not continue
/// through a tooth, it ends at the tooth's root, and a cap made to meet both
/// of a tooth's free edges has to turn round its tip inside a notch narrower
/// than its own triangles, where it runs into itself or into the faces next to
/// it. Without the teeth the rim runs along their roots.
///
/// One round: the faces behind the teeth are surface. The teeth of a rim the
/// mark holds go whether they are marked or not: a surface lasso does not
/// mark the ones that look away from the camera.
///
/// With `mark` the mesh is the region around a mark, and a rim on a loose
/// piece that lies in another rim's hole is not capped at all: see
/// [`loose_pieces_in_holes`].
fn trim_rim_teeth(
    mesh: &MeshEditBuffers,
    rims: &[&[usize]],
    owner_by_edge: &BoundaryOwners,
    mark: Option<&MarkedFaces>,
) -> TrimmedRims {
    let debris = mark.map(|mark| loose_pieces_in_holes(mesh, rims, mark));
    let mut rims: Vec<TrimmedRim> = rims
        .iter()
        .enumerate()
        .filter(|&(slot, _)| debris.as_ref().is_none_or(|debris| debris[slot].is_some()))
        .map(|(slot, rim)| {
            let rim_len = rim.len();
            let owner = |index: usize| owner_by_edge.owner(rim[index], rim[(index + 1) % rim_len]);
            // The vertex after edge `index` is a tooth's tip when one face
            // owns that edge and the next, and the face has surface behind it.
            let tip_of: Vec<Option<usize>> = (0..rim_len)
                .map(|index| {
                    let face =
                        owner(index).filter(|&face| owner((index + 1) % rim_len) == Some(face))?;
                    let (root_from, root_to) = (rim[index], rim[(index + 2) % rim_len]);
                    owner_by_edge
                        .owner(root_from, root_to)
                        .is_some()
                        .then_some(face)
                })
                .collect();
            let tips = tip_of.iter().flatten().count();
            let mut pieces = debris
                .as_ref()
                .and_then(|debris| debris[slot].clone())
                .unwrap_or_default();
            // A rim needs three vertices left, and a face that owns the whole
            // rim is not a tooth of it.
            if rim_len - tips < 3 {
                return TrimmedRim {
                    vertices: rim.to_vec(),
                    teeth: pieces,
                };
            }
            pieces.extend(tip_of.iter().flatten());
            TrimmedRim {
                vertices: (0..rim_len)
                    .filter(|&index| tip_of[(index + rim_len - 1) % rim_len].is_none())
                    .map(|index| rim[index])
                    .collect(),
                teeth: pieces,
            }
        })
        .collect();
    let mut teeth: Vec<usize> = rims
        .iter()
        .flat_map(|rim| rim.teeth.iter().copied())
        .collect();
    teeth.sort_unstable();
    teeth.dedup();
    for rim in &mut rims {
        rim.teeth.sort_unstable();
    }

    let mut next_tooth = teeth.into_iter().peekable();
    let indices = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .filter(|&(index, _)| next_tooth.next_if_eq(&index).is_none())
        .flat_map(|(_, triangle)| *triangle)
        .collect();
    TrimmedRims {
        mesh: MeshEditBuffers {
            vertices: mesh.vertices.clone(),
            indices,
            topology: mesh.topology,
        },
        rims,
    }
}

/// For each rim about to be capped, the faces of the loose pieces in its
/// hole; `None` for a rim that is itself on such a piece.
///
/// A piece is loose when it is joined to nothing else and all of it is in the
/// region: a flake the cut left floating. One that lies inside the box of a
/// rim of another piece floats in that rim's hole, where the cap is about to
/// go: left alone it would stick out of the cap, and capped along its own
/// outline it would become a blister. It goes with the cap of that hole, and
/// stays if the hole stays open.
fn loose_pieces_in_holes(
    mesh: &MeshEditBuffers,
    rims: &[&[usize]],
    mark: &MarkedFaces,
) -> Vec<Option<Vec<usize>>> {
    let triangles = mesh.indices.as_chunks::<3>().0;
    // The piece of every vertex: the lowest vertex it is joined to.
    let mut piece_of: Vec<usize> = (0..mesh.vertices.len()).collect();
    let root = |piece_of: &mut [usize], mut vertex: usize| {
        while piece_of[vertex] != vertex {
            piece_of[vertex] = piece_of[piece_of[vertex]];
            vertex = piece_of[vertex];
        }
        vertex
    };
    for triangle in triangles {
        let [a, b, c] = triangle.map(|vertex| root(&mut piece_of, vertex as usize));
        let lowest = a.min(b).min(c);
        for joined in [a, b, c] {
            piece_of[joined] = lowest;
        }
    }
    // Per piece, its box, and whether it is loose: it has faces, and the scan
    // does not go on past any of them.
    let mut boxes = vec![(Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)); mesh.vertices.len()];
    let mut loose = vec![false; mesh.vertices.len()];
    let mut goes_on = vec![false; mesh.vertices.len()];
    for (face, triangle) in triangles.iter().enumerate() {
        let piece = root(&mut piece_of, triangle[0] as usize);
        goes_on[piece] |= mark.is_at_edge(face);
        loose[piece] = !goes_on[piece];
        for &vertex in triangle {
            let point = mesh
                .vertices
                .get(vertex as usize)
                .map_or(Vec3::NAN, |vertex| Vec3::from_array(vertex.position));
            boxes[piece] = (boxes[piece].0.min(point), boxes[piece].1.max(point));
        }
    }

    // The rim whose hole each loose piece lies in: the first that has it.
    let mut hole_of: Vec<Option<usize>> = vec![None; mesh.vertices.len()];
    for (slot, rim) in rims.iter().enumerate() {
        let Some(&first) = rim.first() else {
            continue;
        };
        let own = root(&mut piece_of, first);
        let (lo, hi) = rim_box(mesh, rim);
        for piece in 0..mesh.vertices.len() {
            let (piece_lo, piece_hi) = boxes[piece];
            if piece != own
                && loose[piece]
                && hole_of[piece].is_none()
                && piece_lo.cmpge(lo).all()
                && piece_hi.cmple(hi).all()
            {
                hole_of[piece] = Some(slot);
            }
        }
    }

    let mut debris: Vec<Option<Vec<usize>>> = rims
        .iter()
        .map(|rim| {
            let on_debris = rim
                .first()
                .is_some_and(|&first| hole_of[root(&mut piece_of, first)].is_some());
            (!on_debris).then(Vec::new)
        })
        .collect();
    for (face, triangle) in triangles.iter().enumerate() {
        let piece = root(&mut piece_of, triangle[0] as usize);
        if let Some(faces) = hole_of[piece].and_then(|slot| debris[slot].as_mut()) {
            faces.push(face);
        }
    }
    debris
}

/// Record that `dropped` (ascending) of the surviving triangles are gone:
/// `kept` is over the `original` triangles the run started with.
fn drop_triangles(kept: &mut Option<Vec<bool>>, original: usize, dropped: &[usize]) {
    if dropped.is_empty() {
        return;
    }
    let kept = kept.get_or_insert_with(|| vec![true; original]);
    let mut dropped = dropped.iter().copied().peekable();
    for (survivor, flag) in kept.iter_mut().filter(|flag| **flag).enumerate() {
        if dropped.next_if_eq(&survivor).is_some() {
            *flag = false;
        }
    }
}

/// Take a cut-line healing unless it removed every triangle.
///
/// The pre-clean does that to a soup whose coincident corners carry different
/// payloads: nothing welds, so all three edges of every triangle read as an
/// isolated nick and the pass deletes the whole mesh. Refusing the destructive
/// result leaves the caller's mesh intact; the filler then walks the original
/// buffers, which for a small soup is the slow-but-allowed case and for a large
/// one is refused before this point.
fn accept_rim_healing(
    mesh: &MeshEditBuffers,
    outcome: Option<RimHealOutcome>,
) -> Option<RimHealOutcome> {
    outcome.filter(|outcome| !outcome.mesh.indices.is_empty() || mesh.indices.is_empty())
}

/// Result when no hole was closed and nothing was healed: every input buffer
/// preserved, with what the run counted.
fn unchanged_fill_result(
    mesh: &MeshEditBuffers,
    counts: FillInputCounts,
    stats: FillLoopStats,
) -> MeshEditResult {
    MeshEditResult {
        mesh: mesh.clone(),
        report: MeshEditReport {
            input_vertices: counts.vertices,
            input_triangles: counts.triangles,
            output_vertices: counts.vertices,
            output_triangles: counts.triangles,
            removed_triangles: 0,
            filled_holes: 0,
            skipped_border_rims: stats.skipped_border,
            skipped_oversize_rims: stats.skipped_oversize,
            skipped_damaged_rims: stats.skipped_degenerate,
            skipped_partial_rims: stats.skipped_partial,
            healed_rims: 0,
            warnings: skip_warnings(stats),
        },
    }
}

/// Public contract: one [`MeshEditWarning::DegenerateGeometry`] per loop
/// skipped as border, oversize or damaged. The report's `skipped_*` counters
/// carry the per-reason breakdown.
fn skip_warnings(stats: FillLoopStats) -> Vec<MeshEditWarning> {
    vec![
        MeshEditWarning::DegenerateGeometry;
        stats.skipped_border + stats.skipped_oversize + stats.skipped_degenerate
    ]
}

/// Assemble the result. `recompute_normals` is off when the filler already
/// left current normals on everything it changed.
fn finalize_fill_result(
    topology: MeshTopology,
    filled: FilledRims,
    compact_vertices: bool,
    recompute_normals: bool,
    counts: FillInputCounts,
) -> Result<MeshEditResult, MeshEditError> {
    let FilledRims {
        surface,
        stats,
        healed_rims,
    } = filled;
    let warnings = skip_warnings(stats);
    let removed_triangles = surface
        .kept
        .as_ref()
        .map_or(0, |kept| kept.iter().filter(|&&kept| !kept).count());
    let (mut vertices, indices) = if compact_vertices {
        let surviving_vertex_indices =
            surviving_vertex_indices(&surface.indices, surface.vertices.len())?;
        let (copied_vertices, remap) =
            copy_surviving_vertices(&surface.vertices, &surviving_vertex_indices)?;
        let remapped_indices = remap_triangle_indices(&surface.indices, &remap)?;
        (copied_vertices, remapped_indices)
    } else {
        (surface.vertices, surface.indices)
    };

    if recompute_normals {
        recompute_all_normals(&mut vertices, &indices)?;
    }
    let output_vertices = vertices.len();
    let output_triangles = indices.len() / 3;

    Ok(MeshEditResult {
        mesh: MeshEditBuffers {
            vertices,
            indices,
            topology,
        },
        report: MeshEditReport {
            input_vertices: counts.vertices,
            input_triangles: counts.triangles,
            output_vertices,
            output_triangles,
            removed_triangles,
            filled_holes: stats.filled,
            skipped_border_rims: stats.skipped_border,
            skipped_oversize_rims: stats.skipped_oversize,
            skipped_damaged_rims: stats.skipped_degenerate,
            skipped_partial_rims: stats.skipped_partial,
            healed_rims,
            warnings,
        },
    })
}

/// The vertices the triangles use, in the order they first use them.
fn surviving_vertex_indices(
    indices: &[u32],
    vertex_count: usize,
) -> Result<Vec<usize>, MeshEditError> {
    let mut seen = vec![false; vertex_count];
    let mut surviving = Vec::new();
    for (triangle_index, &raw_index) in indices.iter().enumerate() {
        let index = vertex_index(raw_index, triangle_index)?;
        // An index past the vertices is reported by the copy that follows.
        if seen.get(index).is_none_or(|seen| !*seen) {
            if let Some(seen) = seen.get_mut(index) {
                *seen = true;
            }
            surviving.push(index);
        }
    }
    Ok(surviving)
}

/// The surrounding mesh plus its precomputed vertex adjacency and triangle
/// incidence, shared by every loop fill in a run (adjacency gives a cap the
/// ring outside its rim; incidence backs the piercing guard).
struct LoopFillContext<'a> {
    mesh: &'a MeshEditBuffers,
    adjacency: &'a [Vec<usize>],
    incidence: &'a VertexTriangleIncidence,
}

/// One rim being capped: its vertices in ring order, their positions, and the
/// surface around it the guard checks a cap against.
struct Rim<'a> {
    vertices: &'a [usize],
    positions: Vec<Vec3>,
    neighbourhood: HashSet<usize>,
    /// What the scan adds at each rim edge, from the vertex of the same index
    /// to the next.
    support: Vec<RimSupport>,
    /// The triangles the surface already has on the rim's vertices.
    taken: TakenTriangles,
}

/// Triangulate and emit one boundary loop. Returns `true` when a complete cap
/// was emitted (the hole is now closed), `false` when the loop was skipped.
///
/// Cap strategy, in order (first success wins; every candidate is a full,
/// watertight cap — a partial cap is never emitted):
///
/// 1. Tiny holes (`< MIN_SHAPED_LOOP`): the plain planar ear-clip fan.
/// 2. Larger holes: a shaped cap, refined to the rim's density and laid on the
///    surface that continues the scan across the seam. Its base is the planar
///    ear clip where the rim's projection does not fold, the minimum-weight
///    membrane otherwise.
/// 3. Fallbacks for a refused/failed cap, each self-intersection guarded, first
///    non-piercing wins: the compact minimum-weight membrane (uncapped in size
///    via hierarchical splitting — good for deep sockets and strongly wrapped
///    rims) then the flat ear-clip lid (good where the membrane grazes a wall).
///    Only for rims simple in 3D, so an hourglass crossing is never baked in.
///
/// No cap lays a triangle onto one the surface already has: the membrane is
/// built around those, and a planar ear clip that uses one is dropped. A
/// 3-edge rim around a lone free-standing triangle is therefore never capped on
/// any path: its only cover is the triangle's reverse twin, a zero-volume
/// doubled sliver. (The pre-cleaning pass removes such triangles up front when
/// enabled.)
fn triangulate_loop(
    context: &LoopFillContext<'_>,
    boundary_loop: &[usize],
    options: MeshEditOptions,
    caps: &mut Caps,
) -> Result<bool, MeshEditError> {
    let mesh = context.mesh;
    let loop_len = boundary_loop.len();
    if loop_len < 3 {
        return Ok(false);
    }
    let rim = Rim {
        vertices: boundary_loop,
        positions: boundary_loop
            .iter()
            .map(|&vertex_index| vertex_position(mesh, vertex_index))
            .collect::<Result<_, _>>()?,
        neighbourhood: rim_neighbourhood(boundary_loop, context.adjacency),
        support: rim_outside_support(mesh, boundary_loop, context.incidence),
        taken: rim_taken_triangles(mesh, boundary_loop, context.incidence),
    };
    let ear = ear_clip_cap(mesh, boundary_loop)?;

    // 1) Tiny holes: the plain planar ear-clip fan, guarded.
    if !ear.is_empty()
        && loop_len < MIN_SHAPED_LOOP
        && !plain_cap_collides(context, &rim, &ear, caps)
    {
        emit_plain_cap(caps, boundary_loop, &ear)?;
        return Ok(true);
    }

    // 2) Larger holes: a shaped cap on the planar ear clip (bounded — a rim
    // past `MAX_PLANAR_LOOP` goes straight to the membrane).
    if !ear.is_empty()
        && (MIN_SHAPED_LOOP..=MAX_PLANAR_LOOP).contains(&loop_len)
        && emit_shaped_cap(
            context,
            &rim,
            (ear.clone(), CapDomain::Plane),
            options,
            caps,
        )?
    {
        return Ok(true);
    }

    // The hierarchical membrane splits rims past the DP leaf, so the largest
    // rims (thousands of edges) still get a full watertight cover.
    let membrane = if rim_is_simple_3d(&rim.positions) {
        min_weight_triangulation_any(&rim.positions, &rim.support, &rim.taken)
    } else {
        None
    };

    // A rim whose projection folds has no planar base, and the raw membrane is
    // a flat lid with no interior vertices to shape: refine the membrane and
    // shape that.
    if loop_len >= MIN_SHAPED_LOOP {
        if let Some(membrane) = membrane.as_ref() {
            if emit_shaped_cap(
                context,
                &rim,
                (membrane.clone(), CapDomain::Space),
                options,
                caps,
            )? {
                return Ok(true);
            }
        }
    }

    // 3) Plain guarded fallbacks, first non-piercing wins; the membrane first,
    // then the flat ear lid.
    let ear_lid = (!ear.is_empty()).then_some(ear);
    for cap in [membrane.as_ref(), ear_lid.as_ref()].into_iter().flatten() {
        if !plain_cap_collides(context, &rim, cap, caps) {
            emit_plain_cap(caps, boundary_loop, cap)?;
            return Ok(true);
        }
    }

    Ok(false)
}

/// Whether a candidate cap runs into the surface: it doubles a triangle the
/// surface has, or pierces itself or the surface around its rim.
fn cap_collides(
    context: &LoopFillContext<'_>,
    rim: &Rim<'_>,
    candidate: &CapCandidate<'_>,
) -> bool {
    cap_doubles_surface(candidate.triangles, &rim.taken)
        || candidate_pierces(
            context.mesh,
            context.incidence,
            &rim.neighbourhood,
            candidate,
        )
}

/// [`cap_collides`] for a rim-only cap (no generated interior).
fn plain_cap_collides(
    context: &LoopFillContext<'_>,
    rim: &Rim<'_>,
    triangles: &[[usize; 3]],
    caps: &Caps,
) -> bool {
    cap_collides(
        context,
        rim,
        &CapCandidate {
            rim: rim.vertices,
            rim_positions: &rim.positions,
            generated: &[],
            generated_base: context.mesh.vertices.len() + caps.vertices.len(),
            triangles,
        },
    )
}

/// Emit a rim-only cap (local indices resolve directly into the rim).
fn emit_plain_cap(
    caps: &mut Caps,
    boundary_loop: &[usize],
    cap_triangles: &[[usize; 3]],
) -> Result<(), MeshEditError> {
    for triangle in cap_triangles {
        for &local in triangle {
            push_cap_index(&mut caps.indices, boundary_loop[local])?;
        }
    }
    Ok(())
}

/// The full vertex payloads of a rim, in ring order.
fn rim_edit_vertices(
    mesh: &MeshEditBuffers,
    boundary_loop: &[usize],
) -> Result<Vec<EditVertex>, MeshEditError> {
    boundary_loop
        .iter()
        .map(|&vertex_index| {
            mesh.vertices
                .get(vertex_index)
                .copied()
                .ok_or_else(|| MeshEditError::MalformedMesh {
                    reason: format!(
                        "boundary loop vertex index {vertex_index} is out of range for \
                         vertex_count {}",
                        mesh.vertices.len()
                    ),
                })
        })
        .collect::<Result<_, _>>()
}

/// Refine a rim-only cap, give it a shape, guard it against running into the
/// surface, and emit it. The shape is the thin plate that leaves the rim the
/// way the scan arrives; where that runs into the surface around the rim, the
/// soap film across the rim; where that does too, nothing, and the loop is
/// left to the plain fallbacks.
///
/// A membrane that needs no interior vertices is left to the plain guarded
/// path as well.
fn emit_shaped_cap(
    context: &LoopFillContext<'_>,
    rim: &Rim<'_>,
    (base, domain): (Vec<[usize; 3]>, CapDomain),
    options: MeshEditOptions,
    caps: &mut Caps,
) -> Result<bool, MeshEditError> {
    let mesh = context.mesh;
    let loop_len = rim.vertices.len();
    let mut cap = refine_cap(
        &rim_edit_vertices(mesh, rim.vertices)?,
        base,
        domain,
        &rim.taken,
        options.attribute_policy.generated_vertex_policy,
    );
    if cap.generated.is_empty() && domain == CapDomain::Space {
        return Ok(false);
    }

    let generated_base = mesh.vertices.len() + caps.vertices.len();
    // Whether the cap doubles a scan triangle is settled by its triangles, so
    // a base that does is dropped before any shape is computed for it.
    if cap_doubles_surface(cap.triangles(), &rim.taken) {
        return Ok(false);
    }
    let accepted = [Continuity::Tangent, Continuity::Position]
        .into_iter()
        .find_map(|continuity| {
            let interior = shape_cap(&rim.positions, &cap, &rim.support, continuity);
            let candidate = CapCandidate {
                rim: rim.vertices,
                rim_positions: &rim.positions,
                generated: &interior,
                generated_base,
                triangles: cap.triangles(),
            };
            let refused =
                candidate_pierces(mesh, context.incidence, &rim.neighbourhood, &candidate);
            (!refused).then_some(interior)
        });
    let Some(interior) = accepted else {
        return Ok(false);
    };
    for (vertex, position) in cap.generated.iter_mut().zip(interior) {
        vertex.position = position.to_array();
    }

    for triangle in cap.triangles() {
        for &local in triangle {
            let global = if local < loop_len {
                rim.vertices[local]
            } else {
                generated_base + (local - loop_len)
            };
            push_cap_index(&mut caps.indices, global)?;
        }
    }
    caps.vertices.extend(cap.generated);
    Ok(true)
}
