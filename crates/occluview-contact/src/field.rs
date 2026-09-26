//! The measurement: every vertex of one surface against the other surface.
//!
//! # One probe, two directions
//!
//! Each side is measured against the other, so a caller painting both arches
//! (the case viewer does; a technician reads the bite from whichever side is
//! facing them) has both fields from one call. The two indices are built one at
//! a time and dropped before the next is built: a million-vertex scan pair
//! holds two grids from the same memory budget, and the peak decides whether a
//! laptop can finish this without running out of memory.
//!
//! # The value
//!
//! Positive is a gap to the opposing surface, zero is exact touch, negative is
//! penetration depth, and [`NO_CONTACT_MM`] means nothing was inside reach. The
//! gap magnitude is the true Euclidean distance to the closest point, while the
//! sign comes from that point's triangle normal — so near a cusp edge the two
//! are not the same number, and the distance is the one that is reported. A
//! vertex that is *inside* the antagonist by less than
//! [`PENETRATION_EPS_MM`](crate::PENETRATION_EPS_MM) reports the positive gap
//! instead, because below a micrometre the sign is decided by scanner noise
//! rather than by the bite.
//!
//! # Cancellation
//!
//! The flag is checked between sides and every
//! [`CANCEL_CHECK_STRIDE`](crate::CANCEL_CHECK_STRIDE) vertices inside one, and
//! a cancelled measurement returns what it had rather than nothing: the caller
//! that cancelled is closing a panel, and the values it already paid for are
//! the ones it is about to throw away anyway.

use std::time::Instant;

use glam::DVec3;
use occluview_align::{CancelFlag, Soup, SurfaceIndex};
use rayon::prelude::*;

use crate::stats::{self, ContactStats};
use crate::{CANCEL_CHECK_STRIDE, NO_CONTACT_MM, PENETRATION_EPS_MM, SEARCH_RADIUS_MM};

/// What to measure and how the result is post-processed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactSettings {
    /// Farthest a vertex may look for the opposing surface, in millimetres.
    ///
    /// Only the gap side: penetration is always searched to
    /// [`SEARCH_RADIUS_MM`], because a penetration deeper than the requested
    /// gap reach is the reading, not a miss. A non-finite or non-positive value
    /// falls back to the default.
    pub search_radius_mm: f64,
    /// Collapse each connected penetration patch to its peak depth.
    ///
    /// Off by default, which is what the shipped case viewer runs with. On is
    /// the right answer for a multi-hue ramp: without it the rim of an
    /// interference paints every intermediate depth as a rainbow ring around
    /// the saturated centre. Off keeps the force distribution *inside* a mark,
    /// which is the reading a technician wants from the tightness law.
    pub flatten_patches: bool,
}

impl Default for ContactSettings {
    fn default() -> Self {
        Self {
            // The default gap reach is the penetration reach, so a plain
            // measurement answers "how close is anything" over the whole band
            // the tightness law can paint and then some.
            search_radius_mm: SEARCH_RADIUS_MM,
            flatten_patches: false,
        }
    }
}

/// What the measurement cost and how much of it landed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContactDiagnostics {
    /// Wall time for the whole measurement, in milliseconds.
    pub worker_ms: f64,
    /// Vertices on the side the marks are painted on.
    pub subject_verts: usize,
    /// Vertices on the surface it is measured against.
    pub antagonist_verts: usize,
    /// Subject vertices that carry a measurement.
    pub subject_measured: u32,
    /// Antagonist vertices that carry a measurement.
    pub antagonist_measured: u32,
    /// Subject vertices in penetration (deeper than the sign dead-band).
    pub subject_penetrating: u32,
    /// Antagonist vertices in penetration.
    pub antagonist_penetrating: u32,
    /// Connected contact patches on the subject surface.
    pub contacts: u32,
}

/// One measurement: both signed fields and what they counted.
#[derive(Clone, Debug, PartialEq)]
pub struct ContactField {
    /// Signed millimetres per subject vertex, in subject vertex order.
    pub subject_signed_mm: Vec<f32>,
    /// Signed millimetres per antagonist vertex, in antagonist vertex order.
    pub antagonist_signed_mm: Vec<f32>,
    /// What the subject surface's own field counted.
    pub stats: ContactStats,
    /// Cost and coverage of this measurement.
    pub diagnostics: ContactDiagnostics,
}

