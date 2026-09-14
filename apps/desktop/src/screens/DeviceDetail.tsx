import { useEffect, useId, useMemo, useState } from 'react';
import { useRuntime, toNoticeError } from '../hooks/useRuntime';
import { useHistory } from '../hooks/useHistory';
import { usePolled } from '../hooks/usePoll';
import { api } from '../lib/ipc';
import { capabilityCount, primarySensor } from '../lib/devices';
import {
  EMPTY,
  capabilityKindLabel,
  deviceTypeLabel,
  formatClock,
  formatDateTime,
  formatRelative,
  formatRange,
  formatValue,
  isNumericUnit,
  transportLabel,
  unavailableReasonLabel,
} from '../lib/format';
import {
  Badge,
  EmptyState,
  Field,
  InlineNotice,
  KeyValue,
  Panel,
  Spinner,
} from '../components/primitives';
import { DeviceStatusBadge, ReadingValue, UnavailableBadge } from '../components/status';
import { LineChart } from '../components/LineChart';
import type {
  Capability,
  CapabilityValue,
  DeviceView,
  NoticeError,
  WriteReport,
} from '../types';

const HISTORY_POINTS = 120;

export function DeviceDetail({
  deviceId,
  onBack,
  onOpenSettings,
}: {
  deviceId: string;
  onBack: () => void;
  onOpenSettings: () => void;
}) {
  const { snapshot, events, refresh } = useRuntime();
  const view = snapshot?.devices.find((entry) => entry.device.id === deviceId);
  const tick = snapshot?.generated_at_ms ?? 0;

  const numericCapabilities = useMemo(
    () =>
      (view?.device.capabilities ?? []).filter(
        (capability) => capability.readable && isNumericUnit(capability.unit),
      ),
    [view],
  );

  const [chartCapabilityId, setChartCapabilityId] = useState<string | undefined>(undefined);
  useEffect(() => {
    if (!view) return;
    const preferred = primarySensor(view);
    setChartCapabilityId((current) => {
      if (current && numericCapabilities.some((capability) => capability.id === current)) return current;
      return preferred?.id ?? numericCapabilities[0]?.id;
    });
  }, [view, numericCapabilities]);

  const chartCapability = numericCapabilities.find((capability) => capability.id === chartCapabilityId);
  const history = useHistory(deviceId, chartCapability?.id, HISTORY_POINTS, tick);
  const audit = usePolled(() => api.listAuditLog(200), tick, { throttleMs: 3000 });

  const recentWrites = useMemo(() => {
    type Row = { key: string; at_ms: number; title: string; detail: string; tone: 'ok' | 'warn' | 'danger' };
    const rows: Row[] = [];

    for (const entry of audit.data ?? []) {
      if (entry.kind !== 'write' || !entry.report || entry.report.device_id !== deviceId) continue;
      const report = entry.report;
      rows.push({
        key: `audit-${report.at_ms}-${report.capability}-${report.status}`,
        at_ms: report.at_ms,
        title: `${report.capability_name}: ${formatValue(report.requested, 'none')} → ${
          report.applied === undefined ? 'not applied' : formatValue(report.applied, 'none')
        }`,
        detail: [
          report.status,
          report.clamped ? 'clamped by limits' : null,
          report.simulated ? 'simulated' : null,
          report.error_code ? `error ${report.error_code}` : null,
          describeOrigin(report),
          report.detail ?? null,
        ]
          .filter(Boolean)
          .join(' · '),
        tone: report.status === 'rejected' ? 'danger' : report.simulated ? 'warn' : 'ok',
      });
    }

    for (const event of events) {
      if (event.device_id !== deviceId || event.category !== 'write') continue;
      rows.push({
        key: event.key,
        at_ms: event.at_ms,
        title: event.message,
        detail: event.detail ?? '',
        tone: event.level === 'warn' ? 'warn' : 'ok',
      });
    }

    const seen = new Set<string>();
    return rows
      .sort((a, b) => b.at_ms - a.at_ms)
      .filter((row) => {
        if (seen.has(row.key)) return false;
        seen.add(row.key);
        return true;
      })
      .slice(0, 40);
  }, [audit.data, events, deviceId]);

  if (!snapshot) {
    return (
      <div className="content">
        <Panel>
          <Spinner label="Loading device…" />
        </Panel>
      </div>
    );
  }

  if (!view) {
    return (
      <div className="content">
        <EmptyState
          title="This device is no longer present"
          body={`Device “${deviceId}” is not part of the current snapshot. It may have been unplugged, hidden, or never discovered by an enabled adapter.`}
          steps={[
            'Go back to Devices and rescan the hardware.',
            'Check Settings → Adapters: a disabled adapter hides all of its devices.',
          ]}
          actions={
            <>
              <button type="button" className="btn btn--primary" onClick={onBack}>
                Back to devices
              </button>
              <button
                type="button"
                className="btn"
                onClick={() => {
                  void api.scanDevices().then(() => refresh());
                }}
              >
                Rescan hardware
              </button>
            </>
          }
        />
      </div>
    );
  }

  const counts = capabilityCount(view);
  const writeCapabilities = view.device.capabilities.filter((capability) => capability.writable);

  return (
    <div className="content">
      <div className="row">
        <button type="button" className="btn btn--sm btn--ghost" onClick={onBack}>
          ← All devices
        </button>
      </div>

      <Panel
        title={view.device.name}
        subtitle={`${deviceTypeLabel(view.device.type)} · ${view.device.vendor}${
          view.device.model && view.device.model !== view.device.name ? ` · ${view.device.model}` : ''
        }`}
        actions={
          <>
            <DeviceStatusBadge status={view.status} />
            <Badge tone={view.enabled ? 'ok' : 'neutral'}>{view.enabled ? 'Enabled' : 'Disabled'}</Badge>
          </>
        }
      >
        <KeyValue
          rows={[
            { key: 'Device id', value: <span className="mono">{view.device.id}</span> },
            { key: 'Adapter', value: view.adapter },
            { key: 'Transport', value: transportLabel(view.device.transport) },
            {
              key: 'Capabilities',
              value: `${counts.readable} readable · ${counts.writable} writable`,
            },
            { key: 'First seen', value: formatDateTime(view.first_seen_ms) },
            {
              key: 'Last seen',
              value: `${formatDateTime(view.last_seen_ms)} (${formatRelative(view.last_seen_ms)})`,
            },
            {
              key: 'State timestamp',
              value: view.state ? formatDateTime(view.state.timestamp_ms) : EMPTY,
            },
            {
              key: 'Tags',
              value:
                view.device.tags && view.device.tags.length > 0 ? (
                  <span className="pill-row">
                    {view.device.tags.map((tag) => (
                      <span className="tag" key={tag}>
                        {tag}
                      </span>
                    ))}
                  </span>
                ) : (
                  EMPTY
                ),
            },
          ]}
        />
        {view.state?.message ? (
          <p className="small muted" style={{ marginTop: 'var(--space-3)' }}>
            {view.state.message}
          </p>
        ) : null}
      </Panel>

      {writeCapabilities.length > 0 ? (
        <Panel
          title="Controls"
          subtitle="Writes are sent through the safety layer; a refusal is reported verbatim"
        >
          <div className="grid grid--two">
            {writeCapabilities.map((capability) => (
              <WriteControl
                key={capability.id}
                deviceId={view.device.id}
                capability={capability}
                current={currentValue(view, capability.id)}
                disabled={!view.enabled}
              />
            ))}
          </div>
        </Panel>
      ) : (
        <Panel title="Controls">
          <p className="muted">
            This device is read-only. None of its capabilities are writable, so OpenHardwareOS will
            never try to change it.
          </p>
        </Panel>
      )}

      <Panel
        title="History"
        subtitle={`Up to ${HISTORY_POINTS} samples, refreshed with every poll`}
        actions={
          numericCapabilities.length > 0 ? (
            <>
              <label className="visually-hidden" htmlFor="chart-capability">
                Sensor to plot
              </label>
              <select
                id="chart-capability"
                className="select"
                value={chartCapabilityId ?? ''}
                onChange={(event) => setChartCapabilityId(event.target.value)}
                style={{ maxWidth: '280px' }}
              >
                {numericCapabilities.map((capability) => (
                  <option key={capability.id} value={capability.id}>
                    {capability.name} ({capability.id})
                  </option>
                ))}
              </select>
            </>
          ) : null
        }
      >
        {!chartCapability ? (
          <p className="muted">
            {EMPTY} This device declares no numeric sensor that can be plotted.
          </p>
        ) : history.error ? (
          <InlineNotice tone="error" title="History is unavailable">
            <p>{history.error.message}</p>
            {history.error.hint ? <p className="inline-notice__hint">{history.error.hint}</p> : null}
          </InlineNotice>
        ) : history.samples.length === 0 && !history.loading ? (
          <p className="muted">
            No samples recorded yet for {chartCapability.name}. History builds up while the app runs.
          </p>
        ) : (
          <LineChart
            samples={history.samples}
            unit={chartCapability.unit}
            label={`${view.device.name} — ${chartCapability.name}`}
          />
        )}
      </Panel>

      <Panel title="Capabilities" subtitle={`${view.device.capabilities.length} declared`} flush>
        <div className="table-wrap">
          <table className="table">
            <caption>
              Every capability this device declares, with the current value or the reason it is
              unavailable.
            </caption>
            <thead>
              <tr>
                <th scope="col">Id</th>
                <th scope="col">Name</th>
                <th scope="col">Kind</th>
                <th scope="col">Unit</th>
                <th scope="col">Range</th>
                <th scope="col">Access</th>
                <th scope="col">Current</th>
                <th scope="col">Status</th>
              </tr>
            </thead>
            <tbody>
              {view.device.capabilities.map((capability) => {
                const reading = view.state?.readings.find(
                  (item) => item.capability === capability.id,
                );
                return (
                  <tr key={capability.id}>
                    <td className="table__id">{capability.id}</td>
                    <td>
                      {capability.name}
                      {capability.safety_critical ? (
                        <>
                          {' '}
                          <Badge tone="warn" plain title="A write to this capability can affect hardware safety">
                            safety critical
                          </Badge>
                        </>
                      ) : null}
                      {capability.description ? (
                        <p className="tiny dim">{capability.description}</p>
                      ) : null}
                    </td>
                    <td>{capabilityKindLabel(capability.kind)}</td>
                    <td className="mono">{capability.unit}</td>
                    <td>{formatRange(capability.min, capability.max, capability.unit)}</td>
                    <td>
                      <span className="row" style={{ gap: 'var(--space-1)' }}>
                        {capability.readable ? <Badge tone="info" plain>read</Badge> : null}
                        {capability.writable ? <Badge tone="accent" plain>write</Badge> : null}
                        {!capability.readable && !capability.writable ? (
                          <Badge plain>metadata</Badge>
                        ) : null}
                      </span>
                    </td>
                    <td>
                      {reading?.status === 'ok' ? (
                        <ReadingValue value={reading.value} unit={capability.unit} />
                      ) : (
                        <span className="dim">{EMPTY}</span>
                      )}
                    </td>
                    <td>
                      {reading?.status === 'unavailable' ? (
                        <span className="row" style={{ gap: 'var(--space-1)' }}>
                          <UnavailableBadge reason={reading.reason} />
                          <span className="tiny dim">{unavailableReasonLabel(reading.reason)}</span>
                        </span>
                      ) : reading?.status === 'ok' ? (
                        <Badge tone="ok">ok</Badge>
                      ) : (
                        <Badge plain>not reported</Badge>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </Panel>

      <Panel title="Recent writes" subtitle="This device only, newest first">
        {recentWrites.length === 0 ? (
          <p className="muted">
            Nothing has been written to {view.device.name} yet. Manual writes and rule actions both
            appear here.
          </p>
        ) : (
          <ul className="feed" aria-live="polite" aria-label="Recent writes for this device">
            {recentWrites.map((row) => (
              <li className="feed__item" key={row.key}>
                <span className="feed__time">{formatClock(row.at_ms)}</span>
                <span className={`feed__level feed__level--${row.tone === 'danger' ? 'error' : row.tone}`}>
                  {row.tone === 'danger' ? 'rejected' : row.tone === 'warn' ? 'simulated' : 'applied'}
                </span>
                <span className="feed__message">
                  {row.title}
                  {row.detail ? <span className="feed__detail"> — {row.detail}</span> : null}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Panel>

      {!view.enabled ? (
        <InlineNotice tone="warn" title="This device is disabled">
          <p>
            Reads and writes are skipped while the device is disabled. Enable it on the Devices
            screen to control it again.
          </p>
          <button type="button" className="btn btn--sm" onClick={onOpenSettings}>
            Open settings
          </button>
        </InlineNotice>
      ) : null}
    </div>
  );
}

function currentValue(view: DeviceView, capabilityId: string): CapabilityValue | undefined {
  const reading = view.state?.readings.find((item) => item.capability === capabilityId);
  return reading?.status === 'ok' ? reading.value : undefined;
}

function describeOrigin(report: WriteReport): string {
  const origin = report.origin;
  switch (origin.kind) {
    case 'manual':
      return 'manual write';
    case 'automation':
      return `rule ${origin.rule_id}`;
    case 'safety':
      return `safety: ${origin.reason}`;
    case 'startup':
      return 'startup';
    case 'shutdown':
      return 'shutdown';
    case 'api':
      return 'api';
    default:
      return 'unknown origin';
  }
}

/**
 * One writable capability: slider/number/select plus Apply, always showing what
 * was requested next to what the backend actually applied.
 */
function WriteControl({
  deviceId,
  capability,
  current,
  disabled,
}: {
  deviceId: string;
  capability: Capability;
  current: CapabilityValue | undefined;
  disabled: boolean;
}) {
  const inputId = useId();
  const isNumeric = isNumericUnit(capability.unit);
  const [draft, setDraft] = useState<CapabilityValue>(() => {
    if (typeof current === 'number') return current;
    if (typeof current === 'boolean') return current;
    if (typeof current === 'string') return current;
    if (capability.values && capability.values.length > 0) return capability.values[0] ?? '';
    if (isNumeric) return capability.min ?? 0;
    return '';
  });
  const [report, setReport] = useState<WriteReport | null>(null);
  const [failure, setFailure] = useState<NoticeError | null>(null);
  const [pending, setPending] = useState(false);

  useEffect(() => {
    if (typeof current === 'number' || typeof current === 'boolean' || typeof current === 'string') {
      setDraft(current);
    }
  }, [current]);

  const submit = async () => {
    setPending(true);
    setFailure(null);
    try {
      const result = await api.writeCapability(deviceId, capability.id, draft);
      setReport(result);
      if (result.status === 'rejected') {
        setFailure({
          code: result.error_code ?? 'rejected',
          message: result.detail ?? 'The backend rejected this write.',
          hint: 'Check the safety policy and the device state before retrying.',
        });
      }
    } catch (cause) {
      setReport(null);
      setFailure(toNoticeError(cause));
    } finally {
      setPending(false);
    }
  };

  const numeric = typeof draft === 'number' ? draft : 0;
  const rangeMin = capability.min ?? 0;
  const rangeMax = capability.max ?? 100;
  const step = capability.step ?? 1;
  const currentReading =
    current === undefined ? 'not reported' : formatValue(current, capability.unit);

  return (
    <div className="stack">
      <div className="row row--between">
        <p className="strong">
          {capability.name} <span className="dim mono tiny">{capability.id}</span>
        </p>
        {capability.safety_critical ? (
          <Badge tone="warn" title="A write here can affect hardware safety">
            safety critical
          </Badge>
        ) : null}
      </div>
      <p className="small muted">
        Current: {currentReading} · Range {formatRange(capability.min, capability.max, capability.unit)}
        {capability.step !== undefined ? ` · step ${capability.step}` : ''}
      </p>

      <Field
        label={`Requested ${capability.name}`}
        htmlFor={inputId}
        labelAs={capability.unit === 'boolean' ? 'span' : 'label'}
        hint={
          capability.description ??
          'The backend may clamp the value, apply it, or reject it — the outcome is shown below.'
        }
      >
        {capability.values && capability.values.length > 0 && !isNumeric ? (
          <select
            id={inputId}
            className="select"
            value={String(draft)}
            disabled={disabled || pending}
            onChange={(event) => setDraft(event.target.value)}
          >
            {capability.values.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        ) : capability.unit === 'boolean' ? (
          <label className="checkbox" htmlFor={inputId}>
            <input
              id={inputId}
              type="checkbox"
              checked={draft === true}
              disabled={disabled || pending}
              onChange={(event) => setDraft(event.target.checked)}
            />
            <span className="checkbox__text">
              <span className="checkbox__title">{draft === true ? 'On' : 'Off'}</span>
            </span>
          </label>
        ) : (
          <div className="slider">
            <input
              id={inputId}
              type="range"
              min={rangeMin}
              max={rangeMax}
              step={step}
              value={numeric}
              disabled={disabled || pending}
              onChange={(event) => setDraft(Number(event.target.value))}
            />
            <input
              className="input input--number"
              type="number"
              min={rangeMin}
              max={rangeMax}
              step={step}
              value={numeric}
              disabled={disabled || pending}
              aria-label={`${capability.name} value`}
              onChange={(event) => {
                const next = Number(event.target.value);
                setDraft(Number.isFinite(next) ? next : rangeMin);
              }}
            />
            <span className="slider__value">{formatValue(numeric, capability.unit)}</span>
          </div>
        )}
      </Field>

      <div className="row">
        <button
          type="button"
          className="btn btn--primary"
          onClick={() => void submit()}
          disabled={disabled || pending}
        >
          {pending ? 'Applying…' : 'Apply'}
        </button>
        {disabled ? <span className="small dim">Device is disabled.</span> : null}
      </div>

      {report ? (
        <div className="stack stack--tight">
          <p className="small">
            Requested <strong>{formatValue(report.requested, capability.unit)}</strong> → applied{' '}
            <strong>
              {report.applied === undefined
                ? 'nothing'
                : formatValue(report.applied, capability.unit)}
            </strong>
          </p>
          <div className="row">
            <Badge tone={report.status === 'rejected' ? 'danger' : report.status === 'simulated' ? 'warn' : 'ok'}>
              {report.status}
            </Badge>
            {report.clamped ? (
              <Badge tone="warn" title="The value was clamped to the allowed range">
                clamped
              </Badge>
            ) : null}
            {report.simulated ? (
              <Badge tone="warn" title="Nothing was sent to hardware: dry run or simulated device">
                simulated
              </Badge>
            ) : null}
            <span className="tiny dim">
              {formatClock(report.at_ms)} · {describeOrigin(report)}
            </span>
          </div>
        </div>
      ) : null}

      {failure ? (
        <InlineNotice tone="error" title={`Write refused (${failure.code})`}>
          <p>{failure.message}</p>
          {failure.hint ? <p className="inline-notice__hint">{failure.hint}</p> : null}
          {failure.unsupported ? (
            <p className="inline-notice__hint">
              This hardware does not support the requested control.
            </p>
          ) : null}
        </InlineNotice>
      ) : null}
    </div>
  );
}
