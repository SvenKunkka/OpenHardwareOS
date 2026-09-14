//! Who really owns a channel — and when "someone else has it" is evidence rather
//! than an assumption.
//!
//! Round 3 stopped a stale handover from overwriting the rule that owns a channel.
//! It decided that by *declaration*: if an enabled rule named the channel as its
//! target, the handover was marked `Superseded` — resolved, off the record, done.
//! But naming a channel is not driving it. The review's reproduction:
//!
//! 1. R1 has written 55 % to fan A and is retargeted to fan B, so A is owed a
//!    handover to the fail-safe duty;
//! 2. R2 is enabled and targets fan A, but its sensor is gone and its
//!    `on_sensor_missing` is `hold`, so R2 writes nothing at all;
//! 3. the next tick sees R2's declaration, calls the channel taken over, and the
//!    fail-safe duty is never applied — while fan A sits at R1's abandoned 55 % and
//!    nothing anywhere records that it is unprotected.
//!
//! The distinction these tests enforce: a channel may be **claimed** (an enabled
//! rule targets it — so the engine must not fight that rule by writing), but a
//! handover is only **resolved** when the new owner has actually taken control
//! (a confirmed write to that channel), or when the fail-safe duty itself is
//! confirmed. A claimed-but-uncontrolled channel keeps its responsibility and its
//! reason on the record, waits a bounded time, and then fails visibly rather than
//! being dropped.
//!
//! The rig is the shared one from `ohm_integration_tests`: two fans, three writable
//! channels, each able to apply, accept-without-confirming, or refuse, plus a sensor
//! that can be made to stop answering. Nothing here touches real hardware.

use ohm_automation::{AutomationEngine, Fallback, FallbackAction, HandoverState, Rule, RuleStatus};
use ohm_core::ids::capability as caps;
use ohm_integration_tests::{
    Behaviour, FAIL_SAFE, PERCENT, PWM, RIG0, RIG1, flat_rule as flat, rig_session as session,
};

/// One engine cycle, with the runtime polled first — the order the app uses. The poll
/// matters in these tests: a faked sensor fault only becomes visible to the engine
/// once the runtime has re-read the device.
async fn tick(runtime: &ohm_runtime::Runtime, engine: &AutomationEngine) {
    runtime.poll_once().await.expect("poll");
    engine.tick_force().await;
}

async fn ticks(runtime: &ohm_runtime::Runtime, engine: &AutomationEngine, count: usize) {
    for _ in 0..count {
        tick(runtime, engine).await;
    }
}

/// A rule whose source can go missing, told to hold its last value when it does.
fn holding_rule(id: &str, source_device: &str, target_device: &str, output: f64) -> Rule {
    Rule::new(
        id,
        id,
        ohm_automation::Source::sensor(source_device, caps::TEMPERATURE_CORE),
        ohm_automation::Target::new(target_device, caps::FAN_SPEED_PERCENT),
        ohm_automation::Curve::expect([(0.0, output), (100.0, output)]),
    )
    .expect("valid rule")
    .with_deadband(0.0)
    .with_fallback(Fallback {
        on_sensor_missing: FallbackAction::Hold,
        ..Fallback::default()
    })
}

/// The handover record for a channel, if there is one.
fn record(engine: &AutomationEngine, device: &str) -> Option<ohm_automation::HandoverReport> {
    engine
        .handovers()
        .into_iter()
        .find(|report| report.device.as_str() == device)
}

/// Sets up the reproduction: A was driven by R1, R1 moved to B, R2 claims A.
async fn claimed_channel(
    owner_fails: bool,
) -> (
    tempfile::TempDir,
    ohm_runtime::Runtime,
    AutomationEngine,
    std::sync::Arc<ohm_integration_tests::Rig>,
) {
    let (temp, runtime, engine, rig) = session().await;
    // R1 drives fan A and really takes control of it.
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&runtime, &engine).await;
    assert_eq!(rig.held(RIG0, PERCENT), 55.0, "R1 takes channel A");

    // R1 moves to another channel: A is now owed a handover.
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();

    // R2 claims A, but cannot see its source. With `hold` it writes nothing at all.
    let mut successor = holding_rule("r2", RIG0, RIG0, 30.0);
    successor.enabled = false;
    engine.save_rule(successor).unwrap();
    if owner_fails {
        rig.set_reading_unavailable(RIG0, caps::TEMPERATURE_CORE);
        // The runtime has to re-read the device before the engine can see the fault.
        runtime.poll_once().await.expect("poll");
    }
    engine.set_rule_enabled("r2", true).unwrap();
    (temp, runtime, engine, rig)
}

// ---------------------------------------------------------------------------
// 1. A claim is not a takeover
// ---------------------------------------------------------------------------

