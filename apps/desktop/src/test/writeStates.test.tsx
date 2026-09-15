/**
 * What the screens must say about the two states the backend can now produce.
 *
 * These are rendered-behaviour tests, not helper tests: the real screens run
 * against the real providers and the real user actions are driven, with only
 * `src/lib/ipc.ts` mocked. A helper can be right while the screen still paints
 * an `unconfirmed` write green, so every assertion here is made against the
 * markup the components actually emit (badge tone classes, feed level markers,
 * visible sentences).
 *
 * 1. `unconfirmed` — the device accepted the write but never confirmed the
 *    value. It is neither success nor failure: it must never read as success,
 *    and it must never claim the requested value was applied.
 * 2. `rejected` — known to have failed, with the backend's own reason.
 * 3. A legacy `release` fallback — substituted in memory; the file on disk is
 *    untouched and the fail-safe duty still protects the machine.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';

vi.mock('../lib/ipc', async () => (await import('./ipcMock')).createIpcMock());

import { Automation } from '../screens/Automation';
import { DeviceDetail } from '../screens/DeviceDetail';
import { Diagnostics } from '../screens/Diagnostics';
import { ruleStatusHint, ruleStatusLabel } from '../lib/format';
import { isSimpleFallback } from '../lib/curve';
import type { FallbackAction, RuntimeEvent } from '../types';
import { renderWithProviders } from './helpers';
import { ipcMock, resetIpcMock } from './ipcMock';
import { ToastViewport } from '../hooks/useToast';
import {
  FAN_ID,
  GPU_ID,
  handoverFixture,
  ruleFileNoteFixture,
  ruleFixture,
  writeAuditEntry,
  writeReportFixture,
} from './fixtures';

/** The backend's own reason, shown verbatim. */
const UNCONFIRMED_DETAIL = 'the channel could not be read back after the write was accepted';
const REJECTED_DETAIL = 'below the minimum duty of 20 %: the safety policy refused the write';
const NOTE_MESSAGE =
  'The file asks for “release”, which no adapter in this build can perform, so the fail-safe duty runs instead.';
const NOTE_HINT = 'Edit the rule and choose “safe default” to make the file match what actually runs.';
const NOTE_PATH = '/home/user/.config/openhardwareos/rules/gpu.json';

function renderDevice() {
  return renderWithProviders(
    <>
      <DeviceDetail deviceId={GPU_ID} onBack={() => undefined} onOpenSettings={() => undefined} />
      {/* The app shell renders the viewport; without it a pushed toast is invisible. */}
      <ToastViewport />
    </>,
  );
}

/** Type a value into the fan-duty control and click Apply, like a user would. */
async function applyFanDuty(value: string): Promise<void> {
  const input = await screen.findByLabelText('Fan duty value');
  // The change has to be committed before the button is clicked: Apply is
  // disabled while the field is empty or invalid, and a click on a disabled
  // button does nothing at all. Firing both inside one `act` let this pass
  // locally and fail on a CI runner (the click arrived while the field was still
  // empty, no write was attempted, and the test blamed the product).
  await act(async () => {
    fireEvent.change(input, { target: { value } });
  });
  const apply = screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement;
  await waitFor(() => expect(apply.disabled).toBe(false));
  // Awaiting inside `act` lets the submit's own promise settle before the
  // assertions run, so the test sees the finished state, not a half-open one.
  await act(async () => {
    fireEvent.click(apply);
  });
  // Prove the click reached the handler before the caller asserts on the result:
  // on a loaded CI runner the click could otherwise be observed before React had
  // run it, and the failure looked like a product defect.
  await waitFor(() => expect(ipcMock().api.writeCapability).toHaveBeenCalled());
}

/** The row of a feed whose text contains `needle`. */
function rowContaining(rows: HTMLElement[], needle: string): HTMLElement {
  const row = rows.find((item) => (item.textContent ?? '').includes(needle));
  if (!row) throw new Error(`no feed row contains ${JSON.stringify(needle)}`);
  return row;
}

/** The level marker element (`.feed__level`) the feed colours and labels a row with. */
function levelOf(row: HTMLElement): HTMLElement {
  const level = row.querySelector('.feed__level');
  if (!level) throw new Error('the feed row has no level marker');
  return level as HTMLElement;
}

