//! What happens to a channel when a rule stops driving it.
//!
//! Round 2 fixed the case "the rule was retargeted from fan A to fan B" and stopped
//! there. A review found three groups of holes left around the same idea — *a rule
//! leaving a channel behind it* — and this file pins each of them down with the
//! observable hardware calls, not with log strings:
//!
//! 1. **Not every way of leaving control was covered.** `RuleChange` compared only
//!    `target.device`, so switching *capability* on the same device was not a
//!    change at all; disabling a rule, deleting it, or deleting its file and
//!    reloading dropped the rule's memory without ever handing the channel over.
//! 2. **A handover was queued for channels the rule never controlled.** Queuing did
//!    not look at whether the rule was enabled, or had ever written to that channel,
//!    and the queue was executed without re-checking who owns the channel *now*. A
//!    disabled rule could therefore seize a channel out from under the rule that
//!    really controls it, and a channel edited away before the first tick could be
//!    handed over even though nothing had ever driven it.
//! 3. **A failed handover disappeared.** The queue was drained with `mem::take`
//!    before the write was attempted, so an `Unconfirmed` or failed handover left
//!    nothing behind: no retry, no visible record, and a channel stuck at the old
//!    low duty forever once the link came back.
//!
//! The rig (in the shared test harness) is a fake adapter with two fans and three
//! writable channels
//! (`fan.rig.0/fan.speed_percent`, `fan.rig.0/fan.pwm`, `fan.rig.1/fan.speed_percent`),
//! each of which can be told to apply, to accept without confirming, or to refuse.
//! Nothing here goes near real hardware.

use ohm_automation::AutomationEngine;
use ohm_integration_tests::{
    Behaviour, FAIL_SAFE, PERCENT, PWM, RIG0, RIG1, flat_rule as flat, rig_session as session,
};

/// Drive the engine directly: these tests are about the engine's control
/// bookkeeping, so no wall-clock time and no polling loop are involved.
async fn tick(engine: &AutomationEngine) {
    engine.tick_force().await;
}

// ---------------------------------------------------------------------------
// 1. Every way of leaving control
// ---------------------------------------------------------------------------

/// Changing the *capability* on the same device is a retarget, and it was invisible:
/// `RuleChange` compared device ids only, so the old channel was never handed over
/// and the new channel's first write was suppressed by the old channel's value.
#[tokio::test]
async fn switching_capability_on_the_same_device_hands_over_the_old_channel() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;
    assert_eq!(rig.held(RIG0, PERCENT), 55.0, "the rule drives the channel");

    // Same device, different capability: a different channel, still a retarget.
    engine.save_rule(flat("r1", RIG0, PWM, 90.0)).unwrap();
    tick(&engine).await;

    assert_eq!(
        rig.held(RIG0, PWM),
        90.0,
        "the new channel must be written on the first evaluation, not suppressed by the old \
         channel's value"
    );
    assert_eq!(
        rig.held(RIG0, PERCENT),
        FAIL_SAFE,
        "the abandoned channel must be handed over to the fail-safe duty"
    );
}

/// Disabling a rule is leaving control. It used to leave the channel at whatever the
/// curve last said, forever.
#[tokio::test]
async fn disabling_a_rule_hands_its_channel_over() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;
    assert_eq!(rig.held(RIG0, PERCENT), 55.0);

    engine.set_rule_enabled("r1", false).unwrap();
    tick(&engine).await;

    assert_eq!(
        rig.held(RIG0, PERCENT),
        FAIL_SAFE,
        "a disabled rule must not leave its channel at the last curve value"
    );
}

/// Deleting a rule is leaving control too.
#[tokio::test]
async fn deleting_a_rule_hands_its_channel_over() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;
    assert_eq!(rig.held(RIG0, PERCENT), 55.0);

    engine.delete_rule("r1").unwrap();
    tick(&engine).await;

    assert_eq!(
        rig.held(RIG0, PERCENT),
        FAIL_SAFE,
        "deleting a rule must not abandon its channel at the old value"
    );
}

/// A rule whose file disappears and is reloaded is the same event as a deletion.
#[tokio::test]
async fn a_rule_file_removed_from_disk_is_handed_over_on_reload() {
    let (temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;
    assert_eq!(rig.held(RIG0, PERCENT), 55.0);

    std::fs::remove_file(temp.path().join("rules").join("r1.yaml")).unwrap();
    engine.load_rules().unwrap();
    tick(&engine).await;

    assert!(
        engine.rule("r1").is_none(),
        "the rule is gone from the active set"
    );
    assert_eq!(
        rig.held(RIG0, PERCENT),
        FAIL_SAFE,
        "a rule that vanishes from disk must be handed over like a deleted one"
    );
}

/// The counterpart: a source change keeps the *same* channel, under the *same* rule,
/// so it must not be handed over — the channel still has an owner.
#[tokio::test]
async fn changing_only_the_source_does_not_hand_the_channel_over() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;
    let baseline = rig.attempts(RIG0, PERCENT);

    let mut moved = flat("r1", RIG0, PERCENT, 55.0);
    moved.source = ohm_automation::Source::sensor(RIG1, "temperature.core");
    engine.save_rule(moved).unwrap();
    tick(&engine).await;

    assert_eq!(
        rig.handover_requests(RIG0, PERCENT),
        0,
        "the channel never lost its owner, so no handover may be written to it"
    );
    assert!(
        rig.attempts(RIG0, PERCENT) >= baseline,
        "the rule keeps driving its own channel"
    );
}

