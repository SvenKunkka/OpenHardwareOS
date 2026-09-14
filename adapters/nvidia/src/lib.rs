//! NVIDIA GPU telemetry and fan control through NVML.
//!
//! # What this adapter can and cannot do
//!
//! | Item | Status |
//! |------|--------|
//! | GPU name, core temperature, utilisation, board power, memory, clock | read, no elevation |
//! | Fan speed in percent (per fan) | read |
//! | Fan speed in RPM | read where the driver implements `nvmlDeviceGetFanSpeedRPM` |
//! | **Hotspot / junction temperature** | **not exposed by NVML** — LibreHardwareMonitor reads it through NVAPI |
//! | Fan *setting* | `nvmlDeviceSetFanSpeed_v2`, documented for Maxwell and newer, **requires Administrator/root** |
//!
//! Two consequences worth stating plainly, because they shape the UX:
//!
//! 1. A refused fan write is normal, not a bug. NVML answers
//!    `NVML_ERROR_NO_PERMISSION` when the process is not elevated, and some
//!    SKUs/drivers refuse third party fan control entirely. This adapter
//!    reports `permission_denied` / `vendor_limitation` and never pretends the
//!    fan moved.
//! 2. NVML has **no fan curve API** — only a duty plus a policy enum. A curve is
//!    the runtime's job (poll, evaluate, set), which is exactly the loop the
//!    automation engine implements.
//!
//! No NVIDIA binary is redistributed: `nvml-wrapper` loads the copy that ships
//! with the installed driver, and on machines without an NVIDIA driver the
//! adapter simply reports itself unavailable.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::error::NvmlError;
use ohm_adapter_api::{
    AdapterCapabilities, AdapterInfo, AdapterStatus, HardwareAdapter, WriteOutcome,
};
use ohm_core::{AdapterId, DeviceId, OhmError, Result};
use ohm_device_model::{
    Capability, Device, DeviceState, Reading, UnavailableReason, Unit, Value, caps,
};

/// Adapter id, also the device id namespace.
pub const ADAPTER_ID: &str = "nvidia";
/// Display name.
pub const ADAPTER_NAME: &str = "NVIDIA (NVML)";

/// NVIDIA GPU provider.
#[derive(Debug)]
pub struct NvidiaAdapter {
    /// `None` when NVML could not be initialised, which is the normal case on
    /// machines without an NVIDIA driver.
    nvml: Option<Nvml>,
    /// Why initialisation failed, kept for the UI.
    unavailable: Option<(UnavailableReason, String)>,
}

impl Default for NvidiaAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl NvidiaAdapter {
    /// Try to initialise NVML.
    pub fn new() -> Self {
        match Nvml::init() {
            Ok(nvml) => {
                tracing::debug!("NVML initialised");
                Self {
                    nvml: Some(nvml),
                    unavailable: None,
                }
            }
            Err(err) => {
                let (reason, detail) = classify_init_error(&err);
                tracing::debug!(detail, "NVML unavailable");
                Self {
                    nvml: None,
                    unavailable: Some((reason, detail)),
                }
            }
        }
    }

    /// As a trait object, ready to register on a runtime.
    pub fn boxed() -> Arc<dyn HardwareAdapter> {
        Arc::new(Self::new())
    }

    /// `true` when NVML is usable on this machine.
    pub fn is_available(&self) -> bool {
        self.nvml.is_some()
    }

    /// Why the adapter is unavailable, if it is.
    pub fn unavailable_reason(&self) -> Option<(UnavailableReason, String)> {
        self.unavailable.clone()
    }

    fn nvml(&self) -> Result<&Nvml> {
        self.nvml.as_ref().ok_or_else(|| {
            let (_, detail) = self.unavailable.clone().unwrap_or((
                UnavailableReason::Unsupported,
                "NVML is not available".into(),
            ));
            OhmError::AdapterUnavailable {
                adapter: ADAPTER_ID.to_string(),
                detail,
            }
        })
    }

