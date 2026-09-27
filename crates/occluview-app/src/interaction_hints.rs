//! The operator-facing controls catalogue.
//!
//! This is data, not another input router. The handlers in the
//! app remain the authority for behavior; this catalogue gives the Help
//! surface and the viewport reminder one spelling for the controls they
//! already expose.

use crate::app_settings::ScrollBehavior;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HintRow {
    pub(crate) gesture: &'static str,
    /// Catalog key rendering the localized action. Gestures stay invariant
    /// input vocabulary (physical keys and buttons, like shortcuts).
    pub(crate) key: crate::i18n::MessageId,
}

impl HintRow {
    pub(crate) fn action_key(self, scroll_behavior: ScrollBehavior) -> crate::i18n::MessageId {
        if self.gesture == TRACKPAD_SCROLL_GESTURE {
            trackpad_scroll_action_key(scroll_behavior)
        } else {
            self.key
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HintSection {
    /// Catalog key rendering the localized section title.
    pub(crate) key: crate::i18n::MessageId,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ContextualHint {
    pub(crate) context: HintContext,
    pub(crate) scroll_behavior: ScrollBehavior,
}

const TRACKPAD_SCROLL_GESTURE: &str = "Two-finger scroll";

const NAVIGATION: &[HintRow] = &[
    HintRow {
        gesture: "RMB drag",
        key: crate::i18n::message_id!("help-hint-navigation-orbit-the-camera"),
    },
    HintRow {
        gesture: "MMB drag",
        key: crate::i18n::message_id!("help-hint-navigation-pan-the-camera"),
    },
    #[cfg(target_os = "macos")]
    HintRow {
        gesture: TRACKPAD_SCROLL_GESTURE,
        key: crate::i18n::message_id!("help-hint-navigation-pan-the-camera"),
    },
    HintRow {
        gesture: "LMB + RMB drag",
        key: crate::i18n::message_id!("help-hint-navigation-pan-the-camera-2"),
    },
    HintRow {
        gesture: "Wheel",
        key: crate::i18n::message_id!("help-hint-navigation-zoom-toward-the-pointer"),
    },
    HintRow {
        gesture: "Pinch",
        key: crate::i18n::message_id!("help-hint-navigation-zoom-toward-the-pointer"),
    },
    HintRow {
        gesture: "MMB click",
        key: crate::i18n::message_id!("help-hint-navigation-recenter-on-the-surface"),
    },
    HintRow {
        gesture: "Double-click",
        key: crate::i18n::message_id!("help-hint-navigation-recenter-on-the-surface-when-enabled"),
    },
    HintRow {
        gesture: "RMB click",
        key: crate::i18n::message_id!(
            "help-hint-navigation-open-the-layer-or-scene-menu-when-stationary"
        ),
    },
];

const TOOLS: &[HintRow] = &[
    HintRow {
        gesture: "Ctrl+O",
        key: crate::i18n::message_id!("help-hint-tools-open-a-file"),
    },
    HintRow {
        gesture: "C",
        key: crate::i18n::message_id!("help-hint-tools-open-cut-view"),
    },
    HintRow {
        gesture: "M",
        key: crate::i18n::message_id!("help-hint-tools-arm-the-ruler"),
    },
    HintRow {
        gesture: "T",
        key: crate::i18n::message_id!("help-hint-tools-arm-thickness"),
    },
    HintRow {
        gesture: "A",
        key: crate::i18n::message_id!("help-hint-tools-open-align"),
    },
    HintRow {
        gesture: "E",
        key: crate::i18n::message_id!("help-hint-tools-open-mesh-editing"),
    },
];

const MESH_EDITING: &[HintRow] = &[
    HintRow {
        gesture: "LMB click",
        key: crate::i18n::message_id!("help-hint-mesh-editing-select-a-face"),
    },
    HintRow {
        gesture: "Shift+click",
        key: crate::i18n::message_id!("help-hint-mesh-editing-unmark-a-face-or-screen-selection"),
    },
    HintRow {
        gesture: "Rectangle drag",
        key: crate::i18n::message_id!("help-hint-mesh-editing-select-faces-in-a-screen-rectangle"),
    },
    HintRow {
        gesture: "Lasso points",
        key: crate::i18n::message_id!("help-hint-mesh-editing-draw-a-freehand-selection-outline"),
    },
    HintRow {
        gesture: "Enter / double-click",
        key: crate::i18n::message_id!("help-hint-mesh-editing-close-and-apply-a-lasso-outline"),
    },
    HintRow {
        gesture: "Esc",
        key: crate::i18n::message_id!("help-hint-mesh-editing-cancel-the-active-lasso-outline"),
    },
    HintRow {
        gesture: "Ctrl+A",
        key: crate::i18n::message_id!("help-hint-mesh-editing-select-all-visible-faces"),
    },
    HintRow {
        gesture: "Delete / Backspace",
        key: crate::i18n::message_id!("help-hint-mesh-editing-delete-selected-faces"),
    },
    HintRow {
        gesture: "Ctrl+Z",
        key: crate::i18n::message_id!("help-hint-mesh-editing-undo-the-last-mesh-edit"),
    },
    HintRow {
        gesture: "Ctrl+Y / Ctrl+Shift+Z",
        key: crate::i18n::message_id!("help-hint-mesh-editing-redo-the-last-mesh-edit"),
    },
];

const SCULPT: &[HintRow] = &[
    HintRow {
        gesture: "1",
        key: crate::i18n::message_id!("help-hint-sculpt-choose-add-remove"),
    },
    HintRow {
        gesture: "2",
        key: crate::i18n::message_id!("help-hint-sculpt-choose-smooth"),
    },
    HintRow {
        gesture: "LMB drag",
        key: crate::i18n::message_id!("help-hint-sculpt-sculpt-under-the-brush"),
    },
    HintRow {
        gesture: "Shift + LMB drag",
        key: crate::i18n::message_id!(
            "help-hint-sculpt-remove-or-strengthen-the-active-brush-mode"
        ),
    },
    HintRow {
        gesture: "Shift+wheel",
        key: crate::i18n::message_id!("help-hint-sculpt-change-brush-size"),
    },
    HintRow {
        gesture: "Ctrl+wheel",
        key: crate::i18n::message_id!("help-hint-sculpt-change-brush-intensity"),
    },
];

const ALIGN_AND_MEASURE: &[HintRow] = &[
    HintRow {
        gesture: "LMB click",
        key: crate::i18n::message_id!(
            "help-hint-align-measure-place-an-alignment-point-or-measurement-point"
        ),
    },
    HintRow {
        gesture: "LMB click on a ruler line",
        key: crate::i18n::message_id!("help-hint-align-measure-end-a-ruler-on-a-ruler-line"),
    },
    HintRow {
        gesture: "Shift in Ruler",
        key: crate::i18n::message_id!("help-hint-align-measure-switch-between-any-angle-and-90"),
    },
    HintRow {
        gesture: "Ctrl/Command + LMB drag",
        key: crate::i18n::message_id!(
            "help-hint-align-measure-rotate-a-scan-in-align-s-manual-mode"
        ),
    },
    HintRow {
        gesture: "Shift + LMB drag",
        key: crate::i18n::message_id!("help-hint-align-measure-erase-an-align-exclusion-region"),
    },
    HintRow {
        gesture: "Shift+wheel",
        key: crate::i18n::message_id!("help-hint-align-measure-change-align-exclusion-brush-size"),
    },
    HintRow {
        gesture: "RMB click",
        key: crate::i18n::message_id!(
            "help-hint-align-measure-undo-the-last-alignment-point-when-stationary"
        ),
    },
    HintRow {
        gesture: "RMB click in the ruler",
        key: crate::i18n::message_id!("help-hint-align-measure-clear-measurements-when-stationary"),
    },
    HintRow {
        gesture: "Esc",
        key: crate::i18n::message_id!("help-hint-align-measure-close-the-active-measurement-tool"),
    },
];

const CUT_VIEW: &[HintRow] = &[
    HintRow {
        gesture: "LMB click / drag",
        key: crate::i18n::message_id!("help-hint-cut-view-plant-or-move-the-cut-disc"),
    },
    HintRow {
        gesture: "Ctrl+wheel in Section",
        key: crate::i18n::message_id!("help-hint-cut-view-change-disc-size"),
    },
    HintRow {
        gesture: "Wheel in Section",
        key: crate::i18n::message_id!("help-hint-cut-view-zoom-the-section-view"),
    },
    HintRow {
        gesture: "F",
        key: crate::i18n::message_id!("help-hint-cut-view-flip-the-kept-half-while-planted"),
    },
    HintRow {
        gesture: "Esc",
        key: crate::i18n::message_id!("help-hint-cut-view-unplant-the-disc-or-close-cut-view"),
    },
];

const LAYERS_AND_EXPLORER_PREVIEW: &[HintRow] = &[
    HintRow {
        gesture: "Ctrl+Middle-click",
        key: crate::i18n::message_id!("help-hint-layers-preview-hide-the-layer-under-the-pointer"),
    },
    HintRow {
        gesture: "Ctrl+Shift+Middle-click",
        key: crate::i18n::message_id!("help-hint-layers-preview-restore-the-last-hidden-layer"),
    },
    HintRow {
        gesture: "Shift+Middle-click",
        key: crate::i18n::message_id!("help-hint-layers-preview-toggle-layer-translucency"),
    },
    HintRow {
        gesture: "RMB drag in Explorer Preview",
        key: crate::i18n::message_id!("help-hint-layers-preview-orbit-the-preview-model"),
    },
    HintRow {
        gesture: "Wheel in Explorer Preview",
        key: crate::i18n::message_id!("help-hint-layers-preview-zoom-the-preview-model"),
    },
    HintRow {
        gesture: "F in Explorer Preview",
        key: crate::i18n::message_id!("help-hint-layers-preview-frame-the-preview-model"),
    },
    HintRow {
        gesture: "W in Explorer Preview",
        key: crate::i18n::message_id!("help-hint-layers-preview-toggle-preview-wireframe"),
    },
];

/// The occlusal contact reading: how to open one, what the one slider does, and
/// which of the two readings answers which question.
const CONTACTS: &[HintRow] = &[
    HintRow {
        gesture: "RMB on a layer",
        key: crate::i18n::message_id!(
            "help-hint-contacts-read-its-occlusal-contacts-against-the-scan-it-bites"
        ),
    },
    HintRow {
        gesture: "Pointer over the map",
        key: crate::i18n::message_id!("help-hint-contacts-read-the-contact-depth-under-the-cursor"),
    },
    HintRow {
        gesture: "Heavy at",
        key: crate::i18n::message_id!(
            "help-hint-contacts-move-the-depth-the-ramp-calls-fully-loaded"
        ),
    },
    HintRow {
        gesture: "Contacts / Approach",
        key: crate::i18n::message_id!(
            "help-hint-contacts-switch-between-marks-only-and-the-whole-approach"
        ),
    },
    HintRow {
        gesture: "Esc",
        key: crate::i18n::message_id!(
            "help-hint-contacts-close-the-reading-and-take-the-marks-off-both-scans"
        ),
    },
];

pub(crate) const ALL_SECTIONS: &[HintSection] = &[
    HintSection {
        key: crate::i18n::message_id!("help-section-navigation"),
        rows: NAVIGATION,
    },
    HintSection {
        key: crate::i18n::message_id!("help-section-tools"),
        rows: TOOLS,
    },
    HintSection {
        key: crate::i18n::message_id!("help-section-mesh-editing"),
        rows: MESH_EDITING,
    },
    HintSection {
        key: crate::i18n::message_id!("help-section-sculpt"),
        rows: SCULPT,
    },
    HintSection {
        key: crate::i18n::message_id!("help-section-align-measure"),
        rows: ALIGN_AND_MEASURE,
    },
    HintSection {
        key: crate::i18n::message_id!("help-section-cut-view"),
        rows: CUT_VIEW,
    },
    HintSection {
        key: crate::i18n::message_id!("help-section-contacts"),
        rows: CONTACTS,
    },
    HintSection {
        key: crate::i18n::message_id!("help-section-layers-preview"),
        rows: LAYERS_AND_EXPLORER_PREVIEW,
    },
];

pub(crate) const fn contextual_line(
    context: HintContext,
    _scroll_behavior: ScrollBehavior,
) -> &'static str {
    match context {
        #[cfg(target_os = "macos")]
        HintContext::Navigation => macos_navigation_line(_scroll_behavior),
        #[cfg(not(target_os = "macos"))]
        HintContext::Navigation => {
            "RMB drag orbit · MMB drag pan · Wheel/pinch zoom · MMB click focus"
        }
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
pub(crate) const fn contextual_line_key(
    context: HintContext,
    _scroll_behavior: ScrollBehavior,
) -> crate::i18n::MessageId {
    match context {
        #[cfg(target_os = "macos")]
        HintContext::Navigation => macos_navigation_line_key(_scroll_behavior),
        #[cfg(not(target_os = "macos"))]
        HintContext::Navigation => crate::i18n::message_id!("help-hintline-navigation"),
        HintContext::MeshEditing => crate::i18n::message_id!("help-hintline-mesh-editing"),
        HintContext::Sculpt => crate::i18n::message_id!("help-hintline-sculpt"),
        HintContext::Align => crate::i18n::message_id!("help-hintline-align"),
        HintContext::Cut => crate::i18n::message_id!("help-hintline-cut"),
        HintContext::Measure => crate::i18n::message_id!("help-hintline-measure"),
        HintContext::Contacts => crate::i18n::message_id!("help-hintline-contacts"),
    }
}

#[cfg(target_os = "macos")]
const fn macos_navigation_line(scroll_behavior: ScrollBehavior) -> &'static str {
    match scroll_behavior {
        ScrollBehavior::Pan => {
            "RMB drag orbit · MMB drag pan · Trackpad scroll pan · Wheel/pinch zoom · MMB click focus"
        }
        ScrollBehavior::Zoom => {
            "RMB drag orbit · MMB drag pan · Trackpad scroll zoom · Wheel/pinch zoom · MMB click focus"
        }
    }
}

#[cfg(target_os = "macos")]
const fn macos_navigation_line_key(scroll_behavior: ScrollBehavior) -> crate::i18n::MessageId {
    match scroll_behavior {
        ScrollBehavior::Pan => crate::i18n::message_id!("help-hintline-navigation-macos-pan"),
        ScrollBehavior::Zoom => crate::i18n::message_id!("help-hintline-navigation-macos-zoom"),
    }
}

const fn trackpad_scroll_action_key(scroll_behavior: ScrollBehavior) -> crate::i18n::MessageId {
    match scroll_behavior {
        ScrollBehavior::Pan => {
            crate::i18n::message_id!("help-hint-navigation-pan-the-camera")
        }
        ScrollBehavior::Zoom => {
            crate::i18n::message_id!("help-hint-navigation-zoom-toward-the-pointer")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        contextual_line, contextual_line_key, HintContext, ALL_SECTIONS, TRACKPAD_SCROLL_GESTURE,
    };
    use crate::app_settings::ScrollBehavior;

    const CONTEXTS_UNDER_TEST: &[HintContext] = &[
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

    #[test]
    fn registered_contextual_lines_are_pinned_to_the_catalog() {
        #![allow(clippy::expect_used)]
        let catalog = crate::i18n::catalog::Catalog::build("en").expect("en builds");
        for behavior in [ScrollBehavior::Pan, ScrollBehavior::Zoom] {
            for context in CONTEXTS_UNDER_TEST {
                assert_eq!(
                    catalog
                        .text(contextual_line_key(*context, behavior).as_str())
                        .as_deref(),
                    Some(contextual_line(*context, behavior)),
                    "{context:?} hint line drifted from its catalog entry"
                );
            }
        }
    }

    #[test]
    fn registered_contexts_have_a_contextual_line() {
        for behavior in [ScrollBehavior::Pan, ScrollBehavior::Zoom] {
            for context in CONTEXTS_UNDER_TEST {
                assert!(!contextual_line(*context, behavior).is_empty());
            }
        }
    }

    #[test]
    fn navigation_help_matches_trackpad_scroll_support() {
        #![allow(clippy::expect_used)]
        let macos = cfg!(target_os = "macos");
        let navigation = ALL_SECTIONS
            .iter()
            .find(|section| section.key.as_str() == "help-section-navigation")
            .expect("navigation section exists");
        assert_eq!(
            navigation
                .rows
                .iter()
                .any(|row| row.gesture == TRACKPAD_SCROLL_GESTURE),
            macos
        );

        let catalog = crate::i18n::catalog::Catalog::build("en").expect("en builds");
        let behavior = ScrollBehavior::Pan;
        let line = catalog
            .text(contextual_line_key(HintContext::Navigation, behavior).as_str())
            .expect("navigation hint line exists");
        assert_eq!(line.contains("Trackpad scroll pan"), macos);
        assert_eq!(
            contextual_line(HintContext::Navigation, behavior).contains("Trackpad scroll pan"),
            macos
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_trackpad_hints_follow_the_selected_scroll_action() {
        #![allow(clippy::expect_used)]
        use super::{macos_navigation_line, macos_navigation_line_key};

        let catalog = crate::i18n::catalog::Catalog::build("en").expect("en builds");
        let navigation = ALL_SECTIONS
            .iter()
            .find(|section| section.key.as_str() == "help-section-navigation")
            .expect("navigation section exists");
        let trackpad = navigation
            .rows
            .iter()
            .find(|row| row.gesture == TRACKPAD_SCROLL_GESTURE)
            .expect("macOS navigation includes the trackpad row");
        for (behavior, action_key, line_key, line) in [
            (
                ScrollBehavior::Pan,
                crate::i18n::message_id!("help-hint-navigation-pan-the-camera"),
                crate::i18n::message_id!("help-hintline-navigation-macos-pan"),
                macos_navigation_line(ScrollBehavior::Pan),
            ),
            (
                ScrollBehavior::Zoom,
                crate::i18n::message_id!("help-hint-navigation-zoom-toward-the-pointer"),
                crate::i18n::message_id!("help-hintline-navigation-macos-zoom"),
                macos_navigation_line(ScrollBehavior::Zoom),
            ),
        ] {
            assert_eq!(trackpad.action_key(behavior), action_key);
            assert_eq!(macos_navigation_line_key(behavior), line_key);
            assert_eq!(catalog.text(line_key.as_str()).as_deref(), Some(line));
            assert_eq!(
                catalog
                    .text(trackpad.action_key(behavior).as_str())
                    .as_deref(),
                Some(match behavior {
                    ScrollBehavior::Pan => "Pan the camera",
                    ScrollBehavior::Zoom => "Zoom toward the pointer",
                })
            );
        }
    }
}
