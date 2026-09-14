//! The OpenHardwareOS automation engine.
//!
//! # The model
//!
//! ```text
//!  Sensor ──▶ Rule / Curve ──▶ Actuator
//! ```
//!
//! A [`Rule`] is plain data: where to read, how to map it, where to write, and
//! how to fail safely. It is stored as YAML in `<config>/rules`, so it can be
//! read, diffed, reviewed and shared.
//!
//! ```yaml
//! name: GPU Cooling
//! source:
//!   device: gpu.nvidia.0
//!   capability: temperature.core
//! target:
//!   device: fan.system.0
//!   capability: fan.speed_percent
//! curve:
//!   - [40, 20]
//!   - [60, 35]
//!   - [70, 50]
//!   - [80, 80]
//!   - [85, 100]
//! hysteresis: 2
//! update_interval_ms: 1000
//! fallback:
//!   on_sensor_missing: safe_default
//!   on_write_failure: safe_default
//! ```
//!
//! # Guarantees
//!
//! * **Curve mapping** — linear interpolation, clamped at both ends.
//! * **Min / Max** — rule level (`min_output` / `max_output`) *and* hardware
//!   level (the capability range), whichever is tighter.
//! * **Deadband** — changes smaller than `deadband` % are not written.
//! * **Hysteresis** — the output only falls once the source has dropped
//!   `hysteresis` units below the value that set the current output, which is
//!   what stops a fan from oscillating around a curve knee.
//! * **Update interval** — a rule is evaluated at most once per
//!   `update_interval_ms`.
//! * **Emergency fallback** — losing a sensor drives the output to the safety
//!   policy's fail-safe duty (or an explicit value, or releases control)
//!   instead of leaving a fan wherever it happened to be.
//!
//! # Where this is going
//!
//! The MVP deliberately implements only `sensor -> curve -> actuator`. The data
//! model already carries the pieces the roadmap needs:
//!
//! ```text
//! WHEN  <- a future `condition` block (load above X, app running, scene active)
//! IF/AND/OR <- aggregate sources (already: max / min / avg)
//! THEN  <- the target binding
//! SCENE / PROFILE <- groups of rules enabled together
//! EVENT <- the runtime event bus
//! ```
//!
//! A future *Natural Language Automation* module will translate a sentence such
//! as "keep the GPU cool while gaming, quiet otherwise" into exactly this YAML,
//! which is then **validated by [`AutomationEngine::check_rule`] before it can
//! touch hardware**. The AI never gets a write path of its own — that is what
//! keeps an LLM's mistakes from becoming a hardware incident.

pub mod curve;
pub mod engine;
pub mod evaluator;
pub mod examples;
pub mod handover;
pub mod recovery;
pub mod rule;
pub mod store;

pub use curve::{ControlPoint, Curve, gpu_cooling_curve};
pub use engine::{
    AutomationEngine, AutomationStats, MAX_CONSECUTIVE_UNCONFIRMED, RuleCheck, RuleConflict,
    TICK_INTERVAL_MS, merge_suggestions,
};
pub use evaluator::{
    ControlHold, Evaluation, EvaluationInput, RuleOutcome, RuleState, RuleStatus, evaluate,
};
pub use handover::{
    HANDOVER_OWNER_WAIT_TICKS, HANDOVER_RETRY_TICKS, HandoverReport, HandoverState,
    MAX_HANDOVER_ATTEMPTS,
};
pub use recovery::{
    RECOVERY_FILE, RECOVERY_VERSION, RecoveryRecord, RecoveryStore, StoredHandover, StoredHold,
};
pub use rule::{
    Aggregate, Comparator, Condition, DEFAULT_DEADBAND, DEFAULT_HYSTERESIS,
    DEFAULT_UPDATE_INTERVAL_MS, Fallback, FallbackAction, MIN_UPDATE_INTERVAL_MS, OtherwiseAction,
    Rule, SensorRef, Source, Target,
};
pub use store::{LoadReport, RuleFileError, RuleFileNote, RuleStore};
