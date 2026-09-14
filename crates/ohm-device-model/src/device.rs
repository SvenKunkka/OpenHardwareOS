//! The vendor neutral device description produced by adapters during discovery.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use ohm_core::{AdapterId, CapabilityId, DeviceId, OhmError, Result};
use serde::{Deserialize, Serialize};

use crate::capability::{Capability, CapabilityKind};

/// What kind of hardware a device is.
///
/// The list already covers the roadmap beyond cooling (RGB, displays, input,
/// power) so that new adapters do not need a model change to register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    Cpu,
    Gpu,
    Motherboard,
    Memory,
    Storage,
    Fan,
    Pump,
    TemperatureSensor,
    PowerSupply,
    Keyboard,
    Mouse,
    Rgb,
    Display,
    Hub,
    Unknown,
}

impl DeviceType {
    pub const ALL: [DeviceType; 15] = [
        DeviceType::Cpu,
        DeviceType::Gpu,
        DeviceType::Motherboard,
        DeviceType::Memory,
        DeviceType::Storage,
        DeviceType::Fan,
        DeviceType::Pump,
        DeviceType::TemperatureSensor,
        DeviceType::PowerSupply,
        DeviceType::Keyboard,
        DeviceType::Mouse,
        DeviceType::Rgb,
        DeviceType::Display,
        DeviceType::Hub,
        DeviceType::Unknown,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
            Self::Motherboard => "motherboard",
            Self::Memory => "memory",
            Self::Storage => "storage",
            Self::Fan => "fan",
            Self::Pump => "pump",
            Self::TemperatureSensor => "temperature_sensor",
            Self::PowerSupply => "power_supply",
            Self::Keyboard => "keyboard",
            Self::Mouse => "mouse",
            Self::Rgb => "rgb",
            Self::Display => "display",
            Self::Hub => "hub",
            Self::Unknown => "unknown",
        }
    }

    /// Grouping used by the Overview page.
    pub fn is_thermal_source(&self) -> bool {
        matches!(
            self,
            Self::Cpu | Self::Gpu | Self::Storage | Self::TemperatureSensor | Self::Motherboard
        )
    }

    /// `true` when the device is something an automation rule can drive.
    pub fn is_actuatable(&self) -> bool {
        matches!(self, Self::Fan | Self::Pump | Self::Rgb | Self::Display)
    }

    /// `true` when the device needs a minimum safe duty.
    pub fn requires_safe_floor(&self) -> bool {
        matches!(self, Self::Fan | Self::Pump)
    }
}

impl fmt::Display for DeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DeviceType {
    type Err = OhmError;

    fn from_str(s: &str) -> Result<Self> {
        let normalised = s.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        Ok(match normalised.as_str() {
            "cpu" => Self::Cpu,
            "gpu" => Self::Gpu,
            "motherboard" | "mainboard" => Self::Motherboard,
            "memory" | "ram" => Self::Memory,
            "storage" | "ssd" | "nvme" | "disk" | "hdd" => Self::Storage,
            "fan" => Self::Fan,
            "pump" | "aio" => Self::Pump,
            "temperature_sensor" | "temperature" | "temp" | "sensor" => Self::TemperatureSensor,
            "power_supply" | "psu" => Self::PowerSupply,
            "keyboard" => Self::Keyboard,
            "mouse" => Self::Mouse,
            "rgb" | "lighting" => Self::Rgb,
            "display" | "screen" => Self::Display,
            "hub" | "usb_hub" => Self::Hub,
            other => return Err(OhmError::Config(format!("unknown device type `{other}`"))),
        })
    }
}

/// How the runtime reaches the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// In-process OS APIs (WMI, sysfs, sysinfo).
    System,
    /// NVIDIA NVML.
    Nvidia,
    /// AMD ADL / ROCm SMI.
    Amd,
    /// USB HID, the transport the future OpenFan uses.
    UsbHid,
    /// USB CDC / virtual serial, used by the future OpenHub.
    UsbCdc,
    Serial,
    /// HTTP JSON, e.g. LibreHardwareMonitor's built-in web server.
    Web,
    /// Out-of-process bridge (a .NET helper hosting LibreHardwareMonitorLib).
    Bridge,
    /// Simulated hardware.
    Mock,
    Unknown,
}

