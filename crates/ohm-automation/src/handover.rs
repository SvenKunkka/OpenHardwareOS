//! Channels that were left behind when a rule stopped driving them.
//!
//! When a rule stops driving an output — it was retargeted, disabled, deleted, or its
//! file disappeared — that channel is left at whatever the curve last asked for. The
//! engine hands it to the fail-safe duty instead of abandoning it, but the write can
//! fail, or be accepted without ever being confirmed. That handover is therefore a
//! piece of work with a lifetime of its own, and this module is its record.
//!
//! Three rules shape the design:
//!
//! * **A failed handover must not disappear.** It is retried on a bounded cadence,
//!   with a bounded number of attempts, and whatever the last error was stays
//!   readable — along with the *first* one, because the cause of the failure is more
//!   useful than the most recent symptom.
//! * **An unfinished handover must stay visible even after its rule is gone.** The
//!   rule id is kept as data, not as a pointer.
//! * **Nothing is replayed blindly.** Ownership is re-checked before every attempt:
//!   a channel that has a live owner is not ours to touch any more.

use std::collections::VecDeque;

use ohm_core::{CapabilityId, DeviceId, RuleId};
use serde::{Deserialize, Serialize};

/// How many engine ticks apart two attempts at the same handover are allowed to be.
///
/// A tick is [`crate::TICK_INTERVAL_MS`], so this is about half a second: fast enough
/// that a briefly unreadable channel is recovered promptly, slow enough that a
/// permanently dead one is not hammered.
pub const HANDOVER_RETRY_TICKS: u64 = 5;

/// Attempts a single handover gets before it is parked as [`HandoverState::Failed`].
///
/// Parking is not giving up silently: the record stays readable and
/// `AutomationEngine::retry_failed_handovers` is the documented way to re-arm it.
pub const MAX_HANDOVER_ATTEMPTS: u32 = 5;

/// How long a channel may stay claimed-but-uncontrolled before the handover is
/// reported as failed rather than waiting silently.
///
/// A rule that targets a channel but never drives it — its sensor is gone and its
/// fallback is `hold`, its writes are refused — leaves the channel exactly as
/// unprotected as no owner at all. Waiting is right for a while (the rule may be one
/// tick away from taking over, and writing under it would fight the rule that owns
/// the channel); waiting for ever is not, because nothing would ever tell the user.
pub const HANDOVER_OWNER_WAIT_TICKS: u64 = 50;

/// How many finished handovers are kept for inspection.
const HISTORY_LIMIT: usize = 32;

/// Where a handover got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoverState {
    /// Queued, and not yet confirmed. It is retried on a bounded cadence.
    Pending,
    /// Recovered from a previous session. The record proves the channel was owed the
    /// fail-safe duty *then*; it says nothing about the device *now*. The engine
    /// verifies the device, the capability, the current owner and a fresh reading
    /// before applying the safety policy — it never replays what the old session was
    /// about to do, and never treats a stored value as confirmed.
    NeedsVerification,
    /// An enabled rule targets this channel but has not taken control of it — it has
    /// produced no output, or its writes are unconfirmed or refused. The engine
    /// deliberately does **not** write the fail-safe duty while a rule claims the
    /// channel (that would fight the rule and leave the hardware and the display
    /// disagreeing), but naming a channel is not driving it: the responsibility stays
    /// on the record, with the claimant named, until it is either taken over for real
    /// or the claim disappears.
    AwaitingOwner,
    /// The fail-safe duty was written and confirmed by the device.
    Confirmed,
    /// The attempt budget is spent. The record stays, and a user action re-arms it.
    Failed,
    /// The channel has a live owner again, so handing it to the fail-safe duty would
    /// have fought the rule that now drives it. Nothing was written.
    Superseded,
}

