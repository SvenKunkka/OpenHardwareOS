//! Mock hardware provider.
//!
//! Why this exists: *no* part of OpenHardwareOS may depend on the user owning
//! controllable fans. The mock provider gives you a complete, physically
//! plausible machine — a CPU, a GPU with a controllable fan, an NVMe SSD,
//! chassis fans and an AIO pump — that heats up under load and cools down when
//! the fans spin faster.
//!
//! That makes four things possible without hardware:
//!
//! * developing the UI and the runtime,
//! * observing the real `temperature -> rule -> fan speed -> temperature`
//!   feedback loop,
//! * testing edge cases on demand (write failures, disconnected sensors,
//!   unplugged devices) through [`MockFaults`],
//! * running the whole test-suite in CI.
//!
//! The simulation is deterministic when [`MockConfig::deterministic`] is used:
//! manual clock, no noise.

pub mod devices;
pub mod model;

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use ohm_adapter_api::{
    AdapterCapabilities, AdapterInfo, AdapterStatus, HardwareAdapter, WriteOutcome, WriteStatus,
};
use ohm_core::{CapabilityId, DeviceId, OhmError, Result, ids::capability as caps};
use ohm_device_model::{Capability, Device, DeviceState, Reading, UnavailableReason, Value};
use parking_lot::{Mutex, RwLock};

pub use model::{ClockMode, LoadProfile, MockConfig, MockDevices, MockFaults};

/// Adapter id, also the device id namespace.
pub const ADAPTER_ID: &str = devices::ADAPTER_ID;
/// Display name.
pub const ADAPTER_NAME: &str = devices::ADAPTER_NAME;

use devices::{cpu_id, gpu_id, instance_index, is_duty, ssd_id};
use model::MockState;

/// Snapshot of the simulator, rendered by the desktop UI's Mock panel.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MockStatus {
    pub sim_ms: u64,
    pub clock: ClockMode,
    pub ambient_c: f64,
    pub gpu_temp_c: f64,
    pub cpu_temp_c: f64,
    pub ssd_temp_c: f64,
    pub gpu_load: f64,
    pub cpu_load: f64,
    pub gpu_fan_duty: f64,
    pub gpu_fan_rpm: f64,
    pub fan_duties: Vec<f64>,
    pub fan_rpms: Vec<f64>,
    pub pump_duties: Vec<f64>,
    pub noise: f64,
    pub faults: MockFaults,
}

/// The simulated machine.
#[derive(Debug)]
pub struct MockAdapter {
    config: RwLock<MockConfig>,
    state: Mutex<MockState>,
    /// Wall clock anchor for [`ClockMode::Wall`].
    last_advance_ms: Mutex<i64>,
    /// Manual overrides set from the UI (sliders).
    overrides: RwLock<std::collections::HashMap<(DeviceId, CapabilityId), f64>>,
}

