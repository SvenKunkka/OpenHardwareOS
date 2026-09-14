//! Adapter SPI: the only way hardware enters OpenHardwareOS.
//!
//! Everything vendor specific lives behind [`HardwareAdapter`]. The runtime
//! never knows whether a fan is a SuperIO header on a motherboard, an NVML
//! handle or a future OpenFan on USB HID — it only sees devices and
//! capabilities.
//!
//! The contract every adapter must implement:
//!
//! ```text
//! probe()      -> can this adapter work on this machine right now?
//! discover()   -> which devices exist?
//! read_state() -> what are their readings?
//! write()      -> set one actuator value
//! ```
//!
//! `subscribe / poll` is expressed by the runtime calling `discover()` and
//! `read_all()` on a cadence; push capable transports (USB, protocol devices)
//! can additionally surface events through the runtime event bus.

use std::any::Any;

use async_trait::async_trait;
use ohm_core::{AdapterId, CapabilityId, DeviceId, Result};
use ohm_device_model::{Capability, Device, DeviceState, Reading, UnavailableReason, Value};
use serde::{Deserialize, Serialize};

/// Static description of an adapter, shown in Settings -> Providers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterInfo {
    /// Stable id, also used as the device id namespace, e.g. `mock`.
    pub id: AdapterId,
    /// Display name, e.g. "Mock Hardware".
    pub name: String,
    /// Prefix used when composing device ids, e.g. `gpu.mock.0`.
    pub namespace: String,
    #[serde(default)]
    pub description: String,
    /// Adapter (not crate) version.
    #[serde(default)]
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub homepage: Option<String>,
    /// `true` when writing requires Administrator / root.
    #[serde(default)]
    pub requires_admin: bool,
    /// `true` when the adapter can report devices appearing/disappearing
    /// without a full rescan (used to tune the discovery cadence).
    #[serde(default)]
    pub supports_hotplug: bool,
    /// Extra capabilities the adapter exposes to the runtime.
    #[serde(default)]
    pub capabilities: AdapterCapabilities,
}

impl AdapterInfo {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        namespace: impl Into<String>,
    ) -> Self {
        let namespace = namespace.into();
        Self {
            id: AdapterId::new_unchecked(id),
            name: name.into(),
            namespace,
            description: String::new(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            homepage: None,
            requires_admin: false,
            supports_hotplug: false,
            capabilities: AdapterCapabilities::default(),
        }
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    #[must_use]
    pub fn with_homepage(mut self, homepage: impl Into<String>) -> Self {
        self.homepage = Some(homepage.into());
        self
    }

    #[must_use]
    pub fn requires_admin(mut self) -> Self {
        self.requires_admin = true;
        self
    }

    #[must_use]
    pub fn with_hotplug(mut self) -> Self {
        self.supports_hotplug = true;
        self
    }

    #[must_use]
    pub fn with_capabilities(mut self, capabilities: AdapterCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }
}

/// What an adapter can do, used by the runtime to decide how to schedule it.
///
/// `Default` is the read-only provider: no writes, no cooling control, no
/// elevation, and the global polling cadence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterCapabilities {
    /// Adapter can set at least one actuator on at least one device.
    pub can_write: bool,
    /// Adapter exposes fan/pump duty controls.
    pub can_control_cooling: bool,
    /// Adapter needs elevated privileges for its *write* path.
    pub write_requires_admin: bool,
    /// Suggested poll cadence in milliseconds. `None` means "use the global
    /// polling interval".
    pub poll_interval_ms: Option<u64>,
    /// Suggested discovery cadence in milliseconds.
    pub discovery_interval_ms: Option<u64>,
}

impl AdapterCapabilities {
    /// A read-only telemetry provider.
    pub fn read_only() -> Self {
        Self::default()
    }

