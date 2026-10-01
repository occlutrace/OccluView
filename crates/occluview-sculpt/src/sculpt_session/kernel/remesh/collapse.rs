use super::*;

mod commit;

///
/// Validation produces this plan and nothing else, and the commit consumes it:
/// every rejection gate reads the surface before any write mutates it, so no
/// mutation happens before a decision is final.
struct CollapsePlan {
    survivor: u32,
    retired: u32,
    survivor_raw: u32,
    retired_raw: u32,
    /// Where the survivor lands: the midpoint, or its own position when it is a
    /// pinned boundary vertex.
    survivor_target: DVec3,
    /// The survivor's material point after the merge, by the same share of the
    /// same edge as its live move. `None` for a pinned survivor, which does
    /// not move at all.
    survivor_material: Option<DVec3>,
    /// `(face, before, after)` for every surviving face that changes a corner.
    rewires: Vec<(u32, [u32; 3], [u32; 3])>,
    /// The swap-deleted slots, in captured order.
    removals: Vec<CollapseRemoval>,
}

/// One swap-deleted face removal, captured before any mutation so that a
/// failure rejects with zero journal impact.
struct CollapseRemoval {
    slot: u32,
    last: u32,
    at_removed: [u32; 3],
    at_removed_origin: u32,
    at_last: [u32; 3],
    at_last_origin: u32,
    last_triple: [u32; 3],
}

