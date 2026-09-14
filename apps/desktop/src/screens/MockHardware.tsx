import { useEffect, useState } from 'react';
import { useRuntime } from '../hooks/useRuntime';
import { usePolled } from '../hooks/usePoll';
import { api } from '../lib/ipc';
import { formatClock, formatValue } from '../lib/format';
import { Badge, Field, InlineNotice, Panel, SectionHead, Spinner, Switch } from '../components/primitives';
import type { MockProfile, MockStatus } from '../types';

/**
 * The simulated-hardware panel. This is how the closed loop
 * `GPU temperature → rule → fan RPM` is demonstrated without real hardware.
 */
export function MockHardware({ onOpenSettings }: { onOpenSettings: () => void }) {
  const { snapshot, perform, refresh } = useRuntime();
  const status = usePolled(() => api.mockStatus(), snapshot?.generated_at_ms, { throttleMs: 800 });
  const [local, setLocal] = useState<MockStatus | null>(null);

  useEffect(() => {
    if (status.data) setLocal(status.data);
  }, [status.data]);

  const mock = local;

  if (!snapshot) {
    return (
      <div className="content">
        <Panel title="Simulated hardware">
          <Spinner label="Waiting for the runtime…" />
        </Panel>
      </div>
    );
  }

  if (status.error) {
    return (
      <div className="content">
        <InlineNotice tone="error" title="The simulated hardware did not answer">
          <p>{status.error.message}</p>
          {status.error.hint ? <p className="inline-notice__hint">{status.error.hint}</p> : null}
          <button type="button" className="btn btn--sm" onClick={() => status.reload()}>
            Retry
          </button>
        </InlineNotice>
      </div>
    );
  }

  if (!mock) {
    return (
      <div className="content">
        <Panel title="Simulated hardware">
          <p className="muted">
            The mock protocol device is not running, so there is nothing to simulate.
          </p>
          <div className="row">
            <button type="button" className="btn btn--primary" onClick={onOpenSettings}>
              Enable it in settings
            </button>
          </div>
        </Panel>
      </div>
    );
  }

  const apply = async (label: string, action: () => Promise<MockStatus | null>) => {
    const next = await perform(label, action, { refresh: false });
    if (next) setLocal(next);
    await refresh();
  };

  return (
    <div className="content">
      <Panel
        className="sim-panel"
        title={
          <>
            <span className="sim-ribbon">Simulated hardware</span> <span>Mock protocol device</span>
          </>
        }
        subtitle="Everything on this screen is generated in software. No real fan, pump or sensor is touched."
        actions={<Badge tone="warn">{mock.clock === 'manual' ? 'manual clock' : 'wall clock'}</Badge>}
      >
        <InlineNotice tone="warn" title="SIMULATED HARDWARE">
          <p>
            Values below are synthetic. They are useful for demonstrating the control loop
            <strong> GPU temperature → rule → fan RPM</strong> and for trying rules before they touch
            real hardware. Simulated writes are marked as such wherever a write report is shown.
          </p>
        </InlineNotice>

        <div className="form-grid" style={{ marginTop: 'var(--space-4)' }}>
          <div className="stack stack--tight">
            <p className="field__label">Live simulation state</p>
            <dl className="kv">
              <dt className="kv__key">Simulated clock</dt>
              <dd className="kv__value">
                {formatClock(Date.now() - (mock.sim_ms % 86_400_000))} · sim {Math.round(mock.sim_ms / 1000)} s
              </dd>
              <dt className="kv__key">GPU</dt>
              <dd className="kv__value">
                {formatValue(mock.gpu_temp_c, 'celsius')} · load {formatValue(mock.gpu_load, 'percent')} ·
                fan {formatValue(mock.gpu_fan_duty, 'percent')} ({formatValue(mock.gpu_fan_rpm, 'rpm')})
              </dd>
              <dt className="kv__key">CPU</dt>
              <dd className="kv__value">
                {formatValue(mock.cpu_temp_c, 'celsius')} · load {formatValue(mock.cpu_load, 'percent')}
              </dd>
              <dt className="kv__key">SSD</dt>
              <dd className="kv__value">{formatValue(mock.ssd_temp_c, 'celsius')}</dd>
              <dt className="kv__key">Ambient</dt>
              <dd className="kv__value">{formatValue(mock.ambient_c, 'celsius')}</dd>
              <dt className="kv__key">Chassis fans</dt>
              <dd className="kv__value">
                {mock.fan_rpms.length === 0
                  ? '—'
                  : mock.fan_rpms
                      .map((rpm, index) => `${rpm} RPM (${mock.fan_duties[index] ?? 0}%)`)
                      .join(' · ')}
              </dd>
              <dt className="kv__key">Pump</dt>
              <dd className="kv__value">
                {mock.pump_duties.length === 0 ? '—' : mock.pump_duties.map((duty) => `${duty} %`).join(' · ')}
              </dd>
              <dt className="kv__key">Noise</dt>
              <dd className="kv__value">{formatValue(mock.noise, 'percent')}</dd>
            </dl>
          </div>

          <div className="stack">
            <SectionHead title="Load" hint="Drive the simulated GPU and CPU" />
            <label className="field__label" htmlFor="mock-gpu-load">
              GPU load
            </label>
            <div className="slider">
              <input
                id="mock-gpu-load"
                type="range"
                min={0}
                max={100}
                step={1}
                value={mock.gpu_load}
                onChange={(event) => setLocal({ ...mock, gpu_load: Number(event.target.value) })}
                onPointerUp={(event) =>
                  void apply('Set GPU load', () =>
                    api.mockSetLoad(Number((event.target as HTMLInputElement).value), mock.cpu_load),
                  )
                }
                onKeyUp={(event) =>
                  void apply('Set GPU load', () =>
                    api.mockSetLoad(Number((event.target as HTMLInputElement).value), mock.cpu_load),
                  )
                }
              />
              <span className="slider__value">{formatValue(mock.gpu_load, 'percent')}</span>
            </div>
            <label className="field__label" htmlFor="mock-cpu-load">
              CPU load
            </label>
            <div className="slider">
              <input
                id="mock-cpu-load"
                type="range"
                min={0}
                max={100}
                step={1}
                value={mock.cpu_load}
                onChange={(event) => setLocal({ ...mock, cpu_load: Number(event.target.value) })}
                onPointerUp={(event) =>
                  void apply('Set CPU load', () =>
                    api.mockSetLoad(mock.gpu_load, Number((event.target as HTMLInputElement).value)),
                  )
                }
                onKeyUp={(event) =>
                  void apply('Set CPU load', () =>
                    api.mockSetLoad(mock.gpu_load, Number((event.target as HTMLInputElement).value)),
                  )
                }
              />
              <span className="slider__value">{formatValue(mock.cpu_load, 'percent')}</span>
            </div>

            <SectionHead title="Profiles" hint="Canned scenarios" />
            <div className="row">
              {(['idle', 'gaming', 'wave'] as MockProfile[]).map((profile) => (
                <button
                  key={profile}
                  type="button"
                  className="btn btn--sm"
                  onClick={() => void apply(`Apply “${profile}” profile`, () => api.mockApplyProfile(profile))}
                >
                  {profile === 'idle' ? 'Idle' : profile === 'gaming' ? 'Gaming' : 'Wave'}
                </button>
              ))}
            </div>

            <Field
              label="Ambient temperature"
              htmlFor="mock-ambient"
              hint="Raises every simulated temperature"
            >
              <div className="slider">
                <input
                  id="mock-ambient"
                  type="range"
                  min={10}
                  max={45}
                  step={0.5}
                  value={mock.ambient_c}
                  onChange={(event) => setLocal({ ...mock, ambient_c: Number(event.target.value) })}
                  onPointerUp={(event) =>
                    void apply('Set ambient temperature', () =>
                      api.mockSetAmbient(Number((event.target as HTMLInputElement).value)),
                    )
                  }
                />
                <span className="slider__value">{formatValue(mock.ambient_c, 'celsius')}</span>
              </div>
            </Field>

            <Field
              label="Force GPU temperature"
              htmlFor="mock-force-gpu"
              hint="Pins the GPU sensor to a value so a rule can be watched reacting to it"
            >
              <ForceTemperature
                onApply={(celsius) =>
                  void apply('Force GPU temperature', () => api.mockForceGpuTemperature(celsius))
                }
              />
            </Field>
          </div>
        </div>
      </Panel>

      <Panel
        className="sim-panel"
        title="Fault injection"
        subtitle="Prove the fallbacks and the safety policy work"
      >
        <div className="form-grid">
          <Switch
            id="mock-fail-writes"
            checked={mock.faults.fail_all_writes}
            onChange={(next) =>
              void apply('Toggle write failures', () =>
                api.mockSetFaults(next, mock.faults.unavailable_readings.length > 0, mock.faults.unplug_devices.length > 0),
              )
            }
            label="Fail all writes"
            hint="Every write is rejected: rules should fall back to their write-failure action."
          />
          <Switch
            id="mock-disconnect-gpu-temp"
            checked={mock.faults.unavailable_readings.length > 0}
            onChange={(next) =>
              void apply('Toggle GPU temperature fault', () =>
                api.mockSetFaults(mock.faults.fail_all_writes, next, mock.faults.unplug_devices.length > 0),
              )
            }
            label="Disconnect GPU temperature"
            hint="The GPU sensor times out: the rule must use its sensor-missing fallback."
          />
          <Switch
            id="mock-unplug-fan"
            checked={mock.faults.unplug_devices.length > 0}
            onChange={(next) =>
              void apply('Toggle unplugged fan', () =>
                api.mockSetFaults(mock.faults.fail_all_writes, mock.faults.unavailable_readings.length > 0, next),
              )
            }
            label="Unplug a fan"
            hint="One fan disappears: the device goes offline and the cooling summary loses it."
          />
        </div>

        {mock.faults.fail_all_writes ||
        mock.faults.unavailable_readings.length > 0 ||
        mock.faults.unplug_devices.length > 0 ? (
          <InlineNotice tone="warn" title="Faults are currently injected">
            <ul className="stack--tight">
              {mock.faults.fail_all_writes ? <li>• All writes are being rejected.</li> : null}
              {mock.faults.unavailable_readings.map(([device, capability, reason]) => (
                <li key={`${device}-${capability}`}>
                  • {device} · {capability} is unavailable ({reason})
                </li>
              ))}
              {mock.faults.unplug_devices.map((device) => (
                <li key={device}>• {device} is unplugged</li>
              ))}
            </ul>
            <button
              type="button"
              className="btn btn--sm"
              onClick={() =>
                void apply('Clear all faults', () => api.mockSetFaults(false, false, false))
              }
            >
              Clear all faults
            </button>
          </InlineNotice>
        ) : (
          <p className="small muted">No fault is injected. The simulation behaves normally.</p>
        )}
      </Panel>
    </div>
  );
}

function ForceTemperature({ onApply }: { onApply: (celsius: number) => void }) {
  const [value, setValue] = useState('75');
  return (
    <div className="row">
      <input
        id="mock-force-gpu"
        className="input input--number"
        type="number"
        min={0}
        max={120}
        step={1}
        value={value}
        onChange={(event) => setValue(event.target.value)}
      />
      <button
        type="button"
        className="btn btn--sm"
        onClick={() => {
          const celsius = Number(value);
          if (Number.isFinite(celsius)) onApply(celsius);
        }}
      >
        Force {value} °C
      </button>
    </div>
  );
}
