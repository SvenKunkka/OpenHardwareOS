//! OS level sensors: what the operating system itself will tell us.
//!
//! | Reading | Windows | Linux | macOS | Source |
//! |---------|---------|-------|-------|--------|
//! | CPU name / vendor | yes | yes | yes | `sysinfo` |
//! | CPU load | yes | yes | yes | `sysinfo` |
//! | CPU frequency | rated clock | yes | rated clock | `sysinfo` |
//! | Thermal zones | sometimes | yes | sometimes | `sysinfo::Components` |
//! | Memory used / total | yes | yes | yes | `sysinfo` |
//! | SSD temperature | WMI reliability counters | via components | not exposed | `windows` module |
//! | Fan RPM | **no** | **yes** (hwmon) | **no** | `hwmon` module on Linux |
//! | Fan/PWM control | **no** | **no, deliberately** | **no** | see `hwmon` and the `lhm` adapter |
//!
//! Where a fan reading comes from is platform business, and the table above is the
//! honest summary of it. Windows exposes no chassis tachometer through an OS API,
//! and macOS exposes none at all; on Linux the kernel's **hwmon** subsystem does,
//! for the boards whose driver implements it. That is what the `hwmon` module
//! reads — read-only, because writing `pwm<N>` needs root and a wrong channel is
//! the classic "fans stop" failure. A driver that controls the channel itself
//! says so, and the channel is described rather than driven.
//!
//! Everything here is read-only, needs no elevation and never panics: a missing
//! sensor becomes a [`Reading`] with an [`UnavailableReason`].

pub mod devices;
pub mod hwmon;

#[cfg(windows)]
pub mod windows;

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use ohm_adapter_api::{
    AdapterCapabilities, AdapterInfo, AdapterState, AdapterStatus, HardwareAdapter, WriteOutcome,
};
use ohm_core::{AdapterId, DeviceId, OhmError, Result};
use ohm_device_model::{Capability, Device, DeviceState, Reading, UnavailableReason, Unit, Value};
use parking_lot::Mutex;
use sysinfo::{Components, Disks, System};

use devices::{component_device_id, cpu_device_id, disk_device_id};

/// Adapter id, also the device id namespace.
pub const ADAPTER_ID: &str = "system";
/// Display name.
pub const ADAPTER_NAME: &str = "Operating System";

/// Read-only OS level telemetry.
#[derive(Debug)]
pub struct SystemAdapter {
    system: Mutex<System>,
    components: Mutex<Components>,
    disks: Mutex<Disks>,
    /// Successful refreshes, used to explain the first-sample warm-up.
    polls: Mutex<u64>,
    /// Cached processor brand string.
    cpu_brand: Mutex<String>,
    /// Where to look for hwmon chips: `/sys/class/hwmon` on Linux, `None`
    /// elsewhere, a fixture directory in tests.
    hwmon_root: Option<std::path::PathBuf>,
    /// The chips and fan channels found at the last discovery.
    hwmon: Mutex<Option<hwmon::HwmonTree>>,
}

impl Default for SystemAdapter {
    fn default() -> Self {
        Self::new()
    }
}

/// `/sys/class/hwmon` on Linux; nothing on the platforms that have no sysfs.
fn default_hwmon_root() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        Some(std::path::PathBuf::from(hwmon::DEFAULT_ROOT))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

impl SystemAdapter {
    pub fn new() -> Self {
        Self::with_hwmon_root(default_hwmon_root())
    }

    /// Read fan channels from `root` instead of the platform default.
    ///
    /// Used by the tests with a fixture tree, and by anyone whose sysfs is
    /// mounted somewhere else. `None` disables the hwmon source entirely.
    pub fn with_hwmon_root(root: Option<std::path::PathBuf>) -> Self {
        let mut adapter = Self::blank();
        adapter.hwmon_root = root;
        adapter
    }

    fn blank() -> Self {
        let mut system = System::new();
        system.refresh_cpu_all();
        system.refresh_memory();
        let cpu_brand = system
            .cpus()
            .first()
            .map(|cpu| cpu.brand().trim().to_string())
            .unwrap_or_default();

        Self {
            system: Mutex::new(system),
            components: Mutex::new(Components::new_with_refreshed_list()),
            disks: Mutex::new(Disks::new_with_refreshed_list()),
            polls: Mutex::new(0),
            cpu_brand: Mutex::new(cpu_brand),
            hwmon_root: None,
            hwmon: Mutex::new(None),
        }
    }

