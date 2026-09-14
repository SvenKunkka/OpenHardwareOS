//! Simulation model behind the mock adapter.
//!
//! The mock hardware is not a set of frozen numbers: it is a small thermal
//! model. Fan duty affects temperature, temperature follows load, and load
//! follows a profile. That is what makes the closed loop
//! `Mock GPU temperature -> automation rule -> Mock fan speed -> lower GPU
//! temperature` observable without any real hardware.

use ohm_core::{CapabilityId, DeviceId};
use ohm_device_model::UnavailableReason;
use serde::{Deserialize, Serialize};

/// How the simulation clock advances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClockMode {
    /// Follow the wall clock. Used by the running app.
    #[default]
    Wall,
    /// Only advance when [`MockAdapter::tick`](crate::MockAdapter::tick) is
    /// called. Used by deterministic tests.
    Manual,
}

/// A synthetic load curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoadProfile {
    /// Fixed load, e.g. `0.1` for idle.
    Constant { load: f64 },
    /// Smooth wave between `min` and `max`, useful for demos.
    Wave {
        min: f64,
        max: f64,
        period_ms: u64,
        phase: f64,
    },
    /// A staircase: `(offset_ms, load)` steps, repeating after the last one.
    Steps { steps: Vec<(u64, f64)> },
}

impl Default for LoadProfile {
    fn default() -> Self {
        Self::Constant { load: 0.05 }
    }
}

impl LoadProfile {
    /// Load in `0.0..=1.0` at simulation time `t_ms`.
    pub fn load_at(&self, t_ms: u64) -> f64 {
        match self {
            Self::Constant { load } => load.clamp(0.0, 1.0),
            Self::Wave {
                min,
                max,
                period_ms,
                phase,
            } => {
                let period = (*period_ms).max(1) as f64;
                let x = ((t_ms as f64 / period) * std::f64::consts::TAU + phase).sin();
                let mid = (min + max) / 2.0;
                let amplitude = (max - min) / 2.0;
                (mid + amplitude * x).clamp(0.0, 1.0)
            }
            Self::Steps { steps } => {
                if steps.is_empty() {
                    return 0.0;
                }
                let total: u64 = steps.iter().map(|(d, _)| *d).sum();
                let t = if total == 0 { 0 } else { t_ms % total };
                let mut acc = 0;
                for (duration, load) in steps {
                    acc += *duration;
                    if t < acc {
                        return load.clamp(0.0, 1.0);
                    }
                }
                steps.last().map(|(_, l)| *l).unwrap_or(0.0).clamp(0.0, 1.0)
            }
        }
    }

    /// A demo profile: idle -> gaming -> idle, on a 40 second loop.
    pub fn gaming_demo() -> Self {
        Self::Steps {
            steps: vec![(10_000, 0.08), (15_000, 0.95), (15_000, 0.35)],
        }
    }
}

/// Injected faults, used by the test-suite to prove the runtime degrades
/// gracefully instead of pretending everything works.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MockFaults {
    /// Every write fails, as if the driver refused it.
    pub fail_all_writes: bool,
    /// Writes to these `(device, capability)` pairs fail.
    pub fail_writes_on: Vec<(DeviceId, CapabilityId)>,
    /// These `(device, capability)` pairs report as unavailable instead of a
    /// value, simulating a disconnected or unsupported sensor.
    pub unavailable_readings: Vec<(DeviceId, CapabilityId, UnavailableReason)>,
    /// The device disappears from discovery entirely.
    pub unplug_devices: Vec<DeviceId>,
}

impl MockFaults {
    /// A mock where writes fail, to exercise the fail-safe path.
    pub fn writes_fail() -> Self {
        Self {
            fail_all_writes: true,
            ..Default::default()
        }
    }

    /// A mock where one sensor stops reporting.
    pub fn sensor_disconnected(device: &str, capability: &str) -> Self {
        Self {
            unavailable_readings: vec![(
                DeviceId::new_unchecked(device),
                CapabilityId::new_unchecked(capability),
                UnavailableReason::ReadError,
            )],
            ..Default::default()
        }
    }

    /// A mock where a device is unplugged (hotplug test).
    pub fn device_unplugged(device: &str) -> Self {
        Self {
            unplug_devices: vec![DeviceId::new_unchecked(device)],
            ..Default::default()
        }
    }

    pub fn write_blocked(&self, device: &DeviceId, capability: &CapabilityId) -> bool {
        self.fail_all_writes
            || self
                .fail_writes_on
                .iter()
                .any(|(d, c)| d == device && c == capability)
    }

    pub fn reading_unavailable(
        &self,
        device: &DeviceId,
        capability: &CapabilityId,
    ) -> Option<UnavailableReason> {
        self.unavailable_readings
            .iter()
            .find(|(d, c, _)| d == device && c == capability)
            .map(|(_, _, reason)| *reason)
    }

