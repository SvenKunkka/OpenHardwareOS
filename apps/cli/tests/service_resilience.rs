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

    /// A fixture whose rule drives the prepared hwmon channel itself.
    ///
    /// The other fixtures drive the simulated fan, which is the right default — but a
    /// channel can only be *switched* by writing it, so a test about handing a switched
    /// channel over needs the real one: `pwm1_enable` starts at 2 (the driver controls
    /// it), the configuration confirms the channel, and the service is started without
    /// `--mock`, so its writes land in the prepared tree.
    fn writable(name: &str) -> Self {
        let fixture = Self::new(name);
        let hwmon = fixture.dir.join("hwmon").join("hwmon0");
        fs::write(hwmon.join("pwm1_enable"), "2\n").expect("driver-controlled mode");
        fs::write(
            fixture.dir.join("settings.json"),
            format!(
                r#"{{"adapter_settings": {{"system": {{"pwm_write_allow": ["fan.system.{CHIP}_fan1"]}}}}}}"#
            ),
        )
        .expect("settings");
        fs::write(
            fixture.dir.join("rules").join("chassis.yaml"),
            format!(
                "name: Chassis intake\n\
                 id: chassis-intake\n\
                 enabled: true\n\
                 source: {{ device: fan.system.{CHIP}_fan1, capability: fan.rpm }}\n\
                 target: {{ device: fan.system.{CHIP}_fan1, capability: fan.speed_percent }}\n\
                 curve:\n\
                 \x20 - [0, 40]\n\
                 \x20 - [3000, 100]\n\
                 hysteresis: 2\n\
                 deadband: 0\n\
                 update_interval_ms: 250\n"
            ),
        )
        .expect("rule file");
        fixture
    }

    /// The kernel's ownership file of the prepared channel.
    fn pwm_enable(&self) -> String {
        fs::read_to_string(self.dir.join("hwmon").join("hwmon0").join("pwm1_enable"))
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    /// Wait until the kernel file says `expected`.
    fn wait_for_pwm_enable(&self, expected: &str, what: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut last = String::new();
        while Instant::now() < deadline {
            last = self.pwm_enable();
            if last == expected {
                return last;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("pwm1_enable never became {expected} ({what}); it is {last}");
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

    /// `service-1.log`, `service-2.log`, … — see [`Running::log`].
    fn next_service_log(&self) -> PathBuf {
        let next = fs::read_dir(&self.dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| entry.file_name().to_string_lossy().starts_with("service-"))
                    .count()
            })
            .unwrap_or(0)
            + 1;
        self.dir.join(format!("service-{next}.log"))
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
    /// This process's own output.
    ///
    /// One file per process, not one per fixture: a test that starts a second service
    /// used to truncate the file the first one still had open, so the first one's
    /// later writes landed at its old offset and a log assertion read whatever
    /// happened to be there. It passed in isolation and failed under a loaded
    /// parallel run — the flakiest possible arrangement, and a test that cannot read
    /// the evidence it asserts on is not evidence.
    log: PathBuf,
}

impl Running {
    fn start(fixture: &Fixture, extra: &[&str]) -> Self {
        Self::start_with(fixture, true, extra)
    }

    /// `mock: false` starts the real providers, which is what writes to the prepared
    /// tree instead of reporting the write as simulated.
    fn start_with(fixture: &Fixture, mock: bool, extra: &[&str]) -> Self {
        let log = fixture.next_service_log();
        let out = fs::File::create(&log).expect("output file");
        let mut args = vec![
            "--log-level",
            "debug",
            "service",
            "run",
            "--heartbeat-ms",
            "100",
        ];
        if mock {
            args.push("--mock");
        }
        // Without `--mock` every write must still be real, and a dry-run would make the
        // whole point of this test unreachable.
        args.extend_from_slice(extra);
        let child = Command::new(BIN)
            .args(args)
            .env("OHM_CONFIG_DIR", &fixture.dir)
            .env("OHM_HWMON_ROOT", fixture.dir.join("hwmon"))
            .stdout(Stdio::from(out.try_clone().expect("clone")))
            .stderr(Stdio::from(out))
            .spawn()
            .expect("spawn ohm-cli service");
        Self { child, log }
    }

    /// Only the suspend test asks who the service is; the sensor test never needs to.
    #[cfg(unix)]
    fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Everything this process has written, once it has exited.
    fn log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// `SIGKILL`: no shutdown, no cleanup — the state file and the switched channel
    /// stay exactly as they are, which is the case the hand-over exists for.
    #[cfg(unix)]
    fn kill_hard(mut self) {
        self.signal("-KILL");
        let _ = self.child.wait();
    }

    #[cfg(unix)]
    fn signal(&self, signal: &str) {
        let status = Command::new("kill")
            .args([signal, &self.pid().to_string()])
            .status()
            .expect("send signal");
        assert!(status.success(), "kill {signal} {} failed", self.pid());
    }

    /// Wait for the process to exit **by itself**, without sending it anything.
    ///
    /// The stand-down path is a decision the process makes on its own; a test that
    /// signalled it would be testing its own signal instead.
    #[cfg(unix)]
    fn wait_for_exit(&mut self, within: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
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

/// The half of the suspend story the test above cannot reach: while a service is
/// stopped, another one may legitimately take the channels over. The woken process
/// must notice and stand down — writing "my file" over the new owner's record and
/// driving the same fans is how two services end up fighting over one header.
///
/// `stop_immediately` chooses *when* the first service is stopped: as soon as it has
/// claimed the file, or once it is running the rules. Both are real — a laptop can
/// sleep while a service is starting at boot — and both must end the same way, which
/// is why the assertions below are identical for the two callers.
#[cfg(unix)]
fn takeover_after_a_stop(name: &str, stop_immediately: bool) {
    let fixture = Fixture::new(name);
    let first = Running::start(&fixture, &[]);
    let claimed = fixture.wait_for_state("the first owner's state file");
    assert_eq!(claimed["pid"].as_u64().unwrap(), u64::from(first.pid()));
    if !stop_immediately {
        // Running the rules, not merely owning the file.
        let running = fixture.wait_for_ticks_above(0);
        assert_eq!(running["pid"].as_u64().unwrap(), u64::from(first.pid()));
    }

    // Stopped past the heartbeat window. From outside this is indistinguishable from
    // a sleeping machine, which is exactly why the heartbeat window is what decides
    // ownership while the process cannot speak for itself.
    first.signal("-STOP");
    std::thread::sleep(Duration::from_millis(1_500));
    let during = fixture.run_cli(&["service", "status"]);
    assert_eq!(
        during.status.code(),
        Some(1),
        "the stopped service is not considered alive: {}",
        String::from_utf8_lossy(&during.stdout)
    );

    // A second service takes the stale file over and starts driving the rules.
    let second = Running::start(&fixture, &[]);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut taken = None;
    while Instant::now() < deadline {
        if let Some(state) = fixture.state()
            && state["pid"].as_u64() == Some(u64::from(second.pid()))
        {
            taken = Some(state);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let taken = taken.expect("the second service takes the channels over");
    assert_eq!(taken["pid"].as_u64(), Some(u64::from(second.pid())));

    // The first one wakes up. Its next heartbeat must find the file is not its own.
    first.signal("-CONT");
    let mut first = first;
    let exit = first
        .wait_for_exit(Duration::from_secs(30))
        .expect("the woken service stops instead of carrying on");
    let log = first.log();
    assert_eq!(
        exit.code(),
        Some(4),
        "it reports the loss rather than a clean stop; log:\n{log}"
    );
    assert!(
        log.contains("took over the channels"),
        "and says who owns them now:\n{log}"
    );
    assert!(
        log.contains("left untouched"),
        "and that it did not touch that process's file:\n{log}"
    );
    assert!(
        !log.contains("is pid                      "),
        "the message has no whitespace artefact in it:\n{log}"
    );

    // The second service still owns the file, and is still working: the first one
    // neither overwrote the record nor deleted it on the way out.
    let after = fixture
        .state()
        .expect("the successor's state file survives");
    assert_eq!(after["pid"].as_u64(), Some(u64::from(second.pid())));
    let ticks = after["ticks"].as_u64().unwrap_or(0);
    let moved = fixture.wait_for_ticks_above(ticks);
    assert!(
        moved["ticks"].as_u64().unwrap_or(0) > ticks,
        "the successor keeps running the rules after the other one stood down"
    );

    let exit = second.stop();
    assert_eq!(exit.code(), Some(0), "the successor stops cleanly");
    assert!(!fixture.state_path().exists(), "and leaves nothing behind");
}

#[cfg(unix)]
#[test]
fn a_service_that_wakes_up_after_a_takeover_stands_down_instead_of_writing() {
    takeover_after_a_stop("takeover-running", false);
}

/// The same property, for a suspend that lands in the middle of the service's own
/// start-up: the claim happens before the engine starts, so a machine that sleeps in
/// that window wakes up with its file already taken over.
#[cfg(unix)]
#[test]
fn a_service_suspended_during_its_own_start_up_also_stands_down() {
    takeover_after_a_stop("takeover-starting", true);
}

/// The hand-over the previous round recorded as a known gap.
///
/// A service that switches a channel to manual mode and is then killed never runs its
/// own shutdown, so the channel stays switched and the only record of what it was
/// before is the state file it left behind. A successor that does not adopt that
/// record reads the *switched* value as the original and faithfully restores it — a fan
/// left in manual mode for good. This test walks that whole path with a real file.
#[cfg(unix)]
#[test]
fn a_channel_left_switched_by_a_killed_service_comes_back() {
    let fixture = Fixture::writable("handover");
    assert_eq!(
        fixture.pwm_enable(),
        "2",
        "the driver controls the channel to begin with"
    );

    // The first service takes control of the channel to write it.
    let first = Running::start_with(&fixture, false, &[]);
    fixture.wait_for_pwm_enable("1", "the first service switched the channel to manual");
    // And it publishes what it switched, which is the only thing that can save the
    // channel after it is gone. The publication does not wait for the next heartbeat:
    // a service killed in that window would leave a switched channel with no record.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut taken = Vec::new();
    while Instant::now() < deadline {
        taken = fixture
            .state()
            .and_then(|state| state["taken"].as_array().cloned())
            .unwrap_or_default();
        if !taken.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        taken.len(),
        1,
        "the owner publishes what it switched: {taken:?}"
    );
    assert_eq!(taken[0]["device_id"], format!("fan.system.{CHIP}_fan1"));
    assert_eq!(taken[0]["original"], 2, "{taken:?}");

    // Killed hard: no shutdown, no release — exactly a crash or a power cut.
    first.kill_hard();
    assert_eq!(fixture.pwm_enable(), "1", "the channel is still switched");
    assert!(
        fixture.state_path().exists(),
        "and its record is still there, which is what the next start takes over"
    );

    // Nothing was cleaned up, so the file still looks alive for one heartbeat window —
    // three intervals, which is exactly the latency this design accepts for a crash.
    // A second service started inside that window is refused, and that is the
    // one-writer rule working, not a failure: wait for the documented staleness instead
    // of guessing a sleep.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if fixture.run_cli(&["service", "status"]).status.code() == Some(1) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the killed owner's file never went stale"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // The successor takes the record over and puts the channel back where the dead
    // service said it was — then takes control again for its own writes, remembering
    // the *original* value rather than the one it found.
    let second = Running::start_with(&fixture, false, &[]);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut adopted = None;
    while Instant::now() < deadline {
        if let Some(state) = fixture.state()
            && state["pid"].as_u64() == Some(u64::from(second.pid()))
            && state["ticks"].as_u64().unwrap_or(0) > 0
        {
            adopted = Some(state);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let log = second.log();
    assert!(
        adopted.is_some(),
        "the successor runs the rules; its log so far:\n{log}"
    );
    assert!(
        log.contains("left 1 channel(s) switched to manual mode"),
        "it says what it inherited:\n{log}"
    );
    assert!(
        log.contains("put a channel left in manual mode by a previous owner back under its driver"),
        "and that it put that channel back:\n{log}"
    );

    // The proof is the state of the kernel file once the successor stops cleanly: 2,
    // the value the *first* service recorded. Without the hand-over this process would
    // have remembered the 1 it found and written that back instead.
    let exit = second.stop();
    assert_eq!(exit.code(), Some(0), "the successor stops cleanly");
    assert_eq!(
        fixture.pwm_enable(),
        "2",
        "the driver owns the channel again, because the dead owner's record was adopted"
    );
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
