//! Control responsibility that outlives the process.
//!
//! Two kinds of responsibility are written to disk, and only two:
//!
//! * **Unresolved handovers** — a channel a rule stopped driving, still owed the
//!   fail-safe duty. Without this, a crash while one is pending loses the only record
//!   that a channel is unprotected, and the rule that abandoned it may not exist any
//!   more to notice.
//! * **Issued-but-unknown writes** — a rule wrote to a channel and never learned the
//!   result (`ControlHold { confirmed: None }`). The channel may have moved; nobody
//!   verified it. That is a responsibility, not a reading.
//!
//! What is deliberately **not** written, because restoring it would be a lie:
//!
//! * confirmed values — a value read by a previous process is not evidence about now;
//! * curve positions, hysteresis anchors, gate state — the next evaluation computes
//!   them from live readings, and a stale curve position would drive a fan to a value
//!   nobody asked for today;
//! * anything at all about resolved handovers, which have no outstanding work.
//!
//! A recovered item is therefore **not** restored as if it were live. It comes back as
//! [`HandoverState::NeedsVerification`]: on the record, visible, and awaiting a fresh
//! look at the device — is it still there, is the capability still exposed, does some
//! enabled rule drive it now, does it read at all — before the safety policy is
//! applied to it. Recovery never replays an old command; it re-decides.
//!
//! The file is written atomically (temp file in the same directory, then rename), so a
//! crash mid-write cannot leave a half-record behind. A failure to write is reported
//! and surfaced — never swallowed, because silently losing this file is exactly the
//! failure it exists to prevent.

use std::path::{Path, PathBuf};

use ohm_core::{CapabilityId, DeviceId, OhmError, Result, RuleId};
use serde::{Deserialize, Serialize};

use crate::handover::HandoverState;

/// Bumped when the on-disk shape changes incompatibly. A file from a newer version is
/// refused rather than guessed at.
pub const RECOVERY_VERSION: u32 = 1;

/// The file, inside the application's config directory.
pub const RECOVERY_FILE: &str = "control-state.json";

/// One unresolved handover, as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredHandover {
    pub device: DeviceId,
    pub capability: CapabilityId,
    pub from_rule: RuleId,
    pub reason: String,
    pub state: HandoverState,
    pub attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub first_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_error: Option<String>,
    pub queued_at_ms: i64,
    pub last_attempt_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub claimant: Option<RuleId>,
    pub claimed_ticks: u64,
    pub parked_by_claim: bool,
}

/// A write that was issued and whose result was never learned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredHold {
    pub rule_id: RuleId,
    pub device: DeviceId,
    pub capability: CapabilityId,
    /// When the write was issued.
    pub last_write_ms: i64,
}

/// The whole record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveryRecord {
    pub version: u32,
    pub saved_at_ms: i64,
    #[serde(default)]
    pub handovers: Vec<StoredHandover>,
    #[serde(default)]
    pub holds: Vec<StoredHold>,
}

impl RecoveryRecord {
    pub fn new(saved_at_ms: i64) -> Self {
        Self {
            version: RECOVERY_VERSION,
            saved_at_ms,
            handovers: Vec::new(),
            holds: Vec::new(),
        }
    }

    /// Is there anything to recover?
    pub fn is_empty(&self) -> bool {
        self.handovers.is_empty() && self.holds.is_empty()
    }
}

/// Where the record lives.
#[derive(Debug, Clone)]
pub struct RecoveryStore {
    path: PathBuf,
}

