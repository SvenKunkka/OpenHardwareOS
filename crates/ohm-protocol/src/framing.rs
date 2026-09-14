//! Framing for the Open Device Protocol.
//!
//! ```text
//! ┌────────┬──────────────┬───────┬─────────────────┐
//! │ 0xAA55 │ length (u16) │ crc8  │ payload (JSON)  │
//! └────────┴──────────────┴───────┴─────────────────┘
//! ```
//!
//! A fixed two byte magic plus a length prefix is enough for the transports the
//! MVP targets (USB CDC, USB HID with a fixed report size, plain serial): the
//! decoder is a small state machine that can resynchronise after a truncated or
//! corrupt frame, which is exactly what happens on a hot-plugged USB device.
//!
//! The checksum is CRC-8 (polynomial `0x07`) over the payload, so a corrupt
//! frame is detected rather than being fed to the JSON parser.

use ohm_core::{OhmError, Result};

/// Frame delimiter, first byte.
pub const MAGIC_0: u8 = 0xAA;
/// Frame delimiter, second byte.
pub const MAGIC_1: u8 = 0x55;
/// Header size: magic (2) + length (2) + crc (1).
pub const HEADER_LEN: usize = 5;
/// Largest payload we accept.
///
/// Protocol messages are small JSON documents; an 8 KiB ceiling keeps a
/// corrupted or hostile length field from making the decoder buffer megabytes.
pub const MAX_PAYLOAD: usize = 8 * 1024;

/// CRC-8/ATM (poly `0x07`, init `0x00`).
pub fn crc8(data: &[u8]) -> u8 {
    crc8_update(0, data)
}

/// Continue a CRC-8 computation over another chunk, so a firmware image can be
/// checksummed while it is being uploaded chunk by chunk.
pub fn crc8_update(seed: u8, data: &[u8]) -> u8 {
    let mut crc = seed;
    for byte in data {
        crc ^= *byte;
        for _ in 0..8 {
            if crc & 0x80 != 0 {
                crc = (crc << 1) ^ 0x07;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// Wrap a payload in a frame.
pub fn encode(payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > MAX_PAYLOAD {
        return Err(OhmError::Protocol(format!(
            "payload of {} bytes exceeds the {MAX_PAYLOAD} byte limit",
            payload.len()
        )));
    }
    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.push(MAGIC_0);
    frame.push(MAGIC_1);
    frame.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    frame.push(crc8(payload));
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Incremental frame decoder.
///
/// Feed it whatever the transport delivered; it yields complete, checksum
/// verified payloads and silently drops garbage (counting it for diagnostics).
#[derive(Debug, Default)]
pub struct Decoder {
    buffer: Vec<u8>,
    dropped_frames: u64,
    resyncs: u64,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Frames discarded because of a bad checksum or an impossible length.
    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames
    }

    /// Times the decoder had to skip bytes to find the next magic.
    pub fn resyncs(&self) -> u64 {
        self.resyncs
    }

    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }

    /// Append received bytes and return every complete payload.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            // Resynchronise on the magic.
            while self.buffer.len() >= 2
                && !(self.buffer[0] == MAGIC_0 && self.buffer[1] == MAGIC_1)
            {
                self.buffer.remove(0);
                self.resyncs += 1;
            }
            if self.buffer.len() < HEADER_LEN {
                break;
            }

            let length = u16::from_le_bytes([self.buffer[2], self.buffer[3]]) as usize;
            if length > MAX_PAYLOAD {
                // Impossible length: drop the magic and resynchronise.
                self.buffer.drain(..2);
                self.dropped_frames += 1;
                continue;
            }
            if self.buffer.len() < HEADER_LEN + length {
                break;
            }

            let crc = self.buffer[4];
            let payload = self.buffer[HEADER_LEN..HEADER_LEN + length].to_vec();
            if crc8(&payload) != crc {
                self.buffer.drain(..2);
                self.dropped_frames += 1;
                continue;
            }
            self.buffer.drain(..HEADER_LEN + length);
            out.push(payload);
        }
        out
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_single_frame() {
        let payload = b"{\"op\":\"PING\"}";
        let frame = encode(payload).unwrap();
        assert_eq!(frame[0], MAGIC_0);
        assert_eq!(frame[1], MAGIC_1);
        assert_eq!(frame.len(), HEADER_LEN + payload.len());

        let mut decoder = Decoder::new();
        let frames = decoder.push(&frame);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], payload);
        assert_eq!(decoder.buffered(), 0);
    }

    #[test]
    fn handles_fragmented_delivery() {
        let frame = encode(b"hello device").unwrap();
        let mut decoder = Decoder::new();
        let mut frames = Vec::new();
        for byte in &frame {
            frames.extend(decoder.push(&[*byte]));
        }
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], b"hello device");
    }

    #[test]
    fn handles_coalesced_delivery() {
        let mut stream = encode(b"first").unwrap();
        stream.extend(encode(b"second").unwrap());
        stream.extend(encode(b"third").unwrap());
        let mut decoder = Decoder::new();
        let frames = decoder.push(&stream);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[2], b"third");
    }

    #[test]
    fn resynchronises_after_garbage() {
        let mut stream = vec![0x00, 0xFF, 0x13];
        stream.extend(encode(b"real").unwrap());
        let mut decoder = Decoder::new();
        let frames = decoder.push(&stream);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], b"real");
        assert!(decoder.resyncs() >= 3);
    }

    #[test]
    fn rejects_a_corrupt_payload() {
        let mut frame = encode(b"important").unwrap();
        let last = frame.len() - 1;
        frame[last] ^= 0xFF;
        let mut decoder = Decoder::new();
        assert!(decoder.push(&frame).is_empty());
        assert_eq!(decoder.dropped_frames(), 1);
    }

    #[test]
    fn rejects_an_impossible_length() {
        let frame = vec![MAGIC_0, MAGIC_1, 0xFF, 0xFF, 0x00, 0x01];
        let mut decoder = Decoder::new();
        assert!(decoder.push(&frame).is_empty());
        assert_eq!(decoder.dropped_frames(), 1);
    }

    #[test]
    fn empty_payload_is_legal_and_checked() {
        let frame = encode(&[]).unwrap();
        let mut decoder = Decoder::new();
        let frames = decoder.push(&frame);
        assert_eq!(frames.len(), 1);
        assert!(frames[0].is_empty());
    }

    #[test]
    fn oversized_payload_is_refused() {
        let too_big = vec![0u8; MAX_PAYLOAD + 1];
        assert!(encode(&too_big).is_err());
    }

    #[test]
    fn crc_can_be_computed_incrementally() {
        let all = crc8(b"openfan firmware image");
        let first = crc8_update(0, b"openfan ");
        let second = crc8_update(first, b"firmware image");
        assert_eq!(all, second);
    }

    #[test]
    fn crc_is_stable() {
        // Known-good values for CRC-8/ATM.
        assert_eq!(crc8(b""), 0x00);
        assert_eq!(crc8(b"123456789"), 0xF4);
        assert_ne!(crc8(b"abc"), crc8(b"abd"));
    }

    #[test]
    fn reset_clears_partial_state() {
        let frame = encode(b"partial").unwrap();
        let mut decoder = Decoder::new();
        decoder.push(&frame[..3]);
        assert!(decoder.buffered() > 0);
        decoder.reset();
        assert_eq!(decoder.buffered(), 0);
        assert_eq!(decoder.push(&frame).len(), 1);
    }
}