impl HandoverState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::NeedsVerification => "needs_verification",
            Self::AwaitingOwner => "awaiting_owner",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
            Self::Superseded => "superseded",
        }
    }

    /// Still owed: either queued, or parked after spending its attempt budget.
    ///
    /// A parked handover is unresolved business — the channel is still at whatever the
    /// abandoned curve last said — so it stays in front of the user rather than being
    /// filed away with the successes.
    pub fn is_unresolved(&self) -> bool {
        matches!(
            self,
            Self::Pending | Self::NeedsVerification | Self::AwaitingOwner | Self::Failed
        )
    }

    /// Is this a record restored from a previous session, still to be checked against
    /// the hardware as it is now?
    pub fn needs_verification(&self) -> bool {
        matches!(self, Self::NeedsVerification)
    }

    /// Settled: either the fail-safe duty was confirmed, or another rule owns the
    /// channel now.
    pub fn is_resolved(&self) -> bool {
        !self.is_unresolved()
    }

    /// May the engine write this channel on its own initiative?
    ///
    /// Only while the handover is pending. A parked one waits for an explicit
    /// [`crate::AutomationEngine::retry_failed_handovers`].
    pub fn is_attemptable(&self) -> bool {
        matches!(self, Self::Pending)
    }
}

/// One channel awaiting handover, as reported to the CLI and the app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandoverReport {
    pub device: DeviceId,
    pub capability: CapabilityId,
    /// The rule that abandoned the channel. Kept even if that rule no longer exists.
    pub from_rule: RuleId,
    /// Why the channel was abandoned, in one sentence.
    pub reason: String,
    pub state: HandoverState,
    /// Attempts made so far.
    pub attempts: u32,
    /// The first failure. The cause is more useful than the latest symptom, so it is
    /// never overwritten.
    ///
    /// The `Option`s below are omitted from the wire when absent, rather than sent as
    /// `null`: the TypeScript contract declares them optional, and a consumer checking
    /// for `undefined` would otherwise be reading a value it never receives. The
    /// repository's `WriteReport` does the same for its `applied` value.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub first_error: Option<String>,
    /// The most recent failure.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_error: Option<String>,
    pub queued_at_ms: i64,
    pub last_attempt_ms: i64,
    /// The confirmed fail-safe value, once the handover succeeded.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub confirmed_value: Option<f64>,
    /// The rule that took the channel over, when that is why it was superseded.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub superseded_by: Option<RuleId>,
    /// The enabled rule that targets the channel without driving it, when that is
    /// what the handover is waiting for. Visible on purpose: "someone else has it" is
    /// only reassuring if you can see who, and that they are actually driving.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub claimant: Option<RuleId>,
    /// Ticks spent waiting for that claimant to take control.
    pub claimed_ticks: u64,
}

impl HandoverReport {
    /// One line, for a log or a CLI table.
    pub fn summary(&self) -> String {
        let base = format!(
            "{}/{} (left by {}): {}",
            self.device,
            self.capability,
            self.from_rule,
            self.state.as_str()
        );
        match self.state {
            HandoverState::Pending => format!("{base} — {} attempt(s)", self.attempts),
            HandoverState::Confirmed => match self.confirmed_value {
                Some(value) => format!("{base} at {value:.0} %"),
                None => base,
            },
            HandoverState::Failed => format!(
                "{base} after {} attempt(s): {}",
                self.attempts,
                self.first_error.as_deref().unwrap_or("no reason recorded")
            ),
            HandoverState::Superseded => match &self.superseded_by {
                Some(rule) => format!("{base} — {rule} took it over"),
                None => format!("{base} — the channel has an owner again"),
            },
            HandoverState::NeedsVerification => format!(
                "{base} — recovered from a previous session; the device has not been checked yet"
            ),
            HandoverState::AwaitingOwner => match &self.claimant {
                Some(rule) => format!(
                    "{base} — claimed by {rule}, which has not driven it ({} tick(s) waiting)",
                    self.claimed_ticks
                ),
                None => format!("{base} — waiting for its new owner to take control"),
            },
        }
    }
}

