/**
 * The `src/lib/ipc.ts` stand-in used by every frontend test.
 *
 * Tests mock the module (never the Tauri bridge): `vi.mock('../lib/ipc', ...)`
 * resolves to `createIpcMock()`, and the test configures the returned promises
 * through `ipcMock()`. The baseline answers are the happy path — a mounted
 * snapshot, no rules, no conflicts — so each test only overrides what it is
 * actually about.
 */

import { vi } from 'vitest';
import { capabilityIndexFixture, snapshotFixture } from './fixtures';
import type {
  CapabilityIndex,
  HandoverReport,
  RuleCheck,
  RuleConflict,
  RuleFileNote,
  RuntimeSnapshot,
} from '../types';

/** Every command `api` exposes; a missing one would be a TypeError at call time. */
const API_METHODS = [
  'appInfo',
  'getSnapshot',
  'scanDevices',
  'getDevices',
  'getDevice',
  'getHistory',
  'writeCapability',
  'setDeviceEnabled',
  'setAdapterEnabled',
  'getSettings',
  'updateSettings',
  'capabilityIndex',
  'getAdapters',
  'listRules',
  'ruleOutcomes',
  'ruleConflicts',
  'ruleCompatibilityNotes',
  'ruleHandovers',
  'ruleRetryHandovers',
  'checkRule',
  'saveRule',
  'deleteRule',
  'setRuleEnabled',
  'suggestRules',
  'listAuditLog',
  'automationStats',
  'mockStatus',
  'mockSetLoad',
  'mockApplyProfile',
  'mockSetAmbient',
  'mockForceGpuTemperature',
  'mockSetFaults',
  'mockSetChannelFault',
  'ipcProbeReport',
  'openConfigDir',
  'openLogDir',
] as const;

type ApiMethod = (typeof API_METHODS)[number];

export type MockApi = Record<ApiMethod, ReturnType<typeof vi.fn>>;

/** A stand-in for the real `BackendError` (same fields, same `instanceof`). */
export class MockBackendError extends Error {
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

export interface IpcMock {
  api: MockApi;
  isDemoMode: boolean;
  subscribeRuntime: ReturnType<typeof vi.fn>;
  onSnapshot: ReturnType<typeof vi.fn>;
  onRuntimeEvent: ReturnType<typeof vi.fn>;
  BackendError: typeof MockBackendError;
}

let current: IpcMock | null = null;

const unlisten = async (): Promise<() => void> => () => undefined;

/** The mock module the tests substitute for `src/lib/ipc.ts`. */
export function createIpcMock(): IpcMock {
  const api = Object.fromEntries(API_METHODS.map((name) => [name, vi.fn()])) as MockApi;
  const mock: IpcMock = {
    api,
    isDemoMode: false,
    subscribeRuntime: vi.fn(unlisten),
    onSnapshot: vi.fn(unlisten),
    onRuntimeEvent: vi.fn(unlisten),
    BackendError: MockBackendError,
  };
  current = mock;
  applyBaseline(mock);
  return mock;
}

/** The mock created for the current test file. */
export function ipcMock(): IpcMock {
  if (!current) throw new Error('createIpcMock() has not run: is vi.mock("../lib/ipc") in place?');
  return current;
}

function applyBaseline(mock: IpcMock): void {
  const snapshot: RuntimeSnapshot = snapshotFixture();
  const index: CapabilityIndex = capabilityIndexFixture();
  const clean: RuleCheck = { errors: [], warnings: [] };
  mock.api.getSnapshot.mockResolvedValue(snapshot);
  mock.api.scanDevices.mockResolvedValue(snapshot);
  mock.api.getSettings.mockResolvedValue(snapshot.settings);
  mock.api.updateSettings.mockResolvedValue(snapshot.settings);
  mock.api.setAdapterEnabled.mockResolvedValue(snapshot.settings);
  mock.api.getAdapters.mockResolvedValue([]);
  mock.api.appInfo.mockResolvedValue({
    version: '0.1.0-test',
    platform: 'test',
    config_root: '/tmp/ohm',
    protocol_version: '1',
    os: 'test',
    arch: 'test',
    elevated: false,
  });
  mock.api.capabilityIndex.mockResolvedValue(index);
  mock.api.listRules.mockResolvedValue([]);
  mock.api.ruleOutcomes.mockResolvedValue([]);
  mock.api.ruleConflicts.mockResolvedValue([] as RuleConflict[]);
  mock.api.ruleCompatibilityNotes.mockResolvedValue([] as RuleFileNote[]);
  mock.api.ruleHandovers.mockResolvedValue([] as HandoverReport[]);
  mock.api.ruleRetryHandovers.mockResolvedValue(0);
  mock.api.checkRule.mockResolvedValue(clean);
  mock.api.saveRule.mockImplementation(async (rule: unknown) => rule);
  mock.api.deleteRule.mockResolvedValue(true);
  mock.api.setRuleEnabled.mockImplementation(async () => undefined);
  mock.api.suggestRules.mockResolvedValue([]);
  mock.api.listAuditLog.mockResolvedValue([]);
  mock.api.automationStats.mockResolvedValue({
    rules: 0,
    enabled_rules: 0,
    ticks: 0,
    evaluations: 0,
    writes: 0,
    skipped: 0,
    fallbacks: 0,
    failures: 0,
    last_tick_ms: 0,
  });
  mock.api.mockSetChannelFault.mockResolvedValue(null);
  mock.api.ipcProbeReport.mockResolvedValue('');
  mock.api.mockStatus.mockResolvedValue(null);
  mock.api.openConfigDir.mockResolvedValue(null);
  mock.api.openLogDir.mockResolvedValue(null);
}

/** Reset the call history and put the baseline answers back, between tests. */
export function resetIpcMock(): IpcMock {
  const mock = ipcMock();
  vi.clearAllMocks();
  applyBaseline(mock);
  return mock;
}