impl Transport {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Nvidia => "nvidia",
            Self::Amd => "amd",
            Self::UsbHid => "usb_hid",
            Self::UsbCdc => "usb_cdc",
            Self::Serial => "serial",
            Self::Web => "web",
            Self::Bridge => "bridge",
            Self::Mock => "mock",
            Self::Unknown => "unknown",
        }
    }

    /// `true` when writing is expected to require elevated privileges.
    pub fn needs_elevation_to_write(&self) -> bool {
        matches!(
            self,
            Self::System | Self::Nvidia | Self::Amd | Self::Bridge | Self::Web
        )
    }
}

impl fmt::Display for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Transport {
    type Err = OhmError;

    fn from_str(s: &str) -> Result<Self> {
        let normalised = s.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        Ok(match normalised.as_str() {
            "system" | "os" => Self::System,
            "nvidia" | "nvml" => Self::Nvidia,
            "amd" | "adl" => Self::Amd,
            "usb_hid" | "hid" => Self::UsbHid,
            "usb_cdc" | "cdc" => Self::UsbCdc,
            "serial" | "uart" => Self::Serial,
            "web" | "http" => Self::Web,
            "bridge" | "sidecar" => Self::Bridge,
            "mock" | "simulated" => Self::Mock,
            other => return Err(OhmError::Config(format!("unknown transport `{other}`"))),
        })
    }
}

/// A discovered piece of hardware.
///
/// `Device` is a *description*: it says what the hardware can do. Live numbers
/// live in [`DeviceState`](crate::DeviceState).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    /// Serialised as `type`, matching the documented device model.
    #[serde(rename = "type")]
    pub device_type: DeviceType,
    #[serde(default)]
    pub vendor: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model: Option<String>,
    pub transport: Transport,
    /// Id of the adapter that owns this device.
    pub adapter: AdapterId,
    pub capabilities: Vec<Capability>,
    /// Free-form adapter specific data (PCI id, SMBus address, ...).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub metadata: BTreeMap<String, String>,
    /// Optional labels such as `cooling`, `hot-pluggable`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tags: Vec<String>,
}

