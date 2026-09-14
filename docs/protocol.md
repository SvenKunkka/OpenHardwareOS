# OpenHardwareOS — Open Device Protocol (ODP) v0.1

ODP is the contract between OpenHardwareOS and the hardware the project intends to
build (OpenHub, OpenFan, and later case displays, knobs and AI decks). It lives in
`crates/ohm-protocol` and is implemented end to end against a simulated device, so
the "plug it in and it appears in the UI" flow can be demonstrated and tested today,
on any machine. Everything else in the workspace talks about devices and
capabilities, so a device has to *say* what it is and what it can do over a wire that
is cheap to implement on an MCU — writing OpenFan firmware means implementing the
descriptor and the message set, not writing a driver for us.

```text
USB connected
    ↓  GET_DEVICE_INFO → descriptor      ↓  SET_STATE → validated, acknowledged write
    ↓  GET_CAPABILITIES → capabilities   ↓  SUBSCRIBE_EVENT → pushed updates
    ↓  GET_STATE → readings              ↓  GET_FIRMWARE_INFO / ENTER_BOOTLOADER / UPDATE_FIRMWARE
Register runtime Device → UI appears automatically
```

---

## 1. Status

From the status table in `crates/ohm-protocol/src/lib.rs`:

| Part | State |
|---|---|
| framing + checksum (`framing.rs`) | implemented |
| messages and descriptors (`messages.rs`, `descriptor.rs`) | implemented |
| loopback + stream transports (`transport.rs`) | implemented |
| mock OpenFan device — 4 channels, local fallback, bootloader, firmware update (`mock.rs`) | implemented |
| real USB HID / CDC enumeration | roadmap (v0.4) |

`docs/roadmap.md` agrees and is the authority on dates: Open Device Protocol is
marked shipped in the v0.4 column, real USB HID/CDC enumeration and the OpenHub
hardware are roadmap in that same column, and OpenFan specification + simulation
ships in v0.5 with the hardware as roadmap. Its "Known gaps" section adds that no
manifest depends on a HID or serial crate.

## 2. Frames

```text
┌────────┬────────────────┬───────┬──────────────────────┐
│ 0xAA55 │ length (u16 LE)│ CRC-8 │ payload (JSON)       │
└────────┴────────────────┴───────┴──────────────────────┘
   MAGIC_0  MAGIC_1   HEADER_LEN = 5
```

| Constant | Value | Notes |
|---|---|---|
| `MAGIC_0`, `MAGIC_1` | `0xAA`, `0x55` | fixed two-byte delimiter |
| `HEADER_LEN` | 5 | magic (2) + length (2) + crc (1) |
| length | payload bytes, `u16` little-endian | **not** including the header |
| `MAX_PAYLOAD` | `8 * 1024` (8192) | 8 KiB ceiling so a corrupt length field cannot make the decoder buffer megabytes; `encode` refuses more, the decoder drops such a frame |
| CRC | CRC-8/ATM, polynomial `0x07`, init `0x00` | over the **payload only**, in `encode` and in the decoder |

`crc8(data)` and `crc8_update(seed, data)` implement it; the incremental form lets
firmware be checksummed chunk by chunk. Known-good values asserted in
`framing.rs`: `crc8(b"") == 0x00`, `crc8(b"123456789") == 0xF4`. A complete PING
request on the wire (payload `{"op":"PING","nonce":42}`, 24 bytes, CRC `0x5D`):

```text
AA 55 18 00 5D 7B 22 6F 70 22 3A 22 50 49 4E 47 22 2C 22 6E 6F 6E 63 65 22 3A 34 32 7D
└─ magic ─┘ └len─┘ crc  └──────────────── payload ────────────────────────┘
```

`Decoder` is an incremental state machine, which is what makes the same code work
for HID reports, CDC chunks and serial bytes: `push(&[u8]) -> Vec<Vec<u8>>` returns
every complete, checksum-verified payload, resynchronising on `0xAA 0x55`
(`resyncs()`) and counting a `dropped_frames()` for an impossible length
(> `MAX_PAYLOAD`) or a CRC mismatch; `buffered()`/`reset()` expose and clear partial
state. Fragmentation, coalescing and recovery after leading garbage are covered by
tests in `framing.rs`.

## 3. Messages

