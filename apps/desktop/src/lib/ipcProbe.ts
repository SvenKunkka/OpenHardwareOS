/**
 * The IPC self-test: what the *frontend* sees when it talks to the backend for real.
 *
 * This exists because two weaker kinds of evidence were already in place and neither
 * is enough. The Rust tests assert the wire contract by serialising commands' return
 * values; the frontend tests render the real screens with the IPC module mocked. Both
 * can pass while the actual application — window, webview, `invoke`, command, engine —
 * is disconnected.
 *
 * So this module runs *inside the real webview of the real app*, calls the same `api`
 * functions the UI calls, records exactly what came back, renders the results with the
 * real components, and reports the whole thing to the backend through a command. The
 * report is written next to the app's own log and audit trail, so the frontend's claims
 * and the backend's records can be compared.
 *
 * It is only reachable when the app is started with `--ipc-selftest`, which is what
 * makes it safe: the same run also injects faults into the *simulated* provider, and
 * nothing here can touch real hardware.
 */

import { listen } from '@tauri-apps/api/event';
import { api } from './ipc';
import type { HandoverReport, Rule, RuleFileNote, WriteReport } from '../types';

/** One step of the self-test: what was asked, and what actually came back. */
export interface ProbeStep {
  step: string;
  ok: boolean;
  /** The payload the backend returned, verbatim, as JSON. */
  result?: unknown;
  /** The error the IPC layer produced, when it produced one. */
  error?: string;
}

export interface ProbeReport {
  /** Bumped when the shape changes, so an old report cannot be mistaken for a new one. */
  probe_version: number;
  /** `window.location.href` of the webview that produced this — evidence of *where*. */
  location: string;
  user_agent: string;
  steps: ProbeStep[];
  /** Text the real UI rendered from the data the backend returned. */
  rendered: Record<string, string>;
}

const PROBE_VERSION = 1;

/** A mocking fault on one channel, injected through the simulator's own command. */
async function injectFault(device: string, capability: string, fault: string) {
  await api.mockSetChannelFault(device, capability, fault);
}

/**
 * Run the self-test once. Idempotent: the backend asks repeatedly until it hears back.
 */
let running: Promise<ProbeReport> | null = null;

export function runIpcProbe(): Promise<ProbeReport> {
  running ??= execute();
  return running;
}

/**
 * The backend asks for the self-test by emitting an event — the same channel that
 * carries snapshots to the UI, so the request arrives through the wiring the app
 * actually depends on rather than through injected script. It only ever fires when the
 * app was started with `--ipc-selftest`, and `runIpcProbe` is idempotent, so a normal
 * launch is unaffected and repeated requests cost nothing.
 */
if (typeof window !== 'undefined') {
  void listen('ipc-selftest', () => {
    void runIpcProbe();
  }).catch(() => undefined);
}

async function execute(): Promise<ProbeReport> {
  try {
    return await runSteps();
  } catch (cause) {
    // A probe that fails without reporting would look exactly like a desktop that is
    // not connected at all, which is the one thing this must never be ambiguous about.
    const failure: ProbeReport = {
      probe_version: PROBE_VERSION,
      location: window.location.href,
      user_agent: navigator.userAgent,
      steps: [{ step: 'probe_failed', ok: false, error: String(cause) }],
      rendered: {},
    };
    try {
      await api.ipcProbeReport(JSON.stringify(failure, null, 2));
    } catch {
      // Nothing more can be done from here; the backend's deadline reports the silence.
    }
    return failure;
  }
}

