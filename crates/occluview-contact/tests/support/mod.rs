//! Meshes the contact tests are built from.
//!
//! Only what the tests need: flat plates, compositions of them, and the
//! plumbing that turns a pair into a field. Anything richer would hide the
//! geometry a test is actually asserting about, and these tests are about
//! geometry and signs.
#![allow(dead_code)]

use occluview_align::{CancelFlag, Soup};
use occluview_contact::{compute_contact_field, ContactField, ContactSettings};

/// A mesh under construction: positions as xyz triples, triangle indices.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    /// Interleaved xyz positions, in millimetres.
    pub positions: Vec<f32>,
    /// Triangle indices into `positions`.
    pub indices: Vec<u32>,
}

impl Mesh {
    /// A mesh from points and triangles.
    pub fn new(points: &[[f64; 3]], indices: &[u32]) -> Self {
        let mut positions = Vec::with_capacity(points.len() * 3);
        for point in points {
            positions.push(to_f32(point[0]));
            positions.push(to_f32(point[1]));
            positions.push(to_f32(point[2]));
        }
        Self {
            positions,
            indices: indices.to_vec(),
        }
    }

    /// The borrow a measurement takes.
    pub fn soup(&self) -> Soup<'_> {
        Soup {
            positions: &self.positions,
            indices: &self.indices,
            mask: None,
        }
    }

    /// Move every vertex by `dz` along z, in millimetres.
    pub fn shifted_z(mut self, dz: f64) -> Self {
        for vertex in self.positions.as_chunks_mut::<3>().0 {
            vertex[2] = to_f32(f64::from(vertex[2]) + dz);
        }
        self
    }

    /// Number of whole vertices.
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// One vertex, promoted to double precision.
    pub fn vertex(&self, index: usize) -> [f64; 3] {
        let offset = index * 3;
        [
            f64::from(self.positions[offset]),
            f64::from(self.positions[offset + 1]),
            f64::from(self.positions[offset + 2]),
        ]
    }

    /// Re-aim every triangle at -z, which is what "flip the normals" does to a
    /// scan and what the sign convention is measured against.
    pub fn reversed(mut self) -> Self {
        for triangle in self.indices.as_chunks_mut::<3>().0 {
            triangle.swap(1, 2);
        }
        self
    }
}

/// A flat plate in the xy plane, `cells_x` by `cells_y` quads on a half-extent.
///
/// Wound so the triangle normal points at +z; `flip` reverses it.
// Two half-extents, two cell counts, a height and a winding: six numbers that
// are each a different axis of the same rectangle, and bundling them would name
// the bundle rather than the geometry the tests assert about.
#[allow(clippy::too_many_arguments)]
pub fn plate(
    half_x: f64,
    half_y: f64,
    cells_x: usize,
    cells_y: usize,
    z_mm: f64,
    flip: bool,
) -> Mesh {
    let mut points = Vec::with_capacity((cells_x + 1) * (cells_y + 1));
    for row in 0..=cells_y {
        let y = -half_y + 2.0 * half_y * index_fraction(row, cells_y);
        for column in 0..=cells_x {
            let x = -half_x + 2.0 * half_x * index_fraction(column, cells_x);
            points.push([x, y, z_mm]);
        }
    }
    let stride = u32::try_from(cells_x + 1).unwrap_or(u32::MAX);
    let mut indices = Vec::with_capacity(cells_x * cells_y * 6);
    for row in 0..cells_y {
        for column in 0..cells_x {
            let row = u32::try_from(row).unwrap_or(u32::MAX);
            let column = u32::try_from(column).unwrap_or(u32::MAX);
            let corner = row * stride + column;
            let next_row = corner + stride;
            if flip {
                indices.extend_from_slice(&[corner, next_row, corner + 1]);
                indices.extend_from_slice(&[corner + 1, next_row, next_row + 1]);
            } else {
                indices.extend_from_slice(&[corner, corner + 1, next_row]);
                indices.extend_from_slice(&[corner + 1, next_row + 1, next_row]);
            }
        }
    }
    Mesh::new(&points, &indices)
}

/// One quad: two triangles spanning `[x0, x1] × [y0, y1]` at height `z_mm`.
// Four corners and a height is four numbers and a height; bundling them would
// name the bundle rather than the geometry the tests are about.
#[allow(clippy::too_many_arguments)]
pub fn quad(x0: f64, x1: f64, y0: f64, y1: f64, z_mm: f64, flip: bool) -> Mesh {
    let points = [
        [x0, y0, z_mm],
        [x1, y0, z_mm],
        [x0, y1, z_mm],
        [x1, y1, z_mm],
    ];
    let indices: [u32; 6] = if flip {
        [0, 2, 1, 1, 2, 3]
    } else {
        [0, 1, 2, 1, 3, 2]
    };
    Mesh::new(&points, &indices)
}

/// Every input mesh in one, indices rebased.
pub fn combine(meshes: &[Mesh]) -> Mesh {
    let mut combined = Mesh::default();
    for mesh in meshes {
        let offset = u32::try_from(combined.vertex_count()).unwrap_or(u32::MAX);
        combined.positions.extend_from_slice(&mesh.positions);
        combined
            .indices
            .extend(mesh.indices.iter().map(|index| index + offset));
    }
    combined
}

/// Measure `subject` against `antagonist` with the default settings.
pub fn field(subject: &Mesh, antagonist: &Mesh) -> ContactField {
    field_with(subject, antagonist, ContactSettings::default())
}

/// Measure with explicit settings.
pub fn field_with(subject: &Mesh, antagonist: &Mesh, settings: ContactSettings) -> ContactField {
    compute_contact_field(
        subject.soup(),
        antagonist.soup(),
        settings,
        &CancelFlag::new(),
    )
}

/// Measure with the flag already pulled, which is how a closing panel cancels.
pub fn field_cancelled(subject: &Mesh, antagonist: &Mesh) -> ContactField {
    let cancel = CancelFlag::new();
    cancel.cancel();
    compute_contact_field(
        subject.soup(),
        antagonist.soup(),
        ContactSettings::default(),
        &cancel,
    )
}

/// How far along a run of `count` samples index `index` sits, in 0..=1.
fn index_fraction(index: usize, count: usize) -> f64 {
    if count == 0 {
        0.0
    } else {
        to_f64(index) / to_f64(count)
    }
}

/// Millimetres and field values are stored as `f32` on the wire.
#[allow(clippy::cast_possible_truncation)]
fn to_f32(value: f64) -> f32 {
    value as f32
}

/// Vertex counts and grid indices as scalars.
#[allow(clippy::cast_precision_loss)]
fn to_f64(value: usize) -> f64 {
    value as f64
}