Requests (`Request`) and responses (`Response`) are internally tagged JSON:
`#[serde(tag = "op", rename_all = "SCREAMING_SNAKE_CASE")]`. JSON was chosen for
v0.1 because it is inspectable, diffable and trivial to write on an MCU; the module
doc notes a future firmware revision can move to a compact binary encoding behind
the same opcodes without changing the Rust side's shape.

### Requests

| Opcode | Fields | Semantics |
|---|---|---|
| `GET_DEVICE_INFO` | — | identify the device (handshake) |
| `GET_CAPABILITIES` | — | ask what the device can do |
| `GET_STATE` | `capability?` | read one capability, or everything when omitted |
| `SET_STATE` | `capability`, `value` | write one actuator |
| `SUBSCRIBE_EVENT` | `capability?`, `min_interval_ms?` | start streaming changes (default 1000 ms; the mock clamps to 100–60000) |
| `UNSUBSCRIBE_EVENT` | `subscription` | stop a subscription |
| `GET_FIRMWARE_INFO` | — | firmware identification for support/update flows |
| `ENTER_BOOTLOADER` | `confirm` | must be explicit |
| `UPDATE_FIRMWARE` | `offset`, `total`, `data_hex`, `image_crc8` | push a firmware chunk |
| `PING` | `nonce` | liveness/latency probe |

`Request::opcode()` returns the wire name; `is_mutating()` is true for `SET_STATE`,
`ENTER_BOOTLOADER`, `UPDATE_FIRMWARE` and both subscription messages.
`Request::encode()` frames `serde_json::to_vec(self)`, `Request::decode(payload)` is
a plain `serde_json::from_slice` — a malformed body is a parse error the caller must
handle. Exact payloads (optional fields omitted when `None`):

```json
{"op":"GET_DEVICE_INFO"}
{"op":"GET_CAPABILITIES"}
{"op":"GET_STATE"}
{"op":"GET_STATE","capability":"fan.rpm"}
{"op":"SET_STATE","capability":"fan.speed_percent","value":60.0}
{"op":"SUBSCRIBE_EVENT","min_interval_ms":500}
{"op":"UNSUBSCRIBE_EVENT","subscription":1}
{"op":"GET_FIRMWARE_INFO"}
{"op":"ENTER_BOOTLOADER","confirm":true}
{"op":"UPDATE_FIRMWARE","offset":0,"total":22,"data_hex":"6F70656E66616E206669726D7761726520302E322E30","image_crc8":253}
{"op":"PING","nonce":42}
```

### Responses

| Opcode | Fields | Answers |
|---|---|---|
| `DEVICE_INFO` | `descriptor` | `GET_DEVICE_INFO` |
| `CAPABILITIES` | `capabilities: [CapabilityDescriptor]` | `GET_CAPABILITIES` |
| `STATE` | `readings: [Reading]` | `GET_STATE` |
| `STATE_SET` | `capability`, `applied`, `note?` | `SET_STATE` — `note` explains a clamp |
| `SUBSCRIBED` / `UNSUBSCRIBED` | `subscription` (+ `interval_ms` on subscribe) | the subscription messages |
| `EVENT` | `readings` | asynchronous push (declared; never sent today — see §13) |
| `FIRMWARE_INFO` | `firmware` | `GET_FIRMWARE_INFO` |
| `BOOTLOADER` | `entered`, `detail?` | `ENTER_BOOTLOADER` |
| `FIRMWARE_CHUNK` | `received`, `total` | `UPDATE_FIRMWARE` progress/completion |
| `PONG` | `nonce`, `uptime_ms` | `PING` |
| `ERROR` | `code`, `message` | anything that went wrong |

```json
{"op":"STATE","readings":[{"capability":"fan.speed_percent","status":"ok","value":40.0}]}
{"op":"STATE_SET","capability":"fan.speed_percent","applied":85.0}
{"op":"SUBSCRIBED","subscription":1,"interval_ms":500}
{"op":"UNSUBSCRIBED","subscription":1}
{"op":"BOOTLOADER","entered":true,"detail":"mock device entered bootloader mode"}
{"op":"FIRMWARE_CHUNK","received":22,"total":22}
{"op":"PONG","nonce":42,"uptime_ms":1500}
{"op":"ERROR","code":"NOT_WRITABLE","message":"`fan.rpm` is read-only"}
```