impl Device {
    /// Start building a device description with an empty capability list.
    pub fn new(
        id: DeviceId,
        name: impl Into<String>,
        device_type: DeviceType,
        transport: Transport,
        adapter: AdapterId,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            device_type,
            vendor: String::new(),
            model: None,
            transport,
            adapter,
            capabilities: Vec::new(),
            metadata: BTreeMap::new(),
            tags: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_vendor(mut self, vendor: impl Into<String>) -> Self {
        self.vendor = vendor.into();
        self
    }

    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    #[must_use]
    pub fn with_capability(mut self, capability: Capability) -> Self {
        self.capabilities.push(capability);
        self
    }

    #[must_use]
    pub fn with_capabilities(mut self, capabilities: impl IntoIterator<Item = Capability>) -> Self {
        self.capabilities.extend(capabilities);
        self
    }

    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Look up a capability by id.
    pub fn capability(&self, id: &CapabilityId) -> Option<&Capability> {
        self.capabilities.iter().find(|c| &c.id == id)
    }

    /// Look up a capability by raw id string.
    pub fn capability_str(&self, id: &str) -> Option<&Capability> {
        self.capabilities.iter().find(|c| c.id.as_str() == id)
    }

    /// Does this device declare the capability?
    pub fn supports(&self, id: &str) -> bool {
        self.capability_str(id).is_some()
    }

    pub fn capabilities_of_kind(&self, kind: CapabilityKind) -> impl Iterator<Item = &Capability> {
        self.capabilities.iter().filter(move |c| c.kind == kind)
    }

    pub fn sensors(&self) -> impl Iterator<Item = &Capability> {
        self.capabilities_of_kind(CapabilityKind::Sensor)
    }

    pub fn actuators(&self) -> impl Iterator<Item = &Capability> {
        self.capabilities_of_kind(CapabilityKind::Actuator)
    }

    /// Actuators that can actually be written to.
    pub fn writable_capabilities(&self) -> impl Iterator<Item = &Capability> {
        self.capabilities.iter().filter(|c| c.writable)
    }

    /// `true` when at least one capability is writable, i.e. the device can be
    /// driven by an automation rule.
    pub fn is_controllable(&self) -> bool {
        self.capabilities.iter().any(|c| c.writable)
    }

    /// Display label for the UI, e.g. `NVIDIA GeForce RTX 5090`.
    pub fn display_name(&self) -> &str {
        &self.name
    }

    /// Guard against an adapter registering nonsense.
    pub fn validate(&self) -> Result<()> {
        if self.capabilities.is_empty() {
            return Err(OhmError::Adapter {
                adapter: self.adapter.to_string(),
                detail: format!("device `{}` declares no capabilities", self.id),
            });
        }
        let mut seen = Vec::with_capacity(self.capabilities.len());
        for cap in &self.capabilities {
            if seen.contains(&cap.id) {
                return Err(OhmError::Adapter {
                    adapter: self.adapter.to_string(),
                    detail: format!("device `{}` declares `{}` twice", self.id, cap.id),
                });
            }
            seen.push(cap.id.clone());
            if cap.writable && (cap.min.is_some() != cap.max.is_some()) {
                return Err(OhmError::Adapter {
                    adapter: self.adapter.to_string(),
                    detail: format!(
                        "device `{}` capability `{}` declares only one range bound",
                        self.id, cap.id
                    ),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit::Unit;
    use ohm_core::ids::capability as caps;

    fn gpu() -> Device {
        Device::new(
            DeviceId::new("gpu.nvidia.0").unwrap(),
            "NVIDIA GeForce RTX 5090",
            DeviceType::Gpu,
            Transport::Nvidia,
            AdapterId::new("nvidia").unwrap(),
        )
        .with_vendor("NVIDIA")
        .with_capability(Capability::sensor(
            caps::TEMPERATURE_CORE,
            "Core Temperature",
            Unit::Celsius,
        ))
        .with_capability(Capability::sensor(
            caps::POWER_TOTAL,
            "Total Power",
            Unit::Watt,
        ))
        .with_capability(Capability::actuator(
            caps::FAN_SPEED_PERCENT,
            "GPU Fan Speed",
            Unit::Percent,
            0.0,
            100.0,
        ))
    }

    #[test]
    fn lookup_helpers() {
        let gpu = gpu();
        assert!(gpu.supports(caps::TEMPERATURE_CORE));
        assert!(!gpu.supports(caps::FAN_RPM));
        assert_eq!(gpu.sensors().count(), 2);
        assert_eq!(gpu.actuators().count(), 1);
        assert!(gpu.is_controllable());
        assert_eq!(
            gpu.capability_str(caps::TEMPERATURE_CORE).unwrap().unit,
            Unit::Celsius
        );
    }

    #[test]
    fn json_uses_type_key_and_snake_case() {
        let json = serde_json::to_value(gpu()).unwrap();
        assert_eq!(json["id"], "gpu.nvidia.0");
        assert_eq!(json["type"], "gpu");
        assert_eq!(json["transport"], "nvidia");
        assert_eq!(json["adapter"], "nvidia");
        assert_eq!(json["capabilities"][0]["id"], "temperature.core");
        assert!(json.get("metadata").is_none());
    }

    #[test]
    fn validation_rejects_empty_and_duplicate_capabilities() {
        let empty = Device::new(
            DeviceId::new("fan.system.0").unwrap(),
            "Chassis Fan",
            DeviceType::Fan,
            Transport::System,
            AdapterId::new("system").unwrap(),
        );
        assert!(empty.validate().is_err());

        let dup =
            empty
                .clone()
                .with_capability(Capability::sensor(caps::FAN_RPM, "Fan RPM", Unit::Rpm));
        let dup = dup.with_capability(Capability::sensor(caps::FAN_RPM, "Fan RPM", Unit::Rpm));
        assert!(dup.validate().is_err());
        assert!(
            empty
                .with_capability(Capability::sensor(caps::FAN_RPM, "Fan RPM", Unit::Rpm))
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn type_and_transport_parse() {
        assert_eq!("NVMe".parse::<DeviceType>().unwrap(), DeviceType::Storage);
        assert!("温度".parse::<DeviceType>().is_err());
        assert_eq!("usb-hid".parse::<Transport>().unwrap(), Transport::UsbHid);
        assert!(DeviceType::Pump.requires_safe_floor());
        assert!(DeviceType::Fan.is_actuatable());
        assert!(!DeviceType::Gpu.is_actuatable());
    }
}