    fn gpu_indices(&self) -> Vec<u32> {
        let Ok(nvml) = self.nvml() else {
            return Vec::new();
        };
        match nvml.device_count() {
            Ok(count) => (0..count).collect(),
            Err(err) => {
                tracing::debug!(error = %err, "NVML device_count failed");
                Vec::new()
            }
        }
    }

    fn build_devices(&self) -> Vec<Device> {
        let Ok(nvml) = self.nvml() else {
            return Vec::new();
        };
        let adapter = AdapterId::new_unchecked(ADAPTER_ID);
        let mut devices = Vec::new();

        for index in self.gpu_indices() {
            let Ok(device) = nvml.device_by_index(index) else {
                continue;
            };
            let name = device
                .name()
                .unwrap_or_else(|_| format!("NVIDIA GPU {index}"));
            let fans = device.num_fans().unwrap_or(0);
            let uuid = device.uuid().ok();

            let mut gpu = Device::new(
                gpu_device_id(index),
                name.clone(),
                ohm_device_model::DeviceType::Gpu,
                ohm_device_model::Transport::Nvidia,
                adapter.clone(),
            )
            .with_vendor("NVIDIA")
            .with_model(name)
            .with_capability(Capability::sensor(
                caps::TEMPERATURE_CORE,
                "Core Temperature",
                Unit::Celsius,
            ))
            .with_capability(Capability::sensor(
                caps::GPU_LOAD,
                "GPU Load",
                Unit::Percent,
            ))
            .with_capability(Capability::sensor(
                caps::POWER_GPU,
                "Board Power",
                Unit::Watt,
            ))
            .with_capability(Capability::sensor(
                caps::MEMORY_USED,
                "Memory Used",
                Unit::Byte,
            ))
            .with_capability(Capability::sensor(
                caps::CLOCK_MHZ,
                "Graphics Clock",
                Unit::Megahertz,
            ))
            .with_metadata("nvml_index", index.to_string())
            .with_metadata("fan_count", fans.to_string())
            .with_metadata("source", "NVML");

            if let Some(uuid) = uuid {
                gpu = gpu.with_metadata("uuid", uuid);
            }

            // Hotspot is not available from NVML. Declaring it and reporting
            // `unsupported` is more honest than hiding it: the UI then explains
            // where the reading would come from.
            gpu = gpu.with_capability(
                Capability::sensor(
                    caps::TEMPERATURE_HOTSPOT,
                    "Hotspot Temperature",
                    Unit::Celsius,
                )
                .with_description("Not exposed by NVML; LibreHardwareMonitor reads it via NVAPI"),
            );

            if fans > 0 {
                gpu = gpu
                    .with_capability(
                        Capability::sensor(caps::FAN_RPM, "GPU Fan RPM", Unit::Rpm)
                            .with_description("nvmlDeviceGetFanSpeedRPM"),
                    )
                    .with_capability(
                        Capability::actuator(
                            caps::FAN_SPEED_PERCENT,
                            "GPU Fan Speed",
                            Unit::Percent,
                            0.0,
                            100.0,
                        )
                        .with_description(
                            "nvmlDeviceSetFanSpeed_v2; requires Administrator/elevation",
                        ),
                    );
            }

            if let Err(err) = gpu.validate() {
                tracing::warn!(error = %err, "skipping an invalid NVIDIA device description");
                continue;
            }
            devices.push(gpu);
        }

        devices
    }

