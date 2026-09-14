import { useEffect, useMemo, useState } from 'react';
import { useRuntime } from '../hooks/useRuntime';
import { usePolled } from '../hooks/usePoll';
import { api } from '../lib/ipc';
import { adapterStateLabel, unavailableReasonLabel } from '../lib/format';
import { applyTheme, useTheme } from '../lib/theme';
import {
  Badge,
  Field,
  InlineNotice,
  Panel,
  SectionHead,
  Segmented,
  Spinner,
  Switch,
} from '../components/primitives';
import { AdapterStateBadge } from '../components/status';
import type { LogLevel, Settings, Theme } from '../types';

const LOG_LEVELS: LogLevel[] = ['error', 'warn', 'info', 'debug', 'trace'];

export function SettingsScreen() {
  const { snapshot, perform, refresh, demoMode } = useRuntime();
  const [draft, setDraft] = useState<Settings | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const adapters = usePolled(() => api.getAdapters(), snapshot?.generated_at_ms, {
    throttleMs: 10_000,
  });

  useEffect(() => {
    if (snapshot?.settings && draft === null) setDraft(snapshot.settings);
  }, [snapshot, draft]);

  // The theme previews live from the draft; leaving the screen restores the
  // saved theme so an abandoned change does not stick.
  const theme = draft?.theme ?? 'system';
  useTheme(theme);
  const savedTheme = snapshot?.settings.theme ?? 'system';
  useEffect(() => () => applyTheme(savedTheme), [savedTheme]);

  const dirty = useMemo(() => {
    if (!snapshot || !draft) return false;
    return JSON.stringify(snapshot.settings) !== JSON.stringify(draft);
  }, [snapshot, draft]);

  if (!snapshot || !draft) {
    return (
      <div className="content">
        <Panel title="Settings">
          <Spinner label="Loading settings…" />
        </Panel>
      </div>
    );
  }

  const update = (patch: Partial<Settings>) => {
    setDraft({ ...draft, ...patch });
    setError(null);
  };

  const updateSafety = (patch: Partial<Settings['safety']>) => {
    setDraft({ ...draft, safety: { ...draft.safety, ...patch } });
    setError(null);
  };

  const validate = (): string | null => {
    if (draft.polling_interval_ms < 100) return 'The polling interval must be at least 100 ms.';
    if (draft.discovery_interval_ms < 1000) return 'The discovery interval must be at least 1000 ms.';
    if (draft.history_points < 10) return 'Keep at least 10 history points.';
    if (draft.safety.min_duty_percent < 0 || draft.safety.min_duty_percent > 100) {
      return 'The minimum fan duty must be between 0 and 100 %.';
    }
    if (draft.safety.emergency_temp_c <= 0) return 'The emergency temperature must be above 0 °C.';
    if (draft.safety.max_write_delta_percent < 0) return 'The ramp limit cannot be negative.';
    return null;
  };

  const save = async () => {
    const problem = validate();
    if (problem) {
      setError(problem);
      return;
    }
    setBusy(true);
    await perform('Save settings', () => api.updateSettings(draft), {
      success: 'Settings saved. The runtime applies them immediately.',
    });
    setBusy(false);
  };

  const toggleAdapter = async (adapter: string, enabled: boolean) => {
    const next = await perform(
      `${enabled ? 'Enable' : 'Disable'} adapter “${adapter}”`,
      () => api.setAdapterEnabled(adapter, enabled),
      { success: `Adapter “${adapter}” ${enabled ? 'enabled' : 'disabled'}.`, refresh: false },
    );
    if (next) setDraft(next);
    adapters.reload();
    await refresh();
  };

  const numberField = (
    label: string,
    value: number,
    onChange: (next: number) => void,
    options: { min?: number; max?: number; step?: number; hint?: string; unit?: string } = {},
  ) => {
    const id = `settings-${label.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`;
    return (
      <Field
        label={options.unit ? `${label} (${options.unit})` : label}
        htmlFor={id}
        hint={options.hint}
      >
        <input
          id={id}
          className="input input--number"
          type="number"
          min={options.min}
          max={options.max}
          step={options.step ?? 1}
          value={Number.isFinite(value) ? value : 0}
          onChange={(event) => {
            const next = Number(event.target.value);
            onChange(Number.isFinite(next) ? next : (options.min ?? 0));
          }}
        />
      </Field>
    );
  };

  return (
    <div className="content">
      {demoMode ? (
        <InlineNotice tone="warn" title="Changes are not persisted">
          <p>
            This window runs outside Tauri, so settings are kept in memory by the demo backend and
            reset when you reload the page.
          </p>
        </InlineNotice>
      ) : null}

      <Panel
        title="Settings"
        subtitle={dirty ? 'Unsaved changes' : 'Everything is saved'}
        actions={
          <>
            {dirty ? <Badge tone="warn">unsaved</Badge> : <Badge tone="ok">saved</Badge>}
            <button
              type="button"
              className="btn btn--sm btn--ghost"
              onClick={() => setDraft(snapshot.settings)}
              disabled={!dirty || busy}
            >
              Discard
            </button>
            <button
              type="button"
              className="btn btn--sm btn--primary"
              onClick={() => void save()}
              disabled={!dirty || busy}
            >
              {busy ? 'Saving…' : 'Save changes'}
            </button>
          </>
        }
      >
        {error ? (
          <InlineNotice tone="error" title="Check these values">
            <p>{error}</p>
          </InlineNotice>
        ) : null}

        <SectionHead title="Monitoring" hint="How often the runtime reads sensors and looks for new devices" />
        <div className="form-grid">
          {numberField(
            'Polling interval',
            draft.polling_interval_ms,
            (next) => update({ polling_interval_ms: next }),
            { min: 100, step: 100, unit: 'ms', hint: 'Minimum 100 ms. Short intervals cost CPU time.' },
          )}
          {numberField(
            'Discovery interval',
            draft.discovery_interval_ms,
            (next) => update({ discovery_interval_ms: next }),
            { min: 1000, step: 1000, unit: 'ms', hint: 'How often adapters are asked for new devices.' },
          )}
          {numberField(
            'History points',
            draft.history_points,
            (next) => update({ history_points: next }),
            { min: 10, step: 10, unit: 'samples', hint: 'Per capability, kept in memory only.' },
          )}
          <Field label="Log level" htmlFor="settings-log-level" hint="Detail of the diagnostic log">
            <select
              id="settings-log-level"
              className="select"
              value={draft.log_level}
              onChange={(event) => update({ log_level: event.target.value as LogLevel })}
            >
              {LOG_LEVELS.map((level) => (
                <option key={level} value={level}>
                  {level}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Theme" labelAs="span" hint="System follows the Windows light/dark setting">
            <Segmented<Theme>
              legend="Theme"
              value={draft.theme}
              onChange={(next) => update({ theme: next })}
              options={[
                { value: 'system', label: 'System' },
                { value: 'dark', label: 'Dark' },
                { value: 'light', label: 'Light' },
              ]}
            />
          </Field>
        </div>

        <SectionHead title="Startup and window" />
        <div className="form-grid">
          <Switch
            id="settings-start-with-windows"
            checked={draft.start_with_windows}
            onChange={(next) => update({ start_with_windows: next })}
            label="Start with Windows"
            hint="Launch monitoring when you sign in."
          />
          <Switch
            id="settings-minimize-to-tray"
            checked={draft.minimize_to_tray}
            onChange={(next) => update({ minimize_to_tray: next })}
            label="Minimize to tray"
            hint="Minimising hides the window; monitoring continues and the tray icon brings it back."
          />
          <Switch
            id="settings-start-minimized"
            checked={draft.start_minimized}
            onChange={(next) => update({ start_minimized: next })}
            label="Start minimized"
            hint="Open straight to the tray icon."
          />
          <Switch
            id="settings-close-to-tray"
            checked={draft.close_to_tray}
            onChange={(next) => update({ close_to_tray: next })}
            label="Close to tray"
            hint="Closing the window keeps the runtime alive; quit from the tray menu instead."
          />
        </div>

        <SectionHead title="Features" />
        <div className="form-grid">
          <Switch
            id="settings-experimental"
            checked={draft.experimental_features}
            onChange={(next) => update({ experimental_features: next })}
            label="Experimental features"
            hint="Unlocks the simulated hardware panel and other unfinished work."
          />
          <Switch
            id="settings-developer-mode"
            checked={draft.developer_mode}
            onChange={(next) => update({ developer_mode: next })}
            label="Developer mode"
            hint="Shows the Diagnostics screen with the audit log and live events."
          />
          <Switch
            id="settings-mock-device"
            checked={draft.enable_mock_protocol_device}
            onChange={(next) => update({ enable_mock_protocol_device: next })}
            label="Enable mock protocol device"
            hint="Adds a simulated fan/pump so the control loop can be tried safely."
          />
          <Switch
            id="settings-automation"
            checked={draft.automation_enabled}
            onChange={(next) => update({ automation_enabled: next })}
            label="Automation enabled"
            hint="Master switch for every cooling rule."
          />
          <Switch
            id="settings-dry-run"
            checked={draft.dry_run}
            onChange={(next) => update({ dry_run: next })}
            label="Dry run"
            hint="Evaluate rules and log writes, but never touch the hardware."
          />
        </div>
      </Panel>

      <Panel
        title="Safety policy"
        subtitle="These limits protect your hardware"
        actions={<Badge tone={draft.safety.enabled ? 'ok' : 'warn'}>{draft.safety.enabled ? 'enabled' : 'off'}</Badge>}
      >
        <InlineNotice tone={draft.safety.enabled ? 'ok' : 'warn'} title="These protect your hardware">
          <p>
            {draft.safety.enabled
              ? 'Every write — manual, automated or scheduled — passes through this policy. It clamps duty cycles, ramps changes slowly and takes over at the emergency temperature, even if a rule misbehaves. Turning it off removes those guarantees.'
              : 'The safety policy is switched off: rules and manual writes go straight to the hardware. Only do this on hardware you can afford to damage.'}
          </p>
        </InlineNotice>

        <div className="form-grid" style={{ marginTop: 'var(--space-4)' }}>
          <Switch
            id="safety-enabled"
            checked={draft.safety.enabled}
            onChange={(next) => updateSafety({ enabled: next })}
            label="Enforce the safety policy"
            hint="Strongly recommended. Required for pre-set risk action on most fan controllers."
          />
          <Switch
            id="safety-min-duty"
            checked={draft.safety.require_min_duty}
            onChange={(next) => updateSafety({ require_min_duty: next })}
            label="Require a minimum duty"
            hint="Never let a fan stop entirely: some pumps stall below their minimum."
          />
          {numberField(
            'Minimum fan duty',
            draft.safety.min_duty_percent,
            (next) => updateSafety({ min_duty_percent: next }),
            { min: 0, max: 100, unit: '%', hint: 'No fan is ever driven below this.' },
          )}
          {numberField(
            'Minimum pump duty',
            draft.safety.pump_min_duty_percent,
            (next) => updateSafety({ pump_min_duty_percent: next }),
            { min: 0, max: 100, unit: '%', hint: 'Pumps need a higher floor than fans to keep liquid moving.' },
          )}
          {numberField(
            'Emergency temperature',
            draft.safety.emergency_temp_c,
            (next) => updateSafety({ emergency_temp_c: next }),
            { min: 30, max: 120, unit: '°C', hint: 'Above this, the emergency override takes over.' },
          )}
          {numberField(
            'Emergency duty',
            draft.safety.emergency_duty_percent,
            (next) => updateSafety({ emergency_duty_percent: next }),
            { min: 0, max: 100, unit: '%', hint: 'Duty forced while the emergency temperature is exceeded.' },
          )}
          {numberField(
            'Fail-safe duty',
            draft.safety.fail_safe_duty_percent,
            (next) => updateSafety({ fail_safe_duty_percent: next }),
            { min: 0, max: 100, unit: '%', hint: 'Used when a rule asks for the safe default.' },
          )}
          {numberField(
            'Ramp limit per write',
            draft.safety.max_write_delta_percent,
            (next) => updateSafety({ max_write_delta_percent: next }),
            { min: 0, max: 100, unit: '%', hint: 'Largest duty change a single write may make; keeps fans quiet and bearings happy.' },
          )}
          {numberField(
            'Sensor stale after',
            draft.safety.sensor_stale_after_s,
            (next) => updateSafety({ sensor_stale_after_s: next }),
            { min: 1, unit: 's', hint: 'A reading older than this is treated as missing.' },
          )}
          <Switch
            id="safety-emergency-override"
            checked={draft.safety.emergency_override_enabled}
            onChange={(next) => updateSafety({ emergency_override_enabled: next })}
            label="Allow the emergency override"
            hint="Lets the policy ignore curve output and force cooling at the emergency temperature."
          />
          <Switch
            id="safety-relinquish"
            checked={draft.safety.relinquish_on_exit}
            onChange={(next) => updateSafety({ relinquish_on_exit: next })}
            label="Relinquish control on exit"
            hint="Hand fans back to the BIOS or vendor tool when OpenHardwareOS closes."
          />
        </div>
      </Panel>

      <Panel
        title="Adapters"
        subtitle="Disable an adapter to stop it loading and polling"
        actions={
          <button type="button" className="btn btn--sm" onClick={() => adapters.reload()}>
            Refresh status
          </button>
        }
      >
        {adapters.error ? (
          <InlineNotice tone="error" title="Could not read adapter status">
            <p>{adapters.error.message}</p>
          </InlineNotice>
        ) : adapters.data === null ? (
          <Spinner label="Reading adapters…" />
        ) : adapters.data.length === 0 ? (
          <p className="muted">
            No adapter is installed. Enable the mock protocol device above to try the app without
            hardware.
          </p>
        ) : (
          <div className="table-wrap">
            <table className="table">
              <caption>Adapter state, permissions and how many devices each one found.</caption>
              <thead>
                <tr>
                  <th scope="col">Adapter</th>
                  <th scope="col">State</th>
                  <th scope="col">Controls cooling</th>
                  <th scope="col">Admin</th>
                  <th scope="col">Devices</th>
                  <th scope="col">Enabled</th>
                </tr>
              </thead>
              <tbody>
                {adapters.data.map((adapter) => {
                  const disabled = draft.disabled_adapters.includes(adapter.info.id);
                  return (
                    <tr key={adapter.info.id}>
                      <td>
                        <p className="strong">{adapter.info.name}</p>
                        <p className="tiny dim">
                          {adapter.info.id} · v{adapter.info.version} · {adapter.info.namespace}
                        </p>
                        <p className="tiny dim">{adapter.info.description}</p>
                        {adapter.status.reason ? (
                          <p className="tiny dim">
                            {unavailableReasonLabel(adapter.status.reason)}
                            {adapter.status.detail ? ` — ${adapter.status.detail}` : ''}
                          </p>
                        ) : null}
                      </td>
                      <td>
                        <AdapterStateBadge state={adapter.status.state} />
                        <p className="tiny dim">{adapterStateLabel(adapter.status.state)}</p>
                      </td>
                      <td>{adapter.info.capabilities.can_control_cooling ? 'Yes' : 'No'}</td>
                      <td>
                        {adapter.info.capabilities.write_requires_admin || adapter.info.requires_admin
                          ? 'Required for writes'
                          : 'Not required'}
                      </td>
                      <td>{adapter.status.device_count}</td>
                      <td>
                        <label className="switch">
                          <input
                            type="checkbox"
                            checked={!disabled}
                            onChange={(event) => void toggleAdapter(adapter.info.id, event.target.checked)}
                            aria-label={`${disabled ? 'Enable' : 'Disable'} adapter ${adapter.info.name}`}
                          />
                          <span className="switch__track" aria-hidden="true">
                            <span className="switch__thumb" />
                          </span>
                          <span className="switch__label">{disabled ? 'Disabled' : 'Enabled'}</span>
                        </label>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </Panel>

      <Panel title="Files" subtitle="Where configuration and logs live">
        <div className="row">
          <button
            type="button"
            className="btn"
            onClick={() =>
              void perform('Open config folder', () => api.openConfigDir(), {
                refresh: false,
                success: 'Config folder opened.',
              })
            }
          >
            Open config folder
          </button>
          <button
            type="button"
            className="btn"
            onClick={() =>
              void perform('Open log folder', () => api.openLogDir(), {
                refresh: false,
                success: 'Log folder opened.',
              })
            }
          >
            Open log folder
          </button>
          <span className="small dim">
            {demoMode
              ? 'Not available in the browser demo.'
              : 'Opens the folder in Windows Explorer.'}
          </span>
        </div>
      </Panel>
    </div>
  );
}
