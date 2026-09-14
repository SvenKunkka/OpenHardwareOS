/**
 * The "Only when…" editor in the Automation form.
 *
 * These tests drive the real form through the mocked `src/lib/ipc.ts` and look at
 * what reaches `save_rule`, so they cover serialisation, not an internal helper:
 * an ungated rule must carry no `when` key at all, and a gated rule must round
 * trip unchanged.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, screen, waitFor } from '@testing-library/react';

vi.mock('../lib/ipc', async () => (await import('./ipcMock')).createIpcMock());

import { Automation } from '../screens/Automation';
import { renderWithProviders } from './helpers';
import { ipcMock, resetIpcMock } from './ipcMock';
import { GPU_ID, capabilityIndexFixture, ruleFixture } from './fixtures';
import type { Rule } from '../types';

const LOAD_SOURCE = `${GPU_ID}|load.gpu`;

/** Open the form for a brand-new rule and wait for it to exist. */
async function openNewRuleForm(): Promise<void> {
  renderWithProviders(<Automation />);
  const buttons = await screen.findAllByRole('button', { name: 'New rule' });
  fireEvent.click(buttons[0] as HTMLElement);
  await screen.findByLabelText('Rule name');
}

async function save(): Promise<void> {
  const buttons = screen.getAllByRole('button', { name: 'Check & save' });
  fireEvent.click(buttons[0] as HTMLElement);
}

function savedRule(): Rule {
  const calls = ipcMock().api.saveRule.mock.calls;
  expect(calls).toHaveLength(1);
  return calls[0]?.[0] as Rule;
}

describe('the “only when…” gate in the rule form', () => {
  beforeEach(() => {
    resetIpcMock();
  });

  it('stores no `when` at all while the gate is off', async () => {
    await openNewRuleForm();

    // The switch is off, and the form says so in plain language.
    const toggle = screen.getByLabelText(/Gate this rule on a reading/) as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    expect(screen.getByTestId('gate-summary').textContent).toContain('No condition');

    await save();
    await waitFor(() => expect(ipcMock().api.saveRule).toHaveBeenCalledTimes(1));

    const rule = savedRule();
    expect('when' in rule).toBe(false);
    expect(Object.hasOwn(rule, 'when')).toBe(false);
    // What Tauri actually serialises: the key must not appear as `null` either.
    expect(JSON.parse(JSON.stringify(rule))).not.toHaveProperty('when');
  });

  it('serialises op, threshold and otherwise when the gate is on', async () => {
    await openNewRuleForm();

    fireEvent.click(screen.getByLabelText(/Gate this rule on a reading/));
    fireEvent.change(await screen.findByLabelText('Condition sensor'), {
      target: { value: LOAD_SOURCE },
    });
    fireEvent.change(screen.getByLabelText('Comparison'), { target: { value: 'lte' } });
    fireEvent.change(screen.getByLabelText('Threshold (%)'), { target: { value: '55' } });
    fireEvent.change(screen.getByLabelText('Otherwise'), { target: { value: 'fixed' } });
    fireEvent.change(screen.getByLabelText('Otherwise duty (%)'), { target: { value: '40' } });

    // The threshold follows the capability's unit and step.
    expect((screen.getByLabelText('Threshold (%)') as HTMLInputElement).step).toBe('1');

    expect(screen.getByTestId('gate-summary').textContent).toBe(
      'Only while gpu.mock.0/load.gpu ≤ 55 %; otherwise drive 40 % instead.',
    );

    await save();
    await waitFor(() => expect(ipcMock().api.saveRule).toHaveBeenCalledTimes(1));

    expect(savedRule().when).toEqual({
      source: { device: GPU_ID, capability: 'load.gpu' },
      op: 'lte',
      value: 55,
      otherwise: { fixed: { percent: 40 } },
    });
  });

  it('offers a temperature threshold in tenths of a degree and names the fail-safe duty', async () => {
    await openNewRuleForm();

    fireEvent.click(screen.getByLabelText(/Gate this rule on a reading/));
    fireEvent.change(await screen.findByLabelText('Condition sensor'), {
      target: { value: `${GPU_ID}|temperature.core` },
    });
    fireEvent.change(screen.getByLabelText('Comparison'), { target: { value: 'gt' } });
    fireEvent.change(screen.getByLabelText('Threshold (°C)'), { target: { value: '62.5' } });

    expect((screen.getByLabelText('Threshold (°C)') as HTMLInputElement).step).toBe('0.1');
    // The default "otherwise" is the runtime's fail-safe duty, shown with its value.
    expect(
      screen.getByRole('option', { name: /Safe default — the fail-safe duty \(70 %\)/ }),
    ).toBeTruthy();
    expect(screen.getByTestId('gate-summary').textContent).toBe(
      'Only while gpu.mock.0/temperature.core > 62.5 °C; otherwise fall back to the safe default (70 %).',
    );

    await save();
    await waitFor(() => expect(ipcMock().api.saveRule).toHaveBeenCalledTimes(1));

    expect(savedRule().when).toEqual({
      source: { device: GPU_ID, capability: 'temperature.core' },
      op: 'gt',
      value: 62.5,
      otherwise: 'safe_default',
    });
  });

  it('refuses to save a gate with no sensor or a non-numeric threshold', async () => {
    await openNewRuleForm();

    fireEvent.click(screen.getByLabelText(/Gate this rule on a reading/));
    fireEvent.change(await screen.findByLabelText('Threshold (%)'), { target: { value: '' } });
    fireEvent.change(screen.getByLabelText('Condition sensor'), { target: { value: '' } });

    await save();

    expect(await screen.findByText(/Choose the sensor the “only when…” condition reads/)).toBeTruthy();
    expect(screen.getByText(/The condition threshold must be a number/)).toBeTruthy();
    expect(ipcMock().api.checkRule).not.toHaveBeenCalled();
    expect(ipcMock().api.saveRule).not.toHaveBeenCalled();
  });
});