impl MockAdapter {
    /// Build a simulator with an explicit configuration.
    pub fn new(config: MockConfig) -> Self {
        let state = MockState::new(&config);
        Self {
            config: RwLock::new(config),
            state: Mutex::new(state),
            last_advance_ms: Mutex::new(ohm_core::now_ms()),
            overrides: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Build a simulator and return it as a `dyn HardwareAdapter`, ready to be
    /// registered on a runtime.
    pub fn boxed(config: MockConfig) -> Arc<dyn HardwareAdapter> {
        Arc::new(Self::new(config))
    }

    /// The default simulator: idle machine, wall clock, two fans, one probe.
    pub fn with_defaults() -> Self {
        Self::new(MockConfig::default())
    }

    /// Current configuration.
    pub fn config(&self) -> MockConfig {
        self.config.read().clone()
    }

    /// Mutate the configuration. Structural changes (adding or removing
    /// devices) take effect on the next discovery cycle.
    pub fn update_config(&self, mutate: impl FnOnce(&mut MockConfig)) {
        let mut config = self.config.write();
        let previous_devices = config.devices.clone();
        mutate(&mut config);
        if config.devices != previous_devices {
            let config = config.clone();
            let mut state = self.state.lock();
            state
                .fan_duties
                .resize(config.devices.fans, config.initial_fan_duty);
            state
                .pump_duties
                .resize(config.devices.pumps, config.initial_fan_duty.max(60.0));
        }
    }

    // ------------------------------------------------------------ UI controls

    /// Set the GPU load immediately (replaces any profile).
    pub fn set_gpu_load(&self, load: f64) {
        self.update_config(|config| config.gpu_load = LoadProfile::Constant { load });
    }

    /// Set the CPU load immediately.
    pub fn set_cpu_load(&self, load: f64) {
        self.update_config(|config| config.cpu_load = LoadProfile::Constant { load });
    }

    /// Drive both loads with a profile (the UI's "run a scenario" button).
    pub fn set_load_profile(&self, profile: LoadProfile) {
        self.update_config(|config| {
            config.gpu_load = profile.clone();
            config.cpu_load = profile;
        });
    }

    /// Only the GPU follows a profile.
    pub fn set_gpu_profile(&self, profile: LoadProfile) {
        self.update_config(|config| config.gpu_load = profile);
    }

    /// Ambient (room) temperature the simulation relaxes towards.
    pub fn set_ambient_temp(&self, celsius: f64) {
        self.update_config(|config| config.ambient_c = celsius.clamp(-20.0, 60.0));
    }

    /// Jump the GPU temperature, e.g. to demonstrate an emergency.
    pub fn force_gpu_temperature(&self, celsius: f64) {
        self.state.lock().gpu_temp_c = celsius;
    }

    /// Force the CPU temperature.
    pub fn force_cpu_temperature(&self, celsius: f64) {
        self.state.lock().cpu_temp_c = celsius;
    }

    /// Inject or clear faults.
    pub fn set_faults(&self, faults: MockFaults) {
        self.update_config(|config| config.faults = faults);
    }

    pub fn faults(&self) -> MockFaults {
        self.config.read().faults.clone()
    }

    /// Override one numeric reading ("what if the SSD is 60 °C?").
    pub fn set_reading(&self, device: &str, capability: &str, value: f64) {
        let key = (
            DeviceId::new_unchecked(device),
            CapabilityId::new_unchecked(capability),
        );
        self.overrides.write().insert(key, value);
    }

    /// Drop an override.
    pub fn clear_reading(&self, device: &str, capability: &str) {
        let key = (
            DeviceId::new_unchecked(device),
            CapabilityId::new_unchecked(capability),
        );
        self.overrides.write().remove(&key);
    }

    /// Advance a manual-clock simulation.
    pub fn tick(&self, dt_ms: u64) {
        let config = self.config.read().clone();
        self.state.lock().advance(dt_ms, &config);
    }

    /// Advance by one step and return the resulting status.
    pub fn step(&self, dt_ms: u64) -> MockStatus {
        self.tick(dt_ms);
        self.status()
    }

    /// Current simulator state, for the UI panel and for assertions.
    pub fn status(&self) -> MockStatus {
        let config = self.config.read().clone();
        let state = self.state.lock();
        MockStatus {
            sim_ms: state.sim_ms,
            clock: config.clock,
            ambient_c: config.ambient_c,
            gpu_temp_c: round1(state.gpu_temp_c),
            cpu_temp_c: round1(state.cpu_temp_c),
            ssd_temp_c: round1(state.ssd_temp_c),
            gpu_load: round3(state.gpu_load),
            cpu_load: round3(state.cpu_load),
            gpu_fan_duty: round1(state.gpu_fan_duty),
            gpu_fan_rpm: state.rpm_for(state.gpu_fan_duty, config.min_fan_rpm, config.max_fan_rpm),
            fan_duties: state.fan_duties.iter().map(|d| round1(*d)).collect(),
            fan_rpms: state
                .fan_duties
                .iter()
                .map(|d| state.rpm_for(*d, config.min_fan_rpm, config.max_fan_rpm))
                .collect(),
            pump_duties: state.pump_duties.iter().map(|d| round1(*d)).collect(),
            noise: config.noise,
            faults: config.faults,
        }
    }

    // -------------------------------------------------------------- internals

    /// Advance the wall-clock driven simulation, if needed.
    fn advance_clock(&self) {
        let config = self.config.read().clone();
        if config.clock == ClockMode::Manual {
            return;
        }
        let now = ohm_core::now_ms();
        let dt = {
            let mut last = self.last_advance_ms.lock();
            let dt = (now - *last).max(0) as u64;
            *last = now;
            dt
        };
        if dt == 0 {
            return;
        }
        self.state.lock().advance(dt, &config);
    }

    fn duty_slot(&self, device: &DeviceId) -> Option<(bool, usize)> {
        match device.as_str().split('.').next() {
            Some("fan") => instance_index(device).map(|i| (false, i)),
            Some("pump") => instance_index(device).map(|i| (true, i)),
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn reading(
        &self,
        device: &DeviceId,
        capability: &str,
        value: f64,
        unit: ohm_device_model::Unit,
        config: &MockConfig,
        state: &mut MockState,
    ) -> Reading {
        let cap_id = CapabilityId::new_unchecked(capability);
        if let Some(reason) = config.faults.reading_unavailable(device, &cap_id) {
            return Reading::unavailable(
                cap_id,
                reason,
                Some(format!("injected mock fault on {device}/{capability}")),
            );
        }
        if let Some(overridden) = self.overrides.read().get(&(device.clone(), cap_id.clone())) {
            return Reading::ok(cap_id, tidy(*overridden, unit));
        }
        let jitter = noise_amplitude(state, config.noise, unit);
        Reading::ok(cap_id, tidy(value + jitter, unit))
    }

    /// Build the readings of one device.
    fn readings_for(
        &self,
        device: &Device,
        config: &MockConfig,
        state: &mut MockState,
    ) -> Vec<Reading> {
        use model::physics;
        use ohm_device_model::Unit;

        let mut readings = Vec::new();
        let id = &device.id;
        let min_rpm = config.min_fan_rpm;
        let max_rpm = config.max_fan_rpm;

        if id == &cpu_id() {
            let temp = state.cpu_temp_c;
            let load = state.cpu_load;
            readings.push(self.reading(
                id,
                caps::TEMPERATURE_CORE,
                temp,
                Unit::Celsius,
                config,
                state,
            ));
            readings.push(self.reading(
                id,
                caps::CPU_LOAD,
                load * 100.0,
                Unit::Percent,
                config,
                state,
            ));
            readings.push(self.reading(
                id,
                caps::POWER_TOTAL,
                physics::CPU_IDLE_W + physics::CPU_LOAD_W * load,
                Unit::Watt,
                config,
                state,
            ));
            readings.push(self.reading(
                id,
                caps::CLOCK_MHZ,
                3800.0 + 1400.0 * load,
                Unit::Megahertz,
                config,
                state,
            ));
        } else if id == &gpu_id() {
            let temp = state.gpu_temp_c;
            let load = state.gpu_load;
            let duty = state.gpu_fan_duty;
            readings.push(self.reading(
                id,
                caps::TEMPERATURE_CORE,
                temp,
                Unit::Celsius,
                config,
                state,
            ));
            readings.push(self.reading(
                id,
                caps::TEMPERATURE_HOTSPOT,
                temp + physics::HOTSPOT_OFFSET_C + 6.0 * load,
                Unit::Celsius,
                config,
                state,
            ));
            readings.push(self.reading(
                id,
                caps::GPU_LOAD,
                load * 100.0,
                Unit::Percent,
                config,
                state,
            ));
            readings.push(self.reading(
                id,
                caps::POWER_GPU,
                physics::GPU_IDLE_W + physics::GPU_LOAD_W * load,
                Unit::Watt,
                config,
                state,
            ));
            readings.push(self.reading(
                id,
                caps::FAN_RPM,
                state.rpm_for(duty, min_rpm, max_rpm),
                Unit::Rpm,
                config,
                state,
            ));
            readings.push(self.reading(
                id,
                caps::FAN_SPEED_PERCENT,
                duty,
                Unit::Percent,
                config,
                state,
            ));
        } else if id == &ssd_id() {
            let temp = state.ssd_temp_c;
            readings.push(self.reading(
                id,
                caps::TEMPERATURE_CORE,
                temp,
                Unit::Celsius,
                config,
                state,
            ));
            readings.push(self.reading(
                id,
                caps::DISK_FREE,
                1_412_345_678_912.0,
                Unit::Byte,
                config,
                state,
            ));
        } else if id.as_str().starts_with("fan.") {
            let index = instance_index(id).unwrap_or(0);
            let duty = state.fan_duties.get(index).copied().unwrap_or(0.0);
            readings.push(self.reading(
                id,
                caps::FAN_RPM,
                state.rpm_for(duty, min_rpm, max_rpm),
                Unit::Rpm,
                config,
                state,
            ));
            readings.push(self.reading(
                id,
                caps::FAN_SPEED_PERCENT,
                duty,
                Unit::Percent,
                config,
                state,
            ));
        } else if id.as_str().starts_with("pump.") {
            let index = instance_index(id).unwrap_or(0);
            let duty = state.pump_duties.get(index).copied().unwrap_or(60.0);
            readings.push(self.reading(
                id,
                caps::PUMP_RPM,
                state.rpm_for(duty, config.pump_min_rpm, config.pump_max_rpm),
                Unit::Rpm,
                config,
                state,
            ));
            readings.push(self.reading(
                id,
                caps::PUMP_SPEED_PERCENT,
                duty,
                Unit::Percent,
                config,
                state,
            ));
        } else if id.as_str().starts_with("temperature.") {
            let value = (state.cpu_temp_c + state.gpu_temp_c) / 2.0 - 6.0;
            readings.push(self.reading(
                id,
                caps::TEMPERATURE_SYSTEM,
                value,
                Unit::Celsius,
                config,
                state,
            ));
        }

        readings
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// Round a reading to the precision its unit implies, so the mock does not
/// pretend to have more resolution than real hardware.
fn tidy(value: f64, unit: ohm_device_model::Unit) -> Value {
    use ohm_device_model::Unit;
    match unit {
        Unit::Celsius | Unit::Fahrenheit | Unit::Percent | Unit::Pwm => {
            Value::Number(round1(value))
        }
        Unit::Rpm | Unit::Byte | Unit::Count | Unit::Megahertz | Unit::Hertz => {
            Value::Integer(value.round() as i64)
        }
        Unit::Watt | Unit::Milliwatt => Value::Number(round1(value)),
        _ => Value::Number(value),
    }
}

/// Noise amplitude for a unit, scaled by the configured noise level.
fn noise_amplitude(state: &mut MockState, noise: f64, unit: ohm_device_model::Unit) -> f64 {
    use ohm_device_model::Unit;
    let scale = match unit {
        Unit::Celsius => 0.35,
        Unit::Percent => 0.5,
        Unit::Rpm => 12.0,
        Unit::Watt => 2.0,
        _ => 0.0,
    };
    state.noise(noise * scale)
}

#[async_trait]
impl HardwareAdapter for MockAdapter {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::new(
            devices::ADAPTER_ID,
            devices::ADAPTER_NAME,
            devices::ADAPTER_ID,
        )
        .with_description(
            "Deterministic simulated PC hardware (CPU, GPU, SSD, fans, pump). \
                 Use it to run the whole cooling loop without real hardware.",
        )
        .with_hotplug()
        .with_capabilities(AdapterCapabilities {
            can_write: true,
            can_control_cooling: true,
            write_requires_admin: false,
            poll_interval_ms: None,
            discovery_interval_ms: None,
        })
    }

    async fn probe(&self) -> AdapterStatus {
        let count = devices::build_devices(&self.config.read()).len();
        AdapterStatus::available(self.id(), count)
    }

    async fn discover(&self) -> Result<Vec<Device>> {
        let config = self.config.read().clone();
        Ok(devices::build_devices(&config)
            .into_iter()
            .filter(|device| !config.faults.is_unplugged(&device.id))
            .collect())
    }

    async fn read_state(&self, device: &Device) -> Result<DeviceState> {
        self.advance_clock();
        let config = self.config.read().clone();
        let mut state = self.state.lock();

        if config.faults.is_unplugged(&device.id) {
            return Ok(DeviceState::new(device.id.clone(), ohm_core::now_ms())
                .offline(UnavailableReason::NotPresent, "unplugged (mock fault)"));
        }

        let readings = self.readings_for(device, &config, &mut state);
        Ok(DeviceState::new(device.id.clone(), ohm_core::now_ms()).with_readings(readings))
    }

    async fn read_all(&self, devices: &[Device]) -> Vec<(DeviceId, Result<DeviceState>)> {
        // One clock advance for the whole cycle keeps every device consistent.
        self.advance_clock();
        let config = self.config.read().clone();
        let mut state = self.state.lock();
        devices
            .iter()
            .map(|device| {
                if config.faults.is_unplugged(&device.id) {
                    return (
                        device.id.clone(),
                        Ok(DeviceState::new(device.id.clone(), ohm_core::now_ms())
                            .offline(UnavailableReason::NotPresent, "unplugged (mock fault)")),
                    );
                }
                let readings = self.readings_for(device, &config, &mut state);
                (
                    device.id.clone(),
                    Ok(DeviceState::new(device.id.clone(), ohm_core::now_ms())
                        .with_readings(readings)),
                )
            })
            .collect()
    }

    async fn write(
        &self,
        device: &Device,
        capability: &Capability,
        value: &Value,
    ) -> Result<WriteOutcome> {
        // 1. Injected failures, so the fail-safe path can be tested.
        if self
            .config
            .read()
            .faults
            .write_blocked(&device.id, &capability.id)
        {
            return Err(OhmError::WriteRejected {
                device: device.id.to_string(),
                capability: capability.id.to_string(),
                detail: "injected mock write failure".into(),
            });
        }

        if !capability.writable || !is_duty(&capability.id) {
            return Err(OhmError::CapabilityNotWritable {
                device: device.id.to_string(),
                capability: capability.id.to_string(),
            });
        }
        // Belt and braces: a locked GPU fan is neither advertised nor accepted,
        // so a stale cached capability cannot drive it either.
        if device.id == gpu_id() && !self.config.read().gpu_fan_control {
            return Err(OhmError::WriteRejected {
                device: device.id.to_string(),
                capability: capability.id.to_string(),
                detail: "this simulated GPU does not allow third party fan control".into(),
            });
        }
        capability.validate(value)?;
        let requested = value.as_f64().ok_or_else(|| OhmError::InvalidValue {
            device: device.id.to_string(),
            capability: capability.id.to_string(),
            detail: format!("expected a number, got `{value}`"),
        })?;
        let applied = capability.clamp(requested);

        {
            let mut state = self.state.lock();
            if device.id == gpu_id() {
                state.gpu_fan_duty = applied;
            } else if let Some((is_pump, index)) = self.duty_slot(&device.id) {
                let slots = if is_pump {
                    &mut state.pump_duties
                } else {
                    &mut state.fan_duties
                };
                if index >= slots.len() {
                    return Err(OhmError::DeviceNotFound(device.id.to_string()));
                }
                slots[index] = applied;
            } else {
                return Err(OhmError::WriteRejected {
                    device: device.id.to_string(),
                    capability: capability.id.to_string(),
                    detail: "this simulated device has no controllable output".into(),
                });
            }
        }

        if (applied - requested).abs() > f64::EPSILON {
            return Ok(WriteOutcome {
                status: WriteStatus::Simulated,
                applied: Some(Value::Number(applied)),
                detail: Some(format!(
                    "clamped from {requested} to {applied} by the declared range"
                )),
            });
        }
        Ok(WriteOutcome::simulated(
            Value::Number(applied),
            format!("simulated by {} (mock hardware)", devices::ADAPTER_NAME),
        ))
    }

    async fn shutdown(&self) -> Result<()> {
        tracing::debug!("mock adapter shutdown: nothing to release");
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_device_model::Unit;

    fn adapter() -> MockAdapter {
        MockAdapter::new(MockConfig::deterministic())
    }

    fn device_by_id(devices: &[Device], id: &str) -> Device {
        devices
            .iter()
            .find(|d| d.id.as_str() == id)
            .expect("device present")
            .clone()
    }

    #[tokio::test]
    async fn discovery_returns_the_configured_machine() {
        let adapter = adapter();
        let devices = adapter.discover().await.unwrap();
        let ids: Vec<&str> = devices.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "cpu.mock.0",
                "gpu.mock.0",
                "ssd.mock.0",
                "fan.mock.0",
                "fan.mock.1",
                "temperature.mock.0"
            ]
        );
        for device in &devices {
            device.validate().unwrap();
        }
        assert_eq!(adapter.probe().await.device_count, 6);
    }

    #[tokio::test]
    async fn readings_are_physically_plausible() {
        let adapter = adapter();
        let devices = adapter.discover().await.unwrap();
        let gpu = device_by_id(&devices, "gpu.mock.0");
        let state = adapter.read_state(&gpu).await.unwrap();
        assert!(state.online);
        let core = state.number(caps::TEMPERATURE_CORE).unwrap();
        let hotspot = state.number(caps::TEMPERATURE_HOTSPOT).unwrap();
        assert!((20.0..110.0).contains(&core), "core={core}");
        assert!(hotspot > core, "hotspot should exceed core");
        assert_eq!(state.number(caps::FAN_RPM), Some(400.0 + 1600.0 * 0.4));
        assert!(state.value(caps::POWER_GPU).is_some());
    }

    #[tokio::test]
    async fn fan_duty_changes_rpm_and_temperature() {
        let adapter = MockAdapter::new(MockConfig {
            gpu_load: LoadProfile::Constant { load: 1.0 },
            ..MockConfig::deterministic()
        });
        let devices = adapter.discover().await.unwrap();
        let gpu = device_by_id(&devices, "gpu.mock.0");
        let control = gpu.capability_str(caps::FAN_SPEED_PERCENT).unwrap();

        adapter
            .write(&gpu, control, &Value::Number(100.0))
            .await
            .unwrap();
        for _ in 0..220 {
            adapter.tick(1_000);
        }
        let cool = adapter.read_state(&gpu).await.unwrap();

        adapter
            .write(&gpu, control, &Value::Number(0.0))
            .await
            .unwrap();
        for _ in 0..220 {
            adapter.tick(1_000);
        }
        let hot = adapter.read_state(&gpu).await.unwrap();

        assert!(
            hot.number(caps::TEMPERATURE_CORE).unwrap()
                > cool.number(caps::TEMPERATURE_CORE).unwrap() + 10.0
        );
        assert_eq!(cool.number(caps::FAN_RPM), Some(2000.0));
        assert_eq!(hot.number(caps::FAN_RPM), Some(0.0));
    }

    #[tokio::test]
    async fn writes_are_validated_and_reported_as_simulated() {
        let adapter = adapter();
        let devices = adapter.discover().await.unwrap();
        let fan = device_by_id(&devices, "fan.mock.0");
        let control = fan.capability_str(caps::FAN_SPEED_PERCENT).unwrap();

        // A legal value is applied and honestly reported as simulated.
        let outcome = adapter
            .write(&fan, control, &Value::Number(75.0))
            .await
            .unwrap();
        assert_eq!(outcome.status, WriteStatus::Simulated);
        assert_eq!(outcome.applied, Some(Value::Number(75.0)));
        assert!(outcome.detail.unwrap().contains("simulated"));

        // The adapter refuses to guess: out of range is an error, not a clamp.
        // (The runtime clamps before it ever reaches this point.)
        let err = adapter
            .write(&fan, control, &Value::Number(180.0))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "value_out_of_range");

        let err = adapter
            .write(&fan, control, &Value::Text("loud".into()))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "invalid_value");
    }

    #[tokio::test]
    async fn stepped_controls_are_quantised() {
        let adapter = adapter();
        let devices = adapter.discover().await.unwrap();
        let gpu = device_by_id(&devices, "gpu.mock.0");
        let control = gpu.capability_str(caps::FAN_SPEED_PERCENT).unwrap();
        // The mock declares a plain 0-100 range, so 42.37 lands as requested.
        let outcome = adapter
            .write(&gpu, control, &Value::Number(42.37))
            .await
            .unwrap();
        assert_eq!(outcome.applied, Some(Value::Number(42.37)));
    }

    #[tokio::test]
    async fn read_only_capabilities_cannot_be_written() {
        let adapter = adapter();
        let devices = adapter.discover().await.unwrap();
        let fan = device_by_id(&devices, "fan.mock.0");
        let rpm = fan.capability_str(caps::FAN_RPM).unwrap();
        let err = adapter
            .write(&fan, rpm, &Value::Number(1000.0))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "capability_read_only");
    }

    #[tokio::test]
    async fn injected_write_failure_is_surfaced_not_swallowed() {
        let adapter = adapter();
        adapter.set_faults(MockFaults::writes_fail());
        let devices = adapter.discover().await.unwrap();
        let fan = device_by_id(&devices, "fan.mock.0");
        let control = fan.capability_str(caps::FAN_SPEED_PERCENT).unwrap();
        let err = adapter
            .write(&fan, control, &Value::Number(50.0))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "write_rejected");
        assert!(err.hint().contains("Administrator"));
    }

