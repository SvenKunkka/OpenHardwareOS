//! Bundled adapters and the policy that decides which ones run.
//!
//! The runtime does not know this crate exists: it is handed a
//! `Vec<Arc<dyn HardwareAdapter>>` and works with whatever is in it. This module
//! exists so that the app, the CLI and the tests all assemble that list the same
//! way instead of each inventing its own.
//!
//! # Registration order is priority order
//!
//! ```text
//! lhm   LibreHardwareMonitor — richest real hardware model, the only source of
//!                             motherboard fan RPM *and* fan control
//! nvidia NVML                 — only when LHM is not already reporting GPUs
//! system OS APIs              — always, read-only, no elevation
//! opd   Open Device Protocol  — future OpenHub/OpenFan (simulated today)
//! mock  simulated machine     — opt-in, for demos, tests and CI
//! ```
//!
//! # Why NVML steps aside while LHM is *usable*
//!
//! Both see the same physical GPU, so registering both would show the user two
//! `RTX 5090` entries with half the readings each. LHM reports *more* (it reads
//! the hotspot through NVAPI, which NVML does not expose), so LHM wins — but only
//! while it is actually there. The adapter declares the relationship
//! (`AdapterInfo::yields_to`) and the **runtime** decides from the probe result on
//! every discovery cycle.
//!
//! This used to be decided from intent: NVML was not even constructed when
//! `adapter_settings` enabled LHM. On Linux, or on Windows with
//! LibreHardwareMonitor not running, that left the machine with no GPU provider at
//! all — and the user had no way to tell why. Setting
//! `adapter_settings.nvidia.always = true` still forces NVML to stay registered
//! whatever LHM does, for users who prefer NVML's fan control.

pub mod prelude {
    pub use ohm_adapter_api::{AdapterInfo, AdapterStatus, HardwareAdapter};
    pub use ohm_adapter_lhm::LhmAdapter;
    pub use ohm_adapter_mock::MockAdapter;
    pub use ohm_adapter_nvidia::NvidiaAdapter;
    pub use ohm_adapter_opd::OpdAdapter;
    pub use ohm_adapter_system::SystemAdapter;
}

use std::sync::Arc;

use ohm_adapter_api::{AdapterInfo, HardwareAdapter};
use ohm_adapter_lhm::{LhmAdapter, WebConfig};
use ohm_adapter_mock::{MockAdapter, MockConfig};
use ohm_adapter_nvidia::NvidiaAdapter;
use ohm_adapter_opd::OpdAdapter;
use ohm_adapter_system::SystemAdapter;
use ohm_runtime::Settings;

/// Which providers to instantiate, and how.
#[derive(Debug, Clone, PartialEq)]
pub struct AdapterOptions {
    /// Operating system telemetry. Read-only, never elevation.
    pub system: bool,
    /// LibreHardwareMonitor web server.
    pub lhm: bool,
    /// LibreHardwareMonitor connection settings.
    pub lhm_config: WebConfig,
    /// NVML GPU telemetry.
    pub nvidia: bool,
    /// `true` when NVML stands by while LibreHardwareMonitor is usable. Set from
    /// `adapter_settings.nvidia.always`: forcing NVML keeps it registered even
    /// when LHM is there.
    pub nvidia_fallback: bool,
    /// Open Device Protocol devices.
    pub protocol_device: bool,
    /// Simulated hardware.
    pub mock: bool,
    /// Configuration of the simulated machine.
    pub mock_config: MockConfig,
}

impl Default for AdapterOptions {
    fn default() -> Self {
        Self {
            system: true,
            lhm: true,
            lhm_config: WebConfig::default(),
            nvidia: true,
            nvidia_fallback: true,
            protocol_device: false,
            mock: false,
            mock_config: MockConfig::default(),
        }
    }
}

impl AdapterOptions {
    /// The options a real installation should start with: every real provider
    /// on, nothing simulated.
    pub fn production() -> Self {
        Self::default()
    }

    /// Everything on, including simulated hardware — the demo and CI profile.
    pub fn with_simulated() -> Self {
        Self {
            protocol_device: true,
            mock: true,
            mock_config: MockConfig::gaming_demo(),
            // LHM is on, so NVML defers to it exactly as it does in production.
            nvidia: false,
            ..Self::default()
        }
    }

