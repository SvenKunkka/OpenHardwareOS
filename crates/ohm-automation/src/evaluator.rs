//! Rule evaluation: pure logic, no hardware, no I/O.
//!
//! Everything interesting about a cooling rule happens here, which is why this
//! module is exhaustively tested:
//!
//! ```text
//! source value ─▶ curve ─▶ clamp ─▶ hysteresis ─▶ deadband ─▶ write?
//!      │
//!      └─ missing ─▶ fallback (hold / fail-safe / fixed / release)
//! ```
//!
//! [`evaluate`] takes a rule, its mutable runtime state and the current sensor
//! value, and returns a decision. The engine then performs the write and feeds
//! the result back in.

use ohm_core::RuleId;
use serde::{Deserialize, Serialize};

use crate::rule::{Condition, FallbackAction, Rule};

/// What a rule is doing right now, as shown in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleStatus {
    /// Switched off by the user.
    Disabled,
    /// Never evaluated yet.
    Idle,
    /// Computed a value and (possibly) wrote it.
    Applied,
    /// Computed the same value as before: nothing to write.
    Held,
    /// The source is gone and the fallback is in charge.
    Fallback,
    /// The rule's `when` condition is false: it is standing down, and the
    /// configured `otherwise` duty is in force.
    Gated,
    /// The device accepted the write but the resulting value is unknown. The rule
    /// holds no confirmed output, and will retry on its next evaluation.
    Unconfirmed,
    /// Control was released back to the firmware.
    Released,
    /// The last write failed.
    Error,
}

impl RuleStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Idle => "idle",
            Self::Applied => "applied",
            Self::Held => "held",
            Self::Fallback => "fallback",
            Self::Gated => "gated",
            Self::Unconfirmed => "unconfirmed",
            Self::Released => "released",
            Self::Error => "error",
        }
    }

    /// `true` when the rule is actively steering its output from the curve.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Applied | Self::Held)
    }

    /// `true` when the rule has stood down on purpose (condition false). The
    /// output is still safe, it is just not being driven by the curve.
    pub fn is_standing_down(&self) -> bool {
        matches!(self, Self::Gated | Self::Fallback | Self::Released)
    }

    /// `true` when the rule cannot vouch for its output: the device took the
    /// request but never confirmed the value.
    pub fn is_unconfirmed(&self) -> bool {
        matches!(self, Self::Unconfirmed)
    }
}

/// Per-rule memory between evaluations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleState {
    pub rule_id: RuleId,
    /// Last value that was successfully written.
    pub applied_output: Option<f64>,
    /// Source value at the moment the current output was set. Hysteresis is
    /// measured against this.
    pub armed_input: f64,
    /// Last observed source value.
    pub last_input: Option<f64>,
    pub last_input_ms: i64,
    /// Last observed value of the `when` condition's sensor.
    pub last_condition: Option<f64>,
    pub last_condition_ms: i64,
    /// Did the `when` condition hold on the last evaluation?
    pub gate_open: bool,
    /// Has the "standing down" notice already been published for this stretch?
    /// Keeps the event stream quiet while a rule sits behind a closed gate.
    pub gate_announced: bool,
    pub last_write_ms: i64,
    /// When the rule wants to be evaluated again.
    pub next_due_ms: i64,
    pub last_status: RuleStatus,
    pub last_message: String,
    pub evaluations: u64,
    pub writes: u64,
    pub skipped: u64,
    pub fallbacks: u64,
    /// `true` while the rule has deliberately given up control.
    pub released: bool,
    /// Writes the device accepted but never confirmed, since the rule started.
    pub unconfirmed_writes: u64,
    /// Unconfirmed writes in a row. Reset by any confirmed write, and by the
    /// failure policy so the fallback is re-attempted on a bounded cadence.
    pub consecutive_unconfirmed: u32,
}

impl RuleState {
    pub fn new(rule_id: RuleId) -> Self {
        Self {
            rule_id,
            applied_output: None,
            armed_input: f64::NAN,
            last_input: None,
            last_input_ms: 0,
            last_condition: None,
            last_condition_ms: 0,
            gate_open: false,
            gate_announced: false,
            last_write_ms: 0,
            next_due_ms: 0,
            last_status: RuleStatus::Idle,
            last_message: "not evaluated yet".into(),
            evaluations: 0,
            writes: 0,
            skipped: 0,
            fallbacks: 0,
            released: false,
            unconfirmed_writes: 0,
            consecutive_unconfirmed: 0,
        }
    }

    /// Reset the control memory, e.g. after the device was disabled.
    pub fn reset(&mut self) {
        self.applied_output = None;
        self.armed_input = f64::NAN;
        self.released = false;
        self.gate_open = false;
        self.gate_announced = false;
        self.consecutive_unconfirmed = 0;
    }
}

/// Inputs to one evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationInput {
    /// Current source value, or `None` when the sensor is missing/stale.
    pub source_value: Option<f64>,
    /// Current value of the rule's `when` sensor, when the rule has one.
    ///
    /// `None` on a gated rule means "the condition source is missing or stale",
    /// which follows the same sensor-failure policy as the main source.
    pub condition_value: Option<f64>,
    pub now_ms: i64,
    /// Hardware range of the target capability.
    pub target_min: f64,
    pub target_max: f64,
    /// Duty the runtime's safety policy wants when we lose the sensor.
    pub safe_default_duty: f64,
    /// Human readable source description, used in messages.
    pub source_label: String,
}

