/**
 * How the Automation screen reports the two new backend behaviours:
 *
 * - a `gated` rule is shown as "standing down" — a calm, labelled state, never an
 *   error;
 * - `rule_conflicts()` is surfaced with the owner, the blocked rule, the target
 *   and the backend's own resolution sentence;
 * - a refused save shows the backend's message verbatim (never a generic one).
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, screen, waitFor } from '@testing-library/react';

vi.mock('../lib/ipc', async () => (await import('./ipcMock')).createIpcMock());

import { Automation } from '../screens/Automation';
import { renderWithProviders } from './helpers';
import { MockBackendError, ipcMock, resetIpcMock } from './ipcMock';
import { GPU_ID, conflictFixture, outcomeFixture, ruleFixture } from './fixtures';

const CONFLICT_MESSAGE =
  '`Fan B` cannot be enabled: `Fan A` already drives fan.mock.0/fan.speed_percent. Two enabled rules may not share an output — disable or retarget one of them.';

describe('a gated rule on its card', () => {
  beforeEach(() => {
    resetIpcMock();
  });

  it('renders "standing down" as a distinct, non-error state', async () => {
    ipcMock().api.listRules.mockResolvedValue([
      ruleFixture({
        when: {
          source: { device: GPU_ID, capability: 'load.gpu' },
          op: 'gt',
          value: 60,
          otherwise: 'safe_default',
        },
      }),
    ]);
    ipcMock().api.ruleOutcomes.mockResolvedValue([
      outcomeFixture({ status: 'gated', message: 'Condition not met; standing down.' }),
    ]);

    renderWithProviders(<Automation />);

    // The label says what is happening, and its title says it is not a fault.
    const badge = await screen.findByTitle(/This is normal, not a fault/);
    expect(badge.textContent).toBe('Standing down');
    expect(badge.className).not.toContain('danger');
    expect(badge.className).not.toContain('error');

    // The card also spells out the gate and the duty that replaced the curve.
    expect(screen.getByText('when load.gpu > 60 %')).toBeTruthy();
    const standingDown = screen.getByTestId('rule-standing-down');
    expect(standingDown.textContent).toContain('The condition is not met');
    expect(standingDown.textContent).toContain('fall back to the safe default (70 %)');
  });

  it('does not confuse standing down with an error status', async () => {
    ipcMock().api.listRules.mockResolvedValue([ruleFixture()]);
    ipcMock().api.ruleOutcomes.mockResolvedValue([
      outcomeFixture({ status: 'error', message: 'write failed: device offline' }),
    ]);

    renderWithProviders(<Automation />);

    const badge = await screen.findByTitle(/The last write failed/);
    expect(badge.textContent).toBe('Error');
    expect(badge.className).toContain('badge--danger');
    expect(screen.queryByTestId('rule-standing-down')).toBeNull();
  });
});

describe('automation that another process is running', () => {
  beforeEach(() => {
    resetIpcMock();
  });

  /**
   * A background service can own the hardware channels. The engine then deliberately
   * does not start, and without saying so the screen looks like automation that is
   * simply broken — the reason matters more than the fact.
   */
  it('says which process holds the channels and what this window will still do', async () => {
    ipcMock().api.automationStats.mockResolvedValue({
      rules: 1,
      enabled_rules: 1,
      ticks: 0,
      evaluations: 0,
      writes: 0,
      skipped: 0,
      fallbacks: 0,
      failures: 0,
      last_tick_ms: 0,
      blocked_by:
        'another process (pid 4242) is running the rules and owns the hardware channels; automation is not started here',
    });

    renderWithProviders(<Automation />);

    const notice = await screen.findByTestId('automation-blocked');
    expect(notice.textContent).toContain('pid 4242');
    expect(screen.getByText(/Another process is running the rules/)).toBeTruthy();
    expect(screen.getByText(/will not write while the other process owns the channels/)).toBeTruthy();
  });

  it('says nothing when this process is the one running the rules', async () => {
    renderWithProviders(<Automation />);
    // The default fixture has no `blocked_by`, and no notice may appear for it.
    await waitFor(() => expect(ipcMock().api.automationStats).toHaveBeenCalled());
    expect(screen.queryByTestId('automation-blocked')).toBeNull();
  });
});

