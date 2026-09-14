//! The failure paths the brief calls out explicitly:
//!
//! ```text
//! Sensor disconnect     Actuator failure     Invalid value
//! Device hotplug        Pump floor           Write permission
//! ```
//!
//! A Hardware OS is judged by what it does when things go wrong, so each of
//! these asserts both the *behaviour* (what the hardware ends up doing) and the
//! *reporting* (what the user is told).

use ohm_adapter_mock::{LoadProfile, MockConfig, MockFaults};
use ohm_automation::{Fallback, FallbackAction, RuleStatus};
use ohm_core::ids::capability as caps;
use ohm_device_model::{DeviceState, UnavailableReason, Value};
use ohm_integration_tests::{Session, gpu_sensor_disconnected, heavy_load, idle_load};
use ohm_runtime::{DeviceStatus, WriteOrigin};

/// Losing a sensor must never leave a fan wherever it happened to be.
#[tokio::test]
async fn sensor_disconnect_falls_back_to_a_safe_duty() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;
    session.engine.save_rule(session.gpu_rule()).unwrap();
    for _ in 0..10 {
        session.step(1_000).await;
    }
    let before = session.fan_duty();
    assert!(before > 25.0, "the fan should be running: {before}");

    // The sensor disappears.
    session.mock.set_faults(gpu_sensor_disconnected());
    session.step(1_000).await;
    session.engine.tick_force().await;

    let outcome = session.engine.outcome("gpu-cooling").unwrap();
    assert_eq!(outcome.status, RuleStatus::Fallback);
    assert_eq!(
        outcome.applied_output,
        Some(70.0),
        "the fail-safe duty from the safety policy"
    );
    assert_eq!(session.fan_duty(), 70.0);

    // The device itself is reported as degraded, with the reason attached.
    let gpu = session.runtime.device("gpu.mock.0").unwrap();
    assert_eq!(gpu.status, DeviceStatus::Degraded);
    let reading = gpu
        .state
        .as_ref()
        .unwrap()
        .get(caps::TEMPERATURE_CORE)
        .unwrap();
    assert_eq!(reading.reason(), Some(UnavailableReason::ReadError));
    assert!(
        reading.value().is_none(),
        "a missing reading must not be zero"
    );

    // When the sensor returns, the rule takes control again on its own.
    session.mock.set_faults(MockFaults::default());
    session.step(1_000).await;
    session.engine.tick_force().await;
    let outcome = session.engine.outcome("gpu-cooling").unwrap();
    assert!(
        matches!(outcome.status, RuleStatus::Applied | RuleStatus::Held),
        "unexpected status {:?}",
        outcome.status
    );
    session.shutdown().await;
}

/// A rule configured to release control must give the channel back instead of
/// guessing.
#[tokio::test]
async fn release_fallback_hands_control_back() {
    let session = Session::simulated().await;
    let rule = session.gpu_rule().with_fallback(Fallback {
        on_sensor_missing: FallbackAction::Release,
        ..Fallback::default()
    });
    session.engine.save_rule(rule).unwrap();
    session.step(1_000).await;

    session.mock.set_faults(gpu_sensor_disconnected());
    session.step(1_000).await;
    session.engine.tick_force().await;

    let outcome = session.engine.outcome("gpu-cooling").unwrap();
    assert_eq!(outcome.status, RuleStatus::Released);
    assert!(outcome.message.contains("releasing control"));
    session.shutdown().await;
}

