//! OpenHardwareOS desktop backend.
//!
//! The desktop app is a *shell* around the runtime and the automation engine:
//!
//! ```text
//! React UI ──invoke──▶ commands.rs ──▶ Runtime ──▶ adapters ──▶ hardware
//!          ◀──event─── events.rs   ◀── RuntimeEvent bus
//! ```
//!
//! Nothing here talks to hardware. That is the architectural rule the whole
//! project is built on: the UI cannot bypass the safety policy or the audit log
//! because it has no other way to reach a fan.

pub mod autostart;
pub mod commands;
pub mod events;
pub mod shell;
pub mod state;
pub mod tray;

use ohm_adapters::AdapterOptions;
use ohm_automation::AutomationEngine;
use ohm_core::ConfigPaths;
use ohm_runtime::{Runtime, SettingsStore};
use state::AppState;
use tauri::{Emitter, Manager, WindowEvent};

/// Command line flags the desktop binary understands.
///
/// Parsed once, in one place, so the behaviour can be unit tested without
/// opening a window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StartupFlags {
    /// `--dry-run`: never write to hardware, whatever the saved settings say.
    pub dry_run: bool,
    /// `--mock`: register the simulated providers even when they are switched
    /// off in Settings.
    pub mock: bool,
    /// `--selftest`: start headlessly, run one automation cycle and exit.
    pub selftest: bool,
    /// `--ipc-selftest`: start the **real** application — real window, real webview,
    /// real IPC — and ask the frontend to exercise the command surface, then exit.
    ///
    /// Deliberately distinct from `--selftest`, which returns before the Tauri builder
    /// is constructed and therefore proves nothing about the desktop path.
    pub ipc_selftest: bool,
}

impl StartupFlags {
    /// Parse the process arguments (the program name is ignored).
    pub fn parse(args: &[String]) -> Self {
        let has = |flag: &str| args.iter().any(|arg| arg == flag);
        Self {
            dry_run: has("--dry-run"),
            mock: has("--mock"),
            selftest: has("--selftest"),
            ipc_selftest: has("--ipc-selftest"),
        }
    }

    /// `true` when any flag was given.
    pub fn is_empty(&self) -> bool {
        !(self.dry_run || self.mock || self.selftest || self.ipc_selftest)
    }
}

/// Fold the command line flags into the loaded settings.
///
/// `--dry-run` is deliberately *not* persisted: it is a per-invocation safety
/// switch, so a user can hand their machine to someone else with
/// `OpenHardwareOS --dry-run` and nothing they do in the UI will reach a fan.
pub fn apply_startup_overrides(
    mut settings: ohm_runtime::Settings,
    flags: StartupFlags,
) -> ohm_runtime::Settings {
    if flags.dry_run {
        settings.dry_run = true;
    }
    settings
}

/// Should a close request hide the window instead of ending the process?
pub fn should_hide_on_close(settings: &ohm_runtime::Settings) -> bool {
    settings.close_to_tray
}

/// Should a minimised window be hidden into the tray?
///
/// Only when the user asked for it: otherwise minimising behaves normally.
pub fn should_hide_on_minimize(settings: &ohm_runtime::Settings, minimized: bool) -> bool {
    settings.minimize_to_tray && minimized
}

