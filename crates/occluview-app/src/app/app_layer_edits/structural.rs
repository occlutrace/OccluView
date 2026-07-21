//! Structural executors: Cut selection to a new layer and Separate connected
//! components, preserving the source layer's presentation state.
//!
//! # Separate contract ("Separate marked area", exocad-style)
//!
//! Separate splits the MARKED material off a layer. Connectivity is computed on
//! the mesh's TRUE shared topology — STL and other soup formats store three
//! independent vertices per triangle, so the kernel welds those bit-identical
//! corners back before analysis (`selected_connected_components_in_mesh`).
//! Without that weld a soup model read as one component per selected triangle and
//! Separate exploded into hundreds of thousands of one-triangle "parts" (the
//! owner's 317k-confetti report); computing components on welded topology makes
//! that failure mode structurally impossible.
//!
//! Given a valid selection, the results are:
//! - **One connected marked patch** (e.g. an open scan, one lasso patch) →
//!   the source keeps the unselected remainder and ONE new layer holds the patch.
//! - **A through-mode lasso that marks two walls of a closed hollow model** (the
//!   ray pierces both the outer and the inner wall) → the two marked caps are
//!   geometrically disjoint, so each becomes its OWN new layer. Remainder + 2
//!   parts.
//! - **Several disjoint marked islands** → each island becomes its own layer, in
//!   deterministic order (ascending by lowest source-triangle index), with each
//!   part stepping the tint palette so the split is visible.
//!
//! The REMAINDER (everything unselected) always stays a SINGLE layer, even when
//! removing the marked islands disconnects it into multiple shells — matching
//! exocad's "the rest stays the base". A whole-mesh selection is refused upstream
//! (it would leave a dead empty source), and a selection that fragments past
//! [`MAX_SEPARATE_COMPONENTS`] is refused with an honest count instead of a layer
//! storm.

use super::super::{EditModeController, LayerContextAction, LayerContextApply, Scene};
use super::selection_ops::selected_face_edit_result;
use super::SelectedFaceEditContext;
use occluview_core::{
    selected_connected_components_in_mesh, CoreError, FaceSelection, Mesh, SceneMesh, Vertex,
};

/// Upper bound on the layers a single Separate ("Divide") may spawn. A dental
/// user never wants hundreds of new layers; a selection that fragments past
/// this is a noisy lasso, not a real divide, so we refuse with an honest status
/// instead of grinding the scene. Exposed for the status caller and tests.
pub(super) const MAX_SEPARATE_COMPONENTS: usize = 256;

pub(super) fn apply_cut_selection_to_new_layer(
    scene: &mut Scene,
    context: SelectedFaceEditContext,
    selection: &FaceSelection,
    edit_mode: &mut EditModeController,
) -> Result<LayerContextApply, CoreError> {
    let Some(source_entry) = scene.meshes().get(context.index).cloned() else {
        let _ = edit_mode.finish_layer_edit_noop(context.token);
        return Ok(LayerContextApply::default());
    };
    if source_entry.id() != context.layer_id {
        let _ = edit_mode.finish_layer_edit_noop(context.token);
        return Ok(LayerContextApply::default());
    }

    let (remainder, extracted) = match cut_selection_meshes(&source_entry, selection) {
        Ok(meshes) => meshes,
        Err(error) => {
            let _ = edit_mode.finish_layer_edit_error(context.token, error.to_string());
            return Err(error);
        }
    };

    let Some(entry) = scene.meshes_mut().get_mut(context.index) else {
        let _ = edit_mode.finish_layer_edit_noop(context.token);
        return Ok(LayerContextApply::default());
    };
    if entry.id() != context.layer_id {
        let _ = edit_mode.finish_layer_edit_noop(context.token);
        return Ok(LayerContextApply::default());
    }

    entry.mesh = remainder;
    scene.insert(
        context.index + 1,
        clone_layer_with_mesh(&source_entry, extracted)
            .with_tint(crate::layer_actions::next_layer_tint(source_entry.tint)),
    );
    // Stamp the structural snapshot with this post-op id-set so a later undo
    // can refuse honestly if a layer is appended/removed before it runs.
    let _ = edit_mode.finish_scene_edit_success(context.token, scene);
    Ok(structural_scene_apply())
}