// ---------------------------------------------------------------------------
// 2. Handovers must follow real control
// ---------------------------------------------------------------------------

/// The reproduction from the review. A *disabled* rule that never drove a channel
/// must not be able to seize it: the enabled rule that really controls it must keep
/// both the channel and the display of it.
#[tokio::test]
async fn a_disabled_rule_that_never_drove_a_channel_cannot_hand_it_over() {
    let (_temp, runtime, engine, rig) = session().await;
    // R2 controls fan 0 at 40 %.
    engine.save_rule(flat("r2", RIG0, PERCENT, 40.0)).unwrap();
    tick(&engine).await;
    assert_eq!(rig.held(RIG0, PERCENT), 40.0);

    // R1 targets the same channel but is disabled, so it owns nothing.
    let mut idle = flat("r1", RIG0, PERCENT, 55.0);
    idle.enabled = false;
    engine.save_rule(idle).unwrap();

    // Retargeting the disabled rule must not touch the channel at all.
    let mut moved = flat("r1", RIG1, PERCENT, 55.0);
    moved.enabled = false;
    engine.save_rule(moved).unwrap();
    tick(&engine).await;

    assert_eq!(
        rig.held(RIG0, PERCENT),
        40.0,
        "the channel is still owned by R2 and nobody may write the fail-safe duty onto it"
    );
    assert_eq!(
        rig.attempts(RIG0, PERCENT),
        1,
        "no safety write may reach a channel that never lost its owner: {:?}",
        rig.requested(RIG0, PERCENT)
    );

    // And the audit agrees: the only write to that channel is R2's.
    let audit = runtime.audit().tail(40).unwrap();
    let safety_writes = audit
        .iter()
        .filter(|entry| {
            entry["kind"] == "write"
                && entry["report"]["device_id"] == RIG0
                && entry["report"]["origin"]["kind"] == "safety"
        })
        .count();
    assert_eq!(safety_writes, 0, "no handover was warranted here");
}

/// A handover queued for a channel that a *new* owner already drives must not
/// overwrite that owner. Otherwise the channel ends up at the fail-safe duty while
/// the owning rule believes — and displays — its own value.
#[tokio::test]
async fn a_handover_is_superseded_by_the_rule_that_now_owns_the_channel() {
    let (_temp, runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;
    assert_eq!(rig.held(RIG0, PERCENT), 55.0);

    // R2 is ready to take channel 0 over, but stays disabled for now so that R1 can
    // legally be retargeted away from it.
    let mut successor = flat("r2", RIG0, PERCENT, 40.0);
    successor.enabled = false;
    engine.save_rule(successor).unwrap();

    // R1 moves to fan 1, which queues the handover of fan 0 …
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    // … and before the next tick R2 takes fan 0 over.
    engine.set_rule_enabled("r2", true).unwrap();
    tick(&engine).await;

    assert_eq!(
        rig.held(RIG0, PERCENT),
        40.0,
        "the new owner's value must survive: the stale handover may not overwrite it \
         (requests: {:?})",
        rig.requested(RIG0, PERCENT)
    );
    assert_eq!(
        rig.held(RIG1, PERCENT),
        55.0,
        "and R1 drives the channel it moved to"
    );

    let audit = runtime.audit().tail(60).unwrap();
    let late_handover = audit.iter().any(|entry| {
        entry["kind"] == "write"
            && entry["report"]["device_id"] == RIG0
            && entry["report"]["origin"]["kind"] == "safety"
            && entry["report"]["applied"] == FAIL_SAFE
    });
    assert!(
        !late_handover,
        "a channel with a live owner must not be driven to the fail-safe duty"
    );
}

/// Editing the target twice before the engine ever runs must not hand over a channel
/// that has never been driven: the intermediate target owned nothing.
#[tokio::test]
async fn retargeting_twice_before_the_first_tick_never_touches_the_middle_channel() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;
    assert_eq!(rig.held(RIG0, PERCENT), 55.0);

    // A -> B -> C with no tick in between. B was never evaluated, let alone written.
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    engine.save_rule(flat("r1", RIG0, PWM, 90.0)).unwrap();
    tick(&engine).await;

    assert_eq!(
        rig.attempts(RIG1, PERCENT),
        0,
        "the channel that was targeted but never evaluated must not be written at all: {:?}",
        rig.requested(RIG1, PERCENT)
    );
    assert_eq!(
        rig.held(RIG0, PERCENT),
        FAIL_SAFE,
        "the channel the rule really controlled is handed over"
    );
    assert_eq!(rig.held(RIG0, PWM), 90.0, "and the final target is driven");
}

