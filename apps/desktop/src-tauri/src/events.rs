//! Bridging runtime events to the webview.
//!
//! Two streams reach the UI:
//!
//! * `runtime-event` — the verbatim tagged [`RuntimeEvent`], used for the live
//!   activity feed and for targeted refreshes;
//! * `snapshot` — a full [`RuntimeSnapshot`], rate limited to 4 Hz, which is the
//!   UI's primary data source.
//!
//! Rate limiting matters: a poll cycle can publish a dozen events, and pushing a
//! snapshot per event would make the webview do far more work than the hardware
//! polling itself.

use std::time::{Duration, Instant};

use ohm_runtime::{RuntimeEvent, RuntimeSnapshot};
use tauri::{AppHandle, Emitter, Runtime};

/// Minimum gap between two pushed snapshots.
pub const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(250);
/// Heartbeat interval, so the UI stays fresh even when nothing changes.
pub const HEARTBEAT: Duration = Duration::from_secs(1);

/// Push a snapshot to the webview.
pub fn emit_snapshot<R: Runtime>(app: &AppHandle<R>, runtime: &ohm_runtime::Runtime) {
    let snapshot: RuntimeSnapshot = runtime.snapshot();
    if let Err(error) = app.emit("snapshot", &snapshot) {
        tracing::debug!(error = %error, "could not push a snapshot to the UI");
    }
}

/// Push one runtime event.
pub fn emit_event<R: Runtime>(app: &AppHandle<R>, event: &RuntimeEvent) {
    if let Err(error) = app.emit("runtime-event", event) {
        tracing::debug!(error = %error, "could not push a runtime event to the UI");
    }
}

/// Should this event trigger an immediate snapshot?
///
/// Structural changes must be visible instantly; value changes can wait for the
/// rate limiter.
pub fn is_structural(event: &RuntimeEvent) -> bool {
    matches!(
        event,
        RuntimeEvent::DeviceAdded { .. }
            | RuntimeEvent::DeviceRemoved { .. }
            | RuntimeEvent::DeviceEnabledChanged { .. }
            | RuntimeEvent::AdapterStatusChanged { .. }
            | RuntimeEvent::RuntimeStarted { .. }
            | RuntimeEvent::RuntimeStopped { .. }
    )
}

/// Spawn the bridge. Returns immediately; the task ends when the runtime does.
pub fn spawn<R: Runtime>(app: AppHandle<R>, runtime: ohm_runtime::Runtime) {
    let mut receiver = runtime.subscribe();

    tauri::async_runtime::spawn(async move {
        let mut last_snapshot = Instant::now()
            .checked_sub(SNAPSHOT_INTERVAL)
            .unwrap_or_else(Instant::now);
        let mut ticker = tokio::time::interval(HEARTBEAT);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                received = receiver.recv() => match received {
                    Ok(event) => {
                        emit_event(&app, &event);
                        let structural = is_structural(&event);
                        if structural || last_snapshot.elapsed() >= SNAPSHOT_INTERVAL {
                            emit_snapshot(&app, &runtime);
                            last_snapshot = Instant::now();
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        // The UI was slower than the bus: resynchronise from the
                        // authoritative state instead of replaying events.
                        tracing::debug!(missed, "UI fell behind the event bus, resynchronising");
                        emit_snapshot(&app, &runtime);
                        last_snapshot = Instant::now();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = ticker.tick() => {
                    emit_snapshot(&app, &runtime);
                    last_snapshot = Instant::now();
                }
            }
        }
        tracing::debug!("event bridge stopped");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_core::DeviceId;
    use ohm_device_model::UnavailableReason;

    #[test]
    fn structural_events_bypass_the_rate_limiter() {
        assert!(is_structural(&RuntimeEvent::DeviceRemoved {
            device_id: DeviceId::new("fan.system.0").unwrap(),
            name: "Chassis Fan".into(),
            reason: UnavailableReason::NotPresent,
        }));
        assert!(is_structural(&RuntimeEvent::RuntimeStarted {
            at_ms: 0,
            device_count: 1
        }));
        assert!(!is_structural(&RuntimeEvent::Log {
            level: "info".into(),
            message: "hello".into(),
            at_ms: 0
        }));
        assert!(!is_structural(&RuntimeEvent::ReadingChanged {
            device_id: DeviceId::new("fan.system.0").unwrap(),
            capability: ohm_core::CapabilityId::new("fan.rpm").unwrap(),
            value: None,
            previous: None
        }));
    }

    #[test]
    fn intervals_are_sane() {
        assert!(SNAPSHOT_INTERVAL >= Duration::from_millis(100));
        assert!(SNAPSHOT_INTERVAL <= HEARTBEAT);
    }
}
