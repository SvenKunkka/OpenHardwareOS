//! The automation rule model.
//!
//! A rule is data, not code: `source -> curve -> target`, plus the safety knobs
//! that keep it predictable. It is stored as YAML so a user can read, diff and
//! share it, and so a future AI layer can *propose* rules that are validated
//! and then executed by this same engine — never by the AI directly.
//!
//! ```yaml
//! name: GPU Cooling
//! source:
//!   device: gpu.mock.0
//!   capability: temperature.core
//! target:
//!   device: fan.mock.0
//!   capability: fan.speed_percent
//! curve:
//!   - [40, 20]
//!   - [60, 35]
//!   - [70, 50]
//!   - [80, 80]
//!   - [85, 100]
//! hysteresis: 2
//! update_interval_ms: 1000
//! ```

use ohm_core::{CapabilityId, DeviceId, OhmError, Result, RuleId};
use serde::{Deserialize, Serialize};

use crate::curve::Curve;

/// Default re-evaluation period.
pub const DEFAULT_UPDATE_INTERVAL_MS: u64 = 1_000;
/// Lowest period we accept, to protect the hardware and the CPU.
pub const MIN_UPDATE_INTERVAL_MS: u64 = 100;
/// Default hysteresis in source units (°C).
pub const DEFAULT_HYSTERESIS: f64 = 2.0;
/// Default output deadband in output units (%).
pub const DEFAULT_DEADBAND: f64 = 0.5;

/// A reference to one capability of one device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorRef {
    pub device: DeviceId,
    pub capability: CapabilityId,
}

impl SensorRef {
    pub fn new(device: &str, capability: &str) -> Self {
        Self {
            device: DeviceId::new_unchecked(device),
            capability: CapabilityId::new_unchecked(capability),
        }
    }

    /// `gpu.mock.0/temperature.core`
    pub fn qualified_id(&self) -> String {
        format!("{}/{}", self.device, self.capability)
    }
}

impl std::fmt::Display for SensorRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.qualified_id())
    }
}

/// How several sensors are reduced to one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Aggregate {
    /// The hottest sensor wins. The safe default for cooling.
    #[default]
    Max,
    /// The coolest sensor wins.
    Min,
    /// Arithmetic mean.
    Avg,
}

impl Aggregate {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Max => "max",
            Self::Min => "min",
            Self::Avg => "avg",
        }
    }

    /// Reduce the available readings. Missing sensors are ignored; an empty
    /// input means "no data".
    pub fn reduce(&self, values: &[f64]) -> Option<f64> {
        let values: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
        if values.is_empty() {
            return None;
        }
        Some(match self {
            Self::Max => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            Self::Min => values.iter().copied().fold(f64::INFINITY, f64::min),
            Self::Avg => values.iter().sum::<f64>() / values.len() as f64,
        })
    }
}

/// Where a rule reads from.
///
/// Either one sensor, or several combined with [`Aggregate`]:
///
/// ```yaml
/// source: {device: gpu.mock.0, capability: temperature.core}
/// # or
/// source:
///   aggregate: max
///   sensors:
///     - {device: cpu.mock.0, capability: temperature.core}
///     - {device: gpu.mock.0, capability: temperature.core}
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Source {
    /// A single sensor.
    Single {
        device: DeviceId,
        capability: CapabilityId,
    },
    /// Several sensors reduced by `aggregate`.
    Combined {
        #[serde(default)]
        aggregate: Aggregate,
        sensors: Vec<SensorRef>,
    },
}

impl Source {
    /// One sensor.
    pub fn sensor(device: &str, capability: &str) -> Self {
        Self::Single {
            device: DeviceId::new_unchecked(device),
            capability: CapabilityId::new_unchecked(capability),
        }
    }

    /// `MAX(CPU, GPU)` style combination.
    pub fn combined(aggregate: Aggregate, sensors: Vec<SensorRef>) -> Self {
        Self::Combined { aggregate, sensors }
    }

