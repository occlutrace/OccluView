//! Applying a finished fit, and what a landed fit is allowed to authorize.
//!
//! A fit is computed from a snapshot of the pair. It is applied only while the
//! scene still matches that snapshot, and a heatmap is authorized only while
//! the scene still matches the state the fit left behind.
use super::super::SceneContext;
use crate::align::align_state::FitKey;
#[cfg(test)]
use eframe::egui;
use occluview_align::Rigid;

impl SceneContext<'_> {
    /// Everything about the pair and the matching settings that a fit depends
    /// on. `None` until both roles name live, distinct layers.
    pub(in crate::app) fn current_fit_key(&self) -> Option<FitKey> {
        let scene = self.document.scene.as_ref()?;
        let roles = [
            self.tools.align.tool.moving_layer()?,
            self.tools.align.tool.fixed_layer()?,
        ];
        if roles[0] == roles[1] {
            return None;
        }
        let moving = super::layer_of(scene, roles[0])?;
        let fixed = super::layer_of(scene, roles[1])?;
        Some(FitKey {
            content_revision: self.document.content_revision,
            roles,
            geometry: [moving.mesh.geometry_id(), fixed.mesh.geometry_id()],
            transforms: [moving.transform, fixed.transform],
            visible: [moving.visible, fixed.visible],
            mask_revision: self.tools.align.markings.revision(),
            influence_radius: self.tools.align.settings.influence_radius_mm.to_bits(),
            orientation: self.tools.align.settings.orientation,
        })
    }

    /// Move the moving scan by a fit's correction, as one undo step.
    ///
    /// The correction is relative to the placement the job was submitted with,
    /// so it is composed with that placement exactly once, and only while the
    /// pair is still what the job saw. Returns whether the scan now sits in
    /// the fitted pose; a refusal leaves the scene alone and says why.
    pub(in crate::app) fn apply_fit_correction(&mut self, correction: Rigid) -> bool {
        let Some(key) = self.tools.align.pending_fit.take() else {
            return false;
        };
        let proper = correction.is_finite()
            && (correction.rotation.length_squared() - 1.).abs() <= UNIT_ROTATION_TOLERANCE;
        let absolute = correction.to_affine() * key.transforms[0];
        let refusal = if self.current_fit_key().as_ref() != Some(&key) {
            Some(crate::i18n::message_id!("align-fit-outdated"))
        } else if !proper || !absolute.is_finite() {
            Some(crate::i18n::message_id!("align-reject-nonfinite"))
        } else if !self.commit_align_affine(absolute) {
            Some(crate::i18n::message_id!("align-status-pose-refused"))
        } else {
            None
        };
        if let Some(refusal) = refusal {
            self.tools.align.status = Some(self.ui.locale.tr(refusal));
        }
        refusal.is_none()
    }

    /// Whether the pair still sits exactly where the last Best fit left it.
    pub(in crate::app) fn alignment_measurement_ready(&self) -> bool {
        self.tools
            .align
            .fitted
            .as_ref()
            .is_some_and(|fitted| self.current_fit_key().as_ref() == Some(&fitted.key))
    }

    /// Deliver `outcome` as the result of a job submitted from the pair as it
    /// stands now.
    #[cfg(test)]
    pub(in crate::app) fn land_fit_for_tests(
        &mut self,
        outcome: crate::align::align_worker::AlignOutcome,
    ) {
        self.tools.align.pending_fit = self.current_fit_key();
        let worker = self.align_worker_mut();
        worker.publish_for_tests(worker.generation(), outcome);
        self.drain_align_worker(&egui::Context::default());
    }

    /// Declare the pair fitted where it stands, the way a landed Best fit does.
    #[cfg(test)]
    pub(in crate::app) fn mark_fitted_for_tests(&mut self) {
        self.tools.align.fitted =
            self.current_fit_key()
                .map(|key| crate::align::align_state::FittedAlignment {
                    key,
                    confidence: occluview_align::Confidence::Weak,
                });
    }
}

/// How far a rotation's squared length may sit from one and still be applied.
const UNIT_ROTATION_TOLERANCE: f64 = 2e-12;
