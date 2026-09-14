//! Which provider reports a device is decided by what is actually there.
//!
//! NVML and LibreHardwareMonitor describe the same physical GPU, so the bundled
//! adapter set makes one of them a fallback for the other. That relationship used
//! to be applied from the *settings*: NVML was not constructed at all when
//! `adapter_settings` enabled LHM. On Linux, or on Windows with
//! LibreHardwareMonitor not running, that left the machine with no GPU provider —
//! and nothing in the UI explained why.
//!
//! Now the fallback declares the relationship (`AdapterInfo::yields_to`) and the
//! runtime decides from the probe result, on every discovery cycle. These tests
//! drive that cycle with two providers whose availability is under the test's
//! control, and assert what the user ends up seeing: one GPU, never two, and a
//! status that says why the other one is quiet.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use ohm_adapter_api::{
    AdapterCapabilities, AdapterInfo, AdapterState, AdapterStatus, HardwareAdapter, WriteOutcome,
};
use ohm_core::{AdapterId, ConfigPaths, DeviceId};
use ohm_device_model::{
    Capability, Device, DeviceState, DeviceType, Reading, Transport, UnavailableReason, Unit, Value,
};
use ohm_runtime::{DeviceStatus, Runtime, Settings};

/// A GPU provider that can be switched off and on while the test runs.
struct FakeGpu {
    id: &'static str,
    available: Arc<AtomicBool>,
    yields_to: Option<&'static str>,
}

impl FakeGpu {
    fn new(id: &'static str, yields_to: Option<&'static str>) -> (Arc<Self>, Arc<AtomicBool>) {
        let available = Arc::new(AtomicBool::new(true));
        (
            Arc::new(Self {
                id,
                available: Arc::clone(&available),
                yields_to,
            }),
            available,
        )
    }
}

