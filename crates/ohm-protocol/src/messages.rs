//! Protocol messages.
//!
//! The vocabulary is deliberately the one from the project brief:
//!
//! ```text
//! GET_DEVICE_INFO     GET_CAPABILITIES   GET_STATE
//! SET_STATE           SUBSCRIBE_EVENT    GET_FIRMWARE_INFO
//! ENTER_BOOTLOADER    UPDATE_FIRMWARE    PING
//! ```
//!
//! Messages are JSON for the MVP: it is inspectable, diffable and trivial to
//! implement on an MCU with a small JSON writer. A future firmware revision can
//! switch to a compact binary encoding behind the same opcodes without changing
//! the Rust side's shape.

use ohm_device_model::{Reading, Value};
use serde::{Deserialize, Serialize};

use crate::descriptor::DeviceDescriptor;

/// Major version: breaking changes only.
pub const PROTOCOL_MAJOR: u16 = 0;
/// Minor version: additive changes.
pub const PROTOCOL_MINOR: u16 = 1;

/// Human readable protocol version, e.g. `0.1`.
pub fn protocol_version() -> String {
    format!("{PROTOCOL_MAJOR}.{PROTOCOL_MINOR}")
}

/// Can we talk to a device speaking `major.minor`?
///
/// Same major, and the device must not be *newer* than us: a device that only
/// knows a newer protocol may use messages we do not implement.
pub fn is_compatible(major: u16, minor: u16) -> bool {
    major == PROTOCOL_MAJOR && minor <= PROTOCOL_MINOR
}

/// A request from the host to a device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Request {
    /// Identify the device.
    GetDeviceInfo,
    /// Ask what the device can do.
    GetCapabilities,
    /// Read one capability, or everything when `capability` is `None`.
    GetState {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        capability: Option<String>,
    },
    /// Write one actuator.
    SetState { capability: String, value: Value },
    /// Start streaming changes for a capability.
    SubscribeEvent {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        capability: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        min_interval_ms: Option<u64>,
    },
    /// Stop a subscription.
    UnsubscribeEvent { subscription: u32 },
    /// Firmware identification, for support and update flows.
    GetFirmwareInfo,
    /// Jump to the bootloader. Must be explicit, and is refused unless the
    /// device is in a safe state.
    EnterBootloader { confirm: bool },
    /// Push a firmware chunk.
    UpdateFirmware {
        offset: u32,
        total: u32,
        /// Hex encoded chunk, so the payload stays JSON.
        data_hex: String,
        /// CRC-8 of the whole image, checked when the last chunk arrives.
        image_crc8: u8,
    },
    /// Liveness and latency probe.
    Ping { nonce: u32 },
}

impl Request {
    /// Opcode name, used in logs and tests.
    pub fn opcode(&self) -> &'static str {
        match self {
            Self::GetDeviceInfo => "GET_DEVICE_INFO",
            Self::GetCapabilities => "GET_CAPABILITIES",
            Self::GetState { .. } => "GET_STATE",
            Self::SetState { .. } => "SET_STATE",
            Self::SubscribeEvent { .. } => "SUBSCRIBE_EVENT",
            Self::UnsubscribeEvent { .. } => "UNSUBSCRIBE_EVENT",
            Self::GetFirmwareInfo => "GET_FIRMWARE_INFO",
            Self::EnterBootloader { .. } => "ENTER_BOOTLOADER",
            Self::UpdateFirmware { .. } => "UPDATE_FIRMWARE",
            Self::Ping { .. } => "PING",
        }
    }

    /// `true` for messages that change device state.
    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            Self::SetState { .. }
                | Self::EnterBootloader { .. }
                | Self::UpdateFirmware { .. }
                | Self::SubscribeEvent { .. }
                | Self::UnsubscribeEvent { .. }
        )
    }

    pub fn encode(&self) -> ohm_core::Result<Vec<u8>> {
        let json = serde_json::to_vec(self)?;
        crate::framing::encode(&json)
    }

    pub fn decode(payload: &[u8]) -> ohm_core::Result<Self> {
        Ok(serde_json::from_slice(payload)?)
    }
}

