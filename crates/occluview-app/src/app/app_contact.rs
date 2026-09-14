//! The occlusal contact reading: opening it, keeping it honest, and its panel.
//!
//! The panel carries two controls that matter — which reading is shown, and the
//! depth that reads as fully loaded — and everything else on it is there to say
//! what the colours mean, because a heat map without a legend is a picture
//! rather than a measurement.
//!
//! NOTHING HERE RE-MEASURES FOR A DISPLAY CHANGE. Moving the slider rewrites the
//! stop table in the per-mesh uniform and repaints; only geometry, pose, or the
//! patch rule make the worker run again. That is what makes the slider worth
//! dragging: an operator finds the boundary between a light contact and a heavy
//! one by moving it and watching, not by typing numbers and waiting.

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
    // -------------------------------------------------------------- menu verbs

    /// Run one contact action raised by the layer context menu.
    ///
    /// Both verbs are about the SUBJECT of a reading: `Contacts` opens one on
    /// the clicked layer (taking down whatever the previous reading painted),
    /// and `HideContacts` closes it. A reading is a property of the pair, so
    /// hiding it from either participant closes the same reading.
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

    // ---------------------------------------------------------------- opening

    /// Open a contact reading on `layer` against whatever it bites.
    ///
    /// Returns whether a reading was opened, so the caller can decide whether
    /// the click consumed the menu.
    pub(super) fn begin_contacts_from_layer(&mut self, scene: &Scene, layer: SceneMeshId) -> bool {
        let Some(antagonist) = crate::contact::antagonist_for(scene, layer) else {
            // The menu disables the entry on a case that cannot support a
            // reading, so reaching here means the scene changed between the menu
            // being drawn and the click landing. Say why rather than opening a
            // panel that can only show an empty map.
            self.ui.status_message = Some(self.ui.locale.tr("contact-status-needs-second"));
            return false;
        };
        // Whatever was wearing marks stops wearing them the moment the reading
        // moves, or the operator is looking at two maps and one legend. That
        // includes the align heatmap: it is a *different* measurement of the
        // same two scans, painted through the same measured-map treatment, and
        // a contact legend beside a deviation ramp describes neither.
        if self.tools.align.settings.show_deviation
            || !matches!(
                self.tools.align.overlay,
                super::app_align_display::AlignOverlay::Nothing
            )
        {
            self.tools.align.settings.show_deviation = false;
            // This already invalidates the scene for the layers it restores;
            // a second invalidation here would repeat that work for nothing.
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
        // The fields reached the GPU through the prepared-scene path, so the
        // scene has to be re-prepared for the layers to come back in their own
        // colours.
        self.mark_scene_materials_changed();
        ctx.request_repaint();
    }

    /// Drop a reading whose scans are no longer both in the scene, or whose
    /// surfaces have moved since they were measured.
    ///
    /// Called from the frame loop rather than from every edit that could
    /// invalidate it: a layer can leave through a removal, an undo, a crop or a
    /// separate, and a reading left pointing at a missing scan would go on
    /// showing marks measured against something that is not there. The same loop
    /// notices the operator moving a scan, which changes every distance in the
    /// map and must re-measure rather than leave a stale one under a live
    /// legend.
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
        // A hand drag moves a scan every frame, so the measurement's keys change
        // every frame. Measuring on each one would restart a full surface index
        // build per frame — work that is thrown away before it finishes, on a
        // path that exists to end in a new pose anyway. The reading is held
        // until the drag ends, then measured once against where the scan landed.
        if self.align_hand_drag_active() {
            self.tools.contacts.hold_for_drag();
            return;
        }
        self.tools.contacts.resume_after_drag();
        // Which of the two scans stopped being readable decides what the panel
        // says: a sentence about the other one sends the operator to unhide the
        // wrong layer.
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
        // The scene moved under a finished reading: take the old marks down
        // before the new ones exist, so the operator never reads a stale map as
        // the current one.
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

    /// Whether the Align tool is dragging a scan by hand right now.
    ///
    /// A contact reading is measured against layer poses, and a hand drag
    /// rewrites one every frame, so this is the one interaction that has to hold
    /// the reading back rather than chase it.
    fn align_hand_drag_active(&self) -> bool {
        self.tools.align.drag.is_some()
    }

    // --------------------------------------------------------------- computing

    /// Build and queue one measurement for the open pair.
    ///
    /// Visible to the panel module because the patch rule is the one control
    /// that is an input to the measurement.
    pub(super) fn submit_contacts_job(&mut self) {
        let Some(scene) = self.document.scene.clone() else {
            return;
        };
        let Some(pair) = self.tools.contacts.pair() else {
            return;
        };
        let flatten = self.tools.contacts.flatten_patches();
        let Some(keys) = contact_job_keys(&scene, pair, flatten) else {
            // The pair names a layer that is not in the scene. That is a
            // different condition from a surface the compute cannot use, and it
            // is the one the frame loop's `forget_missing` normally catches
            // first — this is the retry path arriving between the two.
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

        // Handed over by `Arc`: the arrays are built once per geometry and pose,
        // not once per submit, and a reading is re-submitted whenever the
        // operator nudges a scan.
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
            // No thread, or a queue the worker cannot use. The reading records a
            // refusal keyed on the same inputs a real one would have used, so it
            // shows Failed once and the frame loop does not re-queue a job that
            // cannot run.
            tracing::warn!("contact reading could not be submitted to its worker");
            self.tools.contacts.mark_unavailable(keys);
            return;
        };
        self.tools
            .contacts
            .mark_submitted(ContactRequest { id, keys, pair }, status);
    }

    /// Whether a finished measurement still describes the scene on screen.
    ///
    /// The request identity says which submission an answer belongs to. It does
    /// not say the surfaces have not moved since: a hand drag rewrites a pose
    /// every frame, and the reading stays submitted across it. The keys are
    /// recomputed from the live scene here, before any field, statistic, or
    /// status is touched, so an answer measured against a pose that is no longer
    /// on screen is dropped rather than painted.
    ///
    /// `false` when the pair or the scenes it names cannot be resolved: an
    /// answer about a layer that has left the scene describes nothing on screen.
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
                // The answer to a measurement the operator moved past: it may
                // not touch the fields, the statistics, the status, or the
                // in-flight record.
                continue;
            };
            // The identity says which submission this answers. It does not say
            // the scene still looks the way that submission was built from: a
            // drag rewrites a pose every frame while the reading is held, and
            // the request stays in flight through all of it. The keys of the
            // scene as it is now are the second half of the question, and they
            // are asked before anything is stored.
            if !self.completion_still_describes_the_scene(&request) {
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
                    // The pair is further apart than the reading reaches only
                    // when NOTHING was measured. An empty contact AREA is a
                    // different fact: under a paint band narrower than the
                    // search radius the Approach map is full while no patch
                    // passes the touch gate, and a sentence claiming the
                    // surfaces never met would be false next to that map.
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

    // ---------------------------------------------------------------- painting

    /// The GPU sources for the current scene, contact paint included.
    ///
    /// One place decides how a layer reaches the viewport: whether it shows a
    /// measurement, and which ramp that measurement is read through. The ramp is
    /// rebuilt from the live scale every call — it is sixteen `vec4`s, and
    /// rebuilding it is cheaper than tracking when it went stale.
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

    /// The per-frame material/visibility updates for the current scene, contact
    /// paint included.
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
                    // The revision travels with the bytes: the renderer
                    // re-uploads and rebuilds this layer's bind group exactly
                    // when it changes, so a rebuilt scene that still carries the
                    // same field costs nothing.
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

/// The row length the field was actually packed with.
///
/// Read from the packed texture rather than assumed to be the preferred 1024:
/// the shader turns a vertex index into a texel with this number, so a field
/// packed wider to fit the device limit would decode every vertex after the
/// first row from the wrong texel. A layer with no field reports the preferred
/// width; nothing reads it there, because `contact_map` is zero.
fn field_width(field: Option<&ContactLayerField>) -> u32 {
    field.map_or(CONTACT_FIELD_TEXTURE_WIDTH, |field| field.texels.width)
}

/// Pack one field through the contact crate's own rule — the crate owns the
/// "nothing measured here" sentinel, so the bytes the GPU reads and the rule
/// the hover readout applies come from one place.
///
/// The row length is derived from the vertex count and the texture dimension
/// this app requests of every device, not taken from the preferred constant: a
/// field is `ceil(n / width)` rows tall, so a scan large enough would overflow
/// the limit and the texture could not be created at all — the reading would
/// fail on exactly the case with the most to read.
fn pack(values: &[f32]) -> Option<Arc<ContactFieldTexels>> {
    let width = crate::contact::contact_field_width(
        values.len(),
        crate::app_bootstrap::MAX_RENDER_TEXTURE_DIMENSION,
    )?;
    let packed = occluview_contact::pack_field_texels(values, width);
    ContactFieldTexels::new(packed.rgba, packed.width, packed.height).map(Arc::new)
}
