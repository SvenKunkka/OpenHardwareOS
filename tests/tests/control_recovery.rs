//! Control responsibility that outlives the process.
//!
//! A handover that is still owed, and a write whose result was never learned, are the
//! two pieces of state where losing the record means losing track of a channel nobody
//! is protecting. Both used to live only in memory: kill the app while a handover is
//! pending, restart, and the new process had no idea — no pending item, an empty
//! `ohm-cli handovers`, nothing on the Diagnostics panel.
//!
//! These tests do what a restart does: build an engine, leave work unresolved, drop
//! the whole thing, then build a *new* engine and runtime over the same isolated config
//! directory and check what came back. What must come back is the responsibility —
//! never a restored curve value, never a stored output treated as confirmed, and never
//! a blind replay of the command the old session was about to send.

use ohm_adapter_api::HardwareAdapter;
use ohm_automation::{AutomationEngine, HandoverState, RECOVERY_FILE, RuleStore};
use ohm_core::ConfigPaths;
use ohm_integration_tests::{
    Behaviour, FAIL_SAFE, PERCENT, RIG0, RIG1, Rig, flat_rule as flat, rig_session,
};
use ohm_runtime::{Runtime, Settings};
use std::sync::Arc;

/// The recovery file inside a config directory.
fn recovery_file(paths: &ConfigPaths) -> std::path::PathBuf {
    paths.root().join(RECOVERY_FILE)
}

/// A fresh runtime + engine over an existing config directory, using the same rig — the
/// restart, without a new temporary directory.
async fn restart(paths: &ConfigPaths, rig: &Arc<Rig>) -> (Runtime, AutomationEngine) {
    let adapter: Arc<dyn HardwareAdapter> = Arc::clone(rig) as Arc<dyn HardwareAdapter>;
    let runtime = Runtime::new(paths.clone(), Settings::default(), vec![adapter]).unwrap();
    runtime.start().await.unwrap();
    let engine = AutomationEngine::new(runtime.clone(), RuleStore::from_paths(paths));
    (runtime, engine)
}

async fn tick(runtime: &Runtime, engine: &AutomationEngine) {
    runtime.poll_once().await.expect("poll");
    engine.tick_force().await;
}

/// A handover that could not be completed must be there after a restart — with its
/// channel, its reason, its cause and its attempt count — and must not be acted on
/// until the device has been checked.
#[tokio::test]
async fn an_unfinished_handover_survives_a_restart() {
    let (temp, runtime, engine, rig) = rig_session().await;
    let paths = ConfigPaths::from_root(temp.path());

    // R1 drives fan 0, then moves away: fan 0 is owed the fail-safe duty.
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&runtime, &engine).await;
    // …and the channel refuses the handover, so it stays unresolved.
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Refusing);
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    tick(&runtime, &engine).await;
    tick(&runtime, &engine).await;

    let before = engine
        .handovers()
        .into_iter()
        .find(|report| report.device.as_str() == RIG0)
        .expect("owed before the restart");
    assert!(before.state.is_unresolved(), "{}", before.summary());
    assert!(
        recovery_file(&paths).exists(),
        "the responsibility must be on disk, not only in memory"
    );

    // The process goes away. Everything in memory is gone with it.
    engine.stop().await;
    let _ = runtime.shutdown().await;
    drop(engine);
    drop(runtime);

    // A new process over the same config directory.
    let (runtime2, engine2) = restart(&paths, &rig).await;
    engine2.load_rules().unwrap();

    let recovered = engine2
        .handovers()
        .into_iter()
        .find(|report| report.device.as_str() == RIG0)
        .expect("the responsibility must come back");
    assert_eq!(
        recovered.state,
        HandoverState::NeedsVerification,
        "it comes back needing verification, not as live work: {}",
        recovered.summary()
    );
    assert_eq!(
        recovered.capability.as_str(),
        PERCENT,
        "the channel identity is kept"
    );
    assert_eq!(recovered.from_rule.as_str(), "r1");
    assert!(
        recovered.first_error.is_some(),
        "the original cause survives the restart: {recovered:?}"
    );
    assert!(
        recovered
            .reason
            .contains("recovered from the previous session"),
        "the record says where it came from: {}",
        recovered.reason
    );
    assert_eq!(engine2.unfinished_handovers(), 1);

    // Verification happens against the hardware as it is now, and then the ordinary
    // safety policy applies: the channel is healthy, so it is handed over.
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Applied);
    tick(&runtime2, &engine2).await;
    tick(&runtime2, &engine2).await;
    assert_eq!(
        rig.held(RIG0, PERCENT),
        FAIL_SAFE,
        "the recovered responsibility is discharged through the fail-safe duty, not by \
         replaying what the old session was about to do: {:?}",
        rig.requested(RIG0, PERCENT)
    );
    assert_eq!(
        engine2
            .handovers()
            .into_iter()
            .find(|report| report.device.as_str() == RIG0)
            .map(|report| report.state),
        Some(HandoverState::Confirmed)
    );

    engine2.stop().await;
    let _ = runtime2.shutdown().await;
}

