//! The background service's lifecycle, driven as a real process.
//!
//! These tests start the actual `ohm-cli` binary, not the library: the questions
//! here are process questions — does a second instance refuse to start, does
//! `SIGTERM` bring the machine back to a state where nothing is driving the
//! channels, does a clean stop leave anything behind that would block the next
//! start. None of that can be answered in-process.
//!
//! Simulated hardware throughout (`--mock`), which also forces dry-run: no test in
//! this file can reach a real fan, and the state file says so.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_ohm-cli");

/// A service process that is killed if the test fails before it stops.
struct Service {
    child: Child,
    dir: PathBuf,
    log: PathBuf,
}

impl Service {
    fn spawn(dir: &Path, args: &[&str]) -> Self {
        let log = dir.join(format!("run-{}.log", std::process::id()));
        let output = std::fs::File::create(&log).expect("log file");
        let child = Command::new(BIN)
            .args(args)
            .env("OHM_CONFIG_DIR", dir)
            .stdout(Stdio::from(output.try_clone().expect("clone")))
            .stderr(Stdio::from(output))
            .spawn()
            .expect("spawn ohm-cli");
        Self {
            child,
            dir: dir.to_path_buf(),
            log,
        }
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("service.json")
    }

    fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Wait for the state file to exist, returning what it says.
    fn wait_for_state(&self, timeout: Duration) -> serde_json::Value {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(self.state_path())
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
            {
                return value;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "no state file appeared within {timeout:?}; log:\n{}",
            self.log_text()
        );
    }

    fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Ask it to stop the way a service manager does, and wait for it to exit.
    #[cfg(unix)]
    fn terminate(&mut self) -> std::process::ExitStatus {
        let pid = self.child.id().to_string();
        let status = Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .expect("send SIGTERM");
        assert!(status.success(), "kill -TERM {pid} failed");
        self.wait()
    }

    fn wait(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match self.child.try_wait().expect("try_wait") {
                Some(status) => return status,
                None if Instant::now() > deadline => {
                    let _ = self.child.kill();
                    panic!(
                        "the service did not exit within 30 s; log:\n{}",
                        self.log_text()
                    );
                }
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    fn run_cli(&self, args: &[&str]) -> std::process::Output {
        run_cli(&self.dir, args)
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if self.is_running() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn config_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ohm-service-test-{}-{name}-{}",
        std::process::id(),
        ohm_core::now_ms()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("config dir");
    dir
}

/// Run the CLI against a config directory and collect its output.
fn run_cli(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .env("OHM_CONFIG_DIR", dir)
        .output()
        .expect("run ohm-cli")
}

#[test]
fn status_says_nothing_is_running_before_anything_starts() {
    let dir = config_dir("status-empty");
    let output = run_cli(&dir, &["service", "status"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "`service status` exits 1 when nothing is running"
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("running:    no"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bounded_run_finishes_on_its_own_and_leaves_nothing_behind() {
    let dir = config_dir("bounded");
    let mut service = Service::spawn(
        &dir,
        &[
            "service",
            "run",
            "--mock",
            "--max-ticks",
            "2",
            "--heartbeat-ms",
            "100",
        ],
    );
    let status = service.wait();
    let log = service.log_text();
    assert_eq!(status.code(), Some(0), "log:\n{log}");
    assert!(log.contains("reached --max-ticks"), "{log}");
    assert!(
        !service.state_path().exists(),
        "a clean stop removes the state file: {}",
        service.state_path().display()
    );
    assert!(
        log.contains("nothing is driving the channels now"),
        "the stop is reported, not assumed: {log}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn sigterm_stops_the_service_and_releases_the_channels() {
    let dir = config_dir("sigterm");
    let mut service = Service::spawn(&dir, &["service", "run", "--mock", "--heartbeat-ms", "100"]);

    // It is running, and the file says who and what.
    let state = service.wait_for_state(Duration::from_secs(20));
    assert_eq!(state["simulated"], serde_json::json!(true));
    assert_eq!(
        state["dry_run"],
        serde_json::json!(true),
        "mock forces dry-run"
    );
    assert!(state["pid"].as_u64().unwrap_or(0) > 0);

    // A second service is refused rather than allowed to fight over channels.
    let second = service.run_cli(&["service", "run", "--mock"]);
    assert_eq!(
        second.status.code(),
        Some(3),
        "a second service exits 3: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let message = String::from_utf8_lossy(&second.stderr);
    assert!(message.contains("already running"), "{message}");
    assert!(
        message.contains(&state["pid"].as_u64().unwrap().to_string()),
        "the refusal names the process that holds the file: {message}"
    );

    // `status` agrees with the file.
    let status = service.run_cli(&["service", "status"]);
    assert_eq!(status.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&status.stdout).contains("running:    yes"));

    // SIGTERM is what a service manager sends.
    let exit = service.terminate();
    let log = service.log_text();
    assert_eq!(exit.code(), Some(0), "log:\n{log}");
    assert!(log.contains("stop requested"), "{log}");
    assert!(
        !service.state_path().exists(),
        "the state file is removed on a clean stop"
    );
    let after = service.run_cli(&["service", "status"]);
    assert_eq!(
        after.status.code(),
        Some(1),
        "and nothing is running after it"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_state_file_left_by_a_dead_process_is_taken_over() {
    let dir = config_dir("stale");
    std::fs::create_dir_all(&dir).expect("config dir");
    // What a service killed with SIGKILL leaves: a file whose heartbeat stopped.
    let stale = serde_json::json!({
        "pid": 999_999,
        "version": "0.0.0",
        "started_at_ms": ohm_core::now_ms() - 600_000,
        "heartbeat_at_ms": ohm_core::now_ms() - 600_000,
        "heartbeat_interval_ms": 1000,
        "ticks": 12,
        "rules": 0,
        "simulated": true,
        "dry_run": true
    });
    std::fs::write(
        dir.join("service.json"),
        serde_json::to_string_pretty(&stale).unwrap(),
    )
    .expect("write stale state");

    // `status` reports it as not running, and says what it found.
    let output = run_cli(&dir, &["service", "status"]);
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1));
    assert!(text.contains("running:    no"), "{text}");
    assert!(text.contains("999999"), "it names the dead pid: {text}");

    // And a new service takes the file over instead of refusing to start.
    let mut fresh = Service::spawn(
        &dir,
        &[
            "service",
            "run",
            "--mock",
            "--max-ticks",
            "2",
            "--heartbeat-ms",
            "100",
        ],
    );
    let exit = fresh.wait();
    assert_eq!(exit.code(), Some(0), "log:\n{}", fresh.log_text());
    assert!(!fresh.state_path().exists());
    let _ = std::fs::remove_dir_all(&dir);
}
