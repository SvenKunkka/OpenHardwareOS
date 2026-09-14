import { useCallback, useMemo, useState } from 'react';
import { toNoticeError, useRuntime } from '../hooks/useRuntime';
import { usePolled } from '../hooks/usePoll';
import { api } from '../lib/ipc';
import { sourceValue } from '../lib/devices';
import {
  CHASSIS_TEMPLATE_CURVE,
  GPU_TEMPLATE_CURVE,
  describeFallbackAction,
  describeSource,
  evaluateCurve,
  isSimpleFallback,
  newRuleId,
  sortCurve,
} from '../lib/curve';
import {
  COMPARATORS,
  comparatorSymbol,
  describeGate,
  describeGateChip,
  describeOtherwise,
  isEqualityComparator,
  otherwiseAction,
  otherwiseChoice,
  parseSensorKey,
  refFromIndex,
  sensorKey,
  thresholdStep,
  thresholdSuffix,
  unitFromSnapshot,
  type OtherwiseChoice,
} from '../lib/condition';
import { EMPTY, deviceTypeLabel, formatRelative, formatValue } from '../lib/format';
import {
  Badge,
  EmptyState,
  Field,
  InlineNotice,
  Panel,
  SectionHead,
  Spinner,
  Switch,
} from '../components/primitives';
import { RuleStatusBadge } from '../components/status';
import { CurvePreview } from '../components/CurvePreview';
import type {
  CapabilityIndex,
  Comparator,
  Condition,
  ControlPointTuple,
  FallbackAction,
  NoticeError,
  Rule,
  RuleConflict,
  RuleOutcome,
  RuntimeSnapshot,
  Source,
} from '../types';

type FallbackChoice = 'hold' | 'safe_default' | 'release' | 'fixed';

interface CurveRow {
  key: string;
  input: string;
  output: string;
}

interface RuleDraft {
  id: string;
  name: string;
  enabled: boolean;
  description: string;
  sourceMode: 'single' | 'aggregate';
  sourceKey: string;
  aggregate: 'max' | 'min' | 'avg';
  sensorKeys: string[];
  targetKey: string;
  curveRows: CurveRow[];
  hysteresis: string;
  deadband: string;
  updateInterval: string;
  minOutput: string;
  maxOutput: string;
  fallbackMissing: FallbackChoice;
  fallbackWrite: FallbackChoice;
  fixedPercent: string;
  sensorTimeout: string;
  priority: string;
  /** Off means "no `when` at all": the rule is always active. */
  whenEnabled: boolean;
  whenSourceKey: string;
  whenOp: Comparator;
  whenValue: string;
  whenOtherwise: OtherwiseChoice;
  whenFixedPercent: string;
}

/** The threshold a brand-new gate starts at; harmless until the user edits it. */
const DEFAULT_WHEN_VALUE = '60';
const DEFAULT_WHEN_FIXED_PERCENT = '50';

function toFallbackChoice(action: FallbackAction): FallbackChoice {
  return isSimpleFallback(action);
}

function emptyDraft(index: CapabilityIndex | null): RuleDraft {
  const firstSource = index?.sources[0];
  const firstTarget = index?.targets[0];
  return {
    id: newRuleId(),
    name: 'New cooling rule',
    enabled: true,
    description: '',
    sourceMode: 'single',
    sourceKey: firstSource ? sensorKey(firstSource) : '',
    aggregate: 'max',
    sensorKeys: firstSource ? [sensorKey(firstSource)] : [],
    targetKey: firstTarget ? sensorKey(firstTarget) : '',
    curveRows: curveToRows(GPU_TEMPLATE_CURVE),
    hysteresis: '3',
    deadband: '2',
    updateInterval: '2000',
    minOutput: '20',
    maxOutput: '100',
    fallbackMissing: 'safe_default',
    fallbackWrite: 'hold',
    fixedPercent: '50',
    sensorTimeout: '10',
    priority: '10',
    whenEnabled: false,
    whenSourceKey: firstSource ? sensorKey(firstSource) : '',
    whenOp: 'gt',
    whenValue: DEFAULT_WHEN_VALUE,
    whenOtherwise: 'safe_default',
    whenFixedPercent: DEFAULT_WHEN_FIXED_PERCENT,
  };
}

function curveToRows(curve: ControlPointTuple[]): CurveRow[] {
  return sortCurve(curve).map((point, index) => ({
    key: `row-${index}-${point[0]}`,
    input: String(point[0]),
    output: String(point[1]),
  }));
}

function draftFromRule(rule: Rule): RuleDraft {
  const source = rule.source;
  const fallbackMissing = toFallbackChoice(rule.fallback.on_sensor_missing);
  const fallbackWrite = toFallbackChoice(rule.fallback.on_write_failure);
  const when = rule.when;
  return {
    id: rule.id,
    name: rule.name,
    enabled: rule.enabled,
    description: rule.description ?? '',
    sourceMode: 'device' in source ? 'single' : 'aggregate',
    sourceKey: 'device' in source ? `${source.device}|${source.capability}` : '',
    aggregate: 'device' in source ? 'max' : source.aggregate,
    sensorKeys: 'device' in source ? [] : source.sensors.map((sensor) => `${sensor.device}|${sensor.capability}`),
    targetKey: `${rule.target.device}|${rule.target.capability}`,
    curveRows: curveToRows(rule.curve),
    hysteresis: String(rule.hysteresis),
    deadband: String(rule.deadband),
    updateInterval: String(rule.update_interval_ms),
    minOutput: rule.min_output === undefined ? '' : String(rule.min_output),
    maxOutput: rule.max_output === undefined ? '' : String(rule.max_output),
    fallbackMissing,
    fallbackWrite,
    fixedPercent: String(
      typeof rule.fallback.on_sensor_missing === 'object'
        ? rule.fallback.on_sensor_missing.fixed.percent
        : 50,
    ),
    sensorTimeout: String(rule.fallback.sensor_timeout_s),
    priority: String(rule.priority),
    whenEnabled: when !== undefined,
    whenSourceKey: when ? `${when.source.device}|${when.source.capability}` : '',
    whenOp: when?.op ?? 'gt',
    whenValue: when ? String(when.value) : DEFAULT_WHEN_VALUE,
    whenOtherwise: when ? otherwiseChoice(when.otherwise) : 'safe_default',
    whenFixedPercent:
      when && typeof when.otherwise === 'object'
        ? String(when.otherwise.fixed.percent)
        : DEFAULT_WHEN_FIXED_PERCENT,
  };
}

