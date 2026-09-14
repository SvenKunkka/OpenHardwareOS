//! The Tauri command surface.
//!
//! Every command is a thin adapter over the runtime or the automation engine.
//! That is deliberate: the desktop app has no hardware logic of its own, so the
//! CLI, the tests and the UI all observe exactly the same behaviour.
//!
//! Shape of the contract (mirrored by `apps/desktop/src/lib/ipc.ts`):
//!
//! * reads return owned snapshots (`RuntimeSnapshot`, `DeviceView`, ...),
//! * writes go through [`Runtime::write_value`], so they inherit range
//!   validation, the safety policy and the audit log,
//! * failures become [`CommandError`], never a panic.

use std::sync::Arc;

use ohm_adapter_api::HardwareAdapter;
use ohm_adapter_mock::{LoadProfile, MockAdapter, MockFaults, MockStatus};
use ohm_automation::examples::suggest_for;
use ohm_automation::{Rule, RuleCheck, RuleConflict, RuleOutcome, RuleStore, merge_suggestions};
use ohm_core::{OhmError, RuleId};
use ohm_device_model::Value;
use ohm_runtime::{
    AdapterView, CapabilityIndex, DeviceView, RuntimeSnapshot, Sample, Settings, WriteOrigin,
};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::{AppState, CommandError, CommandResult};

/// What the UI shows in the About box and in Diagnostics.
#[derive(Debug, Clone, Serialize)]
pub struct AppInfo {
    pub version: String,
    pub platform: String,
    pub config_root: String,
    pub protocol_version: String,
    pub os: String,
    pub arch: String,
    pub elevated: bool,
    pub dry_run: bool,
    pub automation_enabled: bool,
    pub uptime_ms: i64,
    pub adapters: Vec<String>,
}

// ---------------------------------------------------------------------------
// Application
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> CommandResult<AppInfo> {
    let settings = state.runtime.settings();
    Ok(AppInfo {
        version: ohm_core::VERSION.to_string(),
        platform: std::env::consts::OS.to_string(),
        config_root: state.paths.root().display().to_string(),
        protocol_version: ohm_protocol::protocol_version(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        arch: std::env::consts::ARCH.to_string(),
        elevated: crate::autostart::is_elevated(),
        dry_run: settings.dry_run || state.dry_run_override,
        automation_enabled: settings.automation_enabled,
        uptime_ms: state.uptime_ms(),
        adapters: state
            .runtime
            .adapter_views()
            .into_iter()
            .map(|view| view.info.id.to_string())
            .collect(),
    })
}

// ---------------------------------------------------------------------------
// Snapshots and devices
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_snapshot(state: State<'_, AppState>) -> CommandResult<RuntimeSnapshot> {
    Ok(state.runtime.snapshot())
}

/// Re-enumerate the hardware now, then return a fresh snapshot.
#[tauri::command]
pub async fn scan_devices(state: State<'_, AppState>) -> CommandResult<RuntimeSnapshot> {
    state.runtime.refresh_devices().await?;
    state.runtime.poll_once().await?;
    Ok(state.runtime.snapshot())
}

#[tauri::command]
pub fn get_devices(state: State<'_, AppState>) -> CommandResult<Vec<DeviceView>> {
    Ok(state.runtime.devices())
}

#[tauri::command]
pub fn get_device(state: State<'_, AppState>, device: String) -> CommandResult<DeviceView> {
    state
        .runtime
        .device(&device)
        .ok_or_else(|| CommandError::not_found(format!("device `{device}`")))
}

#[tauri::command]
pub fn get_history(
    state: State<'_, AppState>,
    device: String,
    capability: String,
    limit: usize,
) -> CommandResult<Vec<Sample>> {
    Ok(state
        .runtime
        .history(&device, &capability, limit.clamp(1, 10_000)))
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

/// Convert the JSON value the frontend sends into a device value.
fn to_value(value: serde_json::Value) -> CommandResult<Value> {
    Ok(match value {
        serde_json::Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                Value::Integer(integer)
            } else {
                Value::Number(number.as_f64().unwrap_or_default())
            }
        }
        serde_json::Value::Bool(flag) => Value::Bool(flag),
        serde_json::Value::String(text) => Value::Text(text),
        other => {
            return Err(CommandError::new(
                "invalid_value",
                format!("unsupported value `{other}`"),
                "Send a number, a boolean or a string.",
            ));
        }
    })
}

