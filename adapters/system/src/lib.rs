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
    AdapterCapabilities, AdapterInfo, AdapterState, AdapterStatus, HardwareAdapter, TakenControl,
    WriteAccess, WriteOutcome,
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
    /// hwmon channels this build is allowed to write, by device id.
    ///
    /// Empty unless somebody listed a channel in
    /// `adapter_settings.system.pwm_write_allow`. Reading never depends on this.
    pwm_allow: std::collections::BTreeSet<String>,
    /// Channels whose ownership this adapter switched, and what it was.
    ///
    /// Filled when a write takes a channel away from its driver, drained by
    /// `shutdown` to put it back. Only channels that were actually switched appear
    /// here: a channel that was already manual has nothing to restore.
    taken: Mutex<std::collections::BTreeMap<String, hwmon::ControlMode>>,
}

impl Default for SystemAdapter {
    fn default() -> Self {
        Self::new()
    }
}

/// Environment variable that points the hwmon source at another tree.
///
/// Exists so a whole *process* — not just a unit test — can be run against a
/// prepared sysfs tree: the service's sensor-loss and recovery behaviour is a
/// question about what happens between two polls, and answering it needs a fan
/// channel that can be made to disappear while the process keeps running.
///
/// Reading only: writing `pwm<N>` is not implemented in this build, so pointing
/// this at a directory changes what is *read* and never what is written.
pub const ENV_HWMON_ROOT: &str = "OHM_HWMON_ROOT";