    /// Every sensor this source reads.
    pub fn sensors(&self) -> Vec<SensorRef> {
        match self {
            Self::Single { device, capability } => vec![SensorRef {
                device: device.clone(),
                capability: capability.clone(),
            }],
            Self::Combined { sensors, .. } => sensors.clone(),
        }
    }

    pub fn aggregate(&self) -> Option<Aggregate> {
        match self {
            Self::Single { .. } => None,
            Self::Combined { aggregate, .. } => Some(*aggregate),
        }
    }

    /// Human readable label, e.g. `MAX(cpu.mock.0/temperature.core, gpu...)`.
    pub fn label(&self) -> String {
        match self {
            Self::Single { device, capability } => format!("{device}/{capability}"),
            Self::Combined { aggregate, sensors } => {
                let inner = sensors
                    .iter()
                    .map(SensorRef::qualified_id)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({inner})", aggregate.as_str().to_uppercase())
            }
        }
    }
}

/// Where a rule writes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub device: DeviceId,
    pub capability: CapabilityId,
}

impl Target {
    pub fn new(device: &str, capability: &str) -> Self {
        Self {
            device: DeviceId::new_unchecked(device),
            capability: CapabilityId::new_unchecked(capability),
        }
    }

    pub fn qualified_id(&self) -> String {
        format!("{}/{}", self.device, self.capability)
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.qualified_id())
    }
}

/// What to do when the rule cannot compute a value any more.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FallbackAction {
    /// Keep the last output (the fan stays where it was).
    Hold,
    /// Drive the output to the runtime's fail-safe duty (safety policy,
    /// default 70 %). This is the default: losing a sensor must never leave a
    /// fan slow.
    #[default]
    SafeDefault,
    /// Drive the output to an explicit percentage.
    Fixed { percent: f64 },
    /// Stop controlling and say so, leaving the firmware in charge.
    Release,
}

impl FallbackAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::SafeDefault => "safe_default",
            Self::Fixed { .. } => "fixed",
            Self::Release => "release",
        }
    }

    /// The duty this action wants, if it wants one.
    pub fn duty(&self, safe_default: f64) -> Option<f64> {
        match self {
            Self::Hold | Self::Release => None,
            Self::SafeDefault => Some(safe_default),
            Self::Fixed { percent } => Some(*percent),
        }
    }
}

/// Failure handling for a rule.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fallback {
    /// Sensor disappeared.
    pub on_sensor_missing: FallbackAction,
    /// The write was refused by the hardware.
    pub on_write_failure: FallbackAction,
    /// How long the source may be silent before the rule gives up, in seconds.
    /// `0` means "react to the first missing reading".
    pub sensor_timeout_s: u64,
}

impl Default for Fallback {
    fn default() -> Self {
        Self {
            on_sensor_missing: FallbackAction::SafeDefault,
            on_write_failure: FallbackAction::SafeDefault,
            sensor_timeout_s: 0,
        }
    }
}

/// How two numbers are compared in a [`Condition`].
///
/// Only the six comparisons a person would expect. `eq`/`ne` are implemented with
/// a small tolerance because sensor readings are floating point; `check_rule`
/// warns when they are used on an analog reading, where `gte`/`lte` say what the
/// author actually means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparator {
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Gte,
    /// Less than.
    Lt,
    /// Less than or equal.
    Lte,
    /// Equal, within [`Comparator::EQUALITY_TOLERANCE`].
    Eq,
    /// Not equal, beyond [`Comparator::EQUALITY_TOLERANCE`].
    Ne,
}

impl Comparator {
    /// Absolute tolerance used by `eq` / `ne`.
    ///
    /// Sensor readings are quantised well above this (the state store's change
    /// epsilon starts at 0.01), so the tolerance only absorbs float noise.
    pub const EQUALITY_TOLERANCE: f64 = 1e-6;