#[tauri::command]
pub async fn write_capability(
    state: State<'_, AppState>,
    device: String,
    capability: String,
    value: serde_json::Value,
) -> CommandResult<ohm_runtime::WriteReport> {
    let value = to_value(value)?;
    let report = state
        .runtime
        .write_value(&device, &capability, value, WriteOrigin::Manual)
        .await?;
    Ok(report)
}

#[tauri::command]
pub fn set_device_enabled(
    state: State<'_, AppState>,
    device: String,
    enabled: bool,
) -> CommandResult<()> {
    state.runtime.set_device_enabled(&device, enabled)?;
    Ok(())
}

#[tauri::command]
pub fn set_adapter_enabled(
    state: State<'_, AppState>,
    adapter: String,
    enabled: bool,
) -> CommandResult<Settings> {
    let settings = state.runtime.set_adapter_enabled(&adapter, enabled)?;
    Ok(settings)
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> CommandResult<Settings> {
    Ok(state.runtime.settings())
}

/// Persist new settings.
///
/// Several settings change what the runtime does *structurally* (disabled
/// adapters, simulated hardware, polling interval), so they are re-applied here
/// rather than only being written to disk.
#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> CommandResult<Settings> {
    let before = state.runtime.settings();
    let saved = state
        .runtime
        .update_settings(|current| *current = settings)?;

    // Turning simulated hardware (or the protocol device) on or off changes
    // which adapters exist. The adapter list is fixed at startup in this MVP, so
    // tell the user to restart instead of pretending it took effect.
    let mock_changed = before.experimental_features != saved.experimental_features
        || before.enable_mock_protocol_device != saved.enable_mock_protocol_device;
    if mock_changed {
        state.runtime.log(
            "info",
            "Simulated hardware visibility changed. Restart OpenHardwareOS to apply it.",
        );
    }

    // "Start with Windows" is the one setting that has to reach outside the
    // app. It is applied best-effort and reported honestly: on non-Windows
    // platforms it is not available at all, and the setting is then corrected
    // rather than left claiming something that will not happen.
    if before.start_with_windows != saved.start_with_windows {
        match crate::autostart::set_enabled(saved.start_with_windows) {
            Ok(actual) if actual == saved.start_with_windows => {
                state.runtime.log(
                    "info",
                    if actual {
                        "OpenHardwareOS will start with the operating system."
                    } else {
                        "Start with the operating system disabled."
                    },
                );
            }
            Ok(actual) => {
                state.runtime.log(
                    "warn",
                    format!("the startup entry could not be applied (now: {actual})"),
                );
                let corrected = state
                    .runtime
                    .update_settings(|settings| settings.start_with_windows = actual)?;
                return Ok(corrected);
            }
            Err(error) => {
                state
                    .runtime
                    .log("warn", format!("{} {}", error.message, error.hint));
                let corrected = state
                    .runtime
                    .update_settings(|settings| settings.start_with_windows = false)?;
                return Ok(corrected);
            }
        }
    }

    // The polling and discovery intervals need no action here: the runtime's
    // loops read the settings on every cycle.
    Ok(saved)
}

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn capability_index(state: State<'_, AppState>) -> CommandResult<CapabilityIndex> {
    Ok(state.runtime.capability_index())
}

#[tauri::command]
pub fn get_adapters(state: State<'_, AppState>) -> CommandResult<Vec<AdapterView>> {
    Ok(state.runtime.adapter_views())
}

/// Static description of every provider this build can offer.
#[tauri::command]
pub fn adapter_catalogue() -> CommandResult<Vec<ohm_adapter_api::AdapterInfo>> {
    Ok(ohm_adapters::adapter_catalogue())
}

// ---------------------------------------------------------------------------
// Automation
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_rules(state: State<'_, AppState>) -> CommandResult<Vec<Rule>> {
    Ok(state.engine.rules())
}

#[tauri::command]
pub fn rule_outcomes(state: State<'_, AppState>) -> CommandResult<Vec<RuleOutcome>> {
    Ok(state.engine.outcomes())
}

