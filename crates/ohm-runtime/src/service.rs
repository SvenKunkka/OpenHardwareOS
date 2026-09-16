//! Bookkeeping for the background service: one state file, one writer.
//!
//! The desktop application and a background service both drive the same channels,
//! and two processes writing one fan is the failure this module exists to prevent.
//! There is no cross-process handover in this build, so the rule is simpler and
//! stricter: **one writer at a time**, and the file says who that is.
//!
//! How "is it still running?" is answered without a platform-specific API:
//!
//! * the owner rewrites the file on a heartbeat (every second by default), so a
//!   file whose heartbeat is older than a few intervals belongs to a process that
//!   died — including one killed with `SIGKILL`, which never gets to clean up;
//! * the file records the pid for the human reading it, and on Linux `/proc/<pid>`
//!   settles the question exactly;
//! * taking over is atomic: the file is created with `create_new`, so two
//!   processes that both decide a stale file is reclaimable cannot both win.
//!
//! A clean exit removes the file, so the common case leaves nothing behind.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Missed heartbeats before a state file is treated as abandoned.
///
/// Three intervals is enough to ride out a scheduling hiccup — a suspended
/// laptop resumes well past any short window, and a service that was suspended
/// should not be considered alive while another wants the channels — but not so
/// long that a crashed service blocks a restart for minutes.
pub const HEARTBEAT_TOLERANCE: u64 = 3;

/// One rule, as the service's state file reports it.
///
/// A service whose status cannot say what the rules are *doing* is not much use to
/// the person running it: "3 rules" does not distinguish "cooling normally" from
/// "sitting in a fail-safe because a sensor went away".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceOutcome {
    pub id: String,
    /// `RuleStatus` as text (`applied`, `fallback`, `gated`, `error`, …).
    pub status: String,
    /// The value the rule last had confirmed, when it has one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub applied: Option<f64>,
    /// The rule's own sentence about what it is doing and why.
    pub message: String,
}

/// What the owning process writes about itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceState {
    /// Process id of the owner. Informational, and exact on Linux.
    pub pid: u32,
    /// The build that wrote it, so a stale file from another version is visible.
    pub version: String,
    pub started_at_ms: i64,
    /// Rewritten on every heartbeat. This is what decides liveness.
    pub heartbeat_at_ms: i64,
    pub heartbeat_interval_ms: u64,
    /// Automation cycles completed.
    pub ticks: u64,
    /// Rules the service loaded.
    pub rules: usize,
    /// What each rule is doing right now. Capped, because a state file nobody can
    /// read is not a status report.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outcomes: Vec<ServiceOutcome>,
    /// True when the simulated providers are registered.
    #[serde(default)]
    pub simulated: bool,
    /// True when writes cannot reach hardware.
    #[serde(default)]
    pub dry_run: bool,
    /// How many times this owner woke up after wall-clock time it did not run
    /// through — a suspend, a hibernation, a very long stall. Recorded because the
    /// gap is the one thing a heartbeat cannot show: while the process was frozen
    /// its own file looked abandoned, and whoever read it had to decide alone.
    #[serde(default)]
    pub resumes: u64,
    /// The most recent such gap, in wall-clock milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_resume_gap_ms: Option<i64>,
    /// Channels this owner switched away from their drivers and still owes back.
    ///
    /// Published for the same reason the heartbeat is: a process that takes over after
    /// this one dies inherits its channels, and "which ones, and what were they before"
    /// is knowledge that dies with it. [`crate::Runtime::taken_controls`] is where the
    /// list comes from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub taken: Vec<ohm_adapter_api::TakenControl>,
}

impl ServiceState {
    /// A claim for this process, with nothing recorded yet.
    pub fn claim(
        pid: u32,
        version: impl Into<String>,
        heartbeat_interval_ms: u64,
        simulated: bool,
        dry_run: bool,
    ) -> Self {
        let now = ohm_core::now_ms();
        Self {
            pid,
            version: version.into(),
            started_at_ms: now,
            heartbeat_at_ms: now,
            heartbeat_interval_ms: heartbeat_interval_ms.max(50),
            ticks: 0,
            rules: 0,
            outcomes: Vec::new(),
            simulated,
            dry_run,
            resumes: 0,
            last_resume_gap_ms: None,
            taken: Vec::new(),
        }
    }

