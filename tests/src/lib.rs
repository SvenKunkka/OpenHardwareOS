//! Shared harness for the OpenHardwareOS integration tests.
//!
//! Every test builds a *real* runtime with *real* adapters — the simulated
//! provider from `ohm-adapter-mock`, the Open Device Protocol adapter and, where
//! useful, the operating system adapter — and drives it through the same public
//! API the desktop app and the CLI use. Nothing is stubbed out except the
//! hardware itself.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use ohm_adapter_api::{
    AdapterCapabilities, AdapterInfo, AdapterStatus, HardwareAdapter, WriteOutcome,
};
use ohm_adapter_mock::{LoadProfile, MockAdapter, MockConfig, MockFaults};
use ohm_adapters::{AdapterOptions, build_adapters};
use ohm_automation::{AutomationEngine, Rule, RuleStore};
use ohm_core::{ConfigPaths, DeviceId};
use ohm_device_model::{
    Capability, Device, DeviceState, DeviceType, Reading, Transport, Unit, Value,
};
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

// A rig for testing control bookkeeping.
//
// Two fans and three writable channels
// (`fan.rig.0/fan.speed_percent`, `fan.rig.0/fan.pwm`, `fan.rig.1/fan.speed_percent`),
// each of which can be told to apply a write, to accept it without confirming it, or
// to refuse it. Tests assert on the calls that reached the fake hardware — the values
// asked for, in order, and what the hardware is actually left at — rather than on log
// strings, because the defect being guarded against is a write that should not have
// happened.

/// The two simulated fans, and the channels they expose.
pub const RIG0: &str = "fan.rig.0";
pub const RIG1: &str = "fan.rig.1";
pub const PERCENT: &str = "fan.speed_percent";
pub const PWM: &str = "fan.pwm";

/// The duty a channel is left at when nobody drives it any more: the fail-safe duty
/// from the default safety policy.
pub const FAIL_SAFE: f64 = 70.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    /// The write is applied and the hardware is known to be at the value.
    Applied,
    /// The device accepted the request; the resulting value is unknown.
    Unconfirmed,
    /// The device refused the write outright.
    Refusing,
}

#[derive(Debug, Default, Clone)]
struct ChannelLog {
    attempts: usize,
    requested: Vec<f64>,
    held: f64,
}

/// A two-fan rig whose channels can misbehave on demand.
#[derive(Debug)]
pub struct Rig {
    channels: parking_lot::Mutex<HashMap<String, ChannelLog>>,
    behaviour: parking_lot::Mutex<HashMap<String, Behaviour>>,
    /// `device/capability` pairs that report as unavailable, like an unplugged probe
    /// or a sensor the driver stopped answering for.
    unavailable: parking_lot::Mutex<std::collections::HashSet<String>>,
}

impl Rig {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            channels: parking_lot::Mutex::new(HashMap::new()),
            behaviour: parking_lot::Mutex::new(HashMap::new()),
            unavailable: parking_lot::Mutex::new(std::collections::HashSet::new()),
        })
    }

    /// Make a reading unavailable (a disconnected probe, a sensor that stopped
    /// answering). Used to exercise what a rule does when it cannot see its source.
    pub fn set_reading_unavailable(&self, device: &str, capability: &str) {
        self.unavailable
            .lock()
            .insert(Self::key(device, capability));
    }

    /// The sensor answers again.
    pub fn set_reading_available(&self, device: &str, capability: &str) {
        self.unavailable
            .lock()
            .remove(&Self::key(device, capability));
    }

    pub fn key(device: &str, capability: &str) -> String {
        format!("{device}/{capability}")
    }

    pub fn set_behaviour(&self, device: &str, capability: &str, behaviour: Behaviour) {
        self.behaviour
            .lock()
            .insert(Self::key(device, capability), behaviour);
    }

    /// How many write attempts reached this channel.
    pub fn attempts(&self, device: &str, capability: &str) -> usize {
        self.channels
            .lock()
            .get(&Self::key(device, capability))
            .map(|log| log.attempts)
            .unwrap_or(0)
    }

    /// Every value that was asked of this channel, in order.
    pub fn requested(&self, device: &str, capability: &str) -> Vec<f64> {
        self.channels
            .lock()
            .get(&Self::key(device, capability))
            .map(|log| log.requested.clone())
            .unwrap_or_default()
    }

    /// What the fake hardware is actually at: only a confirmed write moves this.
    pub fn held(&self, device: &str, capability: &str) -> f64 {
        self.channels
            .lock()
            .get(&Self::key(device, capability))
            .map(|log| log.held)
            .unwrap_or(0.0)
    }

    /// Values this channel was asked for by a *safety* write, i.e. a handover.
    pub fn handover_requests(&self, device: &str, capability: &str) -> usize {
        self.requested(device, capability)
            .iter()
            .filter(|value| **value == FAIL_SAFE)
            .count()
    }
}

