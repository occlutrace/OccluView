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
    /// Catalog key rendering the localized action. The English `action`
    /// literal stays beside it: source guards pin the operator wording and
    /// the README sync reads it. Gestures stay invariant input vocabulary
    /// (physical keys and buttons, like shortcuts).
    pub(crate) key: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HintSection {
    pub(crate) title: &'static str,
    /// Catalog key rendering the localized section title.
    pub(crate) key: &'static str,
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
        key: "help-hint-navigation-orbit-the-camera",
    },
    HintRow {
        gesture: "MMB drag",
        action: "Pan the camera",
        key: "help-hint-navigation-pan-the-camera",
    },
    HintRow {
        gesture: "LMB + RMB drag",
        action: "Pan the camera",
        key: "help-hint-navigation-pan-the-camera-2",
    },
    HintRow {
        gesture: "Wheel",
        action: "Zoom toward the pointer",
        key: "help-hint-navigation-zoom-toward-the-pointer",
    },
    HintRow {
        gesture: "MMB click",
        action: "Recenter on the surface",
        key: "help-hint-navigation-recenter-on-the-surface",
    },
    HintRow {
        gesture: "Double-click",
        action: "Recenter on the surface when enabled",
        key: "help-hint-navigation-recenter-on-the-surface-when-enabled",
    },
    HintRow {
        gesture: "RMB click",
        action: "Open the layer or scene menu when stationary",
        key: "help-hint-navigation-open-the-layer-or-scene-menu-when-stationary",
    },
];

const TOOLS: &[HintRow] = &[
    HintRow {
        gesture: "Ctrl+O",
        action: "Open a file",
        key: "help-hint-tools-open-a-file",
    },
    HintRow {
        gesture: "C",
        action: "Open Cut View",
        key: "help-hint-tools-open-cut-view",
    },
    HintRow {
        gesture: "M",
        action: "Arm the Ruler",
        key: "help-hint-tools-arm-the-ruler",
    },
    HintRow {
        gesture: "T",
        action: "Arm Thickness",
        key: "help-hint-tools-arm-thickness",
    },
    HintRow {
        gesture: "A",
        action: "Open Align",
        key: "help-hint-tools-open-align",
    },
    HintRow {
        gesture: "E",
        action: "Open Mesh Editing",
        key: "help-hint-tools-open-mesh-editing",
    },
];

const MESH_EDITING: &[HintRow] = &[
    HintRow {
        gesture: "LMB click",
        action: "Select a face",
        key: "help-hint-mesh-editing-select-a-face",
    },
    HintRow {
        gesture: "Shift+click",
        action: "Unmark a face or screen selection",
        key: "help-hint-mesh-editing-unmark-a-face-or-screen-selection",
    },
    HintRow {
        gesture: "Rectangle drag",
        action: "Select faces in a screen rectangle",
        key: "help-hint-mesh-editing-select-faces-in-a-screen-rectangle",
    },
    HintRow {
        gesture: "Lasso points",
        action: "Draw a freehand selection outline",
        key: "help-hint-mesh-editing-draw-a-freehand-selection-outline",
    },
    HintRow {
        gesture: "Enter / double-click",
        action: "Close and apply a lasso outline",
        key: "help-hint-mesh-editing-close-and-apply-a-lasso-outline",
    },
    HintRow {
        gesture: "Esc",
        action: "Cancel the active lasso outline",
        key: "help-hint-mesh-editing-cancel-the-active-lasso-outline",
    },
    HintRow {
        gesture: "Ctrl+A",
        action: "Select all visible faces",
        key: "help-hint-mesh-editing-select-all-visible-faces",
    },
    HintRow {
        gesture: "Delete / Backspace",
        action: "Delete selected faces",
        key: "help-hint-mesh-editing-delete-selected-faces",
    },
    HintRow {
        gesture: "Ctrl+Z",
        action: "Undo the last mesh edit",
        key: "help-hint-mesh-editing-undo-the-last-mesh-edit",
    },
    HintRow {
        gesture: "Ctrl+Y / Ctrl+Shift+Z",
        action: "Redo the last mesh edit",
        key: "help-hint-mesh-editing-redo-the-last-mesh-edit",
    },
];

const SCULPT: &[HintRow] = &[
    HintRow {
        gesture: "1",
        action: "Choose Add/Remove",
        key: "help-hint-sculpt-choose-add-remove",
    },
    HintRow {
        gesture: "2",
        action: "Choose Smooth",
        key: "help-hint-sculpt-choose-smooth",
    },
    HintRow {
        gesture: "LMB drag",
        action: "Sculpt under the brush",
        key: "help-hint-sculpt-sculpt-under-the-brush",
    },
    HintRow {
        gesture: "Shift + LMB drag",
        action: "Remove or strengthen the active brush mode",
        key: "help-hint-sculpt-remove-or-strengthen-the-active-brush-mode",
    },
    HintRow {
        gesture: "Shift+wheel",
        action: "Change brush size",
        key: "help-hint-sculpt-change-brush-size",
    },
    HintRow {
        gesture: "Ctrl+wheel",
        action: "Change brush intensity",
        key: "help-hint-sculpt-change-brush-intensity",
    },
];

