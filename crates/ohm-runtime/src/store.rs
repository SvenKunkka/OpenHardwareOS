//! The state store: the authoritative, always-available view of hardware state.
//!
//! Two things live here:
//!
//! * **latest** — the most recent [`DeviceState`] per device, replaced on every
//!   poll. UI reads never block on hardware.
//! * **history** — a bounded ring buffer of numeric samples per capability,
//!   which is what the Overview charts draw.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ohm_core::{CapabilityId, DeviceId};
use ohm_device_model::{DeviceState, Reading, ReadingStatus, Unit, Value};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// One numeric sample in the history ring.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub at_ms: i64,
    pub value: f64,
}

/// A change detected between two polls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingChange {
    pub device_id: DeviceId,
    pub capability: CapabilityId,
    pub previous: Option<Value>,
    pub current: Option<Value>,
    /// `true` when the value appeared or disappeared (status transition).
    pub transition: bool,
}

/// How much a reading must move before it counts as a change.
///
/// Sensors jitter; publishing every 0.01 °C would flood the UI and the event
/// bus. The epsilon is unit aware.
pub fn reading_epsilon(unit: Unit) -> f64 {
    match unit {
        Unit::Celsius | Unit::Fahrenheit => 0.1,
        Unit::Percent | Unit::Pwm => 0.5,
        Unit::Rpm => 1.0,
        Unit::Watt | Unit::Milliwatt => 0.5,
        Unit::Volt | Unit::Ampere => 0.01,
        Unit::Hertz | Unit::Megahertz => 1.0,
        Unit::Byte => 1.0,
        Unit::Second | Unit::Millisecond => 0.001,
        _ => 0.0,
    }
}