describe('manual control: a write the device never confirmed', () => {
  beforeEach(() => {
    resetIpcMock();
    ipcMock().api.getHistory.mockResolvedValue([]);
    ipcMock().api.writeCapability.mockResolvedValue(
      writeReportFixture({ status: 'unconfirmed', applied: undefined, detail: UNCONFIRMED_DETAIL }),
    );
  });

  it('reports the request as unconfirmed and never as success', async () => {
    renderDevice();
    await applyFanDuty('80');

    const result = await screen.findByTestId('write-result');
    // The write really was attempted with the value the user typed.
    expect(ipcMock().api.writeCapability).toHaveBeenCalledWith(GPU_ID, 'fan.speed_percent', 80);

    // (a) no success styling and not the word "applied" anywhere in the panel.
    const badge = screen.getByText('requested, not confirmed');
    expect(badge.className).toContain('badge--warn');
    expect(badge.className).not.toContain('badge--ok');
    expect(result.textContent).not.toMatch(/applied/i);
    expect(badge.getAttribute('title')).toMatch(/never confirmed/i);

    // (b) it says the request went out and was not confirmed.
    expect(screen.getByTestId('write-result-known').textContent).toMatch(/accepted and sent/i);
    expect(screen.getByTestId('write-result-known').textContent).toMatch(/unknown/i);
    expect(screen.getByTestId('write-unconfirmed-hint').textContent).toMatch(/retries this write/i);
    expect(screen.getByTestId('write-unconfirmed-hint').textContent).toMatch(/fail-safe duty/i);

    // (c) the backend's own reason, verbatim.
    expect(screen.getByTestId('write-unconfirmed-message').textContent).toContain(
      UNCONFIRMED_DETAIL,
    );

    // (d) the requested value is never presented as the applied value, and the
    // missing value is never dressed up as "nothing" or "not applied".
    const value = screen.getByTestId('write-result-value');
    expect(value.textContent).toContain('value unknown — not confirmed');
    expect(value.textContent).not.toMatch(/→\s*80/);
    expect(value.textContent).toContain('Requested 80 %');
    expect(result.textContent).not.toMatch(/nothing|not applied/i);
    // The toast makes the same claim globally, with the same reason.
    expect(screen.getByText('Fan duty was not confirmed')).toBeTruthy();
  });

  it('moves from "not confirmed" to the confirmed value when the retry is read back', async () => {
    // A counter rather than queued one-shot answers: the test stays independent
    // of its neighbours even when an earlier assertion fails.
    let submits = 0;
    ipcMock().api.writeCapability.mockImplementation(async () =>
      submits++ === 0
        ? writeReportFixture({ status: 'unconfirmed', applied: undefined, detail: UNCONFIRMED_DETAIL })
        : writeReportFixture({ status: 'applied', requested: 80, applied: 80 }),
    );

    renderDevice();
    await applyFanDuty('80');

    expect(await screen.findByText('requested, not confirmed')).toBeTruthy();
    expect(screen.getByTestId('write-result-value').textContent).not.toMatch(/→\s*80/);

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
    });

    await waitFor(() => expect(screen.queryByText('requested, not confirmed')).toBeNull());
    const value = screen.getByTestId('write-result-value');
    expect(value.textContent).toBe('Requested 80 % → 80 %');
    expect(screen.getByTestId('write-result-known').textContent).toMatch(/confirmed the write/i);
    expect(screen.queryByTestId('write-unconfirmed-message')).toBeNull();

    const badge = screen.getByText('applied');
    expect(badge.className).toContain('badge--ok');
  });
});

