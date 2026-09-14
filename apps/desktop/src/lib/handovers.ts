/**
 * The shared presentation contract for a `HandoverReport`.
 *
 * A handover exists because a rule stopped driving a channel — retargeted,
 * disabled, deleted, or its file removed — and the runtime owes that channel to
 * the fail-safe duty. The distinction this module protects is *owed* versus
 * *settled*: while a handover is `pending` or `failed` the channel is still at
 * whatever the abandoned rule last said, and the fail-safe duty is not confirmed
 * in force. Nothing in that state may be rendered as fine, and a `failed` one is
 * worse than a `pending` one because nothing will try again on its own.
 *
 * Every map and switch is exhaustive over `HandoverState` on purpose: adding a
 * state to the wire type has to break the build until it is handled.
 */

import type { Tone } from '../components/primitives';
import type { HandoverReport, HandoverState } from '../types';

/** Tone per state: an owed handover warns, a parked one is an error. */
export const HANDOVER_STATE_TONE: Record<HandoverState, Tone> = {
  // Queued, still being retried, and not confirmed yet.
  pending: 'warn',
  // Out of attempts: still owed, and now it needs a human.
  failed: 'danger',
  confirmed: 'ok',
  superseded: 'neutral',
};

/** Short label, safe inside a badge. */
export function handoverStateLabel(state: HandoverState): string {
  switch (state) {
    case 'pending':
      return 'pending';
    case 'failed':
      return 'failed';
    case 'confirmed':
      return 'confirmed';
    case 'superseded':
      return 'superseded';
  }
}

/** The full sentence behind the badge, used as its `title` and in the row. */
export function handoverStateHint(state: HandoverState): string {
  switch (state) {
    case 'pending':
      return 'The runtime is still trying to hand this channel to the fail-safe duty. Until that is confirmed the channel is not known to be protected.';
    case 'failed':
      return 'Every attempt to hand this channel to the fail-safe duty failed, and the runtime has stopped retrying it. This channel needs you to act: retry the handover, or fix the reason the write failed.';
    case 'confirmed':
      return 'The fail-safe duty was written and the device confirmed it, so this channel is protected.';
    case 'superseded':
      return 'Another rule drives this channel now, so handing it to the fail-safe duty would have fought that rule. Nothing was written.';
  }
}

/** True while the fail-safe duty is still owed to the channel. */
export function handoverIsOwed(state: HandoverState): boolean {
  switch (state) {
    case 'pending':
    case 'failed':
      return true;
    case 'confirmed':
    case 'superseded':
      return false;
  }
}

/** `device/capability`, the channel the handover is about. */
export function handoverChannel(report: HandoverReport): string {
  return `${report.device}/${report.capability}`;
}

/**
 * The error text worth showing: the cause first, then the latest symptom only
 * when it says something the cause did not.
 */
export function handoverErrors(report: HandoverReport): { cause?: string; latest?: string } {
  const cause = report.first_error ?? undefined;
  const latest = report.last_error && report.last_error !== cause ? report.last_error : undefined;
  return { cause, latest };
}