`readings` reuses the runtime's `Reading` type verbatim, so a device's state
serialises exactly like `DeviceState.readings` in `docs/device-model.md` §5
(`{"capability":..,"status":"ok","value":..}` or
`{"capability":..,"status":"unavailable","reason":..,"detail":..}`).

## 4. Errors

`ProtocolErrorCode` (`messages.rs`) is `SCREAMING_SNAKE_CASE` on the wire:

| Code | Meaning | Maps to `UnavailableReason` |
|---|---|---|
| `UNKNOWN_OPCODE` | opcode not known to this firmware | `unsupported` |
| `UNSUPPORTED` | operation exists but is not supported on this revision | `unsupported` |
| `UNKNOWN_CAPABILITY` | capability does not exist | `hardware_limitation` |
| `NOT_WRITABLE` | capability is read-only | `vendor_limitation` |
| `INVALID_VALUE` | outside the supported range | `hardware_limitation` |
| `BUSY` | device busy (e.g. during a firmware update) | `not_present` |
| `BOOTLOADER_MODE` | rejected because the device is in bootloader mode | `not_present` |
| `FIRMWARE_REJECTED` | invalid firmware image | `read_error` |
| `FRAMING` | framing/checksum failure | `read_error` |
| `UNAUTHORIZED` | needs authorisation the host does not have | `permission_denied` |
| `INTERNAL` | anything else | `read_error` |

The mapping is `ProtocolErrorCode::unavailable_reason()`, exposed to adapters as
`ohm_protocol::error_reason(code)`. It keeps the runtime's honesty rule intact: an
unsupported device must not surface as a red error. Note the asymmetry —
`UNSUPPORTED` is *not* a failure (`UnavailableReason::is_failure()` is false for it),
while `FRAMING`, `FIRMWARE_REJECTED` and `INTERNAL` are. `OpdAdapter::protocol_error`
is the worked example: it formats a `WriteRejected` whose detail starts with
`vendor_limitation:` or `permission_denied:` so the UI explains the limitation.

## 5. Version negotiation

`PROTOCOL_MAJOR = 0` (breaking changes only), `PROTOCOL_MINOR = 1` (additive),
`protocol_version() == "0.1"`.

`is_compatible(major, minor) == major == PROTOCOL_MAJOR && minor <= PROTOCOL_MINOR`:
same major, and the device must not be *newer* than the host, because a device that
only knows a newer protocol may use messages we do not implement. Asserted:
`(0,0)` and `(0,1)` compatible; `(0,2)` and `(1,0)` not.

The check runs in three places, so no path can skip it: `FirmwareInfo::is_compatible()`,
`handshake()` (rejects with `OhmError::Protocol` before the descriptor is used), and
`DeviceDescriptor::to_device()` (rejects again when building the runtime device).
The version also exists independently of the protocol crate as
`ohm_core::OPEN_DEVICE_PROTOCOL_VERSION = (0, 1)`, so an adapter can negotiate
without depending on `ohm-protocol`.

## 6. Descriptors → runtime device

`DeviceDescriptor` is a separate type from the runtime `Device` on purpose: the
protocol is a contract with *firmware* and must be able to evolve (or gain a
compact encoding) without silently changing the runtime model. It carries
`vendor`, `product`, optional `model`/`serial`, `device_type`, `transport`,
`firmware: FirmwareInfo` and `capabilities: [CapabilityDescriptor]`.

`CapabilityDescriptor` mirrors `Capability` field for field (id, name, kind, unit,
readable, writable, min, max, step, values, description, safety_critical) and has
its own `validate(&Value) -> Result<f64, (ProtocolErrorCode, String)>` — the check a
real firmware should implement device-side: read-only → `NOT_WRITABLE`;
enumeration mismatch, non-numeric, non-finite or out-of-range → `INVALID_VALUE`.

The descriptor of `MockOpenFan::new(4)` — i.e. exactly what the simulated device
answers to `GET_DEVICE_INFO` (wrapped here in the response; the descriptor itself
is the `descriptor` object, and `safety_critical: false` is always serialised):