/// Rules that fight over one output, as resolved at load time.
#[tauri::command]
pub fn rule_conflicts(state: State<'_, AppState>) -> CommandResult<Vec<RuleConflict>> {
    Ok(state.engine.conflicts())
}

#[tauri::command]
pub fn check_rule(state: State<'_, AppState>, rule: Rule) -> CommandResult<RuleCheck> {
    Ok(state.engine.check_rule(&rule))
}

#[tauri::command]
pub fn save_rule(state: State<'_, AppState>, rule: Rule) -> CommandResult<Rule> {
    Ok(state.engine.save_rule(rule)?)
}

#[tauri::command]
pub fn delete_rule(state: State<'_, AppState>, id: String) -> CommandResult<bool> {
    Ok(state.engine.delete_rule(&id)?)
}

#[tauri::command]
pub fn set_rule_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> CommandResult<Rule> {
    Ok(state.engine.set_rule_enabled(&id, enabled)?)
}

/// Starter rules built from the hardware that is actually attached.
#[tauri::command]
pub fn suggest_rules(state: State<'_, AppState>) -> CommandResult<Vec<Rule>> {
    let existing = state.engine.rules();
    let suggestions = suggest_for(state.engine.runtime());
    Ok(merge_suggestions(&existing, suggestions))
}

/// The rule file directory, for the "open folder" button.
#[tauri::command]
pub fn rules_directory(state: State<'_, AppState>) -> CommandResult<String> {
    Ok(state.engine.store().directory().display().to_string())
}

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_audit_log(
    state: State<'_, AppState>,
    limit: usize,
) -> CommandResult<Vec<serde_json::Value>> {
    Ok(state.runtime.audit().tail(limit.clamp(1, 5_000))?)
}

#[tauri::command]
pub fn automation_stats(
    state: State<'_, AppState>,
) -> CommandResult<ohm_automation::AutomationStats> {
    Ok(state.engine.stats())
}

#[tauri::command]
pub fn runtime_stats(state: State<'_, AppState>) -> CommandResult<ohm_runtime::RuntimeStats> {
    Ok(state.runtime.stats())
}

// ---------------------------------------------------------------------------
// Simulated hardware controls
// ---------------------------------------------------------------------------

/// Borrow the simulated provider behind a registered adapter.
///
/// The caller keeps the `Arc` alive for as long as the borrow is used, so there
/// is no downcast trickery and no unsafe code.
fn mock_ref<'a>(adapter: &'a Arc<dyn HardwareAdapter>) -> CommandResult<&'a MockAdapter> {
    adapter
        .as_any()
        .downcast_ref::<MockAdapter>()
        .ok_or_else(|| {
            CommandError::new(
                "internal_error",
                "the mock provider is registered under an unexpected type",
                "This is a bug: please report it.",
            )
        })
}

#[tauri::command]
pub fn mock_status(state: State<'_, AppState>) -> CommandResult<Option<MockStatus>> {
    let Some(adapter) = state.mock_adapter() else {
        return Ok(None);
    };
    Ok(Some(mock_ref(&adapter)?.status()))
}

#[tauri::command]
pub fn mock_set_load(
    state: State<'_, AppState>,
    gpu: f64,
    cpu: f64,
) -> CommandResult<Option<MockStatus>> {
    let Some(adapter) = state.mock_adapter() else {
        return Ok(None);
    };
    let mock = mock_ref(&adapter)?;
    mock.set_gpu_load(gpu.clamp(0.0, 1.0));
    mock.set_cpu_load(cpu.clamp(0.0, 1.0));
    Ok(Some(mock.status()))
}

#[tauri::command]
pub fn mock_apply_profile(
    state: State<'_, AppState>,
    profile: String,
) -> CommandResult<Option<MockStatus>> {
    let Some(adapter) = state.mock_adapter() else {
        return Ok(None);
    };
    let mock = mock_ref(&adapter)?;
    let profile = match profile.as_str() {
        "idle" => LoadProfile::Constant { load: 0.05 },
        "gaming" => LoadProfile::gaming_demo(),
        "wave" => LoadProfile::Wave {
            min: 0.1,
            max: 0.95,
            period_ms: 30_000,
            phase: 0.0,
        },
        other => {
            return Err(CommandError::new(
                "invalid_value",
                format!("unknown profile `{other}`"),
                "Use `idle`, `gaming` or `wave`.",
            ));
        }
    };
    mock.set_load_profile(profile);
    Ok(Some(mock.status()))
}