    fn read_one(&self, device: &Device) -> DeviceState {
        let mut state = DeviceState::new(device.id.clone(), ohm_core::now_ms());
        let Some(index) = gpu_index_of(device) else {
            return state;
        };
        let Ok(nvml) = self.nvml() else {
            return state.offline(
                self.unavailable
                    .clone()
                    .map(|(reason, _)| reason)
                    .unwrap_or(UnavailableReason::ReadError),
                self.unavailable
                    .clone()
                    .map(|(_, detail)| detail)
                    .unwrap_or_else(|| "NVML is not available".into()),
            );
        };
        let Ok(gpu) = nvml.device_by_index(index) else {
            return state.offline(
                UnavailableReason::NotPresent,
                format!("NVML no longer reports GPU {index}"),
            );
        };

        // Temperature.
        match gpu.temperature(TemperatureSensor::Gpu) {
            Ok(celsius) => state.set(Reading::ok(
                caps::TEMPERATURE_CORE,
                Value::Number(f64::from(celsius)),
            )),
            Err(err) => state.set(Reading::unavailable(
                caps::TEMPERATURE_CORE,
                classify(&err),
                Some(err.to_string()),
            )),
        }

        // Hotspot: deliberately reported as unsupported.
        state.set(Reading::unavailable(
            caps::TEMPERATURE_HOTSPOT,
            UnavailableReason::Unsupported,
            Some(
                "NVML does not expose the hotspot/junction sensor; \
                 install LibreHardwareMonitor to read it"
                    .to_string(),
            ),
        ));

        // Utilisation.
        match gpu.utilization_rates() {
            Ok(utilisation) => {
                state.set(Reading::ok(
                    caps::GPU_LOAD,
                    Value::Number(f64::from(utilisation.gpu)),
                ));
            }
            Err(err) => state.set(Reading::unavailable(
                caps::GPU_LOAD,
                classify(&err),
                Some(err.to_string()),
            )),
        }

        // Power (NVML reports milliwatts).
        match gpu.power_usage() {
            Ok(milliwatts) => state.set(Reading::ok(
                caps::POWER_GPU,
                Value::Number((f64::from(milliwatts) / 1000.0 * 10.0).round() / 10.0),
            )),
            Err(err) => state.set(Reading::unavailable(
                caps::POWER_GPU,
                classify(&err),
                Some(err.to_string()),
            )),
        }

        // Memory.
        match gpu.memory_info() {
            Ok(memory) => state.set(Reading::ok(
                caps::MEMORY_USED,
                Value::Integer(memory.used.min(i64::MAX as u64) as i64),
            )),
            Err(err) => state.set(Reading::unavailable(
                caps::MEMORY_USED,
                classify(&err),
                Some(err.to_string()),
            )),
        }

        // Clock.
        match gpu.clock_info(Clock::Graphics) {
            Ok(mhz) => state.set(Reading::ok(caps::CLOCK_MHZ, Value::Integer(i64::from(mhz)))),
            Err(err) => state.set(Reading::unavailable(
                caps::CLOCK_MHZ,
                classify(&err),
                Some(err.to_string()),
            )),
        }

        // Fans: only when the device declares them.
        if device.supports(caps::FAN_SPEED_PERCENT) {
            match gpu.fan_speed(0) {
                Ok(percent) => state.set(Reading::ok(
                    caps::FAN_SPEED_PERCENT,
                    Value::Number(f64::from(percent)),
                )),
                Err(err) => state.set(Reading::unavailable(
                    caps::FAN_SPEED_PERCENT,
                    classify(&err),
                    Some(err.to_string()),
                )),
            }
        }
        if device.supports(caps::FAN_RPM) {
            match gpu.fan_speed_rpm(0) {
                Ok(rpm) => state.set(Reading::ok(caps::FAN_RPM, Value::Integer(i64::from(rpm)))),
                Err(err) => state.set(Reading::unavailable(
                    caps::FAN_RPM,
                    classify(&err),
                    Some(format!(
                        "{err} (nvmlDeviceGetFanSpeedRPM is not implemented by every driver)"
                    )),
                )),
            }
        }

        state
    }
}

/// `gpu.nvidia.{index}`
pub fn gpu_device_id(index: u32) -> DeviceId {
    DeviceId::compose("gpu", ADAPTER_ID, index as usize)
}

