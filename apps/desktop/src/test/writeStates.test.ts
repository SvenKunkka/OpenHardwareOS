/**
 * Two states the backend can now produce, and the editor must not contradict it:
 *
 * 1. `unconfirmed` — the device accepted a write but never confirmed the value.
 *    It is neither success nor failure, and the badge has to say so.
 * 2. `release` — a fallback action no adapter in this build can perform. The
 *    editor must not offer it, and a rule file that still contains it must load as
 *    the safe default the runtime actually runs.
 */

import { describe, expect, it } from 'vitest';
import { ruleStatusHint, ruleStatusLabel } from '../lib/format';
import { isSimpleFallback } from '../lib/curve';
import type { FallbackAction } from '../types';

describe('the unconfirmed rule status', () => {
  it('is labelled as unconfirmed, not as applied or as an error', () => {
    expect(ruleStatusLabel('unconfirmed')).toBe('Unconfirmed');
    expect(ruleStatusLabel('applied')).toBe('Applied');
  });

  it('explains what is unknown and what happens next', () => {
    const hint = ruleStatusHint('unconfirmed');
    expect(hint).toBeDefined();
    expect(hint).toMatch(/did not confirm/i);
    expect(hint).toMatch(/retry/i);
    expect(hint).toMatch(/fail-safe/i);
  });
});

describe('the release fallback action', () => {
  it('is never offered as an editor choice', () => {
    // `isSimpleFallback` is what feeds the `<select>`; a value it cannot return is
    // a value the user cannot choose.
    const actions: FallbackAction[] = ['hold', 'safe_default', 'release', { fixed: { percent: 40 } }];
    const choices = actions.map((action) => isSimpleFallback(action));
    expect(choices).not.toContain('release');
  });

  it('shows a legacy release rule as the safe default the runtime runs', () => {
    // The backend substitutes `safe_default` when it loads such a file, so the form
    // must show the same thing rather than a value that cannot be saved.
    expect(isSimpleFallback('release')).toBe('safe_default');
  });

  it('leaves the other actions alone', () => {
    expect(isSimpleFallback('hold')).toBe('hold');
    expect(isSimpleFallback('safe_default')).toBe('safe_default');
    expect(isSimpleFallback({ fixed: { percent: 55 } })).toBe('fixed');
  });
});
