//! Validated identifiers used by devices, capabilities, adapters and rules.
//!
//! Identifiers are stable, human readable and vendor neutral, e.g.
//! `gpu.nvidia.0`, `fan.system.1`, `temperature.core`. They are the only way
//! the automation engine refers to hardware, which is what keeps business logic
//! free of model specific branches.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{OhmError, Result};

const MAX_ID_LEN: usize = 96;

fn validate(value: &str, kind: &str) -> Result<()> {
    if value.is_empty() {
        return Err(OhmError::InvalidId {
            value: value.to_string(),
            detail: format!("{kind} must not be empty"),
        });
    }
    if value.len() > MAX_ID_LEN {
        return Err(OhmError::InvalidId {
            value: value.to_string(),
            detail: format!("{kind} must be at most {MAX_ID_LEN} characters"),
        });
    }
    let ok = value.chars().all(|c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-' | ':')
    });
    if !ok {
        return Err(OhmError::InvalidId {
            value: value.to_string(),
            detail: format!(
                "{kind} may only contain lowercase ascii letters, digits and `.`, `_`, `-`, `:`"
            ),
        });
    }
    Ok(())
}

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validate and wrap a raw string.
            pub fn new(value: impl Into<String>) -> Result<Self> {
                let value = value.into();
                validate(&value, $kind)?;
                Ok(Self(value))
            }

            /// Wrap without validation. Only for internally generated ids.
            pub fn new_unchecked(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.0)
            }
        }

        impl FromStr for $name {
            type Err = OhmError;

            fn from_str(s: &str) -> Result<Self> {
                Self::new(s)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        /// Unchecked conversion from a string literal. Ids are normally built
        /// through [`Self::new`]; this exists so the well known capability
        /// constants can be passed directly to constructors.
        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&String> for $name {
            fn from(value: &String) -> Self {
                Self(value.clone())
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

string_id!(
    /// Identifies a device inside the runtime, e.g. `gpu.nvidia.0`.
    DeviceId,
    "device id"
);
string_id!(
    /// Identifies a capability inside a device, e.g. `temperature.core`.
    CapabilityId,
    "capability id"
);
string_id!(
    /// Identifies a hardware adapter, e.g. `mock`.
    AdapterId,
    "adapter id"
);
string_id!(
    /// Identifies an automation rule, e.g. `gpu-cooling`.
    RuleId,
    "rule id"
);

impl DeviceId {
    /// Build a device id from a namespace, a vendor/kind and an instance index.
    ///
    /// `DeviceId::compose("gpu", "nvidia", 0)` yields `gpu.nvidia.0`.
    pub fn compose(namespace: &str, qualifier: &str, index: usize) -> Self {
        let qualifier = qualifier
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();
        Self(format!("{namespace}.{qualifier}.{index}"))
    }
}

impl CapabilityId {
    /// `temperature.core` style helper.
    pub fn namespaced(namespace: &str, name: &str) -> Self {
        Self(format!("{namespace}.{name}"))
    }
}

/// Well known capability ids.
///
/// Adapters are encouraged to use these constants so that automation rules stay
/// portable between a mock device and real hardware.
pub mod capability {
    /// CPU / GPU / SSD die temperature, in Celsius.
    pub const TEMPERATURE_CORE: &str = "temperature.core";
    /// GPU hotspot (junction) temperature, in Celsius.
    pub const TEMPERATURE_HOTSPOT: &str = "temperature.hotspot";
    /// Motherboard / system temperature, in Celsius.
    pub const TEMPERATURE_SYSTEM: &str = "temperature.system";
    /// Generic utilization, in percent.
    pub const LOAD: &str = "load.total";
    /// CPU total utilization, in percent.
    pub const CPU_LOAD: &str = "load.cpu";
    /// GPU core utilization, in percent.
    pub const GPU_LOAD: &str = "load.gpu";
    /// Package power draw, in watt.
    pub const POWER_TOTAL: &str = "power.total";
    /// GPU board power draw, in watt.
    pub const POWER_GPU: &str = "power.gpu";
    /// Fan tachometer reading, in rpm.
    pub const FAN_RPM: &str = "fan.rpm";
    /// Fan control set point, in percent.
    pub const FAN_SPEED_PERCENT: &str = "fan.speed_percent";
    /// Fan control set point, in pwm duty (0-255).
    pub const FAN_PWM: &str = "fan.pwm";
    /// Pump control set point, in percent.
    pub const PUMP_SPEED_PERCENT: &str = "pump.speed_percent";
    /// Pump tachometer reading, in rpm.
    pub const PUMP_RPM: &str = "pump.rpm";
    /// Clock frequency, in megahertz.
    pub const CLOCK_MHZ: &str = "clock.mhz";
    /// Last known adapter/hardware status message.
    pub const STATUS_MESSAGE: &str = "status.message";
    /// Memory used, in bytes.
    pub const MEMORY_USED: &str = "memory.used";
    /// Total memory of a device, in bytes.
    pub const MEMORY_TOTAL: &str = "memory.total";
    /// Free disk space, in bytes.
    pub const DISK_FREE: &str = "storage.free";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_dotted_ids() {
        let id = DeviceId::new("gpu.nvidia.0").unwrap();
        assert_eq!(id.as_str(), "gpu.nvidia.0");
        assert_eq!(id.to_string(), "gpu.nvidia.0");
    }

    #[test]
    fn rejects_uppercase_and_spaces() {
        assert!(DeviceId::new("GPU 0").is_err());
        assert!(DeviceId::new("").is_err());
        assert!(DeviceId::new("a".repeat(MAX_ID_LEN + 1)).is_err());
    }

    #[test]
    fn compose_normalises_qualifier() {
        assert_eq!(
            DeviceId::compose("gpu", "NVIDIA GeForce", 0).as_str(),
            "gpu.nvidia_geforce.0"
        );
    }

    #[test]
    fn serde_is_transparent() {
        let json = serde_json::to_string(&CapabilityId::new("fan.rpm").unwrap()).unwrap();
        assert_eq!(json, "\"fan.rpm\"");
        let back: CapabilityId = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_str(), "fan.rpm");
    }

    #[test]
    fn ordering_is_deterministic() {
        let mut ids = [
            DeviceId::new("fan.system.1").unwrap(),
            DeviceId::new("fan.system.0").unwrap(),
        ];
        ids.sort();
        assert_eq!(ids[0].as_str(), "fan.system.0");
    }
}
