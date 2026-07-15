use super::job::BridgeSplitJobOutput;
use glam::{Affine3A, Vec3};
use occluview_core::{
    BridgeSplitError, BridgeSplitRequest, CoreBridgeSplitError, SceneMesh, SceneMeshId,
};

pub(crate) const DEFAULT_BRIDGE_SPLIT_KERF_MM: f32 = 0.05;
pub(crate) const MIN_BRIDGE_SPLIT_KERF_MM: f32 = 0.01;
pub(crate) const MAX_BRIDGE_SPLIT_KERF_MM: f32 = 1.0;
pub(crate) const MAX_BRIDGE_SPLIT_DISC_RADIUS_MM: f32 = 60.0;

pub(crate) fn clamp_bridge_split_kerf_mm(value: f32) -> f32 {
    if !value.is_finite() {
        return DEFAULT_BRIDGE_SPLIT_KERF_MM;
    }
    value.clamp(MIN_BRIDGE_SPLIT_KERF_MM, MAX_BRIDGE_SPLIT_KERF_MM)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BridgeSplitPose {
    pub(crate) center: Vec3,
    pub(crate) normal: Vec3,
    pub(crate) radius_mm: f32,
}

impl BridgeSplitPose {
    fn request(self, kerf_mm: f32) -> BridgeSplitRequest {
        BridgeSplitRequest {
            center: self.center,
            normal: self.normal,
            kerf_mm,
            disc_radius_mm: self.radius_mm,
            max_disc_radius_mm: MAX_BRIDGE_SPLIT_DISC_RADIUS_MM,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BridgeSplitTarget {
    pub(crate) layer_id: SceneMeshId,
    pub(crate) topology_id: u64,
    pub(crate) transform: [u32; 12],
}

impl BridgeSplitTarget {
    pub(crate) fn capture(entry: &SceneMesh) -> Self {
        Self {
            layer_id: entry.id(),
            topology_id: entry.mesh.topology_id(),
            transform: affine_bits(&entry.transform),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BridgeSplitGuard {
    pub(crate) session_id: u64,
    pub(crate) generation: u64,
    pub(crate) target: BridgeSplitTarget,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BridgeSplitToolError {
    WorkerStopped,
    InvalidTransform { reason: String },
    Conversion { reason: String },
    Kernel(BridgeSplitError),
    Core { reason: String },
    RobustCsg { reason: String },
}

impl From<CoreBridgeSplitError> for BridgeSplitToolError {
    fn from(value: CoreBridgeSplitError) -> Self {
        match value {
            CoreBridgeSplitError::InvalidTransform { reason } => Self::InvalidTransform { reason },
            CoreBridgeSplitError::Conversion { reason } => Self::Conversion { reason },
            CoreBridgeSplitError::Kernel(error) => Self::Kernel(error),
            CoreBridgeSplitError::Core(error) => Self::Core {
                reason: error.to_string(),
            },
            CoreBridgeSplitError::RobustCsg { reason } => Self::RobustCsg { reason },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BridgeSplitMode {
    #[default]
    Off,
    Following,
    PlantedPending,
    PlantedReady,
    Failed,
}

#[derive(Clone, Debug)]
pub(crate) struct BridgeSplitPreview {
    pub(crate) guard: BridgeSplitGuard,
    pub(crate) result: occluview_core::CoreBridgeSplitResult,
}

#[derive(Clone, Debug)]
pub(crate) struct BridgeSplitSession {
    mode: BridgeSplitMode,
    target: Option<BridgeSplitTarget>,
    pose: Option<BridgeSplitPose>,
    kerf_mm: f32,
    session_id: u64,
    generation: u64,
    preview: Option<BridgeSplitPreview>,
    failure: Option<BridgeSplitToolError>,
}

impl Default for BridgeSplitSession {
    fn default() -> Self {
        Self {
            mode: BridgeSplitMode::Off,
            target: None,
            pose: None,
            kerf_mm: DEFAULT_BRIDGE_SPLIT_KERF_MM,
            session_id: 0,
            generation: 0,
            preview: None,
            failure: None,
        }
    }
}

impl BridgeSplitSession {
    pub(crate) fn mode(&self) -> BridgeSplitMode {
        self.mode
    }

    pub(crate) fn target(&self) -> Option<BridgeSplitTarget> {
        self.target
    }

    #[cfg(test)]
    pub(crate) fn pose(&self) -> Option<BridgeSplitPose> {
        self.pose
    }

    pub(crate) fn kerf_mm(&self) -> f32 {
        self.kerf_mm
    }

    #[cfg(test)]
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn preview(&self) -> Option<&BridgeSplitPreview> {
        self.preview.as_ref()
    }

    pub(crate) fn failure(&self) -> Option<&BridgeSplitToolError> {
        self.failure.as_ref()
    }

    pub(crate) fn can_apply(&self) -> bool {
        matches!(self.mode, BridgeSplitMode::PlantedReady) && self.preview.is_some()
    }

    pub(crate) fn start(&mut self, target: BridgeSplitTarget) {
        self.session_id = next_nonzero_session_id(self.session_id);
        self.mode = BridgeSplitMode::Following;
        self.target = Some(target);
        self.pose = None;
        self.kerf_mm = DEFAULT_BRIDGE_SPLIT_KERF_MM;
        self.generation = 0;
        self.preview = None;
        self.failure = None;
    }

    pub(crate) fn set_follow_pose(&mut self, pose: Option<BridgeSplitPose>) -> bool {
        if self.mode != BridgeSplitMode::Following {
            return false;
        }
        if self.pose == pose {
            return false;
        }
        self.pose = pose;
        true
    }

    pub(crate) fn plant(&mut self, pose: BridgeSplitPose) -> Option<BridgeSplitGuard> {
        if self.mode != BridgeSplitMode::Following {
            return None;
        }
        self.pose = Some(pose);
        self.bump_generation_pending()
    }

    pub(crate) fn update_pose(&mut self, pose: BridgeSplitPose) -> Option<BridgeSplitGuard> {
        if !matches!(
            self.mode,
            BridgeSplitMode::PlantedPending
                | BridgeSplitMode::PlantedReady
                | BridgeSplitMode::Failed
        ) {
            return None;
        }
        if same_pose_bits(self.pose, Some(pose)) {
            return None;
        }
        self.pose = Some(pose);
        self.bump_generation_pending()
    }

    pub(crate) fn set_kerf_mm(&mut self, value: f32) -> Option<BridgeSplitGuard> {
        let clamped = clamp_bridge_split_kerf_mm(value);
        if self.kerf_mm.to_bits() == clamped.to_bits() {
            return None;
        }
        self.kerf_mm = clamped;
        if matches!(
            self.mode,
            BridgeSplitMode::PlantedPending
                | BridgeSplitMode::PlantedReady
                | BridgeSplitMode::Failed
        ) {
            return self.bump_generation_pending();
        }
        self.preview = None;
        self.failure = None;
        None
    }

    pub(crate) fn current_guard(&self) -> Option<BridgeSplitGuard> {
        let target = self.target?;
        if matches!(self.mode, BridgeSplitMode::Following | BridgeSplitMode::Off) {
            return None;
        }
        Some(BridgeSplitGuard {
            session_id: self.session_id,
            generation: self.generation,
            target,
        })
    }

    pub(crate) fn current_request(&self) -> Option<BridgeSplitRequest> {
        if self.mode != BridgeSplitMode::PlantedPending {
            return None;
        }
        self.pose.map(|pose| pose.request(self.kerf_mm))
    }

    pub(crate) fn apply_job_output(
        &mut self,
        live_target: Option<BridgeSplitTarget>,
        output: BridgeSplitJobOutput,
    ) -> bool {
        if self.target != Some(output.guard.target)
            || live_target != Some(output.guard.target)
            || self.session_id != output.guard.session_id
            || self.generation != output.guard.generation
        {
            return false;
        }
        if !matches!(
            self.mode,
            BridgeSplitMode::PlantedPending
                | BridgeSplitMode::PlantedReady
                | BridgeSplitMode::Failed
        ) {
            return false;
        }
        match output.result {
            Ok(result) => {
                self.preview = Some(BridgeSplitPreview {
                    guard: output.guard,
                    result,
                });
                self.failure = None;
                self.mode = BridgeSplitMode::PlantedReady;
            }
            Err(error) => {
                self.preview = None;
                self.failure = Some(error);
                self.mode = BridgeSplitMode::Failed;
            }
        }
        true
    }

    pub(crate) fn cancel(&mut self) {
        let session_id = self.session_id;
        *self = Self::default();
        self.session_id = session_id;
    }

    fn bump_generation_pending(&mut self) -> Option<BridgeSplitGuard> {
        let target = self.target?;
        self.generation = self.generation.saturating_add(1);
        self.preview = None;
        self.failure = None;
        self.mode = BridgeSplitMode::PlantedPending;
        Some(BridgeSplitGuard {
            session_id: self.session_id,
            generation: self.generation,
            target,
        })
    }
}

pub(crate) const fn next_nonzero_session_id(current: u64) -> u64 {
    match current.checked_add(1) {
        Some(next) => next,
        None => 1,
    }
}

fn same_pose_bits(lhs: Option<BridgeSplitPose>, rhs: Option<BridgeSplitPose>) -> bool {
    match (lhs, rhs) {
        (Some(lhs), Some(rhs)) => pose_bits(lhs) == pose_bits(rhs),
        (None, None) => true,
        _ => false,
    }
}

fn pose_bits(pose: BridgeSplitPose) -> [u32; 7] {
    [
        pose.center.x.to_bits(),
        pose.center.y.to_bits(),
        pose.center.z.to_bits(),
        pose.normal.x.to_bits(),
        pose.normal.y.to_bits(),
        pose.normal.z.to_bits(),
        pose.radius_mm.to_bits(),
    ]
}

fn affine_bits(transform: &Affine3A) -> [u32; 12] {
    let matrix = transform.matrix3;
    let columns = [
        matrix.x_axis.to_array(),
        matrix.y_axis.to_array(),
        matrix.z_axis.to_array(),
        transform.translation.to_array(),
    ];
    let mut bits = [0u32; 12];
    for (column, slots) in columns.iter().zip(bits.chunks_exact_mut(3)) {
        for (value, slot) in column.iter().zip(slots.iter_mut()) {
            *slot = value.to_bits();
        }
    }
    bits
}