#[async_trait]
impl HardwareAdapter for Rig {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::new("rig", "Test Rig", "rig")
            .with_capabilities(AdapterCapabilities::cooling_control())
    }

    async fn probe(&self) -> AdapterStatus {
        AdapterStatus::available(self.id(), 2)
    }

    async fn discover(&self) -> ohm_core::Result<Vec<Device>> {
        Ok(vec![
            Device::new(
                DeviceId::new(RIG0).unwrap(),
                "Rig Fan 0",
                DeviceType::Fan,
                Transport::Mock,
                self.id(),
            )
            .with_capability(Capability::sensor(
                "temperature.core",
                "Temperature",
                Unit::Celsius,
            ))
            .with_capability(Capability::actuator(
                PERCENT,
                "Fan Speed",
                Unit::Percent,
                0.0,
                100.0,
            ))
            // A second writable channel on the *same* device, so that "same device,
            // different capability" is a real target change.
            .with_capability(Capability::actuator(
                PWM,
                "Fan PWM",
                Unit::Pwm,
                0.0,
                255.0,
            )),
            Device::new(
                DeviceId::new(RIG1).unwrap(),
                "Rig Fan 1",
                DeviceType::Fan,
                Transport::Mock,
                self.id(),
            )
            .with_capability(Capability::sensor(
                "temperature.core",
                "Temperature",
                Unit::Celsius,
            ))
            .with_capability(Capability::actuator(
                PERCENT,
                "Fan Speed",
                Unit::Percent,
                0.0,
                100.0,
            )),
        ])
    }

    async fn read_state(&self, device: &Device) -> ohm_core::Result<DeviceState> {
        let mut state = DeviceState::new(device.id.clone(), ohm_core::now_ms());
        let sensor = Self::key(device.id.as_str(), "temperature.core");
        if self.unavailable.lock().contains(&sensor) {
            state = state.with_reading(Reading::unavailable(
                "temperature.core",
                ohm_device_model::UnavailableReason::ReadError,
                Some("the probe stopped answering".into()),
            ));
        } else {
            state = state.with_reading(Reading::ok("temperature.core", 50.0));
        }
        for capability in &device.capabilities {
            if capability.kind == ohm_device_model::CapabilityKind::Actuator {
                state = state.with_reading(Reading::ok(
                    capability.id.as_str(),
                    self.held(device.id.as_str(), capability.id.as_str()),
                ));
            }
        }
        Ok(state)
    }

    async fn write(
        &self,
        device: &Device,
        capability: &Capability,
        value: &Value,
    ) -> ohm_core::Result<WriteOutcome> {
        let key = Self::key(device.id.as_str(), capability.id.as_str());
        let requested = value.as_f64().unwrap_or_default();
        let behaviour = self
            .behaviour
            .lock()
            .get(&key)
            .copied()
            .unwrap_or(Behaviour::Applied);
        {
            let mut channels = self.channels.lock();
            let log = channels.entry(key.clone()).or_default();
            log.attempts += 1;
            log.requested.push(requested);
            // Whatever the outcome, the fake hardware only *moves* when the write is
            // confirmed. An unconfirmed or refused write leaves it where it was.
            if behaviour == Behaviour::Applied {
                log.held = requested;
            }
        }
        match behaviour {
            Behaviour::Applied => Ok(WriteOutcome::applied(Value::Number(requested))),
            Behaviour::Unconfirmed => Ok(WriteOutcome::unconfirmed(format!(
                "the channel {key} could not be read back after accepting {requested:.0}"
            ))),
            Behaviour::Refusing => Ok(WriteOutcome::rejected(format!(
                "the channel {key} refused the write"
            ))),
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A rule with a flat curve, so the requested value is unambiguous.
pub fn flat_rule(id: &str, target_device: &str, target_capability: &str, output: f64) -> Rule {
    Rule::new(
        id,
        format!("Rule {id}").as_str(),
        ohm_automation::Source::sensor(RIG0, "temperature.core"),
        ohm_automation::Target::new(target_device, target_capability),
        ohm_automation::Curve::expect([(0.0, output), (100.0, output)]),
    )
    .expect("valid rule")
    .with_deadband(0.0)
}

/// A runtime, engine and rig for control-bookkeeping tests.
pub async fn rig_session() -> (tempfile::TempDir, Runtime, AutomationEngine, Arc<Rig>) {
    let temp = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::from_root(temp.path());
    let rig = Rig::new();
    let adapter: Arc<dyn HardwareAdapter> = Arc::clone(&rig) as Arc<dyn HardwareAdapter>;
    let runtime = Runtime::new(paths.clone(), Settings::default(), vec![adapter]).unwrap();
    runtime.start().await.unwrap();
    let engine = AutomationEngine::new(runtime.clone(), RuleStore::from_paths(&paths));
    (temp, runtime, engine, rig)
}