    pub const ALL: [Comparator; 6] = [
        Comparator::Gt,
        Comparator::Gte,
        Comparator::Lt,
        Comparator::Lte,
        Comparator::Eq,
        Comparator::Ne,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Gt => "gt",
            Self::Gte => "gte",
            Self::Lt => "lt",
            Self::Lte => "lte",
            Self::Eq => "eq",
            Self::Ne => "ne",
        }
    }

    /// The operator as a symbol, for the UI and for messages.
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Gt => ">",
            Self::Gte => "≥",
            Self::Lt => "<",
            Self::Lte => "≤",
            Self::Eq => "=",
            Self::Ne => "≠",
        }
    }

    /// `true` when `reading` satisfies the comparison against `threshold`.
    ///
    /// A non-finite reading never satisfies anything: a broken sensor must not
    /// accidentally look like "load above 60 %".
    pub fn satisfied_by(&self, reading: f64, threshold: f64) -> bool {
        if !reading.is_finite() || !threshold.is_finite() {
            return false;
        }
        match self {
            Self::Gt => reading > threshold,
            Self::Gte => reading >= threshold,
            Self::Lt => reading < threshold,
            Self::Lte => reading <= threshold,
            Self::Eq => (reading - threshold).abs() <= Self::EQUALITY_TOLERANCE,
            Self::Ne => (reading - threshold).abs() > Self::EQUALITY_TOLERANCE,
        }
    }

    /// `true` when the comparison is fragile on an analog reading.
    pub fn is_equality(&self) -> bool {
        matches!(self, Self::Eq | Self::Ne)
    }
}

/// What a rule does while its [`Condition`] is **false**.
///
/// Deliberately does not offer an unbounded "hold": leaving a fan at whatever
/// value it happened to have, indefinitely, because a load condition went quiet
/// is exactly the failure mode the safety layer exists to prevent. `release` is
/// also absent — no adapter in this build advertises a channel it can hand back
/// mid-run (`LibreHardwareMonitorAdapter::shutdown` releases on exit, which is a
/// different thing), so offering it here would be a promise we cannot keep.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OtherwiseAction {
    /// Drive the target to the runtime's fail-safe duty (safety policy).
    #[default]
    SafeDefault,
    /// Drive the target to an explicit percentage.
    Fixed { percent: f64 },
}

impl OtherwiseAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SafeDefault => "safe_default",
            Self::Fixed { .. } => "fixed",
        }
    }

    /// The duty this action wants, if it wants one.
    pub fn duty(&self, safe_default: f64) -> Option<f64> {
        match self {
            Self::SafeDefault => Some(safe_default),
            Self::Fixed { percent } => Some(*percent),
        }
    }
}

/// An optional numeric gate on a rule: `when {source, op, value}`.
///
/// While the condition holds, the rule behaves exactly as before. While it does
/// not, the rule stops steering the fan **and applies [`OtherwiseAction`]**, so a
/// machine that stops gaming does not keep the fan where gaming left it.
///
/// ```yaml
/// when:
///   source: {device: gpu.mock.0, capability: load.gpu}
///   op: gt
///   value: 60
///   otherwise: safe_default
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    /// The sensor the condition reads. Same shape as a rule source sensor, so it
    /// may point at any readable capability of any device.
    pub source: SensorRef,
    pub op: Comparator,
    /// Threshold, in the unit of `source.capability`.
    pub value: f64,
    /// What to do while the condition is false.
    #[serde(default)]
    pub otherwise: OtherwiseAction,
}

impl Condition {
    /// Build a condition.
    pub fn new(device: &str, capability: &str, op: Comparator, value: f64) -> Self {
        Self {
            source: SensorRef::new(device, capability),
            op,
            value,
            otherwise: OtherwiseAction::default(),
        }
    }

    #[must_use]
    pub fn with_otherwise(mut self, otherwise: OtherwiseAction) -> Self {
        self.otherwise = otherwise;
        self
    }

    /// Human readable form, e.g. `gpu.mock.0/load.gpu > 60`.
    pub fn label(&self) -> String {
        format!(
            "{} {} {}",
            self.source.qualified_id(),
            self.op.symbol(),
            crate::curve::ControlPoint::new(0.0, self.value).output
        )
    }

    /// Structural validation.
    pub fn validate(&self) -> Result<()> {
        if !self.value.is_finite() {
            return Err(OhmError::Automation(
                "a condition threshold must be a finite number".into(),
            ));
        }
        if let OtherwiseAction::Fixed { percent } = self.otherwise
            && !(0.0..=100.0).contains(&percent)
        {
            return Err(OhmError::Automation(
                "when.otherwise.fixed percentage must be between 0 and 100".into(),
            ));
        }
        Ok(())
    }
}

