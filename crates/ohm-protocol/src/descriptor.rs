//! Device descriptors: how a device introduces itself.
//!
//! This is the heart of the "plug it in and the UI appears" promise: a device
//! reports its type, identity and **capabilities**, and the runtime turns that
//! into a [`Device`] without knowing anything about the hardware. Writing an
//! OpenFan firmware therefore means implementing this struct, not a driver.

use ohm_core::{AdapterId, DeviceId, OhmError, Result};
use ohm_device_model::{Capability, CapabilityKind, Device, DeviceType, Transport, Unit, Value};
use serde::{Deserialize, Serialize};

use crate::messages::{PROTOCOL_MAJOR, PROTOCOL_MINOR, is_compatible};

/// Firmware identification reported by the device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareInfo {
    pub version: String,
    #[serde(default)]
    pub build: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub git: Option<String>,
    /// Protocol version the firmware speaks.
    pub protocol_major: u16,
    pub protocol_minor: u16,
}

impl FirmwareInfo {
    pub fn new(version: &str) -> Self {
        Self {
            version: version.to_string(),
            build: String::new(),
            git: None,
            protocol_major: PROTOCOL_MAJOR,
            protocol_minor: PROTOCOL_MINOR,
        }
    }

    /// Can this host talk to that firmware?
    pub fn is_compatible(&self) -> bool {
        is_compatible(self.protocol_major, self.protocol_minor)
    }

    pub fn protocol_version(&self) -> String {
        format!("{}.{}", self.protocol_major, self.protocol_minor)
    }
}

/// A capability as advertised on the wire.
///
/// Mirrors [`Capability`] but stays a separate type on purpose: the protocol is
/// a *contract with firmware*, and it must be able to evolve (or gain a compact
/// encoding) without silently changing the runtime model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub id: String,
    pub name: String,
    pub kind: CapabilityKind,
    pub unit: Unit,
    pub readable: bool,
    pub writable: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub step: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    #[serde(default)]
    pub safety_critical: bool,
}

impl CapabilityDescriptor {
    /// A read-only measurement.
    pub fn sensor(id: &str, name: &str, unit: Unit) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            kind: CapabilityKind::Sensor,
            unit,
            readable: true,
            writable: false,
            min: None,
            max: None,
            step: None,
            values: Vec::new(),
            description: None,
            safety_critical: false,
        }
    }

    /// A writable control with a range.
    pub fn actuator(id: &str, name: &str, unit: Unit, min: f64, max: f64) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            kind: CapabilityKind::Actuator,
            unit,
            readable: true,
            writable: true,
            min: Some(min),
            max: Some(max),
            step: None,
            values: Vec::new(),
            description: None,
            safety_critical: false,
        }
    }

    #[must_use]
    pub fn with_description(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }

    #[must_use]
    pub fn safety_critical(mut self) -> Self {
        self.safety_critical = true;
        self
    }

    /// Validate a value the host wants to write.
    pub fn validate(
        &self,
        value: &Value,
    ) -> std::result::Result<f64, (crate::ProtocolErrorCode, String)> {
        use crate::ProtocolErrorCode as Code;
        if !self.writable {
            return Err((Code::NotWritable, format!("`{}` is read-only", self.id)));
        }
        if !self.values.is_empty() {
            let text = value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string());
            if !self.values.iter().any(|allowed| allowed == &text) {
                return Err((
                    Code::InvalidValue,
                    format!("`{text}` is not one of {}", self.values.join(", ")),
                ));
            }
            return Ok(0.0);
        }
        let Some(numeric) = value.as_f64() else {
            return Err((
                Code::InvalidValue,
                format!("expected a number, got `{value}`"),
            ));
        };
        if !numeric.is_finite() {
            return Err((Code::InvalidValue, "value is not finite".into()));
        }
        let min = self.min.unwrap_or(f64::NEG_INFINITY);
        let max = self.max.unwrap_or(f64::INFINITY);
        if numeric < min || numeric > max {
            return Err((
                Code::InvalidValue,
                format!("{numeric} is outside {min}..={max}"),
            ));
        }
        Ok(numeric)
    }
}

impl From<&Capability> for CapabilityDescriptor {
    fn from(capability: &Capability) -> Self {
        Self {
            id: capability.id.to_string(),
            name: capability.name.clone(),
            kind: capability.kind,
            unit: capability.unit,
            readable: capability.readable,
            writable: capability.writable,
            min: capability.min,
            max: capability.max,
            step: capability.step,
            values: capability.values.clone(),
            description: capability.description.clone(),
            safety_critical: capability.safety_critical,
        }
    }
}

