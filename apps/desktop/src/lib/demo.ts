/**
 * DEMO-ONLY backend.
 *
 * Used exclusively when `window.__TAURI_INTERNALS__` is undefined, i.e. when the
 * UI runs in a plain browser via `npm run dev`. It returns a small, plausible
 * static dataset so every screen can be rendered without the Rust backend. It is
 * never reachable inside the Tauri app, and it never claims to be real hardware:
 * the UI labels it as simulated wherever it is shown.
 */

import type {
  AdapterView,
  AppInfo,
  AuditEntry,
  AutomationStats,
  Capability,
  CapabilityValue,
  Condition,
  Device,
  DeviceView,
  MockProfile,
  MockStatus,
  Reading,
  Rule,
  RuleConflict,
  RuleOutcome,
  RuntimeEvent,
  RuntimeSnapshot,
  Sample,
  Settings,
  WriteReport,
} from '../types';
import { evaluateCurve, GPU_TEMPLATE_CURVE } from './curve';

export interface DemoBackend {
  ready: boolean;
  call: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  /** Subscribe to demo `snapshot` / `runtime-event` pushes. Returns an unlisten fn. */
  subscribe: (
    onSnapshot: (snapshot: RuntimeSnapshot) => void,
    onEvent: (event: RuntimeEvent) => void,
  ) => () => void;
}

const DEMO_NOTE = 'Demo data — no real hardware is being read.';

