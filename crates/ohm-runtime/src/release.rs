//! What actually happened when control was handed back.
//!
//! "Hand the fans back safely" is three separate claims, and the exit path used
//! to collapse them into one number:
//!
//! 1. the fail-safe duty was **written**;
//! 2. the device **read it back** — without that the value is unknown;
//! 3. the adapter **gave ownership back** to the firmware.
//!
//! The old `release_control()` returned a `usize` that counted case 1, so an
//! unconfirmed write — the adapter accepted the request but could not read the
//! channel back — was reported and audited as *"outputs handed back to
//! firmware"*. That is exactly the kind of claim this project refuses to make
//! elsewhere. This module keeps the three apart, and only the buckets that are
//! known say so.
//!
//! Nothing here deletes a failure: a refused or failed channel stays in the
//! report with its reason, and the runtime records each bucket separately.

use serde::{Deserialize, Serialize};

use ohm_adapter_api::WriteStatus;
use ohm_core::{AdapterId, CapabilityId, DeviceId};
use ohm_device_model::Value;

/// One channel the exit path tried to put at its fail-safe duty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReleasedChannel {
    pub device_id: DeviceId,
    pub device_name: String,
    pub capability: CapabilityId,
    /// The fail-safe duty that was requested.
    pub duty: f64,
    /// What the device ended up at, when it was confirmed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub applied: Option<Value>,
    pub status: WriteStatus,
    /// Why the value is unconfirmed, or why the write was refused.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub detail: Option<String>,
}

impl ReleasedChannel {
    /// The channel was written and read back.
    pub fn is_confirmed(&self) -> bool {
        matches!(self.status, WriteStatus::Applied)
    }
}

/// An adapter whose shutdown reported a problem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShutdownFailure {
    pub adapter: AdapterId,
    pub detail: String,
}

/// Every channel the exit path touched, split by what is actually known.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ControlRelease {
    /// Written and read back: the device is known to be at the fail-safe duty.
    pub confirmed: Vec<ReleasedChannel>,
    /// Accepted, but the value could not be read back. Unknown — not success.
    pub unconfirmed: Vec<ReleasedChannel>,
    /// Refused before the hardware: the safety policy or the device said no.
    pub rejected: Vec<ReleasedChannel>,
    /// The write itself failed, with the reason.
    pub failed: Vec<ReleasedChannel>,
    /// Simulated hardware or a dry run: nothing real was written.
    pub simulated: Vec<ReleasedChannel>,
    /// Cooling adapters that reported handing control back to the firmware.
    pub relinquished: Vec<AdapterId>,
    /// Cooling adapters that drive channels but do **not** hand control back on
    /// shutdown. Their channels keep the fail-safe duty the exit path wrote and
    /// nothing else takes them over, so this is a caveat the user must see.
    pub without_hand_back: Vec<AdapterId>,
    /// Adapters whose shutdown reported a problem.
    pub shutdown_failed: Vec<ShutdownFailure>,
    /// `true` when `relinquish_on_exit` is off, so nothing was written at all.
    pub skipped_by_config: bool,
}

impl ControlRelease {
    /// Channels the exit path tried to write.
    pub fn attempted(&self) -> usize {
        self.confirmed.len()
            + self.unconfirmed.len()
            + self.rejected.len()
            + self.failed.len()
            + self.simulated.len()
    }

    /// Channels known to be at their fail-safe duty.
    pub fn confirmed_count(&self) -> usize {
        self.confirmed.len()
    }

    /// `true` when nothing is left unknown or broken.
    pub fn is_clean(&self) -> bool {
        self.unconfirmed.is_empty()
            && self.rejected.is_empty()
            && self.failed.is_empty()
            && self.shutdown_failed.is_empty()
    }

    /// Add one channel to the bucket its status names.
    pub fn push(&mut self, channel: ReleasedChannel) {
        match channel.status {
            WriteStatus::Applied => self.confirmed.push(channel),
            WriteStatus::Unconfirmed => self.unconfirmed.push(channel),
            WriteStatus::Rejected => self.rejected.push(channel),
            WriteStatus::Simulated => self.simulated.push(channel),
        }
    }

    /// Add a channel whose write returned an error.
    ///
    /// A refusal (`safety_blocked`, `capability_read_only`, and the other
    /// pre-hardware codes) is not the same thing as a hardware or adapter
    /// failure, and the report says which happened. The status stays `Rejected`
    /// in both cases because that is what the audit trail records for a write
    /// that never landed; the bucket carries the distinction.
    pub fn push_failure(&mut self, channel: ReleasedChannel, code: &str) {
        if is_refusal(code) {
            self.rejected.push(channel);
        } else {
            self.failed.push(channel);
        }
    }

    /// One line that can be printed or audited without over-claiming.
    pub fn summary(&self) -> String {
        if self.skipped_by_config {
            return "control release skipped: relinquish_on_exit is off, so nothing was written"
                .to_string();
        }
        let mut parts = vec![format!(
            "{} of {} output(s) confirmed at the fail-safe duty",
            self.confirmed.len(),
            self.attempted()
        )];
        if !self.unconfirmed.is_empty() {
            parts.push(format!(
                "{} written but not read back (value unknown)",
                self.unconfirmed.len()
            ));
        }
        if !self.rejected.is_empty() {
            parts.push(format!("{} refused", self.rejected.len()));
        }
        if !self.failed.is_empty() {
            parts.push(format!("{} failed", self.failed.len()));
        }
        if !self.simulated.is_empty() {
            parts.push(format!("{} simulated", self.simulated.len()));
        }
        if !self.relinquished.is_empty() {
            parts.push(format!(
                "{} cooling adapter(s) reported handing control back",
                self.relinquished.len()
            ));
        }
        if !self.without_hand_back.is_empty() {
            parts.push(format!(
                "{} cooling adapter(s) do not hand control back on shutdown",
                self.without_hand_back.len()
            ));
        }
        if !self.shutdown_failed.is_empty() {
            parts.push(format!(
                "{} adapter shutdown(s) failed",
                self.shutdown_failed.len()
            ));
        }
        parts.join("; ")
    }

