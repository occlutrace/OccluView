//! The operator-facing controls catalogue.
//!
//! This is data, not another input router. The handlers in the
//! app remain the authority for behavior; this catalogue gives the Help
//! surface and the viewport reminder one spelling for the controls they
//! already expose.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HintRow {
    pub(crate) gesture: &'static str,
    /// Catalog key rendering the localized action. Gestures stay invariant
    /// input vocabulary (physical keys and buttons, like shortcuts).
    pub(crate) key: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HintSection {
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
    Contacts,
}

const NAVIGATION: &[HintRow] = &[
    HintRow {
        gesture: "RMB drag",
        key: "help-hint-navigation-orbit-the-camera",
    },
    HintRow {
        gesture: "MMB drag",
        key: "help-hint-navigation-pan-the-camera",
    },
    HintRow {
        gesture: "Two-finger scroll",
        key: "help-hint-navigation-pan-the-camera",
    },
    HintRow {
        gesture: "LMB + RMB drag",
        key: "help-hint-navigation-pan-the-camera-2",
    },
    HintRow {
        gesture: "Wheel",
        key: "help-hint-navigation-zoom-toward-the-pointer",
    },
    HintRow {
        gesture: "Pinch",
        key: "help-hint-navigation-zoom-toward-the-pointer",
    },
    HintRow {
        gesture: "MMB click",
        key: "help-hint-navigation-recenter-on-the-surface",
    },
    HintRow {
        gesture: "Double-click",
        key: "help-hint-navigation-recenter-on-the-surface-when-enabled",
    },
    HintRow {
        gesture: "RMB click",
        key: "help-hint-navigation-open-the-layer-or-scene-menu-when-stationary",
    },
];

const TOOLS: &[HintRow] = &[
    HintRow {
        gesture: "Ctrl+O",
        key: "help-hint-tools-open-a-file",
    },
    HintRow {
        gesture: "C",
        key: "help-hint-tools-open-cut-view",
    },
    HintRow {
        gesture: "M",
        key: "help-hint-tools-arm-the-ruler",
    },
    HintRow {
        gesture: "T",
        key: "help-hint-tools-arm-thickness",
    },
    HintRow {
        gesture: "A",
        key: "help-hint-tools-open-align",
    },
    HintRow {
        gesture: "E",
        key: "help-hint-tools-open-mesh-editing",
    },
];

const MESH_EDITING: &[HintRow] = &[
    HintRow {
        gesture: "LMB click",
        key: "help-hint-mesh-editing-select-a-face",
    },
    HintRow {
        gesture: "Shift+click",
        key: "help-hint-mesh-editing-unmark-a-face-or-screen-selection",
    },
    HintRow {
        gesture: "Rectangle drag",
        key: "help-hint-mesh-editing-select-faces-in-a-screen-rectangle",
    },
    HintRow {
        gesture: "Lasso points",
        key: "help-hint-mesh-editing-draw-a-freehand-selection-outline",
    },
    HintRow {
        gesture: "Enter / double-click",
        key: "help-hint-mesh-editing-close-and-apply-a-lasso-outline",
    },
    HintRow {
        gesture: "Esc",
        key: "help-hint-mesh-editing-cancel-the-active-lasso-outline",
    },
    HintRow {
        gesture: "Ctrl+A",
        key: "help-hint-mesh-editing-select-all-visible-faces",
    },
    HintRow {
        gesture: "Delete / Backspace",
        key: "help-hint-mesh-editing-delete-selected-faces",
    },
    HintRow {
        gesture: "Ctrl+Z",
        key: "help-hint-mesh-editing-undo-the-last-mesh-edit",
    },
    HintRow {
        gesture: "Ctrl+Y / Ctrl+Shift+Z",
        key: "help-hint-mesh-editing-redo-the-last-mesh-edit",
    },
];

const SCULPT: &[HintRow] = &[
    HintRow {
        gesture: "1",
        key: "help-hint-sculpt-choose-add-remove",
    },
    HintRow {
        gesture: "2",
        key: "help-hint-sculpt-choose-smooth",
    },
    HintRow {
        gesture: "LMB drag",
        key: "help-hint-sculpt-sculpt-under-the-brush",
    },
    HintRow {
        gesture: "Shift + LMB drag",
        key: "help-hint-sculpt-remove-or-strengthen-the-active-brush-mode",
    },
    HintRow {
        gesture: "Shift+wheel",
        key: "help-hint-sculpt-change-brush-size",
    },
    HintRow {
        gesture: "Ctrl+wheel",
        key: "help-hint-sculpt-change-brush-intensity",
    },
];

const ALIGN_AND_MEASURE: &[HintRow] = &[
    HintRow {
        gesture: "LMB click",
        key: "help-hint-align-measure-place-an-alignment-point-or-measurement-point",
    },
    HintRow {
        gesture: "LMB click on a ruler line",
        key: "help-hint-align-measure-drop-a-perpendicular-onto-a-ruler-line",
    },
    HintRow {
        gesture: "Ctrl/Command + LMB drag",
        key: "help-hint-align-measure-rotate-a-scan-in-align-s-manual-mode",
    },
    HintRow {
        gesture: "Shift + LMB drag",
        key: "help-hint-align-measure-erase-an-align-exclusion-region",
    },
    HintRow {
        gesture: "Shift+wheel",
        key: "help-hint-align-measure-change-align-exclusion-brush-size",
    },
    HintRow {
        gesture: "RMB click",
        key: "help-hint-align-measure-undo-the-last-alignment-point-when-stationary",
    },
    HintRow {
        gesture: "RMB click in the ruler",
        key: "help-hint-align-measure-clear-measurements-when-stationary",
    },
    HintRow {
        gesture: "Esc",
        key: "help-hint-align-measure-close-the-active-measurement-tool",
    },
];