    /// As a trait object, ready to register on a runtime.
    pub fn boxed() -> Arc<dyn HardwareAdapter> {
        Arc::new(Self::new())
    }

    pub fn cpu_brand(&self) -> String {
        self.cpu_brand.lock().clone()
    }

    /// Number of refreshes performed so far.
    pub fn poll_count(&self) -> u64 {
        *self.polls.lock()
    }

    /// Refresh every OS level source. Cheap (a few milliseconds) but blocking,
    /// which is why the runtime calls it from its own poll loop rather than
    /// from an async context that must stay responsive.
    fn refresh(&self) {
        {
            let mut system = self.system.lock();
            system.refresh_cpu_usage();
            system.refresh_cpu_frequency();
            system.refresh_memory();
        }
        self.components.lock().refresh(true);
        self.disks.lock().refresh(true);
        *self.polls.lock() += 1;
    }

    /// Average CPU utilisation in percent.
    fn cpu_load(&self) -> f64 {
        let system = self.system.lock();
        let cpus = system.cpus();
        if cpus.is_empty() {
            return f64::from(system.global_cpu_usage());
        }
        let total: f32 = cpus.iter().map(|cpu| cpu.cpu_usage()).sum();
        f64::from(total) / cpus.len() as f64
    }

    /// Rated or current clock in MHz, whichever the platform provides.
    fn cpu_clock_mhz(&self) -> f64 {
        let system = self.system.lock();
        system
            .cpus()
            .first()
            .map(|cpu| cpu.frequency() as f64)
            .unwrap_or(0.0)
    }

    /// The thermal zone that best represents the CPU package, if any.
    fn cpu_component_temperature(&self) -> Option<(String, f64)> {
        let components = self.components.lock();
        let mut best: Option<(u8, String, f64)> = None;
        for component in components.list() {
            if devices::is_derived_zone(component.label()) {
                continue;
            }
            let Some(temperature) = component.temperature() else {
                continue;
            };
            let temperature = f64::from(temperature);
            if !(-40.0..=150.0).contains(&temperature) {
                continue;
            }
            let score = devices::cpu_component_score(component.label());
            if score == 0 {
                continue;
            }
            let candidate = (score, component.label().to_string(), temperature);
            if best
                .as_ref()
                .map(|current| candidate.0 > current.0)
                .unwrap_or(true)
            {
                best = Some(candidate);
            }
        }
        best.map(|(_, label, temperature)| (label, temperature))
    }

    /// Why the CPU temperature is missing, phrased for a human.
    fn cpu_temperature_unavailable(&self) -> (UnavailableReason, String) {
        let components = self.components.lock();
        if components.list().is_empty() {
            (
                UnavailableReason::Unsupported,
                "this operating system exposes no thermal zones to user space".to_string(),
            )
        } else {
            (
                UnavailableReason::HardwareLimitation,
                format!(
                    "{} thermal zone(s) reported, but none is a CPU package sensor",
                    components.list().len()
                ),
            )
        }
    }

    /// Windows storage temperatures, when the platform can produce them.
    #[cfg(windows)]
    fn disk_temperatures(&self) -> Option<std::collections::HashMap<String, f64>> {
        match windows::storage_temperatures() {
            Ok(map) => Some(map),
            Err(failure) => {
                tracing::debug!(
                    reason = failure.reason().as_str(),
                    detail = failure.detail(),
                    "storage reliability counters unavailable"
                );
                None
            }
        }
    }

    #[cfg(not(windows))]
    fn disk_temperatures(&self) -> Option<std::collections::HashMap<String, f64>> {
        None
    }

    /// Why a drive temperature is missing on this platform.
    fn disk_temperature_unavailable(&self) -> (UnavailableReason, String) {
        #[cfg(windows)]
        {
            match windows::storage_temperatures() {
                Err(failure) => (failure.reason(), failure.detail()),
                Ok(_) => (
                    UnavailableReason::HardwareLimitation,
                    "the drive does not report reliability counters".to_string(),
                ),
            }
        }
        #[cfg(not(windows))]
        {
            (
                UnavailableReason::Unsupported,
                "this platform does not expose NVMe/SATA drive temperatures to user space; \
                 LibreHardwareMonitor (Windows) or smartctl can provide them"
                    .to_string(),
            )
        }
    }

