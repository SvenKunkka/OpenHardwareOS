import { useMemo } from 'react';
import { EMPTY, formatValue } from '../lib/format';
import type { Sample, Unit } from '../types';

/**
 * Hand-written inline-SVG sparkline for the last N samples. No charting library.
 * A missing series renders as a dashed placeholder rather than as a flat zero.
 */
export function Sparkline({
  samples,
  unit,
  label,
  height = 40,
}: {
  samples: Sample[];
  unit: Unit;
  /** Accessible description of what is plotted. */
  label: string;
  height?: number;
}) {
  const width = 240;
  const path = useMemo(() => {
    if (samples.length < 2) return null;
    let min = Number.POSITIVE_INFINITY;
    let max = Number.NEGATIVE_INFINITY;
    for (const sample of samples) {
      if (sample.value < min) min = sample.value;
      if (sample.value > max) max = sample.value;
    }
    if (!Number.isFinite(min) || !Number.isFinite(max)) return null;
    // Give a flat series a little vertical room so it stays visible.
    const span = max - min < 1e-6 ? Math.max(Math.abs(max) * 0.05, 1) : max - min;
    const pad = height === 0 ? 0 : 3;
    const usable = height - pad * 2;
    const stepX = width / (samples.length - 1);
    let d = '';
    for (let i = 0; i < samples.length; i += 1) {
      const sample = samples[i];
      if (!sample) continue;
      const x = i * stepX;
      const y = pad + usable - ((sample.value - min) / span) * usable;
      d += `${i === 0 ? 'M' : 'L'}${x.toFixed(2)} ${y.toFixed(2)}`;
    }
    const area = `${d}L${width} ${height}L0 ${height}Z`;
    return { line: d, area };
  }, [samples, height]);

  const latest = samples.length > 0 ? samples[samples.length - 1] : undefined;

  return (
    <svg
      className="spark"
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      role="img"
      aria-label={`${label}: ${latest ? formatValue(latest.value, unit) : `no data (${EMPTY})`} across ${samples.length} samples`}
    >
      {path ? (
        <>
          <path className="spark__area" d={path.area} />
          <path className="spark__line" d={path.line} vectorEffect="non-scaling-stroke" />
        </>
      ) : (
        <line
          className="spark--empty"
          x1="0"
          y1={height / 2}
          x2={width}
          y2={height / 2}
          vectorEffect="non-scaling-stroke"
        />
      )}
    </svg>
  );
}
