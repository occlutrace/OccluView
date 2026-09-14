//! Application integration for contact readings and their viewport display.

use eframe::egui;
use occluview_core::{Scene, SceneMeshId};
use occluview_render::{ContactFieldTexels, ContactPaintSource};
use std::sync::Arc;

use super::OccluViewApp;
use crate::contact::{
    can_read_contacts, contact_job_keys, ContactLayerField, ContactPair, ContactRequest,
    ContactState, ContactStatus, CONTACT_FIELD_TEXTURE_WIDTH,
};
use crate::contact_worker::{ContactFailure, ContactJob, ContactOutcome};

impl OccluViewApp {
    /// Apply a contact action from the layer context menu.
    pub(super) fn apply_contact_context_action(
        &mut self,
        scene: &Scene,
        request: crate::layer_actions::LayerContextRequest,
    ) {
        let Some(entry) = scene.meshes().get(request.index) else {
            return;
        };
        if entry.id() != request.layer_id {
            return;
        }
        match request.action {
            crate::layer_actions::LayerContextAction::Contacts => {
                let label = crate::layers_overlay::layer_label(
                    &self.persistence.current_paths,
                    entry,
                    request.index,
                    &self.ui.locale,
                );
                if self.begin_contacts_from_layer(scene, request.layer_id) {
                    self.ui.status_message = Some(
                        self.ui
                            .locale
                            .tr_with("contact-opened", &[("label", &label)]),
                    );
                }
            }
            crate::layer_actions::LayerContextAction::HideContacts
                if self.tools.contacts.is_open() =>
            {
                let ctx = self.ui.repaint_ctx.clone();
                self.close_contacts(&ctx);
                self.ui.status_message = Some(self.ui.locale.tr("contact-closed"));
            }
            _ => {}
        }
    }

    /// Open a contact reading on `layer` against the nearest eligible layer.
    pub(super) fn begin_contacts_from_layer(&mut self, scene: &Scene, layer: SceneMeshId) -> bool {
        let Some(antagonist) = crate::contact::antagonist_for(scene, layer) else {
            // The scene may have changed since the menu was drawn.
            self.ui.status_message = Some(self.ui.locale.tr("contact-status-needs-second"));
            return false;
        };
        // Contact and deviation overlays are mutually exclusive.
        if self.tools.align.settings.show_deviation
            || !matches!(
                self.tools.align.overlay,
                super::app_align_display::AlignOverlay::Nothing
            )
        {
            self.tools.align.settings.show_deviation = false;
            self.clear_deviation_overlay();
        }
        self.tools.contacts.open(ContactPair {
            subject: layer,
            antagonist,
        });
        self.submit_contacts_job();
        true
    }

    /// Close the reading and take its marks off both scans.
    pub(super) fn close_contacts(&mut self, ctx: &egui::Context) {
        if self.tools.contacts.close().is_none() {
            return;
        }
        if let Some(worker) = self.tools.contacts.worker() {
            worker.bump_generation();
        }
        // Re-prepare layers after removing the contact fields.
        self.mark_scene_materials_changed();
        ctx.request_repaint();
    }

    /// Reconcile the open reading with the current scene.
    pub(super) fn sync_contacts_with_scene(&mut self, ctx: &egui::Context) {
        if !self.tools.contacts.is_open() {
            return;
        }
        let Some(scene) = self.document.scene.clone() else {
            self.close_contacts(ctx);
            return;
        };
        let orphaned = self.tools.contacts.forget_missing(&scene);
        if !orphaned.is_empty() {
            self.mark_scene_materials_changed();
            ctx.request_repaint();
            return;
        }
        let Some(pair) = self.tools.contacts.pair() else {
            return;
        };
        // Defer measurement until a hand drag ends.
        if self.align_hand_drag_active() {
            self.tools.contacts.hold_for_drag();
            return;
        }
        self.tools.contacts.resume_after_drag();
        // Report which side of the pair became unusable.
        if !can_read_contacts(&scene, pair.subject) {
            self.tools
                .contacts
                .status_override(ContactStatus::SubjectUnusable);
            return;
        }
        if !can_read_contacts(&scene, pair.antagonist) {
            self.tools
                .contacts
                .status_override(ContactStatus::AntagonistUnusable);
            return;
        }
        let Some(keys) = contact_job_keys(&scene, pair, self.tools.contacts.flatten_patches())
        else {
            return;
        };
        if !self.tools.contacts.needs_measurement(keys) {
            return;
        }
        // Remove stale fields before submitting the replacement measurement.
        let resubmitted = self.tools.contacts.measured_keys().is_some();
        if resubmitted {
            self.tools.contacts.drop_fields();
            self.mark_scene_materials_changed();
        }
        self.submit_contacts_job();
    }

