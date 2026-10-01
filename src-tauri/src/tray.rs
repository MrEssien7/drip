// Notification-area icon: Show, Install hooks…, Uninstall hooks, Quit.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter};

use crate::island::WINDOW_LABEL;

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let install = MenuItem::with_id(app, "install", "Install Claude Code hooks…", true, None::<&str>)?;
    let uninstall = MenuItem::with_id(app, "uninstall", "Uninstall hooks", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &sep1, &install, &uninstall, &sep2, &quit])?;

    TrayIconBuilder::with_id("drip")
        .icon(crate::icon::tray_icon())
        .tooltip("Drip")
        .menu(&menu)
        .on_menu_event(|app: &AppHandle, event| match event.id.as_ref() {
            "show" => {
                let _ = app.emit_to(WINDOW_LABEL, "show", ());
            }
            "install" => crate::open_installer(app, true),
            "uninstall" => crate::open_installer(app, false),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}
