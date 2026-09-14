/**
 * Wire types for the OpenHardwareOS backend.
 *
 * These mirror the frozen Tauri backend contract exactly: every JSON field is
 * `snake_case`, every temperature is Celsius, every timestamp is Unix
 * milliseconds. Nothing in this file may be changed without a matching backend
 * change.
 */

export type Unit =
  | 'celsius'
  | 'fahrenheit'
  | 'rpm'
  | 'percent'
  | 'pwm'
  | 'watt'
  | 'milliwatt'
  | 'volt'
  | 'ampere'
  | 'hertz'
  | 'megahertz'
  | 'byte'
  | 'second'
  | 'millisecond'
  | 'count'
  | 'boolean'
  | 'text'
  | 'none';

export type CapabilityKind = 'sensor' | 'actuator' | 'event' | 'info';

export type DeviceType =
  | 'cpu'
  | 'gpu'
  | 'motherboard'
  | 'memory'
  | 'storage'
  | 'fan'
  | 'pump'
  | 'temperature_sensor'
  | 'power_supply'
  | 'keyboard'
  | 'mouse'
  | 'rgb'
  | 'display'
  | 'hub'
  | 'unknown';

export type Transport =
  | 'system'
  | 'nvidia'
  | 'amd'
  | 'usb_hid'
  | 'usb_cdc'
  | 'serial'
  | 'web'
  | 'bridge'
  | 'mock'
  | 'unknown';

export type DeviceStatus = 'online' | 'offline' | 'disabled' | 'degraded';

export type UnavailableReason =
  | 'unsupported'
  | 'not_present'
  | 'permission_denied'
  | 'hardware_limitation'
  | 'vendor_limitation'
  | 'driver_missing'
  | 'timeout'
  | 'disabled'
  | 'read_error'
  | 'unknown';

export type AdapterState = 'not_probed' | 'available' | 'degraded' | 'unavailable' | 'error';

export type RuleStatus =
  | 'disabled'
  | 'idle'
  | 'applied'
  | 'held'
  /** The rule's `when` condition is false: it is standing down on purpose. */
  | 'gated' | 'unconfirmed'
  | 'fallback'
  | 'released'
  | 'error';

export type Theme = 'system' | 'dark' | 'light';

export type LogLevel = 'error' | 'warn' | 'info' | 'debug' | 'trace';

/** A JSON value a capability can read or write. */
export type CapabilityValue = number | boolean | string;

// ---------------------------------------------------------------------------
// Devices and capabilities
// ---------------------------------------------------------------------------

export interface Capability {
  id: string;
  name: string;
  kind: CapabilityKind;
  unit: Unit;
  readable: boolean;
  writable: boolean;
  min?: number;
  max?: number;
  step?: number;
  values?: string[];
  description?: string;
  poll_interval_ms?: number;
  safety_critical?: boolean;
}

export interface Device {
  id: string;
  name: string;
  /** Serialized by the backend as `type` (a `device_type` key would break the wire). */
  type: DeviceType;
  vendor: string;
  model?: string;
  transport: Transport;
  adapter: string;
  capabilities: Capability[];
  metadata?: Record<string, string>;
  tags?: string[];
}

/**
 * `Reading` is flattened on the wire: `{capability, status: "ok", value}` or
 * `{capability, status: "unavailable", reason, detail?}`.
 */
export type Reading =
  | { capability: string; status: 'ok'; value: CapabilityValue }
  | {
      capability: string;
      status: 'unavailable';
      reason: UnavailableReason;
      detail?: string;
    };

export interface DeviceState {
  device: string;
  timestamp_ms: number;
  readings: Reading[];
  online: boolean;
  message?: string;
}

export interface DeviceView {
  device: Device;
  enabled: boolean;
  status: DeviceStatus;
  state?: DeviceState;
  adapter: string;
  first_seen_ms: number;
  last_seen_ms: number;
}

export interface AdapterCapabilities {
  can_write: boolean;
  can_control_cooling: boolean;
  write_requires_admin: boolean;
  poll_interval_ms?: number;
  discovery_interval_ms?: number;
}