    /// Simulated hardware only: deterministic, no real hardware touched. This is
    /// what the test-suite uses.
    pub fn simulated_only() -> Self {
        Self {
            system: false,
            lhm: false,
            nvidia: false,
            nvidia_fallback: true,
            lhm_config: WebConfig::default(),
            protocol_device: true,
            mock: true,
            mock_config: MockConfig::deterministic(),
        }
    }

    /// Build from persisted settings.
    ///
    /// * providers are on unless the user disabled them
    /// * simulated hardware follows `experimental_features`, and can always be
    ///   forced on through `adapter_settings.mock.enabled`
    /// * NVML is registered whenever it is enabled, and stands by *while LHM is
    ///   usable* — the runtime decides that from the probe, not from the settings
    ///   (see the module docs). `adapter_settings.nvidia.always` removes the
    ///   standby relationship entirely.
    pub fn from_settings(settings: &Settings) -> Self {
        let lhm_enabled = settings.adapter_enabled(ohm_adapter_lhm::ADAPTER_ID);
        // NVML is a fallback for LHM: both describe the same GPU, and LHM reports
        // more. `nvidia.always` opts out of that relationship.
        let nvidia_forced = settings
            .adapter_setting("nvidia", "always")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let mock_requested = settings
            .adapter_setting("mock", "enabled")
            .and_then(|value| value.as_bool());
        let mock = match mock_requested {
            Some(explicit) => explicit,
            None => settings.experimental_features,
        };

        Self {
            system: settings.adapter_enabled(ohm_adapter_system::ADAPTER_ID),
            lhm: lhm_enabled,
            lhm_config: WebConfig::from_json(settings.adapter_config(ohm_adapter_lhm::ADAPTER_ID)),
            // Registered on its own merits. Whether it ends up reporting devices
            // is decided after probing, in the runtime.
            nvidia: settings.adapter_enabled(ohm_adapter_nvidia::ADAPTER_ID),
            nvidia_fallback: !nvidia_forced,
            protocol_device: settings.enable_mock_protocol_device && settings.experimental_features,
            mock,
            mock_config: mock_config_from_settings(settings),
        }
    }

    /// Every provider that would be instantiated, in registration order.
    pub fn enabled_ids(&self) -> Vec<&'static str> {
        let mut ids = Vec::new();
        if self.lhm {
            ids.push(ohm_adapter_lhm::ADAPTER_ID);
        }
        if self.nvidia {
            ids.push(ohm_adapter_nvidia::ADAPTER_ID);
        }
        if self.system {
            ids.push(ohm_adapter_system::ADAPTER_ID);
        }
        if self.protocol_device {
            ids.push(ohm_adapter_opd::ADAPTER_ID);
        }
        if self.mock {
            ids.push(ohm_adapter_mock::ADAPTER_ID);
        }
        ids
    }
}

/// Read the simulated machine's configuration from `adapter_settings.mock`.
fn mock_config_from_settings(settings: &Settings) -> MockConfig {
    let mut config = MockConfig::default();
    let Some(bag) = settings.adapter_config(ohm_adapter_mock::ADAPTER_ID) else {
        return config;
    };
    if let Some(profile) = bag.get("profile").and_then(|value| value.as_str()) {
        config = match profile {
            "idle" => MockConfig::idle(),
            "gaming" => MockConfig::gaming_demo(),
            "deterministic" => MockConfig::deterministic(),
            _ => config,
        };
    }
    if let Some(noise) = bag.get("noise").and_then(|value| value.as_f64()) {
        config.noise = noise.clamp(0.0, 10.0);
    }
    if let Some(fans) = bag.get("fans").and_then(|value| value.as_u64()) {
        config.devices.fans = (fans as usize).min(8);
    }
    if let Some(pumps) = bag.get("pumps").and_then(|value| value.as_u64()) {
        config.devices.pumps = (pumps as usize).min(4);
    }
    if let Some(enabled) = bag
        .get("enable_gpu_fan_control")
        .and_then(|value| value.as_bool())
    {
        // Reproduces a machine whose GPU fan cannot be driven: the capability is
        // not advertised, so no rule can target it.
        config.gpu_fan_control = enabled;
    }
    config
}

