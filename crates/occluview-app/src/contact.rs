//! Contact state, request identity, and display helpers.

use glam::Vec3;
use occluview_align::Soup;
use occluview_contact::{
    is_no_contact, ContactLaw, ContactReading, ContactScale, ContactStats, CLINICAL, LOAD_MAX_MM,
    LOAD_MIN_MM, TIGHTNESS,
};
use occluview_core::{Scene, SceneMesh, SceneMeshId};
use occluview_render::ContactFieldTexels;
use std::sync::Arc;

use crate::align_geometry::{transform_key, AlignGeometry};
use crate::contact_worker::{ContactFailure, ContactJobKeys, ContactWorker};

/// Which contact display law is active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ContactMode {
    /// Where the surfaces meet, coloured by how hard. The default.
    #[default]
    Marks,
    /// How close the antagonist is everywhere, load included.
    Approach,
}

impl ContactMode {
    /// The colour law this reading paints with.
    pub(crate) fn law(self) -> &'static ContactLaw {
        match self {
            Self::Marks => &TIGHTNESS,
            Self::Approach => &CLINICAL,
        }
    }

    /// Catalog key for the label on the mode button.
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Marks => "contact-mode-marks",
            Self::Approach => "contact-mode-approach",
        }
    }

    /// Catalog key for the one line explaining what the mode is for.
    pub(crate) fn hint_key(self) -> &'static str {
        match self {
            Self::Marks => "contact-mode-marks-hint",
            Self::Approach => "contact-mode-approach-hint",
        }
    }

    /// Both modes, in the order the panel offers them.
    pub(crate) const ALL: [Self; 2] = [Self::Marks, Self::Approach];
}

/// Which two scans a contact reading runs between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContactPair {
    /// The scan wearing the marks.
    pub(crate) subject: SceneMeshId,
    /// The scan it is measured against.
    pub(crate) antagonist: SceneMeshId,
}

/// A submitted contact measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContactRequest {
    pub(crate) id: u64,
    pub(crate) keys: ContactJobKeys,
    pub(crate) pair: ContactPair,
}

/// What the panel is currently saying. Typed, so the copy lives in the catalog
/// and the worker never renders a sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContactStatus {
    /// A job is in flight.
    Measuring,
    /// The scene changed under a finished reading and it is being re-measured.
    Remeasuring,
    /// Nothing in the scene can be measured against.
    NeedsSecond,
    /// The scan wearing the marks is hidden or is not a surface any more, so
    /// the reading is on hold.
    SubjectUnusable,
    /// The surface the reading runs against is hidden or is not a surface.
    AntagonistUnusable,
    /// Every vertex was outside the search radius: the two surfaces are simply
    /// further apart than a contact reading reaches.
    NoOverlap,
    /// The job produced nothing trustworthy.
    Failed(ContactFailure),
}

impl ContactStatus {
    /// Catalog key for this status.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Measuring => "contact-status-measuring",
            Self::Remeasuring => "contact-status-remeasuring",
            Self::NeedsSecond => "contact-status-needs-second",
            Self::SubjectUnusable => "contact-status-subject-unusable",
            Self::AntagonistUnusable => "contact-status-antagonist-unusable",
            Self::NoOverlap => "contact-status-no-overlap",
            Self::Failed(ContactFailure::NoSurface) => "contact-status-no-surface",
            Self::Failed(ContactFailure::Worker) => "contact-status-worker-failed",
        }
    }
}

/// One layer's measured values and packed viewport texture.
pub(crate) struct ContactLayerField {
    /// The layer this field belongs to.
    pub(crate) layer: SceneMeshId,
    /// Signed millimetres per vertex, in the layer's own vertex order.
    pub(crate) signed_mm: Arc<Vec<f32>>,
    /// The same field packed for the GPU.
    pub(crate) texels: Arc<ContactFieldTexels>,
    /// Monotone identity of `texels`, so the renderer rebuilds the layer's bind
    /// group exactly when the field changes and never otherwise.
    pub(crate) revision: u64,
}

