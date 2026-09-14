//! Discovery: probing adapters and enumerating hardware.
//!
//! Discovery is deliberately split from reconciliation:
//!
//! * [`DiscoveryManager::discover_all`] awaits the adapters (slow, I/O bound,
//!   never holds a lock) and returns raw results.
//! * [`DeviceTable::reconcile`](crate::device_table::DeviceTable::reconcile)
//!   then applies them under a lock, which is where hotplug detection happens.
//!
//! An adapter that fails discovery never takes the runtime down: it is marked
//! with a reason (`driver_missing`, `permission_denied`, ...) and the rest of
//! the system keeps running.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use ohm_adapter_api::{AdapterState, AdapterStatus, HardwareAdapter};
use ohm_core::{AdapterId, Result};
use ohm_device_model::{Device, UnavailableReason};
use parking_lot::RwLock;

use crate::config::Settings;
use crate::snapshot::AdapterView;

/// Raw result of one full discovery cycle, before reconciliation.
#[derive(Debug, Default)]
pub struct DiscoveryOutcome {
    /// Per adapter health after the cycle.
    pub statuses: Vec<AdapterStatus>,
    /// Devices each usable adapter reported.
    pub devices: BTreeMap<AdapterId, Vec<Device>>,
    /// Adapters that failed, with the reason.
    pub errors: Vec<(AdapterId, String)>,
    /// Wall clock duration of the cycle.
    pub duration_ms: u64,
}

impl DiscoveryOutcome {
    /// Total number of devices reported this cycle.
    pub fn device_count(&self) -> usize {
        self.devices.values().map(Vec::len).sum()
    }
}

/// Outcome of shutting one adapter down.
///
/// [`HardwareAdapter::shutdown`] is the point where an adapter hands control of
/// its channels back to the firmware, so its result is evidence. A failure is
/// recorded with its reason because the adapter may still be holding a channel
/// the user believes was released.
#[derive(Debug, Clone, PartialEq)]
pub struct AdapterShutdown {
    pub adapter: AdapterId,
    /// `true` when this adapter can drive cooling hardware, i.e. when its
    /// shutdown is the moment firmware control comes back.
    pub controls_cooling: bool,
    /// `true` when the adapter declares that its shutdown really hands those
    /// channels back. An adapter that returns `Ok(())` without doing anything is
    /// not evidence of a hand-back, so it says so in its capabilities instead.
    pub hands_back: bool,
    /// `None` when the adapter reported a clean hand-back.
    pub error: Option<String>,
}

/// Owns the adapters and their health.
pub struct DiscoveryManager {
    adapters: Vec<Arc<dyn HardwareAdapter>>,
    statuses: RwLock<BTreeMap<AdapterId, AdapterStatus>>,
}

impl std::fmt::Debug for DiscoveryManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscoveryManager")
            .field(
                "adapters",
                &self
                    .adapters
                    .iter()
                    .map(|a| a.info().id.to_string())
                    .collect::<Vec<_>>(),
            )
            .field("statuses", &self.statuses.read())
            .finish()
    }
}

impl DiscoveryManager {
    /// Register adapters in the order they should be probed. Registration order
    /// matters when two adapters expose the same hardware: the first one wins.
    pub fn new(adapters: Vec<Arc<dyn HardwareAdapter>>) -> Self {
        let statuses = adapters
            .iter()
            .map(|a| {
                let id = a.info().id;
                (id.clone(), AdapterStatus::not_probed(id))
            })
            .collect();
        Self {
            adapters,
            statuses: RwLock::new(statuses),
        }
    }

    pub fn adapters(&self) -> &[Arc<dyn HardwareAdapter>] {
        &self.adapters
    }

    pub fn adapter(&self, id: &str) -> Option<Arc<dyn HardwareAdapter>> {
        self.adapters
            .iter()
            .find(|a| a.info().id.as_str() == id)
            .cloned()
    }

    pub fn adapter_ids(&self) -> Vec<AdapterId> {
        self.adapters.iter().map(|a| a.info().id).collect()
    }

    pub fn len(&self) -> usize {
        self.adapters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.adapters.is_empty()
    }

    /// Current health of one adapter.
    pub fn status(&self, id: &str) -> Option<AdapterStatus> {
        self.statuses
            .read()
            .get(&AdapterId::new_unchecked(id))
            .cloned()
    }

