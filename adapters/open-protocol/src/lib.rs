//! Open Device Protocol adapter.
//!
//! This adapter is the proof that the protocol crate is real: it performs the
//! full plug-in flow against a device that speaks ODP —
//!
//! ```text
//! transport attached
//!     ↓ handshake()          GET_DEVICE_INFO   -> descriptor
//!     ↓ capabilities         GET_CAPABILITIES  -> fan.speed_percent, fan.rpm, ...
//!     ↓ device description   descriptor.to_device()
//!     ↓ runtime registration (the runtime does that, not us)
//!     ↓ reads and writes     GET_STATE / SET_STATE
//! ```
//!
//! Today the device on the other side is [`MockOpenFan`], so the whole path can
//! be demonstrated on a machine with no OpenHardwareOS hardware. When the first
//! real OpenHub/OpenFan exists, only the transport changes: a USB HID or CDC
//! transport implements [`DeviceTransport`](ohm_protocol::DeviceTransport) and
//! this adapter registers it instead — no change to the runtime, the automation
//! engine or the UI. That is the entire point of the layering.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use ohm_adapter_api::{
    AdapterCapabilities, AdapterInfo, AdapterStatus, HardwareAdapter, WriteOutcome,
};
use ohm_core::{AdapterId, OhmError, Result};
use ohm_device_model::{Capability, Device, DeviceState, UnavailableReason, Value};
use ohm_protocol::mock::MockOpenFan;
use ohm_protocol::{
    DeviceTransport, LoopbackTransport, ProtocolErrorCode, Request, Response, handshake,
};
use parking_lot::Mutex;

/// Adapter id, also the device id namespace.
pub const ADAPTER_ID: &str = "opd";
/// Display name.
pub const ADAPTER_NAME: &str = "Open Device Protocol";

fn adapter_id() -> AdapterId {
    AdapterId::new_unchecked(ADAPTER_ID)
}

/// Brings ODP devices into the runtime.
///
/// The transport is behind a mutex because a serial link is a single resource:
/// two concurrent writes must not interleave frames.
pub struct OpdAdapter {
    transport: Mutex<LoopbackTransport<MockOpenFan>>,
    /// Devices registered by the last discovery.
    devices: Mutex<Vec<Device>>,
    /// Firmware/descriptor cache, used for `probe()` and metadata.
    descriptor: Mutex<Option<ohm_protocol::DeviceDescriptor>>,
}

impl std::fmt::Debug for OpdAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpdAdapter")
            .field("devices", &self.devices.lock().len())
            .field("handshaken", &self.descriptor.lock().is_some())
            .finish()
    }
}

impl Default for OpdAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl OpdAdapter {
    /// An adapter talking to the simulated four channel OpenFan.
    pub fn new() -> Self {
        Self::with_device(MockOpenFan::new(4))
    }

    /// An adapter talking to an arbitrary ODP device.
    pub fn with_device(device: MockOpenFan) -> Self {
        Self {
            transport: Mutex::new(LoopbackTransport::new(device)),
            devices: Mutex::new(Vec::new()),
            descriptor: Mutex::new(None),
        }
    }

    /// As a trait object, ready to register on a runtime.
    pub fn boxed() -> Arc<dyn HardwareAdapter> {
        Arc::new(Self::new())
    }

    /// Advance the simulated device clock (used by the UI's mock panel).
    pub fn tick(&mut self, dt_ms: u64) {
        self.transport.lock().device_mut().tick(dt_ms);
    }

    /// Has the device gone silent on us and engaged its own fallback?
    pub fn fallback_engaged(&self) -> bool {
        self.transport.lock().device().fallback_engaged()
    }

    /// Current duty of the simulated device, in percent.
    pub fn duty(&self) -> f64 {
        self.transport.lock().device().duty()
    }

    /// Cached descriptor, if the handshake has run.
    pub fn descriptor(&self) -> Option<ohm_protocol::DeviceDescriptor> {
        self.descriptor.lock().clone()
    }

    /// Call the device, converting protocol errors into runtime errors.
    fn call(&self, request: Request) -> Result<Response> {
        let response = self.transport.lock().call(request)?;
        match response {
            Response::Error { code, message } => Err(OhmError::WriteRejected {
                device: "<device>".into(),
                capability: "<capability>".into(),
                detail: format!("device answered {}: {message}", code.as_str()),
            }),
            other => Ok(other),
        }
    }

    /// Perform the handshake and (re)build the device list.
    fn handshake_and_discover(&self) -> Result<Vec<Device>> {
        let mut transport = self.transport.lock();
        let descriptor = handshake(&mut *transport)?;
        let device = descriptor.to_device(&adapter_id(), 0)?;
        *self.descriptor.lock() = Some(descriptor);
        *self.devices.lock() = vec![device.clone()];
        Ok(vec![device])
    }

