/**
 * Decoding of the tagged `runtime-event` union into feed entries.
 *
 * Colour is never the only signal: every entry carries a text level label and a
 * category, both rendered as text in the activity feeds.
 */

import type { ActivityEntry, LogLevel, RuntimeEvent } from '../types';
import { formatValue, humanise } from './format';

let sequence = 0;

/**
 * The automation kinds that deserve their own sentence in the feed. Everything
 * else (`rule_applied`, `rule_saved`, …) is already clear from its own name.
 */
const AUTOMATION_KIND: Record<string, { level: LogLevel; label: string }> = {
  rule_gated: {
    level: 'info',
    label: 'Rule standing down: its “when” condition is not met',
  },
  rule_gate_open: {
    level: 'info',
    label: 'Rule steering again: its “when” condition holds',
  },
  rule_conflict_disabled: {
    level: 'warn',
    label: 'Rule disabled: another enabled rule already drives its output',
  },
  rule_conflict_skipped: {
    level: 'warn',
    label: 'Rule skipped: another enabled rule already drives its output',
  },
};

function levelFor(event: RuntimeEvent): LogLevel {
  switch (event.type) {
    case 'safety_triggered':
    case 'write_rejected':
      return 'warn';
    case 'runtime_stopped':
    case 'device_removed':
      return 'warn';
    case 'log':
      return (event.level as LogLevel) ?? 'info';
    default:
      return 'info';
  }
}

/** `runtime-event` -> one activity-feed row. */
export function decodeEvent(event: RuntimeEvent): ActivityEntry {
  sequence += 1;
  const base = { key: `evt-${sequence}`, level: levelFor(event) } as const;

  switch (event.type) {
    case 'runtime_started':
      return {
        ...base,
        at_ms: event.at_ms,
        category: 'runtime',
        message: `Monitoring started with ${event.device_count} device${event.device_count === 1 ? '' : 's'}.`,
      };
    case 'runtime_stopped':
      return { ...base, at_ms: event.at_ms, category: 'runtime', message: 'Monitoring stopped.' };
    case 'adapter_status_changed':
      return {
        ...base,
        at_ms: event.status.status.checked_at_ms,
        category: 'adapter',
        message: `Adapter “${event.status.info.name}” is ${humanise(event.status.status.state)}.`,
        detail:
          event.status.status.detail ??
          (event.status.status.reason ? humanise(event.status.status.reason) : undefined),
      };
    case 'device_added':
      return {
        ...base,
        at_ms: Date.now(),
        category: 'device',
        device_id: event.device.id,
        message: `Device added: ${event.device.name}.`,
        detail: `${humanise(event.device.type)} via ${event.device.adapter}`,
      };
    case 'device_removed':
      return {
        ...base,
        at_ms: Date.now(),
        category: 'device',
        device_id: event.device_id,
        message: `Device removed: ${event.name}.`,
        detail: humanise(event.reason),
      };
    case 'device_status_changed':
      return {
        ...base,
        at_ms: Date.now(),
        category: 'device',
        device_id: event.device_id,
        message: `${event.device_id} is now ${humanise(event.status)}.`,
      };
    case 'device_enabled_changed':
      return {
        ...base,
        at_ms: Date.now(),
        category: 'device',
        device_id: event.device_id,
        message: `${event.device_id} was ${event.enabled ? 'enabled' : 'disabled'}.`,
      };
    case 'state_changed':
      return {
        ...base,
        at_ms: event.state.timestamp_ms,
        category: 'device',
        device_id: event.state.device,
        message: `State updated for ${event.state.device} (${event.state.readings.length} readings).`,
        detail: event.state.message,
      };
    case 'reading_changed':
      return {
        ...base,
        at_ms: Date.now(),
        category: 'device',
        device_id: event.device_id,
        message: `${event.capability} → ${event.value === undefined ? 'unavailable' : formatValue(event.value, 'none')}`,
        detail:
          event.previous === undefined ? undefined : `previous ${formatValue(event.previous, 'none')}`,
      };
    case 'write_performed':
      return {
        ...base,
        at_ms: event.report.at_ms,
        category: 'write',
        device_id: event.report.device_id,
        message: `${event.report.capability_name} write ${event.report.status}${event.report.clamped ? ' (clamped)' : ''}.`,
        detail: event.report.detail ?? `requested ${describe(event.report.requested)}`,
      };
    case 'write_rejected':
      return {
        ...base,
        at_ms: Date.now(),
        category: 'write',
        device_id: event.device_id,
        message: `Write to ${event.capability} was rejected (${event.error_code}).`,
        detail: event.detail,
      };
    case 'safety_triggered':
      return {
        ...base,
        at_ms: event.at_ms,
        category: 'safety',
        message: `Safety intervened: ${event.kind}.`,
        detail: event.detail,
      };
    case 'automation': {
      const kind = AUTOMATION_KIND[event.kind];
      const detail = event.rule_id
        ? event.detail
          ? `${event.rule_id} — ${event.detail}`
          : event.rule_id
        : event.detail;
      return {
        ...base,
        level: kind?.level ?? base.level,
        at_ms: event.at_ms,
        category: 'automation',
        message: kind ? `${kind.label}.` : `Automation ${event.kind}.`,
        detail,
      };
    }
    case 'log':
      return {
        ...base,
        at_ms: event.at_ms,
        category: 'log',
        message: event.message,
      };
    default:
      return { ...base, at_ms: Date.now(), category: 'runtime', message: 'Unknown event.' };
  }
}

function describe(value: number | boolean | string): string {
  if (typeof value === 'number') return String(value);
  if (typeof value === 'boolean') return value ? 'on' : 'off';
  return value;
}
