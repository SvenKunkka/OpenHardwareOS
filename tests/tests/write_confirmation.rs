//! What the automation engine does with a write whose value is **unknown**.
//!
//! The adapter contract distinguishes three outcomes (see `WriteStatus`):
//!
//! * `Applied` / `Simulated` — the device is known to be at a value;
//! * `Unconfirmed` — the request was accepted, the resulting value is unknown;
//! * `Rejected` — the device refused.
//!
//! Collapsing "unconfirmed" into "applied" is how an application ends up
//! remembering a fan speed it never verified, counting a write that may not have
//! happened, and then *skipping* the retry because the unverified value looks
//! like the current state. These tests pin down all three consequences.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use ohm_adapter_api::{
    AdapterCapabilities, AdapterInfo, AdapterStatus, HardwareAdapter, WriteOutcome,
};
use ohm_automation::{AutomationEngine, MAX_CONSECUTIVE_UNCONFIRMED, Rule, RuleStatus, RuleStore};
use ohm_core::ids::capability as caps;
use ohm_core::{AdapterId, ConfigPaths, DeviceId};
use ohm_device_model::{
    Capability, Device, DeviceState, DeviceType, Reading, Transport, Unit, Value,
};
use ohm_runtime::{Runtime, Settings};

/// A fan whose writes are acknowledged but whose value stays unknown, until the
/// test decides otherwise.
struct UnconfirmingFan {
    writes: AtomicUsize,
    /// Number of initial writes to report as unconfirmed; everything after that is
    /// confirmed at the requested value.
    unconfirmed_writes: usize,
    /// The duty the "hardware" ends up at, recorded only for confirmed writes.
    held: parking_lot::Mutex<f64>,
}

impl UnconfirmingFan {
    fn new(unconfirmed_writes: usize) -> Arc<Self> {
        Arc::new(Self {
            writes: AtomicUsize::new(0),
            unconfirmed_writes,
            held: parking_lot::Mutex::new(20.0),
        })
    }

    fn writes(&self) -> usize {
        self.writes.load(Ordering::Relaxed)
    }

    fn held(&self) -> f64 {
        *self.held.lock()
    }
}