```json
{
  "op": "DEVICE_INFO",
  "descriptor": {
    "vendor": "OpenHardwareOS",
    "product": "OpenFan 4",
    "model": "OHOS-OPENFAN-4",
    "serial": "OF4-SIM-0001",
    "device_type": "fan",
    "transport": "usb_hid",
    "firmware": {
      "version": "0.1.0-sim", "build": "mock", "git": "0000000",
      "protocol_major": 0, "protocol_minor": 1
    },
    "capabilities": [
      { "id": "fan.speed_percent", "name": "Fan Speed", "kind": "actuator",
        "unit": "percent", "readable": true, "writable": true, "min": 0.0, "max": 100.0,
        "description": "PWM duty applied to every channel of the group",
        "safety_critical": false },
      { "id": "fan.rpm", "name": "Fan RPM", "kind": "sensor", "unit": "rpm",
        "readable": true, "writable": false,
        "description": "Tachometer reading of channel 1", "safety_critical": false },
      { "id": "temperature.core", "name": "Onboard Temperature", "kind": "sensor",
        "unit": "celsius", "readable": true, "writable": false,
        "description": "Thermistor next to the fan header", "safety_critical": false },
      { "id": "status.message", "name": "Firmware Version", "kind": "info",
        "unit": "text", "readable": true, "writable": false,
        "description": "Reported by GET_FIRMWARE_INFO", "safety_critical": false }
    ]
  }
}
```

Conversion rules (`DeviceDescriptor::to_device(adapter, index)`):

* refused when `capabilities` is empty or `firmware` is incompatible;
* `DeviceId` is `DeviceId::compose(device_type, qualifier, index)`, where the
  qualifier is `vendor-serial` (or `vendor-product` with no serial), lowercased
  with non-alphanumerics replaced by `_`. The 4-channel mock registers as
  `fan.openhardwareos_of4_0001.0`, and the same descriptor always yields the same id;
* the result is `Device::validate()`d before being returned;
* metadata carries `protocol_version` and `firmware`, plus `model`, `serial` and
  `firmware_git` when present — how the UI shows a firmware version without a
  vendor-specific code path.

Note the naming difference from the runtime model: the device type field is `"type"`
in `Device` but `"device_type"` in `DeviceDescriptor` (different types, see above).

## 7. Transports

`DeviceTransport` (`transport.rs`) is deliberately byte oriented:

```rust
fn send_frame(&mut self, frame: &[u8]) -> Result<()>;
fn receive_frame(&mut self) -> Result<Option<Vec<u8>>>;   // Ok(None) = nothing arrived
fn describe(&self) -> String;
fn call(&mut self, request: Request) -> Result<Response>;  // provided: encode, send, receive, decode
```

`call` frames the request, sends it and decodes the reply, returning
`OhmError::Protocol("{describe} did not answer {opcode}")` when the transport yields
`Ok(None)`. Because the interface moves bytes, not messages, chunking and report
boundaries are the transport's business and framing stays in one place.

### `LoopbackTransport` — implemented

An in-process `ProtocolDevice` (the mock OpenFan, the test harness) reached through
the *real* framing and message codec: `send_frame` runs the frame through a
`Decoder`, dispatches each decoded payload to `ProtocolDevice::handle` and queues the
encoded responses (`receive_frame` decodes them again). A frame that fails to decode
returns an error and never reaches the device (`calls()` stays at 0); a payload that
is not valid JSON produces a wire `ERROR{code:"FRAMING"}` instead of a panic.
`calls()`, `device()` and `device_mut()` serve tests and the UI's simulator panel
(`OpdAdapter::tick`, `duty`, `fallback_engaged`).

### `StreamTransport` — implemented

`StreamTransport<R: Read + Send, W: Write + Send>`: `send_frame` writes and flushes;
`receive_frame` returns a queued payload if ready, otherwise reads up to 1024 bytes
into the `Decoder`. A read of 0 bytes returns `Ok(None)` — exactly what a
disconnected USB device looks like, asserted by `stream_transport_reports_eof`.
`dropped_frames()` exposes the decoder's counter.

### What a USB HID transport must do to plug in

No HID or serial crate is a dependency today (`crates/ohm-protocol/Cargo.toml`
lists only `ohm-core`, `ohm-device-model`, `serde`, `serde_json`, `thiserror`,
`tracing`), and no code opens a real USB device — **enumeration is roadmap, not
implemented**. A HID transport is a new `impl DeviceTransport`:

1. **Enumerate** candidates by VID/PID and usage page, keeping each device's path
   (a HID path is the stable handle, not a bus index).
2. **`send_frame(&[u8])`** — write the framed bytes as one or more *output reports*,
   padding to the report size when the device needs fixed-size reports. Framing is
   length-prefixed, so padding is harmless: the receiver resynchronises on `0xAA55`.
3. **`receive_frame()`** — read input reports into a per-device `Decoder` and return
   the first complete payload (queue the rest); `Ok(None)` when nothing is pending
   rather than blocking, and `Ok(None)`/an error on disconnect so the runtime marks
   the device offline instead of hanging.
4. **`describe()`** — a human string such as `hid://vid:pid#path`, used in logs and
   error messages.
5. **Register it** like the mock adapter does: `handshake()` → `to_device()` → return
   the `Device` from `discover()` (full recipe in §11).

CDC/serial is simpler: `StreamTransport` already implements `DeviceTransport` over
any `Read + Write` pair, so a serial port is a constructor, not a new transport.

## 8. The plug-in sequence, as exercised today

`adapters/open-protocol` (`OpdAdapter`) is the proof that the protocol is real:
`probe()` and `discover()` both run `handshake_and_discover()` (handshake +
`to_device`), `read_state()` sends `GET_STATE`, `write()` sends `SET_STATE`, and it is
registered with `write_requires_admin: false` — an ODP device needs no elevation,
which is one of the reasons for having our own hardware. The exchange the tests drive
(`crates/ohm-protocol/src/mock.rs` — `full_plug_in_flow_works_over_the_wire`;
`tests/tests/protocol_flow.rs` — `the_plug_in_protocol_exchange_is_complete`,
`an_opd_device_is_registered_like_any_other`):

| # | Host sends | Device answers | Result |
|---|---|---|---|
| 1 | `GET_DEVICE_INFO` | `DEVICE_INFO` | `handshake()` returns the descriptor, version checked; `to_device()` builds `fan.openhardwareos_of4_0001.0` |
| 2 | `GET_CAPABILITIES` | `CAPABILITIES` (4) | the same list the descriptor carried |
| 3 | `GET_STATE` (no capability) | `STATE` (4 readings) | `fan.speed_percent` = 40.0, `fan.rpm` = 1040, `temperature.core` ≈ 32.0, `status.message` = `"0.1.0-sim"` |
| 4 | `SET_STATE` `fan.speed_percent` = 85.0 | `STATE_SET` `applied` 85.0 | the next `GET_STATE` for `fan.rpm` reads 1760 (400 + 1600 × duty/100) |
| 5 | `SUBSCRIBE_EVENT` (`min_interval_ms` 500) | `SUBSCRIBED` `{subscription:1, interval_ms:500}` | acknowledged; the subscription counter is incremented |
| 5b | `UNSUBSCRIBE_EVENT` `{subscription:1}` | `UNSUBSCRIBED` | counter decremented |
| 6 | `GET_FIRMWARE_INFO` / `PING` | `FIRMWARE_INFO` / `PONG` | firmware version/build/git/protocol; `uptime_ms` = simulated uptime |

Then the runtime side: `Runtime::refresh_devices` registers the device like any
other, the UI shows it with no code change, and a runtime write returns
`WriteStatus::Applied` (not `Simulated`) — `rules_can_target_a_protocol_device`
drives a GPU→OpenFan curve this way. The invalid-write path is proven too: 140 %
returns `value_out_of_range` and `fan.rpm` returns `capability_read_only`, both
*before* the device is contacted. One deliberate shortcut: the adapter reads
capabilities from the handshake descriptor and never sends `GET_CAPABILITIES` — a
device that answers `GET_DEVICE_INFO` completely makes the second round trip
unnecessary.

## 9. Device-side local fallback (required of official hardware)

The host can disappear — crashed process, suspended machine, pulled cable, wedged
automation engine. A device that simply holds the last duty it was told keeps a fan
wherever the host last wanted it, which is the wrong failure mode. `MockOpenFan`
therefore implements a local fallback:

* `DEFAULT_FALLBACK_DUTY = 70.0`, `DEFAULT_FALLBACK_AFTER_MS = 5000`;
* `tick(dt_ms)` advances the simulated clock; if no request has been handled for
  longer than `fallback_after_ms`, the device sets `fallback_engaged = true` and
  drives its own duty to `fallback_duty` (logged as a warning);
* **any** handled request counts as host contact (`host_spoke()`), which clears the
  engaged flag and hands control back;
* `set_local_fallback(duty, after_ms)` / `disable_local_fallback()` let tests
  exercise both the protection and the thermal model.

`OpdAdapter::shutdown` documents why the host has nothing to release for an ODP
device: *"The device keeps its own local fallback curve, so there is nothing to
release: if we stop talking it protects itself. This is the behaviour official
hardware is required to implement."*
`a_device_that_loses_its_host_keeps_itself_cool` asserts the full cycle (low duty →
silence → 85 % self-protection → host returns → control restored). It must be
device-side because every host-side mechanism (a supervisor loop, a fail-safe write,
`relinquish_on_exit`) needs a live, talking host; the only guarantee that survives
the host dying is the one in firmware.

## 10. Firmware update

Flow, exactly as `MockOpenFan::handle` implements it:

```text
GET_FIRMWARE_INFO   → FIRMWARE_INFO{version, build, git, protocol_major/minor}
ENTER_BOOTLOADER{confirm:false} → BOOTLOADER{entered:false, detail:"explicit confirmation required"}
ENTER_BOOTLOADER{confirm:true}  → BOOTLOADER{entered:true}
   (in bootloader mode only UPDATE_FIRMWARE and PING are accepted; anything else,
    including SET_STATE, answers ERROR{code:"BOOTLOADER_MODE"})
UPDATE_FIRMWARE{offset,total,data_hex,image_crc8}   ← repeated; offset must equal
    the bytes received so far
   ├─ total > MAX_FIRMWARE_BYTES (1 MiB) | bad hex     → ERROR{FIRMWARE_REJECTED}
   ├─ offset != firmware_received                     → ERROR{FIRMWARE_REJECTED}
   ├─ more chunks to come                             → FIRMWARE_CHUNK{received,total}
   └─ last chunk: crc8_update(chunks) vs image_crc8
        ├─ mismatch (counters reset) → ERROR{FIRMWARE_REJECTED}, computed vs expected
        └─ match → version := "0.1.0-sim+updated", bootloader exits, counters reset
```

Details that matter to a firmware author:

* **Chunked, incremental CRC.** The device keeps a running CRC-8 seeded by
  `crc8_update(seed, chunk)`, so it never buffers a whole image; the host sends the
  same value over the whole image as `image_crc8` on every chunk (the mock compares
  it only on the last one).
* **`total` is authoritative.** A chunk with a different `total` resets the CRC and
  the counters — the upload restarts. There is no abort message.
* **The hex payload keeps the frame JSON** at the cost of size (uppercase in the
  tests, either case accepted by `decode_hex`, with odd-length and parse checks).
  `MAX_PAYLOAD` (8 KiB) bounds one *frame* and hex doubles the image, so a 1 MiB
  image takes roughly 260 chunks.
* **What the mock refuses, on purpose:** unconfirmed bootloader entry, images over
  1 MiB, non-contiguous offsets, malformed hex, a CRC mismatch, and every control
  operation while in bootloader mode. It **does not** implement signature
  verification, hardware-revision checks, rollback protection or a recovery image —
  a production bootloader needs all four; the mock only fixes the *message*
  contract. `docs/roadmap.md` (v0.5) flags bricking on a failed update as the main
  hardware risk, with a bootloader that "always enumerates and never requires a
  valid application image" as the mitigation.
* `MAX_FIRMWARE_BYTES` is `pub` in `crates/ohm-protocol/src/mock.rs` but **not**
  re-exported from the crate root (which exports `MockOpenFan`,
  `DEFAULT_FALLBACK_AFTER_MS`, `DEFAULT_FALLBACK_DUTY`). On success the mock bumps
  its version string; a real device would reboot and re-enumerate, which the host
  notices through its normal discovery cycle — there is no "reconnect" message.

## 11. Writing an adapter for a real ODP device