    /// Escape closes the reading, but never steals the key from a dialog or
    /// from the Align tool's own Escape.
    pub(super) fn handle_contact_escape(&mut self, ctx: &egui::Context) {
        if !self.tools.contacts.is_open()
            || self.ui.modal_dialog_open()
            || self.tools.bridge_split_active()
        {
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.close_contacts(ctx);
        }
    }

    /// Whether a hand drag currently changes a measured pose.
    fn align_hand_drag_active(&self) -> bool {
        self.tools.align.drag.is_some()
    }

    /// Build and queue a measurement for the open pair.
    pub(super) fn submit_contacts_job(&mut self) {
        let Some(scene) = self.document.scene.clone() else {
            return;
        };
        let Some(pair) = self.tools.contacts.pair() else {
            return;
        };
        let flatten = self.tools.contacts.flatten_patches();
        let Some(keys) = contact_job_keys(&scene, pair, flatten) else {
            // The pair became invalid between reconciliation and submit.
            self.tools
                .contacts
                .status_override(ContactStatus::NeedsSecond);
            return;
        };
        let (Some(subject), Some(antagonist)) = (
            scene
                .meshes()
                .iter()
                .find(|entry| entry.id() == pair.subject),
            scene
                .meshes()
                .iter()
                .find(|entry| entry.id() == pair.antagonist),
        ) else {
            return;
        };

        // Reuse the prepared arrays for this geometry and pose.
        let geometry = &mut self.tools.contacts.geometry;
        let subject_positions = geometry.world_positions(subject);
        let subject_indices = geometry.indices(subject);
        let antagonist_positions = geometry.world_positions(antagonist);
        let antagonist_indices = geometry.indices(antagonist);

        let status = if self.tools.contacts.measured_keys().is_some() {
            ContactStatus::Remeasuring
        } else {
            ContactStatus::Measuring
        };
        let worker = self.tools.contacts.worker_mut();
        let generation = worker.generation();
        let submitted = worker.submit(ContactJob {
            generation,
            request_id: 0,
            keys,
            subject_positions,
            subject_indices,
            antagonist_positions,
            antagonist_indices,
            settings: occluview_contact::ContactSettings {
                search_radius_mm: occluview_contact::SEARCH_RADIUS_MM,
                flatten_patches: flatten,
            },
        });
        let Some(id) = submitted else {
            tracing::warn!("contact reading could not be submitted to its worker");
            self.tools.contacts.mark_unavailable(keys);
            return;
        };
        self.tools
            .contacts
            .mark_submitted(ContactRequest { id, keys, pair }, status);
    }

    /// Check that a completion still matches the live scene.
    fn completion_still_describes_the_scene(&self, request: &ContactRequest) -> bool {
        let Some(scene) = self.document.scene.as_ref() else {
            return false;
        };
        let flatten = request.keys.flatten_patches;
        contact_job_keys(scene, request.pair, flatten).is_some_and(|live| live == request.keys)
    }

    /// Take finished measurements and put them on the scans.
    pub(super) fn drain_contacts_worker(&mut self, ctx: &egui::Context) {
        let Some(worker) = self.tools.contacts.worker() else {
            return;
        };
        let completions = worker.drain();
        if completions.is_empty() {
            return;
        }
        let mut accepted = false;
        for completion in completions {
            let Some(request) = self
                .tools
                .contacts
                .matching_request(completion.request_id, completion.keys)
            else {
                continue;
            };
            if !self.completion_still_describes_the_scene(&request) {
                self.tools
                    .contacts
                    .mark_answer_dropped(request.id, request.keys);
                accepted = true;
                continue;
            }
            match completion.outcome {
                ContactOutcome::Measured {
                    subject_signed_mm,
                    antagonist_signed_mm,
                    stats,
                    diagnostics,
                } => {
                    tracing::debug!(
                        subject_verts = diagnostics.subject_verts,
                        antagonist_verts = diagnostics.antagonist_verts,
                        subject_measured = diagnostics.subject_measured,
                        subject_penetrating = diagnostics.subject_penetrating,
                        worker_ms = diagnostics.worker_ms,
                        contacts = stats.contacts,
                        "contact field applied"
                    );
                    let Some((subject_field, antagonist_field)) = pack_fields(
                        &mut self.tools.contacts,
                        request.pair,
                        subject_signed_mm,
                        antagonist_signed_mm,
                    ) else {
                        self.tools.contacts.mark_failed(
                            request.id,
                            request.keys,
                            ContactFailure::Worker,
                        );
                        accepted = true;
                        continue;
                    };
                    if !self.tools.contacts.store_measured(
                        request,
                        subject_field,
                        antagonist_field,
                        stats,
                    ) {
                        continue;
                    }
                    accepted = true;
                    if diagnostics.subject_measured == 0 && diagnostics.antagonist_measured == 0 {
                        self.tools
                            .contacts
                            .status_override(ContactStatus::NoOverlap);
                    }
                }
                ContactOutcome::Failed(failure) => {
                    accepted |= self
                        .tools
                        .contacts
                        .mark_failed(request.id, request.keys, failure);
                }
            }
        }
        if accepted {
            self.mark_scene_materials_changed();
            ctx.request_repaint();
        }
    }

