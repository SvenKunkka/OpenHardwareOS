//! Capabilities: the only contract between the runtime and the hardware.

use std::fmt;
use std::str::FromStr;

use ohm_core::{CapabilityId, OhmError, Result};
use serde::{Deserialize, Serialize};

use crate::unit::Unit;
use crate::value::{Value, trim_float};

/// What a capability *is*, independently of the device carrying it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    /// Read-only measurement.
    Sensor,
    /// Writable set point.
    Actuator,
    /// Something the device can push asynchronously (button press, threshold).
    Event,
    /// Static or slow moving information (firmware version, model name).
    Info,
}

impl CapabilityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sensor => "sensor",
            Self::Actuator => "actuator",
            Self::Event => "event",
            Self::Info => "info",
        }
    }

    pub fn is_sensor(&self) -> bool {
        matches!(self, Self::Sensor)
    }

    pub fn is_actuator(&self) -> bool {
        matches!(self, Self::Actuator)
    }
}

impl fmt::Display for CapabilityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CapabilityKind {
    type Err = OhmError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "sensor" | "measurement" => Ok(Self::Sensor),
            "actuator" | "control" => Ok(Self::Actuator),
            "event" => Ok(Self::Event),
            "info" => Ok(Self::Info),
            other => Err(OhmError::Config(format!(
                "unknown capability kind `{other}` (expected sensor, actuator, event or info)"
            ))),
        }
    }
}

/// A single thing a device can do or report.
///
/// Business logic must branch on `id` + `kind` + `unit`, never on the device
/// model. That is what lets `gpu.nvidia.0/temperature.core` drive
/// `fan.system.0/fan.speed_percent` on one machine and `mock.gpu.0` drive
/// `mock.fan.0` on another with the same rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capability {
    pub id: CapabilityId,
    /// Human readable label, e.g. "Core Temperature".
    pub name: String,
    pub kind: CapabilityKind,
    pub unit: Unit,
    pub readable: bool,
    pub writable: bool,
    /// Inclusive lower bound for writable capabilities.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min: Option<f64>,
    /// Inclusive upper bound for writable capabilities.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max: Option<f64>,
    /// Smallest meaningful increment, when the hardware quantises the value.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub step: Option<f64>,
    /// Allowed enumeration values for text capabilities (e.g. control modes).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    /// Suggested poll cadence for this capability in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub poll_interval_ms: Option<u64>,
    /// `true` for capabilities where a wrong value can damage hardware (pumps,
    /// fans with a minimum duty). The safety layer enforces a floor for these.
    #[serde(default)]
    pub safety_critical: bool,
}

impl Capability {
    /// A read-only sensor.
    pub fn sensor(id: impl Into<String>, name: impl Into<String>, unit: Unit) -> Self {
        Self {
            id: CapabilityId::new_unchecked(id),
            name: name.into(),
            kind: CapabilityKind::Sensor,
            unit,
            readable: true,
            writable: false,
            min: None,
            max: None,
            step: None,
            values: Vec::new(),
            description: None,
            poll_interval_ms: None,
            safety_critical: false,
        }
    }

    /// A writable actuator with an explicit range.
    pub fn actuator(
        id: impl Into<String>,
        name: impl Into<String>,
        unit: Unit,
        min: f64,
        max: f64,
    ) -> Self {
        Self {
            id: CapabilityId::new_unchecked(id),
            name: name.into(),
            kind: CapabilityKind::Actuator,
            unit,
            readable: true,
            writable: true,
            min: Some(min),
            max: Some(max),
            step: None,
            values: Vec::new(),
            description: None,
            poll_interval_ms: None,
            safety_critical: false,
        }
    }

    /// A read-only information field.
    pub fn info(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: CapabilityId::new_unchecked(id),
            name: name.into(),
            kind: CapabilityKind::Info,
            unit: Unit::Text,
            readable: true,
            writable: false,
            min: None,
            max: None,
            step: None,
            values: Vec::new(),
            description: None,
            poll_interval_ms: None,
            safety_critical: false,
        }
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn with_step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    #[must_use]
    pub fn with_poll_interval_ms(mut self, ms: u64) -> Self {
        self.poll_interval_ms = Some(ms);
        self
    }

    /// Mark the capability as one that must never be driven to an unsafe value.
    #[must_use]
    pub fn safety_critical(mut self) -> Self {
        self.safety_critical = true;
        self
    }

    #[must_use]
    pub fn with_values(mut self, values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.values = values.into_iter().map(Into::into).collect();
        self
    }

    /// `true` when this capability looks like a fan/pump duty control.
    pub fn is_duty_control(&self) -> bool {
        self.kind.is_actuator() && self.unit.is_duty()
    }

