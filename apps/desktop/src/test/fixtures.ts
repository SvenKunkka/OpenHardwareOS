/**
 * Fixtures for the frontend tests.
 *
 * Everything here mirrors the wire shapes in `src/types.ts`: a mock GPU with a
 * temperature sensor, a load sensor and a duty actuator, the capability index the
 * form is built from, and small builders for rules, outcomes and rule conflicts.
 */

import type {
  AdapterView,
  Capability,
  CapabilityIndex,
  CapabilityRef,
  Device,
  DeviceView,
  Rule,
  RuleConflict,
  RuleOutcome,
  RuntimeSnapshot,
  SafetyPolicy,
  Settings,
} from '../types';

export const GPU_ID = 'gpu.mock.0';
export const FAN_ID = 'fan.mock.0';

const capability = (over: Partial<Capability> & { id: string }): Capability => ({
  name: over.id,
  kind: 'sensor',
  unit: 'celsius',
  readable: true,
  writable: false,
  ...over,
});

export const GPU_CAPABILITIES: Capability[] = [
  capability({ id: 'temperature.core', name: 'Core temperature', unit: 'celsius', min: 0, max: 110 }),
  capability({ id: 'load.gpu', name: 'GPU load', unit: 'percent', min: 0, max: 100 }),
  capability({
    id: 'fan.speed_percent',
    name: 'Fan duty',
    kind: 'actuator',
    unit: 'percent',
    readable: true,
    writable: true,
    min: 0,
    max: 100,
  }),
];

export const CPU_CAPABILITIES: Capability[] = [
  capability({ id: 'temperature.package', name: 'Package temperature', unit: 'celsius', min: 0, max: 110 }),
];

const gpu: Device = {
  id: GPU_ID,
  name: 'Mock GPU',
  type: 'gpu',
  vendor: 'OpenHardwareOS',
  transport: 'mock',
  adapter: 'mock',
  capabilities: GPU_CAPABILITIES,
};

const cpu: Device = {
  id: 'cpu.mock.0',
  name: 'Mock CPU',
  type: 'cpu',
  vendor: 'OpenHardwareOS',
  transport: 'mock',
  adapter: 'mock',
  capabilities: CPU_CAPABILITIES,
};

export const SAFETY: SafetyPolicy = {
  enabled: true,
  require_min_duty: true,
  min_duty_percent: 20,
  pump_min_duty_percent: 60,
  emergency_temp_c: 95,
  emergency_duty_percent: 100,
  fail_safe_duty_percent: 70,
  emergency_override_enabled: true,
  relinquish_on_exit: true,
  max_write_delta_percent: 25,
  sensor_stale_after_s: 10,
};

export const SETTINGS: Settings = {
  polling_interval_ms: 1000,
  discovery_interval_ms: 10_000,
  history_points: 3600,
  log_level: 'info',
  start_with_windows: false,
  minimize_to_tray: true,
  start_minimized: false,
  close_to_tray: true,
  theme: 'dark',
  experimental_features: true,
  developer_mode: false,
  automation_enabled: true,
  dry_run: false,
  enable_mock_protocol_device: true,
  disabled_adapters: [],
  disabled_devices: [],
  safety: SAFETY,
  adapter_settings: {},
};

function view(device: Device, readings: { capability: string; value: number }[]): DeviceView {
  return {
    device,
    enabled: true,
    status: 'online',
    state: {
      device: device.id,
      timestamp_ms: 1_700_000_000_000,
      online: true,
      readings: readings.map((reading) => ({
        capability: reading.capability,
        status: 'ok' as const,
        value: reading.value,
      })),
    },
    adapter: 'mock',
    first_seen_ms: 1_700_000_000_000,
    last_seen_ms: 1_700_000_000_000,
  };
}

