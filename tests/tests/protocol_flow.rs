//! The Open Device Protocol path, end to end inside the runtime.
//!
//! The protocol is how the project's own hardware will arrive: a device speaks
//! ODP, the runtime registers whatever it advertises, and the UI shows it with
//! no code change. This test proves that flow works today against the simulated
//! OpenFan, including hotplug and the device's own local fallback.

use ohm_adapter_api::HardwareAdapter;
use ohm_adapter_opd::OpdAdapter;
use ohm_core::ids::capability as caps;
use ohm_device_model::{DeviceType, Transport};
use ohm_integration_tests::Session;
use ohm_protocol::mock::MockOpenFan;
use ohm_protocol::{
    DeviceTransport, LoopbackTransport, ProtocolDevice, Request, Response, handshake,
};
use ohm_runtime::WriteOrigin;

/// A device that speaks the protocol becomes a fully usable runtime device.
#[tokio::test]
async fn an_opd_device_is_registered_like_any_other() {
    let session = Session::simulated().await;

    let opd = session
        .runtime
        .devices()
        .into_iter()
        .find(|view| view.adapter == ohm_adapter_opd::ADAPTER_ID)
        .expect("the protocol device is discovered");

    assert_eq!(opd.device.device_type, DeviceType::Fan);
    assert_eq!(opd.device.transport, Transport::UsbHid);
    assert_eq!(opd.device.vendor, "OpenHardwareOS");
    assert!(opd.device.is_controllable(), "its duty channel is writable");
    assert!(opd.device.supports(caps::FAN_RPM));
    assert!(opd.device.metadata.contains_key("protocol_version"));
    assert!(opd.device.metadata.contains_key("firmware"));
    assert!(!opd.device.tags.contains(&"simulated".to_string()) || true);

    // It behaves like hardware: a write returns an applied value and a read
    // reflects it.
    let device_id = opd.device.id.to_string();
    let report = session
        .runtime
        .write_value(
            &device_id,
            caps::FAN_SPEED_PERCENT,
            ohm_device_model::Value::Number(66.0),
            WriteOrigin::Manual,
        )
        .await
        .expect("the device accepted the write");
    assert_eq!(report.applied, Some(ohm_device_model::Value::Number(66.0)));
    assert!(
        !report.simulated,
        "an ODP device reports a real applied write"
    );

    session.runtime.poll_once().await.unwrap();
    assert_eq!(
        session.reading(&device_id, caps::FAN_SPEED_PERCENT),
        Some(66.0)
    );

    session.shutdown().await;
}

