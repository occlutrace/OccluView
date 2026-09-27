use super::*;

/// Everything a validated diagonal flip needs in order to be applied.
///
/// Same split as the collapse: the flip interleaved fourteen rejection gates
/// with the two triangle rewrites and their journal, row and index updates.
/// Validation now returns this plan and the commit applies it.
struct FlipPlan {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    t0: u32,
    t1: u32,
    before0: [u32; 3],
    before1: [u32; 3],
    after0: [u32; 3],
    after1: [u32; 3],
    n0: [u32; 3],
    n1: [u32; 3],
}

impl SculptSession {
    /// Replace the shared diagonal of two adjacent faces when the swap improves
    /// their shape. Validation and mutation are separate.
    // the edge, policy and journal are one candidate decision.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::sculpt_session::kernel) fn try_flip_edge(
        &mut self,
        a: u32,
        b: u32,
        diagonal_limit: f64,
        tolerance: f64,
        journal: &mut TopoJournal,
    ) -> bool {
        let Some(plan) = self.plan_flip_edge(a, b, diagonal_limit, tolerance) else {
            return false;
        };
        self.commit_flip_edge(plan, journal)
    }

    /// The surface direction at an interior edge, from its two incident faces.
    ///
    /// This is the only sound source for the orientation test, because the
    /// direction belongs to the edge, not to the brush. A triangle the pointer
    /// last hit is not a property of the edge at all, and the heal invalidates
    /// it against itself: a collapse
    /// truncates the triangle array and moves the tail face into a freed slot,
    /// so mid-heal the latched slot can be past the live count (every collapse
    /// then refused on a missing normal, for that cycle and every cycle after,
    /// while flips still ran) or can hold a face from elsewhere on the model
    /// (every orientation test then judged against the wrong side of the
    /// surface). Both failures were self-inflicted, and both are impossible once
    /// the normal is read from the two faces the operation is actually about.
    ///
    /// A consistently wound manifold gives its two incident faces the same
    /// geometric normal, so their sum is the local sheet direction. If the sum
    /// degenerates, the edge carries opposed or zero-area faces; that is not a
    /// surface an operation may rewrite, so the caller refuses.
    pub(super) fn edge_sheet_normal(&self, t0: u32, t1: u32) -> Option<DVec3> {
        let sum = self.triangle_normal(t0)? + self.triangle_normal(t1)?;
        (sum.length() > 1e-12).then(|| sum.normalize_or_zero())
    }

    /// Decide whether the diagonal `(a, b)` may be flipped, and describe how.
    /// Reads only: every rejection returns `None` with the surface untouched.
    // one candidate edge's full validation.
    #[allow(clippy::too_many_lines)]
    fn plan_flip_edge(
        &self,
        a: u32,
        b: u32,
        diagonal_limit: f64,
        tolerance: f64,
    ) -> Option<FlipPlan> {
        if a == b {
            return None;
        }
        // Retired groups are gone and sheets never fuse: every corner
        // shares one live component.
        let component = self
            .sheet_component
            .get(a as usize)
            .copied()
            .unwrap_or(u32::MAX);
        // One face per direction: a clean interior edge carries its two
        // incident triangles with opposite orientations.
        let mut incident = self.topology.edge_triangles(a, b).to_vec();
        incident.extend_from_slice(&self.topology.edge_triangles(b, a));
        incident.sort_unstable();
        incident.dedup();
        if incident.len() != 2 || incident[0] == incident[1] {
            return None;
        }
        let (t0, t1) = (incident[0], incident[1]);
        // The rows name the faces; the array decides. A row can still list a
        // face the edge no longer carries — the heal patches thousands of rows
        // incrementally — and a flip that trusts it rewrites two triangles that
        // were never adjacent, which is a crack in the surface. The split path
        // has always re-derived its plan from the corners; the flip must too.
        //
        // The opposite corner is found in GROUP space. Comparing the corner's
        // vertex id against `a`/`b` only works while group ids and vertex ids
        // coincide, which a welded surface (one group, several vertex slots)
        // and a densified one both break.
        let opposite_corner = |session: &Self, tri: u32| -> Option<([u32; 3], u32)> {
            let groups = session.topology.triangle(tri)?;
            for slot in 0..3 {
                let (from, to) = (groups[slot], groups[(slot + 1) % 3]);
                if (from == a && to == b) || (from == b && to == a) {
                    return Some((groups, groups[(slot + 2) % 3]));
                }
            }
            None
        };
        let (Some((f0, c)), Some((f1, d))) = (opposite_corner(self, t0), opposite_corner(self, t1))
        else {
            return None;
        };
        if a == c || a == d || b == c || b == d || c == d {
            return None;
        }
        // A flip rewrites which corners form the two faces and writes no vertex
        // position at all. Refusing it here would leave a dense band
        // permanently unrepaired, while every new face is still validated
        // against the session surface below. Liveness is checked for all four
        // corners just below.
        for &group in &[a, b, c, d] {
            if !self.group_is_live(group) {
                return None;
            }
            if self.sheet_component.get(group as usize).copied() != Some(component) {
                return None;
            }
        }
        // Conservative seam gate: every corner must be the sole member of
        // its welded group, so group-space rewiring maps onto raw indices
        // one-to-one. A seamed edge waits for the compacting snapshot path.
        for &group in &[a, b, c, d] {
            if self.topology.members(group).len() != 1 {
                return None;
            }
        }
        // The replacement diagonal must not already exist.
        if !self.topology.edge_triangles(c, d).is_empty()
            || !self.topology.edge_triangles(d, c).is_empty()
        {
            return None;
        }
        // Nor may it be longer than the splitter would tolerate. A flip that
        // mints a long edge is the repair creating the defect it removes.
        if (self.group_v(c) - self.group_v(d)).length() > diagonal_limit {
            return None;
        }
        let (pa, pb, pc, pd) = (
            self.group_v(a),
            self.group_v(b),
            self.group_v(c),
            self.group_v(d),
        );
        let old_quality = tri_quality(pa, pb, pc).min(tri_quality(pb, pa, pd));
        // Valence objective, alongside shape.
        //
        // A flip moves exactly one neighbour from `a` and `b` to `c` and `d`, so
        // their valences change by one each. Quality alone is blind to how many
        // faces meet at a vertex, which is the "bad valence" the operator sees as
        // a fan: a vertex can sit in perfectly well-shaped triangles and still
        // carry the wrong number of them. Summing the squared deviation from the
        // interior ideal of six lets a flip walk a fan toward six while refusing
        // the trade that would fix one vertex by breaking two (squaring is what
        // makes that trade a loss).
        //
        // Boundary vertices want four, not six: an open edge has one face where
        // an interior edge has two, so counting them the same would push a flip
        // the wrong way along a rim — which is where stitched patches live.
        // Target valence 6 interior, 4 on an open boundary — CGAL's
        // `is_border ? 4 : 6` in its flip predicate, and the same target the
        // Botsch-Kobbelt loop names.
        let valence = |group: u32| self.topology.neighbors(group).len() as f64;
        let desired = |group: u32| {
            if self.group_is_boundary(group) {
                4.0
            } else {
                6.0
            }
        };
        let deviation = |group: u32, delta: f64| {
            let excess = valence(group) + delta - desired(group);
            excess * excess
        };
        // a and b each lose one neighbour; c and d each gain one.
        let valence_before =
            deviation(a, 0.0) + deviation(b, 0.0) + deviation(c, 0.0) + deviation(d, 0.0);
        let valence_after =
            deviation(a, -1.0) + deviation(b, -1.0) + deviation(c, 1.0) + deviation(d, 1.0);
        // The orientation reference is read from the two faces this edge
        // actually carries, so it can never go stale behind the heal's own
        // rewires and can never describe a different part of the model.
        let sheet = self.edge_sheet_normal(t0, t1)?;
        let mut choice: Option<([u32; 3], [u32; 3])> = None;
        // Preserve the scale of the two live faces being replaced. Opening
        // material coordinates are unrelated to this diagonal after remeshing.
        let area_floor = {
            let corner = |group: u32| self.group_v(group);
            let area = triangle_cross([corner(a), corner(b), corner(c)])
                .length()
                .min(triangle_cross([corner(b), corner(a), corner(d)]).length());
            area * guards::MIN_SESSION_AREA_RATIO
        };
        let old_pair = [f0, f1];
        for candidate in [([c, d, b], [c, a, d]), ([c, b, d], [c, d, a])] {
            let (n0, n1) = candidate;
            if !replacement_preserves_raw_boundary(&old_pair, &[n0, n1]) {
                continue;
            }
            let q0 = tri_quality(
                self.group_v(n0[0]),
                self.group_v(n0[1]),
                self.group_v(n0[2]),
            );
            let q1 = tri_quality(
                self.group_v(n1[0]),
                self.group_v(n1[1]),
                self.group_v(n1[2]),
            );
            // Valence decides, quality is a veto rather than a score.
            //
            // A flip that improves valence must not be refused because it left
            // the pair's worst triangle a hair below where it started, or valence
            // could never be repaired on a mesh whose flips trade a little shape
            // for it — and the reverse trade, a shape gain that makes valence
            // worse, is the one this criterion exists to reject. The epsilon
            // floor still refuses a genuine shape loss.
            let valence_improves = valence_after + FLIP_QUALITY_EPSILON < valence_before;
            let quality_holds = q0.min(q1) + FLIP_QUALITY_EPSILON >= old_quality;
            let quality_improves = q0.min(q1) > old_quality + FLIP_QUALITY_EPSILON;
            if !((valence_improves && quality_holds) || quality_improves) {
                continue;
            }
            let face_normal = |tri: [u32; 3]| {
                (self.group_v(tri[1]) - self.group_v(tri[0]))
                    .cross(self.group_v(tri[2]) - self.group_v(tri[0]))
                    .normalize_or_zero()
            };
            if face_normal(n0).dot(sheet) <= 1e-9 || face_normal(n1).dot(sheet) <= 1e-9 {
                continue;
            }
            let smallest = triangle_cross([
                self.group_v(n0[0]),
                self.group_v(n0[1]),
                self.group_v(n0[2]),
            ])
            .length()
            .min(
                triangle_cross([
                    self.group_v(n1[0]),
                    self.group_v(n1[1]),
                    self.group_v(n1[2]),
                ])
                .length(),
            );
            if smallest < area_floor {
                continue;
            }
            choice = Some((n0, n1));
            break;
        }
        let (n0, n1) = choice?;
        // Virtual result: the links of a, b, c and d must stay single fans.
        // A flip that pinches a vertex would be published and rejected later.
        // Checked after the cheap shape and valence tests, which most
        // diagonals fail; it reads only the corners, not the chosen winding.
        if !self.flip_result_is_manifold(a, b, c, d, t0, t1) {
            return None;
        }
        // Raw corners follow the group order one-to-one through the
        // single-member gate above.
        let raw = |group: u32| self.topology.members(group)[0];
        let before0 = f0.map(raw);
        let before1 = f1.map(raw);
        let after0 = [raw(n0[0]), raw(n0[1]), raw(n0[2])];
        let after1 = [raw(n1[0]), raw(n1[1]), raw(n1[2])];
        // The two new faces must stay on the two they replace, and the removed
        // diagonal on the new pair: the only places two triangulations of the
        // same four points can part.
        let mut before = LocalSurface::new();
        before.push(t0, f0.map(|group| self.group_v(group)));
        before.push(t1, f1.map(|group| self.group_v(group)));
        let mut after = LocalSurface::new();
        for (slot, corners) in [(t0, after0), (t1, after1)] {
            let candidate = corners.map(|vertex| self.v(vertex));
            if !Self::triangle_final_is_safe(candidate, candidate)
                || !before.covers(candidate, tolerance, Some(slot))
            {
                return None;
            }
            after.push(slot, candidate);
        }
        if !after.within((pa + pb) * 0.5, tolerance, None) {
            return None;
        }
        Some(FlipPlan {
            a,
            b,
            c,
            d,
            t0,
            t1,
            before0,
            before1,
            after0,
            after1,
            n0,
            n1,
        })
    }

    /// Apply a validated flip. The single `return false` is a staleness check:
    /// if the two face slots no longer hold the corners the plan was built from,
    /// nothing is applied.
    fn commit_flip_edge(&mut self, plan: FlipPlan, journal: &mut TopoJournal) -> bool {
        let FlipPlan {
            a,
            b,
            c,
            d,
            t0,
            t1,
            before0,
            before1,
            after0,
            after1,
            n0,
            n1,
        } = plan;
        let offset0 = t0 as usize * 3;
        let offset1 = t1 as usize * 3;
        if self.tris.get(offset0..offset0 + 3) != Some(&before0[..])
            || self.tris.get(offset1..offset1 + 3) != Some(&before1[..])
        {
            return false;
        }
        self.tris[offset0..offset0 + 3].copy_from_slice(&after0);
        self.tris[offset1..offset1 + 3].copy_from_slice(&after1);
        self.topology.rewrite_triangle(t0, n0);
        self.topology.rewrite_triangle(t1, n1);
        journal.push_rewire(topology_journal::TopoRewire {
            tri: t0,
            before: before0,
            after: after0,
        });
        journal.push_rewire(topology_journal::TopoRewire {
            tri: t1,
            before: before1,
            after: after1,
        });
        // Incident rows recomputed exactly: only the two rewired faces
        // change membership, and only for the four corner groups.
        for &group in &[a, b, c, d] {
            let mut row = self.topology.incident_or_empty(group);
            row.retain(|&tri| tri != t0 && tri != t1);
            if n0.contains(&group) {
                row.push(t0);
            }
            if n1.contains(&group) {
                row.push(t1);
            }
            Self::set_incident_row(&mut self.topology, group, row);
        }
        let scope = [a, b, c, d];
        Self::refresh_neighbor_rows(&mut self.topology, &scope);
        // The ray grid catches up at the end of the step, with every face the
        // cycle touched.
        self.topo_touched.extend_from_slice(&scope);
        true
    }

    /// Flip every admissible interior edge of the footprint whose diagonal
    /// is worth replacing. Bounded by the policy op budget.
    // the footprint, policy, target and journal are one sweep.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::sculpt_session::kernel) fn flip_footprint_inner(
        &mut self,
        region: &[SurfacePoint],
        policy: &RemeshPolicy,
        target: f64,
        tolerance: f64,
        journal: &mut TopoJournal,
    ) {
        // The longest diagonal a flip may create.
        //
        // A flip is chosen for its valence and its shape, and neither term looks
        // at how LONG the new diagonal is: a pair of thin triangles can both
        // improve in quality while the diagonal that replaces them is far longer
        // than the edge it removed. That is a new long edge — the density defect
        // this heal exists to remove, minted by the repair itself — and it is
        // how a stroke over a density seam produced an Apply-rejected face.
        //
        // Bounded by the same single target the splitter uses, so a flip can
        // never create an edge the splitter would immediately cut back.
        //
        // Measuring it from the live footprint density instead would be
        // self-referential: in an over-dense patch the local scale is the dense
        // value, so the limit tightens to the patch's own tight spacing and the
        // flips that would even the patch out — the ones whose new diagonal is
        // longer than the old but still at target — are refused. One target for
        // both directions keeps the patch retriangulable.
        let diagonal_limit = if target.is_finite() && target > 0.0 {
            target * policy.split_hysteresis
        } else {
            // No usable target: leave the flip unconstrained rather than
            // inventing a limit, since the quality and valence terms still
            // apply.
            f64::INFINITY
        };
        let mut edges: Vec<(u32, u32)> = Vec::new();
        // Each edge once, at its first visit in footprint order, by stamp: a
        // neighbour already walked has listed the edge already.
        let walked = self.next_stamp();
        'walk: for point in region {
            let group = point.group;
            self.group_stamp[group as usize] = walked;
            for &neighbor in self.topology.neighbors(group) {
                if edges.len() >= policy.max_candidates_per_dab {
                    break 'walk;
                }
                if self.group_stamp[neighbor as usize] == walked {
                    continue;
                }
                edges.push(if group < neighbor {
                    (group, neighbor)
                } else {
                    (neighbor, group)
                });
            }
        }
        for (a, b) in edges {
            if !self.topology_operation_open(policy, journal) {
                break;
            }
            if self.try_flip_edge(a, b, diagonal_limit, tolerance, journal) {
                self.dab_topo_ops += 1;
            }
        }
    }
}