/// A short sensor outage inside the configured grace period must not abandon the
/// curve; past it, the fail-safe duty takes over.
#[tokio::test]
async fn a_configured_grace_period_rides_out_a_sensor_blip() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;

    // The grace period is wall-clock time (a sensor timeout is about real time,
    // not simulated steps), so the test keeps it short and really waits.
    let rule = session.gpu_rule().with_fallback(Fallback {
        on_sensor_missing: FallbackAction::SafeDefault,
        sensor_timeout_s: 1,
        ..Fallback::default()
    });
    session.engine.save_rule(rule).unwrap();
    for _ in 0..10 {
        session.step(1_000).await;
    }
    let duty_before = session.fan_duty();
    assert!(duty_before > 25.0);

    // The sensor disappears. A single blip is absorbed: the fan keeps its value
    // instead of being driven to the fail-safe duty.
    session.mock.set_faults(gpu_sensor_disconnected());
    session.step(1_000).await;
    session.engine.tick_force().await;
    let outcome = session.engine.outcome("gpu-cooling").unwrap();
    assert_ne!(
        outcome.status,
        RuleStatus::Fallback,
        "a blip inside the grace period must not trigger the fallback"
    );
    assert_eq!(
        session.fan_duty(),
        duty_before,
        "the fan must not have moved during the grace period"
    );
    assert_eq!(outcome.fallbacks, 0);

    // Past the grace period the fail-safe duty is applied. Polling continues
    // throughout, so this is the grace period expiring — not the runtime going
    // stale, which would fall back for a different reason.
    for _ in 0..12 {
        session.step_without_rules(1_000).await;
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        session.engine.tick_force().await;
    }
    let outcome = session.engine.outcome("gpu-cooling").unwrap();
    assert_eq!(
        outcome.status,
        RuleStatus::Fallback,
        "after the grace period the rule must fail safe (message: {})",
        outcome.message
    );
    assert_eq!(session.fan_duty(), 70.0);
    assert!(outcome.fallbacks >= 1);

    session.shutdown().await;
}

/// Without a grace period the first missing reading falls back immediately, so
/// the default stays predictable.
#[tokio::test]
async fn the_default_reacts_to_the_first_missing_reading() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;
    let rule = session.gpu_rule();
    assert_eq!(rule.fallback.sensor_timeout_s, 0);
    session.engine.save_rule(rule).unwrap();
    for _ in 0..5 {
        session.step(1_000).await;
    }

    session.mock.set_faults(gpu_sensor_disconnected());
    session.step(1_000).await;
    session.engine.tick_force().await;
    let outcome = session.engine.outcome("gpu-cooling").unwrap();
    assert_eq!(outcome.status, RuleStatus::Fallback);
    assert_eq!(session.fan_duty(), 70.0);

    session.shutdown().await;
}

/// An actuator that refuses writes must be reported, audited, and the manager
/// must still try the configured fallback.
#[tokio::test]
async fn actuator_failure_is_reported_and_audited() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;
    session.engine.save_rule(session.gpu_rule()).unwrap();
    session.step(1_000).await;
    assert!(session.engine.outcome("gpu-cooling").unwrap().writes > 0);

    // From now on the driver refuses every write.
    session.mock.set_faults(MockFaults::writes_fail());
    session.step(1_000).await;
    session.engine.tick_force().await;

    let outcome = session.engine.outcome("gpu-cooling").unwrap();
    assert_eq!(
        outcome.status,
        RuleStatus::Error,
        "message: {}",
        outcome.message
    );
    assert!(outcome.message.contains("failed"));

    // The refusal reached the audit log with a machine readable code.
    let audit = session.runtime.audit().tail(50).unwrap();
    let rejection = audit
        .iter()
        .rev()
        .find(|entry| entry["report"]["status"] == "rejected")
        .expect("a rejection must be audited");
    assert_eq!(rejection["report"]["error_code"], "write_rejected");
    assert!(session.runtime.stats().writes_rejected > 0);
    session.shutdown().await;
}

/// Invalid values never reach an adapter: they are rejected by the runtime with
/// a precise reason.
#[tokio::test]
async fn invalid_values_are_rejected_before_the_hardware() {
    let session = Session::simulated().await;
    let duties_before = session.mock.status().fan_duties.clone();

    for (value, expected) in [
        (Value::Number(180.0), "value_out_of_range"),
        (Value::Number(-10.0), "value_out_of_range"),
        (Value::Number(f64::NAN), "invalid_value"),
        (Value::Text("loud".into()), "invalid_value"),
    ] {
        let error = session
            .runtime
            .write_value(
                "fan.mock.0",
                caps::FAN_SPEED_PERCENT,
                value,
                WriteOrigin::Manual,
            )
            .await
            .expect_err("must be rejected");
        assert_eq!(error.code(), expected);
    }

    assert_eq!(
        session.mock.status().fan_duties,
        duties_before,
        "the simulated hardware must not have moved at all"
    );
    session.shutdown().await;
}