    /// Age of the last heartbeat.
    pub fn age_ms(&self, now_ms: i64) -> i64 {
        now_ms.saturating_sub(self.heartbeat_at_ms)
    }

    /// Has this owner written a heartbeat recently enough to count as alive?
    pub fn is_fresh(&self, now_ms: i64) -> bool {
        let window = self
            .heartbeat_interval_ms
            .max(1)
            .saturating_mul(HEARTBEAT_TOLERANCE) as i64;
        self.age_ms(now_ms) <= window
    }
}

/// A wake-up that spanned wall-clock time the process did not run through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuspendGap {
    /// Wall-clock milliseconds between the two observations.
    pub wall_ms: i64,
    /// Milliseconds the process itself measured between them.
    pub process_ms: i64,
}

impl SuspendGap {
    /// The part of the gap the process did not live through.
    pub fn missed_ms(&self) -> i64 {
        (self.wall_ms - self.process_ms).max(0)
    }

    /// A sentence for the log, naming both numbers: the size of the gap is the
    /// evidence, and a reader needs to see that the two clocks disagreed.
    pub fn describe(&self) -> String {
        format!(
            "woke up after {} of wall-clock time in which this process measured only {} — a \
             suspend, a hibernation or a very long stall; while it was frozen its state file \
             looked abandoned to anything that read it",
            crate::release::format_gap(self.wall_ms),
            crate::release::format_gap(self.process_ms)
        )
    }
}

/// Notices that the process was not running for a stretch of wall-clock time.
///
/// Two clocks, deliberately: wall-clock time keeps moving while a machine is
/// suspended, and the process's own monotonic clock does not (on Linux and macOS it
/// stops with the machine). Their disagreement *is* the detection — a process that
/// was merely stopped with `SIGSTOP`, or starved of CPU, sees both clocks advance
/// together and is not reported here, because from its own point of view no time was
/// lost.
///
/// What the caller does with it matters more than the detection: before writing
/// anything again it must ask who owns the channels now. While the process was
/// frozen its heartbeat aged past the window, and another process is entitled to
/// have taken over.
#[derive(Debug, Clone)]
pub struct WakeupDetector {
    threshold_ms: i64,
    last_wall_ms: Option<i64>,
    last_process_ms: Option<i64>,
}

impl WakeupDetector {
    /// `threshold_ms` is how far the two clocks must disagree before it counts.
    pub fn new(threshold_ms: i64) -> Self {
        Self {
            threshold_ms: threshold_ms.max(0),
            last_wall_ms: None,
            last_process_ms: None,
        }
    }

    /// Feed one observation of both clocks; the first one only establishes a baseline.
    pub fn observe(&mut self, wall_ms: i64, process_ms: i64) -> Option<SuspendGap> {
        let previous = self.last_wall_ms.zip(self.last_process_ms);
        self.last_wall_ms = Some(wall_ms);
        self.last_process_ms = Some(process_ms);

        let (last_wall_ms, last_process_ms) = previous?;
        let wall = wall_ms.saturating_sub(last_wall_ms);
        let process = process_ms.saturating_sub(last_process_ms);
        if wall.saturating_sub(process) >= self.threshold_ms {
            return Some(SuspendGap {
                wall_ms: wall,
                process_ms: process,
            });
        }
        None
    }
}

/// Why a service could not start.
#[derive(Debug)]
pub enum ServiceError {
    /// Another process holds the state file and is still writing heartbeats.
    AlreadyRunning {
        state: Box<ServiceState>,
        age_ms: i64,
    },
    /// The file could not be written, with the path that failed.
    Write {
        path: PathBuf,
        error: std::io::Error,
    },
    /// This process no longer owns the state file: another one took the channels
    /// while this one was not running — a suspend, a long stall, a debugger. It is
    /// the one thing a writer must never do anything about except stop: the file is
    /// now that process's record, and the hardware is its responsibility.
    OwnershipLost {
        /// The pid this process claimed with.
        pid: u32,
        /// Whoever holds the file now, when it can be read.
        found: Option<Box<ServiceState>>,
    },
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning { state, age_ms } => write!(
                f,
                "a service is already running (pid {}, started {}, last heartbeat {} ms ago)",
                state.pid,
                crate::release::format_ms(state.started_at_ms),
                age_ms
            ),
            Self::Write { path, error } => {
                write!(f, "could not write {}: {error}", path.display())
            }
            Self::OwnershipLost { pid, found } => match found {
                Some(other) => write!(
                    f,
                    "another process took over the channels (pid {} started {}; this one is pid {}), so this one stopped instead of writing over its record or onto the same channels",
                    other.pid,
                    crate::release::format_ms(other.started_at_ms),
                    pid
                ),
                None => write!(
                    f,
                    "the state file this process (pid {}) claimed is gone, so it no longer owns the channels and stopped rather than claim them again silently",
                    pid
                ),
            },
        }
    }
}