impl From<&CapabilityDescriptor> for Capability {
    fn from(descriptor: &CapabilityDescriptor) -> Self {
        Self {
            id: ohm_core::CapabilityId::new_unchecked(descriptor.id.clone()),
            name: descriptor.name.clone(),
            kind: descriptor.kind,
            unit: descriptor.unit,
            readable: descriptor.readable,
            writable: descriptor.writable,
            min: descriptor.min,
            max: descriptor.max,
            step: descriptor.step,
            values: descriptor.values.clone(),
            description: descriptor.description.clone(),
            poll_interval_ms: None,
            safety_critical: descriptor.safety_critical,
        }
    }
}

/// What a device tells the host about itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceDescriptor {
    pub vendor: String,
    pub product: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub serial: Option<String>,
    pub device_type: DeviceType,
    pub transport: Transport,
    pub firmware: FirmwareInfo,
    pub capabilities: Vec<CapabilityDescriptor>,
}

impl DeviceDescriptor {
    /// Build a descriptor for a fan/pump controller.
    pub fn new(vendor: &str, product: &str, device_type: DeviceType, transport: Transport) -> Self {
        Self {
            vendor: vendor.to_string(),
            product: product.to_string(),
            model: None,
            serial: None,
            device_type,
            transport,
            firmware: FirmwareInfo::new("0.0.0"),
            capabilities: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_serial(mut self, serial: &str) -> Self {
        self.serial = Some(serial.to_string());
        self
    }

    #[must_use]
    pub fn with_model(mut self, model: &str) -> Self {
        self.model = Some(model.to_string());
        self
    }

    #[must_use]
    pub fn with_firmware(mut self, firmware: FirmwareInfo) -> Self {
        self.firmware = firmware;
        self
    }

    #[must_use]
    pub fn with_capabilities(
        mut self,
        capabilities: impl IntoIterator<Item = CapabilityDescriptor>,
    ) -> Self {
        self.capabilities = capabilities.into_iter().collect();
        self
    }

    /// Look up one capability.
    pub fn capability(&self, id: &str) -> Option<&CapabilityDescriptor> {
        self.capabilities.iter().find(|c| c.id == id)
    }

    /// Stable, filesystem and log friendly qualifier for the device id.
    pub fn qualifier(&self) -> String {
        let base = match &self.serial {
            Some(serial) if !serial.is_empty() => format!("{}-{}", self.vendor, serial),
            _ => format!("{}-{}", self.vendor, self.product),
        };
        base.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_string()
    }

    /// The id this device will get in the runtime.
    pub fn device_id(&self, index: usize) -> DeviceId {
        DeviceId::compose(self.device_type.as_str(), &self.qualifier(), index)
    }

    /// Convert the descriptor into a runtime [`Device`].
    pub fn to_device(&self, adapter: &AdapterId, index: usize) -> Result<Device> {
        if self.capabilities.is_empty() {
            return Err(OhmError::Protocol(format!(
                "device `{}` advertised no capabilities",
                self.product
            )));
        }
        if !self.firmware.is_compatible() {
            return Err(OhmError::Protocol(format!(
                "device `{}` speaks protocol {} but this host implements {}.{}",
                self.product,
                self.firmware.protocol_version(),
                PROTOCOL_MAJOR,
                PROTOCOL_MINOR
            )));
        }
        let mut device = Device::new(
            self.device_id(index),
            self.product.clone(),
            self.device_type,
            self.transport,
            adapter.clone(),
        )
        .with_vendor(self.vendor.clone())
        .with_metadata("protocol_version", self.firmware.protocol_version())
        .with_metadata("firmware", self.firmware.version.clone())
        .with_capabilities(self.capabilities.iter().map(Capability::from));

        if let Some(model) = &self.model {
            device.model = Some(model.clone());
            device.metadata.insert("model".into(), model.clone());
        }
        if let Some(serial) = &self.serial {
            device.metadata.insert("serial".into(), serial.clone());
        }
        if let Some(git) = &self.firmware.git {
            device.metadata.insert("firmware_git".into(), git.clone());
        }
        device.validate()?;
        Ok(device)
    }

    /// Compact one line summary for logs.
    pub fn summary(&self) -> String {
        format!(
            "{} {} ({} capabilities, protocol {}, fw {})",
            self.vendor,
            self.product,
            self.capabilities.len(),
            self.firmware.protocol_version(),
            self.firmware.version
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProtocolErrorCode;

    fn fan_descriptor() -> DeviceDescriptor {
        DeviceDescriptor::new(
            "OpenHardwareOS",
            "OpenFan 4",
            DeviceType::Fan,
            Transport::UsbHid,
        )
        .with_serial("OF4-0001")
        .with_firmware(FirmwareInfo::new("0.1.0"))
        .with_capabilities([
            CapabilityDescriptor::actuator(
                "fan.speed_percent",
                "Fan Speed",
                Unit::Percent,
                0.0,
                100.0,
            )
            .with_description("PWM duty for the fan header"),
            CapabilityDescriptor::sensor("fan.rpm", "Fan RPM", Unit::Rpm),
            CapabilityDescriptor::sensor("temperature.core", "Temperature", Unit::Celsius),
        ])
    }

    #[test]
    fn descriptor_becomes_a_runtime_device() {
        let descriptor = fan_descriptor();
        let adapter = AdapterId::new("opd").unwrap();
        let device = descriptor.to_device(&adapter, 0).unwrap();
        assert_eq!(device.id.as_str(), "fan.openhardwareos_of4_0001.0");
        assert_eq!(device.name, "OpenFan 4");
        assert_eq!(device.device_type, DeviceType::Fan);
        assert_eq!(device.transport, Transport::UsbHid);
        assert_eq!(device.vendor, "OpenHardwareOS");
        assert_eq!(
            device.metadata.get("serial").map(String::as_str),
            Some("OF4-0001")
        );
        assert_eq!(
            device.metadata.get("firmware").map(String::as_str),
            Some("0.1.0")
        );
        assert!(device.is_controllable());
        assert_eq!(device.sensors().count(), 2);
        assert_eq!(device.actuators().count(), 1);
    }

    #[test]
    fn ids_are_stable_across_runs() {
        let descriptor = fan_descriptor();
        assert_eq!(descriptor.qualifier(), "openhardwareos_of4_0001");
        assert_eq!(descriptor.device_id(0), descriptor.device_id(0));
        assert_ne!(descriptor.device_id(0), descriptor.device_id(1));

        // Without a serial the vendor+product is used.
        let mut anonymous = fan_descriptor();
        anonymous.serial = None;
        assert_eq!(anonymous.qualifier(), "openhardwareos_openfan_4");
    }

    #[test]
    fn a_device_without_capabilities_is_refused() {
        let descriptor = DeviceDescriptor::new("X", "Y", DeviceType::Fan, Transport::UsbCdc);
        let adapter = AdapterId::new("opd").unwrap();
        assert!(descriptor.to_device(&adapter, 0).is_err());
    }

    #[test]
    fn an_incompatible_firmware_is_refused() {
        let mut descriptor = fan_descriptor();
        descriptor.firmware.protocol_minor = 99;
        assert!(!descriptor.firmware.is_compatible());
        let adapter = AdapterId::new("opd").unwrap();
        let err = descriptor.to_device(&adapter, 0).unwrap_err();
        assert!(err.to_string().contains("protocol"));
    }

    #[test]
    fn capability_validation_matches_the_device_contract() {
        let descriptor = fan_descriptor();
        let control = descriptor.capability("fan.speed_percent").unwrap();
        assert_eq!(control.validate(&Value::Number(75.0)), Ok(75.0));
        assert!(matches!(
            control.validate(&Value::Number(120.0)),
            Err((ProtocolErrorCode::InvalidValue, _))
        ));
        assert!(matches!(
            control.validate(&Value::Text("max".into())),
            Err((ProtocolErrorCode::InvalidValue, _))
        ));

        let rpm = descriptor.capability("fan.rpm").unwrap();
        assert!(matches!(
            rpm.validate(&Value::Number(1200.0)),
            Err((ProtocolErrorCode::NotWritable, _))
        ));
    }

    #[test]
    fn capability_conversion_is_lossless_enough() {
        let descriptor = fan_descriptor();
        let capability: Capability = descriptor.capability("fan.speed_percent").unwrap().into();
        assert_eq!(capability.id.as_str(), "fan.speed_percent");
        assert_eq!(capability.min, Some(0.0));
        assert_eq!(capability.max, Some(100.0));
        assert!(capability.writable);
        let roundtrip = CapabilityDescriptor::from(&capability);
        assert_eq!(
            roundtrip,
            *descriptor.capability("fan.speed_percent").unwrap()
        );
    }

    #[test]
    fn summary_and_firmware_helpers() {
        let descriptor = fan_descriptor();
        assert!(descriptor.summary().contains("OpenFan 4"));
        assert!(descriptor.summary().contains("3 capabilities"));
        assert_eq!(descriptor.firmware.protocol_version(), "0.1");
        assert_eq!(descriptor.capability("nope"), None);
    }
}