/// The contact view: what it is showing, and how it is set.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct ContactState {
    pair: Option<ContactPair>,
    mode: ContactMode,
    load_mm: f64,
    /// Whether each connected penetration patch is collapsed to its peak.
    flatten_patches: bool,
    /// Whether the status on screen explains an unusable side rather than a
    /// measurement. Cleared as soon as both sides are readable again.
    unusable_override: bool,
    status: Option<ContactStatus>,
    /// The fields on screen, at most one per layer and never more than the two
    /// participants.
    fields: Vec<ContactLayerField>,
    /// The keys the on-screen fields were measured from.
    measured: Option<ContactJobKeys>,
    in_flight: Option<ContactRequest>,
    /// The keys of the last failed request.
    failed: Option<ContactJobKeys>,
    /// Whether measurement is held while a scan is dragged by hand.
    held: bool,
    /// What the last measurement found, for the panel's numbers.
    stats: Option<ContactStats>,
    worker: Option<ContactWorker>,
    /// Prepared worker arrays keyed by geometry and pose.
    pub(crate) geometry: AlignGeometry,
    next_revision: u64,
    /// Whether the details popover is showing beside the bar.
    details_open: bool,
}

impl Default for ContactState {
    fn default() -> Self {
        Self {
            pair: None,
            mode: ContactMode::Marks,
            load_mm: TIGHTNESS.load_mm,
            flatten_patches: false,
            unusable_override: false,
            status: None,
            fields: Vec::new(),
            measured: None,
            in_flight: None,
            failed: None,
            held: false,
            stats: None,
            worker: None,
            geometry: AlignGeometry::default(),
            next_revision: 0,
            details_open: false,
        }
    }
}

impl ContactState {
    /// Which two scans the reading runs between, or `None` when it is closed.
    pub(crate) fn pair(&self) -> Option<ContactPair> {
        self.pair
    }

    /// Whether a reading is on screen.
    pub(crate) fn is_open(&self) -> bool {
        self.pair.is_some()
    }

    /// The reading currently shown.
    pub(crate) fn mode(&self) -> ContactMode {
        self.mode
    }

    /// The depth the ramp reads fully loaded at, in millimetres.
    pub(crate) fn load_mm(&self) -> f64 {
        self.load_mm
    }

    /// Whether penetration patches are collapsed to their peak.
    pub(crate) fn flatten_patches(&self) -> bool {
        self.flatten_patches
    }

    /// The sentence the panel is showing.
    pub(crate) fn status(&self) -> Option<ContactStatus> {
        self.status
    }

    /// The numbers the last measurement produced.
    pub(crate) fn stats(&self) -> Option<ContactStats> {
        self.stats
    }

    /// The display scale for the active law and threshold.
    pub(crate) fn scale(&self) -> ContactScale {
        ContactScale::new(self.mode.law(), self.load_mm)
    }

    /// One layer's finished field, if it has one.
    pub(crate) fn field_for(&self, layer: SceneMeshId) -> Option<&ContactLayerField> {
        self.fields.iter().find(|field| field.layer == layer)
    }

    /// The fields currently on screen.
    pub(crate) fn fields(&self) -> &[ContactLayerField] {
        &self.fields
    }

    /// Open a reading on `pair` and clear its previous result.
    pub(crate) fn open(&mut self, pair: ContactPair) {
        self.pair = Some(pair);
        self.load_mm = self.mode.law().load_mm;
        self.fields.clear();
        self.measured = None;
        self.in_flight = None;
        self.failed = None;
        self.stats = None;
        self.status = Some(ContactStatus::Measuring);
    }

    /// Close the reading, returning the pair that was on screen.
    pub(crate) fn close(&mut self) -> Option<ContactPair> {
        self.fields.clear();
        self.measured = None;
        self.in_flight = None;
        self.failed = None;
        self.stats = None;
        self.status = None;
        self.pair.take()
    }

    /// Replace the panel status without changing the reading.
    pub(crate) fn status_override(&mut self, status: ContactStatus) {
        self.status = Some(status);
        self.unusable_override = true;
    }