describe('the output-conflict section', () => {
  beforeEach(() => {
    resetIpcMock();
  });

  it('lists the owner, the blocked rule, the target and the resolution', async () => {
    ipcMock().api.listRules.mockResolvedValue([
      ruleFixture({ id: 'rule-gpu-fan-a', name: 'Fan A' }),
      ruleFixture({ id: 'rule-gpu-fan-b', name: 'Fan B', enabled: false }),
    ]);
    ipcMock().api.ruleConflicts.mockResolvedValue([conflictFixture()]);

    renderWithProviders(<Automation />);

    expect(await screen.findByText('Output conflicts (1)')).toBeTruthy();
    expect(screen.getByTestId('conflict-owner').textContent).toBe('Fan A');
    expect(screen.getByTestId('conflict-blocked').textContent).toBe('Fan B');
    expect(screen.getByTestId('conflict-target').textContent).toBe(
      'fan.mock.0/fan.speed_percent',
    );
    // The backend's sentence is shown verbatim.
    expect(screen.getByTestId('conflict-resolution').textContent).toBe(
      'resolved at load time: this rule is disabled',
    );
    expect(screen.getByText('Two enabled rules want the same output')).toBeTruthy();
  });

  it('says so plainly when no output is contested', async () => {
    ipcMock().api.listRules.mockResolvedValue([ruleFixture()]);

    renderWithProviders(<Automation />);

    expect(await screen.findByText('Output conflicts')).toBeTruthy();
    expect(screen.getByText(/No two enabled rules share an output/)).toBeTruthy();
  });
});

describe('a rule the runtime refuses', () => {
  beforeEach(() => {
    resetIpcMock();
  });

  it('shows the backend message and hint verbatim for an automation_error', async () => {
    ipcMock().api.saveRule.mockRejectedValue(
      new MockBackendError('automation_error', CONFLICT_MESSAGE, 'See the log for details.'),
    );

    renderWithProviders(<Automation />);
    const newRuleButtons = await screen.findAllByRole('button', { name: 'New rule' });
    fireEvent.click(newRuleButtons[0] as HTMLElement);
    await screen.findByLabelText('Rule name');

    fireEvent.click(screen.getAllByRole('button', { name: 'Check & save' })[0] as HTMLElement);

    const message = await screen.findByTestId('save-error-message');
    expect(message.textContent).toBe(CONFLICT_MESSAGE);
    expect(screen.getByText('The runtime refused this rule')).toBeTruthy();
    expect(screen.getByText('See the log for details.')).toBeTruthy();
    // Not the generic wording.
    expect(screen.queryByText('The rule could not be saved')).toBeNull();
  });

  it('warns above the form when check_rule already reports the clash', async () => {
    ipcMock().api.checkRule.mockResolvedValue({
      errors: [
        'Fan A already drives fan.mock.0/fan.speed_percent; disable it or point this rule at another output',
      ],
      warnings: [],
    });

    renderWithProviders(<Automation />);
    const newRuleButtons = await screen.findAllByRole('button', { name: 'New rule' });
    fireEvent.click(newRuleButtons[0] as HTMLElement);
    await screen.findByLabelText('Rule name');

    fireEvent.click(screen.getAllByRole('button', { name: 'Check & save' })[0] as HTMLElement);

    expect(
      await screen.findByText('Another enabled rule already drives this output'),
    ).toBeTruthy();
    expect(
      screen.getByText(
        /Fan A already drives fan\.mock\.0\/fan\.speed_percent; disable it or point this rule at another output/,
      ),
    ).toBeTruthy();
    expect(ipcMock().api.saveRule).not.toHaveBeenCalled();
  });

  it('refreshes the conflict list after a rule is enabled and after one is deleted', async () => {
    ipcMock().api.listRules.mockResolvedValue([
      ruleFixture({ id: 'rule-gpu-fan-b', name: 'Fan B', enabled: false }),
    ]);
    ipcMock().api.ruleConflicts.mockResolvedValue([conflictFixture()]);

    renderWithProviders(<Automation />);
    await screen.findByText('Output conflicts (1)');
    const afterLoad = ipcMock().api.ruleConflicts.mock.calls.length;

    fireEvent.click(screen.getByLabelText('Enable rule Fan B'));

    await waitFor(() => expect(ipcMock().api.setRuleEnabled).toHaveBeenCalledWith('rule-gpu-fan-b', true));
    await waitFor(() =>
      expect(ipcMock().api.ruleConflicts.mock.calls.length).toBeGreaterThan(afterLoad),
    );

    const afterEnable = ipcMock().api.ruleConflicts.mock.calls.length;
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    fireEvent.click(screen.getByRole('button', { name: 'Confirm delete' }));

    await waitFor(() => expect(ipcMock().api.deleteRule).toHaveBeenCalledWith('rule-gpu-fan-b'));
    await waitFor(() =>
      expect(ipcMock().api.ruleConflicts.mock.calls.length).toBeGreaterThan(afterEnable),
    );
  });

  it('reports a refused enable with the backend wording, not a generic failure', async () => {
    ipcMock().api.listRules.mockResolvedValue([
      ruleFixture({ id: 'rule-gpu-fan-b', name: 'Fan B', enabled: false }),
    ]);
    ipcMock().api.setRuleEnabled.mockRejectedValue(
      new MockBackendError('automation_error', CONFLICT_MESSAGE, 'See the log for details.'),
    );

    renderWithProviders(<Automation />);
    fireEvent.click(await screen.findByLabelText('Enable rule Fan B'));

    expect((await screen.findByTestId('action-error-message')).textContent).toBe(CONFLICT_MESSAGE);
  });
});
