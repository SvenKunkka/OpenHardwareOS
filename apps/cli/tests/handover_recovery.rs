//! What an operator sees after a crash, through the real CLI.
//!
//! The engine's recovery tests prove the responsibility comes back on the books. This
//! test runs the actual `ohm-cli` binary against a config directory that contains a
//! record written by a "previous session", because the point of the record is that a
//! *person* can find out that a channel is unprotected — and a fact that only exists
//! inside a Rust API is not much use to them.
//!
//! The desktop reads the same engine state through `rule_handovers`; the assertions
//! here are therefore about the shared facts (channel, state, reason, what to do), not
//! about CLI wording.

use std::path::Path;
use std::process::Command;

/// A minimal control record, as a previous session would have left it.
fn record(device: &str, state: &str) -> String {
    format!(
        r#"{{
  "version": 1,
  "saved_at_ms": 1700000000000,
  "handovers": [
    {{
      "device": "{device}",
      "capability": "fan.speed_percent",
      "from_rule": "gone-rule",
      "reason": "rule `gone-rule` was deleted; {device}/fan.speed_percent is no longer driven by it",
      "state": "{state}",
      "attempts": 3,
      "first_error": "the device refused the fail-safe duty: the channel refused the write",
      "queued_at_ms": 1700000000000,
      "last_attempt_ms": 1700000000500,
      "claimed_ticks": 0,
      "parked_by_claim": false
    }}
  ],
  "holds": []
}}"#
    )
}

/// Run the CLI against a config directory and return its combined output.
fn run_cli(config_dir: &Path, args: &[&str]) -> (String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_ohm-cli"))
        .arg("--config-dir")
        .arg(config_dir)
        .args(args)
        .output()
        .expect("the CLI binary runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (text, output.status.code().unwrap_or(-1))
}

/// A record from a previous session must be visible to the operator, with the channel,
/// the rule that left it, the original cause, and an honest statement of what it means.
#[test]
fn a_recovered_handover_is_visible_with_its_cause_and_consequence() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path();
    std::fs::write(
        config.join("control-state.json"),
        record("fan.mock.0", "pending"),
    )
    .unwrap();

    let (text, code) = run_cli(config, &["handovers"]);
    assert_eq!(code, 0, "the command must succeed: {text}");
    assert!(
        text.contains("fan.mock.0/fan.speed_percent"),
        "the channel must be named: {text}"
    );
    assert!(
        text.contains("gone-rule"),
        "the rule that left it must be named, even though it no longer exists: {text}"
    );
    assert!(
        // This session has no such fan, so checking the record against the machine
        // turns the item into a failure — which is the truth about *this* machine, and
        // exactly what the operator needs to see.
        text.contains("could not be resolved") || text.contains("failed"),
        "the item must be checked against this machine and reported honestly: {text}"
    );
    assert!(
        text.contains("refused"),
        "the original cause must survive: {text}"
    );
    assert!(
        text.contains("still not protected"),
        "and the consequence must be stated plainly: {text}"
    );
}

/// A record whose device is not on this machine is a failure the operator can act on,
/// not a silent drop — and one that cannot be discharged here still counts as owed.
#[test]
fn a_recovered_handover_for_a_missing_device_is_reported_as_unfinished() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path();
    std::fs::write(
        config.join("control-state.json"),
        record("fan.not_here.0", "failed"),
    )
    .unwrap();

    let (text, code) = run_cli(config, &["handovers"]);
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("fan.not_here.0/fan.speed_percent"),
        "the channel is still named: {text}"
    );
    assert!(
        text.contains("failed"),
        "it is reported as unfinished: {text}"
    );
    assert!(
        text.contains("--retry"),
        "and the way to try again is offered: {text}"
    );
    assert!(
        text.contains("was not found") || text.contains("could not be resolved"),
        "the reason must say the device is not here: {text}"
    );
    assert!(
        text.contains("3 attempt"),
        "and the attempts it made before the restart are kept: {text}"
    );
}

/// `--retry` is an explicit action, and it must say what it did even when there is
/// nothing to re-arm.
#[test]
fn retry_reports_what_it_did() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path();
    std::fs::write(
        config.join("control-state.json"),
        record("fan.mock.0", "failed"),
    )
    .unwrap();

    let (text, code) = run_cli(config, &["handovers", "--retry"]);
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("re-armed 1 handover"),
        "the re-arm must report the item it took on: {text}"
    );
    assert!(
        text.contains("fan.mock.0/fan.speed_percent"),
        "and the item must still be listed: {text}"
    );
}

/// A damaged record must not stop the tool from running, and must not be presented as
/// if nothing were wrong.
#[test]
fn a_damaged_record_is_reported_and_does_not_hide_the_problem() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path();
    std::fs::write(config.join("control-state.json"), "{ not json").unwrap();

    let (text, code) = run_cli(config, &["handovers"]);
    assert_eq!(
        code, 0,
        "a damaged record must not make the CLI fail to run: {text}"
    );
    assert!(
        text.contains("control record") || text.contains("could not be read"),
        "and the problem must be visible rather than silently ignored: {text}"
    );
}
