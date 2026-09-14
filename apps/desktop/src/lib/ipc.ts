/**
 * Every backend call goes through this module.
 *
 * - `invoke` is wrapped in one typed function per command.
 * - Rejections are normalised into a thrown `BackendError`, so the UI can always
 *   show `message` + `hint` instead of a raw stack trace.
 * - When `window.__TAURI_INTERNALS__` is undefined (plain `npm run dev` in a
 *   browser) the DEMO-ONLY backend in `./demo` is used instead. This is the only
 *   place that decision is made.
 */

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { createDemoBackend } from './demo';
import type {
  AdapterView,
  AppInfo,
  AuditEntry,
  AutomationStats,
  CapabilityIndex,
  CapabilityValue,
  DeviceView,
  MockProfile,
  MockStatus,
  Rule,
  RuleCheck,
  RuleConflict,
  HandoverReport,
  RuleFileNote,
  RuleOutcome,
  RuntimeEvent,
  RuntimeSnapshot,
  Sample,
  Settings,
  WriteReport,
} from '../types';

/** Normalised backend failure: mirrors the serialized error the backend rejects with. */
export class BackendError extends Error {
  readonly code: string;
  readonly hint?: string;
  readonly unsupported: boolean;

  constructor(code: string, message: string, hint?: string, unsupported = false) {
    super(message);
    this.name = 'BackendError';
    this.code = code;
    this.hint = hint;
    this.unsupported = unsupported;
  }
}

/** True when the UI is running in a plain browser against the demo backend. */
export const isDemoMode: boolean =
  typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window);

const demo = isDemoMode ? createDemoBackend() : null;

function normaliseError(error: unknown): BackendError {
  if (error instanceof BackendError) return error;
  if (typeof error === 'string') {
    try {
      return normaliseError(JSON.parse(error));
    } catch {
      return new BackendError('unknown', error);
    }
  }
  if (error && typeof error === 'object') {
    const record = error as Record<string, unknown>;
    const message =
      typeof record.message === 'string'
        ? record.message
        : error instanceof Error
          ? error.message
          : 'The backend returned an unknown error.';
    const code = typeof record.code === 'string' ? record.code : 'unknown';
    const hint = typeof record.hint === 'string' ? record.hint : undefined;
    const unsupported = record.unsupported === true;
    return new BackendError(code, message, hint, unsupported);
  }
  return new BackendError('unknown', 'The backend returned an unknown error.');
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (demo) return (await demo.call(command, args)) as T;
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw normaliseError(error);
  }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

