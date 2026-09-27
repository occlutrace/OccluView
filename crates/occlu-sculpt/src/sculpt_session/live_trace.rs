//! Per-pointer-call evidence for the operator console.
//!
//! The pasteable stroke report only exists after pointer-up, so a hole that
//! lives only while the button is down never appears there. This snapshot is
//! packed onto every dab reply: flags, refusals, journal deltas, a footprint
//! optional audit of the triangles the brush just touched, and the clay
//! kinematics that actually moved vertices. The footprint audit runs only for
//! explicit browser diagnostics; scalar production counters stay cheap. The
//! live-topology words report the cycle each dab ran. Camera-back and edge-on
//! counts remain useful because those faces stay indexed even when a one-sided
//! draw hides them.

use super::*;
use std::collections::{HashMap, HashSet};

/// Packed live dab evidence. Wire order is shared with the browser decoder.
#[derive(Clone, Copy, Debug)]
pub struct LiveTrace {
    /// Packed integer evidence words.
    pub words: [u32; Self::WORDS],
    /// Packed float evidence values.
    pub floats: [f32; Self::FLOATS],
}

impl LiveTrace {
    /// Number of integer words per trace.
    pub const WORDS: usize = 51;
    /// Number of float values per trace.
    pub const FLOATS: usize = 24;
}

/// Packed into every live dab so a consumer cannot pair its own engine
/// version with a stale kernel. Bump it when the clay or remesh law changes.
pub const SCULPT_LIVE_KERNEL: u32 = 516;

impl Default for LiveTrace {
    fn default() -> Self {
        Self {
            words: [0; Self::WORDS],
            floats: [0.0; Self::FLOATS],
        }
    }
}

/// Accumulators for one pointer call (up to two spaced dabs). Pose fields
/// are the last dab; counters and spread are the call union.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LiveKinematics {
    /// The pose this pointer call applied, for the published live line.
    pub mode: u32,
    pub radius: f32,
    pub strength: f32,
    pub pre_densify_ops: u32,
    pub remesh_ops: u32,
    pub keep_restored: u32,
    pub keep_candidates: u32,
    pub weighted: u32,
    pub proposals: u32,
    pub local_normal: bool,
    pub hit_sheet: bool,
    pub nx: f32,
    pub ny: f32,
    pub nz: f32,
    pub facing: f32,
    pub amplitude: f32,
    pub gain: f32,
    pub front_min: f32,
    pub front_max: f32,
    pub front_mean: f32,
    pub toward_frac: f32,
    pub cx: f32,
    pub cy: f32,
    pub cz: f32,
    pub vx: f32,
    pub vy: f32,
    pub vz: f32,
    pub hn_x: f32,
    pub hn_y: f32,
    pub hn_z: f32,
    pub target_mm: f32,
    pub n_spread_deg: f32,
    pub flood_mm: f32,
    pub peak_move_mm: f32,
}