/// Build and run the application.
///
/// Returns an error only when the runtime cannot be created at all (for example
/// when the config directory is not writable).
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let paths = ConfigPaths::discover()?;
    paths.ensure()?;

    let store = SettingsStore::new(&paths);
    let settings = store.load()?;
    let _log_guard = ohm_core::logging::init(settings.log_level, Some(&paths))?;

    let flags = StartupFlags::parse(&std::env::args().collect::<Vec<_>>());
    // `--dry-run` must change what the runtime *does*, not only what it reports.
    let settings = apply_startup_overrides(settings, flags);

    tracing::info!(
        version = ohm_core::VERSION,
        config = %paths.root().display(),
        dry_run = settings.dry_run,
        "starting OpenHardwareOS"
    );

    let mut options = AdapterOptions::from_settings(&settings);
    if flags.mock {
        options.mock = true;
        options.protocol_device = true;
    }
    let adapters = ohm_adapters::build_adapters(&options);
    tracing::info!(
        adapters = ?options.enabled_ids(),
        "hardware providers registered"
    );

    let runtime = Runtime::new(paths.clone(), settings.clone(), adapters)?;

    let engine = AutomationEngine::from_runtime(runtime.clone());

    if flags.selftest {
        return selftest_report(runtime, engine, paths);
    }

    let state = AppState {
        runtime: runtime.clone(),
        engine: engine.clone(),
        paths: paths.clone(),
        started_at_ms: ohm_core::now_ms(),
        dry_run_override: flags.dry_run,
    };
    if flags.ipc_selftest {
        let report = paths.root().join(commands::IPC_PROBE_FILE);
        tracing::info!(
            path = %report.display(),
            version = ohm_core::VERSION,
            "IPC self-test requested: the frontend will exercise the command surface"
        );
        commands::arm_ipc_probe(report);
    }

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_snapshot,
            commands::scan_devices,
            commands::get_devices,
            commands::get_device,
            commands::get_history,
            commands::write_capability,
            commands::set_device_enabled,
            commands::set_adapter_enabled,
            commands::get_settings,
            commands::update_settings,
            commands::capability_index,
            commands::get_adapters,
            commands::adapter_catalogue,
            commands::list_rules,
            commands::rule_outcomes,
            commands::rule_conflicts,
            commands::rule_compatibility_notes,
            commands::rule_handovers,
            commands::rule_retry_handovers,
            commands::check_rule,
            commands::save_rule,
            commands::delete_rule,
            commands::set_rule_enabled,
            commands::suggest_rules,
            commands::rules_directory,
            commands::list_audit_log,
            commands::automation_stats,
            commands::runtime_stats,
            commands::mock_status,
            commands::mock_set_load,
            commands::mock_apply_profile,
            commands::mock_set_ambient,
            commands::mock_force_gpu_temperature,
            commands::mock_set_faults,
            commands::mock_set_channel_fault,
            commands::ipc_probe_report,
            commands::open_folder,
            commands::open_config_dir,
            commands::open_log_dir,
            commands::open_rules_dir,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            if let Err(error) = tray::install(&handle) {
                // A missing tray (for example on a stripped Linux session) must
                // not stop the app from running.
                tracing::warn!(error = %error, "could not install the tray icon");
            }

            let settings = runtime.settings();
            if settings.start_minimized
                && let Some(window) = app.get_webview_window("main")
            {
                let _ = window.hide();
            }

            events::spawn(handle.clone(), runtime.clone());

            if commands::ipc_probe_armed() {
                // The window and its webview are real, and the frontend bundle is the
                // one the app ships. The backend asks it — over Tauri's own event
                // channel, the same one that carries snapshots to the UI — to exercise
                // the command surface, and waits for it to report back through a real
                // command. Nothing here is a stub: this is the desktop path, and if any
                // link of it is broken the report never arrives.
                let probe_handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
                    let mut attempts = 0usize;
                    while std::time::Instant::now() < deadline && !commands::ipc_probe_done() {
                        // Re-sent until the frontend answers: the listener is registered
                        // when its bundle loads, which may be after this task starts.
                        if let Err(error) = probe_handle.emit(commands::IPC_PROBE_EVENT, ()) {
                            tracing::warn!(error = %error, "could not ask the frontend");
                        }
                        attempts += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                    let done = commands::ipc_probe_done();
                    if done {
                        tracing::info!(attempts, "the frontend reported the IPC self-test");
                    } else {
                        tracing::error!(
                            attempts,
                            "the IPC self-test did not report back before the deadline"
                        );
                    }
                    probe_handle.exit(if done { 0 } else { 3 });
                });
            }

            let runtime = runtime.clone();
            let engine = engine.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = runtime.start().await {
                    tracing::error!(error = %error, "the runtime failed to start");
                }
                if let Err(error) = engine.ensure_example() {
                    tracing::warn!(error = %error, "could not install the example rule");
                }
                if let Err(error) = engine.start().await {
                    tracing::error!(error = %error, "the automation engine failed to start");
                }
                events::emit_snapshot(&handle, &runtime);
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            let state = window.app_handle().state::<AppState>();
            let settings = state.runtime.settings();
            match event {
                WindowEvent::CloseRequested { api, .. } if should_hide_on_close(&settings) => {
                    api.prevent_close();
                    let _ = window.hide();
                    tracing::debug!("window hidden instead of closed");
                }
                // Tauri has no dedicated "minimized" event, so the state is
                // checked on resize. Hiding (rather than leaving a taskbar entry
                // behind) is what "minimize to tray" means to a user.
                WindowEvent::Resized(_)
                    if should_hide_on_minimize(
                        &settings,
                        window.is_minimized().unwrap_or(false),
                    ) =>
                {
                    let _ = window.hide();
                    tracing::debug!("window minimized to the tray");
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())?
        .run(|app, event| {
            // The last chance to hand the fans back to the firmware.
            if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                let state = app.state::<AppState>();
                if state.runtime.is_running() {
                    api.prevent_exit();
                    let runtime = state.runtime.clone();
                    let engine = state.engine.clone();
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        engine.stop().await;
                        if let Err(error) = runtime.shutdown().await {
                            tracing::warn!(error = %error, "shutdown reported a problem");
                        }
                        app.exit(0);
                    });
                }
            }
        });

    Ok(())
}

