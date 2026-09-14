//! The `when` gate, end to end: schema -> parse -> engine -> simulated hardware.
//!
//! The unit tests in `crates/ohm-automation/src/evaluator.rs` pin the decision
//! logic. This file proves the whole chain works on a live runtime: the gate is
//! read from a second sensor, the rule stands down when it closes, it resumes
//! when it reopens, and the emergency ceiling cannot be gated away.

use ohm_adapter_mock::MockConfig;
use ohm_automation::{Comparator, Condition, OtherwiseAction, RuleStatus};
use ohm_core::ids::capability as caps;
use ohm_integration_tests::{Session, heavy_load, idle_load};
use ohm_runtime::Settings;

const GATED_YAML: &str = r#"
name: GPU Cooling (gaming only)
source: { device: gpu.mock.0, capability: temperature.core }
when:
  source: { device: gpu.mock.0, capability: load.gpu }
  op: gt
  value: 60
  otherwise: safe_default
target: { device: fan.mock.0, capability: fan.speed_percent }
curve:
  - [40, 20]
  - [60, 35]
  - [70, 50]
  - [80, 80]
  - [85, 100]
hysteresis: 2
deadband: 2
update_interval_ms: 1000
fallback:
  on_sensor_missing: safe_default
  on_write_failure: safe_default
  sensor_timeout_s: 5
"#;

/// The shipped example in `examples/rules` must stay loadable, so a schema change
/// that breaks it is a test failure rather than a surprise for a user.
#[test]
fn the_shipped_gated_example_parses_and_validates() {
    let raw = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/rules/gpu-cooling-gaming-only.yaml"),
    )
    .expect("the example exists");
    let rule: ohm_automation::Rule = serde_yaml_ng::from_str(&raw).expect("it parses");
    rule.validate().expect("it is structurally valid");

    let condition = rule.when.expect("it is gated");
    assert_eq!(condition.op, Comparator::Gt);
    assert_eq!(condition.value, 60.0);
    assert_eq!(condition.source.capability.as_str(), caps::GPU_LOAD);
    assert_eq!(rule.source.label(), "gpu.mock.0/temperature.core");
}

#[tokio::test]
async fn the_gate_stands_the_rule_down_and_lets_it_resume() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: idle_load(),
        ..MockConfig::deterministic()
    })
    .await;
    let rule: ohm_automation::Rule = serde_yaml_ng::from_str(GATED_YAML).unwrap();
    session.engine.save_rule(rule).unwrap();

    // Idle machine: the gate is closed from the first evaluation, so the rule is
    // standing down at the fail-safe duty rather than steering a curve.
    for _ in 0..5 {
        session.step(1_000).await;
    }
    let standing_down = session.engine.outcome("gpu-cooling-gaming-only").unwrap();
    assert_eq!(
        standing_down.status,
        RuleStatus::Gated,
        "message: {}",
        standing_down.message
    );
    assert_eq!(standing_down.applied_output, Some(70.0));
    assert_eq!(session.fan_duty(), 70.0);

    // Now the GPU gets busy: the gate opens and the curve takes over.
    session.mock.set_load_profile(heavy_load());
    for _ in 0..60 {
        session.step(1_000).await;
    }
    let steering = session.engine.outcome("gpu-cooling-gaming-only").unwrap();
    assert!(
        matches!(steering.status, RuleStatus::Applied | RuleStatus::Held),
        "unexpected status {:?}: {}",
        steering.status,
        steering.message
    );
    // The curve is in charge again. Note the duty can legitimately be *below* the
    // stand-down duty at moderate temperatures — that is the point of the gate:
    // the value now comes from the curve, not from `otherwise`.
    let temperature = session
        .reading("gpu.mock.0", caps::TEMPERATURE_CORE)
        .expect("temperature");
    let expected = session
        .engine
        .rule("gpu-cooling-gaming-only")
        .unwrap()
        .curve
        .eval(temperature)
        .clamp(25.0, 100.0);
    // The applied value lags the instantaneous curve by at most the rule's
    // deadband (2 % here): the rule only rewrites when the curve has moved that
    // far, which is exactly what keeps a fan from twitching.
    let deadband = session
        .engine
        .rule("gpu-cooling-gaming-only")
        .unwrap()
        .deadband;
    assert!(
        (session.fan_duty() - expected).abs() <= deadband + 1.0,
        "the fan must follow the curve while the gate is open: duty {} vs curve {} at {} °C \
         (deadband {deadband})",
        session.fan_duty(),
        expected,
        temperature
    );

    // And it stands down again when the load goes away.
    session.mock.set_load_profile(idle_load());
    for _ in 0..40 {
        session.step(1_000).await;
    }
    let again = session.engine.outcome("gpu-cooling-gaming-only").unwrap();
    assert_eq!(again.status, RuleStatus::Gated);
    assert_eq!(session.fan_duty(), 70.0);

    session.shutdown().await;
}