    /// Build GPU sources for the current scene, including contact paint.
    pub(super) fn prepared_scene_sources<'a>(
        &self,
        scene: &'a Scene,
    ) -> Vec<occluview_render::PreparedSceneSource<'a>> {
        let scale = self.tools.contacts.scale();
        scene
            .meshes()
            .iter()
            .map(|entry| {
                let field = self.tools.contacts.field_for(entry.id());
                occluview_render::PreparedSceneSource {
                    mesh: &entry.mesh,
                    uniform: super::app_render_contact::scene_mesh_uniform_with_contacts(
                        entry,
                        field.map(|_| &scale),
                        field_width(field),
                    ),
                    visible: entry.visible,
                    wireframe: entry.wireframe,
                    contact: field.map(contact_paint),
                }
            })
            .collect()
    }

    /// Build per-frame material and visibility updates for the current scene.
    pub(super) fn prepared_scene_updates(
        &self,
        scene: &Scene,
    ) -> Vec<occluview_render::PreparedSceneUpdate> {
        let scale = self.tools.contacts.scale();
        scene
            .meshes()
            .iter()
            .map(|entry| {
                let field = self.tools.contacts.field_for(entry.id());
                occluview_render::PreparedSceneUpdate {
                    topology: occluview_render::PreparedSceneTopology::from_mesh(&entry.mesh),
                    uniform: super::app_render_contact::scene_mesh_uniform_with_contacts(
                        entry,
                        field.map(|_| &scale),
                        field_width(field),
                    ),
                    visible: entry.visible,
                    wireframe: entry.wireframe,
                    contact: field.map(contact_paint),
                }
            })
            .collect()
    }
}

/// Pack both sides' fields and hand back the two ready-to-paint layers.
fn pack_fields(
    state: &mut ContactState,
    pair: ContactPair,
    subject_signed_mm: Vec<f32>,
    antagonist_signed_mm: Vec<f32>,
) -> Option<(ContactLayerField, ContactLayerField)> {
    let subject_texels = pack(&subject_signed_mm)?;
    let antagonist_texels = pack(&antagonist_signed_mm)?;
    let subject_revision = state.take_revision();
    let antagonist_revision = state.take_revision();
    Some((
        ContactLayerField {
            layer: pair.subject,
            signed_mm: Arc::new(subject_signed_mm),
            texels: subject_texels,
            revision: subject_revision,
        },
        ContactLayerField {
            layer: pair.antagonist,
            signed_mm: Arc::new(antagonist_signed_mm),
            texels: antagonist_texels,
            revision: antagonist_revision,
        },
    ))
}

/// The GPU paint for one layer's finished field.
fn contact_paint(field: &ContactLayerField) -> ContactPaintSource {
    ContactPaintSource::new(Arc::clone(&field.texels), field.revision)
}

/// Return the row length stored in the packed field.
fn field_width(field: Option<&ContactLayerField>) -> u32 {
    field.map_or(CONTACT_FIELD_TEXTURE_WIDTH, |field| field.texels.width)
}

/// Pack one field using the contact crate's sentinel and device texture limit.
fn pack(values: &[f32]) -> Option<Arc<ContactFieldTexels>> {
    let width = crate::contact::contact_field_width(
        values.len(),
        crate::app_bootstrap::MAX_RENDER_TEXTURE_DIMENSION,
    )?;
    let packed = occluview_contact::pack_field_texels(values, width);
    ContactFieldTexels::new(packed.rgba, packed.width, packed.height).map(Arc::new)
}