    pub fn is_unplugged(&self, device: &DeviceId) -> bool {
        self.unplug_devices.contains(device)
    }
}

/// Which simulated devices exist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MockDevices {
    pub cpu: bool,
    pub gpu: bool,
    pub ssd: bool,
    pub fans: usize,
    pub pumps: usize,
    pub temperature_sensors: usize,
}

impl Default for MockDevices {
    fn default() -> Self {
        Self {
            cpu: true,
            gpu: true,
            ssd: true,
            fans: 2,
            pumps: 0,
            temperature_sensors: 1,
        }
    }
}

/// Everything that configures the simulated machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MockConfig {
    pub devices: MockDevices,
    pub clock: ClockMode,
    /// Room temperature the simulation relaxes towards, in Celsius.
    pub ambient_c: f64,
    /// Starting GPU temperature in Celsius.
    pub gpu_start_c: f64,
    /// Starting CPU temperature in Celsius.
    pub cpu_start_c: f64,
    /// Starting SSD temperature in Celsius.
    pub ssd_start_c: f64,
    pub gpu_load: LoadProfile,
    pub cpu_load: LoadProfile,
    /// Noise added to readings, in the unit of the reading. `0.0` is fully
    /// deterministic.
    pub noise: f64,
    /// Deterministic seed for the noise generator.
    pub seed: u64,
    /// Does the simulated GPU expose a writable fan channel?
    ///
    /// Setting this to `false` reproduces the machines (and the vendor-locked
    /// SKUs) where the GPU fan simply cannot be driven by third party software:
    /// the capability is not advertised at all, so no rule can target it.
    pub gpu_fan_control: bool,
    /// Fan duty applied at startup, in percent.
    pub initial_fan_duty: f64,
    pub min_fan_rpm: f64,
    pub max_fan_rpm: f64,
    pub pump_min_rpm: f64,
    pub pump_max_rpm: f64,
    /// Default faults.
    pub faults: MockFaults,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            devices: MockDevices::default(),
            clock: ClockMode::Wall,
            ambient_c: 25.0,
            gpu_start_c: 42.0,
            cpu_start_c: 38.0,
            ssd_start_c: 33.0,
            gpu_load: LoadProfile::default(),
            cpu_load: LoadProfile::default(),
            noise: 0.0,
            seed: 0x5EED_1234_ABCD_0001,
            gpu_fan_control: true,
            initial_fan_duty: 40.0,
            min_fan_rpm: 400.0,
            max_fan_rpm: 2000.0,
            pump_min_rpm: 1800.0,
            pump_max_rpm: 3000.0,
            faults: MockFaults::default(),
        }
    }
}

impl MockConfig {
    /// Fully deterministic: manual clock, no noise, fixed initial values.
    pub fn deterministic() -> Self {
        Self {
            clock: ClockMode::Manual,
            noise: 0.0,
            ..Default::default()
        }
    }

    /// A quiet idle machine.
    pub fn idle() -> Self {
        Self {
            gpu_load: LoadProfile::Constant { load: 0.02 },
            cpu_load: LoadProfile::Constant { load: 0.05 },
            ..Default::default()
        }
    }

    /// A machine under a gaming-like load cycle, for demos.
    pub fn gaming_demo() -> Self {
        Self {
            gpu_load: LoadProfile::gaming_demo(),
            cpu_load: LoadProfile::Wave {
                min: 0.15,
                max: 0.85,
                period_ms: 18_000,
                phase: 0.0,
            },
            ..Default::default()
        }
    }
}

/// Thermal constants of the simulation.
pub(crate) mod physics {
    /// Extra degrees at full load with no airflow.
    pub const GPU_LOAD_HEAT_C: f64 = 72.0;
    /// Degrees removed by a fan at 100 % duty.
    pub const GPU_FAN_COOLING_C: f64 = 38.0;
    pub const GPU_TIME_CONSTANT_S: f64 = 7.0;

    pub const CPU_LOAD_HEAT_C: f64 = 58.0;
    pub const CPU_FAN_COOLING_C: f64 = 26.0;
    pub const CPU_TIME_CONSTANT_S: f64 = 5.0;

    pub const SSD_LOAD_HEAT_C: f64 = 9.0;
    pub const SSD_TIME_CONSTANT_S: f64 = 45.0;

    /// Idle power draw.
    pub const GPU_IDLE_W: f64 = 28.0;
    pub const GPU_LOAD_W: f64 = 420.0;
    pub const CPU_IDLE_W: f64 = 22.0;
    pub const CPU_LOAD_W: f64 = 130.0;

    /// Hotspot is always above the core temperature.
    pub const HOTSPOT_OFFSET_C: f64 = 11.0;
}