export function snapshotFixture(settings: Partial<Settings> = {}): RuntimeSnapshot {
  return {
    generated_at_ms: 1_700_000_000_000,
    started_at_ms: 1_699_000_000_000,
    devices: [
      view(gpu, [
        { capability: 'temperature.core', value: 62 },
        { capability: 'load.gpu', value: 78 },
        { capability: 'fan.speed_percent', value: 45 },
      ]),
      view(cpu, [{ capability: 'temperature.package', value: 48 }]),
    ],
    adapters: [],
    settings: { ...SETTINGS, ...settings },
    stats: {
      poll_cycles: 10,
      discovery_cycles: 1,
      writes_attempted: 4,
      writes_applied: 4,
      writes_rejected: 0,
      safety_interventions: 0,
      events_published: 12,
      last_poll_ms: 1_700_000_000_000,
      last_poll_duration_ms: 7,
      last_poll_errors: 0,
    },
    has_controllable_hardware: true,
  };
}

const ref = (device: Device, cap: Capability, current?: number): CapabilityRef => ({
  device_id: device.id,
  device_name: device.name,
  device_type: device.type,
  adapter: device.adapter,
  capability: cap,
  current,
});

/** Sources are listed load-first on purpose: the form takes `sources[0]`. */
export function capabilityIndexFixture(): CapabilityIndex {
  const load = GPU_CAPABILITIES.find((cap) => cap.id === 'load.gpu') as Capability;
  const gpuTemp = GPU_CAPABILITIES.find((cap) => cap.id === 'temperature.core') as Capability;
  const duty = GPU_CAPABILITIES.find((cap) => cap.id === 'fan.speed_percent') as Capability;
  const cpuTemp = CPU_CAPABILITIES[0] as Capability;
  return {
    sources: [ref(gpu, load, 78), ref(gpu, gpuTemp, 62), ref(cpu, cpuTemp, 48)],
    targets: [ref(gpu, duty, 45)],
  };
}

export function ruleFixture(over: Partial<Rule> = {}): Rule {
  return {
    id: 'rule-gpu-load',
    name: 'Fan A',
    enabled: true,
    source: { device: GPU_ID, capability: 'temperature.core' },
    target: { device: GPU_ID, capability: 'fan.speed_percent' },
    curve: [
      [40, 20],
      [60, 35],
      [80, 80],
    ],
    hysteresis: 3,
    deadband: 2,
    update_interval_ms: 2000,
    min_output: 20,
    max_output: 100,
    fallback: {
      on_sensor_missing: 'safe_default',
      on_write_failure: 'hold',
      sensor_timeout_s: 10,
    },
    priority: 10,
    ...over,
  };
}

export function outcomeFixture(over: Partial<RuleOutcome> = {}): RuleOutcome {
  return {
    rule_id: 'rule-gpu-load',
    name: 'Fan A',
    enabled: true,
    status: 'applied',
    source: `${GPU_ID} · temperature.core`,
    target: `${GPU_ID} · fan.speed_percent`,
    message: 'Set fan duty to 45 %.',
    evaluations: 12,
    writes: 3,
    skipped: 9,
    fallbacks: 0,
    at_ms: 1_700_000_000_000,
    ...over,
  };
}

export function conflictFixture(over: Partial<RuleConflict> = {}): RuleConflict {
  return {
    owner_id: 'rule-gpu-fan-a',
    owner_name: 'Fan A',
    blocked_id: 'rule-gpu-fan-b',
    blocked_name: 'Fan B',
    target: 'fan.mock.0/fan.speed_percent',
    resolution: 'resolved at load time: this rule is disabled',
    ...over,
  };
}

export function adapterFixture(over: Partial<AdapterView['info']> = {}): AdapterView {
  return {
    info: {
      id: 'mock',
      name: 'Simulated hardware',
      namespace: 'mock',
      description: 'A simulated fan and pump for trying the app without hardware.',
      version: '0.1.0',
      requires_admin: false,
      supports_hotplug: true,
      capabilities: {
        can_write: true,
        can_control_cooling: true,
        write_requires_admin: false,
      },
      ...over,
    },
    status: {
      adapter: 'mock',
      state: 'available',
      checked_at_ms: 1_700_000_000_000,
      device_count: 2,
    },
  };
}
