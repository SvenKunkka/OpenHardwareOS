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

use std::path::{Path, PathBuf};

use ohm_adapters::AdapterOptions;
use ohm_automation::AutomationEngine;
use ohm_core::ConfigPaths;
use ohm_runtime::{Runtime, Settings, SettingsStore};
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
/// What an `--ipc-selftest` run must have before anything is created.
///
/// The probe drives the command surface deliberately, including commands with side
/// effects, so its safety cannot depend on how it was launched. It ran as
/// `ohm-desktop --mock --ipc-selftest` and trusted the launcher: `--ipc-selftest` alone
/// armed it against whatever config directory the app would have used — including the
/// real per-user one — and `--mock` only *adds* the simulated provider, leaving
/// LibreHardwareMonitor, the operating system provider and NVML switched on. A probe
/// that calls `rule_retry_handovers` could then have re-armed a real channel's failed
/// handover on a real machine.
///
/// These checks therefore run before the config directory is created, before settings
/// are applied, before the runtime, the engine and every control loop exist, and they
/// refuse rather than falling back to anything.
#[derive(Debug)]
struct ProbeIsolation {
    paths: ConfigPaths,
    settings: Settings,
    /// Simulated hardware only, whatever the settings say — the refusal above means the
    /// settings cannot disagree, but the options actually used are built here rather
    /// than inferred later.
    options: AdapterOptions,
}

impl ProbeIsolation {
    /// Refuse unless the run is explicitly, verifiably isolated.
    fn resolve(override_root: Option<PathBuf>, default_root: &Path) -> Result<Self, String> {
        let env_name = ohm_core::paths::ENV_CONFIG_DIR;
        let Some(root) = override_root.filter(|path| !path.as_os_str().is_empty()) else {
            return Err(format!(
                "refusing to run the IPC self-test without an explicit configuration directory. \
                 The self-test exercises commands that write, so it must not be able to reach a \
                 real installation: set {env_name} to a dedicated directory (a fresh temporary one \
                 is ideal) and disable the real hardware providers in its settings.json. This run \
                 would otherwise have used {}.",
                default_root.display()
            ));
        };

        // The real configuration directory, or anything under it, is the user's — not a
        // place to point a probe that writes.
        if root == default_root || root.starts_with(default_root) {
            return Err(format!(
                "refusing to run the IPC self-test against the real configuration directory ({}). \
                 Point {env_name} at a directory outside it, with the real hardware providers \
                 disabled.",
                root.display()
            ));
        }
        if !root.is_dir() {
            return Err(format!(
                "refusing to run the IPC self-test: the configuration directory {} does not exist. \
                 Create it first — the self-test will not create a configuration directory it was \
                 not explicitly given.",
                root.display()
            ));
        }

        let paths = ConfigPaths::from_root(&root);
        let settings = SettingsStore::new(&paths).load().map_err(|error| {
            format!("could not read the settings in {}: {error}", root.display())
        })?;
        let configured = AdapterOptions::from_settings(&settings);
        // The names the settings file itself uses, so the refusal can tell an operator
        // exactly which `disabled_adapters` entry to add.
        let mut real = Vec::new();
        if configured.system {
            real.push("system");
        }
        if configured.lhm {
            real.push("lhm");
        }
        if configured.nvidia {
            real.push("nvidia");
        }
        if !real.is_empty() {
            return Err(format!(
                "refusing to run the IPC self-test: the settings in {} enable real hardware \
                 provider(s) {real:?}. A simulated-only run must not be able to touch real \
                 hardware: add them to `disabled_adapters` in that settings.json, or leave the \
                 file without them enabled.",
                root.display()
            ));
        }
        if !configured.mock && !configured.protocol_device {
            return Err(format!(
                "refusing to run the IPC self-test: no simulated provider is enabled in {}. \
                 Enable the simulated hardware (or the protocol device) so the probe has something \
                 to exercise.",
                root.display()
            ));
        }

        Ok(Self {
            paths,
            settings,
            options: AdapterOptions::simulated_only(),
        })
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let flags = StartupFlags::parse(&std::env::args().collect::<Vec<_>>());

    // The probe's isolation is established first, before `paths.ensure()` can create a
    // directory, before settings are applied and before any provider, runtime or engine
    // exists. A refusal here exits non-zero and leaves the machine alone.
    let isolation = if flags.ipc_selftest {
        let override_root = std::env::var_os(ohm_core::paths::ENV_CONFIG_DIR).map(PathBuf::from);
        let default_root = ConfigPaths::discover_with(None)?.root().to_path_buf();
        // A refusal here is deliberately before logging exists: it has to be readable in
        // the console of whoever launched it, not only in a log file inside a directory
        // the run may not have been allowed to touch.
        let isolation = ProbeIsolation::resolve(override_root, &default_root)?;
        eprintln!(
            "OpenHardwareOS: IPC self-test, isolated to {} (simulated providers only)",
            isolation.paths.root().display()
        );
        Some(isolation)
    } else {
        None
    };

    let paths = match &isolation {
        Some(isolation) => isolation.paths.clone(),
        None => ConfigPaths::discover()?,
    };
    paths.ensure()?;

    let store = SettingsStore::new(&paths);
    let settings = match &isolation {
        Some(isolation) => isolation.settings.clone(),
        None => store.load()?,
    };
    let _log_guard = ohm_core::logging::init(settings.log_level, Some(&paths))?;

    // `--dry-run` must change what the runtime *does*, not only what it reports.
    let settings = apply_startup_overrides(settings, flags);

    tracing::info!(
        version = ohm_core::VERSION,
        config = %paths.root().display(),
        dry_run = settings.dry_run,
        "starting OpenHardwareOS"
    );

    let mut options = match &isolation {
        // Simulated only, decided by `ProbeIsolation` and not by the settings file.
        Some(isolation) => isolation.options.clone(),
        None => AdapterOptions::from_settings(&settings),
    };
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

    /// Write a settings file for a simulated-only probe run.
    fn write_probe_settings(root: &Path, extra: &str) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(
            root.join("settings.json"),
            format!(
                r#"{{"experimental_features": true, "disabled_adapters": ["system", "lhm", "nvidia"]{extra}}}"#
            ),
        )
        .unwrap();
    }

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// The hole that mattered most: `--ipc-selftest` on its own pointed the probe at
    /// whatever configuration directory the app would have used.
    #[test]
    fn a_probe_without_an_explicit_configuration_directory_is_refused() {
        let default = PathBuf::from("/Users/someone/Library/Application Support/OpenHardwareOS");
        let error = ProbeIsolation::resolve(None, &default)
            .expect_err("a probe with no explicit config directory must be refused");
        assert!(
            error.contains("explicit configuration directory"),
            "{error}"
        );
        assert!(
            error.contains(ohm_core::paths::ENV_CONFIG_DIR),
            "the refusal must name the variable to set: {error}"
        );
        assert!(
            error.contains(&default.display().to_string()),
            "and the directory it would otherwise have used: {error}"
        );

        // The same for an empty value, which is not an isolation either.
        let empty = ProbeIsolation::resolve(Some(PathBuf::new()), &default);
        assert!(empty.is_err());
    }

