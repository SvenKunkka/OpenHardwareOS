//! The device table: what is currently attached, who owns it and whether the
//! user enabled it.
//!
//! Reconciliation lives here because hotplug detection is a *table* operation:
//! discovery returns the devices that exist right now, and the table works out
//! what was added, what vanished and what changed.

use std::collections::{BTreeMap, HashMap};

use ohm_core::{AdapterId, CapabilityId, DeviceId, OhmError, Result};
use ohm_device_model::{Device, DeviceState, DeviceType, UnavailableReason, Unit};
use serde::{Deserialize, Serialize};

use crate::snapshot::{DeviceStatus, DeviceView};
use crate::store::StateStore;

/// A device plus runtime bookkeeping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub device: Device,
    pub adapter: AdapterId,
    pub enabled: bool,
    pub status: DeviceStatus,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    /// Reason the device is offline, when it is.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub offline_reason: Option<UnavailableReason>,
}

impl DeviceRecord {
    pub fn id(&self) -> &DeviceId {
        &self.device.id
    }

    pub fn is_online(&self) -> bool {
        self.status == DeviceStatus::Online || self.status == DeviceStatus::Degraded
    }
}

/// Result of inserting one device description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upsert {
    Added,
    /// Description changed (capabilities or metadata).
    Updated,
    /// Same description, only bookkeeping moved.
    Unchanged,
}

/// What reconciliation changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReconcileDiff {
    pub added: Vec<DeviceId>,
    pub removed: Vec<(DeviceId, String, UnavailableReason)>,
    pub updated: Vec<DeviceId>,
}

impl ReconcileDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.updated.is_empty()
    }
}

/// The set of devices the runtime knows about.
#[derive(Debug, Default, Clone)]
pub struct DeviceTable {
    devices: BTreeMap<DeviceId, DeviceRecord>,
}

