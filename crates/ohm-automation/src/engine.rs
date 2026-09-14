//! The automation engine: it turns rules into hardware writes.
//!
//! ```text
//!  every ~100 ms
//!      │
//!      ├─ rule due? ──no──▶ skip
//!      │
//!      ├─ read source(s) from the runtime state store
//!      ├─ evaluate (curve + hysteresis + deadband + fallback)
//!      ├─ write through Runtime::write_value  (range → safety → audit)
//!      └─ record the outcome for the UI
//! ```
//!
//! Two invariants:
//!
//! * the engine **only** writes through [`Runtime::write_value`], so every rule
//!   obeys the same validation, safety policy and audit trail as a manual write;
//! * an error in one rule never affects another, and a rule that cannot reach
//!   its sensor falls back instead of leaving a fan at a stale speed.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use ohm_adapter_api::WriteStatus;
use ohm_core::{CapabilityId, DeviceId, OhmError, Result, RuleId};
use ohm_device_model::Value;
use ohm_runtime::{Runtime, WriteOrigin};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::evaluator::{
    ControlHold, EvaluationInput, RuleOutcome, RuleState, RuleStatus, evaluate,
};
use crate::rule::{Aggregate, FallbackAction, OtherwiseAction, Rule};
use crate::store::{LoadReport, RuleStore};

/// How many unconfirmed writes in a row are tolerated before the rule's
/// write-failure policy runs.
///
/// A channel that accepts requests without ever confirming them is a control
/// failure, but retrying is also the only way to notice it has recovered, so the
/// retries are bounded and then the safety fallback takes over.
pub const MAX_CONSECUTIVE_UNCONFIRMED: u32 = 3;

/// How often the engine wakes up to look for due rules.
pub const TICK_INTERVAL_MS: u64 = 100;

/// Why a rule was rejected before it could run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleCheck {
    /// Hard problems: the rule must not be saved.
    pub errors: Vec<String>,
    /// Soft problems: the rule will run, but probably not as intended.
    pub warnings: Vec<String>,
}

impl RuleCheck {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn into_result(self) -> Result<()> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(OhmError::Automation(self.errors.join("; ")))
        }
    }
}

/// Two enabled rules fighting over one output.
///
/// The MVP has no arbitration, and pretending otherwise was worse than saying so:
/// the runtime used to let the lower-priority rule overwrite the higher-priority
/// one *within the same tick*, which is not a policy anyone can reason about.
/// Instead, a target may be owned by at most one enabled rule, enforced when a
/// rule is created, imported, enabled — and again at tick time, so a
/// hand-edited `rules/*.yaml` cannot bypass it either.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleConflict {
    /// The rule that owns the target (kept enabled).
    pub owner_id: RuleId,
    pub owner_name: String,
    /// The rule that was refused (or disabled in memory) because of the clash.
    pub blocked_id: RuleId,
    pub blocked_name: String,
    /// The contested output.
    pub target: String,
    /// What the runtime did about it.
    pub resolution: String,
}

impl RuleConflict {
    /// One line for the UI and the logs.
    pub fn message(&self) -> String {
        format!(
            "`{}` and `{}` both drive {}. {}",
            self.owner_name, self.blocked_name, self.target, self.resolution
        )
    }
}

/// What changed about a rule, expressed in the terms that matter to its control
/// memory.
///
/// The distinction is deliberate: a change to what the rule *means* invalidates
/// cached state (a value measured for a different output or a different sensor is
/// not evidence about this one), while a metadata-only change must not interrupt
/// control at all.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RuleChange {
    /// The channel the rule used to drive, when it changed.
    ///
    /// The *whole* channel — device **and** capability. Comparing only the device
    /// missed the case "same fan, different control channel", which left the old
    /// channel unmanaged and let the new one inherit the old one's value.
    previous_target: Option<(DeviceId, CapabilityId)>,
    /// The rule was switched on or off.
    enabled_changed: bool,
    source_changed: bool,
    condition_changed: bool,
    /// The mapping itself changed: curve, hysteresis, deadband, limits or fallback.
    control_changed: bool,
}

impl RuleChange {
    fn between(previous: &Rule, next: &Rule) -> Self {
        let previous_channel = (
            previous.target.device.clone(),
            previous.target.capability.clone(),
        );
        let next_channel = (next.target.device.clone(), next.target.capability.clone());
        Self {
            previous_target: (previous_channel != next_channel).then_some(previous_channel),
            enabled_changed: previous.enabled != next.enabled,
            source_changed: previous.source != next.source,
            condition_changed: previous.when != next.when,
            control_changed: previous.curve != next.curve
                || previous.hysteresis != next.hysteresis
                || previous.deadband != next.deadband
                || previous.min_output != next.min_output
                || previous.max_output != next.max_output
                || previous.fallback != next.fallback,
        }
    }

    /// Is the rule giving up the channel it was driving?
    ///
    /// A retarget does, and so does a disable: in both cases the channel loses the
    /// rule that was moving it, and whatever value it is left at is no longer
    /// anyone's decision. A change to the source, the condition or the mapping does
    /// **not**: the same rule still owns the same channel and keeps driving it.
    fn leaves_control(&self) -> bool {
        self.previous_target.is_some() || self.enabled_changed
    }

    /// Does anything need invalidating?
    fn is_semantic(&self) -> bool {
        self.leaves_control()
            || self.source_changed
            || self.condition_changed
            || self.control_changed
    }

    /// One line for the log, or `None` when only metadata moved.
    fn summary(&self) -> Option<String> {
        if !self.is_semantic() {
            return None;
        }
        let mut parts = Vec::new();
        if let Some((device, capability)) = &self.previous_target {
            parts.push(format!("target changed from {device}/{capability}"));
        }
        if self.enabled_changed {
            parts.push("enabled state changed".to_string());
        }
        if self.source_changed {
            parts.push("source changed".to_string());
        }
        if self.condition_changed {
            parts.push("condition changed".to_string());
        }
        if self.control_changed {
            parts.push("mapping changed".to_string());
        }
        Some(parts.join(", "))
    }
}

/// Engine counters, shown in Diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AutomationStats {
    pub rules: usize,
    pub enabled_rules: usize,
    pub ticks: u64,
    pub evaluations: u64,
    pub writes: u64,
    pub skipped: u64,
    pub fallbacks: u64,
    pub failures: u64,
    pub last_tick_ms: i64,
    /// Why the unresolved control responsibility could not be written down, when it
    /// could not. Surfaced here so a front-end can show it: losing that file means
    /// losing track of a channel nobody is protecting.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub persistence_error: Option<String>,
}

#[derive(Debug)]
struct EngineInner {
    runtime: Runtime,
    store: RuleStore,
    rules: RwLock<Vec<Rule>>,
    states: Mutex<HashMap<RuleId, RuleState>>,
    stop_tx: watch::Sender<bool>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    running: AtomicBool,
    ticks: AtomicU64,
    last_tick_ms: AtomicI64,
    /// Unresolved-by-the-user conflicts found when loading rule files.
    conflicts: RwLock<Vec<RuleConflict>>,
    /// Rule files that loaded but had to be adjusted in memory (a `release`
    /// fallback substituted with `safe_default`). Surfaced so the substitution is
    /// visible instead of silent.
    compatibility_notes: RwLock<Vec<crate::store::RuleFileNote>>,
    /// Outputs a rule stopped driving and that still need handing over.
    ///
    /// `save_rule` is synchronous, so it cannot await the handover write itself;
    /// queuing it here means every save path — the form, the CLI, a reloaded file, a
    /// deletion — gets the same treatment, performed at the start of the next tick.
    /// The book also keeps a bounded history, because a handover that failed is a
    /// fact the user needs to be able to see even after its rule is gone.
    handovers: Mutex<crate::handover::HandoverBook>,
    /// Where unresolved control responsibility is written down.
    recovery: crate::recovery::RecoveryStore,
    /// Set when the last attempt to write that record failed. Never cleared silently:
    /// losing this file means losing track of a channel nobody is protecting, so the
    /// failure has to be visible to the user, not just to the log.
    persistence_error: Mutex<Option<String>>,
    /// `true` when the record needs writing before the process could die.
    dirty: AtomicBool,
    /// Recovery happens once, the first time the rules are known.
    recovered: AtomicBool,
}

/// The automation engine.
#[derive(Debug, Clone)]
pub struct AutomationEngine {
    inner: Arc<EngineInner>,
}

impl AutomationEngine {
    /// Build an engine for a runtime, persisting rules in `store`.
    pub fn new(runtime: Runtime, store: RuleStore) -> Self {
        let (stop_tx, _stop_rx) = watch::channel(false);
        let recovery = crate::recovery::RecoveryStore::from_paths(runtime.paths());
        Self {
            inner: Arc::new(EngineInner {
                runtime,
                store,
                rules: RwLock::new(Vec::new()),
                states: Mutex::new(HashMap::new()),
                stop_tx,
                tasks: Mutex::new(Vec::new()),
                running: AtomicBool::new(false),
                ticks: AtomicU64::new(0),
                last_tick_ms: AtomicI64::new(0),
                conflicts: RwLock::new(Vec::new()),
                compatibility_notes: RwLock::new(Vec::new()),
                handovers: Mutex::new(crate::handover::HandoverBook::default()),
                recovery,
                persistence_error: Mutex::new(None),
                dirty: AtomicBool::new(false),
                recovered: AtomicBool::new(false),
            }),
        }
    }

    /// Build an engine storing rules under the runtime's config directory.
    pub fn from_runtime(runtime: Runtime) -> Self {
        let store = RuleStore::from_paths(runtime.paths());
        Self::new(runtime, store)
    }

    pub fn runtime(&self) -> &Runtime {
        &self.inner.runtime
    }

