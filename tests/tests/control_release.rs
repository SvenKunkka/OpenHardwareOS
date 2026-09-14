//! What "hand the fans back" actually reports.
//!
//! The exit path has separate claims to make, and it used to make only one:
//!
//! 1. the fail-safe duty was **written**;
//! 2. the device **read it back** — without that, the value is unknown;
//! 3. the adapter **gave ownership back** to the firmware.
//!
//! `release_control()` returned a count of writes it had *attempted*, and the
//! shutdown path recorded that number as "outputs handed back to firmware". A
//! write the adapter accepted but could never read back — a board that answers
//! `200 OK` and has no readable channel, which is exactly what real hardware
//! produces — was therefore counted as a successful release. These tests pin
//! the claims apart, and pin the failures as failures.
//!
//! Reading the code for those tests also found a second defect: the exit path
//! released `Fan | Pump` devices only, so a **GPU fan** under the runtime's
//! control kept its last duty while the audit said everything had been handed
//! back. `every_writable_duty_channel_is_released_including_the_gpu_fan` is the
//! regression test for that.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use ohm_adapter_api::{
    AdapterCapabilities, AdapterInfo, AdapterStatus, HardwareAdapter, WriteOutcome,
};
use ohm_adapter_mock::{MockAdapter, MockConfig, MockFaults};
use ohm_core::{CapabilityId, ConfigPaths, DeviceId};
use ohm_device_model::{
    Capability, Device, DeviceState, DeviceType, Reading, Transport, Unit, Value,
};
use ohm_runtime::{ControlRelease, Runtime, Settings};