    /// Drop an "unusable" override once both sides can be read again.
    ///
    /// The override exists to explain why nothing is being measured, and it is
    /// set on a frame where a side was hidden. Nothing else clears it, so
    /// without this the bar keeps saying a scan cannot be measured after the
    /// operator shows it again — and when it replaced a failure, the retry chip
    /// it hid never comes back.
    ///
    /// Returns whether the status changed, so the caller can repaint.
    pub(crate) fn clear_unusable_override(&mut self) -> bool {
        if !self.unusable_override {
            return false;
        }
        self.unusable_override = false;
        if matches!(
            self.status,
            Some(ContactStatus::SubjectUnusable | ContactStatus::AntagonistUnusable)
        ) {
            // "Re-measuring…" is shown only when a job will follow. Hiding and
            // showing a layer changes neither the geometry id nor the pose, so
            // the job keys are unchanged and `needs_measurement` alone stays
            // false. The fields are dropped with the override, so the next frame
            // resubmits and the spinner has a job behind it.
            if self.fields.is_empty() {
                self.status = None;
            } else {
                self.fields.clear();
                self.measured = None;
                self.status = Some(ContactStatus::Remeasuring);
            }
        }
        true
    }

    /// Whether an "unusable" override is currently steering the status.
    #[cfg(test)]
    pub(crate) fn has_unusable_override(&self) -> bool {
        self.unusable_override
    }

    /// Set the display threshold, returning whether it changed.
    pub(crate) fn set_load_mm(&mut self, load_mm: f64) -> bool {
        if !load_mm.is_finite() {
            return false;
        }
        let next = load_mm.clamp(LOAD_MIN_MM, LOAD_MAX_MM);
        let moved = (next - self.load_mm).abs() > f64::EPSILON;
        self.load_mm = next;
        moved
    }

    /// Switch display laws, returning whether the mode changed.
    pub(crate) fn set_mode(&mut self, mode: ContactMode) -> bool {
        if self.mode == mode {
            return false;
        }
        self.mode = mode;
        self.load_mm = mode.law().load_mm;
        true
    }

    /// Read against a different layer. Returns whether the pair moved.
    ///
    /// The measurement on screen describes the previous antagonist, so it is
    /// dropped: the frame loop re-submits for the new pair.
    pub(crate) fn set_antagonist(&mut self, antagonist: SceneMeshId) -> bool {
        let Some(pair) = self.pair.as_mut() else {
            return false;
        };
        if pair.antagonist == antagonist {
            return false;
        }
        pair.antagonist = antagonist;
        self.drop_fields();
        self.forget_failure();
        self.in_flight = None;
        self.status = Some(ContactStatus::Measuring);
        true
    }

    /// Toggle the patch collapse. Returns whether it changed.
    pub(crate) fn set_flatten_patches(&mut self, flatten: bool) -> bool {
        if self.flatten_patches == flatten {
            return false;
        }
        self.flatten_patches = flatten;
        self.forget_failure();
        true
    }

    /// Record a submitted job.
    pub(crate) fn mark_submitted(&mut self, request: ContactRequest, status: ContactStatus) {
        self.in_flight = Some(request);
        self.status = Some(status);
    }

    /// Return the request currently awaiting a completion.
    #[cfg(test)]
    pub(crate) fn pending_request(&self) -> Option<ContactRequest> {
        self.in_flight
    }

    /// Return the pending request when its identity and keys match.
    pub(crate) fn matching_request(
        &self,
        request_id: u64,
        keys: ContactJobKeys,
    ) -> Option<ContactRequest> {
        self.in_flight
            .filter(|request| request.id == request_id && request.keys == keys)
    }

    /// Whether `keys` needs a new measurement.
    pub(crate) fn needs_measurement(&self, keys: ContactJobKeys) -> bool {
        !self.held
            && self.measured != Some(keys)
            && self.in_flight.is_none_or(|request| request.keys != keys)
            && self.failed != Some(keys)
    }

