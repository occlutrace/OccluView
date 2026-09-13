//! OCCLUSAL CONTACTS: which two layers a reading runs between, and the one
//! number the operator drives.
//!
//! The measurement itself lives in [`occluview_contact`] and runs on the
//! contact worker. This module owns the state around it: the pair, which law
//! the map is read under, the "heavy at" depth, the packed per-layer fields the
//! viewport paints, and the sentence the panel is showing.
//!
//! THE SUBJECT AND THE ANTAGONIST. `subject` is the layer that wears the marks
//! — the one the operator right-clicked. `antagonist` is the surface it bites
//! against: the nearest visible surface, and deliberately nothing cleverer. With
//! the two scans a bite is made of there is one answer and any rule finds it;
//! with a waxup or a preoperative copy in the scene as well, nearest is the one
//! an operator can predict without being told the rule. Hidden layers and point
//! clouds are not candidates — a point cloud has no surface to measure to, and a
//! hidden scan would put the reading on a surface nobody can see.
//!
//! BOTH SURFACES CARRY MARKS. The field is measured in both directions (see
//! [`occluview_contact::compute_contact_field`]) and both layers wear their own
//! reading, because the operator reads the bite from whichever side is facing
//! them and a mark on one arch only is a mark they have to orbit to find.
//!
//! ONE NUMBER, AND IT NEVER RE-MEASURES. Judging a bite is a question about a
//! threshold — where does close stop being contact and start being pressure —
//! and the honest way to answer it is to move the threshold and watch the map.
//! Nothing the slider touches changes a distance: the field is measured once,
//! and the depth at which the ramp reads fully loaded is carried in the shader's
//! stop table, so dragging the slider is a uniform write.

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

/// Which reading the map is showing.
///
/// The two laws answer two different questions about the same bite, so both
/// exist: `Marks` is digital articulating paper (where do the arches meet, and
/// how hard), `Approach` is the T-Scan convention (how close is the antagonist
/// everywhere, load included).
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

/// The one measurement the reading is waiting for.
///
/// Identity is the pair of what the job measures and which submission asked for
/// it, not the scene generation alone: the operator can ask again without
/// changing anything about the scene, and both submissions share a generation.
/// A completion that does not answer the record here is an answer to a question
/// nobody is asking any more, and the reading must not take it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContactRequest {
    /// The identity the worker assigned when the job was queued.
    pub(crate) id: u64,
    /// What the measurement describes.
    pub(crate) keys: ContactJobKeys,
    /// The pair whose two layers the resulting fields belong to.
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
    /// the reading is on hold. It is about the SUBJECT: the scan the operator
    /// right-clicked. A sentence about the other one would send them to unhide
    /// the wrong layer.
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

/// One layer's finished reading: the values, and the texture the viewport
/// paints from.
///
/// The raw values are kept beside the packed texels because the hover readout
/// needs the honest field — the packed texture carries a finite stand-in where a
/// vertex found no opposing surface (an infinity cannot survive interpolation),
/// and "no opposing surface here" is a different answer from "0.5 mm of
/// clearance" when the operator is reading a number off the surface.
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
pub(crate) struct ContactState {
    pair: Option<ContactPair>,
    mode: ContactMode,
    load_mm: f64,
    /// Whether each connected penetration patch is collapsed to its peak.
    ///
    /// Off by default because that is how the reference viewer ships: the force
    /// distribution inside a mark is a reading an operator wants. On, one
    /// contact reads as one flat colour, which is what a multi-hue ramp wants
    /// (without it every mark wears a rim of the intermediate depths it passes
    /// through on its way in).
    flatten_patches: bool,
    status: Option<ContactStatus>,
    /// The fields on screen, at most one per layer and never more than the two
    /// participants.
    fields: Vec<ContactLayerField>,
    /// The keys the on-screen fields were measured from.
    measured: Option<ContactJobKeys>,
    /// The measurement a job is currently in flight for.
    in_flight: Option<ContactRequest>,
    /// The keys the last attempt FAILED on.
    ///
    /// A refusal is a property of the input, so retrying it on the next frame
    /// would re-run the same doomed search forever. The frame loop asks
    /// [`Self::needs_measurement`], which compares against this, so a failure
    /// is reported once and only a change to the pair or the geometry starts a
    /// new attempt.
    failed: Option<ContactJobKeys>,
    /// Whether the reading is being held back because a scan is being dragged
    /// by hand: every frame of that drag changes the distances the reading is
    /// about, so measuring on each one would restart a surface index build per
    /// frame and throw all of them away.
    held: bool,
    /// What the last measurement found, for the panel's numbers.
    stats: Option<ContactStats>,
    worker: Option<ContactWorker>,
    /// The arrays hand to the worker, keyed by geometry and pose so a
    /// re-submit after a display change never copies a mesh.
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

