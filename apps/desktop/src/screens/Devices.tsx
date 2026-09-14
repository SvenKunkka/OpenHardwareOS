import { useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { useRuntime } from '../hooks/useRuntime';
import { api } from '../lib/ipc';
import {
  capabilityCount,
  fanCapability,
  loadCapability,
  powerCapability,
  primarySensor,
  unavailableReadings,
} from '../lib/devices';
import {
  EMPTY,
  deviceTypeLabel,
  formatRelative,
  transportLabel,
  unavailableReasonLabel,
} from '../lib/format';
import { Badge, EmptyState, InlineNotice, Panel, Spinner } from '../components/primitives';
import { DeviceStatusBadge, ReadingValue, UnavailableBadge } from '../components/status';
import type { DeviceStatus, DeviceType, DeviceView } from '../types';

const TYPE_FILTERS: { value: 'all' | DeviceType; label: string }[] = [
  { value: 'all', label: 'All types' },
  { value: 'cpu', label: 'CPU' },
  { value: 'gpu', label: 'GPU' },
  { value: 'storage', label: 'Storage' },
  { value: 'memory', label: 'Memory' },
  { value: 'motherboard', label: 'Motherboard' },
  { value: 'fan', label: 'Fan' },
  { value: 'pump', label: 'Pump' },
  { value: 'temperature_sensor', label: 'Temperature sensor' },
  { value: 'unknown', label: 'Other' },
];

const STATUS_FILTERS: { value: 'all' | DeviceStatus; label: string }[] = [
  { value: 'all', label: 'Any status' },
  { value: 'online', label: 'Online' },
  { value: 'degraded', label: 'Degraded' },
  { value: 'offline', label: 'Offline' },
  { value: 'disabled', label: 'Disabled' },
];

export function Devices({
  onOpenDevice,
  onOpenSettings,
}: {
  onOpenDevice: (deviceId: string) => void;
  onOpenSettings: () => void;
}) {
  const { snapshot, loading, error, perform, refresh } = useRuntime();
  const [query, setQuery] = useState('');
  const [typeFilter, setTypeFilter] = useState<'all' | DeviceType>('all');
  const [statusFilter, setStatusFilter] = useState<'all' | DeviceStatus>('all');
  const [busy, setBusy] = useState<string | null>(null);

  const filtered = useMemo(() => {
    if (!snapshot) return [];
    const needle = query.trim().toLowerCase();
    return snapshot.devices.filter((view) => {
      if (typeFilter !== 'all' && view.device.type !== typeFilter) return false;
      if (statusFilter !== 'all' && view.status !== statusFilter) return false;
      if (!needle) return true;
      const haystack = [
        view.device.name,
        view.device.id,
        view.device.vendor,
        view.device.adapter,
        view.device.model ?? '',
        ...(view.device.tags ?? []),
      ]
        .join(' ')
        .toLowerCase();
      return haystack.includes(needle);
    });
  }, [snapshot, query, typeFilter, statusFilter]);

  if (!snapshot) {
    return (
      <div className="content">
        {error ? (
          <InlineNotice tone="error" title="Could not list devices">
            <p>{error.message}</p>
            {error.hint ? <p className="inline-notice__hint">{error.hint}</p> : null}
            <button type="button" className="btn btn--primary" onClick={() => void refresh()}>
              Try again
            </button>
          </InlineNotice>
        ) : (
          <Panel title="Devices">
            <Spinner label={loading ? 'Discovering devices…' : 'Waiting for the first snapshot…'} />
          </Panel>
        )}
      </div>
    );
  }

  const toggleDevice = async (view: DeviceView) => {
    const next = !view.enabled;
    setBusy(view.device.id);
    // `perform` surfaces both outcomes: a success toast, or the backend's
    // message + hint when the device refuses to change state.
    await perform(
      `${next ? 'Enable' : 'Disable'} ${view.device.name}`,
      () => api.setDeviceEnabled(view.device.id, next),
      { success: `${view.device.name} ${next ? 'enabled' : 'disabled'}.` },
    );
    setBusy(null);
  };

  return (
    <div className="content">
      <Panel
        title="Devices"
        subtitle={`${snapshot.devices.length} discovered · ${filtered.length} shown`}
        actions={
          <>
            <label className="visually-hidden" htmlFor="device-search">
              Search devices
            </label>
            <input
              id="device-search"
              className="input"
              type="search"
              placeholder="Search name, vendor, adapter…"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              style={{ maxWidth: '260px' }}
            />
            <label className="visually-hidden" htmlFor="device-type">
              Filter by device type
            </label>
            <select
              id="device-type"
              className="select"
              value={typeFilter}
              onChange={(event) => setTypeFilter(event.target.value as 'all' | DeviceType)}
              style={{ maxWidth: '180px' }}
            >
              {TYPE_FILTERS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
            <label className="visually-hidden" htmlFor="device-status">
              Filter by status
            </label>
            <select
              id="device-status"
              className="select"
              value={statusFilter}
              onChange={(event) => setStatusFilter(event.target.value as 'all' | DeviceStatus)}
              style={{ maxWidth: '160px' }}
            >
              {STATUS_FILTERS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </>
        }
      >
        {snapshot.devices.length === 0 ? (
          <EmptyState
            title="No devices discovered"
            body="OpenHardwareOS is running, but no adapter reported a device. Disabled adapters are listed in Settings → Adapters."
            steps={[
              'Install and start LibreHardwareMonitor, then rescan, to read motherboard, CPU and storage sensors.',
              'Run the app as Administrator: several vendors hide sensors and fan control from unelevated processes.',
              'Enable Mock hardware in Settings → Experimental features to work with the simulated fan and pump.',
            ]}
            actions={
              <>
                <button type="button" className="btn btn--primary" onClick={() => onOpenSettings()}>
                  Open settings
                </button>
                <button
                  type="button"
                  className="btn"
                  onClick={() => void perform('Rescan hardware', () => api.scanDevices(), { success: 'Hardware rescan complete.' })}
                >
                  Rescan hardware
                </button>
              </>
            }
          />
        ) : filtered.length === 0 ? (
          <EmptyState
            title="No device matches these filters"
            body="Clear the search box or pick a different type/status to see the discovered devices again."
            actions={
              <button
                type="button"
                className="btn"
                onClick={() => {
                  setQuery('');
                  setTypeFilter('all');
                  setStatusFilter('all');
                }}
              >
                Clear filters
              </button>
            }
          />
        ) : (
          <div className="grid grid--cards">
            {filtered.map((view) => (
              <DeviceCard
                key={view.device.id}
                view={view}
                busy={busy === view.device.id}
                onOpen={() => onOpenDevice(view.device.id)}
                onToggle={() => void toggleDevice(view)}
              />
            ))}
          </div>
        )}
      </Panel>
    </div>
  );
}

export function DeviceCard({
  view,
  busy,
  onOpen,
  onToggle,
}: {
  view: DeviceView;
  busy: boolean;
  onOpen: () => void;
  onToggle: () => void;
}) {
  const primary = primarySensor(view);
  const load = loadCapability(view);
  const power = powerCapability(view);
  const fan = fanCapability(view);
  const counts = capabilityCount(view);
  const unavailable = unavailableReadings(view);

  // Only capabilities the device actually declares get a row; a declared but
  // unreadable capability still shows an em dash plus its reason.
  const stats: { label: string; node: ReactNode }[] = [
    { label: primary?.name ?? 'Primary sensor', node: readout(view, primary?.id, primary?.unit) },
  ];
  if (load) stats.push({ label: load.name, node: readout(view, load.id, load.unit) });
  if (power) stats.push({ label: power.name, node: readout(view, power.id, power.unit) });
  if (fan) stats.push({ label: fan.name, node: readout(view, fan.id, fan.unit) });

  return (
    <article className="device-card">
      <header className="device-card__head">
        <div>
          <p className="device-card__name">{view.device.name}</p>
          <p className="device-card__meta">
            {view.device.vendor}
            {view.device.model && view.device.model !== view.device.name ? ` · ${view.device.model}` : ''}
          </p>
        </div>
        <span className="spacer" />
        <Badge tone="accent" plain>
          {deviceTypeLabel(view.device.type)}
        </Badge>
        <DeviceStatusBadge status={view.status} />
      </header>

      <div className="row small muted">
        <span>Adapter: {view.adapter}</span>
        <span aria-hidden="true">·</span>
        <span>Transport: {transportLabel(view.device.transport)}</span>
      </div>
      <div className="row small muted">
        <span>
          {counts.readable} readable · {counts.writable} writable
        </span>
        <span aria-hidden="true">·</span>
        <span>last seen {formatRelative(view.last_seen_ms)}</span>
      </div>

      <div className="reading-list">
        {stats.map((stat) => (
          <div className="reading-list__row" key={stat.label}>
            <span className="reading-list__label">{stat.label}</span>
            <span>{stat.node}</span>
          </div>
        ))}
      </div>

      {unavailable.length > 0 ? (
        <div className="row" style={{ gap: 'var(--space-1)' }}>
          <UnavailableBadge
            reason={
              unavailable[0]?.reading.status === 'unavailable' ? unavailable[0].reading.reason : 'unknown'
            }
          />
          <span className="tiny dim">
            +{unavailable.length} reading{unavailable.length === 1 ? '' : 's'} unavailable (
            {unavailable
              .slice(0, 2)
              .map((item) =>
                item.reading.status === 'unavailable'
                  ? unavailableReasonLabel(item.reading.reason)
                  : '',
              )
              .filter(Boolean)
              .join('; ')}
            {unavailable.length > 2 ? ' …' : ''})
          </span>
        </div>
      ) : null}

      {view.state?.message ? <p className="tiny dim">{view.state.message}</p> : null}

      <div className="row row--between">
        <button type="button" className="btn btn--sm" onClick={onOpen}>
          Open details
          <span className="visually-hidden"> for {view.device.name}</span>
        </button>
        <label className="switch">
          <input
            type="checkbox"
            checked={view.enabled}
            disabled={busy}
            onChange={onToggle}
            aria-label={`${view.enabled ? 'Disable' : 'Enable'} ${view.device.name}`}
          />
          <span className="switch__track" aria-hidden="true">
            <span className="switch__thumb" />
          </span>
          <span className="switch__label">{busy ? 'Applying…' : view.enabled ? 'Enabled' : 'Disabled'}</span>
        </label>
      </div>
    </article>
  );
}

function readout(view: DeviceView, capabilityId: string | undefined, unit: DeviceView['device']['capabilities'][number]['unit'] = 'none') {
  if (!capabilityId) {
    return <ReadingValue value={undefined} unit={unit} unavailableReason="unsupported" />;
  }
  const reading = view.state?.readings.find((item) => item.capability === capabilityId);
  if (!reading) return <span className="dim">{EMPTY}</span>;
  if (reading.status === 'unavailable') {
    return <ReadingValue value={undefined} unit={unit} unavailableReason={reading.reason} />;
  }
  return <ReadingValue value={reading.value} unit={unit} />;
}