impl SculptSession {
    /// Collapse a short welded edge onto its midpoint.
    ///
    /// Validation and mutation are separate: `plan_collapse_edge` decides, this
    /// applies what it decided.
    // the edge, policy and journal are one candidate decision.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::sculpt_session::kernel) fn try_collapse_edge(
        &mut self,
        a: u32,
        b: u32,
        policy: &RemeshPolicy,
        target: f64,
        append_base: u32,
        tolerance: f64,
        journal: &mut TopoJournal,
    ) -> bool {
        let Some(plan) = self.plan_collapse_edge(a, b, policy, target, append_base, tolerance)
        else {
            return false;
        };
        self.commit_collapse_edge(plan, journal)
    }

    // the footprint, policy, target and journal are one sweep.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::sculpt_session::kernel) fn collapse_footprint_inner(
        &mut self,
        region: &[SurfacePoint],
        policy: &RemeshPolicy,
        target: f64,
        append_base: u32,
        tolerance: f64,
        journal: &mut TopoJournal,
    ) {
        let edges = self.footprint_edges(region, policy.max_candidates_per_dab);
        for (a, b) in edges {
            if !self.topology_operation_open(policy, journal) {
                break;
            }
            if self.try_collapse_edge(a, b, policy, target, append_base, tolerance, journal) {
                self.dab_topo_ops += 1;
            }
        }
    }

    /// Decide whether `(a, b)` may be collapsed, and describe exactly how.
    ///
    /// Reads only: every rejection returns `None` with the surface untouched.
    // one candidate edge's full validation.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    fn plan_collapse_edge(
        &self,
        a: u32,
        b: u32,
        policy: &RemeshPolicy,
        target: f64,
        append_base: u32,
        tolerance: f64,
    ) -> Option<CollapsePlan> {
        if a == b || !(target > 0.0) {
            return None;
        }
        // Face-level guards validate the geometry of each surviving triangle;
        // a normal break alone does not make a collapse unsafe.
        let length = (self.group_v(a) - self.group_v(b)).length();
        // One policy target drives both split and collapse decisions. The
        // post-collapse stretch guard preserves fine features without letting
        // local density raise the collapse threshold.
        if length >= policy.collapse_threshold(target) {
            return None;
        }
        // A midpoint can lengthen the survivor's other incident edges. Check
        // every resulting edge against the split bound using the f32-rounded
        // midpoint that the commit writes.
        let mid = (self.group_v(a) + self.group_v(b)) * 0.5;
        let mid = DVec3::new(
            mid.x as f32 as f64,
            mid.y as f32 as f64,
            mid.z as f32 as f64,
        );
        if !mid.is_finite() {
            return None;
        }
        // Choose the endpoint to keep, and where it lands, before validating:
        // every surviving face must be checked against the position the commit
        // will actually write, not against the midpoint. Validating at the
        // midpoint and then committing at `survivor_target` would check a pinned
        // survivor's faces in one place and move them to another, inverting
        // them.
        //
        // Keep an open-boundary endpoint so the scan rim stays fixed. The
        // lower group id breaks an unconstrained tie deterministically.
        let take_b = if self.group_is_boundary(b) && !self.group_is_boundary(a) {
            true
        } else if self.group_is_boundary(a) && !self.group_is_boundary(b) {
            false
        } else {
            b < a
        };
        let (survivor, retired) = if take_b { (b, a) } else { (a, b) };
        let survivor_raw = self.topology.members(survivor)[0];
        let retired_raw = self.topology.members(retired)[0];
        // A boundary survivor stays fixed; every other survivor moves to the
        // edge midpoint. This keeps the scanned margin on its boundary.
        //
        // Placement is validated where it commits: the survivor and its landing
        // point are chosen here, before the checks below, and every surviving
        // face is measured at `survivor_target` — the position the commit
        // writes.
        let survivor_pinned = self.group_is_boundary(survivor);
        let survivor_material = (!survivor_pinned).then(|| {
            let own = self.reference_group_v(survivor);
            let middle = (self.reference_group_v(a) + self.reference_group_v(b)) * 0.5;
            own + (middle - own)
        });
        // A point of the edge lies on the surface already: the merge lands
        // there exactly, with no projection.
        let survivor_target = if survivor_pinned {
            self.group_v(survivor)
        } else {
            stored_position(self.group_v(survivor) + (mid - self.group_v(survivor)))
        };
        {
            // Bound edges from the survivor's actual landing point. A pinned
            // survivor can differ from the midpoint. This position-only check
            // runs before the surface checks.
            let limit = target * policy.split_hysteresis;
            for &group in &[a, b] {
                for &neighbor in self.topology.neighbors(group) {
                    if neighbor == a || neighbor == b {
                        continue;
                    }
                    if (self.group_v(neighbor) - survivor_target).length() > limit {
                        return None;
                    }
                }
            }
        }
        // A merge must preserve the orientation and quality of every surviving
        // live face, and the merged fan must stay on the two stars it replaces:
        // every new face within tolerance of the old surface, and both old
        // endpoints within tolerance of the new fan.
        let before = self.local_surface(&[a, b])?;
        let mut merged = LocalSurface::new();
        let mut checked: Vec<u32> = Vec::new();
        for &group in &[a, b] {
            for &tri in self.topology.incident_triangles(group) {
                let Some(corners) = self.topology.triangle(tri) else {
                    continue;
                };
                // The two faces that span this edge are swap-deleted, so they
                // become (mid, mid, c) by construction. They are not survivors
                // and their degeneracy is the merge, not a defect.
                if corners.contains(&a) && corners.contains(&b) {
                    continue;
                }
                if checked.contains(&tri) {
                    continue;
                }
                checked.push(tri);
                let mut candidate = [DVec3::ZERO; 3];
                let mut baseline = [DVec3::ZERO; 3];
                for (i, &corner) in corners.iter().enumerate() {
                    candidate[i] = if corner == a || corner == b {
                        survivor_target
                    } else {
                        self.group_v(corner)
                    };
                    baseline[i] = self.group_v(corner);
                }
                if !Self::triangle_final_is_safe(baseline, candidate)
                    || !before.covers(candidate, tolerance, Some(tri))
                    || !merged.push(tri, candidate)
                {
                    return None;
                }
            }
        }
        if !merged.within(self.group_v(a), tolerance, None)
            || !merged.within(self.group_v(b), tolerance, None)
        {
            return None;
        }
        for &group in &[a, b] {
            if self.topology.members(group).len() != 1 {
                return None;
            }
        }
        let mut incident = self.topology.edge_triangles(a, b).to_vec();
        incident.extend_from_slice(&self.topology.edge_triangles(b, a));
        incident.sort_unstable();
        incident.dedup();
        if incident.len() != 2 || incident[0] == incident[1] {
            return None;
        }
        let (t0, t1) = (incident[0], incident[1]);
        if t0 >= append_base || t1 >= append_base {
            return None;
        }
        // The orientation reference comes from these two faces, not from the
        // brush: see `edge_sheet_normal`.
        let sheet = self.edge_sheet_normal(t0, t1)?;
        let (Some(f0), Some(f1)) = (self.topology.triangle(t0), self.topology.triangle(t1)) else {
            return None;
        };
        let c = *f0.iter().find(|&&g| g != a && g != b)?;
        let d = *f1.iter().find(|&&g| g != a && g != b)?;
        if a == c || a == d || b == c || b == d || c == d {
            return None;
        }
        // Opposite corners need only be live: the collapse rewires their faces
        // without moving them.
        if !self.group_is_live(c) || !self.group_is_live(d) {
            return None;
        }
        let component = self
            .sheet_component
            .get(a as usize)
            .copied()
            .unwrap_or(u32::MAX);
        for &group in &[b, c, d] {
            if self.sheet_component.get(group as usize).copied() != Some(component) {
                return None;
            }
        }
        // Link condition: the endpoints' common one-ring is exactly the
        // two opposite corners, so no tunnel or fin folds through here.
        let mut common: Vec<u32> = self
            .topology
            .neighbors(a)
            .iter()
            .copied()
            .filter(|g| self.topology.neighbors(b).contains(g))
            .collect();
        common.sort_unstable();
        let mut expected = [c, d];
        expected.sort_unstable();
        if common != expected {
            return None;
        }
        // Virtual result: every affected vertex's link must stay a single
        // cycle/path. This is the full simplicial link condition (shared link
        // edges, tunnel, boundary pinch, disconnected fan), not just the
        // common-neighbour test above.
        if !self.collapse_result_is_manifold(a, b) {
            return None;
        }
        // Both dying faces already face the sculpted sheet: collapsing a
        // backface would smuggle a fold through the orientation checks
        // below, which only see substituted faces.
        for &face in &[t0, t1] {
            let normal = self.triangle_normal(face)?;
            if normal.dot(sheet) <= 1e-9 {
                return None;
            }
        }
        // Midpoint on the frozen edge, f32-rounded like split midpoints.
        // Already computed for the post-collapse length guard above.
        // Every surviving incident face keeps finite area, its orientation,
        // and the session baseline-safety law after the substitution.
        let mut rewires: Vec<(u32, [u32; 3], [u32; 3])> = Vec::new();
        let mut seen_faces: Vec<u32> = vec![t0, t1];
        for &group in &[a, b] {
            for &face in self.topology.incident_triangles(group) {
                if seen_faces.contains(&face) {
                    continue;
                }
                seen_faces.push(face);
                let corners = self.topology.triangle(face)?;
                let raw_off = face as usize * 3;
                let raw = self.tris.get(raw_off..raw_off + 3)?;
                let raw = [raw[0], raw[1], raw[2]];
                let mut after = raw;
                let mut groups_after = corners;
                let mut touched = false;
                for i in 0..3 {
                    if corners[i] == retired {
                        after[i] = self.topology.members(survivor)[0];
                        groups_after[i] = survivor;
                        touched = true;
                    }
                }
                if !touched {
                    continue;
                }
                if after[0] == after[1] || after[1] == after[2] || after[2] == after[0] {
                    return None;
                }
                // Candidate positions: both collapsed corners land on the
                // midpoint at commit (I1: validating only the retired
                // corner would clear moves the survivor cannot make).
                let moved_pts = after.map(|v| {
                    if v == retired_raw {
                        // The retired corner always lands on the survivor.
                        survivor_target
                    } else if v == survivor_raw {
                        survivor_target
                    } else {
                        self.v(v)
                    }
                });
                let normal = (moved_pts[1] - moved_pts[0]).cross(moved_pts[2] - moved_pts[0]);
                if !(normal.length() > 1e-12) || normal.dot(sheet) <= 0.0 {
                    return None;
                }
                let baseline = raw.map(|vertex| self.v(vertex));
                if !Self::triangle_final_is_safe(baseline, moved_pts) {
                    return None;
                }
                let parent_area = triangle_cross(baseline).length();
                if normal.length() < parent_area * guards::MIN_SESSION_AREA_RATIO {
                    return None;
                }
                rewires.push((face, raw, after));
            }
        }
        // Protected positions remain fixed at commit; every substituted face is
        // still checked against the baseline safety rule below.
        // Duplicate faces: no substituted triple may already exist live.
        // Faces being rewired away are excluded: their old triples vanish.
        let rewired_ids: Vec<u32> = rewires.iter().map(|(face, _, _)| *face).collect();
        let mut live_sets: Vec<[u32; 3]> = Vec::new();
        for &group in &[survivor, c, d] {
            for &face in self.topology.incident_triangles(group) {
                if face == t0 || face == t1 || rewired_ids.contains(&face) {
                    continue;
                }
                if let Some(triple) = self.topology.triangle(face) {
                    let mut sorted = triple;
                    sorted.sort_unstable();
                    live_sets.push(sorted);
                }
            }
        }
        for (_, _, after) in &rewires {
            let mut sorted = [
                self.topology.group_of(after[0]),
                self.topology.group_of(after[1]),
                self.topology.group_of(after[2]),
            ];
            sorted.sort_unstable();
            if live_sets.contains(&sorted) {
                return None;
            }
            live_sets.push(sorted);
        }
        // Removal capture before any mutation (C1): the read consults
        // the rewire after-overlay first, so a rewired tail face used as
        // a swap source is captured with post-rewire corners while every
        // failure still rejects pre-mutation with zero journal impact.
        // Dying slots are unaffected by rewires, so their capture is
        // order-free either way.
        let mut removals: Vec<CollapseRemoval> = Vec::with_capacity(2);
        let mut predicted_live = self.live_tris;
        // Slots already rewritten by an earlier removal in this same op
        // read back the predicted content, not the stale vec. The group
        // triple travels in the same overlay entry.
        let mut predicted: Vec<(u32, [u32; 3], u32, [u32; 3])> = Vec::new();
        // note: the capture loop lives just below, before the positions
        // commit (C1 ordering); `dying`, `predicted`, and `predicted_live`
        // are declared here so validation order stays readable.
        let mut dying = [t0, t1];
        dying.sort_unstable_by(|x, y| y.cmp(x));
        // Removal capture (C1, pre-mutation): tail faces used as swap
        // sources are read with post-rewire corners via the overlay.
        for &slot in &dying {
            if slot >= append_base || slot >= predicted_live {
                return None;
            }
            let last = predicted_live - 1;
            let read_slot = |session: &Self,
                             predicted: &[(u32, [u32; 3], u32, [u32; 3])],
                             rewired: &[(u32, [u32; 3], [u32; 3])],
                             slot: u32|
             -> Option<([u32; 3], u32, [u32; 3])> {
                if let Some((_, corners, origin, triple)) =
                    predicted.iter().find(|(s, _, _, _)| *s == slot)
                {
                    return Some((*corners, *origin, *triple));
                }
                if let Some((_, _, after)) = rewired.iter().find(|(f, _, _)| *f == slot) {
                    let off = slot as usize * 3;
                    session.tris.get(off..off + 3)?;
                    let origin = *session.face_origin.get(slot as usize)?;
                    let triple = [
                        session.topology.group_of(after[0]),
                        session.topology.group_of(after[1]),
                        session.topology.group_of(after[2]),
                    ];
                    return Some((*after, origin, triple));
                }
                let off = slot as usize * 3;
                let slice = session.tris.get(off..off + 3)?;
                let corners = [slice[0], slice[1], slice[2]];
                let origin = *session.face_origin.get(slot as usize)?;
                let triple = session.topology.triangle(slot)?;
                Some((corners, origin, triple))
            };
            let (
                Some((at_last, at_last_origin, last_triple)),
                Some((at_removed, at_removed_origin, _)),
            ) = (
                read_slot(self, &predicted, &rewires, last),
                read_slot(self, &predicted, &rewires, slot),
            )
            else {
                return None;
            };
            removals.push(CollapseRemoval {
                slot,
                last,
                at_removed,
                at_removed_origin,
                at_last,
                at_last_origin,
                last_triple,
            });
            predicted.push((slot, at_last, at_last_origin, last_triple));
            predicted_live = last;
        }
        Some(CollapsePlan {
            survivor,
            retired,
            survivor_raw,
            retired_raw,
            survivor_target,
            survivor_material,
            rewires,
            removals,
        })
    }
}