    #[tokio::test]
    async fn disconnected_sensor_reports_unavailable() {
        let adapter = adapter();
        adapter.set_faults(MockFaults::sensor_disconnected(
            "gpu.mock.0",
            caps::TEMPERATURE_CORE,
        ));
        let devices = adapter.discover().await.unwrap();
        let gpu = device_by_id(&devices, "gpu.mock.0");
        let state = adapter.read_state(&gpu).await.unwrap();
        let reading = state.get(caps::TEMPERATURE_CORE).unwrap();
        assert!(!reading.is_ok());
        assert_eq!(reading.reason(), Some(UnavailableReason::ReadError));
        assert!(state.has_failure());
        // Everything else still reads fine.
        assert!(state.number(caps::GPU_LOAD).is_some());
    }

    #[tokio::test]
    async fn unplugged_device_disappears_then_returns() {
        let adapter = adapter();
        adapter.set_faults(MockFaults::device_unplugged("fan.mock.1"));
        let devices = adapter.discover().await.unwrap();
        assert!(!devices.iter().any(|d| d.id.as_str() == "fan.mock.1"));
        assert!(devices.iter().any(|d| d.id.as_str() == "fan.mock.0"));

        adapter.set_faults(MockFaults::default());
        let devices = adapter.discover().await.unwrap();
        assert!(devices.iter().any(|d| d.id.as_str() == "fan.mock.1"));
    }

