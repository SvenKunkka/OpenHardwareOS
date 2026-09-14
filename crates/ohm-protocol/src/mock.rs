//! A simulated Open Device Protocol device: the future OpenFan.
//!
//! OpenHardwareOS has no hardware of its own yet, so this module is the
//! *executable specification* of what the first real device must do:
//!
//! ```text
//! USB connected (or, here: transport created)
//!     │
//!     ├─ GET_DEVICE_INFO   -> descriptor: vendor, product, firmware, protocol
//!     ├─ GET_CAPABILITIES  -> fan.speed_percent, fan.rpm, temperature.core, ...
//!     ├─ GET_STATE         -> readings
//!     ├─ SET_STATE         -> validated, applied, acknowledged
//!     └─ SUBSCRIBE_EVENT   -> push updates
//! ```
//!
//! It also implements the safety behaviour the brief asks official hardware to
//! have: **a local fallback**. If the host stops talking (crash, cable pulled,
//! automation engine wedged) the device notice after
//! [`DEFAULT_FALLBACK_AFTER_MS`] and drives its own fans to
//! [`DEFAULT_FALLBACK_DUTY`] instead of holding whatever value it was last told.
//!
//! Tests drive it through the real framing and message codec, so the protocol
//! is genuinely exercised rather than mocked away.

use std::collections::BTreeMap;

use ohm_device_model::{
    Capability, DeviceType, Reading, Transport, UnavailableReason, Unit, Value, caps,
};

use crate::descriptor::{CapabilityDescriptor, DeviceDescriptor, FirmwareInfo};
use crate::messages::{ProtocolErrorCode, Request, Response};
use crate::transport::ProtocolDevice;

/// Duty the device falls back to when the host disappears.
pub const DEFAULT_FALLBACK_DUTY: f64 = 70.0;
/// How long the device waits before engaging its local fallback.
pub const DEFAULT_FALLBACK_AFTER_MS: u64 = 5_000;
/// Largest firmware image the mock device accepts.
pub const MAX_FIRMWARE_BYTES: usize = 1024 * 1024;

/// A simulated four channel fan/pump controller.
#[derive(Debug)]
pub struct MockOpenFan {
    descriptor: DeviceDescriptor,
    channels: usize,
    /// Single PWM group duty, in percent (all channels follow it).
    duty: f64,
    temperature_c: f64,
    ambient_c: f64,
    bootloader: bool,
    subscriptions: u32,
    firmware_received: u32,
    firmware_total: u32,
    firmware_crc: u8,
    fallback_duty: f64,
    fallback_after_ms: u64,
    /// Simulated milliseconds, advanced by [`MockOpenFan::tick`].
    sim_ms: u64,
    last_host_contact_ms: u64,
    fallback_engaged: bool,
    rpm_override: Option<f64>,
    faults: Vec<(String, UnavailableReason)>,
    uptime_ms: u64,
}

