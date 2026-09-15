//! Read-only projections handed to the UI, the CLI and the automation engine.
//!
//! Snapshots are plain serialisable data: no locks, no handles, no hardware.
//! This is what the Tauri command layer returns.

use ohm_adapter_api::{AdapterInfo, AdapterStatus};
use ohm_device_model::{Device, DeviceState};
use serde::{Deserialize, Serialize};

use crate::config::Settings;

/// Coarse health of a device inside the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    /// Discovered and polled successfully.
    Online,
    /// Discovered, but the last poll failed or the device is gone.
    Offline,
    /// Discovered but disabled by the user; never polled, never written.
    Disabled,
    /// Polled, but some readings are unavailable.
    Degraded,
}

impl DeviceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Disabled => "disabled",
            Self::Degraded => "degraded",
        }
    }
}

/// A device plus everything the runtime knows about it right now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceView {
    pub device: Device,
    pub enabled: bool,
    pub status: DeviceStatus,
    /// `None` until the first successful poll.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub state: Option<DeviceState>,
    /// Adapter that produced the device.
    pub adapter: String,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
}

impl DeviceView {
    /// Number of capabilities the device declares.
    pub fn capability_count(&self) -> usize {
        self.device.capabilities.len()
    }

    /// `true` when this device can be driven by an automation rule.
    pub fn is_controllable(&self) -> bool {
        self.device.is_controllable() && self.enabled
    }
}

/// An adapter plus its current health.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterView {
    pub info: AdapterInfo,
    pub status: AdapterStatus,
}

impl AdapterView {
    pub fn id(&self) -> &str {
        self.info.id.as_str()
    }

    pub fn is_usable(&self) -> bool {
        self.status.is_usable()
    }
}

/// Counters for the Settings -> Diagnostics panel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RuntimeStats {
    pub poll_cycles: u64,
    pub discovery_cycles: u64,
    pub writes_attempted: u64,
    pub writes_applied: u64,
    pub writes_rejected: u64,
    pub safety_interventions: u64,
    pub events_published: u64,
    pub last_poll_ms: i64,
    pub last_poll_duration_ms: u64,
    pub last_poll_errors: usize,
}

/// Everything the UI needs to render one frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeSnapshot {
    pub generated_at_ms: i64,
    pub started_at_ms: i64,
    pub devices: Vec<DeviceView>,
    pub adapters: Vec<AdapterView>,
    pub settings: Settings,
    pub stats: RuntimeStats,
    /// True when the runtime currently owns at least one actuator.
    pub has_controllable_hardware: bool,
}

impl RuntimeSnapshot {
    /// Find a device view by id.
    pub fn device(&self, id: &str) -> Option<&DeviceView> {
        self.devices.iter().find(|d| d.device.id.as_str() == id)
    }

    /// Devices grouped by type, for the Overview page.
    pub fn by_type(&self, device_type: ohm_device_model::DeviceType) -> Vec<&DeviceView> {
        self.devices
            .iter()
            .filter(|d| d.device.device_type == device_type)
            .collect()
    }

    /// Every device that can be driven by a rule.
    pub fn controllable_devices(&self) -> Vec<&DeviceView> {
        self.devices
            .iter()
            .filter(|d| d.is_controllable())
            .collect()
    }

