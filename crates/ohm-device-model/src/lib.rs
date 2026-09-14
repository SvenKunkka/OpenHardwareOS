//! The OpenHardwareOS device model.
//!
//! This crate is the contract every other part of the system speaks:
//!
//! ```text
//! Adapter  ──discover──▶  Device { capabilities }
//! Adapter  ──poll──────▶  DeviceState { readings }
//! Runtime  ──write─────▶  (device, capability, Value)
//! ```
//!
//! Nothing here mentions a vendor, a model number or a driver. Business logic
//! depends on [`DeviceType`] + [`Capability`], which is what makes the same
//! automation rule work against `mock.gpu.0`, an RTX 5090 and a future
//! OpenFan device.

pub mod capability;
pub mod device;
pub mod state;
pub mod unit;
pub mod value;

pub use capability::{Capability, CapabilityKind};
pub use device::{Device, DeviceType, Transport};
pub use state::{DeviceState, Reading, ReadingStatus, UnavailableReason};
pub use unit::Unit;
pub use value::{Value, trim_float};

/// Convenience re-export of the well known capability ids.
pub use ohm_core::ids::capability as caps;

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_core::{AdapterId, DeviceId};

    /// End to end check of the documented model shape: a GPU and a fan, both
    /// described purely by type + capabilities.
    #[test]
    fn documented_device_model_shape() {
        let gpu = Device::new(
            DeviceId::new("gpu.nvidia.0").unwrap(),
            "NVIDIA GeForce RTX 5090",
            DeviceType::Gpu,
            Transport::System,
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
        ));

        let fan = Device::new(
            DeviceId::new("fan.system.0").unwrap(),
            "Chassis Fan 1",
            DeviceType::Fan,
            Transport::System,
            AdapterId::new("system").unwrap(),
        )
        .with_capability(Capability::sensor(caps::FAN_RPM, "Fan RPM", Unit::Rpm))
        .with_capability(Capability::actuator(
            caps::FAN_SPEED_PERCENT,
            "Fan Speed",
            Unit::Percent,
            0.0,
            100.0,
        ));

        gpu.validate().unwrap();
        fan.validate().unwrap();

        let json = serde_json::to_value(&gpu).unwrap();
        assert_eq!(json["capabilities"][0]["kind"], "sensor");
        assert_eq!(json["capabilities"][0]["unit"], "celsius");
        assert_eq!(json["capabilities"][0]["readable"], true);
        assert_eq!(json["capabilities"][0]["writable"], false);
        assert_eq!(json["capabilities"][1]["id"], "power.total");

        let fan_json = serde_json::to_value(&fan).unwrap();
        assert_eq!(fan_json["capabilities"][1]["writable"], true);
        assert_eq!(fan_json["capabilities"][1]["min"], 0.0);
        assert_eq!(fan_json["capabilities"][1]["max"], 100.0);

        // A device can be rebuilt from its serialised form without loss.
        let back: Device = serde_json::from_value(json).unwrap();
        assert_eq!(back, gpu);
    }
}
