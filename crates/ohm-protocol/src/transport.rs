//! Transports: how frames actually reach a device.
//!
//! The MVP ships two implementations:
//!
//! * [`LoopbackTransport`] — an in-process device, used by the mock OpenFan and
//!   by the test-suite. It exercises the real framing and message codec, so the
//!   protocol is genuinely covered by tests rather than stubbed out.
//! * [`StreamTransport`] — a byte stream (USB CDC, serial), which is the shape a
//!   real OpenHub/OpenFan will use. USB HID plugs in here as well: a HID
//!   transport just has to deliver fixed size reports to this same interface.
//!
//! Device discovery is *not* the transport's job: an adapter enumerates
//! transports (HID/CDC devices on the bus), performs the handshake
//! ([`handshake`]) and registers what it finds.

use std::collections::VecDeque;
use std::io::{Read, Write};

use ohm_core::{OhmError, Result};
use ohm_device_model::UnavailableReason;

use crate::descriptor::DeviceDescriptor;
use crate::framing::Decoder;
use crate::messages::{Request, Response};

/// A device side implementation of the protocol.
pub trait ProtocolDevice: Send {
    /// Handle one request. Implementations must never panic and must answer
    /// with [`Response::Error`] for anything they cannot do.
    fn handle(&mut self, request: Request) -> Response;

    /// Announcement used during the handshake.
    fn descriptor(&self) -> &DeviceDescriptor;
}

/// A byte oriented link to one device.
pub trait DeviceTransport: Send {
    /// Send a complete, already framed message.
    fn send_frame(&mut self, frame: &[u8]) -> Result<()>;

    /// Receive one framed message, or `Ok(None)` when nothing arrived (the
    /// caller decides how long to wait by configuring the underlying link).
    fn receive_frame(&mut self) -> Result<Option<Vec<u8>>>;

    /// Human readable identification for logs and the UI.
    fn describe(&self) -> String;

    /// Send a request and decode the response.
    fn call(&mut self, request: Request) -> Result<Response> {
        let frame = request.encode()?;
        self.send_frame(&frame)?;
        match self.receive_frame()? {
            Some(payload) => Response::decode(&payload),
            None => Err(OhmError::Protocol(format!(
                "{} did not answer {}",
                self.describe(),
                request.opcode()
            ))),
        }
    }
}

/// An in-process device reached through the real framing code.
pub struct LoopbackTransport<D: ProtocolDevice> {
    device: D,
    decoder: Decoder,
    pending: VecDeque<Vec<u8>>,
    calls: u64,
}

impl<D: ProtocolDevice> std::fmt::Debug for LoopbackTransport<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopbackTransport")
            .field("device", &self.device.descriptor().product)
            .field("calls", &self.calls)
            .finish()
    }
}

impl<D: ProtocolDevice> LoopbackTransport<D> {
    pub fn new(device: D) -> Self {
        Self {
            device,
            decoder: Decoder::new(),
            pending: VecDeque::new(),
            calls: 0,
        }
    }

    /// Number of requests handled so far.
    pub fn calls(&self) -> u64 {
        self.calls
    }

    pub fn device(&self) -> &D {
        &self.device
    }

    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }
}

impl<D: ProtocolDevice> DeviceTransport for LoopbackTransport<D> {
    fn send_frame(&mut self, frame: &[u8]) -> Result<()> {
        let payloads = self.decoder.push(frame);
        if payloads.is_empty() {
            return Err(OhmError::Protocol(
                "the frame could not be decoded (checksum or length)".into(),
            ));
        }
        for payload in payloads {
            // A decode failure is reported on the wire, exactly like a real
            // device would: the host must be able to survive a bad request.
            let response = match Request::decode(&payload) {
                Ok(request) => self.device.handle(request),
                Err(err) => Response::error(
                    crate::ProtocolErrorCode::Framing,
                    format!("could not parse request: {err}"),
                ),
            };
            self.calls += 1;
            self.pending.push_back(response.encode()?);
        }
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<Vec<u8>>> {
        match self.pending.pop_front() {
            Some(frame) => {
                let payloads = self.decoder.push(&frame);
                match payloads.into_iter().next() {
                    Some(payload) => Ok(Some(payload)),
                    None => Err(OhmError::Protocol(
                        "the response frame was corrupt (a bug in this process)".into(),
                    )),
                }
            }
            None => Ok(None),
        }
    }

    fn describe(&self) -> String {
        format!("loopback({})", self.device.descriptor().product)
    }
}

/// A byte stream transport (USB CDC, serial).
///
/// Chunking is handled by [`Decoder`] on the way in and by the framing on the
/// way out, so a transport only has to move bytes.
pub struct StreamTransport<R: Read + Send, W: Write + Send> {
    reader: R,
    writer: W,
    decoder: Decoder,
    ready: VecDeque<Vec<u8>>,
    label: String,
}

impl<R: Read + Send, W: Write + Send> std::fmt::Debug for StreamTransport<R, W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamTransport")
            .field("label", &self.label)
            .field("dropped_frames", &self.decoder.dropped_frames())
            .finish()
    }
}