const ALIGN_AND_MEASURE: &[HintRow] = &[
    HintRow {
        gesture: "LMB click",
        action: "Place an alignment point or measurement point",
        key: "help-hint-align-measure-place-an-alignment-point-or-measurement-point",
    },
    HintRow {
        gesture: "Ctrl/Command + LMB drag",
        action: "Rotate a scan in Align's Manual mode",
        key: "help-hint-align-measure-rotate-a-scan-in-align-s-manual-mode",
    },
    HintRow {
        gesture: "Shift + LMB drag",
        action: "Erase an Align exclusion region",
        key: "help-hint-align-measure-erase-an-align-exclusion-region",
    },
    HintRow {
        gesture: "Shift+wheel",
        action: "Change Align exclusion-brush size",
        key: "help-hint-align-measure-change-align-exclusion-brush-size",
    },
    HintRow {
        gesture: "RMB click",
        action: "Undo the last alignment point when stationary",
        key: "help-hint-align-measure-undo-the-last-alignment-point-when-stationary",
    },
    HintRow {
        gesture: "RMB click in the ruler",
        action: "Clear measurements when stationary",
        key: "help-hint-align-measure-clear-measurements-when-stationary",
    },
    HintRow {
        gesture: "Esc",
        action: "Close the active measurement tool",
        key: "help-hint-align-measure-close-the-active-measurement-tool",
    },
];

const CUT_VIEW: &[HintRow] = &[
    HintRow {
        gesture: "LMB click / drag",
        action: "Plant or move the cut disc",
        key: "help-hint-cut-view-plant-or-move-the-cut-disc",
    },
    HintRow {
        gesture: "Ctrl+wheel in Section",
        action: "Change disc size",
        key: "help-hint-cut-view-change-disc-size",
    },
    HintRow {
        gesture: "Wheel in Section",
        action: "Zoom the section view",
        key: "help-hint-cut-view-zoom-the-section-view",
    },
    HintRow {
        gesture: "F",
        action: "Flip the kept half while planted",
        key: "help-hint-cut-view-flip-the-kept-half-while-planted",
    },
    HintRow {
        gesture: "Esc",
        action: "Unplant the disc or close Cut View",
        key: "help-hint-cut-view-unplant-the-disc-or-close-cut-view",
    },
];

const LAYERS_AND_EXPLORER_PREVIEW: &[HintRow] = &[
    HintRow {
        gesture: "Ctrl+Middle-click",
        action: "Hide the layer under the pointer",
        key: "help-hint-layers-preview-hide-the-layer-under-the-pointer",
    },
    HintRow {
        gesture: "Ctrl+Shift+Middle-click",
        action: "Restore the last hidden layer",
        key: "help-hint-layers-preview-restore-the-last-hidden-layer",
    },
    HintRow {
        gesture: "Shift+Middle-click",
        action: "Toggle layer translucency",
        key: "help-hint-layers-preview-toggle-layer-translucency",
    },
    HintRow {
        gesture: "RMB drag in Explorer Preview",
        action: "Orbit the preview model",
        key: "help-hint-layers-preview-orbit-the-preview-model",
    },
    HintRow {
        gesture: "Wheel in Explorer Preview",
        action: "Zoom the preview model",
        key: "help-hint-layers-preview-zoom-the-preview-model",
    },
    HintRow {
        gesture: "F in Explorer Preview",
        action: "Frame the preview model",
        key: "help-hint-layers-preview-frame-the-preview-model",
    },
    HintRow {
        gesture: "W in Explorer Preview",
        action: "Toggle preview wireframe",
        key: "help-hint-layers-preview-toggle-preview-wireframe",
    },
];

pub(crate) const ALL_SECTIONS: &[HintSection] = &[
    HintSection {
        title: "Navigation",
        key: "help-section-navigation",
        rows: NAVIGATION,
    },
    HintSection {
        title: "Tools",
        key: "help-section-tools",
        rows: TOOLS,
    },
    HintSection {
        title: "Mesh Editing",
        key: "help-section-mesh-editing",
        rows: MESH_EDITING,
    },
    HintSection {
        title: "Sculpt",
        key: "help-section-sculpt",
        rows: SCULPT,
    },
    HintSection {
        title: "Align and Measure",
        key: "help-section-align-measure",
        rows: ALIGN_AND_MEASURE,
    },
    HintSection {
        title: "Cut View",
        key: "help-section-cut-view",
        rows: CUT_VIEW,
    },
    HintSection {
        title: "Layers and Explorer Preview",
        key: "help-section-layers-preview",
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

/// Catalog key rendering the localized contextual line for each context.
pub(crate) const fn contextual_line_key(context: HintContext) -> &'static str {
    match context {
        HintContext::Navigation => "help-hintline-navigation",
        HintContext::MeshEditing => "help-hintline-mesh-editing",
        HintContext::Sculpt => "help-hintline-sculpt",
        HintContext::Align => "help-hintline-align",
        HintContext::Cut => "help-hintline-cut",
        HintContext::Measure => "help-hintline-measure",
    }
}

#[cfg(test)]
mod tests {
    use super::{contextual_line, contextual_line_key, HintContext, ALL_SECTIONS};

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

    /// The English catalog must render exactly the operator wording pinned in
    /// this source (README sync + documents guard read these literals).
    /// This also keeps the `title`/`action` fields live for the compiler.
    #[test]
    fn english_catalog_matches_source_wording() {
        #![allow(clippy::expect_used)]
        let catalog = crate::i18n::catalog::Catalog::build("en").expect("en builds");
        for section in ALL_SECTIONS {
            assert_eq!(
                catalog.text(section.key).as_deref(),
                Some(section.title),
                "section key drift: {}",
                section.key
            );
            for row in section.rows {
                assert_eq!(
                    catalog.text(row.key).as_deref(),
                    Some(row.action),
                    "row key drift: {}",
                    row.key
                );
            }
        }
        for context in [
            HintContext::Navigation,
            HintContext::MeshEditing,
            HintContext::Sculpt,
            HintContext::Align,
            HintContext::Cut,
            HintContext::Measure,
        ] {
            assert_eq!(
                catalog.text(contextual_line_key(context)).as_deref(),
                Some(contextual_line(context)),
                "hint line drift: {}",
                contextual_line_key(context)
            );
        }
    }
}