fn gpu_index_of(device: &Device) -> Option<u32> {
    device.id.as_str().strip_prefix("gpu.nvidia.")?.parse().ok()
}

/// Map an NVML failure onto the runtime's vocabulary, so a limitation is shown
/// as a limitation rather than as an error.
pub fn classify(err: &NvmlError) -> UnavailableReason {
    match err {
        NvmlError::NotSupported => UnavailableReason::Unsupported,
        NvmlError::NoPermission => UnavailableReason::PermissionDenied,
        NvmlError::NotFound => UnavailableReason::NotPresent,
        NvmlError::DriverNotLoaded | NvmlError::LibraryNotFound => UnavailableReason::DriverMissing,
        NvmlError::Timeout => UnavailableReason::Timeout,
        NvmlError::InvalidArg | NvmlError::InsufficientSize(_) => {
            UnavailableReason::HardwareLimitation
        }
        _ => UnavailableReason::ReadError,
    }
}

fn classify_init_error(err: &NvmlError) -> (UnavailableReason, String) {
    let reason = classify(err);
    let detail = match reason {
        UnavailableReason::DriverMissing => format!(
            "no NVIDIA driver or NVML library found ({err}); this adapter needs an installed \
             NVIDIA driver"
        ),
        UnavailableReason::NotPresent => {
            format!("no NVIDIA GPU present ({err})")
        }
        _ => format!("NVML could not be initialised: {err}"),
    };
    (reason, detail)
}

