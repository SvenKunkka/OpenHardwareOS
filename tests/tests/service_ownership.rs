//! One writer per channel, across processes.
//!
//! A background service and the desktop application run the same rules against the
//! same hardware. Neither can see the other's engine, so the state file is the only
//! thing that can stop them alternating values onto one fan. These tests drive the
//! real `AutomationEngine` against a state file that names a process other than this
//! one, which is what "a service is running" looks like from inside the desktop.
//!
//! The foreign pid is 1 — a process every platform this build targets has, and
//! certainly not this test — rather than a spawned child, so the guard is exercised
//! on Windows and macOS as well as on Linux.

use std::path::PathBuf;

use ohm_adapter_mock::MockConfig;
use ohm_automation::AutomationEngine;
use ohm_integration_tests::{Session, idle_load};
use ohm_runtime::{Runtime, ServiceGuard, ServiceState};

/// A state file that says "a service is running", owned by a process that is not us.
fn state(pid: u32) -> ServiceState {
    ServiceState {
        pid,
        version: "0.0.0".into(),
        started_at_ms: ohm_core::now_ms(),
        heartbeat_at_ms: ohm_core::now_ms(),
        heartbeat_interval_ms: 1_000,
        ticks: 12,
        rules: 1,
        simulated: false,
        dry_run: false,
    }
}

#[tokio::test]
async fn an_engine_stands_down_while_a_service_owns_the_channels() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: idle_load(),
        ..MockConfig::deterministic()
    })
    .await;

    let path = session.runtime.paths().service_state_file();
    let guard = ServiceGuard::acquire(&path, state(1)).expect("claim the state file");

    session.engine.start().await.expect("start returns Ok");

    let stats = session.engine.stats();
    let blocked = stats
        .blocked_by
        .as_deref()
        .expect("the engine says why it is not running");
    assert!(
        blocked.contains("pid 1"),
        "the reason names the process that holds the channels: {blocked}"
    );
    assert_eq!(
        stats.ticks, 0,
        "and no automation cycle ran: an engine that stands down must not tick"
    );

    drop(guard);
    session.shutdown().await;
}

#[tokio::test]
async fn an_engine_started_by_the_service_itself_does_not_block_itself() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: idle_load(),
        ..MockConfig::deterministic()
    })
    .await;

    // The state file names *this* process, which is exactly what the service writes
    // about itself before starting its own rule loop.
    let path = session.runtime.paths().service_state_file();
    let guard =
        ServiceGuard::acquire(&path, state(std::process::id())).expect("claim the state file");

    session.engine.start().await.expect("start");

    assert!(
        session.engine.stats().blocked_by.is_none(),
        "a process is allowed to run the rules it wrote the state file for"
    );

    drop(guard);
    session.shutdown().await;
}

#[tokio::test]
async fn a_stale_state_file_does_not_stop_the_engine() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: idle_load(),
        ..MockConfig::deterministic()
    })
    .await;

    // A file whose heartbeat stopped: a service killed with SIGKILL leaves one.
    // Blocking automation because of it would leave a machine that cannot cool
    // itself until somebody deletes a file by hand.
    let path = session.runtime.paths().service_state_file();
    let mut dead = state(1);
    dead.heartbeat_at_ms = ohm_core::now_ms() - 600_000;
    std::fs::create_dir_all(path.parent().unwrap()).expect("config dir");
    std::fs::write(&path, serde_json::to_string(&dead).unwrap()).expect("write stale file");

    session.engine.start().await.expect("start");
    assert!(
        session.engine.stats().blocked_by.is_none(),
        "an abandoned state file is not an owner"
    );

    session.shutdown().await;
}

/// The guard reads the *runtime's* paths, so a test that pointed the engine at a
/// different directory would prove nothing. This pins that assumption.
#[tokio::test]
async fn the_engine_reads_the_state_file_of_the_runtime_it_belongs_to() {
    let session = Session::simulated_with(MockConfig::deterministic()).await;
    let engine: &AutomationEngine = &session.engine;
    let runtime: &Runtime = &session.runtime;
    let expected: PathBuf = session.paths.service_state_file();
    assert_eq!(runtime.paths().service_state_file(), expected);
    assert_eq!(engine.runtime().paths().service_state_file(), expected);
    session.shutdown().await;
}