impl EvaluationInput {
    /// Convenience constructor for tests.
    pub fn new(source_value: Option<f64>, now_ms: i64) -> Self {
        Self {
            source_value,
            condition_value: None,
            now_ms,
            target_min: 0.0,
            target_max: 100.0,
            safe_default_duty: 70.0,
            source_label: "source".into(),
        }
    }
}

/// The decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub status: RuleStatus,
    /// Value the rule wants the actuator to have.
    pub output: Option<f64>,
    /// Should the engine write `output`?
    pub should_write: bool,
    pub message: String,
    pub next_due_ms: i64,
    /// Always `false` in this build: `release` is refused by validation and
    /// sanitised on load, and a rule that still carries it is stood down at the
    /// fail-safe duty instead. The field remains because the evaluation contract
    /// needs to express it once an adapter can genuinely release a channel.
    pub release: bool,
}

impl Evaluation {
    /// `true` when nothing should be written at all.
    pub fn is_noop(&self) -> bool {
        !self.should_write
    }
}

/// Evaluate one rule.
///
/// Updates the parts of `state` that describe the *decision* (timing, last
/// input, hysteresis anchor). The engine updates the parts that describe the
/// *outcome* (`applied_output`, `writes`, ...) once the write has been tried,
/// so a failed write never poisons the control memory.
pub fn evaluate(rule: &Rule, state: &mut RuleState, input: EvaluationInput) -> Evaluation {
    state.evaluations = state.evaluations.saturating_add(1);
    let next_due_ms = input.now_ms + rule.update_interval_ms as i64;

    if !rule.enabled {
        state.next_due_ms = next_due_ms;
        state.last_status = RuleStatus::Disabled;
        state.last_message = "rule disabled".into();
        return Evaluation {
            status: RuleStatus::Disabled,
            output: None,
            should_write: false,
            message: state.last_message.clone(),
            next_due_ms,
            release: false,
        };
    }

    // The source is silent. Before giving up, a rule may keep steering on the
    // last known value for `fallback.sensor_timeout_s` seconds: a single missed
    // poll (a driver hiccup, a GPU dropping off the bus for a moment) should not
    // make the fan abandon a perfectly good curve.
    let (source_value, silence_ms) = match input.source_value.filter(|value| value.is_finite()) {
        Some(value) => {
            // A fresh reading is the only thing that may move the anchor forward.
            state.last_input = Some(value);
            state.last_input_ms = input.now_ms;
            (value, None)
        }
        None => match stale_value_within_grace(rule, state, &input) {
            // Careful: the anchor deliberately stays where it was. Refreshing it
            // here would slide the grace window forward on every evaluation and
            // the rule would never reach its fallback.
            Some(last) => (last, Some(input.now_ms.saturating_sub(state.last_input_ms))),
            None => return evaluate_fallback(rule, state, &input, next_due_ms),
        },
    };

    state.released = false;

    // 0. The `when` gate runs before the curve: a rule that should not be
    //    steering right now must not compute (or write) a curve value at all.
    if let Some(condition) = &rule.when {
        let gate = resolve_gate(rule, condition, state, &input);
        match gate {
            GateOutcome::Open => {
                state.gate_open = true;
            }
            GateOutcome::Closed { reading } => {
                let was_open = state.gate_open;
                state.gate_open = false;
                return evaluate_gated(
                    rule,
                    condition,
                    state,
                    &input,
                    reading,
                    was_open,
                    next_due_ms,
                );
            }
            GateOutcome::SourceUnavailable => {
                state.gate_open = false;
                return evaluate_fallback(rule, state, &input, next_due_ms);
            }
        }
    } else {
        state.gate_open = true;
    }

    // 1. Curve.
    let curved = rule.curve.eval(source_value);
    // 2. Rule limits then hardware limits.
    let limited = rule.clamp_output(curved);
    let clamped = limited.clamp(input.target_min, input.target_max);

    // 3. Hysteresis: rising immediately, falling only after a real drop.
    let mut target = clamped;
    let mut held_by_hysteresis = false;
    if let Some(applied) = state.applied_output {
        if clamped < applied {
            // Only hold back when we know which input produced the current
            // output. An unknown anchor (for example right after a fail-safe
            // write) must not freeze the rule.
            if state.armed_input.is_finite() && source_value > state.armed_input - rule.hysteresis {
                target = applied;
                held_by_hysteresis = true;
            } else {
                state.armed_input = source_value;
            }
        } else if clamped > applied || !state.armed_input.is_finite() {
            // Rising output, or an unknown anchor: re-anchor on this input.
            state.armed_input = source_value;
        }
    } else {
        state.armed_input = source_value;
    }

    // 4. Deadband: ignore changes too small to matter.
    let delta = state.applied_output.map(|applied| (target - applied).abs());
    let should_write = match delta {
        None => true,
        Some(delta) => delta >= rule.deadband && delta > f64::EPSILON,
    };

    let (status, message) = if should_write {
        (
            RuleStatus::Applied,
            format!("{source_value:.1} -> {target:.0} %"),
        )
    } else if held_by_hysteresis {
        (
            RuleStatus::Held,
            format!(
                "holding {} % until {} drops below {:.1}",
                state.applied_output.unwrap_or(target),
                input.source_label,
                state.armed_input - rule.hysteresis
            ),
        )
    } else {
        (
            RuleStatus::Held,
            format!(
                "no change worth writing ({:.2} % within the {:.2} % deadband)",
                delta.unwrap_or(0.0),
                rule.deadband
            ),
        )
    };

    // When the value came from the grace period, say so: a reading that is
    // being reused must never look like a fresh one.
    let message = match silence_ms {
        Some(silence) => format!(
            "{message} (source silent for {:.1} s, steering on the last known value)",
            silence as f64 / 1000.0
        ),
        None => message,
    };

    state.next_due_ms = next_due_ms;
    state.last_status = status;
    state.last_message = message.clone();
    if !should_write {
        state.skipped = state.skipped.saturating_add(1);
    }

    Evaluation {
        status,
        output: Some(target),
        should_write,
        message,
        next_due_ms,
        release: false,
    }
}

