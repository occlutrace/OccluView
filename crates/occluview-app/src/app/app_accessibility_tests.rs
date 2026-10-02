#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::app::information_dialog::InformationDialog;
use crate::app::settings::panel::settings_popup_id;
use crate::contact::contact::ContactMode;
use crate::i18n::preference::UiLanguagePreference;
use crate::measure::measure_tool::MeasureMode;
use crate::mesh_editor::mesh_editor_overlay::EditorTab;
use eframe::egui;
use std::collections::{HashMap, HashSet};

fn viewport() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1200.0))
}

#[derive(Debug)]
struct AccessibleControl {
    name: String,
    role: String,
    disabled: bool,
    toggled: Option<bool>,
}

const INTERACTIVE_ROLES: &[&str] = &[
    "Button",
    "DefaultButton",
    "CheckBox",
    "RadioButton",
    "RadioGroup",
    "ComboBox",
    "EditableComboBox",
    "DisclosureTriangle",
    "Slider",
    "SpinButton",
    "TextInput",
    "MultilineTextInput",
    "SearchInput",
    "DateInput",
    "DateTimeInput",
    "WeekInput",
    "MonthInput",
    "TimeInput",
    "EmailInput",
    "NumberInput",
    "PasswordInput",
    "PhoneNumberInput",
    "UrlInput",
    "Link",
    "ColorWell",
    "Switch",
    "MenuItem",
    "MenuItemCheckBox",
    "MenuItemRadio",
    "MenuListOption",
    "ListBoxOption",
    "TreeItem",
    "ScrollBar",
    "Tab",
    "Splitter",
];
const INTERACTIVE_ACTIONS: &[egui::accesskit::Action] = &[
    egui::accesskit::Action::Click,
    egui::accesskit::Action::Focus,
    egui::accesskit::Action::Blur,
    egui::accesskit::Action::Collapse,
    egui::accesskit::Action::Expand,
    egui::accesskit::Action::CustomAction,
    egui::accesskit::Action::Decrement,
    egui::accesskit::Action::Increment,
    egui::accesskit::Action::ReplaceSelectedText,
    egui::accesskit::Action::SetTextSelection,
    egui::accesskit::Action::SetValue,
    egui::accesskit::Action::ShowContextMenu,
];

fn controls(output: &egui::FullOutput, surface: &str) -> Vec<AccessibleControl> {
    let update = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap_or_else(|| panic!("{surface}: egui did not build an AccessKit tree"));
    let nodes: HashMap<_, _> = update
        .nodes
        .iter()
        .map(|(node_id, node)| (*node_id, node))
        .collect();
    let root = update
        .tree
        .as_ref()
        .unwrap_or_else(|| panic!("{surface}: the AccessKit tree has no root"))
        .root;
    let mut pending = vec![root];
    let mut visited = HashSet::new();
    let mut controls = Vec::new();
    while let Some(node_id) = pending.pop() {
        assert!(
            visited.insert(node_id),
            "{surface}: the AccessKit tree repeats node {node_id:?}"
        );
        let node = nodes
            .get(&node_id)
            .copied()
            .unwrap_or_else(|| panic!("{surface}: the AccessKit tree omits node {node_id:?}"));
        pending.extend(node.children().iter().copied());
        let role = format!("{:?}", node.role());
        // A fitting scroll area publishes a disabled scrollbar with no action.
        if is_interactive(node, &role) {
            controls.push(AccessibleControl {
                name: accessible_name(node, &nodes),
                role,
                disabled: node.is_disabled(),
                toggled: node
                    .toggled()
                    .map(|toggled| format!("{toggled:?}") == "True"),
            });
        }
    }
    assert_eq!(
        visited.len(),
        nodes.len(),
        "{surface}: every node in the AccessKit update must be reachable from its root"
    );
    let unnamed: Vec<_> = controls
        .iter()
        .filter(|control| control.name.trim().is_empty())
        .collect();
    assert!(
        unnamed.is_empty(),
        "{surface}: unnamed controls: {unnamed:#?}"
    );
    assert!(
        !controls.is_empty(),
        "{surface}: no interactive controls rendered"
    );
    controls
}

