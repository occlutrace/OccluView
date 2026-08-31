use super::{
    cache, oversize_input_error, Duration, FileThumbnailPreflightError, Path,
    StreamThumbnailPreflightError, ThumbnailError, MAX_THUMBNAIL_FILE_BYTES,
    MAX_THUMBNAIL_INPUT_BYTES,
};
use crate::fast_thumb::{
    try_read_fast_thumbnail_mesh_for_kind, try_read_fast_thumbnail_mesh_from_file,
};
use crate::thumbnail_format::infer_thumbnail_format;
use glam::Vec3;
use occluview_core::Mesh;
use occluview_formats::dispatch::{dispatch_by_kind_shaded, read_file_shaded};
use occluview_formats::hps::RuntimeHpsKeyProvider;
use occluview_formats::{FormatError, FormatKind};

// Thumbnail fidelity cutoffs: files at or below the format's cutoff are
// parsed with the canonical `occluview-formats` reader (full triangle/point
// data); larger files go through the `fast_thumb` decimated reader instead.
//
// The cutoffs are no longer about speed, and the numbers say so plainly. Once
// the thumbnail stopped reconstructing normals the canonical readers became
// the faster ones at every size measured here, because the decimating reader
// welds onto a grid and that weld now costs more than the parse it replaces:
//
//     20.5 MB STL   full  42 ms   fast 113 ms
//     46.7 MB STL   full 115 ms   fast 289 ms
//    186.7 MB STL   full 415 ms   fast 938 ms
//     31.7 MB OBJ   full 218 ms   fast 345 ms
//
// What the decimating reader still buys is memory. A full read of that 186 MB
// scan is 3.9 million triangles, about 420 MB of vertices resident, and the
// shell hosts several of these at once in a process it does not own. So the
// cutoffs stay, sized to hold that resident cost inside a surrogate rather
// than to save time, and the formats that were held to a tighter budget than
// their cost justified -- OBJ at 24 MB, PLY at 4 MB, where the fast reader
// declines a faced PLY anyway and the file is read twice for nothing -- move
// up to the same line as STL.
/// A thumbnail keeps the normals the file wrote.
///
/// Welding vertices by position and averaging normals across each run is what
/// makes a faceted scan shade smoothly in the viewport, and it is most of the
/// cost of reading one: 135 ms of a 172 ms read for a 326 000 triangle arch,
/// measured here. At 256 pixels that arch puts a tenth of a pixel under each
/// triangle, so the reconstruction cannot be seen -- the two renders differ by
/// a quarter of one step out of 255. The app still reconstructs; only the
/// picture skips it.
const THUMBNAIL_SHADING: occluview_formats::MeshShading = occluview_formats::MeshShading::AsWritten;

const FULL_FIDELITY_STL_THUMBNAIL_FILE_BYTES: u64 = 40 * 1024 * 1024;
const FULL_FIDELITY_OBJ_THUMBNAIL_FILE_BYTES: u64 = 40 * 1024 * 1024;
const FULL_FIDELITY_PLY_THUMBNAIL_FILE_BYTES: u64 = 40 * 1024 * 1024;

fn reject_oversize_input(bytes: &[u8]) -> Result<(), ThumbnailError> {
    if bytes.len() > MAX_THUMBNAIL_INPUT_BYTES {
        return Err(oversize_input_error(bytes.len()));
    }
    Ok(())
}

pub(super) fn load_thumbnail_mesh_from_bytes(
    extension: Option<&str>,
    bytes: &[u8],
) -> Result<Mesh, ThumbnailError> {
    reject_oversize_input(bytes)?;
    let kind = infer_thumbnail_format(extension, bytes)?;
    load_thumbnail_mesh_from_bytes_kind(kind, bytes)
}

pub(super) fn load_thumbnail_mesh_from_bytes_kind(
    kind: FormatKind,
    bytes: &[u8],
) -> Result<Mesh, ThumbnailError> {
    select_thumbnail_mesh(
        kind,
        prefers_full_fidelity_thumbnail_kind(kind, bytes.len() as u64),
        || dispatch_by_kind_shaded(kind, bytes, &RuntimeHpsKeyProvider, THUMBNAIL_SHADING),
        || try_read_fast_thumbnail_mesh_for_kind(kind, bytes),
    )
}

