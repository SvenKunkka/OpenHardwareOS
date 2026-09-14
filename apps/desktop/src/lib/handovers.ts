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
  // Recovered from a previous session and not yet checked against this machine: the
  // responsibility is real, the device is unknown, so it is never presented as fine.
  needs_verification: 'warn',
  // A rule claims the channel but has not driven it, so nobody is protecting it and
  // the runtime is deliberately not writing under the claimant either.
  awaiting_owner: 'warn',
  // Out of attempts, or waiting for ever: still owed, and now it needs a human.
  failed: 'danger',
  confirmed: 'ok',
  superseded: 'neutral',
};

/** Short label, safe inside a badge. */
export function handoverStateLabel(state: HandoverState): string {
  switch (state) {
    case 'pending':
      return 'pending';
    case 'needs_verification':
      return 'not verified yet';
    case 'awaiting_owner':
      return 'waiting for its new owner';
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
    case 'needs_verification':
      return 'This channel was left owed the fail-safe duty by a previous session of the app and has not been checked against this machine yet. The runtime verifies the device, its capabilities and the current owner before applying the safety policy — it never replays what the previous session was about to do.';
    case 'awaiting_owner':
      return 'A rule targets this channel but has not driven it, so the runtime is not writing under it — the channel is not known to be protected. It will be handed over as soon as that rule takes control, gives the channel up, or the wait runs out.';
    case 'failed':
      return 'Every attempt to hand this channel to the fail-safe duty failed — or the rule that claimed it never took control — and the runtime has stopped retrying. This channel needs you to act: fix or disable the rule named here, then retry the handover.';
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
    case 'needs_verification':
    case 'awaiting_owner':
    case 'failed':
      return true;
    case 'confirmed':
    case 'superseded':
      return false;
  }
}

/**
 * The sentence for a handover that is waiting on a claimant: who claims the channel,
 * for how long, and the fact that the channel is unprotected meanwhile.
 */
export function handoverOwnerNote(report: HandoverReport): string | undefined {
  if (report.state !== 'awaiting_owner') return undefined;
  const who = report.claimant ? `\`${report.claimant}\`` : 'another rule';
  return `${who} targets this channel but has not driven it (${report.claimed_ticks} tick(s) waiting), so the runtime is not writing under it: the channel is not known to be protected.`;
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
