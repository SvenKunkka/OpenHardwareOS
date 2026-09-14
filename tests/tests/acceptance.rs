//! Acceptance scenarios from the project brief, executed end to end.
//!
//! ```text
//! Scenario A — start the app and see CPU / GPU / SSD
//! Scenario B — every device exposes its capabilities
//! Scenario C — Mock GPU temperature -> rule -> fan RPM, dynamically
//! Scenario D — real fan control, or an honest refusal explaining why
//! ```
//!
//! These tests run real runtimes with real adapters; only the hardware is
//! simulated, so they pass on any machine and in CI.

use ohm_adapter_mock::MockConfig;
use ohm_core::ids::capability as caps;
use ohm_device_model::DeviceType;
use ohm_integration_tests::{Session, heavy_load, idle_load};
use ohm_runtime::WriteOrigin;

/// Scenario A on a real machine: the OS adapter reports the CPU, and the
/// storage devices are there even when their temperatures are not readable.
#[tokio::test]
async fn scenario_a_the_machine_is_detected_automatically() {
    let session = Session::with_os_adapter().await;
    let devices = session.runtime.devices();

    // The simulated provider also exposes a CPU, so select the real one.
    let cpu = devices
        .iter()
        .find(|view| view.device.device_type == DeviceType::Cpu && view.adapter == "system")
        .expect("a CPU is always detectable through the operating system");
    assert!(!cpu.device.name.is_empty());
    assert!(cpu.device.supports(caps::CPU_LOAD));
    assert!(cpu.device.metadata.contains_key("cores"));

    // CPU load must be a plausible number, never a placeholder.
    let load = cpu
        .state
        .as_ref()
        .and_then(|state| state.number(caps::CPU_LOAD))
        .expect("CPU load is always readable");
    assert!((0.0..=100.0).contains(&load), "load out of range: {load}");

    // The simulated machine fills in the GPU and SSD that this host may not
    // have, so the Overview page always has something real to show.
    assert!(
        devices
            .iter()
            .any(|view| view.device.device_type == DeviceType::Gpu)
    );
    assert!(
        devices
            .iter()
            .any(|view| view.device.device_type == DeviceType::Storage)
    );

    for view in &devices {
        assert!(!view.device.capabilities.is_empty());
    }
    session.shutdown().await;
}

/// Scenario B: capabilities are declared, and unreadable ones say why.
#[tokio::test]
async fn scenario_b_capabilities_are_complete_and_honest() {
    let session = Session::simulated().await;
    let snapshot = session.runtime.snapshot();

    let gpu = snapshot.device("gpu.mock.0").expect("gpu present");
    let capability_ids: Vec<&str> = gpu
        .device
        .capabilities
        .iter()
        .map(|capability| capability.id.as_str())
        .collect();
    assert!(capability_ids.contains(&caps::TEMPERATURE_CORE));
    assert!(capability_ids.contains(&caps::TEMPERATURE_HOTSPOT));
    assert!(capability_ids.contains(&caps::GPU_LOAD));
    assert!(capability_ids.contains(&caps::POWER_GPU));
    assert!(capability_ids.contains(&caps::FAN_RPM));

    // Every declared capability has a reading, even when it is unavailable.
    let state = gpu.state.as_ref().expect("gpu state");
    for capability in &gpu.device.capabilities {
        let reading = state
            .get(capability.id.as_str())
            .unwrap_or_else(|| panic!("no reading for {}", capability.id));
        if !reading.is_ok() {
            assert!(
                reading.reason().is_some(),
                "an unavailable reading must explain itself"
            );
        }
    }

    // The index the Automation page builds its dropdowns from.
    let index = session.runtime.capability_index();
    assert!(
        index
            .sources
            .iter()
            .any(|source| source.qualified_id() == "gpu.mock.0/temperature.core")
    );
    assert!(
        index
            .targets
            .iter()
            .any(|target| target.qualified_id() == "fan.mock.0/fan.speed_percent")
    );

    // Not one source is writable, and not one target is read-only.
    assert!(
        index
            .sources
            .iter()
            .all(|source| !source.capability.writable)
    );
    assert!(
        index
            .targets
            .iter()
            .all(|target| target.capability.writable)
    );
    session.shutdown().await;
}

