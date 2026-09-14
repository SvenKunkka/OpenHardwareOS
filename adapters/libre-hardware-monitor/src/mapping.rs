//! Turning LibreHardwareMonitor's tree into OpenHardwareOS devices.
//!
//! # How the mapping works
//!
//! * Every **hardware node directly below the root** becomes a device: the CPU,
//!   each GPU, each drive, the motherboard. All descendant sensors are attached
//!   to it (minus fans and controls), which is how a `System` temperature living
//!   under `Motherboard / Nuvoton NCT6687D` still lands on the motherboard
//!   device.
//! * **Fan and Control sensors become devices of their own.** A fan is what a
//!   user automates, so `Fan #1` (RPM) is paired with `Fan Control #1` (duty)
//!   into one controllable fan device, wherever in the tree they live.
//! * Capability ids are normalised (`temperature.core`, `fan.speed_percent`,
//!   ...) so a rule written against real hardware looks exactly like a rule
//!   written against the mock. Duplicates get a stable suffix instead of being
//!   dropped.
//!
//! Every LHM sensor id is remembered per capability: that id is what
//! `GET /Sensor?action=Set&id=...` needs, and it is the only vendor specific
//! string the runtime ever stores (in `Device::metadata`, never in logic).

use std::collections::BTreeMap;

use ohm_core::{AdapterId, CapabilityId, DeviceId};
use ohm_device_model::{
    Capability, Device, DeviceState, DeviceType, Reading, Transport, UnavailableReason, Unit,
    Value, caps,
};

use crate::lhm::{LhmNode, LhmSensorKind, device_type_of};

/// Adapter namespace used in device ids.
pub const NAMESPACE: &str = "lhm";

/// One device's capability -> LHM sensor id mapping.
pub type SensorMap = BTreeMap<CapabilityId, String>;

/// The result of mapping a tree: device descriptions plus where to read them.
#[derive(Debug, Clone, Default)]
pub struct LhmMapping {
    pub devices: Vec<Device>,
    /// `device id -> (capability id -> LHM sensor id)`
    pub sensors: BTreeMap<DeviceId, SensorMap>,
}

impl LhmMapping {
    /// LHM sensor id backing a capability, if any.
    pub fn sensor_id(&self, device: &DeviceId, capability: &CapabilityId) -> Option<&str> {
        self.sensors
            .get(device)
            .and_then(|map| map.get(capability))
            .map(String::as_str)
    }

    pub fn device(&self, id: &str) -> Option<&Device> {
        self.devices.iter().find(|device| device.id.as_str() == id)
    }

    /// Number of writable control channels found.
    pub fn control_count(&self) -> usize {
        self.sensors
            .values()
            .flat_map(|map| map.keys())
            .filter(|capability| {
                matches!(
                    capability.as_str(),
                    caps::FAN_SPEED_PERCENT | caps::PUMP_SPEED_PERCENT
                )
            })
            .count()
    }
}

fn adapter() -> AdapterId {
    AdapterId::new_unchecked(NAMESPACE)
}