/// Hotplug: a device that vanishes is removed, and one that returns is
/// re-registered with the same id, so rules keep working.
#[tokio::test]
async fn device_hotplug_is_detected_in_both_directions() {
    let session = Session::simulated().await;
    session.engine.save_rule(session.gpu_rule()).unwrap();
    session.step(1_000).await;
    assert!(session.runtime.device("fan.mock.0").is_some());

    // Unplug the fan; the table must notice on the next discovery cycle.
    session
        .mock
        .set_faults(MockFaults::device_unplugged("fan.mock.0"));
    session.runtime.refresh_devices().await.unwrap();
    assert!(
        session.runtime.device("fan.mock.0").is_none(),
        "an unplugged device must disappear"
    );

    // The rule that targeted it now reports an error instead of panicking.
    session.engine.tick_force().await;
    let outcome = session.engine.outcome("gpu-cooling").unwrap();
    assert_eq!(outcome.status, RuleStatus::Error);
    assert!(outcome.message.contains("unavailable") || outcome.message.contains("not found"));

    // Plug it back in: same id, same capabilities, rule works again.
    session.mock.set_faults(MockFaults::default());
    session.runtime.refresh_devices().await.unwrap();
    let fan = session.runtime.device("fan.mock.0").expect("fan is back");
    assert!(fan.device.supports(caps::FAN_SPEED_PERCENT));
    session.step(1_000).await;
    session.engine.tick_force().await;
    let outcome = session.engine.outcome("gpu-cooling").unwrap();
    assert!(matches!(
        outcome.status,
        RuleStatus::Applied | RuleStatus::Held
    ));
    session.shutdown().await;
}

/// A rule asking for 0 % on a fan is clamped to the safety floor; a pump can
/// never be stopped at all.
#[tokio::test]
async fn safety_floor_applies_to_fans_and_permanently_to_pumps() {
    let session = Session::simulated_with(MockConfig {
        devices: ohm_adapter_mock::MockDevices {
            pumps: 1,
            ..ohm_adapter_mock::MockDevices::default()
        },
        ..MockConfig::deterministic()
    })
    .await;

    // Direct write below the fan floor.
    let report = session
        .runtime
        .write_value(
            "fan.mock.0",
            caps::FAN_SPEED_PERCENT,
            Value::Number(0.0),
            WriteOrigin::Manual,
        )
        .await
        .unwrap();
    assert!(report.clamped, "the write must be recorded as clamped");
    assert_eq!(report.applied, Some(Value::Number(25.0)));
    assert_eq!(session.mock.status().fan_duties[0], 25.0);

    // A pump is protected by *two* layers. The simulated pump declares a
    // 60-100 % range, so a stop request is refused outright as out of range...
    let error = session
        .runtime
        .write_value(
            "pump.mock.0",
            caps::PUMP_SPEED_PERCENT,
            Value::Number(0.0),
            WriteOrigin::Manual,
        )
        .await
        .expect_err("a pump must never be driven to zero");
    assert_eq!(error.code(), "value_out_of_range");
    assert_eq!(
        session.mock.status().pump_duties[0],
        40.0_f64.max(60.0),
        "the pump must not have moved"
    );

    // ...and the safety policy would raise anything below its own floor, which
    // is the layer that protects a control channel declared as 0-100 %.
    let relaxed = session
        .runtime
        .write_value(
            "pump.mock.0",
            caps::PUMP_SPEED_PERCENT,
            Value::Number(60.0),
            WriteOrigin::Manual,
        )
        .await
        .expect("the minimum legal pump duty is accepted");
    assert_eq!(relaxed.applied, Some(Value::Number(60.0)));
    assert_eq!(session.mock.status().pump_duties[0], 60.0);

    // Even a rule that maps a cold sensor to 0 % cannot defeat the floor.
    let cold_rule = ohm_automation::Rule::new(
        "cold-pump",
        "Cold Pump",
        ohm_automation::Source::sensor("gpu.mock.0", caps::TEMPERATURE_CORE),
        ohm_automation::Target::new("pump.mock.0", caps::PUMP_SPEED_PERCENT),
        ohm_automation::Curve::expect([(20.0, 0.0), (90.0, 100.0)]),
    )
    .unwrap()
    .with_deadband(0.0);
    session.engine.save_rule(cold_rule).unwrap();
    session.mock.force_gpu_temperature(25.0);
    session.step(1_000).await;
    session.engine.tick_force().await;
    assert!(
        session.mock.status().pump_duties[0] >= 60.0,
        "the pump floor must survive the curve"
    );
    session.shutdown().await;
}

