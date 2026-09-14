//! What a user can find out about a channel that was left behind.
//!
//! A handover that fails used to vanish: the queue was emptied before the write was
//! attempted, an `Unconfirmed` or failed write left nothing behind, and the only trace
//! was a log line. The channel stayed at the old duty, and nobody — not the CLI, not
//! the app — could tell that a handover was owed.
//!
//! These tests drive the same rig as `handover_integrity.rs` but assert on the
//! *record*: the state, the attempt count, the errors, who owns the channel now, and
//! the explicit re-arm action. Together the two files cover both halves of the
//! requirement: the hardware calls that must (or must not) happen, and the bookkeeping
//! that must survive them.

use ohm_automation::{
    AutomationEngine, HANDOVER_RETRY_TICKS, HandoverState, MAX_HANDOVER_ATTEMPTS,
};
use ohm_integration_tests::{
    Behaviour, FAIL_SAFE, PERCENT, PWM, RIG0, RIG1, flat_rule as flat, rig_session as session,
};

async fn tick(engine: &AutomationEngine) {
    engine.tick_force().await;
}

/// Run `count` ticks, so tests can express "give the retry cadence time" without
/// hard-coding the loop everywhere.
async fn ticks(engine: &AutomationEngine, count: usize) {
    for _ in 0..count {
        tick(engine).await;
    }
}

/// A queue with nothing in it is not the same as a queue that has been emptied: the
/// handover for a channel nobody drives must be visible from the moment it is owed.
#[tokio::test]
async fn an_owed_handover_is_listed_before_it_is_attempted() {
    let (_temp, _runtime, engine, _rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;

    assert!(
        engine.handovers().is_empty(),
        "a rule that is driving its own channel owes nothing"
    );

    // Retarget, and look *before* the engine has had a chance to act on it.
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    let owed = engine.handovers();
    assert_eq!(
        owed.len(),
        1,
        "the abandoned channel is on the record at once"
    );
    assert_eq!(owed[0].device.as_str(), RIG0);
    assert_eq!(owed[0].capability.as_str(), PERCENT);
    assert_eq!(owed[0].from_rule.as_str(), "r1");
    assert_eq!(owed[0].state, HandoverState::Pending);
    assert_eq!(owed[0].attempts, 0);
    assert!(owed[0].reason.contains("retargeted"), "{}", owed[0].reason);
    assert_eq!(engine.unfinished_handovers(), 1);
}

/// A refused handover keeps the device's own reason, and — because a reason that keeps
/// changing hides the cause — the first one as well as the latest.
#[tokio::test]
async fn a_failing_handover_records_why_and_how_often() {
    let (_temp, runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;

    rig.set_behaviour(RIG0, PERCENT, Behaviour::Refusing);
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    tick(&engine).await;
    ticks(&engine, HANDOVER_RETRY_TICKS as usize).await;

    let report = engine
        .handovers()
        .into_iter()
        .find(|report| report.device.as_str() == RIG0)
        .expect("the handover is still listed");
    assert_eq!(report.state, HandoverState::Pending);
    assert!(
        report.attempts >= 2,
        "it must have been attempted more than once: {}",
        report.attempts
    );
    let reason = report
        .first_error
        .clone()
        .expect("the first failure is kept");
    assert!(
        reason.contains("refused"),
        "the record must carry the device's reason: {reason}"
    );
    assert_eq!(
        report.last_error.as_deref(),
        Some(reason.as_str()),
        "with one kind of failure, first and last agree"
    );
    assert!(report.last_attempt_ms >= report.queued_at_ms);

    // And the same story reaches the user through the audit trail.
    let audit = runtime.audit().tail(60).unwrap();
    assert!(
        audit.iter().any(|entry| {
            entry["kind"] == "write"
                && entry["report"]["device_id"] == RIG0
                && entry["report"]["origin"]["kind"] == "safety"
        }),
        "each attempt is a real, audited safety write"
    );
}

/// An unconfirmed handover is not a success and not a failure: it stays pending, and
/// the record says the value was never confirmed.
#[tokio::test]
async fn an_unconfirmed_handover_stays_pending_with_its_reason() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;

    rig.set_behaviour(RIG0, PERCENT, Behaviour::Unconfirmed);
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    tick(&engine).await;

    let report = &engine.handovers()[0];
    assert_eq!(
        report.state,
        HandoverState::Pending,
        "an unconfirmed write leaves the job open, not done"
    );
    assert_eq!(report.confirmed_value, None, "nothing was confirmed");
    assert!(
        report
            .first_error
            .as_deref()
            .is_some_and(|error| error.contains("not confirmed")),
        "the record must say the write was accepted but not confirmed: {:?}",
        report.first_error
    );
}

/// The attempt budget is spent, the handover is parked — visibly — and nothing more
/// happens until the user says so.
#[tokio::test]
async fn an_exhausted_handover_is_parked_and_can_be_re_armed() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;

    rig.set_behaviour(RIG0, PERCENT, Behaviour::Refusing);
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    // Enough ticks for the whole attempt budget to be spent.
    ticks(
        &engine,
        (MAX_HANDOVER_ATTEMPTS as usize + 1) * (HANDOVER_RETRY_TICKS as usize + 1),
    )
    .await;

    let report = engine
        .handovers()
        .into_iter()
        .find(|report| report.device.as_str() == RIG0)
        .expect("a parked handover stays on the record");
    assert_eq!(report.state, HandoverState::Failed);
    assert_eq!(
        report.attempts, MAX_HANDOVER_ATTEMPTS,
        "the budget is what bounds the retrying"
    );
    assert!(report.first_error.is_some(), "and why it failed is kept");

    // Parked means parked.
    let spent = rig.attempts(RIG0, PERCENT);
    ticks(&engine, 30).await;
    assert_eq!(
        rig.attempts(RIG0, PERCENT),
        spent,
        "an exhausted handover may not keep writing to a broken channel"
    );

    // The documented recovery: fix the channel, then re-arm.
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Applied);
    assert_eq!(engine.retry_failed_handovers(), 1);
    assert_eq!(
        engine.handovers()[0].state,
        HandoverState::Pending,
        "a re-armed handover is unfinished again, and says so"
    );
    tick(&engine).await;
    assert_eq!(rig.held(RIG0, PERCENT), FAIL_SAFE);
    let report = engine
        .handovers()
        .into_iter()
        .find(|report| report.device.as_str() == RIG0)
        .expect("the finished handover is still on record");
    assert_eq!(report.state, HandoverState::Confirmed);
    assert_eq!(report.confirmed_value, Some(FAIL_SAFE));
    assert!(
        report.summary().contains("confirmed"),
        "{}",
        report.summary()
    );
}