impl DeviceTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.devices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &DeviceRecord> {
        self.devices.values()
    }

    pub fn ids(&self) -> Vec<DeviceId> {
        self.devices.keys().cloned().collect()
    }

    pub fn get(&self, id: &DeviceId) -> Option<&DeviceRecord> {
        self.devices.get(id)
    }

    pub fn get_str(&self, id: &str) -> Option<&DeviceRecord> {
        self.devices.get(&DeviceId::new_unchecked(id))
    }

    pub fn get_mut(&mut self, id: &DeviceId) -> Option<&mut DeviceRecord> {
        self.devices.get_mut(id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.get_str(id).is_some()
    }

    /// Devices owned by one adapter.
    pub fn by_adapter(&self, adapter: &AdapterId) -> Vec<&DeviceRecord> {
        self.devices
            .values()
            .filter(|r| &r.adapter == adapter)
            .collect()
    }

    /// Devices of one type, ordered by id for stable UI rendering.
    pub fn of_type(&self, device_type: DeviceType) -> Vec<&DeviceRecord> {
        self.devices
            .values()
            .filter(|r| r.device.device_type == device_type)
            .collect()
    }

    /// Devices that declare a capability, optionally only writable ones.
    pub fn with_capability(&self, capability: &str, writable_only: bool) -> Vec<&DeviceRecord> {
        self.devices
            .values()
            .filter(|r| match r.device.capability_str(capability) {
                Some(cap) => !writable_only || cap.writable,
                None => false,
            })
            .collect()
    }

    /// Devices an automation rule can drive.
    pub fn controllable(&self) -> Vec<&DeviceRecord> {
        self.devices
            .values()
            .filter(|r| r.device.is_controllable())
            .collect()
    }

    /// Members of the cooling loop (fans and pumps), enabled or not.
    pub fn cooling_devices(&self) -> Vec<&DeviceRecord> {
        self.devices
            .values()
            .filter(|r| matches!(r.device.device_type, DeviceType::Fan | DeviceType::Pump))
            .collect()
    }

    /// Every device whose duty this runtime can drive, whatever its type.
    ///
    /// This is what the exit path must release. [`Self::cooling_devices`] is the
    /// fan/pump list, and a GPU is a `Gpu` device with a writable fan channel:
    /// leaving it out meant a GPU fan under our control kept its last duty while
    /// the audit said every output had been handed back.
    pub fn releasable_devices(&self) -> Vec<&DeviceRecord> {
        self.devices
            .values()
            .filter(|r| {
                r.device
                    .capabilities
                    .iter()
                    .any(|c| c.is_duty_control() && c.writable)
            })
            .collect()
    }

    /// Enabled devices as owned clones, ready to hand to an adapter.
    pub fn poll_targets(&self) -> Vec<Device> {
        self.devices
            .values()
            .filter(|r| r.enabled)
            .map(|r| r.device.clone())
            .collect()
    }

    /// Capability units, used by the state store to pick a change epsilon.
    pub fn units_map(&self) -> HashMap<CapabilityId, Unit> {
        let mut map = HashMap::new();
        for record in self.devices.values() {
            for capability in &record.device.capabilities {
                map.insert(capability.id.clone(), capability.unit);
            }
        }
        map
    }

    /// Insert or refresh a device.
    pub fn upsert(&mut self, device: Device, enabled: bool, now_ms: i64) -> Upsert {
        match self.devices.get_mut(&device.id) {
            Some(existing) => {
                existing.last_seen_ms = now_ms;
                existing.offline_reason = None;
                if existing.status == DeviceStatus::Offline {
                    existing.status = DeviceStatus::Online;
                }
                if existing.device == device {
                    Upsert::Unchanged
                } else {
                    // Keep the original discovery timestamp, refresh the rest.
                    existing.device = device;
                    Upsert::Updated
                }
            }
            None => {
                let id = device.id.clone();
                let adapter = device.adapter.clone();
                self.devices.insert(
                    id,
                    DeviceRecord {
                        device,
                        adapter,
                        enabled,
                        status: if enabled {
                            DeviceStatus::Online
                        } else {
                            DeviceStatus::Disabled
                        },
                        first_seen_ms: now_ms,
                        last_seen_ms: now_ms,
                        offline_reason: None,
                    },
                );
                Upsert::Added
            }
        }
    }

    pub fn remove(&mut self, id: &DeviceId) -> Option<DeviceRecord> {
        self.devices.remove(id)
    }

    pub fn set_enabled(&mut self, id: &DeviceId, enabled: bool) -> Result<()> {
        let record = self
            .devices
            .get_mut(id)
            .ok_or_else(|| OhmError::DeviceNotFound(id.to_string()))?;
        record.enabled = enabled;
        if !enabled {
            record.status = DeviceStatus::Disabled;
        } else if record.status == DeviceStatus::Disabled {
            record.status = DeviceStatus::Online;
        }
        Ok(())
    }

    pub fn set_status(
        &mut self,
        id: &DeviceId,
        status: DeviceStatus,
        reason: Option<UnavailableReason>,
    ) {
        if let Some(record) = self.devices.get_mut(id) {
            record.status = if record.enabled {
                status
            } else {
                DeviceStatus::Disabled
            };
            record.offline_reason = reason;
            record.last_seen_ms = ohm_core::now_ms();
        }
    }

    /// Mark a device offline because its adapter stopped reporting it.
    pub fn mark_offline(&mut self, id: &DeviceId, reason: UnavailableReason) {
        self.set_status(id, DeviceStatus::Offline, Some(reason));
    }

    /// Reconcile one adapter's discovery result against the table.
    ///
    /// * devices that appeared are inserted and reported in `added`
    /// * devices that vanished are removed and reported in `removed`
    /// * devices whose capability set changed are reported in `updated`
    pub fn reconcile(
        &mut self,
        adapter: &AdapterId,
        discovered: Vec<Device>,
        settings: &crate::config::Settings,
        now_ms: i64,
    ) -> ReconcileDiff {
        let mut diff = ReconcileDiff::default();
        let mut seen: Vec<DeviceId> = Vec::with_capacity(discovered.len());

        for device in discovered {
            let id = device.id.clone();
            seen.push(id.clone());
            let enabled = settings.device_enabled(id.as_str());
            match self.upsert(device, enabled, now_ms) {
                Upsert::Added => diff.added.push(id),
                Upsert::Updated => diff.updated.push(id),
                Upsert::Unchanged => {}
            }
        }

        let gone: Vec<(DeviceId, String)> = self
            .devices
            .values()
            .filter(|r| &r.adapter == adapter && !seen.contains(r.id()))
            .map(|r| (r.id().clone(), r.device.name.clone()))
            .collect();
        for (id, name) in gone {
            self.devices.remove(&id);
            diff.removed.push((id, name, UnavailableReason::NotPresent));
        }

        diff
    }

    /// Mark every device of an adapter offline (adapter failed or disappeared).
    pub fn mark_adapter_offline(&mut self, adapter: &AdapterId, reason: UnavailableReason) {
        let ids: Vec<DeviceId> = self
            .devices
            .values()
            .filter(|r| &r.adapter == adapter)
            .map(|r| r.id().clone())
            .collect();
        for id in ids {
            self.mark_offline(&id, reason);
        }
    }

    /// Update the coarse status of a device from a fresh poll result.
    pub fn apply_state_status(&mut self, state: &DeviceState) {
        // A device is only `Online` when *every* declared capability could be
        // read. Anything else is honestly reported as `Degraded`, with the
        // per-reading reason explaining which part is missing.
        let status = if !state.online {
            DeviceStatus::Offline
        } else if state.readings.iter().any(|r| !r.is_ok()) {
            DeviceStatus::Degraded
        } else {
            DeviceStatus::Online
        };
        self.set_status(&state.device, status, None);
    }

    /// Project the table into UI friendly views.
    pub fn views(&self, store: &StateStore) -> Vec<DeviceView> {
        self.devices
            .values()
            .map(|record| DeviceView {
                device: record.device.clone(),
                enabled: record.enabled,
                status: record.status,
                state: store.latest(&record.device.id).map(|s| (*s).clone()),
                adapter: record.adapter.to_string(),
                first_seen_ms: record.first_seen_ms,
                last_seen_ms: record.last_seen_ms,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use ohm_device_model::{Capability, Reading, Transport};

    fn fan(index: usize) -> Device {
        Device::new(
            DeviceId::compose("fan", "system", index),
            format!("Chassis Fan {}", index + 1),
            DeviceType::Fan,
            Transport::System,
            AdapterId::new("system").unwrap(),
        )
        .with_capability(Capability::sensor("fan.rpm", "Fan RPM", Unit::Rpm))
        .with_capability(Capability::actuator(
            "fan.speed_percent",
            "Fan Speed",
            Unit::Percent,
            0.0,
            100.0,
        ))
    }

    #[test]
    fn upsert_adds_then_updates() {
        let mut table = DeviceTable::new();
        assert_eq!(table.upsert(fan(0), true, 1), Upsert::Added);
        assert_eq!(table.len(), 1);
        assert_eq!(table.upsert(fan(0), true, 2), Upsert::Unchanged);
        let mut renamed = fan(0);
        renamed.name = "Front Fan".into();
        assert_eq!(table.upsert(renamed, true, 3), Upsert::Updated);
        assert_eq!(
            table.get_str("fan.system.0").unwrap().device.name,
            "Front Fan"
        );
        assert_eq!(table.get_str("fan.system.0").unwrap().first_seen_ms, 1);
        assert_eq!(table.get_str("fan.system.0").unwrap().last_seen_ms, 3);
    }

    #[test]
    fn reconcile_detects_hotplug() {
        let mut table = DeviceTable::new();
        let settings = Settings::default();
        let adapter = AdapterId::new("system").unwrap();

        let diff = table.reconcile(&adapter, vec![fan(0), fan(1)], &settings, 10);
        assert_eq!(diff.added.len(), 2);
        assert!(diff.removed.is_empty());

        // Fan 1 unplugged, fan 2 appears.
        let diff = table.reconcile(&adapter, vec![fan(0), fan(2)], &settings, 20);
        assert_eq!(diff.added, vec![DeviceId::new("fan.system.2").unwrap()]);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0].0.as_str(), "fan.system.1");
        assert_eq!(diff.removed[0].2, UnavailableReason::NotPresent);
        assert!(diff.updated.is_empty());

        // A changed capability set counts as an update, not a re-add.
        let mut upgraded = fan(0);
        upgraded.capabilities.push(Capability::sensor(
            "temperature.system",
            "System Temperature",
            Unit::Celsius,
        ));
        let diff = table.reconcile(&adapter, vec![upgraded], &settings, 30);
        assert_eq!(diff.updated, vec![DeviceId::new("fan.system.0").unwrap()]);
        assert!(diff.added.is_empty());
    }

    #[test]
    fn reconcile_respects_disabled_devices() {
        let mut table = DeviceTable::new();
        let mut settings = Settings::default();
        settings.set_device_enabled(&DeviceId::new("fan.system.0").unwrap(), false);
        table.reconcile(
            &AdapterId::new("system").unwrap(),
            vec![fan(0), fan(1)],
            &settings,
            1,
        );
        assert!(!table.get_str("fan.system.0").unwrap().enabled);
        assert_eq!(
            table.get_str("fan.system.0").unwrap().status,
            DeviceStatus::Disabled
        );
        assert_eq!(
            table.poll_targets().len(),
            1,
            "disabled devices are not polled"
        );
    }

    #[test]
    fn status_transitions() {
        let mut table = DeviceTable::new();
        let id = DeviceId::new("fan.system.0").unwrap();
        table.upsert(fan(0), true, 1);
        assert_eq!(table.get(&id).unwrap().status, DeviceStatus::Online);

        let state = DeviceState::new(id.clone(), 2).with_reading(Reading::ok("fan.rpm", 1200));
        table.apply_state_status(&state);
        assert_eq!(table.get(&id).unwrap().status, DeviceStatus::Online);

        let partial = DeviceState::new(id.clone(), 3)
            .with_reading(Reading::ok("fan.rpm", 1200))
            .with_reading(Reading::unavailable(
                "fan.speed_percent",
                UnavailableReason::PermissionDenied,
                Some("run as admin".to_string()),
            ));
        table.apply_state_status(&partial);
        assert_eq!(table.get(&id).unwrap().status, DeviceStatus::Degraded);

        table.mark_adapter_offline(
            &AdapterId::new("system").unwrap(),
            UnavailableReason::NotPresent,
        );
        assert_eq!(table.get(&id).unwrap().status, DeviceStatus::Offline);
        assert_eq!(
            table.get(&id).unwrap().offline_reason,
            Some(UnavailableReason::NotPresent)
        );

        // Disabling wins over every other status.
        table.set_enabled(&id, false).unwrap();
        table.apply_state_status(&state);
        assert_eq!(table.get(&id).unwrap().status, DeviceStatus::Disabled);
        assert!(
            table
                .set_enabled(&DeviceId::new("nope").unwrap(), true)
                .is_err()
        );
    }

    #[test]
    fn views_carry_state_and_adapter() {
        let mut table = DeviceTable::new();
        table.upsert(fan(0), true, 1);
        let store = StateStore::new(10);
        store.update(
            DeviceState::new(DeviceId::new("fan.system.0").unwrap(), 2)
                .with_reading(Reading::ok("fan.rpm", 1100)),
            &table.units_map(),
        );
        let views = table.views(&store);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].adapter, "system");
        assert_eq!(
            views[0].state.as_ref().unwrap().number("fan.rpm"),
            Some(1100.0)
        );
    }

    #[test]
    fn capability_queries() {
        let mut table = DeviceTable::new();
        table.reconcile(
            &AdapterId::new("system").unwrap(),
            vec![fan(0), fan(1)],
            &Settings::default(),
            1,
        );
        assert_eq!(table.with_capability("fan.rpm", false).len(), 2);
        assert_eq!(table.with_capability("fan.speed_percent", true).len(), 2);
        assert_eq!(table.with_capability("temperature.core", false).len(), 0);
        assert_eq!(table.controllable().len(), 2);
        assert_eq!(table.cooling_devices().len(), 2);
        assert_eq!(table.of_type(DeviceType::Gpu).len(), 0);
        assert_eq!(
            table.by_adapter(&AdapterId::new("system").unwrap()).len(),
            2
        );
        assert_eq!(table.units_map().len(), 2);
        assert_eq!(table.ids().len(), 2);
    }
}