    /// A provider that can drive cooling hardware.
    pub fn cooling_control() -> Self {
        Self {
            can_write: true,
            can_control_cooling: true,
            write_requires_admin: true,
            poll_interval_ms: None,
            discovery_interval_ms: None,
        }
    }
}

/// Health of an adapter at a point in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterState {
    /// `probe()` has not run yet.
    NotProbed,
    /// Fully usable.
    Available,
    /// Usable but limited (for example: read-only because we are not elevated).
    Degraded,
    /// Not usable on this machine.
    Unavailable,
    /// The adapter reported an error while polling.
    Error,
}

impl AdapterState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotProbed => "not_probed",
            Self::Available => "available",
            Self::Degraded => "degraded",
            Self::Unavailable => "unavailable",
            Self::Error => "error",
        }
    }

    /// `true` when the adapter should be polled at all.
    pub fn is_usable(&self) -> bool {
        matches!(self, Self::Available | Self::Degraded | Self::Error)
    }
}

/// Result of [`HardwareAdapter::probe`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterStatus {
    pub adapter: AdapterId,
    pub state: AdapterState,
    /// Machine readable reason when not fully available.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<UnavailableReason>,
    /// Human readable explanation, shown verbatim in the UI.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub detail: Option<String>,
    /// Unix milliseconds of the check.
    pub checked_at_ms: i64,
    /// Number of devices seen at the last successful discovery.
    #[serde(default)]
    pub device_count: usize,
}

impl AdapterStatus {
    pub fn not_probed(adapter: AdapterId) -> Self {
        Self {
            adapter,
            state: AdapterState::NotProbed,
            reason: None,
            detail: None,
            checked_at_ms: 0,
            device_count: 0,
        }
    }

    pub fn available(adapter: AdapterId, device_count: usize) -> Self {
        Self {
            adapter,
            state: AdapterState::Available,
            reason: None,
            detail: None,
            checked_at_ms: ohm_core::now_ms(),
            device_count,
        }
    }

    pub fn degraded(
        adapter: AdapterId,
        reason: UnavailableReason,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            adapter,
            state: AdapterState::Degraded,
            reason: Some(reason),
            detail: Some(detail.into()),
            checked_at_ms: ohm_core::now_ms(),
            device_count: 0,
        }
    }

    pub fn unavailable(
        adapter: AdapterId,
        reason: UnavailableReason,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            adapter,
            state: AdapterState::Unavailable,
            reason: Some(reason),
            detail: Some(detail.into()),
            checked_at_ms: ohm_core::now_ms(),
            device_count: 0,
        }
    }

    pub fn error(adapter: AdapterId, detail: impl Into<String>) -> Self {
        Self {
            adapter,
            state: AdapterState::Error,
            reason: Some(UnavailableReason::ReadError),
            detail: Some(detail.into()),
            checked_at_ms: ohm_core::now_ms(),
            device_count: 0,
        }
    }

    #[must_use]
    pub fn with_device_count(mut self, count: usize) -> Self {
        self.device_count = count;
        self
    }

    pub fn is_usable(&self) -> bool {
        self.state.is_usable()
    }
}

/// How a write ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteStatus {
    /// The hardware accepted the value **and the adapter confirmed it** — either
    /// by reading the channel back, or because it owns the state (simulated
    /// hardware).
    Applied,
    /// The request was accepted but the resulting value is **unknown**: the
    /// channel could not be read back, or answered with nothing usable.
    ///
    /// This exists because "the write request succeeded" and "the device is at
    /// this value" are different claims, and collapsing them is how an
    /// application ends up reporting a fan speed it never verified. An
    /// `Unconfirmed` outcome carries **no** `applied` value: filling it in with
    /// the requested number would be a lie.
    Unconfirmed,
    /// The adapter simulated the write (mock devices, dry-run mode).
    Simulated,
    /// The hardware refused; `detail` explains why.
    Rejected,
}

/// Outcome of a successful (or explicitly rejected) write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WriteOutcome {
    pub status: WriteStatus,
    /// The value as actually applied (after clamping/quantisation).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub applied: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub detail: Option<String>,
}