export interface AdapterInfo {
  id: string;
  name: string;
  namespace: string;
  description: string;
  version: string;
  homepage?: string;
  requires_admin: boolean;
  supports_hotplug: boolean;
  capabilities: AdapterCapabilities;
}

export interface AdapterStatus {
  adapter: string;
  state: AdapterState;
  reason?: UnavailableReason;
  detail?: string;
  checked_at_ms: number;
  device_count: number;
}

export interface AdapterView {
  info: AdapterInfo;
  status: AdapterStatus;
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

export interface SafetyPolicy {
  enabled: boolean;
  require_min_duty: boolean;
  min_duty_percent: number;
  pump_min_duty_percent: number;
  emergency_temp_c: number;
  emergency_duty_percent: number;
  fail_safe_duty_percent: number;
  emergency_override_enabled: boolean;
  relinquish_on_exit: boolean;
  max_write_delta_percent: number;
  sensor_stale_after_s: number;
}

export interface Settings {
  polling_interval_ms: number;
  discovery_interval_ms: number;
  history_points: number;
  log_level: LogLevel;
  start_with_windows: boolean;
  minimize_to_tray: boolean;
  start_minimized: boolean;
  close_to_tray: boolean;
  theme: Theme;
  experimental_features: boolean;
  developer_mode: boolean;
  automation_enabled: boolean;
  dry_run: boolean;
  enable_mock_protocol_device: boolean;
  disabled_adapters: string[];
  disabled_devices: string[];
  safety: SafetyPolicy;
  adapter_settings: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// Runtime snapshot
// ---------------------------------------------------------------------------

export interface RuntimeStats {
  poll_cycles: number;
  discovery_cycles: number;
  writes_attempted: number;
  writes_applied: number;
  writes_rejected: number;
  safety_interventions: number;
  events_published: number;
  last_poll_ms: number;
  last_poll_duration_ms: number;
  last_poll_errors: number;
}

export interface RuntimeSnapshot {
  generated_at_ms: number;
  started_at_ms: number;
  devices: DeviceView[];
  adapters: AdapterView[];
  settings: Settings;
  stats: RuntimeStats;
  has_controllable_hardware: boolean;
}

export interface Sample {
  at_ms: number;
  value: number;
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

export type WriteOrigin =
  | { kind: 'manual' }
  | { kind: 'automation'; rule_id: string }
  | { kind: 'safety'; reason: string }
  | { kind: 'startup' }
  | { kind: 'shutdown' }
  | { kind: 'api' };

/**
 * `unconfirmed` means the device accepted the write but the value could not be
 * read back and confirmed, so the backend deliberately reports no `applied`
 * value: filling in the requested one would state something it cannot vouch for.
 */
export type WriteStatus = 'applied' | 'simulated' | 'unconfirmed' | 'rejected';

export interface WriteReport {
  at_ms: number;
  device_id: string;
  device_name: string;
  device_type: DeviceType;
  capability: string;
  capability_name: string;
  requested: CapabilityValue;
  applied?: CapabilityValue;
  status: WriteStatus;
  clamped: boolean;
  error_code?: string;
  detail?: string;
  origin: WriteOrigin;
  simulated: boolean;
}

// ---------------------------------------------------------------------------
// Automation rules
// ---------------------------------------------------------------------------

export interface SensorRef {
  device: string;
  capability: string;
}

/** Wire shape of one curve control point: `[input, output]`. */
export type ControlPointTuple = [number, number];

export type Source =
  | { device: string; capability: string }
  | { aggregate: 'max' | 'min' | 'avg'; sensors: SensorRef[] };

export type FallbackAction = 'hold' | 'safe_default' | 'release' | { fixed: { percent: number } };

export interface Fallback {
  on_sensor_missing: FallbackAction;
  on_write_failure: FallbackAction;
  sensor_timeout_s: number;
}

export interface RuleTarget {
  device: string;
  capability: string;
}

/** How two numbers are compared inside a `Condition`. */
export type Comparator = 'gt' | 'gte' | 'lt' | 'lte' | 'eq' | 'ne';

/**
 * What a rule does while its condition is false. `safe_default` drives the
 * runtime's fail-safe duty; `fixed` drives an explicit percentage. There is
 * deliberately no "hold whatever the fan had".
 */
export type OtherwiseAction = 'safe_default' | { fixed: { percent: number } };

/** An optional numeric gate on a rule: `when {source, op, value, otherwise}`. */
export interface Condition {
  source: SensorRef;
  op: Comparator;
  /** Threshold, in the unit of `source.capability`. */
  value: number;
  otherwise: OtherwiseAction;
}

export interface Rule {
  id: string;
  name: string;
  enabled: boolean;
  description?: string;
  source: Source;
  /**
   * Optional numeric gate. Absent means "always active" — it is omitted from the
   * wire payload entirely, never sent as `null`.
   */
  when?: Condition;
  target: RuleTarget;
  curve: ControlPointTuple[];
  hysteresis: number;
  deadband: number;
  update_interval_ms: number;
  min_output?: number;
  max_output?: number;
  fallback: Fallback;
  priority: number;
  created_at_ms?: number;
  updated_at_ms?: number;
}

export interface RuleOutcome {
  rule_id: string;
  name: string;
  enabled: boolean;
  status: RuleStatus;
  source: string;
  target: string;
  input?: number;
  output?: number;
  applied_output?: number;
  message: string;
  evaluations: number;
  writes: number;
  skipped: number;
  fallbacks: number;
  at_ms: number;
}

export interface RuleCheck {
  errors: string[];
  warnings: string[];
}

/**
 * Two enabled rules fighting over one output, as the engine reported them.
 *
 * Losers of a clash found at load time are disabled **in memory only**; their
 * files are untouched. `resolution` is a human sentence shown verbatim.
 */
export interface RuleConflict {
  owner_id: string;
  owner_name: string;
  blocked_id: string;
  blocked_name: string;
  /** The contested output, e.g. `fan.mock.0/fan.speed_percent`. */
  target: string;
  resolution: string;
}

/**
 * One substitution the runtime made while loading a legacy rule file.
 *
 * The file on disk is left byte-for-byte untouched: the action it asks for
 * cannot be honoured, so the runtime swaps in the fail-safe duty **in memory**
 * and records what it actually runs instead. The user decides what to do about
 * the file; the machine stays protected in the meantime.
 */
export interface RuleFileNote {
  /** Absolute path of the rule file. */
  path: string;
  rule_id: string;
  /** e.g. `fallback.on_sensor_missing`. */
  field: string;
  /** e.g. `release`. */
  original: string;
  /** e.g. `safe_default (fail-safe duty 70 %)`. */
  effective: string;
  /** One human sentence describing the substitution. */
  message: string;
  /** What the user should do about it. */
  hint: string;
}

/**
 * Where a channel handover got to.
 *
 * `pending` and `failed` are still **owed**: the channel is running on whatever
 * the abandoned rule last said, and the fail-safe duty is not confirmed in
 * force. `confirmed` and `superseded` are settled.
 */
export type HandoverState =
  | 'pending'
  | 'needs_verification'
  | 'awaiting_owner'
  | 'confirmed'
  | 'failed'
  | 'superseded';

/**
 * One channel a rule left behind, as the runtime handed (or tried to hand) it to
 * the fail-safe duty. The record survives the rule that abandoned the channel, so
 * an unfinished handover stays visible even when that rule no longer exists.
 */
export interface HandoverReport {
  device: string;
  capability: string;
  /** The rule that abandoned the channel; it may no longer exist. */
  from_rule: string;
  /** One sentence: why the channel was abandoned. */
  reason: string;
  state: HandoverState;
  attempts: number;
  /** The cause, never overwritten by later symptoms. */
  first_error?: string;
  /** The most recent symptom. */
  last_error?: string;
  queued_at_ms: number;
  last_attempt_ms: number;
  /** Set once the fail-safe duty was confirmed in force. */
  confirmed_value?: number;
  /** The rule that owns the channel now, when that is why it was superseded. */
  superseded_by?: string;
  /**
   * The enabled rule that targets the channel without driving it, when that is what
   * the handover is waiting for. A declared target is not a takeover, so the channel
   * is still owed the fail-safe duty while this is set.
   */
  claimant?: string;
  /** How many ticks have been spent waiting for that claimant to take control. */
  claimed_ticks: number;
}

export interface CapabilityRef {
  device_id: string;
  device_name: string;
  device_type: DeviceType;
  adapter: string;
  capability: Capability;
  current?: CapabilityValue;
}

export interface CapabilityIndex {
  sources: CapabilityRef[];
  targets: CapabilityRef[];
}

// ---------------------------------------------------------------------------
// Audit / automation stats
// ---------------------------------------------------------------------------

export interface AuditEntry {
  at_ms?: number;
  at?: string;
  kind: 'write' | 'lifecycle';
  action?: string;
  detail?: string;
  report?: WriteReport;
}

export interface AutomationStats {
  rules: number;
  enabled_rules: number;
  ticks: number;
  evaluations: number;
  writes: number;
  skipped: number;
  fallbacks: number;
  failures: number;
  last_tick_ms: number;
  /**
   * Set when the unresolved control responsibility could not be written to disk, so a
   * front-end can say so instead of implying it was saved.
   */
  persistence_error?: string;
}

// ---------------------------------------------------------------------------
// Mock hardware
// ---------------------------------------------------------------------------

export interface MockFaults {
  fail_all_writes: boolean;
  fail_writes_on: [string, string][];
  unavailable_readings: [string, string, UnavailableReason][];
  unplug_devices: string[];
}

export interface MockStatus {
  sim_ms: number;
  clock: 'wall' | 'manual';
  ambient_c: number;
  gpu_temp_c: number;
  cpu_temp_c: number;
  ssd_temp_c: number;
  gpu_load: number;
  cpu_load: number;
  gpu_fan_duty: number;
  gpu_fan_rpm: number;
  fan_duties: number[];
  fan_rpms: number[];
  pump_duties: number[];
  noise: number;
  faults: MockFaults;
}

export type MockProfile = 'idle' | 'gaming' | 'wave';

export interface AppInfo {
  version: string;
  platform: string;
  config_root: string;
  protocol_version: string;
  os: string;
  arch: string;
  elevated: boolean;
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

export type RuntimeEvent =
  | { type: 'runtime_started'; at_ms: number; device_count: number }
  | { type: 'runtime_stopped'; at_ms: number }
  | { type: 'adapter_status_changed'; status: AdapterView }
  | { type: 'device_added'; device: Device }
  | { type: 'device_removed'; device_id: string; name: string; reason: UnavailableReason }
  | { type: 'device_status_changed'; device_id: string; status: DeviceStatus }
  | { type: 'device_enabled_changed'; device_id: string; enabled: boolean }
  | { type: 'state_changed'; state: DeviceState }
  | {
      type: 'reading_changed';
      device_id: string;
      capability: string;
      value?: CapabilityValue;
      previous?: CapabilityValue;
    }
  | { type: 'write_performed'; report: WriteReport }
  | { type: 'write_rejected'; device_id: string; capability: string; error_code: string; detail: string }
  | { type: 'safety_triggered'; kind: string; detail: string; at_ms: number }
  | { type: 'automation'; rule_id?: string; kind: string; detail: string; at_ms: number }
  | { type: 'log'; level: string; message: string; at_ms: number };

/** One entry of the in-app activity feed, derived from a `runtime-event`. */
export interface ActivityEntry {
  key: string;
  at_ms: number;
  level: LogLevel;
  category: 'runtime' | 'device' | 'adapter' | 'write' | 'safety' | 'automation' | 'log';
  message: string;
  detail?: string;
  device_id?: string;
}

/** A reading paired with the capability definition that describes it. */
export interface ResolvedReading {
  capability: Capability;
  reading?: Reading;
}

/** An action failure surfaced to the user (never a raw stack trace). */
export interface NoticeError {
  code: string;
  message: string;
  hint?: string;
  unsupported?: boolean;
}
