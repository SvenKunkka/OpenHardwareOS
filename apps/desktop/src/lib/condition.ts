/**
 * The `when` numeric gate: comparators, threshold units and the plain-language
 * summary of a gate.
 *
 * The backend is the authority on all of this (`Comparator::symbol`,
 * `Condition::label`); this module mirrors it so the form and the rule card can
 * speak the same language without an extra round trip.
 */

import { formatValue, unitLabel } from './format';
import { deviceById, findCapability } from './devices';
import type {
  CapabilityIndex,
  Comparator,
  Condition,
  CapabilityRef,
  OtherwiseAction,
  RuntimeSnapshot,
  Unit,
} from '../types';

export interface ComparatorOption {
  value: Comparator;
  /** The symbol the backend prints, e.g. `≥`. */
  symbol: string;
  /** How the comparison reads as a word, for screen readers. */
  words: string;
}

/** The six comparators, in the order the backend declares them. */
export const COMPARATORS: readonly ComparatorOption[] = [
  { value: 'gt', symbol: '>', words: 'is greater than' },
  { value: 'gte', symbol: '≥', words: 'is at least' },
  { value: 'lt', symbol: '<', words: 'is less than' },
  { value: 'lte', symbol: '≤', words: 'is at most' },
  { value: 'eq', symbol: '=', words: 'equals' },
  { value: 'ne', symbol: '≠', words: 'is not equal to' },
];

export function comparatorSymbol(op: Comparator): string {
  return COMPARATORS.find((option) => option.value === op)?.symbol ?? op;
}

export function comparatorWords(op: Comparator): string {
  return COMPARATORS.find((option) => option.value === op)?.words ?? op;
}

/** `eq`/`ne` on an analog reading: `check_rule` warns, and so does the form. */
export function isEqualityComparator(op: Comparator): boolean {
  return op === 'eq' || op === 'ne';
}

/**
 * Step of the threshold input for a unit: temperatures are fine-grained, rpm and
 * percent are whole numbers.
 */
export function thresholdStep(unit: Unit): number {
  return unit === 'celsius' || unit === 'fahrenheit' ? 0.1 : 1;
}

/** True when the threshold should be offered with a unit suffix at all. */
export function thresholdSuffix(unit: Unit): string {
  return unitLabel(unit);
}

export type OtherwiseChoice = 'safe_default' | 'fixed';

export function otherwiseChoice(action: OtherwiseAction): OtherwiseChoice {
  return typeof action === 'string' ? 'safe_default' : 'fixed';
}

export function otherwiseAction(choice: OtherwiseChoice, fixedPercent: string): OtherwiseAction {
  return choice === 'fixed'
    ? { fixed: { percent: Number(fixedPercent) || 0 } }
    : 'safe_default';
}

/**
 * The duty the gate falls back to, as a sentence fragment:
 * `fall back to the safe default (70 %)` or `drive 40 %`.
 */
export function describeOtherwise(action: OtherwiseAction, failSafePercent?: number): string {
  if (typeof action === 'string') {
    const percent =
      failSafePercent === undefined ? undefined : ` (${formatValue(failSafePercent, 'percent')})`;
    return `fall back to the safe default${percent ?? ''}`;
  }
  return `drive ${formatValue(action.fixed.percent, 'percent')} instead`;
}

/** `load.gpu > 60 %` — the condition without the "when". */
export function describeConditionTest(condition: Condition, unit: Unit): string {
  const source = `${condition.source.device}/${condition.source.capability}`;
  const threshold = formatValue(condition.value, unit);
  return `${source} ${comparatorSymbol(condition.op)} ${threshold}`;
}

/**
 * One line a person can check before saving, e.g.
 * `Only while gpu.mock.0/load.gpu > 60 %; otherwise fall back to the safe default (70 %).`
 */
export function describeGate(
  condition: Condition,
  unit: Unit,
  failSafePercent?: number,
): string {
  return `Only while ${describeConditionTest(condition, unit)}; otherwise ${describeOtherwise(
    condition.otherwise,
    failSafePercent,
  )}.`;
}

/** Compact chip text for a rule card, e.g. `when load.gpu > 60 %`. */
export function describeGateChip(condition: Condition, unit: Unit): string {
  const threshold = formatValue(condition.value, unit);
  return `when ${condition.source.capability} ${comparatorSymbol(condition.op)} ${threshold}`;
}

/** The unit of a capability, looked up in the capability index. */
export function unitFromIndex(
  index: CapabilityIndex | null | undefined,
  device: string,
  capability: string,
): Unit | undefined {
  return refFromIndex(index, device, capability)?.capability.unit;
}

export function refFromIndex(
  index: CapabilityIndex | null | undefined,
  device: string,
  capability: string,
): CapabilityRef | undefined {
  return index?.sources.find((ref) => ref.device_id === device && ref.capability.id === capability);
}

/**
 * The unit of a capability from the pushed snapshot. Used where only the
 * snapshot (not the capability index) is at hand, such as a rule card.
 */
export function unitFromSnapshot(
  snapshot: RuntimeSnapshot,
  device: string,
  capability: string,
): Unit | undefined {
  const view = deviceById(snapshot, device);
  return view ? findCapability(view, capability)?.unit : undefined;
}

/** Parse the `device|capability` key the pickers use. */
export function parseSensorKey(key: string): { device: string; capability: string } | undefined {
  const [device, capability] = key.split('|');
  return device && capability ? { device, capability } : undefined;
}

/**
 * The `device|capability` key of a picker option. Both the rule source picker and
 * the condition source picker use it, so their values always line up.
 */
export function sensorKey(ref: CapabilityRef): string {
  return `${ref.device_id}|${ref.capability.id}`;
}
