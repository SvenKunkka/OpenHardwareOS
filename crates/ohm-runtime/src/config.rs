//! User settings and their persistence.
//!
//! Local first: a single `settings.json` in the config directory, written
//! atomically. No account, no cloud, nothing leaves the machine.
//!
//! Settings are forward compatible: unknown keys are ignored, missing keys fall
//! back to [`Settings::default`], and the file is written by
//! [`SettingsStore::save`] with a temp-file + rename so a crash can never leave
//! a truncated configuration behind.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ohm_core::logging::LogLevel;
use ohm_core::{AdapterId, ConfigPaths, DeviceId, OhmError, Result};
use serde::{Deserialize, Serialize};

use crate::safety::SafetyPolicy;

/// UI colour scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    #[default]
    Dark,
    Light,
}

/// Everything the user can configure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// How often sensors are polled, in milliseconds.
    pub polling_interval_ms: u64,
    /// How often adapters are re-enumerated (hotplug detection).
    pub discovery_interval_ms: u64,
    /// How many samples per capability are kept in memory for charts.
    pub history_points: usize,
    pub log_level: LogLevel,

    // --- desktop shell -----------------------------------------------------
    pub start_with_windows: bool,
    pub minimize_to_tray: bool,
    pub start_minimized: bool,
    pub close_to_tray: bool,
    pub theme: Theme,

    // --- features ----------------------------------------------------------
    /// Enables the experimental section: mock hardware, protocol devices,
    /// simulator controls.
    pub experimental_features: bool,
    /// Verbose diagnostics, raw adapter payloads, rule internals.
    pub developer_mode: bool,
    /// Master switch for the automation engine.
    pub automation_enabled: bool,
    /// Never touch the hardware: writes are simulated and audited instead.
    pub dry_run: bool,
    /// Show the Open Device Protocol mock device in discovery.
    pub enable_mock_protocol_device: bool,

    // --- selection ---------------------------------------------------------
    pub disabled_adapters: Vec<AdapterId>,
    pub disabled_devices: Vec<DeviceId>,

    // --- safety ------------------------------------------------------------
    pub safety: SafetyPolicy,

    /// Adapter specific configuration bags (`adapter_settings["mock"] = {...}`).
    pub adapter_settings: BTreeMap<String, serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            polling_interval_ms: 1_000,
            discovery_interval_ms: 5_000,
            history_points: 180,
            log_level: LogLevel::Info,
            start_with_windows: false,
            minimize_to_tray: true,
            start_minimized: false,
            close_to_tray: true,
            theme: Theme::Dark,
            experimental_features: false,
            developer_mode: false,
            automation_enabled: true,
            dry_run: false,
            enable_mock_protocol_device: false,
            disabled_adapters: Vec::new(),
            disabled_devices: Vec::new(),
            safety: SafetyPolicy::default(),
            adapter_settings: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// Hard bounds, also applied after loading a hand-edited file.
    pub const MIN_POLLING_INTERVAL_MS: u64 = 100;
    pub const MAX_POLLING_INTERVAL_MS: u64 = 60_000;
    pub const MIN_DISCOVERY_INTERVAL_MS: u64 = 500;
    pub const MAX_DISCOVERY_INTERVAL_MS: u64 = 600_000;
    pub const MAX_HISTORY_POINTS: usize = 10_000;

    /// Clamp every field into a sane range. Returns `true` when something was
    /// changed, so the caller can log/persist the correction.
    pub fn sanitise(&mut self) -> bool {
        let before = self.clone();
        self.polling_interval_ms = self
            .polling_interval_ms
            .clamp(Self::MIN_POLLING_INTERVAL_MS, Self::MAX_POLLING_INTERVAL_MS);
        self.discovery_interval_ms = self.discovery_interval_ms.clamp(
            Self::MIN_DISCOVERY_INTERVAL_MS,
            Self::MAX_DISCOVERY_INTERVAL_MS,
        );
        if self.discovery_interval_ms < self.polling_interval_ms {
            self.discovery_interval_ms = self.polling_interval_ms;
        }
        self.history_points = self.history_points.clamp(10, Self::MAX_HISTORY_POINTS);
        self.safety.sanitise();

        // De-duplicate the selection lists so toggling is idempotent.
        self.disabled_adapters.sort();
        self.disabled_adapters.dedup();
        self.disabled_devices.sort();
        self.disabled_devices.dedup();

        *self != before
    }

    pub fn adapter_enabled(&self, id: &str) -> bool {
        !self.disabled_adapters.iter().any(|a| a.as_str() == id)
    }

    pub fn device_enabled(&self, id: &str) -> bool {
        !self.disabled_devices.iter().any(|d| d.as_str() == id)
    }

    /// Enable or disable a device, keeping the list normalised.
    pub fn set_device_enabled(&mut self, id: &DeviceId, enabled: bool) {
        self.disabled_devices.retain(|d| d != id);
        if !enabled {
            self.disabled_devices.push(id.clone());
            self.disabled_devices.sort();
        }
    }

    /// Enable or disable an adapter.
    pub fn set_adapter_enabled(&mut self, id: &AdapterId, enabled: bool) {
        self.disabled_adapters.retain(|a| a != id);
        if !enabled {
            self.disabled_adapters.push(id.clone());
            self.disabled_adapters.sort();
        }
    }

    /// Read an adapter specific config bag.
    pub fn adapter_config(&self, adapter: &str) -> Option<&serde_json::Value> {
        self.adapter_settings.get(adapter)
    }

    /// Read one key out of an adapter's config bag.
    pub fn adapter_setting(&self, adapter: &str, key: &str) -> Option<&serde_json::Value> {
        self.adapter_settings
            .get(adapter)
            .and_then(|bag| bag.get(key))
    }

    /// Write one key into an adapter's config bag, creating it when needed.
    pub fn set_adapter_setting(&mut self, adapter: &str, key: &str, value: serde_json::Value) {
        let entry = self
            .adapter_settings
            .entry(adapter.to_string())
            .or_insert_with(|| serde_json::json!({}));
        if !entry.is_object() {
            *entry = serde_json::json!({});
        }
        if let Some(object) = entry.as_object_mut() {
            object.insert(key.to_string(), value);
        }
    }
}

