//! In-slice measuring ruler for the Section panel.
//!
//! The slice window is an orthographic projection along the cut-plane normal, so
//! a panel pixel maps to an exact section-plane point in millimeters. This
//! module owns:
//!   * [`SliceCam`] — the slice camera facts captured at render time,
//!   * [`SlicePlaneMap`] — the pure panel-pixel <-> world-mm mapping (`f64`),
//!   * [`CutRuler`] — a two-point measurement whose markers are anchored in
//!     **world** (section-plane) coordinates so they stay put while the disc
//!     scales/zooms and re-project each frame, and clear when the plane changes.
//!
//! The mapping matches the render exactly through the shared [`SliceBasis`], so
//! a marker sits on the geometry it was clicked on even while the main camera
//! rotates.

use eframe::egui;
use glam::Vec3;

use crate::{measure_draw, probe_section::SliceProbe, ui_theme};

/// Two planes are "the same section" when their normals and offsets agree within
/// these tolerances; a larger change discards the ruler (its section is gone).
const PLANE_NORMAL_EPS: f32 = 1.0e-3;
const PLANE_DISTANCE_EPS: f32 = 1.0e-3;

/// The slice camera facts captured when the offscreen slice was rendered —
/// enough to map panel pixels to section-plane millimeters exactly as drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SliceCam {
    /// Disc center (world) the slice is framed on — the panel-image center.
    pub(crate) focus: Vec3,
    /// Section-plane normal (world, unit). The view looks along it.
    pub(crate) normal: Vec3,
    /// Orthographic half-extent (world mm) mapped to half the panel image.
    pub(crate) half_extent: f32,
}

impl SliceCam {
    /// The `(normal, distance)` signature identifying this slice's plane.
    fn plane_signature(self) -> (Vec3, f32) {
        let normal = self.normal.normalize_or(Vec3::Y);
        (normal, normal.dot(self.focus))
    }
}

/// Orthonormal screen axes for a section image. The basis is supplied by the
/// main viewport so vector contours, raster slices, ruler points, pan and zoom
/// all use one coordinate system.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SliceBasis {
    pub(crate) right: Vec3,
    pub(crate) up: Vec3,
}

impl Default for SliceBasis {
    fn default() -> Self {
        Self::from_normal(Vec3::Y)
    }
}

impl SliceBasis {
    pub(crate) fn from_normal(normal: Vec3) -> Self {
        let (right, up) = occluview_render::slice_view_basis(normal);
        Self { right, up }
    }

    /// Build a basis from projected main-camera axes, with a normal-based
    /// fallback for the singular case where both hints are parallel to the
    /// section plane normal.
    pub(crate) fn from_view_axes(normal: Vec3, right_hint: Vec3, up_hint: Vec3) -> Self {
        let normal = normal.normalize_or(Vec3::Y);
        let projected_right = right_hint - normal * right_hint.dot(normal);
        if projected_right.is_finite() && projected_right.length_squared() > 1.0e-8 {
            let right = projected_right.normalize();
            return Self {
                right,
                up: right.cross(normal).normalize_or(Vec3::Y),
            };
        }

        let projected_up = up_hint - normal * up_hint.dot(normal);
        if projected_up.is_finite() && projected_up.length_squared() > 1.0e-8 {
            let up = projected_up.normalize();
            return Self {
                right: normal.cross(up).normalize_or(Vec3::X),
                up,
            };
        }

        Self::from_normal(normal)
    }
}

/// Pure mapping between the panel image rect and the section plane, in `f64`.
///
/// The panel image spans `[-half_extent, half_extent]` world mm on each axis of
/// the plane's `(right, up)` basis, centered on `focus`.
pub(crate) struct SlicePlaneMap {
    focus: Vec3,
    right: Vec3,
    up: Vec3,
    half_extent: f64,
    image_rect: egui::Rect,
}

impl SlicePlaneMap {
    /// Build the mapping for a slice camera drawn into `image_rect`.
    #[cfg(test)]
    pub(crate) fn new(cam: SliceCam, image_rect: egui::Rect) -> Self {
        Self::new_with_basis(cam, image_rect, SliceBasis::from_normal(cam.normal))
    }

    /// Build the mapping with the exact in-plane axes used by the rendered
    /// section image.
    pub(crate) fn new_with_basis(cam: SliceCam, image_rect: egui::Rect, basis: SliceBasis) -> Self {
        Self {
            focus: cam.focus,
            right: basis.right,
            up: basis.up,
            half_extent: f64::from(cam.half_extent.max(0.1)),
            image_rect,
        }
    }