pub(super) fn apply_separate_selected_components(
    scene: &mut Scene,
    context: SelectedFaceEditContext,
    selection: &FaceSelection,
    edit_mode: &mut EditModeController,
) -> Result<LayerContextApply, CoreError> {
    let Some(source_entry) = scene.meshes().get(context.index).cloned() else {
        let _ = edit_mode.finish_layer_edit_noop(context.token);
        return Ok(LayerContextApply::default());
    };
    if source_entry.id() != context.layer_id {
        let _ = edit_mode.finish_layer_edit_noop(context.token);
        return Ok(LayerContextApply::default());
    }

    let components = match selected_connected_components_in_mesh(&source_entry.mesh, selection) {
        Ok(components) => components,
        Err(error) => {
            let _ = edit_mode.finish_layer_edit_error(context.token, error.to_string());
            return Err(error);
        }
    };
    if components.is_empty() {
        let _ = edit_mode.finish_layer_edit_noop(context.token);
        return Ok(LayerContextApply::default());
    }
    // Sanity cap: a selection that explodes into hundreds of tiny islands is a
    // noisy lasso, not a divide. Refuse (no op); the status caller reports the
    // count. Enforced here too so the direct executor path is safe on its own.
    if components.len() > MAX_SEPARATE_COMPONENTS {
        let _ = edit_mode.finish_layer_edit_noop(context.token);
        return Ok(LayerContextApply::default());
    }

    // Build the remainder and every component mesh in ONE sweep over the source
    // triangles, sharing a single vertex-remap pass. The old path cropped the
    // full mesh once per component — O(components × mesh) time, which is what
    // hung "Divide" on a fragmented half-model selection.
    let split = match split_selection_into_meshes(&source_entry.mesh, &components) {
        Ok(split) => split,
        Err(error) => {
            let _ = edit_mode.finish_layer_edit_error(context.token, error.to_string());
            return Err(error);
        }
    };

    let Some(entry) = scene.meshes_mut().get_mut(context.index) else {
        let _ = edit_mode.finish_layer_edit_noop(context.token);
        return Ok(LayerContextApply::default());
    };
    if entry.id() != context.layer_id {
        let _ = edit_mode.finish_layer_edit_noop(context.token);
        return Ok(LayerContextApply::default());
    }

    entry.mesh = split.remainder;
    // The parts are geometrically coincident with where they sat in the
    // source, so with the source tint they would be invisible as a split
    // (exocad shows a divide by recoloring the pieces). Walk the palette so
    // every part reads as its own layer at a glance.
    let mut part_tint = source_entry.tint;
    for (offset, mesh) in split.components.into_iter().enumerate() {
        part_tint = crate::layer_actions::next_layer_tint(part_tint);
        scene.insert(
            context.index + 1 + offset,
            clone_layer_with_mesh(&source_entry, mesh).with_tint(part_tint),
        );
    }
    // Stamp the structural snapshot with this post-op id-set so a later undo
    // can refuse honestly if a layer is appended/removed before it runs.
    let _ = edit_mode.finish_scene_edit_success(context.token, scene);
    Ok(structural_scene_apply())
}

/// The output of a single-pass Separate: the source layer's remainder plus one
/// extracted mesh per connected component of the selection, in component order.
pub(super) struct SeparatedMeshes {
    pub(super) remainder: Mesh,
    pub(super) components: Vec<Mesh>,
}