/// Build the device model from a freshly fetched tree.
pub fn map_tree(tree: &LhmNode) -> LhmMapping {
    let mut mapping = LhmMapping::default();

    // 1. One device per top level hardware node.
    let mut index = 0usize;
    for hardware in tree.hardware_children() {
        let device_type = device_type_of(hardware.effective_hardware_type());
        if device_type == DeviceType::Unknown && hardware.flatten().len() == 1 {
            continue;
        }
        let id = DeviceId::compose(device_type.as_str(), NAMESPACE, index);
        index += 1;

        let mut sensors = SensorMap::new();
        let mut capabilities: Vec<Capability> = Vec::new();
        for sensor in descendant_sensors(hardware) {
            let Some(kind) = sensor.sensor_type.as_deref().map(LhmSensorKind::from_type) else {
                continue;
            };
            if !kind.is_readable() || matches!(kind, LhmSensorKind::Fan | LhmSensorKind::Control) {
                continue;
            }
            let capability_id = capability_id_for(device_type, sensor, kind);
            let capability_id = uniquify(&capabilities, capability_id);
            capabilities.push(build_capability(&capability_id, sensor, kind));
            sensors.insert(
                CapabilityId::new_unchecked(capability_id),
                sensor.id.clone(),
            );
        }

        if capabilities.is_empty() {
            continue;
        }

        let device = Device::new(
            id.clone(),
            hardware.text.clone(),
            device_type,
            Transport::Web,
            adapter(),
        )
        .with_vendor("LibreHardwareMonitor")
        .with_capabilities(capabilities)
        .with_metadata("lhm_id", hardware.id.clone())
        .with_metadata(
            "lhm_hardware_type",
            hardware.effective_hardware_type().to_string(),
        )
        .with_metadata("source", "LibreHardwareMonitor web server");

        if device.validate().is_ok() {
            mapping.devices.push(device);
            mapping.sensors.insert(id, sensors);
        }
    }

    // 2. Fan and control channels, wherever they live in the tree.
    let mut fan_index = 0usize;
    for hardware in tree.flatten() {
        let fans = fan_channels(hardware);
        if fans.is_empty() {
            continue;
        }
        for channel in fans {
            let Some((device, sensors)) = build_fan_device(hardware, &channel, fan_index) else {
                continue;
            };
            fan_index += 1;
            if device.validate().is_ok() {
                mapping.sensors.insert(device.id.clone(), sensors);
                mapping.devices.push(device);
            }
        }
    }

    mapping
}

/// Sensors below a node, at any depth.
fn descendant_sensors(node: &LhmNode) -> Vec<&LhmNode> {
    node.flatten()
        .into_iter()
        .filter(|n| n.is_sensor())
        .collect()
}

/// One fan channel: a tachometer reading and/or its control.
#[derive(Debug, Clone, PartialEq)]
pub struct FanChannel {
    /// Pairing key, taken from the sensor names (`Fan #1` <-> `Fan Control #1`).
    pub index: Option<usize>,
    /// Display label exactly as LibreHardwareMonitor spells it, e.g. `Fan #1`.
    pub label: String,
    pub rpm: Option<LhmNode>,
    pub control: Option<LhmNode>,
    pub is_pump: bool,
}

/// Pair `Fan #N` with `Fan Control #N` inside one hardware node.
fn fan_channels(hardware: &LhmNode) -> Vec<FanChannel> {
    let mut rpm: BTreeMap<Option<usize>, LhmNode> = BTreeMap::new();
    let mut controls: BTreeMap<Option<usize>, LhmNode> = BTreeMap::new();
    let mut pumps: Vec<Option<usize>> = Vec::new();

    for sensor in hardware.own_sensors() {
        let Some(kind) = sensor.sensor_type.as_deref().map(LhmSensorKind::from_type) else {
            continue;
        };
        let key = sensor.name_index().or_else(|| sensor.id_index());
        let lower = sensor.text.to_ascii_lowercase();
        if lower.contains("pump") {
            pumps.push(key);
        }
        match kind {
            LhmSensorKind::Fan => {
                rpm.insert(key, sensor.clone());
            }
            LhmSensorKind::Control => {
                controls.insert(key, sensor.clone());
            }
            _ => {}
        }
    }

    let mut keys: Vec<Option<usize>> = rpm.keys().chain(controls.keys()).copied().collect();
    keys.sort();
    keys.dedup();

    keys.into_iter()
        .map(|key| {
            let is_pump = pumps.contains(&key);
            let label = rpm
                .get(&key)
                .or_else(|| controls.get(&key))
                .map(|node| node.text.clone())
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| match key {
                    Some(index) => format!("#{}", index + 1),
                    None => "Fan".to_string(),
                });
            FanChannel {
                index: key,
                label,
                rpm: rpm.get(&key).cloned(),
                control: controls.get(&key).cloned(),
                is_pump,
            }
        })
        .collect()
}

