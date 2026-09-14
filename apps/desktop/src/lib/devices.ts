/**
 * Pure helpers for reading device state. Keeping this logic out of components
 * means "no value" is always handled in one place — the UI never invents a zero.
 */

import type {
  AdapterView,
  Capability,
  CapabilityValue,
  DeviceView,
  Reading,
  ResolvedReading,
  RuntimeSnapshot,
  Sample,
  Source,
} from '../types';

export function findCapability(view: DeviceView, capabilityId: string): Capability | undefined {
  return view.device.capabilities.find((capability) => capability.id === capabilityId);
}

export function findReading(view: DeviceView, capabilityId: string): Reading | undefined {
  return view.state?.readings.find((reading) => reading.capability === capabilityId);
}

/** The numeric value of a capability, or `undefined` when missing/unavailable/non-numeric. */
export function numericReading(view: DeviceView, capabilityId: string): number | undefined {
  const reading = findReading(view, capabilityId);
  if (!reading || reading.status !== 'ok') return undefined;
  return typeof reading.value === 'number' ? reading.value : undefined;
}

export function readingValue(view: DeviceView, capabilityId: string): CapabilityValue | undefined {
  const reading = findReading(view, capabilityId);
  return reading && reading.status === 'ok' ? reading.value : undefined;
}

/** Capabilities paired with their current reading, in declaration order. */
export function resolvedReadings(view: DeviceView): ResolvedReading[] {
  return view.device.capabilities.map((capability) => ({
    capability,
    reading: findReading(view, capability.id),
  }));
}

const TEMPERATURE_PREFERENCE = [
  'temperature.package',
  'temperature.core',
  'temperature.composite',
  'temperature.cpu',
  'temperature.gpu',
  'temperature.hotspot',
];

/** The capability used as the card's headline sensor: the best temperature we have. */
export function primarySensor(view: DeviceView): Capability | undefined {
  const celsius = view.device.capabilities.filter(
    (capability) => capability.kind === 'sensor' && capability.unit === 'celsius',
  );
  for (const preferred of TEMPERATURE_PREFERENCE) {
    const match = celsius.find((capability) => capability.id === preferred);
    if (match) return match;
  }
  return (
    celsius[0] ??
    view.device.capabilities.find((capability) => capability.kind === 'sensor' && capability.readable) ??
    view.device.capabilities.find((capability) => capability.readable)
  );
}

function firstByUnitAndHint(
  view: DeviceView,
  unit: Capability['unit'],
  hints: string[],
): Capability | undefined {
  const candidates = view.device.capabilities.filter(
    (capability) => capability.kind === 'sensor' && capability.unit === unit,
  );
  for (const hint of hints) {
    const match = candidates.find((capability) => capability.id.includes(hint));
    if (match) return match;
  }
  return candidates[0];
}

export function loadCapability(view: DeviceView): Capability | undefined {
  return firstByUnitAndHint(view, 'percent', ['load', 'usage', 'util']);
}

export function powerCapability(view: DeviceView): Capability | undefined {
  return firstByUnitAndHint(view, 'watt', ['power', 'draw']);
}

export function fanCapability(view: DeviceView): Capability | undefined {
  return firstByUnitAndHint(view, 'rpm', ['fan', 'speed', 'rpm', 'pump']);
}

export function fanDutyCapability(view: DeviceView): Capability | undefined {
  return view.device.capabilities.find(
    (capability) => capability.writable && capability.kind === 'actuator' && capability.unit === 'percent',
  );
}

export function devicesOfType(snapshot: RuntimeSnapshot, type: DeviceView['device']['type']): DeviceView[] {
  return snapshot.devices.filter((view) => view.device.type === type);
}

export function deviceById(snapshot: RuntimeSnapshot, id: string): DeviceView | undefined {
  return snapshot.devices.find((view) => view.device.id === id);
}

export interface TemperatureHit {
  view: DeviceView;
  capability: Capability;
  value: number;
}

/** Every available temperature reading, for the "hottest" summary. */
export function allTemperatures(snapshot: RuntimeSnapshot): TemperatureHit[] {
  const hits: TemperatureHit[] = [];
  for (const view of snapshot.devices) {
    for (const capability of view.device.capabilities) {
      if (capability.unit !== 'celsius' || capability.kind !== 'sensor') continue;
      const value = numericReading(view, capability.id);
      if (value === undefined) continue;
      hits.push({ view, capability, value });
    }
  }
  return hits;
}

