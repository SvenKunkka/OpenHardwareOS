//! The report a real machine's owner is asked to send back.
//!
//! Its whole purpose is that the file can be read by somebody who is *not* sitting at
//! that machine: it carries what this project reports **and** the platform's own answer
//! to the same questions — the raw `hwmon` files, `/proc/meminfo`, `df` — so a reading
//! can be checked instead of trusted. These tests run the real binary against a
//! prepared tree and assert the file contains both halves.
//!
//! What they cannot check is the part that matters most on a real machine: whether the
//! numbers are *true*. That is what the field checklist is for.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_ohm-cli");

/// A config directory and a prepared hwmon tree with one chip, one tachometer and a
/// PWM channel whose raw contents are known exactly.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ohm-report-{}-{name}-{}",
            std::process::id(),
            ohm_core::now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        let chip = dir.join("hwmon").join("hwmon0");
        fs::create_dir_all(&chip).expect("hwmon dir");
        fs::write(chip.join("name"), "fakechip\n").expect("name");
        fs::write(chip.join("fan1_input"), "1200\n").expect("tachometer");
        fs::write(chip.join("pwm1"), "128\n").expect("pwm");
        fs::write(chip.join("pwm1_enable"), "1\n").expect("pwm mode");
        fs::write(chip.join("temp1_input"), "45500\n").expect("temperature");
        Self { dir }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(BIN)
            .args(args)
            .env("OHM_CONFIG_DIR", &self.dir)
            .env("OHM_HWMON_ROOT", self.dir.join("hwmon"))
            .output()
            .expect("run ohm-cli")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn the_report_carries_our_readings_and_the_platforms_own_answer() {
    let fixture = Fixture::new("both-halves");
    let out = fixture.dir.join("report.json");
    let output = fixture.run(&["report", "--out", out.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "report: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.exists(), "the file was written where it was asked for");

    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).expect("readable")).expect("valid JSON");

    // Our half.
    assert_eq!(report["version"], env!("CARGO_PKG_VERSION"));
    assert!(report["os"].is_string());
    assert!(
        report["generated_at_ms"].as_i64().unwrap_or(0) > 0,
        "the report says when it was taken"
    );
    let fan = report["devices"]
        .as_array()
        .expect("devices")
        .iter()
        .find(|view| view["device"]["id"] == "fan.system.fakechip_fan1")
        .expect("the prepared channel is in the report");
    let readings = fan["state"]["readings"].as_array().expect("readings");
    let rpm = readings
        .iter()
        .find(|reading| reading["capability"] == "fan.rpm")
        .expect("fan.rpm");
    assert_eq!(rpm["value"], 1200.0, "the reading we report");

    // The platform's half: the same file, unparsed, so a reader can check the first
    // against the second without trusting this project's code.
    assert_eq!(
        report["platform"]["hwmon_root"],
        fixture.dir.join("hwmon").to_str().unwrap()
    );
    let chip = &report["platform"]["hwmon"][0];
    assert_eq!(chip["name"], "fakechip");
    assert_eq!(chip["files"]["fan1_input"], "1200");
    assert_eq!(chip["files"]["pwm1"], "128");
    assert_eq!(chip["files"]["pwm1_enable"], "1");
    assert_eq!(chip["files"]["temp1_input"], "45500");
    assert_eq!(
        chip["unreadable"].as_object().map(|map| map.len()),
        Some(0),
        "nothing in the tree was unreadable"
    );

    // And an explanation for a section that is empty, rather than silence.
    // The Windows half of the platform evidence is LibreHardwareMonitor's own sensor
    // list. There is no LHM on a test machine, so what is asserted here is the honest
    // failure: the section says why it is empty and names the server it tried, rather
    // than looking like a machine without sensors. The successful path is covered by
    // `ohm-adapter-lhm`'s own tests against its fake server.
    let lhm = &report["platform"]["lhm"];
    assert!(lhm["url"].is_string(), "it names the server it asked");
    assert_eq!(
        lhm["sensors"].as_array().map(|list| list.len()),
        Some(0),
        "and carries no invented sensors"
    );
    let lhm_notes = lhm["notes"].as_array().expect("notes");
    assert!(
        lhm_notes.iter().any(|note| note
            .as_str()
            .unwrap_or_default()
            .contains("LibreHardwareMonitor")),
        "with a reason a reader can act on: {lhm_notes:?}"
    );

    // And an explanation for a section that is empty, rather than silence — which means
    // two different assertions, because the answer depends on the platform. Linux has
    // `/proc/meminfo` and must show it; everywhere else must say why it has nothing.
    // Writing only the second version made this test pass on macOS and fail on CI's
    // Ubuntu runner: the fourth time in three rounds that an assertion depended on the
    // machine it ran on. Both cases are asserted now, so either platform failing is a
    // failure.
    let notes = report["platform"]["notes"].as_array().expect("notes");
    let meminfo = report["platform"]["meminfo"].as_array().expect("meminfo");
    if cfg!(target_os = "linux") {
        assert!(
            meminfo
                .iter()
                .any(|line| line.as_str().unwrap_or_default().starts_with("MemTotal")),
            "Linux has /proc/meminfo, so the report carries the total a reader can \
             compare against: {meminfo:?}"
        );
        assert!(
            !notes.iter().any(|note| note
                .as_str()
                .unwrap_or_default()
                .contains("meminfo: not readable")),
            "and does not claim it is missing: {notes:?}"
        );
    } else {
        assert!(
            meminfo.is_empty(),
            "there is no /proc/meminfo here, so there is nothing to carry: {meminfo:?}"
        );
        assert!(
            notes
                .iter()
                .any(|note| note.as_str().unwrap_or_default().contains("/proc/meminfo")),
            "and the report says so rather than staying silent: {notes:?}"
        );
    }
}