impl WriteOutcome {
    pub fn applied(value: impl Into<Value>) -> Self {
        Self {
            status: WriteStatus::Applied,
            applied: Some(value.into()),
            detail: None,
        }
    }

    pub fn simulated(value: impl Into<Value>, detail: impl Into<String>) -> Self {
        Self {
            status: WriteStatus::Simulated,
            applied: Some(value.into()),
            detail: Some(detail.into()),
        }
    }

    pub fn rejected(detail: impl Into<String>) -> Self {
        Self {
            status: WriteStatus::Rejected,
            applied: None,
            detail: Some(detail.into()),
        }
    }

    /// The request reached the device, but the value could not be confirmed.
    ///
    /// `applied` is deliberately `None`: the honest answer is "unknown".
    pub fn unconfirmed(detail: impl Into<String>) -> Self {
        Self {
            status: WriteStatus::Unconfirmed,
            applied: None,
            detail: Some(detail.into()),
        }
    }

    /// The write is confirmed: the device is known to be at `applied`.
    pub fn is_applied(&self) -> bool {
        matches!(self.status, WriteStatus::Applied)
    }

    /// The write was accepted but the resulting value is unknown.
    pub fn is_unconfirmed(&self) -> bool {
        matches!(self.status, WriteStatus::Unconfirmed)
    }

    /// Did the hardware (or the simulation) end up in a known state?
    pub fn is_confirmed(&self) -> bool {
        matches!(self.status, WriteStatus::Applied | WriteStatus::Simulated)
    }
}

/// A batch read result: one entry per requested device.
pub type BatchStateResult = Vec<(DeviceId, Result<DeviceState>)>;

/// The hardware provider interface.
///
/// Implementors must never panic on missing hardware: report
/// [`UnavailableReason`] instead.
#[async_trait]
pub trait HardwareAdapter: Send + Sync + 'static {
    /// Static description of the adapter.
    fn info(&self) -> AdapterInfo;

    /// Convenience accessor for the adapter id.
    fn id(&self) -> AdapterId {
        self.info().id
    }

    /// Check whether this adapter can work on this machine *right now*.
    ///
    /// Called before [`HardwareAdapter::discover`] and re-checked periodically.
    async fn probe(&self) -> AdapterStatus;

    /// Enumerate the devices this adapter currently sees.
    ///
    /// Called on every discovery cycle, so it must also be the hotplug
    /// detection point: devices that disappeared simply are not returned.
    async fn discover(&self) -> Result<Vec<Device>>;

    /// Read every capability of one device.
    async fn read_state(&self, device: &Device) -> Result<DeviceState>;

    /// Read many devices in one go. The default implementation polls serially;
    /// adapters with a batch API (NVML, LibreHardwareMonitor) should override
    /// it so a cycle stays cheap.
    async fn read_all(&self, devices: &[Device]) -> BatchStateResult {
        let mut out = Vec::with_capacity(devices.len());
        for device in devices {
            let result = self.read_state(device).await;
            out.push((device.id.clone(), result));
        }
        out
    }

    /// Write one actuator value.
    ///
    /// Implementations must:
    /// * refuse read-only capabilities,
    /// * return [`WriteOutcome::rejected`] (or an error) when the OS/driver
    ///   says no, instead of pretending success,
    /// * never write a value outside the declared range.
    async fn write(
        &self,
        device: &Device,
        capability: &Capability,
        value: &Value,
    ) -> Result<WriteOutcome>;

    /// Release hardware resources. Called on runtime shutdown, which is where
    /// fan control is handed back to the BIOS/firmware.
    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    /// Downcast hook so the UI can reach adapter specific controls (for
    /// example the mock hardware load generator).
    fn as_any(&self) -> &dyn Any;
}

