//! The Hardware Runtime: the single door to the hardware.
//!
//! ```text
//!           ┌──────────────── Runtime ─────────────────┐
//!  UI/CLI ──┤ commands ─▶ write path ─▶ safety ─▶ adapter │──▶ hardware
//!           │ events   ◀─ bus        ◀─ state store      │
//!           └───────────────────────────────────────────┘
//! ```
//!
//! Nothing outside this crate is allowed to talk to an adapter directly. The UI
//! calls [`Runtime::write_value`], the automation engine calls the same method
//! with a different [`WriteOrigin`], and both go through range validation, the
//! [`SafetyPolicy`](crate::safety::SafetyPolicy) gate and the audit log.
//!
//! The runtime also owns two background loops:
//!
//! * **poll** — reads every enabled device on `polling_interval_ms`, publishes
//!   change events and keeps the state store warm.
//! * **discovery** — re-enumerates adapters on `discovery_interval_ms`, which is
//!   how hotplug is detected.
//!
//! A third, continuously evaluated responsibility is inherent to the poll loop:
//! the **safety supervisor**, which enforces the emergency temperature ceiling
//! even when no automation rule is configured.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ohm_adapter_api::{HardwareAdapter, WriteStatus};
use ohm_core::{AdapterId, CapabilityId, ConfigPaths, DeviceId, OhmError, Result};
use ohm_device_model::{Capability, Device, DeviceState, UnavailableReason, Unit, Value};
use parking_lot::{Mutex, RwLock};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::audit::{AuditLog, WriteOrigin, WriteReport};
use crate::bus::{EventBus, RuntimeEvent};
use crate::config::{Settings, SettingsStore};
use crate::device_table::DeviceTable;
use crate::discovery::DiscoveryManager;
use crate::registry::CapabilityRegistry;
use crate::release::{ControlRelease, ReleasedChannel, ShutdownFailure};
use crate::safety::SafetyDecision;
use crate::snapshot::{AdapterView, DeviceStatus, DeviceView, RuntimeSnapshot, RuntimeStats};
use crate::store::{Sample, StateStore};

/// Atomic counters behind [`RuntimeStats`].
#[derive(Debug, Default)]
struct Stats {
    poll_cycles: AtomicU64,
    discovery_cycles: AtomicU64,
    writes_attempted: AtomicU64,
    writes_applied: AtomicU64,
    writes_rejected: AtomicU64,
    safety_interventions: AtomicU64,
    last_poll_ms: AtomicI64,
    last_poll_duration_ms: AtomicU64,
    last_poll_errors: AtomicUsize,
}

impl Stats {
    fn snapshot(&self, events_published: u64) -> RuntimeStats {
        RuntimeStats {
            poll_cycles: self.poll_cycles.load(Ordering::Relaxed),
            discovery_cycles: self.discovery_cycles.load(Ordering::Relaxed),
            writes_attempted: self.writes_attempted.load(Ordering::Relaxed),
            writes_applied: self.writes_applied.load(Ordering::Relaxed),
            writes_rejected: self.writes_rejected.load(Ordering::Relaxed),
            safety_interventions: self.safety_interventions.load(Ordering::Relaxed),
            events_published,
            last_poll_ms: self.last_poll_ms.load(Ordering::Relaxed),
            last_poll_duration_ms: self.last_poll_duration_ms.load(Ordering::Relaxed),
            last_poll_errors: self.last_poll_errors.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug)]
struct RuntimeInner {
    paths: ConfigPaths,
    settings_store: SettingsStore,
    settings: RwLock<Settings>,
    discovery: DiscoveryManager,
    table: RwLock<DeviceTable>,
    store: StateStore,
    bus: EventBus,
    audit: AuditLog,
    stats: Stats,
    started_at_ms: AtomicI64,
    running: AtomicBool,
    emergency_active: AtomicBool,
    /// Serialises writes so two rules cannot interleave on one actuator.
    write_gate: tokio::sync::Mutex<()>,
    /// When each adapter was last polled, so an adapter that declares its own
    /// cadence (`AdapterCapabilities::poll_interval_ms`) is not polled faster
    /// than it asked for.
    adapter_last_poll: Mutex<std::collections::HashMap<AdapterId, i64>>,
    stop_tx: watch::Sender<bool>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

/// The runtime handle. Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

impl Runtime {
    /// Build a runtime with an explicit configuration root and settings.
    pub fn new(
        paths: ConfigPaths,
        settings: Settings,
        adapters: Vec<Arc<dyn HardwareAdapter>>,
    ) -> Result<Self> {
        paths.ensure()?;
        let mut settings = settings;
        if settings.sanitise() {
            tracing::debug!("settings were adjusted into their supported range");
        }

        let (adapters, duplicates) = crate::discovery::deduplicate_adapters(adapters);
        if !duplicates.is_empty() {
            tracing::warn!(
                ?duplicates,
                "duplicate adapters registered, keeping the first"
            );
        }

        let store = StateStore::new(settings.history_points);
        let (stop_tx, _stop_rx) = watch::channel(false);
        let settings_store = SettingsStore::new(&paths);
        let audit = AuditLog::new(paths.audit_log());

        let inner = RuntimeInner {
            paths,
            settings_store,
            settings: RwLock::new(settings),
            discovery: DiscoveryManager::new(adapters),
            table: RwLock::new(DeviceTable::new()),
            store,
            bus: EventBus::default(),
            audit,
            stats: Stats::default(),
            started_at_ms: AtomicI64::new(0),
            running: AtomicBool::new(false),
            emergency_active: AtomicBool::new(false),
            write_gate: tokio::sync::Mutex::new(()),
            adapter_last_poll: Mutex::new(std::collections::HashMap::new()),
            stop_tx,
            tasks: Mutex::new(Vec::new()),
        };

        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Build a runtime from the platform config directory.
    pub fn from_config_dir(adapters: Vec<Arc<dyn HardwareAdapter>>) -> Result<Self> {
        let paths = ConfigPaths::discover()?;
        let settings = SettingsStore::new(&paths).load()?;
        Self::new(paths, settings, adapters)
    }

    // ---------------------------------------------------------------- startup

    /// Probe, discover, poll once and start the background loops.
    pub async fn start(&self) -> Result<RuntimeSnapshot> {
        if self.inner.running.swap(true, Ordering::SeqCst) {
            tracing::debug!("runtime already started");
            return Ok(self.snapshot());
        }
        self.inner
            .started_at_ms
            .store(ohm_core::now_ms(), Ordering::Relaxed);

        let discovery = self.refresh_devices().await?;
        self.poll_once().await?;

        let (device_count, adapter_count) = (discovery.device_count(), self.inner.discovery.len());
        self.inner.audit.record_lifecycle(
            "runtime_started",
            &format!("{adapter_count} adapters, {device_count} devices"),
        );
        self.inner.bus.publish(RuntimeEvent::RuntimeStarted {
            at_ms: self.inner.started_at_ms.load(Ordering::Relaxed),
            device_count,
        });

        self.spawn_loops();
        Ok(self.snapshot())
    }

    fn spawn_loops(&self) {
        let mut tasks = self.inner.tasks.lock();
        if !tasks.is_empty() {
            return;
        }

        // Poll loop.
        {
            let inner = Arc::clone(&self.inner);
            let mut stop = inner.stop_tx.subscribe();
            tasks.push(tokio::spawn(async move {
                loop {
                    let interval = inner.settings.read().polling_interval_ms;
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(interval)) => {}
                        _ = stop.changed() => break,
                    }
                    if !inner.running.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Err(err) = Runtime::poll_inner(&inner).await {
                        tracing::warn!(error = %err, "poll cycle failed");
                    }
                }
                tracing::debug!("poll loop stopped");
            }));
        }