const CUT_VIEW: &[HintRow] = &[
    HintRow {
        gesture: "LMB click / drag",
        key: "help-hint-cut-view-plant-or-move-the-cut-disc",
    },
    HintRow {
        gesture: "Ctrl+wheel in Section",
        key: "help-hint-cut-view-change-disc-size",
    },
    HintRow {
        gesture: "Wheel in Section",
        key: "help-hint-cut-view-zoom-the-section-view",
    },
    HintRow {
        gesture: "F",
        key: "help-hint-cut-view-flip-the-kept-half-while-planted",
    },
    HintRow {
        gesture: "Esc",
        key: "help-hint-cut-view-unplant-the-disc-or-close-cut-view",
    },
];

const LAYERS_AND_EXPLORER_PREVIEW: &[HintRow] = &[
    HintRow {
        gesture: "Ctrl+Middle-click",
        key: "help-hint-layers-preview-hide-the-layer-under-the-pointer",
    },
    HintRow {
        gesture: "Ctrl+Shift+Middle-click",
        key: "help-hint-layers-preview-restore-the-last-hidden-layer",
    },
    HintRow {
        gesture: "Shift+Middle-click",
        key: "help-hint-layers-preview-toggle-layer-translucency",
    },
    HintRow {
        gesture: "RMB drag in Explorer Preview",
        key: "help-hint-layers-preview-orbit-the-preview-model",
    },
    HintRow {
        gesture: "Wheel in Explorer Preview",
        key: "help-hint-layers-preview-zoom-the-preview-model",
    },
    HintRow {
        gesture: "F in Explorer Preview",
        key: "help-hint-layers-preview-frame-the-preview-model",
    },
    HintRow {
        gesture: "W in Explorer Preview",
        key: "help-hint-layers-preview-toggle-preview-wireframe",
    },
];

/// The occlusal contact reading: how to open one, what the one slider does, and
/// which of the two readings answers which question.
const CONTACTS: &[HintRow] = &[
    HintRow {
        gesture: "RMB on a layer",
        key: "help-hint-contacts-read-its-occlusal-contacts-against-the-scan-it-bites",
    },
    HintRow {
        gesture: "Pointer over the map",
        key: "help-hint-contacts-read-the-contact-depth-under-the-cursor",
    },
    HintRow {
        gesture: "Heavy at",
        key: "help-hint-contacts-move-the-depth-the-ramp-calls-fully-loaded",
    },
    HintRow {
        gesture: "Contacts / Approach",
        key: "help-hint-contacts-switch-between-marks-only-and-the-whole-approach",
    },
    HintRow {
        gesture: "Esc",
        key: "help-hint-contacts-close-the-reading-and-take-the-marks-off-both-scans",
    },
];

pub(crate) const ALL_SECTIONS: &[HintSection] = &[
    HintSection {
        key: "help-section-navigation",
        rows: NAVIGATION,
    },
    HintSection {
        key: "help-section-tools",
        rows: TOOLS,
    },
    HintSection {
        key: "help-section-mesh-editing",
        rows: MESH_EDITING,
    },
    HintSection {
        key: "help-section-sculpt",
        rows: SCULPT,
    },
    HintSection {
        key: "help-section-align-measure",
        rows: ALIGN_AND_MEASURE,
    },
    HintSection {
        key: "help-section-cut-view",
        rows: CUT_VIEW,
    },
    HintSection {
        key: "help-section-contacts",
        rows: CONTACTS,
    },
    HintSection {
        key: "help-section-layers-preview",
        rows: LAYERS_AND_EXPLORER_PREVIEW,
    },
];

pub(crate) const fn contextual_line(context: HintContext) -> &'static str {
    match context {
        HintContext::Navigation => "RMB drag orbit · MMB drag pan · Trackpad scroll pan · Wheel/pinch zoom · MMB click focus",
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
        HintContext::Contacts => {
            "Right-click a layer · Show contacts · drag Heavy at to repaint · Esc closes"
        }
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
        HintContext::Contacts => "help-hintline-contacts",
    }
}

#[cfg(test)]
mod tests {
    use super::{contextual_line, contextual_line_key, HintContext, ALL_SECTIONS};

    /// Every context, listed once for the tests that must cover all of them.
    ///
    /// A new variant is forced into `contextual_line_key` by its exhaustive
    /// `match`, and into this list by the two tests below failing without it.
    const ALL_CONTEXTS: &[HintContext] = &[
        HintContext::Navigation,
        HintContext::MeshEditing,
        HintContext::Sculpt,
        HintContext::Align,
        HintContext::Cut,
        HintContext::Measure,
        HintContext::Contacts,
    ];

    #[test]
    fn catalogue_has_every_display_section_with_rows() {
        assert_eq!(ALL_SECTIONS.len(), 8);
        assert!(ALL_SECTIONS.iter().all(|section| !section.rows.is_empty()));
    }

    /// Every context's line must be the English catalog's own text: the
    /// catalogue is what renders, so a source literal that drifts from it is a
    /// second, invisible copy of the wording.
    #[test]
    fn every_contextual_line_is_pinned_to_its_catalog_entry() {
        #![allow(clippy::expect_used)]
        let catalog = crate::i18n::catalog::Catalog::build("en").expect("en builds");
        for context in ALL_CONTEXTS {
            assert_eq!(
                catalog.text(contextual_line_key(*context)).as_deref(),
                Some(contextual_line(*context)),
                "{context:?} hint line drifted from its catalog entry"
            );
        }
    }

    #[test]
    fn every_context_has_a_contextual_line() {
        for context in ALL_CONTEXTS {
            assert!(!contextual_line(*context).is_empty());
        }
    }
}
