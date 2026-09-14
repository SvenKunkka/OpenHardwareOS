import { useCallback, useMemo, useState } from 'react';
import { toNoticeError, useRuntime } from '../hooks/useRuntime';
import { usePolled } from '../hooks/usePoll';
import { api } from '../lib/ipc';
import {
  EMPTY,
  adapterStateLabel,
  formatClock,
  formatDateTime,
  formatRelative,
  formatValue,
  humanise,
  unavailableReasonLabel,
  LOG_LEVEL_ORDER,
} from '../lib/format';
import { Badge, EmptyState, InlineNotice, Panel, SectionHead } from '../components/primitives';
import { AdapterStateBadge, HandoverStateBadge } from '../components/status';
import { describeWriteReport } from '../lib/writeStatus';
import {
  handoverChannel,
  handoverErrors,
  handoverIsOwed,
  handoverStateHint,
} from '../lib/handovers';
import type { AuditEntry, HandoverReport, LogLevel, NoticeError, WriteStatus } from '../types';

const AUDIT_LIMIT = 200;
const EVENT_LIMIT = 200;

const LEVEL_FILTERS: ('all' | LogLevel)[] = ['all', 'error', 'warn', 'info', 'debug', 'trace'];

/**
 * Feed level per write status: an unconfirmed write is unknown, so it gets the
 * warning marker rather than the calm one a confirmed write gets.
 */
const AUDIT_WRITE_LEVEL: Record<WriteStatus, string> = {
  applied: 'info',
  simulated: 'info',
  unconfirmed: 'warn',
  rejected: 'error',
};