impl RecoveryStore {
    /// `<config>/control-state.json`
    pub fn from_paths(paths: &ohm_core::ConfigPaths) -> Self {
        Self {
            path: paths.root().join(RECOVERY_FILE),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write the record atomically.
    ///
    /// The temp file is created in the same directory so the rename is atomic on every
    /// platform this app runs on; a crash before the rename leaves the previous record
    /// intact rather than a truncated one.
    pub fn save(&self, record: &RecoveryRecord) -> Result<()> {
        let json = serde_json::to_string_pretty(record)
            .map_err(|e| OhmError::Config(format!("could not encode the control record: {e}")))?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| OhmError::io(parent, e))?;
        }
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, json).map_err(|e| OhmError::io(&temp, e))?;
        std::fs::rename(&temp, &self.path).map_err(|e| OhmError::io(&self.path, e))?;
        Ok(())
    }

    /// Read the record.
    ///
    /// `Ok(None)` means there was nothing to read, which is a normal state, not an
    /// error. A file that exists but cannot be understood is an error the caller must
    /// *report*: silently ignoring it would hide a responsibility, and inventing one
    /// from a corrupt file would be worse.
    pub fn load(&self) -> Result<Option<RecoveryRecord>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&self.path).map_err(|e| OhmError::io(&self.path, e))?;
        if raw.trim().is_empty() {
            // An empty file is what a crash between create and write leaves behind.
            // There is nothing to recover, and that is a statement about the file, not
            // about the machine.
            return Ok(None);
        }
        let record: RecoveryRecord = serde_json::from_str(&raw).map_err(|e| {
            OhmError::Config(format!(
                "the control record at {} could not be read ({e}); no recovered control \
                 responsibility is in force — check the channels named in the audit log",
                self.path.display()
            ))
        })?;
        if record.version > RECOVERY_VERSION {
            return Err(OhmError::Config(format!(
                "the control record at {} was written by a newer version of OpenHardwareOS \
                 (record version {}, this build understands {RECOVERY_VERSION})",
                self.path.display(),
                record.version
            )));
        }
        Ok(Some(record))
    }

    /// Remove the record. Used once everything in it has been dealt with, so the file
    /// does not claim outstanding work that no longer exists.
    pub fn clear(&self) -> Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(OhmError::io(&self.path, error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, RecoveryStore) {
        let temp = tempfile::tempdir().unwrap();
        let paths = ohm_core::ConfigPaths::from_root(temp.path());
        paths.ensure().unwrap();
        (temp, RecoveryStore::from_paths(&paths))
    }

    fn record() -> RecoveryRecord {
        let mut record = RecoveryRecord::new(1_000);
        record.handovers.push(StoredHandover {
            device: DeviceId::new_unchecked("fan.mock.0"),
            capability: CapabilityId::new_unchecked("fan.speed_percent"),
            from_rule: RuleId::new_unchecked("r1"),
            reason: "rule `r1` was deleted".into(),
            state: HandoverState::Pending,
            attempts: 2,
            first_error: Some("the device refused the fail-safe duty".into()),
            last_error: None,
            queued_at_ms: 10,
            last_attempt_ms: 20,
            claimant: None,
            claimed_ticks: 0,
            parked_by_claim: false,
        });
        record.holds.push(StoredHold {
            rule_id: RuleId::new_unchecked("r2"),
            device: DeviceId::new_unchecked("fan.mock.1"),
            capability: CapabilityId::new_unchecked("fan.speed_percent"),
            last_write_ms: 30,
        });
        record
    }

    #[test]
    fn a_record_round_trips() {
        let (_temp, store) = temp_store();
        let written = record();
        store.save(&written).unwrap();
        let read = store.load().unwrap().expect("the record is there");
        assert_eq!(read, written);
        assert_eq!(read.version, RECOVERY_VERSION);
    }

    #[test]
    fn a_missing_record_is_not_an_error() {
        let (_temp, store) = temp_store();
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn an_empty_file_is_not_an_error() {
        let (_temp, store) = temp_store();
        std::fs::write(store.path(), "").unwrap();
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn a_corrupt_record_is_an_error_that_names_the_file() {
        let (_temp, store) = temp_store();
        std::fs::write(store.path(), "{ this is not json").unwrap();
        let error = store.load().unwrap_err().to_string();
        assert!(error.contains("control record"), "{error}");
        assert!(
            error.contains("control-state.json"),
            "the message must name the file: {error}"
        );
    }

    #[test]
    fn a_record_from_a_newer_version_is_refused_not_guessed_at() {
        let (_temp, store) = temp_store();
        let mut future = record();
        future.version = RECOVERY_VERSION + 1;
        std::fs::write(store.path(), serde_json::to_string(&future).unwrap()).unwrap();
        let error = store.load().unwrap_err().to_string();
        assert!(error.contains("newer version"), "{error}");
    }

    #[test]
    fn saving_replaces_atomically_and_leaves_no_temp_file() {
        let (_temp, store) = temp_store();
        store.save(&record()).unwrap();
        let mut second = record();
        second.handovers.clear();
        store.save(&second).unwrap();
        assert_eq!(store.load().unwrap().unwrap().handovers.len(), 0);
        let temp = store.path().with_extension("json.tmp");
        assert!(
            !temp.exists(),
            "the temporary file must not survive the rename"
        );
    }

    #[test]
    fn saving_into_an_unwritable_place_fails_loudly() {
        let temp = tempfile::tempdir().unwrap();
        let blocked = temp.path().join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        let store = RecoveryStore {
            path: blocked.join(RECOVERY_FILE),
        };
        // Make the directory unwritable for this process.
        let mut permissions = std::fs::metadata(&blocked).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(0o500);
        }
        #[cfg(not(unix))]
        {
            permissions.set_readonly(true);
        }
        std::fs::set_permissions(&blocked, permissions).unwrap();
        let result = store.save(&record());
        // Restore so the directory can be cleaned up.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut restore = std::fs::metadata(&blocked).unwrap().permissions();
            restore.set_mode(0o700);
            std::fs::set_permissions(&blocked, restore).unwrap();
        }
        let error = result.expect_err("writing into a read-only directory must fail");
        assert!(
            !error.to_string().is_empty(),
            "the failure must carry a reason the caller can report"
        );
    }
}