export const api = {
  appInfo: () => call<AppInfo>('app_info'),
  getSnapshot: () => call<RuntimeSnapshot>('get_snapshot'),
  scanDevices: () => call<RuntimeSnapshot>('scan_devices'),
  getDevices: () => call<DeviceView[]>('get_devices'),
  getDevice: (device: string) => call<DeviceView>('get_device', { device }),
  getHistory: (device: string, capability: string, limit: number) =>
    call<Sample[]>('get_history', { device, capability, limit }),
  writeCapability: (device: string, capability: string, value: CapabilityValue) =>
    call<WriteReport>('write_capability', { device, capability, value }),
  setDeviceEnabled: (device: string, enabled: boolean) =>
    call<null>('set_device_enabled', { device, enabled }),
  setAdapterEnabled: (adapter: string, enabled: boolean) =>
    call<Settings>('set_adapter_enabled', { adapter, enabled }),

  getSettings: () => call<Settings>('get_settings'),
  updateSettings: (settings: Settings) => call<Settings>('update_settings', { settings }),

  capabilityIndex: () => call<CapabilityIndex>('capability_index'),
  getAdapters: () => call<AdapterView[]>('get_adapters'),

  listRules: () => call<Rule[]>('list_rules'),
  ruleOutcomes: () => call<RuleOutcome[]>('rule_outcomes'),
  /** Rules that fight over one output, as resolved when the rule files loaded. */
  ruleConflicts: () => call<RuleConflict[]>('rule_conflicts'),
  /**
   * Actions a legacy rule file asks for that this build cannot honour. The
   * runtime substituted the fail-safe duty in memory; the files are untouched.
   */
  ruleCompatibilityNotes: () => call<RuleFileNote[]>('rule_compatibility_notes'),
  /**
   * Channels a rule left behind, including handovers the fail-safe duty has not
   * taken over yet — the record outlives the rule that abandoned the channel.
   */
  ruleHandovers: () => call<HandoverReport[]>('rule_handovers'),
  /** Re-arm every handover that ran out of attempts; returns how many. */

  checkRule: (rule: Rule) => call<RuleCheck>('check_rule', { rule }),
  saveRule: (rule: Rule) => call<Rule>('save_rule', { rule }),
  deleteRule: (id: string) => call<boolean>('delete_rule', { id }),
  setRuleEnabled: (id: string, enabled: boolean) =>
    call<Rule>('set_rule_enabled', { id, enabled }),
  suggestRules: () => call<Rule[]>('suggest_rules'),

  listAuditLog: (limit: number) => call<AuditEntry[]>('list_audit_log', { limit }),
  automationStats: () => call<AutomationStats>('automation_stats'),

  mockStatus: () => call<MockStatus | null>('mock_status'),
  mockSetLoad: (gpu: number, cpu: number) => call<MockStatus | null>('mock_set_load', { gpu, cpu }),
  mockApplyProfile: (profile: MockProfile) =>
    call<MockStatus | null>('mock_apply_profile', { profile }),
  mockSetAmbient: (celsius: number) => call<MockStatus | null>('mock_set_ambient', { celsius }),
  mockForceGpuTemperature: (celsius: number) =>
    call<MockStatus | null>('mock_force_gpu_temperature', { celsius }),
  /** A fault on one simulated channel: `none`, `unconfirmed` or `reject`. */
  mockSetChannelFault: (device: string, capability: string, fault: string) =>
    call<MockStatus | null>('mock_set_channel_fault', { device, capability, fault }),
  /**
   * Re-arm failed handovers. With a channel named, only that channel is re-armed —
   * which is the only form an IPC self-test may use, because re-arming everything
   * could touch real pending work.
   */
  ruleRetryHandovers: (device?: string, capability?: string) =>
    call<number>('rule_retry_handovers', { device, capability }),
  /**
   * Report the result of an IPC self-test run. Only used when the app was started with
   * `--ipc-selftest`; writes the frontend's own account of what it saw next to the
   * app's log so the two can be compared.
   */
  ipcProbeReport: (body: string) => call<string>('ipc_probe_report', { body }),
  mockSetFaults: (failWrites: boolean, disconnectGpuTemperature: boolean, unplugFan: boolean) =>
    call<MockStatus | null>('mock_set_faults', {
      fail_writes: failWrites,
      disconnect_gpu_temperature: disconnectGpuTemperature,
      unplug_fan: unplugFan,
    }),

  openConfigDir: () => call<null>('open_config_dir'),
  openLogDir: () => call<null>('open_log_dir'),
};

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/** Subscribe to pushed snapshots. Resolves to an unlisten function. */
export function onSnapshot(handler: (snapshot: RuntimeSnapshot) => void): Promise<UnlistenFn> {
  if (demo) {
    return Promise.resolve(
      demo.subscribe(
        handler,
        () => undefined,
      ),
    );
  }
  return listen<RuntimeSnapshot>('snapshot', (event) => handler(event.payload));
}

/** Subscribe to the tagged `runtime-event` stream. Resolves to an unlisten function. */
export function onRuntimeEvent(handler: (event: RuntimeEvent) => void): Promise<UnlistenFn> {
  if (demo) {
    return Promise.resolve(
      demo.subscribe(
        () => undefined,
        handler,
      ),
    );
  }
  return listen<RuntimeEvent>('runtime-event', (event) => handler(event.payload));
}

/**
 * The demo backend drives both streams from one ticker, so the two subscriptions
 * are merged into a single one to avoid stepping the simulation twice.
 */
export function subscribeRuntime(
  onSnap: (snapshot: RuntimeSnapshot) => void,
  onEvent: (event: RuntimeEvent) => void,
): Promise<UnlistenFn> {
  if (demo) return Promise.resolve(demo.subscribe(onSnap, onEvent));
  let unlisten: UnlistenFn[] = [];
  let cancelled = false;
  const ready = Promise.all([
    listen<RuntimeSnapshot>('snapshot', (event) => onSnap(event.payload)),
    listen<RuntimeEvent>('runtime-event', (event) => onEvent(event.payload)),
  ]).then((fns) => {
    if (cancelled) fns.forEach((fn) => fn());
    else unlisten = fns;
  });
  void ready;
  return Promise.resolve(() => {
    cancelled = true;
    unlisten.forEach((fn) => fn());
    unlisten = [];
  });
}
