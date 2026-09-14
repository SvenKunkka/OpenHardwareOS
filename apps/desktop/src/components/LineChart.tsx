import { useMemo, useState } from 'react';
import { useElementWidth } from '../hooks/useElementWidth';
import { EMPTY, formatClock, formatValue } from '../lib/format';
import type { Sample, Unit } from '../types';

const PAD_LEFT = 48;
const PAD_RIGHT = 12;
const PAD_TOP = 10;
const PAD_BOTTOM = 20;

interface Geometry {
  min: number;
  max: number;
  points: { x: number; y: number; sample: Sample }[];
}

function buildGeometry(samples: Sample[], width: number, height: number): Geometry | null {
  if (samples.length < 2 || width <= 0) return null;
  let min = Number.POSITIVE_INFINITY;
  let max = Number.NEGATIVE_INFINITY;
  for (const sample of samples) {
    if (sample.value < min) min = sample.value;
    if (sample.value > max) max = sample.value;
  }
  if (!Number.isFinite(min) || !Number.isFinite(max)) return null;
  if (max - min < 1e-6) {
    min -= Math.max(Math.abs(min) * 0.02, 1);
    max += Math.max(Math.abs(max) * 0.02, 1);
  } else {
    const pad = (max - min) * 0.08;
    min -= pad;
    max += pad;
  }
  const plotW = Math.max(width - PAD_LEFT - PAD_RIGHT, 1);
  const plotH = Math.max(height - PAD_TOP - PAD_BOTTOM, 1);
  const stepX = plotW / (samples.length - 1);
  const points = samples.map((sample, index) => ({
    x: PAD_LEFT + index * stepX,
    y: PAD_TOP + plotH - ((sample.value - min) / (max - min)) * plotH,
    sample,
  }));
  return { min, max, points };
}

/**
 * Inline-SVG line chart with y-axis labels and a hover readout. Hovering is
 * pointer-only sugar: the same numbers are in the accessible table below the
 * chart, so nothing is mouse-only.
 */
export function LineChart({
  samples,
  unit,
  label,
  height = 200,
  showTable = true,
}: {
  samples: Sample[];
  unit: Unit;
  label: string;
  height?: number;
  showTable?: boolean;
}) {
  const [ref, width] = useElementWidth<HTMLDivElement>(640);
  const [hover, setHover] = useState<number | null>(null);

  const geometry = useMemo(
    () => buildGeometry(samples, Math.max(width, PAD_LEFT + PAD_RIGHT + 1), height),
    [samples, width, height],
  );

  const ticks = useMemo(() => {
    if (!geometry) return [];
    const { min, max } = geometry;
    return [max, min + (max - min) / 2, min];
  }, [geometry]);

  const hovered = hover !== null && geometry ? geometry.points[hover] : undefined;
  const latest = samples.length > 0 ? samples[samples.length - 1] : undefined;

  const linePath = geometry
    ? geometry.points.map((point, index) => `${index === 0 ? 'M' : 'L'}${point.x.toFixed(1)} ${point.y.toFixed(1)}`).join('')
    : '';
  const areaPath = geometry
    ? `${linePath}L${geometry.points[geometry.points.length - 1]?.x.toFixed(1) ?? PAD_LEFT} ${height - PAD_BOTTOM}L${PAD_LEFT} ${height - PAD_BOTTOM}Z`
    : '';

  const summary = latest
    ? `${formatValue(latest.value, unit)} now, ${samples.length} samples from ${formatClock(samples[0]?.at_ms)} to ${formatClock(latest.at_ms)}`
    : `No samples recorded yet (${EMPTY})`;

  return (
    <div className="chart__frame" ref={ref}>
      <svg
        className="chart"
        width={Math.max(width, 1)}
        height={height}
        role="img"
        aria-label={`${label}: ${summary}`}
        onPointerMove={(event) => {
          if (!geometry) return;
          const rect = event.currentTarget.getBoundingClientRect();
          const x = event.clientX - rect.left;
          const plotW = Math.max(width - PAD_LEFT - PAD_RIGHT, 1);
          const ratio = (x - PAD_LEFT) / plotW;
          const index = Math.round(ratio * (geometry.points.length - 1));
          setHover(Math.min(Math.max(index, 0), geometry.points.length - 1));
        }}
        onPointerLeave={() => setHover(null)}
      >
        <title>{`${label} — ${summary}`}</title>
        {ticks.map((tick, index) => {
          const y =
            PAD_TOP +
            (Math.max(height - PAD_TOP - PAD_BOTTOM, 1) * index) / Math.max(ticks.length - 1, 1);
          return (
            <g key={index}>
              <line className="chart__grid" x1={PAD_LEFT} y1={y} x2={width - PAD_RIGHT} y2={y} />
              <text className="chart__axis-label" x={PAD_LEFT - 6} y={y + 3} textAnchor="end">
                {formatValue(tick, unit)}
              </text>
            </g>
          );
        })}

        {geometry ? (
          <>
            <path className="chart__area" d={areaPath} />
            <path className="chart__line" d={linePath} />
            <text
              className="chart__axis-label"
              x={PAD_LEFT}
              y={height - 6}
              textAnchor="start"
            >
              {formatClock(samples[0]?.at_ms)}
            </text>
            <text className="chart__axis-label" x={width - PAD_RIGHT} y={height - 6} textAnchor="end">
              {formatClock(latest?.at_ms)}
            </text>
            {hovered ? (
              <g>
                <line
                  className="chart__cursor"
                  x1={hovered.x}
                  y1={PAD_TOP}
                  x2={hovered.x}
                  y2={height - PAD_BOTTOM}
                />
                <circle className="chart__marker" cx={hovered.x} cy={hovered.y} r={3.5} />
              </g>
            ) : null}
          </>
        ) : (
          <text className="chart__axis-label" x={width / 2} y={height / 2} textAnchor="middle">
            No history yet
          </text>
        )}
      </svg>

      {hovered ? (
        <p
          className="chart__tooltip"
          style={{
            left: `${hovered.x}px`,
            top: `${hovered.y}px`,
          }}
        >
          {formatValue(hovered.sample.value, unit)} · {formatClock(hovered.sample.at_ms)}
        </p>
      ) : null}

      {showTable && samples.length > 1 ? (
        <details className="small" style={{ marginTop: '8px' }}>
          <summary className="muted">Show history as a table ({samples.length} samples)</summary>
          <div className="scroll-y" style={{ maxHeight: '180px', marginTop: '8px' }}>
            <table className="table">
              <caption>{label} — same data as the chart above</caption>
              <thead>
                <tr>
                  <th scope="col">Time</th>
                  <th scope="col">Value</th>
                </tr>
              </thead>
              <tbody>
                {[...samples]
                  .reverse()
                  .slice(0, 120)
                  .map((sample) => (
                    <tr key={sample.at_ms}>
                      <td className="mono">{formatClock(sample.at_ms)}</td>
                      <td>{formatValue(sample.value, unit)}</td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
        </details>
      ) : null}
    </div>
  );
}