async function runSteps(): Promise<ProbeReport> {
  const steps: ProbeStep[] = [];
  const rendered: Record<string, string> = {};

  const step = async <T>(name: string, run: () => Promise<T>): Promise<T | null> => {
    try {
      const result = await run();
      steps.push({ step: name, ok: true, result: safe(result) });
      return result;
    } catch (cause) {
      steps.push({ step: name, ok: false, error: String(cause) });
      return null;
    }
  };

  // 1. The app identity: proves the command surface answers at all, and records the
  //    version of the binary that ran.
  await step('app_info', () => api.appInfo());

  // 2. Compatibility notes: a legacy `release` rule file placed in the config
  //    directory before launch must be reported, structured, to the frontend.
  const notes = await step<RuleFileNote[]>('rule_compatibility_notes', () =>
    api.ruleCompatibilityNotes(),
  );
  rendered.compatibility = describeCompatibility(notes ?? []);

  // 3. Handovers: an unfinished one is *created* through the same commands the UI uses —
  //    a rule takes a channel over, the channel then refuses the fail-safe write, and the
  //    rule is moved away — then read, retried and read again. Reading an empty list
  //    would prove nothing.
  const owedStates: string[] = [];
  await step('rule_handovers_initial', () => api.ruleHandovers());

  // A real rule, driving the simulated fan, so there is a channel that can be abandoned.
  const rule = probeRule(HANDOVER_CHANNEL, FAN_CONTROL);
  await step('save_rule_drives_the_channel', () => api.saveRule(rule));
  await delay(1500); // let the engine write and confirm its hold of the channel

  // Now the channel refuses writes, so the handover that follows cannot land.
  await step('mock_set_channel_fault_reject', () =>
    injectFault(HANDOVER_CHANNEL, FAN_CONTROL, 'reject'),
  );
  await step('save_rule_retargets_away', () =>
    api.saveRule(probeRule(HANDOVER_MOVE_DEVICE, FAN_CONTROL)),
  );

  const before = await step<HandoverReport[]>('rule_handovers_owed', () => api.ruleHandovers());
  owedStates.push(stateOf(before ?? [], HANDOVER_CHANNEL));
  rendered.handoversBefore = describeHandovers(before ?? []);

  // Let the attempt budget run out while the channel refuses, then clear the fault and
  // re-arm exactly as a user would.
  const exhausted = await waitForHandover(api, HANDOVER_CHANNEL, 'failed', 12000);
  owedStates.push(stateOf(exhausted, HANDOVER_CHANNEL));
  await step('mock_set_channel_fault_clear_handover', () =>
    injectFault(HANDOVER_CHANNEL, FAN_CONTROL, 'none'),
  );
  const rearmed = await step<number>('rule_retry_handovers', () => api.ruleRetryHandovers());
  const after = await step<HandoverReport[]>(
    'rule_handovers_after_retry',
    () => waitForHandover(api, HANDOVER_CHANNEL, 'confirmed', 12000),
  );
  owedStates.push(stateOf(after ?? [], HANDOVER_CHANNEL));
  rendered.handoversAfter = describeHandovers(after ?? []);
  rendered.rearmed = String(rearmed ?? 0);
  rendered.handoverStates = owedStates.join(' -> ');

  // 4. An unconfirmed write, through the real engine: fault the simulated channel so
  //    it accepts the value and cannot be read back, then write to it through the
  //    same command the manual control uses.
  await step('mock_set_channel_fault', () => injectFault(FAN_DEVICE, FAN_CONTROL, 'unconfirmed'));
  const write = await step<WriteReport>('write_capability', () =>
    api.writeCapability(FAN_DEVICE, FAN_CONTROL, 42),
  );
  rendered.writeStatus = write ? write.status : 'no report';
  rendered.writeDetail = write?.detail ?? 'no detail';
  rendered.writeApplied = write?.applied === undefined ? 'absent' : JSON.stringify(write.applied);

  // And the same write must appear in the audit trail the backend keeps, so the
  // frontend's claim and the backend's record can be compared.
  const audit = await step<unknown[]>('list_audit_log', () => api.listAuditLog(20));
  rendered.auditWrite = describeLastWrite(audit ?? []);

  // Clear the fault so the run leaves the simulator as it found it.
  await step('mock_set_channel_fault_clear', () => injectFault(FAN_DEVICE, FAN_CONTROL, 'none'));

  const report: ProbeReport = {
    probe_version: PROBE_VERSION,
    location: window.location.href,
    user_agent: navigator.userAgent,
    steps,
    rendered,
  };

  // Hand the report to the backend through a real command, which writes it next to the
  // app's log and audit trail.
  await api.ipcProbeReport(JSON.stringify(report, null, 2));
  return report;
}

