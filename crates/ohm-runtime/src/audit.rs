//! The audit trail.
//!
//! Requirement: *every* write to hardware is logged, with the requested value,
//! the applied value, the origin (user / rule / safety) and the outcome. The
//! log is append-only JSON Lines so it can be tailed, grepped and diffed
//! without a database.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use ohm_adapter_api::WriteStatus;
use ohm_core::{CapabilityId, DeviceId, OhmError, Result, RuleId};
use ohm_device_model::{DeviceType, Value};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// Who asked for a write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WriteOrigin {
    /// A person moved a slider or clicked apply.
    Manual,
    /// An automation rule.
    Automation { rule_id: RuleId },
    /// The safety supervisor (emergency curve, fail-safe).
    Safety { reason: String },
    /// Applied while starting up.
    Startup,
    /// Applied while shutting down (control release).
    Shutdown,
    /// Another program driving the API.
    Api,
}

impl WriteOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Automation { .. } => "automation",
            Self::Safety { .. } => "safety",
            Self::Startup => "startup",
            Self::Shutdown => "shutdown",
            Self::Api => "api",
        }
    }

    pub fn rule(&self) -> Option<&RuleId> {
        match self {
            Self::Automation { rule_id } => Some(rule_id),
            _ => None,
        }
    }
}

/// A write attempt, as recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WriteReport {
    pub at_ms: i64,
    pub device_id: DeviceId,
    pub device_name: String,
    pub device_type: DeviceType,
    pub capability: CapabilityId,
    pub capability_name: String,
    pub requested: Value,
    /// The value the hardware ended up with, when known.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub applied: Option<Value>,
    pub status: WriteStatus,
    /// `true` when the safety layer or the device range changed the value.
    #[serde(default)]
    pub clamped: bool,
    /// Set when the write was refused before reaching the hardware.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub detail: Option<String>,
    pub origin: WriteOrigin,
    /// `true` when the value was simulated (dry-run or mock hardware).
    #[serde(default)]
    pub simulated: bool,
}

impl WriteReport {
    /// One line summary for the UI activity feed.
    pub fn summary(&self) -> String {
        let value = self
            .applied
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| self.requested.to_string());
        match self.status {
            WriteStatus::Applied => format!(
                "{} set {} to {}{}",
                self.device_name,
                self.capability_name,
                value,
                if self.clamped { " (clamped)" } else { "" }
            ),
            WriteStatus::Simulated => format!(
                "{} simulated {} = {}",
                self.device_name, self.capability_name, value
            ),
            WriteStatus::Rejected => format!(
                "{} rejected {} = {} ({})",
                self.device_name,
                self.capability_name,
                self.requested,
                self.detail.as_deref().unwrap_or("no reason given")
            ),
        }
    }

    /// `true` when the hardware actually changed.
    pub fn changed_hardware(&self) -> bool {
        matches!(self.status, WriteStatus::Applied | WriteStatus::Simulated)
    }
}

/// Append-only audit sink.
#[derive(Debug)]
pub struct AuditLog {
    path: Option<PathBuf>,
    lock: Mutex<()>,
}