describe('manual control: a write the runtime refused', () => {
  beforeEach(() => {
    resetIpcMock();
    ipcMock().api.getHistory.mockResolvedValue([]);
    ipcMock().api.writeCapability.mockResolvedValue(
      writeReportFixture({
        status: 'rejected',
        requested: 5,
        applied: undefined,
        error_code: 'safety_violation',
        detail: REJECTED_DETAIL,
      }),
    );
  });

  it('shows a refusal with the backend reason, distinct from an unconfirmed write', async () => {
    renderDevice();
    await applyFanDuty('5');

    const result = await screen.findByTestId('write-result');

    // Danger presentation, not the warning an unknown outcome gets.
    const badge = screen.getByText('refused');
    expect(badge.className).toContain('badge--danger');
    expect(badge.className).not.toContain('badge--warn');
    expect(badge.className).not.toContain('badge--ok');

    // The refusal is known to have failed, and it says so.
    const notice = screen.getByTestId('write-failure-message').closest('.inline-notice');
    expect(notice?.className).toContain('inline-notice--error');
    expect(notice?.getAttribute('role')).toBe('alert');
    expect(screen.getByText('Write refused (safety_violation)')).toBeTruthy();
    expect(screen.getByTestId('write-failure-message').textContent).toBe(REJECTED_DETAIL);

    // Nothing changed — and it is not described as an unknown outcome.
    expect(screen.getByTestId('write-result-value').textContent).toContain('unchanged');
    expect(screen.getByTestId('write-result-value').textContent).not.toMatch(/→\s*5/);
    expect(result.textContent).toMatch(/refused the write/i);
    expect(screen.queryByText('requested, not confirmed')).toBeNull();
    expect(screen.queryByTestId('write-unconfirmed-message')).toBeNull();
  });
});

describe('the recent-writes list on a device', () => {
  const applied = writeReportFixture({
    at_ms: 1_700_000_003_000,
    requested: 45,
    applied: 45,
  });
  const unconfirmed = writeReportFixture({
    at_ms: 1_700_000_002_000,
    status: 'unconfirmed',
    requested: 80,
    applied: undefined,
    detail: UNCONFIRMED_DETAIL,
  });
  const rejected = writeReportFixture({
    at_ms: 1_700_000_001_000,
    requested: 5,
    applied: undefined,
    status: 'rejected',
    error_code: 'safety_violation',
    detail: REJECTED_DETAIL,
  });

  beforeEach(() => {
    resetIpcMock();
    ipcMock().api.getHistory.mockResolvedValue([]);
    ipcMock().api.listAuditLog.mockResolvedValue([
      writeAuditEntry(applied),
      writeAuditEntry(unconfirmed),
      writeAuditEntry(rejected),
    ]);
  });

  it('tones each row by its real status: unconfirmed is not success', async () => {
    renderDevice();
    const list = await screen.findByRole('list', { name: 'Recent writes for this device' });
    const rows = within(list).getAllByRole('listitem');
    expect(rows).toHaveLength(3);

    const appliedRow = rowContaining(rows, 'Fan duty: 45 → 45');
    expect(levelOf(appliedRow).textContent).toBe('applied');
    expect(levelOf(appliedRow).className).toContain('feed__level--ok');

    const unconfirmedRow = rowContaining(rows, 'value unknown — not confirmed');
    expect(levelOf(unconfirmedRow).textContent).toBe('requested, not confirmed');
    expect(levelOf(unconfirmedRow).className).toContain('feed__level--warn');
    expect(levelOf(unconfirmedRow).className).not.toContain('feed__level--ok');
    expect(unconfirmedRow.textContent).not.toMatch(/applied/i);
    // The unknown value is stated, and the backend reason is shown with it.
    expect(unconfirmedRow.textContent).not.toMatch(/nothing|not applied/i);
    expect(unconfirmedRow.textContent).toContain(UNCONFIRMED_DETAIL);

    const rejectedRow = rowContaining(rows, 'unchanged — the write was refused');
    expect(levelOf(rejectedRow).textContent).toBe('refused');
    expect(levelOf(rejectedRow).className).toContain('feed__level--error');
    expect(rejectedRow.textContent).toContain(REJECTED_DETAIL);
  });

  it('does not paint the live write event for the same write as success either', async () => {
    // The list merges the audit log with the pushed runtime events, so an
    // unconfirmed write must not arrive through the second door painted green.
    let deliver: ((event: RuntimeEvent) => void) | undefined;
    ipcMock().subscribeRuntime.mockImplementation(
      async (_onSnap: unknown, onEvent: (event: RuntimeEvent) => void) => {
        deliver = onEvent;
        return () => undefined;
      },
    );

    renderDevice();
    await screen.findByLabelText('Fan duty value');

    act(() => {
      deliver?.({ type: 'write_performed', report: unconfirmed });
    });

    const list = await screen.findByRole('list', { name: 'Recent writes for this device' });
    const row = rowContaining(within(list).getAllByRole('listitem'), 'Fan duty write unconfirmed');
    expect(levelOf(row).className).toContain('feed__level--warn');
    expect(levelOf(row).className).not.toContain('feed__level--ok');
    expect(row.textContent).toContain(UNCONFIRMED_DETAIL);
  });
});