    #[tokio::test]
    async fn read_all_shares_one_clock_step() {
        let adapter = MockAdapter::new(MockConfig {
            gpu_load: LoadProfile::Constant { load: 1.0 },
            ..MockConfig::deterministic()
        });
        let devices = adapter.discover().await.unwrap();
        adapter.tick(5_000);
        let before = adapter.status().gpu_temp_c;
        let results = adapter.read_all(&devices).await;
        assert_eq!(results.len(), 6);
        assert!(results.iter().all(|(_, r)| r.is_ok()));
        assert_eq!(
            adapter.status().gpu_temp_c,
            before,
            "read_all must not advance the manual clock"
        );
    }

    #[test]
    fn ui_controls_behave() {
        let adapter = adapter();
        adapter.set_gpu_load(0.8);
        adapter.step(10_000);
        assert!((adapter.status().gpu_load - 0.8).abs() < 1e-9);

        adapter.set_ambient_temp(40.0);
        assert_eq!(adapter.config().ambient_c, 40.0);
        adapter.set_ambient_temp(100.0);
        assert_eq!(adapter.config().ambient_c, 60.0);

        adapter.force_gpu_temperature(95.0);
        assert_eq!(adapter.status().gpu_temp_c, 95.0);

        adapter.set_reading("ssd.mock.0", caps::TEMPERATURE_CORE, 61.5);
        assert!(adapter.overrides.read().contains_key(&(
            ssd_id(),
            CapabilityId::new_unchecked(caps::TEMPERATURE_CORE)
        )));
        adapter.clear_reading("ssd.mock.0", caps::TEMPERATURE_CORE);
        assert!(adapter.overrides.read().is_empty());

        let status = adapter.status();
        assert_eq!(status.fan_duties.len(), 2);
        assert_eq!(status.fan_rpms.len(), 2);
        assert_eq!(status.clock, ClockMode::Manual);
    }