/// `/sys/class/hwmon` on Linux, or `$OHM_HWMON_ROOT` when that is set.
///
/// The override works on every platform, including the ones with no sysfs at all:
/// that is what makes the fixture usable where the code is developed.
fn default_hwmon_root() -> Option<std::path::PathBuf> {
    if let Some(override_root) = std::env::var_os(ENV_HWMON_ROOT) {
        // An empty value is a deliberate "no hwmon source here" rather than a path.
        if override_root.is_empty() {
            return None;
        }
        return Some(std::path::PathBuf::from(override_root));
    }
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

    /// An adapter allowed to write the listed hwmon channels, by device id.
    ///
    /// The list is a person's confirmation that `pwm<N>` drives the header
    /// `fan<N>_input` measures on *their* board. Nothing in the kernel states that,
    /// so nothing in this program can infer it, and an id that matches no device
    /// allows nothing.
    pub fn with_pwm_allow(allow: &[String]) -> Arc<dyn HardwareAdapter> {
        let mut adapter = Self::with_hwmon_root(default_hwmon_root());
        adapter.pwm_allow = allow.iter().cloned().collect();
        Arc::new(adapter)
    }

    /// `true` when this channel may be written.
    fn pwm_writable(&self, device_id: &str) -> bool {
        self.pwm_allow.contains(device_id)
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
            // Read-only until somebody says otherwise, per channel.
            pwm_allow: std::collections::BTreeSet::new(),
            taken: Mutex::new(std::collections::BTreeMap::new()),
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
                    // A writable duty, and *only* for a channel somebody listed.
                    // `pwm<N>` and `fan<N>_input` share a channel number, which is
                    // not a promise that they are the same physical header; that
                    // pairing is a property of the board and no kernel interface
                    // states it, so a person confirms it per channel.
                    let writable = self.pwm_writable(&fan.device_id()) && fan.pwm.is_some();
                    if writable {
                        capabilities.push(Capability::actuator(
                            ohm_device_model::caps::FAN_SPEED_PERCENT,
                            "Fan Duty",
                            Unit::Percent,
                            0.0,
                            100.0,
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
                    .with_metadata(
                        "source",
                        if writable {
                            "linux hwmon (writable: this channel was allowed in settings)"
                        } else {
                            "linux hwmon (read-only)"
                        },
                    );
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

    /// Space a user can write on this mount, straight from `statvfs`.
    ///
    /// `f_bavail` (not `f_bfree`) is what `df` calls "Available": the blocks the current
    /// user may actually use, with the reserved-for-root portion excluded. `f_frsize` is
    /// the block size to multiply by, with `f_bsize` as the fallback some filesystems
    /// leave as the only meaningful one.
    #[cfg(unix)]
    fn available_bytes(mount: &str) -> Option<u64> {
        // `rustix` rather than `libc`: this crate denies `unsafe_code`, and a syscall
        // wrapper is exactly the kind of thing that should not be hand-rolled to get
        // around that.
        let stats = rustix::fs::statvfs(mount).ok()?;
        let block = if stats.f_frsize > 0 {
            stats.f_frsize as u64
        } else {
            stats.f_bsize as u64
        };
        Some((stats.f_bavail as u64).saturating_mul(block))
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
            let (mut free_space, name, mount) = {
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
            // On macOS the library answer is Apple's "available capacity", which counts
            // space the system can reclaim later (purgeable). Every tool a person would
            // check against — `df`, `diskutil`, `statvfs` — reports the space that can be
            // written *now*, and on the machine this was found on the two differed by
            // 6.1 GB. A reading that matches no platform tool is not checkable, so on
            // Unix the kernel is asked directly.
            #[cfg(unix)]
            {
                free_space = Self::available_bytes(&mount).or(free_space);
            }
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
        // Writability is per channel and comes from configuration, so the adapter's
        // own answer depends on it: an adapter with no allowed channel is exactly
        // the read-only provider it has always been.
        let capabilities = if self.pwm_allow.is_empty() {
            AdapterCapabilities::read_only()
        } else {
            // Writing `pwm<N>` needs root on Linux; and this adapter *does* put the
            // channel back under its driver on shutdown, which is why it says so.
            AdapterCapabilities::cooling_control().hands_back_control()
        };
        let description = if self.pwm_allow.is_empty() {
            "CPU utilisation and clock, OS thermal zones, disk space and hwmon fan \
             tachometers. Read-only: no hwmon channel is allowed to be written."
        } else {
            "CPU utilisation and clock, OS thermal zones, disk space and hwmon fan \
             tachometers, with the channels listed in \
             adapter_settings.system.pwm_write_allow writable. Needs root to write; \
             hands control back to the driver on shutdown."
        };
        AdapterInfo::new(ADAPTER_ID, ADAPTER_NAME, ADAPTER_ID)
            .with_description(description)
            .with_capabilities(capabilities)
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
        value: &Value,
    ) -> Result<WriteOutcome> {
        let not_writable = || OhmError::CapabilityNotWritable {
            device: device.id.to_string(),
            capability: capability.id.to_string(),
        };

        if !device.id.as_str().starts_with("fan.system.")
            || capability.id.as_str() != ohm_device_model::caps::FAN_SPEED_PERCENT
        {
            return Err(not_writable());
        }
        if !self.pwm_writable(device.id.as_str()) {
            // A capability that is not advertised cannot be targeted by a rule, so
            // reaching here means something bypassed the registry. Refusing with the
            // reason is the only honest answer.
            return Ok(WriteOutcome::rejected(format!(
                "{} is not in adapter_settings.system.pwm_write_allow, so this build \
                 does not write it",
                device.id
            )));
        }
        let Some(percent) = value.as_f64() else {
            return Ok(WriteOutcome::rejected(format!(
                "a fan duty has to be a number, not {value}"
            )));
        };
        let Some(fan) = self.hwmon_channel(device) else {
            return Ok(WriteOutcome::rejected(
                "the kernel no longer exposes this fan channel".to_string(),
            ));
        };

        // Take the channel away from its driver only if it is not already ours, and
        // only when we could read who owns it. `Err` here is a refusal to write at
        // all: a channel whose owner is unknown is not one to take over.
        match fan.take_control() {
            Ok(Some(previous)) => {
                self.taken
                    .lock()
                    .insert(device.id.to_string(), previous.clone());
                tracing::info!(
                    channel = %device.id,
                    previous = %previous.describe(),
                    "took manual control of an hwmon channel"
                );
            }
            Ok(None) => {}
            Err((_reason, detail)) => {
                return Ok(WriteOutcome::rejected(detail));
            }
        }

        match fan.set_pwm(percent) {
            Ok(raw) => {
                let mut outcome = WriteOutcome::applied(
                    // The kernel's own units, so the report says what the file was
                    // given as well as what was asked for.
                    Value::Number((percent * 10.0).round() / 10.0),
                );
                outcome.detail = Some(format!(
                    "wrote {raw} to {} ({} % of the interface's 255)",
                    fan.pwm
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "pwm".to_string()),
                    percent.round()
                ));
                Ok(outcome)
            }
            Err((reason, detail)) => {
                // A failed write must not leave the channel switched to us.
                if let Some(previous) = self.taken.lock().remove(device.id.as_str()) {
                    let _ = fan.restore_control(Some(previous));
                }
                Ok(WriteOutcome::rejected(format!(
                    "{}: {detail}",
                    reason.as_str()
                )))
            }
        }
    }

    /// Ask the kernel whether this process may write the channel, before anything is
    /// switched.
    ///
    /// The answer comes from the filesystem rather than from a comparison of user ids:
    /// opening `pwm<N>` for writing is refused by the kernel when the process is not
    /// allowed, and it is refused for every other reason too (a read-only mount, a
    /// policy module, a file that is not there). The platform's own message is part of
    /// the refusal because that is the sentence that tells a person what to fix — and
    /// the open is closed again without writing, so asking changes nothing.
    fn write_access(&self, device: &Device, capability: &Capability) -> WriteAccess {
        let wanted = matches!(
            capability.id.as_str(),
            ohm_device_model::caps::FAN_SPEED_PERCENT | ohm_device_model::caps::PUMP_SPEED_PERCENT
        );
        if !device.id.as_str().starts_with("fan.system.") || !wanted {
            // Not something this adapter offers to write, so it has no opinion about it.
            return WriteAccess::Unknown;
        }
        if !self.pwm_writable(device.id.as_str()) {
            return WriteAccess::Denied(format!(
                "{} is not in adapter_settings.system.pwm_write_allow, so this build does not \
                 write it",
                device.id
            ));
        }
        let Some(fan) = self.hwmon_channel(device) else {
            return WriteAccess::Denied(
                "the kernel no longer exposes this fan channel".to_string(),
            );
        };
        let Some(path) = fan.pwm.clone() else {
            return WriteAccess::Denied(format!(
                "{} has no pwm file, so there is nothing to write",
                device.id
            ));
        };
        match std::fs::OpenOptions::new().write(true).open(&path) {
            Ok(_) => WriteAccess::Permitted,
            Err(error) => WriteAccess::Denied(format!(
                "the platform refuses to open {} for writing: {error}",
                path.display()
            )),
        }
    }

    /// What this adapter switched away from its driver and still owes back.
    ///
    /// Published in the service's state file, so a process that takes over after this
    /// one is killed can finish the job instead of reading the switched value as the
    /// original.
    fn taken_controls(&self) -> Vec<TakenControl> {
        self.taken
            .lock()
            .iter()
            .map(|(device_id, mode)| TakenControl {
                device_id: device_id.clone(),
                original: mode.restorable_value(),
                original_text: mode.describe(),
            })
            .collect()
    }

    /// Take responsibility for channels a previous process switched.
    ///
    /// The recorded value is remembered as *this* process's original — that is what
    /// makes its own shutdown put the channel back where it started — and it is also
    /// written back straight away, because a channel left in manual mode by a process
    /// that died should not wait for this one to exit cleanly as well. Channels this
    /// adapter is not allowed to write are skipped and said so: adopting one would be
    /// touching hardware the configuration does not permit.
    fn adopt_taken_controls(&self, taken: &[TakenControl]) -> Vec<String> {
        let mut problems = Vec::new();
        for entry in taken {
            if !self.pwm_allow.contains(&entry.device_id) {
                problems.push(format!(
                    "{}: left in manual mode by a previous owner, but it is not in \
                     `adapter_settings.system.pwm_write_allow`, so this process may not put it \
                     back ({})",
                    entry.device_id, entry.original_text
                ));
                continue;
            }
            let Some(value) = entry.original else {
                problems.push(format!(
                    "{}: the previous owner did not record a value it could write back ({}), so \
                     this channel is left exactly as it is rather than guessed at",
                    entry.device_id, entry.original_text
                ));
                continue;
            };

            let mode = hwmon::ControlMode::from_restored(value);
            self.taken
                .lock()
                .insert(entry.device_id.clone(), mode.clone());
            let tree = self.hwmon.lock();
            let Some(fan) = tree
                .as_ref()
                .and_then(|tree| tree.channel_by_device_id(&entry.device_id))
            else {
                problems.push(format!(
                    "{}: adopted, but the channel is not in this hwmon tree, so it cannot be \
                     written back until it returns",
                    entry.device_id
                ));
                continue;
            };
            match fan.restore_control(Some(mode.clone())) {
                Ok(()) => tracing::info!(
                    channel = %entry.device_id,
                    restored = value,
                    "put a channel left in manual mode by a previous owner back under its driver"
                ),
                Err(detail) => problems.push(format!("{}: {detail}", entry.device_id)),
            }
        }
        problems
    }

    /// Put every channel this adapter switched back under its original owner.
    ///
    /// This is what "hand back to firmware control" means for hwmon: the driver's
    /// curve resumes when `pwm<N>_enable` says so again. A channel whose original
    /// mode could not be read is reported rather than guessed at, and every channel
    /// is attempted even if one fails.
    async fn shutdown(&self) -> Result<()> {
        let taken: Vec<(String, hwmon::ControlMode)> = {
            let mut guard = self.taken.lock();
            let entries = guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            guard.clear();
            entries
        };
        if taken.is_empty() {
            return Ok(());
        }

        let failures: Vec<String> = {
            let tree = self.hwmon.lock();
            taken
                .into_iter()
                .filter_map(|(device_id, previous)| {
                    let fan = tree
                        .as_ref()
                        .and_then(|tree| tree.channel_by_device_id(&device_id));
                    match fan {
                        Some(fan) => fan.restore_control(Some(previous)).err(),
                        None => Some(format!(
                            "{device_id}: the channel is gone, so its owner cannot be restored"
                        )),
                    }
                })
                .collect()
        };

        if failures.is_empty() {
            tracing::info!("hwmon channels handed back to their drivers");
            return Ok(());
        }
        Err(OhmError::Adapter {
            adapter: ADAPTER_ID.to_string(),
            detail: format!(
                "could not hand every hwmon channel back: {}",
                failures.join("; ")
            ),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_adapter_api::WriteStatus;
    // Mode bits are how the refusal below is produced, and they are Unix-only; the
    // helper and the test that uses them carry the same gate.
    use ohm_core::CapabilityId;
    use ohm_device_model::{DeviceType, caps};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

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

    /// The opt-in path, end to end: capability, write, and hand-back.
    ///
    /// The allow-list is a person's confirmation that `pwm<N>` drives the header
    /// `fan<N>_input` measures on their board — nothing in the kernel says so — and
    /// these tests are about what the code does once that confirmation exists.
    #[tokio::test]
    async fn an_allowed_channel_becomes_writable_and_the_others_do_not() {
        let temp = hwmon_fixture();
        let allow = vec!["fan.system.nct6798d_fan1".to_string()];
        let mut adapter = SystemAdapter::with_hwmon_root(Some(temp.path().to_path_buf()));
        adapter.pwm_allow = allow.into_iter().collect();

        let devices = adapter.discover().await.unwrap();
        let allowed = devices
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan1")
            .expect("the allowed channel");
        assert!(
            allowed.is_controllable(),
            "an allowed channel is drivable: {:?}",
            allowed.capabilities
        );
        assert!(allowed.supports(caps::FAN_SPEED_PERCENT));

        let other = devices
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan2")
            .expect("the other channel");
        assert!(
            !other.is_controllable(),
            "a channel nobody allowed stays read-only: {:?}",
            other.capabilities
        );
        assert!(!other.supports(caps::FAN_SPEED_PERCENT));

        // And the adapter says so about itself, including the hand-back.
        let info = adapter.info();
        assert!(info.capabilities.can_write);
        assert!(info.capabilities.can_control_cooling);
        assert!(
            info.capabilities.hands_back_control_on_shutdown,
            "this adapter puts the channel back under its driver"
        );
        assert!(
            info.capabilities.write_requires_admin,
            "writing pwm<N> needs root on Linux"
        );
    }

    #[tokio::test]
    async fn writing_an_allowed_channel_takes_control_and_shutdown_puts_it_back() {
        let temp = hwmon_fixture();
        let mut adapter = SystemAdapter::with_hwmon_root(Some(temp.path().to_path_buf()));
        adapter.pwm_allow = ["fan.system.nct6798d_fan1".to_string()]
            .into_iter()
            .collect();

        let devices = adapter.discover().await.unwrap();
        let device = devices
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan1")
            .expect("the channel");
        let capability = device
            .capability(&CapabilityId::new_unchecked(caps::FAN_SPEED_PERCENT))
            .unwrap();

        // The fixture's chip is driver-controlled (`pwm1_enable` = 2).
        let enable = temp.path().join("hwmon3/pwm1_enable");
        assert_eq!(std::fs::read_to_string(&enable).unwrap().trim(), "2");

        let outcome = adapter
            .write(device, capability, &Value::Number(45.0))
            .await
            .expect("a write");
        assert_eq!(outcome.status, WriteStatus::Applied);
        assert_eq!(outcome.applied, Some(Value::Number(45.0)));
        assert!(
            outcome
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("wrote 115"),
            "45 % of 255 is 115, and the report says what the file was given: {:?}",
            outcome.detail
        );
        assert_eq!(
            std::fs::read_to_string(temp.path().join("hwmon3/pwm1"))
                .unwrap()
                .trim(),
            "115"
        );
        assert_eq!(
            std::fs::read_to_string(&enable).unwrap().trim(),
            "1",
            "manual, so a driver curve cannot overwrite the value"
        );

        // Shutdown is where control goes back.
        adapter.shutdown().await.expect("hand-back succeeds");
        assert_eq!(
            std::fs::read_to_string(&enable).unwrap().trim(),
            "2",
            "the driver owns the channel again"
        );
        // And the duty it last wrote is left where the runtime's fail-safe put it —
        // the mode is what governs, so the leftover byte is the driver's business.
    }

    #[tokio::test]
    async fn a_channel_that_was_not_allowed_refuses_to_be_written() {
        let temp = hwmon_fixture();
        let adapter = SystemAdapter::with_hwmon_root(Some(temp.path().to_path_buf()));
        let devices = adapter.discover().await.unwrap();
        let device = devices
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan1")
            .expect("the channel");

        // No allow-list at all: the capability is not even advertised, but a caller
        // that reaches the write path anyway must be told why, not given silence.
        let capability = Capability::actuator(
            caps::FAN_SPEED_PERCENT,
            "Fan Duty",
            Unit::Percent,
            0.0,
            100.0,
        );
        let outcome = adapter
            .write(device, &capability, &Value::Number(45.0))
            .await
            .expect("a refusal, not an error");
        assert_eq!(outcome.status, WriteStatus::Rejected);
        assert!(
            outcome
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("pwm_write_allow"),
            "{:?}",
            outcome.detail
        );
        assert_eq!(
            std::fs::read_to_string(temp.path().join("hwmon3/pwm1"))
                .unwrap()
                .trim(),
            "128",
            "and nothing was written"
        );
    }

    #[tokio::test]
    async fn a_read_only_capability_of_an_allowed_channel_is_still_refused() {
        let temp = hwmon_fixture();
        let mut adapter = SystemAdapter::with_hwmon_root(Some(temp.path().to_path_buf()));
        adapter.pwm_allow = ["fan.system.nct6798d_fan1".to_string()]
            .into_iter()
            .collect();
        let devices = adapter.discover().await.unwrap();
        let device = devices
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan1")
            .expect("the channel");
        let rpm = device
            .capability(&CapabilityId::new_unchecked(caps::FAN_RPM))
            .unwrap();
        assert!(
            adapter
                .write(device, rpm, &Value::Number(1200.0))
                .await
                .is_err(),
            "a tachometer is not an actuator"
        );
    }

    #[tokio::test]
    async fn shutdown_without_a_write_has_nothing_to_hand_back() {
        let temp = hwmon_fixture();
        let mut adapter = SystemAdapter::with_hwmon_root(Some(temp.path().to_path_buf()));
        adapter.pwm_allow = ["fan.system.nct6798d_fan1".to_string()]
            .into_iter()
            .collect();
        let _ = adapter.discover().await.unwrap();
        adapter.shutdown().await.expect("no-op shutdown");
        assert_eq!(
            std::fs::read_to_string(temp.path().join("hwmon3/pwm1_enable"))
                .unwrap()
                .trim(),
            "2",
            "a channel nobody wrote is left exactly as it was"
        );
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

    // --- handing a switched channel over to the next process ---------------------

    /// An adapter pointed at a fixture tree, allowed to write the listed channels.
    fn writable(root: &std::path::Path, allow: &[&str]) -> SystemAdapter {
        let mut adapter = SystemAdapter::with_hwmon_root(Some(root.to_path_buf()));
        adapter.pwm_allow = allow.iter().map(|id| id.to_string()).collect();
        adapter
    }

    /// The same, with its tree already read.
    ///
    /// Adoption writes a channel back, and a channel is something discovery found: the
    /// runtime starts (which enumerates) before it adopts, so the tests do too.
    async fn writable_and_discovered(root: &std::path::Path, allow: &[&str]) -> SystemAdapter {
        let adapter = writable(root, allow);
        adapter.discover().await.expect("discovery");
        adapter
    }

    fn tree_file(root: &std::path::Path, file: &str) -> String {
        std::fs::read_to_string(root.join("hwmon3").join(file))
            .unwrap()
            .trim()
            .to_string()
    }

    fn a_recorded_original(value: Option<i64>) -> TakenControl {
        TakenControl {
            device_id: "fan.system.nct6798d_fan1".to_string(),
            original: value,
            original_text: "the driver controls this channel".to_string(),
        }
    }

    #[tokio::test]
    async fn what_this_adapter_switched_is_reported_for_the_next_owner() {
        let temp = hwmon_fixture();
        let adapter = writable(temp.path(), &["fan.system.nct6798d_fan1"]);
        let devices = adapter.discover().await.unwrap();
        let device = devices
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan1")
            .expect("the channel");
        let capability = device
            .capability(&CapabilityId::new_unchecked(caps::FAN_SPEED_PERCENT))
            .unwrap();

        // Nothing is owed before a write.
        assert!(adapter.taken_controls().is_empty());

        adapter
            .write(device, capability, &Value::Number(45.0))
            .await
            .expect("a write");
        assert_eq!(
            tree_file(temp.path(), "pwm1_enable"),
            "1",
            "it took control"
        );

        // The record the next process needs: which channel, and what it was.
        let taken = adapter.taken_controls();
        assert_eq!(taken.len(), 1, "{taken:?}");
        assert_eq!(taken[0].device_id, "fan.system.nct6798d_fan1");
        assert_eq!(taken[0].original, Some(2));
        assert!(taken[0].is_restorable());
        assert!(
            taken[0].original_text.contains("driver controls"),
            "{}",
            taken[0].original_text
        );

        adapter.shutdown().await.unwrap();
        assert_eq!(
            tree_file(temp.path(), "pwm1_enable"),
            "2",
            "and it gave it back"
        );
        assert!(
            adapter.taken_controls().is_empty(),
            "nothing is owed once it has been given back"
        );
    }

    #[tokio::test]
    async fn a_channel_left_switched_by_a_dead_process_is_put_back() {
        let temp = hwmon_fixture();
        // What a process that was killed leaves behind: the channel switched to manual
        // and its record the only place the original value still exists.
        std::fs::write(temp.path().join("hwmon3/pwm1_enable"), "1\n").unwrap();
        let adapter = writable_and_discovered(temp.path(), &["fan.system.nct6798d_fan1"]).await;

        let problems = adapter.adopt_taken_controls(&[a_recorded_original(Some(2))]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            tree_file(temp.path(), "pwm1_enable"),
            "2",
            "the channel goes back under its driver straight away, not at some later exit"
        );
        // And this process now owes it back too, so its own clean stop is idempotent
        // rather than a second opinion.
        let taken = adapter.taken_controls();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].original, Some(2));
    }

    #[tokio::test]
    async fn the_exact_mode_is_restored_not_the_nearest_known_one() {
        let temp = hwmon_fixture();
        std::fs::write(temp.path().join("hwmon3/pwm1_enable"), "1\n").unwrap();
        let adapter = writable_and_discovered(temp.path(), &["fan.system.nct6798d_fan1"]).await;

        // 3 is "automatic, using the driver's own curve"; 2 is plain automatic. A
        // hand-over must not decay one into the other.
        let problems = adapter.adopt_taken_controls(&[a_recorded_original(Some(3))]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(tree_file(temp.path(), "pwm1_enable"), "3");
    }

    #[tokio::test]
    async fn a_recorded_mode_that_could_not_be_read_is_refused_rather_than_guessed() {
        let temp = hwmon_fixture();
        std::fs::write(temp.path().join("hwmon3/pwm1_enable"), "1\n").unwrap();
        let adapter = writable_and_discovered(temp.path(), &["fan.system.nct6798d_fan1"]).await;

        let mut entry = a_recorded_original(None);
        entry.original_text = "control mode unreadable (read_error): not a number".to_string();
        let problems = adapter.adopt_taken_controls(&[entry]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("did not record a value it could write back"),
            "{problems:?}"
        );
        assert_eq!(
            tree_file(temp.path(), "pwm1_enable"),
            "1",
            "a mode nobody could read is never written back as a guess"
        );
        assert!(
            adapter.taken_controls().is_empty(),
            "and this process does not claim to owe a channel it cannot put back"
        );
    }

    #[tokio::test]
    async fn a_channel_this_adapter_may_not_write_is_left_alone_and_said_so() {
        let temp = hwmon_fixture();
        std::fs::write(temp.path().join("hwmon3/pwm1_enable"), "1\n").unwrap();
        // The configuration does not confirm this channel, so this build must not
        // touch it — recovering somebody else's mistake is not an exception to that.
        let adapter = writable_and_discovered(temp.path(), &[]).await;

        let problems = adapter.adopt_taken_controls(&[a_recorded_original(Some(2))]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("pwm_write_allow"),
            "the refusal names the setting: {problems:?}"
        );
        assert_eq!(tree_file(temp.path(), "pwm1_enable"), "1");
    }

    // --- what the platform says before a write is attempted -----------------------

    /// Whether this process can write a file it does not own, which is the only thing
    /// that makes the "refused" case below reproducible: root ignores the mode bits, so
    /// a test that assumes a refusal would pass for the wrong reason when run as root.
    #[cfg(unix)]
    fn running_as_root() -> bool {
        let path = std::env::temp_dir().join(format!("ohm-root-probe-{}", std::process::id()));
        std::fs::write(&path, b"probe").unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let writable = std::fs::OpenOptions::new().write(true).open(&path).is_ok();
        permissions.set_mode(0o600);
        let _ = std::fs::set_permissions(&path, permissions);
        let _ = std::fs::remove_file(&path);
        writable
    }

    fn fan_capability() -> CapabilityId {
        CapabilityId::new_unchecked(caps::FAN_SPEED_PERCENT)
    }

    #[tokio::test]
    async fn write_access_is_permitted_for_a_channel_the_configuration_confirms() {
        let temp = hwmon_fixture();
        let adapter = writable_and_discovered(temp.path(), &["fan.system.nct6798d_fan1"]).await;
        let devices = adapter.discover().await.unwrap();
        let device = devices
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan1")
            .expect("the channel");
        let capability = device.capability(&fan_capability()).unwrap();

        let access = adapter.write_access(device, capability);
        assert_eq!(access, WriteAccess::Permitted, "{access:?}");
        assert!(access.is_permitted());
        assert!(access.denial().is_none());
    }

    #[tokio::test]
    async fn write_access_names_the_setting_that_would_allow_the_channel() {
        let temp = hwmon_fixture();
        let adapter = writable_and_discovered(temp.path(), &[]).await;
        let devices = adapter.discover().await.unwrap();
        let device = devices
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan1")
            .expect("the channel");
        // Nothing is confirmed, so the device does not advertise the actuator at all —
        // which is itself the read-only default. The refusal is still answerable for a
        // channel somebody asks about, so the capability is built directly.
        assert!(
            device.capability(&fan_capability()).is_none(),
            "an unconfirmed channel exposes no writable capability"
        );
        let capability =
            Capability::actuator(fan_capability(), "Fan Speed", Unit::Percent, 0.0, 100.0);

        let access = adapter.write_access(device, &capability);
        let reason = access.denial().expect("a refusal");
        assert!(reason.contains("pwm_write_allow"), "{reason}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_access_reports_the_platform_refusing_the_file() {
        if running_as_root() {
            // Root can write a file whose mode forbids it, so the refusal cannot be
            // produced here; the case is covered wherever the tests run unprivileged.
            return;
        }
        let temp = hwmon_fixture();
        let adapter = writable_and_discovered(temp.path(), &["fan.system.nct6798d_fan1"]).await;
        let devices = adapter.discover().await.unwrap();
        let device = devices
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan1")
            .expect("the channel");
        let capability = device.capability(&fan_capability()).unwrap();

        // What a user without root, or on a read-only mount, actually has.
        let pwm = temp.path().join("hwmon3/pwm1");
        let mut permissions = std::fs::metadata(&pwm).unwrap().permissions();
        permissions.set_mode(0o444);
        std::fs::set_permissions(&pwm, permissions).unwrap();

        let access = adapter.write_access(device, capability);
        let reason = access.denial().unwrap_or_else(|| panic!("{access:?}"));
        assert!(
            reason.contains("pwm1"),
            "the refusal names the file: {reason}"
        );
        assert!(
            reason.contains("refuses to open"),
            "and says who refused: {reason}"
        );
    }

    #[tokio::test]
    async fn write_access_is_unknown_for_something_this_adapter_does_not_offer() {
        let temp = hwmon_fixture();
        let adapter = writable_and_discovered(temp.path(), &["fan.system.nct6798d_fan1"]).await;
        let devices = adapter.discover().await.unwrap();
        let device = devices
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan1")
            .expect("the channel");
        let other = CapabilityId::new_unchecked(caps::TEMPERATURE_CORE);

        assert_eq!(
            adapter.write_access(
                device,
                &Capability::sensor(other.clone(), "Core", Unit::Celsius)
            ),
            WriteAccess::Unknown,
            "an adapter has no opinion about a capability it does not write"
        );
    }

    #[tokio::test]
    async fn write_access_says_when_the_channel_is_no_longer_exposed() {
        // Not discovered yet, so this adapter has no tree to look the channel up in —
        // the same answer a channel that went away gets, and never a hopeful yes.
        let temp = hwmon_fixture();
        let adapter = writable(temp.path(), &["fan.system.nct6798d_fan1"]);
        let other = SystemAdapter::with_hwmon_root(Some(temp.path().to_path_buf()));
        let devices = other.discover().await.unwrap();
        let device = devices
            .iter()
            .find(|device| device.id.as_str() == "fan.system.nct6798d_fan1")
            .expect("the channel");
        let capability =
            Capability::actuator(fan_capability(), "Fan Speed", Unit::Percent, 0.0, 100.0);

        let access = adapter.write_access(device, &capability);
        assert!(
            access
                .denial()
                .unwrap_or_default()
                .contains("no longer exposes"),
            "{access:?}"
        );
    }

    /// The free-space reading has to be the number the platform's own tools print.
    ///
    /// On macOS the library's answer was Apple's "available capacity", which counts space
    /// the system may reclaim later: on the machine this was found on it read 48.39 GB
    /// while `df`, `diskutil` and `statvfs` all said 42.29 GB, so the reading matched no
    /// tool a person could check it with — and CI's macOS job said so. This test asks the
    /// kernel the same question the reading does and requires the answers to agree
    /// (allowing for the filesystem changing between the two calls).
    #[cfg(unix)]
    #[tokio::test]
    async fn the_free_space_reading_is_the_number_df_prints() {
        let adapter = writable_and_discovered(std::path::Path::new("/"), &[]).await;
        let devices = adapter.discover().await.unwrap();
        let Some(device) = devices
            .iter()
            .find(|device| device.id.as_str() == "storage.system.0")
        else {
            return; // no non-removable storage on this machine
        };
        let state = adapter.read_state(device).await.unwrap();
        let ours = state.number(caps::DISK_FREE).expect("a free-space reading");
        let mount = device
            .metadata
            .get("mount_point")
            .cloned()
            .unwrap_or_default();
        let stats = rustix::fs::statvfs(mount.as_str()).expect("statvfs");
        let platform = (stats.f_bavail as u64).saturating_mul(if stats.f_frsize > 0 {
            stats.f_frsize as u64
        } else {
            stats.f_bsize as u64
        });

        let difference = (ours as i64 - platform as i64).abs();
        assert!(
            difference < 64 * 1024 * 1024,
            "ours {ours} vs statvfs {platform} on {mount} (delta {difference} B)"
        );
    }
}