/// The `mock.enable_gpu_fan_control` switch, end to end: turning it off must
/// remove the *control* channel while keeping the GPU monitored, and every write
/// path must refuse — not just the UI.
#[tokio::test]
async fn disabling_gpu_fan_control_keeps_monitoring_but_removes_control() {
    use ohm_adapter_api::HardwareAdapter;
    use ohm_adapters::{AdapterOptions, build_adapters};
    use ohm_runtime::SettingsStore;
    use std::sync::Arc;

    let temp = tempfile::tempdir().unwrap();
    let paths = ohm_core::ConfigPaths::from_root(temp.path());
    paths.ensure().unwrap();

    // The user turns GPU fan control off in Settings.
    let mut settings = ohm_runtime::Settings {
        experimental_features: true,
        ..Default::default()
    };
    settings.set_adapter_setting("mock", "enabled", serde_json::json!(true));
    settings.set_adapter_setting("mock", "enable_gpu_fan_control", serde_json::json!(false));
    SettingsStore::new(&paths).save(&settings).unwrap();

    // It survives a restart: the switch is read back from disk.
    let reloaded = SettingsStore::new(&paths).load().unwrap();
    assert_eq!(
        reloaded
            .adapter_setting("mock", "enable_gpu_fan_control")
            .and_then(|value| value.as_bool()),
        Some(false)
    );

    let options = AdapterOptions::from_settings(&reloaded);
    assert!(
        !options.mock_config.gpu_fan_control,
        "the saved switch must reach the adapter configuration"
    );
    let adapters: Vec<Arc<dyn HardwareAdapter>> = build_adapters(&options);
    let runtime = ohm_runtime::Runtime::new(paths.clone(), reloaded, adapters).unwrap();
    runtime.start().await.unwrap();

    // Monitoring is intact: the GPU is there with its sensors.
    let gpu = runtime
        .device("gpu.mock.0")
        .expect("the GPU is still monitored");
    assert!(gpu.device.supports(caps::TEMPERATURE_CORE));
    assert!(gpu.device.supports(caps::GPU_LOAD));
    assert!(gpu.device.supports(caps::POWER_GPU));
    assert!(
        gpu.state
            .as_ref()
            .and_then(|state| state.number(caps::TEMPERATURE_CORE))
            .is_some(),
        "GPU temperature is still read"
    );

    // The fan channel is not advertised at all, so no rule can target it.
    assert!(!gpu.device.is_controllable());
    let fan = gpu
        .device
        .capability_str(caps::FAN_SPEED_PERCENT)
        .expect("the fan is still reported, as a read-only value");
    assert!(!fan.writable);
    let index = runtime.capability_index();
    assert!(
        !index
            .targets
            .iter()
            .any(|target| target.device_id.as_str() == "gpu.mock.0"),
        "a locked GPU fan must not appear as an automation target"
    );

    // And the backend refuses a write even if one is attempted directly.
    let error = runtime
        .write_value(
            "gpu.mock.0",
            caps::FAN_SPEED_PERCENT,
            Value::Number(80.0),
            WriteOrigin::Manual,
        )
        .await
        .expect_err("a locked GPU fan must refuse writes");
    assert_eq!(error.code(), "capability_read_only");

    runtime.shutdown().await.unwrap();
}

/// The two tray switches are persisted and reloaded, and their platform limits
/// are reported instead of silently ignored.
#[tokio::test]
async fn tray_and_autostart_settings_persist_and_report_platform_limits() {
    use ohm_runtime::SettingsStore;

    let temp = tempfile::tempdir().unwrap();
    let paths = ohm_core::ConfigPaths::from_root(temp.path());
    let store = SettingsStore::new(&paths);

    let mut settings = ohm_runtime::Settings::default();
    assert!(
        settings.minimize_to_tray,
        "a tray monitor hides on minimise"
    );
    assert!(settings.close_to_tray);
    settings.minimize_to_tray = false;
    settings.close_to_tray = false;
    settings.start_with_windows = true;
    store.save(&settings).unwrap();

    let reloaded = store.load().unwrap();
    assert!(!reloaded.minimize_to_tray);
    assert!(!reloaded.close_to_tray);
    assert!(reloaded.start_with_windows);

    // The platform limit ("autostart is Windows only, here is what to do
    // instead") belongs to the desktop layer and is asserted there, against the
    // real code rather than a stand-in:
    //   apps/desktop/src-tauri/src/autostart.rs
    //   tests::autostart_is_reported_as_windows_only
}