#[test]
fn a_report_without_a_target_writes_into_the_reports_directory() {
    let fixture = Fixture::new("default-target");
    let output = fixture.run(&["report"]);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("written to:"),
        "it says where the file went: {stdout}"
    );
    let reports = fixture.dir.join("reports");
    let entries: Vec<_> = fs::read_dir(&reports)
        .expect("reports directory exists")
        .flatten()
        .collect();
    assert_eq!(entries.len(), 1, "exactly one report: {entries:?}");
    assert!(
        entries[0].file_name().to_string_lossy().ends_with(".json"),
        "and it is JSON"
    );
}

#[test]
fn the_report_names_the_rules_it_found() {
    let fixture = Fixture::new("rules");
    fs::create_dir_all(fixture.dir.join("rules")).expect("rules dir");
    fs::write(
        fixture.dir.join("rules").join("chassis.yaml"),
        "name: Chassis intake\n\
         id: chassis-intake\n\
         enabled: true\n\
         source: { device: fan.system.fakechip_fan1, capability: fan.rpm }\n\
         target: { device: fan.mock.0, capability: fan.speed_percent }\n\
         curve: [[0, 40], [3000, 100]]\n\
         hysteresis: 2\n\
         deadband: 0\n\
         update_interval_ms: 250\n",
    )
    .expect("rule file");

    let out = fixture.dir.join("report.json");
    let output = fixture.run(&["report", "--mock", "--out", out.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).expect("readable")).expect("valid JSON");
    let rules = report["rules"].as_array().expect("rules");
    assert_eq!(
        rules.len(),
        1,
        "the installed rule is in the report: {rules:?}"
    );
    assert_eq!(rules[0]["id"], "chassis");
    assert_eq!(rules[0]["enabled"], true);
    assert_eq!(rules[0]["source"], "fan.system.fakechip_fan1/fan.rpm");
    assert_eq!(rules[0]["target"], "fan.mock.0/fan.speed_percent");
}