/// What the rule's `when` gate decided.
#[derive(Debug, Clone, Copy, PartialEq)]
enum GateOutcome {
    /// The condition holds: let the rule steer.
    Open,
    /// The condition does not hold, with the reading that failed it.
    Closed { reading: f64 },
    /// The condition's sensor is missing and past its grace period.
    SourceUnavailable,
}

/// Evaluate the gate, applying the same freshness rules as the main source.
///
/// A condition sensor that blips is treated exactly like the rule's own sensor:
/// inside `fallback.sensor_timeout_s` the last known value is reused, and past it
/// the rule follows its sensor-failure policy. A non-finite reading never opens
/// the gate.
fn resolve_gate(
    rule: &Rule,
    condition: &Condition,
    state: &mut RuleState,
    input: &EvaluationInput,
) -> GateOutcome {
    match input.condition_value.filter(|value| value.is_finite()) {
        Some(reading) => {
            state.last_condition = Some(reading);
            state.last_condition_ms = input.now_ms;
            if condition.op.satisfied_by(reading, condition.value) {
                GateOutcome::Open
            } else {
                GateOutcome::Closed { reading }
            }
        }
        None => {
            // Same grace period as the main source, same anchor discipline: only
            // a fresh reading moves the window forward.
            let grace_ms = i64::try_from(rule.fallback.sensor_timeout_s)
                .unwrap_or(i64::MAX)
                .saturating_mul(1_000);
            let last = state.last_condition.filter(|value| value.is_finite());
            match last {
                Some(reading)
                    if grace_ms > 0
                        && state.last_condition_ms > 0
                        && input.now_ms.saturating_sub(state.last_condition_ms) < grace_ms =>
                {
                    if condition.op.satisfied_by(reading, condition.value) {
                        GateOutcome::Open
                    } else {
                        GateOutcome::Closed { reading }
                    }
                }
                _ => GateOutcome::SourceUnavailable,
            }
        }
    }
}

/// The gate is closed: stand the rule down and apply its `otherwise` duty.
#[allow(clippy::too_many_arguments)]
fn evaluate_gated(
    rule: &Rule,
    condition: &Condition,
    state: &mut RuleState,
    input: &EvaluationInput,
    reading: f64,
    _was_open: bool,
    next_due_ms: i64,
) -> Evaluation {
    let duty = condition
        .otherwise
        .duty(input.safe_default_duty)
        .unwrap_or(input.safe_default_duty);
    let duty = duty.clamp(input.target_min, input.target_max);

    let changed = state
        .applied_output
        .map(|applied| (duty - applied).abs() >= f64::EPSILON)
        .unwrap_or(true);

    // The curve is not in charge, so the hysteresis anchor is meaningless:
    // clearing it makes the rule resume from the sensor value it sees when the
    // gate reopens, instead of from a value measured before it closed.
    state.armed_input = f64::NAN;

    let message = if changed {
        format!(
            "condition not met ({}, reading {:.1} {} {:.1}): standing down at {:.0} %",
            condition.source.qualified_id(),
            reading,
            condition.op.symbol(),
            condition.value,
            duty
        )
    } else {
        format!(
            "condition not met ({}, reading {:.1} {} {:.1}): still standing down at {:.0} %",
            condition.source.qualified_id(),
            reading,
            condition.op.symbol(),
            condition.value,
            duty
        )
    };

    state.next_due_ms = next_due_ms;
    state.last_status = RuleStatus::Gated;
    state.last_message = message.clone();
    if !changed {
        state.skipped = state.skipped.saturating_add(1);
    }

    let _ = rule;
    Evaluation {
        status: RuleStatus::Gated,
        output: Some(duty),
        should_write: changed,
        message,
        next_due_ms,
        release: false,
    }
}

