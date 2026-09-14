//! Device descriptions for the simulated machine.
//!
//! The shapes deliberately mirror real hardware (an RTX class GPU, an X3D class
//! CPU, an NVMe SSD, chassis fans, an AIO pump) so automation rules written
//! against the mock keep working when a real adapter shows up.

use ohm_core::{AdapterId, CapabilityId, DeviceId, ids::capability as caps};
use ohm_device_model::{Capability, Device, DeviceType, Transport, Unit};

use crate::model::MockConfig;

/// Adapter id, also the device id namespace.
pub const ADAPTER_ID: &str = "mock";
/// Display name shown in Settings -> Providers.
pub const ADAPTER_NAME: &str = "Mock Hardware";
/// Vendor string put on every simulated device.
pub const VENDOR: &str = "OpenHardwareOS";

/// `cpu.mock.0`
pub fn cpu_id() -> DeviceId {
    DeviceId::compose("cpu", ADAPTER_ID, 0)
}

/// `gpu.mock.0`
pub fn gpu_id() -> DeviceId {
    DeviceId::compose("gpu", ADAPTER_ID, 0)
}

/// `ssd.mock.0`
pub fn ssd_id() -> DeviceId {
    DeviceId::compose("ssd", ADAPTER_ID, 0)
}

/// `fan.mock.{index}`
pub fn fan_id(index: usize) -> DeviceId {
    DeviceId::compose("fan", ADAPTER_ID, index)
}

/// `pump.mock.{index}`
pub fn pump_id(index: usize) -> DeviceId {
    DeviceId::compose("pump", ADAPTER_ID, index)
}

/// `temperature.mock.{index}`
pub fn temperature_sensor_id(index: usize) -> DeviceId {
    DeviceId::compose("temperature", ADAPTER_ID, index)
}

fn adapter() -> AdapterId {
    AdapterId::new_unchecked(ADAPTER_ID)
}

/// Instance index encoded in a device id, e.g. `fan.mock.2` -> `Some(2)`.
pub fn instance_index(id: &DeviceId) -> Option<usize> {
    id.as_str().rsplit('.').next()?.parse().ok()
}

/// The simulated CPU.
pub fn cpu_device() -> Device {
    Device::new(
        cpu_id(),
        "Mock Ryzen 9 9800X3D",
        DeviceType::Cpu,
        Transport::Mock,
        adapter(),
    )
    .with_vendor(VENDOR)
    .with_model("MOCK-CPU-8C16T")
    .with_tag("simulated")
    .with_capabilities([
        Capability::sensor(caps::TEMPERATURE_CORE, "CPU Temperature", Unit::Celsius)
            .with_description("Package temperature of the simulated CPU die"),
        Capability::sensor(caps::CPU_LOAD, "CPU Load", Unit::Percent),
        Capability::sensor(caps::POWER_TOTAL, "Package Power", Unit::Watt),
        Capability::sensor(caps::CLOCK_MHZ, "Core Clock", Unit::Megahertz),
    ])
}

/// The simulated GPU.
///
/// Its fan channel is only advertised when `config.gpu_fan_control` is on, so a
/// machine whose GPU fan cannot be driven looks exactly like one here.
pub fn gpu_device(config: &MockConfig) -> Device {
    let mut device = Device::new(
        gpu_id(),
        "Mock GeForce RTX 5090",
        DeviceType::Gpu,
        Transport::Mock,
        adapter(),
    )
    .with_vendor(VENDOR)
    .with_model("MOCK-GPU-32GB")
    .with_tag("simulated")
    .with_capabilities([
        Capability::sensor(caps::TEMPERATURE_CORE, "Core Temperature", Unit::Celsius),
        Capability::sensor(
            caps::TEMPERATURE_HOTSPOT,
            "Hotspot Temperature",
            Unit::Celsius,
        ),
        Capability::sensor(caps::GPU_LOAD, "GPU Load", Unit::Percent),
        Capability::sensor(caps::POWER_GPU, "Board Power", Unit::Watt),
        Capability::sensor(caps::FAN_RPM, "GPU Fan RPM", Unit::Rpm),
    ]);

    if config.gpu_fan_control {
        device = device.with_capability(
            Capability::actuator(
                caps::FAN_SPEED_PERCENT,
                "GPU Fan Speed",
                Unit::Percent,
                0.0,
                100.0,
            )
            .with_description("Simulated GPU fan control, 0-100 %"),
        );
    } else {
        device = device.with_capability(
            Capability::sensor(caps::FAN_SPEED_PERCENT, "GPU Fan Speed", Unit::Percent)
                .with_description("Read-only: this GPU does not allow third party fan control"),
        );
    }

    device
}

/// The simulated NVMe SSD.
pub fn ssd_device() -> Device {
    Device::new(
        ssd_id(),
        "Mock Samsung 990 PRO 2TB",
        DeviceType::Storage,
        Transport::Mock,
        adapter(),
    )
    .with_vendor(VENDOR)
    .with_tag("simulated")
    .with_capabilities([
        Capability::sensor(caps::TEMPERATURE_CORE, "Drive Temperature", Unit::Celsius),
        Capability::sensor(caps::DISK_FREE, "Free Space", Unit::Byte),
    ])
}

/// A simulated chassis fan: readable RPM plus writable duty.
pub fn fan_device(index: usize) -> Device {
    Device::new(
        fan_id(index),
        format!("Mock Chassis Fan {}", index + 1),
        DeviceType::Fan,
        Transport::Mock,
        adapter(),
    )
    .with_vendor(VENDOR)
    .with_tag("simulated")
    .with_capabilities([
        Capability::sensor(caps::FAN_RPM, "Fan RPM", Unit::Rpm),
        Capability::actuator(
            caps::FAN_SPEED_PERCENT,
            "Fan Speed",
            Unit::Percent,
            0.0,
            100.0,
        )
        .with_description("Simulated PWM duty, 0-100 %"),
    ])
}

