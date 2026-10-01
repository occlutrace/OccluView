use super::*;

impl SculptSession {
    fn collect_layer_triangles(&mut self, groups: &[u32], out: &mut Vec<u32>) {
        out.clear();
        let epoch = self.next_tri_stamp();
        for &group in groups {
            for &triangle in self.topology.incident_triangles(group) {
                if self.tri_marks[triangle as usize] != epoch {
                    self.tri_marks[triangle as usize] = epoch;
                    out.push(triangle);
                }
            }
        }
    }

    fn collect_unsafe_layer_triangles(
        &self,
        triangles: &[u32],
        mode: BrushMode,
        out: &mut Vec<u32>,
    ) {
        out.clear();
        #[cfg(feature = "parallel")]
        if triangles.len() >= PAR_FLOOR {
            use rayon::prelude::*;
            out.resize(triangles.len(), 0);
            out.par_iter_mut()
                .zip(triangles.par_iter())
                .for_each(|(flag, &triangle)| {
                    *flag = u32::from(self.layer_triangle_is_unsafe(triangle, mode));
                });
            let mut unsafe_count = 0;
            for (index, &triangle) in triangles.iter().enumerate() {
                if out[index] != 0 {
                    out[unsafe_count] = triangle;
                    unsafe_count += 1;
                }
            }
            out.truncate(unsafe_count);
            return;
        }
        out.extend(
            triangles
                .iter()
                .copied()
                .filter(|&triangle| self.layer_triangle_is_unsafe(triangle, mode)),
        );
    }

    fn collect_layer_controls(&self, triangle: u32, out: &mut Vec<u32>) {
        out.clear();
        let Some(corners) = self.topology.triangle(triangle) else {
            return;
        };
        for group in corners {
            if self.snapshot_stamp[group as usize] == self.snapshot_generation {
                out.push(group);
            }
        }
    }

    /// Keep the local rollback field continuous over the moving footprint.
    /// A constrained corner may lower its neighbours by at most one eighth per
    /// welded edge, so an isolated zero cannot remain between full-dose
    /// vertices and become the thin peak the operator sees. The wave stops
    /// once it reaches groups whose factor already supplies that transition;
    /// disconnected and distant safe parts keep their full dose.
    fn taper_layer_factors(&mut self, seeds: &[u32]) {
        /// Fall of the rollback field per welded edge. This is the slope the
        /// taper was designed around, and it stays fixed: tying it to the wave
        /// count would silently flatten the taper every time the budget changed.
        const MAX_FACTOR_STEP: f32 = 0.125;

        self.rollback_epoch = self.rollback_epoch.wrapping_add(1);
        if self.rollback_epoch == 0 {
            self.rollback_marks.fill(0);
            self.rollback_epoch = 1;
        }
        let epoch = self.rollback_epoch;
        let mut queue = Vec::with_capacity(seeds.len());
        for &group in seeds {
            if self.rollback_marks[group as usize] != epoch {
                self.rollback_marks[group as usize] = epoch;
                queue.push(group);
            }
        }
        let mut cursor = 0;
        while cursor < queue.len() {
            let group = queue[cursor];
            cursor += 1;
            // `marks` means queued, not permanently visited. A second seed
            // can lower a processed group; clearing its mark lets the tighter
            // ceiling continue through the remaining mesh.
            self.rollback_marks[group as usize] = 0;
            let ceiling = (self.rollback_factor[group as usize] + MAX_FACTOR_STEP).min(1.0);
            for &neighbor in self.topology.neighbors(group) {
                if self.snapshot_stamp[neighbor as usize] != self.snapshot_generation
                    || self.rollback_factor[neighbor as usize] <= ceiling
                {
                    continue;
                }
                self.rollback_factor[neighbor as usize] = ceiling;
                if self.rollback_marks[neighbor as usize] != epoch {
                    self.rollback_marks[neighbor as usize] = epoch;
                    queue.push(neighbor);
                }
            }
        }
    }

