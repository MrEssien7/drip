// Drip: app wiring and the commands the front end calls.

mod agents;
mod droplet;
mod hooks;
mod icon;
mod island;
mod pipe;
mod tray;
// Shared with drip-hook; the app only needs the SID half.
#[allow(dead_code)]
#[path = "../../hook/src/win.rs"]
mod win;

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use island::WINDOW_LABEL;

const INSTALLER_LABEL: &str = "installer";

/// The island is shown once the front end has sized it for the first time.
static SHOWN: AtomicBool = AtomicBool::new(false);

/// The island's answer to a permission request.
#[tauri::command]
fn decide(app: AppHandle, id: String, choice: String) {
    pipe::decide(&app, &id, &choice);
}

/// The front end reports the size the island needs, in logical px.
#[tauri::command]
fn set_island_size(app: AppHandle, width: f64, height: f64) {
    island::place(&app, width, height);
    if !SHOWN.swap(true, Ordering::Relaxed) {
        if let Some(win) = island::window(&app) {
            let _ = win.show();
            // Showing resets the extended style, so this goes after show().
            island::hide_from_alt_tab(&win);
        }
    }
}

#[tauri::command]
fn hooks_preview(install: bool) -> Result<hooks::Preview, String> {
    hooks::preview(install)
}

#[tauri::command]
fn hooks_apply(app: AppHandle, install: bool, fingerprint: String) -> Result<Option<String>, String> {
    hooks::write(&app, install, &fingerprint)
}

pub fn open_installer(app: &AppHandle, install: bool) {
    let mode = if install { "install" } else { "uninstall" };
    if let Some(win) = app.get_webview_window(INSTALLER_LABEL) {
        let _ = win.emit("installer-mode", mode);
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }
    let url = WebviewUrl::App(format!("installer.html?mode={mode}").into());
    let built = WebviewWindowBuilder::new(app, INSTALLER_LABEL, url)
        .title(if install { "Install Claude Code hooks" } else { "Uninstall Claude Code hooks" })
        .inner_size(720.0, 640.0)
        .min_inner_size(520.0, 420.0)
        .center()
        .build();
    if let Err(err) = built {
        eprintln!("[drip] cannot open the installer window: {err}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = app.emit_to(WINDOW_LABEL, "show", ());
        }))
        .manage(pipe::Pending::default())
        .invoke_handler(tauri::generate_handler![decide, set_island_size, hooks_preview, hooks_apply])
        .setup(|app| {
            let handle = app.handle().clone();
            if let Some(win) = island::window(&handle) {
                island::hide_from_alt_tab(&win);
            }
            tray::build(&handle)?;
            pipe::start(handle.clone());
            // Keep an installed relay up to date with this build.
            if hooks::installed() {
                if let Err(err) = hooks::copy_hook_exe(&handle) {
                    eprintln!("[drip] {err}");
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Drip");
}
