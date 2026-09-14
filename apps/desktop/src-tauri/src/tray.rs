//! The tray icon.
//!
//! A monitoring app spends most of its life minimised, so the tray is the real
//! home screen: show/hide the window, force a rescan, open the config folder and
//! quit. Closing the window hides it (when `close_to_tray` is on) instead of
//! ending the runtime, which is what makes "minimise to tray" meaningful.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

use crate::state::AppState;

/// Menu item ids.
pub const MENU_SHOW: &str = "show";
pub const MENU_HIDE: &str = "hide";
pub const MENU_RESCAN: &str = "rescan";
pub const MENU_OPEN_CONFIG: &str = "open-config";
pub const MENU_QUIT: &str = "quit";

/// Install the tray icon and its menu.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, MENU_SHOW, "Show OpenHardwareOS", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, MENU_HIDE, "Hide window", true, None::<&str>)?;
    let rescan = MenuItem::with_id(app, MENU_RESCAN, "Rescan hardware", true, None::<&str>)?;
    let open_config = MenuItem::with_id(
        app,
        MENU_OPEN_CONFIG,
        "Open config folder",
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &show,
            &hide,
            &separator,
            &rescan,
            &open_config,
            &separator,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("openhardwareos")
        .menu(&menu)
        .tooltip("OpenHardwareOS")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    tracing::debug!("tray icon installed");
    Ok(())
}

fn handle_menu<R: Runtime>(app: &AppHandle<R>, id: &str) {
    match id {
        MENU_SHOW => show_window(app),
        MENU_HIDE => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
        }
        MENU_RESCAN => {
            let state = app.state::<AppState>();
            let runtime = state.runtime.clone();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = runtime.refresh_devices().await {
                    tracing::warn!(error = %error, "tray rescan failed");
                }
                let _ = runtime.poll_once().await;
                crate::events::emit_snapshot(&app, &runtime);
            });
        }
        MENU_OPEN_CONFIG => {
            let state = app.state::<AppState>();
            let path = state.paths.root().to_path_buf();
            if let Err(error) = crate::shell::open_path(app, &path) {
                tracing::warn!(error = %error, "could not open the config folder");
            }
        }
        MENU_QUIT => {
            let state = app.state::<AppState>();
            let runtime = state.runtime.clone();
            let engine = state.engine.clone();
            let app = app.clone();
            // Shut down in the background so the UI stays responsive while
            // control is handed back to the firmware.
            tauri::async_runtime::spawn(async move {
                engine.stop().await;
                match runtime.shutdown().await {
                    Ok(release) => {
                        for problem in release.problems() {
                            tracing::warn!(problem, "control was not fully released");
                        }
                        for caveat in release.caveats() {
                            tracing::warn!(caveat, "control hand-back is limited");
                        }
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "runtime shutdown reported a problem")
                    }
                }
                app.exit(0);
            });
        }
        _ => {}
    }
}

/// Bring the window back, restoring it if it was minimised.
pub fn show_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_are_stable() {
        assert_eq!(MENU_SHOW, "show");
        assert_eq!(MENU_QUIT, "quit");
        assert_ne!(MENU_SHOW, MENU_HIDE);
    }
}