    /// Validate one simultaneous brush field against the surface immediately
    /// before this dab. Add grows a layer of material, so stretching relative to
    /// the session-opening tessellation is expected and must not become a
    /// cumulative stop. Smooth is shape flow, so it is bounded by the live
    /// face it is improving rather than the frozen opening mesh.
    fn layer_triangle_is_unsafe(&self, triangle: u32, mode: BrushMode) -> bool {
        let Some(corners) = self.topology.triangle(triangle) else {
            return false;
        };
        let pre = TriangleMeasure::new(corners.map(|group| self.pre_group(group)));
        let now = TriangleMeasure::new(corners.map(|group| self.group_v(group)));
        if (0..3).all(|corner| {
            let a = pre.points[corner];
            let b = now.points[corner];
            a.x == b.x && a.y == b.y && a.z == b.z
        }) {
            return false;
        }
        if !now.area.is_finite() || Self::triangle_flipped(pre, now) {
            return true;
        }

        let tolerance = clay_area_roundoff(pre, now);
        match mode {
            BrushMode::Smooth | BrushMode::Relax => {
                // A coherent normal-flow pass may not fold, tear or hide a
                // face; the remesh that follows repairs a thin one. No camera
                // or frozen-reference test belongs here: changing the
                // silhouette and reducing curvature is Smooth's declared job.
                !Self::triangle_editable_after_move_measured(pre, now)
            }
            BrushMode::Deposit => {
                // Repeated growth legitimately elongates triangles until the
                // live remesh retessellates them, so an opening-mesh quality
                // limit is not an Add limit. Keep face winding and its
                // paintable area independent of the current camera.
                let reference =
                    TriangleMeasure::new(corners.map(|group| self.reference_group_v(group)));
                if now.area + tolerance < (reference.area * MIN_SESSION_AREA_RATIO).min(pre.area) {
                    return true;
                }
                now.area + 1e-18 < LIVE_PAINTABLE_AREA && now.area + 1e-18 < pre.area
            }
            BrushMode::Erode => {
                // Removal can compress the remaining wall. Keep the live
                // shape and immutable apply floors. An already-damaged opening face must
                // remain editable: the immutable floor applies only while the
                // pre-dab face still satisfies it; otherwise the live
                // non-worsening predicate is the authority.
                let reference =
                    TriangleMeasure::new(corners.map(|group| self.reference_group_v(group)));
                if !Self::triangle_editable_after_move_measured(pre, now)
                    || (Self::triangle_shape_is_safe_measured(reference, pre)
                        && !Self::triangle_shape_is_safe_measured(reference, now))
                    || now.area + tolerance
                        < (reference.area * MIN_SESSION_AREA_RATIO).min(pre.area)
                {
                    return true;
                }
                false
            }
            BrushMode::Flatten => {
                // Flatten never reaches the layer commit: `dab_flatten` writes
                // group positions directly, so this predicate never sees it.
                // Fail closed. If a future change does route Flatten through the
                // layer commit, refusing the face makes the dab visibly do
                // nothing instead of publishing geometry no guard has checked;
                // `debug_assert!` is a no-op in release, so the answer here is
                // the one that ships.
                debug_assert!(false, "Flatten bypasses commit_even_layer");
                true
            }
        }
    }