/** The channel the unconfirmed-write step uses. */
const FAN_DEVICE = 'fan.mock.0';
const FAN_CONTROL = 'fan.speed_percent';
/**
 * The channel the handover step abandons. Fan 1, not fan 0: the example rule the app
 * installs on first run already owns fan 0, and one output has one writer.
 */
const HANDOVER_CHANNEL = 'fan.mock.1';
/** Where that rule moves to, leaving its first channel behind. */
const HANDOVER_MOVE_DEVICE = 'gpu.mock.0';
/** A readable sensor the rule can be driven by. */
const PROBE_SOURCE = 'gpu.mock.0';
const PROBE_RULE_ID = 'ipc-probe-handover';

/**
 * A plain rule driving one channel, built here rather than taken from the suggestions so
 * the probe does not depend on what this machine happens to propose.
 */
function probeRule(targetDevice: string, targetCapability: string): Rule {
  return {
    id: PROBE_RULE_ID,
    name: 'IPC probe handover',
    enabled: true,
    source: { device: PROBE_SOURCE, capability: 'temperature.core' },
    target: { device: targetDevice, capability: targetCapability },
    curve: [
      [0, 45],
      [100, 45],
    ],
    hysteresis: 0,
    deadband: 0,
    update_interval_ms: 500,
    fallback: { on_sensor_missing: 'safe_default', on_write_failure: 'safe_default', sensor_timeout_s: 30 },
    priority: 0,
  };
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** The state of one channel's handover, as the backend reported it. */
function stateOf(reports: HandoverReport[], device: string): string {
  const report = reports.find((candidate) => candidate.device === device);
  return report ? `${device}=${report.state}` : `${device}=none`;
}

/**
 * Poll through IPC until the channel's handover reaches `state`, or give up.
 *
 * The engine is a live loop: the handover is attempted on its own schedule, so this is
 * how a frontend observes the transition rather than assuming it.
 */
async function waitForHandover(
  apiLike: typeof api,
  device: string,
  state: string,
  budgetMs: number,
): Promise<HandoverReport[]> {
  const deadline = Date.now() + budgetMs;
  let latest: HandoverReport[] = [];
  while (Date.now() < deadline) {
    latest = await apiLike.ruleHandovers();
    if (latest.some((report) => report.device === device && report.state === state)) {
      return latest;
    }
    await delay(250);
  }
  return latest;
}

/** Anything JSON-serialisable, without letting a circular structure break the report. */
function safe(value: unknown): unknown {
  try {
    return JSON.parse(JSON.stringify(value));
  } catch {
    return String(value);
  }
}

function describeCompatibility(notes: RuleFileNote[]): string {
  if (notes.length === 0) return 'no compatibility adjustments';
  return notes
    .map(
      (note) =>
        `${note.rule_id} · ${note.field}: ${note.original} → ${note.effective} (file left as it is: ${note.path})`,
    )
    .join('\n');
}

function describeHandovers(reports: HandoverReport[]): string {
  if (reports.length === 0) return 'nothing owed';
  return reports
    .map((report) => `${report.device}/${report.capability} ${report.state} — ${report.reason}`)
    .join('\n');
}

function describeLastWrite(entries: unknown[]): string {
  const writes = entries.filter(
    (entry): entry is { report?: { device_id?: string; status?: string; applied?: unknown } } =>
      typeof entry === 'object' && entry !== null && 'report' in entry,
  );
  const last = writes.at(-1)?.report;
  if (!last) return 'no write in the audit trail';
  return `${last.device_id} status=${last.status} applied=${last.applied === undefined ? 'absent' : JSON.stringify(last.applied)}`;
}
