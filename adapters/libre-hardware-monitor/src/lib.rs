//! LibreHardwareMonitor integration.
//!
//! # Why LHM is the primary real-hardware provider
//!
//! Reading a motherboard's fan tachometers and writing its PWM channels needs
//! SuperIO/EC access through a kernel driver — no user-space library does this
//! on Windows. LibreHardwareMonitor (MPL-2.0) already owns that problem, exposes
//! a mature hardware model, and ships a signed driver. Re-implementing it would
//! be both wasteful and a licensing/signing minefield, so OpenHardwareOS *uses*
//! it and keeps its own device model on top.
//!
//! # Transports
//!
//! | Transport | State | Notes |
//! |-----------|-------|-------|
//! | HTTP JSON (`GET /data.json`) | **implemented** | Needs the LHM GUI running with the web server enabled (default port 8085). |
//! | Named pipe / stdio bridge to `LibreHardwareMonitorLib` | roadmap | Keeps MPL-2.0 file-level copyleft out-of-process and removes the GUI dependency; see `docs/decisions/0003-libre-hardware-monitor-integration.md`. |
//! | In-process .NET hosting | rejected | Would drag a .NET runtime and MPL-2.0 sources into our binary. |
//!
//! # What the user must do
//!
//! 1. Install and run LibreHardwareMonitor **as Administrator** (it needs the
//!    driver for SuperIO access).
//! 2. `Options -> Remote Web Server -> Run`.
//! 3. Leave the port at 8085 or set `adapter_settings.lhm.base_url` to match.
//!
//! When the server is unreachable the adapter reports itself `unavailable` with
//! those instructions instead of silently showing nothing — a monitoring app
//! that hides why it has no data is worse than one that says so.

pub mod lhm;
pub mod mapping;
pub mod web;

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use async_trait::async_trait;
use ohm_adapter_api::{
    AdapterCapabilities, AdapterInfo, AdapterStatus, HardwareAdapter, WriteOutcome,
};
use ohm_core::{CapabilityId, DeviceId, OhmError, Result};
use ohm_device_model::{Capability, Device, DeviceState, UnavailableReason, Value, caps};
use parking_lot::RwLock;

use lhm::LhmNode;
use mapping::{LhmMapping, map_tree, state_from_tree};
use web::{LhmConfig, LhmError, LhmWebClient};

pub use lhm::{LhmSensorKind, parse_tree};
pub use mapping::LhmMapping as LibreHardwareMapping;
pub use web::{DEFAULT_BASE_URL, DEFAULT_PORT, LhmConfig as WebConfig};

/// Adapter id, also the device id namespace.
pub const ADAPTER_ID: &str = "lhm";
/// How far a control channel's value may sit from what we asked for before the
/// write is treated as refused rather than applied. One percent covers the
/// rounding LHM does on a percentage channel.
pub const READ_BACK_TOLERANCE_PERCENT: f64 = 1.0;
/// Display name.
pub const ADAPTER_NAME: &str = "LibreHardwareMonitor";

/// OpenHardwareOS's view of a running LibreHardwareMonitor instance.
#[derive(Debug)]
pub struct LhmAdapter {
    client: LhmWebClient,
    /// Last mapping, refreshed on every successful fetch.
    mapping: RwLock<LhmMapping>,
    /// Last failure, surfaced through `probe()`.
    last_error: RwLock<Option<LhmError>>,
    /// `true` once a fetch has succeeded at least once.
    connected: AtomicBool,
    last_success_ms: AtomicI64,
}

impl LhmAdapter {
    /// Adapter talking to `config.base_url`.
    pub fn new(config: LhmConfig) -> Self {
        Self {
            client: LhmWebClient::new(config),
            mapping: RwLock::new(LhmMapping::default()),
            last_error: RwLock::new(None),
            connected: AtomicBool::new(false),
            last_success_ms: AtomicI64::new(0),
        }
    }

    /// Adapter configured from `settings.adapter_settings["lhm"]`.
    pub fn from_settings(value: Option<&serde_json::Value>) -> Self {
        Self::new(LhmConfig::from_json(value))
    }

    /// As a trait object, ready to register on a runtime.
    pub fn boxed(config: LhmConfig) -> Arc<dyn HardwareAdapter> {
        Arc::new(Self::new(config))
    }

