//! Shared harness for the OpenHardwareOS integration tests.
//!
//! Every test builds a *real* runtime with *real* adapters — the simulated
//! provider from `ohm-adapter-mock`, the Open Device Protocol adapter and, where
//! useful, the operating system adapter — and drives it through the same public
//! API the desktop app and the CLI use. Nothing is stubbed out except the
//! hardware itself.

use std::sync::Arc;

use ohm_adapter_api::HardwareAdapter;
use ohm_adapter_mock::{LoadProfile, MockAdapter, MockConfig, MockFaults};
use ohm_adapters::{AdapterOptions, build_adapters};
use ohm_automation::{AutomationEngine, Rule, RuleStore};
use ohm_core::{ConfigPaths, DeviceId};
use ohm_runtime::{Runtime, Settings, SettingsStore};

/// A runtime backed by a deterministic simulated machine, plus its automation
/// engine and the temporary config directory that owns them.
#[derive(Debug)]
pub struct Session {
    /// Kept alive so the temporary directory is not deleted mid-test.
    pub _temp: tempfile::TempDir,
    pub runtime: Runtime,
    pub engine: AutomationEngine,
    pub mock: Arc<MockAdapter>,
    pub paths: ConfigPaths,
}

impl Session {
    /// Simulated hardware only: deterministic, manual clock, no wall-clock
    /// dependence, nothing outside the process.
    pub async fn simulated() -> Self {
        Self::simulated_with(MockConfig::deterministic()).await
    }

    /// Simulated hardware with an explicit configuration.
    pub async fn simulated_with(mock_config: MockConfig) -> Self {
        let temp = tempfile::tempdir().expect("temporary config directory");
        let paths = ConfigPaths::from_root(temp.path());

        let mock = Arc::new(MockAdapter::new(mock_config.clone()));
        let options = AdapterOptions::simulated_only();
        let mut adapters: Vec<Arc<dyn HardwareAdapter>> = build_adapters(&options);
        // Replace the adapter the facade built with our owned handle, so tests
        // can drive the simulation directly.
        adapters.retain(|adapter| adapter.info().id.as_str() != ohm_adapter_mock::ADAPTER_ID);
        adapters.push(mock.clone());

        let settings = Settings {
            dry_run: false,
            polling_interval_ms: 100,
            discovery_interval_ms: 500,
            ..Settings::default()
        };

        let runtime = Runtime::new(paths.clone(), settings, adapters).expect("runtime");
        let engine = AutomationEngine::new(runtime.clone(), RuleStore::from_paths(&paths));

        runtime.start().await.expect("runtime start");
        engine.load_rules().expect("rules load");

        Self {
            _temp: temp,
            runtime,
            engine,
            mock,
            paths,
        }
    }

    /// A runtime with the real operating-system adapter attached, for the
    /// "does it see this machine" checks. Falls back to simulated hardware when
    /// the OS exposes nothing.
    pub async fn with_os_adapter() -> Session {
        let temp = tempfile::tempdir().expect("temporary config directory");
        let paths = ConfigPaths::from_root(temp.path());
        let mut options = AdapterOptions::simulated_only();
        options.system = true;

        let mock = Arc::new(MockAdapter::new(MockConfig::deterministic()));
        let mut adapters = build_adapters(&options);
        adapters.retain(|adapter| adapter.info().id.as_str() != ohm_adapter_mock::ADAPTER_ID);
        adapters.push(mock.clone());

        let runtime = Runtime::new(paths.clone(), Settings::default(), adapters).expect("runtime");
        let engine = AutomationEngine::new(runtime.clone(), RuleStore::from_paths(&paths));
        runtime.start().await.expect("runtime start");

        Self {
            _temp: temp,
            runtime,
            engine,
            mock,
            paths,
        }
    }

    /// Advance the simulation, publish it through the runtime, then let the
    /// automation engine react. This is the order the desktop app uses.
    pub async fn step(&self, simulated_ms: u64) {
        self.mock.tick(simulated_ms);
        self.runtime.poll_once().await.expect("poll");
        self.engine.tick_force().await;
    }

