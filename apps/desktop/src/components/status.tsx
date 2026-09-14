/**
 * Status badges. Every badge carries a text label — colour is never the only
 * signal (WCAG 1.4.1).
 */

import { Badge, type Tone } from './primitives';
import {
  adapterStateLabel,
  deviceStatusLabel,
  ruleStatusHint,
  ruleStatusLabel,
  unavailableReasonBadge,
  unavailableReasonLabel,
} from '../lib/format';
import type {
  AdapterState,
  CapabilityValue,
  DeviceStatus,
  RuleStatus,
  UnavailableReason,
  Unit,
} from '../types';
import { EMPTY, formatValue, formatValueParts } from '../lib/format';

const DEVICE_STATUS_TONE: Record<DeviceStatus, Tone> = {
  online: 'ok',
  degraded: 'warn',
  offline: 'danger',
  disabled: 'neutral',
};

export function DeviceStatusBadge({ status }: { status: DeviceStatus }) {
  return <Badge tone={DEVICE_STATUS_TONE[status]}>{deviceStatusLabel(status)}</Badge>;
}

const ADAPTER_STATE_TONE: Record<AdapterState, Tone> = {
  available: 'ok',
  degraded: 'warn',
  unavailable: 'danger',
  error: 'danger',
  not_probed: 'neutral',
};

export function AdapterStateBadge({ state }: { state: AdapterState }) {
  return <Badge tone={ADAPTER_STATE_TONE[state]}>{adapterStateLabel(state)}</Badge>;
}

const RULE_STATUS_TONE: Record<RuleStatus, Tone> = {
  applied: 'ok',
  idle: 'info',
  held: 'warn',
  // Standing down because a "when" condition is false is normal operation, not a
  // fault: it gets the calm neutral tone and its own label.
  gated: 'neutral',
  fallback: 'warn',
  released: 'neutral',
  disabled: 'neutral',
  error: 'danger',
};

export function RuleStatusBadge({ status }: { status: RuleStatus }) {
  return (
    <Badge tone={RULE_STATUS_TONE[status]} title={ruleStatusHint(status)}>
      {ruleStatusLabel(status)}
    </Badge>
  );
}

/**
 * A grey badge for a reading the hardware cannot provide. The reason is always
 * visible as text; the full sentence is the accessible title.
 */
export function UnavailableBadge({ reason }: { reason: UnavailableReason }) {
  return (
    <Badge tone="neutral" title={unavailableReasonLabel(reason)}>
      {unavailableReasonBadge(reason)}
    </Badge>
  );
}

/** Value + unit, or an em dash plus the reason badge. Never a fake zero. */
export function ReadingValue({
  value,
  unit,
  unavailableReason,
  unavailableDetail,
  className,
}: {
  value: CapabilityValue | undefined;
  unit: Unit;
  unavailableReason?: UnavailableReason;
  unavailableDetail?: string;
  className?: string;
}) {
  if (value === undefined || value === null) {
    return (
      <span className={`row row--tight ${className ?? ''}`}>
        <span className="reading reading--missing">
          <span className="reading__value">{EMPTY}</span>
        </span>
        {unavailableReason ? <UnavailableBadge reason={unavailableReason} /> : null}
        {unavailableDetail ? <span className="tiny dim">{unavailableDetail}</span> : null}
      </span>
    );
  }

  if (typeof value !== 'number') {
    return <span className={`reading ${className ?? ''}`}>{formatValue(value, unit)}</span>;
  }

  const parts = formatValueParts(value, unit);
  return (
    <span className={`reading ${className ?? ''}`}>
      <span className="reading__value">{parts.number}</span>
      {parts.unit ? <span className="reading__unit">{parts.unit}</span> : null}
    </span>
  );
}

/** Small helper for tables: the value text only (unit included). */
export function valueText(value: CapabilityValue | undefined, unit: Unit): string {
  return formatValue(value, unit);
}