/// Load/save [`Settings`] from the config directory.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    /// Store backed by `<config>/settings.json`.
    pub fn new(paths: &ConfigPaths) -> Self {
        Self {
            path: paths.settings_file(),
        }
    }

    /// Store backed by an explicit file path.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load settings, falling back to defaults when the file is missing.
    ///
    /// A corrupt file is not fatal: it is renamed to `settings.json.corrupt`
    /// and defaults are returned, so the app always starts.
    pub fn load(&self) -> Result<Settings> {
        if !self.path.exists() {
            let mut settings = Settings::default();
            settings.sanitise();
            return Ok(settings);
        }
        let raw = std::fs::read_to_string(&self.path).map_err(|e| OhmError::io(&self.path, e))?;
        if raw.trim().is_empty() {
            let mut settings = Settings::default();
            settings.sanitise();
            return Ok(settings);
        }
        match serde_json::from_str::<Settings>(&raw) {
            Ok(mut settings) => {
                settings.sanitise();
                Ok(settings)
            }
            Err(err) => {
                let backup = self.path.with_extension("json.corrupt");
                if let Err(rename_err) = std::fs::rename(&self.path, &backup) {
                    tracing::warn!(error = %rename_err, "could not back up the corrupt settings file");
                }
                tracing::warn!(
                    error = %err,
                    backup = %backup.display(),
                    "settings.json was unreadable, falling back to defaults"
                );
                let mut settings = Settings::default();
                settings.sanitise();
                Ok(settings)
            }
        }
    }

    /// Persist settings atomically (temp file + rename).
    pub fn save(&self, settings: &Settings) -> Result<()> {
        let mut settings = settings.clone();
        settings.sanitise();
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| OhmError::io(parent, e))?;
        }
        let json = serde_json::to_string_pretty(&settings)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json.as_bytes()).map_err(|e| OhmError::io(&tmp, e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| OhmError::io(&self.path, e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let settings = Settings::default();
        assert_eq!(settings.polling_interval_ms, 1_000);
        assert!(settings.automation_enabled);
        assert!(!settings.dry_run);
        assert!(settings.safety.require_min_duty);
        assert_eq!(settings.theme, Theme::Dark);
    }

    #[test]
    fn sanitise_clamps_and_reports() {
        let mut settings = Settings {
            polling_interval_ms: 1,
            discovery_interval_ms: 10,
            history_points: 1_000_000,
            ..Default::default()
        };
        assert!(settings.sanitise());
        assert_eq!(
            settings.polling_interval_ms,
            Settings::MIN_POLLING_INTERVAL_MS
        );
        assert_eq!(
            settings.discovery_interval_ms,
            Settings::MIN_DISCOVERY_INTERVAL_MS
        );
        assert_eq!(settings.history_points, Settings::MAX_HISTORY_POINTS);
        assert!(!settings.sanitise(), "second pass is a no-op");
    }

    #[test]
    fn selection_helpers() {
        let mut settings = Settings::default();
        let fan = DeviceId::new("fan.system.0").unwrap();
        let adapter = AdapterId::new("system").unwrap();
        assert!(settings.device_enabled("fan.system.0"));
        settings.set_device_enabled(&fan, false);
        assert!(!settings.device_enabled("fan.system.0"));
        settings.set_device_enabled(&fan, false);
        assert_eq!(settings.disabled_devices.len(), 1, "no duplicates");
        settings.set_device_enabled(&fan, true);
        assert!(settings.device_enabled("fan.system.0"));

        settings.set_adapter_enabled(&adapter, false);
        assert!(!settings.adapter_enabled("system"));
        assert!(settings.adapter_enabled("mock"));
    }

    #[test]
    fn roundtrip_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(tmp.path().join("settings.json"));
        let mut settings = Settings {
            polling_interval_ms: 2_500,
            experimental_features: true,
            ..Settings::default()
        };
        settings.safety.min_duty_percent = 25.0;
        store.save(&settings).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.polling_interval_ms, 2_500);
        assert!(loaded.experimental_features);
        assert_eq!(loaded.safety.min_duty_percent, 25.0);
        assert!(!tmp.path().join("settings.json.tmp").exists());
    }

    #[test]
    fn missing_file_yields_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(tmp.path().join("nope/settings.json"));
        let loaded = store.load().unwrap();
        assert_eq!(loaded, Settings::default());
    }

    #[test]
    fn corrupt_file_is_quarantined() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(&path, "{ this is not json").unwrap();
        let store = SettingsStore::at(&path);
        let loaded = store.load().unwrap();
        assert_eq!(loaded, Settings::default());
        assert!(path.with_extension("json.corrupt").exists());
        assert!(!path.exists());
    }

    #[test]
    fn adapter_settings_bags_are_addressable() {
        let mut settings = Settings::default();
        assert!(settings.adapter_setting("mock", "enabled").is_none());
        settings.set_adapter_setting("mock", "enabled", serde_json::json!(true));
        settings.set_adapter_setting("mock", "fans", serde_json::json!(4));
        assert_eq!(
            settings.adapter_setting("mock", "enabled"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            settings
                .adapter_setting("mock", "fans")
                .and_then(|v| v.as_u64()),
            Some(4)
        );
        // A non-object value is replaced rather than panicking.
        settings
            .adapter_settings
            .insert("weird".into(), serde_json::json!("not an object"));
        settings.set_adapter_setting("weird", "key", serde_json::json!(1));
        assert_eq!(
            settings
                .adapter_setting("weird", "key")
                .and_then(|v| v.as_i64()),
            Some(1)
        );
    }

    #[test]
    fn partial_and_unknown_keys_are_tolerated() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"polling_interval_ms": 750, "from_the_future": {"x": 1}}"#,
        )
        .unwrap();
        let loaded = SettingsStore::at(&path).load().unwrap();
        assert_eq!(loaded.polling_interval_ms, 750);
        assert_eq!(loaded.history_points, Settings::default().history_points);
    }
}