/**
 * The gate the draft describes. `errors` is empty and `condition` undefined when
 * the toggle is off — that is what keeps `when` out of the wire payload
 * entirely, rather than sending `null` or a default object.
 */
function conditionFromDraft(draft: RuleDraft): { condition?: Condition; errors: string[] } {
  if (!draft.whenEnabled) return { errors: [] };

  const errors: string[] = [];
  const source = parseSensorKey(draft.whenSourceKey);
  if (!source) errors.push('Choose the sensor the “only when…” condition reads.');

  const value = Number(draft.whenValue);
  if (draft.whenValue.trim() === '' || !Number.isFinite(value)) {
    errors.push('The condition threshold must be a number.');
  }

  if (draft.whenOtherwise === 'fixed') {
    const percent = Number(draft.whenFixedPercent);
    if (draft.whenFixedPercent.trim() === '' || !Number.isFinite(percent) || percent < 0 || percent > 100) {
      errors.push('The “otherwise” percentage must be between 0 and 100.');
    }
  }

  if (errors.length > 0 || !source) return { errors };
  return {
    errors,
    condition: {
      source,
      op: draft.whenOp,
      value,
      otherwise: otherwiseAction(draft.whenOtherwise, draft.whenFixedPercent),
    },
  };
}

function fallbackAction(choice: FallbackChoice, fixedPercent: string): FallbackAction {
  switch (choice) {
    case 'hold':
      return 'hold';
    case 'safe_default':
      return 'safe_default';
    case 'release':
      return 'release';
    case 'fixed':
      return { fixed: { percent: Number(fixedPercent) || 0 } };
    default:
      return 'hold';
  }
}

/** Convert the editor draft into a wire `Rule`, collecting blocking errors. */
function draftToRule(draft: RuleDraft): { rule?: Rule; errors: string[] } {
  const errors: string[] = [];

  const rows: ControlPointTuple[] = [];
  for (const row of draft.curveRows) {
    const input = Number(row.input);
    const output = Number(row.output);
    if (row.input.trim() === '' || row.output.trim() === '') {
      errors.push('Every control point needs both an input and an output value.');
      return { errors };
    }
    if (!Number.isFinite(input) || !Number.isFinite(output)) {
      errors.push('Control points must be numbers.');
      return { errors };
    }
    rows.push([input, output]);
  }

  if (rows.length < 2) errors.push('A curve needs at least two control points.');
  const sorted = sortCurve(rows);
  for (let index = 1; index < sorted.length; index += 1) {
    const previous = sorted[index - 1];
    const current = sorted[index];
    if (previous && current && current[0] === previous[0]) {
      errors.push('Two control points share the same input; inputs must be unique.');
      break;
    }
  }

  let source: Source | undefined;
  if (draft.sourceMode === 'single') {
    const [device, capability] = draft.sourceKey.split('|');
    if (!device || !capability) errors.push('Choose a source sensor.');
    else source = { device, capability };
  } else {
    const sensors = draft.sensorKeys
      .map((key) => {
        const [device, capability] = key.split('|');
        return device && capability ? { device, capability } : null;
      })
      .filter((sensor): sensor is { device: string; capability: string } => sensor !== null);
    if (sensors.length === 0) errors.push('An aggregate needs at least one sensor.');
    else source = { aggregate: draft.aggregate, sensors };
  }

  const [targetDevice, targetCapability] = draft.targetKey.split('|');
  if (!targetDevice || !targetCapability) errors.push('Choose a target capability.');

  if (!draft.name.trim()) errors.push('Give the rule a name.');
  if (errors.length > 0 || !source || !targetDevice || !targetCapability) return { errors };

  const minOutput = draft.minOutput.trim() === '' ? undefined : Number(draft.minOutput);
  const maxOutput = draft.maxOutput.trim() === '' ? undefined : Number(draft.maxOutput);
  if (minOutput !== undefined && !Number.isFinite(minOutput)) errors.push('Minimum output must be a number.');
  if (maxOutput !== undefined && !Number.isFinite(maxOutput)) errors.push('Maximum output must be a number.');
  if (errors.length > 0) return { errors };

  const gate = conditionFromDraft(draft);
  if (gate.errors.length > 0) return { errors: [...errors, ...gate.errors] };

  const rule: Rule = {
    id: draft.id,
    name: draft.name.trim(),
    enabled: draft.enabled,
    description: draft.description.trim() === '' ? undefined : draft.description.trim(),
    source,
    // Spread rather than `when: undefined`: an ungated rule must carry no `when`
    // key at all on the wire.
    ...(gate.condition ? { when: gate.condition } : {}),
    target: { device: targetDevice, capability: targetCapability },
    curve: sorted,
    hysteresis: Number(draft.hysteresis) || 0,
    deadband: Number(draft.deadband) || 0,
    update_interval_ms: Number(draft.updateInterval) || 1000,
    min_output: minOutput,
    max_output: maxOutput,
    fallback: {
      on_sensor_missing: fallbackAction(draft.fallbackMissing, draft.fixedPercent),
      on_write_failure: fallbackAction(draft.fallbackWrite, draft.fixedPercent),
      sensor_timeout_s: Number(draft.sensorTimeout) || 10,
    },
    priority: Number(draft.priority) || 0,
  };

  return { rule, errors };
}