/// A response from a device to the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Response {
    /// Answer to [`Request::GetDeviceInfo`].
    DeviceInfo { descriptor: DeviceDescriptor },
    /// Answer to [`Request::GetCapabilities`].
    Capabilities {
        capabilities: Vec<crate::descriptor::CapabilityDescriptor>,
    },
    /// Answer to [`Request::GetState`].
    State { readings: Vec<Reading> },
    /// Answer to [`Request::SetState`].
    StateSet {
        capability: String,
        applied: Value,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        note: Option<String>,
    },
    /// Answer to [`Request::SubscribeEvent`].
    Subscribed { subscription: u32, interval_ms: u64 },
    /// Answer to [`Request::UnsubscribeEvent`].
    Unsubscribed { subscription: u32 },
    /// An asynchronous event pushed by the device.
    Event { readings: Vec<Reading> },
    /// Answer to [`Request::GetFirmwareInfo`].
    FirmwareInfo {
        firmware: crate::descriptor::FirmwareInfo,
    },
    /// Answer to [`Request::EnterBootloader`].
    Bootloader {
        entered: bool,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        detail: Option<String>,
    },
    /// Answer to [`Request::UpdateFirmware`].
    FirmwareChunk { received: u32, total: u32 },
    /// Answer to [`Request::Ping`].
    Pong { nonce: u32, uptime_ms: u64 },
    /// Anything went wrong.
    Error {
        code: ProtocolErrorCode,
        message: String,
    },
}

impl Response {
    pub fn opcode(&self) -> &'static str {
        match self {
            Self::DeviceInfo { .. } => "DEVICE_INFO",
            Self::Capabilities { .. } => "CAPABILITIES",
            Self::State { .. } => "STATE",
            Self::StateSet { .. } => "STATE_SET",
            Self::Subscribed { .. } => "SUBSCRIBED",
            Self::Unsubscribed { .. } => "UNSUBSCRIBED",
            Self::Event { .. } => "EVENT",
            Self::FirmwareInfo { .. } => "FIRMWARE_INFO",
            Self::Bootloader { .. } => "BOOTLOADER",
            Self::FirmwareChunk { .. } => "FIRMWARE_CHUNK",
            Self::Pong { .. } => "PONG",
            Self::Error { .. } => "ERROR",
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }

    /// Turn a transport/parse failure into a wire error.
    pub fn error(code: ProtocolErrorCode, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
        }
    }

    pub fn encode(&self) -> ohm_core::Result<Vec<u8>> {
        let json = serde_json::to_vec(self)?;
        crate::framing::encode(&json)
    }

    pub fn decode(payload: &[u8]) -> ohm_core::Result<Self> {
        Ok(serde_json::from_slice(payload)?)
    }
}

/// Machine readable failure codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProtocolErrorCode {
    /// The opcode is not known to this firmware.
    UnknownOpcode,
    /// The operation exists but is not supported on this revision.
    Unsupported,
    /// The capability does not exist.
    UnknownCapability,
    /// The capability is read-only.
    NotWritable,
    /// The value is outside the supported range.
    InvalidValue,
    /// The device is busy (for example while updating firmware).
    Busy,
    /// The request is rejected because the device is in bootloader mode.
    BootloaderMode,
    /// A firmware image is invalid.
    FirmwareRejected,
    /// Framing or checksum failure.
    Framing,
    /// The request needs authorisation the host does not have.
    Unauthorized,
    /// Anything else.
    Internal,
}