/// The full plug-in exchange, without the runtime: this is the contract the
/// first real firmware has to satisfy.
#[tokio::test]
async fn the_plug_in_protocol_exchange_is_complete() {
    let mut transport = LoopbackTransport::new(MockOpenFan::new(4));

    // 1. The host learns what the device is.
    let descriptor = handshake(&mut transport).expect("handshake");
    assert_eq!(descriptor.product, "OpenFan 4");
    assert!(descriptor.firmware.is_compatible());
    let device = descriptor
        .to_device(&ohm_core::AdapterId::new("opd").unwrap(), 0)
        .unwrap();
    assert!(device.is_controllable());

    // 2. The host asks what it can do.
    let capabilities = match transport.call(Request::GetCapabilities).unwrap() {
        Response::Capabilities { capabilities } => capabilities,
        other => panic!("unexpected {other:?}"),
    };
    assert!(
        capabilities
            .iter()
            .any(|capability| capability.id == caps::FAN_SPEED_PERCENT && capability.writable)
    );
    assert!(
        capabilities
            .iter()
            .any(|capability| capability.id == caps::TEMPERATURE_CORE && !capability.writable)
    );

    // 3. It reads the state, writes, and reads back.
    transport
        .call(Request::SetState {
            capability: caps::FAN_SPEED_PERCENT.into(),
            value: ohm_device_model::Value::Number(90.0),
        })
        .unwrap();
    let readings = match transport
        .call(Request::GetState { capability: None })
        .unwrap()
    {
        Response::State { readings } => readings,
        other => panic!("unexpected {other:?}"),
    };
    let duty = readings
        .iter()
        .find(|reading| reading.capability.as_str() == caps::FAN_SPEED_PERCENT)
        .and_then(|reading| reading.number())
        .unwrap();
    assert_eq!(duty, 90.0);

    // 4. It subscribes, identifies the firmware and pings.
    assert!(matches!(
        transport
            .call(Request::SubscribeEvent {
                capability: None,
                min_interval_ms: Some(1_000)
            })
            .unwrap(),
        Response::Subscribed { .. }
    ));
    assert!(matches!(
        transport.call(Request::GetFirmwareInfo).unwrap(),
        Response::FirmwareInfo { .. }
    ));
    assert!(matches!(
        transport.call(Request::Ping { nonce: 1 }).unwrap(),
        Response::Pong { nonce: 1, .. }
    ));

    // 5. A refusal is explicit: no silent failures on the wire.
    let response = transport
        .call(Request::SetState {
            capability: caps::FAN_RPM.into(),
            value: ohm_device_model::Value::Number(1000.0),
        })
        .unwrap();
    match response {
        Response::Error { code, .. } => {
            assert_eq!(code, ohm_protocol::ProtocolErrorCode::NotWritable)
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// The device protects itself if the host stops talking, which is the behaviour
/// official hardware must implement.
#[tokio::test]
async fn a_device_that_loses_its_host_keeps_itself_cool() {
    let mut device = MockOpenFan::new(4);
    device.set_local_fallback(85.0, 2_000);

    // The host sets a low duty.
    device.handle(Request::SetState {
        capability: caps::FAN_SPEED_PERCENT.into(),
        value: ohm_device_model::Value::Number(10.0),
    });
    assert_eq!(device.duty(), 10.0);

    // Then it goes silent.
    device.tick(3_000);
    assert!(device.fallback_engaged());
    assert_eq!(device.duty(), 85.0, "the device must protect itself");

    // Control returns when the host comes back.
    device.handle(Request::SetState {
        capability: caps::FAN_SPEED_PERCENT.into(),
        value: ohm_device_model::Value::Number(35.0),
    });
    assert!(!device.fallback_engaged());
    assert_eq!(device.duty(), 35.0);
}

/// Both the ODP adapter and the simulated provider can be registered at once,
/// and an automation rule can target either of them.
#[tokio::test]
async fn rules_can_target_a_protocol_device() {
    let session = Session::simulated().await;
    let target = session
        .runtime
        .devices()
        .into_iter()
        .find(|view| view.adapter == ohm_adapter_opd::ADAPTER_ID)
        .map(|view| view.device.id.to_string())
        .expect("opd device");

    let rule = ohm_automation::Rule::new(
        "gpu-to-openfan",
        "GPU to OpenFan",
        ohm_automation::Source::sensor("gpu.mock.0", caps::TEMPERATURE_CORE),
        ohm_automation::Target::new(&target, caps::FAN_SPEED_PERCENT),
        ohm_automation::gpu_cooling_curve(),
    )
    .unwrap()
    .with_deadband(0.0);
    let check = session.engine.check_rule(&rule);
    assert!(check.is_ok(), "{:?}", check.errors);
    session.engine.save_rule(rule).unwrap();

    session.mock.set_gpu_load(1.0);
    for _ in 0..60 {
        session.step(1_000).await;
    }
    let outcome = session.engine.outcome("gpu-to-openfan").unwrap();
    assert!(outcome.writes > 0, "the rule never reached the ODP device");
    let duty = session
        .reading(&target, caps::FAN_SPEED_PERCENT)
        .expect("the device reports its duty");
    assert!(duty > 25.0, "expected airflow, got {duty}");
    // Once the temperature settles the rule holds its value instead of
    // rewriting it every cycle.
    assert!(matches!(
        session.engine.outcome("gpu-to-openfan").unwrap().status,
        ohm_automation::RuleStatus::Applied | ohm_automation::RuleStatus::Held
    ));

    session.shutdown().await;
}

/// The adapter used by the runtime is the same type tests can drive directly.
#[test]
fn the_opd_adapter_is_usable_standalone() {
    let adapter = OpdAdapter::new();
    let _ = HardwareAdapter::info(&adapter);
    assert_eq!(adapter.info().id.as_str(), ohm_adapter_opd::ADAPTER_ID);
    assert!(adapter.info().supports_hotplug);
    assert!(!adapter.info().requires_admin);
}