export function Automation() {
  const { snapshot, perform, refresh } = useRuntime();
  const tick = snapshot?.generated_at_ms ?? 0;

  const rules = usePolled(() => api.listRules(), tick, { throttleMs: 4000 });
  const outcomes = usePolled(() => api.ruleOutcomes(), tick, { throttleMs: 1200 });
  const capabilityIndex = usePolled(() => api.capabilityIndex(), tick, { throttleMs: 8000 });
  const suggestions = usePolled(() => api.suggestRules(), tick, { throttleMs: 15000 });
  const conflicts = usePolled(() => api.ruleConflicts(), tick, { throttleMs: 4000 });

  const [editing, setEditing] = useState<RuleDraft | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [actionError, setActionError] = useState<NoticeError | null>(null);

  const reloadAll = useCallback(async () => {
    rules.reload();
    outcomes.reload();
    conflicts.reload();
    await refresh();
  }, [rules, outcomes, conflicts, refresh]);

  /**
   * Run a rule action, keep the backend's own message (never a generic one) for
   * an inline notice, and reload the conflict list afterwards: enabling or
   * deleting a rule changes which output is owned by whom.
   */
  const runAction = useCallback(
    async <T,>(
      label: string,
      action: () => Promise<T>,
      success: string,
    ): Promise<T | undefined> => {
      setActionError(null);
      const captured: { notice: NoticeError | null } = { notice: null };
      const result = await perform(
        label,
        async () => {
          try {
            return await action();
          } catch (cause) {
            captured.notice = toNoticeError(cause);
            throw cause;
          }
        },
        { refresh: false, success },
      );
      if (captured.notice) setActionError(captured.notice);
      else await reloadAll();
      return result;
    },
    [perform, reloadAll],
  );

  if (!snapshot) {
    return (
      <div className="content">
        <Panel title="Automation">
          <Spinner label="Loading rules…" />
        </Panel>
      </div>
    );
  }

  const outcomeByRule = new Map<string, RuleOutcome>();
  for (const outcome of outcomes.data ?? []) outcomeByRule.set(outcome.rule_id, outcome);

  const ruleList = rules.data ?? [];
  const index = capabilityIndex.data;

  return (
    <div className="content">
      <Panel
        title="Automation"
        subtitle="Temperature to fan-speed curves, evaluated by the Rust runtime"
        actions={
          <>
            <button
              type="button"
              className="btn btn--sm"
              onClick={() => void perform('Rescan hardware', () => api.scanDevices(), { success: 'Hardware rescan complete.' })}
            >
              Rescan
            </button>
            <button
              type="button"
              className="btn btn--sm btn--primary"
              onClick={() => {
                setEditing(emptyDraft(index ?? null));
                setIsNew(true);
              }}
            >
              New rule
            </button>
          </>
        }
      >
        <div className="row small muted">
          <span>
            Automation is <strong>{snapshot.settings.automation_enabled ? 'enabled' : 'disabled'}</strong>
            {' —'} change it in Settings.
          </span>
          {snapshot.settings.dry_run ? (
            <Badge tone="warn" title="Writes are logged but not sent to hardware">
              dry run
            </Badge>
          ) : null}
        </div>

        {rules.error ? (
          <InlineNotice tone="error" title="Could not load rules">
            <p>{rules.error.message}</p>
            {rules.error.hint ? <p className="inline-notice__hint">{rules.error.hint}</p> : null}
            <button type="button" className="btn btn--sm" onClick={() => rules.reload()}>
              Retry
            </button>
          </InlineNotice>
        ) : null}
      </Panel>

      <RuleConflictsPanel
        conflicts={conflicts.data ?? []}
        error={conflicts.error}
        onReload={() => conflicts.reload()}
        onEdit={(rule) => {
          setEditing(draftFromRule(rule));
          setIsNew(false);
        }}
        rules={ruleList}
      />

      {actionError ? (
        <InlineNotice tone="error" title="The runtime refused this change">
          <p data-testid="action-error-message">{actionError.message}</p>
          {actionError.hint ? <p className="inline-notice__hint">{actionError.hint}</p> : null}
          <button type="button" className="btn btn--sm" onClick={() => setActionError(null)}>
            Dismiss
          </button>
        </InlineNotice>
      ) : null}

      {editing ? (
        <RuleForm
          draft={editing}
          isNew={isNew}
          index={index ?? null}
          snapshot={snapshot}
          onChange={setEditing}
          onCancel={() => {
            setEditing(null);
            setIsNew(false);
          }}
          onSaved={async () => {
            setEditing(null);
            setIsNew(false);
            await reloadAll();
          }}
        />
      ) : null}

      <Panel
        title="Rules"
        subtitle={`${ruleList.length} configured · ${(outcomes.data ?? []).filter((o) => o.status === 'applied').length} applied on the last tick`}
      >
        {ruleList.length === 0 ? (
          <EmptyState
            title="No cooling rules yet"
            body="A rule watches a temperature sensor and writes a fan duty through a curve. Nothing is automated until you add one."
            steps={[
              'Click “New rule”, or add a suggestion below.',
              'Pick a source sensor (for example GPU core temperature) and a target (the GPU fan duty).',
              'Load the documented GPU curve, then save: the rule is checked before it is stored.',
            ]}
            actions={
              <button
                type="button"
                className="btn btn--primary"
                onClick={() => {
                  setEditing(emptyDraft(index ?? null));
                  setIsNew(true);
                }}
              >
                New rule
              </button>
            }
          />
        ) : (
          <div className="stack">
            {ruleList.map((rule) => (
              <RuleCard
                key={rule.id}
                rule={rule}
                outcome={outcomeByRule.get(rule.id)}
                snapshot={snapshot}
                onEdit={() => {
                  setEditing(draftFromRule(rule));
                  setIsNew(false);
                }}
                onToggle={(enabled) =>
                  void runAction(
                    `${enabled ? 'Enable' : 'Disable'} “${rule.name}”`,
                    () => api.setRuleEnabled(rule.id, enabled),
                    `“${rule.name}” ${enabled ? 'enabled' : 'disabled'}.`,
                  )
                }
                onDelete={() =>
                  void runAction(
                    `Delete “${rule.name}”`,
                    async () => {
                      const deleted = await api.deleteRule(rule.id);
                      if (!deleted) throw { code: 'not_found', message: 'No such rule.', hint: 'Reload the list and try again.' };
                      return deleted;
                    },
                    `“${rule.name}” deleted.`,
                  )
                }
              />
            ))}
          </div>
        )}
      </Panel>

      <Panel
        title="Suggested for your hardware"
        subtitle="Generated from the capabilities your adapters report"
      >
        {suggestions.loading && suggestions.data === null ? (
          <Spinner label="Looking for suggestions…" />
        ) : (suggestions.data ?? []).length === 0 ? (
          <p className="muted">
            No suggestion can be made from the current capabilities. Suggestions need at least one
            readable temperature sensor and one writable percentage actuator.
          </p>
        ) : (
          <div className="stack">
            {(suggestions.data ?? []).map((rule) => (
              <div className="list-button" key={rule.id} style={{ cursor: 'default' }}>
                <div className="stack stack--tight" style={{ flex: 1 }}>
                  <p className="strong">{rule.name}</p>
                  <p className="small muted">
                    {describeSource(rule.source)} → {rule.target.device} · {rule.target.capability}
                  </p>
                  {rule.description ? <p className="tiny dim">{rule.description}</p> : null}
                </div>
                <button
                  type="button"
                  className="btn btn--sm btn--primary"
                  onClick={() =>
                    void runAction(
                      `Add “${rule.name}”`,
                      () => api.saveRule({ ...rule, enabled: false }),
                      `“${rule.name}” added, disabled. Review it, then enable it.`,
                    )
                  }
                >
                  Add (disabled)
                </button>
              </div>
            ))}
          </div>
        )}
      </Panel>
    </div>
  );
}