    #[tokio::test]
    async fn reading_overrides_and_noise() {
        let adapter = MockAdapter::new(MockConfig {
            noise: 0.5,
            ..MockConfig::deterministic()
        });
        adapter.set_reading("ssd.mock.0", caps::TEMPERATURE_CORE, 61.5);
        let devices = adapter.discover().await.unwrap();
        let ssd = device_by_id(&devices, "ssd.mock.0");
        let state = adapter.read_state(&ssd).await.unwrap();
        assert_eq!(state.number(caps::TEMPERATURE_CORE), Some(61.5));

        // With noise the value wobbles but stays plausible.
        let gpu = device_by_id(&devices, "gpu.mock.0");
        let mut seen = std::collections::HashSet::new();
        for _ in 0..20 {
            let state = adapter.read_state(&gpu).await.unwrap();
            seen.insert(state.number(caps::TEMPERATURE_CORE).unwrap().to_string());
        }
        assert!(seen.len() > 1, "noise should vary the reading");
    }

    #[test]
    fn rounding_and_tidy() {
        assert_eq!(round1(1.24), 1.2);
        assert_eq!(round3(0.1234), 0.123);
        assert_eq!(tidy(1120.4, Unit::Rpm), Value::Integer(1120));
        assert_eq!(tidy(68.44, Unit::Celsius), Value::Number(68.4));
        assert_eq!(tidy(3_800.6, Unit::Megahertz), Value::Integer(3801));
    }