    /// Validate a value against this capability *without* touching hardware.
    ///
    /// Rejects: writes to read-only capabilities, non numeric values for
    /// numeric units, values outside `min..=max`, and values that are not part
    /// of an enumeration.
    pub fn validate(&self, value: &Value) -> Result<()> {
        let ctx = |detail: String| OhmError::InvalidValue {
            device: "<device>".into(),
            capability: self.id.to_string(),
            detail,
        };

        if !self.writable {
            return Err(OhmError::CapabilityNotWritable {
                device: "<device>".into(),
                capability: self.id.to_string(),
            });
        }

        if !self.values.is_empty() {
            let text = match value.as_str() {
                Some(t) => t.to_string(),
                None => value.to_string(),
            };
            if !self.values.iter().any(|v| v == &text) {
                return Err(ctx(format!(
                    "`{text}` is not one of {}",
                    self.values.join(", ")
                )));
            }
            return Ok(());
        }

        let numeric = value
            .as_f64()
            .ok_or_else(|| ctx(format!("expected a numeric value, got `{value}`")))?;

        if !numeric.is_finite() {
            return Err(ctx(format!("value `{numeric}` is not finite")));
        }

        let min = self.min.unwrap_or(f64::NEG_INFINITY);
        let max = self.max.unwrap_or(f64::INFINITY);
        if numeric < min || numeric > max {
            return Err(OhmError::ValueOutOfRange {
                device: "<device>".into(),
                capability: self.id.to_string(),
                value: numeric,
                min,
                max,
            });
        }
        Ok(())
    }

    /// Clamp a value into the declared range, quantise it to `step` and return
    /// the value that will actually be written.
    pub fn clamp(&self, value: f64) -> f64 {
        let mut v = value;
        if let Some(min) = self.min {
            v = v.max(min);
        }
        if let Some(max) = self.max {
            v = v.min(max);
        }
        if let Some(step) = self.step.filter(|step| *step > 0.0) {
            v = (v / step).round() * step;
            if let Some(min) = self.min {
                v = v.max(min);
            }
            if let Some(max) = self.max {
                v = v.min(max);
            }
        }
        v
    }

    /// Short label used by the UI, e.g. `0-100 %`.
    pub fn range_label(&self) -> Option<String> {
        match (self.min, self.max) {
            (Some(min), Some(max)) => Some(format!(
                "{}-{}{}",
                trim_float(min),
                trim_float(max),
                self.unit.suffix()
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_core::ids::capability as caps;

    fn fan_control() -> Capability {
        Capability::actuator(
            caps::FAN_SPEED_PERCENT,
            "Fan Speed",
            Unit::Percent,
            0.0,
            100.0,
        )
    }

    #[test]
    fn sensor_is_read_only() {
        let c = Capability::sensor(caps::TEMPERATURE_CORE, "Core Temperature", Unit::Celsius);
        assert!(c.readable);
        assert!(!c.writable);
        assert!(c.kind.is_sensor());
        assert!(c.validate(&Value::Number(60.0)).is_err());
    }

    #[test]
    fn actuator_range_is_enforced() {
        let c = fan_control();
        assert!(c.validate(&Value::Number(70.0)).is_ok());
        let err = c.validate(&Value::Number(101.0)).unwrap_err();
        assert_eq!(err.code(), "value_out_of_range");
        assert!(c.validate(&Value::Number(-1.0)).is_err());
    }

    #[test]
    fn actuator_rejects_text() {
        let c = fan_control();
        let err = c.validate(&Value::Text("max".into())).unwrap_err();
        assert_eq!(err.code(), "invalid_value");
    }

    #[test]
    fn actuator_rejects_nan() {
        let c = fan_control();
        assert!(c.validate(&Value::Number(f64::NAN)).is_err());
    }

    #[test]
    fn enumeration_is_enforced() {
        let c = Capability::actuator("fan.mode", "Mode", Unit::Text, 0.0, 0.0)
            .with_values(["software", "default", "auto"]);
        assert!(c.validate(&Value::Text("software".into())).is_ok());
        assert!(c.validate(&Value::Text("turbo".into())).is_err());
    }

    #[test]
    fn clamping_and_quantising() {
        let c = fan_control().with_step(5.0);
        assert_eq!(c.clamp(103.0), 100.0);
        assert_eq!(c.clamp(-4.0), 0.0);
        assert_eq!(c.clamp(42.4), 40.0);
        assert_eq!(c.clamp(43.0), 45.0);
    }

    #[test]
    fn json_shape_matches_the_documented_model() {
        let c = fan_control();
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["id"], "fan.speed_percent");
        assert_eq!(json["kind"], "actuator");
        assert_eq!(json["unit"], "percent");
        assert_eq!(json["readable"], true);
        assert_eq!(json["writable"], true);
        assert_eq!(json["min"], 0.0);
        assert_eq!(json["max"], 100.0);
        // Optional fields stay out of the payload when unset.
        assert!(json.get("step").is_none());
    }

    #[test]
    fn duty_control_detection_and_labels() {
        assert!(fan_control().is_duty_control());
        assert_eq!(fan_control().range_label().unwrap(), "0-100 %");
        let pump = Capability::actuator("pump.speed_percent", "Pump", Unit::Percent, 20.0, 100.0)
            .safety_critical();
        assert!(pump.safety_critical);
        assert_eq!(pump.range_label().unwrap(), "20-100 %");
    }
}