/**
 * Enabled rules that fight over one output, exactly as the engine reported them.
 * The runtime resolves them deterministically at load time (highest priority,
 * then lowest id wins) and stands the losers down **in memory only** — the rule
 * files are untouched, so the user has to decide what to do.
 */
function RuleConflictsPanel({
  conflicts,
  error,
  rules,
  onReload,
  onEdit,
}: {
  conflicts: RuleConflict[];
  error: NoticeError | null;
  rules: Rule[];
  onReload: () => void;
  onEdit: (rule: Rule) => void;
}) {
  if (error) {
    return (
      <Panel title="Output conflicts">
        <InlineNotice tone="error" title="Could not read the conflict list">
          <p>{error.message}</p>
          <button type="button" className="btn btn--sm" onClick={onReload}>
            Retry
          </button>
        </InlineNotice>
      </Panel>
    );
  }

  if (conflicts.length === 0) {
    return (
      <Panel
        title="Output conflicts"
        subtitle="One enabled rule per output, checked when a rule is stored, enabled or loaded"
      >
        <p className="small muted">
          No two enabled rules share an output. If one ever does, the runtime keeps the
          higher-priority rule enabled, stands the other one down and lists it here.
        </p>
      </Panel>
    );
  }

  return (
    <Panel
      title={`Output conflicts (${conflicts.length})`}
      subtitle="Resolved in memory at load time — the rule files were not changed"
      actions={
        <button type="button" className="btn btn--sm" onClick={onReload}>
          Refresh
        </button>
      }
    >
      <InlineNotice tone="warn" title="Two enabled rules want the same output">
        <p>
          Only one enabled rule may drive an output. The rule that lost keeps its settings but is
          not evaluated until you disable or retarget one of the two.
        </p>
      </InlineNotice>

      <div className="stack" style={{ marginTop: 'var(--space-4)' }}>
        {conflicts.map((conflict) => {
          const blocked = rules.find((rule) => rule.id === conflict.blocked_id);
          return (
            <div
              className="stack stack--tight"
              key={`${conflict.owner_id}-${conflict.blocked_id}`}
              data-testid="rule-conflict"
              style={{ paddingBottom: 'var(--space-3)', borderBottom: '1px solid var(--border)' }}
            >
              <div className="row">
                <Badge tone="warn" title="This rule keeps the output">
                  owner
                </Badge>
                <p className="small">
                  <span data-testid="conflict-owner">{conflict.owner_name}</span>
                  <span className="dim"> ({conflict.owner_id})</span>
                </p>
              </div>
              <div className="row">
                <Badge tone="neutral" title="This rule was stood down in memory">
                  blocked
                </Badge>
                <p className="small">
                  <span data-testid="conflict-blocked">{conflict.blocked_name}</span>
                  <span className="dim"> ({conflict.blocked_id})</span>
                </p>
              </div>
              <p className="small">
                <span className="dim">Target</span>{' '}
                <span data-testid="conflict-target">{conflict.target}</span>
              </p>
              <p className="small muted" data-testid="conflict-resolution">
                {conflict.resolution}
              </p>
              {blocked ? (
                <div className="row">
                  <button type="button" className="btn btn--sm" onClick={() => onEdit(blocked)}>
                    Retarget “{blocked.name}”
                  </button>
                </div>
              ) : null}
            </div>
          );
        })}
      </div>
    </Panel>
  );
}