#[async_trait]
impl HardwareAdapter for FakeGpu {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::new(self.id, format!("{} GPU provider", self.id), self.id)
            .with_capabilities(AdapterCapabilities::read_only())
            .yielding_to(self.yields_to.map(AdapterId::new_unchecked))
    }

    async fn probe(&self) -> AdapterStatus {
        if self.available.load(Ordering::SeqCst) {
            AdapterStatus::available(self.id(), 1)
        } else {
            AdapterStatus::unavailable(
                self.id(),
                UnavailableReason::NotPresent,
                "no driver on this machine",
            )
        }
    }

    async fn discover(&self) -> ohm_core::Result<Vec<Device>> {
        Ok(vec![
            Device::new(
                DeviceId::new(format!("gpu.{}.0", self.id)).unwrap(),
                format!("GPU from {}", self.id),
                DeviceType::Gpu,
                Transport::System,
                self.id(),
            )
            .with_capability(Capability::sensor(
                "temperature.core",
                "GPU Core",
                Unit::Celsius,
            )),
        ])
    }

    async fn read_state(&self, device: &Device) -> ohm_core::Result<DeviceState> {
        Ok(DeviceState::new(device.id.clone(), ohm_core::now_ms())
            .with_reading(Reading::ok("temperature.core", 42.0)))
    }

    async fn write(
        &self,
        _device: &Device,
        _capability: &Capability,
        _value: &Value,
    ) -> ohm_core::Result<WriteOutcome> {
        Ok(WriteOutcome::rejected("this provider is read-only"))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A runtime with a primary provider and one that stands by for it.
async fn rig() -> (tempfile::TempDir, Runtime, Arc<AtomicBool>, Arc<AtomicBool>) {
    let temp = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::from_root(temp.path());
    let (primary, primary_available) = FakeGpu::new("primary", None);
    let (fallback, fallback_available) = FakeGpu::new("fallback", Some("primary"));
    let adapters: Vec<Arc<dyn HardwareAdapter>> = vec![
        Arc::clone(&fallback) as Arc<dyn HardwareAdapter>,
        Arc::clone(&primary) as Arc<dyn HardwareAdapter>,
    ];
    let runtime = Runtime::new(paths, Settings::default(), adapters).unwrap();
    runtime.start().await.unwrap();
    (temp, runtime, primary_available, fallback_available)
}

fn devices(runtime: &Runtime) -> Vec<String> {
    runtime
        .snapshot()
        .devices
        .iter()
        .map(|view| view.device.id.to_string())
        .collect()
}

/// Status of one device. A device whose provider stops reporting it stays in the
/// model as `Offline` — that is the hotplug design, and it is what the user sees —
/// so "is it reporting" is a status question, not an existence question.
fn device_status(runtime: &Runtime, id: &str) -> Option<DeviceStatus> {
    runtime
        .snapshot()
        .devices
        .iter()
        .find(|view| view.device.id.as_str() == id)
        .map(|view| view.status)
}

fn adapter_state(runtime: &Runtime, id: &str) -> (AdapterState, Option<String>) {
    let snapshot = runtime.snapshot();
    let view = snapshot
        .adapters
        .iter()
        .find(|view| view.id() == id)
        .unwrap_or_else(|| panic!("{id} is registered"));
    (view.status.state, view.status.detail.clone())
}

/// A usable primary means the fallback reports nothing — and says why.
#[tokio::test]
async fn a_usable_primary_keeps_the_fallback_quiet() {
    let (_temp, runtime, _primary, _fallback) = rig().await;

    let found = devices(&runtime);
    assert!(
        found.iter().any(|id| id == "gpu.primary.0"),
        "the primary must report its device: {found:?}"
    );
    assert!(
        !found.iter().any(|id| id == "gpu.fallback.0"),
        "the same physical GPU must not appear twice: {found:?}"
    );

    let (state, detail) = adapter_state(&runtime, "fallback");
    assert_eq!(state, AdapterState::Unavailable);
    let detail = detail.expect("the fallback says why it is quiet");
    assert!(detail.contains("standing by"), "{detail}");
    assert!(detail.contains("primary"), "{detail}");
    assert!(
        detail.contains("adapter_settings.fallback.always = true"),
        "the way to force it must be in the message: {detail}"
    );
}

/// The defect: with the primary merely *enabled* (not running), the fallback was
/// dropped too, so the user saw no GPU at all.
#[tokio::test]
async fn a_primary_that_is_not_there_hands_over_to_the_fallback() {
    let (_temp, runtime, primary_available, _fallback) = rig().await;
    primary_available.store(false, Ordering::SeqCst);

    runtime.refresh_devices().await.expect("discovery cycle");

    assert_eq!(
        device_status(&runtime, "gpu.fallback.0"),
        Some(DeviceStatus::Online),
        "with the primary gone the fallback must report the GPU"
    );
    assert_eq!(
        device_status(&runtime, "gpu.primary.0"),
        Some(DeviceStatus::Offline),
        "the primary's device stays in the model as offline, but stops reporting"
    );
    let (state, _) = adapter_state(&runtime, "fallback");
    assert_eq!(state, AdapterState::Available);
    let (primary_state, _) = adapter_state(&runtime, "primary");
    assert_eq!(primary_state, AdapterState::Unavailable);
}

/// And the hand-over is re-decided every cycle, so a provider that appears later
/// takes over again instead of leaving two GPUs on screen.
#[tokio::test]
async fn the_hand_over_is_re_decided_on_every_cycle() {
    let (_temp, runtime, primary_available, _fallback) = rig().await;

    // Primary leaves: the fallback takes over.
    primary_available.store(false, Ordering::SeqCst);
    runtime.refresh_devices().await.expect("cycle 1");
    assert_eq!(
        device_status(&runtime, "gpu.fallback.0"),
        Some(DeviceStatus::Online)
    );

    // Primary comes back: the fallback stands by again, without a restart.
    primary_available.store(true, Ordering::SeqCst);
    runtime.refresh_devices().await.expect("cycle 2");
    assert_eq!(
        device_status(&runtime, "gpu.primary.0"),
        Some(DeviceStatus::Online),
        "the primary reports again"
    );
    assert_eq!(
        device_status(&runtime, "gpu.fallback.0"),
        Some(DeviceStatus::Offline),
        "the fallback must step aside again once the primary is back, so the user \
         still sees one GPU"
    );
    let (state, detail) = adapter_state(&runtime, "fallback");
    assert_eq!(state, AdapterState::Unavailable);
    assert!(detail.unwrap_or_default().contains("standing by"));
}