/// Instantiate the adapters described by `options`.
pub fn build_adapters(options: &AdapterOptions) -> Vec<Arc<dyn HardwareAdapter>> {
    let mut adapters: Vec<Arc<dyn HardwareAdapter>> = Vec::new();

    if options.lhm {
        adapters.push(LhmAdapter::boxed(options.lhm_config.clone()));
    }
    if options.nvidia {
        let primary = options
            .nvidia_fallback
            .then(|| ohm_core::AdapterId::new_unchecked(ohm_adapter_lhm::ADAPTER_ID));
        adapters.push(NvidiaAdapter::boxed_yielding_to(primary));
    }
    if options.system {
        adapters.push(SystemAdapter::boxed());
    }
    if options.protocol_device {
        adapters.push(OpdAdapter::boxed());
    }
    if options.mock {
        adapters.push(MockAdapter::boxed(options.mock_config.clone()));
    }

    adapters
}

/// Static information about every provider the build can offer, including the
/// ones that are currently disabled. Used by Settings -> Providers.
pub fn adapter_catalogue() -> Vec<AdapterInfo> {
    vec![
        LhmAdapter::new(WebConfig::default()).info(),
        NvidiaAdapter::new().info(),
        SystemAdapter::new().info(),
        OpdAdapter::new().info(),
        MockAdapter::new(MockConfig::default()).info(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_are_production_safe() {
        let options = AdapterOptions::default();
        assert!(options.system);
        assert!(options.lhm);
        assert!(options.nvidia);
        assert!(!options.mock, "simulated hardware must be opt-in");
        assert!(!options.protocol_device);
        assert_eq!(options.enabled_ids(), vec!["lhm", "nvidia", "system"]);
    }

    #[test]
    fn simulated_profiles() {
        let demo = AdapterOptions::with_simulated();
        assert!(demo.mock && demo.protocol_device && demo.system && demo.lhm);
        assert!(!demo.nvidia, "NVML defers to LHM here too");
        assert_eq!(demo.enabled_ids(), vec!["lhm", "system", "opd", "mock"]);

        let only = AdapterOptions::simulated_only();
        assert!(!only.system && !only.lhm && !only.nvidia);
        assert!(only.mock_config.clock == ohm_adapter_mock::ClockMode::Manual);
        assert_eq!(only.enabled_ids(), vec!["opd", "mock"]);
    }

    #[test]
    fn settings_drive_the_selection() {
        let mut settings = Settings::default();
        // Default: real providers only.
        let options = AdapterOptions::from_settings(&settings);
        assert!(options.system && options.lhm);
        // NVML is registered even though LHM is enabled: the question "is LHM
        // actually there?" is answered by the probe, not by the settings. It used
        // to be dropped here, which left Linux and any Windows machine without
        // LibreHardwareMonitor running with no GPU provider at all.
        assert!(options.nvidia, "availability decides, not intent");
        assert!(options.nvidia_fallback, "it stands by while LHM is usable");
        assert!(!options.mock);

        // Simulated hardware follows the experimental switch.
        settings.experimental_features = true;
        settings.enable_mock_protocol_device = true;
        let options = AdapterOptions::from_settings(&settings);
        assert!(options.mock);
        assert!(options.protocol_device);
        assert_eq!(
            options.enabled_ids(),
            vec!["lhm", "nvidia", "system", "opd", "mock"]
        );

        // `nvidia.always` removes the standby relationship: NVML reports its
        // devices even when LHM is there.
        settings.set_adapter_setting("nvidia", "always", serde_json::json!(true));
        let forced = AdapterOptions::from_settings(&settings);
        assert!(forced.nvidia && !forced.nvidia_fallback);

        // Disabling a provider removes it.
        settings.set_adapter_enabled(
            &ohm_core::AdapterId::new(ohm_adapter_lhm::ADAPTER_ID).unwrap(),
            false,
        );
        let options = AdapterOptions::from_settings(&settings);
        assert!(!options.lhm);
        assert!(options.nvidia, "NVML stays registered when LHM is off");
        assert_eq!(
            options.enabled_ids(),
            vec!["nvidia", "system", "opd", "mock"]
        );
    }

    #[test]
    fn the_fallback_relationship_is_declared_on_the_adapter() {
        // The runtime needs the declaration to arbitrate; without it both
        // providers would report the same GPU.
        let with_fallback = AdapterOptions {
            lhm: true,
            nvidia: true,
            nvidia_fallback: true,
            ..AdapterOptions::simulated_only()
        };
        let nvidia = build_adapters(&with_fallback)
            .into_iter()
            .find(|adapter| adapter.info().id.as_str() == ohm_adapter_nvidia::ADAPTER_ID)
            .expect("nvidia registered");
        assert_eq!(
            nvidia.info().yields_to.map(|id| id.to_string()),
            Some(ohm_adapter_lhm::ADAPTER_ID.to_string())
        );

        let forced = AdapterOptions {
            nvidia_fallback: false,
            ..with_fallback
        };
        let nvidia = build_adapters(&forced)
            .into_iter()
            .find(|adapter| adapter.info().id.as_str() == ohm_adapter_nvidia::ADAPTER_ID)
            .expect("nvidia registered");
        assert!(
            nvidia.info().yields_to.is_none(),
            "asking for NVML explicitly must remove the standby relationship"
        );
    }

    #[test]
    fn settings_can_force_nvml_and_mock() {
        let mut settings = Settings::default();
        settings.set_adapter_setting("nvidia", "always", serde_json::json!(true));
        settings.set_adapter_setting("mock", "enabled", serde_json::json!(true));
        let options = AdapterOptions::from_settings(&settings);
        assert!(options.nvidia, "explicitly requested");
        assert!(options.mock, "explicitly requested");
    }

    #[test]
    fn the_gpu_fan_control_switch_is_real() {
        // Regression: this key used to be a no-op branch that set a field to the
        // value it already had, so the setting did nothing at all.
        let mut settings = Settings::default();
        assert!(mock_config_from_settings(&settings).gpu_fan_control);

        settings.set_adapter_setting("mock", "enable_gpu_fan_control", serde_json::json!(false));
        assert!(
            !mock_config_from_settings(&settings).gpu_fan_control,
            "disabling GPU fan control must reach the simulated machine"
        );

        settings.set_adapter_setting("mock", "enable_gpu_fan_control", serde_json::json!(true));
        assert!(mock_config_from_settings(&settings).gpu_fan_control);
    }

    #[test]
    fn mock_configuration_comes_from_settings() {
        let mut settings = Settings::default();
        settings.set_adapter_setting("mock", "profile", serde_json::json!("gaming"));
        settings.set_adapter_setting("mock", "noise", serde_json::json!(0.5));
        settings.set_adapter_setting("mock", "fans", serde_json::json!(4));
        settings.set_adapter_setting("mock", "pumps", serde_json::json!(1));
        let config = mock_config_from_settings(&settings);
        assert_eq!(config.devices.fans, 4);
        assert_eq!(config.devices.pumps, 1);
        assert_eq!(config.noise, 0.5);
        assert!(config.gpu_load.load_at(0) < 0.2);

        settings.set_adapter_setting("mock", "noise", serde_json::json!(100));
        assert_eq!(mock_config_from_settings(&settings).noise, 10.0);
    }

    #[test]
    fn building_adapters_follows_the_options() {
        let adapters = build_adapters(&AdapterOptions::simulated_only());
        let ids: Vec<String> = adapters.iter().map(|a| a.info().id.to_string()).collect();
        assert_eq!(ids, vec!["opd", "mock"]);

        let adapters = build_adapters(&AdapterOptions::production());
        assert_eq!(adapters.len(), 3);
    }

    #[test]
    fn catalogue_lists_every_provider() {
        let catalogue = adapter_catalogue();
        let ids: Vec<&str> = catalogue.iter().map(|info| info.id.as_str()).collect();
        assert_eq!(ids, vec!["lhm", "nvidia", "system", "opd", "mock"]);
        for info in &catalogue {
            assert!(!info.description.is_empty(), "{:?}", info.id);
            assert!(!info.name.is_empty());
        }
        // Only the cooling providers advertise write access.
        assert!(catalogue[0].capabilities.can_control_cooling);
        assert!(!catalogue[2].capabilities.can_write);
    }
}