function RuleCard({
  rule,
  outcome,
  snapshot,
  onEdit,
  onToggle,
  onDelete,
}: {
  rule: Rule;
  outcome: RuleOutcome | undefined;
  snapshot: RuntimeSnapshot;
  onEdit: () => void;
  onToggle: (enabled: boolean) => void;
  onDelete: () => void;
}) {
  const live = sourceValue(rule.source, snapshot);
  const [confirming, setConfirming] = useState(false);
  const gateUnit = rule.when
    ? (unitFromSnapshot(snapshot, rule.when.source.device, rule.when.source.capability) ?? 'none')
    : 'none';
  const failSafePercent = snapshot.settings.safety.fail_safe_duty_percent;
  const standingDown = outcome?.status === 'gated';

  return (
    <article className="stack" style={{ paddingBottom: 'var(--space-4)', borderBottom: '1px solid var(--border)' }}>
      <div className="row">
        <p className="strong" style={{ fontSize: 'var(--step-1)' }}>
          {rule.name}
        </p>
        <RuleStatusBadge status={outcome?.status ?? (rule.enabled ? 'idle' : 'disabled')} />
        {!rule.enabled ? <Badge tone="neutral">disabled</Badge> : null}
        {rule.when ? (
          <Badge
            plain
            tone="info"
            title={describeGate(rule.when, gateUnit, failSafePercent)}
          >
            {describeGateChip(rule.when, gateUnit)}
          </Badge>
        ) : null}
        <Badge
          plain
          title="Evaluation order: higher priority is evaluated and written first. If two rules share a target, the last write in a cycle wins, so give them the same intent or separate outputs."
        >
          priority {rule.priority}
        </Badge>
        <span className="spacer" />
        <label className="switch">
          <input
            type="checkbox"
            checked={rule.enabled}
            onChange={(event) => onToggle(event.target.checked)}
            aria-label={`${rule.enabled ? 'Disable' : 'Enable'} rule ${rule.name}`}
          />
          <span className="switch__track" aria-hidden="true">
            <span className="switch__thumb" />
          </span>
          <span className="switch__label">{rule.enabled ? 'Enabled' : 'Disabled'}</span>
        </label>
        <button
          type="button"
          className="btn btn--sm"
          onClick={onEdit}
          aria-label={`Edit rule ${rule.name}`}
        >
          Edit
        </button>
        {confirming ? (
          <>
            <button type="button" className="btn btn--sm btn--danger" onClick={onDelete}>
              Confirm delete
            </button>
            <button type="button" className="btn btn--sm btn--ghost" onClick={() => setConfirming(false)}>
              Cancel
            </button>
          </>
        ) : (
          <button type="button" className="btn btn--sm btn--danger" onClick={() => setConfirming(true)}>
            Delete
          </button>
        )}
      </div>

      <div className="grid grid--two">
        <div className="stack stack--tight">
          <p className="small">
            <span className="dim">Source</span> {describeSource(rule.source)}
          </p>
          <p className="small">
            <span className="dim">Target</span> {rule.target.device} · {rule.target.capability}
          </p>
          {rule.when ? (
            <p className="small">
              <span className="dim">Gate</span>{' '}
              {describeGate(rule.when, gateUnit, failSafePercent)}
            </p>
          ) : null}
          {standingDown ? (
            <p className="small" data-testid="rule-standing-down">
              <span className="dim">Standing down</span> The condition is not met, so the curve is
              not steering this output — instead the rule will{' '}
              {describeOtherwise(rule.when?.otherwise ?? 'safe_default', failSafePercent)}.
            </p>
          ) : null}
          <p className="small">
            <span className="dim">Live</span>{' '}
            {live === undefined ? (
              <>
                {EMPTY} <Badge tone="neutral">source unavailable</Badge>
              </>
            ) : (
              <>
                {formatValue(live, 'celsius')} → {formatValue(evaluateCurve(rule.curve, live), 'percent')}
              </>
            )}
          </p>
          <p className="small muted">
            Hysteresis {rule.hysteresis} · deadband {rule.deadband} · every {rule.update_interval_ms} ms
            {rule.min_output !== undefined || rule.max_output !== undefined
              ? ` · output limits ${rule.min_output ?? EMPTY}–${rule.max_output ?? EMPTY} %`
              : ''}
          </p>
          <p className="small muted">
            Fallbacks: sensor missing → {describeFallbackAction(rule.fallback.on_sensor_missing)}; write
            failure → {describeFallbackAction(rule.fallback.on_write_failure)} (timeout{' '}
            {rule.fallback.sensor_timeout_s} s)
          </p>
          {rule.description ? <p className="tiny dim">{rule.description}</p> : null}
          <p className="tiny dim">
            Evaluations {outcome?.evaluations ?? 0} · writes {outcome?.writes ?? 0} · skipped{' '}
            {outcome?.skipped ?? 0} · fallbacks {outcome?.fallbacks ?? 0}
            {outcome ? ` · ${formatRelative(outcome.at_ms)}` : ''}
          </p>
          {outcome?.message ? <p className="small">{outcome.message}</p> : null}
        </div>
        <div>
          <CurvePreview
            curve={rule.curve}
            current={live}
            label={`Curve for ${rule.name}`}
            height={150}
          />
        </div>
      </div>
    </article>
  );
}

