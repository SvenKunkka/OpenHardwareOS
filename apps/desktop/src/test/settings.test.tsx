/**
 * Settings must call the backend with the payload the backend expects. The IPC
 * module is mocked, so these tests assert the arguments — no Tauri, no hardware.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, screen, waitFor } from '@testing-library/react';

vi.mock('../lib/ipc', async () => (await import('./ipcMock')).createIpcMock());

import { SettingsScreen } from '../screens/Settings';
import { renderWithProviders } from './helpers';
import { ipcMock, resetIpcMock } from './ipcMock';
import { adapterFixture, snapshotFixture } from './fixtures';
import type { Settings } from '../types';

function settingsSent(): Settings {
  const calls = ipcMock().api.updateSettings.mock.calls;
  expect(calls).toHaveLength(1);
  return calls[0]?.[0] as Settings;
}

describe('Settings', () => {
  beforeEach(() => {
    resetIpcMock();
  });

  it('saves the whole settings object with the toggled flag', async () => {
    renderWithProviders(<SettingsScreen />);
    await screen.findByText('Everything is saved');

    fireEvent.click(screen.getByLabelText(/Start with Windows/));
    fireEvent.click(screen.getByRole('button', { name: 'Save changes' }));

    await waitFor(() => expect(ipcMock().api.updateSettings).toHaveBeenCalledTimes(1));

    const sent = settingsSent();
    expect(sent.start_with_windows).toBe(true);
    expect(sent).toEqual({ ...snapshotFixture().settings, start_with_windows: true });
  });

  it('keeps the safety policy intact while a feature switch is flipped', async () => {
    renderWithProviders(<SettingsScreen />);
    await screen.findByText('Everything is saved');

    fireEvent.click(screen.getByLabelText(/Dry run/));
    fireEvent.click(screen.getByLabelText(/Enforce the safety policy/));
    fireEvent.click(screen.getByRole('button', { name: 'Save changes' }));

    await waitFor(() => expect(ipcMock().api.updateSettings).toHaveBeenCalledTimes(1));

    const sent = settingsSent();
    expect(sent.dry_run).toBe(true);
    expect(sent.safety.enabled).toBe(false);
    // Untouched fields are sent as they were, not dropped.
    expect(sent.polling_interval_ms).toBe(snapshotFixture().settings.polling_interval_ms);
    expect(sent.safety.fail_safe_duty_percent).toBe(
      snapshotFixture().settings.safety.fail_safe_duty_percent,
    );
  });

  it('refuses to save an out-of-range value instead of calling the backend', async () => {
    renderWithProviders(<SettingsScreen />);
    await screen.findByText('Everything is saved');

    fireEvent.change(screen.getByLabelText(/Polling interval/), { target: { value: '10' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save changes' }));

    expect(await screen.findByText('The polling interval must be at least 100 ms.')).toBeTruthy();
    expect(ipcMock().api.updateSettings).not.toHaveBeenCalled();
  });

  it('enables and disables an adapter through set_adapter_enabled', async () => {
    ipcMock().api.getAdapters.mockResolvedValue([adapterFixture()]);

    renderWithProviders(<SettingsScreen />);
    await screen.findByText('Everything is saved');

    fireEvent.click(await screen.findByLabelText('Disable adapter Simulated hardware'));

    await waitFor(() =>
      expect(ipcMock().api.setAdapterEnabled).toHaveBeenCalledWith('mock', false),
    );
    expect(ipcMock().api.updateSettings).not.toHaveBeenCalled();
  });
});