/** Single factory for the whole demo backend. */
export function createDemoBackend(): DemoBackend {
  const startedAt = Date.now();
  let simMs = 1_800_000; // 30 simulated minutes so the charts are never empty
  let ambient = 24;
  let gpuLoad = 34;
  let cpuLoad = 22;
  let gpuTemp = 46;
  let cpuTemp = 44;
  let ssdTemp = 38;
  let gpuDuty = 30;
  let profile: MockProfile | null = null;

  let fanDuties = [42, 38];
  let pumpDuties = [60];
  let faults: MockStatus['faults'] = {
    fail_all_writes: false,
    fail_writes_on: [],
    unavailable_readings: [],
    unplug_devices: [],
  };

  let settings: Settings = {
    polling_interval_ms: 1000,
    discovery_interval_ms: 10_000,
    history_points: 3600,
    log_level: 'info',
    start_with_windows: false,
    minimize_to_tray: true,
    start_minimized: false,
    close_to_tray: true,
    theme: 'system',
    experimental_features: true,
    developer_mode: false,
    automation_enabled: true,
    dry_run: false,
    enable_mock_protocol_device: true,
    disabled_adapters: [],
    disabled_devices: [],
    safety: {
      enabled: true,
      require_min_duty: true,
      min_duty_percent: 20,
      pump_min_duty_percent: 60,
      emergency_temp_c: 95,
      emergency_duty_percent: 100,
      fail_safe_duty_percent: 80,
      emergency_override_enabled: true,
      relinquish_on_exit: true,
      max_write_delta_percent: 25,
      sensor_stale_after_s: 10,
    },
    adapter_settings: {},
  };

  const appInfo: AppInfo = {
    version: '0.1.0-demo',
    platform: 'browser',
    config_root: '/demo/openhardwareos',
    protocol_version: '1',
    os: 'browser',
    arch: 'wasm',
    elevated: false,
  };

  // --- capabilities ---------------------------------------------------------

  const cap = (
    id: string,
    name: string,
    kind: Capability['kind'],
    unit: Capability['unit'],
    extra: Partial<Capability> = {},
  ): Capability => ({ id, name, kind, unit, readable: true, writable: false, ...extra });

  const percent = (id: string, name: string): Capability =>
    cap(id, name, 'actuator', 'percent', { writable: true, min: 0, max: 100, step: 1 });

  const cpu: Device = {
    id: 'cpu.package',
    name: 'AMD Ryzen 7 7800X3D',
    type: 'cpu',
    vendor: 'AMD',
    model: 'Ryzen 7 7800X3D',
    transport: 'system',
    adapter: 'system',
    capabilities: [
      cap('temperature.package', 'Package temperature', 'sensor', 'celsius'),
      cap('temperature.cores_max', 'Hottest core', 'sensor', 'celsius'),
      cap('load.total', 'Total load', 'sensor', 'percent'),
      cap('power.package', 'Package power', 'sensor', 'watt'),
      cap('clock.cores_max', 'Max core clock', 'sensor', 'megahertz'),
    ],
    tags: ['primary'],
  };

  const gpu: Device = {
    id: 'gpu.nvidia0',
    name: 'NVIDIA GeForce RTX 4070',
    type: 'gpu',
    vendor: 'NVIDIA',
    model: 'GeForce RTX 4070',
    transport: 'nvidia',
    adapter: 'nvidia',
    capabilities: [
      cap('temperature.core', 'Core temperature', 'sensor', 'celsius'),
      cap('temperature.hotspot', 'Hotspot temperature', 'sensor', 'celsius'),
      cap('load.gpu', 'GPU load', 'sensor', 'percent'),
      cap('power.draw', 'Board power', 'sensor', 'watt'),
      cap('clock.core', 'Core clock', 'sensor', 'megahertz'),
      cap('fan.speed', 'Fan speed', 'sensor', 'rpm'),
      percent('fan.duty', 'Fan duty'),
      cap('info.name', 'Board name', 'info', 'text'),
    ],
    tags: ['controllable'],
  };

  const storage: Device = {
    id: 'storage.nvme0',
    name: 'Samsung SSD 990 PRO 2TB',
    type: 'storage',
    vendor: 'Samsung',
    model: '990 PRO 2TB',
    transport: 'system',
    adapter: 'system',
    capabilities: [
      cap('temperature.composite', 'Composite temperature', 'sensor', 'celsius'),
      cap('temperature.sensor1', 'NAND temperature', 'sensor', 'celsius'),
      cap('usage.percent', 'Life used', 'sensor', 'percent'),
      cap('data.read_bytes', 'Data read', 'sensor', 'byte'),
    ],
  };

  const makeFan = (index: number, role: string): Device => ({
    id: `fan.chassis${index}`,
    name: `Chassis fan ${index} (${role})`,
    type: 'fan',
    vendor: 'Generic',
    transport: 'mock',
    adapter: 'mock',
    capabilities: [
      cap('fan.speed', 'Fan speed', 'sensor', 'rpm'),
      cap('fan.duty', 'Fan duty', 'actuator', 'percent', {
        writable: true,
        min: 0,
        max: 100,
        step: 1,
        safety_critical: true,
      }),
    ],
  });

  const pump: Device = {
    id: 'pump.aio0',
    name: 'AIO pump',
    type: 'pump',
    vendor: 'Generic',
    transport: 'mock',
    adapter: 'mock',
    capabilities: [cap('pump.rpm', 'Pump speed', 'sensor', 'rpm'), percent('pump.duty', 'Pump duty')],
  };

  const devices: Device[] = [cpu, gpu, storage, makeFan(1, 'front intake'), makeFan(2, 'rear exhaust'), pump];

  const adapterViews: AdapterView[] = [
    {
      info: {
        id: 'system',
        name: 'System sensors',
        namespace: 'ohm.system',
        description: 'Motherboard, CPU and storage sensors exposed by the operating system.',
        version: '0.1.0',
        requires_admin: false,
        supports_hotplug: false,
        capabilities: { can_write: false, can_control_cooling: false, write_requires_admin: false, poll_interval_ms: 1000 },
      },
      status: { adapter: 'system', state: 'available', checked_at_ms: Date.now(), device_count: 2 },
    },
    {
      info: {
        id: 'nvidia',
        name: 'NVIDIA NVML',
        namespace: 'ohm.nvidia',
        description: 'GPU temperature, power and fan control through the NVIDIA driver.',
        version: '0.1.0',
        requires_admin: true,
        supports_hotplug: true,
        capabilities: { can_write: true, can_control_cooling: true, write_requires_admin: true, poll_interval_ms: 1000 },
      },
      status: { adapter: 'nvidia', state: 'degraded', reason: 'permission_denied', detail: 'Running without Administrator rights (demo).', checked_at_ms: Date.now(), device_count: 1 },
    },
    {
      info: {
        id: 'mock',
        name: 'Mock protocol device',
        namespace: 'ohm.mock',
        description: 'Simulated fans and pump used to demonstrate the control loop without hardware.',
        version: '0.1.0',
        requires_admin: false,
        supports_hotplug: true,
        capabilities: { can_write: true, can_control_cooling: true, write_requires_admin: false, poll_interval_ms: 1000 },
      },
      status: { adapter: 'mock', state: 'available', checked_at_ms: Date.now(), device_count: 3 },
    },
    {
      info: {
        id: 'libre_hardware_monitor',
        name: 'LibreHardwareMonitor',
        namespace: 'ohm.lhm',
        description: 'Bridge to a locally running LibreHardwareMonitor instance.',
        version: '0.1.0',
        homepage: 'https://github.com/LibreHardwareMonitor/LibreHardwareMonitor',
        requires_admin: false,
        supports_hotplug: true,
        capabilities: { can_write: false, can_control_cooling: false, write_requires_admin: false, poll_interval_ms: 2000 },
      },
      status: {
        adapter: 'libre_hardware_monitor',
        state: 'unavailable',
        reason: 'driver_missing',
        detail: 'LibreHardwareMonitor web server not reachable on 127.0.0.1:8085 (demo).',
        checked_at_ms: Date.now(),
        device_count: 0,
      },
    },
  ];

  // --- live values ----------------------------------------------------------

  let enabled: Record<string, boolean> = {};

  let rules: Rule[] = [
    {
      id: 'rule-demo-gpu',
      name: 'GPU temperature → GPU fan',
      enabled: true,
      description: 'Ramps the GPU fan from 20% at 40 °C to 100% at 85 °C.',
      source: { device: gpu.id, capability: 'temperature.core' },
      target: { device: gpu.id, capability: 'fan.duty' },
      curve: GPU_TEMPLATE_CURVE.map((point) => [point[0], point[1]] as [number, number]),
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
      created_at_ms: startedAt,
      updated_at_ms: startedAt,
    },
  ];

  const history = new Map<string, Sample[]>();
  const audit: AuditEntry[] = [
    { at_ms: startedAt, kind: 'lifecycle', action: 'started', detail: DEMO_NOTE },
  ];

  const readValue = (deviceId: string, capability: string): CapabilityValue | undefined => {
    const fault = faults.unavailable_readings.find(([d, c]) => d === deviceId && c === capability);
    if (fault) return undefined;
    if (faults.unplug_devices.includes(deviceId)) return undefined;
    switch (`${deviceId}/${capability}`) {
      case `${cpu.id}/temperature.package`:
        return round(cpuTemp, 1);
      case `${cpu.id}/temperature.cores_max`:
        return round(cpuTemp + 4, 1);
      case `${cpu.id}/load.total`:
        return round(cpuLoad, 0);
      case `${cpu.id}/power.package`:
        return round(35 + cpuLoad * 1.15, 1);
      case `${cpu.id}/clock.cores_max`:
        return 4200 + Math.round(cpuLoad * 8);
      case `${gpu.id}/temperature.core`:
        return round(gpuTemp, 1);
      case `${gpu.id}/temperature.hotspot`:
        return round(gpuTemp + 11, 1);
      case `${gpu.id}/load.gpu`:
        return round(gpuLoad, 0);
      case `${gpu.id}/power.draw`:
        return round(28 + gpuLoad * 1.7, 1);
      case `${gpu.id}/clock.core`:
        return 900 + Math.round(gpuLoad * 18);
      case `${gpu.id}/fan.speed`:
        return Math.round((gpuDuty / 100) * 2200);
      case `${gpu.id}/fan.duty`:
        return round(gpuDuty, 0);
      case `${gpu.id}/info.name`:
        return 'GeForce RTX 4070';
      case `${storage.id}/temperature.composite`:
        return round(ssdTemp, 1);
      case `${storage.id}/temperature.sensor1`:
        return round(ssdTemp + 6, 1);
      case `${storage.id}/usage.percent`:
        return 4;
      case `${storage.id}/data.read_bytes`:
        return 4_812_000_000_000;
      case `fan.chassis1/fan.speed`:
        return Math.round((fanDuties[0] ?? 0) * 14);
      case `fan.chassis1/fan.duty`:
        return round(fanDuties[0] ?? 0, 0);
      case `fan.chassis2/fan.speed`:
        return Math.round((fanDuties[1] ?? 0) * 13);
      case `fan.chassis2/fan.duty`:
        return round(fanDuties[1] ?? 0, 0);
      case `${pump.id}/pump.rpm`:
        return Math.round((pumpDuties[0] ?? 0) * 45);
      case `${pump.id}/pump.duty`:
        return round(pumpDuties[0] ?? 0, 0);
      default:
        return undefined;
    }
  };

  const readingsFor = (device: Device): Reading[] =>
    device.capabilities.map((c) => {
      if (!c.readable) {
        return { capability: c.id, status: 'unavailable', reason: 'unsupported' } as Reading;
      }
      const faulted = faults.unavailable_readings.find(([d, cc]) => d === device.id && cc === c.id);
      if (faulted) {
        return {
          capability: c.id,
          status: 'unavailable',
          reason: faulted[2],
          detail: 'Injected by the mock fault switch.',
        } as Reading;
      }
      const value = readValue(device.id, c.id);
      if (value === undefined) {
        return {
          capability: c.id,
          status: 'unavailable',
          reason: device.id === storage.id ? 'vendor_limitation' : 'unsupported',
          detail: 'Not exposed by this adapter.',
        } as Reading;
      }
      if (c.unit === 'megahertz') return { capability: c.id, status: 'ok', value };
      return { capability: c.id, status: 'ok', value };
    });

  const deviceStatus = (device: Device): DeviceView['status'] => {
    if (enabled[device.id] === false) return 'disabled';
    if (faults.unplug_devices.includes(device.id)) return 'offline';
    return 'online';
  };

  const nowMs = (): number => Date.now();

  /**
   * The two simulated sensors a demo `when` gate can read. Anything else is
   * reported as "cannot tell", and the rule keeps steering.
   */
  const mockReading = (deviceId: string, capability: string): number | undefined => {
    if (deviceId !== gpu.id) return undefined;
    switch (capability) {
      case 'load.gpu':
        return gpuLoad;
      case 'temperature.core':
        return gpuTemp;
      default:
        return undefined;
    }
  };

  const gateSatisfied = (condition: Condition): boolean => {
    const reading = mockReading(condition.source.device, condition.source.capability);
    if (reading === undefined) return true;
    switch (condition.op) {
      case 'gt':
        return reading > condition.value;
      case 'gte':
        return reading >= condition.value;
      case 'lt':
        return reading < condition.value;
      case 'lte':
        return reading <= condition.value;
      case 'eq':
        return Math.abs(reading - condition.value) <= 1e-6;
      case 'ne':
        return Math.abs(reading - condition.value) > 1e-6;
      default:
        return true;
    }
  };

  const deviceView = (device: Device): DeviceView => ({
    device,
    enabled: enabled[device.id] !== false,
    status: deviceStatus(device),
    state: {
      device: device.id,
      timestamp_ms: nowMs(),
      readings: readingsFor(device),
      online: deviceStatus(device) === 'online',
      message: DEMO_NOTE,
    },
    adapter: device.adapter,
    first_seen_ms: startedAt,
    last_seen_ms: nowMs(),
  });

  const views = (): DeviceView[] => devices.map(deviceView);

  let outcomes: RuleOutcome[] = [];

  // --- simulation -----------------------------------------------------------

  const pushSample = (deviceId: string, capability: string, value: number, atMs: number): void => {
    const key = `${deviceId}/${capability}`;
    const list = history.get(key) ?? [];
    list.push({ at_ms: atMs, value });
    if (list.length > 300) list.splice(0, list.length - 300);
    history.set(key, list);
  };

  const step = (): void => {
    simMs += settings.polling_interval_ms;
    const at = nowMs();

    // Ambient wobble keeps the charts alive and plausible.
    if (profile === 'wave') {
      gpuLoad = round(50 + 45 * Math.sin(simMs / 12_000), 0);
      cpuLoad = round(40 + 35 * Math.sin(simMs / 17_000 + 1), 0);
    }

    const gpuEquilibrium = ambient + 22 + gpuLoad * 0.45 - gpuDuty * 0.28;
    const cpuEquilibrium = ambient + 20 + cpuLoad * 0.4 - (fanDuties[0] ?? 0) * 0.25;
    gpuTemp += (gpuEquilibrium - gpuTemp) * 0.12;
    cpuTemp += (cpuEquilibrium - cpuTemp) * 0.1;
    ssdTemp += (ambient + 12 + gpuLoad * 0.06 - ssdTemp) * 0.05;
    pumpDuties = [round(Math.max(settings.safety.pump_min_duty_percent, 55 + gpuLoad * 0.2), 0)];

    // Run the enabled rules — this is the closed loop the panel demonstrates.
    const gpuRule = rules.find((r) => r.id === 'rule-demo-gpu');
    if (gpuRule && gpuRule.enabled && settings.automation_enabled && gpuRule.when && !gateSatisfied(gpuRule.when)) {
      // Standing down: the condition is false, so the configured otherwise duty
      // is driven instead of the curve output.
      const wanted =
        typeof gpuRule.when.otherwise === 'object'
          ? gpuRule.when.otherwise.fixed.percent
          : settings.safety.fail_safe_duty_percent;
      gpuDuty = round(
        gpuDuty +
          clamp(
            clamp(wanted, gpuRule.min_output ?? 0, gpuRule.max_output ?? 100) - gpuDuty,
            -settings.safety.max_write_delta_percent,
            settings.safety.max_write_delta_percent,
          ),
        0,
      );
      outcomes = [
        {
          rule_id: gpuRule.id,
          name: gpuRule.name,
          enabled: true,
          status: 'gated',
          source: `${gpu.name} · ${gpuTemp.toFixed(1)} °C`,
          target: `${gpu.name} · fan duty`,
          input: round(gpuTemp, 1),
          output: wanted,
          applied_output: gpuDuty,
          message: `Condition not met (${gpuRule.when.source.capability} ${gpuRule.when.op} ${gpuRule.when.value}); standing down on the otherwise duty (${gpuDuty} %).`,
          evaluations: 1,
          writes: settings.dry_run ? 0 : 1,
          skipped: 0,
          fallbacks: 0,
          at_ms: at,
        },
      ];
    } else if (gpuRule && gpuRule.enabled && settings.automation_enabled) {
      const target = round(
        clamp(evaluateCurve(gpuRule.curve, gpuTemp), gpuRule.min_output ?? 0, gpuRule.max_output ?? 100),
        0,
      );
      gpuDuty = round(gpuDuty + clamp(target - gpuDuty, -settings.safety.max_write_delta_percent, settings.safety.max_write_delta_percent), 0);
      outcomes = [
        {
          rule_id: gpuRule.id,
          name: gpuRule.name,
          enabled: true,
          status: 'applied',
          source: `${gpu.name} · ${gpuTemp.toFixed(1)} °C`,
          target: `${gpu.name} · fan duty`,
          input: round(gpuTemp, 1),
          output: target,
          applied_output: gpuDuty,
          message: settings.dry_run ? 'Dry run: write simulated.' : `Set fan duty to ${gpuDuty} %.`,
          evaluations: 1,
          writes: settings.dry_run ? 0 : 1,
          skipped: 0,
          fallbacks: 0,
          at_ms: at,
        },
      ];
    } else if (gpuRule) {
      outcomes = [
        {
          rule_id: gpuRule.id,
          name: gpuRule.name,
          enabled: gpuRule.enabled,
          status: gpuRule.enabled ? 'idle' : 'disabled',
          source: `${gpu.name} · ${gpuTemp.toFixed(1)} °C`,
          target: `${gpu.name} · fan duty`,
          input: round(gpuTemp, 1),
          message: gpuRule.enabled ? 'Automation is switched off.' : 'Rule disabled.',
          evaluations: 0,
          writes: 0,
          skipped: 1,
          fallbacks: 0,
          at_ms: at,
        },
      ];
    }

    // Chassis fans follow CPU temperature with a simple demo ramp.
    const cpuDuty = round(clamp(25 + (cpuTemp - 35) * 1.4, 25, 90), 0);
    fanDuties = [cpuDuty, round(clamp(cpuDuty - 4, 20, 90), 0)];

    for (const device of devices) {
      for (const c of device.capabilities) {
        const v = readValue(device.id, c.id);
        if (typeof v === 'number') pushSample(device.id, c.id, v, at);
      }
    }
  };

  const snapshot = (): RuntimeSnapshot => {
    const s = settings;
    return {
      generated_at_ms: nowMs(),
      started_at_ms: startedAt,
      devices: views(),
      adapters: adapterViews,
      settings: s,
      stats: {
        poll_cycles: Math.floor(simMs / Math.max(s.polling_interval_ms, 100)),
        discovery_cycles: Math.floor(simMs / Math.max(s.discovery_interval_ms, 1000)),
        writes_attempted: 42,
        writes_applied: 42,
        writes_rejected: 0,
        safety_interventions: 0,
        events_published: 128,
        last_poll_ms: nowMs() - 120,
        last_poll_duration_ms: 7,
        last_poll_errors: 0,
      },
      has_controllable_hardware: true,
    };
  };

  // --- mock hardware commands ----------------------------------------------

  const mockStatus = (): MockStatus => ({
    sim_ms: simMs,
    clock: 'wall',
    ambient_c: round(ambient, 1),
    gpu_temp_c: round(gpuTemp, 1),
    cpu_temp_c: round(cpuTemp, 1),
    ssd_temp_c: round(ssdTemp, 1),
    gpu_load: round(gpuLoad, 0),
    cpu_load: round(cpuLoad, 0),
    gpu_fan_duty: round(gpuDuty, 0),
    gpu_fan_rpm: Math.round((gpuDuty / 100) * 2200),
    fan_duties: fanDuties.slice(),
    fan_rpms: fanDuties.map((d) => Math.round(d * 14)),
    pump_duties: pumpDuties.slice(),
    noise: round(clamp(gpuDuty * 0.4 + (fanDuties[0] ?? 0) * 0.3, 0, 100), 0),
    faults: {
      fail_all_writes: faults.fail_all_writes,
      fail_writes_on: faults.fail_writes_on.map((p) => [p[0], p[1]] as [string, string]),
      unavailable_readings: faults.unavailable_readings.map(
        (r) => [r[0], r[1], r[2]] as [string, string, (typeof faults.unavailable_readings)[number][2]],
      ),
      unplug_devices: faults.unplug_devices.slice(),
    },
  });

  const writeReport = (
    deviceId: string,
    capabilityId: string,
    requested: CapabilityValue,
  ): WriteReport => {
    const device = devices.find((d) => d.id === deviceId) ?? gpu;
    const capability = device.capabilities.find((c) => c.id === capabilityId);
    const base = {
      at_ms: nowMs(),
      device_id: device.id,
      device_name: device.name,
      device_type: device.type,
      capability: capabilityId,
      capability_name: capability?.name ?? capabilityId,
      requested,
      origin: { kind: 'manual' } as const,
      simulated: false,
    };
    const fail =
      faults.fail_all_writes ||
      faults.fail_writes_on.some(([d, c]) => d === deviceId && c === capabilityId);

    if (fail) {
      const report: WriteReport = {
        ...base,
        status: 'rejected',
        clamped: false,
        error_code: 'write_failed',
        detail: 'The mock fault switch is rejecting every write to this capability.',
      };
      audit.unshift({ at_ms: report.at_ms, kind: 'write', action: 'rejected', report });
      return report;
    }

    let applied = requested;
    let clamped = false;
    if (typeof requested === 'number' && capability && (capability.min !== undefined || capability.max !== undefined)) {
      const next = clamp(requested, capability.min ?? requested, capability.max ?? requested);
      if (next !== requested) {
        clamped = true;
        applied = next;
      }
    }
    if (typeof applied === 'number') {
      if (deviceId === gpu.id && capabilityId === 'fan.duty') gpuDuty = applied;
      if (deviceId === 'fan.chassis1' && capabilityId === 'fan.duty') fanDuties[0] = applied;
      if (deviceId === 'fan.chassis2' && capabilityId === 'fan.duty') fanDuties[1] = applied;
      if (deviceId === pump.id) pumpDuties[0] = applied;
    }
    const report: WriteReport = { ...base, applied, status: 'applied', clamped };
    audit.unshift({ at_ms: report.at_ms, kind: 'write', action: 'applied', report });
    return report;
  };

  const ruleOutcomes = (): RuleOutcome[] => {
    if (outcomes.length > 0) return outcomes;
    return rules.map((r) => ({
      rule_id: r.id,
      name: r.name,
      enabled: r.enabled,
      status: r.enabled ? 'idle' : 'disabled',
      source: describeSource(r.source, devices),
      target: describeTarget(r.target, devices),
      message: r.enabled ? 'Waiting for the next evaluation.' : 'Rule disabled.',
      evaluations: 0,
      writes: 0,
      skipped: 0,
      fallbacks: 0,
      at_ms: nowMs(),
    }));
  };

  const checkRule = (rule: Rule): { errors: string[]; warnings: string[] } => {
    const errors: string[] = [];
    const warnings: string[] = [];
    if (!rule.name.trim()) errors.push('Give the rule a name.');
    if (rule.curve.length < 2) errors.push('A curve needs at least two control points.');
    if (!rule.target.device || !rule.target.capability) errors.push('Choose a target capability.');
    if ('device' in rule.source && (!rule.source.device || !rule.source.capability)) {
      errors.push('Choose a source sensor.');
    }
    if ('sensors' in rule.source && rule.source.sensors.length === 0) {
      errors.push('An aggregate needs at least one sensor.');
    }
    for (let i = 1; i < rule.curve.length; i += 1) {
      const prev = rule.curve[i - 1];
      const cur = rule.curve[i];
      if (prev && cur && cur[0] <= prev[0]) {
        errors.push('Curve inputs must strictly increase.');
        break;
      }
    }
    if (rule.hysteresis < 0) errors.push('Hysteresis cannot be negative.');
    if (rule.deadband < 0) errors.push('Deadband cannot be negative.');
    if (rule.update_interval_ms < 250) warnings.push('Update intervals below 250 ms can wear out fans.');
    if (rule.max_output !== undefined && rule.min_output !== undefined && rule.max_output <= rule.min_output) {
      errors.push('Maximum output must be greater than minimum output.');
    }
    if (rule.curve.some((p) => p[1] > 100)) warnings.push('Outputs above 100 % will be clamped.');

    // The `when` gate, mirroring `check_rule` in the Rust engine.
    if (rule.when) {
      if (rule.when.op === 'eq' || rule.when.op === 'ne') {
        warnings.push(
          `when uses \`${rule.when.op}\` on ${rule.when.source.device}/${rule.when.source.capability}, an analog reading; \`gte\`/\`lte\` say what you mean and will not flip with a 0.1 wobble`,
        );
      }
      if (typeof rule.when.otherwise === 'object' && (rule.when.otherwise.fixed.percent < 0 || rule.when.otherwise.fixed.percent > 100)) {
        errors.push('when.otherwise.fixed percentage must be between 0 and 100');
      }
    }

    // One enabled rule per output.
    if (rule.enabled) {
      const owner = conflictingRule(rule);
      if (owner) {
        errors.push(
          `${owner.name} already drives ${targetId(rule)}; disable it or point this rule at another output`,
        );
      }
    }
    return { errors, warnings };
  };

  const targetId = (rule: Rule): string => `${rule.target.device}/${rule.target.capability}`;

  /** The enabled rule that already owns this rule's target, if any. */
  const conflictingRule = (rule: Rule): Rule | undefined =>
    rules.find(
      (existing) => existing.enabled && existing.id !== rule.id && targetId(existing) === targetId(rule),
    );

  /** The refusal the Rust engine raises, word for word. */
  const conflictError = (candidate: Rule, owner: Rule): Error =>
    new Error(
      `\`${candidate.name}\` cannot be enabled: \`${owner.name}\` already drives ${targetId(candidate)}. Two enabled rules may not share an output — disable or retarget one of them.`,
    );

  /** Rule files that fight over one output, in the shape `rule_conflicts` returns. */
  const ruleConflicts = (): RuleConflict[] => {
    const owners: Rule[] = [];
    const conflicts: RuleConflict[] = [];
    for (const rule of rules.filter((r) => r.enabled)) {
      const owner = owners.find((candidate) => targetId(candidate) === targetId(rule));
      if (owner) {
        conflicts.push({
          owner_id: owner.id,
          owner_name: owner.name,
          blocked_id: rule.id,
          blocked_name: rule.name,
          target: targetId(rule),
          resolution: 'resolved at load time: this rule is disabled',
        });
      } else {
        owners.push(rule);
      }
    }
    return conflicts;
  };

  const capabilityIndex = (): { sources: ReturnType<typeof refs>; targets: ReturnType<typeof refs> } => ({
    sources: refs('sensor'),
    targets: refs('actuator'),
  });

  function refs(kind: 'sensor' | 'actuator') {
    return views().flatMap((v) =>
      v.device.capabilities
        .filter((c) => (kind === 'sensor' ? c.readable && c.kind === 'sensor' : c.writable))
        .map((c) => {
          const reading = v.state?.readings.find((r) => r.capability === c.id);
          return {
            device_id: v.device.id,
            device_name: v.device.name,
            device_type: v.device.type,
            adapter: v.device.adapter,
            capability: c,
            current: reading && reading.status === 'ok' ? reading.value : undefined,
          };
        }),
    );
  }

  // --- command dispatch -----------------------------------------------------

  const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    app_info: () => appInfo,
    get_snapshot: () => snapshot(),
    scan_devices: () => snapshot(),
    get_devices: () => views(),
    get_device: (a) => views().find((v) => v.device.id === a.device) ?? gpuViewFallback(views()),
    get_history: (a) => {
      const list = history.get(`${String(a.device)}/${String(a.capability)}`) ?? [];
      const limit = typeof a.limit === 'number' ? a.limit : 120;
      return list.slice(Math.max(0, list.length - limit));
    },
    write_capability: (a) =>
      writeReport(String(a.device), String(a.capability), a.value as CapabilityValue),
    set_device_enabled: (a) => {
      enabled = { ...enabled, [String(a.device)]: Boolean(a.enabled) };
      return null;
    },
    set_adapter_enabled: (a) => {
      const id = String(a.adapter);
      const on = Boolean(a.enabled);
      settings = {
        ...settings,
        disabled_adapters: on
          ? settings.disabled_adapters.filter((x) => x !== id)
          : [...new Set([...settings.disabled_adapters, id])],
      };
      return settings;
    },
    get_settings: () => settings,
    update_settings: (a) => {
      settings = a.settings as Settings;
      return settings;
    },
    capability_index: () => capabilityIndex(),
    get_adapters: () => adapterViews,
    list_rules: () => rules,
    rule_outcomes: () => ruleOutcomes(),
    rule_conflicts: () => ruleConflicts(),
    check_rule: (a) => checkRule(a.rule as Rule),
    save_rule: (a) => {
      const rule = a.rule as Rule;
      if (rule.enabled) {
        const owner = conflictingRule(rule);
        if (owner) throw conflictError(rule, owner);
      }
      const mapped: Rule = {
        ...rule,
        updated_at_ms: nowMs(),
        created_at_ms: rule.created_at_ms ?? nowMs(),
      };
      rules = rules.some((r) => r.id === mapped.id)
        ? rules.map((r) => (r.id === mapped.id ? mapped : r))
        : [...rules, mapped];
      return mapped;
    },
    delete_rule: (a) => {
      const before = rules.length;
      rules = rules.filter((r) => r.id !== String(a.id));
      return rules.length !== before;
    },
    set_rule_enabled: (a) => {
      const id = String(a.id);
      const wanted = Boolean(a.enabled);
      const existing = rules.find((r) => r.id === id);
      if (wanted && existing) {
        const owner = conflictingRule(existing);
        if (owner) throw conflictError(existing, owner);
      }
      let updated: Rule | undefined;
      rules = rules.map((r) => {
        if (r.id !== id) return r;
        updated = { ...r, enabled: wanted };
        return updated;
      });
      return updated ?? rules[0] ?? null;
    },
    suggest_rules: () => [
      {
        ...(rules[0] as Rule),
        id: 'rule-suggested-cpu',
        name: 'CPU temperature → chassis fan 1',
        enabled: false,
        source: { device: cpu.id, capability: 'temperature.package' },
        target: { device: 'fan.chassis1', capability: 'fan.duty' },
        curve: [
          [45, 25],
          [60, 40],
          [75, 65],
          [85, 100],
        ] as Rule['curve'],
        description: 'Suggested because a chassis fan and a CPU temperature sensor are both available.',
      },
    ],
    list_audit_log: (a) => audit.slice(0, typeof a.limit === 'number' ? a.limit : 200),
    automation_stats: (): AutomationStats => ({
      rules: rules.length,
      enabled_rules: rules.filter((r) => r.enabled).length,
      ticks: Math.floor(simMs / 2000),
      evaluations: Math.floor(simMs / 2000),
      writes: 42,
      skipped: 3,
      fallbacks: 0,
      failures: 0,
      last_tick_ms: nowMs() - 400,
    }),
    mock_status: () => mockStatus(),
    mock_set_load: (a) => {
      profile = null;
      gpuLoad = clamp(Number(a.gpu), 0, 100);
      cpuLoad = clamp(Number(a.cpu), 0, 100);
      return mockStatus();
    },
    mock_apply_profile: (a) => {
      profile = a.profile as MockProfile;
      if (profile === 'idle') {
        gpuLoad = 5;
        cpuLoad = 8;
        ambient = 23;
      } else if (profile === 'gaming') {
        gpuLoad = 92;
        cpuLoad = 62;
        ambient = 26;
      }
      return mockStatus();
    },
    mock_set_ambient: (a) => {
      ambient = Number(a.celsius);
      return mockStatus();
    },
    mock_force_gpu_temperature: (a) => {
      gpuTemp = Number(a.celsius);
      return mockStatus();
    },
    mock_set_faults: (a) => {
      faults = {
        fail_all_writes: Boolean(a.fail_writes),
        fail_writes_on: faults.fail_writes_on,
        unavailable_readings:
          Boolean(a.disconnect_gpu_temperature) && !faults.unavailable_readings.some((r) => r[0] === gpu.id)
            ? [...faults.unavailable_readings, [gpu.id, 'temperature.core', 'timeout']]
            : faults.unavailable_readings.filter((r) => r[0] !== gpu.id),
        unplug_devices: Boolean(a.unplug_fan)
          ? [...new Set([...faults.unplug_devices, 'fan.chassis2'])]
          : faults.unplug_devices.filter((d) => d !== 'fan.chassis2'),
      };
      return mockStatus();
    },
    open_config_dir: () => null,
    open_log_dir: () => null,
  };

  const listeners: { onSnapshot: ((s: RuntimeSnapshot) => void)[]; onEvent: ((e: RuntimeEvent) => void)[] } = {
    onSnapshot: [],
    onEvent: [],
  };

  let timer: number | undefined;

  return {
    ready: true,
    async call(command, args = {}) {
      const handler = handlers[command];
      if (!handler) {
        throw {
          code: 'unsupported',
          message: `The demo backend does not implement "${command}".`,
          hint: 'Run the app inside Tauri to use this command.',
          unsupported: true,
        };
      }
      // A touch of latency keeps loading states honest.
      await new Promise((resolve) => setTimeout(resolve, 30));
      return handler(args);
    },
    subscribe(onSnapshot, onEvent) {
      listeners.onSnapshot.push(onSnapshot);
      listeners.onEvent.push(onEvent);
      if (timer === undefined) {
        step();
        onSnapshot(snapshot());
        timer = window.setInterval(() => {
          step();
          const snap = snapshot();
          for (const l of listeners.onSnapshot) l(snap);
          if (Math.random() < 0.25) {
            const device = devices[Math.floor(Math.random() * devices.length)];
            if (device) {
              const event: RuntimeEvent = {
                type: 'automation',
                rule_id: 'rule-demo-gpu',
                kind: 'applied',
                detail: `GPU fan duty → ${gpuDuty} % (${gpuTemp.toFixed(1)} °C)`,
                at_ms: nowMs(),
              };
              for (const l of listeners.onEvent) l(event);
            }
          }
        }, Math.max(settings.polling_interval_ms, 250));
      }
      return () => {
        listeners.onSnapshot = listeners.onSnapshot.filter((l) => l !== onSnapshot);
        listeners.onEvent = listeners.onEvent.filter((l) => l !== onEvent);
        if (listeners.onSnapshot.length === 0 && timer !== undefined) {
          window.clearInterval(timer);
          timer = undefined;
        }
      };
    },
  };
}

function gpuViewFallback(all: DeviceView[]): DeviceView {
  const first = all[0];
  if (!first) throw { code: 'not_found', message: 'No devices are available in the demo backend.' };
  return first;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

function round(value: number, digits: number): number {
  const factor = 10 ** digits;
  return Math.round(value * factor) / factor;
}

function describeSource(source: Rule['source'], devices: Device[]): string {
  if ('device' in source) {
    const device = devices.find((d) => d.id === source.device);
    return `${device?.name ?? source.device} · ${source.capability}`;
  }
  return `${source.aggregate.toUpperCase()} of ${source.sensors.length} sensor(s)`;
}

function describeTarget(target: Rule['target'], devices: Device[]): string {
  const device = devices.find((d) => d.id === target.device);
  return `${device?.name ?? target.device} · ${target.capability}`;
}