describe('loading an existing rule back into the form', () => {
  beforeEach(() => {
    resetIpcMock();
  });

  it('shows a stored `when` unchanged and saves it back unchanged', async () => {
    const stored = ruleFixture({
      when: {
        source: { device: GPU_ID, capability: 'load.gpu' },
        op: 'gte',
        value: 60,
        otherwise: 'safe_default',
      },
    });
    ipcMock().api.listRules.mockResolvedValue([stored]);

    renderWithProviders(<Automation />);
    fireEvent.click(await screen.findByRole('button', { name: 'Edit rule Fan A' }));
    await screen.findByLabelText('Rule name');

    expect((screen.getByLabelText(/Gate this rule on a reading/) as HTMLInputElement).checked).toBe(true);
    expect((screen.getByLabelText('Condition sensor') as HTMLSelectElement).value).toBe(LOAD_SOURCE);
    expect((screen.getByLabelText('Comparison') as HTMLSelectElement).value).toBe('gte');
    expect((screen.getByLabelText('Threshold (%)') as HTMLInputElement).value).toBe('60');
    expect((screen.getByLabelText('Otherwise') as HTMLSelectElement).value).toBe('safe_default');
    expect(screen.getByTestId('gate-summary').textContent).toBe(
      'Only while gpu.mock.0/load.gpu ≥ 60 %; otherwise fall back to the safe default (70 %).',
    );

    await save();
    await waitFor(() => expect(ipcMock().api.saveRule).toHaveBeenCalledTimes(1));

    expect(savedRule().when).toEqual(stored.when);
  });

  it('leaves a rule without a `when` without one', async () => {
    ipcMock().api.listRules.mockResolvedValue([ruleFixture()]);

    renderWithProviders(<Automation />);
    fireEvent.click(await screen.findByRole('button', { name: 'Edit rule Fan A' }));
    await screen.findByLabelText('Rule name');

    expect((screen.getByLabelText(/Gate this rule on a reading/) as HTMLInputElement).checked).toBe(false);

    await save();
    await waitFor(() => expect(ipcMock().api.saveRule).toHaveBeenCalledTimes(1));

    expect('when' in savedRule()).toBe(false);
  });

  it('keeps the rule source list and the condition source list in the same order', async () => {
    await openNewRuleForm();

    const index = capabilityIndexFixture();
    const expected = index.sources.map(
      (ref) => `${ref.device_name} · ${ref.capability.name} (${ref.capability.unit})`,
    );

    fireEvent.click(screen.getByLabelText(/Gate this rule on a reading/));
    const ruleSource = screen.getByLabelText('Sensor') as HTMLSelectElement;
    const gateSource = (await screen.findByLabelText('Condition sensor')) as HTMLSelectElement;

    const optionsOf = (select: HTMLSelectElement): string[] =>
      Array.from(select.querySelectorAll('option'))
        .slice(1)
        .map((option) => (option.textContent ?? '').split(' — now')[0] as string);

    expect(optionsOf(gateSource)).toEqual(expected);
    expect(optionsOf(ruleSource)).toEqual(expected);
  });
});
