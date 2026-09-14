//! Device descriptions for OS level sensors.

use ohm_core::DeviceId;
use ohm_device_model::DeviceType;

/// `cpu.system.0`
pub fn cpu_device_id() -> DeviceId {
    DeviceId::compose(DeviceType::Cpu.as_str(), super::ADAPTER_ID, 0)
}

/// `temperature.system.{index}` — one per OS thermal zone.
pub fn component_device_id(index: usize) -> DeviceId {
    DeviceId::compose("temperature", super::ADAPTER_ID, index)
}

/// `memory.system.0` — the machine's RAM.
///
/// One device, not one per module: the operating system reports totals, and
/// inventing per-module devices from a total would be a guess.
pub fn memory_device_id() -> DeviceId {
    DeviceId::new_unchecked("memory.system.0")
}

/// `storage.system.{index}` — one per non-removable volume.
pub fn disk_device_id(index: usize) -> DeviceId {
    DeviceId::compose("storage", super::ADAPTER_ID, index)
}

/// Best effort vendor extraction from a CPU brand string.
///
/// `sysinfo` gives us a marketing string such as `AMD Ryzen 9 7950X 16-Core
/// Processor`, with no vendor field, so the vendor has to be inferred.
pub fn vendor_from_brand(brand: &str) -> String {
    let lower = brand.to_ascii_lowercase();
    for (needle, vendor) in [
        ("amd", "AMD"),
        ("ryzen", "AMD"),
        ("athlon", "AMD"),
        ("epyc", "AMD"),
        ("intel", "Intel"),
        ("core", "Intel"),
        ("xeon", "Intel"),
        ("pentium", "Intel"),
        ("celeron", "Intel"),
        ("apple", "Apple"),
        ("qualcomm", "Qualcomm"),
        ("snapdragon", "Qualcomm"),
        ("arm", "Arm"),
    ] {
        if lower.contains(needle) {
            return vendor.to_string();
        }
    }
    if brand.trim().is_empty() {
        String::new()
    } else {
        "Unknown".to_string()
    }
}

/// How strongly a thermal zone label looks like a CPU package sensor.
///
/// Higher is better; `0` means "not a CPU sensor". The labels come straight
/// from the platform (`k10temp`, `coretemp`, `Tctl`, `CPU`, ...), and matching
/// them is the only way to pick the right zone without vendor SDKs.
pub fn cpu_component_score(label: &str) -> u8 {
    let lower = label.to_ascii_lowercase();

    // Exclusions come first: "GPU Core" must never win over "CPU Package".
    for needle in ["gpu", "nvme", "ssd", "drive", "battery", "ambient"] {
        if lower.contains(needle) {
            return 0;
        }
    }

    for (needle, score) in [
        ("package", 100),
        ("tctl", 95),
        ("tdie", 95),
        ("cpu", 90),
        ("coretemp", 90),
        ("k10temp", 90),
        ("zenpower", 90),
        ("core temperature", 85),
        ("processor", 70),
        ("core", 70),
        ("soc", 40),
        ("apu", 40),
        ("acpi", 20),
    ] {
        if lower.contains(needle) {
            return score;
        }
    }
    0
}

/// `true` when a thermal zone label should be skipped entirely.
///
/// Calibration and "device" zones carry nothing a user can act on, and on Apple
/// Silicon they outnumber the meaningful ones several times over.
pub fn is_derived_zone(label: &str) -> bool {
    let lower = label.to_ascii_lowercase();
    lower.contains("battery")
        || lower.contains("ac adapter")
        || lower.contains("tcal")
        || lower.contains("tdev")
}

/// How many thermal zone devices a machine may expose.
pub const MAX_THERMAL_ZONE_DEVICES: usize = 6;

/// Pick the thermal zones worth showing, in their original order.
///
/// The rule is simple: zones that name something a person recognises (CPU, SSD,
/// GPU, enclosure, ...) win, then the rest, and calibration zones never appear.
/// At most [`MAX_THERMAL_ZONE_DEVICES`] are kept, so a machine that reports 40
/// internal PMU probes still yields a readable device list.
pub fn select_thermal_zones(
    components: &[sysinfo::Component],
) -> Vec<(usize, &sysinfo::Component)> {
    const USEFUL: [&str; 12] = [
        "cpu",
        "package",
        "soc",
        "gpu",
        "ssd",
        "nvme",
        "nand",
        "battery",
        "ambient",
        "enclosure",
        "board",
        "system",
    ];

    let mut candidates: Vec<(u8, usize, &sysinfo::Component)> = components
        .iter()
        .enumerate()
        .filter(|(_, component)| {
            component.temperature().is_some() && !is_derived_zone(component.label())
        })
        .map(|(index, component)| {
            let lower = component.label().to_ascii_lowercase();
            let useful = USEFUL.iter().any(|needle| lower.contains(needle));
            let score = if useful {
                200u8.saturating_add(cpu_component_score(component.label()))
            } else {
                100
            };
            (score, index, component)
        })
        .collect();

    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    candidates.truncate(MAX_THERMAL_ZONE_DEVICES);
    candidates.sort_by_key(|(_, index, _)| *index);
    candidates
        .into_iter()
        .map(|(_, index, component)| (index, component))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable() {
        assert_eq!(cpu_device_id().as_str(), "cpu.system.0");
        assert_eq!(component_device_id(2).as_str(), "temperature.system.2");
        assert_eq!(disk_device_id(1).as_str(), "storage.system.1");
    }

    #[test]
    fn vendor_detection() {
        assert_eq!(vendor_from_brand("AMD Ryzen 9 9800X3D"), "AMD");
        assert_eq!(vendor_from_brand("Intel(R) Core(TM) i9-14900K"), "Intel");
        assert_eq!(vendor_from_brand("Apple M4 Pro"), "Apple");
        assert_eq!(vendor_from_brand(""), "");
        assert_eq!(vendor_from_brand("Totally Unknown Chip"), "Unknown");
    }

    #[test]
    fn cpu_zone_scoring_prefers_the_package() {
        assert!(cpu_component_score("Tctl") > cpu_component_score("Core 0"));
        assert_eq!(cpu_component_score("coretemp"), 90);
        assert_eq!(cpu_component_score("Package id 0"), 100);
        assert_eq!(cpu_component_score("NVMe Temperature"), 0);
        assert_eq!(cpu_component_score("GPU Core"), 0);
        assert_eq!(cpu_component_score("Battery"), 0);
    }

    #[test]
    fn derived_zones_are_identified() {
        assert!(is_derived_zone("Battery Thermal Zone"));
        assert!(is_derived_zone("PMU tcal"));
        assert!(is_derived_zone("PMU2 tdev3"));
        assert!(!is_derived_zone("CPU Package"));
        assert!(!is_derived_zone("NAND CH0 temp"));
    }

    #[test]
    fn zone_selection_keeps_the_useful_ones_and_caps_the_rest() {
        // A machine like an Apple Silicon laptop: many anonymous PMU probes and
        // two zones a human recognises.
        let components = sysinfo::Components::new_with_refreshed_list();
        let labels: Vec<String> = components
            .list()
            .iter()
            .map(|component| component.label().to_string())
            .collect();
        let selected = select_thermal_zones(components.list());
        assert!(selected.len() <= MAX_THERMAL_ZONE_DEVICES);
        for (index, component) in &selected {
            assert!(!is_derived_zone(component.label()));
            assert!(component.temperature().is_some());
            assert!(labels.get(*index).is_some());
        }
        // Selection keeps the original ordering so device ids stay stable.
        assert!(selected.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }
}