    /// Advance without letting the engine run, to observe fallbacks.
    pub async fn step_without_rules(&self, simulated_ms: u64) {
        self.mock.tick(simulated_ms);
        self.runtime.poll_once().await.expect("poll");
    }

    /// The documented GPU cooling rule, pointed at the simulated machine.
    pub fn gpu_rule(&self) -> Rule {
        Rule::new(
            "gpu-cooling",
            "GPU Cooling",
            ohm_automation::Source::sensor("gpu.mock.0", ohm_device_model::caps::TEMPERATURE_CORE),
            ohm_automation::Target::new("fan.mock.0", ohm_device_model::caps::FAN_SPEED_PERCENT),
            ohm_automation::gpu_cooling_curve(),
        )
        .expect("valid rule")
        .with_hysteresis(2.0)
        .with_deadband(0.0)
    }

    /// Read one capability through the runtime.
    pub fn reading(&self, device: &str, capability: &str) -> Option<f64> {
        self.runtime.reading(device, capability)
    }

    /// Current duty of the first simulated chassis fan.
    pub fn fan_duty(&self) -> f64 {
        self.mock
            .status()
            .fan_duties
            .first()
            .copied()
            .unwrap_or(0.0)
    }

    /// Stop everything; safe to call at the end of a test.
    pub async fn shutdown(&self) {
        self.engine.stop().await;
        let _ = self.runtime.shutdown().await;
    }
}

/// Convenience: a fault configuration that disconnects the GPU sensor.
pub fn gpu_sensor_disconnected() -> MockFaults {
    MockFaults::sensor_disconnected("gpu.mock.0", ohm_device_model::caps::TEMPERATURE_CORE)
}

/// Convenience: a load profile that heats the machine hard.
pub fn heavy_load() -> LoadProfile {
    LoadProfile::Constant { load: 1.0 }
}

/// Convenience: an idle load profile.
pub fn idle_load() -> LoadProfile {
    LoadProfile::Constant { load: 0.02 }
}

/// Every device id currently known to a runtime, sorted.
pub fn device_ids(runtime: &Runtime) -> Vec<String> {
    runtime
        .devices()
        .into_iter()
        .map(|view| view.device.id.to_string())
        .collect()
}

/// Load settings from a config directory, as the app would.
pub fn load_settings(paths: &ConfigPaths) -> Settings {
    SettingsStore::new(paths).load().expect("settings load")
}

/// A device id literal, for terser tests.
pub fn device(id: &str) -> DeviceId {
    DeviceId::new_unchecked(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_harness_starts_a_working_runtime() {
        let session = Session::simulated().await;
        let ids = device_ids(&session.runtime);
        assert!(ids.iter().any(|id| id == "gpu.mock.0"));
        assert!(ids.iter().any(|id| id == "fan.mock.0"));
        assert!(session.reading("gpu.mock.0", "temperature.core").is_some());
        session.shutdown().await;
    }

    #[tokio::test]
    async fn stepping_advances_the_simulation() {
        let session = Session::simulated_with(MockConfig {
            gpu_load: heavy_load(),
            ..MockConfig::deterministic()
        })
        .await;
        let before = session.reading("gpu.mock.0", "temperature.core").unwrap();
        for _ in 0..20 {
            session.step_without_rules(1_000).await;
        }
        let after = session.reading("gpu.mock.0", "temperature.core").unwrap();
        assert!(after > before, "{before} -> {after}");
        session.shutdown().await;
    }

    #[test]
    fn settings_round_trip_through_the_temp_dir() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::from_root(temp.path());
        let settings = Settings {
            polling_interval_ms: 250,
            ..Settings::default()
        };
        SettingsStore::new(&paths).save(&settings).unwrap();
        assert_eq!(load_settings(&paths).polling_interval_ms, 250);
    }
}