describe('the audit log on Diagnostics', () => {
  const applied = writeReportFixture({ at_ms: 1_700_000_003_000, requested: 45, applied: 45 });
  const unconfirmed = writeReportFixture({
    at_ms: 1_700_000_002_000,
    status: 'unconfirmed',
    requested: 80,
    applied: undefined,
    detail: UNCONFIRMED_DETAIL,
  });
  const rejected = writeReportFixture({
    at_ms: 1_700_000_001_000,
    requested: 5,
    applied: undefined,
    status: 'rejected',
    error_code: 'safety_violation',
    detail: REJECTED_DETAIL,
  });

  beforeEach(() => {
    resetIpcMock();
    ipcMock().api.listAuditLog.mockResolvedValue([
      writeAuditEntry(applied),
      writeAuditEntry(unconfirmed),
      writeAuditEntry(rejected),
    ]);
  });

  it('describes an unconfirmed write as unknown, with its reason, never as "not applied"', async () => {
    renderWithProviders(<Diagnostics onOpenSettings={() => undefined} />);
    const list = await screen.findByRole('list', { name: 'Audit log' });
    const rows = within(list).getAllByRole('listitem');
    expect(rows).toHaveLength(3);

    const unconfirmedRow = rowContaining(rows, '(unconfirmed)');
    expect(unconfirmedRow.textContent).not.toContain('not applied');
    expect(unconfirmedRow.textContent).not.toContain('nothing');
    expect(unconfirmedRow.textContent).toContain('value unknown — not confirmed');
    expect(unconfirmedRow.textContent).toContain(UNCONFIRMED_DETAIL);
    expect(levelOf(unconfirmedRow).className).toContain('feed__level--warn');

    // The known outcomes keep their own markers, so the three are distinguishable.
    expect(levelOf(rowContaining(rows, '(applied)')).className).toContain('feed__level--info');
    expect(levelOf(rowContaining(rows, '(rejected)')).className).toContain('feed__level--error');
    expect(rowContaining(rows, '(rejected)').textContent).toContain(REJECTED_DETAIL);
  });
});