/// Scenario C: the closed loop. This is the test the whole MVP exists to pass.
#[tokio::test]
async fn scenario_c_mock_gpu_temperature_drives_the_fan() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        gpu_start_c: 45.0,
        ..MockConfig::deterministic()
    })
    .await;
    session
        .engine
        .save_rule(session.gpu_rule())
        .expect("rule installs");

    // The GPU heats up, and the rule follows it: more heat, more airflow.
    let mut duties = Vec::new();
    let mut temperatures = Vec::new();
    for _ in 0..90 {
        session.step(1_000).await;
        duties.push(session.fan_duty());
        temperatures.push(
            session
                .reading("gpu.mock.0", caps::TEMPERATURE_CORE)
                .expect("temperature"),
        );
    }

    let first_duty = duties[0];
    let peak_duty = duties.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(
        peak_duty > first_duty,
        "the fan must respond to rising temperature ({first_duty} -> {peak_duty})"
    );

    // The write really reached the simulated hardware.
    let outcome = session.engine.outcome("gpu-cooling").expect("rule outcome");
    assert!(outcome.writes > 0, "the rule never wrote");
    assert_eq!(outcome.fallbacks, 0);

    // And every write is in the audit log.
    let audit = session.runtime.audit().tail(200).expect("audit");
    assert!(
        audit
            .iter()
            .any(|entry| entry["kind"] == "write"
                && entry["report"]["origin"]["kind"] == "automation"),
        "automation writes must be audited"
    );

    // Now stop the load: the machine cools and the fan eases off.
    session.mock.set_gpu_load(0.02);
    for _ in 0..400 {
        session.step(1_000).await;
    }
    let idle_duty = session.fan_duty();
    let idle_temp = session
        .reading("gpu.mock.0", caps::TEMPERATURE_CORE)
        .expect("temperature");
    assert!(
        idle_duty < peak_duty,
        "an idle GPU should not keep the fan at {idle_duty} % (peak was {peak_duty} %)"
    );
    assert!(
        idle_temp < 60.0,
        "the machine should have cooled: {idle_temp}"
    );

    session.shutdown().await;
}

/// Scenario D: when real fan control exists it works; when it does not, the
/// refusal explains itself and nothing is faked.
#[tokio::test]
async fn scenario_d_real_writes_are_attempted_and_refusals_are_explained() {
    let session = Session::simulated().await;

    // A simulated fan accepts the write and reports how it was applied.
    let report = session
        .runtime
        .write_value(
            "fan.mock.0",
            caps::FAN_SPEED_PERCENT,
            ohm_device_model::Value::Number(72.0),
            WriteOrigin::Manual,
        )
        .await
        .expect("simulated write succeeds");
    assert_eq!(report.applied, Some(ohm_device_model::Value::Number(72.0)));
    assert!(report.simulated, "simulated hardware must say so");
    assert_eq!(session.fan_duty(), 72.0);

    // A read-only capability is refused with a machine readable reason and a
    // hint that tells the user what to do.
    let error = session
        .runtime
        .write_value(
            "fan.mock.0",
            caps::FAN_RPM,
            ohm_device_model::Value::Number(1200.0),
            WriteOrigin::Manual,
        )
        .await
        .expect_err("read-only writes must fail");
    assert_eq!(error.code(), "capability_read_only");
    assert!(error.is_unsupported());
    assert!(!error.hint().is_empty());

    // A refused write is still audited: nothing may change hardware silently.
    let audit = session.runtime.audit().tail(20).expect("audit");
    assert!(
        audit
            .iter()
            .any(|entry| entry["report"]["status"] == "rejected")
    );

    // The same is true for a device that does not exist.
    let error = session
        .runtime
        .write_value(
            "fan.ghost.0",
            caps::FAN_SPEED_PERCENT,
            ohm_device_model::Value::Number(50.0),
            WriteOrigin::Manual,
        )
        .await
        .expect_err("unknown devices must fail");
    assert_eq!(error.code(), "device_not_found");

    session.shutdown().await;
}

/// The whole point of the safety layer: an over-temperature machine is forced
/// to full airflow even with no rules configured at all.
#[tokio::test]
async fn emergency_override_protects_an_unattended_machine() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: idle_load(),
        ..MockConfig::deterministic()
    })
    .await;

    // No rules at all: only the runtime's own supervisor is active.
    assert_eq!(session.engine.rule_count(), 0);
    session.mock.force_gpu_temperature(96.0);
    session.step_without_rules(1_000).await;

    let duty = session.fan_duty();
    assert!(
        duty >= 100.0,
        "an emergency must force every controlled output to maximum, got {duty}"
    );
    assert!(session.runtime.stats().safety_interventions > 0);

    // The UI is told, and the audit log records why.
    let audit = session.runtime.audit().tail(20).expect("audit");
    assert!(
        audit
            .iter()
            .any(|entry| entry["action"] == "emergency_override")
    );

    // The supervisor only ever *raises* airflow; it deliberately never decides
    // on its own that a fan may slow down again. Bringing the duty back to a
    // normal value is the automation engine's (or the user's) job, and this is
    // what that handover looks like.
    session.mock.force_gpu_temperature(45.0);
    session.mock.set_load_profile(idle_load());
    session
        .engine
        .save_rule(session.gpu_rule())
        .expect("rule installs");
    for _ in 0..200 {
        session.step(1_000).await;
    }
    let idle_duty = session.fan_duty();
    assert!(
        idle_duty < 100.0,
        "once the rule is in charge an idle machine must not stay at 100 % (got {idle_duty})"
    );
    assert_eq!(
        session.engine.outcome("gpu-cooling").unwrap().fallbacks,
        0,
        "a recovered machine must not be in fallback"
    );

    session.shutdown().await;
}