/// The last known value, when the rule is still inside its grace period.
///
/// `fallback.sensor_timeout_s` is the *opt-in* version of "do not give up yet":
/// `0` (the default) keeps the strict behaviour of reacting to the first missing
/// reading, while a positive value allows a short outage to pass unnoticed.
fn stale_value_within_grace(
    rule: &Rule,
    state: &RuleState,
    input: &EvaluationInput,
) -> Option<f64> {
    let grace_ms = i64::try_from(rule.fallback.sensor_timeout_s)
        .unwrap_or(i64::MAX)
        .saturating_mul(1_000);
    if grace_ms <= 0 {
        return None;
    }
    let last = state.last_input.filter(|value| value.is_finite())?;
    if state.last_input_ms <= 0 {
        return None;
    }
    let silence_ms = input.now_ms.saturating_sub(state.last_input_ms);
    (silence_ms < grace_ms).then_some(last)
}

/// The fallback path: the source value is missing.
fn evaluate_fallback(
    rule: &Rule,
    state: &mut RuleState,
    input: &EvaluationInput,
    next_due_ms: i64,
) -> Evaluation {
    state.fallbacks = state.fallbacks.saturating_add(1);
    state.next_due_ms = next_due_ms;
    let action = rule.fallback.on_sensor_missing;

    let (status, output, should_write, release, message) = match action {
        FallbackAction::Hold => (
            RuleStatus::Fallback,
            state.applied_output,
            false,
            false,
            format!(
                "{} is missing: holding the last output{}",
                input.source_label,
                state
                    .applied_output
                    .map(|v| format!(" ({v:.0} %)"))
                    .unwrap_or_else(|| " (none yet)".into())
            ),
        ),
        FallbackAction::SafeDefault | FallbackAction::Fixed { .. } => {
            let duty = action
                .duty(input.safe_default_duty)
                .unwrap_or(input.safe_default_duty);
            let duty = duty.clamp(input.target_min, input.target_max);
            let changed = state
                .applied_output
                .map(|applied| (duty - applied).abs() >= f64::EPSILON)
                .unwrap_or(true);
            (
                RuleStatus::Fallback,
                Some(duty),
                changed,
                false,
                format!(
                    "{} is missing: falling back to {:.0} %",
                    input.source_label, duty
                ),
            )
        }
        // Defence in depth. `release` cannot be saved or loaded (see
        // `FallbackAction::Release`), but if a rule reaches evaluation carrying it
        // — a hand-built rule, a future import path — it must not silently do
        // nothing. It stands the rule down at the fail-safe duty instead, and says
        // why.
        FallbackAction::Release => {
            let duty = input
                .safe_default_duty
                .clamp(input.target_min, input.target_max);
            let changed = state
                .applied_output
                .map(|applied| (duty - applied).abs() >= f64::EPSILON)
                .unwrap_or(true);
            (
                RuleStatus::Fallback,
                Some(duty),
                changed,
                false,
                format!(
                    "{} is missing: `release` is not supported by any adapter in this build, so \
                     the {:.0} % fail-safe duty was applied instead",
                    input.source_label, duty
                ),
            )
        }
    };

    state.released = release || state.released;
    state.last_status = status;
    state.last_message = message.clone();
    if !should_write {
        state.skipped = state.skipped.saturating_add(1);
    }

    Evaluation {
        status,
        output,
        should_write,
        message,
        next_due_ms,
        release,
    }
}

/// A snapshot of a rule plus its live state, for the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleOutcome {
    pub rule_id: RuleId,
    pub name: String,
    pub enabled: bool,
    pub status: RuleStatus,
    pub source: String,
    pub target: String,
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub applied_output: Option<f64>,
    pub message: String,
    pub evaluations: u64,
    pub writes: u64,
    pub skipped: u64,
    pub fallbacks: u64,
    /// Writes the device accepted but never confirmed.
    pub unconfirmed: u64,
    pub at_ms: i64,
}

impl RuleOutcome {
    /// Build the UI projection of a rule + state.
    pub fn from_parts(rule: &Rule, state: &RuleState, at_ms: i64) -> Self {
        Self {
            rule_id: rule.id.clone(),
            name: rule.name.clone(),
            enabled: rule.enabled,
            status: if rule.enabled {
                state.last_status
            } else {
                RuleStatus::Disabled
            },
            source: rule.source.label(),
            target: rule.target.qualified_id(),
            input: state.last_input,
            output: state.applied_output,
            applied_output: state.applied_output,
            message: state.last_message.clone(),
            evaluations: state.evaluations,
            writes: state.writes,
            skipped: state.skipped,
            fallbacks: state.fallbacks,
            unconfirmed: state.unconfirmed_writes,
            at_ms,
        }
    }