#[async_trait]
impl HardwareAdapter for NvidiaAdapter {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::new(ADAPTER_ID, ADAPTER_NAME, ADAPTER_ID)
            .with_description(
                "NVIDIA GPU temperature, utilisation, power and fan telemetry through NVML. \
                 Fan control is offered but requires elevation and is refused by some \
                 driver/SKU combinations.",
            )
            .requires_admin()
            .with_capabilities(AdapterCapabilities {
                can_write: true,
                can_control_cooling: true,
                write_requires_admin: true,
                poll_interval_ms: None,
                discovery_interval_ms: None,
            })
    }

    async fn probe(&self) -> AdapterStatus {
        match self.nvml() {
            Ok(nvml) => match nvml.device_count() {
                Ok(0) => AdapterStatus::unavailable(
                    self.id(),
                    UnavailableReason::NotPresent,
                    "NVML is available but reports no GPUs",
                ),
                Ok(count) => AdapterStatus::available(self.id(), count as usize),
                Err(err) => AdapterStatus::error(self.id(), err.to_string()),
            },
            Err(_) => {
                let (reason, detail) = self
                    .unavailable
                    .clone()
                    .unwrap_or((UnavailableReason::Unknown, "NVML unavailable".into()));
                AdapterStatus::unavailable(self.id(), reason, detail)
            }
        }
    }

    async fn discover(&self) -> Result<Vec<Device>> {
        if self.nvml.is_none() {
            let (reason, detail) = self
                .unavailable
                .clone()
                .unwrap_or((UnavailableReason::Unsupported, "NVML unavailable".into()));
            let _ = reason;
            return Err(OhmError::AdapterUnavailable {
                adapter: ADAPTER_ID.to_string(),
                detail,
            });
        }
        Ok(self.build_devices())
    }

    async fn read_state(&self, device: &Device) -> Result<DeviceState> {
        Ok(self.read_one(device))
    }

    async fn read_all(&self, devices: &[Device]) -> Vec<(DeviceId, Result<DeviceState>)> {
        devices
            .iter()
            .map(|device| (device.id.clone(), Ok(self.read_one(device))))
            .collect()
    }

    async fn write(
        &self,
        device: &Device,
        capability: &Capability,
        value: &Value,
    ) -> Result<WriteOutcome> {
        if capability.id.as_str() != caps::FAN_SPEED_PERCENT {
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

        let nvml = self.nvml()?;
        let index =
            gpu_index_of(device).ok_or_else(|| OhmError::DeviceNotFound(device.id.to_string()))?;
        let mut gpu = nvml
            .device_by_index(index)
            .map_err(|err| OhmError::WriteRejected {
                device: device.id.to_string(),
                capability: capability.id.to_string(),
                detail: format!("NVML no longer reports this GPU: {err}"),
            })?;

        match gpu.set_fan_speed(0, applied.round().clamp(0.0, 100.0) as u32) {
            Ok(()) => Ok(WriteOutcome::applied(Value::Number(applied))),
            Err(err) => Err(OhmError::WriteRejected {
                device: device.id.to_string(),
                capability: capability.id.to_string(),
                detail: match classify(&err) {
                    UnavailableReason::PermissionDenied => format!(
                        "NVML refused the fan write ({err}). NVIDIA requires Administrator/root \
                         for `nvmlDeviceSetFanSpeed_v2`; run OpenHardwareOS elevated to control \
                         GPU fans."
                    ),
                    UnavailableReason::Unsupported => format!(
                        "this driver or GPU does not allow third party fan control ({err}). \
                         Use the vendor tool for the GPU fan, or let the GPU manage it."
                    ),
                    _ => format!("NVML refused the fan write: {err}"),
                },
            }),
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_adapter_api::AdapterState;

    /// The tests must pass on a machine without NVIDIA hardware *and* on one
    /// with it, so they assert the contract rather than a fixed outcome.
    #[tokio::test]
    async fn probe_is_honest_on_any_machine() {
        let adapter = NvidiaAdapter::new();
        let status = adapter.probe().await;
        assert_eq!(status.adapter.as_str(), ADAPTER_ID);
        if adapter.is_available() {
            assert!(status.is_usable());
        } else {
            assert_eq!(status.state, AdapterState::Unavailable);
            assert!(status.reason.is_some());
            let detail = status.detail.unwrap();
            assert!(!detail.is_empty());
            assert!(
                detail.contains("driver") || detail.contains("GPU") || detail.contains("NVML"),
                "unhelpful detail: {detail}"
            );
        }
    }

    #[tokio::test]
    async fn discovery_matches_availability() {
        let adapter = NvidiaAdapter::new();
        match adapter.discover().await {
            Ok(devices) => {
                assert!(adapter.is_available());
                for device in &devices {
                    device.validate().unwrap();
                    assert_eq!(device.vendor, "NVIDIA");
                    assert_eq!(
                        device.is_controllable(),
                        device.capability_str(caps::FAN_SPEED_PERCENT).is_some()
                    );
                    assert!(device.supports(caps::TEMPERATURE_CORE));
                    assert!(device.supports(caps::TEMPERATURE_HOTSPOT));
                    assert!(device.metadata.contains_key("nvml_index"));
                }
            }
            Err(err) => {
                assert!(!adapter.is_available());
                assert_eq!(err.code(), "adapter_unavailable");
            }
        }
    }

    #[tokio::test]
    async fn readings_are_plausible_or_explained() {
        let adapter = NvidiaAdapter::new();
        let Ok(devices) = adapter.discover().await else {
            return; // no NVIDIA hardware here
        };
        for device in devices.iter().take(1) {
            let state = adapter.read_state(device).await.unwrap();
            if let Some(reading) = state.get(caps::TEMPERATURE_CORE) {
                if reading.is_ok() {
                    let celsius = reading.number().unwrap();
                    assert!((0.0..=120.0).contains(&celsius), "{celsius}");
                } else {
                    assert!(reading.reason().is_some());
                }
            }
            // The hotspot reading is always an explicit limitation.
            let hotspot = state.get(caps::TEMPERATURE_HOTSPOT).unwrap();
            assert_eq!(hotspot.reason(), Some(UnavailableReason::Unsupported));
            assert!(hotspot.value().is_none());
        }
    }

    #[tokio::test]
    async fn writes_are_refused_with_a_useful_reason_without_nvidia() {
        let adapter = NvidiaAdapter::new();
        if adapter.is_available() {
            return;
        }
        // Build a device description by hand: the write path must refuse it
        // before touching NVML at all.
        let device = Device::new(
            gpu_device_id(0),
            "RTX",
            ohm_device_model::DeviceType::Gpu,
            ohm_device_model::Transport::Nvidia,
            AdapterId::new_unchecked(ADAPTER_ID),
        )
        .with_capability(Capability::actuator(
            caps::FAN_SPEED_PERCENT,
            "GPU Fan",
            Unit::Percent,
            0.0,
            100.0,
        ));
        let capability = device.capability_str(caps::FAN_SPEED_PERCENT).unwrap();
        let err = adapter
            .write(&device, capability, &Value::Number(50.0))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "adapter_unavailable");
        assert!(err.hint().contains("LibreHardwareMonitor") || err.hint().contains("provider"));
    }

    #[tokio::test]
    async fn invalid_values_are_rejected_before_nvml() {
        let adapter = NvidiaAdapter::new();
        let device = Device::new(
            gpu_device_id(0),
            "RTX",
            ohm_device_model::DeviceType::Gpu,
            ohm_device_model::Transport::Nvidia,
            AdapterId::new_unchecked(ADAPTER_ID),
        )
        .with_capability(Capability::actuator(
            caps::FAN_SPEED_PERCENT,
            "GPU Fan",
            Unit::Percent,
            0.0,
            100.0,
        ))
        .with_capability(Capability::sensor(caps::FAN_RPM, "RPM", Unit::Rpm));
        let control = device.capability_str(caps::FAN_SPEED_PERCENT).unwrap();

        assert_eq!(
            adapter
                .write(&device, control, &Value::Number(150.0))
                .await
                .unwrap_err()
                .code(),
            "value_out_of_range"
        );
        assert_eq!(
            adapter
                .write(&device, control, &Value::Text("fast".into()))
                .await
                .unwrap_err()
                .code(),
            "invalid_value"
        );
        let rpm = device.capability_str(caps::FAN_RPM).unwrap();
        assert_eq!(
            adapter
                .write(&device, rpm, &Value::Number(1000.0))
                .await
                .unwrap_err()
                .code(),
            "capability_read_only"
        );
    }

    #[test]
    fn error_classification() {
        assert_eq!(
            classify(&NvmlError::NotSupported),
            UnavailableReason::Unsupported
        );
        assert_eq!(
            classify(&NvmlError::NoPermission),
            UnavailableReason::PermissionDenied
        );
        assert_eq!(
            classify(&NvmlError::NotFound),
            UnavailableReason::NotPresent
        );
        assert_eq!(
            classify(&NvmlError::DriverNotLoaded),
            UnavailableReason::DriverMissing
        );
        assert_eq!(
            classify(&NvmlError::LibraryNotFound),
            UnavailableReason::DriverMissing
        );
        assert_eq!(classify(&NvmlError::Timeout), UnavailableReason::Timeout);
        assert_eq!(
            classify(&NvmlError::InvalidArg),
            UnavailableReason::HardwareLimitation
        );

        let (reason, detail) = classify_init_error(&NvmlError::LibraryNotFound);
        assert_eq!(reason, UnavailableReason::DriverMissing);
        assert!(detail.contains("driver"));
    }

    #[test]
    fn device_ids_and_adapter_metadata() {
        assert_eq!(gpu_device_id(0).as_str(), "gpu.nvidia.0");
        assert_eq!(gpu_device_id(3).as_str(), "gpu.nvidia.3");
        let info = NvidiaAdapter::new().info();
        assert!(info.requires_admin);
        assert_eq!(info.namespace, ADAPTER_ID);
        assert!(info.description.contains("NVML"));
        assert_eq!(NvidiaAdapter::default().info().id.as_str(), ADAPTER_ID);
        assert_eq!(NvidiaAdapter::boxed().info().id.as_str(), ADAPTER_ID);
    }
}
