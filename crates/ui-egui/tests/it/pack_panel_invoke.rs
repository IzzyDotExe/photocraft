//! The Pack Explorer panel opens through the Window menu, in legacy and customized workspaces.

use photocraft_ui_egui::control::{ControlRequest, handle};
use photocraft_ui_egui::PhotocraftApp;
use serde_json::{Value, json};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

/// A control request as the control channel sends it, then the frame's sync.
fn control(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let ctx = egui::Context::default();
    let (req, rx) = ControlRequest::new(method, params);
    handle(app, &ctx, &req);
    app.sync_views();
    rx.try_recv().unwrap_or(Value::Null)
}

/// The Window menu's Pack Explorer row, if the catalog lists it.
fn pack_row(app: &PhotocraftApp) -> photocraft_ui_egui::menus::MenuItem {
    photocraft_ui_egui::menus::menu_items(app)
        .into_iter()
        .find(|i| i.id == "window.panel.pack")
        .expect("the Window menu lists Pack Explorer")
}

#[test]
fn pack_panel_opens_from_the_window_menu() {
    let mut app = app();
    // The menu item is live, enabled, unchecked, under Window.
    let row = pack_row(&app);
    assert!(row.enabled, "Pack Explorer starts enabled");
    assert_eq!(row.checked, Some(false), "the panel starts closed");
    assert_eq!(row.path.first().map(String::as_str), Some("Window"));

    // Invoking the item migrates the legacy dock into the workspace with the Pack tab on it.
    let reply = control(&mut app, "ui.menu.invoke", json!({"id": "window.panel.pack"}));
    assert_eq!(reply, Value::Null, "the menu item runs without error");
    assert!(app.ui.panels.pack, "the Pack panel should be shown");
    let workspace = app.ui.docking.as_ref().expect("a customized workspace after a dock command");
    assert!(workspace.layout.contains(&"pack".to_string()), "the layout should contain the pack panel");

    // The menu's checkmark tracks the panel.
    assert_eq!(pack_row(&app).checked, Some(true));
}

#[test]
fn pack_panel_closes_again() {
    let mut app = app();
    control(&mut app, "ui.menu.invoke", json!({"id": "window.panel.pack"}));
    assert!(app.ui.panels.pack);
    control(&mut app, "ui.menu.invoke", json!({"id": "window.panel.pack"}));
    assert!(!app.ui.panels.pack, "a second invoke closes the panel again");
    assert_eq!(pack_row(&app).checked, Some(false));
}