/// Mutable simulation state.
#[derive(Debug, Clone)]
pub(crate) struct MockState {
    /// Milliseconds of simulated time.
    pub sim_ms: u64,
    pub gpu_temp_c: f64,
    pub cpu_temp_c: f64,
    pub ssd_temp_c: f64,
    pub gpu_load: f64,
    pub cpu_load: f64,
    /// Fan duties in percent, indexed like `fan.mock.N`.
    pub fan_duties: Vec<f64>,
    /// Pump duties in percent, indexed like `pump.mock.N`.
    pub pump_duties: Vec<f64>,
    /// GPU fan duty (the GPU owns its own fan).
    pub gpu_fan_duty: f64,
    pub rng: u64,
}

impl MockState {
    pub fn new(config: &MockConfig) -> Self {
        Self {
            sim_ms: 0,
            gpu_temp_c: config.gpu_start_c,
            cpu_temp_c: config.cpu_start_c,
            ssd_temp_c: config.ssd_start_c,
            gpu_load: config.gpu_load.load_at(0),
            cpu_load: config.cpu_load.load_at(0),
            fan_duties: vec![config.initial_fan_duty; config.devices.fans],
            pump_duties: vec![config.initial_fan_duty.max(60.0); config.devices.pumps],
            gpu_fan_duty: config.initial_fan_duty,
            rng: config.seed,
        }
    }