    /// Base URL currently in use.
    pub fn base_url(&self) -> &str {
        self.client.base_url()
    }

    /// `true` when the last fetch succeeded.
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Current device mapping.
    pub fn mapping(&self) -> LhmMapping {
        self.mapping.read().clone()
    }

    /// Fetch the tree and re-map it.
    fn refresh(&self) -> std::result::Result<(LhmNode, LhmMapping), LhmError> {
        match self.client.fetch_tree() {
            Ok(tree) => {
                let mapping = map_tree(&tree);
                *self.mapping.write() = mapping.clone();
                *self.last_error.write() = None;
                self.connected.store(true, Ordering::Relaxed);
                self.last_success_ms
                    .store(ohm_core::now_ms(), Ordering::Relaxed);
                Ok((tree, mapping))
            }
            Err(error) => {
                self.connected.store(false, Ordering::Relaxed);
                let detail = error.detail(self.base_url());
                tracing::debug!(detail, "LibreHardwareMonitor fetch failed");
                *self.last_error.write() = Some(error.clone());
                Err(error)
            }
        }
    }

    fn unavailable_error(&self) -> OhmError {
        let error = self
            .last_error
            .read()
            .clone()
            .unwrap_or(LhmError::Unreachable("no successful fetch yet".into()));
        OhmError::AdapterUnavailable {
            adapter: ADAPTER_ID.to_string(),
            detail: error.detail(self.base_url()),
        }
    }
}

