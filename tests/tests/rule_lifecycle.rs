//! The rule file lifecycle: the documented YAML, written by a user, loaded by
//! the engine, driving real hardware, surviving a restart.
//!
//! This is the "plain data, not code" promise, verified end to end.

use ohm_adapter_mock::MockConfig;
use ohm_core::ids::capability as caps;
use ohm_integration_tests::{Session, heavy_load};

/// Exactly the YAML from the project brief.
const DOCUMENTED_YAML: &str = r#"
name: GPU Cooling
source:
  device: gpu.mock.0
  capability: temperature.core
target:
  device: fan.mock.0
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

#[tokio::test]
async fn documented_yaml_runs_the_cooling_loop() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;

    // Write the rule by hand, exactly where a user would put it.
    let rules_dir = session.paths.rules_dir();
    std::fs::create_dir_all(&rules_dir).unwrap();
    std::fs::write(rules_dir.join("gpu-cooling.yaml"), DOCUMENTED_YAML).unwrap();

    // Reload from disk, as a restart would.
    let report = session.engine.load_rules().expect("rules load");
    assert!(report.is_clean(), "{:?}", report.errors);
    assert_eq!(report.rules.len(), 1);

    let rule = session.engine.rule("gpu-cooling").expect("rule present");
    assert_eq!(rule.name, "GPU Cooling");
    assert_eq!(rule.hysteresis, 2.0);
    assert_eq!(rule.curve.eval(70.0), 50.0);

    // Run it: the GPU heats, the fan follows the curve.
    for _ in 0..90 {
        session.step(1_000).await;
    }
    let outcome = session.engine.outcome("gpu-cooling").unwrap();
    assert!(outcome.writes > 0, "the YAML rule never wrote");
    assert!(
        session.fan_duty() >= 80.0,
        "a hot GPU must be followed by high airflow, got {}",
        session.fan_duty()
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_combined_source_rule_uses_the_hottest_sensor() {
    let session = Session::simulated().await;
    let yaml = r#"
name: System Cooling
source:
  aggregate: max
  sensors:
    - {device: cpu.mock.0, capability: temperature.core}
    - {device: gpu.mock.0, capability: temperature.core}
target:
  device: fan.mock.1
  capability: fan.speed_percent
curve:
  - [40, 20]
  - [85, 100]
hysteresis: 2
deadband: 1
update_interval_ms: 500
"#;
    let rule: ohm_automation::Rule = serde_yaml_ng::from_str(yaml).unwrap();
    assert_eq!(
        rule.source.aggregate(),
        Some(ohm_automation::Aggregate::Max)
    );
    let check = session.engine.check_rule(&rule);
    assert!(check.is_ok(), "{:?}", check.errors);
    session.engine.save_rule(rule).unwrap();

    // Heat only the GPU: the MAX source must follow it.
    session.mock.set_gpu_load(1.0);
    session.mock.set_cpu_load(0.02);
    for _ in 0..60 {
        session.step(1_000).await;
    }
    let outcome = session.engine.outcome("system-cooling").unwrap();
    let gpu_temp = session
        .reading("gpu.mock.0", caps::TEMPERATURE_CORE)
        .unwrap();
    assert!(
        outcome.input.unwrap() >= gpu_temp - 0.5,
        "MAX must follow the hot sensor ({:?} vs {gpu_temp})",
        outcome.input
    );
    assert!(session.mock.status().fan_duties[1] > 25.0);

    session.shutdown().await;
}

#[tokio::test]
async fn a_rule_for_missing_hardware_is_rejected_with_a_reason() {
    let session = Session::simulated().await;
    let yaml = r#"
name: Nonexistent Fan
source: {device: gpu.nvidia.0, capability: temperature.core}
target: {device: fan.system.0, capability: fan.speed_percent}
curve: [[40, 20], [85, 100]]
"#;
    let rule: ohm_automation::Rule = serde_yaml_ng::from_str(yaml).unwrap();
    let check = session.engine.check_rule(&rule);
    assert!(!check.is_ok());
    assert!(check.errors.iter().any(|e| e.contains("device_not_found")
        || e.contains("not found")
        || e.contains("no such")));

    let error = session.engine.save_rule(rule).unwrap_err();
    assert!(error.to_string().contains("not found") || error.to_string().contains("device"));

    session.shutdown().await;
}

#[tokio::test]
async fn a_broken_rule_file_does_not_stop_the_app() {
    let session = Session::simulated().await;
    let rules_dir = session.paths.rules_dir();
    std::fs::create_dir_all(&rules_dir).unwrap();
    std::fs::write(rules_dir.join("broken.yaml"), "name: [this is not valid").unwrap();
    std::fs::write(rules_dir.join("incomplete.yaml"), "name: No Source\n").unwrap();

    // A good rule next to the broken ones still loads and works.
    session
        .engine
        .save_rule(session.gpu_rule())
        .expect("the good rule saves");

    let report = session
        .engine
        .load_rules()
        .expect("loading never fails hard");
    assert_eq!(report.rules.len(), 1, "only the good rule loads");
    assert_eq!(report.errors.len(), 2, "both broken files are reported");
    assert_eq!(session.engine.rule_count(), 1);

    session.shutdown().await;
}

#[tokio::test]
async fn editing_a_rule_replaces_it_and_reloads_immediately() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;
    let mut rule = session.gpu_rule();
    session.engine.save_rule(rule.clone()).unwrap();
    session.step(1_000).await;
    let first = session.fan_duty();

    // Tighten the curve: 60 °C now means 100 % instead of 35 %.
    rule.curve = ohm_automation::Curve::expect([(40.0, 50.0), (60.0, 100.0)]);
    session.engine.save_rule(rule).unwrap();

    // The rule was re-armed, so the next tick applies the new curve at once.
    session.step(1_000).await;
    let second = session.fan_duty();
    assert!(
        second > first,
        "the edited curve must take effect immediately ({first} -> {second})"
    );
    assert_eq!(session.engine.rule_count(), 1, "editing must not duplicate");

    // And it is on disk in a readable form.
    let raw = std::fs::read_to_string(session.paths.rules_dir().join("gpu-cooling.yaml")).unwrap();
    assert!(raw.contains("name: GPU Cooling"));
    assert!(
        raw.contains("[60.0, 100.0]") || raw.contains("- - 60.0"),
        "{raw}"
    );

    session.shutdown().await;
}