    /// The scale the map is painted with: the mode's law at the operator's load
    /// depth.
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

    /// Open a reading on `pair`.
    ///
    /// The load depth resets to the law's own, so opening a reading always
    /// starts from the number the law was designed around rather than from
    /// wherever the previous case left the slider. Everything the last reading
    /// put on screen is dropped before the new pair is recorded, or the operator
    /// is looking at two maps and one legend.
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

    /// Replace the panel's sentence without touching the reading.
    ///
    /// Used for the conditions that are about the scene rather than about the
    /// job: nothing to measure against, or a surface that stopped being one.
    pub(crate) fn status_override(&mut self, status: ContactStatus) {
        self.status = Some(status);
    }

    /// Move the slider. Returns whether it actually moved.
    ///
    /// The value is clamped to the range the panel offers, so a settings file
    /// from a future version or a keyboard nudge cannot put the ramp somewhere
    /// no legend describes.
    pub(crate) fn set_load_mm(&mut self, load_mm: f64) -> bool {
        if !load_mm.is_finite() {
            return false;
        }
        let next = load_mm.clamp(LOAD_MIN_MM, LOAD_MAX_MM);
        let moved = (next - self.load_mm).abs() > f64::EPSILON;
        self.load_mm = next;
        moved
    }

    /// Switch reading. Returns whether it actually changed.
    ///
    /// The load depth follows the law, because the two laws call different
    /// depths "loaded" and carrying a number across would silently re-scale the
    /// map the operator was just looking at. No re-measure happens either way:
    /// the field is what the surfaces do, not what the ramp says about it.
    pub(crate) fn set_mode(&mut self, mode: ContactMode) -> bool {
        if self.mode == mode {
            return false;
        }
        self.mode = mode;
        self.load_mm = mode.law().load_mm;
        true
    }

    /// Toggle the patch collapse. Returns whether it changed.
    pub(crate) fn set_flatten_patches(&mut self, flatten: bool) -> bool {
        if self.flatten_patches == flatten {
            return false;
        }
        self.flatten_patches = flatten;
        // The patch rule is an input to the measurement, so changing it is a
        // reason to measure again even after a refusal.
        self.forget_failure();
        true
    }

    /// Record a submitted job.
    pub(crate) fn mark_submitted(&mut self, request: ContactRequest, status: ContactStatus) {
        self.in_flight = Some(request);
        self.status = Some(status);
    }

    /// The submission the reading is waiting for, if any.
    ///
    /// Read by the delivery tests, which have to name the request an answer is
    /// published for; production reaches the same record through
    /// [`Self::matching_request`].
    #[cfg(test)]
    pub(crate) fn pending_request(&self) -> Option<ContactRequest> {
        self.in_flight
    }

    /// The in-flight request a completion answers, or `None` when it answers a
    /// measurement the operator has moved past.
    ///
    /// This is the gate a finished job passes before it may touch the fields,
    /// the statistics, the status, or the in-flight record. It is deliberately
    /// keyed on the request identity *and* on what that request measured: two
    /// submissions of the same pair at the same pose are still two different
    /// measurements, and the older one's answer carries numbers from the moment
    /// it was queued.
    pub(crate) fn matching_request(
        &self,
        request_id: u64,
        keys: ContactJobKeys,
    ) -> Option<ContactRequest> {
        self.in_flight
            .filter(|request| request.id == request_id && request.keys == keys)
    }

    /// Whether `keys` still has to be measured.
    ///
    /// False while a job for it is in flight, false while its result is on
    /// screen, and false after a refusal — a refusal is about the input, and
    /// retrying it every frame would re-run a doomed search for as long as the
    /// operator leaves the panel open.
    pub(crate) fn needs_measurement(&self, keys: ContactJobKeys) -> bool {
        !self.held
            && self.measured != Some(keys)
            && self.in_flight.is_none_or(|request| request.keys != keys)
            && self.failed != Some(keys)
    }

    /// Put a finished measurement on screen, if it still answers the current
    /// request.
    ///
    /// One entry point for the whole application of a result, so no call site
    /// can store the fields first and check the identity afterwards. Returns
    /// whether the reading took it.
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

    /// Note that a finished answer was discarded because the surfaces it
    /// describes are no longer the ones on screen.
    ///
    /// The request is over either way, and clearing the in-flight record is what
    /// lets the frame loop measure again for the scene as it is now. Leaving it
    /// would hold [`Self::needs_measurement`] false for as long as the live keys
    /// happened to equal the request's again — which an exact round trip of a
    /// hand drag produces — and the panel would sit on "re-measuring" with no
    /// job running and nothing left to submit it.
    ///
    /// A refusal the panel is already showing is left alone: it has a remedy of
    /// its own, and this is not what produced it.
    pub(crate) fn mark_answer_dropped(&mut self, request_id: u64, keys: ContactJobKeys) {
        if self.matching_request(request_id, keys).is_none() {
            return;
        }
        self.in_flight = None;
        if !matches!(self.status, Some(ContactStatus::Failed(_))) {
            self.status = Some(ContactStatus::Remeasuring);
        }
    }

