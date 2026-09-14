import { useMemo } from 'react';
import { useRuntime } from '../hooks/useRuntime';
import { useHistory } from '../hooks/useHistory';
import { usePolled } from '../hooks/usePoll';
import { api } from '../lib/ipc';
import {
  allTemperatures,
  fanRows,
  findReading,
  hottestTemperature,
  loadCapability,
  monitoringOnlyReasons,
  numericReading,
  powerCapability,
  primarySensor,
  unavailableReadings,
} from '../lib/devices';
import {
  EMPTY,
  deviceStatusLabel,
  deviceTypeLabel,
  formatRelative,
  formatValue,
  unavailableReasonLabel,
} from '../lib/format';
import { EmptyState, InlineNotice, Panel, Spinner } from '../components/primitives';
import { ReadingValue, UnavailableBadge } from '../components/status';
import { Sparkline } from '../components/Sparkline';
import type { Route } from '../components/Sidebar';
import type { Capability, DeviceView, RuntimeSnapshot } from '../types';

const SPARK_POINTS = 120;

export function Overview({
  onOpenDevice,
  onNavigate,
}: {
  onOpenDevice: (deviceId: string) => void;
  onNavigate: (route: Route) => void;
}) {
  const { snapshot, loading, error, refresh, demoMode } = useRuntime();
  const tick = snapshot?.generated_at_ms;

  const outcomes = usePolled(() => api.ruleOutcomes(), tick, { throttleMs: 1500 });

  if (!snapshot) {
    return (
      <div className="content">
        {error ? (
          <InlineNotice tone="error" title="Could not read the runtime snapshot">
            <p>{error.message}</p>
            {error.hint ? <p className="inline-notice__hint">{error.hint}</p> : null}
            <div className="row">
              <button type="button" className="btn btn--primary" onClick={() => void refresh()}>
                Try again
              </button>
            </div>
          </InlineNotice>
        ) : (
          <Panel title="Overview">
            <Spinner label={loading ? 'Reading hardware…' : 'Waiting for the first snapshot…'} />
          </Panel>
        )}
      </div>
    );
  }

  return (
    <OverviewBody
      snapshot={snapshot}
      demoMode={demoMode}
      outcomeCount={outcomes.data?.length ?? 0}
      appliedCount={(outcomes.data ?? []).filter((outcome) => outcome.status === 'applied').length}
      onOpenDevice={onOpenDevice}
      onNavigate={onNavigate}
    />
  );
}