    /// The problems, one line each, in a form a person or a log can carry.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for channel in &self.unconfirmed {
            problems.push(format!(
                "{}/{} was written at the fail-safe duty but not read back: {}",
                channel.device_id.as_str(),
                channel.capability.as_str(),
                channel.detail.as_deref().unwrap_or("no reason given")
            ));
        }
        for channel in &self.rejected {
            problems.push(format!(
                "{}/{} refused the fail-safe duty: {}",
                channel.device_id.as_str(),
                channel.capability.as_str(),
                channel.detail.as_deref().unwrap_or("no reason given")
            ));
        }
        for channel in &self.failed {
            problems.push(format!(
                "{}/{} could not be written: {}",
                channel.device_id.as_str(),
                channel.capability.as_str(),
                channel.detail.as_deref().unwrap_or("no reason given")
            ));
        }
        for failure in &self.shutdown_failed {
            problems.push(format!(
                "adapter {} did not shut down cleanly: {}",
                failure.adapter.as_str(),
                failure.detail
            ));
        }
        problems
    }

    /// Things that are not failures but that the user has to know: an adapter
    /// that keeps its channels after shutdown is not a released channel.
    pub fn caveats(&self) -> Vec<String> {
        self.without_hand_back
            .iter()
            .map(|adapter| {
                format!(
                    "adapter {} does not hand control back on shutdown: its channels keep the duty we wrote",
                    adapter.as_str()
                )
            })
            .collect()
    }
}

/// `true` when an error code means "refused before the hardware" rather than
/// "the write itself failed".
pub fn is_refusal(code: &str) -> bool {
    matches!(
        code,
        "safety_blocked"
            | "value_out_of_range"
            | "invalid_value"
            | "capability_read_only"
            | "device_not_found"
            | "capability_not_found"
            | "device_disabled"
            | "invalid_id"
            | "write_rejected"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(status: WriteStatus) -> ReleasedChannel {
        ReleasedChannel {
            device_id: DeviceId::compose("fan", "mock", 0),
            device_name: "Mock Chassis Fan 1".into(),
            capability: CapabilityId::new_unchecked("fan.speed_percent"),
            duty: 60.0,
            applied: None,
            status,
            detail: Some("injected".into()),
        }
    }

    #[test]
    fn an_unconfirmed_channel_is_never_counted_as_confirmed() {
        let mut release = ControlRelease::default();
        release.push(channel(WriteStatus::Unconfirmed));
        assert_eq!(release.attempted(), 1);
        assert_eq!(release.confirmed_count(), 0);
        assert!(!release.is_clean());
        assert!(release.summary().contains("not read back"));
        assert_eq!(release.problems().len(), 1);
    }

    #[test]
    fn a_confirmed_channel_is_clean_and_says_so() {
        let mut release = ControlRelease::default();
        release.push(channel(WriteStatus::Applied));
        assert_eq!(release.confirmed_count(), 1);
        assert!(release.is_clean());
        assert!(release.problems().is_empty());
        assert!(release.summary().starts_with("1 of 1 output(s) confirmed"));
    }

    #[test]
    fn a_refusal_is_not_the_same_as_a_failure() {
        let refused = [
            "safety_blocked",
            "capability_read_only",
            "device_not_found",
            "write_rejected",
        ];
        for code in refused {
            let mut release = ControlRelease::default();
            release.push_failure(channel(WriteStatus::Rejected), code);
            assert_eq!(release.rejected.len(), 1, "{code}");
            assert!(release.failed.is_empty(), "{code}");
        }
        for code in ["adapter_error", "io_error", "protocol_error"] {
            let mut release = ControlRelease::default();
            release.push_failure(channel(WriteStatus::Rejected), code);
            assert_eq!(release.failed.len(), 1, "{code}");
            assert!(release.rejected.is_empty(), "{code}");
        }
    }

    #[test]
    fn a_skipped_release_claims_nothing() {
        let release = ControlRelease {
            skipped_by_config: true,
            ..Default::default()
        };
        assert_eq!(release.attempted(), 0);
        assert!(release.summary().contains("skipped"));
    }

    #[test]
    fn the_summary_names_every_bucket_that_is_not_empty() {
        let mut release = ControlRelease::default();
        release.push(channel(WriteStatus::Applied));
        release.push(channel(WriteStatus::Unconfirmed));
        release.push_failure(channel(WriteStatus::Rejected), "adapter_error");
        release.relinquished.push(AdapterId::new_unchecked("mock"));
        release.shutdown_failed.push(ShutdownFailure {
            adapter: AdapterId::new_unchecked("lhm"),
            detail: "server gone".into(),
        });
        let summary = release.summary();
        for expected in [
            "1 of 3",
            "not read back",
            "1 failed",
            "handing control back",
            "shutdown(s) failed",
        ] {
            assert!(summary.contains(expected), "{summary}");
        }
        assert_eq!(release.problems().len(), 3);
    }
}