pub(super) fn load_thumbnail_mesh_from_file(
    path: &Path,
    metadata: cache::ThumbnailFileMetadata,
) -> Result<Mesh, ThumbnailError> {
    if metadata.byte_len > MAX_THUMBNAIL_FILE_BYTES as u64 {
        return Err(cache::oversize_file_error(
            usize::try_from(metadata.byte_len).unwrap_or(usize::MAX),
        ));
    }
    // The file loaders infer the format from the real bytes themselves; `kind`
    // is only the label for the corrupt-error path, so derive it cheaply from
    // the extension rather than re-probing.
    select_thumbnail_mesh(
        thumbnail_kind_from_extension(path),
        prefers_full_fidelity_thumbnail_parse(path, &metadata),
        || read_file_shaded(path, &RuntimeHpsKeyProvider, THUMBNAIL_SHADING),
        || try_read_fast_thumbnail_mesh_from_file(path),
    )
}

fn thumbnail_kind_from_extension(path: &Path) -> FormatKind {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("obj") => FormatKind::Obj,
        Some("ply") => FormatKind::Ply,
        _ => FormatKind::Stl,
    }
}

/// Pick the mesh a thumbnail should render, preferring whichever source is
/// appropriate for the file size but ALWAYS returning renderable geometry.
///
/// The fast surrogate can decline (returns `None`) or emit geometry that would
/// render as a fully transparent tile (an all-degenerate / non-finite file
/// clusters to zero triangles); the full reader can likewise return a mesh with
/// no renderable area. This routes around a blank result from either source,
/// and only when *both* fail does it surface a corrupt-file error so the COM
/// layer paints a placeholder — the public entry point never returns a
/// fully-transparent bitmap.
fn select_thumbnail_mesh(
    kind: FormatKind,
    prefer_full: bool,
    full: impl FnOnce() -> Result<Mesh, FormatError>,
    fast: impl FnOnce() -> Option<Mesh>,
) -> Result<Mesh, ThumbnailError> {
    if prefer_full {
        // Inside the fidelity budget: trust the canonical reader, but fall back
        // to the fast surrogate if it fails OR returns nothing renderable.
        match full() {
            Ok(mesh) if thumbnail_mesh_is_renderable(&mesh) => return Ok(mesh),
            full_result => {
                if let Some(mesh) = fast() {
                    if thumbnail_mesh_is_renderable(&mesh) {
                        return Ok(mesh);
                    }
                }
                return match full_result {
                    Err(error) => Err(error.into()),
                    Ok(_) => Err(non_renderable_thumbnail_error(kind).into()),
                };
            }
        }
    }

    // Above the fidelity budget: try the fast surrogate first, then the full
    // reader, keeping the first source that yields renderable geometry.
    if let Some(mesh) = fast() {
        if thumbnail_mesh_is_renderable(&mesh) {
            return Ok(mesh);
        }
    }
    let mesh = full()?;
    if thumbnail_mesh_is_renderable(&mesh) {
        Ok(mesh)
    } else {
        Err(non_renderable_thumbnail_error(kind).into())
    }
}

/// True if `mesh` will project to visible pixels in the thumbnail.
///
/// Guards against the two ways a loaded mesh renders to a fully transparent
/// tile: no vertices / a non-finite (NaN or Inf) bounding box (dead camera
/// framing), and a triangle mesh with no drawable area (every triangle
/// degenerate — a point or a line — so nothing rasterizes). A point cloud only
/// needs some spatial extent; the renderer splats its vertices.
fn thumbnail_mesh_is_renderable(mesh: &Mesh) -> bool {
    if mesh.vertices().is_empty() {
        return false;
    }
    let bbox = mesh.bbox_cached();
    let extent = bbox.size();
    if bbox.is_empty() || !extent.is_finite() {
        return false;
    }
    if mesh.is_point_cloud() {
        return extent.max_element() > 0.0;
    }
    // A triangle mesh needs at least one triangle with real (finite, non-zero)
    // area. A 2D-spanning bbox is not enough: an all-zero-area soup (every
    // triangle a coincident point, scattered across a plane) has a 2D box yet
    // rasterizes to nothing. Scan for the first drawable triangle — O(1) for a
    // healthy surface, worst case O(triangles) for a wholly degenerate file.
    mesh_has_drawable_triangle(mesh)
}