/// A successful handover is recorded with the value it confirmed, and is not retried.
#[tokio::test]
async fn a_confirmed_handover_is_recorded_with_its_value() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    tick(&engine).await;

    assert_eq!(engine.unfinished_handovers(), 0);
    let report = engine
        .handovers()
        .into_iter()
        .find(|report| report.device.as_str() == RIG0)
        .expect("the handover is on record");
    assert_eq!(report.state, HandoverState::Confirmed);
    assert_eq!(report.confirmed_value, Some(FAIL_SAFE));
    assert_eq!(report.attempts, 1);

    let writes = rig.attempts(RIG0, PERCENT);
    ticks(&engine, 20).await;
    assert_eq!(
        rig.attempts(RIG0, PERCENT),
        writes,
        "a confirmed handover is done, not repeatedly re-applied"
    );
}

/// The handover outlives the rule that created it: the rule id is data, not a pointer.
#[tokio::test]
async fn a_handover_outlives_its_rule() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;

    rig.set_behaviour(RIG0, PERCENT, Behaviour::Refusing);
    engine.delete_rule("r1").unwrap();
    tick(&engine).await;

    assert!(engine.rule("r1").is_none());
    let report = engine
        .handovers()
        .into_iter()
        .find(|report| report.device.as_str() == RIG0)
        .expect("the record must survive the rule");
    assert_eq!(report.from_rule.as_str(), "r1");
    assert_eq!(report.state, HandoverState::Pending);
    assert!(
        report.reason.contains("deleted"),
        "and must say the rule was deleted: {}",
        report.reason
    );
    // It keeps being worked on, because the channel is still unprotected.
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Applied);
    ticks(&engine, HANDOVER_RETRY_TICKS as usize + 2).await;
    assert_eq!(rig.held(RIG0, PERCENT), FAIL_SAFE);
}

/// Editing the same rule twice before the engine runs is one unfinished job for one
/// channel — and the second edit must not reset what the first one already learned.
#[tokio::test]
async fn repeated_edits_of_one_channel_stay_one_handover() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;

    rig.set_behaviour(RIG0, PERCENT, Behaviour::Refusing);
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    tick(&engine).await;
    let attempts_after_first = rig.attempts(RIG0, PERCENT);

    // Three more edits, none of which involve the abandoned channel.
    for output in [60.0, 65.0, 70.0] {
        engine.save_rule(flat("r1", RIG1, PERCENT, output)).unwrap();
    }
    let reports = engine.handovers();
    assert_eq!(
        reports
            .iter()
            .filter(|report| report.device.as_str() == RIG0)
            .count(),
        1,
        "one channel is one handover: {:?}",
        reports.iter().map(|r| r.summary()).collect::<Vec<_>>()
    );
    assert!(
        reports[0].attempts >= 1,
        "and the attempts already made are not forgotten"
    );
    assert!(attempts_after_first >= 1);
}

/// A channel a rule took over again is not handed to the fail-safe duty — and the
/// record says so rather than staying pending for ever.
#[tokio::test]
async fn a_superseded_handover_names_the_rule_that_took_over() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;

    let mut successor = flat("r2", RIG0, PERCENT, 40.0);
    successor.enabled = false;
    engine.save_rule(successor).unwrap();
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    engine.set_rule_enabled("r2", true).unwrap();
    tick(&engine).await;

    assert_eq!(
        rig.held(RIG0, PERCENT),
        40.0,
        "the new owner decides the value"
    );
    let report = engine
        .handovers()
        .into_iter()
        .find(|report| report.device.as_str() == RIG0)
        .expect("the dropped handover is still on record");
    assert_eq!(report.state, HandoverState::Superseded);
    assert_eq!(
        report.superseded_by.as_ref().map(|rule| rule.as_str()),
        Some("r2")
    );
    assert_eq!(report.attempts, 0, "and nothing was written");
    assert!(
        report.summary().contains("r2"),
        "the summary must name the owner: {}",
        report.summary()
    );
}

/// A channel that was only ever targeted — never driven — leaves no handover to show,
/// because there was nothing to hand over.
#[tokio::test]
async fn a_channel_that_was_never_driven_leaves_no_record() {
    let (_temp, _runtime, engine, _rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    // Retarget twice without ever evaluating: neither channel was ever driven.
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    engine.save_rule(flat("r1", RIG0, PWM, 90.0)).unwrap();

    assert!(
        engine.handovers().is_empty(),
        "no channel was ever controlled, so no handover is owed: {:?}",
        engine.handovers()
    );
}