    /// Store a completion if it answers the pending request.
    pub(crate) fn store_measured(
        &mut self,
        request: ContactRequest,
        subject_field: ContactLayerField,
        antagonist_field: ContactLayerField,
        stats: ContactStats,
    ) -> bool {
        if self.matching_request(request.id, request.keys).is_none() {
            return false;
        }
        self.fields.clear();
        self.fields.push(subject_field);
        self.fields.push(antagonist_field);
        self.measured = Some(request.keys);
        self.in_flight = None;
        self.failed = None;
        self.stats = Some(stats);
        self.status = None;
        true
    }

    /// Release a dropped completion and allow the scene to be measured again.
    pub(crate) fn mark_answer_dropped(&mut self, request_id: u64, keys: ContactJobKeys) {
        if self.matching_request(request_id, keys).is_none() {
            return;
        }
        self.in_flight = None;
        if !matches!(self.status, Some(ContactStatus::Failed(_))) {
            self.status = Some(ContactStatus::Remeasuring);
        }
    }

    pub(crate) fn mark_failed(
        &mut self,
        request_id: u64,
        keys: ContactJobKeys,
        failure: ContactFailure,
    ) -> bool {
        if self.matching_request(request_id, keys).is_none() {
            return false;
        }
        self.in_flight = None;
        self.fields.clear();
        self.measured = None;
        self.failed = Some(keys);
        self.stats = None;
        self.status = Some(ContactStatus::Failed(failure));
        true
    }

    /// Mark a request as unavailable because its worker could not run.
    pub(crate) fn mark_unavailable(&mut self, keys: ContactJobKeys) {
        self.in_flight = None;
        self.fields.clear();
        self.measured = None;
        self.failed = Some(keys);
        self.stats = None;
        self.status = Some(ContactStatus::Failed(ContactFailure::Worker));
    }

    /// Take the next field revision number.
    pub(crate) fn take_revision(&mut self) -> u64 {
        self.next_revision = self.next_revision.wrapping_add(1);
        self.next_revision
    }

    /// The keys currently on screen, if the fields describe the live scene.
    pub(crate) fn measured_keys(&self) -> Option<ContactJobKeys> {
        self.measured
    }

    /// The keys of the request the viewer is waiting for, if any.
    pub(crate) fn in_flight_keys(&self) -> Option<ContactJobKeys> {
        self.in_flight.as_ref().map(|request| request.keys)
    }

    /// Whether a measurement is queued or running.
    pub(crate) fn is_busy(&self) -> bool {
        self.in_flight.is_some() || self.worker.as_ref().is_some_and(ContactWorker::is_busy)
    }

    /// Whether the details popover is open.
    pub(crate) fn details_open(&self) -> bool {
        self.details_open
    }

    /// Toggle the details popover.
    pub(crate) fn toggle_details(&mut self) {
        self.details_open = !self.details_open;
    }

    /// The worker, started on first use.
    ///
    /// A worker that has latched a failure is replaced here: its thread has
    /// exited and it refuses every later job, so keeping it would make the
    /// "Read again" the bar offers a button that can never work.
    pub(crate) fn worker_mut(&mut self) -> &mut ContactWorker {
        if self.worker.as_ref().is_some_and(ContactWorker::has_failed) {
            self.worker = None;
        }
        self.worker.get_or_insert_with(ContactWorker::spawn)
    }

    #[cfg(test)]
    pub(crate) fn install_worker_for_tests(&mut self, worker: ContactWorker) {
        self.worker = Some(worker);
    }

    /// The worker, if one has been started.
    pub(crate) fn worker(&self) -> Option<&ContactWorker> {
        self.worker.as_ref()
    }

    /// Drop measured fields while keeping the pair.
    pub(crate) fn drop_fields(&mut self) {
        self.fields.clear();
        self.measured = None;
        self.stats = None;
    }

    /// Whether the panel should offer a retry.
    pub(crate) fn refused(&self) -> bool {
        matches!(self.status, Some(ContactStatus::Failed(_)))
    }

    /// Hold measurement and clear fields while a scan is dragged.
    pub(crate) fn hold_for_drag(&mut self) {
        if self.held {
            return;
        }
        self.held = true;
        self.drop_fields();
        if !matches!(self.status, Some(ContactStatus::Failed(_))) {
            self.status = Some(ContactStatus::Remeasuring);
        }
    }