/// A simulated AIO pump. The duty floor is part of the capability, and the
/// safety policy refuses to drive it below it.
pub fn pump_device(index: usize) -> Device {
    Device::new(
        pump_id(index),
        format!("Mock AIO Pump {}", index + 1),
        DeviceType::Pump,
        Transport::Mock,
        adapter(),
    )
    .with_vendor(VENDOR)
    .with_tag("simulated")
    .with_capabilities([
        Capability::sensor(caps::PUMP_RPM, "Pump RPM", Unit::Rpm),
        Capability::actuator(
            caps::PUMP_SPEED_PERCENT,
            "Pump Speed",
            Unit::Percent,
            60.0,
            100.0,
        )
        .with_description("Simulated pump duty, 60-100 % (never stopped)")
        .safety_critical(),
    ])
}

/// A simulated standalone temperature probe.
pub fn temperature_sensor_device(index: usize) -> Device {
    Device::new(
        temperature_sensor_id(index),
        format!("Mock Temperature Probe {}", index + 1),
        DeviceType::TemperatureSensor,
        Transport::Mock,
        adapter(),
    )
    .with_vendor(VENDOR)
    .with_tag("simulated")
    .with_capabilities([Capability::sensor(
        caps::TEMPERATURE_SYSTEM,
        "System Temperature",
        Unit::Celsius,
    )])
}

/// Every device the configuration asks for.
pub fn build_devices(config: &MockConfig) -> Vec<Device> {
    let mut devices = Vec::new();
    if config.devices.cpu {
        devices.push(cpu_device());
    }
    if config.devices.gpu {
        devices.push(gpu_device(config));
    }
    if config.devices.ssd {
        devices.push(ssd_device());
    }
    for index in 0..config.devices.fans {
        devices.push(fan_device(index));
    }
    for index in 0..config.devices.pumps {
        devices.push(pump_device(index));
    }
    for index in 0..config.devices.temperature_sensors {
        devices.push(temperature_sensor_device(index));
    }
    devices
}

/// Is this capability a duty control (writable fan/pump speed)?
pub fn is_duty(capability: &CapabilityId) -> bool {
    matches!(
        capability.as_str(),
        caps::FAN_SPEED_PERCENT | caps::PUMP_SPEED_PERCENT
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable() {
        assert_eq!(cpu_id().as_str(), "cpu.mock.0");
        assert_eq!(gpu_id().as_str(), "gpu.mock.0");
        assert_eq!(ssd_id().as_str(), "ssd.mock.0");
        assert_eq!(fan_id(3).as_str(), "fan.mock.3");
        assert_eq!(instance_index(&fan_id(12)), Some(12));
        assert_eq!(instance_index(&DeviceId::new("fan.mock.x").unwrap()), None);
    }

    #[test]
    fn devices_are_valid_and_capability_complete() {
        for device in build_devices(&MockConfig::default()) {
            device.validate().unwrap();
            assert!(!device.capabilities.is_empty());
            assert_eq!(device.adapter.as_str(), ADAPTER_ID);
        }
    }

    #[test]
    fn gpu_is_controllable_and_cpu_is_not() {
        let gpu = gpu_device(&MockConfig::default());
        assert!(gpu.is_controllable());
        assert!(gpu.supports(caps::FAN_SPEED_PERCENT));
        assert!(gpu.supports(caps::TEMPERATURE_HOTSPOT));
        let cpu = cpu_device();
        assert!(!cpu.is_controllable());
    }

    #[test]
    fn a_locked_gpu_advertises_no_writable_fan_channel() {
        // A machine whose GPU fan cannot be driven must not pretend otherwise:
        // no actuator is advertised, so no rule can target it.
        let locked = MockConfig {
            gpu_fan_control: false,
            ..MockConfig::default()
        };
        let gpu = gpu_device(&locked);
        let fan = gpu
            .capability_str(caps::FAN_SPEED_PERCENT)
            .expect("still readable");
        assert!(!fan.writable, "a locked GPU fan must not be writable");
        assert!(!gpu.is_controllable());
        gpu.validate().unwrap();

        // The default machine keeps its controllable fan.
        let unlocked = gpu_device(&MockConfig::default());
        assert!(unlocked.is_controllable());
        assert!(
            unlocked
                .capability_str(caps::FAN_SPEED_PERCENT)
                .unwrap()
                .writable
        );
    }

    #[test]
    fn pump_floor_is_declared() {
        let pump = pump_device(0);
        let control = pump.capability_str(caps::PUMP_SPEED_PERCENT).unwrap();
        assert_eq!(control.min, Some(60.0));
        assert!(control.safety_critical);
        assert!(is_duty(&control.id));
        assert!(is_duty(
            &CapabilityId::new(caps::FAN_SPEED_PERCENT).unwrap()
        ));
        assert!(!is_duty(&CapabilityId::new(caps::FAN_RPM).unwrap()));
    }

    #[test]
    fn device_set_follows_config() {
        let config = MockConfig {
            devices: crate::model::MockDevices {
                cpu: false,
                gpu: true,
                ssd: false,
                fans: 3,
                pumps: 1,
                temperature_sensors: 2,
            },
            ..MockConfig::default()
        };
        let devices = build_devices(&config);
        assert_eq!(devices.len(), 7);
        assert!(devices.iter().all(|d| d.device_type != DeviceType::Cpu));
        assert_eq!(
            devices
                .iter()
                .filter(|d| d.device_type == DeviceType::Fan)
                .count(),
            3
        );
    }
}
