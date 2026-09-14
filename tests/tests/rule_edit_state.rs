//! What happens to a rule's control memory when the rule is edited.
//!
//! Saving a rule used to reset only its schedule (`next_due_ms`). Everything else
//! survived: the last applied output, the hysteresis anchor, and the cached source
//! and condition readings. Retarget a rule from fan A to fan B and the engine
//! would compare the new curve output against **A's** value, decide nothing had
//! changed, skip the first write to B entirely — and report B as being at A's
//! speed.
//!
//! The rule enforced here: a change to what the rule *means* (target, source,
//! condition, curve shape) invalidates the memory that no longer applies, while a
//! change to metadata alone must not interrupt control.

use ohm_adapter_mock::MockConfig;
use ohm_automation::{Comparator, Condition, Rule, RuleStatus};
use ohm_core::ids::capability as caps;
use ohm_integration_tests::{Session, heavy_load};

/// A flat curve, so the requested value is unambiguous.
fn flat_rule(target: &str, output: f64) -> Rule {
    Rule::new(
        "retarget",
        "Retargetable",
        ohm_automation::Source::sensor("gpu.mock.0", caps::TEMPERATURE_CORE),
        ohm_automation::Target::new(target, caps::FAN_SPEED_PERCENT),
        ohm_automation::Curve::expect([(0.0, output), (100.0, output)]),
    )
    .expect("valid rule")
    .with_deadband(0.0)
}

/// The core of the defect: the new target must be driven from its own evidence on
/// the first evaluation, and the old target must not be left unmanaged.
#[tokio::test]
async fn retargeting_a_rule_drives_the_new_output_and_hands_over_the_old_one() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;
    // The simulated fans start at 40 %.
    assert_eq!(session.mock.status().fan_duties, vec![40.0, 40.0]);

    session
        .engine
        .save_rule(flat_rule("fan.mock.0", 55.0))
        .unwrap();
    session.step(1_000).await;
    assert_eq!(
        session.mock.status().fan_duties[0],
        55.0,
        "the rule drives fan 0 first"
    );

    // Retarget the same rule id at the other fan. The curve still asks for 55 %,
    // which is exactly what the rule believes fan 0 is at.
    session
        .engine
        .save_rule(flat_rule("fan.mock.1", 55.0))
        .unwrap();
    session.step(1_000).await;

    let duties = session.mock.status().fan_duties;
    assert_eq!(
        duties[1], 55.0,
        "the new target must be written on the first evaluation, not suppressed by the old target's value"
    );

    // The old output is handed over explicitly rather than abandoned at whatever
    // the curve last said.
    assert_eq!(
        duties[0], 70.0,
        "the abandoned output must be driven to the fail-safe duty"
    );

    let outcome = session.engine.outcome("retarget").unwrap();
    assert_eq!(outcome.target, "fan.mock.1/fan.speed_percent");
    assert_eq!(
        outcome.applied_output,
        Some(55.0),
        "the rule reports the value of the target it now owns"
    );

    session.shutdown().await;
}

/// The handover must be visible in the audit trail, with the reason attached.
#[tokio::test]
async fn the_handover_of_an_abandoned_output_is_audited() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;
    session
        .engine
        .save_rule(flat_rule("fan.mock.0", 55.0))
        .unwrap();
    session.step(1_000).await;
    session
        .engine
        .save_rule(flat_rule("fan.mock.1", 55.0))
        .unwrap();
    // The handover is queued by the save and performed at the start of the next
    // tick, because saving is synchronous and the write is not.
    session.step(1_000).await;

    let audit = session.runtime.audit().tail(30).unwrap();
    let handover = audit
        .iter()
        .find(|entry| {
            entry["kind"] == "write"
                && entry["report"]["device_id"] == "fan.mock.0"
                && entry["report"]["origin"]["kind"] == "safety"
        })
        .expect("the handover must be audited as a safety write");
    assert_eq!(handover["report"]["applied"], 70.0);
    assert!(
        handover["report"]["origin"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("retarget") || reason.contains("no longer")),
        "the audit must say why the old output was taken over: {}",
        handover["report"]["origin"]
    );

    session.shutdown().await;
}

