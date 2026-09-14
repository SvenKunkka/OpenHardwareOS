//! Live readings for a device.
//!
//! The MVP must never crash or lie when a sensor is missing. Every capability
//! therefore has an explicit *status*: either a value, or a machine readable
//! reason explaining why there is none (`unsupported`, `permission_denied`,
//! `vendor_limitation`, ...).

use std::fmt;
use std::str::FromStr;

use ohm_core::{CapabilityId, DeviceId, OhmError};
use serde::{Deserialize, Serialize};

use crate::value::Value;

/// Why a capability currently has no value.
///
/// This is the vocabulary behind the UI badges and behind the automation
/// engine's fallback decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// The hardware/driver simply does not implement this.
    Unsupported,
    /// The device is known but not attached right now (unplugged, GPU powered
    /// down).
    NotPresent,
    /// The OS refused access; usually "run as Administrator".
    PermissionDenied,
    /// The chip does not expose the value (no sensor wired, no tachometer).
    HardwareLimitation,
    /// The vendor tool/driver owns the value and blocks third parties.
    VendorLimitation,
    /// A required driver or helper is not installed.
    DriverMissing,
    /// The adapter timed out.
    Timeout,
    /// The device is disabled by the user.
    Disabled,
    /// A read went wrong; see the detail string.
    ReadError,
    Unknown,
}

impl UnavailableReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::NotPresent => "not_present",
            Self::PermissionDenied => "permission_denied",
            Self::HardwareLimitation => "hardware_limitation",
            Self::VendorLimitation => "vendor_limitation",
            Self::DriverMissing => "driver_missing",
            Self::Timeout => "timeout",
            Self::Disabled => "disabled",
            Self::ReadError => "read_error",
            Self::Unknown => "unknown",
        }
    }

    /// Should the UI show this as an error (red) or as an expected limitation
    /// (grey)? Only real failures are errors.
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::ReadError | Self::Timeout | Self::Unknown)
    }

    pub fn as_str(&self) -> &'static str {
        self.code()
    }
}

impl fmt::Display for UnavailableReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for UnavailableReason {
    type Err = OhmError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "unsupported" => Self::Unsupported,
            "not_present" => Self::NotPresent,
            "permission_denied" => Self::PermissionDenied,
            "hardware_limitation" => Self::HardwareLimitation,
            "vendor_limitation" => Self::VendorLimitation,
            "driver_missing" => Self::DriverMissing,
            "timeout" => Self::Timeout,
            "disabled" => Self::Disabled,
            "read_error" => Self::ReadError,
            "unknown" => Self::Unknown,
            other => {
                return Err(OhmError::Config(format!(
                    "unknown unavailable reason `{other}`"
                )));
            }
        })
    }
}

/// The status of one capability in a [`DeviceState`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReadingStatus {
    /// A fresh value.
    Ok { value: Value },
    /// No value, with a reason the UI can display verbatim.
    Unavailable {
        reason: UnavailableReason,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        detail: Option<String>,
    },
}

/// One capability reading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub capability: CapabilityId,
    #[serde(flatten)]
    pub status: ReadingStatus,
}

impl Reading {
    /// A successful reading.
    pub fn ok(capability: impl Into<CapabilityId>, value: impl Into<Value>) -> Self {
        Self {
            capability: capability.into(),
            status: ReadingStatus::Ok {
                value: value.into(),
            },
        }
    }

    /// A missing reading with an explanation.
    pub fn unavailable(
        capability: impl Into<CapabilityId>,
        reason: UnavailableReason,
        detail: impl Into<Option<String>>,
    ) -> Self {
        Self {
            capability: capability.into(),
            status: ReadingStatus::Unavailable {
                reason,
                detail: detail.into(),
            },
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self.status, ReadingStatus::Ok { .. })
    }

    /// The raw value, when available.
    pub fn value(&self) -> Option<&Value> {
        match &self.status {
            ReadingStatus::Ok { value } => Some(value),
            ReadingStatus::Unavailable { .. } => None,
        }
    }

    /// The numeric value, when available and numeric.
    pub fn number(&self) -> Option<f64> {
        self.value().and_then(Value::as_f64)
    }

    /// The reason, when unavailable.
    pub fn reason(&self) -> Option<UnavailableReason> {
        match &self.status {
            ReadingStatus::Ok { .. } => None,
            ReadingStatus::Unavailable { reason, .. } => Some(*reason),
        }
    }
}

/// A snapshot of every capability of one device at one point in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceState {
    pub device: DeviceId,
    /// Unix milliseconds.
    pub timestamp_ms: i64,
    #[serde(default)]
    pub readings: Vec<Reading>,
    /// `false` when the device disappeared between discovery and polling.
    #[serde(default = "default_true")]
    pub online: bool,
    /// Adapter supplied note, e.g. "requires Administrator".
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub message: Option<String>,
}

fn default_true() -> bool {
    true
}

impl DeviceState {
    pub fn new(device: DeviceId, timestamp_ms: i64) -> Self {
        Self {
            device,
            timestamp_ms,
            readings: Vec::new(),
            online: true,
            message: None,
        }
    }