impl std::error::Error for ServiceError {}

/// Read the state file, if it exists and parses.
///
/// A file that cannot be parsed is *not* an error here: it is reported as absent,
/// and the next `acquire` takes it over. The alternative — refusing to start
/// because a file is corrupt — would leave a machine that cannot run its rules
/// until somebody deletes a file by hand.
pub fn read_state(path: &Path) -> Option<ServiceState> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// The state of a service that is running now, if one is.
///
/// "Running" is: a state file exists, its heartbeat is fresh, and — on Linux,
/// where the answer is cheap and exact — the process it names still exists.
pub fn running(path: &Path) -> Option<ServiceState> {
    let state = read_state(path)?;
    let now = ohm_core::now_ms();
    if !state.is_fresh(now) {
        return None;
    }
    if !pid_plausible(state.pid) {
        return None;
    }
    Some(state)
}

/// The state file's owner, whatever its age — for `service status`, which has to
/// be able to say "there is a file here from pid 1234, 40 minutes old".
pub fn stale_or_running(path: &Path) -> Option<(ServiceState, bool)> {
    let state = read_state(path)?;
    let fresh = state.is_fresh(ohm_core::now_ms()) && pid_plausible(state.pid);
    Some((state, fresh))
}

/// Is a process with this pid present?
///
/// On Linux this is exact. Elsewhere the heartbeat alone decides, which is
/// deliberate: guessing wrong in the direction of "alive" would block a restart,
/// and guessing wrong towards "dead" would let two processes drive one fan — so
/// only a platform that can answer for certain is asked.
fn pid_plausible(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

/// The owner of the state file while it lives, and the heartbeats it writes.
#[derive(Debug)]
pub struct ServiceGuard {
    path: PathBuf,
    state: ServiceState,
    /// The record this process replaced, when it took over an abandoned file.
    ///
    /// Kept because it carries the one thing the successor cannot work out for itself:
    /// which channels the previous owner switched away from their drivers. See
    /// [`ServiceGuard::predecessor`].
    predecessor: Option<ServiceState>,
}

impl ServiceGuard {
    /// Claim the state file, or explain who already has it.
    pub fn acquire(path: &Path, state: ServiceState) -> Result<Self, ServiceError> {
        let mut predecessor = None;
        if let Some((existing, fresh)) = stale_or_running(path) {
            if fresh {
                let age = existing.age_ms(ohm_core::now_ms());
                return Err(ServiceError::AlreadyRunning {
                    state: Box::new(existing),
                    age_ms: age,
                });
            }
            predecessor = Some(existing);
            // Abandoned: the owner stopped writing heartbeats and never cleaned
            // up (killed with SIGKILL, a power cut, a suspended machine that was
            // restarted). Removing it is not enough on its own — two processes
            // can reach this line together — so the create below is the decision.
            let _ = fs::remove_file(path);
        } else if path.exists() {
            // Present but unreadable: no process can be claiming it, because a
            // claimant writes valid JSON before it does anything else. Left in
            // place it would block every future start until somebody deleted a
            // file by hand.
            let _ = fs::remove_file(path);
        }
        Self::create(path, state, predecessor)
    }

    /// The abandoned record this process replaced, if any.
    ///
    /// A successor is expected to adopt its `taken` list before it writes anything: a
    /// channel the previous owner switched stays switched, and reading its current
    /// value as "the original" is how a fan is left in manual mode for good.
    pub fn predecessor(&self) -> Option<&ServiceState> {
        self.predecessor.as_ref()
    }

    fn create(
        path: &Path,
        state: ServiceState,
        predecessor: Option<ServiceState>,
    ) -> Result<Self, ServiceError> {
        if let Some(parent) = path.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            return Err(ServiceError::Write {
                path: parent.to_path_buf(),
                error,
            });
        }
        // `create_new` is atomic on every platform this build targets: exactly one
        // of two racing processes gets the file, and the other gets `AlreadyExists`.
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut file) => {
                let text = serde_json::to_string_pretty(&state).unwrap_or_else(|_| "{}".into());
                if let Err(error) = file.write_all(text.as_bytes()) {
                    let _ = fs::remove_file(path);
                    return Err(ServiceError::Write {
                        path: path.to_path_buf(),
                        error,
                    });
                }
                Ok(Self {
                    path: path.to_path_buf(),
                    state,
                    predecessor,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = read_state(path);
                Err(match existing {
                    Some(state) => {
                        let age_ms = state.age_ms(ohm_core::now_ms());
                        ServiceError::AlreadyRunning {
                            state: Box::new(state),
                            age_ms,
                        }
                    }
                    None => ServiceError::Write {
                        path: path.to_path_buf(),
                        error,
                    },
                })
            }
            Err(error) => Err(ServiceError::Write {
                path: path.to_path_buf(),
                error,
            }),
        }
    }

    /// Does the file this process claimed still belong to it?
    ///
    /// The heartbeat window is deliberately shorter than a suspend: a machine that
    /// slept for an hour must not keep the channels reserved while somebody else
    /// wants them. That decision is only safe if the owner checks the answer when it
    /// wakes — otherwise the woken process rewrites the file (and drives the same
    /// fan) as if nothing had happened, which is how a machine ends up with two
    /// services alternating values onto one header.
    pub fn verify_ownership(&self) -> Result<(), ServiceError> {
        match read_state(&self.path) {
            Some(current) if current.pid == self.state.pid => Ok(()),
            found => Err(ServiceError::OwnershipLost {
                pid: self.state.pid,
                found: found.map(Box::new),
            }),
        }
    }

    /// Publish what this owner has switched away from its drivers.
    pub fn set_taken(&mut self, taken: Vec<ohm_adapter_api::TakenControl>) {
        self.state.taken = taken;
    }

    /// Record a wake-up that spanned wall-clock time this process did not run.
    pub fn record_resume(&mut self, gap_ms: i64) {
        self.state.resumes = self.state.resumes.saturating_add(1);
        self.state.last_resume_gap_ms = Some(gap_ms);
    }

    /// Rewrite the file with the current counters.
    ///
    /// Written through a temporary file and renamed, so a reader never sees half a
    /// state file: `service status` may be run at any moment. The ownership check
    /// comes first and is not optional: a rename replaces whatever is there, so
    /// without it this call is how one service deletes another's record.
    pub fn heartbeat(
        &mut self,
        ticks: u64,
        rules: usize,
        outcomes: Vec<ServiceOutcome>,
    ) -> Result<(), ServiceError> {
        self.verify_ownership()?;
        self.state.ticks = ticks;
        self.state.rules = rules;
        self.state.outcomes = outcomes;
        self.state.heartbeat_at_ms = ohm_core::now_ms();
        let text = serde_json::to_string_pretty(&self.state).unwrap_or_else(|_| "{}".into());
        let temporary = self.path.with_extension("json.tmp");
        let write = || -> std::io::Result<()> {
            fs::write(&temporary, text.as_bytes())?;
            fs::rename(&temporary, &self.path)
        };
        write().map_err(|error| ServiceError::Write {
            path: self.path.clone(),
            error,
        })
    }

    pub fn state(&self) -> &ServiceState {
        &self.state
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ServiceGuard {
    fn drop(&mut self) {
        // A clean stop leaves nothing behind. A crash leaves the file, and the
        // heartbeat makes it recognisable as abandoned.
        //
        // Only this process's own record is removed. A process that lost ownership
        // still drops this value on its way out, and deleting the file then would
        // erase the *new* owner's record — leaving its channels claimed by nobody.
        if self.verify_ownership().is_ok() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// The pid these fixtures name.
    ///
    /// Our own: liveness is checked for real on Linux (`/proc/<pid>`), so a made-up
    /// number makes a "fresh" state file look abandoned there and the test passes for
    /// the wrong reason — which is exactly what happened on CI while macOS, where the
    /// check is deliberately skipped, was green.
    fn live_pid() -> u32 {
        std::process::id()
    }

    fn state(pid: u32, interval_ms: u64) -> ServiceState {
        ServiceState::claim(pid, "0.0.0", interval_ms, true, true)
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ohm-service-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir.join("service.json")
    }

    #[test]
    fn a_fresh_state_file_means_the_service_is_running() {
        let path = temp_path("fresh");
        let now = ohm_core::now_ms();
        let mut written = state(live_pid(), 1_000);
        written.heartbeat_at_ms = now;
        fs::write(&path, serde_json::to_string(&written).unwrap()).unwrap();

        let found = running(&path).expect("running");
        assert_eq!(found.pid, live_pid());
        assert_eq!(found.ticks, 0);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_old_heartbeat_means_the_owner_is_gone() {
        let path = temp_path("stale");
        let mut written = state(4243, 1_000);
        written.heartbeat_at_ms = ohm_core::now_ms() - 10_000;
        fs::write(&path, serde_json::to_string(&written).unwrap()).unwrap();
        assert!(
            running(&path).is_none(),
            "a 10 s old heartbeat is not alive"
        );
        let (found, live) = stale_or_running(&path).expect("a file is there");
        assert!(!live);
        assert_eq!(found.pid, 4243);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_second_service_is_refused_and_told_who_holds_the_file() {
        let path = temp_path("second");
        let first = ServiceGuard::acquire(&path, state(live_pid(), 1_000)).expect("first acquires");
        // The second attempt is from this same process, and is still refused: the
        // rule is about the *file*, not about who is asking.
        match ServiceGuard::acquire(&path, state(live_pid(), 1_000)) {
            Err(ServiceError::AlreadyRunning { state, .. }) => assert_eq!(state.pid, live_pid()),
            other => panic!("expected a refusal, got {other:?}"),
        }
        // And the file still belongs to the first one.
        assert_eq!(read_state(&path).unwrap().pid, live_pid());
        drop(first);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_abandoned_file_is_taken_over() {
        let path = temp_path("takeover");
        let mut dead = state(live_pid(), 1_000);
        dead.heartbeat_at_ms = ohm_core::now_ms() - 60_000;
        fs::write(&path, serde_json::to_string(&dead).unwrap()).unwrap();

        let guard = ServiceGuard::acquire(&path, state(live_pid(), 1_000)).expect("takeover");
        assert_eq!(guard.state().pid, live_pid());
        assert_eq!(read_state(&path).unwrap().pid, live_pid());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_corrupt_file_does_not_block_the_service() {
        let path = temp_path("corrupt");
        fs::write(&path, b"{ this is not json").unwrap();
        let guard = ServiceGuard::acquire(&path, state(live_pid(), 1_000)).expect("takeover");
        assert_eq!(guard.state().pid, live_pid());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn heartbeats_move_the_file_forward_and_a_clean_stop_removes_it() {
        let path = temp_path("heartbeat");
        let mut guard = ServiceGuard::acquire(&path, state(8001, 1_000)).expect("acquire");
        let first = read_state(&path).expect("written");
        guard
            .heartbeat(
                7,
                2,
                vec![ServiceOutcome {
                    id: "chassis".into(),
                    status: "applied".into(),
                    applied: Some(64.0),
                    message: "1200.0 -> 64 %".into(),
                }],
            )
            .expect("heartbeat");
        let second = read_state(&path).expect("still written");
        assert_eq!(second.ticks, 7);
        assert_eq!(second.rules, 2);
        assert_eq!(second.outcomes.len(), 1);
        assert_eq!(second.outcomes[0].status, "applied");
        assert_eq!(second.outcomes[0].applied, Some(64.0));
        assert!(second.heartbeat_at_ms >= first.heartbeat_at_ms);
        assert!(second.is_fresh(ohm_core::now_ms()));
        let path_copy = guard.path().to_path_buf();
        drop(guard);
        assert!(!path_copy.exists(), "a clean stop leaves no state file");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_heartbeat_window_follows_the_interval_it_was_written_with() {
        let now = ohm_core::now_ms();
        let mut written = state(9001, 1_000);
        written.heartbeat_at_ms = now - 2_500;
        assert!(written.is_fresh(now), "2.5 s is inside a 3 s window");
        written.heartbeat_interval_ms = 10_000;
        assert!(written.is_fresh(now), "a 30 s window tolerates it");
        written.heartbeat_interval_ms = 100;
        assert!(!written.is_fresh(now), "a 300 ms window does not");
    }

    // --- identity across a wake-up, and across a takeover ------------------------

    #[test]
    fn a_suspend_is_detected_from_the_two_clocks_disagreeing() {
        let mut detector = WakeupDetector::new(5_000);
        assert_eq!(
            detector.observe(1_000_000, 10_000),
            None,
            "the first call is a baseline"
        );

        // Eight hours of wall-clock time, one second of process time: the machine
        // slept, and this process did not run through those eight hours.
        let gap = detector
            .observe(1_000_000 + 8 * 3_600_000, 11_000)
            .expect("a suspend");
        assert_eq!(gap.wall_ms, 28_800_000);
        assert_eq!(gap.process_ms, 1_000);
        assert_eq!(gap.missed_ms(), 28_799_000);
        assert!(gap.describe().contains("8 h 0 min"), "{}", gap.describe());
        assert!(gap.describe().contains("1.000 s"), "{}", gap.describe());
    }

    #[test]
    fn a_process_that_was_merely_stopped_is_not_a_suspend() {
        // `SIGSTOP`, a starved container, a debugger pause: both clocks advance by
        // the same amount, because from this process's point of view no time was
        // lost. Reporting a suspend here would be a false alarm.
        let mut detector = WakeupDetector::new(5_000);
        assert_eq!(detector.observe(1_000_000, 10_000), None);
        assert_eq!(detector.observe(1_000_000 + 30_000, 40_000), None);
    }

    #[test]
    fn ordinary_ticks_are_not_a_suspend() {
        let mut detector = WakeupDetector::new(1_000);
        assert_eq!(detector.observe(0, 0), None);
        for step in 1..10 {
            assert_eq!(detector.observe(step * 200, step * 200), None);
        }
        // A brief disagreement below the threshold is scheduling noise, not a wake-up.
        assert_eq!(detector.observe(2_000 + 500, 1_800 + 500), None);
    }

    #[test]
    fn a_gap_that_never_happened_is_not_negative() {
        let gap = SuspendGap {
            wall_ms: 1_000,
            process_ms: 4_000,
        };
        assert_eq!(gap.missed_ms(), 0);
    }

    #[test]
    fn a_heartbeat_that_lost_ownership_does_not_write_over_the_new_owner() {
        let path = temp_path("lost").join("service.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut guard = ServiceGuard::acquire(&path, state(1, 100)).expect("claim");

        // Another process took the channels while this one was not running.
        let successor = ServiceState::claim(2, "9.9.9", 100, true, true);
        fs::write(&path, serde_json::to_string_pretty(&successor).unwrap()).unwrap();
        let before = fs::read_to_string(&path).unwrap();

        let error = guard
            .heartbeat(7, 3, Vec::new())
            .expect_err("ownership is gone");
        match &error {
            ServiceError::OwnershipLost { pid, found } => {
                assert_eq!(*pid, 1);
                assert_eq!(found.as_ref().map(|state| state.pid), Some(2));
            }
            other => panic!("expected OwnershipLost, got {other:?}"),
        }
        assert!(
            error.to_string().contains("took over the channels"),
            "{error}"
        );
        // The file is the successor's record, byte for byte: a rename would have
        // replaced it, and `service status` would then report the wrong owner.
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn a_deleted_state_file_is_lost_ownership_rather_than_a_new_claim() {
        let path = temp_path("deleted").join("service.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut guard = ServiceGuard::acquire(&path, state(1, 100)).expect("claim");
        fs::remove_file(&path).unwrap();

        let error = guard
            .heartbeat(1, 1, Vec::new())
            .expect_err("the file is gone");
        match &error {
            ServiceError::OwnershipLost { found, .. } => assert!(found.is_none(), "{found:?}"),
            other => panic!("expected OwnershipLost, got {other:?}"),
        }
        // Re-claiming silently would be the dangerous half of this: the process
        // cannot know whether the file was removed by a hand or by a successor that
        // has not written its own yet.
        assert!(!path.exists(), "a heartbeat must not recreate the file");
    }

    #[test]
    fn dropping_a_guard_that_lost_ownership_leaves_the_successors_file() {
        let path = temp_path("successor").join("service.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let guard = ServiceGuard::acquire(&path, state(1, 100)).expect("claim");
        let successor = ServiceState::claim(2, "9.9.9", 100, true, true);
        fs::write(&path, serde_json::to_string_pretty(&successor).unwrap()).unwrap();

        drop(guard);
        let left = read_state(&path).expect("the successor's file must survive");
        assert_eq!(left.pid, 2);
    }

    #[test]
    fn dropping_a_guard_that_still_owns_removes_its_file() {
        let path = temp_path("clean").join("service.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let guard = ServiceGuard::acquire(&path, state(1, 100)).expect("claim");
        drop(guard);
        assert!(!path.exists(), "a clean stop leaves nothing behind");
    }

    #[test]
    fn a_resume_is_counted_and_persisted_for_the_next_reader() {
        let path = temp_path("resume").join("service.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut guard = ServiceGuard::acquire(&path, state(1, 100)).expect("claim");
        guard.record_resume(28_800_000);
        guard.record_resume(1_500);
        guard.heartbeat(2, 1, Vec::new()).expect("still the owner");

        let written = read_state(&path).expect("state file");
        assert_eq!(written.resumes, 2);
        assert_eq!(written.last_resume_gap_ms, Some(1_500));
    }

    #[test]
    fn a_state_file_from_before_this_version_reads_as_no_resumes() {
        // The fields are additive: an older owner's file must still parse, or a
        // service started after an upgrade would refuse to take over.
        let json = r#"{"pid":1,"version":"0.1.10","started_at_ms":1,"heartbeat_at_ms":2,
                       "heartbeat_interval_ms":100,"ticks":3,"rules":1}"#;
        let state: ServiceState = serde_json::from_str(json).expect("an older file parses");
        assert_eq!(state.resumes, 0);
        assert_eq!(state.last_resume_gap_ms, None);
    }

    // --- handing a switched channel to the process that takes over ----------------

    fn taken_entry() -> ohm_adapter_api::TakenControl {
        ohm_adapter_api::TakenControl {
            device_id: "fan.system.nct6798d_fan1".to_string(),
            original: Some(2),
            original_text: "the driver controls this channel".to_string(),
        }
    }

    #[test]
    fn what_an_owner_switched_is_published_for_whoever_comes_next() {
        let path = temp_path("taken").join("service.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut guard = ServiceGuard::acquire(&path, state(1, 100)).expect("claim");
        guard.set_taken(vec![taken_entry()]);
        guard.heartbeat(1, 1, Vec::new()).expect("heartbeat");

        let written = read_state(&path).expect("state file");
        assert_eq!(written.taken, vec![taken_entry()]);
    }

    #[test]
    fn a_successor_is_given_its_predecessors_record() {
        let path = temp_path("predecessor").join("service.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        // A first owner that switched a channel and then died: stale heartbeat, record
        // intact.
        let mut dead = ServiceState::claim(1, "0.1.10", 100, false, false);
        dead.taken = vec![taken_entry()];
        dead.heartbeat_at_ms = ohm_core::now_ms() - 600_000;
        fs::write(&path, serde_json::to_string_pretty(&dead).unwrap()).unwrap();

        let guard = ServiceGuard::acquire(&path, state(2, 100)).expect("take over");
        let predecessor = guard.predecessor().expect("the record it replaced");
        assert_eq!(predecessor.pid, 1);
        assert_eq!(predecessor.version, "0.1.10");
        assert_eq!(
            predecessor.taken,
            vec![taken_entry()],
            "the successor must be able to adopt what the dead owner switched"
        );

        // A fresh start has nothing to adopt, and says so by being empty rather than
        // by inventing a predecessor.
        let fresh = temp_path("fresh").join("service.json");
        fs::create_dir_all(fresh.parent().unwrap()).unwrap();
        let guard = ServiceGuard::acquire(&fresh, state(3, 100)).expect("claim");
        assert!(guard.predecessor().is_none());
    }

    #[test]
    fn a_state_file_written_before_this_version_has_nothing_to_adopt() {
        // Upgrade path: the field is additive, or an old file would either fail to
        // parse (blocking takeover) or be read as "the previous owner switched
        // something" when it switched nothing.
        let json = r#"{"pid":1,"version":"0.1.10","started_at_ms":1,"heartbeat_at_ms":2,
                       "heartbeat_interval_ms":100,"ticks":3,"rules":1}"#;
        let state: ServiceState = serde_json::from_str(json).expect("an older file parses");
        assert!(state.taken.is_empty());
    }
}