function OverviewBody({
  snapshot,
  demoMode,
  outcomeCount,
  appliedCount,
  onOpenDevice,
  onNavigate,
}: {
  snapshot: RuntimeSnapshot;
  demoMode: boolean;
  outcomeCount: number;
  appliedCount: number;
  onOpenDevice: (deviceId: string) => void;
  onNavigate: (route: Route) => void;
}) {
  const tick = snapshot.generated_at_ms;
  const cpu = snapshot.devices.find((view) => view.device.type === 'cpu');
  const gpu = snapshot.devices.find((view) => view.device.type === 'gpu');
  const storage = snapshot.devices.find((view) => view.device.type === 'storage');
  const cooling = useMemo(
    () => snapshot.devices.filter((view) => view.device.type === 'fan' || view.device.type === 'pump'),
    [snapshot.devices],
  );

  const monitoringReasons = monitoringOnlyReasons(snapshot);
  const hottest = hottestTemperature(snapshot);
  const fans = fanRows(snapshot);
  const unavailable = snapshot.devices
    .map((view) => ({ view, items: unavailableReadings(view) }))
    .filter((entry) => entry.items.length > 0);

  const coolingPrimary = useMemo(() => {
    const rows = fans
      .map((row) => row.rpm)
      .filter((value): value is number => value !== undefined);
    return rows.length > 0 ? Math.max(...rows) : undefined;
  }, [fans]);

  const coolingTarget = cooling[0] ?? gpu;
  const coolingDuty = coolingTarget
    ? (fanRows(snapshot).find((row) => row.view.device.id === coolingTarget.device.id)?.duty ?? undefined)
    : undefined;

  return (
    <div className="content">
      {demoMode ? (
        <InlineNotice tone="warn" title="Demo data">
          <p>
            This window is not running inside Tauri, so a small built-in demo dataset is shown
            instead of real hardware. Launch the desktop app for live readings.
          </p>
        </InlineNotice>
      ) : null}

      {!snapshot.has_controllable_hardware ? (
        <div className="banner banner--warn" role="status">
          <div className="banner__body">
            <p className="banner__title">Monitoring only — no controllable cooling detected</p>
            <p>
              Temperatures are read normally, but nothing can change fan or pump speed yet. Rules
              will report <strong>fallback</strong> or <strong>idle</strong> until a controllable
              device appears.
            </p>
            <ul className="stack--tight">
              {monitoringReasons.map((reason) => (
                <li key={reason} className="small">
                  • {reason}
                </li>
              ))}
            </ul>
            <div className="row">
              <button
                type="button"
                className="btn btn--sm"
                onClick={() => onNavigate({ name: 'settings' })}
              >
                Open settings
              </button>
              <button
                type="button"
                className="btn btn--sm"
                onClick={() => onNavigate({ name: 'devices' })}
              >
                Review devices
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {snapshot.devices.length === 0 ? (
        <EmptyState
          title="No hardware has been discovered yet"
          body="OpenHardwareOS found no devices to read. Nothing is faked: an empty list means no adapter reported a device."
          steps={[
            'Install and run LibreHardwareMonitor (its web server exposes motherboard, CPU and storage sensors), then rescan.',
            'Run the app as Administrator so the NVIDIA/vendor drivers allow sensor and fan access.',
            'Enable Mock hardware in Settings → Experimental features for a fully simulated demo, or start the mock protocol device.',
          ]}
          actions={
            <button type="button" className="btn" onClick={() => onNavigate({ name: 'settings' })}>
              Open settings
            </button>
          }
        />
      ) : (
        <section className="grid grid--cards" aria-label="Hardware summary">
          <SensorCard
            title="Processor"
            fallbackSubtitle="No CPU detected"
            view={cpu}
            tick={tick}
            onOpen={onOpenDevice}
          />
          <SensorCard
            title="Graphics"
            fallbackSubtitle="No GPU detected"
            view={gpu}
            tick={tick}
            onOpen={onOpenDevice}
          />
          <SensorCard
            title="Storage"
            fallbackSubtitle="No storage device detected"
            view={storage}
            tick={tick}
            onOpen={onOpenDevice}
          />
          <CoolingCard
            primary={coolingPrimary}
            duty={coolingDuty}
            fans={fans}
            coolingDeviceCount={cooling.length}
            target={coolingTarget}
            tick={tick}
            onOpen={onOpenDevice}
            onNavigate={onNavigate}
          />
        </section>
      )}

      <Panel title="System" subtitle="Aggregate state of the whole machine">
        <div className="grid grid--two">
          <div className="stack stack--tight">
            <p className="field__label">Hottest temperature</p>
            {hottest ? (
              <>
                <p className="metric__value" style={{ fontSize: 'var(--step-3)' }}>
                  {formatValue(hottest.value, 'celsius')}
                </p>
                <p className="small muted">
                  {hottest.view.device.name} · {hottest.capability.name}
                </p>
              </>
            ) : (
              <>
                <p className="metric__value" style={{ fontSize: 'var(--step-3)' }}>
                  {EMPTY}
                </p>
                <p className="small muted">
                  {allTemperatures(snapshot).length === 0
                    ? 'No temperature sensors are reporting. Run as Administrator or install LibreHardwareMonitor.'
                    : 'No temperature reading available.'}
                </p>
              </>
            )}
          </div>

          <div className="stack stack--tight">
            <p className="field__label">Fans &amp; pumps</p>
            {fans.length === 0 ? (
              <p className="small muted">
                {EMPTY} No fan or pump RPM is reported by the enabled adapters.
              </p>
            ) : (
              <ul className="stack--tight">
                {fans.map((row) => (
                  <li key={row.view.device.id} className="row row--between">
                    <span className="small">{row.view.device.name}</span>
                    <span className="row">
                      <ReadingValue value={row.rpm} unit="rpm" />
                      {row.duty !== undefined ? (
                        <ReadingValue value={row.duty} unit="percent" className="small muted" />
                      ) : null}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </div>

          <div className="stack stack--tight">
            <p className="field__label">Automation</p>
            <p className="metric__value" style={{ fontSize: 'var(--step-3)' }}>
              {appliedCount}
              <span className="metric__unit"> / {outcomeCount} applied</span>
            </p>
            <p className="small muted">
              {snapshot.settings.automation_enabled
                ? 'Automation is enabled.'
                : 'Automation is switched off in Settings.'}
              {snapshot.settings.dry_run ? ' Dry run is on: writes are simulated.' : ''}
            </p>
            <div className="row">
              <button
                type="button"
                className="btn btn--sm"
                onClick={() => onNavigate({ name: 'automation' })}
              >
                Manage rules
              </button>
            </div>
          </div>

          <div className="stack stack--tight">
            <p className="field__label">Polling</p>
            <p className="small">
              Every {snapshot.settings.polling_interval_ms} ms · last poll{' '}
              {snapshot.stats.last_poll_duration_ms} ms · {snapshot.stats.last_poll_errors} error(s)
            </p>
            <p className="small muted">
              {snapshot.stats.poll_cycles.toLocaleString('en-US')} poll cycles ·{' '}
              {snapshot.stats.discovery_cycles.toLocaleString('en-US')} discovery cycles · snapshot{' '}
              {formatRelative(snapshot.generated_at_ms)}
            </p>
            <p className="small muted">
              {snapshot.stats.writes_applied} writes applied ·{' '}
              {snapshot.stats.writes_rejected} rejected · {snapshot.stats.safety_interventions}{' '}
              safety intervention(s)
            </p>
          </div>
        </div>
      </Panel>

      <Panel
        title="Readings the hardware does not provide"
        subtitle="Shown as text, never as a zero"
      >
        {unavailable.length === 0 ? (
          <p className="muted">Every declared capability reported a value in the last poll.</p>
        ) : (
          <div className="stack">
            {unavailable.map((entry) => (
              <div key={entry.view.device.id} className="stack stack--tight">
                <p className="strong small">
                  {entry.view.device.name}{' '}
                  <span className="dim">
                    ({entry.view.device.id} · {deviceTypeLabel(entry.view.device.type)})
                  </span>
                </p>
                <ul className="stack--tight">
                  {entry.items.map((item) => (
                    <li key={item.capability.id} className="row">
                      <UnavailableBadge
                        reason={item.reading.status === 'unavailable' ? item.reading.reason : 'unknown'}
                      />
                      <span className="small">{item.capability.name}</span>
                      <span className="tiny dim">
                        {item.reading.status === 'unavailable'
                          ? unavailableReasonLabel(item.reading.reason)
                          : ''}
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        )}
      </Panel>
    </div>
  );
}

function SensorCard({
  title,
  view,
  tick,
  onOpen,
  fallbackSubtitle,
}: {
  title: string;
  view: DeviceView | undefined;
  tick: number;
  onOpen: (deviceId: string) => void;
  fallbackSubtitle: string;
}) {
  const primary = view ? primarySensor(view) : undefined;
  const history = useHistory(view?.device.id, primary?.id, SPARK_POINTS, tick);

  if (!view) {
    return (
      <article className="metric" aria-label={`${title}: not detected`}>
        <header className="metric__head">
          <div>
            <p className="metric__name">{title}</p>
            <p className="metric__vendor">{fallbackSubtitle}</p>
          </div>
        </header>
        <p className="metric__value">{EMPTY}</p>
        <p className="small muted">
          No device of this kind was reported by the enabled adapters. Nothing is substituted for
          the missing data.
        </p>
      </article>
    );
  }

  const primaryValue = primary ? numericReading(view, primary.id) : undefined;
  const primaryReading = primary
    ? view.state?.readings.find((reading) => reading.capability === primary.id)
    : undefined;
  const load = loadCapability(view);
  const power = powerCapability(view);
  const loadReading = load ? findReading(view, load.id) : undefined;
  const powerReading = power ? findReading(view, power.id) : undefined;

  return (
    <article className="metric" aria-label={`${title}: ${view.device.name}`}>
      <header className="metric__head">
        <div>
          <p className="metric__name">{view.device.name}</p>
          <p className="metric__vendor">
            {deviceTypeLabel(view.device.type)} · {view.device.vendor} · {view.adapter}
          </p>
        </div>
        <span className="spacer" />
        <button
          type="button"
          className="btn btn--sm btn--ghost"
          onClick={() => onOpen(view.device.id)}
          aria-label={`Open details for ${view.device.name}`}
        >
          Details
        </button>
      </header>

      <div className="metric__primary">
        {primary ? (
          <>
            <ReadingValue
              value={primaryValue}
              unit={primary.unit}
              unavailableReason={
                primaryReading?.status === 'unavailable' ? primaryReading.reason : undefined
              }
            />
            <span className="tiny dim">{primary.name}</span>
          </>
        ) : (
          <span className="metric__value">{EMPTY}</span>
        )}
      </div>

      <div className="metric__rows">
        <div className="metric__stat">
          <span className="metric__stat-label">Load</span>
          <span className="metric__stat-value">
            <ReadingValue
              value={loadReading?.status === 'ok' ? loadReading.value : undefined}
              unit={load?.unit ?? 'percent'}
              unavailableReason={
                loadReading?.status === 'unavailable' ? loadReading.reason : load ? undefined : 'unsupported'
              }
            />
          </span>
        </div>
        <div className="metric__stat">
          <span className="metric__stat-label">Power</span>
          <span className="metric__stat-value">
            <ReadingValue
              value={powerReading?.status === 'ok' ? powerReading.value : undefined}
              unit={power?.unit ?? 'watt'}
              unavailableReason={
                powerReading?.status === 'unavailable' ? powerReading.reason : power ? undefined : 'unsupported'
              }
            />
          </span>
        </div>
        <div className="metric__stat">
          <span className="metric__stat-label">Status</span>
          <span className="metric__stat-value small">{deviceStatusLabel(view.status)}</span>
        </div>
      </div>

      {primary ? (
        <Sparkline
          samples={history.samples}
          unit={primary.unit}
          label={`${view.device.name} ${primary.name}`}
        />
      ) : null}
      <p className="tiny dim">
        {primary
          ? `${primary.name} · last ${SPARK_POINTS} samples${history.error ? ' (history unavailable)' : ''}`
          : 'No readable sensor is declared for this device.'}
      </p>
    </article>
  );
}

function CoolingCard({
  primary,
  duty,
  fans,
  coolingDeviceCount,
  target,
  tick,
  onOpen,
  onNavigate,
}: {
  primary: number | undefined;
  duty: number | undefined;
  fans: ReturnType<typeof fanRows>;
  /** Fan and pump devices; GPU fans are counted separately below. */
  coolingDeviceCount: number;
  target: DeviceView | undefined;
  tick: number;
  onOpen: (deviceId: string) => void;
  onNavigate: (route: Route) => void;
}) {
  const dutyCapability: Capability | undefined = target
    ? target.device.capabilities.find((capability) => capability.writable)
    : undefined;
  const history = useHistory(target?.device.id, dutyCapability?.id, SPARK_POINTS, tick);

  return (
    <article className="metric" aria-label="Cooling">
      <header className="metric__head">
        <div>
          <p className="metric__name">Cooling</p>
          <p className="metric__vendor">
            {coolingDeviceCount} fan/pump device{coolingDeviceCount === 1 ? '' : 's'} reporting ·{' '}
            {fans.length} speed reading{fans.length === 1 ? '' : 's'}
          </p>
        </div>
        <span className="spacer" />
        {target ? (
          <button
            type="button"
            className="btn btn--sm btn--ghost"
            onClick={() => onOpen(target.device.id)}
            aria-label={`Open details for ${target.device.name}`}
          >
            Details
          </button>
        ) : null}
      </header>

      <div className="metric__primary">
        {primary !== undefined ? (
          <>
            <span className="metric__value">{formatValue(primary, 'rpm')}</span>
          </>
        ) : (
          <ReadingValue value={undefined} unit="rpm" unavailableReason="unsupported" />
        )}
      </div>
      <p className="tiny dim">Highest fan speed across all cooling devices</p>

      <div className="metric__rows">
        <div className="metric__stat">
          <span className="metric__stat-label">Fan devices</span>
          <span className="metric__stat-value">{fans.length}</span>
        </div>
        <div className="metric__stat">
          <span className="metric__stat-label">Reference duty</span>
          <span className="metric__stat-value">
            <ReadingValue value={duty} unit="percent" unavailableReason={duty === undefined ? 'unsupported' : undefined} />
          </span>
        </div>
      </div>

      {target && dutyCapability ? (
        <Sparkline samples={history.samples} unit={dutyCapability.unit} label={`${target.device.name} duty`} />
      ) : (
        <p className="small muted">
          {EMPTY} No writable cooling device is available, so there is no duty cycle to plot.
        </p>
      )}

      <div className="row">
        <button
          type="button"
          className="btn btn--sm"
          onClick={() => onNavigate({ name: 'automation' })}
        >
          Cooling rules
        </button>
        <button
          type="button"
          className="btn btn--sm btn--ghost"
          onClick={() => onNavigate({ name: 'devices' })}
        >
          All devices
        </button>
      </div>
    </article>
  );
}