    /// The drag ended; the next frame may measure again.
    pub(crate) fn resume_after_drag(&mut self) {
        self.held = false;
    }

    /// Allow a new attempt for the current inputs.
    pub(crate) fn forget_failure(&mut self) {
        self.failed = None;
    }

    /// Close a reading whose pair has left the scene and return affected layers.
    pub(crate) fn forget_missing(&mut self, scene: &Scene) -> Vec<SceneMeshId> {
        let Some(pair) = self.pair else {
            return Vec::new();
        };
        let present = |id: SceneMeshId| scene.meshes().iter().any(|entry| entry.id() == id);
        if present(pair.subject) && present(pair.antagonist) {
            return Vec::new();
        }
        let mut orphaned = Vec::new();
        if present(pair.subject) {
            orphaned.push(pair.subject);
        }
        for field in &self.fields {
            if !orphaned.contains(&field.layer) {
                orphaned.push(field.layer);
            }
        }
        self.close();
        orphaned
    }
}

/// Every layer a reading on `layer` could run against, nearest first.
///
/// A scene with an upper and a lower arch has one obvious answer, and the
/// operator should not have to give it. A case with an upper, a lower, a
/// pre-op and a wax-up does not: two of those sit at almost the same centre,
/// and proximity alone cannot say which one is the antagonist. The list is
/// therefore the complete answer to "what can this be measured against", and the
/// automatic pick is its head — the operator is offered the same order the
/// rule uses.
///
/// A layer needs triangles to be measured at all: the worker refuses a surface
/// without them, so offering one would turn a choice into a failure.
pub(crate) fn antagonist_candidates(scene: &Scene, layer: SceneMeshId) -> Vec<SceneMeshId> {
    let Some(subject) = scene.meshes().iter().find(|entry| entry.id() == layer) else {
        return Vec::new();
    };
    if subject.mesh.is_point_cloud() {
        return Vec::new();
    }
    let subject_centre = world_center(subject);
    let mut candidates: Vec<(f32, SceneMeshId)> = scene
        .meshes()
        .iter()
        .filter(|entry| entry.id() != layer)
        .filter(|entry| entry.visible && !entry.mesh.is_point_cloud())
        .filter(|entry| !entry.mesh.bbox_cached().is_empty())
        .filter(|entry| entry.mesh.indices().len() >= 3)
        .map(|entry| {
            (
                world_center(entry).distance_squared(subject_centre),
                entry.id(),
            )
        })
        .collect();
    // Ties go to the lower id rather than to scene order, so the same scene
    // always produces the same list.
    candidates.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
    });
    candidates.into_iter().map(|(_, id)| id).collect()
}

/// Choose the nearest visible triangle mesh as the antagonist.
pub(crate) fn antagonist_for(scene: &Scene, layer: SceneMeshId) -> Option<SceneMeshId> {
    antagonist_candidates(scene, layer).into_iter().next()
}

/// Compute the centre of a layer's transformed bounding box.
fn world_center(entry: &SceneMesh) -> Vec3 {
    let bbox = entry.mesh.bbox_cached();
    if bbox.is_empty() {
        return Vec3::ZERO;
    }
    let corners = [
        bbox.min,
        Vec3::new(bbox.min.x, bbox.min.y, bbox.max.z),
        Vec3::new(bbox.min.x, bbox.max.y, bbox.min.z),
        Vec3::new(bbox.min.x, bbox.max.y, bbox.max.z),
        Vec3::new(bbox.max.x, bbox.min.y, bbox.min.z),
        Vec3::new(bbox.max.x, bbox.min.y, bbox.max.z),
        Vec3::new(bbox.max.x, bbox.max.y, bbox.min.z),
        bbox.max,
    ];
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for corner in corners {
        let world = entry.transform.transform_point3(corner);
        min = min.min(world);
        max = max.max(world);
    }
    (min + max) * 0.5
}

/// Whether `layer` has a visible triangle-mesh antagonist.
pub(crate) fn can_read_contacts(scene: &Scene, layer: SceneMeshId) -> bool {
    let Some(entry) = scene.meshes().iter().find(|entry| entry.id() == layer) else {
        return false;
    };
    entry.visible
        && !entry.mesh.is_point_cloud()
        && entry.mesh.indices().len() >= 3
        && antagonist_for(scene, layer).is_some()
}