/// A complete automation rule.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rule {
    pub id: RuleId,
    pub name: String,
    #[serde(default = "crate::rule::default_true")]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    pub source: Source,
    /// Optional numeric gate; absent means "always active".
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub when: Option<Condition>,
    pub target: Target,
    pub curve: Curve,
    /// Asymmetry in source units: the fan only slows down once the temperature
    /// has dropped this far below the point that set the current speed.
    pub hysteresis: f64,
    /// Smallest output change worth writing, in output units.
    pub deadband: f64,
    /// Re-evaluation period.
    pub update_interval_ms: u64,
    /// Lower clamp applied after the curve.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub min_output: Option<f64>,
    /// Upper clamp applied after the curve.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_output: Option<f64>,
    pub fallback: Fallback,
    /// Higher priority rules are evaluated and written first.
    pub priority: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub updated_at_ms: Option<i64>,
}

pub(crate) fn default_true() -> bool {
    true
}

/// Deserialisation shadow: `id` is optional in YAML and derived from the name
/// (or from the file name, by the rule store) when absent.
#[derive(Deserialize)]
struct RuleDef {
    id: Option<String>,
    name: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    description: Option<String>,
    source: Source,
    #[serde(default)]
    when: Option<Condition>,
    target: Target,
    curve: Curve,
    #[serde(default = "default_hysteresis")]
    hysteresis: f64,
    #[serde(default = "default_deadband")]
    deadband: f64,
    #[serde(default = "default_interval")]
    update_interval_ms: u64,
    #[serde(default)]
    min_output: Option<f64>,
    #[serde(default)]
    max_output: Option<f64>,
    #[serde(default)]
    fallback: Fallback,
    #[serde(default)]
    priority: u32,
    #[serde(default)]
    created_at_ms: Option<i64>,
    #[serde(default)]
    updated_at_ms: Option<i64>,
}

fn default_hysteresis() -> f64 {
    DEFAULT_HYSTERESIS
}

fn default_deadband() -> f64 {
    DEFAULT_DEADBAND
}

fn default_interval() -> u64 {
    DEFAULT_UPDATE_INTERVAL_MS
}

impl<'de> Deserialize<'de> for Rule {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let def = RuleDef::deserialize(deserializer)?;
        let id = match def.id {
            Some(id) => RuleId::new(id).map_err(serde::de::Error::custom)?,
            None => RuleId::new_unchecked(slugify(&def.name)),
        };
        Ok(Self {
            id,
            name: def.name,
            enabled: def.enabled,
            description: def.description,
            source: def.source,
            when: def.when,
            target: def.target,
            curve: def.curve,
            hysteresis: def.hysteresis,
            deadband: def.deadband,
            update_interval_ms: def.update_interval_ms,
            min_output: def.min_output,
            max_output: def.max_output,
            fallback: def.fallback,
            priority: def.priority,
            created_at_ms: def.created_at_ms,
            updated_at_ms: def.updated_at_ms,
        })
    }
}

/// Turn a rule name into a valid id: `GPU Cooling` -> `gpu-cooling`.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("rule");
    }
    out
}

impl Rule {
    /// A rule with the documented defaults, no id derivation surprises.
    pub fn new(id: &str, name: &str, source: Source, target: Target, curve: Curve) -> Result<Self> {
        let rule = Self {
            id: RuleId::new(id)?,
            name: name.to_string(),
            enabled: true,
            description: None,
            source,
            when: None,
            target,
            curve,
            hysteresis: DEFAULT_HYSTERESIS,
            deadband: DEFAULT_DEADBAND,
            update_interval_ms: DEFAULT_UPDATE_INTERVAL_MS,
            min_output: None,
            max_output: None,
            fallback: Fallback::default(),
            priority: 0,
            created_at_ms: Some(ohm_core::now_ms()),
            updated_at_ms: None,
        };
        rule.validate()?;
        Ok(rule)
    }