describe('the channel handovers on Diagnostics', () => {
  beforeEach(() => {
    resetIpcMock();
  });

  it('shows an owed handover in full and never as settled', async () => {
    ipcMock().api.ruleHandovers.mockResolvedValue([
      handoverFixture({
        state: 'pending',
        attempts: 2,
        first_error: 'the device did not answer within the write timeout',
        last_error: 'the channel could not be read back',
      }),
    ]);

    renderWithProviders(<Diagnostics onOpenSettings={() => undefined} />);

    const owed = await screen.findByTestId('handover');
    expect(owed.getAttribute('data-state')).toBe('pending');
    expect(screen.getByTestId('handover-channel').textContent).toBe(`${FAN_ID}/fan.speed_percent`);
    expect(screen.getByTestId('handover-from-rule').textContent).toBe('rule-gpu-fan-a');
    expect(screen.getByTestId('handover-reason').textContent).toBe(
      'The rule was disabled while it was driving this channel.',
    );
    expect(screen.getByTestId('handover-attempts').textContent).toContain('2 attempts');
    // The cause first, then the latest symptom because it differs.
    expect(screen.getByTestId('handover-cause').textContent).toContain(
      'the device did not answer within the write timeout',
    );
    expect(screen.getByTestId('handover-latest').textContent).toContain(
      'the channel could not be read back',
    );

    // It is not presented as fine, and not filed away as history.
    const badge = within(owed).getByText('pending');
    expect(badge.className).toContain('badge--warn');
    expect(badge.className).not.toContain('badge--ok');
    expect(screen.getByTestId('handover-state-note').textContent).toMatch(
      /not known to be protected/i,
    );
    expect(screen.getByText('Channel handovers (1 owed)')).toBeTruthy();
    expect(screen.queryByTestId('handover-resolved')).toBeNull();
    expect(screen.queryByTestId('handover-none-owed')).toBeNull();
  });

  it('marks a failed handover as needing action, and retrying re-arms and refreshes it', async () => {
    // A counter rather than queued one-shot answers: the test stays independent
    // of its neighbours even when an earlier assertion fails.
    let reads = 0;
    ipcMock().api.ruleHandovers.mockImplementation(async () =>
      reads++ === 0
        ? [
            handoverFixture({
              state: 'failed',
              attempts: 3,
              first_error: 'write refused: the safety policy rejected the fail-safe duty',
              // Identical to the cause: the row must not repeat it.
              last_error: 'write refused: the safety policy rejected the fail-safe duty',
            }),
          ]
        : [handoverFixture({ state: 'pending', attempts: 1 })],
    );
    ipcMock().api.ruleRetryHandovers.mockResolvedValue(1);

    renderWithProviders(<Diagnostics onOpenSettings={() => undefined} />);

    const failed = await screen.findByTestId('handover');
    expect(failed.getAttribute('data-state')).toBe('failed');
    const badge = within(failed).getByText('failed');
    expect(badge.className).toContain('badge--danger');
    expect(badge.className).not.toContain('badge--warn');
    expect(badge.className).not.toContain('badge--ok');
    expect(screen.getByTestId('handover-state-note').textContent).toMatch(/stopped retrying/i);
    expect(screen.getByTestId('handover-state-note').textContent).toMatch(/needs you to act/i);
    expect(screen.getByTestId('handover-cause').textContent).toContain('write refused');
    expect(screen.queryByTestId('handover-latest')).toBeNull();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Retry failed handovers/ }));
    });

    expect(ipcMock().api.ruleRetryHandovers).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('handover-retry-result').textContent).toContain('Re-armed 1 handover');
    // The queue is read again, and the re-armed channel is pending, not failed.
    await waitFor(() =>
      expect(screen.getByTestId('handover').getAttribute('data-state')).toBe('pending'),
    );
    expect(ipcMock().api.ruleHandovers.mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(screen.queryByRole('button', { name: /Retry failed handovers/ })).toBeNull();
  });

  it('shows a channel claimed but never driven as owed, naming the claimant', async () => {
    // The case that looked reassuring and was not: a rule targets the channel, so it
    // has an "owner", but it has never written to it — the runtime deliberately does
    // not write under a rule that owns the channel, so nothing is protecting it.
    ipcMock().api.ruleHandovers.mockResolvedValue([
      handoverFixture({
        state: 'awaiting_owner',
        attempts: 1,
        claimant: 'rule-gpu-fan-b',
        claimed_ticks: 7,
        first_error: undefined,
        last_error: undefined,
        reason:
          'fan.mock.0/fan.speed_percent is claimed by rule `rule-gpu-fan-b` (last status: fallback) but not driven by it',
      }),
    ]);

    renderWithProviders(<Diagnostics onOpenSettings={() => undefined} />);

    const row = await screen.findByTestId('handover');
    expect(row.getAttribute('data-state')).toBe('awaiting_owner');
    const badge = within(row).getByText('waiting for its new owner');
    expect(badge.className).toContain('badge--warn');
    expect(badge.className).not.toContain('badge--ok');
    // The claimant and how long it has been waiting are on the screen, not just in
    // the log: "someone else has it" is only reassuring if you can see who.
    const note = within(row).getByTestId('handover-claimant');
    expect(note.textContent).toContain('rule-gpu-fan-b');
    expect(note.textContent).toContain('7');
    expect(note.textContent).toMatch(/not known to be protected/i);
    expect(within(row).getByTestId('handover-state-note').textContent).toMatch(
      /not known to be protected/i,
    );
    // It is owed work, so it is listed as such and counted.
    expect(screen.getByText('Channel handovers (1 owed)')).toBeTruthy();
    // A waiting handover has not failed, so the retry control is not offered for it.
    expect(screen.queryByRole('button', { name: /Retry failed handovers/ })).toBeNull();
    expect(screen.queryByTestId('handover-resolved')).toBeNull();
  });

  it('shows a responsibility recovered from a previous session as owed and unchecked', async () => {
    ipcMock().api.ruleHandovers.mockResolvedValue([
      handoverFixture({
        state: 'needs_verification',
        attempts: 3,
        first_error: 'the device refused the fail-safe duty',
        reason:
          'rule `gone-rule` was deleted (recovered from the previous session; the device has not been checked yet)',
      }),
    ]);

    renderWithProviders(<Diagnostics onOpenSettings={() => undefined} />);

    const row = await screen.findByTestId('handover');
    expect(row.getAttribute('data-state')).toBe('needs_verification');
    const badge = within(row).getByText('not verified yet');
    expect(badge.className).toContain('badge--warn');
    expect(badge.className).not.toContain('badge--ok');
    // What it *is*: work owed by a previous session of the app, not a live claim about
    // the device, and not something the runtime will replay.
    expect(within(row).getByTestId('handover-state-note').textContent).toMatch(
      /previous session/i,
    );
    expect(within(row).getByTestId('handover-state-note').textContent).toMatch(
      /never replays/i,
    );
    // The cause and the attempt history survive the restart.
    expect(within(row).getByTestId('handover-cause').textContent).toContain(
      'the device refused the fail-safe duty',
    );
    expect(within(row).getByTestId('handover-attempts').textContent).toContain('3 attempts');
    expect(screen.getByText('Channel handovers (1 owed)')).toBeTruthy();
    expect(screen.queryByTestId('handover-resolved')).toBeNull();
  });

  it('says so when unresolved responsibility could not be recorded at all', async () => {
    // The engine reports this through `automation_stats`; the panel must not imply the
    // list it is showing would survive a restart.
    ipcMock().api.automationStats.mockResolvedValue({
      rules: 1,
      enabled_rules: 1,
      ticks: 12,
      evaluations: 12,
      writes: 2,
      skipped: 0,
      fallbacks: 0,
      failures: 0,
      last_tick_ms: 1_700_000_000_000,
      persistence_error:
        'could not record unresolved control responsibility: permission denied',
    });
    ipcMock().api.ruleHandovers.mockResolvedValue([handoverFixture({ state: 'pending' })]);

    renderWithProviders(<Diagnostics onOpenSettings={() => undefined} />);

    const notice = await screen.findByTestId('handover-persistence-error');
    expect(notice.textContent).toContain('permission denied');
    expect(notice.textContent).toMatch(/may not be recovered after a restart/i);
    expect(notice.querySelector('.inline-notice--error')).not.toBeNull();
  });

  it('files a resolved handover as history, never as owed work', async () => {
    ipcMock().api.ruleHandovers.mockResolvedValue([
      handoverFixture({
        state: 'confirmed',
        attempts: 1,
        confirmed_value: 70,
        queued_at_ms: 1_700_000_000_000,
      }),
      handoverFixture({
        state: 'superseded',
        superseded_by: 'rule-gpu-fan-b',
        queued_at_ms: 1_700_000_100_000,
      }),
    ]);

    renderWithProviders(<Diagnostics onOpenSettings={() => undefined} />);

    const history = await screen.findByRole('list', { name: 'Resolved handovers' });
    expect(screen.getByText('Resolved handovers (2)')).toBeTruthy();
    // Nothing is owed, so nothing is presented as unpaid work.
    expect(screen.queryByTestId('handover')).toBeNull();
    expect(screen.getByTestId('handover-none-owed')).toBeTruthy();
    expect(screen.getByText('Channel handovers')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /Retry failed handovers/ })).toBeNull();

    const rows = within(history).getAllByRole('listitem');
    expect(rows).toHaveLength(2);
    const confirmed = rowContaining(rows, 'confirmed');
    expect(confirmed.textContent).toContain('70 %');
    const superseded = rowContaining(rows, 'superseded');
    expect(superseded.textContent).toContain('rule-gpu-fan-b');
    expect(superseded.textContent).toContain('nothing was written');
  });
});

