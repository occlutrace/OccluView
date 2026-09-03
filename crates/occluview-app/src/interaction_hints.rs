//! The operator-facing controls catalogue.
//!
//! This is deliberately data, not another input router. The handlers in the
//! app remain the authority for behavior; this catalogue gives the Help
//! surface and the viewport reminder one spelling for the controls they
//! already expose.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HintRow {
    pub(crate) gesture: &'static str,
    pub(crate) action: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HintSection {
    pub(crate) title: &'static str,
    pub(crate) rows: &'static [HintRow],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HintContext {
    Navigation,
    MeshEditing,
    Sculpt,
    Align,
    Cut,
    Measure,
}

const NAVIGATION: &[HintRow] = &[
    HintRow {
        gesture: "RMB drag",
        action: "Orbit the camera",
    },
    HintRow {
        gesture: "MMB drag",
        action: "Pan the camera",
    },
    HintRow {
        gesture: "LMB + RMB drag",
        action: "Pan the camera",
    },
    HintRow {
        gesture: "Wheel",
        action: "Zoom toward the pointer",
    },
    HintRow {
        gesture: "MMB click",
        action: "Recenter on the surface",
    },
    HintRow {
        gesture: "Double-click",
        action: "Recenter on the surface when enabled",
    },
    HintRow {
        gesture: "RMB click",
        action: "Open the layer or scene menu when stationary",
    },
];

const TOOLS: &[HintRow] = &[
    HintRow {
        gesture: "Ctrl+O",
        action: "Open a file",
    },
    HintRow {
        gesture: "C",
        action: "Open Cut View",
    },
    HintRow {
        gesture: "M",
        action: "Arm the Ruler",
    },
    HintRow {
        gesture: "T",
        action: "Arm Thickness",
    },
    HintRow {
        gesture: "A",
        action: "Open Align",
    },
    HintRow {
        gesture: "E",
        action: "Open Mesh Editing",
    },
];

const MESH_EDITING: &[HintRow] = &[
    HintRow {
        gesture: "LMB click",
        action: "Select a face",
    },
    HintRow {
        gesture: "Shift+click",
        action: "Unmark a face or screen selection",
    },
    HintRow {
        gesture: "Rectangle drag",
        action: "Select faces in a screen rectangle",
    },
    HintRow {
        gesture: "Lasso points",
        action: "Draw a freehand selection outline",
    },
    HintRow {
        gesture: "Enter / double-click",
        action: "Close and apply a lasso outline",
    },
    HintRow {
        gesture: "Esc",
        action: "Cancel the active lasso outline",
    },
    HintRow {
        gesture: "Ctrl+A",
        action: "Select all visible faces",
    },
    HintRow {
        gesture: "Delete / Backspace",
        action: "Delete selected faces",
    },
    HintRow {
        gesture: "Ctrl+Z",
        action: "Undo the last mesh edit",
    },
    HintRow {
        gesture: "Ctrl+Y / Ctrl+Shift+Z",
        action: "Redo the last mesh edit",
    },
];

const SCULPT: &[HintRow] = &[
    HintRow {
        gesture: "1",
        action: "Choose Add/Remove",
    },
    HintRow {
        gesture: "2",
        action: "Choose Smooth",
    },
    HintRow {
        gesture: "LMB drag",
        action: "Sculpt under the brush",
    },
    HintRow {
        gesture: "Shift + LMB drag",
        action: "Remove or strengthen the active brush mode",
    },
    HintRow {
        gesture: "Shift+wheel",
        action: "Change brush size",
    },
    HintRow {
        gesture: "Ctrl+wheel",
        action: "Change brush intensity",
    },
];

const ALIGN_AND_MEASURE: &[HintRow] = &[
    HintRow {
        gesture: "LMB click",
        action: "Place an alignment point or measurement point",
    },
    HintRow {
        gesture: "Ctrl/Command + LMB drag",
        action: "Rotate a scan in Align's Manual mode",
    },
    HintRow {
        gesture: "Shift + LMB drag",
        action: "Erase an Align exclusion region",
    },
    HintRow {
        gesture: "Shift+wheel",
        action: "Change Align exclusion-brush size",
    },
    HintRow {
        gesture: "RMB click",
        action: "Undo the last alignment point when stationary",
    },
    HintRow {
        gesture: "RMB click in the ruler",
        action: "Clear measurements when stationary",
    },
    HintRow {
        gesture: "Esc",
        action: "Close the active measurement tool",
    },
];