    /// Status plus static info for every adapter.
    pub fn views(&self) -> Vec<AdapterView> {
        self.adapters
            .iter()
            .map(|adapter| {
                let info = adapter.info();
                let status = self
                    .status(info.id.as_str())
                    .unwrap_or_else(|| AdapterStatus::not_probed(info.id.clone()));
                AdapterView { info, status }
            })
            .collect()
    }

    fn store_status(&self, status: AdapterStatus) {
        self.statuses.write().insert(status.adapter.clone(), status);
    }

    /// Probe one adapter, honouring the user's enable/disable choice.
    pub async fn probe(
        &self,
        adapter: &Arc<dyn HardwareAdapter>,
        settings: &Settings,
    ) -> AdapterStatus {
        let info = adapter.info();
        if !settings.adapter_enabled(info.id.as_str()) {
            let status = AdapterStatus {
                adapter: info.id,
                state: AdapterState::Unavailable,
                reason: Some(UnavailableReason::Disabled),
                detail: Some("disabled in Settings".to_string()),
                checked_at_ms: ohm_core::now_ms(),
                device_count: 0,
            };
            self.store_status(status.clone());
            return status;
        }

        let status = adapter.probe().await;
        self.store_status(status.clone());
        status
    }

    /// Probe every registered adapter.
    pub async fn probe_all(&self, settings: &Settings) -> Vec<AdapterStatus> {
        let mut out = Vec::with_capacity(self.adapters.len());
        for adapter in &self.adapters {
            out.push(self.probe(adapter, settings).await);
        }
        out
    }

    /// Run a full discovery cycle.
    ///
    /// Adapters that are disabled or unavailable are skipped, but they still
    /// report a status so the UI can explain *why* nothing shows up.
    pub async fn discover_all(&self, settings: &Settings) -> DiscoveryOutcome {
        let started = Instant::now();
        let mut outcome = DiscoveryOutcome::default();

        for adapter in &self.adapters {
            let status = self.probe(adapter, settings).await;
            let info = adapter.info();

            if !status.is_usable() {
                tracing::debug!(
                    adapter = info.id.as_str(),
                    state = status.state.as_str(),
                    detail = status.detail.as_deref().unwrap_or(""),
                    "adapter skipped during discovery"
                );
                outcome.statuses.push(status);
                continue;
            }

            match adapter.discover().await {
                Ok(devices) => {
                    let devices = validate_devices(&info.id, devices);
                    let count = devices.len();
                    let status = if count == 0 {
                        AdapterStatus {
                            state: AdapterState::Degraded,
                            reason: Some(UnavailableReason::NotPresent),
                            detail: Some(format!("{} started but reported no devices", info.name)),
                            device_count: 0,
                            checked_at_ms: ohm_core::now_ms(),
                            ..status
                        }
                    } else {
                        AdapterStatus {
                            state: AdapterState::Available,
                            reason: None,
                            detail: None,
                            device_count: count,
                            checked_at_ms: ohm_core::now_ms(),
                            ..status
                        }
                    };
                    self.store_status(status.clone());
                    outcome.statuses.push(status);
                    outcome.devices.insert(info.id, devices);
                }
                Err(err) => {
                    let status = AdapterStatus {
                        state: AdapterState::Error,
                        reason: Some(UnavailableReason::ReadError),
                        detail: Some(err.to_string()),
                        device_count: 0,
                        checked_at_ms: ohm_core::now_ms(),
                        ..status
                    };
                    self.store_status(status.clone());
                    outcome.errors.push((info.id.clone(), err.to_string()));
                    outcome.statuses.push(status);
                }
            }
        }

        outcome.duration_ms = started.elapsed().as_millis() as u64;
        outcome
    }

    /// Shut every adapter down, which is where control is released back to the
    /// firmware. Errors are collected instead of aborting the shutdown, and a
    /// success is recorded too: "the adapter handed control back" is evidence,
    /// and so is "it did not".
    pub async fn shutdown_all(&self) -> Vec<AdapterShutdown> {
        let mut outcomes = Vec::with_capacity(self.adapters.len());
        for adapter in &self.adapters {
            let info = adapter.info();
            let error = adapter.shutdown().await.err().map(|err| err.to_string());
            outcomes.push(AdapterShutdown {
                controls_cooling: info.capabilities.can_control_cooling,
                hands_back: info.capabilities.hands_back_control_on_shutdown,
                adapter: info.id,
                error,
            });
        }
        outcomes
    }