/// The reproduction: the claiming rule writes nothing, so the channel is not taken
/// over — and the handover must not be filed as resolved.
#[tokio::test]
async fn a_claimed_channel_that_nobody_took_over_is_not_reported_as_resolved() {
    let (_temp, runtime, engine, rig) = claimed_channel(true).await;
    ticks(&runtime, &engine, 3).await;

    // The claiming rule really did write nothing.
    assert_eq!(
        rig.attempts(RIG0, PERCENT),
        1,
        "only R1's original write reached channel A: {:?}",
        rig.requested(RIG0, PERCENT)
    );
    assert!(
        engine.outcome("r2").is_some_and(|outcome| matches!(
            outcome.status,
            RuleStatus::Fallback | RuleStatus::Held | RuleStatus::Idle
        )),
        "the claiming rule has produced no output of its own: {:?}",
        engine.outcome("r2")
    );

    let owed = record(&engine, RIG0).expect("the handover must still be on the record");
    assert!(
        owed.state.is_unresolved(),
        "a channel that was abandoned and not taken over is still owed: {:?}",
        owed.summary()
    );
    assert!(
        owed.reason.contains("r2"),
        "the record must name who claims it and why it is not resolved: {}",
        owed.reason
    );
    assert_eq!(engine.unfinished_handovers(), 1);
}

/// The same thing, but the claim comes from a rule that is *trying* and failing: an
/// unconfirmed write is not a takeover either, because the value is unknown.
#[tokio::test]
async fn an_owner_whose_write_is_unconfirmed_has_not_taken_the_channel_over() {
    let (_temp, runtime, engine, rig) = claimed_channel(false).await;
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Unconfirmed);
    ticks(&runtime, &engine, 3).await;

    assert!(
        rig.attempts(RIG0, PERCENT) >= 2,
        "the claiming rule did try to write: {:?}",
        rig.requested(RIG0, PERCENT)
    );
    let report = engine.outcome("r2").expect("outcome");
    assert!(
        matches!(report.status, RuleStatus::Unconfirmed | RuleStatus::Error),
        "the owner's own status must say the write was not confirmed: {:?}",
        report.status
    );
    assert_eq!(report.applied_output, None, "nothing was confirmed");

    let owed = record(&engine, RIG0).expect("the handover is still owed");
    assert!(
        owed.state.is_unresolved(),
        "an unconfirmed write is not a takeover: {:?}",
        owed.summary()
    );
}

/// And a claiming rule whose writes are *refused* has not taken it over either. The
/// handover must stay visible — a refused owner plus a dropped handover would mean a
/// channel nobody watches.
#[tokio::test]
async fn an_owner_whose_writes_are_refused_has_not_taken_the_channel_over() {
    let (_temp, runtime, engine, rig) = claimed_channel(false).await;
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Refusing);
    ticks(&runtime, &engine, 3).await;

    assert_eq!(
        rig.held(RIG0, PERCENT),
        55.0,
        "nothing the owner asked for reached the hardware"
    );
    let owed = record(&engine, RIG0).expect("the handover is still owed");
    assert!(
        owed.state.is_unresolved(),
        "a refusing owner is not a takeover: {:?}",
        owed.summary()
    );
}

/// Waiting must not be silent or endless: after a bounded number of ticks, a claim
/// that never becomes control is a failure the user can see and act on.
#[tokio::test]
async fn a_claim_that_never_becomes_control_fails_visibly() {
    let (_temp, runtime, engine, _rig) = claimed_channel(true).await;
    // Well past the documented wait.
    ticks(
        &runtime,
        &engine,
        ohm_automation::HANDOVER_OWNER_WAIT_TICKS as usize + 5,
    )
    .await;

    let owed = record(&engine, RIG0).expect("the handover is still on the record");
    assert_eq!(
        owed.state,
        HandoverState::Failed,
        "an unmet claim must end as a visible failure, not as an endless wait: {}",
        owed.summary()
    );
    assert!(
        owed.reason.contains("r2"),
        "the failure must name the rule that claimed the channel: {}",
        owed.reason
    );
    assert_eq!(engine.unfinished_handovers(), 1, "and it is still owed");
}

// ---------------------------------------------------------------------------
// 2. When the claim ends, the handover resumes — without being replayed blindly
// ---------------------------------------------------------------------------