/// A handover in progress.
#[derive(Debug, Clone)]
pub(crate) struct Handover {
    pub device: DeviceId,
    pub capability: CapabilityId,
    pub from_rule: RuleId,
    pub reason: String,
    pub state: HandoverState,
    pub attempts: u32,
    pub first_error: Option<String>,
    pub last_error: Option<String>,
    pub queued_at_ms: i64,
    pub last_attempt_ms: i64,
    pub confirmed_value: Option<f64>,
    pub superseded_by: Option<RuleId>,
    /// The rule currently claiming the channel without driving it.
    pub claimant: Option<RuleId>,
    /// Ticks spent in [`HandoverState::AwaitingOwner`].
    pub claimed_ticks: u64,
    /// `true` when the handover was parked *because* a claimant never took control,
    /// as opposed to failing on its own write. Such a handover resumes by itself once
    /// the claim disappears: the reason it was parked is gone.
    pub parked_by_claim: bool,
    /// The first tick on which this handover may be attempted.
    pub next_attempt_tick: u64,
}

impl Handover {
    pub fn report(&self) -> HandoverReport {
        HandoverReport {
            device: self.device.clone(),
            capability: self.capability.clone(),
            from_rule: self.from_rule.clone(),
            reason: self.reason.clone(),
            state: self.state,
            attempts: self.attempts,
            first_error: self.first_error.clone(),
            last_error: self.last_error.clone(),
            queued_at_ms: self.queued_at_ms,
            last_attempt_ms: self.last_attempt_ms,
            confirmed_value: self.confirmed_value,
            superseded_by: self.superseded_by.clone(),
            claimant: self.claimant.clone(),
            claimed_ticks: self.claimed_ticks,
        }
    }
}

/// The queue of channels awaiting handover, plus a bounded history of finished ones.
#[derive(Debug, Default)]
pub(crate) struct HandoverBook {
    open: Vec<Handover>,
    history: VecDeque<Handover>,
}

impl HandoverBook {
    /// Queue a channel, or fold it into the handover already waiting for it.
    ///
    /// Folding matters: a user editing a rule twice before the engine runs is one
    /// unfinished job, not two, and the attempt budget deliberately survives the edit
    /// so that an edit loop cannot buy a failing channel more writes.
    pub fn queue(
        &mut self,
        device: DeviceId,
        capability: CapabilityId,
        from_rule: RuleId,
        reason: String,
        tick: u64,
        now_ms: i64,
    ) {
        if let Some(existing) = self
            .open
            .iter_mut()
            .find(|item| item.device == device && item.capability == capability)
        {
            existing.from_rule = from_rule;
            existing.reason = reason;
            if existing.next_attempt_tick > tick {
                existing.next_attempt_tick = tick;
            }
            return;
        }
        self.open.push(Handover {
            device,
            capability,
            from_rule,
            reason,
            state: HandoverState::Pending,
            attempts: 0,
            first_error: None,
            last_error: None,
            queued_at_ms: now_ms,
            last_attempt_ms: 0,
            confirmed_value: None,
            superseded_by: None,
            claimant: None,
            claimed_ticks: 0,
            parked_by_claim: false,
            next_attempt_tick: tick,
        });
    }

    /// Handovers still owed: pending ones, and parked ones waiting for a re-arm.
    pub fn open_count(&self) -> usize {
        self.open.len()
    }

    /// Every handover on record: the unresolved ones first, then the recent history.
    pub fn reports(&self) -> Vec<HandoverReport> {
        self.open
            .iter()
            .chain(self.history.iter())
            .map(Handover::report)
            .collect()
    }

    /// Every unresolved handover: pending, awaiting an owner that never took control,
    /// and parked.
    pub fn unresolved(&self) -> &[Handover] {
        &self.open
    }