/// A write whose result was never learned is a responsibility too: the channel may have
/// moved. If the rule that made it no longer drives the channel, the record must bring
/// it back for the fail-safe duty.
#[tokio::test]
async fn an_unconfirmed_write_is_a_recoverable_responsibility() {
    let (temp, runtime, engine, rig) = rig_session().await;
    let paths = ConfigPaths::from_root(temp.path());

    // The rule writes, the channel never confirms, and then the rule is deleted: the
    // channel may be at a value nobody verified.
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Unconfirmed);
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&runtime, &engine).await;
    assert_eq!(
        engine.outcome("r1").unwrap().applied_output,
        None,
        "nothing was confirmed"
    );
    // The app goes away while the rule is still enabled, so no handover is queued for
    // the channel — only the issued-but-unconfirmed write is on the record.
    engine.stop().await;
    let _ = runtime.shutdown().await;
    drop(engine);
    drop(runtime);
    assert!(
        recovery_file(&paths).exists(),
        "the unverified write must be on disk"
    );

    // While the app is closed, the rule file is removed (the user tidied up, another
    // tool deleted it). Nobody is answerable for that channel any more.
    std::fs::remove_file(paths.rules_dir().join("r1.yaml")).unwrap();

    let (runtime2, engine2) = restart(&paths, &rig).await;
    engine2.load_rules().unwrap();
    assert!(
        engine2.rule("r1").is_none(),
        "the rule is gone from this session"
    );

    let recovered = engine2
        .handovers()
        .into_iter()
        .find(|report| report.device.as_str() == RIG0)
        .expect("the unverified write must come back as a responsibility");
    assert_eq!(recovered.state, HandoverState::NeedsVerification);
    assert!(
        recovered.reason.contains("never confirmed"),
        "and must say what happened: {}",
        recovered.reason
    );
    assert!(
        recovered.first_error.is_some(),
        "with a cause the user can read: {recovered:?}"
    );

    // With the channel healthy again, the recovered responsibility is discharged.
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Applied);
    tick(&runtime2, &engine2).await;
    tick(&runtime2, &engine2).await;
    assert_eq!(rig.held(RIG0, PERCENT), FAIL_SAFE);
    assert_eq!(
        engine2
            .handovers()
            .into_iter()
            .find(|report| report.device.as_str() == RIG0)
            .map(|report| report.state),
        Some(HandoverState::Confirmed)
    );

    engine2.stop().await;
    let _ = runtime2.shutdown().await;
}

/// A rule that still drives the channel will settle its own unknown write by writing
/// again — so nothing is recovered for it, and nothing is written behind its back.
#[tokio::test]
async fn an_unconfirmed_write_by_a_still_enabled_rule_is_not_double_handled() {
    let (temp, runtime, engine, rig) = rig_session().await;
    let paths = ConfigPaths::from_root(temp.path());

    rig.set_behaviour(RIG0, PERCENT, Behaviour::Unconfirmed);
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&runtime, &engine).await;
    engine.stop().await;
    let _ = runtime.shutdown().await;
    drop(engine);
    drop(runtime);

    let (runtime2, engine2) = restart(&paths, &rig).await;
    engine2.load_rules().unwrap();
    assert!(
        engine2.handovers().is_empty(),
        "a channel its own rule still drives is not owed a handover: {:?}",
        engine2
            .handovers()
            .iter()
            .map(|report| report.summary())
            .collect::<Vec<_>>()
    );

    // The rule takes it back over by writing again, and nothing else wrote to it.
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Applied);
    tick(&runtime2, &engine2).await;
    tick(&runtime2, &engine2).await;
    assert_eq!(
        engine2.outcome("r1").unwrap().applied_output,
        Some(55.0),
        "the rule is driving its channel again"
    );
    assert_eq!(
        rig.held(RIG0, PERCENT),
        55.0,
        "and the value on the channel is the rule's, not the fail-safe duty: {:?}",
        rig.requested(RIG0, PERCENT)
    );

    engine2.stop().await;
    let _ = runtime2.shutdown().await;
}

