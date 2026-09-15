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
    /// True when the simulated providers are registered.
    #[serde(default)]
    pub simulated: bool,
    /// True when writes cannot reach hardware.
    #[serde(default)]
    pub dry_run: bool,
}

impl ServiceState {
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
}

impl ServiceGuard {
    /// Claim the state file, or explain who already has it.
    pub fn acquire(path: &Path, state: ServiceState) -> Result<Self, ServiceError> {
        if let Some((existing, fresh)) = stale_or_running(path) {
            if fresh {
                let age = existing.age_ms(ohm_core::now_ms());
                return Err(ServiceError::AlreadyRunning {
                    state: Box::new(existing),
                    age_ms: age,
                });
            }
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
        Self::create(path, state)
    }

    fn create(path: &Path, state: ServiceState) -> Result<Self, ServiceError> {
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

    /// Rewrite the file with the current counters.
    ///
    /// Written through a temporary file and renamed, so a reader never sees half a
    /// state file: `service status` may be run at any moment.
    pub fn heartbeat(&mut self, ticks: u64, rules: usize) -> Result<(), ServiceError> {
        self.state.ticks = ticks;
        self.state.rules = rules;
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
        let _ = fs::remove_file(&self.path);
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
        ServiceState {
            pid,
            version: "0.0.0".into(),
            started_at_ms: ohm_core::now_ms(),
            heartbeat_at_ms: ohm_core::now_ms(),
            heartbeat_interval_ms: interval_ms,
            ticks: 0,
            rules: 0,
            simulated: true,
            dry_run: true,
        }
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
        guard.heartbeat(7, 2).expect("heartbeat");
        let second = read_state(&path).expect("still written");
        assert_eq!(second.ticks, 7);
        assert_eq!(second.rules, 2);
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
}