/// The owner takes control for real: now the handover is genuinely resolved, with
/// evidence, and the fail-safe duty is *not* written over the owner's value.
#[tokio::test]
async fn a_confirmed_takeover_resolves_the_handover_without_writing() {
    let (_temp, runtime, engine, rig) = claimed_channel(true).await;
    ticks(&runtime, &engine, 2).await;
    assert!(
        record(&engine, RIG0).is_some_and(|report| report.state.is_unresolved()),
        "owed while the claiming rule has produced nothing"
    );

    // The sensor answers again, so the owner produces an output and the write is
    // confirmed. That is the evidence the handover was waiting for.
    rig.set_reading_available(RIG0, caps::TEMPERATURE_CORE);
    ticks(&runtime, &engine, 2).await;

    let owner = engine.outcome("r2").expect("outcome");
    assert_eq!(
        owner.applied_output,
        Some(30.0),
        "the owner is driving the channel now: {:?} {}",
        owner.status,
        owner.message
    );
    assert_eq!(
        rig.held(RIG0, PERCENT),
        30.0,
        "the channel carries the owner's value, not the fail-safe duty: {:?}",
        rig.requested(RIG0, PERCENT)
    );

    let report = record(&engine, RIG0).expect("the handover is on the record");
    assert_eq!(
        report.state,
        HandoverState::Superseded,
        "it is resolved *by the takeover*, with evidence: {}",
        report.summary()
    );
    assert_eq!(
        report.superseded_by.as_ref().map(|rule| rule.as_str()),
        Some("r2"),
        "and it names who took it over"
    );
    assert_eq!(engine.unfinished_handovers(), 0);
    assert!(
        rig.handover_requests(RIG0, PERCENT) == 0,
        "the fail-safe duty must never have been written onto a channel the owner drives: {:?}",
        rig.requested(RIG0, PERCENT)
    );
}

/// Disabling the claiming rule hands the channel back to the handover, which then
/// completes normally: the claiming rule no longer exists as an owner.
#[tokio::test]
async fn disabling_the_claiming_rule_lets_the_handover_proceed() {
    let (_temp, runtime, engine, rig) = claimed_channel(true).await;
    ticks(&runtime, &engine, 3).await;
    assert_eq!(
        rig.held(RIG0, PERCENT),
        55.0,
        "nothing was written while the claim stood"
    );

    engine.set_rule_enabled("r2", false).unwrap();
    ticks(
        &runtime,
        &engine,
        ohm_automation::HANDOVER_RETRY_TICKS as usize + 3,
    )
    .await;

    assert_eq!(
        rig.held(RIG0, PERCENT),
        FAIL_SAFE,
        "with the claim gone, the abandoned channel is handed over: {:?}",
        rig.requested(RIG0, PERCENT)
    );
    let owed = record(&engine, RIG0).expect("the record stays");
    assert_eq!(owed.state, HandoverState::Confirmed);
}

/// Deleting the claiming rule does the same.
#[tokio::test]
async fn deleting_the_claiming_rule_lets_the_handover_proceed() {
    let (_temp, runtime, engine, rig) = claimed_channel(true).await;
    ticks(&runtime, &engine, 3).await;

    engine.delete_rule("r2").unwrap();
    ticks(
        &runtime,
        &engine,
        ohm_automation::HANDOVER_RETRY_TICKS as usize + 3,
    )
    .await;

    assert_eq!(rig.held(RIG0, PERCENT), FAIL_SAFE);
    assert_eq!(
        record(&engine, RIG0).map(|r| r.state),
        Some(HandoverState::Confirmed)
    );
}

/// Retargeting the claiming rule away is the same story.
#[tokio::test]
async fn retargeting_the_claiming_rule_lets_the_handover_proceed() {
    let (_temp, runtime, engine, rig) = claimed_channel(true).await;
    ticks(&runtime, &engine, 3).await;

    // A free channel: r1 now owns fan 1, so r2 moves to fan 0's other control channel.
    engine.save_rule(flat("r2", RIG0, PWM, 40.0)).unwrap();
    ticks(
        &runtime,
        &engine,
        ohm_automation::HANDOVER_RETRY_TICKS as usize + 3,
    )
    .await;

    assert_eq!(
        rig.held(RIG0, PERCENT),
        FAIL_SAFE,
        "channel A has no claimant any more"
    );
}

/// The round-3 guarantee must survive: once an owner has *confirmed* control, a
/// stale handover may not overwrite it.
#[tokio::test]
async fn a_confirmed_owner_is_never_overwritten_by_a_stale_handover() {
    let (_temp, runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&runtime, &engine).await;
    assert_eq!(rig.held(RIG0, PERCENT), 55.0);

    let mut successor = flat("r2", RIG0, PERCENT, 40.0);
    successor.enabled = false;
    engine.save_rule(successor).unwrap();
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    engine.set_rule_enabled("r2", true).unwrap();
    tick(&runtime, &engine).await;

    assert_eq!(rig.held(RIG0, PERCENT), 40.0, "the owner drives it");
    ticks(&runtime, &engine, 12).await;
    assert_eq!(
        rig.held(RIG0, PERCENT),
        40.0,
        "and keeps it: {:?}",
        rig.requested(RIG0, PERCENT)
    );
    assert_eq!(
        record(&engine, RIG0).map(|r| r.state),
        Some(HandoverState::Superseded),
        "resolved because the owner confirmed control, not because it declared a target"
    );
}