/// The duty channels the exit path drives, read from the running machine so the
/// test cannot drift from the real target list.
fn duty_channels(runtime: &Runtime) -> Vec<(String, String)> {
    runtime
        .snapshot()
        .devices
        .iter()
        .filter(|view| view.enabled)
        .flat_map(|view| {
            view.device
                .capabilities
                .iter()
                .filter(|capability| capability.is_duty_control() && capability.writable)
                .map(|capability| {
                    (
                        view.device.id.as_str().to_string(),
                        capability.id.as_str().to_string(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Lifecycle entries as `(action, detail)`, oldest first.
fn lifecycle(runtime: &Runtime) -> Vec<(String, String)> {
    runtime
        .audit()
        .tail(400)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            let action = entry.get("action")?.as_str()?.to_string();
            if !action.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                return None;
            }
            let detail = entry
                .get("detail")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            Some((action, detail))
        })
        .collect()
}

fn detail_of<'a>(entries: &'a [(String, String)], action: &str) -> Option<&'a str> {
    entries
        .iter()
        .rev()
        .find(|(name, _)| name == action)
        .map(|(_, detail)| detail.as_str())
}

/// The simulated machine, alone: every write it accepts is a *simulated* one.
async fn mock_runtime(faults: MockFaults) -> (tempfile::TempDir, Runtime) {
    let temp = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::from_root(temp.path());
    let config = MockConfig {
        faults,
        ..MockConfig::deterministic()
    };
    let adapter: Arc<dyn HardwareAdapter> = Arc::new(MockAdapter::new(config));
    let runtime = Runtime::new(paths, Settings::default(), vec![adapter]).unwrap();
    runtime.start().await.unwrap();
    (temp, runtime)
}

/// A fan that answers honestly, with configurable honesty about the hand-back.
struct TestFan {
    id: &'static str,
    /// `true` when its shutdown reports a failure instead of a hand-back.
    fails_shutdown: bool,
    /// `true` when it declares that its shutdown really restores firmware control.
    hands_back: bool,
}

#[async_trait]
impl HardwareAdapter for TestFan {
    fn info(&self) -> AdapterInfo {
        let capabilities = AdapterCapabilities {
            can_write: true,
            can_control_cooling: true,
            write_requires_admin: false,
            hands_back_control_on_shutdown: self.hands_back,
            poll_interval_ms: None,
            discovery_interval_ms: None,
        };
        AdapterInfo::new(self.id, "Test Fan Controller", self.id)
            .with_capabilities(capabilities)
    }

    async fn probe(&self) -> AdapterStatus {
        AdapterStatus::available(self.id(), 1)
    }

    async fn discover(&self) -> ohm_core::Result<Vec<Device>> {
        Ok(vec![
            Device::new(
                DeviceId::new(format!("fan.{}.0", self.id)).unwrap(),
                "Test Fan",
                DeviceType::Fan,
                Transport::Mock,
                self.id(),
            )
            .with_capability(Capability::actuator(
                "fan.speed_percent",
                "Fan Speed",
                Unit::Percent,
                0.0,
                100.0,
            )),
        ])
    }

    async fn read_state(&self, device: &Device) -> ohm_core::Result<DeviceState> {
        Ok(DeviceState::new(device.id.clone(), ohm_core::now_ms())
            .with_reading(Reading::ok("fan.speed_percent", 40.0)))
    }

    async fn write(
        &self,
        _device: &Device,
        _capability: &Capability,
        value: &Value,
    ) -> ohm_core::Result<WriteOutcome> {
        Ok(WriteOutcome::applied(Value::Number(
            value.as_f64().unwrap_or_default(),
        )))
    }

    async fn shutdown(&self) -> ohm_core::Result<()> {
        if self.fails_shutdown {
            // The firmware hand-back failed: the channel may still be ours.
            return Err(ohm_core::OhmError::Adapter {
                adapter: self.id().to_string(),
                detail: "the firmware refused to take the channel back".to_string(),
            });
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

async fn fan_runtime(fan: TestFan) -> (tempfile::TempDir, Runtime) {
    let temp = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::from_root(temp.path());
    let adapter: Arc<dyn HardwareAdapter> = Arc::new(fan);
    let runtime = Runtime::new(paths, Settings::default(), vec![adapter]).unwrap();
    runtime.start().await.unwrap();
    (temp, runtime)
}

/// A confirmed write is a release, and the audit says what was confirmed.
#[tokio::test]
async fn a_confirmed_fail_safe_write_is_reported_as_a_release() {
    let (_temp, runtime) = fan_runtime(TestFan {
        id: "honest",
        fails_shutdown: false,
        hands_back: true,
    })
    .await;
    let channels = duty_channels(&runtime);
    assert_eq!(channels.len(), 1, "the fixture fan has one duty channel");

    let release: ControlRelease = runtime.shutdown().await.expect("shutdown");

    assert_eq!(release.attempted(), 1, "{}", release.summary());
    assert_eq!(release.confirmed.len(), 1, "{}", release.summary());
    assert!(release.unconfirmed.is_empty(), "{}", release.summary());
    assert!(release.rejected.is_empty(), "{}", release.summary());
    assert!(release.failed.is_empty(), "{}", release.summary());
    assert!(release.is_clean());
    assert!(release.problems().is_empty());
    assert!(release.caveats().is_empty());
    assert_eq!(
        release.confirmed[0].applied,
        Some(Value::Number(release.confirmed[0].duty)),
        "a confirmed release carries the value it was confirmed at"
    );

    let entries = lifecycle(&runtime);
    let released = detail_of(&entries, "control_released").expect("control_released entry");
    assert!(
        released.contains("confirmed at the fail-safe duty"),
        "the entry must claim only what was read back: {released}"
    );
    assert_eq!(detail_of(&entries, "runtime_stopped"), Some("clean shutdown"));
    assert!(detail_of(&entries, "control_release_unconfirmed").is_none());
    assert_eq!(
        detail_of(&entries, "control_relinquished"),
        Some("1 cooling adapter(s) reported handing control back to the firmware: honest")
    );
}

/// The exit path must release every channel it can drive — a GPU fan included.
#[tokio::test]
async fn every_writable_duty_channel_is_released_including_the_gpu_fan() {
    let (_temp, runtime) = mock_runtime(MockFaults::default()).await;
    let channels = duty_channels(&runtime);
    assert!(
        channels.iter().any(|(device, _)| device.starts_with("gpu.")),
        "the simulated machine must expose a GPU fan channel: {channels:?}"
    );

    let release = runtime.shutdown().await.expect("shutdown");

    assert_eq!(
        release.attempted(),
        channels.len(),
        "every channel this runtime can drive must be released: {} (channels: {channels:?})",
        release.summary()
    );
    // Simulated hardware reports simulated writes: the report must not call them
    // confirmed, and must not call them problems either.
    assert_eq!(release.simulated.len(), channels.len(), "{}", release.summary());
    assert!(release.confirmed.is_empty(), "{}", release.summary());
    assert!(release.is_clean(), "{}", release.summary());
    assert!(release.problems().is_empty());
    assert_eq!(
        detail_of(&lifecycle(&runtime), "runtime_stopped"),
        Some("clean shutdown")
    );
}

/// The defect: an accepted-but-unreadable write was counted as a release.
#[tokio::test]
async fn an_unconfirmed_fail_safe_write_is_never_reported_as_released() {
    // The fault list is built from the running machine, so it covers every
    // channel the exit path will touch.
    let (_temp_probe, probe) = mock_runtime(MockFaults::default()).await;
    let channels = duty_channels(&probe);
    probe.shutdown().await.expect("probe shutdown");
    assert!(!channels.is_empty());

    let faults = MockFaults {
        unconfirmed_writes_on: channels
            .iter()
            .map(|(device, capability)| {
                (
                    DeviceId::new_unchecked(device),
                    CapabilityId::new_unchecked(capability),
                )
            })
            .collect(),
        ..MockFaults::default()
    };
    let (_temp, runtime) = mock_runtime(faults).await;

    let release = runtime.shutdown().await.expect("shutdown");

    assert_eq!(release.attempted(), channels.len(), "{}", release.summary());
    assert!(
        release.confirmed.is_empty(),
        "nothing was read back, so nothing may be called released: {}",
        release.summary()
    );
    assert_eq!(release.unconfirmed.len(), channels.len(), "{}", release.summary());
    assert!(!release.is_clean());
    assert_eq!(release.problems().len(), channels.len());
    assert!(release.summary().contains("not read back"));

    let entries = lifecycle(&runtime);
    assert!(
        detail_of(&entries, "control_released").is_none(),
        "an unconfirmed write must not produce a release entry"
    );
    let unconfirmed = detail_of(&entries, "control_release_unconfirmed").expect("entry");
    assert!(
        unconfirmed.contains("not a confirmed release"),
        "the entry must refuse the claim: {unconfirmed}"
    );
    let stopped = detail_of(&entries, "runtime_stopped").expect("runtime_stopped");
    assert!(
        stopped.starts_with("shutdown with control problems"),
        "the shutdown must not be recorded as clean: {stopped}"
    );
}

/// A device that refuses the duty is refused — not "released" and not "failed".
#[tokio::test]
async fn a_device_that_refuses_the_duty_is_reported_as_refused() {
    let (_temp_probe, probe) = mock_runtime(MockFaults::default()).await;
    let channels = duty_channels(&probe);
    probe.shutdown().await.expect("probe shutdown");

    let (_temp, runtime) = mock_runtime(MockFaults::writes_fail()).await;
    let release = runtime.shutdown().await.expect("shutdown");

    assert!(release.confirmed.is_empty(), "{}", release.summary());
    assert_eq!(release.rejected.len(), channels.len(), "{}", release.summary());
    assert!(release.failed.is_empty(), "{}", release.summary());
    assert!(release.simulated.is_empty(), "{}", release.summary());
    assert_eq!(release.problems().len(), channels.len());

    let entries = lifecycle(&runtime);
    assert!(detail_of(&entries, "control_released").is_none());
    let failed = detail_of(&entries, "control_release_failed").expect("entry");
    assert!(failed.contains("refused"), "{failed}");
    assert!(detail_of(&entries, "control_release_unconfirmed").is_none());
}

/// `relinquish_on_exit = false` means nothing is written — and nothing is claimed.
#[tokio::test]
async fn turning_off_relinquish_on_exit_writes_nothing_and_says_so() {
    let (_temp, runtime) = mock_runtime(MockFaults::default()).await;
    runtime
        .update_settings(|settings| settings.safety.relinquish_on_exit = false)
        .expect("settings update");

    let release = runtime.shutdown().await.expect("shutdown");

    assert!(release.skipped_by_config);
    assert_eq!(release.attempted(), 0);
    assert!(release.confirmed.is_empty());
    assert!(release.is_clean(), "nothing was attempted, so nothing is wrong");
    assert!(release.summary().contains("skipped"));

    let entries = lifecycle(&runtime);
    assert!(detail_of(&entries, "control_release_skipped").is_some());
    assert!(detail_of(&entries, "control_released").is_none());

    let shutdown_writes = runtime
        .audit()
        .tail(400)
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| entry.get("origin").and_then(|o| o.as_str()) == Some("shutdown"))
        .count();
    assert_eq!(shutdown_writes, 0, "no write may happen when release is off");
}

/// An adapter that keeps its channels is reported, not counted as a hand-back.
///
/// This is the NVML case: it declares cooling control, its `shutdown` is the
/// trait default (`Ok(())` — it does nothing), and the channel it drove keeps the
/// last duty. The old code counted that `Ok` as "handed back to firmware".
#[tokio::test]
async fn an_adapter_that_does_not_hand_control_back_is_reported() {
    let (_temp, runtime) = fan_runtime(TestFan {
        id: "clingy",
        fails_shutdown: false,
        hands_back: false,
    })
    .await;

    let release = runtime.shutdown().await.expect("shutdown");

    assert_eq!(release.confirmed.len(), 1, "{}", release.summary());
    assert!(
        release.relinquished.is_empty(),
        "an adapter that does not hand back must not be listed as one: {}",
        release.summary()
    );
    assert_eq!(release.without_hand_back.len(), 1);
    assert_eq!(release.without_hand_back[0].as_str(), "clingy");
    assert!(release.shutdown_failed.is_empty());
    assert_eq!(release.caveats().len(), 1);
    assert!(release.caveats()[0].contains("clingy"));
    // It is a caveat, not a failure: the value is confirmed, so it is not a problem.
    assert!(release.problems().is_empty());

    let entries = lifecycle(&runtime);
    let not_handed = detail_of(&entries, "control_not_handed_back").expect("entry");
    assert!(not_handed.contains("clingy"), "{not_handed}");
    assert!(
        not_handed.contains("only release"),
        "the entry must say what the release actually was: {not_handed}"
    );
    assert!(detail_of(&entries, "control_relinquished").is_none());
}

/// A failed hand-back is a failure, and it is named with its reason.
#[tokio::test]
async fn an_adapter_that_cannot_hand_control_back_is_reported_separately() {
    let (_temp, runtime) = fan_runtime(TestFan {
        id: "stubborn",
        fails_shutdown: true,
        hands_back: true,
    })
    .await;

    let release = runtime.shutdown().await.expect("shutdown");

    // The value itself was written and read back...
    assert_eq!(release.confirmed.len(), 1, "{}", release.summary());
    assert!(release.unconfirmed.is_empty(), "{}", release.summary());
    // ...but the adapter could not give the channel back.
    assert_eq!(release.shutdown_failed.len(), 1, "{}", release.summary());
    assert_eq!(release.shutdown_failed[0].adapter.as_str(), "stubborn");
    assert!(
        release.shutdown_failed[0].detail.contains("refused to take"),
        "the reason must survive: {}",
        release.shutdown_failed[0].detail
    );
    assert!(release.relinquished.is_empty());
    assert!(!release.is_clean());
    assert!(
        release
            .problems()
            .iter()
            .any(|problem| problem.contains("did not shut down cleanly"))
    );

    let entries = lifecycle(&runtime);
    let shutdown_failed = detail_of(&entries, "adapter_shutdown_failed").expect("entry");
    assert!(shutdown_failed.contains("stubborn"), "{shutdown_failed}");
    assert!(shutdown_failed.contains("refused to take"), "{shutdown_failed}");
    let stopped = detail_of(&entries, "runtime_stopped").expect("runtime_stopped");
    assert!(
        stopped.starts_with("shutdown with control problems"),
        "a failed hand-back is not a clean shutdown: {stopped}"
    );
}