/// Headless startup check: start the runtime, run a few cycles and print what
/// was found. Used by the test-suite and by anyone verifying an installation
/// without opening a window.
fn selftest_report(
    runtime: Runtime,
    engine: AutomationEngine,
    paths: ConfigPaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let report = tauri::async_runtime::block_on(async {
        let _first = runtime.start().await?;
        engine.ensure_example()?;
        engine.start().await?;
        // A short run so the automation engine actually evaluates once.
        tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
        engine.stop().await;
        let snapshot = runtime.snapshot();
        runtime.shutdown().await?;
        Ok::<_, ohm_core::OhmError>(snapshot)
    })?;

    println!("OpenHardwareOS {} selftest", ohm_core::VERSION);
    println!("config:   {}", paths.root().display());
    println!(
        "adapters: {} registered, {} usable",
        report.adapters.len(),
        report.adapters.iter().filter(|a| a.is_usable()).count()
    );
    println!(
        "devices:  {} discovered, {} controllable",
        report.devices.len(),
        report.controllable_devices().len()
    );
    for view in runtime.adapter_views() {
        println!(
            "  - {:<8} {:<12} {}",
            view.info.id.as_str(),
            view.status.state.as_str(),
            view.status.detail.clone().unwrap_or_default()
        );
    }
    for outcome in engine.outcomes() {
        println!(
            "  rule {:<20} {:<10} {}",
            outcome.rule_id.as_str(),
            format!("{:?}", outcome.status).to_lowercase(),
            outcome.message
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn flags_are_parsed_from_the_command_line() {
        let flags = StartupFlags::parse(&args(&["OpenHardwareOS", "--dry-run", "--mock"]));
        assert!(flags.dry_run && flags.mock);
        assert!(!flags.selftest);
        assert!(!flags.is_empty());

        assert!(StartupFlags::parse(&args(&["OpenHardwareOS"])).is_empty());
        // Unknown flags are ignored rather than fatal.
        assert!(StartupFlags::parse(&args(&["OpenHardwareOS", "--nope"])).is_empty());
    }

    #[test]
    fn window_policy_follows_the_settings() {
        use ohm_runtime::Settings;

        // Defaults: a tray-resident monitor hides on both close and minimise,
        // and the tray icon is the way back (see `tray::show_window`).
        let settings = Settings::default();
        assert!(should_hide_on_close(&settings));
        assert!(should_hide_on_minimize(&settings, true));
        assert!(
            !should_hide_on_minimize(&settings, false),
            "an un-minimised window must stay visible"
        );

        // Turning the switch off restores ordinary minimising.
        let settings = Settings {
            minimize_to_tray: false,
            ..Settings::default()
        };
        assert!(!should_hide_on_minimize(&settings, true));

        // Both switches can be turned off, which restores plain window behaviour.
        let settings = Settings {
            close_to_tray: false,
            minimize_to_tray: false,
            ..Settings::default()
        };
        assert!(!should_hide_on_close(&settings));
        assert!(!should_hide_on_minimize(&settings, true));
    }

    #[test]
    fn dry_run_actually_reaches_the_write_path() {
        // This is the regression that mattered: the flag used to be parsed and
        // displayed, but never applied, so writes still reached hardware.
        let settings = ohm_runtime::Settings::default();
        assert!(!settings.dry_run);

        let forced = apply_startup_overrides(
            settings.clone(),
            StartupFlags {
                dry_run: true,
                ..StartupFlags::default()
            },
        );
        assert!(
            forced.dry_run,
            "`--dry-run` must set Settings::dry_run, which is what the runtime checks"
        );

        // Without the flag, the saved settings are left untouched.
        let untouched = apply_startup_overrides(settings.clone(), StartupFlags::default());
        assert_eq!(untouched, settings);

        // And `--mock` alone must not silently make the app read-only.
        let mock_only = apply_startup_overrides(
            settings.clone(),
            StartupFlags {
                mock: true,
                ..StartupFlags::default()
            },
        );
        assert!(!mock_only.dry_run);
    }
}
