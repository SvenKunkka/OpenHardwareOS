//! Ready made rules: the shipped example and the suggestions the UI offers.
//!
//! These are the cheapest possible on-ramp: a first-run user gets a working
//! `temperature -> fan` rule without designing a curve. Suggestions are built
//! from the *live* capability index, so they work on mock hardware, on an RTX
//! card and on a future OpenFan alike.

use ohm_core::{CapabilityId, DeviceId, RuleId};
use ohm_device_model::DeviceType;
use ohm_runtime::Runtime;

use crate::curve::Curve;
use crate::rule::{Aggregate, Rule, SensorRef, Source, Target};

/// Id of the shipped GPU cooling rule.
pub const GPU_COOLING_ID: &str = "gpu-cooling";
/// Id of the CPU cooling rule.
pub const CPU_COOLING_ID: &str = "cpu-cooling";
/// Id of the combined CPU + GPU rule.
pub const COMBINED_COOLING_ID: &str = "system-cooling";

/// The curve used by every shipped example.
pub fn default_curve() -> Curve {
    crate::curve::gpu_cooling_curve()
}

/// The documented example, pointed at mock hardware.
pub fn gpu_cooling_example() -> Rule {
    Rule::new(
        GPU_COOLING_ID,
        "GPU Cooling",
        Source::sensor("gpu.mock.0", "temperature.core"),
        Target::new("fan.mock.0", "fan.speed_percent"),
        default_curve(),
    )
    .expect("the example rule is valid")
    .with_description("Spins the chassis fans up as the GPU heats up.")
}

/// A CPU variant of the example.
pub fn cpu_cooling_example() -> Rule {
    Rule::new(
        CPU_COOLING_ID,
        "CPU Cooling",
        Source::sensor("cpu.mock.0", "temperature.core"),
        Target::new("fan.mock.1", "fan.speed_percent"),
        Curve::expect([(45.0, 25.0), (60.0, 40.0), (75.0, 65.0), (85.0, 100.0)]),
    )
    .expect("the CPU example rule is valid")
    .with_description("Reacts to the CPU package temperature.")
}

/// `MAX(CPU, GPU) -> fans`, the rule most people actually want.
pub fn combined_cooling_example() -> Rule {
    Rule::new(
        COMBINED_COOLING_ID,
        "System Cooling (CPU + GPU)",
        Source::combined(
            Aggregate::Max,
            vec![
                SensorRef::new("cpu.mock.0", "temperature.core"),
                SensorRef::new("gpu.mock.0", "temperature.core"),
            ],
        ),
        Target::new("fan.mock.0", "fan.speed_percent"),
        default_curve(),
    )
    .expect("the combined example rule is valid")
    .with_description("Drives the fans from whichever of the CPU or GPU is hotter.")
}