/// Measure `subject` against `antagonist`, both already posed into world space.
///
/// Both are [`Soup`] — indexed triangles — and their normals come from winding
/// rather than from imported vertex normals, so the sign describes the geometry
/// on screen. The caller composes layer transforms itself, as the align job
/// does: this crate never sees a layer, so it can never disagree with the
/// renderer about where a scan is.
pub fn compute_contact_field(
    subject: Soup<'_>,
    antagonist: Soup<'_>,
    settings: ContactSettings,
    cancel: &CancelFlag,
) -> ContactField {
    let started = Instant::now();
    let mut diagnostics = ContactDiagnostics {
        subject_verts: subject.vertex_count(),
        antagonist_verts: antagonist.vertex_count(),
        ..ContactDiagnostics::default()
    };

    let mut subject_signed_mm = vec![NO_CONTACT_MM; subject.vertex_count()];
    let mut antagonist_signed_mm = vec![NO_CONTACT_MM; antagonist.vertex_count()];

    if !cancel.is_cancelled() {
        if let Some(index) = SurfaceIndex::build(antagonist) {
            subject_signed_mm = probe(subject.positions, &index, settings.search_radius_mm, cancel);
        }
        // The index of one side is dropped before the other is built: two
        // grids from a million-vertex pair is the memory peak that decides
        // whether this finishes on a laptop at all.
        if !cancel.is_cancelled() {
            if let Some(index) = SurfaceIndex::build(subject) {
                antagonist_signed_mm = probe(
                    antagonist.positions,
                    &index,
                    settings.search_radius_mm,
                    cancel,
                );
            }
        }
    }

    if settings.flatten_patches {
        crate::components::flatten_penetration_patches(subject.positions, &mut subject_signed_mm);
        crate::components::flatten_penetration_patches(
            antagonist.positions,
            &mut antagonist_signed_mm,
        );
    }

    let stats = stats::compute(subject.positions, subject.indices, &subject_signed_mm);
    diagnostics.subject_measured = measured(&subject_signed_mm);
    diagnostics.antagonist_measured = measured(&antagonist_signed_mm);
    diagnostics.subject_penetrating = penetrating(&subject_signed_mm);
    diagnostics.antagonist_penetrating = penetrating(&antagonist_signed_mm);
    diagnostics.contacts = stats.contacts;
    diagnostics.worker_ms = started.elapsed().as_secs_f64() * 1000.0;

    ContactField {
        subject_signed_mm,
        antagonist_signed_mm,
        stats,
        diagnostics,
    }
}

/// Measure every vertex of `positions` against `index`.
///
/// Parallel and deterministic: each slot is written by exactly one task, so the
/// result does not depend on how the work was split or how many threads ran it.
fn probe(
    positions: &[f32],
    index: &SurfaceIndex,
    gap_reach_mm: f64,
    cancel: &CancelFlag,
) -> Vec<f32> {
    let vertex_count = positions.len() / 3;
    let mut values = vec![NO_CONTACT_MM; vertex_count];
    let reach = search_reach(gap_reach_mm);
    let gap_gate = gap_gate(gap_reach_mm);
    values
        .par_iter_mut()
        .enumerate()
        .for_each(|(vertex, slot)| {
            if vertex % CANCEL_CHECK_STRIDE == 0 && cancel.is_cancelled() {
                return;
            }
            let offset = vertex * 3;
            let (Some(x), Some(y), Some(z)) = (
                positions.get(offset),
                positions.get(offset + 1),
                positions.get(offset + 2),
            ) else {
                return;
            };
            let point = DVec3::new(f64::from(*x), f64::from(*y), f64::from(*z));
            if !point.is_finite() {
                return;
            }
            if let Some(hit) = index.nearest(point, reach) {
                *slot = classify(point - hit.point, hit.normal, gap_gate);
            }
        });
    values
}

/// The signed value of one probe, or [`NO_CONTACT_MM`].
fn classify(delta: DVec3, normal: DVec3, gap_gate_mm: f64) -> f32 {
    let signed = delta.dot(normal);
    if signed < -f64::from(PENETRATION_EPS_MM) {
        return to_f32(signed);
    }
    let gap = delta.length();
    if gap <= gap_gate_mm {
        to_f32(gap)
    } else {
        NO_CONTACT_MM
    }
}

/// The radius the spatial index is asked for.
///
/// Never less than [`SEARCH_RADIUS_MM`]: the penetration direction has to reach
/// past the deepest thing any law can paint, or a strong interference renders as
/// a clean tooth in the middle of its own mark.
fn search_reach(gap_reach_mm: f64) -> f64 {
    if gap_reach_mm.is_finite() && gap_reach_mm > SEARCH_RADIUS_MM {
        gap_reach_mm
    } else {
        SEARCH_RADIUS_MM
    }
}

/// The gate the gap side is reported through.
fn gap_gate(gap_reach_mm: f64) -> f64 {
    if gap_reach_mm.is_finite() && gap_reach_mm > 0.0 {
        gap_reach_mm
    } else {
        SEARCH_RADIUS_MM
    }
}

/// How many vertices carry a measurement.
fn measured(values: &[f32]) -> u32 {
    to_u32(values.iter().filter(|value| value.is_finite()).count())
}

/// How many vertices are measurably inside the opposing surface.
fn penetrating(values: &[f32]) -> u32 {
    to_u32(
        values
            .iter()
            .filter(|value| value.is_finite() && **value < -PENETRATION_EPS_MM)
            .count(),
    )
}

/// A value narrowed for the field, saturating rather than producing infinity.
#[allow(clippy::cast_possible_truncation)]
fn to_f32(value: f64) -> f32 {
    value as f32
}

/// A count narrowed for the diagnostics, saturating rather than wrapping.
#[allow(clippy::cast_possible_truncation)]
fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
