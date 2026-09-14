/**
 * The gate and conflict automation kinds must read like events, not like raw
 * identifiers, and a rule standing down must not be decoded as a problem.
 */

import { describe, expect, it } from 'vitest';
import { decodeEvent } from '../lib/events';

describe('automation events in the live feed', () => {
  it('decodes rule_gated as an informational stand-down', () => {
    const entry = decodeEvent({
      type: 'automation',
      rule_id: 'rule-gpu-load',
      kind: 'rule_gated',
      detail: 'gpu.mock.0/load.gpu > 60 is false; drive 70 %',
      at_ms: 1_700_000_000_000,
    });

    expect(entry.category).toBe('automation');
    expect(entry.level).toBe('info');
    expect(entry.message).toBe('Rule standing down: its “when” condition is not met.');
    expect(entry.detail).toBe('rule-gpu-load — gpu.mock.0/load.gpu > 60 is false; drive 70 %');
  });

  it('decodes the conflict kinds as warnings, naming the rule', () => {
    const disabled = decodeEvent({
      type: 'automation',
      rule_id: 'rule-gpu-fan-b',
      kind: 'rule_conflict_disabled',
      detail: 'Fan A already drives fan.mock.0/fan.speed_percent',
      at_ms: 1_700_000_000_000,
    });
    const skipped = decodeEvent({
      type: 'automation',
      rule_id: 'rule-gpu-fan-b',
      kind: 'rule_conflict_skipped',
      detail: 'Fan A already drives fan.mock.0/fan.speed_percent',
      at_ms: 1_700_000_000_000,
    });

    expect(disabled.level).toBe('warn');
    expect(skipped.level).toBe('warn');
    expect(disabled.message).toContain('another enabled rule already drives its output');
    expect(skipped.detail).toContain('rule-gpu-fan-b');
  });

  it('keeps an unknown automation kind readable', () => {
    const entry = decodeEvent({
      type: 'automation',
      kind: 'rule_saved',
      detail: 'Fan A saved',
      at_ms: 1_700_000_000_000,
    });

    expect(entry.level).toBe('info');
    expect(entry.message).toBe('Automation rule_saved.');
    expect(entry.detail).toBe('Fan A saved');
  });
});