    /// Record that the channel is claimed by a rule that has not taken control of it.
    /// Returns how many ticks it has now been waiting. Writes nothing.
    pub fn mark_awaiting(
        &mut self,
        device: &DeviceId,
        capability: &CapabilityId,
        claimant: &RuleId,
        reason: String,
    ) -> u64 {
        let Some(index) = self
            .open
            .iter()
            .position(|item| &item.device == device && &item.capability == capability)
        else {
            return 0;
        };
        let item = &mut self.open[index];
        // Only a pending handover starts waiting; one that is already waiting keeps
        // counting, and a parked (failed) one is not un-parked by a claim appearing.
        if item.state == HandoverState::Pending {
            item.state = HandoverState::AwaitingOwner;
            item.claimed_ticks = 0;
        }
        if item.state == HandoverState::AwaitingOwner {
            item.claimed_ticks = item.claimed_ticks.saturating_add(1);
            item.claimant = Some(claimant.clone());
            item.reason = reason;
        }
        item.claimed_ticks
    }

    /// The claim went unmet for too long: park the handover as failed, while keeping
    /// it on the record, and remember *why* it was parked.
    pub fn park_unmet_claim(
        &mut self,
        device: &DeviceId,
        capability: &CapabilityId,
        reason: String,
    ) -> bool {
        let Some(index) = self
            .open
            .iter()
            .position(|item| &item.device == device && &item.capability == capability)
        else {
            return false;
        };
        let item = &mut self.open[index];
        if item.state != HandoverState::AwaitingOwner {
            return false;
        }
        item.state = HandoverState::Failed;
        item.parked_by_claim = true;
        item.reason = reason;
        if item.first_error.is_none() {
            item.first_error = Some(format!(
                "the rule that claimed this channel never took control of it ({} tick(s))",
                item.claimed_ticks
            ));
        }
        item.last_error = item.first_error.clone();
        true
    }

    /// The claim is gone. A handover parked *because of* that claim resumes by itself
    /// — the reason it stopped no longer exists — and comes back as pending.
    pub fn release_claim(
        &mut self,
        device: &DeviceId,
        capability: &CapabilityId,
        tick: u64,
    ) -> bool {
        let Some(index) = self
            .open
            .iter()
            .position(|item| &item.device == device && &item.capability == capability)
        else {
            return false;
        };
        let item = &mut self.open[index];
        let was_awaiting = item.state == HandoverState::AwaitingOwner;
        let was_claim_parked = item.state == HandoverState::Failed && item.parked_by_claim;
        if !was_awaiting && !was_claim_parked {
            return false;
        }
        item.state = HandoverState::Pending;
        item.claimant = None;
        item.claimed_ticks = 0;
        item.parked_by_claim = false;
        // A resumed handover gets a fresh budget: the previous attempts were not
        // failures of the channel, they were the channel being left alone.
        item.attempts = 0;
        item.next_attempt_tick = tick;
        true
    }

    /// Put a handover recovered from disk back on the record.
    ///
    /// It arrives as [`HandoverState::NeedsVerification`] whatever it was when it was
    /// saved: the state described the *old* session's view of the device, and the only
    /// honest starting point for this one is "not checked yet".
    pub fn restore(&mut self, item: Handover, now_ms: i64) {
        let mut item = item;
        item.state = HandoverState::NeedsVerification;
        item.queued_at_ms = item.queued_at_ms.min(now_ms);
        self.open.push(item);
    }

    /// A recovered item that has been checked and now behaves like a fresh one.
    pub fn verified(&mut self, device: &DeviceId, capability: &CapabilityId, tick: u64) -> bool {
        let Some(index) = self
            .open
            .iter()
            .position(|item| &item.device == device && &item.capability == capability)
        else {
            return false;
        };
        let item = &mut self.open[index];
        if !item.state.needs_verification() {
            return false;
        }
        item.state = HandoverState::Pending;
        // A verified handover starts with a fresh attempt budget: the attempts it made
        // in the previous session say nothing about the device now, and carrying them
        // over could park a healthy channel as failed before it was even tried.
        item.attempts = 0;
        item.next_attempt_tick = tick;
        true
    }

