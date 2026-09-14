//! Shared application state and the error type the frontend sees.

use std::sync::Arc;

use ohm_adapter_api::HardwareAdapter;
use ohm_adapter_mock::MockConfig;
use ohm_automation::AutomationEngine;
use ohm_core::{ConfigPaths, OhmError};
use ohm_runtime::Runtime;
use serde::Serialize;

/// Everything the Tauri commands operate on.
///
/// The UI never touches an adapter directly: it goes through
/// [`Runtime`](ohm_runtime::Runtime), exactly like the CLI and the automation
/// engine do.
#[derive(Debug)]
pub struct AppState {
    pub runtime: Runtime,
    pub engine: AutomationEngine,
    pub paths: ConfigPaths,
    pub started_at_ms: i64,
    /// `true` when the app was started with `--dry-run`.
    pub dry_run_override: bool,
}

impl AppState {
    pub fn new(runtime: Runtime, paths: ConfigPaths, engine: AutomationEngine) -> Self {
        Self {
            runtime,
            engine,
            paths,
            started_at_ms: ohm_core::now_ms(),
            dry_run_override: false,
        }
    }

    /// Uptime in milliseconds.
    pub fn uptime_ms(&self) -> i64 {
        ohm_core::now_ms() - self.started_at_ms
    }

    /// The simulated hardware provider, when it is registered.
    ///
    /// The mock adapter's controls all take `&self`, so holding the `Arc` while
    /// calling them is all that is needed — no downcast trickery, no unsafe.
    pub fn mock_adapter(&self) -> Option<Arc<dyn HardwareAdapter>> {
        self.runtime.adapter(ohm_adapter_mock::ADAPTER_ID)
    }
}

/// The error shape the frontend expects.
///
/// `code`, `hint` and `unsupported` exist so the UI can explain *why* something
/// could not be done (read-only hardware, missing driver, needs elevation)
/// instead of showing a stack trace.
#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
    pub hint: String,
    pub unsupported: bool,
}

impl CommandError {
    pub fn new(code: &str, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            hint: hint.into(),
            unsupported: false,
        }
    }

    /// The command was given something that does not exist.
    pub fn not_found(what: impl Into<String>) -> Self {
        Self::new(
            "not_found",
            what,
            "Rescan devices. The hardware may have been unplugged or disabled.",
        )
    }

    /// The feature is not available in this build or on this platform.
    pub fn unavailable(what: impl Into<String>, hint: impl Into<String>) -> Self {
        let mut error = Self::new("unavailable", what, hint);
        error.unsupported = true;
        error
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}

impl From<OhmError> for CommandError {
    fn from(error: OhmError) -> Self {
        Self {
            code: error.code().to_string(),
            message: error.to_string(),
            hint: error.hint().to_string(),
            unsupported: error.is_unsupported(),
        }
    }
}

impl From<serde_json::Error> for CommandError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(
            "serde_error",
            error.to_string(),
            "This is a bug: please report the command and the values you used.",
        )
    }
}

/// Convenience alias for command results.
pub type CommandResult<T> = std::result::Result<T, CommandError>;

/// Configuration of the simulated machine, taken from settings.
pub fn mock_config_or_default(state: &AppState) -> MockConfig {
    let settings = state.runtime.settings();
    match settings.adapter_config(ohm_adapter_mock::ADAPTER_ID) {
        Some(bag) => {
            let profile = bag.get("profile").and_then(|value| value.as_str());
            match profile {
                Some("gaming") => MockConfig::gaming_demo(),
                Some("idle") => MockConfig::idle(),
                _ => MockConfig::default(),
            }
        }
        None => MockConfig::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_the_ui_fields() {
        let error = CommandError::from(OhmError::CapabilityNotWritable {
            device: "fan.system.0".into(),
            capability: "fan.rpm".into(),
        });
        assert_eq!(error.code, "capability_read_only");
        assert!(error.unsupported, "read-only hardware is a limitation");
        assert!(!error.hint.is_empty());
        assert!(error.to_string().contains("capability_read_only"));

        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["code"], "capability_read_only");
        assert_eq!(json["unsupported"], true);
    }

    #[test]
    fn helper_constructors() {
        let error = CommandError::not_found("device `x`");
        assert_eq!(error.code, "not_found");
        assert!(error.hint.contains("Rescan"));
        let error = CommandError::unavailable("tray", "needs a desktop session");
        assert!(error.unsupported);
    }

    #[test]
    fn serde_errors_are_wrapped() {
        let error = CommandError::from(serde_json::from_str::<i32>("nope").unwrap_err());
        assert_eq!(error.code, "serde_error");
    }
}