#[tauri::command]
pub fn mock_set_ambient(
    state: State<'_, AppState>,
    celsius: f64,
) -> CommandResult<Option<MockStatus>> {
    let Some(adapter) = state.mock_adapter() else {
        return Ok(None);
    };
    let mock = mock_ref(&adapter)?;
    mock.set_ambient_temp(celsius);
    Ok(Some(mock.status()))
}

#[tauri::command]
pub fn mock_force_gpu_temperature(
    state: State<'_, AppState>,
    celsius: f64,
) -> CommandResult<Option<MockStatus>> {
    let Some(adapter) = state.mock_adapter() else {
        return Ok(None);
    };
    let mock = mock_ref(&adapter)?;
    mock.force_gpu_temperature(celsius);
    Ok(Some(mock.status()))
}

#[tauri::command]
pub fn mock_set_faults(
    state: State<'_, AppState>,
    fail_writes: bool,
    disconnect_gpu_temperature: bool,
    unplug_fan: bool,
) -> CommandResult<Option<MockStatus>> {
    let Some(adapter) = state.mock_adapter() else {
        return Ok(None);
    };
    let mock = mock_ref(&adapter)?;

    let mut faults = MockFaults::default();
    faults.fail_all_writes = fail_writes;
    if disconnect_gpu_temperature {
        faults.unavailable_readings.push((
            ohm_core::DeviceId::new_unchecked("gpu.mock.0"),
            ohm_core::CapabilityId::new_unchecked(ohm_core::ids::capability::TEMPERATURE_CORE),
            ohm_device_model::UnavailableReason::ReadError,
        ));
    }
    if unplug_fan {
        faults
            .unplug_devices
            .push(ohm_core::DeviceId::new_unchecked("fan.mock.0"));
    }
    mock.set_faults(faults);
    Ok(Some(mock.status()))
}

// ---------------------------------------------------------------------------
// Shell helpers
// ---------------------------------------------------------------------------

/// Folder to reveal in the file manager.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Folder {
    Config,
    Logs,
    Rules,
}

#[tauri::command]
pub fn open_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    folder: Folder,
) -> CommandResult<()> {
    let path = match folder {
        Folder::Config => state.paths.root().to_path_buf(),
        Folder::Logs => state.paths.logs_dir(),
        Folder::Rules => state.engine.store().directory().to_path_buf(),
    };
    crate::shell::open_path(&app, &path)
}

#[tauri::command]
pub fn open_config_dir(app: tauri::AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    crate::shell::open_path(&app, state.paths.root())
}

#[tauri::command]
pub fn open_log_dir(app: tauri::AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    crate::shell::open_path(&app, &state.paths.logs_dir())
}

#[tauri::command]
pub fn open_rules_dir(app: tauri::AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    crate::shell::open_path(&app, state.engine.store().directory())
}

// ---------------------------------------------------------------------------
// Startup helpers used by `run()`
// ---------------------------------------------------------------------------

/// Rules directory for a runtime, so the engine and the UI agree.
pub fn rule_store_for(state: &AppState) -> RuleStore {
    RuleStore::new(state.paths.rules_dir())
}

/// The id of a rule, parsed with a helpful error.
pub fn rule_id(value: &str) -> CommandResult<RuleId> {
    RuleId::new(value).map_err(|error| CommandError::from(OhmError::from(error)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_values_are_converted_faithfully() {
        assert_eq!(to_value(serde_json::json!(42)).unwrap(), Value::Integer(42));
        assert_eq!(
            to_value(serde_json::json!(42.5)).unwrap(),
            Value::Number(42.5)
        );
        assert_eq!(
            to_value(serde_json::json!(true)).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            to_value(serde_json::json!("software")).unwrap(),
            Value::Text("software".into())
        );
        let error = to_value(serde_json::json!({"nested": 1})).unwrap_err();
        assert_eq!(error.code, "invalid_value");
    }

    #[test]
    fn rule_ids_are_validated() {
        assert_eq!(rule_id("gpu-cooling").unwrap().as_str(), "gpu-cooling");
        assert!(rule_id("Bad Id").is_err());
    }
}