    /// Panel pixel (absolute egui coords) -> world point on the section plane.
    /// The mm mapping runs in `f64`; the final world point narrows to `f32`
    /// (world geometry is `f32` — narrowing a small mm coordinate is exact here).
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn panel_to_world(&self, pos: egui::Pos2) -> Vec3 {
        let w = f64::from(self.image_rect.width().max(1.0));
        let h = f64::from(self.image_rect.height().max(1.0));
        let ndc_x = (f64::from(pos.x - self.image_rect.left()) / w) * 2.0 - 1.0;
        let ndc_y = 1.0 - (f64::from(pos.y - self.image_rect.top()) / h) * 2.0;
        let a = ndc_x * self.half_extent;
        let b = ndc_y * self.half_extent;
        self.focus + self.right * (a as f32) + self.up * (b as f32)
    }

    /// The in-plane world offset to add to the framing focus so the section
    /// point currently under `pointer` follows a drag of `drag_delta` panel
    /// pixels — i.e. the grab-and-move pan of a normal 3D window. Because both
    /// mapped points lie on the section plane, their difference is in-plane, so
    /// the plane offset (`normal · focus`) never changes and the mm mapping stays
    /// exact under any pan.
    pub(crate) fn pan_delta_for_drag(&self, pointer: egui::Pos2, drag_delta: egui::Vec2) -> Vec3 {
        self.panel_to_world(pointer - drag_delta) - self.panel_to_world(pointer)
    }

    /// World point on the section plane -> panel pixel (absolute egui coords).
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn world_to_panel(&self, world: Vec3) -> egui::Pos2 {
        let d = world - self.focus;
        let ndc_x = f64::from(self.right.dot(d)) / self.half_extent;
        let ndc_y = f64::from(self.up.dot(d)) / self.half_extent;
        let w = f64::from(self.image_rect.width().max(1.0));
        let h = f64::from(self.image_rect.height().max(1.0));
        let px = (ndc_x * 0.5 + 0.5) * w;
        let py = (0.5 - ndc_y * 0.5) * h;
        egui::pos2(
            self.image_rect.left() + px as f32,
            self.image_rect.top() + py as f32,
        )
    }

    /// Zoom-to-cursor: given the current slice framing (`focus`, `half_extent`)
    /// and a target `half_ratio` (`new_half / old_half`, `< 1` magnifies),
    /// return the `(new_focus, new_half)` that keeps the section point currently
    /// under `cursor` fixed at the same panel pixel. Pure and testable — the
    /// classic zoom-to-point math for an orthographic slice camera.
    ///
    /// The new focus stays on the section plane: it moves only along the plane's
    /// `(right, up)` basis, so `normal · focus` (the plane offset) is unchanged.
    #[cfg(test)]
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
    pub(crate) fn zoom_focus_at_cursor(
        focus: Vec3,
        half_extent: f32,
        normal: Vec3,
        image_rect: egui::Rect,
        cursor: egui::Pos2,
        half_ratio: f32,
    ) -> (Vec3, f32) {
        Self::zoom_focus_at_cursor_with_basis(
            focus,
            half_extent,
            image_rect,
            cursor,
            half_ratio,
            SliceBasis::from_normal(normal),
        )
    }

    /// Oriented zoom-to-cursor variant used by the live Section panel when its
    /// basis follows the primary camera.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
    pub(crate) fn zoom_focus_at_cursor_with_basis(
        focus: Vec3,
        half_extent: f32,
        image_rect: egui::Rect,
        cursor: egui::Pos2,
        half_ratio: f32,
        basis: SliceBasis,
    ) -> (Vec3, f32) {
        let right = basis.right;
        let up = basis.up;
        let half = f64::from(half_extent.max(0.1));
        let w = f64::from(image_rect.width().max(1.0));
        let h = f64::from(image_rect.height().max(1.0));
        let ndc_x = (f64::from(cursor.x - image_rect.left()) / w) * 2.0 - 1.0;
        let ndc_y = 1.0 - (f64::from(cursor.y - image_rect.top()) / h) * 2.0;
        let dir = right * (ndc_x as f32) + up * (ndc_y as f32);
        let world_cursor = focus + dir * (half as f32);
        let new_half = (half * f64::from(half_ratio)) as f32;
        let new_focus = world_cursor - dir * new_half;
        (new_focus, new_half)
    }

