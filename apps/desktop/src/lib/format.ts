/**
 * Formatting helpers: values with units, timestamps and human labels for the
 * backend's snake_case enums. Every missing value is rendered as an em dash by
 * `formatValue`, never as a fake zero.
 */

import type {
  AdapterState,
  CapabilityKind,
  CapabilityValue,
  DeviceStatus,
  DeviceType,
  LogLevel,
  RuleStatus,
  Transport,
  UnavailableReason,
  Unit,
} from '../types';

/** Shown whenever a value could not be read. */
export const EMPTY = '—';

// ---------------------------------------------------------------------------
// Units and values
// ---------------------------------------------------------------------------

const UNIT_SUFFIX: Record<Unit, { suffix: string; digits: number }> = {
  celsius: { suffix: '°C', digits: 1 },
  fahrenheit: { suffix: '°F', digits: 1 },
  rpm: { suffix: 'RPM', digits: 0 },
  percent: { suffix: '%', digits: 0 },
  pwm: { suffix: 'PWM', digits: 0 },
  watt: { suffix: 'W', digits: 1 },
  milliwatt: { suffix: 'mW', digits: 0 },
  volt: { suffix: 'V', digits: 3 },
  ampere: { suffix: 'A', digits: 2 },
  hertz: { suffix: 'Hz', digits: 0 },
  megahertz: { suffix: 'MHz', digits: 0 },
  byte: { suffix: 'B', digits: 0 },
  second: { suffix: 's', digits: 1 },
  millisecond: { suffix: 'ms', digits: 0 },
  count: { suffix: '', digits: 0 },
  boolean: { suffix: '', digits: 0 },
  text: { suffix: '', digits: 0 },
  none: { suffix: '', digits: 0 },
};

export function unitLabel(unit: Unit): string {
  return UNIT_SUFFIX[unit]?.suffix ?? '';
}

export function isNumericUnit(unit: Unit): boolean {
  return unit !== 'boolean' && unit !== 'text' && unit !== 'none';
}

/** Format a number for display, using SI-ish abbreviations for byte counts. */
export function formatNumber(value: number, digits = 1): string {
  if (!Number.isFinite(value)) return EMPTY;
  if (Number.isInteger(value)) return value.toLocaleString('en-US');
  return value.toLocaleString('en-US', { minimumFractionDigits: digits, maximumFractionDigits: digits });
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes)) return EMPTY;
  const units = ['B', 'kB', 'MB', 'GB', 'TB', 'PB'];
  let value = bytes;
  let index = 0;
  while (Math.abs(value) >= 1000 && index < units.length - 1) {
    value /= 1000;
    index += 1;
  }
  return `${value.toFixed(index === 0 ? 0 : 1)} ${units[index] ?? 'B'}`;
}

/**
 * `formatValue(68.4, 'celsius')` -> `68.4 °C`, `formatValue(1120, 'rpm')` -> `1,120 RPM`.
 * `undefined`/`null` render as an em dash so the UI can never imply a zero reading.
 */
export function formatValue(value: CapabilityValue | undefined | null, unit: Unit): string {
  if (value === undefined || value === null) return EMPTY;
  if (typeof value === 'boolean') return value ? 'On' : 'Off';
  if (typeof value === 'string') return value;
  if (unit === 'byte') return formatBytes(value);
  const spec = UNIT_SUFFIX[unit] ?? { suffix: '', digits: 1 };
  const number = formatNumber(value, spec.digits);
  return spec.suffix ? `${number} ${spec.suffix}` : number;
}

/** The numeric part only, for use next to a separately styled unit. */
export function formatValueParts(
  value: CapabilityValue | undefined | null,
  unit: Unit,
): { number: string; unit: string } {
  if (value === undefined || value === null) return { number: EMPTY, unit: '' };
  if (typeof value === 'boolean') return { number: value ? 'On' : 'Off', unit: '' };
  if (typeof value === 'string') return { number: value, unit: '' };
  if (unit === 'byte') {
    const [number = EMPTY, suffix = ''] = formatBytes(value).split(' ');
    return { number, unit: suffix };
  }
  const spec = UNIT_SUFFIX[unit] ?? { suffix: '', digits: 1 };
  return { number: formatNumber(value, spec.digits), unit: spec.suffix };
}

