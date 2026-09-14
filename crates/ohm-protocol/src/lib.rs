//! The Open Device Protocol (ODP).
//!
//! This crate is the *contract* between OpenHardwareOS and the hardware the
//! project intends to build (OpenHub, OpenFan, and later case displays, knobs
//! and AI decks). The MVP has no such hardware, so the crate exists to make
//! sure the architecture does not paint itself into a corner — and to be an
//! executable specification for the first firmware.
//!
//! # Why a protocol at all
//!
//! Everything else in the workspace talks about devices and capabilities. A
//! real device has to *say* what it is and what it can do, over a wire that is
//! cheap to implement on an MCU:
//!
//! ```text
//! USB connected
//!     ↓  GET_DEVICE_INFO     descriptor: vendor, product, device type, firmware
//!     ↓  GET_CAPABILITIES    fan.speed_percent, fan.rpm, temperature.core, ...
//!     ↓  GET_STATE           readings
//!     ↓  SET_STATE           validated write, acknowledged
//!     ↓  SUBSCRIBE_EVENT     change driven updates instead of polling
//!     ↓  GET_FIRMWARE_INFO / ENTER_BOOTLOADER / UPDATE_FIRMWARE
//!     ↓
//! Register Runtime Device -> UI appears automatically
//! ```
//!
//! # Transports
//!
//! USB HID and USB CDC/serial only — no custom physical bus. See
//! [`transport`]: [`transport::DeviceTransport`] is byte oriented, so a HID
//! implementation just delivers fixed size reports into the same
//! [`framing::Decoder`].
//!
//! # Frames
//!
//! ```text
//! ┌────────┬──────────────┬───────┬─────────────────┐
//! │ 0xAA55 │ length (u16) │ crc8  │ payload (JSON)  │
//! └────────┴──────────────┴───────┴─────────────────┘
//! ```
//!
//! # Status
//!
//! | Part | State |
//! |------|-------|
//! | framing + checksum | implemented |
//! | messages and descriptors | implemented |
//! | loopback + stream transports | implemented |
//! | mock OpenFan device (4 channels, local fallback, bootloader, firmware update) | implemented |
//! | real USB HID / CDC enumeration | roadmap (v0.4) |
//!
//! The mock device is wired into the desktop app behind Settings ->
//! Experimental, so the whole "plug in a device and it appears" flow can be
//! demonstrated today.

pub mod descriptor;
pub mod framing;
pub mod messages;
pub mod mock;
pub mod transport;

pub use descriptor::{CapabilityDescriptor, DeviceDescriptor, FirmwareInfo};
pub use framing::{Decoder, MAX_PAYLOAD, crc8, crc8_update};
pub use messages::{
    PROTOCOL_MAJOR, PROTOCOL_MINOR, ProtocolErrorCode, Request, Response, is_compatible,
    protocol_version,
};
pub use mock::{DEFAULT_FALLBACK_AFTER_MS, DEFAULT_FALLBACK_DUTY, MockOpenFan};
pub use transport::{
    DeviceTransport, LoopbackTransport, ProtocolDevice, StreamTransport, error_reason, handshake,
};
