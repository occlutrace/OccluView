#![allow(clippy::expect_used)]

use super::AlignTab;

/// The kept English tab labels render from the catalog verbatim.
#[test]
fn english_tab_labels_match_source_wording() {
    let catalog = crate::i18n::catalog::Catalog::build("en").expect("en builds");
    for tab in [AlignTab::Automatically, AlignTab::Manually] {
        assert_eq!(catalog.text(tab.label_key()).as_deref(), Some(tab.label()));
    }
}

fn production() -> &'static str {
    let source = crate::primary_ui_tests::production_source(include_str!("align_panel.rs"));
    source
        .split_once("\n#[cfg(test)]")
        .map_or(source, |(before, _)| before)
}

/// The whole point of this tool is that there is no object picker. If a
/// control ever names a target or a role, the simplification is gone.
#[test]
fn no_control_in_the_window_names_a_target_a_source_or_a_role() {
    for literal in production().split('"').skip(1).step_by(2) {
        let lowered = literal.to_lowercase();
        for banned in ["target", "source object", "primary object", "role"] {
            assert!(
                !lowered.contains(banned),
                "a control says {literal:?}, which names {banned}"
            );
        }
    }
}

/// The window has to be draggable like the mesh editor: a panel pinned to a
/// corner covers the very geometry the operator is clicking on.
#[test]
fn the_window_is_movable_and_constrained_to_the_viewport() {
    let source = production();
    assert!(source.contains("align-panel-title"));
    assert!(source.contains(".default_pos(default_pos)"));
    assert!(source.contains(".constrain_to(viewport_rect)"));
    assert!(
        !source.contains(".anchor("),
        "an anchored window cannot be moved"
    );
}

/// The window exposes explicit Cancel and Done actions.
#[test]
fn the_window_ends_in_cancel_and_done() {
    let commit = production()
        .split_once("fn commit(")
        .map(|(_, rest)| rest)
        .expect("a commit row");
    assert!(commit.contains("AlignPanelAction::Cancel"));
    assert!(commit.contains("AlignPanelAction::Done"));
}

/// Preserve the established control labels.
#[test]
fn the_controls_carry_the_labels_operators_already_know() {
    let source = format!(
        "{}{}",
        production(),
        crate::primary_ui_tests::production_source(include_str!("align_panel_settings.rs"))
    );
    for label in [
        "\"align-back\"",
        "\"align-fit-perform\"",
        "\"align-fit-refine\"",
        "\"align-matching-parts\"",
        "\"align-max-influence\"",
        "\"align-orientation-match\"",
        "\"align-orientation-inverted\"",
        "\"align-orientation-ignored\"",
        "\"align-exclude\"",
    ] {
        assert!(source.contains(label), "the window is missing {label}");
    }
}

/// The exclusion brush belongs to the automatic tab.
#[test]
fn the_exclusion_brush_belongs_to_the_automatic_tab() {
    let source = production();
    let manual = source
        .split_once("fn manually(")
        .and_then(|(_, rest)| rest.split_once("\n/// What the tool is waiting for"))
        .map(|(block, _)| block)
        .expect("a manual tab body");
    for absent in ["excluding", "brush", "Brush"] {
        assert!(
            !manual.contains(absent),
            "the manual tab mentions {absent}, which belongs to the automatic tab"
        );
    }
    let automatic = source
        .split_once("fn automatically(")
        .and_then(|(_, rest)| rest.split_once("\n/// The Manually tab"))
        .map(|(block, _)| block)
        .expect("an automatic tab body");
    assert!(automatic.contains("exclude(ui, view.excluding, enabled, locale)"));
}

/// The manual tab exposes Undo and Redo.
#[test]
fn the_manual_tab_offers_the_history_buttons() {
    let manual = production()
        .split_once("fn manually(")
        .map(|(_, rest)| rest)
        .expect("a manual tab body");
    assert!(manual.contains("AlignPanelAction::Undo"));
    assert!(manual.contains("AlignPanelAction::Redo"));
}

/// The map target is fixed by the measurement model.
#[test]
fn the_window_never_asks_which_surface_carries_the_map() {
    let source = production();
    for gone in ["SwapMapped", "AppIcon::Swap", "other scan instead"] {
        assert!(!source.contains(gone), "{gone} is back in the window");
    }
}

#[test]
fn custom_align_controls_show_keyboard_focus() {
    let source = production();
    for control in ["tab_strip", "chip", "fit_button"] {
        let body = source
            .split_once(&format!("fn {control}("))
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        assert!(
            body.contains("response.has_focus()"),
            "{control} is focusable but has no visible focus treatment"
        );
    }
}

#[test]
fn custom_align_controls_publish_accessible_button_roles() {
    let source = production();
    for control in ["tab_strip", "chip", "fit_button"] {
        let body = source
            .split_once(&format!("fn {control}("))
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        assert!(
            body.contains("response.widget_info") && body.contains("egui::WidgetType::Button"),
            "{control} must expose a semantic button to AccessKit"
        );
    }
}

#[test]
fn compact_constraint_chips_keep_visual_labels_compact_but_semantic_names() {
    let source = production();
    let compact = source
        .split_once("fn compact_icon_chip(")
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    assert!(
        compact.contains("accessibility_label")
            && compact.contains("chip_with_accessibility")
            && compact.contains("\"\""),
        "constraint chips need an icon-only visual and a non-empty semantic label"
    );
    assert!(
        source.contains("&locale.tr(value.label_key())"),
        "each constraint chip must provide its localized semantic name"
    );
}
