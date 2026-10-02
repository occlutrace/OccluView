//! Candidate preview and operator acceptance own different authority.
use super::super::SceneContext;
use crate::align::align_state::{AcceptedAlignment, AlignmentReview, ReviewKey};
use occluview_align::AlignmentSearchResult;
use occluview_core::SceneMeshId;

impl SceneContext<'_> {
    /// Capture all scene inputs that can invalidate candidate acceptance.
    pub(in crate::app) fn current_alignment_key(&self) -> Option<ReviewKey> {
        let scene = self.document.scene.as_ref()?;
        let roles = [
            self.tools.align.tool.moving_layer()?,
            self.tools.align.tool.fixed_layer()?,
        ];
        if roles[0] == roles[1] {
            return None;
        }
        let find = |id| scene.meshes().iter().find(|m| m.id() == id);
        let moving = find(roles[0])?;
        let fixed = find(roles[1])?;
        Some(ReviewKey {
            generation: self
                .tools
                .align
                .worker
                .as_ref()
                .map_or(0, crate::align::align_worker::AlignWorker::generation),
            content_revision: self.document.content_revision,
            roles,
            geometry: [moving.mesh.geometry_id(), fixed.mesh.geometry_id()],
            transforms: [moving.transform, fixed.transform],
            visible: [moving.visible, fixed.visible],
            mask_revision: self.tools.align.markings.revision(),
            matching: [
                self.tools.align.settings.influence_radius_mm.to_bits(),
                self.tools.align.settings.matching_ratio.to_bits(),
            ],
            orientation: self.tools.align.settings.orientation,
        })
    }

    /// Install evidence without touching the scene, history, or map authority.
    pub(in crate::app) fn install_alignment_review(
        &mut self,
        generation: u64,
        candidates: AlignmentSearchResult,
    ) {
        let key = self.tools.align.pending_review.take().or({
            // Test completions can isolate delivery without launching a solver.
            #[cfg(test)]
            {
                self.current_alignment_key()
            }
            #[cfg(not(test))]
            {
                None
            }
        });
        let Some(key) = key else {
            return;
        };
        if key.generation != generation
            || self.current_alignment_key().as_ref() != Some(&key)
            || !key.visible.into_iter().all(|v| v)
            || !(1..=5).contains(&candidates.candidates.len())
            || candidates.candidates.iter().any(|c| !proper_pose(c.pose))
        {
            self.invalidate_alignment_review();
            return;
        }
        self.tools.align.review = Some(AlignmentReview {
            key,
            candidates,
            selected: 0,
            preview_enabled: true,
        });
        self.tools.align.status = Some(
            self.ui
                .locale
                .tr(crate::i18n::message_id!("align-review-ready")),
        );
        if let Some(reason) = self
            .tools
            .align
            .review
            .as_ref()
            .and_then(|r| r.candidates.candidates.first())
            .and_then(|c| {
                c.reasons.iter().find_map(|reason| match reason {
                    occluview_align::EvidenceReason::LegacyRejected(reason) => Some(*reason),
                    _ => None,
                })
            })
        {
            let (key, a, b) = super::results::fit_rejection_parts(reason);
            self.tools.align.status = Some(
                self.ui
                    .locale
                    .tr_with(key, &[("a", a.as_str()), ("b", b.as_str())]),
            );
        }
        self.render.invalidation.scene_geometry_changed();
        self.ui.repaint_ctx.request_repaint();
    }

    /// Change the selected render-only correction. Cycling never edits a scene.
    pub(in crate::app) fn cycle_alignment_candidate(&mut self, forward: bool) {
        let Some(review) = self.tools.align.review.as_mut() else {
            return;
        };
        let count = review.candidates.candidates.len();
        if count == 0 {
            return;
        }
        review.selected = if forward {
            (review.selected + 1) % count
        } else {
            (review.selected + count - 1) % count
        };
        self.render.invalidation.scene_geometry_changed();
        self.ui.repaint_ctx.request_repaint();
    }

    /// Toggle a render override, leaving authored transforms and CPU meshes intact.
    pub(in crate::app) fn toggle_alignment_preview(&mut self) {
        if let Some(review) = self.tools.align.review.as_mut() {
            review.preview_enabled = !review.preview_enabled;
            self.render.invalidation.scene_geometry_changed();
            self.ui.repaint_ctx.request_repaint();
        }
    }

    /// Current render transform; a stale or overflowing correction is ignored.
    pub(in crate::app) fn alignment_preview_transform(
        &self,
        layer: SceneMeshId,
    ) -> Option<glam::Affine3A> {
        let review = self.tools.align.review.as_ref()?;
        if !review.preview_enabled
            || layer != review.key.roles[0]
            || self.current_alignment_key().as_ref() != Some(&review.key)
        {
            return None;
        }
        let pose = review.candidates.candidates.get(review.selected)?.pose;
        if !proper_pose(pose) {
            return None;
        }
        let absolute = pose.to_affine() * review.key.transforms[0];
        finite_affine(absolute).then_some(absolute)
    }

    /// Revalidate and commit exactly one absolute affine; confidence is preserved.
    pub(in crate::app) fn accept_alignment_candidate(&mut self) -> bool {
        let Some(review) = self.tools.align.review.as_ref() else {
            return false;
        };
        if self.current_alignment_key().as_ref() != Some(&review.key)
            || !review.key.visible.into_iter().all(|v| v)
        {
            self.invalidate_alignment_review();
            return false;
        }
        let Some(candidate) = review.candidates.candidates.get(review.selected) else {
            return false;
        };
        if !proper_pose(candidate.pose) {
            self.invalidate_alignment_review();
            return false;
        }
        let absolute = candidate.pose.to_affine() * review.key.transforms[0];
        if !finite_affine(absolute) {
            self.invalidate_alignment_review();
            return false;
        }
        let id = candidate.id;
        let confidence = candidate.confidence;
        let evidence = candidate.evidence.clone();
        if !self.commit_align_affine(absolute) {
            self.tools.align.status = Some(
                self.ui
                    .locale
                    .tr(crate::i18n::message_id!("align-status-pose-refused")),
            );
            return false;
        }
        self.tools.align.review = None;
        self.tools.align.accepted = Some(AcceptedAlignment {
            key: self.current_alignment_key(),
            candidate_id: id,
            confidence,
            evidence,
        });
        self.tools.align.settings.show_deviation = false;
        self.tools.align.status = Some(
            self.ui
                .locale
                .tr(crate::i18n::message_id!("align-review-accepted")),
        );
        self.render.invalidation.scene_geometry_changed();
        self.ui.repaint_ctx.request_repaint();
        true
    }

    /// Complete revision equality is required before distance measurement.
    pub(in crate::app) fn alignment_measurement_ready(&self) -> bool {
        let Some(accepted) = self.tools.align.accepted.as_ref() else {
            return false;
        };
        tracing::trace!(candidate_family = accepted.candidate_id.family, candidate = accepted.candidate_id.proposal,
            confidence = ?accepted.confidence, verification_complete = accepted.evidence.verification_complete, "alignment measurement authority");
        if let Some(key) = accepted.key.as_ref() {
            self.current_alignment_key().as_ref() == Some(key)
        } else {
            cfg!(test)
        }
    }

    fn invalidate_alignment_review(&mut self) {
        self.tools.align.review = None;
        self.tools.align.pending_review = None;
        self.tools.align.status = Some(
            self.ui
                .locale
                .tr(crate::i18n::message_id!("align-review-invalidated")),
        );
        self.render.invalidation.scene_geometry_changed();
        self.ui.repaint_ctx.request_repaint();
    }
}

fn proper_pose(pose: occluview_align::Rigid) -> bool {
    pose.is_finite() && (pose.rotation.length_squared() - 1.).abs() <= 2e-12
}
fn finite_affine(pose: glam::Affine3A) -> bool {
    pose.to_cols_array().into_iter().all(f32::is_finite)
}