1. **Transport.** Implement `DeviceTransport` for your bus (HID checklist: §7); for
   serial/CDC construct a `StreamTransport` over the port's reader and writer.
2. **Handshake.** `handshake(&mut transport)?` sends `GET_DEVICE_INFO` and checks the
   version. Map failures in `probe()` to `AdapterStatus::unavailable(reason, detail)`
   — `ohm_protocol::error_reason(code)` — so discovery never loses *why*.
3. **Device.** `descriptor.to_device(&adapter_id, index)`, returned from
   `discover()`; ids derive from the descriptor, so keep the index stable per boot.
4. **Read.** Send `GET_STATE` and build a `DeviceState`. For every declared capability
   the device did not report, insert `Reading::unavailable(..,
   UnavailableReason::Unsupported, detail)` rather than omitting it —
   `OpdAdapter::read_state` does this so the UI shows a limitation, not a blank.
5. **Write.** `capability.validate(value)?` then `capability.clamp(..)` (the runtime
   already did both; the adapter is the last line of defence), then `SET_STATE`.
   Convert `Response::Error { code, message }` through `code.unavailable_reason()`
   into `OhmError::WriteRejected` (see `OpdAdapter::protocol_error`). Never report
   `Applied` when the device answered an error.
6. **Shutdown.** With a device-side fallback there is nothing to release; otherwise
   release the outputs here.
7. **Registration.** Add it to `crates/adapters` (`AdapterOptions`, `build_adapters`,
   `adapter_catalogue`), declare `AdapterCapabilities`, set `with_hotplug()`.
8. **Tests.** Copy `tests/tests/protocol_flow.rs`: a real `Runtime`, the real adapter,
   and `MockOpenFan` behind a `LoopbackTransport` for protocol assertions.

## 12. Licensing and compatibility

* **No vendor SDK is involved.** USB HID and USB CDC/serial are standard buses
  enumerated by the OS, and the protocol on top is ours; nothing in
  `crates/ohm-protocol` links or loads a vendor library.
* **The blockers in `docs/research.md` §7.3 do not apply here**: AMD ADL/ADLX
  (proprietary EULA, cannot appear in an Apache-2.0 tree), redistributing NVIDIA's
  `nvml.dll`, and bundling PawnIO (`GPL-2.0-or-later`; detect and deep-link).
* **Firmware must not require redistributing proprietary binaries.** A device that
  only works with a vendor's closed SDK, or that needs us to ship a vendor blob,
  cannot be supported: the adapter stays Apache-2.0 and dependency-free, the posture
  of "ship no NVIDIA binary" (`docs/research.md` §3.4). Firmware sources should be
  permissively licensed so users can rebuild what they own.
* **Why this is the interesting hardware class.** `docs/research.md` §2.2 notes USB
  fan-controller devices are the most reliable write path because *the device is
  designed to be controlled*, unlike SuperIO PWM where board firmware may fight back.
  An ODP device is that property, standardised. Engineering rationale, not legal
  advice; `docs/research.md` §7 has the licence citations.

## 13. Not implemented (roadmap)

| Item | Evidence |
|---|---|
| Real USB HID / CDC enumeration | status table in `crates/ohm-protocol/src/lib.rs` ("roadmap (v0.4)"); `docs/roadmap.md` (no manifest depends on a HID/serial crate); `crates/ohm-protocol/Cargo.toml` |
| `Response::Event` is never sent | the variant is declared and subscriptions are answered with `SUBSCRIBED`/`UNSUBSCRIBED` plus a counter, but nothing constructs `Response::Event`: there is no push path and the runtime polls instead |
| Firmware signature / revision / rollback checks | `MockOpenFan` validates size, offsets, hex and CRC only |
| Firmware update abort/resume | no abort message: recovery needs a new `total` (which resets the accumulator) or a reboot into the bootloader |
| Per-capability subscription state | only a `subscriptions: u32` counter exists; capability and interval are logged, not stored |
| A host-side firmware update command | `OpdAdapter` exposes no update method and no Tauri/CLI command drives `ENTER_BOOTLOADER`/`UPDATE_FIRMWARE`; only tests and the mock drive it |
| ODP devices beyond fan/pump controllers (display, knob, deck) | named as future hardware classes in `crates/ohm-protocol/src/lib.rs`; no descriptors or capabilities exist |