describe('the compatibility adjustments on the Automation screen', () => {
  beforeEach(() => {
    resetIpcMock();
    ipcMock().api.listRules.mockResolvedValue([ruleFixture()]);
  });

  it('shows the substitution, the file it came from, and that the file was not modified', async () => {
    ipcMock().api.ruleCompatibilityNotes.mockResolvedValue([ruleFileNoteFixture()]);

    renderWithProviders(<Automation />);

    expect(await screen.findByText('Compatibility adjustments (1)')).toBeTruthy();
    expect(screen.getByTestId('compatibility-rule').textContent).toBe('rule-gpu-load');
    expect(screen.getByTestId('compatibility-field').textContent).toBe(
      'fallback.on_sensor_missing',
    );
    expect(screen.getByTestId('compatibility-original').textContent).toBe('release');
    expect(screen.getByTestId('compatibility-effective').textContent).toBe(
      'safe_default (fail-safe duty 70 %)',
    );
    expect(screen.getByTestId('compatibility-message').textContent).toBe(NOTE_MESSAGE);
    expect(screen.getByTestId('compatibility-hint').textContent).toBe(NOTE_HINT);
    expect(screen.getByTestId('compatibility-path').textContent).toBe(NOTE_PATH);

    // The file on disk is untouched, and the machine is still protected.
    expect(screen.getByText(/were left exactly as they are/)).toBeTruthy();
    const untouched = screen.getByTestId('compatibility-file-untouched').textContent ?? '';
    expect(untouched).toMatch(/still protected/i);
    expect(untouched).toMatch(/fail-safe duty/i);
  });

  it('says so plainly when no file needed a compatibility adjustment', async () => {
    renderWithProviders(<Automation />);

    expect(await screen.findByText('Compatibility adjustments')).toBeTruthy();
    expect(
      screen.getByText(/No rule file needs a compatibility adjustment/),
    ).toBeTruthy();
    expect(screen.queryByTestId('rule-compatibility-note')).toBeNull();
  });
});