const CUT_VIEW: &[HintRow] = &[
    HintRow {
        gesture: "LMB click / drag",
        action: "Plant or move the cut disc",
    },
    HintRow {
        gesture: "Ctrl+wheel in Section",
        action: "Change disc size",
    },
    HintRow {
        gesture: "Wheel in Section",
        action: "Zoom the section view",
    },
    HintRow {
        gesture: "F",
        action: "Flip the kept half while planted",
    },
    HintRow {
        gesture: "Esc",
        action: "Unplant the disc or close Cut View",
    },
];

const LAYERS_AND_EXPLORER_PREVIEW: &[HintRow] = &[
    HintRow {
        gesture: "Ctrl+Middle-click",
        action: "Hide the layer under the pointer",
    },
    HintRow {
        gesture: "Ctrl+Shift+Middle-click",
        action: "Restore the last hidden layer",
    },
    HintRow {
        gesture: "Shift+Middle-click",
        action: "Toggle layer translucency",
    },
    HintRow {
        gesture: "RMB drag in Explorer Preview",
        action: "Orbit the preview model",
    },
    HintRow {
        gesture: "Wheel in Explorer Preview",
        action: "Zoom the preview model",
    },
    HintRow {
        gesture: "F in Explorer Preview",
        action: "Frame the preview model",
    },
    HintRow {
        gesture: "W in Explorer Preview",
        action: "Toggle preview wireframe",
    },
];

pub(crate) const ALL_SECTIONS: &[HintSection] = &[
    HintSection {
        title: "Navigation",
        rows: NAVIGATION,
    },
    HintSection {
        title: "Tools",
        rows: TOOLS,
    },
    HintSection {
        title: "Mesh Editing",
        rows: MESH_EDITING,
    },
    HintSection {
        title: "Sculpt",
        rows: SCULPT,
    },
    HintSection {
        title: "Align and Measure",
        rows: ALIGN_AND_MEASURE,
    },
    HintSection {
        title: "Cut View",
        rows: CUT_VIEW,
    },
    HintSection {
        title: "Layers and Explorer Preview",
        rows: LAYERS_AND_EXPLORER_PREVIEW,
    },
];

pub(crate) const fn contextual_line(context: HintContext) -> &'static str {
    match context {
        HintContext::Navigation => "RMB drag orbit · MMB drag pan · Wheel zoom · MMB click focus",
        HintContext::MeshEditing => {
            "LMB select · Shift+click unmark · Drag rectangle · Ctrl+Z undo"
        }
        HintContext::Sculpt => {
            "LMB sculpt · Shift changes mode · Shift+wheel size · Ctrl+wheel force"
        }
        HintContext::Align => "LMB place · Ctrl/Command+drag rotate · Shift+drag erase · RMB undo",
        HintContext::Cut => {
            "LMB plant or move · Ctrl+wheel in Section resizes · F flips · Esc closes"
        }
        HintContext::Measure => "LMB measure · RMB clears · Wheel zooms · Esc closes",
    }
}

#[cfg(test)]
mod tests {
    use super::{contextual_line, HintContext, ALL_SECTIONS};

    #[test]
    fn catalogue_has_every_display_section_with_rows() {
        assert_eq!(ALL_SECTIONS.len(), 7);
        assert!(ALL_SECTIONS.iter().all(|section| !section.rows.is_empty()));
    }

    #[test]
    fn every_context_has_a_contextual_line() {
        for context in [
            HintContext::Navigation,
            HintContext::MeshEditing,
            HintContext::Sculpt,
            HintContext::Align,
            HintContext::Cut,
            HintContext::Measure,
        ] {
            assert!(!contextual_line(context).is_empty());
        }
    }
}