fn is_interactive(node: &egui::accesskit::Node, role: &str) -> bool {
    let role_is_interactive =
        INTERACTIVE_ROLES.contains(&role) && !(role == "ScrollBar" && node.is_disabled());
    let action_is_interactive = !node.is_disabled()
        && INTERACTIVE_ACTIONS
            .iter()
            .any(|action| node.supports_action(*action));
    role_is_interactive || action_is_interactive
}

fn accessible_name(
    node: &egui::accesskit::Node,
    nodes: &HashMap<egui::accesskit::NodeId, &egui::accesskit::Node>,
) -> String {
    if let Some(label) = node.label().filter(|label| !label.trim().is_empty()) {
        return label.to_owned();
    }
    if node.role() == egui::accesskit::Role::Label {
        return node
            .value()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_default()
            .to_owned();
    }

    node.labelled_by()
        .iter()
        .filter_map(|label_id| {
            nodes
                .get(label_id)
                .filter(|label_node| label_node.role() == egui::accesskit::Role::Label)
                .and_then(|label_node| label_node.value())
                .filter(|label| !label.trim().is_empty())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn a_control_value_does_not_substitute_for_its_accessible_name() {
    let mut slider = egui::accesskit::Node::new(egui::accesskit::Role::Slider);
    slider.set_value("50");

    assert!(accessible_name(&slider, &HashMap::new()).trim().is_empty());
}

fn assert_role(controls: &[AccessibleControl], name: &str, role: &str) {
    assert!(
        controls
            .iter()
            .any(|control| control.name == name && control.role == role),
        "missing {role} {name:?}; controls={controls:#?}"
    );
}

fn assert_toggled(controls: &[AccessibleControl], name: &str) {
    let control = controls
        .iter()
        .find(|control| control.name == name && control.toggled.is_some())
        .unwrap_or_else(|| panic!("missing toggle {name:?}; controls={controls:#?}"));
    assert_eq!(
        control.toggled,
        Some(true),
        "{name:?} must expose its active state"
    );
}

fn assert_disabled(controls: &[AccessibleControl], name: &str) {
    let control = controls
        .iter()
        .find(|control| control.name == name)
        .unwrap_or_else(|| panic!("missing disabled control {name:?}; controls={controls:#?}"));
    assert!(control.disabled, "{name:?} must be disabled");
}

fn app_with_scene(ctx: &egui::Context) -> OccluViewApp {
    let mut app = OccluViewApp::new_for_tests(ctx.clone());
    app.workspace.scenes[0].document.scene =
        Some(app_test_support::named_scene("Upper arch", 0.0).into());
    app
}

fn input() -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(viewport()),
        ..Default::default()
    }
}

#[test]
fn viewer_surfaces_publish_named_controls_with_roles_and_toggle_states() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = app_with_scene(&ctx);
    app.persistence.recent_files.push("prior-case.stl");
    app.workspace.scenes[0]
        .tools
        .measure
        .arm(MeasureMode::Ruler);
    app.workspace.scenes[0].tools.align.brush.set_armed(true);
    egui::Popup::open_id(&ctx, settings_popup_id());

    let mut output = ctx.run_ui(input(), |ui| {
        let render_ctx = ui.ctx().clone();
        app.active_context()
            .expect("live test scene")
            .show_toolbar(ui);
        app.active_context()
            .expect("live test scene")
            .show_layers_overlay(ui, viewport(), &render_ctx);
        app.active_context()
            .expect("live test scene")
            .show_align_panel(&render_ctx, viewport());
        app.active_context()
            .expect("live test scene")
            .show_ruler_options(&render_ctx, viewport());
    });
    output.textures_delta.clear();
    let controls = controls(&output, "toolbar, layers, align, ruler strip, and settings");

    for name in [
        "Open",
        "Recent files",
        "Add",
        "Cut View",
        "Ruler",
        "Thickness",
        "Align",
        "Edit",
        "Settings",
        "Upper arch",
        "Layers: Upper arch",
        "Hide layer: Upper arch",
        "Layer opacity: Upper arch",
        "Choose tint: Upper arch",
        "Remove layer: Upper arch",
        "Align scans",
        "Adjust pose",
        "1. Perform alignment",
        "2. Best fit matching",
        "matching parts",
        "max influence",
        "Close the brush — the markings are kept",
        "brush size",
        "automatic radius",
        "Any angle",
        "90°",
        "UI scale",
        "Orbit speed",
        "Zoom speed",
        "Check now",
        "Keyboard shortcuts",
        "About OccluView",
    ] {
        assert!(
            controls.iter().any(|control| control.name == name),
            "missing accessible control {name:?}; controls={controls:#?}"
        );
    }
    for name in [
        "Open",
        "Recent files",
        "Add",
        "Cut View",
        "Ruler",
        "Thickness",
        "Align",
        "Edit",
        "Settings",
    ] {
        assert_role(&controls, name, "Button");
    }
    assert_role(&controls, "matching parts", "Slider");
    assert_role(&controls, "max influence", "Slider");
    assert_role(&controls, "matching parts", "SpinButton");
    assert_role(&controls, "max influence", "SpinButton");
    assert_role(&controls, "UI scale", "Slider");
    assert_role(&controls, "Orbit speed", "Slider");
    assert_role(&controls, "Zoom speed", "Slider");
    assert_role(&controls, "Any angle", "Button");
    assert_role(&controls, "90°", "Button");
    assert_toggled(&controls, "Ruler");
    assert_toggled(&controls, "Settings");
    assert_toggled(&controls, "Align scans");
    assert!(
        controls.iter().any(
            |control| ["Any angle", "90°"].contains(&control.name.as_str())
                && control.toggled == Some(true)
        ),
        "the ruler strip must expose the active angle rule: {controls:#?}"
    );
}