/// Partition `source` into the unselected remainder and one mesh per component
/// in a single sweep over the triangles. Every triangle is routed to a bucket:
/// its component id, or the shared remainder bucket for any triangle in no
/// component (i.e. unselected — components only ever contain selected faces). A
/// reused per-vertex stamp assigns compact local indices — O(vertices +
/// triangles) total, not O(components × mesh). Vertex attributes (color/UV/
/// normal) are copied verbatim and the source name/texture are preserved,
/// matching a per-component crop without re-scanning the whole mesh each time.
pub(super) fn split_selection_into_meshes(
    source: &Mesh,
    components: &[Vec<usize>],
) -> Result<SeparatedMeshes, CoreError> {
    let source_vertices = source.vertices();
    let source_indices = source.indices();
    let triangle_count = source.triangle_count();

    let remainder_bucket = components.len();
    let bucket_count = components.len() + 1;
    let mut bucket_of_triangle = vec![remainder_bucket; triangle_count];
    for (component_id, component) in components.iter().enumerate() {
        for &triangle in component {
            if let Some(slot) = bucket_of_triangle.get_mut(triangle) {
                *slot = component_id;
            }
        }
    }

    // stamp[v] = bucket that last copied vertex v; local[v] = its index there.
    let mut stamp = vec![usize::MAX; source_vertices.len()];
    let mut local = vec![0u32; source_vertices.len()];
    let mut out_vertices: Vec<Vec<Vertex>> = vec![Vec::new(); bucket_count];
    let mut out_indices: Vec<Vec<u32>> = vec![Vec::new(); bucket_count];

    for (triangle, &bucket) in bucket_of_triangle.iter().enumerate() {
        let corners = source_indices
            .get(triangle * 3..triangle * 3 + 3)
            .ok_or_else(|| CoreError::Geometry(format!("triangle {triangle} is out of range")))?;
        for &raw in corners {
            let old = raw as usize;
            if stamp[old] != bucket {
                let local_index = u32::try_from(out_vertices[bucket].len()).map_err(|_| {
                    CoreError::Geometry("component vertex count exceeds u32".to_string())
                })?;
                let vertex = *source_vertices.get(old).ok_or_else(|| {
                    CoreError::Geometry(format!("vertex index {old} is out of range"))
                })?;
                stamp[old] = bucket;
                local[old] = local_index;
                out_vertices[bucket].push(vertex);
            }
            out_indices[bucket].push(local[old]);
        }
    }

    let mut built = Vec::with_capacity(bucket_count);
    for (vertices, indices) in out_vertices.into_iter().zip(out_indices) {
        built.push(build_mesh_like(source, vertices, indices)?);
    }
    // The remainder bucket is the last one; the rest stay in component order.
    let remainder = built
        .pop()
        .ok_or_else(|| CoreError::Geometry("missing remainder bucket".to_string()))?;
    Ok(SeparatedMeshes {
        remainder,
        components: built,
    })
}

/// Rebuild a core mesh from split parts, preserving the source name and
/// (when the parts carry UVs) its decoded texture — the metadata policy the
/// crop adapter applies, without the full-mesh round trip.
fn build_mesh_like(
    source: &Mesh,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
) -> Result<Mesh, CoreError> {
    let mut mesh = Mesh::new(source.name().map(str::to_owned), vertices, indices)?;
    if let Some(texture) = source.texture() {
        if mesh.has_uvs() {
            mesh.set_texture(texture.clone());
        }
    }
    Ok(mesh)
}

pub(super) fn structural_scene_apply() -> LayerContextApply {
    LayerContextApply {
        scene_changed: true,
        structural_scene_change: true,
        ..LayerContextApply::default()
    }
}

pub(super) fn clone_layer_with_mesh(source: &SceneMesh, mesh: Mesh) -> SceneMesh {
    let mut cloned = SceneMesh::new(mesh)
        .with_transform(source.transform)
        .with_tint(source.tint)
        .with_opacity(source.opacity)
        .with_wireframe(source.wireframe)
        .with_show_vertex_colors(source.show_vertex_colors)
        .with_show_texture(source.show_texture);
    cloned.visible = source.visible;
    cloned.show_orientation = source.show_orientation;
    cloned
}

/// Build the two pure mesh results needed by Cut without changing a scene or
/// edit-session state. The single-layer executor and the multi-layer batch
/// coordinator both use the same selected-face adapters here.
pub(super) fn cut_selection_meshes(
    source: &SceneMesh,
    selection: &FaceSelection,
) -> Result<(Mesh, Mesh), CoreError> {
    let remainder = selected_face_edit_result(
        &source.mesh,
        selection,
        LayerContextAction::DeleteSelectedFaces,
    )?;
    let extracted = selected_face_edit_result(
        &source.mesh,
        selection,
        LayerContextAction::CropToSelectedFaces,
    )?;
    Ok((remainder.mesh, extracted.mesh))
}