/// Dry run: every write is audited as simulated and no hardware moves.
#[tokio::test]
async fn dry_run_never_touches_the_hardware() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: heavy_load(),
        ..MockConfig::deterministic()
    })
    .await;
    session
        .runtime
        .update_settings(|settings| settings.dry_run = true)
        .unwrap();
    session.engine.save_rule(session.gpu_rule()).unwrap();

    let duties_before = session.mock.status().fan_duties.clone();
    for _ in 0..10 {
        session.step(1_000).await;
    }
    assert_eq!(
        session.mock.status().fan_duties,
        duties_before,
        "dry run must not move a fan"
    );

    let audit = session.runtime.audit().tail(50).unwrap();
    assert!(
        audit
            .iter()
            .any(|entry| entry["report"]["status"] == "simulated")
    );
    session.shutdown().await;
}

/// Settings are durable, and a malformed file cannot stop the app from starting.
#[tokio::test]
async fn settings_survive_a_restart_and_survive_corruption() {
    let session = Session::simulated().await;
    session
        .runtime
        .update_settings(|settings| {
            settings.polling_interval_ms = 750;
            settings.minimize_to_tray = false;
        })
        .unwrap();
    let paths = session.paths.clone();
    session.shutdown().await;

    let reloaded = ohm_integration_tests::load_settings(&paths);
    assert_eq!(reloaded.polling_interval_ms, 750);
    assert!(!reloaded.minimize_to_tray);

    // Corrupt it: the store must quarantine the file and fall back to defaults.
    std::fs::write(paths.settings_file(), "{ not json at all").unwrap();
    let recovered = ohm_integration_tests::load_settings(&paths);
    assert_eq!(
        recovered.polling_interval_ms,
        ohm_runtime::Settings::default().polling_interval_ms
    );
    assert!(
        paths
            .settings_file()
            .with_extension("json.corrupt")
            .exists()
    );
}

/// Polling keeps the state store warm and never reports a failure for a healthy
/// simulated machine.
#[tokio::test]
async fn polling_is_consistent_across_devices() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: idle_load(),
        ..MockConfig::deterministic()
    })
    .await;

    for _ in 0..5 {
        session.step_without_rules(500).await;
    }
    let stats = session.runtime.stats();
    assert!(stats.poll_cycles >= 5);
    assert_eq!(stats.last_poll_errors, 0);

    // Every device has a state whose timestamp is the same poll instant.
    let states: Vec<DeviceState> = session
        .runtime
        .devices()
        .into_iter()
        .filter_map(|view| view.state)
        .collect();
    assert!(!states.is_empty());
    let first = states[0].timestamp_ms;
    assert!(
        states
            .iter()
            .all(|state| (state.timestamp_ms - first).abs() < 1_000)
    );

    session.shutdown().await;
    assert!(!session.runtime.is_running());
}

/// A load profile that never loads: the machine stays quiet and no write storm
/// happens.
#[tokio::test]
async fn a_quiet_machine_does_not_get_written_to_constantly() {
    let session = Session::simulated_with(MockConfig {
        gpu_load: idle_load(),
        cpu_load: idle_load(),
        ..MockConfig::deterministic()
    })
    .await;
    session
        .engine
        .save_rule(session.gpu_rule().with_deadband(2.0))
        .unwrap();

    for _ in 0..200 {
        session.step(500).await;
    }
    let outcome = session.engine.outcome("gpu-cooling").unwrap();
    assert!(
        outcome.skipped > outcome.writes,
        "an idle machine must mostly skip writes ({:?})",
        outcome
    );
    assert_eq!(outcome.fallbacks, 0);
    let _ = LoadProfile::Constant { load: 0.02 };
    session.shutdown().await;
}