#[async_trait]
impl HardwareAdapter for UnconfirmingFan {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::new("unconfirming", "Unconfirming Fan", "unconfirming")
            .with_capabilities(AdapterCapabilities::cooling_control())
    }

    async fn probe(&self) -> AdapterStatus {
        AdapterStatus::available(self.id(), 1)
    }

    async fn discover(&self) -> ohm_core::Result<Vec<Device>> {
        Ok(vec![
            Device::new(
                DeviceId::new("fan.unconfirming.0").unwrap(),
                "Acknowledged But Unconfirmed",
                DeviceType::Fan,
                Transport::Mock,
                self.id(),
            )
            // A constant temperature source, so the curve output is predictable;
            // the actuator is what the rule writes to.
            .with_capability(Capability::sensor(
                "temperature.core",
                "Temperature",
                Unit::Celsius,
            ))
            .with_capability(Capability::actuator(
                "fan.speed_percent",
                "Fan Speed",
                Unit::Percent,
                0.0,
                100.0,
            )),
        ])
    }

    async fn read_state(&self, device: &Device) -> ohm_core::Result<DeviceState> {
        Ok(DeviceState::new(device.id.clone(), ohm_core::now_ms())
            .with_reading(Reading::ok("temperature.core", 50.0))
            .with_reading(Reading::ok("fan.speed_percent", self.held())))
    }

    async fn write(
        &self,
        _device: &Device,
        capability: &Capability,
        value: &Value,
    ) -> ohm_core::Result<WriteOutcome> {
        let index = self.writes.fetch_add(1, Ordering::Relaxed);
        let requested = value.as_f64().unwrap_or_default();
        if index < self.unconfirmed_writes {
            // Accepted, no idea what happened.
            return Ok(WriteOutcome::unconfirmed(format!(
                "the channel could not be read back after accepting {requested:.0} %"
            )));
        }
        *self.held.lock() = requested;
        Ok(WriteOutcome::applied(Value::Number(requested)))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A runtime whose only actuator is the unconfirming fan, plus the rule that
/// drives it.
async fn session(
    unconfirmed_writes: usize,
) -> (
    tempfile::TempDir,
    Runtime,
    AutomationEngine,
    Arc<UnconfirmingFan>,
) {
    let temp = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::from_root(temp.path());
    let fan = UnconfirmingFan::new(unconfirmed_writes);
    let adapter: Arc<dyn HardwareAdapter> = Arc::clone(&fan) as Arc<dyn HardwareAdapter>;
    let runtime = Runtime::new(paths.clone(), Settings::default(), vec![adapter]).unwrap();
    runtime.start().await.unwrap();
    let engine = AutomationEngine::new(runtime.clone(), RuleStore::from_paths(&paths));
    (temp, runtime, engine, fan)
}

fn rule() -> Rule {
    // A flat curve so the requested value is predictable: 40 % at every temperature.
    Rule::new(
        "unconfirmed",
        "Unconfirmed Fan",
        ohm_automation::Source::sensor("fan.unconfirming.0", "temperature.core"),
        ohm_automation::Target::new("fan.unconfirming.0", "fan.speed_percent"),
        ohm_automation::Curve::expect([(0.0, 40.0), (100.0, 40.0)]),
    )
    .unwrap()
    .with_deadband(0.0)
}

/// The unverified requested value must not become the rule's idea of the output.
#[tokio::test]
async fn an_unconfirmed_write_is_not_recorded_as_the_applied_value() {
    let (_temp, runtime, engine, fan) = session(1).await;
    engine.save_rule(rule()).unwrap();

    engine.tick_force().await;
    let outcome = engine.outcome("unconfirmed").expect("outcome");
    assert_eq!(fan.writes(), 1);
    assert_ne!(
        outcome.status,
        RuleStatus::Applied,
        "an unconfirmed write is not an applied write"
    );
    assert_eq!(
        outcome.applied_output, None,
        "the rule must not claim the fan is at 40 %: nobody verified that"
    );
    assert_eq!(
        outcome.writes, 0,
        "nothing was confirmed, so nothing was applied"
    );
    assert!(
        outcome.message.contains("not confirmed")
            || outcome.message.contains("unconfirmed")
            || outcome.message.contains("unknown"),
        "the message must say the value is unknown: {}",
        outcome.message
    );

    // The audit trail carries the same story: no applied value.
    let audit = runtime.audit().tail(10).unwrap();
    let entry = audit
        .iter()
        .find(|entry| entry["kind"] == "write")
        .expect("the attempt is audited");
    assert_eq!(entry["report"]["status"], "unconfirmed");
    assert!(
        entry["report"]["applied"].is_null(),
        "an unknown value must be absent from the audit entry, not filled in: {}",
        entry["report"]
    );
    assert!(
        entry["report"]["detail"]
            .as_str()
            .unwrap()
            .contains("read back")
    );
}

/// The deadband must not treat an unverified value as the current state, which
/// would silently stop the rule from ever confirming anything.
#[tokio::test]
async fn an_unconfirmed_write_is_retried_instead_of_being_deduplicated() {
    let (_temp, _runtime, engine, fan) = session(2).await;
    engine.save_rule(rule()).unwrap();

    // First attempt: unconfirmed.
    engine.tick_force().await;
    assert_eq!(fan.writes(), 1);
    assert_eq!(engine.outcome("unconfirmed").unwrap().applied_output, None);

    // Second attempt happens even though the curve value did not change — the
    // whole point is that we do not know the output is at 40 % yet.
    engine.tick_force().await;
    assert_eq!(fan.writes(), 2, "the rule must retry an unconfirmed write");

    // Third attempt is confirmed, and only now does the rule hold a value.
    engine.tick_force().await;
    assert_eq!(fan.writes(), 3);
    let outcome = engine.outcome("unconfirmed").unwrap();
    assert_eq!(outcome.status, RuleStatus::Applied);
    assert_eq!(outcome.applied_output, Some(40.0));
    assert_eq!(fan.held(), 40.0);

    // And now that the value IS confirmed, the deadband does its job again.
    engine.tick_force().await;
    assert_eq!(
        fan.writes(),
        3,
        "a confirmed value that has not changed is not rewritten"
    );
}

/// Repeated failure to confirm is a control failure: the rule's own write-failure
/// policy has to run — which is what attempts the safety fail-safe duty — and the
/// retrying has to stay bounded rather than spin.
#[tokio::test]
async fn repeated_unconfirmed_writes_trigger_the_write_failure_policy() {
    // Every write stays unconfirmed.
    let (_temp, runtime, engine, fan) = session(usize::MAX).await;
    engine.save_rule(rule()).unwrap();

    let mut attempts_at_failure = None;
    let mut ticks = 0;
    for _ in 0..12 {
        engine.tick_force().await;
        ticks += 1;
        if engine.outcome("unconfirmed").unwrap().status == RuleStatus::Error {
            attempts_at_failure = Some(fan.writes());
            break;
        }
    }

    let attempts =
        attempts_at_failure.expect("after repeated unconfirmed writes the failure policy must run");
    assert!(
        attempts >= MAX_CONSECUTIVE_UNCONFIRMED as usize,
        "the rule must try more than once before giving up: {attempts}"
    );
    assert!(
        attempts <= 2 * MAX_CONSECUTIVE_UNCONFIRMED as usize,
        "…and the retrying must stay bounded: {attempts} attempts over {ticks} ticks"
    );

    // The failure is visible, and it says what is actually wrong.
    let outcome = engine.outcome("unconfirmed").unwrap();
    assert_eq!(outcome.status, RuleStatus::Error);
    assert!(
        outcome.message.contains("never confirmed")
            || outcome.message.contains("cannot be verified"),
        "the message must explain the unconfirmed control: {}",
        outcome.message
    );
    assert!(engine.stats().failures >= 1);
    assert!(outcome.applied_output.is_none(), "still nothing confirmed");

    // The safety fail-safe duty was attempted through the ordinary write path, with
    // the safety origin, and the audit shows it even though it could not be
    // confirmed either.
    let audit = runtime.audit().tail(60).unwrap();
    let safety_attempt = audit
        .iter()
        .find(|entry| entry["kind"] == "write" && entry["report"]["origin"]["kind"] == "safety");
    let safety_attempt = safety_attempt.expect("the fail-safe duty must be attempted and audited");
    // Why the safety write happened lives in the origin, and what happened to it in
    // the status: both must be present, because either alone is misleading.
    assert!(
        safety_attempt["report"]["origin"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("unconfirmed write failure")),
        "the audit entry must say why the safety write happened: {}",
        safety_attempt["report"]
    );
    assert_eq!(
        safety_attempt["report"]["status"], "unconfirmed",
        "the fail-safe attempt was itself never confirmed, and the audit must not pretend otherwise"
    );
    assert!(
        safety_attempt["report"]["applied"].is_null(),
        "no applied value may be recorded for an unconfirmed write"
    );
}

/// A recovered channel must return the rule to normal control.
#[tokio::test]
async fn control_recovers_once_a_write_is_confirmed_again() {
    let (_temp, _runtime, engine, fan) = session(1).await;
    engine.save_rule(rule()).unwrap();

    engine.tick_force().await;
    assert_eq!(engine.outcome("unconfirmed").unwrap().applied_output, None);

    engine.tick_force().await;
    let recovered = engine.outcome("unconfirmed").unwrap();
    assert_eq!(recovered.status, RuleStatus::Applied);
    assert_eq!(recovered.applied_output, Some(40.0));
    assert_eq!(recovered.message.contains("not confirmed"), false);
    assert_eq!(
        engine.stats().failures,
        0,
        "a single unconfirmed write is not a failure"
    );
}