export function hottestTemperature(snapshot: RuntimeSnapshot): TemperatureHit | undefined {
  return allTemperatures(snapshot).sort((a, b) => b.value - a.value)[0];
}

export interface FanRow {
  view: DeviceView;
  rpm?: number;
  duty?: number;
  dutyCapability?: Capability;
}

/** Fans and pumps with their RPM/duty, including ones that are currently unreadable. */
export function fanRows(snapshot: RuntimeSnapshot): FanRow[] {
  return snapshot.devices
    .filter((view) => view.device.type === 'fan' || view.device.type === 'pump' || view.device.type === 'gpu')
    .map((view) => {
      const rpmCapability = fanCapability(view);
      const dutyCapability = fanDutyCapability(view);
      return {
        view,
        rpm: rpmCapability ? numericReading(view, rpmCapability.id) : undefined,
        duty: dutyCapability ? numericReading(view, dutyCapability.id) : undefined,
        dutyCapability,
      };
    })
    .filter((row) => row.rpm !== undefined || row.duty !== undefined);
}

/** Readings that are explicitly unavailable, for the "not hidden" list. */
export function unavailableReadings(view: DeviceView): { capability: Capability; reading: Reading }[] {
  const out: { capability: Capability; reading: Reading }[] = [];
  for (const capability of view.device.capabilities) {
    const reading = findReading(view, capability.id);
    if (reading && reading.status === 'unavailable') out.push({ capability, reading });
  }
  return out;
}

export function capabilityCount(view: DeviceView): { readable: number; writable: number } {
  let readable = 0;
  let writable = 0;
  for (const capability of view.device.capabilities) {
    if (capability.readable) readable += 1;
    if (capability.writable) writable += 1;
  }
  return { readable, writable };
}

/**
 * The live input value of a rule source: one sensor, or the aggregate over
 * several. Returns `undefined` when any needed reading is missing — an
 * aggregate is never computed from partial data.
 */
export function sourceValue(source: Source, snapshot: RuntimeSnapshot): number | undefined {
  if ('device' in source) {
    const view = deviceById(snapshot, source.device);
    return view ? numericReading(view, source.capability) : undefined;
  }
  const values: number[] = [];
  for (const sensor of source.sensors) {
    const view = deviceById(snapshot, sensor.device);
    const value = view ? numericReading(view, sensor.capability) : undefined;
    if (value === undefined) return undefined;
    values.push(value);
  }
  if (values.length === 0) return undefined;
  switch (source.aggregate) {
    case 'max':
      return Math.max(...values);
    case 'min':
      return Math.min(...values);
    case 'avg':
      return values.reduce((sum, value) => sum + value, 0) / values.length;
    default:
      return undefined;
  }
}

export function sortedSamples(samples: Sample[]): Sample[] {
  return [...samples].sort((a, b) => a.at_ms - b.at_ms);
}

export function sampleRange(samples: Sample[]): { min: number; max: number } | null {
  if (samples.length === 0) return null;
  let min = Number.POSITIVE_INFINITY;
  let max = Number.NEGATIVE_INFINITY;
  for (const sample of samples) {
    if (sample.value < min) min = sample.value;
    if (sample.value > max) max = sample.value;
  }
  if (!Number.isFinite(min) || !Number.isFinite(max)) return null;
  return { min, max };
}

/**
 * Why the machine has no controllable cooling, derived from adapter statuses.
 * Returns `null` when there is nothing to warn about.
 */
export function monitoringOnlyReasons(snapshot: RuntimeSnapshot): string[] {
  const reasons: string[] = [];
  const adapters: AdapterView[] = snapshot.adapters;

  const controllable = adapters.filter((adapter) => adapter.info.capabilities.can_control_cooling);
  if (controllable.length === 0) {
    reasons.push(
      'No installed adapter can control cooling. Enable an adapter that supports fan control, or enable the simulated hardware to explore the app.',
    );
  }
  for (const adapter of adapters) {
    if (adapter.status.state === 'available' || adapter.status.state === 'not_probed') continue;
    const detail = adapter.status.detail ? ` — ${adapter.status.detail}` : '';
    const reason = adapter.status.reason ? ` (${adapter.status.reason})` : '';
    reasons.push(`${adapter.info.name}: ${adapter.status.state}${reason}${detail}`);
  }
  if (snapshot.settings.enable_mock_protocol_device === false) {
    reasons.push(
      'The simulated hardware device is switched off, so the fan-control loop cannot be demonstrated.',
    );
  }
  return reasons;
}