#[test]
fn localized_toolbar_controls_fit_narrow_windows_without_overlapping() {
    for tag in ["en", "ru", "de", "es", "fr", "it", "pt-BR"] {
        for width in [400.0, 600.0, 900.0, 1600.0] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut app = app_with_scene(&ctx);
            app.ui
                .locale
                .set_preference(UiLanguagePreference::Explicit(tag));
            app.workspace.scenes[0]
                .tools
                .measure
                .arm(MeasureMode::Ruler);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    app.active_context().expect("active scene").show_toolbar(ui);
                },
            );
            output.textures_delta.clear();
            let named = controls(&output, "responsive toolbar");
            for key in [
                crate::i18n::message_id!("toolbar-open-label"),
                crate::i18n::message_id!("toolbar-add-label"),
                crate::i18n::message_id!("toolbar-cut-label"),
                crate::i18n::message_id!("toolbar-ruler-label"),
                crate::i18n::message_id!("toolbar-thickness-label"),
                crate::i18n::message_id!("toolbar-align-label"),
                crate::i18n::message_id!("toolbar-edit-label"),
                crate::i18n::message_id!("toolbar-settings-label"),
            ] {
                assert_role(&named, &app.ui.locale.tr(key), "Button");
            }
            let update = output
                .platform_output
                .accesskit_update
                .as_ref()
                .expect("accessibility");
            let mut bounds: Vec<_> = update
                .nodes
                .iter()
                .filter_map(|(_, node)| {
                    matches!(
                        node.role(),
                        egui::accesskit::Role::Button | egui::accesskit::Role::MenuItem
                    )
                    .then(|| node.bounds())
                    .flatten()
                })
                .collect();
            assert!(!bounds.is_empty());
            bounds.sort_by(|a, b| a.x0.total_cmp(&b.x0));
            for rect in &bounds {
                assert!(
                    rect.x0 >= 0.0 && rect.x1 <= f64::from(width),
                    "{tag}, {width}: outside viewport: {rect:?}"
                );
            }
            for pair in bounds.windows(2) {
                assert!(
                    pair[0].x1 <= pair[1].x0 + 0.5,
                    "{tag}, {width}: toolbar overlap: {pair:?}"
                );
            }
        }
    }
}

#[test]
fn unavailable_toolbar_commands_expose_disabled_state() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = OccluViewApp::new_for_tests(ctx.clone());
    let mut output = ctx.run_ui(input(), |ui| {
        app.active_context()
            .expect("live test scene")
            .show_toolbar(ui);
    });
    output.textures_delta.clear();
    let controls = controls(&output, "empty-scene toolbar");

    for name in [
        "Recent files",
        "Cut View",
        "Ruler",
        "Thickness",
        "Align",
        "Edit",
    ] {
        assert_disabled(&controls, name);
    }
    assert!(
        controls
            .iter()
            .any(|control| control.name == "Add" && !control.disabled),
        "an empty scene must accept added files"
    );
}