impl<R: Read + Send, W: Write + Send> StreamTransport<R, W> {
    pub fn new(reader: R, writer: W, label: impl Into<String>) -> Self {
        Self {
            reader,
            writer,
            decoder: Decoder::new(),
            ready: VecDeque::new(),
            label: label.into(),
        }
    }

    /// Frames dropped so far (checksum or length failures).
    pub fn dropped_frames(&self) -> u64 {
        self.decoder.dropped_frames()
    }
}

impl<R: Read + Send, W: Write + Send> DeviceTransport for StreamTransport<R, W> {
    fn send_frame(&mut self, frame: &[u8]) -> Result<()> {
        self.writer
            .write_all(frame)
            .map_err(|e| OhmError::Protocol(format!("write failed on {}: {e}", self.label)))?;
        self.writer
            .flush()
            .map_err(|e| OhmError::Protocol(format!("flush failed on {}: {e}", self.label)))?;
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<Vec<u8>>> {
        if let Some(payload) = self.ready.pop_front() {
            return Ok(Some(payload));
        }
        let mut buffer = [0u8; 1024];
        let read = self
            .reader
            .read(&mut buffer)
            .map_err(|e| OhmError::Protocol(format!("read failed on {}: {e}", self.label)))?;
        if read == 0 {
            return Ok(None);
        }
        self.ready.extend(self.decoder.push(&buffer[..read]));
        Ok(self.ready.pop_front())
    }

    fn describe(&self) -> String {
        self.label.clone()
    }
}

/// Perform the protocol handshake: identify the device and check compatibility.
pub fn handshake(transport: &mut dyn DeviceTransport) -> Result<DeviceDescriptor> {
    match transport.call(Request::GetDeviceInfo)? {
        Response::DeviceInfo { descriptor } => {
            if !descriptor.firmware.is_compatible() {
                return Err(OhmError::Protocol(format!(
                    "{} speaks protocol {}, this host implements {}",
                    descriptor.product,
                    descriptor.firmware.protocol_version(),
                    crate::messages::protocol_version()
                )));
            }
            tracing::info!(
                transport = transport.describe(),
                device = descriptor.summary(),
                "open device protocol handshake complete"
            );
            Ok(descriptor)
        }
        Response::Error { code, message } => Err(OhmError::Protocol(format!(
            "device refused the handshake ({}): {message}",
            code.as_str()
        ))),
        other => Err(OhmError::Protocol(format!(
            "unexpected answer to GET_DEVICE_INFO: {}",
            other.opcode()
        ))),
    }
}

/// Map a protocol error response onto the runtime's vocabulary, so an
/// unsupported device shows up as `unsupported` instead of a red error.
pub fn error_reason(code: crate::ProtocolErrorCode) -> UnavailableReason {
    code.unavailable_reason()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{CapabilityDescriptor, DeviceDescriptor, FirmwareInfo};
    use crate::messages::ProtocolErrorCode;
    use crate::mock::MockOpenFan;
    use ohm_device_model::{DeviceType, Reading, Transport, Value};

    struct EchoDevice {
        descriptor: DeviceDescriptor,
    }

    impl ProtocolDevice for EchoDevice {
        fn handle(&mut self, request: Request) -> Response {
            match request {
                Request::GetDeviceInfo => Response::DeviceInfo {
                    descriptor: self.descriptor.clone(),
                },
                Request::Ping { nonce } => Response::Pong {
                    nonce,
                    uptime_ms: 1234,
                },
                _ => Response::error(ProtocolErrorCode::UnknownOpcode, "echo only"),
            }
        }

        fn descriptor(&self) -> &DeviceDescriptor {
            &self.descriptor
        }
    }

    fn echo() -> EchoDevice {
        EchoDevice {
            descriptor: DeviceDescriptor::new("Test", "Echo", DeviceType::Fan, Transport::UsbCdc)
                .with_firmware(FirmwareInfo::new("1.0.0"))
                .with_capabilities([CapabilityDescriptor::sensor(
                    "fan.rpm",
                    "Fan",
                    ohm_device_model::Unit::Rpm,
                )]),
        }
    }

    #[test]
    fn loopback_roundtrip() {
        let mut transport = LoopbackTransport::new(echo());
        let response = transport.call(Request::Ping { nonce: 7 }).unwrap();
        assert_eq!(
            response,
            Response::Pong {
                nonce: 7,
                uptime_ms: 1234
            }
        );
        assert_eq!(transport.calls(), 1);
        assert!(transport.describe().contains("Echo"));
    }

    #[test]
    fn handshake_reads_the_descriptor() {
        let mut transport = LoopbackTransport::new(echo());
        let descriptor = handshake(&mut transport).unwrap();
        assert_eq!(descriptor.product, "Echo");
        assert_eq!(descriptor.firmware.version, "1.0.0");
    }

    #[test]
    fn handshake_refuses_an_incompatible_device() {
        let mut device = echo();
        device.descriptor.firmware.protocol_major = 9;
        let mut transport = LoopbackTransport::new(device);
        let err = handshake(&mut transport).unwrap_err();
        assert!(err.to_string().contains("protocol"), "{err}");
    }

    #[test]
    fn a_bad_request_is_reported_on_the_wire() {
        let mut transport = LoopbackTransport::new(echo());
        // A frame with a valid checksum but an unknown opcode.
        let json = br#"{"op":"NOT_A_THING"}"#;
        let frame = crate::framing::encode(json).unwrap();
        transport.send_frame(&frame).unwrap();
        let payload = transport.receive_frame().unwrap().unwrap();
        let response = Response::decode(&payload).unwrap();
        assert!(matches!(
            response,
            Response::Error {
                code: ProtocolErrorCode::Framing,
                ..
            }
        ));
    }

    #[test]
    fn a_corrupt_frame_never_reaches_the_device() {
        let mut transport = LoopbackTransport::new(echo());
        let mut frame = crate::framing::encode(br#"{"op":"PING","nonce":1}"#).unwrap();
        let last = frame.len() - 1;
        frame[last] ^= 0xFF;
        assert!(transport.send_frame(&frame).is_err());
        assert_eq!(transport.calls(), 0);
    }

    #[test]
    fn stream_transport_moves_bytes() {
        // A pipe pair stands in for a USB CDC link.
        let (client_reader, mut device_writer) = os_pipe();
        let (mut device_reader, client_writer) = os_pipe();

        let mut transport = StreamTransport::new(client_reader, client_writer, "stdin");

        // A device thread answers one PING.
        let handle = std::thread::spawn(move || {
            let mut decoder = Decoder::new();
            let mut buffer = [0u8; 256];
            let read = device_reader.read(&mut buffer).unwrap();
            let payloads = decoder.push(&buffer[..read]);
            let request = Request::decode(&payloads[0]).unwrap();
            assert_eq!(request, Request::Ping { nonce: 99 });
            let response = Response::Pong {
                nonce: 99,
                uptime_ms: 42,
            };
            device_writer
                .write_all(&response.encode().unwrap())
                .unwrap();
            device_writer.flush().unwrap();
        });

        let response = transport.call(Request::Ping { nonce: 99 }).unwrap();
        handle.join().unwrap();
        assert_eq!(
            response,
            Response::Pong {
                nonce: 99,
                uptime_ms: 42
            }
        );
        assert_eq!(transport.describe(), "stdin");
        assert_eq!(transport.dropped_frames(), 0);
    }

    #[test]
    fn stream_transport_reports_eof() {
        let (reader, writer) = os_pipe();
        // Closing the write end is what a disconnected USB device looks like.
        drop(writer);
        let (_keep_alive, out) = os_pipe();
        let mut transport = StreamTransport::new(reader, out, "unplugged");
        assert!(transport.receive_frame().unwrap().is_none());
    }

    /// Minimal in-process pipe, so the transport test needs no extra dependency.
    fn os_pipe() -> (std::io::PipeReader, std::io::PipeWriter) {
        std::io::pipe().expect("pipe")
    }

    #[test]
    fn error_reasons_map_to_runtime_vocabulary() {
        assert_eq!(
            error_reason(ProtocolErrorCode::NotWritable),
            UnavailableReason::VendorLimitation
        );
        assert_eq!(
            error_reason(ProtocolErrorCode::Unauthorized),
            UnavailableReason::PermissionDenied
        );
    }

    #[test]
    fn mock_device_is_reachable_through_the_transport() {
        let mut transport = LoopbackTransport::new(MockOpenFan::new(4));
        let response = transport
            .call(Request::GetState { capability: None })
            .unwrap();
        match response {
            Response::State { readings } => {
                assert!(!readings.is_empty());
                assert!(readings.iter().all(|r: &Reading| r.is_ok()));
            }
            other => panic!("unexpected {other:?}"),
        }
        let response = transport
            .call(Request::SetState {
                capability: "fan.speed_percent".into(),
                value: Value::Number(60.0),
            })
            .unwrap();
        assert!(matches!(response, Response::StateSet { .. }));
    }
}