    /// The real configuration directory — or anything inside it — is not a place to
    /// point a probe that writes.
    #[test]
    fn a_probe_pointed_at_the_real_configuration_directory_is_refused() {
        let temp = temp_root();
        let default = temp.path().join("Application Support/OpenHardwareOS");
        std::fs::create_dir_all(&default).unwrap();

        let error = ProbeIsolation::resolve(Some(default.clone()), &default)
            .expect_err("the real config directory must be refused");
        assert!(error.contains("real configuration directory"), "{error}");

        let inside = default.join("scratch");
        std::fs::create_dir_all(&inside).unwrap();
        let error = ProbeIsolation::resolve(Some(inside), &default)
            .expect_err("a directory inside the real one must be refused");
        assert!(error.contains("real configuration directory"), "{error}");
    }

    /// A directory that does not exist is not created on the probe's behalf: creating
    /// configuration is exactly the side effect the checks exist to prevent.
    #[test]
    fn a_probe_with_a_missing_configuration_directory_is_refused_and_creates_nothing() {
        let temp = temp_root();
        let missing = temp.path().join("never-created");
        let error = ProbeIsolation::resolve(Some(missing.clone()), &temp.path().join("real"))
            .expect_err("a missing directory must be refused");
        assert!(error.contains("does not exist"), "{error}");
        assert!(
            !missing.exists(),
            "the refusal must not have created the directory"
        );
    }

    /// `--mock` only *adds* the simulated provider; it never switched the real ones
    /// off, so a probe could have run with LHM and the OS provider live.
    #[test]
    fn a_probe_with_a_real_provider_enabled_is_refused() {
        let temp = temp_root();
        let root = temp.path().join("probe");
        std::fs::create_dir_all(&root).unwrap();
        // The default settings enable the real providers.
        std::fs::write(root.join("settings.json"), "{}").unwrap();
        let error = ProbeIsolation::resolve(Some(root.clone()), &temp.path().join("real"))
            .expect_err("default settings enable real providers and must be refused");
        assert!(error.contains("real hardware provider"), "{error}");
        assert!(error.contains("disabled_adapters"), "{error}");
        // Only LHM enabled is still a refusal, and the message names it.
        let only_lhm = temp.path().join("only-lhm");
        write_probe_settings(&only_lhm, "");
        std::fs::write(
            only_lhm.join("settings.json"),
            r#"{"disabled_adapters": ["system", "nvidia"], "experimental_features": true}"#,
        )
        .unwrap();
        let error = ProbeIsolation::resolve(Some(only_lhm), &temp.path().join("real"))
            .expect_err("a live LHM provider must be refused");
        assert!(error.contains("lhm"), "{error}");
    }

    /// The legitimate case: a dedicated directory whose settings disable every real
    /// provider and enable the simulator.
    #[test]
    fn an_isolated_probe_is_accepted_and_is_given_simulated_providers_only() {
        let temp = temp_root();
        let root = temp.path().join("probe");
        write_probe_settings(&root, "");
        let isolation = ProbeIsolation::resolve(Some(root.clone()), &temp.path().join("real"))
            .expect("an isolated, simulated-only probe is allowed");
        assert_eq!(isolation.paths.root(), root.as_path());
        let mut ids = isolation.options.enabled_ids();
        ids.sort_unstable();
        assert_eq!(
            ids,
            vec!["mock".to_string(), "opd".to_string()],
            "only simulated providers are constructed, whatever the settings say"
        );
        assert!(!isolation.options.lhm && !isolation.options.system && !isolation.options.nvidia);
    }

    /// A configuration directory with no simulator in it at all gives the probe nothing
    /// to exercise, which is a refusal rather than a silent no-op.
    #[test]
    fn a_probe_without_any_simulated_provider_is_refused() {
        let temp = temp_root();
        let root = temp.path().join("probe");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("settings.json"),
            r#"{"disabled_adapters": ["system", "lhm", "nvidia", "mock", "opd"]}"#,
        )
        .unwrap();
        let error = ProbeIsolation::resolve(Some(root), &temp.path().join("real"))
            .expect_err("no simulated provider must be refused");
        assert!(error.contains("simulated provider"), "{error}");
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