    pub fn store(&self) -> &RuleStore {
        &self.inner.store
    }

    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::Relaxed)
    }

    /// Load rules from disk, replacing the in-memory set.
    pub fn load_rules(&self) -> Result<LoadReport> {
        let report = self.inner.store.load_report();
        // The rules as they were, so a file that changed on disk is treated exactly
        // like an edit through the form.
        let previous: Vec<Rule> = self.rules();
        // Second: what each of them was actually driving. Captured now because the
        // reload below drops the state of every rule whose file is gone — and that
        // state is the only record of the channel that now needs handing over.
        let previous_holds: Vec<(RuleId, ControlHold)> = {
            let states = self.inner.states.lock();
            previous
                .iter()
                .filter_map(|rule| {
                    states
                        .get(&rule.id)
                        .and_then(|state| state.control.clone())
                        .map(|hold| (rule.id.clone(), hold))
                })
                .collect()
        };
        {
            let mut rules = self.inner.rules.write();
            *rules = report.rules.clone();
            rules.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
        }
        {
            let mut states = self.inner.states.lock();
            states.retain(|id, _| report.rules.iter().any(|r| &r.id == id));
            for rule in &report.rules {
                states
                    .entry(rule.id.clone())
                    .or_insert_with(|| RuleState::new(rule.id.clone()));
            }
        }
        if !report.is_clean() {
            self.inner.runtime.log(
                "warn",
                format!("{} rule file(s) could not be loaded", report.errors.len()),
            );
        }
        // A rule whose file changed meaningfully gets the same treatment as an edited
        // one: the old output is handed over and stale state is dropped.
        for loaded in &report.rules {
            if let Some(before) = previous.iter().find(|rule| rule.id == loaded.id) {
                let change = RuleChange::between(before, loaded);
                if change.is_semantic() {
                    self.apply_rule_change(before, &change);
                }
            }
        }
        // A rule whose file is gone has left control just as surely as a deleted one.
        // Its state was dropped above, so the hold captured before the reload is what
        // says which channel is now unowned.
        for (id, hold) in previous_holds {
            if report.rules.iter().any(|rule| rule.id == id) {
                continue;
            }
            self.hand_over(id, Some(hold), None, "was removed from disk");
        }

        *self.inner.compatibility_notes.write() = report.notes.clone();
        for note in &report.notes {
            self.inner.runtime.log(
                "warn",
                format!("automation rule `{}`: {}", note.rule_id, note.message),
            );
            self.inner.runtime.publish_automation(
                Some(note.rule_id.clone()),
                "rule_compatibility_note",
                note.message.clone(),
            );
        }
        self.resolve_loaded_conflicts();
        // Once, after the rules are known: an issued-but-unconfirmed write made by a
        // rule that still drives the channel needs no recovery (the rule will settle it
        // by writing again), and that question cannot be answered before the rules load.
        self.flush_control_state();
        if !self.inner.recovered.swap(true, Ordering::SeqCst)
            && let Err(error) = self.recover_control_state()
        {
            // Reported, never ignored: the record exists because a channel may be
            // unprotected, so nobody may quietly lose it.
            self.inner.runtime.publish_automation(
                None,
                "control_state_recovery_failed",
                error.to_string(),
            );
        }
        Ok(report)
    }

    /// Start the evaluation loop.
    pub async fn start(&self) -> Result<()> {
        if self.inner.running.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.load_rules()?;

        if self.inner.runtime.settings().automation_enabled {
            tracing::info!(rules = self.rules().len(), "automation engine started");
            self.inner.runtime.publish_automation(
                None,
                "engine_started",
                format!("{} rule(s) loaded", self.rules().len()),
            );
        }

        let inner = Arc::clone(&self.inner);
        let mut stop = inner.stop_tx.subscribe();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(TICK_INTERVAL_MS)) => {}
                    _ = stop.changed() => break,
                }
                if !inner.running.load(Ordering::Relaxed) {
                    break;
                }
                let engine = AutomationEngine {
                    inner: Arc::clone(&inner),
                };
                engine.tick().await;
            }
            tracing::debug!("automation loop stopped");
        });
        self.inner.tasks.lock().push(handle);
        Ok(())
    }

    /// Stop the loop. Rules keep their state so a restart resumes smoothly.
    pub async fn stop(&self) {
        self.inner.running.store(false, Ordering::SeqCst);
        let _ = self.inner.stop_tx.send(true);
        let handles: Vec<JoinHandle<()>> = std::mem::take(&mut *self.inner.tasks.lock());
        for handle in handles {
            let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
        }
        // Last chance to record what is still owed before the process goes away.
        if !self.inner.handovers.lock().unresolved().is_empty() {
            self.inner.dirty.store(true, Ordering::SeqCst);
        }
        self.flush_control_state();
        self.inner
            .runtime
            .publish_automation(None, "engine_stopped", "automation loop stopped");
    }

    /// Evaluate every rule that is due.
    pub async fn tick(&self) -> Vec<RuleOutcome> {
        self.tick_inner(false).await
    }

    /// Evaluate every enabled rule *now*, ignoring `update_interval_ms`.
    ///
    /// Used by the CLI's time-compressed simulator and by tests; the scheduled
    /// [`AutomationEngine::tick`] is what runs in the app.
    pub async fn tick_force(&self) -> Vec<RuleOutcome> {
        self.tick_inner(true).await
    }

    /// Perform the handovers queued by rule edits.
    ///
    /// Runs before any rule is evaluated, so an output nobody drives any more is safe
    /// before the engine starts moving the one that replaced it.
    ///
    /// Two things are re-checked *before every attempt*, not decided once at queue
    /// time:
    ///
    /// * **Ownership.** A channel that an enabled rule targets again belongs to that
    ///   rule, and driving it to the fail-safe duty would fight its real owner —
    ///   leaving the hardware at one value while the owner displays another. Such a
    ///   handover is superseded and nothing is written.
    /// * **Cadence.** A failed handover is retried every
    ///   [`crate::HANDOVER_RETRY_TICKS`] ticks and no more, so a channel that is
    ///   simply gone cannot become a write loop. The attempt budget is bounded too;
    ///   spending it parks the handover as `Failed` — on record, and re-armable by
    ///   the user — instead of dropping it.
    async fn perform_pending_handovers(&self, tick: u64, now: i64) {
        // Recovered items are checked against the hardware as it is *now* before the
        // safety policy is applied to them. Nothing from the previous session is
        // replayed: the device has to be present, the channel writable, and the data
        // fresh, or the item is reported as failed instead of acted on.
        self.verify_recovered_state();

        let duty = self.inner.runtime.settings().safety.fail_safe_duty_percent;
        // Unresolved handovers only: pending, waiting for a claimant, and parked.
        // Resolved ones have been filed away and must never be revisited.
        let pending: Vec<crate::handover::Handover> = self
            .inner
            .handovers
            .lock()
            .unresolved()
            .iter()
            .filter(|item| item.state.is_unresolved())
            .cloned()
            .collect();

        // Anything this loop decides changes the record: an item that becomes resolved
        // must disappear from it, or the next session would replay work already done.
        if !pending.is_empty() {
            self.inner.dirty.store(true, Ordering::SeqCst);
        }

        for handover in pending {
            let device = &handover.device;
            let capability = &handover.capability;

            // 1. Has an enabled rule *taken the channel over*? That means it has
            //    driven it and the device confirmed the value — naming the channel as
            //    a target is not evidence, and treating it as evidence is how a
            //    channel ends up abandoned at an old duty while nobody notices.
            if let Some(owner) = self.confirmed_owner_of(device, capability) {
                let superseded =
                    self.inner
                        .handovers
                        .lock()
                        .supersede(device, capability, Some(owner.clone()));
                if superseded {
                    let message = format!(
                        "{device}/{capability} was taken over by {owner}, which has confirmed a write \
                         to it; the handover queued by {} is closed",
                        handover.from_rule
                    );
                    self.inner.runtime.publish_automation(
                        Some(handover.from_rule.clone()),
                        "rule_handover_superseded",
                        message.clone(),
                    );
                    self.inner.runtime.log("info", message);
                }
                continue;
            }

            // 2. Is the channel *claimed* by an enabled rule that has not driven it?
            //    Do not write — that would fight the rule that owns the channel and
            //    leave the hardware disagreeing with what the rule reports — but do not
            //    pretend the channel is safe either. Keep it owed, name the claimant,
            //    and let the wait be bounded.
            if let Some(claimant) = self.claimant_of(device, capability) {
                let status = self
                    .owner_status(&claimant)
                    .unwrap_or_else(|| "not evaluated yet".to_string());
                let reason = format!(
                    "{device}/{capability} is claimed by rule `{claimant}` ({status}) but not driven \
                     by it; the channel is still at the value left by `{}`",
                    handover.from_rule
                );
                let waited = self.inner.handovers.lock().mark_awaiting(
                    device,
                    capability,
                    &claimant,
                    reason.clone(),
                );
                if waited > crate::handover::HANDOVER_OWNER_WAIT_TICKS {
                    let parked = self.inner.handovers.lock().park_unmet_claim(
                        device,
                        capability,
                        format!(
                            "rule `{claimant}` has claimed {device}/{capability} for {waited} ticks \
                             without driving it; the channel is unprotected. Fix or disable that \
                             rule, then re-arm this handover."
                        ),
                    );
                    if parked {
                        let message = format!(
                            "gave up waiting for `{claimant}` to take over {device}/{capability} \
                             after {waited} ticks; the channel is still unprotected"
                        );
                        self.inner.runtime.log("warn", message.clone());
                        self.inner.runtime.publish_automation(
                            Some(handover.from_rule.clone()),
                            "rule_handover_failed",
                            message,
                        );
                    }
                } else if waited == 1 {
                    // One notice when the wait starts, not one per tick.
                    self.inner.runtime.log("info", reason.clone());
                    self.inner.runtime.publish_automation(
                        Some(handover.from_rule.clone()),
                        "rule_handover_awaiting_owner",
                        reason,
                    );
                }
                continue;
            }

            // 3. No claimant. A handover parked because of a claim resumes by itself —
            //    the reason it stopped is gone — while one parked by its own failed
            //    writes waits for an explicit user re-arm.
            if handover.state != crate::handover::HandoverState::Pending {
                let resumed = self
                    .inner
                    .handovers
                    .lock()
                    .release_claim(device, capability, tick);
                if !resumed {
                    continue;
                }
                self.inner.runtime.log(
                    "info",
                    format!("the claim on {device}/{capability} is gone; its handover resumes"),
                );
            }

            // 4. Is this attempt due? A retry may not happen on the very next tick.
            if !self.inner.handovers.lock().is_due(device, capability, tick) {
                continue;
            }

            // 5. Attempt it, and record what actually happened rather than what was
            //    intended.
            let origin = WriteOrigin::Safety {
                reason: handover.reason.clone(),
            };
            let outcome = self
                .inner
                .runtime
                .write_value(
                    device.as_str(),
                    capability.as_str(),
                    Value::Number(duty),
                    origin,
                )
                .await;

            let (confirmed, error) = match &outcome {
                Ok(report) => match report.status {
                    WriteStatus::Applied | WriteStatus::Simulated => {
                        (report.applied.as_ref().and_then(Value::as_f64), None)
                    }
                    WriteStatus::Unconfirmed => (
                        None,
                        Some(format!(
                            "accepted but not confirmed: {}",
                            report.detail.as_deref().unwrap_or("no reason given")
                        )),
                    ),
                    WriteStatus::Rejected => (
                        None,
                        Some(format!(
                            "the device refused the fail-safe duty: {}",
                            report.detail.as_deref().unwrap_or("no reason given")
                        )),
                    ),
                },
                Err(error) => (None, Some(error.to_string())),
            };

            let mut book = self.inner.handovers.lock();
            match confirmed {
                Some(value) => {
                    book.confirm(device, capability, Some(value), now);
                    drop(book);
                    let message = format!(
                        "{device}/{capability} set to {value:.0} % after {} stopped driving it",
                        handover.from_rule
                    );
                    self.inner.runtime.publish_automation(
                        Some(handover.from_rule.clone()),
                        "rule_handover",
                        message.clone(),
                    );
                    self.inner.runtime.log("info", message);
                }
                None => {
                    // An attempt was made and did not settle the channel: the original
                    // reason for the handover and the reason it failed are both kept.
                    let error = error.unwrap_or_else(|| "the write was not confirmed".into());
                    let state =
                        book.record_attempt(device, capability, Some(error.clone()), tick, now);
                    drop(book);
                    let message = format!(
                        "could not hand {device}/{capability} over to the {duty:.0} % fail-safe duty: \
                         {error}"
                    );
                    self.inner.runtime.log("warn", message.clone());
                    self.inner.runtime.publish_automation(
                        Some(handover.from_rule.clone()),
                        if state == crate::handover::HandoverState::Failed {
                            "rule_handover_failed"
                        } else {
                            "rule_handover_retry"
                        },
                        message,
                    );
                }
            }
        }
    }

    /// Check every recovered item against the device as it is now.
    ///
    /// Public because a diagnostic that only reads — `ohm-cli handovers` — still needs
    /// the record it prints to be true: an item stuck at "not checked yet" because
    /// nothing ever ticks would be a report about the file, not about the machine.
    /// Verification writes nothing to hardware; it only resolves channels and marks
    /// what it found.
    ///
    /// Deliberately not a replay of what the previous session was about to do: an item
    /// becomes an ordinary pending handover (subject to the normal ownership checks and
    /// the safety policy) only once its channel is known to exist and be writable. If
    /// the channel is gone, the item fails *visibly* with the reason, because a
    /// responsibility that cannot be discharged is a fact the user must see.
    pub fn verify_recovered_state(&self) {
        let items: Vec<(DeviceId, CapabilityId)> = self
            .inner
            .handovers
            .lock()
            .unresolved()
            .iter()
            .filter(|item| item.state.needs_verification())
            .map(|item| (item.device.clone(), item.capability.clone()))
            .collect();
        if items.is_empty() {
            return;
        }
        // Acting on readings that may be minutes old is the thing the safety policy
        // exists to avoid, so verification waits for fresh data rather than guessing.
        if self.inner.runtime.is_stale(3) {
            return;
        }
        for (device, capability) in items {
            match self
                .inner
                .runtime
                .resolve(device.as_str(), capability.as_str())
            {
                Ok((_, target)) if !target.writable => {
                    self.fail_recovered(
                        &device,
                        &capability,
                        format!(
                            "{device}/{capability} is not writable any more, so the fail-safe duty \
                             cannot be applied to it. Check the device, then re-arm this handover."
                        ),
                    );
                }
                Ok(_) => {
                    let verified = self.inner.handovers.lock().verified(
                        &device,
                        &capability,
                        self.inner.ticks.load(Ordering::Relaxed),
                    );
                    if verified {
                        self.inner.dirty.store(true, Ordering::SeqCst);
                        let message = format!(
                            "{device}/{capability} exists and is writable; the recovered handover \
                             is now live and will be applied by the usual safety policy"
                        );
                        self.inner.runtime.log("info", message.clone());
                        self.inner.runtime.publish_automation(
                            None,
                            "control_state_verified",
                            message,
                        );
                    }
                }
                Err(error) => {
                    self.fail_recovered(
                        &device,
                        &capability,
                        format!(
                            "{device}/{capability} could not be resolved on this machine ({error}), \
                             so the responsibility recovered from the previous session cannot be \
                             discharged against it. Re-arm it after the device is back."
                        ),
                    );
                }
            }
        }
    }

    /// A recovered item whose channel cannot be verified: keep it, fail it visibly.
    fn fail_recovered(&self, device: &DeviceId, capability: &CapabilityId, reason: String) {
        let failed = self
            .inner
            .handovers
            .lock()
            .fail_recovered(device, capability, reason.clone());
        if failed {
            self.inner.dirty.store(true, Ordering::SeqCst);
            self.inner.runtime.log("warn", reason.clone());
            self.inner
                .runtime
                .publish_automation(None, "control_state_unverified", reason);
        }
    }

    /// Which enabled rule claims this channel as its target, if any?
    ///
    /// A claim is a reason **not to write** — the channel is about to be driven by
    /// that rule, and a safety write would fight it — but it is not evidence that the
    /// channel is being driven. See [`Self::confirmed_owner_of`].
    fn claimant_of(&self, device: &DeviceId, capability: &CapabilityId) -> Option<RuleId> {
        self.rules()
            .into_iter()
            .find(|rule| {
                rule.enabled
                    && &rule.target.device == device
                    && &rule.target.capability == capability
            })
            .map(|rule| rule.id)
    }

    /// Which enabled rule has **actually taken this channel over**?
    ///
    /// Evidence, not intent: the rule holds this channel and the device confirmed a
    /// value for it. A rule whose writes are unconfirmed, refused, or never issued
    /// does not own the channel in any sense the engine can act on, so a handover for
    /// it stays owed.
    fn confirmed_owner_of(&self, device: &DeviceId, capability: &CapabilityId) -> Option<RuleId> {
        let claimant = self.claimant_of(device, capability)?;
        let states = self.inner.states.lock();
        let hold = states.get(&claimant)?.control.as_ref()?;
        hold.is(device, capability)
            .then_some(())
            .filter(|_| hold.confirmed.is_some())
            .map(|_| claimant)
    }

    /// The rule's own last status, as text, for a diagnostic message.
    fn owner_status(&self, rule_id: &RuleId) -> Option<String> {
        self.inner
            .states
            .lock()
            .get(rule_id)
            .map(|state| format!("last status: {}", state.last_status.as_str()))
    }

    async fn tick_inner(&self, force: bool) -> Vec<RuleOutcome> {
        let now = ohm_core::now_ms();
        let tick = self.inner.ticks.fetch_add(1, Ordering::Relaxed);
        self.inner.last_tick_ms.store(now, Ordering::Relaxed);

        let settings = self.inner.runtime.settings();
        if !settings.automation_enabled {
            return Vec::new();
        }

        // Outputs abandoned by a rule edit are made safe before anything else.
        self.perform_pending_handovers(tick, now).await;

        // A stalled runtime means stale sensors: fall back rather than act on
        // numbers that may be minutes old.
        let source_unavailable = self.inner.runtime.is_stale(3);
        let safe_default_duty = settings.safety.fail_safe_duty_percent;

        let rules = self.rules();
        let mut outcomes = Vec::new();
        // One output, one writer per cycle. Even if a conflicting rule reached the
        // active set — a hand-edited file, an import path, a future API — the
        // runtime must never alternate two values onto one channel.
        let mut claimed_targets: Vec<String> = Vec::new();

        for rule in rules {
            if !rule.enabled {
                continue;
            }
            let target_id = rule.target.qualified_id();
            if claimed_targets.contains(&target_id) {
                let message = format!(
                    "another enabled rule already drove {target_id} this cycle; standing down"
                );
                tracing::warn!(rule = rule.id.as_str(), "{message}");
                self.inner.runtime.publish_automation(
                    Some(rule.id.clone()),
                    "rule_conflict_skipped",
                    message.clone(),
                );
                let mut state = self
                    .inner
                    .states
                    .lock()
                    .get(&rule.id)
                    .cloned()
                    .unwrap_or_else(|| RuleState::new(rule.id.clone()));
                state.last_status = RuleStatus::Error;
                state.last_message = message;
                state.next_due_ms = now + rule.update_interval_ms as i64;
                self.remember(&rule.id, state.clone());
                outcomes.push(RuleOutcome::from_parts(&rule, &state, now));
                continue;
            }
            claimed_targets.push(target_id);
            let state_snapshot = {
                let mut states = self.inner.states.lock();
                let state = states
                    .entry(rule.id.clone())
                    .or_insert_with(|| RuleState::new(rule.id.clone()));
                if !force && state.next_due_ms > now {
                    continue;
                }
                state.clone()
            };

            let outcome = self
                .run_rule(
                    &rule,
                    state_snapshot,
                    now,
                    source_unavailable,
                    safe_default_duty,
                )
                .await;
            outcomes.push(outcome);
        }

        // Anything the tick changed about unresolved control responsibility goes to
        // disk here, so a crash after this point loses nothing that was known.
        self.flush_control_state();
        outcomes
    }

    async fn run_rule(
        &self,
        rule: &Rule,
        mut state: RuleState,
        now: i64,
        source_unavailable: bool,
        safe_default_duty: f64,
    ) -> RuleOutcome {
        let target = self
            .inner
            .runtime
            .resolve(rule.target.device.as_str(), rule.target.capability.as_str());

        let (device, capability) = match target {
            Ok(resolved) => resolved,
            Err(err) => {
                state.last_status = RuleStatus::Error;
                state.last_message = format!("target unavailable: {err}");
                state.next_due_ms = now + rule.update_interval_ms as i64;
                self.remember(&rule.id, state.clone());
                self.inner.runtime.publish_automation(
                    Some(rule.id.clone()),
                    "rule_error",
                    err.to_string(),
                );
                return RuleOutcome::from_parts(rule, &state, now);
            }
        };

        let source_value = self.read_source(rule, source_unavailable);
        let condition_value = self.read_condition(rule, source_unavailable);
        let evaluation = evaluate(
            rule,
            &mut state,
            EvaluationInput {
                source_value,
                condition_value,
                now_ms: now,
                target_min: capability.min.unwrap_or(0.0),
                target_max: capability.max.unwrap_or(100.0),
                safe_default_duty,
                source_label: rule.source.label(),
            },
        );

        let mut wrote = false;
        if let Some(output) = evaluation.output.filter(|_| evaluation.should_write) {
            let origin = WriteOrigin::Automation {
                rule_id: rule.id.clone(),
            };
            match self
                .inner
                .runtime
                .write_value(
                    device.id.as_str(),
                    capability.id.as_str(),
                    Value::Number(output),
                    origin,
                )
                .await
            {
                Ok(report) => match report.status {
                    // The device is known to be at a value.
                    WriteStatus::Applied | WriteStatus::Simulated => {
                        wrote = true;
                        // Never fall back to the requested value: a confirmed write
                        // carries a confirmed value, and if it somehow does not, the
                        // honest answer is "unknown" rather than the request.
                        state.applied_output = report.applied.as_ref().and_then(Value::as_f64);
                        // The rule owns this channel from now on, and the value is
                        // confirmed. This is what a later handover is judged against.
                        state.control = Some(ControlHold {
                            device: device.id.clone(),
                            capability: capability.id.clone(),
                            confirmed: state.applied_output,
                            last_write_ms: now,
                        });
                        state.writes = state.writes.saturating_add(1);
                        state.consecutive_unconfirmed = 0;
                        state.last_write_ms = now;
                        // Keep the evaluation's verdict: a fail-safe write is still
                        // a fallback, not a normal `Applied`.
                        state.last_status = evaluation.status;
                        let confirmed = report
                            .applied
                            .as_ref()
                            .map(Value::to_string)
                            .unwrap_or_else(|| "unknown".into());
                        state.last_message = format!("{} (wrote {confirmed})", evaluation.message);
                        self.inner.runtime.publish_automation(
                            Some(rule.id.clone()),
                            "rule_applied",
                            format!(
                                "{} -> {confirmed} on {}",
                                rule.source.label(),
                                rule.target.qualified_id()
                            ),
                        );
                    }
                    // Accepted, but nobody knows what the device is at now.
                    WriteStatus::Unconfirmed => {
                        state.unconfirmed_writes = state.unconfirmed_writes.saturating_add(1);
                        state.consecutive_unconfirmed =
                            state.consecutive_unconfirmed.saturating_add(1);
                        // Forget the output state: we must not dedupe against a value
                        // we never verified, and the next evaluation has to try again.
                        state.applied_output = None;
                        // The write was issued, so the channel may have moved even
                        // though nobody saw it move: the rule is still answerable for
                        // it, which is what `confirmed: None` records.
                        state.control = Some(ControlHold {
                            device: device.id.clone(),
                            capability: capability.id.clone(),
                            confirmed: None,
                            last_write_ms: now,
                        });
                        state.last_write_ms = now;
                        state.last_status = RuleStatus::Unconfirmed;
                        // The crash window that matters: a write reached the device and
                        // its result was never learned. If the process dies before the
                        // next tick, this record is the only trace of it. The state is
                        // committed to the engine *first* — persisting before that would
                        // write an empty record and then mark the books clean.
                        self.remember(&rule.id, state.clone());
                        self.inner.dirty.store(true, Ordering::SeqCst);
                        self.flush_control_state();
                        state.last_message = format!(
                            "{} — accepted {:.0} % but not confirmed: {}",
                            evaluation.message,
                            output,
                            report.detail.as_deref().unwrap_or("no reason given")
                        );
                        self.inner.runtime.publish_automation(
                            Some(rule.id.clone()),
                            "rule_unconfirmed",
                            state.last_message.clone(),
                        );
                        self.inner
                            .runtime
                            .log("warn", format!("{}: {}", rule.name, state.last_message));

                        if state.consecutive_unconfirmed >= MAX_CONSECUTIVE_UNCONFIRMED {
                            // Bounded retries, then the rule's own failure policy —
                            // which is what attempts the safety fail-safe duty.
                            let detail = format!(
                                "{:.0} % was requested {MAX_CONSECUTIVE_UNCONFIRMED} times and never confirmed; control cannot be verified",
                                output
                            );
                            state.consecutive_unconfirmed = 0;
                            let error = OhmError::WriteRejected {
                                device: device.id.to_string(),
                                capability: capability.id.to_string(),
                                detail,
                            };
                            self.handle_write_failure(
                                rule,
                                &mut state,
                                &error,
                                &device,
                                &capability,
                                now,
                            )
                            .await;
                        }
                    }
                    // `Rejected` never reaches here: the runtime returns it as an
                    // error, and the `Err` arm below handles it.
                    WriteStatus::Rejected => {}
                },
                Err(err) => {
                    self.handle_write_failure(rule, &mut state, &err, &device, &capability, now)
                        .await;
                }
            }
        }

        if !wrote && evaluation.status == RuleStatus::Fallback {
            self.inner.runtime.publish_automation(
                Some(rule.id.clone()),
                "rule_fallback",
                evaluation.message.clone(),
            );
        }
        if evaluation.status == RuleStatus::Gated && !state.gate_announced {
            state.gate_announced = true;
            self.inner.runtime.publish_automation(
                Some(rule.id.clone()),
                "rule_gated",
                evaluation.message.clone(),
            );
            self.inner
                .runtime
                .log("info", format!("{}: {}", rule.name, evaluation.message));
        } else if evaluation.status != RuleStatus::Gated && state.gate_announced {
            state.gate_announced = false;
            self.inner.runtime.publish_automation(
                Some(rule.id.clone()),
                "rule_gate_open",
                format!("{} is steering again", rule.name),
            );
        }
        if evaluation.release {
            // Control went back to the firmware, so there is no channel to hand over
            // — and the rule must stop claiming one it no longer drives.
            state.control = None;
            state.applied_output = None;
            self.inner.runtime.publish_automation(
                Some(rule.id.clone()),
                "rule_released",
                evaluation.message.clone(),
            );
            self.inner
                .runtime
                .log("warn", format!("{}: {}", rule.name, evaluation.message));
        }

        self.remember(&rule.id, state.clone());
        RuleOutcome::from_parts(rule, &state, now)
    }

    /// The rule's write was refused. The runtime already applied the
    /// hardware-level fail-safe duty; the rule decides what *it* does next.
    async fn handle_write_failure(
        &self,
        rule: &Rule,
        state: &mut RuleState,
        error: &OhmError,
        device: &ohm_device_model::Device,
        capability: &ohm_device_model::Capability,
        now: i64,
    ) {
        state.last_status = RuleStatus::Error;
        state.last_message = format!("write failed: {error}");
        state.writes = state.writes.saturating_add(1);
        // The rule tried to write this channel and the result is unknown: it may or
        // may not have moved. It stays answerable for the channel.
        state.control = Some(ControlHold {
            device: device.id.clone(),
            capability: capability.id.clone(),
            confirmed: None,
            last_write_ms: now,
        });
        // Same ordering rule as above: the responsibility has to be in the engine's
        // state before it can be written down.
        self.remember(&rule.id, state.clone());
        self.inner.dirty.store(true, Ordering::SeqCst);
        self.flush_control_state();
        self.inner.runtime.publish_automation(
            Some(rule.id.clone()),
            "rule_error",
            format!("{}: {error}", rule.target.qualified_id()),
        );
        self.inner.runtime.log(
            "error",
            format!("{}: write to {} failed: {error}", rule.name, capability.id),
        );

        match rule.fallback.on_write_failure {
            // Unreachable through config (refused on save, sanitised on load), and
            // deliberately folded into the safe branch rather than left as a no-op:
            // an action that cannot be performed must never be reported as done.
            FallbackAction::Hold => {}
            FallbackAction::SafeDefault
            | FallbackAction::Fixed { .. }
            | FallbackAction::Release => {
                let duty = rule
                    .fallback
                    .on_write_failure
                    .duty(self.inner.runtime.settings().safety.fail_safe_duty_percent)
                    .unwrap_or_else(|| self.inner.runtime.settings().safety.fail_safe_duty_percent);
                let origin = WriteOrigin::Safety {
                    reason: format!("automation rule {} write failure", rule.id),
                };
                match self
                    .inner
                    .runtime
                    .write_value(
                        device.id.as_str(),
                        capability.id.as_str(),
                        Value::Number(duty),
                        origin,
                    )
                    .await
                {
                    Ok(report) => {
                        state.applied_output = report.applied.as_ref().and_then(Value::as_f64);
                        // The original cause must survive: "we fell back" without
                        // saying why is how a real problem gets hidden behind a
                        // reassuring message. The fail-safe write is also reported
                        // for what it was — attempted, and confirmed or not.
                        let outcome = match report.status {
                            WriteStatus::Applied | WriteStatus::Simulated => "applied",
                            WriteStatus::Unconfirmed => "accepted but not confirmed",
                            WriteStatus::Rejected => "rejected",
                        };
                        state.last_message =
                            format!("{error} — fail-safe duty of {duty:.0} % {outcome}");
                        self.inner.runtime.publish_automation(
                            Some(rule.id.clone()),
                            "rule_fallback",
                            state.last_message.clone(),
                        );
                    }
                    Err(second) => {
                        state.last_message = format!(
                            "{error} — the {duty:.0} % fail-safe duty failed too: {second}"
                        );
                        self.inner.runtime.log(
                            "error",
                            format!(
                                "{}: {}. Control may be lost.",
                                rule.name, state.last_message
                            ),
                        );
                    }
                }
            }
        }
        state.next_due_ms = now + rule.update_interval_ms as i64;
    }

    /// Read the rule's `when` sensor, if it has one.
    ///
    /// Deliberately *not* folded into [`AutomationEngine::read_source`]: the two
    /// sensors are independent, and a missing gate reading must not be confused
    /// with a missing curve source.
    fn read_condition(&self, rule: &Rule, source_unavailable: bool) -> Option<f64> {
        if source_unavailable {
            return None;
        }
        let condition = rule.when.as_ref()?;
        self.inner.runtime.reading(
            condition.source.device.as_str(),
            condition.source.capability.as_str(),
        )
    }

    /// Read the rule's source value from the runtime state store.
    fn read_source(&self, rule: &Rule, source_unavailable: bool) -> Option<f64> {
        if source_unavailable {
            return None;
        }
        let values: Vec<f64> = rule
            .source
            .sensors()
            .iter()
            .filter_map(|sensor| {
                self.inner
                    .runtime
                    .reading(sensor.device.as_str(), sensor.capability.as_str())
            })
            .collect();
        match rule.source.aggregate() {
            None => values.first().copied(),
            Some(aggregate) => aggregate.reduce(&values),
        }
    }

    fn remember(&self, id: &RuleId, state: RuleState) {
        self.inner.states.lock().insert(id.clone(), state);
    }

    // ------------------------------------------------------------------ rules

    pub fn rules(&self) -> Vec<Rule> {
        self.inner.rules.read().clone()
    }

    pub fn rule(&self, id: &str) -> Option<Rule> {
        self.inner
            .rules
            .read()
            .iter()
            .find(|r| r.id.as_str() == id)
            .cloned()
    }

    pub fn rule_count(&self) -> usize {
        self.inner.rules.read().len()
    }

    /// Live state of every rule.
    pub fn outcomes(&self) -> Vec<RuleOutcome> {
        let now = ohm_core::now_ms();
        let states = self.inner.states.lock();
        self.inner
            .rules
            .read()
            .iter()
            .map(|rule| {
                let state = states
                    .get(&rule.id)
                    .cloned()
                    .unwrap_or_else(|| RuleState::new(rule.id.clone()));
                RuleOutcome::from_parts(rule, &state, now)
            })
            .collect()
    }

    pub fn outcome(&self, id: &str) -> Option<RuleOutcome> {
        self.outcomes()
            .into_iter()
            .find(|o| o.rule_id.as_str() == id)
    }

    /// The enabled rule that already owns this rule's target, if any.
    ///
    /// `candidate` itself is excluded, so re-saving a rule is never a conflict
    /// with its own previous version.
    pub fn conflicting_rule(&self, candidate: &Rule) -> Option<Rule> {
        let target = candidate.target.qualified_id();
        self.inner
            .rules
            .read()
            .iter()
            .find(|existing| {
                existing.enabled
                    && existing.id != candidate.id
                    && existing.target.qualified_id() == target
            })
            .cloned()
    }

    /// Conflicts found in the rule files, resolved but not silently rewritten.
    pub fn conflicts(&self) -> Vec<RuleConflict> {
        self.inner.conflicts.read().clone()
    }

    /// Rules that loaded with an in-memory adjustment, with the reason.
    ///
    /// The rule files are never rewritten; this is how the user finds out that the
    /// machine is running something other than what the file says.
    /// Write the unresolved control responsibility to disk, atomically.
    ///
    /// Failure is recorded and reported rather than swallowed: if this file cannot be
    /// written, a crash would take the last record of a channel nobody protects with
    /// it, and the user has to know that is the situation.
    pub fn persist_control_state(&self) -> Result<()> {
        let record = self.build_recovery_record();
        let result = self.inner.recovery.save(&record);
        match &result {
            Ok(()) => {
                self.inner.dirty.store(false, Ordering::SeqCst);
                // Nothing outstanding means no file: a stale record would claim work
                // that has already been done.
                if record.is_empty()
                    && let Err(error) = self.inner.recovery.clear()
                {
                    tracing::warn!(error = %error, "could not remove the control record");
                }
                let mut slot = self.inner.persistence_error.lock();
                if slot.take().is_some() {
                    self.inner.runtime.publish_automation(
                        None,
                        "control_state_persisted",
                        "unresolved control responsibility is being recorded again".to_string(),
                    );
                }
            }
            Err(error) => {
                let message = format!(
                    "could not record unresolved control responsibility: {error}. If the app \
                     stops now, a channel that is owed the fail-safe duty may not be recovered."
                );
                {
                    let mut slot = self.inner.persistence_error.lock();
                    *slot = Some(message.clone());
                }
                self.inner.runtime.log("error", message.clone());
                self.inner.runtime.publish_automation(
                    None,
                    "control_state_persistence_failed",
                    message,
                );
            }
        }
        result
    }

    /// Write the record if anything changed since the last write.
    ///
    /// One place, called from every path that can change what is owed — a rule edit, a
    /// deletion, a reload, a tick, shutdown — because the alternative is a list of call
    /// sites to keep in step, and one that gets forgotten is a responsibility that dies
    /// with the process.
    fn flush_control_state(&self) {
        if self.inner.dirty.load(Ordering::SeqCst) {
            let _ = self.persist_control_state();
        }
    }

    /// Why the last attempt to record control responsibility failed, if it did.
    pub fn persistence_error(&self) -> Option<String> {
        self.inner.persistence_error.lock().clone()
    }

    /// The control responsibility as it would be written down.
    fn build_recovery_record(&self) -> crate::recovery::RecoveryRecord {
        let mut record = crate::recovery::RecoveryRecord::new(ohm_core::now_ms());
        for item in self.inner.handovers.lock().unresolved() {
            record.handovers.push(crate::recovery::StoredHandover {
                device: item.device.clone(),
                capability: item.capability.clone(),
                from_rule: item.from_rule.clone(),
                reason: item.reason.clone(),
                state: item.state,
                attempts: item.attempts,
                first_error: item.first_error.clone(),
                last_error: item.last_error.clone(),
                queued_at_ms: item.queued_at_ms,
                last_attempt_ms: item.last_attempt_ms,
                claimant: item.claimant.clone(),
                claimed_ticks: item.claimed_ticks,
                parked_by_claim: item.parked_by_claim,
            });
        }
        // A write that was issued and never confirmed: the channel may have moved and
        // nobody saw it. That is a live responsibility even while the rule exists.
        for (rule_id, state) in self.inner.states.lock().iter() {
            if let Some(hold) = &state.control
                && hold.confirmed.is_none()
            {
                record.holds.push(crate::recovery::StoredHold {
                    rule_id: rule_id.clone(),
                    device: hold.device.clone(),
                    capability: hold.capability.clone(),
                    last_write_ms: hold.last_write_ms,
                });
            }
        }
        record
    }

    /// Read the record written by a previous session and put it back on the books.
    ///
    /// Recovered items come back as *needing verification*, never as live work: the
    /// device, its capabilities and the current owner are checked against the hardware
    /// as it is now before the safety policy is applied to them.
    pub fn recover_control_state(&self) -> Result<usize> {
        let record = match self.inner.recovery.load() {
            Ok(Some(record)) => record,
            Ok(None) => return Ok(0),
            Err(error) => {
                // A record that cannot be read is reported, not ignored and not
                // guessed at: the caller decides what to tell the user, and nothing is
                // invented from a damaged file.
                let message = format!("{error}");
                self.inner.runtime.log("error", message.clone());
                self.inner.runtime.publish_automation(
                    None,
                    "control_state_unreadable",
                    message.clone(),
                );
                let mut slot = self.inner.persistence_error.lock();
                *slot = Some(message);
                return Err(error);
            }
        };

        let now = ohm_core::now_ms();
        let mut recovered = 0;
        {
            let mut book = self.inner.handovers.lock();
            for stored in record.handovers {
                book.restore(
                    crate::handover::Handover {
                        device: stored.device,
                        capability: stored.capability,
                        from_rule: stored.from_rule,
                        reason: format!(
                            "{} (recovered from the previous session; the device has not been \
                             checked yet)",
                            stored.reason
                        ),
                        state: stored.state,
                        attempts: stored.attempts,
                        first_error: stored.first_error,
                        last_error: stored.last_error,
                        queued_at_ms: stored.queued_at_ms,
                        last_attempt_ms: stored.last_attempt_ms,
                        confirmed_value: None,
                        superseded_by: None,
                        claimant: stored.claimant,
                        claimed_ticks: stored.claimed_ticks,
                        parked_by_claim: stored.parked_by_claim,
                        next_attempt_tick: 0,
                    },
                    now,
                );
                recovered += 1;
            }
        }

        // An issued-but-unconfirmed write: if the rule that made it is still enabled it
        // will write again and settle the question by itself, so there is nothing to
        // recover. If it is not, the channel may be at a value nobody verified and
        // nobody is answerable for — that is owed the fail-safe duty.
        let rules = self.rules();
        for hold in record.holds {
            // The *channel*, not just the device: a rule retargeted from one control
            // channel to another on the same device no longer drives the one it left
            // unverified, and dropping that hold would lose the responsibility.
            let still_driven = rules.iter().any(|rule| {
                rule.enabled
                    && rule.id == hold.rule_id
                    && rule.target.device == hold.device
                    && rule.target.capability == hold.capability
            });
            if still_driven {
                continue;
            }
            let mut book = self.inner.handovers.lock();
            book.restore(
                crate::handover::Handover {
                    device: hold.device.clone(),
                    capability: hold.capability.clone(),
                    from_rule: hold.rule_id.clone(),
                    reason: format!(
                        "a write to {}/{} was issued by `{}` and never confirmed, and that rule \
                         no longer drives it (recovered from the previous session)",
                        hold.device, hold.capability, hold.rule_id
                    ),
                    state: crate::handover::HandoverState::NeedsVerification,
                    attempts: 0,
                    first_error: Some(
                        "a write was issued and its result was never learned".to_string(),
                    ),
                    last_error: None,
                    queued_at_ms: hold.last_write_ms,
                    last_attempt_ms: hold.last_write_ms,
                    confirmed_value: None,
                    superseded_by: None,
                    claimant: None,
                    claimed_ticks: 0,
                    parked_by_claim: false,
                    next_attempt_tick: 0,
                },
                now,
            );
            recovered += 1;
        }

        if recovered > 0 {
            self.inner.dirty.store(true, Ordering::SeqCst);
            let message = format!(
                "{recovered} unresolved control responsibility item(s) recovered from the \
                 previous session; each will be checked against the hardware before anything \
                 is written"
            );
            self.inner.runtime.log("warn", message.clone());
            self.inner
                .runtime
                .publish_automation(None, "control_state_recovered", message);
        }
        Ok(recovered)
    }

    /// Channels left behind by a rule edit, and what became of them.
    ///
    /// Unfinished handovers come first; finished ones are kept as a bounded history.
    /// This is the only place the answer survives, because the rule that abandoned the
    /// channel may itself be gone — and a channel still sitting at an old duty with a
    /// failed handover is exactly the thing a user must be able to see.
    pub fn handovers(&self) -> Vec<crate::handover::HandoverReport> {
        self.inner.handovers.lock().reports()
    }

    /// How many handovers are still unfinished.
    pub fn unfinished_handovers(&self) -> usize {
        self.inner.handovers.lock().open_count()
    }

    /// Re-arm every handover that ran out of attempts, and report how many.
    ///
    /// The recovery action for an exhausted handover: the user fixes whatever was
    /// wrong (starts the service, restores the connection) and asks for another go.
    /// It is explicit on purpose — an engine that retries for ever is an engine that
    /// writes to a broken channel for ever.
    pub fn retry_failed_handovers(&self) -> usize {
        let tick = self.inner.ticks.load(Ordering::Relaxed);
        self.inner.handovers.lock().retry_failed(tick)
    }

    /// Re-arm one channel's failed handover, and nothing else.
    ///
    /// The scoped form exists because a caller that means to act on one channel must
    /// not be able to re-arm a pending responsibility somewhere else — a verification
    /// harness that re-armed *every* failed handover would be reaching into the user's
    /// hardware to do it. Returns `true` when that channel's handover was re-armed.
    pub fn retry_failed_handover(
        &self,
        device: &ohm_core::DeviceId,
        capability: &ohm_core::CapabilityId,
    ) -> bool {
        let tick = self.inner.ticks.load(Ordering::Relaxed);
        self.inner
            .handovers
            .lock()
            .retry_failed_one(device, capability, tick)
    }

    pub fn compatibility_notes(&self) -> Vec<crate::store::RuleFileNote> {
        self.inner.compatibility_notes.read().clone()
    }

    /// The one wording used for a clash, wherever it is reported (form, import,
    /// enable, or a file loaded at startup).
    fn conflict_message(candidate: &Rule, owner: &Rule) -> String {
        format!(
            "`{}` cannot drive {}: `{}` already owns that output. Each output may be driven by \
             only one enabled rule — disable `{}`, retarget this rule, or delete one of them.",
            candidate.name,
            candidate.target.qualified_id(),
            owner.name,
            owner.name
        )
    }

    /// The same clash as a hard error.
    fn conflict_error(candidate: &Rule, owner: &Rule) -> OhmError {
        OhmError::Automation(Self::conflict_message(candidate, owner))
    }

    /// Validate a rule against the hardware that is currently attached.
    pub fn check_rule(&self, rule: &Rule) -> RuleCheck {
        let mut check = RuleCheck {
            errors: Vec::new(),
            warnings: Vec::new(),
        };
        if let Err(err) = rule.validate() {
            check.errors.push(err.to_string());
        }

        for sensor in rule.source.sensors() {
            match self
                .inner
                .runtime
                .resolve(sensor.device.as_str(), sensor.capability.as_str())
            {
                Ok((device, capability)) => {
                    if !capability.readable || !capability.kind.is_sensor() {
                        check.errors.push(format!(
                            "{} is not a readable sensor",
                            sensor.qualified_id()
                        ));
                    } else if !capability.unit.is_temperature() {
                        check.warnings.push(format!(
                            "{} is a {} sensor; a cooling curve normally reads a temperature",
                            sensor.qualified_id(),
                            capability.unit
                        ));
                    }
                    if !device.is_controllable() && !capability.readable {
                        check.warnings.push(format!("{} looks offline", device.id));
                    }
                }
                Err(err) => check
                    .errors
                    .push(format!("source {}: {err}", sensor.qualified_id())),
            }
        }

        match self
            .inner
            .runtime
            .resolve(rule.target.device.as_str(), rule.target.capability.as_str())
        {
            Ok((device, capability)) => {
                if !capability.writable {
                    check.errors.push(format!(
                        "{} is read-only; pick an actuator",
                        rule.target.qualified_id()
                    ));
                }
                if device.device_type.requires_safe_floor() && !capability.unit.is_duty() {
                    check.warnings.push(format!(
                        "{} controls a {} through a {} capability",
                        rule.target.qualified_id(),
                        device.device_type,
                        capability.unit
                    ));
                }
                let (min, max) = (
                    capability.min.unwrap_or(0.0),
                    capability.max.unwrap_or(100.0),
                );
                let (curve_min, curve_max) = (rule.curve.min_output(), rule.curve.max_output());
                let (low, high) = rule.effective_output_range();
                if curve_max > max || curve_min < min {
                    check.warnings.push(format!(
                        "curve output {curve_min:.0}-{curve_max:.0} % is outside the device range \
                         {min:.0}-{max:.0} %; values will be clamped"
                    ));
                }
                if low > high {
                    check.errors.push("output limits are inverted".into());
                }
                if let Some(min_output) = rule.min_output.filter(|value| *value < min) {
                    check.warnings.push(format!(
                        "min_output {min_output:.0} % is below the device minimum {min:.0} %"
                    ));
                }
                match capability.is_duty_control() {
                    true => {}
                    false => check.warnings.push(format!(
                        "{} is not a duty control; the safety floor cannot be applied",
                        rule.target.qualified_id()
                    )),
                }
            }
            Err(err) => check
                .errors
                .push(format!("target {}: {err}", rule.target.qualified_id())),
        }

        // The `when` gate.
        if let Some(condition) = &rule.when {
            match self.inner.runtime.resolve(
                condition.source.device.as_str(),
                condition.source.capability.as_str(),
            ) {
                Ok((_device, capability)) => {
                    if !capability.readable || !capability.kind.is_sensor() {
                        check.errors.push(format!(
                            "when source {} is not a readable sensor",
                            condition.source.qualified_id()
                        ));
                    }
                    if condition.op.is_equality() {
                        check.warnings.push(format!(
                            "when uses `{}` on {}, an analog reading; `gte`/`lte` say what you \
                             mean and will not flip with a 0.1 {} wobble",
                            condition.op.as_str(),
                            condition.source.qualified_id(),
                            capability.unit.suffix().trim()
                        ));
                    }
                    // A percentage threshold outside 0-100 can never be met.
                    if matches!(capability.unit, ohm_device_model::Unit::Percent)
                        && !(0.0..=100.0).contains(&condition.value)
                    {
                        check.warnings.push(format!(
                            "when threshold {} is outside the 0-100 % range of {}; the condition \
                             can never be satisfied",
                            condition.value,
                            condition.source.qualified_id()
                        ));
                    }
                }
                Err(err) => check.errors.push(format!(
                    "when source {}: {err}",
                    condition.source.qualified_id()
                )),
            }
            if let OtherwiseAction::Fixed { percent } = condition.otherwise
                && !(0.0..=100.0).contains(&percent)
            {
                check
                    .errors
                    .push("when.otherwise.fixed percentage must be between 0 and 100".into());
            }
        }

        // One enabled rule per output: report the clash here so the form can
        // explain it before the user hits save.
        if rule.enabled
            && let Some(owner) = self.conflicting_rule(rule)
        {
            check.errors.push(Self::conflict_message(rule, &owner));
        }

        if rule.hysteresis == 0.0 {
            check
                .warnings
                .push("hysteresis is 0: the fan may oscillate around a curve knee".into());
        }
        if rule.update_interval_ms < 250 {
            check.warnings.push(format!(
                "update_interval_ms is {}; values below 250 ms can flood the hardware",
                rule.update_interval_ms
            ));
        }
        check
    }

    /// Find rules that fight over one output and stand the later ones down.
    ///
    /// Nothing is deleted or rewritten on disk: the user is told which rule was
    /// disabled and why, and can fix the file or retarget the rule. The owner is
    /// chosen deterministically (highest `priority`, then lowest id), so the same
    /// files always resolve the same way.
    fn resolve_loaded_conflicts(&self) {
        let mut disabled: Vec<(RuleId, String)> = Vec::new();
        let mut conflicts: Vec<RuleConflict> = Vec::new();
        {
            let mut rules = self.inner.rules.write();
            rules.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
            let mut owners: Vec<(String, RuleId, String)> = Vec::new();
            for rule in rules.iter_mut() {
                if !rule.enabled {
                    continue;
                }
                let target = rule.target.qualified_id();
                match owners.iter().find(|(owned, _, _)| *owned == target) {
                    Some((_, owner_id, owner_name)) => {
                        let resolution = format!(
                            "`{}` was disabled for this session; the file is untouched, so fix \
                             it, retarget it, or delete it",
                            rule.name
                        );
                        conflicts.push(RuleConflict {
                            owner_id: owner_id.clone(),
                            owner_name: owner_name.clone(),
                            blocked_id: rule.id.clone(),
                            blocked_name: rule.name.clone(),
                            target: target.clone(),
                            resolution: resolution.clone(),
                        });
                        rule.enabled = false;
                        disabled.push((rule.id.clone(), resolution));
                    }
                    None => owners.push((target, rule.id.clone(), rule.name.clone())),
                }
            }
        }

        for (id, resolution) in &disabled {
            tracing::warn!(rule = id.as_str(), resolution, "rule conflict on load");
            self.inner
                .runtime
                .log("warn", format!("automation rule `{id}`: {resolution}"));
            self.inner.runtime.publish_automation(
                Some(id.clone()),
                "rule_conflict_disabled",
                resolution.clone(),
            );
            if let Some(state) = self.inner.states.lock().get_mut(id) {
                state.last_status = RuleStatus::Disabled;
                state.last_message = resolution.clone();
            }
        }

        *self.inner.conflicts.write() = conflicts;
    }

    /// Recompute the conflict list from the currently active rules.
    ///
    /// `resolve_loaded_conflicts` stands losers down in memory; this keeps the
    /// reported list honest after a save or a delete fixes the situation, so the
    /// UI stops showing a stale warning.
    fn refresh_conflicts_for_active_set(&self) {
        let active: Vec<Rule> = self.rules();
        let mut owners: Vec<(String, RuleId, String)> = Vec::new();
        let mut conflicts = Vec::new();
        for rule in active.iter().filter(|rule| rule.enabled) {
            let target = rule.target.qualified_id();
            match owners.iter().find(|(owned, _, _)| *owned == target) {
                Some((_, owner_id, owner_name)) => conflicts.push(RuleConflict {
                    owner_id: owner_id.clone(),
                    owner_name: owner_name.clone(),
                    blocked_id: rule.id.clone(),
                    blocked_name: rule.name.clone(),
                    target,
                    resolution: "resolved at load time: this rule is disabled".to_string(),
                }),
                None => owners.push((target, rule.id.clone(), rule.name.clone())),
            }
        }
        *self.inner.conflicts.write() = conflicts;
    }

    /// React to a rule whose meaning changed.
    ///
    /// Two things happen, in this order:
    ///
    /// 1. If the output moved, the **old** output is driven to the safety fail-safe
    ///    duty and the attempt is audited. Abandoning it at whatever the curve last
    ///    said would leave a fan nobody manages, at a speed nobody chose.
    /// 2. The cached state that no longer applies is dropped: an output value
    ///    measured on a different channel, a reading from a different sensor, a gate
    ///    that was opened by a different condition.
    ///
    /// Metadata-only changes take neither step, so renaming a rule cannot interrupt
    /// control.
    /// Reconcile the engine's memory with a rule that now means something else.
    ///
    /// Called from every path that can change a rule: the form, the CLI, a reloaded
    /// file, a deletion, an enable/disable. It answers one question — *did this rule
    /// stop driving a channel it was driving?* — and if so, queues that channel for
    /// handover. A rule that never took control of the channel is not entitled to hand
    /// it over, and a rule whose channel did not change keeps driving it.
    fn apply_rule_change(&self, previous: &Rule, change: &RuleChange) {
        let Some(summary) = change.summary() else {
            return;
        };
        tracing::info!(
            rule = previous.id.as_str(),
            change = summary.as_str(),
            "rule meaning changed, invalidating control memory"
        );

        // What the rule actually held, as opposed to what it declared. A disabled
        // rule, or one whose write never landed, holds nothing.
        let held = self
            .inner
            .states
            .lock()
            .get(&previous.id)
            .and_then(|state| state.control.clone());

        if change.leaves_control() {
            let why = if change.previous_target.is_some() {
                "was retargeted"
            } else {
                "was disabled"
            };
            self.hand_over(
                previous.id.clone(),
                held,
                change.previous_target.as_ref(),
                why,
            );
            // The queue changed; write it down now rather than at the end of a tick
            // that may never come. A failure is reported by `persist_control_state`
            // and surfaced through `persistence_error`.
            self.flush_control_state();
        }

        // 2. Drop the memory that no longer describes this rule.
        if let Some(state) = self.inner.states.lock().get_mut(&previous.id) {
            if change.leaves_control() {
                // Another channel's value is not evidence about this one, and a rule
                // that does not drive anything must not claim ownership of what it
                // used to drive.
                state.applied_output = None;
                state.control = None;
                state.armed_input = f64::NAN;
            }
            if change.source_changed {
                // Including the grace-period anchor: a reading from the old sensor
                // must not stand in for the new one.
                state.last_input = None;
                state.last_input_ms = 0;
                state.armed_input = f64::NAN;
            }
            if change.condition_changed {
                state.last_condition = None;
                state.last_condition_ms = 0;
                state.gate_open = false;
                state.gate_announced = false;
            }
            if change.control_changed {
                // The hysteresis anchor referred to the old mapping.
                state.armed_input = f64::NAN;
            }
            state.next_due_ms = 0;
        }
    }

    /// Queue a channel for handover, if this rule was really driving one.
    ///
    /// `held` is what the rule's own memory says it controlled — `None` when it never
    /// wrote, or already gave control up. `previous_target` is the channel its *config*
    /// used to name, used to make sure the channel being handed over is the one the
    /// rule actually drove: a target that was changed before the engine ever ran was
    /// never controlled, and handing it over would write a channel nobody ever touched.
    fn hand_over(
        &self,
        rule_id: RuleId,
        held: Option<ControlHold>,
        previous_target: Option<&(DeviceId, CapabilityId)>,
        why: &str,
    ) {
        let Some(hold) = held.filter(|hold| {
            previous_target.is_none_or(|(device, capability)| hold.is(device, capability))
        }) else {
            return;
        };
        let tick = self.inner.ticks.load(Ordering::Relaxed);
        self.inner.handovers.lock().queue(
            hold.device.clone(),
            hold.capability.clone(),
            rule_id.clone(),
            format!(
                "rule `{rule_id}` {why}; {}/{} is no longer driven by it",
                hold.device, hold.capability
            ),
            tick,
            ohm_core::now_ms(),
        );
        // Queueing *is* the change that has to be recorded, so the book is marked dirty
        // here rather than at each call site: a path that queues a handover and forgets
        // to mark it — a rule file that vanished on reload, say — would lose the
        // responsibility if the process died before the next tick.
        self.inner.dirty.store(true, Ordering::SeqCst);
    }

    /// Validate and persist a rule, then make it live.
    pub fn save_rule(&self, mut rule: Rule) -> Result<Rule> {
        let check = self.check_rule(&rule);
        check.into_result()?;
        // The version being replaced, so a semantic change can be detected after
        // the write succeeds (see `RuleChange`).
        let previous = self.rule(rule.id.as_str());
        // Refuse a second enabled rule on an owned output, whatever route it came
        // in through (the form, a hand-written file, an import, an API call).
        if rule.enabled
            && let Some(owner) = self.conflicting_rule(&rule)
        {
            return Err(Self::conflict_error(&rule, &owner));
        }
        let existing = self.rule(rule.id.as_str());
        if existing.is_none() {
            rule.created_at_ms.get_or_insert_with(ohm_core::now_ms);
        } else if rule.created_at_ms.is_none() {
            rule.created_at_ms = existing.and_then(|r| r.created_at_ms);
        }
        rule.updated_at_ms = Some(ohm_core::now_ms());
        rule.validate()?;
        self.inner.store.save(&rule)?;

        {
            let mut rules = self.inner.rules.write();
            rules.retain(|r| r.id != rule.id);
            rules.push(rule.clone());
            rules.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
        }
        // A rule that was just edited must be re-evaluated immediately.
        self.inner
            .states
            .lock()
            .entry(rule.id.clone())
            .and_modify(|state| state.next_due_ms = 0)
            .or_insert_with(|| RuleState::new(rule.id.clone()));

        // The active set changed: this save may have fixed a reported conflict.
        self.refresh_conflicts_for_active_set();

        // A change to what the rule means invalidates the state that described the
        // old meaning — and hands the old output over on the way.
        if let Some(previous) = previous {
            let change = RuleChange::between(&previous, &rule);
            self.apply_rule_change(&previous, &change);
        }
        self.inner
            .runtime
            .publish_automation(Some(rule.id.clone()), "rule_saved", rule.summary());
        self.inner
            .runtime
            .log("info", format!("automation rule saved: {}", rule.name));
        Ok(rule)
    }

    /// Delete a rule and forget its state.
    ///
    /// Deleting is leaving control: whatever channel this rule was driving would
    /// otherwise sit at the last curve value forever, with nothing left to say so.
    pub fn delete_rule(&self, id: &str) -> Result<bool> {
        let rule_id = RuleId::new(id)?;
        // Read what the rule held *before* it is forgotten: the handover is owed by
        // the channel, not by the rule, so the rule may disappear entirely and the
        // handover must still happen — and stay visible if it fails.
        let held = self
            .inner
            .states
            .lock()
            .get(&rule_id)
            .and_then(|state| state.control.clone());
        let existed = {
            let mut rules = self.inner.rules.write();
            let before = rules.len();
            rules.retain(|r| r.id != rule_id);
            rules.len() != before
        };
        let file_removed = self.inner.store.delete(&rule_id)?;
        self.inner.states.lock().remove(&rule_id);
        if held.is_some() {
            self.hand_over(rule_id.clone(), held, None, "was deleted");
        }
        self.flush_control_state();
        // Deleting the winner may have freed the target another rule wants.
        self.refresh_conflicts_for_active_set();
        if existed || file_removed {
            self.inner.runtime.publish_automation(
                Some(rule_id.clone()),
                "rule_deleted",
                format!("rule {rule_id} deleted"),
            );
        }
        Ok(existed || file_removed)
    }

    /// Enable or disable a rule without deleting it.
    pub fn set_rule_enabled(&self, id: &str, enabled: bool) -> Result<Rule> {
        let mut rule = self
            .rule(id)
            .ok_or_else(|| OhmError::Automation(format!("no rule with id `{id}`")))?;
        if enabled
            && !rule.enabled
            && let Some(owner) = self.conflicting_rule(&rule)
        {
            return Err(Self::conflict_error(&rule, &owner));
        }
        rule.enabled = enabled;
        let saved = self.save_rule(rule)?;
        if let Some(state) = self
            .inner
            .states
            .lock()
            .get_mut(&saved.id)
            .filter(|_| !enabled)
        {
            state.last_status = RuleStatus::Disabled;
            state.last_message = "disabled by the user".into();
        }
        Ok(saved)
    }

    /// Write the shipped example rule if no rule exists yet.
    pub fn ensure_example(&self) -> Result<bool> {
        if self.rule_count() > 0 {
            return Ok(false);
        }
        let rule = crate::examples::gpu_cooling_example();
        // Only install the mock example when its devices are actually present;
        // otherwise derive one from the live hardware.
        if self.check_rule(&rule).is_ok() {
            self.save_rule(rule)?;
            return Ok(true);
        }
        let suggestion = crate::examples::suggest_for(&self.inner.runtime)
            .into_iter()
            .next();
        match suggestion {
            Some(rule) => {
                self.save_rule(rule)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Reset all control memory (after a device was re-enabled, for example).
    pub fn reset_states(&self) {
        let mut states = self.inner.states.lock();
        for state in states.values_mut() {
            state.reset();
        }
    }

    pub fn stats(&self) -> AutomationStats {
        let outcomes = self.outcomes();
        AutomationStats {
            rules: outcomes.len(),
            enabled_rules: outcomes.iter().filter(|o| o.enabled).count(),
            ticks: self.inner.ticks.load(Ordering::Relaxed),
            evaluations: outcomes.iter().map(|o| o.evaluations).sum(),
            writes: outcomes.iter().map(|o| o.writes).sum(),
            skipped: outcomes.iter().map(|o| o.skipped).sum(),
            fallbacks: outcomes.iter().map(|o| o.fallbacks).sum(),
            failures: outcomes
                .iter()
                .filter(|o| o.status == RuleStatus::Error)
                .count() as u64,
            last_tick_ms: self.inner.last_tick_ms.load(Ordering::Relaxed),
            persistence_error: self.persistence_error(),
        }
    }
}

/// Reduce a list of rules to the ones that should survive a first-run import
/// (skips ids that already exist).
pub fn merge_suggestions(existing: &[Rule], suggestions: Vec<Rule>) -> Vec<Rule> {
    suggestions
        .into_iter()
        .filter(|candidate| !existing.iter().any(|rule| rule.id == candidate.id))
        .collect()
}

/// Helper used by the UI's "MAX(CPU, GPU)" template.
pub fn aggregate_label(aggregate: Aggregate) -> &'static str {
    aggregate.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::{Fallback, Source, Target};
    use ohm_adapter_api::HardwareAdapter;
    use ohm_adapter_mock::{LoadProfile, MockAdapter, MockConfig, MockFaults};
    use ohm_core::ConfigPaths;
    use ohm_device_model::DeviceType;
    use ohm_runtime::Settings;

    /// A runtime wired to a deterministic mock machine with manual time.
    async fn engine_with_mock() -> (tempfile::TempDir, AutomationEngine, Arc<MockAdapter>) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::from_root(tmp.path());
        let mock = Arc::new(MockAdapter::new(MockConfig::deterministic()));
        let mock_adapter: Arc<dyn HardwareAdapter> = mock.clone();
        let adapters: Vec<Arc<dyn HardwareAdapter>> = vec![mock_adapter];
        let runtime = Runtime::new(paths, Settings::default(), adapters).unwrap();
        runtime.refresh_devices().await.unwrap();
        let store = RuleStore::new(tmp.path().join("rules"));
        let engine = AutomationEngine::new(runtime, store);
        (tmp, engine, mock)
    }

    fn gpu_rule() -> Rule {
        Rule::new(
            "test-gpu",
            "Test GPU",
            Source::sensor("gpu.mock.0", "temperature.core"),
            Target::new("fan.mock.0", "fan.speed_percent"),
            crate::curve::gpu_cooling_curve(),
        )
        .unwrap()
        .with_hysteresis(2.0)
        .with_deadband(0.0)
        .with_update_interval_ms(100)
    }

    #[tokio::test]
    async fn rule_is_validated_against_live_hardware() {
        let (_tmp, engine, _mock) = engine_with_mock().await;
        let rule = gpu_rule();
        let check = engine.check_rule(&rule);
        assert!(check.is_ok(), "unexpected errors: {:?}", check.errors);

        // A tachometer is readable, so it is a legal (if unusual) source: a
        // warning, not an error.
        let unusual_source = Rule {
            source: Source::sensor("gpu.mock.0", "fan.rpm"),
            ..gpu_rule()
        };
        let check = engine.check_rule(&unusual_source);
        assert!(check.is_ok(), "{:?}", check.errors);
        assert!(check.warnings.iter().any(|w| w.contains("temperature")));

        // A device that does not exist is a hard error.
        let missing_device = Rule {
            source: Source::sensor("gpu.nope.0", "temperature.core"),
            ..gpu_rule()
        };
        assert!(!engine.check_rule(&missing_device).is_ok());

        let missing = Rule {
            source: Source::sensor("gpu.nope.0", "temperature.core"),
            ..gpu_rule()
        };
        assert!(
            engine
                .check_rule(&missing)
                .errors
                .iter()
                .any(|e| e.contains("device_not_found") || e.contains("not found"))
        );

        let read_only_target = Rule {
            target: Target::new("fan.mock.0", "fan.rpm"),
            ..gpu_rule()
        };
        assert!(!engine.check_rule(&read_only_target).is_ok());
    }

    #[tokio::test]
    async fn tick_writes_the_curve_value_and_engages_the_loop() {
        let (_tmp, engine, mock) = engine_with_mock().await;
        mock.set_gpu_load(1.0);
        mock.tick(120_000); // let the GPU reach its hot equilibrium
        engine.runtime().poll_once().await.unwrap();
        let target_temp = engine
            .runtime()
            .reading("gpu.mock.0", "temperature.core")
            .unwrap();
        assert!(target_temp > 80.0, "expected a hot GPU, got {target_temp}");

        engine.save_rule(gpu_rule()).unwrap();
        let outcomes = engine.tick().await;
        assert_eq!(outcomes.len(), 1);
        let outcome = &outcomes[0];
        assert_eq!(outcome.status, RuleStatus::Applied);
        assert_eq!(outcome.applied_output, Some(100.0));

        // The write really reached the mock: the fan is now at 100 %.
        assert_eq!(mock.status().fan_duties[0], 100.0);

        // And an immediate re-evaluation with an unchanged temperature writes
        // nothing (the deadband and the hysteresis both hold it).
        let second = engine.tick_force().await;
        assert_eq!(second[0].status, RuleStatus::Held);
        assert_eq!(second[0].writes, 1);
        assert_eq!(second[0].skipped, 1);
    }

    /// Scenario C of the acceptance criteria, end to end:
    /// hot GPU -> rule -> fan duty -> cooler GPU.
    #[tokio::test]
    async fn closed_loop_cools_the_gpu_down() {
        let (_tmp, engine, mock) = engine_with_mock().await;
        mock.set_gpu_load(1.0);
        mock.force_gpu_temperature(92.0);
        engine.runtime().poll_once().await.unwrap();
        let start = engine
            .runtime()
            .reading("gpu.mock.0", "temperature.core")
            .unwrap();
        assert!(start > 90.0);
        engine.save_rule(gpu_rule()).unwrap();

        // Run the real loop: advance the simulation, poll the runtime (which is
        // what feeds the state store), then let the engine evaluate.
        for _ in 0..240 {
            mock.tick(500);
            engine.runtime().poll_once().await.unwrap();
            engine.tick_force().await;
        }

        let status = mock.status();
        let end = engine
            .runtime()
            .reading("gpu.mock.0", "temperature.core")
            .unwrap();
        assert_eq!(
            status.fan_duties[0], 100.0,
            "the rule should have spun the fan up"
        );
        assert!(
            end < start - 15.0,
            "the closed loop should have cooled the GPU: {start} -> {end}"
        );

        // Cool the machine down and check that the rule eases the fan off.
        mock.set_gpu_load(0.05);
        for _ in 0..600 {
            mock.tick(500);
            engine.runtime().poll_once().await.unwrap();
            engine.tick_force().await;
        }
        let idle_duty = mock.status().fan_duties[0];
        assert!(
            idle_duty < 60.0,
            "an idle GPU should not keep the fan at {idle_duty} %"
        );
    }

    #[tokio::test]
    async fn update_interval_is_respected() {
        let (_tmp, engine, mock) = engine_with_mock().await;
        mock.set_gpu_load(0.9);
        let rule = gpu_rule().with_update_interval_ms(5_000);
        engine.save_rule(rule).unwrap();
        assert_eq!(engine.tick().await.len(), 1);
        // Immediately afterwards the rule is not due yet.
        assert_eq!(engine.tick().await.len(), 0);
    }

    #[tokio::test]
    async fn sensor_loss_triggers_the_fallback() {
        let (_tmp, engine, mock) = engine_with_mock().await;
        mock.set_gpu_load(0.2);
        engine.save_rule(gpu_rule()).unwrap();
        engine.tick().await;

        engine.runtime().poll_once().await.unwrap();
        mock.set_faults(MockFaults::sensor_disconnected(
            "gpu.mock.0",
            "temperature.core",
        ));
        engine.runtime().poll_once().await.unwrap();

        let outcomes = engine.tick_force().await;
        assert_eq!(outcomes[0].status, RuleStatus::Fallback);
        assert_eq!(outcomes[0].applied_output, Some(70.0), "fail-safe duty");
        assert_eq!(mock.status().fan_duties[0], 70.0);

        // The rule recovers on its own once the sensor reports again.
        mock.set_faults(MockFaults::default());
        engine.runtime().poll_once().await.unwrap();
        let recovered = engine.tick_force().await;
        assert_eq!(recovered[0].status, RuleStatus::Applied);
    }

    /// `release` used to set a flag, publish an event and leave the fan exactly
    /// where it was. It is now refused outright, so nothing can configure it.
    #[tokio::test]
    async fn release_cannot_be_saved_or_enabled() {
        let (_tmp, engine, _mock) = engine_with_mock().await;

        let rule = gpu_rule().with_fallback(Fallback {
            on_sensor_missing: FallbackAction::Release,
            ..Fallback::default()
        });
        let check = engine.check_rule(&rule);
        assert!(!check.is_ok(), "release must be a validation error");
        assert!(
            check.errors.iter().any(|error| error.contains("release")),
            "{:?}",
            check.errors
        );
        assert!(engine.save_rule(rule).is_err());

        // The same applies to the write-failure policy.
        let rule = gpu_rule().with_fallback(Fallback {
            on_write_failure: FallbackAction::Release,
            ..Fallback::default()
        });
        assert!(engine.save_rule(rule).is_err());
    }

    /// Defence in depth: if a rule carrying `release` reaches the evaluator anyway
    /// (a hand-built rule, a future import path), the machine must still be
    /// protected, and nothing may claim a release happened.
    #[tokio::test]
    async fn a_stray_release_rule_still_protects_the_machine() {
        let (_tmp, engine, mock) = engine_with_mock().await;
        mock.set_gpu_load(0.2);
        let rule = gpu_rule().with_fallback(Fallback {
            on_sensor_missing: FallbackAction::Release,
            ..Fallback::default()
        });
        {
            // Bypass validation on purpose, which is the only way this is reachable.
            let mut rules = engine.inner.rules.write();
            rules.push(rule.clone());
        }
        engine.runtime().poll_once().await.unwrap();
        engine.tick_force().await;

        // The sensor disappears.
        mock.set_faults(MockFaults::sensor_disconnected(
            "gpu.mock.0",
            "temperature.core",
        ));
        engine.runtime().poll_once().await.unwrap();
        engine.tick_force().await;

        let outcome = engine.outcome("test-gpu").unwrap();
        assert_ne!(
            outcome.status,
            RuleStatus::Released,
            "nothing was released, so nothing may say it was"
        );
        assert_eq!(outcome.status, RuleStatus::Fallback);
        assert!(
            outcome.message.contains("not supported"),
            "the message must say release is unsupported: {}",
            outcome.message
        );
        assert_eq!(
            outcome.applied_output,
            Some(70.0),
            "the fail-safe duty must actually have been applied"
        );
        assert_eq!(mock.status().fan_duties[0], 70.0, "the fan really moved");
    }

    #[tokio::test]
    async fn rules_persist_and_reload() {
        let (tmp, engine, _mock) = engine_with_mock().await;
        engine.save_rule(gpu_rule()).unwrap();
        assert!(tmp.path().join("rules/test-gpu.yaml").exists());

        let runtime = engine.runtime().clone();
        let reloaded = AutomationEngine::new(runtime, RuleStore::new(tmp.path().join("rules")));
        let report = reloaded.load_rules().unwrap();
        assert_eq!(report.rules.len(), 1);
        assert_eq!(report.rules[0].id.as_str(), "test-gpu");
        assert!(report.is_clean());
    }

    #[tokio::test]
    async fn enable_disable_and_delete() {
        let (_tmp, engine, mock) = engine_with_mock().await;
        mock.set_gpu_load(1.0);
        mock.tick(60_000);
        engine.save_rule(gpu_rule()).unwrap();
        assert_eq!(engine.rule_count(), 1);

        engine.set_rule_enabled("test-gpu", false).unwrap();
        assert!(engine.tick().await.is_empty());
        assert_eq!(
            engine.outcome("test-gpu").unwrap().status,
            RuleStatus::Disabled
        );
        assert!(!engine.rule("test-gpu").unwrap().enabled);

        engine.set_rule_enabled("test-gpu", true).unwrap();
        assert_eq!(engine.tick().await.len(), 1);

        assert!(engine.delete_rule("test-gpu").unwrap());
        assert_eq!(engine.rule_count(), 0);
        assert!(!engine.delete_rule("test-gpu").unwrap());
        assert!(engine.set_rule_enabled("test-gpu", true).is_err());
        assert!(engine.outcome("test-gpu").is_none());
    }

    #[tokio::test]
    async fn automation_can_be_switched_off_globally() {
        let (_tmp, engine, _mock) = engine_with_mock().await;
        engine.save_rule(gpu_rule()).unwrap();
        engine
            .runtime()
            .update_settings(|s| s.automation_enabled = false)
            .unwrap();
        assert!(engine.tick().await.is_empty());
    }

    #[tokio::test]
    async fn stale_runtime_forces_the_fallback_path() {
        let (_tmp, engine, mock) = engine_with_mock().await;
        mock.set_gpu_load(0.5);
        engine
            .runtime()
            .update_settings(|settings| settings.polling_interval_ms = 100)
            .unwrap();
        engine.save_rule(gpu_rule()).unwrap();
        engine.runtime().poll_once().await.unwrap();
        engine.tick().await;

        // Stop polling entirely: the cached value goes stale and the rule must
        // stop trusting it.
        tokio::time::sleep(Duration::from_millis(450)).await;
        assert!(engine.runtime().is_stale(3));
        let outcomes = engine.tick_force().await;
        assert_eq!(outcomes[0].status, RuleStatus::Fallback);
        assert_eq!(outcomes[0].applied_output, Some(70.0));
    }

    #[tokio::test]
    async fn suggestions_match_the_attached_hardware() {
        let (_tmp, engine, _mock) = engine_with_mock().await;
        let suggestions = crate::examples::suggest_for(engine.runtime());
        assert_eq!(suggestions.len(), 3);
        for rule in &suggestions {
            assert!(rule.validate().is_ok());
            let check = engine.check_rule(rule);
            assert!(check.is_ok(), "{:?}", check.errors);
        }
        assert_eq!(suggestions[0].id.as_str(), "gpu-cooling");
        assert!(
            suggestions
                .iter()
                .any(|r| r.source.aggregate() == Some(Aggregate::Max))
        );

        // Merging skips ids that are already in use.
        let merged = merge_suggestions(&suggestions, suggestions.clone());
        assert!(merged.is_empty());
    }

    #[tokio::test]
    async fn ensure_example_installs_something_usable() {
        let (_tmp, engine, _mock) = engine_with_mock().await;
        assert!(engine.ensure_example().unwrap());
        assert_eq!(engine.rule_count(), 1);
        assert!(!engine.ensure_example().unwrap());
        assert_eq!(engine.rule_count(), 1);
    }

    #[tokio::test]
    async fn combined_source_uses_the_hottest_sensor() {
        let (_tmp, engine, mock) = engine_with_mock().await;
        mock.set_gpu_load(1.0);
        mock.set_cpu_load(0.05);
        mock.tick(120_000);

        let rule = Rule::new(
            "combined",
            "Combined",
            Source::combined(
                Aggregate::Max,
                vec![
                    crate::rule::SensorRef::new("cpu.mock.0", "temperature.core"),
                    crate::rule::SensorRef::new("gpu.mock.0", "temperature.core"),
                ],
            ),
            Target::new("fan.mock.0", "fan.speed_percent"),
            crate::curve::gpu_cooling_curve(),
        )
        .unwrap()
        .with_deadband(0.0);
        engine.save_rule(rule).unwrap();

        engine.runtime().poll_once().await.unwrap();
        let outcomes = engine.tick().await;
        let hottest = mock.status().gpu_temp_c;
        assert!(
            outcomes[0].input.unwrap() >= hottest - 1.0,
            "the MAX source should follow the GPU ({:?} vs {hottest})",
            outcomes[0].input
        );
    }

    #[tokio::test]
    async fn engine_start_and_stop_are_safe() {
        let (_tmp, engine, mock) = engine_with_mock().await;
        mock.set_gpu_load(0.6);
        engine.ensure_example().unwrap();
        engine.start().await.unwrap();
        assert!(engine.is_running());
        tokio::time::sleep(Duration::from_millis(250)).await;
        engine.stop().await;
        assert!(!engine.is_running());
        assert!(engine.stats().ticks >= 1);

        // Starting twice, then stopping twice, must not panic.
        engine.start().await.unwrap();
        engine.stop().await;
        engine.stop().await;
    }

    #[tokio::test]
    async fn pump_rules_respect_the_safety_floor() {
        let (tmp, engine, mock) = engine_with_mock().await;
        // Add a pump to the simulated machine.
        mock.update_config(|config| config.devices.pumps = 1);
        engine.runtime().refresh_devices().await.unwrap();
        assert!(engine.runtime().device("pump.mock.0").is_some());

        let rule = Rule::new(
            "pump-rule",
            "Pump",
            Source::sensor("gpu.mock.0", "temperature.core"),
            Target::new("pump.mock.0", "pump.speed_percent"),
            crate::curve::Curve::expect([(20.0, 0.0), (90.0, 100.0)]),
        )
        .unwrap()
        .with_deadband(0.0);
        engine.save_rule(rule).unwrap();

        let outcomes = engine.tick().await;
        assert!(
            outcomes[0].applied_output.unwrap() >= 60.0,
            "the pump must never be driven below its floor: {:?}",
            outcomes[0].applied_output
        );
        let _ = tmp;
    }

    #[tokio::test]
    async fn rule_for_a_missing_device_reports_an_error_without_panicking() {
        let (_tmp, engine, _mock) = engine_with_mock().await;
        let rule = Rule {
            id: RuleId::new("ghost").unwrap(),
            target: Target::new("fan.ghost.0", "fan.speed_percent"),
            ..gpu_rule()
        };
        // Bypass validation to simulate hardware disappearing after the fact.
        {
            let mut rules = engine.inner.rules.write();
            rules.push(rule.clone());
        }
        let outcomes = engine.tick().await;
        assert_eq!(outcomes[0].status, RuleStatus::Error);
        assert!(outcomes[0].message.contains("unavailable"));
    }

    #[tokio::test]
    async fn stats_are_aggregated() {
        let (_tmp, engine, mock) = engine_with_mock().await;
        mock.set_gpu_load(0.7);
        engine.save_rule(gpu_rule()).unwrap();
        engine.tick().await;
        engine.tick().await;
        let stats = engine.stats();
        assert_eq!(stats.rules, 1);
        assert_eq!(stats.enabled_rules, 1);
        assert_eq!(stats.writes, 1);
        assert_eq!(stats.ticks, 2);
        assert!(stats.last_tick_ms > 0);
        assert_eq!(aggregate_label(Aggregate::Avg), "avg");
        assert_eq!(
            crate::rule::Source::sensor("gpu.mock.0", "temperature.core").sensors()[0].device,
            ohm_core::DeviceId::new("gpu.mock.0").unwrap()
        );
        assert_eq!(
            engine
                .runtime()
                .device("gpu.mock.0")
                .unwrap()
                .device
                .device_type,
            DeviceType::Gpu
        );
        assert_eq!(LoadProfile::default(), LoadProfile::Constant { load: 0.05 });
    }

    // ------------------------------------------------- conflict enforcement
    fn second_rule_on_the_same_target(id: &str) -> Rule {
        Rule::new(
            id,
            "Second Fan Rule",
            Source::sensor("gpu.mock.0", "temperature.core"),
            Target::new("fan.mock.0", "fan.speed_percent"),
            crate::curve::gpu_cooling_curve(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn a_second_enabled_rule_on_one_output_is_refused() {
        let (_tmp, engine, _mock) = engine_with_mock().await;
        engine.save_rule(gpu_rule()).unwrap();

        let error = engine
            .save_rule(second_rule_on_the_same_target("second"))
            .expect_err("a second writer on one output must be refused");
        let message = error.to_string();
        assert!(message.contains("Test GPU"), "{message}");
        assert!(
            message.contains("fan.mock.0/fan.speed_percent"),
            "{message}"
        );
        assert!(message.contains("disable"), "{message}");
        assert_eq!(engine.rule_count(), 1, "nothing may have been saved");

        // The same clash is reported by `check_rule`, so the form can warn first.
        let check = engine.check_rule(&second_rule_on_the_same_target("second"));
        assert!(!check.is_ok());
        assert!(check.errors.iter().any(|e| e.contains("already owns")));
    }

    #[tokio::test]
    async fn a_disabled_rule_does_not_own_its_output() {
        let (_tmp, engine, _mock) = engine_with_mock().await;
        let mut first = gpu_rule();
        first.enabled = false;
        engine.save_rule(first).unwrap();

        // The target is free, so another enabled rule may take it.
        engine
            .save_rule(second_rule_on_the_same_target("second"))
            .expect("a disabled rule owns nothing");
        assert_eq!(engine.rule_count(), 2);
        assert!(engine.conflicts().is_empty(), "no live conflict");
    }

    #[tokio::test]
    async fn enabling_a_conflicting_rule_is_refused() {
        let (_tmp, engine, _mock) = engine_with_mock().await;
        engine.save_rule(gpu_rule()).unwrap();
        let mut second = second_rule_on_the_same_target("second");
        second.enabled = false;
        engine.save_rule(second).unwrap();

        let error = engine
            .set_rule_enabled("second", true)
            .expect_err("enabling a conflicting rule must be refused");
        assert!(error.to_string().contains("already owns"));
        assert!(!engine.rule("second").unwrap().enabled, "still disabled");
    }

    #[tokio::test]
    async fn rules_loaded_from_files_resolve_conflicts_and_say_so() {
        let (_tmp, engine, _mock) = engine_with_mock().await;
        // Two hand-written files pointing at the same fan, bypassing the form.
        let mut winner = gpu_rule();
        winner.priority = 10;
        let mut loser = second_rule_on_the_same_target("loser");
        loser.enabled = true;
        engine.store().save(&winner).unwrap();
        engine.store().save(&loser).unwrap();

        let report = engine.load_rules().unwrap();
        assert_eq!(report.rules.len(), 2, "both files still load");

        // Highest priority keeps the output; the other is stood down in memory.
        assert!(engine.rule("test-gpu").unwrap().enabled);
        assert!(!engine.rule("loser").unwrap().enabled);
        let conflicts = engine.conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].owner_id.as_str(), "test-gpu");
        assert_eq!(conflicts[0].blocked_id.as_str(), "loser");
        assert!(conflicts[0].resolution.contains("the file is untouched"));
        assert!(
            conflicts[0]
                .message()
                .contains("fan.mock.0/fan.speed_percent")
        );

        // The file itself is untouched, so the user can still fix it by hand.
        assert!(engine.store().exists(&RuleId::new("loser").unwrap()));
    }

    #[tokio::test]
    async fn a_conflict_reaching_the_active_set_is_not_written_twice_in_a_cycle() {
        let (_tmp, engine, mock) = engine_with_mock().await;
        mock.set_gpu_load(1.0);
        engine.save_rule(gpu_rule()).unwrap();
        // Poll once so the runtime is not stale: a stale runtime falls back on
        // purpose (see `stale_runtime_forces_the_fallback_path`), which would
        // make this test pass for the wrong reason.
        engine.runtime().poll_once().await.unwrap();

        // Simulate an entry point that bypasses the guard (a future import path,
        // an older file loaded straight into the active set).
        {
            let mut rules = engine.inner.rules.write();
            rules.push(second_rule_on_the_same_target("sneaky"));
        }

        let outcomes = engine.tick_force().await;
        let sneaky = outcomes
            .iter()
            .find(|o| o.rule_id.as_str() == "sneaky")
            .expect("the sneaky rule is reported, not silently ignored");
        assert_eq!(sneaky.status, RuleStatus::Error);
        assert!(
            sneaky.message.contains("already drove"),
            "{}",
            sneaky.message
        );
        assert_eq!(
            sneaky.writes, 0,
            "the second writer must not touch the output"
        );
        // The output carries exactly what the one legitimate writer asked for.
        let owner = outcomes
            .iter()
            .find(|o| o.rule_id.as_str() == "test-gpu")
            .expect("the owning rule is present");
        assert!(owner.writes > 0, "the owner wrote");
        assert_eq!(
            mock.status().fan_duties[0],
            owner.applied_output.unwrap(),
            "one writer, one value"
        );
    }

    #[tokio::test]
    async fn forced_tick_ignores_the_schedule() {
        let (_tmp, engine, mock) = engine_with_mock().await;
        mock.set_gpu_load(0.9);
        engine
            .save_rule(gpu_rule().with_update_interval_ms(60_000))
            .unwrap();
        assert_eq!(engine.tick().await.len(), 1);
        assert!(engine.tick().await.is_empty(), "not due yet");
        assert_eq!(
            engine.tick_force().await.len(),
            1,
            "forced ticks ignore the schedule"
        );
    }
}
