use occluview_core::SceneMeshId;

/// Stable app-level identity for a layer edit session.
///
/// Scene indices shift when layers are appended or removed; edit sessions use
/// this key so future result application can reject stale layer targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LayerKey(u64);

impl LayerKey {
    #[cfg(test)]
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    pub(crate) const fn from_scene_mesh_id(id: SceneMeshId) -> Self {
        Self(id.get())
    }

    #[cfg(test)]
    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

/// Which primary-gesture the mesh editor's face selection is in. The three
/// modes are mutually exclusive: only one gesture can own the primary click at a
/// time. Surface/Through (front-facing vs. through-mesh) is an ORTHOGONAL flag
/// that refines Marquee and Lasso; it does not apply to Object, because an
/// object is a whole connected component regardless of which way its facets
/// face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SelectGesture {
    /// Default resting gesture: a primary drag sweeps a marquee rectangle and a
    /// stationary primary click marks the single face under the cursor.
    #[default]
    Marquee,
    /// Freehand lasso: primary presses/drag place an outline that marks every
    /// triangle it encloses (exocad "Mark triangles").
    Lasso,
    /// Object pick: a stationary primary click selects the WHOLE connected
    /// component (one object of a multi-object STL) under the cursor. A drag is
    /// left to the camera.
    Object,
}

/// Token attached to background edit work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct EditSessionToken(u64);

impl EditSessionToken {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    #[cfg(test)]
    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

/// Mesh edit operation names understood by app edit sessions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditModeCommand {
    BridgeSplit,
    InvertNormals,
    DeleteSelectedFaces,
    CropToSelectedFaces,
    CutSelectionToNewLayer,
    SeparateSelectedComponents,
    CloseHoles,
    RepairMesh,
}

/// Result of attempting to finish a busy edit operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BusyFinish {
    Applied,
    Stale,
    NotBusy,
}

/// State machine for one active mesh-edit session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EditModeState {
    Inactive,
    ActiveClean {
        layer: LayerKey,
    },
    ActiveDirty {
        layer: LayerKey,
    },
    Busy {
        layer: LayerKey,
        token: EditSessionToken,
        command: EditModeCommand,
        was_dirty: bool,
    },
    Error {
        layer: Option<LayerKey>,
        message: String,
        recoverable: bool,
    },
}

impl Default for EditModeState {
    fn default() -> Self {
        Self::Inactive
    }
}

impl EditModeState {
    pub(crate) fn start(&mut self, layer: LayerKey) -> bool {
        if matches!(self, Self::Busy { .. }) {
            return false;
        }
        *self = Self::ActiveClean { layer };
        true
    }

    #[cfg(test)]
    pub(crate) fn active_layer(&self) -> Option<LayerKey> {
        match self {
            Self::Inactive => None,
            Self::ActiveClean { layer }
            | Self::ActiveDirty { layer }
            | Self::Busy { layer, .. } => Some(*layer),
            Self::Error { layer, .. } => *layer,
        }
    }

    #[cfg(test)]
    pub(crate) fn is_dirty(&self) -> bool {
        matches!(
            self,
            Self::ActiveDirty { .. }
                | Self::Busy {
                    was_dirty: true,
                    ..
                }
        )
    }

    #[cfg(test)]
    pub(crate) fn mark_dirty(&mut self) -> bool {
        let layer = match self {
            Self::ActiveClean { layer } | Self::ActiveDirty { layer } => *layer,
            _ => return false,
        };
        *self = Self::ActiveDirty { layer };
        true
    }

    pub(crate) fn confirm_discard(&mut self) {
        *self = Self::Inactive;
    }

    pub(crate) fn begin_busy(&mut self, command: EditModeCommand, token: EditSessionToken) -> bool {
        let (layer, was_dirty) = match *self {
            Self::ActiveClean { layer } => (layer, false),
            Self::ActiveDirty { layer } => (layer, true),
            _ => return false,
        };
        *self = Self::Busy {
            layer,
            token,
            command,
            was_dirty,
        };
        true
    }

    pub(crate) fn finish_busy_success(
        &mut self,
        token: EditSessionToken,
        dirty_after: bool,
    ) -> BusyFinish {
        let Self::Busy {
            layer,
            token: active_token,
            was_dirty,
            ..
        } = *self
        else {
            return BusyFinish::NotBusy;
        };

        if active_token != token {
            return BusyFinish::Stale;
        }

        *self = if was_dirty || dirty_after {
            Self::ActiveDirty { layer }
        } else {
            Self::ActiveClean { layer }
        };
        BusyFinish::Applied
    }

    pub(crate) fn finish_busy_error(
        &mut self,
        token: EditSessionToken,
        message: String,
    ) -> BusyFinish {
        let Self::Busy {
            layer,
            token: active_token,
            ..
        } = *self
        else {
            return BusyFinish::NotBusy;
        };

        if active_token != token {
            return BusyFinish::Stale;
        }

        *self = Self::Error {
            layer: Some(layer),
            message,
            recoverable: true,
        };
        BusyFinish::Applied
    }

    #[cfg(test)]
    pub(crate) fn clear_error(&mut self) -> bool {
        let Self::Error {
            layer, recoverable, ..
        } = *self
        else {
            return false;
        };
        *self = if recoverable {
            layer.map_or(Self::Inactive, |layer| Self::ActiveDirty { layer })
        } else {
            Self::Inactive
        };
        true
    }
}