/// Build the device for one fan channel.
fn build_fan_device(
    hardware: &LhmNode,
    channel: &FanChannel,
    fan_index: usize,
) -> Option<(Device, SensorMap)> {
    let mut sensors = SensorMap::new();
    let mut capabilities = Vec::new();

    // Use LibreHardwareMonitor's own spelling (`Fan #1`, `GPU Fan`, `Pump #2`)
    // so the UI matches what the user sees in LHM.
    let channel_label = channel.label.clone();
    let device_type = if channel.is_pump {
        DeviceType::Pump
    } else {
        DeviceType::Fan
    };

    let mut name = channel_label.clone();
    if name.is_empty() || name.eq_ignore_ascii_case("fan") || name.eq_ignore_ascii_case("control") {
        name = if channel.is_pump { "Pump" } else { "Fan" }.to_string();
    }
    if !hardware.text.is_empty() && hardware.text != name {
        name.push_str(" — ");
        name.push_str(&hardware.text);
    }

    if channel.rpm.is_some() {
        let (id, _) = if channel.is_pump {
            (caps::PUMP_RPM, Unit::Rpm)
        } else {
            (caps::FAN_RPM, Unit::Rpm)
        };
        capabilities.push(Capability::sensor(id, "Fan RPM", Unit::Rpm));
        if let Some(node) = &channel.rpm {
            sensors.insert(CapabilityId::new_unchecked(id), node.id.clone());
        }
    }

    if let Some(control) = &channel.control {
        let (id, label) = if channel.is_pump {
            (caps::PUMP_SPEED_PERCENT, "Pump Speed")
        } else {
            (caps::FAN_SPEED_PERCENT, "Fan Speed")
        };
        let mut capability = Capability::actuator(id, label, Unit::Percent, 0.0, 100.0)
            .with_description("LibreHardwareMonitor control channel (requires Administrator)");
        if channel.is_pump {
            capability = capability.safety_critical();
        }
        capabilities.push(capability);
        sensors.insert(CapabilityId::new_unchecked(id), control.id.clone());
    }

    if capabilities.is_empty() {
        return None;
    }

    let device = Device::new(
        DeviceId::compose(device_type.as_str(), NAMESPACE, fan_index),
        name,
        device_type,
        Transport::Web,
        adapter(),
    )
    .with_vendor("LibreHardwareMonitor")
    .with_capabilities(capabilities)
    .with_metadata("lhm_id", hardware.id.clone())
    .with_metadata("lhm_channel", channel_label)
    .with_metadata("source", "LibreHardwareMonitor web server");

    Some((device, sensors))
}

/// Normalise an LHM sensor into a capability id.
fn capability_id_for(device_type: DeviceType, sensor: &LhmNode, kind: LhmSensorKind) -> String {
    let name = sensor.text.to_ascii_lowercase();
    let slug = slugify(&sensor.text);
    match kind {
        LhmSensorKind::Temperature => {
            if name.contains("hot") && name.contains("spot") {
                caps::TEMPERATURE_HOTSPOT.to_string()
            } else if name.contains("package")
                || name.contains("tctl")
                || name.contains("tdie")
                || name.contains("cpu")
                || (device_type == DeviceType::Gpu && name.contains("core"))
                || (device_type == DeviceType::Storage)
            {
                caps::TEMPERATURE_CORE.to_string()
            } else if name.contains("motherboard")
                || name.contains("system")
                || name.contains("chipset")
            {
                caps::TEMPERATURE_SYSTEM.to_string()
            } else {
                format!("temperature.{slug}")
            }
        }
        LhmSensorKind::Load => {
            if name.contains("cpu") || (device_type == DeviceType::Cpu && name.contains("total")) {
                caps::CPU_LOAD.to_string()
            } else if name.contains("gpu")
                || (device_type == DeviceType::Gpu && name.contains("core"))
            {
                caps::GPU_LOAD.to_string()
            } else {
                format!("load.{slug}")
            }
        }
        LhmSensorKind::Power => {
            if device_type == DeviceType::Gpu {
                caps::POWER_GPU.to_string()
            } else {
                caps::POWER_TOTAL.to_string()
            }
        }
        LhmSensorKind::Clock => caps::CLOCK_MHZ.to_string(),
        LhmSensorKind::Voltage => format!("voltage.{slug}"),
        LhmSensorKind::Current => format!("current.{slug}"),
        LhmSensorKind::Data => {
            if name.contains("memory") && name.contains("used") {
                caps::MEMORY_USED.to_string()
            } else {
                format!("data.{slug}")
            }
        }
        LhmSensorKind::Fan | LhmSensorKind::Control => format!("fan.{slug}"),
        LhmSensorKind::Other => format!("sensor.{slug}"),
    }
}

