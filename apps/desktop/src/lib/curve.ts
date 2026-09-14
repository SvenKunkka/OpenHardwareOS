/**
 * Fan-curve maths shared by the automation editor, the curve preview and the
 * demo backend. A curve is an array of `[input, output]` control points.
 */

import type { ControlPointTuple, FallbackAction, Source } from '../types';

/** The documented GPU curve used by the "load template" button. */
export const GPU_TEMPLATE_CURVE: ControlPointTuple[] = [
  [40, 20],
  [60, 35],
  [70, 50],
  [80, 80],
  [85, 100],
];

/** A gentler starting point for a CPU/chassis fan. */
export const CHASSIS_TEMPLATE_CURVE: ControlPointTuple[] = [
  [35, 25],
  [50, 35],
  [65, 55],
  [80, 85],
  [90, 100],
];

/** Strictly increasing by input, so the backend always receives a valid curve. */
export function sortCurve(curve: ControlPointTuple[]): ControlPointTuple[] {
  return [...curve].sort((a, b) => a[0] - b[0]);
}

/**
 * Linear interpolation between the two surrounding points, clamped outside the
 * curve's input range.
 */
export function evaluateCurve(curve: ControlPointTuple[], input: number): number {
  if (curve.length === 0) return 0;
  const points = sortCurve(curve);
  const first = points[0];
  const last = points[points.length - 1];
  if (!first || !last) return 0;
  if (input <= first[0]) return first[1];
  if (input >= last[0]) return last[1];
  for (let index = 1; index < points.length; index += 1) {
    const previous = points[index - 1];
    const current = points[index];
    if (!previous || !current) continue;
    if (input <= current[0]) {
      const span = current[0] - previous[0];
      if (span === 0) return current[1];
      const ratio = (input - previous[0]) / span;
      return previous[1] + ratio * (current[1] - previous[1]);
    }
  }
  return last[1];
}

export function curveInputBounds(curve: ControlPointTuple[]): { min: number; max: number } {
  if (curve.length === 0) return { min: 0, max: 100 };
  let min = Number.POSITIVE_INFINITY;
  let max = Number.NEGATIVE_INFINITY;
  for (const point of curve) {
    if (point[0] < min) min = point[0];
    if (point[0] > max) max = point[0];
  }
  return { min, max: max === min ? min + 1 : max };
}

export function curveOutputBounds(curve: ControlPointTuple[]): { min: number; max: number } {
  if (curve.length === 0) return { min: 0, max: 100 };
  let min = Number.POSITIVE_INFINITY;
  let max = Number.NEGATIVE_INFINITY;
  for (const point of curve) {
    if (point[1] < min) min = point[1];
    if (point[1] > max) max = point[1];
  }
  return { min, max: max === min ? min + 1 : max };
}

export function describeSource(source: Source): string {
  if ('device' in source) return `${source.device} · ${source.capability}`;
  return `${source.aggregate.toUpperCase()} of ${source.sensors.length} sensor${source.sensors.length === 1 ? '' : 's'}`;
}

export function describeFallbackAction(action: FallbackAction): string {
  if (typeof action === 'string') {
    switch (action) {
      case 'hold':
        return 'Hold the last output';
      case 'safe_default':
        return 'Use the fail-safe output';
      case 'release':
        return 'Release control to the hardware';
      default:
        return action;
    }
  }
  return `Force ${action.fixed.percent} %`;
}

/**
 * The action as a single choice, for the editor's `<select>`.
 *
 * `release` is a legal wire value (an old rule file may contain it) but it is not
 * something this build can perform, so a rule that loaded with it is shown as the
 * fail-safe default — which is what the runtime actually runs after sanitising the
 * file. Offering it as an option would let a user configure an action that does
 * nothing.
 */
export function isSimpleFallback(action: FallbackAction): 'hold' | 'safe_default' | 'fixed' {
  if (typeof action !== 'string') return 'fixed';
  return action === 'release' ? 'safe_default' : action;
}

/** An unsaved-but-valid id; the backend is free to replace it. */
export function newRuleId(): string {
  const random = Math.random().toString(36).slice(2, 8);
  return `rule-${Date.now().toString(36)}-${random}`;
}