/// Work that is already finished must not come back. A confirmed handover is done, and
/// re-doing it would write to a channel that has an owner.
#[tokio::test]
async fn a_completed_handover_is_not_replayed_after_a_restart() {
    let (temp, runtime, engine, rig) = rig_session().await;
    let paths = ConfigPaths::from_root(temp.path());

    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&runtime, &engine).await;
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    tick(&runtime, &engine).await;
    assert_eq!(rig.held(RIG0, PERCENT), FAIL_SAFE, "handed over");
    // With nothing left owed, the record file is removed rather than left claiming work.
    assert!(
        !recovery_file(&paths).exists(),
        "a clean state must not leave a record claiming outstanding work"
    );

    engine.stop().await;
    let _ = runtime.shutdown().await;
    // The runtime's own shutdown path also writes to controlled channels (audited with
    // the `shutdown` origin), so the count to compare against is the one *after* the
    // first session has fully gone away.
    drop(engine);
    drop(runtime);

    let writes_after_shutdown = rig.attempts(RIG0, PERCENT);
    let (runtime2, engine2) = restart(&paths, &rig).await;
    engine2.load_rules().unwrap();
    assert!(
        engine2
            .handovers()
            .iter()
            .all(|report| report.state.is_resolved()),
        "nothing unresolved after the restart"
    );
    for _ in 0..6 {
        tick(&runtime2, &engine2).await;
    }
    assert_eq!(
        rig.attempts(RIG0, PERCENT),
        writes_after_shutdown,
        "a completed handover must not be replayed: {:?}",
        rig.requested(RIG0, PERCENT)
    );

    engine2.stop().await;
    let _ = runtime2.shutdown().await;
}

/// A channel that no longer exists on this machine cannot be handed over. The
/// responsibility is not dropped and not invented away: it is reported as failed, with
/// the reason, and left for the user to re-arm once the device is back.
#[tokio::test]
async fn a_recovered_channel_whose_device_is_gone_fails_visibly() {
    let (temp, runtime, engine, rig) = rig_session().await;
    let paths = ConfigPaths::from_root(temp.path());

    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&runtime, &engine).await;
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Refusing);
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    tick(&runtime, &engine).await;
    engine.stop().await;
    let _ = runtime.shutdown().await;
    drop(engine);
    drop(runtime);

    // A machine where that fan is not present: the record names `fan.rig.0`, and this
    // "restart" only has fan 1.
    let adapter: Arc<dyn HardwareAdapter> = Arc::clone(&rig) as Arc<dyn HardwareAdapter>;
    let runtime2 = Runtime::new(paths.clone(), Settings::default(), vec![adapter]).unwrap();
    runtime2.start().await.unwrap();
    // Remove fan 0 from the rig's discovery by pointing the record at a channel that no
    // device exposes — the honest simulation of "this machine does not have that fan".
    let store = ohm_automation::RecoveryStore::from_paths(&paths);
    let mut record = store.load().unwrap().expect("a record exists");
    let stale = record
        .handovers
        .iter_mut()
        .find(|item| item.device.as_str() == RIG0)
        .expect("the owed channel is in the record");
    stale.device = ohm_core::DeviceId::new("fan.rig.9").unwrap();
    stale.capability = ohm_core::CapabilityId::new(PERCENT).unwrap();
    store.save(&record).unwrap();

    let engine2 = AutomationEngine::new(runtime2.clone(), RuleStore::from_paths(&paths));
    engine2.load_rules().unwrap();
    tick(&runtime2, &engine2).await;

    let recovered = engine2
        .handovers()
        .into_iter()
        .find(|report| report.device.as_str() == "fan.rig.9")
        .expect("the responsibility is still on the record");
    assert_eq!(
        recovered.state,
        HandoverState::Failed,
        "a channel that cannot be resolved is a visible failure: {}",
        recovered.summary()
    );
    assert!(
        recovered.reason.contains("fan.rig.9"),
        "and the reason names the channel: {recovered:?}"
    );
    assert!(
        recovered
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("fan.rig.9")),
        "the verification failure is the latest error: {recovered:?}"
    );
    assert_eq!(
        engine2.retry_failed_handovers(),
        1,
        "a failed recovered item is explicitly re-armable"
    );
    assert_eq!(
        rig.attempts("fan.rig.9", PERCENT),
        0,
        "and nothing was written to a channel that does not exist"
    );

    engine2.stop().await;
    let _ = runtime2.shutdown().await;
}