#[tokio::test]
async fn the_emergency_ceiling_cannot_be_gated_away() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: idle_load(),
        ..MockConfig::deterministic()
    })
    .await;
    // A gate that is closed by construction: the load can never exceed 60 %.
    // `load.gpu` is reported in PERCENT (`adapters/mock` multiplies by 100), so a
    // threshold of 99 means "only when the GPU is essentially saturated". Using
    // 0.99 here would compare a fraction against a percentage and the gate would
    // be open all the time — the unit of a threshold is the unit of its source.
    let rule = session.gpu_rule().with_condition(
        Condition::new("gpu.mock.0", caps::GPU_LOAD, Comparator::Gt, 99.0)
            .with_otherwise(OtherwiseAction::Fixed { percent: 30.0 }),
    );
    session.engine.save_rule(rule).unwrap();
    for _ in 0..5 {
        session.step(1_000).await;
    }
    assert_eq!(
        session.engine.outcome("gpu-cooling").unwrap().status,
        RuleStatus::Gated
    );
    assert_eq!(
        session.fan_duty(),
        30.0,
        "the gate stands the rule down at 30 %"
    );

    // The machine gets dangerously hot while the rule is standing down.
    session.mock.force_gpu_temperature(95.0);
    session.step(1_000).await;

    assert!(
        session.fan_duty() >= 100.0,
        "the emergency ceiling must override a closed gate, got {}",
        session.fan_duty()
    );
    assert!(session.runtime.stats().safety_interventions > 0);
    assert!(
        session
            .engine
            .outcome("gpu-cooling")
            .unwrap()
            .message
            .contains("condition not met")
    );

    session.shutdown().await;
}

#[tokio::test]
async fn an_invalid_threshold_is_refused_before_it_can_be_saved() {
    let session = Session::simulated().await;
    let mut rule = session.gpu_rule().with_condition(Condition::new(
        "gpu.mock.0",
        caps::GPU_LOAD,
        Comparator::Gt,
        f64::NAN,
    ));
    let check = session.engine.check_rule(&rule);
    assert!(!check.is_ok(), "a NaN threshold must be an error");
    assert!(session.engine.save_rule(rule.clone()).is_err());

    // A condition reading a capability that does not exist is refused too.
    rule.when = Some(Condition::new(
        "gpu.mock.0",
        "does.not.exist",
        Comparator::Gt,
        1.0,
    ));
    assert!(!session.engine.check_rule(&rule).is_ok());

    // A threshold outside the source's range cannot be met: a warning, not an error.
    rule.when = Some(Condition::new(
        "gpu.mock.0",
        caps::GPU_LOAD,
        Comparator::Gt,
        250.0,
    ));
    let check = session.engine.check_rule(&rule);
    assert!(check.is_ok(), "{:?}", check.errors);
    assert!(
        check
            .warnings
            .iter()
            .any(|w| w.contains("never be satisfied")),
        "{:?}",
        check.warnings
    );

    // Equality on an analog reading is allowed but flagged.
    rule.when = Some(Condition::new(
        "gpu.mock.0",
        caps::GPU_LOAD,
        Comparator::Eq,
        50.0,
    ));
    let check = session.engine.check_rule(&rule);
    assert!(
        check.warnings.iter().any(|w| w.contains("gte")),
        "{:?}",
        check.warnings
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_gated_rule_that_reaches_a_target_owned_by_another_rule_is_still_refused() {
    // The conflict rule applies to gated and ungated rules alike.
    let session = Session::simulated().await;
    session.engine.save_rule(session.gpu_rule()).unwrap();
    let gated: ohm_automation::Rule = serde_yaml_ng::from_str(GATED_YAML).unwrap();
    let error = session
        .engine
        .save_rule(gated)
        .expect_err("the same output is already owned");
    assert!(error.to_string().contains("already owns"), "{error}");

    session.shutdown().await;
}

/// A gated rule must not be able to run while automation is switched off.
#[tokio::test]
async fn automation_off_silences_gated_rules_too() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;
    let rule: ohm_automation::Rule = serde_yaml_ng::from_str(GATED_YAML).unwrap();
    session.engine.save_rule(rule).unwrap();
    session
        .runtime
        .update_settings(|settings| settings.automation_enabled = false)
        .unwrap();

    let duties_before = session.mock.status().fan_duties.clone();
    for _ in 0..10 {
        session.step(1_000).await;
    }
    assert_eq!(
        session.mock.status().fan_duties,
        duties_before,
        "no rule may write while automation is off"
    );
    assert_eq!(
        session.runtime.settings().automation_enabled,
        false,
        "the setting is honestly reported"
    );
    let _ = Settings::default();
    session.shutdown().await;
}