    /// The documented GPU cooling example.
    pub fn gpu_cooling_example() -> Self {
        Self::new(
            "gpu-cooling",
            "GPU Cooling",
            Source::sensor("gpu.mock.0", "temperature.core"),
            Target::new("fan.mock.0", "fan.speed_percent"),
            crate::curve::gpu_cooling_curve(),
        )
        .expect("valid example rule")
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Gate this rule behind a numeric condition.
    #[must_use]
    pub fn with_condition(mut self, condition: Condition) -> Self {
        self.when = Some(condition);
        self
    }

    /// Is the rule gated?
    pub fn is_conditional(&self) -> bool {
        self.when.is_some()
    }

    /// The condition's sensor, when there is a condition.
    pub fn condition_source(&self) -> Option<&SensorRef> {
        self.when.as_ref().map(|condition| &condition.source)
    }

    #[must_use]
    pub fn with_hysteresis(mut self, hysteresis: f64) -> Self {
        self.hysteresis = hysteresis;
        self
    }

    #[must_use]
    pub fn with_deadband(mut self, deadband: f64) -> Self {
        self.deadband = deadband;
        self
    }

    #[must_use]
    pub fn with_update_interval_ms(mut self, ms: u64) -> Self {
        self.update_interval_ms = ms;
        self
    }

    #[must_use]
    pub fn with_output_limits(mut self, min: f64, max: f64) -> Self {
        self.min_output = Some(min);
        self.max_output = Some(max);
        self
    }

    #[must_use]
    pub fn with_fallback(mut self, fallback: Fallback) -> Self {
        self.fallback = fallback;
        self
    }

    /// Every device this rule touches (read or write).
    pub fn devices(&self) -> Vec<DeviceId> {
        let mut devices: Vec<DeviceId> = self
            .source
            .sensors()
            .into_iter()
            .map(|s| s.device)
            .collect();
        if let Some(condition) = &self.when
            && !devices.contains(&condition.source.device)
        {
            devices.push(condition.source.device.clone());
        }
        if !devices.contains(&self.target.device) {
            devices.push(self.target.device.clone());
        }
        devices
    }

    /// Output limits actually applied: rule limits intersected with the curve.
    pub fn effective_output_range(&self) -> (f64, f64) {
        let low = self.min_output.unwrap_or(f64::NEG_INFINITY);
        let high = self.max_output.unwrap_or(f64::INFINITY);
        (low, high)
    }

    /// Structural validation. Hardware existence is checked separately by the
    /// engine, which is the only layer that can see the device table.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(OhmError::Automation("a rule needs a name".into()));
        }
        self.curve.validate()?;
        if self.source.sensors().is_empty() {
            return Err(OhmError::Automation(
                "a rule needs at least one source sensor".into(),
            ));
        }
        if matches!(self.source, Source::Combined { ref sensors, .. } if sensors.is_empty()) {
            return Err(OhmError::Automation(
                "a combined source needs at least one sensor".into(),
            ));
        }
        if let Some(condition) = &self.when {
            condition.validate()?;
            if condition.source == self.target_as_sensor() {
                return Err(OhmError::Automation(
                    "a condition may not read the capability the rule writes".into(),
                ));
            }
        }
        if !self.hysteresis.is_finite() || self.hysteresis < 0.0 {
            return Err(OhmError::Automation(
                "hysteresis must be zero or a positive number".into(),
            ));
        }
        if !self.deadband.is_finite() || self.deadband < 0.0 {
            return Err(OhmError::Automation(
                "deadband must be zero or a positive number".into(),
            ));
        }
        if self.update_interval_ms < MIN_UPDATE_INTERVAL_MS {
            return Err(OhmError::Automation(format!(
                "update_interval_ms must be at least {MIN_UPDATE_INTERVAL_MS}"
            )));
        }
        match (self.min_output, self.max_output) {
            (Some(min), Some(max)) if min > max => {
                return Err(OhmError::Automation(format!(
                    "min_output ({min}) is above max_output ({max})"
                )));
            }
            (Some(min), _) if !(0.0..=100.0).contains(&min) => {
                return Err(OhmError::Automation(
                    "min_output must be between 0 and 100".into(),
                ));
            }
            (_, Some(max)) if !(0.0..=100.0).contains(&max) => {
                return Err(OhmError::Automation(
                    "max_output must be between 0 and 100".into(),
                ));
            }
            _ => {}
        }
        // Both policies are checked separately so the error names the one that
        // is wrong.
        if let FallbackAction::Fixed { percent } = self.fallback.on_sensor_missing
            && !(0.0..=100.0).contains(&percent)
        {
            return Err(OhmError::Automation(
                "fallback.on_sensor_missing percentage must be between 0 and 100".into(),
            ));
        }
        if let FallbackAction::Fixed { percent } = self.fallback.on_write_failure
            && !(0.0..=100.0).contains(&percent)
        {
            return Err(OhmError::Automation(
                "fallback.on_write_failure percentage must be between 0 and 100".into(),
            ));
        }
        Ok(())
    }

    /// The target expressed as a sensor reference, for comparison checks.
    fn target_as_sensor(&self) -> SensorRef {
        SensorRef {
            device: self.target.device.clone(),
            capability: self.target.capability.clone(),
        }
    }

    /// Clamp a curve output to the rule limits.
    pub fn clamp_output(&self, output: f64) -> f64 {
        let (low, high) = self.effective_output_range();
        output.clamp(low, high)
    }

    /// One line summary for logs and the UI.
    pub fn summary(&self) -> String {
        let gate = match &self.when {
            Some(condition) => format!(" when {}", condition.label()),
            None => String::new(),
        };
        format!(
            "{}: {} -> {} ({} points, hysteresis {:.1}, every {} ms){}",
            self.name,
            self.source.label(),
            self.target.qualified_id(),
            self.curve.len(),
            self.hysteresis,
            self.update_interval_ms,
            gate
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_yaml_parses() {
        let yaml = r#"
name: GPU Cooling
source:
  device: gpu.nvidia.0
  capability: temperature.core
target:
  device: fan.system.0
  capability: fan.speed_percent
curve:
  - [40, 20]
  - [60, 35]
  - [70, 50]
  - [80, 80]
  - [85, 100]
hysteresis: 2
update_interval_ms: 1000
"#;
        let rule: Rule = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(rule.name, "GPU Cooling");
        assert_eq!(rule.id.as_str(), "gpu-cooling");
        assert_eq!(rule.source.label(), "gpu.nvidia.0/temperature.core");
        assert_eq!(rule.target.qualified_id(), "fan.system.0/fan.speed_percent");
        assert_eq!(rule.curve.eval(70.0), 50.0);
        assert_eq!(rule.hysteresis, 2.0);
        assert_eq!(rule.update_interval_ms, 1_000);
        assert!(rule.enabled);
        assert_eq!(rule.deadband, DEFAULT_DEADBAND);
        assert_eq!(rule.fallback.on_sensor_missing, FallbackAction::SafeDefault);
        rule.validate().unwrap();
    }

    #[test]
    fn roundtrip_keeps_every_field() {
        let rule = Rule::gpu_cooling_example()
            .with_description("keeps the GPU under control")
            .with_output_limits(20.0, 100.0)
            .with_fallback(Fallback {
                on_sensor_missing: FallbackAction::Fixed { percent: 80.0 },
                on_write_failure: FallbackAction::Release,
                sensor_timeout_s: 5,
            });
        let yaml = serde_yaml_ng::to_string(&rule).unwrap();
        let back: Rule = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn combined_source_parses_and_labels() {
        let yaml = r#"
name: CPU and GPU
source:
  aggregate: max
  sensors:
    - {device: cpu.mock.0, capability: temperature.core}
    - {device: gpu.mock.0, capability: temperature.core}
target: {device: fan.mock.0, capability: fan.speed_percent}
curve: [[40, 20], [85, 100]]
"#;
        let rule: Rule = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(rule.source.aggregate(), Some(Aggregate::Max));
        assert_eq!(rule.source.sensors().len(), 2);
        assert_eq!(
            rule.source.label(),
            "MAX(cpu.mock.0/temperature.core, gpu.mock.0/temperature.core)"
        );
        assert_eq!(rule.devices().len(), 3);
    }

    #[test]
    fn aggregate_reduction() {
        assert_eq!(Aggregate::Max.reduce(&[60.0, 75.0, 55.0]), Some(75.0));
        assert_eq!(Aggregate::Min.reduce(&[60.0, 75.0, 55.0]), Some(55.0));
        assert_eq!(Aggregate::Avg.reduce(&[60.0, 80.0]), Some(70.0));
        assert_eq!(Aggregate::Max.reduce(&[]), None);
        assert_eq!(Aggregate::Max.reduce(&[f64::NAN]), None);
        assert_eq!(Aggregate::Max.reduce(&[f64::NAN, 42.0]), Some(42.0));
        assert_eq!(Aggregate::default(), Aggregate::Max);
    }

    #[test]
    fn slugify_is_stable() {
        assert_eq!(slugify("GPU Cooling"), "gpu-cooling");
        assert_eq!(slugify("  CPU + GPU  MAX!! "), "cpu-gpu-max");
        assert_eq!(slugify("..."), "rule");
        assert_eq!(slugify("Already-Slugged"), "already-slugged");
    }

    #[test]
    fn structural_validation_catches_mistakes() {
        let mut rule = Rule::gpu_cooling_example();
        rule.update_interval_ms = 10;
        assert!(rule.validate().is_err());
        rule.update_interval_ms = 1_000;

        rule.hysteresis = -1.0;
        assert!(rule.validate().is_err());
        rule.hysteresis = 2.0;

        rule.min_output = Some(90.0);
        rule.max_output = Some(20.0);
        assert!(rule.validate().is_err());
        rule.min_output = None;
        rule.max_output = Some(150.0);
        assert!(rule.validate().is_err());
        rule.max_output = Some(90.0);

        rule.fallback.on_sensor_missing = FallbackAction::Fixed { percent: 150.0 };
        assert!(rule.validate().is_err());
        rule.fallback.on_sensor_missing = FallbackAction::SafeDefault;

        let mut named = rule.clone();
        named.name = "   ".into();
        assert!(named.validate().is_err());

        rule.validate().unwrap();
    }

    #[test]
    fn output_clamping_uses_rule_limits() {
        let rule = Rule::gpu_cooling_example().with_output_limits(30.0, 90.0);
        assert_eq!(rule.clamp_output(10.0), 30.0);
        assert_eq!(rule.clamp_output(50.0), 50.0);
        assert_eq!(rule.clamp_output(100.0), 90.0);
        assert_eq!(rule.effective_output_range(), (30.0, 90.0));

        let unbounded = Rule::gpu_cooling_example();
        assert_eq!(unbounded.clamp_output(-5.0), -5.0);
    }

    #[test]
    fn summary_is_readable() {
        let summary = Rule::gpu_cooling_example().summary();
        assert!(summary.contains("GPU Cooling"));
        assert!(summary.contains("gpu.mock.0/temperature.core"));
        assert!(summary.contains("fan.mock.0/fan.speed_percent"));
        assert!(summary.contains("5 points"));
    }

    #[test]
    fn fallback_actions_expose_their_duty() {
        assert_eq!(FallbackAction::Hold.duty(70.0), None);
        assert_eq!(FallbackAction::Release.duty(70.0), None);
        assert_eq!(FallbackAction::SafeDefault.duty(70.0), Some(70.0));
        assert_eq!(
            FallbackAction::Fixed { percent: 42.0 }.duty(70.0),
            Some(42.0)
        );
        assert_eq!(FallbackAction::default(), FallbackAction::SafeDefault);
        assert_eq!(FallbackAction::Hold.as_str(), "hold");
    }

    #[test]
    fn missing_required_fields_are_reported() {
        let err = serde_yaml_ng::from_str::<Rule>("name: Broken\ncurve: [[1,2],[3,4]]")
            .unwrap_err()
            .to_string();
        assert!(err.contains("source"), "unexpected error: {err}");
    }
}