    /// Small deterministic jitter (xorshift64*), used when `noise > 0`.
    pub fn noise(&mut self, amplitude: f64) -> f64 {
        if amplitude <= 0.0 {
            return 0.0;
        }
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let unit = ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64) / ((1u64 << 53) as f64);
        (unit * 2.0 - 1.0) * amplitude
    }

    /// Average duty across the chassis fans, used to cool the CPU.
    pub fn system_duty(&self) -> f64 {
        if self.fan_duties.is_empty() {
            0.0
        } else {
            self.fan_duties.iter().sum::<f64>() / self.fan_duties.len() as f64
        }
    }

    /// Advance the thermal model by `dt_ms`.
    pub fn advance(&mut self, dt_ms: u64, config: &MockConfig) {
        let dt_s = (dt_ms as f64 / 1000.0).clamp(0.0, 60.0);
        if dt_s <= 0.0 {
            return;
        }
        self.sim_ms = self.sim_ms.saturating_add(dt_ms);
        self.gpu_load = config.gpu_load.load_at(self.sim_ms);
        self.cpu_load = config.cpu_load.load_at(self.sim_ms);

        // First order relaxation towards the equilibrium temperature.
        //
        // The GPU is cooled by its own fan *and* by the chassis fans: a case fan
        // rule must visibly change the GPU temperature, otherwise the whole
        // `GPU temperature -> chassis fan` loop would be untestable.
        let gpu_airflow = (0.5 * self.gpu_fan_duty + 0.5 * self.system_duty()).clamp(0.0, 100.0);
        let gpu_target = config.ambient_c + physics::GPU_LOAD_HEAT_C * self.gpu_load
            - physics::GPU_FAN_COOLING_C * (gpu_airflow / 100.0);
        let alpha_gpu = (dt_s / physics::GPU_TIME_CONSTANT_S).clamp(0.0, 1.0);
        self.gpu_temp_c += (gpu_target - self.gpu_temp_c) * alpha_gpu;

        let cpu_target = config.ambient_c + physics::CPU_LOAD_HEAT_C * self.cpu_load
            - physics::CPU_FAN_COOLING_C * (self.system_duty() / 100.0);
        let alpha_cpu = (dt_s / physics::CPU_TIME_CONSTANT_S).clamp(0.0, 1.0);
        self.cpu_temp_c += (cpu_target - self.cpu_temp_c) * alpha_cpu;

        let ssd_target = config.ambient_c
            + 8.0
            + physics::SSD_LOAD_HEAT_C * self.cpu_load.max(self.gpu_load) * 0.5;
        let alpha_ssd = (dt_s / physics::SSD_TIME_CONSTANT_S).clamp(0.0, 1.0);
        self.ssd_temp_c += (ssd_target - self.ssd_temp_c) * alpha_ssd;

        // Never go below ambient in the simulation.
        self.gpu_temp_c = self.gpu_temp_c.max(config.ambient_c);
        self.cpu_temp_c = self.cpu_temp_c.max(config.ambient_c);
        self.ssd_temp_c = self.ssd_temp_c.max(config.ambient_c);
    }

    /// Tachometer reading for a duty, with a small linear noise.
    pub fn rpm_for(&self, duty: f64, min: f64, max: f64) -> f64 {
        if duty <= 0.0 {
            return 0.0;
        }
        let duty = duty.clamp(0.0, 100.0);
        (min + (max - min) * (duty / 100.0)).round()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_profile_is_clamped() {
        assert_eq!(LoadProfile::Constant { load: 1.5 }.load_at(0), 1.0);
        assert_eq!(LoadProfile::Constant { load: -0.5 }.load_at(0), 0.0);
    }

    #[test]
    fn wave_profile_stays_in_range() {
        let profile = LoadProfile::Wave {
            min: 0.1,
            max: 0.9,
            period_ms: 1_000,
            phase: 0.0,
        };
        for t in (0..3_000).step_by(37) {
            let load = profile.load_at(t);
            assert!((0.1..=0.9).contains(&load), "t={t} load={load}");
        }
    }

    #[test]
    fn steps_profile_repeats() {
        let profile = LoadProfile::Steps {
            steps: vec![(1_000, 0.2), (1_000, 0.8)],
        };
        assert_eq!(profile.load_at(500), 0.2);
        assert_eq!(profile.load_at(1500), 0.8);
        assert_eq!(profile.load_at(2500), 0.2);
        assert_eq!(LoadProfile::Steps { steps: vec![] }.load_at(10), 0.0);
    }

    #[test]
    fn gaming_demo_cycles_between_idle_and_load() {
        let profile = LoadProfile::gaming_demo();
        assert!(profile.load_at(5_000) < 0.2);
        assert!(profile.load_at(20_000) > 0.9);
        assert!(profile.load_at(33_000) < 0.5);
    }

    #[test]
    fn noise_is_deterministic_and_bounded() {
        let config = MockConfig::deterministic();
        let mut a = MockState::new(&config);
        let mut b = MockState::new(&config);
        for _ in 0..100 {
            let x = a.noise(1.0);
            let y = b.noise(1.0);
            assert_eq!(x, y);
            assert!(x.abs() <= 1.0);
        }
        assert_eq!(a.noise(0.0), 0.0);
    }

    #[test]
    fn more_airflow_means_lower_temperature() {
        let config = MockConfig {
            gpu_load: LoadProfile::Constant { load: 1.0 },
            ..MockConfig::deterministic()
        };
        let mut hot = MockState::new(&config);
        hot.gpu_fan_duty = 0.0;
        let mut cool = MockState::new(&config);
        cool.gpu_fan_duty = 100.0;
        for _ in 0..120 {
            hot.advance(1_000, &config);
            cool.advance(1_000, &config);
        }
        assert!(
            hot.gpu_temp_c > cool.gpu_temp_c + 15.0,
            "hot={} cool={}",
            hot.gpu_temp_c,
            cool.gpu_temp_c
        );
        assert!(cool.gpu_temp_c > config.ambient_c);
    }

    #[test]
    fn idle_machine_stays_cool() {
        let config = MockConfig::deterministic();
        let mut state = MockState::new(&config);
        state.gpu_fan_duty = 30.0;
        for _ in 0..300 {
            state.advance(1_000, &config);
        }
        assert!(state.gpu_temp_c < 40.0, "gpu={}", state.gpu_temp_c);
        assert!(state.cpu_temp_c < 45.0, "cpu={}", state.cpu_temp_c);
    }

    #[test]
    fn rpm_mapping() {
        let state = MockState::new(&MockConfig::deterministic());
        assert_eq!(state.rpm_for(0.0, 400.0, 2000.0), 0.0);
        assert_eq!(state.rpm_for(50.0, 400.0, 2000.0), 1200.0);
        assert_eq!(state.rpm_for(100.0, 400.0, 2000.0), 2000.0);
        assert_eq!(state.rpm_for(200.0, 400.0, 2000.0), 2000.0);
    }

    #[test]
    fn faults_are_queryable() {
        let faults = MockFaults::writes_fail();
        assert!(faults.write_blocked(
            &DeviceId::new("fan.mock.0").unwrap(),
            &CapabilityId::new("fan.speed_percent").unwrap()
        ));

        let faults = MockFaults::sensor_disconnected("gpu.mock.0", "temperature.core");
        assert_eq!(
            faults.reading_unavailable(
                &DeviceId::new("gpu.mock.0").unwrap(),
                &CapabilityId::new("temperature.core").unwrap()
            ),
            Some(UnavailableReason::ReadError)
        );
        assert_eq!(
            faults.reading_unavailable(
                &DeviceId::new("gpu.mock.0").unwrap(),
                &CapabilityId::new("fan.rpm").unwrap()
            ),
            None
        );

        let faults = MockFaults::device_unplugged("fan.mock.1");
        assert!(faults.is_unplugged(&DeviceId::new("fan.mock.1").unwrap()));
        assert!(!faults.is_unplugged(&DeviceId::new("fan.mock.0").unwrap()));
    }
}
