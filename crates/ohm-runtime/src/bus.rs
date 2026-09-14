//! The runtime event bus.
//!
//! One `tokio::sync::broadcast` channel carries every state change, device
//! hotplug, write and safety decision. The UI subscribes to it; the automation
//! engine both publishes and consumes it.
//!
//! Events are *notifications*: the authoritative state always lives in
//! [`StateStore`](crate::store::StateStore), so a slow or lagging subscriber
//! can never corrupt anything.

use std::sync::atomic::{AtomicU64, Ordering};

use ohm_core::{AdapterId, CapabilityId, DeviceId, RuleId};
use ohm_device_model::{Device, DeviceState, UnavailableReason, Value};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::snapshot::DeviceStatus;

/// Capacity of the broadcast channel. Subscribers that fall further behind than
/// this receive [`broadcast::error::RecvError::Lagged`] and resynchronise from
/// the state store.
pub const EVENT_CHANNEL_CAPACITY: usize = 2048;

/// Something that happened in the runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEvent {
    RuntimeStarted {
        at_ms: i64,
        device_count: usize,
    },
    RuntimeStopped {
        at_ms: i64,
    },
    AdapterStatusChanged {
        status: crate::snapshot::AdapterView,
    },
    DeviceAdded {
        device: Box<Device>,
    },
    DeviceRemoved {
        device_id: DeviceId,
        name: String,
        reason: UnavailableReason,
    },
    DeviceStatusChanged {
        device_id: DeviceId,
        status: DeviceStatus,
    },
    DeviceEnabledChanged {
        device_id: DeviceId,
        enabled: bool,
    },
    /// A full poll result. Published once per device per poll cycle.
    StateChanged {
        state: Box<DeviceState>,
    },
    /// A single value changed, used for charts and cheap UI updates.
    ReadingChanged {
        device_id: DeviceId,
        capability: CapabilityId,
        value: Option<Value>,
        previous: Option<Value>,
    },
    /// A write reached (or was refused by) the hardware.
    WritePerformed {
        report: Box<crate::audit::WriteReport>,
    },
    /// A write never reached the hardware: range, permission or safety error.
    WriteRejected {
        device_id: DeviceId,
        capability: CapabilityId,
        error_code: String,
        detail: String,
    },
    /// The safety supervisor acted (emergency curve, sensor loss fallback, ...).
    SafetyTriggered {
        kind: String,
        detail: String,
        at_ms: i64,
    },
    /// Automation engine news, so the engine does not need its own bus.
    Automation {
        /// `None` for engine-level events (started, stopped, config reloaded).
        rule_id: Option<RuleId>,
        kind: String,
        detail: String,
        at_ms: i64,
    },
    /// A user visible log line (Settings -> Logs).
    Log {
        level: String,
        message: String,
        at_ms: i64,
    },
}

impl RuntimeEvent {
    /// Short tag used by the UI and by tests.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::RuntimeStarted { .. } => "runtime_started",
            Self::RuntimeStopped { .. } => "runtime_stopped",
            Self::AdapterStatusChanged { .. } => "adapter_status_changed",
            Self::DeviceAdded { .. } => "device_added",
            Self::DeviceRemoved { .. } => "device_removed",
            Self::DeviceStatusChanged { .. } => "device_status_changed",
            Self::DeviceEnabledChanged { .. } => "device_enabled_changed",
            Self::StateChanged { .. } => "state_changed",
            Self::ReadingChanged { .. } => "reading_changed",
            Self::WritePerformed { .. } => "write_performed",
            Self::WriteRejected { .. } => "write_rejected",
            Self::SafetyTriggered { .. } => "safety_triggered",
            Self::Automation { .. } => "automation",
            Self::Log { .. } => "log",
        }
    }
}

/// Fan-out bus for [`RuntimeEvent`]s.
#[derive(Debug)]
pub struct EventBus {
    sender: broadcast::Sender<RuntimeEvent>,
    published: AtomicU64,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(EVENT_CHANNEL_CAPACITY)
    }
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _receiver) = broadcast::channel(capacity.max(16));
        Self {
            sender,
            published: AtomicU64::new(0),
        }
    }

    /// Publish an event. Returns the number of receivers it reached; a zero is
    /// normal (nobody is listening) and never an error.
    pub fn publish(&self, event: RuntimeEvent) -> usize {
        self.published.fetch_add(1, Ordering::Relaxed);
        tracing::trace!(kind = event.kind(), "runtime event");
        self.sender.send(event).unwrap_or(0)
    }

    /// Subscribe to future events.
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.sender.subscribe()
    }

    /// Number of receivers currently attached.
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Total number of published events, used by diagnostics.
    pub fn published(&self) -> u64 {
        self.published.load(Ordering::Relaxed)
    }
}

/// Convenience constructor for adapter status events.
pub fn adapter_event(view: crate::snapshot::AdapterView) -> RuntimeEvent {
    RuntimeEvent::AdapterStatusChanged { status: view }
}

/// Convenience constructor for a safety event.
pub fn safety_event(kind: impl Into<String>, detail: impl Into<String>) -> RuntimeEvent {
    RuntimeEvent::SafetyTriggered {
        kind: kind.into(),
        detail: detail.into(),
        at_ms: ohm_core::now_ms(),
    }
}

/// Convenience constructor for an adapter id payload.
pub fn adapter_id(id: &str) -> AdapterId {
    AdapterId::new_unchecked(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_core::DeviceId;

    #[tokio::test]
    async fn publish_reaches_subscribers() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();
        let sent = bus.publish(RuntimeEvent::DeviceRemoved {
            device_id: DeviceId::new("fan.system.0").unwrap(),
            name: "Chassis Fan".into(),
            reason: UnavailableReason::NotPresent,
        });
        assert_eq!(sent, 1);
        let received = rx.recv().await.unwrap();
        assert_eq!(received.kind(), "device_removed");
        assert_eq!(bus.published(), 1);
    }

    #[tokio::test]
    async fn publish_without_subscribers_is_not_an_error() {
        let bus = EventBus::new(16);
        assert_eq!(bus.publish(RuntimeEvent::RuntimeStopped { at_ms: 1 }), 0);
        assert_eq!(bus.receiver_count(), 0);
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let bus = EventBus::new(16);
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        assert_eq!(bus.publish(safety_event("emergency", "90C")), 2);
        assert_eq!(a.recv().await.unwrap().kind(), "safety_triggered");
        assert_eq!(b.recv().await.unwrap().kind(), "safety_triggered");
    }

    #[test]
    fn event_json_is_tagged() {
        let event = RuntimeEvent::Log {
            level: "info".into(),
            message: "hello".into(),
            at_ms: 42,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "log");
        assert_eq!(json["level"], "info");
    }
}