fn mesh_has_drawable_triangle(mesh: &Mesh) -> bool {
    let vertices = mesh.vertices();
    for triangle in mesh.indices().as_chunks::<3>().0 {
        let a = Vec3::from_array(vertices[triangle[0] as usize].position);
        let b = Vec3::from_array(vertices[triangle[1] as usize].position);
        let c = Vec3::from_array(vertices[triangle[2] as usize].position);
        if a.is_finite()
            && b.is_finite()
            && c.is_finite()
            && (b - a).cross(c - a).length_squared() > 0.0
        {
            return true;
        }
    }
    false
}

/// A corrupt-file error for a recognized format that produced no renderable
/// geometry. Maps to the corrupt-badge placeholder (see
/// `render_thumb::placeholder_kind_for_error`): the format name is real (never
/// the `"thumbnail …"` sentinel that marks a quiet policy decision).
fn non_renderable_thumbnail_error(kind: FormatKind) -> FormatError {
    let format = match kind {
        FormatKind::Stl => "STL",
        FormatKind::Obj => "OBJ",
        FormatKind::Ply => "PLY",
        _ => "mesh",
    };
    FormatError::Malformed {
        format,
        offset: 0,
        reason: "no renderable geometry (all triangles degenerate or non-finite)".to_string(),
    }
}

fn path_has_supported_thumbnail_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.trim_start_matches('.').to_ascii_lowercase())
        .is_some_and(|extension| {
            crate::SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension == *supported)
        })
}

pub(super) fn prepare_file_thumbnail_render(
    path: &Path,
    timeout: Duration,
) -> Result<cache::FileThumbnailRenderPlan, FileThumbnailPreflightError> {
    if !path_has_supported_thumbnail_extension(path) {
        return Err(FileThumbnailPreflightError::UnsupportedExtension);
    }

    let (metadata, is_regular_file) = cache::thumbnail_file_metadata_checked(path)
        .map_err(FileThumbnailPreflightError::Metadata)?;
    if !is_regular_file {
        return Err(FileThumbnailPreflightError::NotAFile);
    }
    if metadata.byte_len > MAX_THUMBNAIL_FILE_BYTES as u64 {
        return Err(FileThumbnailPreflightError::Oversize {
            byte_len: usize::try_from(metadata.byte_len).unwrap_or(usize::MAX),
        });
    }

    Ok(cache::FileThumbnailRenderPlan {
        metadata,
        cache_key: cache::ThumbnailFileCacheKey::new(path, &metadata),
        // The shell gets one wall-clock budget for queueing, decoding, and
        // rendering. Adding setup and render budgets made a mixed folder wait
        // up to fourteen seconds per request before Explorer gave up.
        wait_timeout: timeout,
    })
}

pub(super) fn prepare_stream_thumbnail_render(
    extension: Option<&str>,
    bytes: &[u8],
    timeout: Duration,
) -> Result<cache::StreamThumbnailRenderPlan, StreamThumbnailPreflightError> {
    if bytes.len() > MAX_THUMBNAIL_INPUT_BYTES {
        return Err(StreamThumbnailPreflightError::Oversize {
            byte_len: bytes.len(),
        });
    }
    let kind =
        infer_thumbnail_format(extension, bytes).map_err(StreamThumbnailPreflightError::Format)?;

    Ok(cache::StreamThumbnailRenderPlan {
        kind,
        cache_key: cache::ThumbnailStreamCacheKey::new(kind, bytes),
        wait_timeout: timeout,
    })
}

fn prefers_full_fidelity_thumbnail_parse(
    path: &Path,
    metadata: &cache::ThumbnailFileMetadata,
) -> bool {
    let Some(extension) = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
    else {
        return false;
    };

    let byte_limit = match extension.as_str() {
        "stl" => FULL_FIDELITY_STL_THUMBNAIL_FILE_BYTES,
        "obj" => FULL_FIDELITY_OBJ_THUMBNAIL_FILE_BYTES,
        "ply" => FULL_FIDELITY_PLY_THUMBNAIL_FILE_BYTES,
        _ => return false,
    };

    metadata.byte_len <= byte_limit
}

fn prefers_full_fidelity_thumbnail_kind(kind: FormatKind, byte_len: u64) -> bool {
    let byte_limit = match kind {
        FormatKind::Stl => FULL_FIDELITY_STL_THUMBNAIL_FILE_BYTES,
        FormatKind::Obj => FULL_FIDELITY_OBJ_THUMBNAIL_FILE_BYTES,
        FormatKind::Ply => FULL_FIDELITY_PLY_THUMBNAIL_FILE_BYTES,
        _ => return true,
    };

    byte_len <= byte_limit
}