    #[must_use]
    pub fn with_reading(mut self, reading: Reading) -> Self {
        self.readings.push(reading);
        self
    }

    #[must_use]
    pub fn with_readings(mut self, readings: impl IntoIterator<Item = Reading>) -> Self {
        self.readings.extend(readings);
        self
    }

    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Mark an entire poll as failed (device unplugged, adapter down).
    #[must_use]
    pub fn offline(mut self, reason: UnavailableReason, detail: impl Into<String>) -> Self {
        self.online = false;
        let detail = detail.into();
        for reading in &mut self.readings {
            reading.status = ReadingStatus::Unavailable {
                reason,
                detail: Some(detail.clone()),
            };
        }
        self
    }

    /// Insert or replace a reading.
    pub fn set(&mut self, reading: Reading) {
        match self
            .readings
            .iter_mut()
            .find(|r| r.capability == reading.capability)
        {
            Some(slot) => *slot = reading,
            None => self.readings.push(reading),
        }
    }

    pub fn get(&self, capability: &str) -> Option<&Reading> {
        self.readings
            .iter()
            .find(|r| r.capability.as_str() == capability)
    }

    /// Numeric value of a capability, if it is present and readable.
    pub fn number(&self, capability: &str) -> Option<f64> {
        self.get(capability).and_then(Reading::number)
    }

    /// Raw value of a capability, if present.
    pub fn value(&self, capability: &str) -> Option<&Value> {
        self.get(capability).and_then(Reading::value)
    }

    /// Every capability that could not be read this round.
    pub fn unavailable(&self) -> impl Iterator<Item = &Reading> {
        self.readings.iter().filter(|r| !r.is_ok())
    }

    /// `true` when at least one reading carries a real failure.
    pub fn has_failure(&self) -> bool {
        self.readings
            .iter()
            .filter_map(Reading::reason)
            .any(|r| r.is_failure())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_core::ids::capability as caps;

    fn state() -> DeviceState {
        DeviceState::new(DeviceId::new("gpu.mock.0").unwrap(), 1_700_000_000_000)
            .with_reading(Reading::ok(caps::TEMPERATURE_CORE, 68.5))
            .with_reading(Reading::unavailable(
                caps::TEMPERATURE_HOTSPOT,
                UnavailableReason::Unsupported,
                Some("mock has no hotspot sensor".to_string()),
            ))
    }

    #[test]
    fn reading_accessors() {
        let s = state();
        assert_eq!(s.number(caps::TEMPERATURE_CORE), Some(68.5));
        assert_eq!(s.number(caps::TEMPERATURE_HOTSPOT), None);
        assert!(s.get(caps::TEMPERATURE_CORE).unwrap().is_ok());
        assert_eq!(
            s.get(caps::TEMPERATURE_HOTSPOT).unwrap().reason(),
            Some(UnavailableReason::Unsupported)
        );
        assert_eq!(s.unavailable().count(), 1);
        assert!(
            !s.has_failure(),
            "unsupported is a limitation, not a failure"
        );
    }

    #[test]
    fn offline_marks_everything() {
        let s = state().offline(UnavailableReason::NotPresent, "device unplugged");
        assert!(!s.online);
        assert_eq!(s.unavailable().count(), 2);
        assert!(!s.has_failure(), "not_present is not a failure either");
    }

    #[test]
    fn read_error_is_a_failure() {
        let s = DeviceState::new(DeviceId::new("fan.system.0").unwrap(), 0).with_reading(
            Reading::unavailable(
                caps::FAN_RPM,
                UnavailableReason::ReadError,
                Some("io".into()),
            ),
        );
        assert!(s.has_failure());
    }

    #[test]
    fn set_replaces_in_place() {
        let mut s = state();
        s.set(Reading::ok(caps::TEMPERATURE_CORE, 70.0));
        assert_eq!(s.readings.len(), 2);
        assert_eq!(s.number(caps::TEMPERATURE_CORE), Some(70.0));
    }

    #[test]
    fn json_shape_is_ui_friendly() {
        let json = serde_json::to_value(state()).unwrap();
        assert_eq!(json["device"], "gpu.mock.0");
        assert_eq!(json["online"], true);
        assert_eq!(json["readings"][0]["capability"], "temperature.core");
        assert_eq!(json["readings"][0]["status"], "ok");
        assert_eq!(json["readings"][0]["value"], 68.5);
        assert_eq!(json["readings"][1]["status"], "unavailable");
        assert_eq!(json["readings"][1]["reason"], "unsupported");
    }

    #[test]
    fn reasons_parse_roundtrip() {
        for reason in [
            UnavailableReason::Unsupported,
            UnavailableReason::PermissionDenied,
            UnavailableReason::DriverMissing,
        ] {
            assert_eq!(
                reason.as_str().parse::<UnavailableReason>().unwrap(),
                reason
            );
        }
        assert!("nonsense".parse::<UnavailableReason>().is_err());
    }
}
