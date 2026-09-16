//! What the machine says *before* the rules are left running with no window open.
//!
//! `ohm-cli service check` exists because the failure it prevents is invisible: a
//! service that cannot write looks exactly like a service that has nothing to do —
//! healthy banner, moving cycle counter, and a fan that never moves. The check asks the
//! configured intent (does this configuration confirm a channel? is automation on? is
//! this a dry run?) and then asks the platform whether it would permit the writes, so
//! the answer arrives once, in words a person can act on.
//!
//! These tests run the real binary against prepared trees. What they cannot check is a
//! real fan: no reading here proves air moved.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_ohm-cli");
const CHIP: &str = "fakechip";

/// A config directory and a prepared hwmon tree with one fan channel.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ohm-preflight-{}-{name}-{}",
            std::process::id(),
            ohm_core::now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        let chip = dir.join("hwmon").join("hwmon0");
        fs::create_dir_all(&chip).expect("hwmon dir");
        fs::write(chip.join("name"), format!("{CHIP}\n")).expect("name");
        fs::write(chip.join("fan1_input"), "1200\n").expect("tachometer");
        fs::write(chip.join("pwm1"), "128\n").expect("pwm");
        fs::write(chip.join("pwm1_enable"), "2\n").expect("pwm mode");
        fs::create_dir_all(dir.join("rules")).expect("rules dir");
        fs::write(
            dir.join("rules").join("chassis.yaml"),
            format!(
                "name: Chassis intake\n\
                 id: chassis-intake\n\
                 enabled: true\n\
                 source: {{ device: fan.system.{CHIP}_fan1, capability: fan.rpm }}\n\
                 target: {{ device: fan.system.{CHIP}_fan1, capability: fan.speed_percent }}\n\
                 curve:\n\
                 \x20 - [0, 40]\n\
                 \x20 - [3000, 100]\n"
            ),
        )
        .expect("rule file");
        Self { dir }
    }

    /// A configuration that confirms the channel, so writes are intended.
    fn confirming(mut self) -> Self {
        self.settings(format!(
            r#"{{"adapter_settings": {{"system": {{"pwm_write_allow": ["fan.system.{CHIP}_fan1"]}}}}}}"#
        ));
        self
    }

    fn settings(&mut self, json: String) {
        fs::write(self.dir.join("settings.json"), json).expect("settings");
    }

    fn pwm(&self) -> PathBuf {
        self.dir.join("hwmon").join("hwmon0").join("pwm1")
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(BIN)
            .args(args)
            .env("OHM_CONFIG_DIR", &self.dir)
            .env("OHM_HWMON_ROOT", self.dir.join("hwmon"))
            .output()
            .expect("run ohm-cli")
    }

    fn check(&self) -> (i32, String) {
        let output = self.run(&["--log-level", "error", "service", "check"]);
        (
            output.status.code().unwrap_or(-1),
            format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Whether this process can write a file whose mode forbids it. Root can, so a test
/// that assumes a refusal has to know which of the two it is looking at.
fn running_as_root() -> bool {
    let path = std::env::temp_dir().join(format!("ohm-preflight-root-{}", std::process::id()));
    fs::write(&path, b"probe").expect("probe file");
    let mut permissions = fs::metadata(&path).expect("metadata").permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o000);
        fs::set_permissions(&path, permissions).expect("chmod");
        let writable = fs::OpenOptions::new().write(true).open(&path).is_ok();
        let mut restore = fs::metadata(&path).expect("metadata").permissions();
        restore.set_mode(0o600);
        let _ = fs::set_permissions(&path, restore);
        let _ = fs::remove_file(&path);
        writable
    }
    #[cfg(not(unix))]
    {
        let _ = permissions;
        let _ = fs::remove_file(&path);
        true
    }
}

#[test]
fn a_confirmed_channel_the_platform_permits_is_reported_as_ready() {
    let fixture = Fixture::new("permitted").confirming();
    let (code, text) = fixture.check();

    assert_eq!(code, 0, "a machine that can do the job is ready:\n{text}");
    assert!(
        text.contains(&format!(
            "fan.system.{CHIP}_fan1 / fan.speed_percent: permitted"
        )),
        "the confirmed channel is named and answered for:\n{text}"
    );
    assert!(
        text.contains("verdict:    READY — the rules would run"),
        "and the verdict says so:\n{text}"
    );
    assert!(
        text.contains("automation: enabled") && text.contains("dry run:    off"),
        "with the two settings that decide it printed first:\n{text}"
    );
}