/// Build the identity of a measurement from its pair, geometry, poses, and rule.
pub(crate) fn contact_job_keys(
    scene: &Scene,
    pair: ContactPair,
    flatten_patches: bool,
) -> Option<ContactJobKeys> {
    let entry = |id: SceneMeshId| scene.meshes().iter().find(|entry| entry.id() == id);
    let subject = entry(pair.subject)?;
    let antagonist = entry(pair.antagonist)?;
    Some(ContactJobKeys {
        subject: (subject.mesh.geometry_id(), transform_key(subject.transform)),
        antagonist: (
            antagonist.mesh.geometry_id(),
            transform_key(antagonist.transform),
        ),
        flatten_patches,
    })
}

/// Preferred row length for packed contact fields.
pub(crate) const CONTACT_FIELD_TEXTURE_WIDTH: u32 = 1024;

/// Choose a row length that fits the device texture limit.
#[must_use]
pub(crate) fn contact_field_width(vertex_count: usize, device_limit: u32) -> Option<u32> {
    let count = u32::try_from(vertex_count).ok()?;
    if count == 0 || device_limit == 0 {
        return None;
    }
    if count <= CONTACT_FIELD_TEXTURE_WIDTH.saturating_mul(device_limit) {
        return Some(CONTACT_FIELD_TEXTURE_WIDTH.min(count));
    }
    // Rows would not fit one * texel each: widen until they do.
    let width = count.div_ceil(device_limit);
    (width <= device_limit).then_some(width)
}

/// Build an unmasked world-space surface for contact measurement.
#[must_use]
pub(crate) fn world_soup<'a>(positions: &'a [f32], indices: &'a [u32]) -> Soup<'a> {
    Soup {
        positions,
        indices,
        mask: None,
    }
}

/// Interpolate the measured value at a point on a triangle.
pub(crate) fn field_value_at(
    signed_mm: &[f32],
    entry: &SceneMesh,
    triangle: usize,
    world_point: Vec3,
) -> Option<f32> {
    let indices = entry.mesh.indices();
    let corners = indices.get(triangle * 3..triangle * 3 + 3)?;
    let position = |index: u32| -> Option<Vec3> {
        let vertex = entry.mesh.vertices().get(index as usize)?;
        Some(
            entry
                .transform
                .transform_point3(Vec3::from_array(vertex.position)),
        )
    };
    let (pa, pb, pc) = (
        position(corners[0])?,
        position(corners[1])?,
        position(corners[2])?,
    );
    let (wa, wb, wc) = barycentric(world_point, pa, pb, pc)?;
    occluview_contact::interpolate_field_at_triangle(
        signed_mm,
        indices,
        triangle,
        [f64::from(wa), f64::from(wb), f64::from(wc)],
    )
}

/// Return barycentric coordinates, or `None` for a degenerate triangle.
fn barycentric(point: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<(f32, f32, f32)> {
    let v0 = b - a;
    let v1 = c - a;
    let v2 = point - a;
    let d00 = v0.dot(v0);
    let d01 = v0.dot(v1);
    let d11 = v1.dot(v1);
    let d20 = v2.dot(v0);
    let d21 = v2.dot(v1);
    let denominator = d00 * d11 - d01 * d01;
    if denominator.abs() <= f32::EPSILON {
        return None;
    }
    let along_b = (d11 * d20 - d01 * d21) / denominator;
    let along_c = (d00 * d21 - d01 * d20) / denominator;
    let at_a = 1.0 - along_b - along_c;
    Some((at_a, along_b, along_c))
}

/// Convert a signed field value into the panel's reading type.
pub(crate) fn reading_of(signed_mm: f32) -> Option<ContactReading> {
    if is_no_contact(signed_mm) {
        return None;
    }
    ContactReading::from_signed_mm(signed_mm)
}

#[cfg(test)]
#[path = "contact_tests.rs"]
mod tests;