    /// Translate a protocol failure into the runtime's vocabulary.
    fn protocol_error(code: ProtocolErrorCode, message: &str) -> OhmError {
        let reason = code.unavailable_reason();
        OhmError::WriteRejected {
            device: "<device>".into(),
            capability: "<capability>".into(),
            detail: format!("{}: {message}", reason.as_str()),
        }
    }
}

#[async_trait]
impl HardwareAdapter for OpdAdapter {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::new(ADAPTER_ID, ADAPTER_NAME, "opd")
            .with_description(
                "Devices that implement the Open Device Protocol over USB HID or USB CDC. \
                 Currently serves the simulated OpenFan reference device, which is how the \
                 plug-in-and-it-appears flow is demonstrated before the hardware exists.",
            )
            .with_hotplug()
            .with_capabilities(AdapterCapabilities {
                can_write: true,
                can_control_cooling: true,
                write_requires_admin: false,
                poll_interval_ms: None,
                discovery_interval_ms: None,
            })
    }

    async fn probe(&self) -> AdapterStatus {
        match self.handshake_and_discover() {
            Ok(devices) => AdapterStatus::available(self.id(), devices.len()),
            Err(err) => AdapterStatus::unavailable(
                self.id(),
                UnavailableReason::NotPresent,
                format!("no ODP device answered the handshake: {err}"),
            ),
        }
    }

    async fn discover(&self) -> Result<Vec<Device>> {
        self.handshake_and_discover()
    }

    async fn read_state(&self, device: &Device) -> Result<DeviceState> {
        let response = self.call(Request::GetState { capability: None })?;
        let Response::State { readings } = response else {
            return Err(OhmError::Protocol(format!(
                "unexpected answer to GET_STATE: {}",
                response.opcode()
            )));
        };

        let declared: Vec<&Capability> = device.capabilities.iter().collect();
        let mut state = DeviceState::new(device.id.clone(), ohm_core::now_ms());
        for capability in &declared {
            match readings
                .iter()
                .find(|reading| reading.capability == capability.id)
            {
                Some(reading) => state.set(reading.clone()),
                None => state.set(ohm_device_model::Reading::unavailable(
                    capability.id.clone(),
                    UnavailableReason::Unsupported,
                    Some("the device does not report this capability".to_string()),
                )),
            }
        }
        Ok(state)
    }

    async fn write(
        &self,
        device: &Device,
        capability: &Capability,
        value: &Value,
    ) -> Result<WriteOutcome> {
        if !capability.writable {
            return Err(OhmError::CapabilityNotWritable {
                device: device.id.to_string(),
                capability: capability.id.to_string(),
            });
        }
        capability.validate(value)?;
        let requested = value.as_f64().ok_or_else(|| OhmError::InvalidValue {
            device: device.id.to_string(),
            capability: capability.id.to_string(),
            detail: format!("expected a number, got `{value}`"),
        })?;
        let applied = capability.clamp(requested);

        let response = self
            .transport
            .lock()
            .call(Request::SetState {
                capability: capability.id.to_string(),
                value: Value::Number(applied),
            })
            .map_err(|err| OhmError::WriteRejected {
                device: device.id.to_string(),
                capability: capability.id.to_string(),
                detail: err.to_string(),
            })?;

        match response {
            Response::StateSet { applied, note, .. } => Ok(WriteOutcome {
                status: ohm_adapter_api::WriteStatus::Applied,
                applied: Some(applied),
                detail: note,
            }),
            Response::Error { code, message } => Err(Self::protocol_error(code, &message)),
            other => Err(OhmError::Protocol(format!(
                "unexpected answer to SET_STATE: {}",
                other.opcode()
            ))),
        }
    }

    /// The device keeps its own local fallback curve, so there is nothing to
    /// release: if we stop talking it protects itself. This is the behaviour
    /// official hardware is required to implement.
    async fn shutdown(&self) -> Result<()> {
        tracing::debug!("opd adapter shutdown: the device owns its local fallback");
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_device_model::{DeviceType, Transport, caps};

    #[tokio::test]
    async fn handshake_registers_a_device_with_capabilities() {
        let adapter = OpdAdapter::new();
        let devices = adapter.discover().await.unwrap();
        assert_eq!(devices.len(), 1);

        let device = &devices[0];
        device.validate().unwrap();
        assert_eq!(device.device_type, DeviceType::Fan);
        assert_eq!(device.transport, Transport::UsbHid);
        assert_eq!(device.vendor, "OpenHardwareOS");
        assert!(device.is_controllable());
        assert!(device.supports(caps::FAN_SPEED_PERCENT));
        assert!(device.supports(caps::FAN_RPM));
        assert!(device.metadata.contains_key("firmware"));
        assert!(device.metadata.contains_key("protocol_version"));

        let descriptor = adapter.descriptor().unwrap();
        assert_eq!(descriptor.firmware.version, "0.1.0-sim");
    }

    #[tokio::test]
    async fn probe_reports_the_device_count() {
        let adapter = OpdAdapter::new();
        let status = adapter.probe().await;
        assert_eq!(status.state, ohm_adapter_api::AdapterState::Available);
        assert_eq!(status.device_count, 1);
        assert!(adapter.info().supports_hotplug);
    }

    #[tokio::test]
    async fn readings_come_from_get_state() {
        let adapter = OpdAdapter::new();
        let device = adapter.discover().await.unwrap().remove(0);
        let state = adapter.read_state(&device).await.unwrap();
        assert!(state.online);
        assert_eq!(state.number(caps::FAN_SPEED_PERCENT), Some(40.0));
        assert!(state.number(caps::TEMPERATURE_CORE).unwrap() > 20.0);
        // Capabilities the device does not report are marked unavailable.
        assert_eq!(state.readings.len(), device.capabilities.len());
    }

    #[tokio::test]
    async fn writes_go_through_set_state() {
        let adapter = OpdAdapter::new();
        let device = adapter.discover().await.unwrap().remove(0);
        let control = device.capability_str(caps::FAN_SPEED_PERCENT).unwrap();

        let outcome = adapter
            .write(&device, control, &Value::Number(88.0))
            .await
            .unwrap();
        assert!(outcome.is_applied());
        assert_eq!(outcome.applied, Some(Value::Number(88.0)));
        assert_eq!(adapter.duty(), 88.0);

        // The new duty is visible on the next read.
        let state = adapter.read_state(&device).await.unwrap();
        assert_eq!(state.number(caps::FAN_SPEED_PERCENT), Some(88.0));
    }

    #[tokio::test]
    async fn invalid_writes_never_reach_the_device() {
        let adapter = OpdAdapter::new();
        let device = adapter.discover().await.unwrap().remove(0);
        let control = device.capability_str(caps::FAN_SPEED_PERCENT).unwrap();
        let rpm = device.capability_str(caps::FAN_RPM).unwrap();

        assert_eq!(
            adapter
                .write(&device, control, &Value::Number(140.0))
                .await
                .unwrap_err()
                .code(),
            "value_out_of_range"
        );
        assert_eq!(
            adapter
                .write(&device, rpm, &Value::Number(1000.0))
                .await
                .unwrap_err()
                .code(),
            "capability_read_only"
        );
        assert_eq!(adapter.duty(), 40.0, "the device must not have moved");
    }

    #[tokio::test]
    async fn the_device_protects_itself_when_the_host_goes_silent() {
        let mut adapter = OpdAdapter::new();
        let device = adapter.discover().await.unwrap().remove(0);
        let control = device.capability_str(caps::FAN_SPEED_PERCENT).unwrap();
        adapter
            .write(&device, control, &Value::Number(15.0))
            .await
            .unwrap();
        assert_eq!(adapter.duty(), 15.0);

        // Five seconds of silence: the device falls back to 70 % on its own.
        adapter.tick(6_000);
        assert!(adapter.fallback_engaged());
        assert_eq!(adapter.duty(), 70.0);

        // Talking to it again hands control back.
        adapter
            .write(&device, control, &Value::Number(30.0))
            .await
            .unwrap();
        assert!(!adapter.fallback_engaged());
        assert_eq!(adapter.duty(), 30.0);
    }

    #[tokio::test]
    async fn adapter_metadata() {
        let adapter = OpdAdapter::new();
        let info = adapter.info();
        assert_eq!(info.id.as_str(), ADAPTER_ID);
        assert_eq!(info.namespace, "opd");
        assert!(!info.requires_admin, "an ODP device needs no elevation");
        assert!(info.capabilities.can_control_cooling);
        assert_eq!(OpdAdapter::default().info().id.as_str(), ADAPTER_ID);
        assert_eq!(OpdAdapter::boxed().info().id.as_str(), ADAPTER_ID);
        assert!(!format!("{adapter:?}").is_empty());
    }

    #[test]
    fn protocol_errors_map_to_runtime_vocabulary() {
        let err = OpdAdapter::protocol_error(ProtocolErrorCode::NotWritable, "read-only");
        assert!(err.to_string().contains("vendor_limitation"));
        let err = OpdAdapter::protocol_error(ProtocolErrorCode::Unauthorized, "denied");
        assert!(err.to_string().contains("permission_denied"));
    }
}