/// Make a capability id unique inside a device by appending `_2`, `_3`, ...
fn uniquify(existing: &[Capability], candidate: String) -> String {
    if !existing.iter().any(|c| c.id.as_str() == candidate) {
        return candidate;
    }
    for suffix in 2..100 {
        let attempt = format!("{candidate}_{suffix}");
        if !existing.iter().any(|c| c.id.as_str() == attempt) {
            return attempt;
        }
    }
    candidate
}

fn build_capability(id: &str, sensor: &LhmNode, kind: LhmSensorKind) -> Capability {
    let unit = if sensor.unit == Unit::None {
        kind.default_unit()
    } else {
        sensor.unit
    };
    let mut capability = Capability::sensor(id, sensor.text.clone(), unit);
    if let Some(description) = &sensor.raw_value {
        capability.description = Some(format!("LibreHardwareMonitor: {description}"));
    }
    capability
}

/// Lowercase, id safe slug of a display name.
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_separator = true;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_separator = false;
        } else if !last_separator {
            out.push('_');
            last_separator = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("sensor");
    }
    if out.len() > 40 {
        out.truncate(40);
    }
    out
}

/// Build the live state of one device from a freshly fetched tree.
pub fn state_from_tree(
    tree: &LhmNode,
    mapping: &LhmMapping,
    device: &Device,
    at_ms: i64,
) -> DeviceState {
    let mut state = DeviceState::new(device.id.clone(), at_ms);
    let Some(sensors) = mapping.sensors.get(&device.id) else {
        return state.offline(
            UnavailableReason::NotPresent,
            "this device is not part of the current LibreHardwareMonitor tree",
        );
    };

    let mut missing = 0usize;
    for (capability_id, sensor_id) in sensors {
        let Some(node) = tree.find_by_id(sensor_id) else {
            missing += 1;
            state.set(Reading::unavailable(
                capability_id.clone(),
                UnavailableReason::NotPresent,
                Some(format!("`{sensor_id}` disappeared from the LHM tree")),
            ));
            continue;
        };
        match node.value {
            Some(value) => {
                let declared = device.capability(capability_id);
                let value = round_for(declared.map(|c| c.unit).unwrap_or(node.unit), value);
                state.set(Reading::ok(capability_id.clone(), value));
            }
            None => {
                state.set(Reading::unavailable(
                    capability_id.clone(),
                    UnavailableReason::HardwareLimitation,
                    Some(format!(
                        "LibreHardwareMonitor reports `{}` for this sensor",
                        node.raw_value.clone().unwrap_or_else(|| "N/A".into())
                    )),
                ));
            }
        }
    }

    if missing == sensors.len() && !sensors.is_empty() {
        state = state.offline(
            UnavailableReason::NotPresent,
            "every sensor of this device vanished from the LHM tree",
        );
    }
    state
}