/// A damaged record is reported, never guessed at: the engine starts, says what
/// happened, and does not invent a responsibility or drop one silently.
#[tokio::test]
async fn a_corrupt_record_is_reported_not_guessed_at() {
    let (temp, runtime, engine, rig) = rig_session().await;
    let paths = ConfigPaths::from_root(temp.path());
    engine.stop().await;
    let _ = runtime.shutdown().await;
    drop(engine);
    drop(runtime);

    std::fs::write(recovery_file(&paths), "{ not json at all").unwrap();

    let (runtime2, engine2) = restart(&paths, &rig).await;
    // Loading rules must not panic, and must surface the problem.
    let result = engine2.load_rules();
    assert!(result.is_ok(), "a damaged record must not stop the engine");
    let reported = engine2
        .persistence_error()
        .expect("the problem must be visible to the caller");
    assert!(
        reported.contains("control record") || reported.contains("could not be read"),
        "{reported}"
    );
    assert!(
        engine2.handovers().is_empty(),
        "nothing may be invented from a file that cannot be read"
    );

    engine2.stop().await;
    let _ = runtime2.shutdown().await;
}

/// Storage that cannot be written is reported, not silently treated as saved.
#[tokio::test]
async fn a_persistence_failure_is_reported_not_hidden() {
    let (temp, runtime, engine, rig) = rig_session().await;
    let paths = ConfigPaths::from_root(temp.path());

    // A directory in the place of the record file: writing it must fail.
    std::fs::create_dir_all(recovery_file(&paths)).unwrap();

    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&runtime, &engine).await;
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();

    let reported = engine
        .persistence_error()
        .expect("a failed save must be reported, not swallowed");
    assert!(
        reported.contains("could not record"),
        "the message must say what could not be recorded: {reported}"
    );
    assert!(
        reported.contains("fail-safe") || reported.contains("recovered"),
        "and what the consequence is: {reported}"
    );

    engine.stop().await;
    let _ = runtime.shutdown().await;
    let _ = rig;
}

/// The recovered state must be visible through the same API the CLI and the desktop
/// use, so both report the same facts.
#[tokio::test]
async fn the_recovered_state_is_the_same_for_every_reader() {
    let (temp, runtime, engine, rig) = rig_session().await;
    let paths = ConfigPaths::from_root(temp.path());

    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&runtime, &engine).await;
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Refusing);
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    tick(&runtime, &engine).await;
    engine.stop().await;
    let _ = runtime.shutdown().await;
    drop(engine);
    drop(runtime);

    let (runtime2, engine2) = restart(&paths, &rig).await;
    engine2.load_rules().unwrap();

    // One reader: the engine's report list — exactly what `rule_handovers` returns to
    // the desktop and what `ohm-cli handovers` prints. Both front-ends switch on the
    // same state, so both describe the item the same way.
    let reports = engine2.handovers();
    let owed: Vec<_> = reports.iter().filter(|r| r.state.is_unresolved()).collect();
    assert_eq!(
        owed.len(),
        1,
        "one recovered responsibility, on both front-ends"
    );
    let report = owed[0];
    assert_eq!(report.state, HandoverState::NeedsVerification);
    assert!(
        report
            .reason
            .contains("recovered from the previous session"),
        "the shared reason must say the item has not been checked yet: {}",
        report.reason
    );
    assert!(
        report.summary().contains("recovered"),
        "and so must the one-line summary both front-ends show: {}",
        report.summary()
    );

    engine2.stop().await;
    let _ = runtime2.shutdown().await;
}