    /// The straight-line distance between two world section-plane points, in mm
    /// (`f64` throughout the mm mapping).
    pub(crate) fn distance_mm(a: Vec3, b: Vec3) -> f64 {
        let dx = f64::from(a.x) - f64::from(b.x);
        let dy = f64::from(a.y) - f64::from(b.y);
        let dz = f64::from(a.z) - f64::from(b.z);
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

/// One in-slice wall-thickness measurement (the probe-linked seed of feature D
/// and the panel's one-click probe of feature E), anchored in world section-plane
/// coordinates like the two-point ruler.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ThicknessMark {
    entry: Vec3,
    exit: Vec3,
    thickness_mm: f32,
}

/// An in-slice measurement: either a two-point distance (anchors) or a one-click
/// wall thickness. Both anchor in world (section-plane) coordinates so they track
/// the geometry as the disc scales, and are dropped when the section plane
/// changes (a new plane means a different cross-section).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CutRuler {
    anchors: Vec<Vec3>,
    thickness: Option<ThicknessMark>,
    plane: Option<(Vec3, f32)>,
}

impl CutRuler {
    /// Drop the measurement when the live plane no longer matches the one it was
    /// placed against. Cheap; call once per frame with the current slice camera
    /// before drawing.
    pub(crate) fn sync_plane(&mut self, cam: SliceCam) {
        let live = cam.plane_signature();
        let stale = self.plane.is_some_and(|(n, d)| {
            n.dot(live.0) < 1.0 - PLANE_NORMAL_EPS || (d - live.1).abs() > PLANE_DISTANCE_EPS
        });
        if stale {
            self.clear();
        }
    }

    /// Place a distance point at `world` (already on the section plane),
    /// remembering the plane it belongs to. A third placement starts fresh; any
    /// thickness reading is replaced.
    pub(crate) fn place(&mut self, world: Vec3, cam: SliceCam) {
        self.thickness = None;
        if self.anchors.len() >= 2 {
            self.anchors.clear();
        }
        self.anchors.push(world);
        self.plane = Some(cam.plane_signature());
    }

    /// Set the one-click wall-thickness reading (world `entry`/`exit` on the
    /// section plane), replacing any distance points. Shared by the panel probe
    /// (feature E) and the probe-linked seed (feature D).
    pub(crate) fn set_thickness(
        &mut self,
        entry: Vec3,
        exit: Vec3,
        thickness_mm: f32,
        cam: SliceCam,
    ) {
        self.anchors.clear();
        self.thickness = Some(ThicknessMark {
            entry,
            exit,
            thickness_mm,
        });
        self.plane = Some(cam.plane_signature());
    }

    /// Clear every measurement.
    pub(crate) fn clear(&mut self) {
        self.anchors.clear();
        self.thickness = None;
        self.plane = None;
    }

    /// The placed distance anchors (0, 1, or 2 world points).
    #[cfg(test)]
    pub(crate) fn anchors(&self) -> &[Vec3] {
        &self.anchors
    }

    /// The wall-thickness reading in mm, if a thickness measurement is set.
    #[cfg(test)]
    pub(crate) fn thickness_reading_mm(&self) -> Option<f32> {
        self.thickness.map(|t| t.thickness_mm)
    }

    /// The measured distance in mm once two points exist.
    pub(crate) fn distance_mm(&self) -> Option<f64> {
        if let [a, b] = self.anchors.as_slice() {
            Some(SlicePlaneMap::distance_mm(*a, *b))
        } else {
            None
        }
    }

    pub(crate) fn thickness_probe(&self) -> Option<SliceProbe> {
        self.thickness.map(|mark| SliceProbe {
            entry: mark.entry,
            exit: mark.exit,
            thickness_mm: mark.thickness_mm,
        })
    }

    /// Draw the current measurement through `map` (so it scales with zoom/pan),
    /// using the SHARED measure-draw "ray" look so the panel reads identically to
    /// the main-viewport thickness probe.
    pub(crate) fn draw(&self, painter: &egui::Painter, map: &SlicePlaneMap) {
        if let Some(mark) = self.thickness {
            let entry = map.world_to_panel(mark.entry);
            let exit = map.world_to_panel(mark.exit);
            measure_draw::thickness_ray(
                painter,
                entry,
                exit,
                &format!("{:.2} mm", mark.thickness_mm),
            );
            return;
        }
        let points: Vec<egui::Pos2> = self
            .anchors
            .iter()
            .map(|w| map.world_to_panel(*w))
            .collect();
        match points.as_slice() {
            [a, b] => {
                measure_draw::segment(painter, *a, *b);
                measure_draw::anchor_dot(painter, *a);
                measure_draw::anchor_dot(painter, *b);
                if let Some(distance) = self.distance_mm() {
                    let mid = egui::pos2(
                        (a.x + b.x) * 0.5,
                        (a.y + b.y) * 0.5 - measure_draw::LABEL_LIFT_PX,
                    );
                    measure_draw::label_chip(
                        painter,
                        mid,
                        &format!("{distance:.2} mm"),
                        ui_theme::TEXT,
                    );
                }
            }
            [a] => measure_draw::anchor_dot(painter, *a),
            _ => {}
        }
    }
}