/// Round a reading to a sensible precision for its unit.
fn round_for(unit: Unit, value: f64) -> Value {
    match unit {
        Unit::Byte | Unit::Count | Unit::Rpm | Unit::Hertz | Unit::Megahertz => {
            Value::Integer(value.round() as i64)
        }
        Unit::Celsius
        | Unit::Fahrenheit
        | Unit::Percent
        | Unit::Pwm
        | Unit::Watt
        | Unit::Milliwatt => Value::Number((value * 10.0).round() / 10.0),
        Unit::Volt | Unit::Ampere => Value::Number((value * 1000.0).round() / 1000.0),
        _ => Value::Number(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lhm::parse_tree;

    fn mapping() -> (LhmNode, LhmMapping) {
        let tree = parse_tree(include_str!("../tests/fixtures/data.json")).unwrap();
        let mapping = map_tree(&tree);
        (tree, mapping)
    }

    #[test]
    fn hardware_nodes_become_devices() {
        let (_, mapping) = mapping();
        let ids: Vec<&str> = mapping.devices.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&"cpu.lhm.0"), "{ids:?}");
        assert!(ids.contains(&"gpu.lhm.1"), "{ids:?}");
        assert!(ids.contains(&"storage.lhm.2"), "{ids:?}");
        assert!(ids.contains(&"motherboard.lhm.3"), "{ids:?}");
        for device in &mapping.devices {
            device.validate().unwrap();
        }
    }

    #[test]
    fn cpu_capabilities_are_normalised() {
        let (_, mapping) = mapping();
        let cpu = mapping.device("cpu.lhm.0").unwrap();
        assert_eq!(cpu.device_type, DeviceType::Cpu);
        assert_eq!(cpu.name, "AMD Ryzen 9 9800X3D");
        assert!(cpu.supports(caps::TEMPERATURE_CORE));
        assert!(cpu.supports(caps::CPU_LOAD));
        assert!(cpu.supports(caps::POWER_TOTAL));
        assert!(cpu.supports(caps::CLOCK_MHZ));
        // The second temperature arrived as `Tctl/Tdie`: it must be kept, not
        // silently dropped, so it gets a suffixed id.
        assert_eq!(
            cpu.capabilities
                .iter()
                .filter(|c| c.unit == Unit::Celsius)
                .count(),
            2
        );
        assert!(!cpu.is_controllable());
    }

    #[test]
    fn gpu_gets_hotspot_and_gpu_specific_ids() {
        let (_, mapping) = mapping();
        let gpu = mapping.device("gpu.lhm.1").unwrap();
        assert_eq!(gpu.device_type, DeviceType::Gpu);
        assert!(gpu.supports(caps::TEMPERATURE_CORE));
        assert!(gpu.supports(caps::TEMPERATURE_HOTSPOT));
        assert!(gpu.supports(caps::GPU_LOAD));
        assert!(gpu.supports(caps::POWER_GPU));
        assert!(gpu.supports(caps::MEMORY_USED));
        assert!(gpu.supports(caps::CLOCK_MHZ));
    }

    #[test]
    fn motherboard_includes_nested_superio_sensors() {
        let (_, mapping) = mapping();
        let board = mapping.device("motherboard.lhm.3").unwrap();
        assert!(board.supports(caps::TEMPERATURE_SYSTEM));
        assert!(board.supports("voltage.vin0"), "{:?}", board.capabilities);
        assert_eq!(
            board.metadata.get("lhm_hardware_type").unwrap(),
            "Motherboard"
        );
    }

    #[test]
    fn fan_channels_are_paired_into_controllable_devices() {
        let (_, mapping) = mapping();
        let fans: Vec<&Device> = mapping
            .devices
            .iter()
            .filter(|d| d.device_type == DeviceType::Fan)
            .collect();
        // Three chassis channels with RPM plus two extra controls, and the GPU
        // fan: the exact count matters less than the pairing being correct.
        assert!(fans.len() >= 4, "expected fan devices, got {}", fans.len());

        let chassis = fans
            .iter()
            .find(|d| d.name.contains("Nuvoton") && d.name.contains("#1"))
            .expect("chassis channel #1 present");
        assert!(chassis.supports(caps::FAN_RPM));
        assert!(chassis.supports(caps::FAN_SPEED_PERCENT));
        assert!(
            chassis.is_controllable(),
            "a paired channel must be controllable"
        );
        assert_eq!(
            mapping.sensor_id(
                &chassis.id,
                &CapabilityId::new_unchecked(caps::FAN_SPEED_PERCENT)
            ),
            Some("/lpc/nct6687d/control/0")
        );
        assert_eq!(
            mapping.sensor_id(&chassis.id, &CapabilityId::new_unchecked(caps::FAN_RPM)),
            Some("/lpc/nct6687d/fan/0")
        );

        // The GPU fan is paired too, even though its tachometer reads N/A.
        let gpu_fan = fans
            .iter()
            .find(|d| d.name.contains("RTX"))
            .expect("GPU fan present");
        assert!(gpu_fan.supports(caps::FAN_SPEED_PERCENT));
        assert_eq!(
            mapping.sensor_id(
                &gpu_fan.id,
                &CapabilityId::new_unchecked(caps::FAN_SPEED_PERCENT)
            ),
            Some("/gpu/0/control/0")
        );

        // Every control channel in the fixture is reachable: 1 GPU + 5 chassis.
        assert_eq!(mapping.control_count(), 6);
    }

    #[test]
    fn fan_device_names_identify_their_hardware() {
        let (_, mapping) = mapping();
        for device in mapping
            .devices
            .iter()
            .filter(|d| d.device_type == DeviceType::Fan)
        {
            assert!(
                device.name.contains("Fan"),
                "a fan device must say so: {}",
                device.name
            );
            assert!(!device.metadata.get("lhm_channel").unwrap().is_empty());
        }
        // Two different hardware parents, so two differently named channels.
        let names: Vec<&str> = mapping
            .devices
            .iter()
            .filter(|d| d.device_type == DeviceType::Fan)
            .map(|d| d.name.as_str())
            .collect();
        assert!(names.iter().any(|n| n.contains("Nuvoton")));
        assert!(names.iter().any(|n| n.contains("RTX")));
    }

    #[test]
    fn readings_come_from_the_tree() {
        let (tree, mapping) = mapping();
        let gpu = mapping.device("gpu.lhm.1").unwrap();
        let state = state_from_tree(&tree, &mapping, gpu, 1_000);
        assert_eq!(state.number(caps::TEMPERATURE_CORE), Some(76.0));
        assert_eq!(state.number(caps::TEMPERATURE_HOTSPOT), Some(88.5));
        assert_eq!(state.number(caps::GPU_LOAD), Some(91.0));
        assert_eq!(state.number(caps::POWER_GPU), Some(405.2));
        assert_eq!(state.number(caps::CLOCK_MHZ), Some(2610.0));
        assert_eq!(state.number(caps::MEMORY_USED), Some(14_500_000_000.0));
        assert!(!state.has_failure());

        let cpu = mapping.device("cpu.lhm.0").unwrap();
        let cpu_state = state_from_tree(&tree, &mapping, cpu, 1_000);
        assert_eq!(cpu_state.number(caps::TEMPERATURE_CORE), Some(68.2));
        assert_eq!(cpu_state.number(caps::CPU_LOAD), Some(32.0));
        assert_eq!(cpu_state.number(caps::POWER_TOTAL), Some(94.2));
    }

    #[test]
    fn a_fan_reporting_na_is_unavailable_not_zero() {
        let (tree, mapping) = mapping();
        let fan = mapping
            .devices
            .iter()
            .find(|d| {
                d.device_type == DeviceType::Fan
                    && mapping.sensor_id(&d.id, &CapabilityId::new_unchecked(caps::FAN_RPM))
                        == Some("/lpc/nct6687d/fan/3")
            })
            .expect("fan #4 present");
        let state = state_from_tree(&tree, &mapping, fan, 1_000);
        let reading = state.get(caps::FAN_RPM).unwrap();
        assert!(!reading.is_ok());
        assert_eq!(
            reading.reason(),
            Some(UnavailableReason::HardwareLimitation)
        );
        assert!(reading.value().is_none());
    }

    #[test]
    fn a_vanished_sensor_is_reported() {
        let (tree, mapping) = mapping();
        let mut pruned = tree.clone();
        pruned.children.retain(|node| node.id != "/gpu/0");
        let gpu = mapping.device("gpu.lhm.1").unwrap();
        let state = state_from_tree(&pruned, &mapping, gpu, 1_000);
        assert!(!state.online);
        assert!(
            state
                .readings
                .iter()
                .all(|r| r.reason() == Some(UnavailableReason::NotPresent))
        );
    }

    #[test]
    fn duplicate_capability_ids_get_a_suffix() {
        let existing = vec![Capability::sensor("temperature.core", "A", Unit::Celsius)];
        assert_eq!(
            uniquify(&existing, "temperature.other".into()),
            "temperature.other"
        );
        assert_eq!(
            uniquify(&existing, "temperature.core".into()),
            "temperature.core_2"
        );
    }

    #[test]
    fn slugs_are_safe() {
        assert_eq!(slugify("VIN0"), "vin0");
        assert_eq!(slugify("Core (Tctl/Tdie)"), "core_tctl_tdie");
        assert_eq!(slugify("!!!"), "sensor");
        assert!(slugify(&"x".repeat(80)).len() <= 40);
    }

    #[test]
    fn rounding_follows_the_unit() {
        assert_eq!(round_for(Unit::Rpm, 1120.4), Value::Integer(1120));
        assert_eq!(round_for(Unit::Celsius, 68.24), Value::Number(68.2));
        assert_eq!(round_for(Unit::Volt, 12.05123), Value::Number(12.051));
    }
}