impl AuditLog {
    /// An audit log writing to `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
            lock: Mutex::new(()),
        }
    }

    /// An audit log that only emits tracing events (used in tests and when the
    /// config directory is not writable).
    pub fn disabled() -> Self {
        Self {
            path: None,
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    /// Record a write. Never fails the caller: a logging problem must not stop
    /// hardware protection from working, but it is reported loudly.
    pub fn record(&self, report: &WriteReport) {
        match report.status {
            WriteStatus::Applied => tracing::info!(
                device = %report.device_id,
                capability = %report.capability,
                value = %report
                    .applied
                    .as_ref()
                    .unwrap_or(&report.requested),
                origin = report.origin.as_str(),
                "hardware write applied"
            ),
            WriteStatus::Simulated => tracing::info!(
                device = %report.device_id,
                capability = %report.capability,
                origin = report.origin.as_str(),
                "hardware write simulated"
            ),
            WriteStatus::Rejected => tracing::warn!(
                device = %report.device_id,
                capability = %report.capability,
                origin = report.origin.as_str(),
                detail = report.detail.as_deref().unwrap_or(""),
                "hardware write rejected"
            ),
        }
        if let Err(err) = self.append(report) {
            tracing::error!(error = %err, "could not append to the audit log");
        }
    }

    /// Record a lifecycle event (start, stop, adapter failure, ...).
    pub fn record_lifecycle(&self, action: &str, detail: &str) {
        tracing::info!(action, detail, "runtime lifecycle");
        let entry = serde_json::json!({
            "at_ms": ohm_core::now_ms(),
            "at": ohm_core::now_rfc3339(),
            "kind": "lifecycle",
            "action": action,
            "detail": detail,
        });
        if let Err(err) = self.append_json(&entry) {
            tracing::error!(error = %err, "could not append to the audit log");
        }
    }

    fn append(&self, report: &WriteReport) -> Result<()> {
        let entry = serde_json::json!({
            "at_ms": report.at_ms,
            "at": ohm_core::unix_ms_to_rfc3339(report.at_ms),
            "kind": "write",
            "report": report,
        });
        self.append_json(&entry)
    }

    fn append_json(&self, entry: &serde_json::Value) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let _guard = self.lock.lock();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| OhmError::io(parent, e))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| OhmError::io(path, e))?;
        let mut line = serde_json::to_string(entry)?;
        line.push('\n');
        file.write_all(line.as_bytes())
            .map_err(|e| OhmError::io(path, e))?;
        Ok(())
    }

    /// Read the most recent entries (newest last). Used by Settings -> Logs.
    pub fn tail(&self, limit: usize) -> Result<Vec<serde_json::Value>> {
        let Some(path) = &self.path else {
            return Ok(Vec::new());
        };
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = std::fs::read_to_string(path).map_err(|e| OhmError::io(path, e))?;
        let mut entries: Vec<serde_json::Value> = raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        if entries.len() > limit {
            entries.drain(..entries.len() - limit);
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohm_device_model::DeviceType;

    fn report(status: WriteStatus) -> WriteReport {
        WriteReport {
            at_ms: 1_700_000_000_000,
            device_id: DeviceId::new("fan.system.0").unwrap(),
            device_name: "Chassis Fan 1".into(),
            device_type: DeviceType::Fan,
            capability: CapabilityId::new("fan.speed_percent").unwrap(),
            capability_name: "Fan Speed".into(),
            requested: Value::Number(60.0),
            applied: Some(Value::Number(60.0)),
            status,
            clamped: false,
            error_code: None,
            detail: None,
            origin: WriteOrigin::Manual,
            simulated: false,
        }
    }

    #[test]
    fn writes_are_appended_as_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let log = AuditLog::new(tmp.path().join("audit.jsonl"));
        log.record(&report(WriteStatus::Applied));
        log.record(&report(WriteStatus::Rejected));

        let entries = log.tail(10).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["kind"], "write");
        assert_eq!(entries[0]["report"]["status"], "applied");
        assert_eq!(entries[1]["report"]["status"], "rejected");
        assert!(entries[0]["at"].as_str().unwrap().contains('T'));
    }

    #[test]
    fn lifecycle_entries_are_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let log = AuditLog::new(tmp.path().join("audit.jsonl"));
        log.record_lifecycle("runtime_started", "1 adapter, 4 devices");
        let entries = log.tail(10).unwrap();
        assert_eq!(entries[0]["kind"], "lifecycle");
        assert_eq!(entries[0]["action"], "runtime_started");
    }

    #[test]
    fn tail_keeps_the_newest() {
        let tmp = tempfile::tempdir().unwrap();
        let log = AuditLog::new(tmp.path().join("audit.jsonl"));
        for value in 0..25 {
            let mut r = report(WriteStatus::Applied);
            r.requested = Value::Number(f64::from(value));
            log.record(&r);
        }
        let entries = log.tail(5).unwrap();
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[4]["report"]["requested"], 24.0);
    }

    #[test]
    fn disabled_log_is_harmless() {
        let log = AuditLog::disabled();
        log.record(&report(WriteStatus::Applied));
        log.record_lifecycle("test", "nothing");
        assert!(log.tail(10).unwrap().is_empty());
        assert!(!log.is_enabled());
    }

    #[test]
    fn summary_is_human_readable() {
        assert_eq!(
            report(WriteStatus::Applied).summary(),
            "Chassis Fan 1 set Fan Speed to 60"
        );
        let mut clamped = report(WriteStatus::Applied);
        clamped.clamped = true;
        assert!(clamped.summary().contains("(clamped)"));
        let mut rejected = report(WriteStatus::Rejected);
        rejected.detail = Some("permission denied".into());
        assert!(rejected.summary().contains("permission denied"));
        assert!(
            report(WriteStatus::Simulated)
                .summary()
                .contains("simulated")
        );
    }

    #[test]
    fn origin_metadata() {
        let rule = RuleId::new("gpu-cooling").unwrap();
        let origin = WriteOrigin::Automation {
            rule_id: rule.clone(),
        };
        assert_eq!(origin.as_str(), "automation");
        assert_eq!(origin.rule(), Some(&rule));
        assert_eq!(WriteOrigin::Manual.rule(), None);
        let json = serde_json::to_value(&origin).unwrap();
        assert_eq!(json["kind"], "automation");
        assert_eq!(json["rule_id"], "gpu-cooling");
    }
}