    /// Mark a recovered item as failed, with the reason verification failed.
    pub fn fail_recovered(
        &mut self,
        device: &DeviceId,
        capability: &CapabilityId,
        reason: String,
    ) -> bool {
        let Some(index) = self
            .open
            .iter()
            .position(|item| &item.device == device && &item.capability == capability)
        else {
            return false;
        };
        let item = &mut self.open[index];
        if item.state != HandoverState::NeedsVerification {
            return false;
        }
        item.state = HandoverState::Failed;
        item.parked_by_claim = false;
        if item.first_error.is_none() {
            item.first_error = Some(reason.clone());
        }
        item.last_error = Some(reason.clone());
        item.reason = reason;
        true
    }

    /// Is this channel due an attempt?
    pub fn is_due(&self, device: &DeviceId, capability: &CapabilityId, tick: u64) -> bool {
        self.find(device, capability)
            .is_some_and(|item| item.state.is_attemptable() && tick >= item.next_attempt_tick)
    }

    pub(crate) fn find(&self, device: &DeviceId, capability: &CapabilityId) -> Option<&Handover> {
        self.open
            .iter()
            .find(|item| &item.device == device && &item.capability == capability)
    }

    /// Record the outcome of an attempt. Returns the state the handover is in now:
    /// `Pending` while the attempt budget lasts, `Failed` once it is spent.
    pub fn record_attempt(
        &mut self,
        device: &DeviceId,
        capability: &CapabilityId,
        error: Option<String>,
        tick: u64,
        now_ms: i64,
    ) -> HandoverState {
        let Some(index) = self
            .open
            .iter()
            .position(|item| &item.device == device && &item.capability == capability)
        else {
            return HandoverState::Superseded;
        };
        let exhausted;
        {
            let item = &mut self.open[index];
            item.attempts = item.attempts.saturating_add(1);
            item.last_attempt_ms = now_ms;
            item.next_attempt_tick = tick + HANDOVER_RETRY_TICKS;
            if let Some(error) = error {
                // The first failure is the cause; later ones are symptoms.
                if item.first_error.is_none() {
                    item.first_error = Some(error.clone());
                }
                item.last_error = Some(error);
            }
            exhausted = item.attempts >= MAX_HANDOVER_ATTEMPTS;
            if exhausted {
                // Parked, not filed away: the channel is still unprotected, and the
                // user has to be able to see that and ask for another attempt.
                item.state = HandoverState::Failed;
            }
        }
        let _ = index;
        if exhausted {
            HandoverState::Failed
        } else {
            HandoverState::Pending
        }
    }

    /// Mark the handover finished: confirmed at `value`.
    pub fn confirm(
        &mut self,
        device: &DeviceId,
        capability: &CapabilityId,
        value: Option<f64>,
        now_ms: i64,
    ) -> bool {
        let Some(index) = self
            .open
            .iter()
            .position(|item| &item.device == device && &item.capability == capability)
        else {
            return false;
        };
        {
            let item = &mut self.open[index];
            item.state = HandoverState::Confirmed;
            item.confirmed_value = value;
            item.attempts = item.attempts.saturating_add(1);
            item.last_attempt_ms = now_ms;
            item.last_error = None;
        }
        self.finish(index);
        true
    }

    /// The channel has a live owner again: drop the handover without writing.
    pub fn supersede(
        &mut self,
        device: &DeviceId,
        capability: &CapabilityId,
        by: Option<RuleId>,
    ) -> bool {
        let Some(index) = self
            .open
            .iter()
            .position(|item| &item.device == device && &item.capability == capability)
        else {
            return false;
        };
        self.open[index].state = HandoverState::Superseded;
        self.open[index].superseded_by = by;
        self.finish(index);
        true
    }

    /// Re-arm every failed handover. Returns how many were re-armed.
    pub fn retry_failed(&mut self, tick: u64) -> usize {
        let mut rearmed = 0;
        for item in self.open.iter_mut() {
            if item.state == HandoverState::Failed {
                item.state = HandoverState::Pending;
                item.attempts = 0;
                item.next_attempt_tick = tick;
                item.last_error = None;
                rearmed += 1;
            }
        }
        rearmed
    }