impl SculptSession {
    /// Snapshot what this pointer call did and what the live surface looks
    /// like after it.
    // one packed evidence record is built from its counters in one place.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    pub(crate) fn capture_live_trace(
        &self,
        before: DabDiagnostics,
        dabs_in_call: u32,
        moved: &[u32],
        complete: bool,
        hit: bool,
        journal_added_verts: u32,
        journal_added_tris: u32,
        journal_rewired: u32,
        journal_collapsed: u32,
    ) -> LiveTrace {
        let after = diag::peek();
        let delta = |now: u32, then: u32| now.saturating_sub(then);
        let kin = self.live_kin;
        let mut flags = 0u32;
        if self.remesh_armed {
            flags |= 1;
        }
        if self.topo_budget_open {
            flags |= 2;
        }
        if complete {
            flags |= 8;
        }
        if hit {
            flags |= 16;
        }
        if kin.keep_candidates > 0 {
            flags |= 32;
        }
        // Bits 6 and 7 both report the live cycle: it splits (densify) and
        // re-triangulates (remesh) in one pass, so a cycle that spent any
        // operation sets both. Bit 6's old "preserve features" meaning is
        // retired with the bend gate it named.
        if kin.remesh_ops > 0 {
            flags |= 128;
            flags |= 256;
        }
        if kin.local_normal {
            flags |= 512;
        }
        if kin.keep_restored > 0 {
            flags |= 1024;
        }
        if kin.hit_sheet {
            flags |= 2048;
        }
        if self.live_trace_audit {
            flags |= 4096;
        }

        let mut peak_mm = kin.peak_move_mm as f64;
        let mut unmoved = 0u32;
        for &group in &self.dab_groups {
            let index = group as usize;
            if index >= self.group_stamp.len()
                || self.snapshot_stamp[index] != self.snapshot_generation
            {
                continue;
            }
            let travel = (self.group_v(group) - self.pre_group(group)).length();
            if travel > peak_mm {
                peak_mm = travel;
            }
            if travel <= 1e-9 {
                unmoved += 1;
            }
        }

        let audit = if self.live_trace_audit {
            let mut groups: HashSet<u32> = HashSet::new();
            for point in &self.region_points {
                groups.insert(point.group);
            }
            for &vertex in moved {
                if (vertex as usize) < self.verts.len() / 3 {
                    groups.insert(self.topology.group_of(vertex));
                }
            }
            for &group in &self.dab_groups {
                groups.insert(group);
            }
            self.audit_groups(&groups)
        } else {
            FootprintAudit::default()
        };
        // The pose this call applied, from the live kinematics the brush
        // solvers publish. A call that placed no dab reports zeros.
        let (mode, radius, strength) = (kin.mode, kin.radius, kin.strength);
        let journal_bytes = self.stroke_topo_bytes.min(u32::MAX as u64) as u32;
        let facing_code = if kin.facing < 0.0 {
            0
        } else if kin.facing > 0.0 {
            2
        } else {
            1
        };

        let mut words = [0u32; LiveTrace::WORDS];
        words[0] = flags;
        words[1] = dabs_in_call;
        words[2] = self.region_points.len() as u32;
        words[3] = moved.len() as u32;
        words[4] = self.topology.group_count() as u32;
        words[5] = self.live_tris;
        words[6] = self.topo_revision.0;
        words[7] = self.dab_topo_ops as u32;
        words[8] = delta(after.seed_missing, before.seed_missing);
        words[9] = delta(after.region_empty, before.region_empty);
        words[10] = delta(after.clamp_zero, before.clamp_zero);
        words[11] = delta(after.clamp_truncated, before.clamp_truncated);
        words[12] = delta(after.gain_scaled_dabs, before.gain_scaled_dabs);
        words[13] = after.gain_min_permille;
        words[14] = delta(after.rollback_resets, before.rollback_resets);
        words[15] = delta(after.already_unsafe_dabs, before.already_unsafe_dabs);
        words[18] = delta(after.no_move_dabs, before.no_move_dabs);
        words[19] = delta(after.region_points_total, before.region_points_total);
        words[20] = audit.degenerate;
        words[21] = audit.below_floor;
        words[22] = audit.flipped;
        words[23] = audit.boundary;
        words[24] = journal_added_verts;
        words[25] = journal_added_tris;
        words[26] = journal_rewired;
        words[27] = journal_collapsed;
        words[28] = unmoved;
        words[29] = mode;
        words[30] = self.brush_tip as u32;
        words[31] = self.dab_groups.len() as u32;
        words[32] = kin.pre_densify_ops;
        words[33] = kin.remesh_ops;
        words[34] = kin.keep_restored;
        words[35] = kin.keep_candidates;
        words[36] = kin.weighted;
        words[37] = kin.proposals;
        words[38] = self.vertex_count() as u32;
        // Packed kernel identity. A consumer that also stamps its own engine
        // version compares the two: a mismatch means it is reading claims from
        // a different kernel than the one that produced them.
        words[39] = SCULPT_LIVE_KERNEL;
        words[40] = (kin.target_mm * 1000.0).round().max(0.0) as u32;
        words[41] = (kin.toward_frac * 1000.0).round().clamp(0.0, 1000.0) as u32;
        words[42] = self.hit_triangle.unwrap_or(u32::MAX);
        words[43] = (kin.n_spread_deg * 100.0).round().max(0.0) as u32;
        words[44] = (kin.flood_mm * 1000.0).round().max(0.0) as u32;
        words[45] = journal_bytes;
        words[46] = facing_code;
        words[47] = (kin.front_mean * 1000.0).round().clamp(0.0, 1000.0) as u32;
        // Camera-hidden faces stay in the index, so deg/floor/flip stay 0
        // while the operator looks through the stroke. Count them here.
        words[48] = audit.camera_back;
        words[49] = audit.camera_edge;
        words[50] = audit.camera_hid;

        let mut floats = [0.0f32; LiveTrace::FLOATS];
        floats[0] = audit.max_edge_ratio as f32;
        floats[1] = audit.smallest_area as f32;
        floats[2] = peak_mm as f32;
        floats[3] = radius;
        floats[4] = strength;
        floats[5] = kin.nx;
        floats[6] = kin.ny;
        floats[7] = kin.nz;
        floats[8] = kin.facing;
        floats[9] = kin.amplitude;
        floats[10] = kin.gain;
        floats[11] = kin.cx;
        floats[12] = kin.cy;
        floats[13] = kin.cz;
        floats[14] = kin.front_min;
        floats[15] = kin.front_max;
        floats[16] = kin.vx;
        floats[17] = kin.vy;
        floats[18] = kin.vz;
        floats[19] = kin.hn_x;
        floats[20] = kin.hn_y;
        floats[21] = kin.hn_z;
        floats[22] = kin.target_mm;
        floats[23] = kin.n_spread_deg;
        LiveTrace { words, floats }
    }

    fn audit_groups(&self, groups: &HashSet<u32>) -> FootprintAudit {
        let mut out = FootprintAudit::default();
        if groups.is_empty() {
            return out;
        }
        let mut seen: HashSet<u32> = HashSet::new();
        let mut edges: HashMap<(u32, u32), Vec<(u32, u32)>> = HashMap::new();
        // Same draw floor live Smooth and Add refuse to create. A face four
        // orders below a millimetre square is a hole on a one-sided rasterizer.
        for &group in groups {
            if self
                .group_retired
                .get(group as usize)
                .copied()
                .unwrap_or(true)
            {
                continue;
            }
            for &tri in self.topology.incident_triangles(group) {
                if tri >= self.live_tris || !seen.insert(tri) {
                    continue;
                }
                let Some(corners) = self.topology.triangle(tri) else {
                    continue;
                };
                if corners[0] == corners[1] || corners[1] == corners[2] || corners[0] == corners[2]
                {
                    out.degenerate += 1;
                    continue;
                }
                let a = self.group_v(corners[0]);
                let b = self.group_v(corners[1]);
                let c = self.group_v(corners[2]);
                let now_cross = (b - a).cross(c - a);
                let double_area = now_cross.length();
                out.smallest_area = out.smallest_area.min(double_area);
                let (ab, bc, ca) = ((b - a).length(), (c - b).length(), (a - c).length());
                let longest = ab.max(bc).max(ca);
                let shortest = ab.min(bc).min(ca);
                if shortest > 1e-12 {
                    out.max_edge_ratio = out.max_edge_ratio.max(longest / shortest);
                }
                if double_area < guards::LIVE_PAINTABLE_AREA {
                    out.below_floor += 1;
                }
                let view = DVec3::new(
                    self.live_kin.vx as f64,
                    self.live_kin.vy as f64,
                    self.live_kin.vz as f64,
                );
                let facing = self.live_kin.facing as f64;
                if view.length() > 1e-12
                    && facing != 0.0
                    && double_area >= guards::LIVE_PAINTABLE_AREA
                {
                    let view = view.normalize_or_zero();
                    let now_signed = now_cross * facing;
                    // Same toward test as clay: front has signed n·view < 0,
                    // so -n·view is the covering amount. GPU FrontSide draws
                    // the reverse as the red interior, and an edge-on face
                    // as empty — both look like deleted triangles.
                    let now_cam = -now_signed.dot(view);
                    let now_cos = now_cam / double_area;
                    if now_cam < 0.0 {
                        out.camera_back += 1;
                    } else if now_cos.abs() < guards::EDGE_ON_COS {
                        out.camera_edge += 1;
                    }
                    let pre_a = self.pre_group(corners[0]);
                    let pre_b = self.pre_group(corners[1]);
                    let pre_c = self.pre_group(corners[2]);
                    let pre_cross = (pre_b - pre_a).cross(pre_c - pre_a) * facing;
                    let pre_geom = pre_cross.length();
                    if pre_geom > 1e-18 {
                        let pre_cam = -pre_cross.dot(view);
                        let pre_cos = pre_cam / pre_geom;
                        let hidden_now = now_cam < 0.0 || now_cos.abs() < guards::EDGE_ON_COS;
                        if pre_cos > guards::EDGE_ON_COS && hidden_now {
                            out.camera_hid += 1;
                        }
                    }
                }
                for k in 0..3 {
                    let u = corners[k];
                    let v = corners[(k + 1) % 3];
                    edges.entry((u.min(v), u.max(v))).or_default().push((u, v));
                }
            }
        }
        for occ in edges.into_values() {
            if occ.len() == 1 {
                out.boundary += 1;
            } else if occ.len() == 2 && occ[0] == occ[1] {
                out.flipped += 1;
            }
        }
        out
    }
}

#[derive(Clone, Copy)]
struct FootprintAudit {
    degenerate: u32,
    below_floor: u32,
    flipped: u32,
    boundary: u32,
    camera_back: u32,
    camera_edge: u32,
    camera_hid: u32,
    max_edge_ratio: f64,
    smallest_area: f64,
}

impl Default for FootprintAudit {
    fn default() -> Self {
        Self {
            degenerate: 0,
            below_floor: 0,
            flipped: 0,
            boundary: 0,
            camera_back: 0,
            camera_edge: 0,
            camera_hid: 0,
            max_edge_ratio: 0.0,
            smallest_area: f64::INFINITY,
        }
    }
}