    /// Note that the measurement for the in-flight request failed, if it still
    /// is the in-flight request. Returns whether the reading took it.
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

    /// Record that no completion is coming for `keys`, because the worker could
    /// not run the job at all.
    ///
    /// Distinct from a refusal the compute produced in what it means to the
    /// frame loop: the keys land in `failed`, so the reading is not re-queued on
    /// every frame against a worker that cannot execute it. Without this the
    /// panel would sit on "Measuring…" with no executor behind it.
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

    /// Whether a measurement is queued or running.
    ///
    /// The panel asks this to disable the patch toggle while a reading is in
    /// flight: turning that switch mid-compute would queue a second measurement
    /// whose result arrives after the first, and the operator would watch the
    /// older reading land last.
    pub(crate) fn is_busy(&self) -> bool {
        self.in_flight.is_some() || self.worker.as_ref().is_some_and(ContactWorker::is_busy)
    }

    /// Whether the details popover is showing beside the bar.
    ///
    /// Lives in the state rather than in egui memory so it survives the frames
    /// the bar is not drawn (a hidden window, a modal in front).
    pub(crate) fn details_open(&self) -> bool {
        self.details_open
    }

    /// Toggle the details popover.
    pub(crate) fn toggle_details(&mut self) {
        self.details_open = !self.details_open;
    }

    /// The worker, started on first use.
    pub(crate) fn worker_mut(&mut self) -> &mut ContactWorker {
        self.worker.get_or_insert_with(ContactWorker::spawn)
    }

    /// Replace the worker, for tests that need to decide how it behaves.
    #[cfg(test)]
    pub(crate) fn install_worker_for_tests(&mut self, worker: ContactWorker) {
        self.worker = Some(worker);
    }

    /// The worker, if one has been started.
    pub(crate) fn worker(&self) -> Option<&ContactWorker> {
        self.worker.as_ref()
    }

    /// Drop the fields but keep the pair — used when the scene moved under a
    /// finished reading and it has to be taken down before it is re-measured.
    pub(crate) fn drop_fields(&mut self) {
        self.fields.clear();
        self.measured = None;
        self.stats = None;
    }

    /// Whether the last attempt produced no map that can be fixed by trying
    /// again, which is when the panel offers a retry button.
    ///
    /// A hidden scan is not a refusal — the remedy is to unhide it — so it does
    /// not get one. Neither does a pair that simply does not meet: the reading
    /// succeeded and said so.
    pub(crate) fn refused(&self) -> bool {
        matches!(self.status, Some(ContactStatus::Failed(_)))
    }

    /// Hold the reading back: a scan is being dragged, and every frame of the
    /// drag changes the distances in it.
    ///
    /// The marks come off at the same time. Leaving the last measurement up
    /// while the surfaces move under it would show a map that no longer
    /// describes what is on screen — the numbers would be from where the scan
    /// used to be.
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

    /// Forget the last refusal, so the next frame tries again.
    ///
    /// Called when the operator changes something the measurement depends on
    /// (the patch rule), and when a fresh reading opens: a new attempt after an
    /// explicit action is what the operator asked for, a new attempt every frame
    /// is a bug.
    pub(crate) fn forget_failure(&mut self) {
        self.failed = None;
    }

