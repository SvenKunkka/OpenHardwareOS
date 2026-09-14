/**
 * The one place that turns a `WriteReport` into something the UI may show.
 *
 * It exists for `unconfirmed`: the device accepted the request but never
 * confirmed the value it now holds. That state is neither success nor failure,
 * so its tone is never `ok`, its label never claims the value is in force, and a
 * report that carries no `applied` value is described as unknown instead of
 * being rendered as "nothing", "not applied", a fake `0`, or the requested
 * value dressed up as the outcome.
 *
 * Every map and switch here is exhaustive over `WriteStatus` on purpose: adding
 * a status to the wire type has to break the build until it is handled.
 */

import type { Tone } from '../components/primitives';
import { formatValue } from './format';
import type { Unit, WriteReport, WriteStatus } from '../types';

/** Rendered wherever a report carries no `applied` value. */
export const UNKNOWN_APPLIED = 'value unknown — not confirmed';

/** Rendered for a refused write: the value is known not to have changed. */
export const UNCHANGED_APPLIED = 'unchanged — the write was refused';

/** Tone per status: `unconfirmed` warns, it never passes as success. */
export const WRITE_STATUS_TONE: Record<WriteStatus, Tone> = {
  applied: 'ok',
  simulated: 'warn',
  // Accepted but not read back. Unknown deserves attention, so: warn, never ok.
  unconfirmed: 'warn',
  rejected: 'danger',
};

/** Short label, safe inside a badge or a table cell. */
export function writeStatusLabel(status: WriteStatus): string {
  switch (status) {
    case 'applied':
      return 'applied';
    case 'simulated':
      return 'simulated';
    case 'unconfirmed':
      return 'requested, not confirmed';
    case 'rejected':
      return 'refused';
  }
}

/** The full sentence behind the badge, used as its `title`. */
export function writeStatusHint(status: WriteStatus): string {
  switch (status) {
    case 'applied':
      return 'The device confirmed the value it now holds.';
    case 'simulated':
      return 'Nothing was sent to hardware: this was a dry run or a simulated device.';
    case 'unconfirmed':
      return 'The device accepted the request but never confirmed the value it now holds, so what is actually in force is unknown. The runtime retries the write; after three consecutive failures the fail-safe duty is applied.';
    case 'rejected':
      return 'The write was refused, so the value did not change. The backend’s reason is shown next to it.';
  }
}

/**
 * The value that is now in force, or an explicit statement of what is unknown.
 * Never a placeholder that could be mistaken for a real reading.
 */
export function appliedValueText(report: WriteReport, unit: Unit): string {
  switch (report.status) {
    case 'applied':
    case 'simulated':
      // A value the backend did not report is still unknown — do not invent one.
      return report.applied === undefined ? UNKNOWN_APPLIED : formatValue(report.applied, unit);
    case 'unconfirmed':
      // The backend withholds `applied` here on purpose: the value was never read
      // back, so showing the requested one would state something untrue.
      return UNKNOWN_APPLIED;
    case 'rejected':
      return report.applied === undefined
        ? UNCHANGED_APPLIED
        : formatValue(report.applied, unit);
  }
}

/**
 * What is actually known about the write, in one sentence, including the
 * backend's own reason verbatim when it gave one.
 */
export function writeStatusSentence(report: WriteReport, unit: Unit): string {
  const reason = report.detail ? ` ${report.detail}` : '';
  switch (report.status) {
    case 'applied':
      return `The device confirmed the write: it now holds ${appliedValueText(report, unit)}.${reason}`;
    case 'simulated':
      return `Nothing was sent to hardware (dry run or simulated device), so no real value changed.${reason}`;
    case 'unconfirmed':
      return `The request was accepted and sent, but the device never confirmed the resulting value, so what it now holds is unknown.${reason}`;
    case 'rejected':
      return `The runtime refused the write, so the value did not change.${reason}`;
  }
}

/** One audit line: `device · capability: requested → outcome (status) — reason`. */
export function describeWriteReport(report: WriteReport): string {
  const detail = report.detail ? ` — ${report.detail}` : '';
  return `${report.device_name} · ${report.capability_name}: ${formatValue(report.requested, 'none')} → ${appliedValueText(report, 'none')} (${report.status})${detail}`;
}