/// Build rules for the hardware that is actually attached.
///
/// Returns an empty list when there is nothing to drive (read-only machines),
/// which the UI reports as "no controllable cooling found" rather than as an
/// error.
pub fn suggest_for(runtime: &Runtime) -> Vec<Rule> {
    let index = runtime.capability_index();
    if index.targets.is_empty() || index.sources.is_empty() {
        return Vec::new();
    }

    let temperature_sources: Vec<&ohm_runtime::CapabilityRef> = index
        .sources
        .iter()
        .filter(|s| s.capability.unit.is_temperature())
        .collect();
    if temperature_sources.is_empty() {
        return Vec::new();
    }

    let target_of = |position: usize| -> Target {
        let candidate = &index.targets[position.min(index.targets.len() - 1)];
        Target::new(
            candidate.device_id.as_str(),
            candidate.capability.id.as_str(),
        )
    };

    let mut rules = Vec::new();

    let gpu = temperature_sources
        .iter()
        .find(|s| s.device_type == DeviceType::Gpu);
    if let Some(gpu) = gpu {
        rules.push(
            Rule::new(
                GPU_COOLING_ID,
                "GPU Cooling",
                Source::sensor(gpu.device_id.as_str(), gpu.capability.id.as_str()),
                target_of(0),
                default_curve(),
            )
            .expect("valid suggestion")
            .with_description(format!(
                "Drives {} from the GPU temperature.",
                index.targets[0].label()
            )),
        );
    }

    let cpu = temperature_sources
        .iter()
        .find(|s| s.device_type == DeviceType::Cpu);
    if let Some(cpu) = cpu {
        rules.push(
            Rule::new(
                CPU_COOLING_ID,
                "CPU Cooling",
                Source::sensor(cpu.device_id.as_str(), cpu.capability.id.as_str()),
                target_of(usize::from(index.targets.len() > 1)),
                Curve::expect([(45.0, 25.0), (60.0, 40.0), (75.0, 65.0), (85.0, 100.0)]),
            )
            .expect("valid suggestion")
            .with_description("Reacts to the CPU package temperature."),
        );
    }

    // The combined rule only makes sense with at least two sources, and it must
    // not silently duplicate the GPU rule's intent.
    if temperature_sources.len() >= 2 && cpu.is_some() && gpu.is_some() {
        let sensors: Vec<SensorRef> = temperature_sources
            .iter()
            .take(4)
            .map(|s| SensorRef::new(s.device_id.as_str(), s.capability.id.as_str()))
            .collect();
        rules.push(
            Rule::new(
                COMBINED_COOLING_ID,
                "System Cooling (MAX of all sensors)",
                Source::combined(Aggregate::Max, sensors),
                target_of(0),
                default_curve(),
            )
            .expect("valid suggestion")
            .with_description("Reacts to whichever sensor is hottest."),
        );
    }

    rules
}

/// A rule that maps one temperature source to one duty actuator, used by the
/// Automation form's "quick create".
pub fn simple_temperature_rule(
    name: &str,
    device: &str,
    capability: &str,
    target_device: &str,
    target_capability: &str,
) -> Rule {
    let id = RuleId::new_unchecked(crate::rule::slugify(name));
    Rule {
        id,
        name: name.to_string(),
        enabled: true,
        description: None,
        source: Source::sensor(device, capability),
        when: None,
        target: Target::new(target_device, target_capability),
        curve: default_curve(),
        hysteresis: crate::rule::DEFAULT_HYSTERESIS,
        deadband: crate::rule::DEFAULT_DEADBAND,
        update_interval_ms: crate::rule::DEFAULT_UPDATE_INTERVAL_MS,
        min_output: None,
        max_output: None,
        fallback: crate::rule::Fallback::default(),
        priority: 0,
        created_at_ms: Some(ohm_core::now_ms()),
        updated_at_ms: None,
    }
}

/// Parse a `device/capability` string, e.g. `gpu.mock.0/temperature.core`.
pub fn parse_reference(value: &str) -> Option<(DeviceId, CapabilityId)> {
    let (device, capability) = value.split_once('/')?;
    Some((
        DeviceId::new(device.trim()).ok()?,
        CapabilityId::new(capability.trim()).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_are_valid() {
        for rule in [
            gpu_cooling_example(),
            cpu_cooling_example(),
            combined_cooling_example(),
        ] {
            rule.validate().unwrap();
            assert!(rule.enabled);
            assert!(rule.description.is_some());
        }
    }

    #[test]
    fn combined_example_uses_max() {
        let rule = combined_cooling_example();
        assert_eq!(rule.source.aggregate(), Some(Aggregate::Max));
        assert_eq!(rule.source.sensors().len(), 2);
        assert_eq!(rule.id.as_str(), COMBINED_COOLING_ID);
    }

    #[test]
    fn references_parse() {
        let (device, capability) = parse_reference("gpu.mock.0/temperature.core").unwrap();
        assert_eq!(device.as_str(), "gpu.mock.0");
        assert_eq!(capability.as_str(), "temperature.core");
        assert!(parse_reference("no-slash").is_none());
        assert!(parse_reference("GPU 0/temperature.core").is_none());
    }

    #[test]
    fn simple_rule_is_valid() {
        let rule = simple_temperature_rule(
            "My Fan Rule",
            "gpu.mock.0",
            "temperature.core",
            "fan.mock.0",
            "fan.speed_percent",
        );
        rule.validate().unwrap();
        assert_eq!(rule.id.as_str(), "my-fan-rule");
        assert_eq!(rule.curve, default_curve());
    }
}