// The rule side of the same contract. The rendered screens above are what prove
// behaviour; these stay because they document the frozen wording.
describe('the unconfirmed rule status', () => {
  it('is labelled as unconfirmed, not as applied or as an error', () => {
    expect(ruleStatusLabel('unconfirmed')).toBe('Unconfirmed');
    expect(ruleStatusLabel('applied')).toBe('Applied');
  });

  it('explains what is unknown and what happens next', () => {
    const hint = ruleStatusHint('unconfirmed');
    expect(hint).toBeDefined();
    expect(hint).toMatch(/did not confirm/i);
    expect(hint).toMatch(/retry/i);
    expect(hint).toMatch(/fail-safe/i);
  });

  it('is painted as a warning on the rule card, never as success', async () => {
    resetIpcMock();
    ipcMock().api.listRules.mockResolvedValue([ruleFixture({ id: FAN_ID, name: 'Fan A' })]);
    ipcMock().api.ruleOutcomes.mockResolvedValue([
      {
        rule_id: FAN_ID,
        name: 'Fan A',
        enabled: true,
        status: 'unconfirmed',
        source: `${GPU_ID} · temperature.core`,
        target: `${GPU_ID} · fan.speed_percent`,
        message: 'The device did not confirm the write.',
        evaluations: 3,
        writes: 3,
        skipped: 0,
        fallbacks: 0,
        at_ms: 1_700_000_000_000,
      },
    ]);

    renderWithProviders(<Automation />);

    const badge = await screen.findByTitle(/did not confirm the value/i);
    expect(badge.textContent).toBe('Unconfirmed');
    expect(badge.className).toContain('badge--warn');
    expect(badge.className).not.toContain('badge--ok');
  });
});

describe('the release fallback action', () => {
  it('is never offered as an editor choice', () => {
    // `isSimpleFallback` is what feeds the `<select>`; a value it cannot return is
    // a value the user cannot choose.
    const actions: FallbackAction[] = ['hold', 'safe_default', 'release', { fixed: { percent: 40 } }];
    const choices = actions.map((action) => isSimpleFallback(action));
    expect(choices).not.toContain('release');
  });

  it('shows a legacy release rule as the safe default the runtime runs', () => {
    // The backend substitutes `safe_default` when it loads such a file, so the form
    // must show the same thing rather than a value that cannot be saved.
    expect(isSimpleFallback('release')).toBe('safe_default');
  });

  it('leaves the other actions alone', () => {
    expect(isSimpleFallback('hold')).toBe('hold');
    expect(isSimpleFallback('safe_default')).toBe('safe_default');
    expect(isSimpleFallback({ fixed: { percent: 55 } })).toBe('fixed');
  });
});
