#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::app::app_settings_panel::settings_popup_id;
use crate::app::information_dialog::InformationDialog;
use crate::i18n::preference::UiLanguagePreference;
use crate::measure_tool::MeasureMode;
use crate::mesh_editor_overlay::EditorTab;
use eframe::egui;

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

fn controls(output: &egui::FullOutput, surface: &str) -> Vec<AccessibleControl> {
    const INTERACTIVE_ROLES: &[&str] = &[
        "Button",
        "CheckBox",
        "RadioButton",
        "RadioGroup",
        "ComboBox",
        "Slider",
        "SpinButton",
        "TextInput",
        "Link",
        "ColorWell",
    ];
    let update = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap_or_else(|| panic!("{surface}: egui did not build an AccessKit tree"));
    let controls: Vec<_> = update
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            let role = format!("{:?}", node.role());
            INTERACTIVE_ROLES
                .contains(&role.as_str())
                .then(|| AccessibleControl {
                    name: accessible_name(node, &update.nodes),
                    role,
                    disabled: node.is_disabled(),
                    toggled: node
                        .toggled()
                        .map(|toggled| format!("{toggled:?}") == "True"),
                })
        })
        .collect();
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

fn accessible_name(
    node: &egui::accesskit::Node,
    nodes: &[(egui::accesskit::NodeId, egui::accesskit::Node)],
) -> String {
    if let Some(label) = node.label().filter(|label| !label.trim().is_empty()) {
        return label.to_owned();
    }

    node.labelled_by()
        .iter()
        .filter_map(|label_id| {
            nodes
                .iter()
                .find(|(node_id, _)| node_id == label_id)
                .and_then(|(_, label_node)| label_node.value())
        })
        .collect::<Vec<_>>()
        .join(" ")
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
        .find(|control| control.name == name)
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
    app.document.scene = Some(app_test_support::named_scene("Upper arch", 0.0).into());
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
    app.tools.measure.arm(MeasureMode::Ruler);
    app.tools.align.brush.set_armed(true);
    egui::Popup::open_id(&ctx, settings_popup_id());

    let mut output = ctx.run_ui(input(), |ui| {
        let render_ctx = ui.ctx().clone();
        app.show_toolbar(ui);
        app.show_layers_overlay(ui, viewport(), &render_ctx);
        app.show_align_panel(&render_ctx, viewport());
        app.show_ruler_options(&render_ctx, viewport());
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
fn unavailable_toolbar_commands_expose_disabled_state() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = OccluViewApp::new_for_tests(ctx.clone());
    let mut output = ctx.run_ui(input(), |ui| app.show_toolbar(ui));
    output.textures_delta.clear();
    let controls = controls(&output, "empty-scene toolbar");

    for name in [
        "Add",
        "Recent files",
        "Cut View",
        "Ruler",
        "Thickness",
        "Align",
        "Edit",
    ] {
        assert_disabled(&controls, name);
    }
}

#[test]
fn mesh_editor_and_sculpt_controls_publish_names_roles_and_selected_tabs() {
    for tab in [EditorTab::EditMesh, EditorTab::Sculpt] {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = app_with_scene(&ctx);
        let scene = app.document.scene.as_ref().unwrap().clone();
        assert!(app
            .document
            .edit_mode
            .begin_face_selection(&scene.meshes()[0], &scene));
        app.tools.editor_tab = tab;

        let mut output = ctx.run_ui(input(), |ui| {
            app.show_mesh_editor_overlay(viewport(), ui.ctx());
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
        app.show_information_dialog(&render_ctx);
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
        title: app.ui.locale.tr("error-open-title"),
        summary: app.ui.locale.tr("load-loader-failed-summary"),
        details: "GPU adapter unavailable".to_owned(),
        action: AppErrorAction::RetryGraphics,
    });
    let mut error = ctx.run_ui(input(), |ui| {
        let render_ctx = ui.ctx().clone();
        app.show_error_dialog(&render_ctx);
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
    let mut output = ctx.run_ui(input(), |ui| app.show_toolbar(ui));
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
        assert_eq!(locale.tr("tint-color-cobalt"), value, "locale {tag}");
    }
}