/// A rule that never wrote anywhere — deleted before its first evaluation — has no
/// channel to hand over.
#[tokio::test]
async fn a_rule_deleted_before_its_first_write_hands_over_nothing() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    engine.delete_rule("r1").unwrap();
    tick(&engine).await;

    assert_eq!(
        rig.attempts(RIG0, PERCENT),
        0,
        "no channel was ever controlled, so nothing may be written: {:?}",
        rig.requested(RIG0, PERCENT)
    );
}

// ---------------------------------------------------------------------------
// 3. A handover that fails must not disappear
// ---------------------------------------------------------------------------

/// The channel refuses the handover. The attempt must not vanish: it has to be
/// retried on a bounded cadence, and the failure has to stay visible.
#[tokio::test]
async fn a_refused_handover_is_retried_on_a_bounded_cadence() {
    let (_temp, runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;

    rig.set_behaviour(RIG0, PERCENT, Behaviour::Refusing);
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    tick(&engine).await;
    let after_first = rig.attempts(RIG0, PERCENT);
    assert!(after_first >= 2, "the handover must actually be attempted");

    // Many ticks in quick succession: a retry must be rate limited, not a spin.
    for _ in 0..20 {
        tick(&engine).await;
    }
    let attempts = rig.attempts(RIG0, PERCENT);
    assert!(
        attempts <= 8,
        "handover retries must stay bounded and rate limited, {attempts} attempts is a spin \
         (requests: {:?})",
        rig.requested(RIG0, PERCENT)
    );

    // The refusal must be recorded as a real attempt with the device's own reason,
    // not swallowed.
    let audit = runtime.audit().tail(80).unwrap();
    let refused = audit.iter().find(|entry| {
        entry["kind"] == "write"
            && entry["report"]["device_id"] == RIG0
            && entry["report"]["status"] == "rejected"
    });
    let refused = refused.expect("the refused handover must be audited");
    assert!(
        refused["report"]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("refused")),
        "the original reason must survive into the record: {}",
        refused["report"]
    );
}

/// Once the channel answers again, the handover completes — exactly once, and
/// without replaying anything stale afterwards.
#[tokio::test]
async fn a_recovered_channel_completes_its_handover_exactly_once() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;

    rig.set_behaviour(RIG0, PERCENT, Behaviour::Unconfirmed);
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    tick(&engine).await;
    assert_eq!(
        rig.held(RIG0, PERCENT),
        55.0,
        "an unconfirmed handover does not move the fake hardware"
    );

    // The channel comes back. Retries are rate limited (see
    // `ohm_automation::HANDOVER_RETRY_TICKS`), so recovery takes more than one tick.
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Applied);
    for _ in 0..(ohm_automation::HANDOVER_RETRY_TICKS as usize + 3) {
        tick(&engine).await;
    }
    assert_eq!(
        rig.held(RIG0, PERCENT),
        FAIL_SAFE,
        "once the channel answers, the handover must complete"
    );

    let settled = rig.attempts(RIG0, PERCENT);
    for _ in 0..10 {
        tick(&engine).await;
    }
    assert_eq!(
        rig.attempts(RIG0, PERCENT),
        settled,
        "a completed handover must not be replayed on later ticks"
    );
}

/// A retry must re-check ownership first: if a new owner has since taken the channel
/// over, coming back online must not replay the stale handover.
#[tokio::test]
async fn a_retry_does_not_replay_a_handover_whose_channel_has_a_new_owner() {
    let (_temp, _runtime, engine, rig) = session().await;
    engine.save_rule(flat("r1", RIG0, PERCENT, 55.0)).unwrap();
    tick(&engine).await;

    // The handover of fan 0 fails, so it stays queued.
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Refusing);
    engine.save_rule(flat("r1", RIG1, PERCENT, 55.0)).unwrap();
    tick(&engine).await;

    // A new owner takes fan 0 over — and can write to it, while the handover still
    // fails on its own terms.
    let mut successor = flat("r2", RIG0, PERCENT, 40.0);
    successor.enabled = false;
    engine.save_rule(successor).unwrap();
    rig.set_behaviour(RIG0, PERCENT, Behaviour::Applied);
    engine.set_rule_enabled("r2", true).unwrap();
    tick(&engine).await;
    assert_eq!(rig.held(RIG0, PERCENT), 40.0, "the new owner drives it");

    let settled = rig.attempts(RIG0, PERCENT);
    for _ in 0..10 {
        tick(&engine).await;
    }
    assert_eq!(
        rig.held(RIG0, PERCENT),
        40.0,
        "the channel belongs to R2 now; the stale handover may not overwrite it \
         (requests: {:?})",
        rig.requested(RIG0, PERCENT)
    );
    assert!(
        rig.attempts(RIG0, PERCENT) <= settled + 1,
        "and no further safety write may be attempted once the channel has an owner          (requests: {:?})",
        rig.requested(RIG0, PERCENT)
    );
}
