//! `fallback: release` — the action that used to do nothing.
//!
//! `FallbackAction::Release` claims to hand a channel back so the firmware takes
//! over. No adapter in this build can do that mid-run (`LibreHardwareMonitorAdapter
//! ::shutdown` releases on exit, which is a different thing), so the engine set a
//! flag, published a "released" event and **left the fan exactly where it was**.
//!
//! The rule this file enforces: an action that cannot be performed must not be
//! accepted, and must not be silently executed as a no-op on a machine that
//! already has it configured. Acceptance is stated in terms of what actually
//! happens to the output — write, lose the source, observe the output — never in
//! terms of a status enum.

use ohm_adapter_mock::MockConfig;
use ohm_automation::{Fallback, FallbackAction, Rule, RuleStatus, RuleStore};
use ohm_core::ConfigPaths;
use ohm_integration_tests::{Session, gpu_sensor_disconnected, heavy_load};

/// Saving a rule that asks for `release` must fail, with a reason that says what
/// to use instead. It used to be accepted and then silently did nothing.
#[tokio::test]
async fn release_cannot_be_saved() {
    let session = Session::simulated().await;
    let action = FallbackAction::Release;

    let rule = session.gpu_rule().with_fallback(Fallback {
        on_sensor_missing: action,
        ..Fallback::default()
    });
    let check = session.engine.check_rule(&rule);
    assert!(
        !check.is_ok(),
        "release must be refused as an error, not accepted: {check:?}"
    );
    assert!(
        check
            .errors
            .iter()
            .any(|error| error.contains("release") && error.contains("safe_default")),
        "the error must name the problem and the alternative: {:?}",
        check.errors
    );
    assert!(
        session.engine.save_rule(rule).is_err(),
        "saving a release rule must fail"
    );

    let write_failure_rule = session.gpu_rule().with_fallback(Fallback {
        on_write_failure: action,
        ..Fallback::default()
    });
    assert!(
        session.engine.save_rule(write_failure_rule).is_err(),
        "on_write_failure: release must fail too"
    );

    session.shutdown().await;
}

/// A machine that already has such a rule on disk must keep the file, be told
/// what happened, and end up doing something safe.
#[tokio::test]
async fn a_legacy_release_rule_loads_with_a_diagnostic_and_still_protects_the_machine() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;

    let legacy = r#"
name: Legacy Release Rule
source: { device: gpu.mock.0, capability: temperature.core }
target: { device: fan.mock.0, capability: fan.speed_percent }
curve:
  - [40, 20]
  - [85, 100]
hysteresis: 2
update_interval_ms: 1000
fallback:
  on_sensor_missing: release
  on_write_failure: release
"#;
    let rules_dir = session.paths.rules_dir();
    std::fs::create_dir_all(&rules_dir).unwrap();
    let path = rules_dir.join("legacy-release.yaml");
    std::fs::write(&path, legacy).unwrap();

    let report = session
        .engine
        .load_rules()
        .expect("loading never fails hard");
    assert_eq!(
        report.rules.len(),
        1,
        "the rule must still load: dropping it would leave the machine unmanaged"
    );

    // Told, not silently rewritten.
    let notes = session.engine.compatibility_notes();
    assert_eq!(
        notes.len(),
        2,
        "both fallback fields asked for release, so both substitutions are reported"
    );
    assert!(
        notes
            .iter()
            .all(|note| note.rule_id.as_str() == "legacy-release")
    );
    assert!(
        notes
            .iter()
            .any(|note| note.message.contains("fallback.on_sensor_missing")),
        "{notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|note| note.message.contains("fallback.on_write_failure")),
        "{notes:?}"
    );
    for note in &notes {
        assert!(
            note.message.contains("release")
                && note.message.contains("safe_default")
                && note.message.contains("not modified"),
            "the note must explain the substitution and that the file is untouched: {}",
            note.message
        );
    }

    // The file keeps its original text, so the user can decide what to do.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(
        on_disk.contains("on_sensor_missing: release"),
        "the rule file must not be rewritten behind the user's back"
    );

    // And the rule now behaves safely: the source disappears, the output moves.
    for _ in 0..10 {
        session.step(1_000).await;
    }
    let steering = session.fan_duty();
    assert!(steering > 25.0, "the rule was steering: {steering}");

    session.mock.set_faults(gpu_sensor_disconnected());
    session.step(1_000).await;
    session.engine.tick_force().await;

    let outcome = session.engine.outcome("legacy-release").unwrap();
    assert_ne!(
        outcome.status,
        RuleStatus::Released,
        "nothing was released, so nothing may claim to have been"
    );
    assert_eq!(
        session.fan_duty(),
        70.0,
        "the sanitised rule must apply the fail-safe duty, not leave the fan where it was"
    );
    assert!(
        outcome.message.contains("missing") || outcome.message.contains("fallback"),
        "the message must describe the fallback that happened: {}",
        outcome.message
    );

    session.shutdown().await;
}

/// The action must also be refused on the paths that do not go through the form.
#[tokio::test]
async fn a_release_rule_imported_as_yaml_is_refused_by_validation() {
    let yaml = r#"
name: Imported Release
source: { device: gpu.mock.0, capability: temperature.core }
target: { device: fan.mock.0, capability: fan.speed_percent }
curve: [[40, 20], [85, 100]]
fallback:
  on_sensor_missing: release
"#;
    let rule: Rule = serde_yaml_ng::from_str(yaml).expect("it parses");

    // Parsing is fine (compatibility), but the structural validation used by every
    // save path must reject it.
    let error = rule
        .validate()
        .expect_err("a release fallback must not validate");
    assert!(error.to_string().contains("release"), "{error}");
}

/// `release` must not be documented as working anywhere the machine can see it.
#[test]
fn a_release_rule_file_reports_its_substitution_through_the_store() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::from_root(temp.path());
    let rules_dir = paths.rules_dir();
    std::fs::create_dir_all(&rules_dir).unwrap();
    std::fs::write(
        rules_dir.join("r.yaml"),
        "name: R\nsource: {device: gpu.mock.0, capability: temperature.core}\n\
         target: {device: fan.mock.0, capability: fan.speed_percent}\n\
         curve: [[40, 20], [85, 100]]\nfallback: {on_sensor_missing: release}\n",
    )
    .unwrap();

    let store = RuleStore::from_paths(&paths);
    let report = store.load_report();
    assert!(
        report.is_clean(),
        "a compatibility substitution is not a load error"
    );
    assert_eq!(report.rules.len(), 1);
    assert_eq!(
        report.rules[0].fallback.on_sensor_missing,
        FallbackAction::SafeDefault,
        "the in-memory rule is what runs, and it must be safe"
    );
    assert_eq!(
        report.notes.len(),
        1,
        "and the substitution must be reported"
    );
}