/// `true` when two values differ enough to be worth publishing.
pub fn value_changed(previous: Option<&Value>, current: Option<&Value>, unit: Unit) -> bool {
    match (previous, current) {
        (None, None) => false,
        (Some(_), None) | (None, Some(_)) => true,
        (Some(a), Some(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => (a - b).abs() >= reading_epsilon(unit).max(f64::EPSILON),
            _ => a != b,
        },
    }
}

/// Latest state plus bounded history.
#[derive(Debug)]
pub struct StateStore {
    latest: RwLock<HashMap<DeviceId, Arc<DeviceState>>>,
    history: RwLock<HashMap<(DeviceId, CapabilityId), VecDeque<Sample>>>,
    history_limit: AtomicUsize,
}

impl Default for StateStore {
    fn default() -> Self {
        Self::new(180)
    }
}

impl StateStore {
    /// Create a store keeping at most `history_limit` samples per capability.
    pub fn new(history_limit: usize) -> Self {
        Self {
            latest: RwLock::new(HashMap::new()),
            history: RwLock::new(HashMap::new()),
            history_limit: AtomicUsize::new(history_limit.max(1)),
        }
    }

    /// Change the history depth. Existing longer histories are trimmed lazily.
    pub fn set_history_limit(&self, limit: usize) {
        self.history_limit.store(limit.max(1), Ordering::Relaxed);
    }

    pub fn history_limit(&self) -> usize {
        self.history_limit.load(Ordering::Relaxed)
    }

    /// Store a new state, returning the readings that changed.
    ///
    /// `units` maps capability id to unit so the change detection can use the
    /// right epsilon. Capabilities missing from the map fall back to an exact
    /// comparison.
    pub fn update(
        &self,
        state: DeviceState,
        units: &HashMap<CapabilityId, Unit>,
    ) -> Vec<ReadingChange> {
        let device_id = state.device.clone();
        let at_ms = state.timestamp_ms;
        let limit = self.history_limit();

        let mut changes = Vec::new();
        {
            let mut history = self.history.write();
            for reading in &state.readings {
                let unit = units
                    .get(&reading.capability)
                    .copied()
                    .unwrap_or(Unit::None);
                let previous = {
                    let latest = self.latest.read();
                    latest
                        .get(&device_id)
                        .and_then(|s| s.get(reading.capability.as_str()).cloned())
                };
                let previous_value = previous.as_ref().and_then(|r| r.value()).cloned();
                let current_value = match &reading.status {
                    ReadingStatus::Ok { value } => Some(value.clone()),
                    ReadingStatus::Unavailable { .. } => None,
                };
                let transition = previous.is_some()
                    && (previous.as_ref().is_some_and(Reading::is_ok) != reading.is_ok());

                if value_changed(previous_value.as_ref(), current_value.as_ref(), unit)
                    || transition
                {
                    changes.push(ReadingChange {
                        device_id: device_id.clone(),
                        capability: reading.capability.clone(),
                        previous: previous_value,
                        current: current_value.clone(),
                        transition,
                    });
                }

                if let Some(value) = current_value.and_then(|v| v.as_f64()) {
                    let entry = history
                        .entry((device_id.clone(), reading.capability.clone()))
                        .or_default();
                    entry.push_back(Sample { at_ms, value });
                    while entry.len() > limit {
                        entry.pop_front();
                    }
                }
            }
        }

        self.latest.write().insert(device_id, Arc::new(state));
        changes
    }

    /// The most recent state of a device.
    pub fn latest(&self, device: &DeviceId) -> Option<Arc<DeviceState>> {
        self.latest.read().get(device).cloned()
    }

    /// Every known state, newest poll first is not guaranteed: ordered by id.
    pub fn all(&self) -> Vec<Arc<DeviceState>> {
        let mut states: Vec<Arc<DeviceState>> = self.latest.read().values().cloned().collect();
        states.sort_by(|a, b| a.device.cmp(&b.device));
        states
    }

    /// Numeric value of one capability, if known.
    pub fn number(&self, device: &str, capability: &str) -> Option<f64> {
        self.latest
            .read()
            .get(&DeviceId::new_unchecked(device))
            .and_then(|s| s.number(capability))
    }

    /// History for one capability, oldest first.
    pub fn history(&self, device: &str, capability: &str, limit: usize) -> Vec<Sample> {
        let key = (
            DeviceId::new_unchecked(device),
            CapabilityId::new_unchecked(capability),
        );
        let history = self.history.read();
        let Some(samples) = history.get(&key) else {
            return Vec::new();
        };
        let skip = samples.len().saturating_sub(limit);
        samples.iter().skip(skip).copied().collect()
    }

    /// Drop everything we know about a device (it was unplugged).
    pub fn remove(&self, device: &DeviceId) -> Option<Arc<DeviceState>> {
        self.history.write().retain(|(id, _), _| id != device);
        self.latest.write().remove(device)
    }

    pub fn clear(&self) {
        self.latest.write().clear();
        self.history.write().clear();
    }

    pub fn device_count(&self) -> usize {
        self.latest.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_device_model::UnavailableReason;

    fn units() -> HashMap<CapabilityId, Unit> {
        HashMap::from([
            (
                CapabilityId::new("temperature.core").unwrap(),
                Unit::Celsius,
            ),
            (CapabilityId::new("fan.rpm").unwrap(), Unit::Rpm),
        ])
    }

    fn state(temp: f64, rpm: i64, at_ms: i64) -> DeviceState {
        DeviceState::new(DeviceId::new("gpu.mock.0").unwrap(), at_ms)
            .with_reading(Reading::ok("temperature.core", temp))
            .with_reading(Reading::ok("fan.rpm", rpm))
    }

    #[test]
    fn first_update_reports_every_reading_as_new() {
        let store = StateStore::new(10);
        let changes = store.update(state(60.0, 1000, 1), &units());
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| c.previous.is_none()));
        assert_eq!(store.device_count(), 1);
    }

    #[test]
    fn small_jitter_does_not_publish_a_change() {
        let store = StateStore::new(10);
        store.update(state(60.0, 1000, 1), &units());
        let changes = store.update(state(60.05, 1000, 2), &units());
        assert!(changes.is_empty(), "jitter below epsilon: {changes:?}");
        let changes = store.update(state(60.2, 1001, 3), &units());
        assert_eq!(changes.len(), 2, "real moves are published");
    }

    #[test]
    fn history_is_bounded_and_ordered() {
        let store = StateStore::new(3);
        for i in 0..10 {
            store.update(state(50.0 + f64::from(i), 1000, i64::from(i)), &units());
        }
        let samples = store.history("gpu.mock.0", "temperature.core", 10);
        assert_eq!(samples.len(), 3);
        assert_eq!(samples.last().unwrap().value, 59.0);
        assert!(samples.windows(2).all(|w| w[0].at_ms <= w[1].at_ms));
        assert_eq!(store.history("gpu.mock.0", "temperature.core", 2).len(), 2);
        assert!(store.history("nope", "temperature.core", 2).is_empty());
    }

    #[test]
    fn transition_to_unavailable_is_a_change() {
        let store = StateStore::new(10);
        store.update(state(60.0, 1000, 1), &units());
        let dropped = DeviceState::new(DeviceId::new("gpu.mock.0").unwrap(), 2).with_reading(
            Reading::unavailable(
                "temperature.core",
                UnavailableReason::ReadError,
                Some("sensor lost".to_string()),
            ),
        );
        let changes = store.update(dropped, &units());
        assert!(changes.iter().any(|c| c.transition && c.current.is_none()));
        assert_eq!(store.number("gpu.mock.0", "temperature.core"), None);
    }

    #[test]
    fn latest_returns_an_arc_snapshot() {
        let store = StateStore::new(10);
        store.update(state(60.0, 1000, 1), &units());
        let latest = store.latest(&DeviceId::new("gpu.mock.0").unwrap()).unwrap();
        assert_eq!(latest.number("fan.rpm"), Some(1000.0));
        assert_eq!(store.all().len(), 1);
        assert_eq!(store.number("gpu.mock.0", "fan.rpm"), Some(1000.0));
    }

    #[test]
    fn remove_forgets_history() {
        let store = StateStore::new(10);
        store.update(state(60.0, 1000, 1), &units());
        let device = DeviceId::new("gpu.mock.0").unwrap();
        assert!(store.remove(&device).is_some());
        assert_eq!(store.device_count(), 0);
        assert!(
            store
                .history("gpu.mock.0", "temperature.core", 10)
                .is_empty()
        );

        store.update(state(60.0, 1000, 2), &units());
        store.clear();
        assert_eq!(store.device_count(), 0);
    }

    #[test]
    fn epsilon_is_unit_aware() {
        assert_eq!(reading_epsilon(Unit::Celsius), 0.1);
        assert_eq!(reading_epsilon(Unit::Rpm), 1.0);
        assert!(value_changed(
            Some(&Value::Integer(1000)),
            Some(&Value::Integer(1001)),
            Unit::Rpm
        ));
        assert!(!value_changed(
            Some(&Value::Integer(1000)),
            Some(&Value::Integer(1000)),
            Unit::Rpm
        ));
        assert!(value_changed(None, Some(&Value::Number(1.0)), Unit::None));
        assert!(!value_changed(None, None, Unit::None));
    }
}