impl ProtocolErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnknownOpcode => "UNKNOWN_OPCODE",
            Self::Unsupported => "UNSUPPORTED",
            Self::UnknownCapability => "UNKNOWN_CAPABILITY",
            Self::NotWritable => "NOT_WRITABLE",
            Self::InvalidValue => "INVALID_VALUE",
            Self::Busy => "BUSY",
            Self::BootloaderMode => "BOOTLOADER_MODE",
            Self::FirmwareRejected => "FIRMWARE_REJECTED",
            Self::Framing => "FRAMING",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Internal => "INTERNAL",
        }
    }

    /// How the runtime should report this to the user.
    pub fn unavailable_reason(&self) -> ohm_device_model::UnavailableReason {
        use ohm_device_model::UnavailableReason as R;
        match self {
            Self::UnknownOpcode | Self::Unsupported => R::Unsupported,
            Self::UnknownCapability => R::HardwareLimitation,
            Self::NotWritable => R::VendorLimitation,
            Self::InvalidValue => R::HardwareLimitation,
            Self::Busy | Self::BootloaderMode => R::NotPresent,
            Self::FirmwareRejected | Self::Framing | Self::Internal => R::ReadError,
            Self::Unauthorized => R::PermissionDenied,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_negotiation() {
        assert!(is_compatible(0, 0));
        assert!(is_compatible(0, 1));
        assert!(!is_compatible(0, 2), "a newer device may use new messages");
        assert!(!is_compatible(1, 0), "a different major is incompatible");
        assert_eq!(protocol_version(), "0.1");
    }

    #[test]
    fn request_roundtrip_tagged_as_documented() {
        let request = Request::GetState {
            capability: Some("fan.rpm".into()),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["op"], "GET_STATE");
        assert_eq!(json["capability"], "fan.rpm");
        let back: Request = serde_json::from_value(json).unwrap();
        assert_eq!(back, request);

        let json = serde_json::to_value(Request::GetDeviceInfo).unwrap();
        assert_eq!(json["op"], "GET_DEVICE_INFO");
        assert_eq!(Request::GetCapabilities.opcode(), "GET_CAPABILITIES");
        assert_eq!(Request::GetFirmwareInfo.opcode(), "GET_FIRMWARE_INFO");
        assert!(
            Request::SetState {
                capability: "fan.speed_percent".into(),
                value: Value::Number(50.0)
            }
            .is_mutating()
        );
        assert!(!Request::Ping { nonce: 1 }.is_mutating());
    }

    #[test]
    fn framed_request_roundtrip() {
        let frame = Request::Ping { nonce: 42 }.encode().unwrap();
        let mut decoder = crate::framing::Decoder::new();
        let payloads = decoder.push(&frame);
        assert_eq!(payloads.len(), 1);
        let request = Request::decode(&payloads[0]).unwrap();
        assert_eq!(request, Request::Ping { nonce: 42 });
    }

    #[test]
    fn framed_response_roundtrip() {
        let response = Response::StateSet {
            capability: "fan.speed_percent".into(),
            applied: Value::Number(80.0),
            note: None,
        };
        let frame = response.encode().unwrap();
        let mut decoder = crate::framing::Decoder::new();
        let payloads = decoder.push(&frame);
        assert_eq!(Response::decode(&payloads[0]).unwrap(), response);
        assert!(!response.is_error());
        assert_eq!(response.opcode(), "STATE_SET");
    }

    #[test]
    fn errors_are_typed() {
        let response = Response::error(ProtocolErrorCode::NotWritable, "read-only channel");
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["op"], "ERROR");
        assert_eq!(json["code"], "NOT_WRITABLE");
        assert!(response.is_error());
        assert_eq!(
            ProtocolErrorCode::Unauthorized.unavailable_reason(),
            ohm_device_model::UnavailableReason::PermissionDenied
        );
        assert_eq!(
            ProtocolErrorCode::Unsupported.unavailable_reason(),
            ohm_device_model::UnavailableReason::Unsupported
        );
        assert_eq!(ProtocolErrorCode::Busy.as_str(), "BUSY");
    }

    #[test]
    fn malformed_payload_is_a_parse_error() {
        assert!(Request::decode(b"{not json").is_err());
        assert!(Request::decode(b"{\"op\":\"NOPE\"}").is_err());
    }
}