    /// Temperature for one disk, when the platform provides it.
    #[allow(unused_variables)]
    fn disk_temperature(&self, name: &str, mount: &str) -> Option<f64> {
        let temperatures = self.disk_temperatures()?;
        #[cfg(windows)]
        {
            windows::temperature_for(&temperatures, name, mount)
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    /// Build the device descriptions from the current OS view.
    fn build_devices(&self) -> Vec<Device> {
        let mut devices = Vec::with_capacity(4);
        let adapter = AdapterId::new_unchecked(ADAPTER_ID);

        // --- CPU ---------------------------------------------------------
        let brand = {
            let mut cached = self.cpu_brand.lock();
            if cached.is_empty() {
                let system = self.system.lock();
                *cached = system
                    .cpus()
                    .first()
                    .map(|cpu| cpu.brand().trim().to_string())
                    .unwrap_or_default();
            }
            cached.clone()
        };
        let name = if brand.is_empty() {
            "Processor".to_string()
        } else {
            brand.clone()
        };
        let cores = self.system.lock().cpus().len();

        let mut cpu = Device::new(
            cpu_device_id(),
            name,
            ohm_device_model::DeviceType::Cpu,
            ohm_device_model::Transport::System,
            adapter.clone(),
        )
        .with_vendor(devices::vendor_from_brand(&brand))
        .with_capability(Capability::sensor(
            ohm_device_model::caps::TEMPERATURE_CORE,
            "CPU Temperature",
            Unit::Celsius,
        ))
        .with_capability(Capability::sensor(
            ohm_device_model::caps::CPU_LOAD,
            "CPU Load",
            Unit::Percent,
        ))
        .with_capability(Capability::sensor(
            ohm_device_model::caps::CLOCK_MHZ,
            "Core Clock",
            Unit::Megahertz,
        ))
        .with_metadata("cores", cores.to_string())
        .with_metadata("source", "operating system");
        if !brand.is_empty() {
            cpu = cpu.with_model(brand);
        }
        devices.push(cpu);

        // --- Thermal zones ----------------------------------------------
        //
        // Some platforms (Apple Silicon in particular) expose dozens of internal
        // zones such as `PMU tdie7`. Showing all of them would bury the useful
        // devices, so the most meaningful ones are selected and the rest are
        // summarised in the CPU device's metadata.
        {
            let components = self.components.lock();
            let selected = devices::select_thermal_zones(components.list());
            let total_zones = components
                .list()
                .iter()
                .filter(|component| component.temperature().is_some())
                .count();
            if total_zones > selected.len()
                && let Some(cpu) = devices.first_mut()
            {
                cpu.metadata
                    .insert("thermal_zones_total".to_string(), total_zones.to_string());
                cpu.metadata.insert(
                    "thermal_zones_shown".to_string(),
                    selected.len().to_string(),
                );
            }

            for (index, component) in selected {
                devices.push(
                    Device::new(
                        component_device_id(index),
                        format!("Thermal Zone: {}", component.label()),
                        ohm_device_model::DeviceType::TemperatureSensor,
                        ohm_device_model::Transport::System,
                        adapter.clone(),
                    )
                    .with_vendor(ohm_core::PRODUCT_NAME)
                    .with_capability(Capability::sensor(
                        ohm_device_model::caps::TEMPERATURE_SYSTEM,
                        "Temperature",
                        Unit::Celsius,
                    ))
                    .with_metadata("zone", component.label().to_string()),
                );
            }
        }

        // --- Storage -----------------------------------------------------
        {
            let disks = self.disks.lock();
            let mut index = 0;
            // macOS exposes every mounted volume, so the same physical disk can
            // appear several times. One device per (name, capacity) is what a
            // user thinks of as "my drive".
            let mut seen: Vec<(String, u64)> = Vec::new();
            for disk in disks.list() {
                if disk.is_removable() {
                    continue;
                }
                let name = disk.name().to_string_lossy().to_string();
                let identity = (name.clone(), disk.total_space());
                if seen.contains(&identity) {
                    continue;
                }
                seen.push(identity);
                let mount = disk.mount_point().display().to_string();
                let label = if name.is_empty() {
                    mount.clone()
                } else {
                    name.clone()
                };
                devices.push(
                    Device::new(
                        disk_device_id(index),
                        format!("Storage: {label}"),
                        ohm_device_model::DeviceType::Storage,
                        ohm_device_model::Transport::System,
                        adapter.clone(),
                    )
                    .with_vendor(ohm_core::PRODUCT_NAME)
                    .with_capability(Capability::sensor(
                        ohm_device_model::caps::TEMPERATURE_CORE,
                        "Drive Temperature",
                        Unit::Celsius,
                    ))
                    .with_capability(Capability::sensor(
                        ohm_device_model::caps::DISK_FREE,
                        "Free Space",
                        Unit::Byte,
                    ))
                    .with_metadata("mount_point", mount)
                    .with_metadata(
                        "file_system",
                        disk.file_system().to_string_lossy().to_string(),
                    )
                    .with_metadata("total_bytes", disk.total_space().to_string())
                    .with_metadata("disk_name", name),
                );
                index += 1;
            }
        }

        // --- Memory ------------------------------------------------------
        {
            let system = self.system.lock();
            let total = system.total_memory();
            let used = system.used_memory();
            if total > 0 {
                devices.push(
                    Device::new(
                        devices::memory_device_id(),
                        "Memory",
                        ohm_device_model::DeviceType::Memory,
                        ohm_device_model::Transport::System,
                        adapter.clone(),
                    )
                    .with_vendor(ohm_core::PRODUCT_NAME)
                    .with_capability(Capability::sensor(
                        ohm_device_model::caps::MEMORY_USED,
                        "Memory Used",
                        Unit::Byte,
                    ))
                    .with_capability(Capability::sensor(
                        ohm_device_model::caps::MEMORY_TOTAL,
                        "Memory Total",
                        Unit::Byte,
                    ))
                    .with_metadata("used_bytes", used.to_string())
                    .with_metadata("total_bytes", total.to_string())
                    .with_metadata("source", "operating system"),
                );
            }
        }

        // --- Fan tachometers (Linux hwmon) -------------------------------
        //
        // Only where the kernel has them, and read-only on purpose: the module
        // explains why writing `pwm<N>` is separate work.
        {
            let tree = self.hwmon.lock();
            if let Some(tree) = tree.as_ref() {
                for (chip, fan) in tree.channels() {
                    let mut capabilities: Vec<Capability> = Vec::new();
                    if fan.rpm.is_some() {
                        capabilities.push(Capability::sensor(
                            ohm_device_model::caps::FAN_RPM,
                            "Fan RPM",
                            Unit::Rpm,
                        ));
                    }
                    if fan.pwm.is_some() {
                        // A *sensor*: the duty the kernel is applying, not a
                        // control this program may drive yet.
                        capabilities.push(Capability::sensor(
                            ohm_device_model::caps::FAN_PWM,
                            "PWM Duty",
                            Unit::Percent,
                        ));
                    }
                    if capabilities.is_empty() {
                        continue;
                    }
                    let mut device = Device::new(
                        DeviceId::new_unchecked(fan.device_id()),
                        fan.display_name(),
                        ohm_device_model::DeviceType::Fan,
                        ohm_device_model::Transport::System,
                        adapter.clone(),
                    )
                    .with_vendor(ohm_core::PRODUCT_NAME)
                    .with_capabilities(capabilities)
                    .with_metadata("hwmon_chip", chip.name.clone())
                    .with_metadata("hwmon_channel", fan.channel.to_string())
                    .with_metadata("source", "linux hwmon (read-only)");
                    if let Some(path) = &fan.rpm {
                        device = device.with_metadata("fan_input", path.display().to_string());
                    }
                    if let Some(path) = &fan.pwm {
                        device = device.with_metadata("pwm_path", path.display().to_string());
                    }
                    if let Some(mode) = fan.current_mode() {
                        device = device.with_metadata("control_mode", mode.describe());
                    }
                    devices.push(device);
                }
            }
        }

        devices
    }

    /// Rescan the hwmon tree, remembering what was found.
    fn scan_hwmon(&self) {
        let Some(root) = self.hwmon_root.clone() else {
            return;
        };
        let tree = hwmon::discover(&root);
        if !tree.notes.is_empty() {
            tracing::debug!(root = %root.display(), notes = ?tree.notes, "hwmon scan notes");
        }
        *self.hwmon.lock() = Some(tree);
    }

    /// The fan channel behind a device id, if the last scan found it.
    fn hwmon_channel(&self, device: &Device) -> Option<hwmon::FanChannel> {
        let tree = self.hwmon.lock();
        let tree = tree.as_ref()?;
        tree.channels()
            .find(|(_, fan)| fan.device_id() == device.id.as_str())
            .map(|(_, fan)| fan.clone())
    }

    /// Read one device.
    fn read_one(&self, device: &Device) -> DeviceState {
        let mut state = DeviceState::new(device.id.clone(), ohm_core::now_ms());

        if device.id == cpu_device_id() {
            match self.cpu_component_temperature() {
                Some((_label, temperature)) => state.set(Reading::ok(
                    ohm_device_model::caps::TEMPERATURE_CORE,
                    Value::Number((temperature * 10.0).round() / 10.0),
                )),
                None => {
                    let (reason, detail) = self.cpu_temperature_unavailable();
                    state.set(Reading::unavailable(
                        ohm_device_model::caps::TEMPERATURE_CORE,
                        reason,
                        Some(detail),
                    ));
                }
            }
            state.set(Reading::ok(
                ohm_device_model::caps::CPU_LOAD,
                Value::Number((self.cpu_load() * 10.0).round() / 10.0),
            ));
            let mhz = self.cpu_clock_mhz();
            if mhz > 0.0 {
                state.set(Reading::ok(
                    ohm_device_model::caps::CLOCK_MHZ,
                    Value::Integer(mhz.round() as i64),
                ));
            } else {
                state.set(Reading::unavailable(
                    ohm_device_model::caps::CLOCK_MHZ,
                    UnavailableReason::Unsupported,
                    Some("the operating system does not report a CPU clock".to_string()),
                ));
            }
            return state;
        }

        if let Some(index) = component_index_of(device) {
            let components = self.components.lock();
            match components.list().get(index).and_then(|c| c.temperature()) {
                Some(temperature) => state.set(Reading::ok(
                    ohm_device_model::caps::TEMPERATURE_SYSTEM,
                    Value::Number((f64::from(temperature) * 10.0).round() / 10.0),
                )),
                None => state.set(Reading::unavailable(
                    ohm_device_model::caps::TEMPERATURE_SYSTEM,
                    UnavailableReason::ReadError,
                    Some("the thermal zone stopped reporting".to_string()),
                )),
            }
            return state;
        }

        if device.id == devices::memory_device_id() {
            let system = self.system.lock();
            let total = system.total_memory();
            let used = system.used_memory();
            state.set(Reading::ok(
                ohm_device_model::caps::MEMORY_USED,
                Value::Integer(used.min(i64::MAX as u64) as i64),
            ));
            state.set(Reading::ok(
                ohm_device_model::caps::MEMORY_TOTAL,
                Value::Integer(total.min(i64::MAX as u64) as i64),
            ));
            return state;
        }

        if device.id.as_str().starts_with("fan.system.") {
            let Some(fan) = self.hwmon_channel(device) else {
                // The last scan does not know this channel any more.
                return state.offline(
                    UnavailableReason::NotPresent,
                    "the kernel no longer exposes this fan channel",
                );
            };
            match fan.read_rpm() {
                Some(Ok(rpm)) => state.set(Reading::ok(
                    ohm_device_model::caps::FAN_RPM,
                    Value::Integer(rpm.round() as i64),
                )),
                Some(Err((reason, detail))) => state.set(Reading::unavailable(
                    ohm_device_model::caps::FAN_RPM,
                    reason,
                    Some(detail),
                )),
                None => {}
            }
            match fan.read_pwm() {
                Some(Ok((percent, _raw))) => state.set(Reading::ok(
                    ohm_device_model::caps::FAN_PWM,
                    Value::Number((percent * 10.0).round() / 10.0),
                )),
                Some(Err((reason, detail))) => state.set(Reading::unavailable(
                    ohm_device_model::caps::FAN_PWM,
                    reason,
                    Some(detail),
                )),
                None => {}
            }
            // The control mode is part of the reading: it says who is driving
            // the channel while this program only watches it.
            if let Some((reason, detail)) = fan.read_mode() {
                state.set(Reading::unavailable(
                    ohm_device_model::caps::STATUS_MESSAGE,
                    reason,
                    Some(format!("pwm control mode unreadable: {detail}")),
                ));
            }
            return state;
        }

        if let Some(index) = disk_index_of(device) {
            let (free_space, name, mount) = {
                let disks = self.disks.lock();
                match disks.list().get(index) {
                    Some(disk) => (
                        Some(disk.available_space()),
                        disk.name().to_string_lossy().to_string(),
                        disk.mount_point().display().to_string(),
                    ),
                    None => (None, String::new(), String::new()),
                }
            };
            let Some(free_space) = free_space else {
                return state.offline(
                    UnavailableReason::NotPresent,
                    "the drive is no longer present",
                );
            };
            state.set(Reading::ok(
                ohm_device_model::caps::DISK_FREE,
                Value::Integer(free_space.min(i64::MAX as u64) as i64),
            ));
            match self.disk_temperature(&name, &mount) {
                Some(celsius) => state.set(Reading::ok(
                    ohm_device_model::caps::TEMPERATURE_CORE,
                    Value::Number((celsius * 10.0).round() / 10.0),
                )),
                None => {
                    let (reason, detail) = self.disk_temperature_unavailable();
                    state.set(Reading::unavailable(
                        ohm_device_model::caps::TEMPERATURE_CORE,
                        reason,
                        Some(detail),
                    ));
                }
            }
        }

        state
    }
}

fn component_index_of(device: &Device) -> Option<usize> {
    device
        .id
        .as_str()
        .strip_prefix("temperature.system.")
        .and_then(|index| index.parse().ok())
}

fn disk_index_of(device: &Device) -> Option<usize> {
    device
        .id
        .as_str()
        .strip_prefix("storage.system.")
        .and_then(|index| index.parse().ok())
}

#[async_trait]
impl HardwareAdapter for SystemAdapter {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::new(ADAPTER_ID, ADAPTER_NAME, ADAPTER_ID)
            .with_description(
                "CPU utilisation and clock, OS thermal zones and disk space. Read-only: \
                 operating systems expose no fan tachometer or PWM control.",
            )
            .with_capabilities(AdapterCapabilities::read_only())
    }

    async fn probe(&self) -> AdapterStatus {
        self.refresh();
        let device_count = self.build_devices().len();
        // The adapter is always usable; it may simply have little to report.
        // Saying so up front is the whole point of `Degraded`.
        let (state, reason, detail) = match self.cpu_component_temperature() {
            Some(_) => (AdapterState::Available, None, None),
            None => {
                let (reason, detail) = self.cpu_temperature_unavailable();
                (AdapterState::Degraded, Some(reason), Some(detail))
            }
        };
        AdapterStatus {
            adapter: self.id(),
            state,
            reason,
            detail,
            checked_at_ms: ohm_core::now_ms(),
            device_count,
        }
    }

    async fn discover(&self) -> Result<Vec<Device>> {
        self.scan_hwmon();
        let devices = self.build_devices();
        if devices.is_empty() {
            return Err(OhmError::AdapterUnavailable {
                adapter: ADAPTER_ID.to_string(),
                detail: "the operating system reported no usable sensors".to_string(),
            });
        }
        devices.iter().try_for_each(Device::validate)?;
        Ok(devices)
    }

    async fn read_state(&self, device: &Device) -> Result<DeviceState> {
        self.refresh();
        Ok(self.read_one(device))
    }

    async fn read_all(&self, devices: &[Device]) -> Vec<(DeviceId, Result<DeviceState>)> {
        // One refresh for the whole cycle, so every device sees the same
        // instant of CPU load.
        self.refresh();
        devices
            .iter()
            .map(|device| (device.id.clone(), Ok(self.read_one(device))))
            .collect()
    }

    async fn write(
        &self,
        device: &Device,
        capability: &Capability,
        _value: &Value,
    ) -> Result<WriteOutcome> {
        Err(OhmError::CapabilityNotWritable {
            device: device.id.to_string(),
            capability: capability.id.to_string(),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_device_model::{DeviceType, caps};

    /// A sysfs tree with one SuperIO chip: two fans, one driver-controlled.
    fn hwmon_fixture() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        let chip = temp.path().join("hwmon3");
        std::fs::create_dir_all(&chip).unwrap();
        std::fs::write(chip.join("name"), "nct6798d\n").unwrap();
        std::fs::write(chip.join("fan1_input"), "1200\n").unwrap();
        std::fs::write(chip.join("fan1_label"), "CPU Fan\n").unwrap();
        std::fs::write(chip.join("pwm1"), "128\n").unwrap();
        std::fs::write(chip.join("pwm1_enable"), "2\n").unwrap();
        std::fs::write(chip.join("fan2_input"), "800\n").unwrap();
        temp
    }

    #[tokio::test]
    async fn memory_is_reported_in_bytes() {
        let adapter = SystemAdapter::new();
        let devices = adapter.discover().await.unwrap();
        let memory = devices
            .iter()
            .find(|device| device.id == devices::memory_device_id())
            .expect("the machine has memory");

        assert_eq!(memory.device_type, DeviceType::Memory);
        assert!(memory.supports(caps::MEMORY_USED));
        assert!(memory.supports(caps::MEMORY_TOTAL));

        let state = adapter.read_state(memory).await.unwrap();
        let used = state.number(caps::MEMORY_USED).expect("used memory");
        let total = state.number(caps::MEMORY_TOTAL).expect("total memory");
        assert!(
            total > 0.0,
            "a machine reporting no memory is not believable"
        );
        assert!(used > 0.0 && used < total, "used {used} of {total}");
    }

    #[tokio::test]
    async fn hwmon_channels_become_read_only_fan_devices() {
        let temp = hwmon_fixture();
        let adapter = SystemAdapter::with_hwmon_root(Some(temp.path().to_path_buf()));
        let devices = adapter.discover().await.unwrap();

        let fans: Vec<&Device> = devices
            .iter()
            .filter(|device| device.id.as_str().starts_with("fan.system."))
            .collect();
        assert_eq!(
            fans.len(),
            2,
            "{:?}",
            fans.iter().map(|d| d.id.as_str()).collect::<Vec<_>>()
        );

        // Identity comes from the chip name and channel number, never from the
        // order the kernel enumerated the chips in.
        let first = fans
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan1")
            .expect("stable id");
        assert_eq!(first.name, "CPU Fan — nct6798d");
        assert!(first.supports(caps::FAN_RPM));
        assert!(first.supports(caps::FAN_PWM));
        assert!(
            !first.is_controllable(),
            "monitoring a channel is not controlling it"
        );
        assert!(
            first
                .capabilities
                .iter()
                .all(|capability| !capability.writable),
            "every hwmon capability is read-only in this release"
        );
        let mode = first.metadata.get("control_mode").expect("who owns it");
        assert!(mode.contains("driver controls"), "{mode}");

        let state = adapter.read_state(first).await.unwrap();
        assert_eq!(state.number(caps::FAN_RPM), Some(1200.0));
        let pwm = state.number(caps::FAN_PWM).expect("duty");
        assert!((pwm - 50.2).abs() < 0.1, "{pwm}");

        // The channel without a pwm file still reports its tachometer.
        let second = fans
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan2")
            .expect("second channel");
        assert!(second.supports(caps::FAN_RPM));
        assert!(!second.supports(caps::FAN_PWM));
        let state = adapter.read_state(second).await.unwrap();
        assert_eq!(state.number(caps::FAN_RPM), Some(800.0));
    }

    #[tokio::test]
    async fn a_fan_channel_that_disappears_reports_a_reason_not_zero() {
        let temp = hwmon_fixture();
        let adapter = SystemAdapter::with_hwmon_root(Some(temp.path().to_path_buf()));
        let devices = adapter.discover().await.unwrap();
        let fan = devices
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan1")
            .unwrap()
            .clone();

        std::fs::remove_file(temp.path().join("hwmon3/fan1_input")).unwrap();
        let state = adapter.read_state(&fan).await.unwrap();
        let reading = state.get(caps::FAN_RPM).expect("the reading is present");
        assert!(!reading.is_ok(), "a vanished file is not a zero");
        assert!(reading.reason().is_some());
        let detail = match &reading.status {
            ohm_device_model::ReadingStatus::Unavailable { detail, .. } => {
                detail.clone().unwrap_or_default()
            }
            ohm_device_model::ReadingStatus::Ok { .. } => String::new(),
        };
        assert!(detail.contains("fan1_input"), "{detail}");
    }

    #[tokio::test]
    async fn no_hwmon_root_means_no_fan_devices() {
        let adapter = SystemAdapter::with_hwmon_root(None);
        let devices = adapter.discover().await.unwrap();
        assert!(
            !devices
                .iter()
                .any(|device| device.id.as_str().starts_with("fan.system.")),
            "without a sysfs root there is nothing to read, and nothing is invented"
        );
    }

    #[tokio::test]
    async fn discovery_finds_the_machine() {
        let adapter = SystemAdapter::new();
        let devices = adapter.discover().await.unwrap();
        assert!(!devices.is_empty());
        for device in &devices {
            device.validate().unwrap();
            assert_eq!(device.adapter.as_str(), ADAPTER_ID);
            assert!(
                !device.is_controllable(),
                "the OS adapter never controls fans"
            );
        }
        assert!(devices.iter().any(|d| d.id == cpu_device_id()));
        assert_eq!(adapter.info().id.as_str(), ADAPTER_ID);
        assert!(!adapter.info().capabilities.can_write);
    }

    #[tokio::test]
    async fn cpu_readings_are_plausible_and_never_panic() {
        let adapter = SystemAdapter::new();
        let devices = adapter.discover().await.unwrap();
        let cpu = devices
            .iter()
            .find(|d| d.id == cpu_device_id())
            .expect("cpu present")
            .clone();

        // Two reads: the first establishes the CPU usage baseline.
        let _ = adapter.read_state(&cpu).await.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(250));
        let state = adapter.read_state(&cpu).await.unwrap();

        let load = state
            .number(caps::CPU_LOAD)
            .expect("load is always available");
        assert!((0.0..=100.0).contains(&load), "load out of range: {load}");

        let reading = state
            .get(caps::TEMPERATURE_CORE)
            .expect("temperature.core must always be reported, even when unavailable");
        if reading.is_ok() {
            let temperature = reading.number().unwrap();
            assert!((-40.0..=150.0).contains(&temperature), "{temperature}");
        } else {
            let reason = reading.reason().unwrap();
            assert!(
                matches!(
                    reason,
                    UnavailableReason::Unsupported | UnavailableReason::HardwareLimitation
                ),
                "unexpected reason: {reason}"
            );
        }
    }

    #[tokio::test]
    async fn storage_reports_space_and_an_honest_temperature() {
        let adapter = SystemAdapter::new();
        let devices = adapter.discover().await.unwrap();
        for disk in devices
            .iter()
            .filter(|d| d.device_type == DeviceType::Storage)
        {
            let state = adapter.read_state(disk).await.unwrap();
            assert!(
                state.number(caps::DISK_FREE).is_some(),
                "free space should always be readable"
            );
            let temperature = state.get(caps::TEMPERATURE_CORE).unwrap();
            if !temperature.is_ok() {
                assert!(temperature.reason().is_some());
            }
        }
    }

    #[tokio::test]
    async fn writing_is_always_refused() {
        let adapter = SystemAdapter::new();
        let devices = adapter.discover().await.unwrap();
        let cpu = devices
            .iter()
            .find(|d| d.id == cpu_device_id())
            .unwrap()
            .clone();
        let capability = cpu.capability_str(caps::CPU_LOAD).unwrap().clone();
        let err = adapter
            .write(&cpu, &capability, &Value::Number(100.0))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "capability_read_only");
        assert!(err.is_unsupported());
    }

    #[tokio::test]
    async fn probe_reports_availability_and_stays_usable() {
        let adapter = SystemAdapter::new();
        let status = adapter.probe().await;
        assert_eq!(status.adapter.as_str(), ADAPTER_ID);
        assert!(status.is_usable(), "the OS adapter is always usable");
        assert!(status.device_count > 0);
        if status.state == AdapterState::Degraded {
            assert!(status.detail.is_some());
            assert!(status.reason.is_some());
        }
    }

    #[tokio::test]
    async fn read_all_shares_one_refresh() {
        let adapter = SystemAdapter::new();
        let devices = adapter.discover().await.unwrap();
        let before = adapter.poll_count();
        let results = adapter.read_all(&devices).await;
        assert_eq!(results.len(), devices.len());
        assert!(results.iter().all(|(_, r)| r.is_ok()));
        assert_eq!(adapter.poll_count(), before + 1, "one refresh per cycle");
    }

    #[test]
    fn cpu_identity_is_reported() {
        let adapter = SystemAdapter::new();
        let brand = adapter.cpu_brand();
        let device = adapter
            .build_devices()
            .into_iter()
            .find(|d| d.id == cpu_device_id())
            .unwrap();
        assert!(!device.name.is_empty());
        assert!(device.metadata.contains_key("cores"));
        if brand.is_empty() {
            assert_eq!(device.name, "Processor");
        } else {
            assert_eq!(device.name, brand);
        }
        assert_eq!(SystemAdapter::default().info().namespace, ADAPTER_ID);
        assert_eq!(SystemAdapter::boxed().info().id.as_str(), ADAPTER_ID);
    }
}