#[test]
fn contact_strip_controls_publish_names_roles_and_selected_state() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = app_with_scene(&ctx);
    let layer_id = app.workspace.scenes[0]
        .document
        .scene
        .as_ref()
        .and_then(|scene| scene.meshes().first())
        .map(SceneMesh::id)
        .expect("the contact strip has a scene layer");
    app.workspace.scenes[0]
        .tools
        .contacts
        .open(crate::contact::contact::ContactPair {
            subject: layer_id,
            antagonist: layer_id,
        });
    let names = [
        (ContactMode::Marks.label_key(), "contact mode", "Button"),
        (ContactMode::Approach.label_key(), "contact mode", "Button"),
        (
            crate::i18n::message_id!("contact-load-label"),
            "contact load",
            "Slider",
        ),
        (
            crate::i18n::message_id!("contact-load-label"),
            "contact load value",
            "SpinButton",
        ),
        (
            crate::i18n::message_id!("contact-details"),
            "contact details",
            "Button",
        ),
        (
            crate::i18n::message_id!("contact-close-hint"),
            "contact close",
            "Button",
        ),
    ];
    let labels: Vec<_> = names
        .iter()
        .map(|(message, _, _)| app.ui.locale.tr(*message))
        .collect();
    let mut output = ctx.run_ui(input(), |ui| {
        let render_ctx = ui.ctx().clone();
        app.active_context()
            .expect("live test scene")
            .show_contact_bar(ui, viewport(), &render_ctx);
    });
    output.textures_delta.clear();
    let controls = controls(&output, "contact strip");

    for ((_, purpose, role), label) in names.iter().zip(&labels) {
        assert_role(&controls, label, role);
        assert!(
            controls.iter().any(|control| control.name == *label),
            "missing {purpose} control {label:?}; controls={controls:#?}"
        );
    }
    assert_toggled(&controls, &labels[0]);
}

#[test]
fn mesh_editor_and_sculpt_controls_publish_names_roles_and_selected_tabs() {
    for tab in [EditorTab::EditMesh, EditorTab::Sculpt] {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = app_with_scene(&ctx);
        let scene = app.workspace.scenes[0]
            .document
            .scene
            .as_ref()
            .unwrap()
            .clone();
        assert!(app.workspace.scenes[0]
            .document
            .edit_mode
            .begin_face_selection(&scene.meshes()[0], &scene));
        app.workspace.scenes[0].tools.editor_tab = tab;

        let mut output = ctx.run_ui(input(), |ui| {
            app.active_context()
                .expect("live test scene")
                .show_mesh_editor_overlay(viewport(), ui.ctx());
        });
        output.textures_delta.clear();
        let controls = controls(&output, "mesh editor and sculpt panel");
        let selected = if tab == EditorTab::EditMesh {
            "Mesh Editing"
        } else {
            "Sculpt"
        };
        assert_role(&controls, "Mesh Editing", "Button");
        assert_role(&controls, "Sculpt", "Button");
        assert_toggled(&controls, selected);
        for name in [
            "Cancel the session (edits are reverted)",
            "Undo",
            "Redo",
            "Cancel",
            "Done",
        ] {
            assert!(
                controls.iter().any(|control| control.name == name),
                "missing editor control {name:?}; controls={controls:#?}"
            );
        }
        match tab {
            EditorTab::EditMesh => {
                for name in [
                    "Lasso",
                    "Object",
                    "Surface",
                    "Through",
                    "All",
                    "None",
                    "Invert",
                    "Delete",
                    "Crop",
                    "Cut",
                    "Separate",
                    "Close holes",
                ] {
                    assert!(
                        controls.iter().any(|control| control.name == name),
                        "missing mesh-edit control {name:?}; controls={controls:#?}"
                    );
                }
                assert_role(&controls, "limit", "CheckBox");
                assert_role(&controls, "Maximum perimeter", "SpinButton");
            }
            EditorTab::Sculpt => {
                assert_role(&controls, "Add / Remove  [1]", "Button");
                assert_role(&controls, "Smooth  [2]", "Button");
                assert_role(&controls, "size", "Slider");
                assert_role(&controls, "force", "Slider");
            }
        }
    }
}