function RuleForm({
  draft,
  isNew,
  index,
  snapshot,
  onChange,
  onCancel,
  onSaved,
}: {
  draft: RuleDraft;
  isNew: boolean;
  index: CapabilityIndex | null;
  snapshot: RuntimeSnapshot;
  onChange: (next: RuleDraft) => void;
  onCancel: () => void;
  onSaved: () => Promise<void>;
}) {
  const [errors, setErrors] = useState<string[]>([]);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [failure, setFailure] = useState<NoticeError | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);

  const sources = index?.sources ?? [];
  const targets = index?.targets ?? [];
  const failSafePercent = snapshot.settings.safety.fail_safe_duty_percent;

  const whenSource = draft.whenEnabled ? parseSensorKey(draft.whenSourceKey) : undefined;
  const whenRef = whenSource ? refFromIndex(index, whenSource.device, whenSource.capability) : undefined;
  const whenUnit = whenRef?.capability.unit ?? 'none';
  const gate = useMemo(() => conditionFromDraft(draft), [draft]);

  const numericCurve = useMemo<ControlPointTuple[]>(
    () =>
      draft.curveRows
        .map((row) => [Number(row.input), Number(row.output)] as ControlPointTuple)
        .filter((point) => Number.isFinite(point[0]) && Number.isFinite(point[1])),
    [draft.curveRows],
  );

  const liveInput = useMemo(() => {
    const parsed = draftToRule({ ...draft, curveRows: draft.curveRows });
    if (!parsed.rule) return undefined;
    return sourceValue(parsed.rule.source, snapshot);
  }, [draft, snapshot]);

  const update = (patch: Partial<RuleDraft>) => onChange({ ...draft, ...patch });

  const handleSave = async () => {
    const local = draftToRule(draft);
    if (!local.rule) {
      setErrors(local.errors);
      setWarnings([]);
      setFailure(null);
      return;
    }
    setBusy(true);
    setFailure(null);
    try {
      const check = await api.checkRule(local.rule);
      setWarnings(check.warnings);
      if (check.errors.length > 0) {
        setErrors(check.errors);
        return;
      }
      setErrors([]);
      await api.saveRule(local.rule);
      setSaved(true);
      await onSaved();
    } catch (cause) {
      // The backend's own sentence (for example the two-rules-one-output
      // refusal) is shown verbatim, with the hint it shipped alongside.
      setErrors([]);
      setFailure(toNoticeError(cause));
    } finally {
      setBusy(false);
    }
  };

  const clash = errors.some((error) => error.includes('already drives'));

  return (
    <Panel
      title={isNew ? 'New rule' : `Edit “${draft.name || draft.id}”`}
      subtitle="Checked with check_rule before saving"
      actions={
        <>
          <button type="button" className="btn btn--sm btn--ghost" onClick={onCancel}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn--sm btn--primary"
            onClick={() => void handleSave()}
            disabled={busy}
          >
            {busy ? 'Saving…' : 'Check & save'}
          </button>
        </>
      }
    >
      <div className="stack">
        {failure ? (
          <InlineNotice
            tone="error"
            title={
              failure.code === 'automation_error'
                ? 'The runtime refused this rule'
                : 'The rule could not be saved'
            }
          >
            <p data-testid="save-error-message">{failure.message}</p>
            {failure.hint ? <p className="inline-notice__hint">{failure.hint}</p> : null}
          </InlineNotice>
        ) : null}

        {errors.length > 0 ? (
          <InlineNotice
            tone="error"
            title={
              clash
                ? 'Another enabled rule already drives this output'
                : `This rule cannot be saved (${errors.length} error${errors.length === 1 ? '' : 's'})`
            }
          >
            <ul className="stack--tight">
              {errors.map((error) => (
                <li key={error}>• {error}</li>
              ))}
            </ul>
          </InlineNotice>
        ) : null}

        {warnings.length > 0 ? (
          <InlineNotice tone="warn" title="Warnings — saving is still allowed">
            <ul className="stack--tight">
              {warnings.map((warning) => (
                <li key={warning}>• {warning}</li>
              ))}
            </ul>
          </InlineNotice>
        ) : null}

        <div className="form-grid">
          <Field label="Rule name" htmlFor="rule-name">
            <input
              id="rule-name"
              className="input"
              value={draft.name}
              onChange={(event) => update({ name: event.target.value })}
            />
          </Field>
          <Field
            label="Priority"
            htmlFor="rule-priority"
            hint="Evaluation order: higher is written first. Two rules on one target conflict — the last write in a cycle wins, so keep them off the same output."
          >
            <input
              id="rule-priority"
              className="input input--number"
              type="number"
              min={0}
              value={draft.priority}
              onChange={(event) => update({ priority: event.target.value })}
            />
          </Field>
          <Field label="Description" htmlFor="rule-description">
            <input
              id="rule-description"
              className="input"
              value={draft.description}
              onChange={(event) => update({ description: event.target.value })}
            />
          </Field>
          <Field
            label="Enabled"
            labelAs="span"
            hint="Disabled rules are stored but never evaluated"
          >
            <label className="switch" htmlFor="rule-enabled">
              <input
                id="rule-enabled"
                type="checkbox"
                checked={draft.enabled}
                onChange={(event) => update({ enabled: event.target.checked })}
              />
              <span className="switch__track" aria-hidden="true">
                <span className="switch__thumb" />
              </span>
              <span className="switch__label">{draft.enabled ? 'On' : 'Off'}</span>
            </label>
          </Field>
        </div>

        <SectionHead title="Source" hint="The temperature the curve reads" />
        <div className="row">
          <div className="segmented" role="group" aria-label="Source kind">
            <button
              type="button"
              className="segmented__option"
              aria-pressed={draft.sourceMode === 'single'}
              onClick={() => update({ sourceMode: 'single' })}
            >
              Single sensor
            </button>
            <button
              type="button"
              className="segmented__option"
              aria-pressed={draft.sourceMode === 'aggregate'}
              onClick={() => update({ sourceMode: 'aggregate' })}
            >
              Aggregate
            </button>
          </div>
        </div>

        {draft.sourceMode === 'single' ? (
          <Field
            label="Sensor"
            htmlFor="rule-source"
            hint={
              sources.length === 0
                ? 'No readable sensor is available. Enable an adapter or run as Administrator.'
                : undefined
            }
          >
            <select
              id="rule-source"
              className="select"
              value={draft.sourceKey}
              onChange={(event) => update({ sourceKey: event.target.value })}
            >
              <option value="">— choose a sensor —</option>
              {sources.map((ref) => (
                <option key={sensorKey(ref)} value={sensorKey(ref)}>
                  {ref.device_name} · {ref.capability.name} ({ref.capability.unit})
                  {ref.current !== undefined ? ` — now ${formatValue(ref.current, ref.capability.unit)}` : ''}
                </option>
              ))}
            </select>
          </Field>
        ) : (
          <>
            <Field label="Aggregate" htmlFor="rule-aggregate" hint="How the chosen sensors are combined">
              <select
                id="rule-aggregate"
                className="select"
                value={draft.aggregate}
                onChange={(event) =>
                  update({ aggregate: event.target.value as 'max' | 'min' | 'avg' })
                }
              >
                <option value="max">Maximum of the sensors</option>
                <option value="min">Minimum of the sensors</option>
                <option value="avg">Average of the sensors</option>
              </select>
            </Field>
            <fieldset className="field" style={{ border: 'none', padding: 0, margin: 0 }}>
              <legend className="field__label">Sensors in the aggregate</legend>
              <div className="stack--tight" style={{ maxHeight: '200px', overflowY: 'auto' }}>
                {sources.length === 0 ? (
                  <p className="small muted">No sensors available.</p>
                ) : (
                  sources.map((ref) => {
                    const key = sensorKey(ref);
                    const checked = draft.sensorKeys.includes(key);
                    return (
                      <label className="checkbox" key={key} htmlFor={`agg-${key}`}>
                        <input
                          id={`agg-${key}`}
                          type="checkbox"
                          checked={checked}
                          onChange={(event) =>
                            update({
                              sensorKeys: event.target.checked
                                ? [...draft.sensorKeys, key]
                                : draft.sensorKeys.filter((item) => item !== key),
                            })
                          }
                        />
                        <span className="checkbox__text">
                          <span className="checkbox__title">
                            {ref.device_name} · {ref.capability.name}
                          </span>
                          <span className="checkbox__hint">
                            {deviceTypeLabel(ref.device_type)} · {ref.capability.id}
                          </span>
                        </span>
                      </label>
                    );
                  })
                )}
              </div>
            </fieldset>
          </>
        )}

        <SectionHead title="Target" hint="What the rule writes" />
        <Field
          label="Target capability"
          htmlFor="rule-target"
          hint={
            targets.length === 0
              ? 'No writable capability is available. Without one, a rule can only report fallback.'
              : 'Only writable capabilities are listed.'
          }
        >
          <select
            id="rule-target"
            className="select"
            value={draft.targetKey}
            onChange={(event) => update({ targetKey: event.target.value })}
          >
            <option value="">— choose a target —</option>
            {targets.map((ref) => (
              <option key={sensorKey(ref)} value={sensorKey(ref)}>
                {ref.device_name} · {ref.capability.name} ({ref.capability.unit})
              </option>
            ))}
          </select>
        </Field>

        <SectionHead
          title="Only when…"
          hint="An optional gate. While the condition does not hold, the rule stands down and drives the target to the “otherwise” duty — it never holds whatever the fan had."
        />
        <Switch
          id="rule-when-enabled"
          checked={draft.whenEnabled}
          onChange={(next) => update({ whenEnabled: next })}
          label="Gate this rule on a reading"
          hint="Off stores no condition at all: the rule is always active."
        />

        {draft.whenEnabled ? (
          <div className="stack stack--tight">
            <div className="form-grid">
              <Field
                label="Condition sensor"
                htmlFor="rule-when-source"
                hint={
                  sources.length === 0
                    ? 'No readable sensor is available, so a condition cannot be checked.'
                    : 'Read like any other sensor: if it goes missing or stale, the rule follows its sensor-missing fallback.'
                }
              >
                <select
                  id="rule-when-source"
                  className="select"
                  value={draft.whenSourceKey}
                  onChange={(event) => update({ whenSourceKey: event.target.value })}
                >
                  <option value="">— choose a sensor —</option>
                  {sources.map((ref) => (
                    <option key={sensorKey(ref)} value={sensorKey(ref)}>
                      {ref.device_name} · {ref.capability.name} ({ref.capability.unit})
                      {ref.current !== undefined ? ` — now ${formatValue(ref.current, ref.capability.unit)}` : ''}
                    </option>
                  ))}
                </select>
              </Field>

              <Field label="Comparison" htmlFor="rule-when-op" hint="How the reading is compared">
                <select
                  id="rule-when-op"
                  className="select"
                  value={draft.whenOp}
                  onChange={(event) => update({ whenOp: event.target.value as Comparator })}
                >
                  {COMPARATORS.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.symbol} — {option.words}
                    </option>
                  ))}
                </select>
              </Field>

              <Field
                label={`Threshold${thresholdSuffix(whenUnit) ? ` (${thresholdSuffix(whenUnit)})` : ''}`}
                htmlFor="rule-when-value"
                hint="In the unit of the condition sensor."
              >
                <input
                  id="rule-when-value"
                  className="input input--number"
                  type="number"
                  step={thresholdStep(whenUnit)}
                  min={whenRef?.capability.min}
                  max={whenRef?.capability.max}
                  value={draft.whenValue}
                  onChange={(event) => update({ whenValue: event.target.value })}
                />
              </Field>

              <Field
                label="Otherwise"
                htmlFor="rule-when-otherwise"
                hint="What the rule drives while the condition does not hold."
              >
                <select
                  id="rule-when-otherwise"
                  className="select"
                  value={draft.whenOtherwise}
                  onChange={(event) =>
                    update({ whenOtherwise: event.target.value as OtherwiseChoice })
                  }
                >
                  <option value="safe_default">
                    Safe default — the fail-safe duty ({formatValue(failSafePercent, 'percent')})
                  </option>
                  <option value="fixed">A fixed percentage</option>
                </select>
              </Field>

              {draft.whenOtherwise === 'fixed' ? (
                <Field
                  label="Otherwise duty (%)"
                  htmlFor="rule-when-fixed"
                  hint="Driven whenever the condition is false."
                >
                  <input
                    id="rule-when-fixed"
                    className="input input--number"
                    type="number"
                    min={0}
                    max={100}
                    step="1"
                    value={draft.whenFixedPercent}
                    onChange={(event) => update({ whenFixedPercent: event.target.value })}
                  />
                </Field>
              ) : null}
            </div>

            {isEqualityComparator(draft.whenOp) ? (
              <InlineNotice tone="warn" title="Equality on an analog reading is fragile">
                <p>
                  {comparatorSymbol(draft.whenOp)} compares within a tiny tolerance, so a reading
                  that wobbles by 0.1 will flip the gate. “≥” or “≤” usually say what you mean.
                  This is a warning only — the rule can still be saved.
                </p>
              </InlineNotice>
            ) : null}

            <p className="small" data-testid="gate-summary">
              {gate.condition ? (
                describeGate(gate.condition, whenUnit, failSafePercent)
              ) : (
                <span className="muted">
                  Fill in the condition to see how the gate reads.
                </span>
              )}
            </p>
          </div>
        ) : (
          <p className="small muted" data-testid="gate-summary">
            No condition: this rule steers its target from the curve whenever it is enabled. The
            emergency temperature supervisor always overrides it.
          </p>
        )}

        <SectionHead
          title="Curve"
          hint="Input (temperature) to output (fan duty). Rows are sorted by input on save."
          actions={
            <div className="row">
              <button
                type="button"
                className="btn btn--sm"
                onClick={() => update({ curveRows: curveToRows(GPU_TEMPLATE_CURVE) })}
              >
                Load GPU template
              </button>
              <button
                type="button"
                className="btn btn--sm"
                onClick={() => update({ curveRows: curveToRows(CHASSIS_TEMPLATE_CURVE) })}
              >
                Load chassis template
              </button>
            </div>
          }
        />

        <div className="curve-editor">
          <div className="stack stack--tight">
            <div className="curve-row" aria-hidden="true">
              <span className="tiny dim">#</span>
              <span className="tiny dim">Input °C</span>
              <span className="tiny dim">Output %</span>
              <span />
            </div>
            <div className="curve-rows">
              {draft.curveRows.map((row, position) => (
                <div className="curve-row" key={row.key}>
                  <span className="curve-row__index">{position + 1}</span>
                  <label className="visually-hidden" htmlFor={`curve-in-${row.key}`}>
                    Control point {position + 1} input
                  </label>
                  <input
                    id={`curve-in-${row.key}`}
                    className="input"
                    type="number"
                    step="1"
                    value={row.input}
                    onChange={(event) =>
                      update({
                        curveRows: draft.curveRows.map((item) =>
                          item.key === row.key ? { ...item, input: event.target.value } : item,
                        ),
                      })
                    }
                  />
                  <label className="visually-hidden" htmlFor={`curve-out-${row.key}`}>
                    Control point {position + 1} output
                  </label>
                  <input
                    id={`curve-out-${row.key}`}
                    className="input"
                    type="number"
                    step="1"
                    min={0}
                    max={100}
                    value={row.output}
                    onChange={(event) =>
                      update({
                        curveRows: draft.curveRows.map((item) =>
                          item.key === row.key ? { ...item, output: event.target.value } : item,
                        ),
                      })
                    }
                  />
                  <button
                    type="button"
                    className="btn btn--sm btn--ghost"
                    aria-label={`Remove control point ${position + 1}`}
                    onClick={() =>
                      update({ curveRows: draft.curveRows.filter((item) => item.key !== row.key) })
                    }
                    disabled={draft.curveRows.length <= 2}
                  >
                    ×
                  </button>
                </div>
              ))}
            </div>
            <button
              type="button"
              className="btn btn--sm"
              onClick={() =>
                update({
                  curveRows: [
                    ...draft.curveRows,
                    {
                      key: `row-new-${Date.now()}`,
                      input: String(
                        Math.round(
                          (numericCurve[numericCurve.length - 1]?.[0] ?? 40) + 5,
                        ),
                      ),
                      output: String(numericCurve[numericCurve.length - 1]?.[1] ?? 50),
                    },
                  ],
                })
              }
            >
              Add control point
            </button>
          </div>

          <div>
            <CurvePreview
              curve={numericCurve}
              current={liveInput}
              label={`Preview of the curve for ${draft.name || 'the new rule'}`}
              height={200}
            />
            {liveInput === undefined ? (
              <p className="tiny dim">
                The source is currently unavailable, so no live point is marked. The rule will follow
                its fallback when it runs.
              </p>
            ) : null}
          </div>
        </div>

        <SectionHead title="Behaviour and fallbacks" />
        <div className="form-grid">
          <Field label="Hysteresis (°C)" htmlFor="rule-hysteresis" hint="Dead zone around the curve to stop oscillation">
            <input
              id="rule-hysteresis"
              className="input input--number"
              type="number"
              min={0}
              step="0.5"
              value={draft.hysteresis}
              onChange={(event) => update({ hysteresis: event.target.value })}
            />
          </Field>
          <Field label="Deadband (% output)" htmlFor="rule-deadband" hint="Ignore changes smaller than this">
            <input
              id="rule-deadband"
              className="input input--number"
              type="number"
              min={0}
              step="0.5"
              value={draft.deadband}
              onChange={(event) => update({ deadband: event.target.value })}
            />
          </Field>
          <Field label="Update interval (ms)" htmlFor="rule-interval" hint="Minimum time between writes">
            <input
              id="rule-interval"
              className="input input--number"
              type="number"
              min={250}
              step="250"
              value={draft.updateInterval}
              onChange={(event) => update({ updateInterval: event.target.value })}
            />
          </Field>
          <Field label="Minimum output (%)" htmlFor="rule-min" hint="Leave empty for no lower limit">
            <input
              id="rule-min"
              className="input input--number"
              type="number"
              min={0}
              max={100}
              value={draft.minOutput}
              onChange={(event) => update({ minOutput: event.target.value })}
            />
          </Field>
          <Field label="Maximum output (%)" htmlFor="rule-max" hint="Leave empty for no upper limit">
            <input
              id="rule-max"
              className="input input--number"
              type="number"
              min={0}
              max={100}
              value={draft.maxOutput}
              onChange={(event) => update({ maxOutput: event.target.value })}
            />
          </Field>
          <Field label="Sensor timeout (s)" htmlFor="rule-timeout" hint="After this, the fallback applies">
            <input
              id="rule-timeout"
              className="input input--number"
              type="number"
              min={1}
              value={draft.sensorTimeout}
              onChange={(event) => update({ sensorTimeout: event.target.value })}
            />
          </Field>
          <Field label="If the sensor is missing" htmlFor="rule-fb-missing">
            <select
              id="rule-fb-missing"
              className="select"
              value={draft.fallbackMissing}
              onChange={(event) =>
                update({ fallbackMissing: event.target.value as FallbackChoice })
              }
            >
              <option value="hold">Hold the last output</option>
              <option value="safe_default">Use the fail-safe output</option>
              <option value="release">Release control to the hardware</option>
              <option value="fixed">Force a fixed percentage</option>
            </select>
          </Field>
          <Field label="If a write fails" htmlFor="rule-fb-write">
            <select
              id="rule-fb-write"
              className="select"
              value={draft.fallbackWrite}
              onChange={(event) => update({ fallbackWrite: event.target.value as FallbackChoice })}
            >
              <option value="hold">Hold the last output</option>
              <option value="safe_default">Use the fail-safe output</option>
              <option value="release">Release control to the hardware</option>
              <option value="fixed">Force a fixed percentage</option>
            </select>
          </Field>
          <Field label="Fixed fallback (%)" htmlFor="rule-fixed" hint="Used when a fallback forces a value">
            <input
              id="rule-fixed"
              className="input input--number"
              type="number"
              min={0}
              max={100}
              value={draft.fixedPercent}
              onChange={(event) => update({ fixedPercent: event.target.value })}
            />
          </Field>
        </div>

        {saved && errors.length === 0 && failure === null ? (
          <InlineNotice tone="ok" title="Rule saved">
            <p>The runtime picked it up and will evaluate it on the next automation tick.</p>
          </InlineNotice>
        ) : null}

        <div className="row">
          <button
            type="button"
            className="btn btn--primary"
            onClick={() => void handleSave()}
            disabled={busy}
          >
            {busy ? 'Saving…' : 'Check & save'}
          </button>
          <button type="button" className="btn btn--ghost" onClick={onCancel}>
            Cancel
          </button>
          <span className="small dim">
            Saving first calls check_rule on the backend; blocking errors stop the save, warnings do
            not.
          </span>
        </div>
      </div>
    </Panel>
  );
}