export function Diagnostics({ onOpenSettings }: { onOpenSettings: () => void }) {
  const { snapshot, events } = useRuntime();
  const tick = snapshot?.generated_at_ms ?? 0;

  const audit = usePolled(() => api.listAuditLog(AUDIT_LIMIT), tick, { throttleMs: 3000 });
  const automation = usePolled(() => api.automationStats(), tick, { throttleMs: 2000 });
  const handovers = usePolled(() => api.ruleHandovers(), tick, { throttleMs: 4000 });

  const [auditQuery, setAuditQuery] = useState('');
  const [auditKind, setAuditKind] = useState<'all' | 'write' | 'lifecycle'>('all');
  const [levelFilter, setLevelFilter] = useState<'all' | LogLevel>('all');
  const [retryMessage, setRetryMessage] = useState<string | null>(null);
  const [retryError, setRetryError] = useState<NoticeError | null>(null);

  /**
   * Re-arm the parked handovers and read the queue again: the backend's count is
   * reported verbatim, because "nothing was re-armed" is a real answer.
   */
  const retryHandovers = useCallback(async () => {
    setRetryError(null);
    setRetryMessage(null);
    try {
      const rearmed = await api.ruleRetryHandovers();
      setRetryMessage(
        rearmed === 0
          ? 'Nothing was re-armed: the runtime found no handover it could take back.'
          : `Re-armed ${rearmed} handover${rearmed === 1 ? '' : 's'}: the runtime will try the fail-safe duty again.`,
      );
      handovers.reload();
    } catch (cause) {
      setRetryError(toNoticeError(cause));
    }
  }, [handovers]);

  const filteredAudit = useMemo(() => {
    const needle = auditQuery.trim().toLowerCase();
    return (audit.data ?? []).filter((entry) => {
      if (auditKind !== 'all' && entry.kind !== auditKind) return false;
      if (!needle) return true;
      return describeAudit(entry).toLowerCase().includes(needle);
    });
  }, [audit.data, auditQuery, auditKind]);

  const filteredEvents = useMemo(() => {
    if (levelFilter === 'all') return events;
    const threshold = LOG_LEVEL_ORDER[levelFilter] ?? 2;
    return events.filter((entry) => (LOG_LEVEL_ORDER[entry.level] ?? 2) <= threshold);
  }, [events, levelFilter]);

  if (!snapshot) {
    return (
      <div className="content">
        <Panel title="Diagnostics">
          <p className="muted">Waiting for the first snapshot…</p>
        </Panel>
      </div>
    );
  }

  return (
    <div className="content">
      <Panel
        title="Diagnostics"
        subtitle="Visible because developer mode is on"
        actions={
          <button type="button" className="btn btn--sm btn--ghost" onClick={onOpenSettings}>
            Settings
          </button>
        }
      >
        <div className="grid grid--two">
          <div>
            <SectionHead title="Runtime" />
            <dl className="kv">
              <dt className="kv__key">Started</dt>
              <dd className="kv__value">
                {formatDateTime(snapshot.started_at_ms)} ({formatRelative(snapshot.started_at_ms)})
              </dd>
              <dt className="kv__key">Generated</dt>
              <dd className="kv__value">{formatDateTime(snapshot.generated_at_ms)}</dd>
              <dt className="kv__key">Poll cycles</dt>
              <dd className="kv__value">{snapshot.stats.poll_cycles.toLocaleString('en-US')}</dd>
              <dt className="kv__key">Discovery cycles</dt>
              <dd className="kv__value">{snapshot.stats.discovery_cycles.toLocaleString('en-US')}</dd>
              <dt className="kv__key">Last poll</dt>
              <dd className="kv__value">
                {formatClock(snapshot.stats.last_poll_ms)} · {snapshot.stats.last_poll_duration_ms} ms
                · {snapshot.stats.last_poll_errors} error(s)
              </dd>
              <dt className="kv__key">Writes</dt>
              <dd className="kv__value">
                {snapshot.stats.writes_attempted} attempted · {snapshot.stats.writes_applied} applied ·{' '}
                {snapshot.stats.writes_rejected} rejected
              </dd>
              <dt className="kv__key">Safety interventions</dt>
              <dd className="kv__value">{snapshot.stats.safety_interventions}</dd>
              <dt className="kv__key">Events published</dt>
              <dd className="kv__value">{snapshot.stats.events_published.toLocaleString('en-US')}</dd>
              <dt className="kv__key">Controllable hardware</dt>
              <dd className="kv__value">{snapshot.has_controllable_hardware ? 'Yes' : 'No'}</dd>
            </dl>
          </div>

          <div>
            <SectionHead title="Automation" />
            {automation.data === null ? (
              <p className="muted">Reading automation statistics…</p>
            ) : (
              <dl className="kv">
                <dt className="kv__key">Rules</dt>
                <dd className="kv__value">
                  {automation.data.rules} configured · {automation.data.enabled_rules} enabled
                </dd>
                <dt className="kv__key">Ticks</dt>
                <dd className="kv__value">{automation.data.ticks.toLocaleString('en-US')}</dd>
                <dt className="kv__key">Evaluations</dt>
                <dd className="kv__value">{automation.data.evaluations.toLocaleString('en-US')}</dd>
                <dt className="kv__key">Writes</dt>
                <dd className="kv__value">{automation.data.writes}</dd>
                <dt className="kv__key">Skipped</dt>
                <dd className="kv__value">{automation.data.skipped}</dd>
                <dt className="kv__key">Fallbacks</dt>
                <dd className="kv__value">{automation.data.fallbacks}</dd>
                <dt className="kv__key">Failures</dt>
                <dd className="kv__value">{automation.data.failures}</dd>
                <dt className="kv__key">Last tick</dt>
                <dd className="kv__value">
                  {formatClock(automation.data.last_tick_ms)} ({formatRelative(automation.data.last_tick_ms)})
                </dd>
              </dl>
            )}
            <p className="small muted" style={{ marginTop: 'var(--space-3)' }}>
              Automation is {snapshot.settings.automation_enabled ? 'enabled' : 'disabled'}
              {snapshot.settings.dry_run ? ' · dry run is on' : ''} · log level{' '}
              {snapshot.settings.log_level}
            </p>
          </div>
        </div>
      </Panel>

      <HandoversPanel
        handovers={handovers.data ?? []}
        error={handovers.error}
        retryError={retryError}
        retryMessage={retryMessage}
        onReload={() => handovers.reload()}
        onRetry={() => void retryHandovers()}
      />

      <Panel title="Adapters" subtitle="State, reason and detail for every installed adapter">
        {snapshot.adapters.length === 0 ? (
          <EmptyState
            title="No adapters installed"
            body="The runtime has no adapter to probe. Enable the mock protocol device in Settings to exercise the app without hardware."
          />
        ) : (
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">Adapter</th>
                  <th scope="col">State</th>
                  <th scope="col">Reason</th>
                  <th scope="col">Detail</th>
                  <th scope="col">Checked</th>
                  <th scope="col">Devices</th>
                </tr>
              </thead>
              <tbody>
                {snapshot.adapters.map((adapter) => (
                  <tr key={adapter.info.id}>
                    <td>
                      <p className="strong">{adapter.info.name}</p>
                      <p className="tiny dim mono">{adapter.info.id}</p>
                    </td>
                    <td>
                      <AdapterStateBadge state={adapter.status.state} />
                      <p className="tiny dim">{adapterStateLabel(adapter.status.state)}</p>
                    </td>
                    <td>{adapter.status.reason ? unavailableReasonLabel(adapter.status.reason) : EMPTY}</td>
                    <td className="small">{adapter.status.detail ?? EMPTY}</td>
                    <td className="small">{formatClock(adapter.status.checked_at_ms)}</td>
                    <td>{adapter.status.device_count}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Panel>

      <Panel
        title="Audit log"
        subtitle={`Last ${AUDIT_LIMIT} entries`}
        actions={
          <>
            <label className="visually-hidden" htmlFor="audit-filter">
              Filter audit log
            </label>
            <input
              id="audit-filter"
              className="input"
              type="search"
              placeholder="Filter entries…"
              value={auditQuery}
              onChange={(event) => setAuditQuery(event.target.value)}
              style={{ maxWidth: '220px' }}
            />
            <label className="visually-hidden" htmlFor="audit-kind">
              Filter by kind
            </label>
            <select
              id="audit-kind"
              className="select"
              value={auditKind}
              onChange={(event) => setAuditKind(event.target.value as 'all' | 'write' | 'lifecycle')}
              style={{ maxWidth: '150px' }}
            >
              <option value="all">All kinds</option>
              <option value="write">Writes</option>
              <option value="lifecycle">Lifecycle</option>
            </select>
          </>
        }
      >
        {audit.error ? (
          <InlineNotice tone="error" title="Could not read the audit log">
            <p>{audit.error.message}</p>
            {audit.error.hint ? <p className="inline-notice__hint">{audit.error.hint}</p> : null}
            <button type="button" className="btn btn--sm" onClick={() => audit.reload()}>
              Retry
            </button>
          </InlineNotice>
        ) : (audit.data ?? []).length === 0 ? (
          <EmptyState
            title="The audit log is empty"
            body="Writes and lifecycle events are recorded here as they happen. Nothing has been written yet."
          />
        ) : filteredAudit.length === 0 ? (
          <p className="muted">No entry matches the current filter.</p>
        ) : (
          <ul className="feed" aria-label="Audit log">
            {filteredAudit.map((entry, index) => (
              <li className="feed__item" key={`${entry.at_ms ?? entry.at ?? index}-${index}`}>
                <span className="feed__time">
                  {entry.at_ms !== undefined ? formatClock(entry.at_ms) : (entry.at ?? EMPTY)}
                </span>
                <span
                  className={`feed__level feed__level--${
                    entry.kind === 'write' && entry.report
                      ? AUDIT_WRITE_LEVEL[entry.report.status]
                      : 'info'
                  }`}
                >
                  {entry.kind}
                </span>
                <span className="feed__message">
                  {describeAudit(entry)}
                  {entry.report?.clamped ? (
                    <>
                      {' '}
                      <Badge tone="warn" plain>
                        clamped
                      </Badge>
                    </>
                  ) : null}
                  {entry.report?.simulated ? (
                    <>
                      {' '}
                      <Badge tone="warn" plain>
                        simulated
                      </Badge>
                    </>
                  ) : null}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Panel>

      <Panel
        title="Live events"
        subtitle={`Last ${EVENT_LIMIT} runtime-event messages, newest first`}
        actions={
          <>
            <label className="visually-hidden" htmlFor="event-level">
              Filter by level
            </label>
            <select
              id="event-level"
              className="select"
              value={levelFilter}
              onChange={(event) => setLevelFilter(event.target.value as 'all' | LogLevel)}
              style={{ maxWidth: '170px' }}
            >
              {LEVEL_FILTERS.map((level) => (
                <option key={level} value={level}>
                  {level === 'all' ? 'All levels' : `Level ≤ ${level}`}
                </option>
              ))}
            </select>
          </>
        }
      >
        {filteredEvents.length === 0 ? (
          <p className="muted">
            No runtime event at this level yet. Events arrive as devices change, rules fire and
            writes are applied.
          </p>
        ) : (
          <ul className="feed" aria-live="polite" aria-label="Live runtime events">
            {filteredEvents.map((entry) => (
              <li className="feed__item" key={entry.key}>
                <span className="feed__time">{formatClock(entry.at_ms)}</span>
                <span className={`feed__level feed__level--${entry.level}`}>{entry.level}</span>
                <span className="feed__message">
                  <span className="tag">{entry.category}</span> {entry.message}
                  {entry.detail ? <span className="feed__detail"> — {entry.detail}</span> : null}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Panel>
    </div>
  );
}

/**
 * Channels a rule left behind, and whether the fail-safe duty actually took them
 * over. An owed handover is unresolved business — the channel is still at
 * whatever the abandoned rule last said — so it is never filed away as history,
 * and a parked one says out loud that nothing is retrying it any more.
 */
function HandoversPanel({
  handovers,
  error,
  retryError,
  retryMessage,
  onReload,
  onRetry,
}: {
  handovers: HandoverReport[];
  error: NoticeError | null;
  retryError: NoticeError | null;
  retryMessage: string | null;
  onReload: () => void;
  onRetry: () => void;
}) {
  const owed = handovers.filter((report) => handoverIsOwed(report.state));
  const settled = handovers.filter((report) => !handoverIsOwed(report.state));
  const failed = owed.filter((report) => report.state === 'failed');

  if (error) {
    return (
      <Panel title="Channel handovers">
        <InlineNotice tone="error" title="Could not read the handover queue">
          <p>{error.message}</p>
          {error.hint ? <p className="inline-notice__hint">{error.hint}</p> : null}
          <button type="button" className="btn btn--sm" onClick={onReload}>
            Retry
          </button>
        </InlineNotice>
      </Panel>
    );
  }

  return (
    <Panel
      title={owed.length > 0 ? `Channel handovers (${owed.length} owed)` : 'Channel handovers'}
      subtitle="Channels a rule left behind, as the runtime handed them to the fail-safe duty"
      actions={
        <>
          {failed.length > 0 ? (
            <button type="button" className="btn btn--sm btn--primary" onClick={onRetry}>
              Retry failed handovers ({failed.length})
            </button>
          ) : null}
          <button type="button" className="btn btn--sm" onClick={onReload}>
            Refresh
          </button>
        </>
      }
    >
      {retryMessage ? (
        <p className="small" data-testid="handover-retry-result">
          {retryMessage}
        </p>
      ) : null}

      {retryError ? (
        <InlineNotice tone="error" title="The runtime would not re-arm the handovers">
          <p data-testid="handover-retry-error">{retryError.message}</p>
          {retryError.hint ? <p className="inline-notice__hint">{retryError.hint}</p> : null}
        </InlineNotice>
      ) : null}

      {owed.length === 0 ? (
        <p className="muted" data-testid="handover-none-owed">
          No channel is waiting for the fail-safe duty. Whenever a rule stops driving an output, the
          channel it left behind is handed over here and stays listed until that is confirmed.
        </p>
      ) : (
        <>
          <InlineNotice tone="warn" title="A channel a rule abandoned is not confirmed protected">
            <p>
              The fail-safe duty has not been confirmed in force on these channels, so what they are
              actually running at is unknown. A failed handover has stopped retrying and needs you to
              act; a pending one is still being attempted.
            </p>
          </InlineNotice>

          <div className="stack" style={{ marginTop: 'var(--space-4)' }}>
            {owed.map((report) => {
              const { cause, latest } = handoverErrors(report);
              return (
                <div
                  className="stack stack--tight"
                  key={`${report.device}-${report.capability}-${report.queued_at_ms}`}
                  data-testid="handover"
                  data-state={report.state}
                  style={{ paddingBottom: 'var(--space-3)', borderBottom: '1px solid var(--border)' }}
                >
                  <div className="row">
                    <HandoverStateBadge state={report.state} />
                    <p className="small">
                      <span className="mono strong" data-testid="handover-channel">
                        {handoverChannel(report)}
                      </span>
                    </p>
                    <span className="tiny dim" data-testid="handover-attempts">
                      {report.attempts} attempt{report.attempts === 1 ? '' : 's'} · last{' '}
                      {formatClock(report.last_attempt_ms)}
                    </span>
                  </div>
                  <p className="small" data-testid="handover-reason">
                    {report.reason}
                  </p>
                  <p className="small">
                    <span className="dim">Left by rule</span>{' '}
                    <span className="mono" data-testid="handover-from-rule">
                      {report.from_rule}
                    </span>
                    <span className="dim"> · queued {formatRelative(report.queued_at_ms)}</span>
                  </p>
                  {cause ? (
                    <p className="small" data-testid="handover-cause">
                      <span className="dim">Cause</span> {cause}
                    </p>
                  ) : null}
                  {latest ? (
                    <p className="small muted" data-testid="handover-latest">
                      <span className="dim">Latest attempt</span> {latest}
                    </p>
                  ) : null}
                  <p
                    className={report.state === 'failed' ? 'small' : 'small muted'}
                    data-testid="handover-state-note"
                  >
                    {handoverStateHint(report.state)}
                  </p>
                </div>
              );
            })}
          </div>
        </>
      )}

      {settled.length > 0 ? (
        <div className="stack stack--tight" style={{ marginTop: 'var(--space-4)' }}>
          <SectionHead title={`Resolved handovers (${settled.length})`} />
          <ul className="feed" aria-label="Resolved handovers">
            {settled.map((report) => (
              <li
                className="feed__item"
                key={`${report.device}-${report.capability}-${report.queued_at_ms}`}
                data-testid="handover-resolved"
              >
                <span className="feed__level feed__level--info">
                  {report.state === 'confirmed' ? 'confirmed' : 'superseded'}
                </span>
                <span className="feed__message">
                  <span className="mono">{handoverChannel(report)}</span>
                  {report.state === 'confirmed'
                    ? ` — the fail-safe duty was confirmed${
                        report.confirmed_value === undefined
                          ? ''
                          : ` at ${formatValue(report.confirmed_value, 'percent')}`
                      }.`
                    : ` — another rule owns this channel now${
                        report.superseded_by ? ` (${report.superseded_by})` : ''
                      }; nothing was written.`}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </Panel>
  );
}

function describeAudit(entry: AuditEntry): string {
  if (entry.report) {
    // Shared with the device screen, so an unconfirmed write is never described
    // as "not applied" here either.
    return describeWriteReport(entry.report);
  }
  const action = entry.action ? humanise(entry.action) : 'Event';
  return `${action}${entry.detail ? ` — ${entry.detail}` : ''}`;
}