    /// Re-arm the failed handover for exactly one channel.
    ///
    /// Returns `false` when that channel has no failed handover, so a caller can tell
    /// "there was nothing to re-arm here" from "it worked" — and cannot silently re-arm
    /// something else by accident.
    pub fn retry_failed_one(
        &mut self,
        device: &DeviceId,
        capability: &CapabilityId,
        tick: u64,
    ) -> bool {
        let Some(item) = self
            .open
            .iter_mut()
            .find(|item| &item.device == device && &item.capability == capability)
        else {
            return false;
        };
        if item.state != HandoverState::Failed {
            return false;
        }
        item.state = HandoverState::Pending;
        item.attempts = 0;
        item.next_attempt_tick = tick;
        item.last_error = None;
        true
    }

    /// Move a resolved item out of the queue and into the bounded history.
    ///
    /// Unresolved items — pending, and parked after a spent budget — stay in the queue
    /// where they are visible and, for the parked ones, re-armable.
    fn finish(&mut self, index: usize) {
        let item = self.open.remove(index);
        if !item.state.is_resolved() {
            self.open.insert(index, item);
            return;
        }
        self.history.push_back(item);
        while self.history.len() > HISTORY_LIMIT {
            self.history.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (DeviceId, CapabilityId, RuleId) {
        (
            DeviceId::new_unchecked("fan.rig.0"),
            CapabilityId::new_unchecked("fan.speed_percent"),
            RuleId::new_unchecked("r1"),
        )
    }

    #[test]
    fn queueing_the_same_channel_twice_keeps_one_item_and_its_budget() {
        let mut book = HandoverBook::default();
        let (device, capability, rule) = ids();
        book.queue(
            device.clone(),
            capability.clone(),
            rule.clone(),
            "first".into(),
            0,
            0,
        );
        book.record_attempt(&device, &capability, Some("refused".into()), 0, 0);
        book.queue(
            device.clone(),
            capability.clone(),
            rule.clone(),
            "second".into(),
            3,
            3,
        );

        assert_eq!(book.open_count(), 1, "one channel is one unfinished job");
        let item = &book.unresolved()[0];
        assert_eq!(
            item.attempts, 1,
            "an edit must not buy the channel more writes"
        );
        assert_eq!(item.reason, "second");
        assert_eq!(item.first_error.as_deref(), Some("refused"));
    }

    #[test]
    fn the_attempt_budget_is_bounded_and_keeps_the_first_error() {
        let mut book = HandoverBook::default();
        let (device, capability, rule) = ids();
        book.queue(
            device.clone(),
            capability.clone(),
            rule,
            "reason".into(),
            0,
            0,
        );
        for attempt in 0..MAX_HANDOVER_ATTEMPTS {
            book.record_attempt(
                &device,
                &capability,
                Some(format!("failure {attempt}")),
                attempt as u64,
                attempt as i64,
            );
        }
        assert_eq!(book.open_count(), 1, "the failed handover stays visible");
        let report = &book.reports()[0];
        assert_eq!(report.state, HandoverState::Failed);
        assert_eq!(report.attempts, MAX_HANDOVER_ATTEMPTS);
        assert_eq!(
            report.first_error.as_deref(),
            Some("failure 0"),
            "the cause must survive the later symptoms"
        );
        assert_eq!(report.last_error.as_deref(), Some("failure 4"));

        assert_eq!(book.retry_failed(100), 1);
        let report = &book.reports()[0];
        assert_eq!(report.state, HandoverState::Pending);
        assert_eq!(
            report.attempts, 0,
            "a user-armed retry starts a fresh budget"
        );
        assert!(book.is_due(&device, &capability, 100));
    }

    #[test]
    fn the_scoped_rearm_touches_one_channel_and_nothing_else() {
        let mut book = HandoverBook::default();
        let fan0 = DeviceId::new_unchecked("fan.mock.0");
        let fan1 = DeviceId::new_unchecked("fan.mock.1");
        let percent = CapabilityId::new_unchecked("fan.speed_percent");
        for (device, rule) in [(&fan0, "r0"), (&fan1, "r1")] {
            book.queue(
                device.clone(),
                percent.clone(),
                RuleId::new_unchecked(rule),
                "reason".into(),
                0,
                0,
            );
            for attempt in 0..MAX_HANDOVER_ATTEMPTS {
                book.record_attempt(device, &percent, Some("refused".into()), attempt as u64, 0);
            }
        }
        assert_eq!(book.reports().len(), 2, "both channels are parked");

        assert!(book.retry_failed_one(&fan1, &percent, 100));
        let rearmed: Vec<_> = book
            .reports()
            .iter()
            .map(|report| (report.device.to_string(), report.state))
            .collect();
        assert!(
            rearmed.contains(&("fan.mock.1".to_string(), HandoverState::Pending)),
            "the named channel is re-armed: {rearmed:?}"
        );
        assert!(
            rearmed.contains(&("fan.mock.0".to_string(), HandoverState::Failed)),
            "and the other one is untouched: {rearmed:?}"
        );

        assert!(
            !book.retry_failed_one(&DeviceId::new_unchecked("fan.mock.9"), &percent, 100),
            "a channel with no failed handover reports that there was nothing to do"
        );
    }

    #[test]
    fn a_confirmed_handover_leaves_the_queue_but_not_the_record() {
        let mut book = HandoverBook::default();
        let (device, capability, rule) = ids();
        book.queue(
            device.clone(),
            capability.clone(),
            rule,
            "reason".into(),
            0,
            0,
        );
        assert!(book.confirm(&device, &capability, Some(70.0), 10));
        assert_eq!(book.open_count(), 0);
        let report = &book.reports()[0];
        assert_eq!(report.state, HandoverState::Confirmed);
        assert_eq!(report.confirmed_value, Some(70.0));
        assert!(!book.is_due(&device, &capability, 999));
    }

    #[test]
    fn attempts_are_rate_limited_by_ticks() {
        let mut book = HandoverBook::default();
        let (device, capability, rule) = ids();
        book.queue(
            device.clone(),
            capability.clone(),
            rule,
            "reason".into(),
            0,
            0,
        );
        assert!(book.is_due(&device, &capability, 0));
        book.record_attempt(&device, &capability, Some("nope".into()), 0, 0);
        assert!(
            !book.is_due(&device, &capability, HANDOVER_RETRY_TICKS - 1),
            "a retry may not happen on the very next tick"
        );
        assert!(book.is_due(&device, &capability, HANDOVER_RETRY_TICKS));
    }

    #[test]
    fn the_history_is_bounded() {
        let mut book = HandoverBook::default();
        let (device, capability, rule) = ids();
        for _ in 0..(HISTORY_LIMIT + 10) {
            book.queue(
                device.clone(),
                capability.clone(),
                rule.clone(),
                "r".into(),
                0,
                0,
            );
            book.confirm(&device, &capability, Some(70.0), 0);
        }
        assert_eq!(book.open_count(), 0);
        assert_eq!(
            book.reports().len(),
            HISTORY_LIMIT,
            "a long-running engine must not accumulate records forever"
        );
    }

    #[test]
    fn a_superseded_channel_records_who_took_it_over() {
        let mut book = HandoverBook::default();
        let (device, capability, rule) = ids();
        book.queue(
            device.clone(),
            capability.clone(),
            rule,
            "reason".into(),
            0,
            0,
        );
        assert!(book.supersede(&device, &capability, Some(RuleId::new_unchecked("r2"))));
        let report = &book.reports()[0];
        assert_eq!(report.state, HandoverState::Superseded);
        assert_eq!(
            report.superseded_by.as_ref().map(RuleId::as_str),
            Some("r2")
        );
        assert_eq!(report.attempts, 0, "and nothing was written");
    }
}