#[async_trait]
impl HardwareAdapter for LhmAdapter {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::new(ADAPTER_ID, ADAPTER_NAME, ADAPTER_ID)
            .with_description(
                "Reads CPU, GPU, storage, motherboard and chassis fan sensors from a running \
                 LibreHardwareMonitor instance, and drives its fan control channels. This is the \
                 only provider that can read and write motherboard fan headers.",
            )
            .with_homepage("https://github.com/LibreHardwareMonitor/LibreHardwareMonitor")
            .requires_admin()
            .with_capabilities(AdapterCapabilities {
                can_write: true,
                can_control_cooling: true,
                // Talking to the web server needs no elevation; *LHM* must be
                // elevated for the writes to reach the SuperIO.
                write_requires_admin: true,
                // `shutdown` sends `SetDefault` to every control channel it
                // knows, which is what hands the channel back to the firmware.
                hands_back_control_on_shutdown: true,
                poll_interval_ms: None,
                discovery_interval_ms: None,
            })
    }

    async fn probe(&self) -> AdapterStatus {
        match self.refresh() {
            Ok((_, mapping)) => {
                if mapping.devices.is_empty() {
                    return AdapterStatus::degraded(
                        self.id(),
                        UnavailableReason::HardwareLimitation,
                        "LibreHardwareMonitor is reachable but exposes no sensors; check that it \
                         is running as Administrator",
                    );
                }
                AdapterStatus::available(self.id(), mapping.devices.len())
            }
            Err(error) => {
                AdapterStatus::unavailable(self.id(), error.reason(), error.detail(self.base_url()))
            }
        }
    }

    async fn discover(&self) -> Result<Vec<Device>> {
        match self.refresh() {
            Ok((_, mapping)) => Ok(mapping.devices),
            Err(_) => Err(self.unavailable_error()),
        }
    }

    async fn read_state(&self, device: &Device) -> Result<DeviceState> {
        let (tree, mapping) = self.refresh().map_err(|_| self.unavailable_error())?;
        Ok(state_from_tree(&tree, &mapping, device, ohm_core::now_ms()))
    }

    async fn read_all(&self, devices: &[Device]) -> Vec<(DeviceId, Result<DeviceState>)> {
        // One HTTP request per cycle covers every device: LHM's tree is a
        // complete snapshot, so polling it once is both cheaper and more
        // consistent than polling per device.
        match self.refresh() {
            Ok((tree, mapping)) => {
                let now = ohm_core::now_ms();
                devices
                    .iter()
                    .map(|device| {
                        (
                            device.id.clone(),
                            Ok(state_from_tree(&tree, &mapping, device, now)),
                        )
                    })
                    .collect()
            }
            Err(_) => {
                let error = self.unavailable_error();
                let detail = error.to_string();
                devices
                    .iter()
                    .map(|device| {
                        (
                            device.id.clone(),
                            Ok(DeviceState::new(device.id.clone(), ohm_core::now_ms())
                                .offline(UnavailableReason::NotPresent, detail.clone())),
                        )
                    })
                    .collect()
            }
        }
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

        // Make sure the mapping is current before resolving the channel.
        let mapping = match self.refresh() {
            Ok((_, mapping)) => mapping,
            Err(_) => return Err(self.unavailable_error()),
        };
        let Some(sensor_id) = mapping.sensor_id(&device.id, &capability.id) else {
            return Err(OhmError::WriteRejected {
                device: device.id.to_string(),
                capability: capability.id.to_string(),
                detail: "LibreHardwareMonitor no longer exposes this control channel".to_string(),
            });
        };
        let sensor_id = sensor_id.to_string();

        if let Err(error) = self.client.set_sensor(&sensor_id, Some(applied)) {
            return Err(OhmError::WriteRejected {
                device: device.id.to_string(),
                capability: capability.id.to_string(),
                detail: error.detail(self.base_url()),
            });
        }

        // Reading the channel back is what separates "the request was accepted"
        // from "the fan is at this value". LHM answers a `Set` with the value it
        // *would* apply; only a `Get` reflects what the SuperIO actually holds,
        // and a channel that silently ignores writes is a real failure mode on
        // locked-down boards.
        match self.client.read_sensor(&sensor_id) {
            Ok(Some(read_back)) => {
                let delta = (read_back - applied).abs();
                if delta > READ_BACK_TOLERANCE_PERCENT {
                    return Err(OhmError::WriteRejected {
                        device: device.id.to_string(),
                        capability: capability.id.to_string(),
                        detail: format!(
                            "LibreHardwareMonitor accepted {applied:.1} % but the channel reports \
                             {read_back:.1} %. The value was not applied — the chip may be \
                             ignoring writes, or a vendor tool may be holding the controller."
                        ),
                    });
                }
                let mut outcome = WriteOutcome::applied(Value::Number(read_back));
                outcome.detail = Some(format!(
                    "channel set point read back as {read_back:.1} % (the fan's actual speed is a \
                     separate reading, not implied by this)"
                ));
                Ok(outcome)
            }
            // A channel that answers `N/A` holds no value we can trust, so the
            // honest result is "unknown", not "applied".
            Ok(None) => Ok(WriteOutcome::unconfirmed(format!(
                "LibreHardwareMonitor accepted the request, but the channel answers `N/A` when read \
                 back, so the set point could not be confirmed (it is neither known to be \
                 {applied:.1} % nor known to be something else)"
            ))),
            Err(error) => Ok(WriteOutcome::unconfirmed(format!(
                "LibreHardwareMonitor accepted the request, but reading the channel back failed \
                 ({}), so the set point could not be confirmed",
                error.detail(self.base_url())
            ))),
        }
    }

    /// Hand every control channel back to the firmware.
    ///
    /// This is what "relinquish on exit" means for real hardware: LHM's
    /// `SetDefault` gives the SuperIO's own fan curve back, so a machine left
    /// unattended cannot be stuck with whatever duty we last wrote.
    async fn shutdown(&self) -> Result<()> {
        let channels: Vec<(CapabilityId, String)> = {
            let mapping = self.mapping.read();
            mapping
                .sensors
                .values()
                .flat_map(|map| {
                    map.iter()
                        .filter(|(capability, _)| {
                            matches!(
                                capability.as_str(),
                                caps::FAN_SPEED_PERCENT | caps::PUMP_SPEED_PERCENT
                            )
                        })
                        .map(|(capability, id)| (capability.clone(), id.clone()))
                        .collect::<Vec<_>>()
                })
                .collect()
        };

        if channels.is_empty() {
            return Ok(());
        }

        let mut released = 0usize;
        let mut failed = 0usize;
        for (_capability, sensor_id) in &channels {
            match self.client.set_sensor(sensor_id, None) {
                Ok(()) => released += 1,
                Err(error) => {
                    failed += 1;
                    tracing::warn!(
                        sensor = %sensor_id,
                        detail = error.detail(self.base_url()),
                        "could not release a fan channel back to the firmware"
                    );
                }
            }
        }
        tracing::info!(
            released,
            failed,
            "released LibreHardwareMonitor control channels"
        );
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod fake_server;
#[cfg(test)]
mod tests {
    use super::*;
    use fake_server::FakeLhm;

    fn adapter(server: &FakeLhm) -> LhmAdapter {
        LhmAdapter::new(LhmConfig {
            base_url: server.base_url(),
            timeout_ms: 1_000,
            ..LhmConfig::default()
        })
    }

    #[tokio::test]
    async fn probe_and_discovery_work_against_a_live_server() {
        let server = FakeLhm::start();
        let adapter = adapter(&server);

        let status = adapter.probe().await;
        assert_eq!(status.state, ohm_adapter_api::AdapterState::Available);
        assert!(status.device_count >= 5, "{}", status.device_count);

        let devices = adapter.discover().await.unwrap();
        assert!(devices.iter().any(|d| d.id.as_str() == "cpu.lhm.0"));
        assert!(devices.iter().any(|d| d.id.as_str() == "gpu.lhm.1"));
        let fan = devices
            .iter()
            .find(|d| d.supports(caps::FAN_SPEED_PERCENT))
            .expect("a controllable fan");
        assert!(fan.is_controllable());
    }

    #[tokio::test]
    async fn readings_arrive_for_every_device() {
        let server = FakeLhm::start();
        let adapter = adapter(&server);
        let devices = adapter.discover().await.unwrap();
        let before = server.requests();
        let results = adapter.read_all(&devices).await;
        assert_eq!(results.len(), devices.len());
        assert!(results.iter().all(|(_, result)| result.is_ok()));
        assert_eq!(
            server.requests() - before,
            1,
            "the whole cycle must cost one HTTP request"
        );

        let gpu_state = results
            .iter()
            .find(|(id, _)| id.as_str() == "gpu.lhm.1")
            .map(|(_, state)| state.as_ref().unwrap().clone())
            .unwrap();
        assert_eq!(gpu_state.number(caps::TEMPERATURE_CORE), Some(76.0));
    }

    #[tokio::test]
    async fn writes_reach_the_control_channel() {
        let server = FakeLhm::start();
        let adapter = adapter(&server);
        let devices = adapter.discover().await.unwrap();
        let fan = devices
            .iter()
            .find(|d| d.name.contains("Nuvoton") && d.name.contains("#1"))
            .expect("chassis fan #1")
            .clone();
        let control = fan.capability_str(caps::FAN_SPEED_PERCENT).unwrap().clone();

        let outcome = adapter
            .write(&fan, &control, &Value::Number(72.0))
            .await
            .unwrap();
        assert!(outcome.is_applied());
        assert_eq!(outcome.applied, Some(Value::Number(72.0)));

        // One `Set` for the write, plus one `Get` for the confirmation.
        let writes = server.writes();
        assert_eq!(writes.len(), 2, "{writes:?}");
        assert!(writes[0].contains("action=Set"));
        assert!(writes[1].contains("action=Get"), "the write is read back");
        assert!(writes[0].contains("id=/lpc/nct6687d/control/0"));
        assert!(writes[0].contains("value=72"));
    }

    #[tokio::test]
    async fn writes_are_validated_before_touching_the_server() {
        let server = FakeLhm::start();
        let adapter = adapter(&server);
        let devices = adapter.discover().await.unwrap();
        let fan = devices
            .iter()
            .find(|d| d.supports(caps::FAN_SPEED_PERCENT))
            .unwrap()
            .clone();
        let control = fan.capability_str(caps::FAN_SPEED_PERCENT).unwrap();

        assert_eq!(
            adapter
                .write(&fan, control, &Value::Number(180.0))
                .await
                .unwrap_err()
                .code(),
            "value_out_of_range"
        );
        let read_only = fan.capability_str(caps::FAN_RPM);
        if let Some(rpm) = read_only {
            assert_eq!(
                adapter
                    .write(&fan, rpm, &Value::Number(1000.0))
                    .await
                    .unwrap_err()
                    .code(),
                "capability_read_only"
            );
        }
        assert!(server.writes().is_empty(), "nothing should have been sent");
    }

    #[tokio::test]
    async fn a_refused_write_is_reported_as_rejected() {
        let server = FakeLhm::start();
        server.fail_writes(true);
        let adapter = adapter(&server);
        let devices = adapter.discover().await.unwrap();
        let fan = devices
            .iter()
            .find(|d| d.supports(caps::FAN_SPEED_PERCENT))
            .unwrap()
            .clone();
        let control = fan.capability_str(caps::FAN_SPEED_PERCENT).unwrap();
        let err = adapter
            .write(&fan, &control.clone(), &Value::Number(50.0))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "write_rejected");
        assert!(err.to_string().contains("Administrator") || err.to_string().contains("refused"));
    }

    #[tokio::test]
    async fn an_unreachable_server_is_explained_not_hidden() {
        // Nothing is listening on this port.
        let adapter = LhmAdapter::new(LhmConfig {
            base_url: "http://127.0.0.1:1".into(),
            timeout_ms: 300,
            ..LhmConfig::default()
        });
        let status = adapter.probe().await;
        assert_eq!(status.state, ohm_adapter_api::AdapterState::Unavailable);
        assert_eq!(status.reason, Some(UnavailableReason::NotPresent));
        let detail = status.detail.unwrap();
        assert!(detail.contains("Remote Web Server"), "{detail}");

        let err = adapter.discover().await.unwrap_err();
        assert_eq!(err.code(), "adapter_unavailable");
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn a_write_is_confirmed_by_reading_the_channel_back() {
        let server = FakeLhm::start();
        let adapter = adapter(&server);
        let devices = adapter.discover().await.unwrap();
        let fan = devices
            .iter()
            .find(|d| d.name.contains("Nuvoton") && d.name.contains("#1"))
            .expect("chassis fan #1")
            .clone();
        let control = fan.capability_str(caps::FAN_SPEED_PERCENT).unwrap().clone();

        let reads_before = server.reads();
        let outcome = adapter
            .write(&fan, &control, &Value::Number(64.0))
            .await
            .expect("the write is accepted");
        assert!(outcome.is_applied());
        assert_eq!(
            outcome.applied,
            Some(Value::Number(64.0)),
            "the reported value is what the channel reports, not just what we asked for"
        );
        assert!(
            server.reads() > reads_before,
            "the adapter must read the channel back to confirm the write"
        );
        assert_eq!(server.channel_value(), 64.0);
    }

    /// A read-back that cannot confirm the value must NOT be reported as applied
    /// with the requested value: nobody knows what the channel holds.
    #[tokio::test]
    async fn an_unreadable_channel_does_not_claim_the_requested_value() {
        for (label, configure) in [
            ("N/A", FakeLhm::read_not_available as fn(&FakeLhm)),
            ("HTTP error", FakeLhm::read_errors),
        ] {
            let server = FakeLhm::start();
            let adapter = adapter(&server);
            let devices = adapter.discover().await.unwrap();
            let fan = devices
                .iter()
                .find(|d| d.name.contains("Nuvoton") && d.name.contains("#1"))
                .expect("chassis fan #1")
                .clone();
            let control = fan.capability_str(caps::FAN_SPEED_PERCENT).unwrap().clone();

            configure(&server);
            let outcome = adapter
                .write(&fan, &control, &Value::Number(73.0))
                .await
                .unwrap_or_else(|error| panic!("{label}: the write was accepted: {error}"));

            assert_ne!(
                outcome.status,
                ohm_adapter_api::WriteStatus::Applied,
                "{label}: an unconfirmed write must not be reported as applied"
            );
            assert_eq!(
                outcome.applied, None,
                "{label}: an unknown value must not be filled in with the requested one"
            );
            assert!(
                outcome
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("not confirmed")
                        || detail.contains("unconfirmed")
                        || detail.contains("could not be confirmed")),
                "{label}: the detail must say the value is unconfirmed, got {:?}",
                outcome.detail
            );
        }
    }

    /// A channel that becomes readable again must return to confirmed writes.
    #[tokio::test]
    async fn confirmation_resumes_once_the_channel_can_be_read_again() {
        let server = FakeLhm::start();
        server.read_not_available();
        let adapter = adapter(&server);
        let devices = adapter.discover().await.unwrap();
        let fan = devices
            .iter()
            .find(|d| d.name.contains("Nuvoton") && d.name.contains("#1"))
            .expect("chassis fan #1")
            .clone();
        let control = fan.capability_str(caps::FAN_SPEED_PERCENT).unwrap().clone();

        assert!(
            adapter
                .write(&fan, &control, &Value::Number(58.0))
                .await
                .unwrap()
                .is_unconfirmed()
        );

        server.read_normally();
        let outcome = adapter
            .write(&fan, &control, &Value::Number(58.0))
            .await
            .unwrap();
        assert!(outcome.is_applied(), "a readable channel confirms again");
        assert_eq!(outcome.applied, Some(Value::Number(58.0)));
    }

    /// Confirming the set point is not the same as observing the fan respond.
    #[tokio::test]
    async fn a_confirmed_set_point_says_nothing_about_airflow() {
        let server = FakeLhm::start();
        let adapter = adapter(&server);
        let devices = adapter.discover().await.unwrap();
        let fan = devices
            .iter()
            .find(|d| d.name.contains("Nuvoton") && d.name.contains("#1"))
            .expect("chassis fan #1")
            .clone();
        let control = fan.capability_str(caps::FAN_SPEED_PERCENT).unwrap().clone();

        let outcome = adapter
            .write(&fan, &control, &Value::Number(66.0))
            .await
            .unwrap();
        assert_eq!(outcome.status, ohm_adapter_api::WriteStatus::Applied);
        let detail = outcome.detail.clone().unwrap_or_default().to_lowercase();
        for forbidden in ["rpm", "spun", "airflow", "responded"] {
            assert!(
                !detail.contains(forbidden),
                "the adapter may only speak about the set point it read back, not about {forbidden}: {detail}"
            );
        }
        assert!(
            detail.contains("set point") || detail.contains("channel"),
            "the confirmation should name what was confirmed: {detail}"
        );
    }

    #[tokio::test]
    async fn a_channel_that_ignores_writes_is_reported_as_refused() {
        // The failure this guards against: LHM answers 200, the SuperIO holds the
        // old value, and the app would otherwise claim success.
        let server = FakeLhm::start();
        server.stick_channel_at(40.0);
        let adapter = adapter(&server);
        let devices = adapter.discover().await.unwrap();
        let fan = devices
            .iter()
            .find(|d| d.name.contains("Nuvoton") && d.name.contains("#1"))
            .expect("chassis fan #1")
            .clone();
        let control = fan.capability_str(caps::FAN_SPEED_PERCENT).unwrap().clone();

        let error = adapter
            .write(&fan, &control, &Value::Number(90.0))
            .await
            .expect_err("a value that did not take must not be reported as applied");
        assert_eq!(error.code(), "write_rejected");
        let message = error.to_string();
        assert!(message.contains("90.0"), "{message}");
        assert!(message.contains("40.0"), "{message}");
        assert!(message.contains("not applied"), "{message}");
    }

    #[tokio::test]
    async fn shutdown_releases_every_control_channel() {
        let server = FakeLhm::start();
        let adapter = adapter(&server);
        let _ = adapter.discover().await.unwrap();
        adapter.shutdown().await.unwrap();

        let writes = server.writes();
        assert!(!writes.is_empty());
        assert!(
            writes.iter().all(|w| w.contains("value=null")),
            "releasing means SetDefault: {writes:?}"
        );
        // One release per control channel found in the fixture.
        assert_eq!(writes.len(), adapter.mapping().control_count());
    }

    #[tokio::test]
    async fn adapter_metadata_is_complete() {
        let server = FakeLhm::start();
        let adapter = adapter(&server);
        let info = adapter.info();
        assert_eq!(info.id.as_str(), ADAPTER_ID);
        assert!(info.requires_admin);
        assert!(info.capabilities.can_control_cooling);
        assert!(info.homepage.unwrap().contains("LibreHardwareMonitor"));
        assert_eq!(adapter.base_url(), server.base_url());
        assert!(
            !LhmAdapter::boxed(LhmConfig::default())
                .info()
                .description
                .is_empty()
        );
    }
}