#[test]
fn layer_context_menu_rows_publish_names_roles_and_disabled_state() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let scene = app_test_support::named_scene("Upper arch", 0.0);
    let entry = &scene.meshes()[0];
    let target = layers_overlay::LayerContextMenuTarget {
        label: "Upper arch".to_owned(),
        index: 0,
        layer_id: entry.id(),
        visible: true,
        wireframe: true,
        face_editable: true,
        can_export: true,
        show_vertex_colors: false,
        show_texture: false,
        has_color_data: false,
        has_texture: false,
        contacts: false,
        can_read_contacts: false,
    };
    let locale = crate::i18n::LocaleManager::for_tests();
    let mut request = None;
    let mut output = ctx.run_ui(input(), |ui| {
        let render_ctx = ui.ctx().clone();
        egui::Area::new(egui::Id::new("accessibility-layer-menu"))
            .fixed_pos(egui::Pos2::new(20.0, 20.0))
            .show(&render_ctx, |ui| {
                layers_overlay::show_layer_context_menu(ui, &target, &mut request, &locale);
            });
    });
    output.textures_delta.clear();
    let controls = controls(&output, "layer context menu");

    for name in [
        "Next tint",
        "Show scan colors",
        "Mesh Editing",
        "Split bridge…",
        "Mesh Repair",
        "Flip normals",
        "Export layer…",
        "Show contacts",
        "Hide wireframe",
        "Remove layer",
    ] {
        assert_role(&controls, name, "Button");
    }
    assert_disabled(&controls, "Show scan colors");
    assert_disabled(&controls, "Show contacts");
    assert_toggled(&controls, "Hide wireframe");
}

#[test]
fn information_and_error_dialog_controls_have_accessible_roles_and_names() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = OccluViewApp::new_for_tests(ctx.clone());
    app.ui.information_dialog = InformationDialog::About;
    let mut about = ctx.run_ui(input(), |ui| {
        let render_ctx = ui.ctx().clone();
        app.active_context()
            .expect("live test scene")
            .show_information_dialog(&render_ctx);
    });
    about.textures_delta.clear();
    let about_controls = controls(&about, "About dialog");
    for name in ["Website", "Source", "Third-party licenses"] {
        assert_role(&about_controls, name, "Link");
    }
    assert_role(&about_controls, "Close", "Button");

    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = OccluViewApp::new_for_tests(ctx.clone());
    app.ui.app_error = Some(AppErrorDialog {
        title: app
            .ui
            .locale
            .tr(crate::i18n::message_id!("error-open-title")),
        summary: app
            .ui
            .locale
            .tr(crate::i18n::message_id!("load-loader-failed-summary")),
        details: "GPU adapter unavailable".to_owned(),
        action: AppErrorAction::RetryGraphics,
    });
    let mut error = ctx.run_ui(input(), |ui| {
        let render_ctx = ui.ctx().clone();
        app.active_context()
            .expect("live test scene")
            .show_error_dialog(&render_ctx);
    });
    error.textures_delta.clear();
    let error_controls = controls(&error, "graphics error dialog");
    assert_role(&error_controls, "Error details", "TextInput");
    assert_disabled(&error_controls, "Error details");
    for name in ["Close", "Copy Details", "Try again"] {
        assert_role(&error_controls, name, "Button");
    }
}

#[test]
fn accessibility_names_follow_the_active_locale() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = app_with_scene(&ctx);
    app.ui
        .locale
        .set_preference(UiLanguagePreference::Explicit("ru"));
    let mut output = ctx.run_ui(input(), |ui| {
        app.active_context()
            .expect("live test scene")
            .show_toolbar(ui);
    });
    output.textures_delta.clear();
    let controls = controls(&output, "Russian toolbar");

    assert_role(&controls, "Открыть", "Button");
    assert_role(&controls, "Настройки", "Button");
}

#[test]
fn layer_tint_palette_names_are_translated_in_all_embedded_locales() {
    let expected = [
        ("en", "Cobalt"),
        ("de", "Kobalt"),
        ("es", "Cobalto"),
        ("fr", "Cobalt"),
        ("it", "Cobalto"),
        ("pt-BR", "Cobalto"),
        ("ru", "Кобальт"),
    ];
    for (tag, value) in expected {
        let mut locale = crate::i18n::LocaleManager::for_tests();
        locale.set_preference(UiLanguagePreference::Explicit(tag));
        assert_eq!(
            locale.tr(crate::i18n::message_id!("tint-color-cobalt")),
            value,
            "locale {tag}"
        );
    }
}