#[test]
fn a_configuration_that_confirms_nothing_is_ready_to_read_only() {
    // No allow-list: the honest answer is not "not ready", it is "nothing here writes".
    let fixture = Fixture::new("read-only");
    let (code, text) = fixture.check();

    assert_eq!(code, 0, "reading is a complete job:\n{text}");
    assert!(text.contains("writable channels: none"), "{text}");
    assert!(
        text.contains("adapter_settings.system.pwm_write_allow"),
        "and it says how to change that:\n{text}"
    );
    assert!(text.contains("READY (READ ONLY)"), "{text}");
}

#[test]
fn a_dry_run_says_nothing_can_reach_hardware() {
    let mut fixture = Fixture::new("dry-run").confirming();
    fixture.settings(format!(
        r#"{{"dry_run": true, "adapter_settings": {{"system": {{"pwm_write_allow": ["fan.system.{CHIP}_fan1"]}}}}}}"#
    ));
    let (code, text) = fixture.check();

    assert_eq!(code, 0, "{text}");
    assert!(text.contains("dry run:    on"), "{text}");
    assert!(text.contains("READY (DRY RUN)"), "{text}");
}

#[test]
fn automation_that_is_switched_off_says_it_would_only_read() {
    let mut fixture = Fixture::new("automation-off").confirming();
    fixture.settings(format!(
        r#"{{"automation_enabled": false, "adapter_settings": {{"system": {{"pwm_write_allow": ["fan.system.{CHIP}_fan1"]}}}}}}"#
    ));
    let (code, text) = fixture.check();

    assert_eq!(code, 0, "{text}");
    assert!(text.contains("automation: disabled in settings"), "{text}");
    assert!(text.contains("READY (READ ONLY)"), "{text}");
}

/// The case the command exists for: a channel somebody confirmed, and a platform that
/// will not let this process write it.
#[cfg(unix)]
#[test]
fn a_confirmed_channel_the_platform_refuses_is_not_ready() {
    if running_as_root() {
        // Root can write the file anyway, so the refusal cannot be produced here.
        return;
    }
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new("refused").confirming();
    let pwm = fixture.pwm();
    let mut permissions = fs::metadata(&pwm).expect("metadata").permissions();
    permissions.set_mode(0o444);
    fs::set_permissions(&pwm, permissions).expect("chmod");

    let (code, text) = fixture.check();
    assert_eq!(code, 1, "a job that cannot be done is not ready:\n{text}");
    assert!(text.contains("REFUSED"), "{text}");
    assert!(
        text.contains("pwm1"),
        "the refusal names the file the platform refused:\n{text}"
    );
    assert!(text.contains("NOT READY"), "{text}");
}

#[test]
fn a_running_service_is_named_and_left_alone() {
    let fixture = Fixture::new("running").confirming();

    // A state file whose owner is alive: this process's own pid exists, and the
    // heartbeat is now, which is exactly what "fresh" means.
    let state = format!(
        r#"{{"pid": {}, "version": "0.1.11", "started_at_ms": {}, "heartbeat_at_ms": {},
            "heartbeat_interval_ms": 1000, "ticks": 7, "rules": 1}}"#,
        std::process::id(),
        ohm_core::now_ms(),
        ohm_core::now_ms()
    );
    fs::write(fixture.dir.join("service.json"), state).expect("state file");

    let (code, text) = fixture.check();
    assert_eq!(code, 0, "another service running is not a failure:\n{text}");
    assert!(
        text.contains("a service is running here (pid"),
        "it says who holds the channels:\n{text}"
    );
    assert!(
        text.contains("7 cycle(s)"),
        "and how far along it is:\n{text}"
    );
    assert!(
        text.contains("does not take the channels from it"),
        "and that asking changed nothing:\n{text}"
    );
    // Read-only means read-only: the preflight must not have touched the record.
    let left = fs::read_to_string(fixture.dir.join("service.json")).expect("state file");
    assert!(left.contains("\"ticks\": 7"), "{left}");
}