    /// Commit one continuous brush field with a local, edge-continuous safety
    /// scale. Unsafe face corners back off together, and the reduction tapers
    /// through their welded neighbours without scaling unrelated regions.
    // The commit has to state the whole layer rule in one place: the factor
    // seeding, the bounded rollback waves, the reset, and the outcome
    // collection are one decision, and splitting them hides which guarantee
    // each branch keeps.
    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn commit_even_layer(
        &mut self,
        proposals: &[(u32, DVec3)],
        mode: BrushMode,
    ) {
        if proposals.is_empty() {
            return;
        }
        let mut origins = std::mem::take(&mut self.layer_origins);
        origins.clear();
        origins.extend(proposals.iter().map(|&(group, _)| self.group_v(group)));
        let mut groups = std::mem::take(&mut self.layer_groups);
        groups.clear();
        groups.extend(proposals.iter().map(|&(group, _)| group));
        // The whole snapped region participates in continuity, including
        // masked/zero-weight and brush-rim groups. They are explicit held
        // controls; only actual proposals start at full dose.
        for &group in &self.dab_groups {
            self.rollback_factor[group as usize] = 0.0;
        }
        for &(group, _) in proposals {
            self.rollback_factor[group as usize] = 1.0;
        }
        let apply = |session: &mut Self| {
            for (index, &(group, target)) in proposals.iter().enumerate() {
                let here = origins[index];
                let factor = session.rollback_factor[group as usize] as f64;
                session.write_group_position(group, here + ((target - here) * factor));
            }
        };
        let mut triangles = std::mem::take(&mut self.layer_triangles);
        self.collect_layer_triangles(&groups, &mut triangles);
        let mut unsafe_triangles = std::mem::take(&mut self.layer_unsafe);
        unsafe_triangles.clear();
        let mut controls = std::mem::take(&mut self.layer_controls);
        controls.clear();
        let mut affected = std::mem::take(&mut self.layer_affected);
        affected.clear();

        apply(self);
        self.collect_unsafe_layer_triangles(&triangles, mode, &mut unsafe_triangles);
        if unsafe_triangles.is_empty() {
            self.layer_triangles = triangles;
            self.layer_origins = origins;
            self.layer_groups = groups;
            self.layer_unsafe = unsafe_triangles;
            self.layer_controls = controls;
            self.layer_affected = affected;
            return;
        }

        diag::bump(|diag| diag.rollback_resets += 1);
        // Freeze every moving control of a rejecting face. A corner outside
        // the snapshot is already fixed, so zeroing all selected corners makes
        // a boundary face exact identity instead of leaving a kink against the
        // fixed corner. The one-eighth envelope carries that constraint into
        // the moving footprint while keeping rejection work bounded
        // independently of brush vertex count.
        for _ in 0..MAX_ROLLBACK_ITERS {
            if unsafe_triangles.is_empty() {
                break;
            }
            affected.clear();
            for &triangle in &unsafe_triangles {
                self.collect_layer_controls(triangle, &mut controls);
                for &group in &controls {
                    if self.rollback_factor[group as usize] > 0.0 {
                        self.rollback_factor[group as usize] = 0.0;
                        affected.push(group);
                    }
                }
            }
            if affected.is_empty() {
                // No control of the rejecting faces is still moving, so another
                // wave would repeat this one unchanged. Stop and let the
                // commit below decide.
                break;
            }
            self.taper_layer_factors(&affected);
            apply(self);
            self.collect_unsafe_layer_triangles(&triangles, mode, &mut unsafe_triangles);
        }
        if !unsafe_triangles.is_empty() {
            // The bounded active set ran out of waves while controls were still
            // moving, so restoring the dab's moving controls together is what
            // makes the rejecting faces exact identity again.
            for &(group, _) in proposals {
                self.rollback_factor[group as usize] = 0.0;
            }
            apply(self);
            self.collect_unsafe_layer_triangles(&triangles, mode, &mut unsafe_triangles);
        }
        debug_assert!(
            unsafe_triangles.is_empty(),
            "restoring the dab controls clears every rejecting face"
        );
        self.layer_triangles = triangles;
        self.layer_origins = origins;
        self.layer_groups = groups;
        self.layer_unsafe = unsafe_triangles;
        self.layer_controls = controls;
        self.layer_affected = affected;
    }
}

/// Cross products use exact decoded f32 positions. A rigid translation can
/// round their differences by an ulp; do not interpret that as new collapse.
pub(super) fn clay_area_roundoff(a: TriangleMeasure, b: TriangleMeasure) -> f64 {
    let mut coordinate = 0.0f64;
    let mut edge = 0.0f64;
    for tri in [a, b] {
        for i in 0..3 {
            let p = tri.points[i];
            coordinate = coordinate.max(p.x.abs()).max(p.y.abs()).max(p.z.abs());
            edge = edge.max((p - tri.points[(i + 1) % 3]).length());
        }
    }
    8.0 * f32::EPSILON as f64 * coordinate * edge
}
