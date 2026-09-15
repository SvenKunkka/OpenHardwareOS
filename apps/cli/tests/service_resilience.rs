//! What the service does when the machine changes underneath it.
//!
//! Two things the goal asks to be verified and that a lifecycle test does not cover:
//!
//! * **a sensor disappears and comes back** — the rule must fall back rather than act
//!   on a number that is no longer there, and it must steer again when the reading
//!   returns;
//! * **the process is suspended and resumed** — the closest a test can come to a
//!   laptop lid closing without suspending the machine it runs on. While it is
//!   stopped its heartbeat ages, so another process may take the channels over; when
//!   it resumes it must carry on, with its ownership intact.
//!
//! The fan channel is a real one, read through the system adapter from a prepared
//! tree (`OHM_HWMON_ROOT`), so the reading path is the code that ships. The tacho is
//! a file, which is what makes "the sensor disappears" a thing this test can do.
//! Simulated providers and dry-run throughout: nothing here can touch a fan.

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_ohm-cli");
const CHIP: &str = "fakechip";

/// A config directory, a prepared hwmon tree, and one rule reading the tachometer in
/// it and driving the simulated chassis fan.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ohm-service-resilience-{}-{name}-{}",
            std::process::id(),
            ohm_core::now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("rules")).expect("config dir");
        let hwmon = dir.join("hwmon").join("hwmon0");
        fs::create_dir_all(&hwmon).expect("hwmon dir");
        fs::write(hwmon.join("name"), format!("{CHIP}\n")).expect("chip name");
        fs::write(hwmon.join("fan1_input"), "1200\n").expect("tachometer");
        fs::write(hwmon.join("pwm1"), "128\n").expect("pwm");
        fs::write(hwmon.join("pwm1_enable"), "1\n").expect("pwm mode");
        fs::write(
            dir.join("rules").join("chassis.yaml"),
            format!(
                "name: Chassis intake\n\
                 id: chassis-intake\n\
                 enabled: true\n\
                 source: {{ device: fan.system.{CHIP}_fan1, capability: fan.rpm }}\n\
                 target: {{ device: fan.mock.0, capability: fan.speed_percent }}\n\
                 curve:\n\
                 \x20 - [0, 40]\n\
                 \x20 - [3000, 100]\n\
                 hysteresis: 2\n\
                 deadband: 0\n\
                 update_interval_ms: 250\n"
            ),
        )
        .expect("rule file");
        Self { dir }
    }

    fn tachometer(&self) -> PathBuf {
        self.dir.join("hwmon").join("hwmon0").join("fan1_input")
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("service.json")
    }

    fn state(&self) -> Option<serde_json::Value> {
        serde_json::from_str(&fs::read_to_string(self.state_path()).ok()?).ok()
    }

    /// Wait until the state file says `predicate` about a rule.
    ///
    /// The state file is what a service publishes about itself, and since this round
    /// it carries each rule's status, applied value and own message — which is the
    /// difference between "3 rules loaded" and "the chassis rule is sitting in a
    /// fail-safe because its tachometer went away".
    fn wait_for_outcome(
        &self,
        rule: &str,
        what: &str,
        predicate: impl Fn(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            if let Some(state) = self.state()
                && let Some(outcomes) = state["outcomes"].as_array()
            {
                for outcome in outcomes {
                    seen.push(format!(
                        "{} {}",
                        outcome["status"].as_str().unwrap_or("?"),
                        outcome["message"].as_str().unwrap_or("")
                    ));
                }
                if let Some(found) = outcomes
                    .iter()
                    .find(|outcome| outcome["id"].as_str() == Some(rule) && predicate(outcome))
                {
                    return found.clone();
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!(
            "never saw {what}; the state file reported:\n{}",
            seen.join("\n")
        );
    }

    // --- helpers used only by the suspend test ---------------------------------
    //
    // `SIGSTOP`/`SIGCONT` are Unix, so that test is Unix, so everything only it calls
    // must be too: `clippy -D warnings` treats them as dead code on Windows, and it
    // is right. This has now cost three CI runs across two rounds — a helper whose
    // usefulness depends on the platform it is compiled for — so the rule is written
    // here: **a helper used only by a `cfg`-gated test carries the same `cfg`.**
    #[cfg(unix)]
    fn wait_for_state(&self, what: &str) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if let Some(state) = self.state() {
                return state;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("never saw {what}");
    }

    /// Wait until the state file's cycle counter passes `previous`.
    ///
    /// Reading the file once and comparing is a race: the file exists from the moment
    /// the service claims it, with the counters still at zero, so a test that asserts
    /// "it moved on" right after resuming can read a snapshot taken before anything
    /// moved and call it a failure. It did, once, under a parallel test run.
    #[cfg(unix)]
    fn wait_for_ticks_above(&self, previous: u64) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut last = self.state();
        while Instant::now() < deadline {
            if let Some(state) = self.state() {
                if state["ticks"].as_u64().unwrap_or(0) > previous {
                    return state;
                }
                last = Some(state);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "the cycle counter never passed {previous}; last state: {}",
            last.map(|value| value.to_string()).unwrap_or_default()
        );
    }

    fn run_cli(&self, args: &[&str]) -> std::process::Output {
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

/// A service process that is stopped if the test ends before it does.
struct Running {
    child: Child,
}

impl Running {
    fn start(fixture: &Fixture, extra: &[&str]) -> Self {
        let out = fs::File::create(fixture.dir.join("service-output.log")).expect("output file");
        let mut args = vec![
            "--log-level",
            "debug",
            "service",
            "run",
            "--mock",
            "--heartbeat-ms",
            "100",
        ];
        args.extend_from_slice(extra);
        let child = Command::new(BIN)
            .args(args)
            .env("OHM_CONFIG_DIR", &fixture.dir)
            .env("OHM_HWMON_ROOT", fixture.dir.join("hwmon"))
            .stdout(Stdio::from(out.try_clone().expect("clone")))
            .stderr(Stdio::from(out))
            .spawn()
            .expect("spawn ohm-cli service");
        Self { child }
    }

    /// Only the suspend test asks who the service is; the sensor test never needs to.
    #[cfg(unix)]
    fn pid(&self) -> u32 {
        self.child.id()
    }

    #[cfg(unix)]
    fn signal(&self, signal: &str) {
        let status = Command::new("kill")
            .args([signal, &self.pid().to_string()])
            .status()
            .expect("send signal");
        assert!(status.success(), "kill {signal} {} failed", self.pid());
    }

    fn stop(mut self) -> std::process::ExitStatus {
        #[cfg(unix)]
        self.signal("-TERM");
        #[cfg(not(unix))]
        {
            let _ = self.child.kill();
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match self.child.try_wait().expect("try_wait") {
                Some(status) => return status,
                None if Instant::now() > deadline => {
                    let _ = self.child.kill();
                    panic!("the service did not stop within 30 s");
                }
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_tachometer_that_disappears_makes_the_rule_fall_back_and_recovery_follows() {
    let fixture = Fixture::new("sensor-loss");
    let service = Running::start(&fixture, &[]);

    // The rule steers from the tachometer the system adapter read out of the tree.
    let healthy = fixture.wait_for_outcome("chassis", "the rule steering", |outcome| {
        outcome["status"].as_str() != Some("fallback")
    });
    assert!(
        !healthy["message"]
            .as_str()
            .unwrap_or("")
            .contains("missing"),
        "a healthy rule does not talk about missing sensors: {healthy}"
    );

    // The tachometer disappears: a driver reload, a chip that stops answering, a
    // cable that moved. The rule must stop trusting the last number it saw — and say
    // which sensor went away, not merely that something did.
    fs::remove_file(fixture.tachometer()).expect("remove the tachometer");
    let lost = fixture.wait_for_outcome("chassis", "the fallback after sensor loss", |outcome| {
        outcome["status"].as_str() == Some("fallback")
    });
    let message = lost["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("fan.system.fakechip_fan1") && message.contains("missing"),
        "the reason names the sensor that went away: {message}"
    );
    assert_eq!(
        lost["applied"].as_f64(),
        Some(70.0),
        "and the fail-safe duty is what the fan is being held at: {lost}"
    );

    // It comes back. The rule must start reading again — not stay in the fail-safe
    // for the rest of the session, which is what it looked like before the state
    // file said what each rule was doing: the fallback *did* end, and the rule then
    // held the fail-safe value until its own hysteresis anchor moved, which is the
    // designed behaviour and not a recovery failure.
    fs::write(fixture.tachometer(), "1250\n").expect("restore the tachometer");
    let recovered = fixture.wait_for_outcome("chassis", "the rule reading again", |outcome| {
        let status = outcome["status"].as_str().unwrap_or_default();
        let message = outcome["message"].as_str().unwrap_or_default();
        status != "fallback" && !message.contains("is missing")
    });
    let message = recovered["message"].as_str().unwrap_or_default();
    assert!(
        recovered["status"].as_str() != Some("fallback"),
        "the fallback ended: {recovered}"
    );
    assert!(
        message.contains("fan.system.fakechip_fan1") || message.contains("1198"),
        "and the rule is talking about the reading that came back: {message}"
    );

    // How it stops is a Unix question: `SIGTERM` is what a service manager sends and
    // what this test can send, while Windows offers a test no way to deliver a console
    // CTRL+C to another process. So the clean-shutdown assertions belong to the Unix
    // runs, and the cross-platform coverage of that path is the bounded run in
    // `service_lifecycle.rs`, which exits on its own everywhere. The *sensor* path
    // above runs on every platform, because that is where the platform-independent
    // behaviour is.
    let exit = service.stop();
    #[cfg(unix)]
    {
        assert_eq!(exit.code(), Some(0), "SIGTERM stops it cleanly");
        assert!(
            !fixture.state_path().exists(),
            "a clean stop leaves no state file"
        );
    }
    #[cfg(not(unix))]
    {
        let _ = exit;
        assert!(
            fixture.state_path().exists(),
            "killed rather than stopped: the state file is left behind, which is what \
             the heartbeat/stale-takeover path exists for"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_suspended_service_is_treated_as_stale_and_resumes_with_its_ownership_intact() {
    let fixture = Fixture::new("suspend");
    let service = Running::start(&fixture, &[]);
    let state = fixture.wait_for_state("the state file");
    assert_eq!(state["pid"].as_u64().unwrap(), u64::from(service.pid()));

    let running = fixture.run_cli(&["service", "status"]);
    assert_eq!(running.status.code(), Some(0), "it is running");

    // Suspended: the heartbeat stops, exactly as it does when a machine sleeps.
    service.signal("-STOP");
    std::thread::sleep(Duration::from_millis(1_500));

    let during = fixture.run_cli(&["service", "status"]);
    let text = String::from_utf8_lossy(&during.stdout);
    assert_eq!(
        during.status.code(),
        Some(1),
        "a suspended service is not running any more: {text}"
    );
    assert!(
        text.contains("running:    no") && text.contains(&service.pid().to_string()),
        "and the report says whose file it is: {text}"
    );

    // Resumed: it is its own owner again, so no takeover is needed and none happens.
    service.signal("-CONT");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut recovered = None;
    while Instant::now() < deadline {
        let status = fixture.run_cli(&["service", "status"]);
        if status.status.code() == Some(0) {
            recovered = Some(status);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let recovered = recovered.expect("the service keeps its heartbeat up after resuming");
    assert!(
        String::from_utf8_lossy(&recovered.stdout).contains(&service.pid().to_string()),
        "still the same process, still the owner"
    );

    // It is working again, not merely alive: the cycle counter moves on.
    let before = state["ticks"].as_u64().unwrap_or(0);
    let after = fixture.wait_for_ticks_above(before);
    assert!(after["ticks"].as_u64().unwrap_or(0) > before);

    let exit = service.stop();
    assert_eq!(exit.code(), Some(0));
    assert!(!fixture.state_path().exists());
}

/// The override the fixtures above depend on, checked where it is cheap to check:
/// pointing the system adapter at a directory must make it read *that* directory.
#[test]
fn the_hwmon_root_override_is_what_makes_these_fixtures_possible() {
    let fixture = Fixture::new("override");
    let output = fixture.run_cli(&["status", "--json"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "status --json: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let snapshot: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("status --json is JSON");
    let ids: Vec<String> = snapshot["devices"]
        .as_array()
        .expect("devices")
        .iter()
        .map(|view| {
            view["device"]["id"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    assert!(
        ids.iter()
            .any(|id| id == &format!("fan.system.{CHIP}_fan1")),
        "the prepared tree is what was read: {ids:?}"
    );
}

/// Keeps clippy honest about the helper being used on every platform.
#[cfg(not(unix))]
#[test]
fn the_suspend_test_needs_a_signal_api_this_platform_does_not_have() {
    let fixture = Fixture::new("no-signals");
    assert!(
        PathBuf::from(&fixture.dir).exists(),
        "the fixture is built the same way everywhere; only the signal test is Unix-only"
    );
}