    #[tokio::test]
    async fn config_changes_resize_actuator_slots() {
        let adapter = adapter();
        adapter.update_config(|config| config.devices.fans = 4);
        let devices = adapter.discover().await.unwrap();
        assert_eq!(
            devices
                .iter()
                .filter(|d| d.id.as_str().starts_with("fan."))
                .count(),
            4
        );
        assert_eq!(adapter.status().fan_duties.len(), 4);
        adapter.update_config(|config| config.devices.fans = 1);
        assert_eq!(adapter.status().fan_duties.len(), 1);
        assert_eq!(adapter.info().id.as_str(), "mock");
    }

    #[tokio::test]
    async fn a_locked_gpu_fan_refuses_writes() {
        let adapter = MockAdapter::new(MockConfig {
            gpu_fan_control: false,
            ..MockConfig::deterministic()
        });
        let devices = adapter.discover().await.unwrap();
        let gpu = device_by_id(&devices, "gpu.mock.0");
        assert!(!gpu.is_controllable(), "no writable channel is advertised");

        // Even a capability kept from an earlier discovery is refused.
        let stale = Capability::actuator(
            caps::FAN_SPEED_PERCENT,
            "GPU Fan Speed",
            Unit::Percent,
            0.0,
            100.0,
        );
        let error = adapter
            .write(&gpu, &stale, &Value::Number(50.0))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "write_rejected");
        assert!(error.to_string().contains("third party fan control"));
    }

    #[tokio::test]
    async fn shutdown_is_a_noop() {
        let adapter = adapter();
        adapter.shutdown().await.unwrap();
    }
}