export function formatRange(
  min: number | undefined,
  max: number | undefined,
  unit: Unit,
): string {
  if (min === undefined && max === undefined) return EMPTY;
  const suffix = unitLabel(unit);
  const lo = min === undefined ? EMPTY : formatNumber(min, 0);
  const hi = max === undefined ? EMPTY : formatNumber(max, 0);
  return suffix ? `${lo} – ${hi} ${suffix}` : `${lo} – ${hi}`;
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

export function formatClock(ms: number | undefined): string {
  if (ms === undefined || !Number.isFinite(ms) || ms === 0) return EMPTY;
  const date = new Date(ms);
  return date.toLocaleTimeString('en-GB', { hour12: false });
}

export function formatDateTime(ms: number | undefined): string {
  if (ms === undefined || !Number.isFinite(ms) || ms === 0) return EMPTY;
  return new Date(ms).toLocaleString('en-GB', { hour12: false });
}

/** "just now", "12 s ago", "4 min ago", "2 h ago". */
export function formatRelative(ms: number | undefined, now = Date.now()): string {
  if (ms === undefined || !Number.isFinite(ms) || ms === 0) return EMPTY;
  const seconds = Math.round((now - ms) / 1000);
  if (seconds < 0) return 'in the future';
  if (seconds < 2) return 'just now';
  if (seconds < 60) return `${seconds} s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  return `${Math.round(hours / 24)} d ago`;
}

export function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min ${s % 60} s`;
  const h = Math.floor(m / 60);
  return `${h} h ${m % 60} min`;
}

// ---------------------------------------------------------------------------
// Human labels for snake_case enums
// ---------------------------------------------------------------------------

export function unavailableReasonLabel(reason: UnavailableReason): string {
  switch (reason) {
    case 'unsupported':
      return 'Not supported by this hardware';
    case 'not_present':
      return 'Not present in this machine';
    case 'permission_denied':
      return 'Access denied — run as Administrator';
    case 'hardware_limitation':
      return 'The hardware cannot expose this';
    case 'vendor_limitation':
      return 'The vendor tool owns this sensor';
    case 'driver_missing':
      return 'A required driver is missing';
    case 'timeout':
      return 'The sensor did not answer in time';
    case 'disabled':
      return 'Reading is disabled';
    case 'read_error':
      return 'The sensor returned an error';
    case 'unknown':
      return 'Unavailable for an unknown reason';
    default:
      return 'Unavailable';
  }
}

/** Short badge text for the same reasons (badges pair colour with this text). */
export function unavailableReasonBadge(reason: UnavailableReason): string {
  switch (reason) {
    case 'unsupported':
      return 'unsupported';
    case 'not_present':
      return 'not present';
    case 'permission_denied':
      return 'permission denied';
    case 'hardware_limitation':
      return 'hardware limit';
    case 'vendor_limitation':
      return 'vendor limited';
    case 'driver_missing':
      return 'driver missing';
    case 'timeout':
      return 'timeout';
    case 'disabled':
      return 'disabled';
    case 'read_error':
      return 'read error';
    default:
      return 'unavailable';
  }
}

export function deviceTypeLabel(type: DeviceType): string {
  switch (type) {
    case 'cpu':
      return 'CPU';
    case 'gpu':
      return 'GPU';
    case 'motherboard':
      return 'Motherboard';
    case 'memory':
      return 'Memory';
    case 'storage':
      return 'Storage';
    case 'fan':
      return 'Fan';
    case 'pump':
      return 'Pump';
    case 'temperature_sensor':
      return 'Temperature sensor';
    case 'power_supply':
      return 'Power supply';
    case 'keyboard':
      return 'Keyboard';
    case 'mouse':
      return 'Mouse';
    case 'rgb':
      return 'Lighting';
    case 'display':
      return 'Display';
    case 'hub':
      return 'Hub';
    default:
      return 'Unknown device';
  }
}

export function deviceStatusLabel(status: DeviceStatus): string {
  switch (status) {
    case 'online':
      return 'Online';
    case 'offline':
      return 'Offline';
    case 'disabled':
      return 'Disabled';
    case 'degraded':
      return 'Degraded';
    default:
      return 'Unknown';
  }
}

export function transportLabel(transport: Transport): string {
  switch (transport) {
    case 'system':
      return 'System';
    case 'nvidia':
      return 'NVIDIA NVML';
    case 'amd':
      return 'AMD ADL';
    case 'usb_hid':
      return 'USB HID';
    case 'usb_cdc':
      return 'USB CDC';
    case 'serial':
      return 'Serial';
    case 'web':
      return 'Web API';
    case 'bridge':
      return 'Bridge';
    case 'mock':
      return 'Mock (simulated)';
    default:
      return 'Unknown';
  }
}

export function adapterStateLabel(state: AdapterState): string {
  switch (state) {
    case 'not_probed':
      return 'Not probed';
    case 'available':
      return 'Available';
    case 'degraded':
      return 'Degraded';
    case 'unavailable':
      return 'Unavailable';
    case 'error':
      return 'Error';
    default:
      return 'Unknown';
  }
}

export function ruleStatusLabel(status: RuleStatus): string {
  switch (status) {
    case 'disabled':
      return 'Disabled';
    case 'idle':
      return 'Idle';
    case 'applied':
      return 'Applied';
    case 'held':
      return 'Held';
    case 'gated':
      // Standing down is deliberate and safe: the label must not read as a fault.
      return 'Standing down';
    case 'fallback':
      return 'Fallback';
    case 'released':
      return 'Released';
    case 'error':
      return 'Error';
    default:
      return 'Unknown';
  }
}

/** The full sentence behind a status badge, used as its `title`. */
export function ruleStatusHint(status: RuleStatus): string | undefined {
  switch (status) {
    case 'gated':
      return 'The rule’s “when” condition is not met, so it has stood down and the configured otherwise duty is in force. This is normal, not a fault.';
    case 'fallback':
      return 'The source sensor is missing or stale, so the rule is following its fallback policy.';
    case 'applied':
      return 'The rule steered its output from the curve on the last evaluation.';
    case 'held':
      return 'The computed output did not change enough to be worth a write.';
    case 'released':
      return 'Control of the output was handed back to the hardware.';
    case 'error':
      return 'The last write failed. The runtime’s fail-safe duty is in force.';
    default:
      return undefined;
  }
}

export function capabilityKindLabel(kind: CapabilityKind): string {
  switch (kind) {
    case 'sensor':
      return 'Sensor';
    case 'actuator':
      return 'Actuator';
    case 'event':
      return 'Event';
    case 'info':
      return 'Info';
    default:
      return 'Unknown';
  }
}

export function logLevelLabel(level: LogLevel | string): string {
  switch (level) {
    case 'error':
      return 'Error';
    case 'warn':
      return 'Warning';
    case 'info':
      return 'Info';
    case 'debug':
      return 'Debug';
    case 'trace':
      return 'Trace';
    default:
      return String(level);
  }
}

export const LOG_LEVEL_ORDER: Record<string, number> = {
  error: 0,
  warn: 1,
  info: 2,
  debug: 3,
  trace: 4,
};

/** Title-case a snake_case identifier as a last-resort label. */
export function humanise(identifier: string): string {
  return identifier
    .split(/[._-]/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

export function percent(ratio: number): string {
  return `${Math.round(ratio * 100)} %`;
}