    /// Forget a pair whose layers are no longer both in the scene.
    ///
    /// Returns the layers whose marks have to come off. A reading can outlive
    /// its premise — a scan leaves the scene through a removal, an undo, a crop
    /// or a separate — and a pair left pointing at a missing scan would go on
    /// claiming a measurement against something that is not there.
    pub(crate) fn forget_missing(&mut self, scene: &Scene) -> Vec<SceneMeshId> {
        let Some(pair) = self.pair else {
            return Vec::new();
        };
        let present = |id: SceneMeshId| scene.meshes().iter().any(|entry| entry.id() == id);
        if present(pair.subject) && present(pair.antagonist) {
            return Vec::new();
        }
        // The subject is the one that matters: its marks are on a surface that
        // is still on screen, and they describe a measurement against something
        // that is not. The antagonist's own marks leave with it.
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

/// The scan `layer` bites against, or `None` when nothing in the scene can be.
///
/// Nearest by bounding-box centre among the visible triangle meshes, with the
/// empty-bounding-box case filtered out. See the module docs for why nearest is
/// the whole rule.
pub(crate) fn antagonist_for(scene: &Scene, layer: SceneMeshId) -> Option<SceneMeshId> {
    let subject = scene.meshes().iter().find(|entry| entry.id() == layer)?;
    if subject.mesh.is_point_cloud() {
        return None;
    }
    let subject_centre = world_center(subject);
    scene
        .meshes()
        .iter()
        .filter(|entry| entry.id() != layer)
        .filter(|entry| entry.visible && !entry.mesh.is_point_cloud())
        .filter(|entry| !entry.mesh.bbox_cached().is_empty())
        .min_by(|left, right| {
            let left_gap = world_center(left).distance_squared(subject_centre);
            let right_gap = world_center(right).distance_squared(subject_centre);
            left_gap.total_cmp(&right_gap)
        })
        .map(SceneMesh::id)
}

/// The centre of a layer's world bounding box.
///
/// Conservative on purpose, exactly as the scene's own framing box is: a rotated
/// layer's world box is the box of its rotated corners, which is larger than the
/// rotated box. Both are used for the same thing here — deciding which of two
/// candidate surfaces a scan sits closer to — and an over-large box never
/// changes which of two scans is the near one in a bite.
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

/// Whether a reading can be opened on `layer` in this scene.
///
/// The context menu asks this to decide whether the entry is offered lit. A
/// reading that needs two surfaces should not be an option that can only explain
/// why it did nothing.
pub(crate) fn can_read_contacts(scene: &Scene, layer: SceneMeshId) -> bool {
    let Some(entry) = scene.meshes().iter().find(|entry| entry.id() == layer) else {
        return false;
    };
    entry.visible
        && !entry.mesh.is_point_cloud()
        && entry.mesh.indices().len() >= 3
        && antagonist_for(scene, layer).is_some()
}

/// The identity of a measurement: everything the distances depend on, and
/// nothing else.
///
/// The display scale is deliberately absent. A slider move changes colours, not
/// distances, so a reading keyed on the load depth would re-measure a million
/// vertices every time the operator nudged the number that is supposed to be
/// free to explore.
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

/// Preferred packed field texture row length.
///
/// The shader only needs the row stride to turn a vertex index into a texel, so
/// this is a texture-shape choice rather than a correctness one: one row per
/// this many vertices keeps the texture near-square for a scan of any size,
/// which matters because a very tall single-column texture is the shape drivers
/// handle worst.
pub(crate) const CONTACT_FIELD_TEXTURE_WIDTH: u32 = 1024;

/// The row length a field of `vertex_count` values must use on this device.
///
/// The preferred width is only a preference: a field is `ceil(n / width)` rows
/// tall, so a large enough scan overflows `max_texture_dimension_2d` and the
/// texture cannot be created at all — the reading would fail on the machine
/// with the most data to read. Widening the row keeps the texture inside the
/// limit; a pathological scan that would still overflow is refused here rather
/// than at texture creation, so the caller gets "not packed" instead of a
/// wgpu validation error.
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

/// The align-crate soup for one layer, already posed into world space.
///
/// The masks the Align tool paints do not travel with a contact reading: those
/// marks are indexed by the align session's own roles, so applying them to a
/// different pair would paint out an arbitrary region of a scan with nothing on
/// screen to say why. This is a separate constructor rather than a default so
/// the rule is written down where the job is built.
#[must_use]
pub(crate) fn world_soup<'a>(positions: &'a [f32], indices: &'a [u32]) -> Soup<'a> {
    Soup {
        positions,
        indices,
        mask: None,
    }
}

/// The signed field value at a point on one triangle of a layer.
///
/// Barycentric, so the number the readout prints is the number the surface
/// carries where the pointer is, not the value at whichever corner happened to
/// be nearest. A corner with no measurement contributes the finite stand-in the
/// GPU uses (an infinity cannot be blended), and a triangle whose three corners
/// all carry no measurement answers `None` — "nothing to measure against here"
/// is a different answer from a distance.
///
/// The blend itself lives in [`occluview_contact`] beside the paint weight, so
/// the number the chip prints and the colour under the pointer come from one
/// implementation; this function only supplies the corner weights the ray hit
/// implies.
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

/// Barycentric coordinates of `point` in the triangle `a`, `b`, `c`.
///
/// `None` for a degenerate triangle, where the coordinates are not defined and
/// any answer would be an invention.
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

/// The magnitude of a reading, and which side of touch it is on.
///
/// A thin re-export of the crate's own rule so the panel and the chip cannot
/// disagree about what "no opposing surface" means: a vertex that found nothing
/// has no reading, which is not zero millimetres of clearance and must never be
/// printed as one.
pub(crate) fn reading_of(signed_mm: f32) -> Option<ContactReading> {
    if is_no_contact(signed_mm) {
        return None;
    }
    ContactReading::from_signed_mm(signed_mm)
}

#[cfg(test)]
#[path = "contact_tests.rs"]
mod tests;
