//! Windows specific sensor sources.
//!
//! Everything here is *best effort* and must degrade to "unavailable" rather
//! than fail: the OS APIs involved are inconsistent across hardware revisions
//! and are frequently disabled by policy or by the vendor's own driver.
//!
//! What this module deliberately does **not** do:
//!
//! * **Fan RPM / fan control** — `Win32_Fan` is documented by Microsoft as not
//!   implementing `SetSpeed`, and in practice returns no instances on most
//!   consumer boards; SuperIO access is what actually works, and that is
//!   LibreHardwareMonitor's job (see the `lhm` adapter). Reporting a fan we
//!   cannot read would be worse than reporting none.
//! * **CPU package temperature** — `MSAcpi_ThermalZoneTemperature` is usually
//!   empty on desktops; the temperature component from `sysinfo` is tried
//!   first, and the LHM adapter is the reliable source.
//!
//! Storage temperature is the exception worth attempting, because Windows does
//! expose it directly for NVMe/SATA drives through the storage reliability
//! counters.

#![cfg(windows)]

use std::collections::HashMap;

use ohm_device_model::UnavailableReason;

/// Why a Windows probe could not produce data.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeFailure {
    /// WMI itself is unavailable (rare, but happens in stripped images).
    WmiUnavailable(String),
    /// The provider answered but returned nothing.
    NoData,
    /// Access was refused.
    AccessDenied,
    /// The query failed.
    QueryFailed(String),
}

impl ProbeFailure {
    pub fn reason(&self) -> UnavailableReason {
        match self {
            Self::WmiUnavailable(_) | Self::QueryFailed(_) => UnavailableReason::ReadError,
            Self::NoData => UnavailableReason::HardwareLimitation,
            Self::AccessDenied => UnavailableReason::PermissionDenied,
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::WmiUnavailable(detail) => format!("WMI is unavailable: {detail}"),
            Self::NoData => "the driver reports no reliability counters for this device".into(),
            Self::AccessDenied => "access denied; run as Administrator".into(),
            Self::QueryFailed(detail) => format!("query failed: {detail}"),
        }
    }
}

/// Storage temperatures keyed by a lowercase device identifier fragment
/// (`physicaldrive0`, or the model string).
///
/// Uses `MSFT_StorageReliabilityCounter.Temperature`, which reports degrees
/// Celsius for NVMe and most SATA SSDs. Drives that do not implement it simply
/// do not appear in the result.
pub fn storage_temperatures() -> Result<HashMap<String, f64>, ProbeFailure> {
    // wmi 0.18 initializes COM as needed. Reliability counters live in the
    // Storage provider's namespace, not the default ROOT\CIMV2 namespace.
    let connection = wmi::WMIConnection::with_namespace_path(r"ROOT\Microsoft\Windows\Storage")
        .map_err(|e| ProbeFailure::WmiUnavailable(e.to_string()))?;

    let rows: Vec<HashMap<String, wmi::Variant>> = connection
        .raw_query("SELECT DeviceId, Temperature FROM MSFT_StorageReliabilityCounter")
        .map_err(|e| ProbeFailure::QueryFailed(e.to_string()))?;

    if rows.is_empty() {
        return Err(ProbeFailure::NoData);
    }

    let mut out = HashMap::new();
    for row in rows {
        let Some(device) = row.get("DeviceId").and_then(variant_to_string) else {
            continue;
        };
        let Some(temperature) = row.get("Temperature").and_then(variant_to_f64) else {
            continue;
        };
        // 0 is what the provider returns when the drive does not report a
        // temperature at all; it is not a plausible die temperature.
        if !(1.0..=120.0).contains(&temperature) {
            continue;
        }
        out.insert(device.to_ascii_lowercase(), temperature);
    }

    if out.is_empty() {
        Err(ProbeFailure::NoData)
    } else {
        Ok(out)
    }
}

/// Best effort lookup of a temperature for one disk.
///
/// Matching is deliberately loose: WMI identifies drives as `\\.\PHYSICALDRIVE0`
/// while `sysinfo` reports names and mount points, so we accept a match on the
/// numeric index, the model string or the mount point.
pub fn temperature_for(
    temperatures: &HashMap<String, f64>,
    disk_name: &str,
    mount_point: &str,
) -> Option<f64> {
    let name = disk_name.to_ascii_lowercase();
    let mount = mount_point.to_ascii_lowercase();
    temperatures
        .iter()
        .find(|(device, _)| {
            let device = device.to_ascii_lowercase();
            (!name.is_empty() && (device.contains(&name) || name.contains(&device)))
                || (!mount.is_empty() && device.contains(&mount))
        })
        .map(|(_, temperature)| *temperature)
}

fn variant_to_string(value: &wmi::Variant) -> Option<String> {
    match value {
        wmi::Variant::String(text) => Some(text.clone()),
        wmi::Variant::Null | wmi::Variant::Empty => None,
        other => Some(format!("{other:?}")),
    }
}

fn variant_to_f64(value: &wmi::Variant) -> Option<f64> {
    match value {
        wmi::Variant::UI1(v) => Some(f64::from(*v)),
        wmi::Variant::UI2(v) => Some(f64::from(*v)),
        wmi::Variant::UI4(v) => Some(f64::from(*v)),
        wmi::Variant::I1(v) => Some(f64::from(*v)),
        wmi::Variant::I2(v) => Some(f64::from(*v)),
        wmi::Variant::I4(v) => Some(f64::from(*v)),
        wmi::Variant::R4(v) => Some(f64::from(*v)),
        wmi::Variant::R8(v) => Some(*v),
        wmi::Variant::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// Is the current process elevated?
///
/// Used only for reporting: the UI explains *why* a write will fail instead of
/// letting the user discover it through an error.
pub fn is_elevated() -> bool {
    // `net session` requires an elevated token; spawning it is the least
    // invasive reliable check on Windows.
    std::process::Command::new("net")
        .arg("session")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_reasons_map_to_the_runtime_vocabulary() {
        assert_eq!(
            ProbeFailure::NoData.reason(),
            UnavailableReason::HardwareLimitation
        );
        assert_eq!(
            ProbeFailure::AccessDenied.reason(),
            UnavailableReason::PermissionDenied
        );
        assert!(ProbeFailure::NoData.detail().contains("reliability"));
    }

    #[test]
    fn temperature_matching_is_forgiving() {
        let mut temperatures = HashMap::new();
        temperatures.insert(r"\\.\physicaldrive1".to_string(), 41.0);
        assert_eq!(
            temperature_for(&temperatures, "physicaldrive1", "C:\\"),
            Some(41.0)
        );
        assert_eq!(
            temperature_for(&temperatures, "Samsung SSD 990 PRO", "C:\\"),
            None
        );
        assert_eq!(temperature_for(&temperatures, "", ""), None);
    }

    #[test]
    fn variant_conversion_handles_the_common_types() {
        assert_eq!(variant_to_f64(&wmi::Variant::UI4(42)), Some(42.0));
        assert_eq!(variant_to_f64(&wmi::Variant::R8(41.5)), Some(41.5));
        assert_eq!(
            variant_to_f64(&wmi::Variant::String("39".into())),
            Some(39.0)
        );
        assert_eq!(variant_to_f64(&wmi::Variant::Null), None);
        assert_eq!(
            variant_to_string(&wmi::Variant::String("disk".into())),
            Some("disk".to_string())
        );
    }
}
