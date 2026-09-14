import { useMemo } from 'react';
import { useElementWidth } from '../hooks/useElementWidth';
import { curveInputBounds, curveOutputBounds, evaluateCurve, sortCurve } from '../lib/curve';
import { formatValue } from '../lib/format';
import type { ControlPointTuple, Unit } from '../types';

const PAD_LEFT = 40;
const PAD_RIGHT = 10;
const PAD_TOP = 10;
const PAD_BOTTOM = 18;

/**
 * The rule curve drawn as inline SVG. Also used in the editor, where the current
 * input temperature is marked with the resulting output.
 */
export function CurvePreview({
  curve,
  current,
  inputUnit = 'celsius',
  outputUnit = 'percent',
  height = 160,
  label,
}: {
  curve: ControlPointTuple[];
  /** Current source value; marks the live intersection on the curve. */
  current?: number;
  inputUnit?: Unit;
  outputUnit?: Unit;
  height?: number;
  label: string;
}) {
  const [ref, width] = useElementWidth<HTMLDivElement>(420);

  const geometry = useMemo(() => {
    const points = sortCurve(curve);
    if (points.length === 0) return null;

    const inputRange = curveInputBounds(points);
    const outputRange = curveOutputBounds(points);
    // Always include 0..100 % on the output axis so curves are comparable.
    const outMin = outputUnit === 'percent' ? Math.min(0, outputRange.min) : outputRange.min;
    const outMax = outputUnit === 'percent' ? Math.max(100, outputRange.max) : outputRange.max;
    const inMin = inputRange.min;
    const inMax = inputRange.max;

    const plotW = Math.max(width - PAD_LEFT - PAD_RIGHT, 1);
    const plotH = Math.max(height - PAD_TOP - PAD_BOTTOM, 1);
    const toX = (input: number) => PAD_LEFT + ((input - inMin) / (inMax - inMin)) * plotW;
    const toY = (output: number) =>
      PAD_TOP + plotH - ((output - outMin) / (outMax - outMin || 1)) * plotH;

    const projected = points.map(([input, output]) => ({ x: toX(input), y: toY(output) }));
    const path = projected
      .map((point, index) => `${index === 0 ? 'M' : 'L'}${point.x.toFixed(1)} ${point.y.toFixed(1)}`)
      .join('');

    const live =
      current === undefined
        ? null
        : { x: toX(current), y: toY(evaluateCurve(points, current)) };

    return { projected, path, live, inMin, inMax, outMin, outMax, toX, toY };
  }, [curve, current, width, height, outputUnit]);

  if (!geometry) {
    return (
      <div className="chart__frame" ref={ref}>
        <p className="dim small">Add control points to see the curve.</p>
      </div>
    );
  }

  const { projected, path, live } = geometry;
  const areaPath = `${path}L${projected[projected.length - 1]?.x.toFixed(1)} ${height - PAD_BOTTOM}L${PAD_LEFT} ${height - PAD_BOTTOM}Z`;

  return (
    <div className="chart__frame" ref={ref}>
      <svg
        className="chart"
        width={Math.max(width, 1)}
        height={height}
        role="img"
        aria-label={`${label}. ${curve.length} control points from ${formatValue(curve[0]?.[0], inputUnit)} to ${formatValue(curve[curve.length - 1]?.[0], inputUnit)}.${live ? ` At ${formatValue(current, inputUnit)} the output is ${formatValue(evaluateCurve(curve, current ?? 0), outputUnit)}.` : ''}`}
      >
        <title>{label}</title>
        {[geometry.outMax, (geometry.outMax + geometry.outMin) / 2, geometry.outMin].map((tick, index) => {
          const y = PAD_TOP + ((height - PAD_TOP - PAD_BOTTOM) * index) / 2;
          return (
            <g key={`y-${index}`}>
              <line className="chart__grid" x1={PAD_LEFT} y1={y} x2={width - PAD_RIGHT} y2={y} />
              <text className="chart__axis-label" x={PAD_LEFT - 5} y={y + 3} textAnchor="end">
                {Math.round(tick)}%
              </text>
            </g>
          );
        })}
        <text className="chart__axis-label" x={PAD_LEFT} y={height - 5} textAnchor="start">
          {formatValue(geometry.inMin, inputUnit)}
        </text>
        <text className="chart__axis-label" x={width - PAD_RIGHT} y={height - 5} textAnchor="end">
          {formatValue(geometry.inMax, inputUnit)}
        </text>

        <path className="chart__area" d={areaPath} />
        <path className="chart__line" d={path} />

        {projected.map((point, index) => (
          <circle key={`p-${index}`} className="curve-point" cx={point.x} cy={point.y} r={3} />
        ))}

        {live ? (
          <g>
            <line
              className="chart__cursor"
              x1={live.x}
              y1={PAD_TOP}
              x2={live.x}
              y2={height - PAD_BOTTOM}
            />
            <circle className="curve-point curve-point--current" cx={live.x} cy={live.y} r={4} />
          </g>
        ) : null}
      </svg>
      {live && current !== undefined ? (
        <p className="tiny muted" style={{ marginTop: '4px' }}>
          Current input {formatValue(current, inputUnit)} → output{' '}
          {formatValue(evaluateCurve(curve, current), outputUnit)}
        </p>
      ) : null}
    </div>
  );
}