    /// Short status line, e.g. `GPU Cooling: 68 ° -> 42 %`.
    pub fn summary(&self) -> String {
        match (self.input, self.applied_output) {
            (Some(input), Some(output)) => {
                format!("{}: {input:.0} -> {output:.0} %", self.name)
            }
            (Some(input), None) => format!("{}: {input:.0} (idle)", self.name),
            _ => format!("{}: {}", self.name, self.message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::{Comparator, Condition, Fallback, OtherwiseAction, Rule, Source, Target};

    fn rule() -> Rule {
        Rule::gpu_cooling_example()
            .with_hysteresis(2.0)
            .with_deadband(0.0)
    }

    fn input(value: Option<f64>, now_ms: i64) -> EvaluationInput {
        EvaluationInput {
            source_value: value,
            condition_value: None,
            now_ms,
            target_min: 0.0,
            target_max: 100.0,
            safe_default_duty: 70.0,
            source_label: "gpu.mock.0/temperature.core".into(),
        }
    }

    #[test]
    fn first_evaluation_writes_the_curve_value() {
        let rule = rule();
        let mut state = RuleState::new(rule.id.clone());
        let evaluation = evaluate(&rule, &mut state, input(Some(70.0), 1_000));
        assert_eq!(evaluation.status, RuleStatus::Applied);
        assert!(evaluation.should_write);
        assert_eq!(evaluation.output, Some(50.0));
        assert_eq!(evaluation.next_due_ms, 2_000);
        // The engine confirms the write.
        state.applied_output = evaluation.output;
        assert_eq!(state.last_status, RuleStatus::Applied);
        assert_eq!(state.evaluations, 1);
    }

    #[test]
    fn disabled_rule_never_writes() {
        let mut rule = rule();
        rule.enabled = false;
        let mut state = RuleState::new(rule.id.clone());
        let evaluation = evaluate(&rule, &mut state, input(Some(95.0), 1_000));
        assert_eq!(evaluation.status, RuleStatus::Disabled);
        assert!(!evaluation.should_write);
        assert!(evaluation.output.is_none());
    }

    #[test]
    fn hysteresis_prevents_flapping_but_allows_a_real_drop() {
        let rule = rule();
        let mut state = RuleState::new(rule.id.clone());

        // 59 °C -> 34.25 %
        let first = evaluate(&rule, &mut state, input(Some(59.0), 1_000));
        assert_eq!(first.output, Some(34.25));
        state.applied_output = first.output;
        state.writes += 1;

        // Rising is immediate.
        let up = evaluate(&rule, &mut state, input(Some(61.0), 2_000));
        assert_eq!(up.status, RuleStatus::Applied);
        assert_eq!(up.output, Some(36.5));
        state.applied_output = up.output;

        // A 1 °C drop is inside the 2 °C hysteresis: hold.
        let hold = evaluate(&rule, &mut state, input(Some(60.0), 3_000));
        assert_eq!(hold.status, RuleStatus::Held);
        assert_eq!(hold.output, Some(36.5));
        assert!(!hold.should_write);

        // Dropping 3 °C below the anchoring input steps down again.
        let down = evaluate(&rule, &mut state, input(Some(58.0), 4_000));
        assert_eq!(down.status, RuleStatus::Applied);
        assert!((down.output.unwrap() - 33.5).abs() < 1e-9);
    }

    #[test]
    fn hysteresis_zero_follows_the_curve_exactly() {
        let rule = rule().with_hysteresis(0.0);
        let mut state = RuleState::new(rule.id.clone());
        let first = evaluate(&rule, &mut state, input(Some(70.0), 1_000));
        state.applied_output = first.output;
        let second = evaluate(&rule, &mut state, input(Some(69.0), 2_000));
        assert_eq!(second.status, RuleStatus::Applied);
        assert_eq!(second.output, Some(48.5));
    }

    #[test]
    fn deadband_suppresses_tiny_changes() {
        let rule = rule().with_deadband(5.0);
        let mut state = RuleState::new(rule.id.clone());
        let first = evaluate(&rule, &mut state, input(Some(70.0), 1_000));
        assert_eq!(first.output, Some(50.0));
        state.applied_output = first.output;

        // 70.4 °C -> 50.6 %: only 0.6 % away, hold.
        let tiny = evaluate(&rule, &mut state, input(Some(70.4), 2_000));
        assert_eq!(tiny.status, RuleStatus::Held);
        assert!(!tiny.should_write);

        // 71.5 °C -> 54.5 %: 4.5 % is still inside the 5 % deadband.
        let small = evaluate(&rule, &mut state, input(Some(71.5), 3_000));
        assert_eq!(small.output, Some(54.5));
        assert!(!small.should_write);

        // 76 °C -> 68 %: 18 % away, write.
        let big = evaluate(&rule, &mut state, input(Some(76.0), 4_000));
        assert!(big.should_write);
        assert_eq!(big.output, Some(68.0));
    }

    // ---------------------------------------------------------------- gate
    fn gated_rule() -> Rule {
        rule().with_condition(Condition::new(
            "gpu.mock.0",
            "load.gpu",
            Comparator::Gt,
            60.0,
        ))
    }

    fn gate_input(source: Option<f64>, condition: Option<f64>, now_ms: i64) -> EvaluationInput {
        EvaluationInput {
            condition_value: condition,
            ..input(source, now_ms)
        }
    }

    #[test]
    fn a_false_condition_stands_the_rule_down_at_the_fail_safe_duty() {
        let rule = gated_rule();
        let mut state = RuleState::new(rule.id.clone());

        // Condition true: the curve steers.
        let open = evaluate(&rule, &mut state, gate_input(Some(70.0), Some(80.0), 1_000));
        assert_eq!(open.status, RuleStatus::Applied);
        assert_eq!(open.output, Some(50.0));
        state.applied_output = open.output;
        assert!(state.gate_open);

        // Condition false: the rule must NOT hold 50 % forever, it stands down.
        let closed = evaluate(&rule, &mut state, gate_input(Some(70.0), Some(10.0), 2_000));
        assert_eq!(closed.status, RuleStatus::Gated);
        assert!(closed.should_write);
        assert_eq!(
            closed.output,
            Some(70.0),
            "a closed gate applies the fail-safe duty, not the last curve value"
        );
        assert!(closed.message.contains("condition not met"));
        assert!(!state.gate_open);
        state.applied_output = closed.output;

        // Repeated evaluations do not rewrite the same value.
        let again = evaluate(&rule, &mut state, gate_input(Some(70.0), Some(10.0), 3_000));
        assert_eq!(again.status, RuleStatus::Gated);
        assert!(!again.should_write);
        assert_eq!(state.skipped, 1);
    }

    #[test]
    fn a_custom_otherwise_duty_is_used() {
        let rule = gated_rule().with_condition(
            Condition::new("gpu.mock.0", "load.gpu", Comparator::Gt, 60.0)
                .with_otherwise(OtherwiseAction::Fixed { percent: 35.0 }),
        );
        let mut state = RuleState::new(rule.id.clone());
        let closed = evaluate(&rule, &mut state, gate_input(Some(70.0), Some(5.0), 1_000));
        assert_eq!(closed.status, RuleStatus::Gated);
        assert_eq!(closed.output, Some(35.0));
    }

    #[test]
    fn the_gate_reopens_and_the_curve_takes_over_again() {
        let rule = gated_rule();
        let mut state = RuleState::new(rule.id.clone());
        let open = evaluate(&rule, &mut state, gate_input(Some(70.0), Some(80.0), 1_000));
        state.applied_output = open.output;
        let closed = evaluate(&rule, &mut state, gate_input(Some(70.0), Some(10.0), 2_000));
        state.applied_output = closed.output;
        assert_eq!(state.applied_output, Some(70.0));

        // Reopening must resume from the curve, and the stale hysteresis anchor
        // from before the gate closed must not pin the output at 70 %.
        let reopened = evaluate(&rule, &mut state, gate_input(Some(70.0), Some(90.0), 3_000));
        assert_eq!(reopened.status, RuleStatus::Applied);
        assert_eq!(reopened.output, Some(50.0));
        assert_eq!(state.last_status, RuleStatus::Applied);
    }

    #[test]
    fn condition_boundaries_are_exact() {
        // Every operator, tested on the threshold itself.
        for (op, satisfied) in [
            (Comparator::Gt, false),
            (Comparator::Gte, true),
            (Comparator::Lt, false),
            (Comparator::Lte, true),
            (Comparator::Eq, true),
            (Comparator::Ne, false),
        ] {
            assert_eq!(
                op.satisfied_by(60.0, 60.0),
                satisfied,
                "{} at the threshold",
                op.as_str()
            );
        }
        assert!(Comparator::Gt.satisfied_by(60.1, 60.0));
        assert!(!Comparator::Lt.satisfied_by(60.1, 60.0));

        // Equality absorbs float noise but not a real difference.
        assert!(Comparator::Eq.satisfied_by(60.0 + 1e-9, 60.0));
        assert!(!Comparator::Eq.satisfied_by(60.001, 60.0));
        assert!(Comparator::Ne.satisfied_by(60.001, 60.0));

        // A broken reading never satisfies anything.
        for op in Comparator::ALL {
            assert!(!op.satisfied_by(f64::NAN, 10.0), "{} NaN", op.as_str());
            assert!(
                !op.satisfied_by(10.0, f64::NAN),
                "{} NaN threshold",
                op.as_str()
            );
            assert!(
                !op.satisfied_by(f64::INFINITY, 10.0) && !op.satisfied_by(10.0, f64::INFINITY),
                "{} infinity",
                op.as_str()
            );
        }
        assert_eq!(Comparator::Gte.symbol(), "≥");
    }

    #[test]
    fn a_missing_condition_source_follows_the_sensor_failure_policy() {
        let rule = gated_rule();
        let mut state = RuleState::new(rule.id.clone());
        let open = evaluate(&rule, &mut state, gate_input(Some(70.0), Some(80.0), 1_000));
        state.applied_output = open.output;

        // No condition reading, no grace period configured: fallback, not a
        // silent "gate is open".
        let missing = evaluate(&rule, &mut state, gate_input(Some(70.0), None, 2_000));
        assert_eq!(missing.status, RuleStatus::Fallback);
        assert_eq!(missing.output, Some(70.0));
        assert_eq!(state.fallbacks, 1);
        assert!(!state.gate_open, "an unknown gate must not count as open");
    }

    #[test]
    fn the_condition_source_gets_the_same_grace_period() {
        let rule = gated_rule().with_fallback(Fallback {
            sensor_timeout_s: 5,
            ..Fallback::default()
        });
        let mut state = RuleState::new(rule.id.clone());
        let open = evaluate(&rule, &mut state, gate_input(Some(70.0), Some(80.0), 1_000));
        state.applied_output = open.output;

        // A blip in the load sensor is absorbed: the last known gate state is
        // kept, so the rule keeps steering (Held, because the curve value did not
        // change) instead of standing down or falling back.
        let blip = evaluate(&rule, &mut state, gate_input(Some(70.0), None, 2_000));
        assert!(
            matches!(blip.status, RuleStatus::Applied | RuleStatus::Held),
            "unexpected status during the blip: {:?}",
            blip.status
        );
        assert!(state.gate_open);
        assert_eq!(state.fallbacks, 0);

        // Past the grace period the rule falls back.
        let expired = evaluate(&rule, &mut state, gate_input(Some(70.0), None, 9_000));
        assert_eq!(expired.status, RuleStatus::Fallback);
    }

    #[test]
    fn a_rule_without_a_condition_is_unaffected() {
        // The gate machinery must be invisible for existing rules.
        let rule = rule();
        assert!(!rule.is_conditional());
        let mut state = RuleState::new(rule.id.clone());
        let evaluation = evaluate(&rule, &mut state, gate_input(Some(70.0), None, 1_000));
        assert_eq!(evaluation.status, RuleStatus::Applied);
        assert_eq!(evaluation.output, Some(50.0));
        assert!(state.gate_open, "an unconditional rule is always open");
    }

    #[test]
    fn a_condition_threshold_that_is_not_finite_is_rejected_by_validation() {
        let mut rule = gated_rule();
        rule.when.as_mut().unwrap().value = f64::NAN;
        assert!(rule.validate().is_err());
        rule.when.as_mut().unwrap().value = 60.0;
        rule.when.as_mut().unwrap().otherwise = OtherwiseAction::Fixed { percent: 150.0 };
        assert!(rule.validate().is_err());
        rule.when.as_mut().unwrap().otherwise = OtherwiseAction::Fixed { percent: 40.0 };
        rule.validate().unwrap();
    }

    #[test]
    fn a_grace_period_rides_out_a_short_sensor_outage() {
        use crate::rule::Fallback;
        let rule = rule().with_fallback(Fallback {
            sensor_timeout_s: 5,
            ..Fallback::default()
        });
        let mut state = RuleState::new(rule.id.clone());

        // Normal evaluation at 70 °C -> 50 %.
        let first = evaluate(&rule, &mut state, input(Some(70.0), 1_000));
        assert_eq!(first.output, Some(50.0));
        state.applied_output = first.output;

        // The sensor goes quiet. Two seconds later we are still inside the grace
        // period, so the rule keeps steering instead of falling back.
        let during = evaluate(&rule, &mut state, input(None, 3_000));
        assert_ne!(during.status, RuleStatus::Fallback, "must not give up yet");
        assert_eq!(during.output, Some(50.0));
        assert_eq!(state.fallbacks, 0);
        assert!(state.last_message.contains("last known") || state.last_message.contains("50"));

        // Past the grace period the fail-safe duty takes over.
        let after = evaluate(&rule, &mut state, input(None, 9_000));
        assert_eq!(after.status, RuleStatus::Fallback);
        assert_eq!(after.output, Some(70.0));
        assert_eq!(state.fallbacks, 1);
    }

    #[test]
    fn a_zero_grace_period_reacts_to_the_first_missing_reading() {
        // The default: no grace period, so a blip falls back immediately. This
        // is what keeps the behaviour predictable unless the user opts in.
        let rule = rule();
        assert_eq!(rule.fallback.sensor_timeout_s, 0);
        let mut state = RuleState::new(rule.id.clone());
        let first = evaluate(&rule, &mut state, input(Some(70.0), 1_000));
        state.applied_output = first.output;
        let missing = evaluate(&rule, &mut state, input(None, 1_100));
        assert_eq!(missing.status, RuleStatus::Fallback);
    }

    #[test]
    fn a_grace_period_never_invents_a_value_it_never_had() {
        use crate::rule::Fallback;
        let rule = rule().with_fallback(Fallback {
            sensor_timeout_s: 30,
            ..Fallback::default()
        });
        let mut state = RuleState::new(rule.id.clone());
        // The very first evaluation has no source at all: fall back, do not
        // guess from a value that does not exist.
        let evaluation = evaluate(&rule, &mut state, input(None, 1_000));
        assert_eq!(evaluation.status, RuleStatus::Fallback);
        assert_eq!(evaluation.output, Some(70.0));
    }

    #[test]
    fn missing_sensor_uses_the_safe_default() {
        let rule = rule();
        let mut state = RuleState::new(rule.id.clone());
        state.applied_output = Some(30.0);
        let evaluation = evaluate(&rule, &mut state, input(None, 1_000));
        assert_eq!(evaluation.status, RuleStatus::Fallback);
        assert_eq!(evaluation.output, Some(70.0));
        assert!(evaluation.should_write);
        assert!(evaluation.message.contains("missing"));
        assert_eq!(state.fallbacks, 1);
    }

    #[test]
    fn fallback_variants_behave() {
        // Hold keeps the last output and writes nothing.
        let hold = rule().with_fallback(Fallback {
            on_sensor_missing: FallbackAction::Hold,
            ..Fallback::default()
        });
        let mut state = RuleState::new(hold.id.clone());
        state.applied_output = Some(45.0);
        let evaluation = evaluate(&hold, &mut state, input(None, 1_000));
        assert_eq!(evaluation.status, RuleStatus::Fallback);
        assert_eq!(evaluation.output, Some(45.0));
        assert!(!evaluation.should_write);

        // Fixed drives an explicit duty.
        let fixed = rule().with_fallback(Fallback {
            on_sensor_missing: FallbackAction::Fixed { percent: 100.0 },
            ..Fallback::default()
        });
        let mut state = RuleState::new(fixed.id.clone());
        let evaluation = evaluate(&fixed, &mut state, input(None, 1_000));
        assert_eq!(evaluation.output, Some(100.0));
        assert!(evaluation.should_write);

        // `release` is refused by validation and sanitised on load. If it reaches
        // evaluation anyway, it must NOT be a no-op: the fail-safe duty is applied
        // and the message says why. Nothing may claim a release happened.
        let release = rule().with_fallback(Fallback {
            on_sensor_missing: FallbackAction::Release,
            ..Fallback::default()
        });
        let mut state = RuleState::new(release.id.clone());
        state.applied_output = Some(45.0);
        let evaluation = evaluate(&release, &mut state, input(None, 1_000));
        assert_eq!(evaluation.status, RuleStatus::Fallback);
        assert!(!evaluation.release, "no adapter can release a channel here");
        assert!(
            evaluation.should_write,
            "the output must actually be driven"
        );
        assert_eq!(evaluation.output, Some(70.0));
        assert!(evaluation.message.contains("not supported"));

        // When the sensor comes back the rule takes over again.
        let recovery = evaluate(&release, &mut state, input(Some(70.0), 2_000));
        assert_eq!(recovery.status, RuleStatus::Applied);
    }

    #[test]
    fn fallback_respects_target_range() {
        let rule = rule().with_fallback(Fallback {
            on_sensor_missing: FallbackAction::Fixed { percent: 100.0 },
            ..Fallback::default()
        });
        let mut state = RuleState::new(rule.id.clone());
        let mut eval_input = input(None, 1_000);
        eval_input.target_min = 40.0;
        eval_input.target_max = 60.0;
        let evaluation = evaluate(&rule, &mut state, eval_input);
        assert_eq!(evaluation.output, Some(60.0));
    }

    #[test]
    fn nan_input_is_treated_as_missing() {
        let rule = rule();
        let mut state = RuleState::new(rule.id.clone());
        let evaluation = evaluate(&rule, &mut state, input(Some(f64::NAN), 1_000));
        assert_eq!(evaluation.status, RuleStatus::Fallback);
    }

    #[test]
    fn rule_output_limits_and_hardware_limits_apply() {
        let rule = rule().with_output_limits(30.0, 80.0);
        let mut state = RuleState::new(rule.id.clone());
        // 85 °C would be 100 %, clamped to 80 %.
        let evaluation = evaluate(&rule, &mut state, input(Some(85.0), 1_000));
        assert_eq!(evaluation.output, Some(80.0));
        state.applied_output = evaluation.output;

        // A pump with a 60 % floor clamps the other way.
        let mut eval_input = input(Some(40.0), 2_000);
        eval_input.target_min = 60.0;
        eval_input.target_max = 100.0;
        let evaluation = evaluate(&rule, &mut state, eval_input);
        assert_eq!(evaluation.output, Some(60.0));
    }

    #[test]
    fn outcome_projection_is_ui_ready() {
        let rule = rule();
        let mut state = RuleState::new(rule.id.clone());
        let evaluation = evaluate(&rule, &mut state, input(Some(70.0), 1_000));
        state.applied_output = evaluation.output;
        state.writes = 1;
        let outcome = RuleOutcome::from_parts(&rule, &state, 1_000);
        assert_eq!(outcome.status, RuleStatus::Applied);
        assert_eq!(outcome.source, "gpu.mock.0/temperature.core");
        assert_eq!(outcome.target, "fan.mock.0/fan.speed_percent");
        assert_eq!(outcome.summary(), "GPU Cooling: 70 -> 50 %");
        assert_eq!(outcome.writes, 1);
        assert!(RuleStatus::Applied.is_active());
        assert_eq!(RuleStatus::Held.as_str(), "held");

        let json = serde_json::to_value(&outcome).unwrap();
        assert_eq!(json["status"], "applied");
        assert_eq!(json["rule_id"], "gpu-cooling");
    }

    #[test]
    fn reset_clears_control_memory() {
        let mut state = RuleState::new(RuleId::new("r").unwrap());
        state.applied_output = Some(50.0);
        state.armed_input = 70.0;
        state.released = true;
        state.reset();
        assert!(state.applied_output.is_none());
        assert!(state.armed_input.is_nan());
        assert!(!state.released);
    }

    #[test]
    fn combined_source_rule_evaluates_the_same_way() {
        use crate::rule::{Aggregate, SensorRef};
        let rule = Rule::new(
            "combined",
            "CPU + GPU",
            Source::combined(
                Aggregate::Max,
                vec![
                    SensorRef::new("cpu.mock.0", "temperature.core"),
                    SensorRef::new("gpu.mock.0", "temperature.core"),
                ],
            ),
            Target::new("fan.mock.0", "fan.speed_percent"),
            crate::curve::gpu_cooling_curve(),
        )
        .unwrap()
        .with_hysteresis(2.0)
        .with_deadband(0.0);
        let mut state = RuleState::new(rule.id.clone());
        let hottest = Aggregate::Max
            .reduce(&[55.0, 82.0])
            .expect("both sensors present");
        let evaluation = evaluate(&rule, &mut state, input(Some(hottest), 1_000));
        assert_eq!(evaluation.output, Some(88.0));
    }
}