    /// Adapters whose last probe said they are usable.
    pub fn usable_adapters(&self) -> Vec<Arc<dyn HardwareAdapter>> {
        self.adapters
            .iter()
            .filter(|a| {
                self.status(a.info().id.as_str())
                    .map(|s| s.is_usable())
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }
}

/// Drop devices an adapter described incorrectly, logging each rejection.
fn validate_devices(adapter: &AdapterId, devices: Vec<Device>) -> Vec<Device> {
    let mut kept = Vec::with_capacity(devices.len());
    let mut seen: Vec<ohm_core::DeviceId> = Vec::with_capacity(devices.len());
    for device in devices {
        if let Err(err) = device.validate() {
            tracing::warn!(
                adapter = adapter.as_str(),
                device = device.id.as_str(),
                error = %err,
                "adapter reported an invalid device, skipping it"
            );
            continue;
        }
        if seen.contains(&device.id) {
            tracing::warn!(
                adapter = adapter.as_str(),
                device = device.id.as_str(),
                "adapter reported the same device twice, keeping the first"
            );
            continue;
        }
        seen.push(device.id.clone());
        kept.push(device);
    }
    kept
}

/// Validate that an adapter id is unique in the list, keeping the first.
pub fn deduplicate_adapters(
    adapters: Vec<Arc<dyn HardwareAdapter>>,
) -> (Vec<Arc<dyn HardwareAdapter>>, Vec<AdapterId>) {
    let mut seen: Vec<AdapterId> = Vec::new();
    let mut duplicates = Vec::new();
    let mut kept = Vec::new();
    for adapter in adapters {
        let id = adapter.info().id;
        if seen.contains(&id) {
            duplicates.push(id);
        } else {
            seen.push(id);
            kept.push(adapter);
        }
    }
    (kept, duplicates)
}

/// Result alias for adapter construction helpers.
pub type AdapterResult = Result<Arc<dyn HardwareAdapter>>;

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ohm_adapter_api::{AdapterInfo, WriteOutcome};
    use ohm_core::{CapabilityId, DeviceId};
    use ohm_device_model::{Capability, DeviceState, DeviceType, Transport, Unit, Value};

    struct StubAdapter {
        info: AdapterInfo,
        state: AdapterState,
        devices: Vec<Device>,
        fail_discovery: bool,
    }

    impl StubAdapter {
        fn build(
            id: &str,
            state: AdapterState,
            device_count: usize,
            fail: bool,
        ) -> Arc<dyn HardwareAdapter> {
            let devices = (0..device_count)
                .map(|i| {
                    Device::new(
                        DeviceId::compose("fan", id, i),
                        format!("{id} fan {i}"),
                        DeviceType::Fan,
                        Transport::Mock,
                        AdapterId::new(id).unwrap(),
                    )
                    .with_capability(Capability::sensor(
                        "fan.rpm",
                        "Fan RPM",
                        Unit::Rpm,
                    ))
                })
                .collect();
            Arc::new(Self {
                info: AdapterInfo::new(id, id, id),
                state,
                devices,
                fail_discovery: fail,
            })
        }

        fn stub(id: &str, state: AdapterState, device_count: usize) -> Arc<dyn HardwareAdapter> {
            Self::build(id, state, device_count, false)
        }

        fn failing(id: &str) -> Arc<dyn HardwareAdapter> {
            Self::build(id, AdapterState::Available, 1, true)
        }
    }

    #[async_trait]
    impl HardwareAdapter for StubAdapter {
        fn info(&self) -> AdapterInfo {
            self.info.clone()
        }

        async fn probe(&self) -> AdapterStatus {
            match self.state {
                AdapterState::Available => AdapterStatus::available(self.info.id.clone(), 0),
                AdapterState::Unavailable => AdapterStatus::unavailable(
                    self.info.id.clone(),
                    UnavailableReason::DriverMissing,
                    "stub driver missing",
                ),
                _ => AdapterStatus::degraded(
                    self.info.id.clone(),
                    UnavailableReason::PermissionDenied,
                    "stub not elevated",
                ),
            }
        }

        async fn discover(&self) -> Result<Vec<Device>> {
            if self.fail_discovery {
                return Err(ohm_core::OhmError::Adapter {
                    adapter: self.info.id.to_string(),
                    detail: "stub discovery failure".into(),
                });
            }
            Ok(self.devices.clone())
        }

        async fn read_state(&self, device: &Device) -> Result<DeviceState> {
            Ok(DeviceState::new(device.id.clone(), ohm_core::now_ms()))
        }

        async fn write(
            &self,
            _device: &Device,
            _capability: &Capability,
            _value: &Value,
        ) -> Result<WriteOutcome> {
            Ok(WriteOutcome::applied(0))
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[tokio::test]
    async fn available_adapters_contribute_devices() {
        let manager = DiscoveryManager::new(vec![
            StubAdapter::stub("mock", AdapterState::Available, 2),
            StubAdapter::stub("lhm", AdapterState::Unavailable, 3),
        ]);
        let outcome = manager.discover_all(&Settings::default()).await;
        assert_eq!(outcome.devices.len(), 1);
        assert_eq!(outcome.devices[&AdapterId::new("mock").unwrap()].len(), 2);
        assert_eq!(outcome.device_count(), 2);
        assert_eq!(
            manager.status("lhm").unwrap().reason,
            Some(UnavailableReason::DriverMissing)
        );
    }

    #[tokio::test]
    async fn zero_devices_is_degraded_not_available() {
        let manager = DiscoveryManager::new(vec![StubAdapter::stub(
            "system",
            AdapterState::Available,
            0,
        )]);
        let outcome = manager.discover_all(&Settings::default()).await;
        let status = manager.status("system").unwrap();
        assert_eq!(status.state, AdapterState::Degraded);
        assert_eq!(status.reason, Some(UnavailableReason::NotPresent));
        assert!(status.detail.unwrap().contains("no devices"));
        assert!(outcome.devices[&AdapterId::new("system").unwrap()].is_empty());
    }

    #[tokio::test]
    async fn disabled_adapters_are_not_probed() {
        let mut settings = Settings::default();
        settings.set_adapter_enabled(&AdapterId::new("mock").unwrap(), false);
        let manager =
            DiscoveryManager::new(vec![StubAdapter::stub("mock", AdapterState::Available, 1)]);
        let outcome = manager.discover_all(&settings).await;
        assert!(outcome.devices.is_empty());
        assert_eq!(
            manager.status("mock").unwrap().reason,
            Some(UnavailableReason::Disabled)
        );
        assert!(manager.usable_adapters().is_empty());
    }

    #[tokio::test]
    async fn discovery_failure_is_contained() {
        let manager = DiscoveryManager::new(vec![StubAdapter::failing("broken")]);
        let outcome = manager.discover_all(&Settings::default()).await;
        assert_eq!(outcome.errors.len(), 1);
        assert_eq!(manager.status("broken").unwrap().state, AdapterState::Error);
        assert!(outcome.devices.is_empty());
    }

    #[test]
    fn deduplicate_keeps_the_first_registration() {
        let (kept, duplicates) = deduplicate_adapters(vec![
            StubAdapter::stub("mock", AdapterState::Available, 1),
            StubAdapter::stub("mock", AdapterState::Available, 2),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(duplicates, vec![AdapterId::new("mock").unwrap()]);
    }

    #[test]
    fn views_include_unprobed_status() {
        let manager =
            DiscoveryManager::new(vec![StubAdapter::stub("mock", AdapterState::Available, 1)]);
        let views = manager.views();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].status.state, AdapterState::NotProbed);
        assert!(!views[0].is_usable());
        assert_eq!(views[0].id(), "mock");
        assert_eq!(manager.adapter_ids().len(), 1);
        assert!(manager.adapter("mock").is_some());
        assert!(manager.adapter("nope").is_none());
        assert_eq!(manager.len(), 1);
        assert!(!manager.is_empty());
    }

    #[test]
    fn invalid_devices_are_dropped() {
        let adapter = AdapterId::new("mock").unwrap();
        let broken = Device::new(
            DeviceId::new("fan.mock.9").unwrap(),
            "No Capabilities",
            DeviceType::Fan,
            Transport::Mock,
            adapter.clone(),
        );
        let good = Device::new(
            DeviceId::new("fan.mock.0").unwrap(),
            "Fine",
            DeviceType::Fan,
            Transport::Mock,
            adapter.clone(),
        )
        .with_capability(Capability::sensor("fan.rpm", "Fan RPM", Unit::Rpm));
        let kept = validate_devices(&adapter, vec![broken, good.clone(), good]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id.as_str(), "fan.mock.0");
    }

    #[test]
    fn capability_id_type_is_usable() {
        let id = CapabilityId::new("fan.rpm").unwrap();
        assert_eq!(id.as_str(), "fan.rpm");
    }
}