/// A new source must not inherit the old one's reading or its grace period.
#[tokio::test]
async fn changing_the_source_discards_the_previous_reading() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;
    let rule = flat_rule("fan.mock.0", 55.0).with_fallback(ohm_automation::Fallback {
        sensor_timeout_s: 30,
        ..ohm_automation::Fallback::default()
    });
    session.engine.save_rule(rule).unwrap();
    session.step(1_000).await;
    assert_eq!(session.mock.status().fan_duties[0], 55.0);

    // Point the rule at a different sensor — one that reports nothing at all.
    let mut moved = flat_rule("fan.mock.0", 55.0).with_fallback(ohm_automation::Fallback {
        sensor_timeout_s: 30,
        ..ohm_automation::Fallback::default()
    });
    moved.source = ohm_automation::Source::sensor("ssd.mock.0", "temperature.core");
    session.mock.set_faults(ohm_adapter_mock::MockFaults {
        unavailable_readings: vec![(
            ohm_core::DeviceId::new_unchecked("ssd.mock.0"),
            ohm_core::CapabilityId::new_unchecked(caps::TEMPERATURE_CORE),
            ohm_device_model::UnavailableReason::ReadError,
        )],
        ..Default::default()
    });
    session.engine.save_rule(moved).unwrap();
    session.step(1_000).await;

    let outcome = session.engine.outcome("retarget").unwrap();
    assert_eq!(
        outcome.status,
        RuleStatus::Fallback,
        "a brand-new source with no reading must fall back, not reuse the old sensor's value \
         through the grace period (message: {})",
        outcome.message
    );
    assert_eq!(
        session.mock.status().fan_duties[0],
        70.0,
        "the fail-safe duty is what protects the machine here"
    );

    session.shutdown().await;
}

/// A changed condition must clear the gate's cached reading and state.
#[tokio::test]
async fn changing_the_condition_discards_the_gate_cache() {
    let session = Session::simulated().await;
    // Gate on GPU load above 99 %: closed on an idle machine.
    let mut rule = flat_rule("fan.mock.0", 55.0).with_condition(Condition::new(
        "gpu.mock.0",
        caps::GPU_LOAD,
        Comparator::Gt,
        99.0,
    ));
    session.engine.save_rule(rule.clone()).unwrap();
    session.step(1_000).await;
    assert_eq!(
        session.engine.outcome("retarget").unwrap().status,
        RuleStatus::Gated
    );

    // Replace the condition with one that is true. The cached "gate closed" state
    // and the cached load reading must not survive the edit.
    rule.when = Some(Condition::new(
        "gpu.mock.0",
        caps::GPU_LOAD,
        Comparator::Lt,
        99.0,
    ));
    session.engine.save_rule(rule).unwrap();
    session.step(1_000).await;

    let outcome = session.engine.outcome("retarget").unwrap();
    assert!(
        matches!(outcome.status, RuleStatus::Applied | RuleStatus::Held),
        "the rule must obey the new condition immediately, not the old gate state: {:?} {}",
        outcome.status,
        outcome.message
    );
    assert_eq!(session.mock.status().fan_duties[0], 55.0);

    session.shutdown().await;
}

/// Metadata-only edits must not interrupt control.
#[tokio::test]
async fn renaming_a_rule_does_not_rewrite_its_output() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;
    session
        .engine
        .save_rule(flat_rule("fan.mock.0", 55.0))
        .unwrap();
    session.step(1_000).await;
    assert_eq!(session.mock.status().fan_duties[0], 55.0);

    let audits_before = session.runtime.audit().tail(50).unwrap().len();
    let mut renamed = flat_rule("fan.mock.0", 55.0);
    renamed.name = "A Better Name".into();
    renamed.description = Some("still the same rule underneath".into());
    session.engine.save_rule(renamed).unwrap();
    session.step(1_000).await;

    assert_eq!(
        session.mock.status().fan_duties[0],
        55.0,
        "the output is unchanged"
    );
    let writes_after = session
        .runtime
        .audit()
        .tail(50)
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == "write")
        .count();
    let writes_before = session
        .runtime
        .audit()
        .tail(50)
        .unwrap()
        .iter()
        .take(audits_before)
        .filter(|entry| entry["kind"] == "write")
        .count();
    assert_eq!(
        writes_after, writes_before,
        "a rename must not produce a hardware write"
    );
    assert_eq!(
        session.engine.outcome("retarget").unwrap().name,
        "A Better Name"
    );

    session.shutdown().await;
}

/// The same rule, changed on disk and reloaded, must behave like an edit.
#[tokio::test]
async fn reloading_a_changed_rule_file_invalidates_the_same_state() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;
    session
        .engine
        .save_rule(flat_rule("fan.mock.0", 55.0))
        .unwrap();
    session.step(1_000).await;
    assert_eq!(session.mock.status().fan_duties[0], 55.0);

    // A hand edit on disk: same id, different target.
    let edited = r#"
name: Retargetable
id: retarget
source: { device: gpu.mock.0, capability: temperature.core }
target: { device: fan.mock.1, capability: fan.speed_percent }
curve:
  - [0, 55]
  - [100, 55]
hysteresis: 2
deadband: 0
update_interval_ms: 1000
"#;
    std::fs::write(session.paths.rules_dir().join("retarget.yaml"), edited).unwrap();
    session.engine.load_rules().unwrap();
    session.step(1_000).await;

    let duties = session.mock.status().fan_duties;
    assert_eq!(
        duties[1], 55.0,
        "a reloaded file with a new target must be written on the first evaluation"
    );
    assert_eq!(duties[0], 70.0, "and the old output is handed over");

    session.shutdown().await;
}