        // Discovery loop (hotplug detection).
        {
            let inner = Arc::clone(&self.inner);
            let mut stop = inner.stop_tx.subscribe();
            tasks.push(tokio::spawn(async move {
                loop {
                    let interval = inner.settings.read().discovery_interval_ms;
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(interval)) => {}
                        _ = stop.changed() => break,
                    }
                    if !inner.running.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Err(err) = Runtime::refresh_inner(&inner).await {
                        tracing::warn!(error = %err, "discovery cycle failed");
                    }
                }
                tracing::debug!("discovery loop stopped");
            }));
        }
    }

    /// Stop the loops, release control and hand the fans back to the firmware.
    ///
    /// The returned [`ControlRelease`] separates what is known: the fail-safe
    /// duty was written, the device read it back, the adapter gave ownership
    /// back. Each of those is audited separately, and a problem is recorded as a
    /// problem — the exit path no longer reports an unconfirmed write as a
    /// successful hand-back.
    /// Ask the adapter that owns a device whether the platform would let this process
    /// write a capability.
    ///
    /// `None` when no adapter claims the device — an answer nobody gave, which callers
    /// must report as such rather than as permission.
    pub fn write_access(
        &self,
        device: &Device,
        capability: &Capability,
    ) -> Option<ohm_adapter_api::WriteAccess> {
        self.inner
            .discovery
            .adapter(device.adapter.as_str())
            .map(|adapter| adapter.write_access(device, capability))
    }

    /// Channels the adapters have switched away from their drivers and still owe back.
    ///
    /// The service publishes this in its state file so that a process taking over after
    /// a crash can put those channels back. Without it, a takeover reads the switched
    /// value as the original and restores *that* — leaving a fan in manual mode for
    /// good, which is the failure this whole handover exists to prevent.
    pub fn taken_controls(&self) -> Vec<ohm_adapter_api::TakenControl> {
        self.inner
            .discovery
            .adapters()
            .iter()
            .flat_map(|adapter| adapter.taken_controls())
            .collect()
    }

    /// Accept responsibility for channels a previous process switched.
    ///
    /// Returns one sentence per channel that could **not** be adopted. A non-empty
    /// result is not an error to ignore: it names channels that nobody will put back
    /// unless a person does it, so callers are expected to report it rather than
    /// swallow it.
    pub fn adopt_taken_controls(&self, taken: &[ohm_adapter_api::TakenControl]) -> Vec<String> {
        if taken.is_empty() {
            return Vec::new();
        }
        let adapters = self.inner.discovery.adapters();
        let mut problems: Vec<String> = adapters
            .iter()
            .flat_map(|adapter| adapter.adopt_taken_controls(taken))
            .collect();

        // Whatever no adapter now holds is a channel nobody will put back. Adapters are
        // expected to say so themselves; this is the net under that expectation, and it
        // stays quiet for channels already mentioned so the report does not repeat itself.
        let held: std::collections::BTreeSet<String> = adapters
            .iter()
            .flat_map(|adapter| adapter.taken_controls())
            .map(|entry| entry.device_id)
            .collect();
        for entry in taken {
            if held.contains(&entry.device_id) {
                continue;
            }
            if problems
                .iter()
                .any(|problem| problem.contains(&entry.device_id))
            {
                continue;
            }
            problems.push(format!(
                "{}: no adapter took responsibility for this channel, so nothing will put it back \
                 ({})",
                entry.device_id, entry.original_text
            ));
        }
        problems
    }

    pub async fn shutdown(&self) -> Result<ControlRelease> {
        if !self.inner.running.swap(false, Ordering::SeqCst) {
            // Still safe to call twice.
            let _ = self.inner.stop_tx.send(true);
            return Ok(ControlRelease::default());
        }
        let _ = self.inner.stop_tx.send(true);

        let handles: Vec<JoinHandle<()>> = {
            let mut tasks = self.inner.tasks.lock();
            std::mem::take(&mut *tasks)
        };
        for handle in handles {
            let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
        }

        let mut release = self.release_control().await;

        if release.skipped_by_config {
            self.inner.audit.record_lifecycle(
                "control_release_skipped",
                "relinquish_on_exit is off: the exit path wrote nothing and claims nothing",
            );
        }
        if !release.confirmed.is_empty() {
            self.inner.audit.record_lifecycle(
                "control_released",
                &format!(
                    "{} output(s) confirmed at the fail-safe duty",
                    release.confirmed.len()
                ),
            );
        }
        if !release.unconfirmed.is_empty() {
            tracing::warn!(
                count = release.unconfirmed.len(),
                "the fail-safe duty was written but not read back; the value is unknown"
            );
            self.inner.audit.record_lifecycle(
                "control_release_unconfirmed",
                &format!(
                    "{} output(s) accepted the fail-safe duty but could not be read back; \
                     their value is unknown and this is not a confirmed release",
                    release.unconfirmed.len()
                ),
            );
        }
        if !release.rejected.is_empty() || !release.failed.is_empty() {
            let detail = release.problems().join(" | ");
            tracing::error!(
                refused = release.rejected.len(),
                failed = release.failed.len(),
                detail,
                "the fail-safe duty could not be applied to every output"
            );
            self.inner.audit.record_lifecycle(
                "control_release_failed",
                &format!(
                    "{} output(s) refused the fail-safe duty and {} failed: {detail}",
                    release.rejected.len(),
                    release.failed.len()
                ),
            );
        }

        for outcome in self.inner.discovery.shutdown_all().await {
            match outcome.error {
                Some(detail) => {
                    tracing::error!(
                        adapter = outcome.adapter.as_str(),
                        detail,
                        "adapter shutdown failed; it may still hold a channel"
                    );
                    self.inner.audit.record_lifecycle(
                        "adapter_shutdown_failed",
                        &format!("{}: {detail}", outcome.adapter),
                    );
                    release.shutdown_failed.push(ShutdownFailure {
                        adapter: outcome.adapter,
                        detail,
                    });
                }
                None => {
                    if outcome.controls_cooling {
                        if outcome.hands_back {
                            release.relinquished.push(outcome.adapter);
                        } else {
                            // It drives channels and it does not give them back:
                            // say so rather than counting `Ok(())` as a release.
                            release.without_hand_back.push(outcome.adapter);
                        }
                    }
                }
            }
        }
        if !release.relinquished.is_empty() {
            let adapters: Vec<&str> = release
                .relinquished
                .iter()
                .map(ohm_core::AdapterId::as_str)
                .collect();
            self.inner.audit.record_lifecycle(
                "control_relinquished",
                &format!(
                    "{} cooling adapter(s) reported handing control back to the firmware: {}",
                    adapters.len(),
                    adapters.join(", ")
                ),
            );
        }
        if !release.without_hand_back.is_empty() {
            let adapters: Vec<&str> = release
                .without_hand_back
                .iter()
                .map(ohm_core::AdapterId::as_str)
                .collect();
            tracing::warn!(
                adapters = adapters.join(", "),
                "this adapter does not hand firmware control back on shutdown; its channels keep their last duty"
            );
            self.inner.audit.record_lifecycle(
                "control_not_handed_back",
                &format!(
                    "{} cooling adapter(s) do not hand control back on shutdown: {}. \
                     The fail-safe write above is the only release for their channels",
                    adapters.len(),
                    adapters.join(", ")
                ),
            );
        }

        let stopped = if release.is_clean() {
            "clean shutdown".to_string()
        } else {
            format!("shutdown with control problems: {}", release.summary())
        };
        self.inner
            .audit
            .record_lifecycle("runtime_stopped", &stopped);
        self.inner.bus.publish(RuntimeEvent::RuntimeStopped {
            at_ms: ohm_core::now_ms(),
        });
        Ok(release)
    }

    // -------------------------------------------------------------- discovery

    /// Run one discovery cycle and reconcile the device table.
    pub async fn refresh_devices(&self) -> Result<crate::discovery::DiscoveryOutcome> {
        Runtime::refresh_inner(&self.inner).await
    }

    async fn refresh_inner(
        inner: &Arc<RuntimeInner>,
    ) -> Result<crate::discovery::DiscoveryOutcome> {
        let settings = inner.settings.read().clone();
        let outcome = inner.discovery.discover_all(&settings).await;
        let now = ohm_core::now_ms();

        let mut added = Vec::new();
        let mut removed: Vec<(DeviceId, String, UnavailableReason)> = Vec::new();
        let mut updated = Vec::new();
        let mut devices_for_store: Vec<Device> = Vec::new();
        {
            let mut table = inner.table.write();

            // Adapters that are unusable: everything they owned goes offline.
            for status in &outcome.statuses {
                if !status.is_usable() {
                    let reason = status.reason.unwrap_or(UnavailableReason::Unknown);
                    table.mark_adapter_offline(&status.adapter, reason);
                }
            }

            for (adapter, devices) in &outcome.devices {
                let diff = table.reconcile(adapter, devices.clone(), &settings, now);
                devices_for_store.extend(diff_devices(&table, &diff.added));
                added.extend(diff.added);
                removed.extend(diff.removed);
                updated.extend(diff.updated);
            }
        }

        if !added.is_empty() || !removed.is_empty() || !updated.is_empty() {
            tracing::info!(
                added = added.len(),
                removed = removed.len(),
                updated = updated.len(),
                "device table reconciled"
            );
        }

        // Seed the state store for brand new devices so the UI has something to
        // render before the next poll completes.
        for device in devices_for_store {
            let state = DeviceState::new(device.id.clone(), now);
            inner.store.update(state, &inner.table.read().units_map());
        }

        for id in &added {
            if let Some(record) = inner.table.read().get(id) {
                inner.bus.publish(RuntimeEvent::DeviceAdded {
                    device: Box::new(record.device.clone()),
                });
            }
        }
        for (id, name, reason) in &removed {
            let device = id.clone();
            inner.store.remove(&device);
            inner.bus.publish(RuntimeEvent::DeviceRemoved {
                device_id: device,
                name: name.clone(),
                reason: *reason,
            });
        }

        for view in inner.discovery.views() {
            inner
                .bus
                .publish(RuntimeEvent::AdapterStatusChanged { status: view });
        }

        inner.stats.discovery_cycles.fetch_add(1, Ordering::Relaxed);
        Ok(outcome)
    }

    // ------------------------------------------------------------------ polls

    /// Read every enabled device once, publish changes and supervise safety.
    pub async fn poll_once(&self) -> Result<usize> {
        Runtime::poll_inner(&self.inner).await
    }

    async fn poll_inner(inner: &Arc<RuntimeInner>) -> Result<usize> {
        let started = Instant::now();
        let (targets, units, adapter_list) = {
            let table = inner.table.read();
            (
                table.poll_targets(),
                table.units_map(),
                inner.discovery.adapters().to_vec(),
            )
        };

        let mut polled = 0usize;
        let mut errors = 0usize;

        for adapter in adapter_list {
            let adapter_id = adapter.info().id;
            let batch: Vec<Device> = targets
                .iter()
                .filter(|d| d.adapter == adapter_id)
                .cloned()
                .collect();
            if batch.is_empty() {
                continue;
            }

            // An adapter may ask for a slower cadence than the global polling
            // interval (a web server, a serial link that dislikes traffic).
            if let Some(hint) = adapter.info().capabilities.poll_interval_ms {
                let last = inner
                    .adapter_last_poll
                    .lock()
                    .get(&adapter_id)
                    .copied()
                    .unwrap_or(0);
                if last > 0 && ohm_core::now_ms().saturating_sub(last) < hint as i64 {
                    tracing::trace!(
                        adapter = adapter_id.as_str(),
                        hint,
                        "adapter not due yet, skipping this cycle"
                    );
                    continue;
                }
            }
            inner
                .adapter_last_poll
                .lock()
                .insert(adapter_id.clone(), ohm_core::now_ms());

            let results = adapter.read_all(&batch).await;
            let mut table = inner.table.write();
            for (device_id, result) in results {
                match result {
                    Ok(state) => {
                        let changes = inner.store.update(state.clone(), &units);
                        table.apply_state_status(&state);
                        polled += 1;
                        for change in changes {
                            inner.bus.publish(RuntimeEvent::ReadingChanged {
                                device_id: change.device_id.clone(),
                                capability: change.capability.clone(),
                                value: change.current.clone(),
                                previous: change.previous.clone(),
                            });
                        }
                        inner.bus.publish(RuntimeEvent::StateChanged {
                            state: Box::new(state),
                        });
                    }
                    Err(err) => {
                        errors += 1;
                        tracing::warn!(
                            adapter = adapter_id.as_str(),
                            device = device_id.as_str(),
                            error = %err,
                            "device poll failed"
                        );
                        table.mark_offline(&device_id, UnavailableReason::ReadError);
                        inner.bus.publish(RuntimeEvent::Log {
                            level: "warn".into(),
                            message: format!("{device_id}: {err}"),
                            at_ms: ohm_core::now_ms(),
                        });
                    }
                }
            }
        }

        inner.stats.poll_cycles.fetch_add(1, Ordering::Relaxed);
        inner
            .stats
            .last_poll_ms
            .store(ohm_core::now_ms(), Ordering::Relaxed);
        inner
            .stats
            .last_poll_duration_ms
            .store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
        inner
            .stats
            .last_poll_errors
            .store(errors, Ordering::Relaxed);

        Self::supervise_safety(inner).await;

        Ok(polled)
    }

    /// The emergency supervisor.
    ///
    /// Runs after every poll and *independently of any automation rule*: if the
    /// hottest temperature reaches the ceiling, every controlled output is
    /// forced to the emergency duty.
    async fn supervise_safety(inner: &Arc<RuntimeInner>) {
        let settings = inner.settings.read().clone();
        if !settings.automation_enabled {
            inner.emergency_active.store(false, Ordering::Relaxed);
            return;
        }

        let hottest = hottest_temperature(inner);
        let Some(duty) = settings.safety.emergency_override(hottest) else {
            if inner.emergency_active.swap(false, Ordering::Relaxed) {
                inner.bus.publish(RuntimeEvent::SafetyTriggered {
                    kind: "emergency_cleared".into(),
                    detail: format!(
                        "hottest temperature back below {} °C",
                        settings.safety.emergency_temp_c
                    ),
                    at_ms: ohm_core::now_ms(),
                });
                inner
                    .audit
                    .record_lifecycle("emergency_cleared", "temperatures back to normal");
            }
            return;
        };

        let targets: Vec<(DeviceId, CapabilityId)> = {
            let table = inner.table.read();
            table
                .cooling_devices()
                .into_iter()
                .filter(|r| r.enabled)
                .flat_map(|r| {
                    r.device
                        .capabilities
                        .iter()
                        .filter(|c| c.is_duty_control() && c.writable)
                        .map(|c| (r.device.id.clone(), c.id.clone()))
                        .collect::<Vec<_>>()
                })
                .collect()
        };

        if targets.is_empty() {
            return;
        }

        let first_time = !inner.emergency_active.swap(true, Ordering::Relaxed);
        let detail = format!(
            "{} °C is at or above the {} °C emergency threshold",
            hottest.unwrap_or_default(),
            settings.safety.emergency_temp_c
        );
        if first_time {
            tracing::warn!(detail, "thermal emergency: forcing cooling to maximum");
            inner
                .stats
                .safety_interventions
                .fetch_add(1, Ordering::Relaxed);
            inner.bus.publish(RuntimeEvent::SafetyTriggered {
                kind: "emergency".into(),
                detail: detail.clone(),
                at_ms: ohm_core::now_ms(),
            });
            inner.audit.record_lifecycle("emergency_override", &detail);
        }

        for (device_id, capability_id) in targets {
            let current = inner
                .store
                .number(device_id.as_str(), capability_id.as_str());
            if current.is_some_and(|c| c >= duty) {
                continue;
            }
            let runtime = Runtime {
                inner: Arc::clone(inner),
            };
            let origin = WriteOrigin::Safety {
                reason: "emergency temperature threshold".to_string(),
            };
            if let Err(err) = runtime
                .write_with_origin(
                    device_id.as_str(),
                    capability_id.as_str(),
                    Value::Number(duty),
                    origin,
                    false,
                )
                .await
            {
                tracing::warn!(
                    device = device_id.as_str(),
                    capability = capability_id.as_str(),
                    error = %err,
                    "emergency write failed"
                );
            }
        }
    }

    // ---------------------------------------------------------------- writes

    /// Write one actuator capability. This is the only public write path.
    pub async fn write_value(
        &self,
        device: &str,
        capability: &str,
        value: Value,
        origin: WriteOrigin,
    ) -> Result<WriteReport> {
        self.write_with_origin(device, capability, value, origin, true)
            .await
    }

    /// Convenience wrapper for a manual duty change from the UI.
    pub async fn set_duty_percent(
        &self,
        device: &str,
        capability: &str,
        percent: f64,
    ) -> Result<WriteReport> {
        self.write_value(
            device,
            capability,
            Value::Number(percent),
            WriteOrigin::Manual,
        )
        .await
    }

    async fn write_with_origin(
        &self,
        device_id: &str,
        capability_id: &str,
        requested: Value,
        origin: WriteOrigin,
        allow_fail_safe: bool,
    ) -> Result<WriteReport> {
        let inner = &self.inner;
        inner.stats.writes_attempted.fetch_add(1, Ordering::Relaxed);

        // 1. Resolve without holding the lock across the await.
        let (device, capability, enabled) = {
            let table = inner.table.read();
            let record = table
                .get_str(device_id)
                .ok_or_else(|| OhmError::DeviceNotFound(device_id.to_string()))?;
            let capability = record
                .device
                .capability_str(capability_id)
                .cloned()
                .ok_or_else(|| OhmError::CapabilityNotFound {
                    device: device_id.to_string(),
                    capability: capability_id.to_string(),
                })?;
            (record.device.clone(), capability, record.enabled)
        };

        let settings = inner.settings.read().clone();

        // 2. Refusals that never reach the hardware are still audited.
        if !enabled {
            return Err(self.reject(
                &device,
                &capability,
                &requested,
                origin,
                OhmError::DeviceDisabled(device.id.to_string()),
            ));
        }
        if !capability.writable {
            return Err(self.reject(
                &device,
                &capability,
                &requested,
                origin,
                OhmError::CapabilityNotWritable {
                    device: device.id.to_string(),
                    capability: capability.id.to_string(),
                },
            ));
        }
        if let Err(err) = capability.validate(&requested) {
            return Err(self.reject(&device, &capability, &requested, origin, err));
        }

        // 3. Clamp to the declared range, then let safety have the final word.
        let numeric = requested.as_f64().unwrap_or(0.0);
        let mut clamped = false;
        let mut value = requested.clone();
        let mut note: Option<String> = None;
        if capability.values.is_empty() {
            let limited = capability.clamp(numeric);
            clamped = (limited - numeric).abs() > f64::EPSILON;
            value = Value::Number(limited);
        }

        if capability.is_duty_control() && capability.values.is_empty() {
            let current = inner
                .store
                .number(device.id.as_str(), capability.id.as_str());
            let hottest = hottest_temperature(inner);
            match settings.safety.check_duty(
                device.device_type,
                value.as_f64().unwrap_or(0.0),
                current,
                hottest,
            ) {
                SafetyDecision::Block { reason } => {
                    inner
                        .stats
                        .safety_interventions
                        .fetch_add(1, Ordering::Relaxed);
                    return Err(self.reject(
                        &device,
                        &capability,
                        &requested,
                        origin,
                        OhmError::SafetyBlocked {
                            device: device.id.to_string(),
                            capability: capability.id.to_string(),
                            detail: reason,
                        },
                    ));
                }
                SafetyDecision::Emergency {
                    value: forced,
                    reason,
                } => {
                    inner
                        .stats
                        .safety_interventions
                        .fetch_add(1, Ordering::Relaxed);
                    inner.bus.publish(RuntimeEvent::SafetyTriggered {
                        kind: "emergency_write".into(),
                        detail: reason.clone(),
                        at_ms: ohm_core::now_ms(),
                    });
                    value = Value::Number(forced);
                    clamped = true;
                    note = Some(reason);
                }
                SafetyDecision::Allow {
                    value: allowed,
                    clamped: clamp_note,
                } => {
                    if let Some(note_text) = clamp_note {
                        clamped = true;
                        value = Value::Number(allowed);
                        note = Some(note_text);
                    }
                }
            }
        }

        // 4. Dry run: audit and publish, but never touch the hardware.
        let adapter = inner.discovery.adapter(device.adapter.as_str());
        if settings.dry_run {
            let report = WriteReport {
                at_ms: ohm_core::now_ms(),
                device_id: device.id.clone(),
                device_name: device.name.clone(),
                device_type: device.device_type,
                capability: capability.id.clone(),
                capability_name: capability.name.clone(),
                requested,
                applied: Some(value),
                status: WriteStatus::Simulated,
                clamped,
                error_code: None,
                detail: Some("dry run: hardware was not touched".into()),
                origin,
                simulated: true,
            };
            self.finish(report.clone());
            return Ok(report);
        }

        let Some(adapter) = adapter else {
            return Err(self.reject(
                &device,
                &capability,
                &requested,
                origin,
                OhmError::AdapterUnavailable {
                    adapter: device.adapter.to_string(),
                    detail: "adapter is not registered".into(),
                },
            ));
        };

        // 5. Ask the hardware, holding the write gate so rules cannot interleave.
        let outcome = {
            let _gate = inner.write_gate.lock().await;
            adapter.write(&device, &capability, &value).await
        };

        match outcome {
            Ok(outcome) => {
                // Only a confirmed outcome may carry a value. Filling an unknown
                // result in with the requested number is how a report ends up
                // claiming a fan speed that was never verified — the same mistake
                // the adapter layer had, one layer down.
                let applied = match outcome.status {
                    WriteStatus::Applied | WriteStatus::Simulated => outcome.applied.clone(),
                    WriteStatus::Unconfirmed | WriteStatus::Rejected => None,
                };
                let report = WriteReport {
                    at_ms: ohm_core::now_ms(),
                    device_id: device.id.clone(),
                    device_name: device.name.clone(),
                    device_type: device.device_type,
                    capability: capability.id.clone(),
                    capability_name: capability.name.clone(),
                    requested,
                    applied: applied.clone(),
                    status: outcome.status,
                    clamped: clamped || applied.as_ref().is_some_and(|applied| applied != &value),
                    error_code: None,
                    detail: outcome.detail.clone().or_else(|| note.clone()),
                    origin,
                    simulated: outcome.status == WriteStatus::Simulated,
                };
                self.finish(report.clone());
                Ok(report)
            }
            Err(err) => {
                let rejected = self.reject(&device, &capability, &requested, origin.clone(), err);
                // Fail safe: a failed write to a duty control must not leave the
                // output at whatever the caller hoped for.
                if allow_fail_safe && capability.is_duty_control() {
                    let fail_safe = settings.safety.fail_safe_duty(device.device_type);
                    let origin = WriteOrigin::Safety {
                        reason: format!("fallback after a failed write: {rejected}"),
                    };
                    if let Err(second) = Box::pin(self.write_with_origin(
                        device.id.as_str(),
                        capability.id.as_str(),
                        Value::Number(fail_safe),
                        origin,
                        false,
                    ))
                    .await
                    {
                        tracing::error!(
                            device = device.id.as_str(),
                            capability = capability.id.as_str(),
                            error = %second,
                            "fail-safe write also failed: control may be lost"
                        );
                    }
                }
                Err(rejected)
            }
        }
    }

    /// Record, publish and count a completed write.
    fn finish(&self, report: WriteReport) {
        let inner = &self.inner;
        match report.status {
            WriteStatus::Rejected => {
                inner.stats.writes_rejected.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                inner.stats.writes_applied.fetch_add(1, Ordering::Relaxed);
            }
        }
        inner.audit.record(&report);
        inner.bus.publish(RuntimeEvent::WritePerformed {
            report: Box::new(report),
        });
    }

    /// Audit + publish a refusal and hand the error back.
    fn reject(
        &self,
        device: &Device,
        capability: &Capability,
        requested: &Value,
        origin: WriteOrigin,
        error: OhmError,
    ) -> OhmError {
        let inner = &self.inner;
        let report = WriteReport {
            at_ms: ohm_core::now_ms(),
            device_id: device.id.clone(),
            device_name: device.name.clone(),
            device_type: device.device_type,
            capability: capability.id.clone(),
            capability_name: capability.name.clone(),
            requested: requested.clone(),
            applied: None,
            status: WriteStatus::Rejected,
            clamped: false,
            error_code: Some(error.code().to_string()),
            detail: Some(error.to_string()),
            origin,
            simulated: false,
        };
        inner.stats.writes_rejected.fetch_add(1, Ordering::Relaxed);
        inner.audit.record(&report);
        inner.bus.publish(RuntimeEvent::WriteRejected {
            device_id: device.id.clone(),
            capability: capability.id.clone(),
            error_code: error.code().to_string(),
            detail: error.to_string(),
        });
        error
    }

    /// Drive every controlled output to the configured fail-safe duty.
    ///
    /// This is what "hand the fans back safely" means for the MVP. The returned
    /// [`ControlRelease`] keeps three claims apart — the duty was *written*, the
    /// device *read it back*, and the adapter *gave ownership back* — because an
    /// unconfirmed write is not a release and must never be counted as one.
    pub async fn release_control(&self) -> ControlRelease {
        let settings = self.settings();
        if !settings.safety.relinquish_on_exit {
            return ControlRelease {
                skipped_by_config: true,
                ..ControlRelease::default()
            };
        }
        let targets: Vec<(DeviceId, String, CapabilityId, f64)> = {
            let table = self.inner.table.read();
            table
                // Every channel this runtime can drive — not just fans and pumps:
                // a GPU fan is a `Gpu` device with a writable duty control, and
                // skipping it left it at its last commanded value on exit.
                .releasable_devices()
                .into_iter()
                .filter(|r| r.enabled)
                .flat_map(|r| {
                    let duty = settings.safety.fail_safe_duty(r.device.device_type);
                    r.device
                        .capabilities
                        .iter()
                        .filter(|c| c.is_duty_control() && c.writable)
                        .map(|c| {
                            (
                                r.device.id.clone(),
                                r.device.name.clone(),
                                c.id.clone(),
                                duty,
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect()
        };

        let mut release = ControlRelease::default();
        for (device_id, device_name, capability_id, duty) in targets {
            let origin = WriteOrigin::Shutdown;
            match self
                .write_with_origin(
                    device_id.as_str(),
                    capability_id.as_str(),
                    Value::Number(duty),
                    origin,
                    false,
                )
                .await
            {
                Ok(report) => release.push(ReleasedChannel {
                    device_id: report.device_id.clone(),
                    device_name: report.device_name.clone(),
                    capability: report.capability.clone(),
                    duty,
                    applied: report.applied.clone(),
                    status: report.status,
                    detail: report.detail.clone(),
                }),
                Err(err) => {
                    tracing::warn!(
                        device = device_id.as_str(),
                        capability = capability_id.as_str(),
                        duty,
                        error = %err,
                        "the fail-safe duty was not applied on shutdown"
                    );
                    release.push_failure(
                        ReleasedChannel {
                            device_id,
                            device_name,
                            capability: capability_id,
                            duty,
                            applied: None,
                            status: WriteStatus::Rejected,
                            detail: Some(err.to_string()),
                        },
                        err.code(),
                    );
                }
            }
        }
        release
    }

    // ----------------------------------------------------------------- reads

    /// Everything the UI needs for one frame.
    pub fn snapshot(&self) -> RuntimeSnapshot {
        let table = self.inner.table.read();
        let devices = table.views(&self.inner.store);
        let has_controllable = devices.iter().any(DeviceView::is_controllable);
        RuntimeSnapshot {
            generated_at_ms: ohm_core::now_ms(),
            started_at_ms: self.inner.started_at_ms.load(Ordering::Relaxed),
            devices,
            adapters: self.inner.discovery.views(),
            settings: self.settings(),
            stats: self.inner.stats.snapshot(self.inner.bus.published()),
            has_controllable_hardware: has_controllable,
        }
    }

    pub fn devices(&self) -> Vec<DeviceView> {
        let table = self.inner.table.read();
        table.views(&self.inner.store)
    }

    pub fn device(&self, id: &str) -> Option<DeviceView> {
        let table = self.inner.table.read();
        let record = table.get_str(id)?;
        Some(DeviceView {
            device: record.device.clone(),
            enabled: record.enabled,
            status: record.status,
            state: self
                .inner
                .store
                .latest(&record.device.id)
                .map(|s| (*s).clone()),
            adapter: record.adapter.to_string(),
            first_seen_ms: record.first_seen_ms,
            last_seen_ms: record.last_seen_ms,
        })
    }

    /// The device description plus one capability, both owned.
    pub fn resolve(&self, device: &str, capability: &str) -> Result<(Device, Capability)> {
        let table = self.inner.table.read();
        let target = CapabilityRegistry::resolve(&table, device, capability)?;
        Ok((target.record.device.clone(), target.capability.clone()))
    }

    pub fn state(&self, id: &str) -> Option<DeviceState> {
        self.inner
            .store
            .latest(&DeviceId::new_unchecked(id))
            .map(|s| (*s).clone())
    }

    pub fn history(&self, device: &str, capability: &str, limit: usize) -> Vec<Sample> {
        self.inner.store.history(device, capability, limit)
    }

    /// Hottest temperature currently known, in Celsius.
    pub fn hottest_temperature(&self) -> Option<f64> {
        hottest_temperature(&self.inner)
    }

    /// Latest numeric value of a capability, for rule evaluation.
    pub fn reading(&self, device: &str, capability: &str) -> Option<f64> {
        self.inner.store.number(device, capability)
    }

    /// `true` when the last poll was longer ago than the runtime expects.
    pub fn is_stale(&self, tolerance_factor: u64) -> bool {
        let last = self.inner.stats.last_poll_ms.load(Ordering::Relaxed);
        if last == 0 {
            return true;
        }
        let interval = self.settings().polling_interval_ms;
        ohm_core::now_ms() - last > (interval * tolerance_factor.max(1)) as i64
    }

    // -------------------------------------------------------------- settings

    pub fn settings(&self) -> Settings {
        self.inner.settings.read().clone()
    }

    /// Mutate settings and persist them. Returns the sanitised result.
    pub fn update_settings(&self, mutate: impl FnOnce(&mut Settings)) -> Result<Settings> {
        let updated = {
            let mut guard = self.inner.settings.write();
            mutate(&mut guard);
            guard.sanitise();
            guard.clone()
        };
        self.inner.store.set_history_limit(updated.history_points);
        self.inner.settings_store.save(&updated)?;
        self.inner.bus.publish(RuntimeEvent::Log {
            level: "info".into(),
            message: "settings saved".into(),
            at_ms: ohm_core::now_ms(),
        });
        Ok(updated)
    }

    /// Enable or disable a device and persist the choice.
    pub fn set_device_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let device_id = DeviceId::new(id)?;
        {
            let mut table = self.inner.table.write();
            table.set_enabled(&device_id, enabled)?;
        }
        self.update_settings(|settings| settings.set_device_enabled(&device_id, enabled))?;
        self.inner.bus.publish(RuntimeEvent::DeviceEnabledChanged {
            device_id: device_id.clone(),
            enabled,
        });
        if let Some(record) = self.inner.table.read().get(&device_id) {
            self.inner.bus.publish(RuntimeEvent::DeviceStatusChanged {
                device_id,
                status: record.status,
            });
        }
        Ok(())
    }

    /// Enable or disable a whole provider.
    pub fn set_adapter_enabled(&self, id: &str, enabled: bool) -> Result<Settings> {
        let adapter_id = AdapterId::new(id)?;
        self.update_settings(|settings| settings.set_adapter_enabled(&adapter_id, enabled))
    }

    // --------------------------------------------------------------- plumbing

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RuntimeEvent> {
        self.inner.bus.subscribe()
    }

    pub fn bus(&self) -> &EventBus {
        &self.inner.bus
    }

    pub fn audit(&self) -> &AuditLog {
        &self.inner.audit
    }

    pub fn paths(&self) -> &ConfigPaths {
        &self.inner.paths
    }

    pub fn adapter(&self, id: &str) -> Option<Arc<dyn HardwareAdapter>> {
        self.inner.discovery.adapter(id)
    }

    pub fn adapter_views(&self) -> Vec<AdapterView> {
        self.inner.discovery.views()
    }

    /// Current index of readable sensors and writable actuators.
    pub fn capability_index(&self) -> crate::registry::CapabilityIndex {
        let table = self.inner.table.read();
        CapabilityRegistry::index(&table)
    }

    pub fn stats(&self) -> RuntimeStats {
        self.inner.stats.snapshot(self.inner.bus.published())
    }

    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::Relaxed)
    }

    pub fn device_count(&self) -> usize {
        self.inner.table.read().len()
    }

    /// Publish a user visible log line (used by the automation engine).
    pub fn log(&self, level: &str, message: impl Into<String>) {
        let message = message.into();
        match level {
            "error" => tracing::error!("{message}"),
            "warn" => tracing::warn!("{message}"),
            "debug" => tracing::debug!("{message}"),
            _ => tracing::info!("{message}"),
        }
        self.inner.bus.publish(RuntimeEvent::Log {
            level: level.to_string(),
            message,
            at_ms: ohm_core::now_ms(),
        });
    }

    /// Publish an automation event on the shared bus.
    pub fn publish_automation(
        &self,
        rule_id: Option<ohm_core::RuleId>,
        kind: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.inner.bus.publish(RuntimeEvent::Automation {
            rule_id,
            kind: kind.into(),
            detail: detail.into(),
            at_ms: ohm_core::now_ms(),
        });
    }
}

/// Devices owned by a reconcile diff, used to seed the state store.
fn diff_devices(table: &DeviceTable, ids: &[DeviceId]) -> Vec<Device> {
    ids.iter()
        .filter_map(|id| table.get(id).map(|r| r.device.clone()))
        .collect()
}

/// Hottest temperature across every enabled device, in Celsius.
fn hottest_temperature(inner: &Arc<RuntimeInner>) -> Option<f64> {
    let units = inner.table.read().units_map();
    let mut hottest: Option<f64> = None;
    for state in inner.store.all() {
        for reading in &state.readings {
            let Some(unit) = units.get(&reading.capability) else {
                continue;
            };
            if !unit.is_temperature() {
                continue;
            }
            let Some(value) = reading.number() else {
                continue;
            };
            let Some(celsius) = unit.to_celsius(value) else {
                continue;
            };
            hottest = Some(match hottest {
                Some(current) => current.max(celsius),
                None => celsius,
            });
        }
    }
    hottest
}

/// How many devices are currently online.
pub fn online_devices(devices: &[DeviceView]) -> usize {
    devices
        .iter()
        .filter(|d| d.status == DeviceStatus::Online || d.status == DeviceStatus::Degraded)
        .count()
}

/// Unit of a capability, if the device declares one.
pub fn capability_unit(devices: &[DeviceView], device: &str, capability: &str) -> Option<Unit> {
    devices
        .iter()
        .find(|d| d.device.id.as_str() == device)
        .and_then(|d| d.device.capability_str(capability))
        .map(|c| c.unit)
}

/// Grouping helper used by the Overview page (kept here so the UI does not
/// depend on the device model internals).
pub fn group_by_type(
    devices: &[DeviceView],
) -> Vec<(ohm_device_model::DeviceType, Vec<&DeviceView>)> {
    use ohm_device_model::DeviceType;
    let mut out = Vec::new();
    for device_type in DeviceType::ALL {
        let group: Vec<&DeviceView> = devices
            .iter()
            .filter(|d| d.device.device_type == device_type)
            .collect();
        if !group.is_empty() {
            out.push((device_type, group));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ohm_adapter_api::{AdapterCapabilities, AdapterInfo, AdapterStatus, WriteOutcome};
    use ohm_device_model::{Capability, DeviceType, Reading, Transport, Unit};
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    /// An adapter that counts how often it was polled and asks for a slow cadence.
    struct SlowAdapter {
        polls: Arc<AtomicUsize>,
        hint_ms: u64,
    }

    #[async_trait]
    impl HardwareAdapter for SlowAdapter {
        fn info(&self) -> AdapterInfo {
            AdapterInfo::new("slow", "Slow", "slow").with_capabilities(AdapterCapabilities {
                can_write: false,
                can_control_cooling: false,
                write_requires_admin: false,
                hands_back_control_on_shutdown: false,
                poll_interval_ms: Some(self.hint_ms),
                discovery_interval_ms: None,
            })
        }

        async fn probe(&self) -> AdapterStatus {
            AdapterStatus::available(self.id(), 1)
        }

        async fn discover(&self) -> Result<Vec<Device>> {
            Ok(vec![
                Device::new(
                    DeviceId::new("temperature.slow.0").unwrap(),
                    "Slow Sensor",
                    DeviceType::TemperatureSensor,
                    Transport::Mock,
                    self.id(),
                )
                .with_capability(Capability::sensor(
                    "temperature.core",
                    "Temperature",
                    Unit::Celsius,
                )),
            ])
        }

        async fn read_state(&self, device: &Device) -> Result<DeviceState> {
            self.polls.fetch_add(1, AtomicOrdering::Relaxed);
            Ok(DeviceState::new(device.id.clone(), ohm_core::now_ms())
                .with_reading(Reading::ok("temperature.core", 42.0)))
        }

        async fn write(
            &self,
            _device: &Device,
            _capability: &Capability,
            _value: &Value,
        ) -> Result<WriteOutcome> {
            Ok(WriteOutcome::rejected("read-only"))
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[tokio::test]
    async fn an_adapter_hint_slows_its_own_polling_without_affecting_others() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::from_root(temp.path());
        let polls = Arc::new(AtomicUsize::new(0));
        let adapter: Arc<dyn HardwareAdapter> = Arc::new(SlowAdapter {
            polls: Arc::clone(&polls),
            hint_ms: 60_000,
        });

        let runtime = Runtime::new(paths, Settings::default(), vec![adapter]).unwrap();
        runtime.refresh_devices().await.unwrap();

        // The first cycle always polls; an immediate second one must not.
        runtime.poll_once().await.unwrap();
        assert_eq!(polls.load(AtomicOrdering::Relaxed), 1);
        runtime.poll_once().await.unwrap();
        assert_eq!(
            polls.load(AtomicOrdering::Relaxed),
            1,
            "the adapter asked for a 60 s cadence and must be respected"
        );

        // The state is still served from the store, so the UI is not starved.
        assert_eq!(
            runtime.reading("temperature.slow.0", "temperature.core"),
            Some(42.0)
        );
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn an_adapter_without_a_hint_is_polled_every_cycle() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::from_root(temp.path());
        let polls = Arc::new(AtomicUsize::new(0));
        let adapter: Arc<dyn HardwareAdapter> = Arc::new(SlowAdapter {
            polls: Arc::clone(&polls),
            hint_ms: 0, // no hint: Some(0) still means "no cadence requested"
        });
        let runtime = Runtime::new(paths, Settings::default(), vec![adapter]).unwrap();
        runtime.refresh_devices().await.unwrap();
        runtime.poll_once().await.unwrap();
        runtime.poll_once().await.unwrap();
        assert_eq!(polls.load(AtomicOrdering::Relaxed), 2);
        runtime.shutdown().await.unwrap();
    }
}