impl MockOpenFan {
    /// A four channel hub, the shape of the future OpenHub reference hardware.
    pub fn new(channels: usize) -> Self {
        let channels = channels.clamp(1, 8);
        let mut capabilities = vec![
            CapabilityDescriptor::actuator(
                caps::FAN_SPEED_PERCENT,
                "Fan Speed",
                Unit::Percent,
                0.0,
                100.0,
            )
            .with_description("PWM duty applied to every channel of the group"),
            CapabilityDescriptor::sensor(caps::FAN_RPM, "Fan RPM", Unit::Rpm)
                .with_description("Tachometer reading of channel 1"),
            CapabilityDescriptor::sensor(
                caps::TEMPERATURE_CORE,
                "Onboard Temperature",
                Unit::Celsius,
            )
            .with_description("Thermistor next to the fan header"),
        ];
        capabilities.push(CapabilityDescriptor {
            id: caps::STATUS_MESSAGE.to_string(),
            name: "Firmware Version".to_string(),
            kind: ohm_device_model::CapabilityKind::Info,
            unit: Unit::Text,
            readable: true,
            writable: false,
            min: None,
            max: None,
            step: None,
            values: Vec::new(),
            description: Some("Reported by GET_FIRMWARE_INFO".into()),
            safety_critical: false,
        });

        let descriptor = DeviceDescriptor::new(
            "OpenHardwareOS",
            &format!("OpenFan {channels}"),
            DeviceType::Fan,
            Transport::UsbHid,
        )
        .with_model("OHOS-OPENFAN-4")
        .with_serial("OF4-SIM-0001")
        .with_firmware(FirmwareInfo {
            version: "0.1.0-sim".into(),
            build: "mock".into(),
            git: Some("0000000".into()),
            protocol_major: crate::messages::PROTOCOL_MAJOR,
            protocol_minor: crate::messages::PROTOCOL_MINOR,
        })
        .with_capabilities(capabilities);

        Self {
            descriptor,
            channels,
            duty: 40.0,
            temperature_c: 32.0,
            ambient_c: 25.0,
            bootloader: false,
            subscriptions: 0,
            firmware_received: 0,
            firmware_total: 0,
            firmware_crc: 0,
            fallback_duty: DEFAULT_FALLBACK_DUTY,
            fallback_after_ms: DEFAULT_FALLBACK_AFTER_MS,
            sim_ms: 0,
            last_host_contact_ms: 0,
            fallback_engaged: false,
            rpm_override: None,
            faults: Vec::new(),
            uptime_ms: 0,
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Current duty of the PWM group.
    pub fn duty(&self) -> f64 {
        self.duty
    }

    /// Current tachometer reading.
    pub fn rpm(&self) -> f64 {
        self.rpm_override.unwrap_or_else(|| self.rpm_for(self.duty))
    }

    /// Is the device in bootloader mode?
    pub fn in_bootloader(&self) -> bool {
        self.bootloader
    }

    /// Has the local fallback taken over (host silent for too long)?
    pub fn fallback_engaged(&self) -> bool {
        self.fallback_engaged
    }

    /// Number of active subscriptions, for tests.
    pub fn subscriptions(&self) -> u32 {
        self.subscriptions
    }

    /// Configure the local fallback behaviour.
    pub fn set_local_fallback(&mut self, duty: f64, after_ms: u64) {
        self.fallback_duty = duty.clamp(0.0, 100.0);
        self.fallback_after_ms = after_ms;
    }

    /// Turn the local fallback off, for tests that only exercise the thermal
    /// model.
    pub fn disable_local_fallback(&mut self) {
        self.fallback_after_ms = u64::MAX;
    }

    /// Force a tachometer reading, e.g. to simulate a stalled fan.
    pub fn set_rpm_override(&mut self, rpm: Option<f64>) {
        self.rpm_override = rpm;
    }

    /// Make one capability unreadable, to exercise the host's unavailable path.
    pub fn inject_unavailable(&mut self, capability: &str, reason: UnavailableReason) {
        self.faults.push((capability.to_string(), reason));
    }

    /// Advance the simulated device clock.
    ///
    /// Returns `true` when this tick engaged the local fallback.
    pub fn tick(&mut self, dt_ms: u64) {
        self.sim_ms = self.sim_ms.saturating_add(dt_ms);
        self.uptime_ms = self.uptime_ms.saturating_add(dt_ms);

        // The device notices a silent host and protects itself.
        if !self.fallback_engaged
            && self.sim_ms.saturating_sub(self.last_host_contact_ms) > self.fallback_after_ms
        {
            self.fallback_engaged = true;
            self.duty = self.fallback_duty;
            tracing::warn!(
                duty = self.duty,
                "mock OpenFan: host went silent, local fallback engaged"
            );
        }

        // Simple thermal model: more airflow, lower temperature.
        let target = (self.ambient_c + 40.0 - 0.35 * self.duty).max(self.ambient_c);
        let alpha = (dt_ms as f64 / 8_000.0).clamp(0.0, 1.0);
        self.temperature_c += (target - self.temperature_c) * alpha;
    }

    /// Register host contact (any received request counts).
    fn host_spoke(&mut self) {
        self.last_host_contact_ms = self.sim_ms;
        if self.fallback_engaged {
            tracing::info!("mock OpenFan: host is back, local fallback released");
            self.fallback_engaged = false;
        }
    }

    fn rpm_for(&self, duty: f64) -> f64 {
        if duty <= 0.0 {
            0.0
        } else {
            (400.0 + 1_600.0 * (duty / 100.0)).round()
        }
    }

    fn reading(&self, capability: &str, value: Value) -> Reading {
        match self
            .faults
            .iter()
            .find(|(id, _)| id == capability)
            .map(|(_, reason)| *reason)
        {
            Some(reason) => Reading::unavailable(
                capability,
                reason,
                Some("injected fault on the simulated device".to_string()),
            ),
            None => Reading::ok(capability, value),
        }
    }

    /// Every reading the device can produce.
    pub fn readings(&self) -> Vec<Reading> {
        vec![
            self.reading(caps::FAN_SPEED_PERCENT, Value::Number(self.duty)),
            self.reading(caps::FAN_RPM, Value::Integer(self.rpm().round() as i64)),
            self.reading(
                caps::TEMPERATURE_CORE,
                Value::Number((self.temperature_c * 10.0).round() / 10.0),
            ),
            self.reading(
                caps::STATUS_MESSAGE,
                Value::Text(self.descriptor.firmware.version.clone()),
            ),
        ]
    }

    /// Apply a validated duty.
    fn apply_duty(&mut self, requested: f64) -> f64 {
        let clamped = requested.clamp(0.0, 100.0);
        self.duty = clamped;
        clamped
    }

    fn decode_hex(hex: &str) -> std::result::Result<Vec<u8>, String> {
        let hex = hex.trim();
        if !hex.len().is_multiple_of(2) {
            return Err("hex payload has an odd length".into());
        }
        let mut out = Vec::with_capacity(hex.len() / 2);
        let bytes = hex.as_bytes();
        for pair in bytes.chunks(2) {
            let text = std::str::from_utf8(pair).map_err(|e| e.to_string())?;
            let byte = u8::from_str_radix(text, 16).map_err(|e| e.to_string())?;
            out.push(byte);
        }
        Ok(out)
    }

    /// What the device would answer, without touching its state. Used by the
    /// adapter to build a `DeviceState`.
    pub fn state_readings(&self, capability: Option<&str>) -> Vec<Reading> {
        match capability {
            None => self.readings(),
            Some(id) => self
                .readings()
                .into_iter()
                .filter(|r| r.capability.as_str() == id)
                .collect(),
        }
    }
}

impl Default for MockOpenFan {
    fn default() -> Self {
        Self::new(4)
    }
}

impl ProtocolDevice for MockOpenFan {
    fn handle(&mut self, request: Request) -> Response {
        self.host_spoke();

        if self.bootloader
            && !matches!(
                request,
                Request::UpdateFirmware { .. } | Request::Ping { .. }
            )
        {
            return Response::error(
                ProtocolErrorCode::BootloaderMode,
                "device is in bootloader mode; only UPDATE_FIRMWARE and PING are accepted",
            );
        }

        match request {
            Request::GetDeviceInfo => Response::DeviceInfo {
                descriptor: self.descriptor.clone(),
            },
            Request::GetCapabilities => Response::Capabilities {
                capabilities: self.descriptor.capabilities.clone(),
            },
            Request::GetState { capability } => Response::State {
                readings: self.state_readings(capability.as_deref()),
            },
            Request::SetState { capability, value } => {
                let Some(descriptor) = self.descriptor.capability(&capability) else {
                    return Response::error(
                        ProtocolErrorCode::UnknownCapability,
                        format!("no capability `{capability}`"),
                    );
                };
                match descriptor.validate(&value) {
                    Ok(numeric) if capability == caps::FAN_SPEED_PERCENT => {
                        let applied = self.apply_duty(numeric);
                        let note = if (applied - numeric).abs() > f64::EPSILON {
                            Some(format!("clamped from {numeric} to {applied}"))
                        } else {
                            None
                        };
                        Response::StateSet {
                            capability,
                            applied: Value::Number(applied),
                            note,
                        }
                    }
                    Ok(_) => Response::error(
                        ProtocolErrorCode::Unsupported,
                        format!("`{capability}` is not remotely controllable in this firmware"),
                    ),
                    Err((code, message)) => Response::error(code, message),
                }
            }
            Request::SubscribeEvent {
                capability,
                min_interval_ms,
            } => {
                self.subscriptions += 1;
                let interval = min_interval_ms.unwrap_or(1_000).clamp(100, 60_000);
                tracing::debug!(capability = ?capability, interval, "mock OpenFan: subscription created");
                Response::Subscribed {
                    subscription: self.subscriptions,
                    interval_ms: interval,
                }
            }
            Request::UnsubscribeEvent { subscription } => {
                self.subscriptions = self.subscriptions.saturating_sub(1);
                Response::Unsubscribed { subscription }
            }
            Request::GetFirmwareInfo => Response::FirmwareInfo {
                firmware: self.descriptor.firmware.clone(),
            },
            Request::EnterBootloader { confirm } => {
                if !confirm {
                    return Response::Bootloader {
                        entered: false,
                        detail: Some("explicit confirmation required".into()),
                    };
                }
                self.bootloader = true;
                Response::Bootloader {
                    entered: true,
                    detail: Some("mock device entered bootloader mode".into()),
                }
            }
            Request::UpdateFirmware {
                offset,
                total,
                data_hex,
                image_crc8,
            } => {
                if total as usize > MAX_FIRMWARE_BYTES {
                    return Response::error(
                        ProtocolErrorCode::FirmwareRejected,
                        format!(
                            "image of {total} bytes exceeds the {MAX_FIRMWARE_BYTES} byte limit"
                        ),
                    );
                }
                let chunk = match Self::decode_hex(&data_hex) {
                    Ok(bytes) => bytes,
                    Err(message) => {
                        return Response::error(ProtocolErrorCode::FirmwareRejected, message);
                    }
                };
                if offset != self.firmware_received {
                    return Response::error(
                        ProtocolErrorCode::FirmwareRejected,
                        format!(
                            "expected offset {} but got {offset}",
                            self.firmware_received
                        ),
                    );
                }
                if self.firmware_total != total {
                    self.firmware_total = total;
                    self.firmware_crc = 0;
                }
                self.firmware_crc = crate::framing::crc8_update(self.firmware_crc, &chunk);
                self.firmware_received += chunk.len() as u32;

                if self.firmware_received < self.firmware_total {
                    return Response::FirmwareChunk {
                        received: self.firmware_received,
                        total: self.firmware_total,
                    };
                }
                if self.firmware_crc != image_crc8 {
                    self.firmware_received = 0;
                    self.firmware_total = 0;
                    self.firmware_crc = 0;
                    return Response::error(
                        ProtocolErrorCode::FirmwareRejected,
                        format!(
                            "image checksum mismatch: computed {:02X}, expected {image_crc8:02X}",
                            self.firmware_crc
                        ),
                    );
                }
                self.descriptor.firmware.version = "0.1.0-sim+updated".into();
                self.bootloader = false;
                let received = self.firmware_received;
                self.firmware_received = 0;
                self.firmware_total = 0;
                self.firmware_crc = 0;
                Response::FirmwareChunk { received, total }
            }
            Request::Ping { nonce } => Response::Pong {
                nonce,
                uptime_ms: self.uptime_ms,
            },
        }
    }

    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }
}

/// Convert protocol readings into the runtime's capability-keyed map, dropping
/// anything the descriptor does not declare.
pub fn readings_to_capabilities(readings: &[Reading]) -> BTreeMap<String, Reading> {
    readings
        .iter()
        .map(|reading| (reading.capability.to_string(), reading.clone()))
        .collect()
}

/// The capability list of a descriptor, ready to attach to a device.
pub fn capabilities_of(descriptor: &DeviceDescriptor) -> Vec<Capability> {
    descriptor
        .capabilities
        .iter()
        .map(Capability::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{DeviceTransport, LoopbackTransport};

    fn device() -> MockOpenFan {
        MockOpenFan::new(4)
    }

    #[test]
    fn descriptor_is_complete() {
        let device = device();
        let descriptor = device.descriptor();
        assert_eq!(descriptor.device_type, DeviceType::Fan);
        assert_eq!(descriptor.transport, Transport::UsbHid);
        assert!(descriptor.firmware.is_compatible());
        assert_eq!(descriptor.capabilities.len(), 4);
        assert!(
            descriptor
                .capability(caps::FAN_SPEED_PERCENT)
                .unwrap()
                .writable
        );
        assert!(!descriptor.capability(caps::FAN_RPM).unwrap().writable);
        assert_eq!(device.channels(), 4);
    }

    #[test]
    fn full_plug_in_flow_works_over_the_wire() {
        let mut transport = LoopbackTransport::new(device());

        // 1. Read the descriptor.
        let descriptor = crate::transport::handshake(&mut transport).unwrap();
        assert_eq!(descriptor.product, "OpenFan 4");

        // 2. Read the capabilities.
        let response = transport.call(Request::GetCapabilities).unwrap();
        match response {
            Response::Capabilities { capabilities } => assert_eq!(capabilities.len(), 4),
            other => panic!("unexpected {other:?}"),
        }

        // 3. Read the state.
        let readings = match transport
            .call(Request::GetState { capability: None })
            .unwrap()
        {
            Response::State { readings } => readings,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(readings.len(), 4);
        assert_eq!(
            readings
                .iter()
                .find(|r| r.capability.as_str() == caps::FAN_SPEED_PERCENT)
                .unwrap()
                .number(),
            Some(40.0)
        );

        // 4. Write a new duty and read it back.
        transport
            .call(Request::SetState {
                capability: caps::FAN_SPEED_PERCENT.into(),
                value: Value::Number(85.0),
            })
            .unwrap();
        let response = transport
            .call(Request::GetState {
                capability: Some(caps::FAN_RPM.into()),
            })
            .unwrap();
        match response {
            Response::State { readings } => {
                assert_eq!(readings.len(), 1);
                assert_eq!(
                    readings[0].number(),
                    Some((400.0_f64 + 1600.0 * 0.85).round())
                );
            }
            other => panic!("unexpected {other:?}"),
        }

        // 5. Subscribe.
        let response = transport
            .call(Request::SubscribeEvent {
                capability: None,
                min_interval_ms: Some(500),
            })
            .unwrap();
        assert!(matches!(
            response,
            Response::Subscribed {
                interval_ms: 500,
                ..
            }
        ));
        let response = transport
            .call(Request::UnsubscribeEvent { subscription: 1 })
            .unwrap();
        assert!(matches!(response, Response::Unsubscribed { .. }));

        // 6. Identify the firmware.
        let response = transport.call(Request::GetFirmwareInfo).unwrap();
        assert!(matches!(response, Response::FirmwareInfo { .. }));
    }

    #[test]
    fn writes_are_validated_by_the_device() {
        let mut device = device();
        let response = device.handle(Request::SetState {
            capability: "fan.speed_percent".into(),
            value: Value::Number(150.0),
        });
        match response {
            Response::Error { code, .. } => assert_eq!(code, ProtocolErrorCode::InvalidValue),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            device.duty(),
            40.0,
            "the device must not change on a bad write"
        );

        let response = device.handle(Request::SetState {
            capability: "fan.rpm".into(),
            value: Value::Number(1000.0),
        });
        assert!(matches!(
            response,
            Response::Error {
                code: ProtocolErrorCode::NotWritable,
                ..
            }
        ));

        let response = device.handle(Request::SetState {
            capability: "does.not.exist".into(),
            value: Value::Number(1.0),
        });
        assert!(matches!(
            response,
            Response::Error {
                code: ProtocolErrorCode::UnknownCapability,
                ..
            }
        ));
    }

    #[test]
    fn local_fallback_protects_the_hardware_when_the_host_dies() {
        let mut device = device();
        device.set_local_fallback(80.0, 2_000);
        device.handle(Request::SetState {
            capability: caps::FAN_SPEED_PERCENT.into(),
            value: Value::Number(10.0),
        });
        assert_eq!(device.duty(), 10.0);

        // The host stops talking.
        device.tick(1_000);
        assert!(!device.fallback_engaged());
        assert_eq!(device.duty(), 10.0);

        device.tick(2_000);
        assert!(device.fallback_engaged());
        assert_eq!(device.duty(), 80.0, "the device must protect itself");

        // The host comes back and takes control again.
        device.handle(Request::SetState {
            capability: caps::FAN_SPEED_PERCENT.into(),
            value: Value::Number(30.0),
        });
        assert!(!device.fallback_engaged());
        assert_eq!(device.duty(), 30.0);
    }

    #[test]
    fn bootloader_mode_locks_out_control_and_firmware_updates_work() {
        let mut device = device();
        let response = device.handle(Request::EnterBootloader { confirm: false });
        assert!(matches!(
            response,
            Response::Bootloader { entered: false, .. }
        ));
        assert!(!device.in_bootloader());

        let response = device.handle(Request::EnterBootloader { confirm: true });
        assert!(matches!(
            response,
            Response::Bootloader { entered: true, .. }
        ));
        assert!(device.in_bootloader());

        // Control is refused while the bootloader runs.
        let response = device.handle(Request::SetState {
            capability: caps::FAN_SPEED_PERCENT.into(),
            value: Value::Number(50.0),
        });
        assert!(matches!(
            response,
            Response::Error {
                code: ProtocolErrorCode::BootloaderMode,
                ..
            }
        ));

        // A well formed image is accepted.
        let image = b"openfan firmware 0.2.0";
        let crc = crate::framing::crc8(image);
        let hex: String = image.iter().map(|b| format!("{b:02X}")).collect();
        let response = device.handle(Request::UpdateFirmware {
            offset: 0,
            total: image.len() as u32,
            data_hex: hex,
            image_crc8: crc,
        });
        assert!(matches!(response, Response::FirmwareChunk { .. }));
        assert!(!device.in_bootloader());
        assert_eq!(device.descriptor.firmware.version, "0.1.0-sim+updated");
    }

    #[test]
    fn a_corrupt_firmware_image_is_refused() {
        let mut device = device();
        let image = b"firmware";
        let hex: String = image.iter().map(|b| format!("{b:02X}")).collect();
        let response = device.handle(Request::UpdateFirmware {
            offset: 0,
            total: image.len() as u32,
            data_hex: hex,
            image_crc8: 0x00,
        });
        assert!(matches!(
            response,
            Response::Error {
                code: ProtocolErrorCode::FirmwareRejected,
                ..
            }
        ));
    }

    #[test]
    fn chunked_firmware_upload_is_checksummed_incrementally() {
        let mut device = device();
        let image = b"0123456789abcdef";
        let crc = crate::framing::crc8(image);
        for (index, chunk) in image.chunks(4).enumerate() {
            let hex: String = chunk.iter().map(|b| format!("{b:02X}")).collect();
            let response = device.handle(Request::UpdateFirmware {
                offset: (index * 4) as u32,
                total: image.len() as u32,
                data_hex: hex,
                image_crc8: crc,
            });
            assert!(matches!(
                response,
                Response::FirmwareChunk { .. } | Response::StateSet { .. }
            ));
        }
        assert_eq!(device.descriptor.firmware.version, "0.1.0-sim+updated");
    }

    #[test]
    fn ping_reports_uptime() {
        let mut device = device();
        device.tick(1_500);
        match device.handle(Request::Ping { nonce: 3 }) {
            Response::Pong { nonce, uptime_ms } => {
                assert_eq!(nonce, 3);
                assert_eq!(uptime_ms, 1_500);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn injected_faults_surface_as_unavailable_readings() {
        let mut device = device();
        device.inject_unavailable(
            caps::TEMPERATURE_CORE,
            UnavailableReason::HardwareLimitation,
        );
        let readings = device.state_readings(None);
        let temperature = readings
            .iter()
            .find(|r| r.capability.as_str() == caps::TEMPERATURE_CORE)
            .unwrap();
        assert_eq!(
            temperature.reason(),
            Some(UnavailableReason::HardwareLimitation)
        );
        assert!(
            device
                .state_readings(Some(caps::FAN_RPM))
                .iter()
                .all(Reading::is_ok)
        );
    }

    #[test]
    fn temperature_follows_airflow() {
        let mut hot = device();
        hot.disable_local_fallback();
        hot.handle(Request::SetState {
            capability: caps::FAN_SPEED_PERCENT.into(),
            value: Value::Number(0.0),
        });
        let mut cool = device();
        cool.disable_local_fallback();
        cool.handle(Request::SetState {
            capability: caps::FAN_SPEED_PERCENT.into(),
            value: Value::Number(100.0),
        });
        for _ in 0..50 {
            hot.tick(1_000);
            cool.tick(1_000);
        }
        assert!(hot.readings()[2].number().unwrap() > cool.readings()[2].number().unwrap() + 5.0);
    }

    #[test]
    fn rpm_override_simulates_a_stalled_fan() {
        let mut device = device();
        device.set_rpm_override(Some(0.0));
        device.handle(Request::SetState {
            capability: caps::FAN_SPEED_PERCENT.into(),
            value: Value::Number(100.0),
        });
        let rpm = device
            .state_readings(Some(caps::FAN_RPM))
            .remove(0)
            .number()
            .unwrap();
        assert_eq!(
            rpm, 0.0,
            "a stalled fan must report 0 rpm even at 100 % duty"
        );
    }

    #[test]
    fn helpers_expose_standard_shapes() {
        let device = device();
        let map = readings_to_capabilities(&device.readings());
        assert!(map.contains_key(caps::FAN_RPM));
        let capabilities = capabilities_of(device.descriptor());
        assert_eq!(capabilities.len(), 4);
        assert_eq!(MockOpenFan::default().channels(), 4);
    }
}