/// Helper for adapters that cannot read a capability at all.
pub fn unsupported(capability: &CapabilityId, detail: impl Into<String>) -> Reading {
    Reading::unavailable(
        capability.clone(),
        UnavailableReason::Unsupported,
        Some(detail.into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_core::ids::capability as caps;
    use ohm_device_model::{Capability, DeviceType, Transport};

    struct EchoAdapter;

    #[async_trait]
    impl HardwareAdapter for EchoAdapter {
        fn info(&self) -> AdapterInfo {
            AdapterInfo::new("echo", "Echo", "echo").with_description("test adapter")
        }

        async fn probe(&self) -> AdapterStatus {
            AdapterStatus::available(self.id(), 1)
        }

        async fn discover(&self) -> Result<Vec<Device>> {
            Ok(vec![
                Device::new(
                    DeviceId::new("fan.echo.0").unwrap(),
                    "Echo Fan",
                    DeviceType::Fan,
                    Transport::Mock,
                    self.id(),
                )
                .with_capability(Capability::sensor(
                    caps::FAN_RPM,
                    "Fan RPM",
                    ohm_device_model::Unit::Rpm,
                ))
                .with_capability(Capability::actuator(
                    caps::FAN_SPEED_PERCENT,
                    "Fan Speed",
                    ohm_device_model::Unit::Percent,
                    0.0,
                    100.0,
                )),
            ])
        }

        async fn read_state(&self, device: &Device) -> Result<DeviceState> {
            Ok(DeviceState::new(device.id.clone(), ohm_core::now_ms())
                .with_reading(Reading::ok(caps::FAN_RPM, 1200)))
        }

        async fn write(
            &self,
            _device: &Device,
            capability: &Capability,
            value: &Value,
        ) -> Result<WriteOutcome> {
            Ok(WriteOutcome::applied(
                capability.clamp(value.as_f64().unwrap_or(0.0)),
            ))
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[tokio::test]
    async fn default_read_all_polls_every_device() {
        let adapter = EchoAdapter;
        let devices = adapter.discover().await.unwrap();
        let results = adapter.read_all(&devices).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.as_str(), "fan.echo.0");
        assert_eq!(
            results[0].1.as_ref().unwrap().number(caps::FAN_RPM),
            Some(1200.0)
        );
    }

    #[tokio::test]
    async fn write_clamps_and_reports_status() {
        let adapter = EchoAdapter;
        let device = adapter.discover().await.unwrap().remove(0);
        let capability = device.capability_str(caps::FAN_SPEED_PERCENT).unwrap();
        let outcome = adapter
            .write(&device, capability, &Value::Number(180.0))
            .await
            .unwrap();
        assert!(outcome.is_applied());
        assert_eq!(outcome.applied, Some(Value::Number(100.0)));
    }

    #[test]
    fn status_helpers() {
        let id = AdapterId::new("mock").unwrap();
        assert_eq!(
            AdapterStatus::not_probed(id.clone()).state,
            AdapterState::NotProbed
        );
        assert!(!AdapterStatus::not_probed(id.clone()).is_usable());
        let unavailable = AdapterStatus::unavailable(
            id.clone(),
            UnavailableReason::DriverMissing,
            "not installed",
        );
        assert_eq!(unavailable.reason, Some(UnavailableReason::DriverMissing));
        assert_eq!(unavailable.detail.as_deref(), Some("not installed"));
        assert!(
            AdapterStatus::degraded(id, UnavailableReason::PermissionDenied, "elevate").is_usable()
        );
    }

    #[test]
    fn adapter_info_serialises() {
        let info = AdapterInfo::new("lhm", "LibreHardwareMonitor", "lhm")
            .with_description("SuperIO + motherboard sensors")
            .requires_admin()
            .with_hotplug();
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["id"], "lhm");
        assert_eq!(json["namespace"], "lhm");
        assert_eq!(json["requires_admin"], true);
        assert_eq!(json["supports_hotplug"], true);
    }
}