    /// Devices that expose at least one sensor reading.
    pub fn sensor_devices(&self) -> Vec<&DeviceView> {
        self.devices
            .iter()
            .filter(|d| {
                d.state
                    .as_ref()
                    .is_some_and(|s| s.readings.iter().any(|r| r.is_ok()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_core::{AdapterId, DeviceId};
    use ohm_device_model::{Capability, DeviceType, Reading, Transport, Unit};

    fn view(id: &str, device_type: DeviceType, controllable: bool) -> DeviceView {
        let mut device = Device::new(
            DeviceId::new(id).unwrap(),
            id,
            device_type,
            Transport::Mock,
            AdapterId::new("mock").unwrap(),
        );
        device.capabilities.push(Capability::sensor(
            "temperature.core",
            "Core Temperature",
            Unit::Celsius,
        ));
        if controllable {
            device.capabilities.push(Capability::actuator(
                "fan.speed_percent",
                "Fan Speed",
                Unit::Percent,
                0.0,
                100.0,
            ));
        }
        DeviceView {
            device,
            enabled: true,
            status: DeviceStatus::Online,
            state: Some(
                DeviceState::new(DeviceId::new(id).unwrap(), 1)
                    .with_reading(Reading::ok("temperature.core", 60.0)),
            ),
            adapter: "mock".to_string(),
            first_seen_ms: 0,
            last_seen_ms: 1,
        }
    }

    /// The JSON `ohm-cli status --json` prints is an interface: `scripts/verify-linux-readings.sh`
    /// reads exactly these paths to compare a reading with the platform's own
    /// source, and `docs/linux-install.md` tells users to run it. Renaming a field
    /// here silently changes what that script compares, so the paths are pinned
    /// where the type is.
    #[test]
    fn the_json_a_cross_check_script_reads_has_these_paths() {
        let value = serde_json::to_value(snapshot()).expect("snapshot serialises");
        let devices = value["devices"].as_array().expect("devices is an array");
        let gpu = &devices[0];
        assert!(gpu["device"]["id"].is_string(), "devices[].device.id");
        assert!(gpu["device"]["type"].is_string(), "devices[].device.type");
        // `metadata` is omitted when empty and an object when it is not: the
        // cross-check reads `metadata.mount_point` off storage devices, so both
        // shapes have to be what the script expects.
        assert!(
            gpu["device"].get("metadata").is_none(),
            "an empty metadata map is omitted"
        );
        let with_metadata = serde_json::to_value(
            Device::new(
                DeviceId::new("storage.system.0").unwrap(),
                "Root",
                DeviceType::Storage,
                Transport::System,
                AdapterId::new("system").unwrap(),
            )
            .with_metadata("mount_point", "/"),
        )
        .expect("device serialises");
        assert_eq!(with_metadata["metadata"]["mount_point"], "/");
        assert!(gpu["adapter"].is_string(), "devices[].adapter");
        let readings = gpu["state"]["readings"]
            .as_array()
            .expect("devices[].state.readings is an array");
        assert!(
            readings[0]["capability"].is_string(),
            "readings[].capability"
        );
        assert_eq!(readings[0]["status"], "ok", "readings[].status");
        assert_eq!(readings[0]["value"], 60.0, "readings[].value");

        // A missing reading carries its reason, which is the other half of what the
        // cross-check reports: "we report nothing" must always say why.
        let unavailable = Reading::unavailable(
            "temperature.hotspot",
            ohm_device_model::UnavailableReason::Unsupported,
            Some("no provider in this build exposes it".to_string()),
        );
        let state =
            DeviceState::new(DeviceId::new("gpu.mock.0").unwrap(), 1).with_reading(unavailable);
        let value = serde_json::to_value(state).expect("state serialises");
        let reading = &value["readings"][0];
        assert_eq!(reading["status"], "unavailable");
        assert!(reading["reason"].is_string(), "readings[].reason");
    }

    fn snapshot() -> RuntimeSnapshot {
        RuntimeSnapshot {
            generated_at_ms: 1,
            started_at_ms: 0,
            devices: vec![
                view("gpu.mock.0", DeviceType::Gpu, false),
                view("fan.mock.0", DeviceType::Fan, true),
            ],
            adapters: Vec::new(),
            settings: Settings::default(),
            stats: RuntimeStats::default(),
            has_controllable_hardware: true,
        }
    }

    #[test]
    fn lookups_work() {
        let snapshot = snapshot();
        assert!(snapshot.device("gpu.mock.0").is_some());
        assert!(snapshot.device("nope").is_none());
        assert_eq!(snapshot.by_type(DeviceType::Gpu).len(), 1);
        assert_eq!(snapshot.controllable_devices().len(), 1);
        assert_eq!(snapshot.sensor_devices().len(), 2);
        assert_eq!(snapshot.devices[0].capability_count(), 1);
        assert!(snapshot.devices[1].is_controllable());
    }

    #[test]
    fn snapshot_roundtrips_through_json() {
        let json = serde_json::to_string(&snapshot()).unwrap();
        let back: RuntimeSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot());
    }

    #[test]
    fn status_codes() {
        assert_eq!(DeviceStatus::Online.as_str(), "online");
        assert_eq!(
            serde_json::to_string(&DeviceStatus::Degraded).unwrap(),
            "\"degraded\""
        );
    }
}
