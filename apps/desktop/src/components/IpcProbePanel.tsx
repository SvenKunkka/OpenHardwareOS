/**
 * What an `--ipc-selftest` run found, shown in the application's own window.
 *
 * The probe's report is written to disk for the harness, but a file is not a
 * screenshot: this panel renders the same findings with the application's own
 * components, so a picture of the window shows the round trip's results rather than
 * merely proving that a webview exists. It appears only when a self-test has actually
 * run — a normal launch never renders it.
 */

import { useEffect, useState } from 'react';
import { Badge, Panel } from './primitives';
import { subscribeIpcProbe, type ProbeReport } from '../lib/ipcProbe';

/** Subscribe to the latest self-test report, if one exists. */
export function useIpcProbeReport(): ProbeReport | null {
  const [report, setReport] = useState<ProbeReport | null>(null);
  useEffect(() => subscribeIpcProbe(setReport), []);
  return report;
}

export function IpcProbePanel({ report }: { report: ProbeReport }) {
  const failures = report.steps.filter((step) => !step.ok);
  const rendered = report.rendered;

  return (
    <div data-testid="ipc-probe-panel">
    <Panel
      title="IPC self-test"
      subtitle="The frontend called the backend through Tauri's own IPC and reports what came back"
    >
      <div className="stack stack--tight">
        <div className="row">
          <Badge tone={failures.length === 0 ? 'ok' : 'danger'}>
            {failures.length === 0 ? 'all steps passed' : `${failures.length} step(s) failed`}
          </Badge>
          <span className="small dim" data-testid="ipc-probe-version">
            probe v{report.probe_version}
          </span>
          <span className="small dim mono" data-testid="ipc-probe-origin">
            {report.location}
          </span>
        </div>

        <ul className="feed" aria-label="IPC self-test steps">
          {report.steps.map((step) => (
            <li className="feed__item" key={step.step}>
              <span className={`feed__level feed__level--${step.ok ? 'ok' : 'error'}`}>
                {step.ok ? 'pass' : 'fail'}
              </span>
              <span className="mono small">{step.step}</span>
              <span className="small muted">
                {step.ok ? summarise(step.result) : (step.error ?? 'failed')}
              </span>
            </li>
          ))}
        </ul>

        <div className="kv">
          <span className="kv__label">Write through IPC</span>
          <span className="kv__value" data-testid="ipc-probe-write">
            {rendered.writeStatus ?? '—'} · applied {rendered.writeApplied ?? '—'}
          </span>
          <span className="kv__label">Handover through IPC</span>
          <span className="kv__value" data-testid="ipc-probe-handovers">
            {rendered.handoverStates ?? '—'}
          </span>
          <span className="kv__label">Scoped retry</span>
          <span className="kv__value" data-testid="ipc-probe-retry">
            re-armed {rendered.rearmed ?? '—'} (only its own channel:{' '}
            {rendered.retryAllowed ?? '—'})
          </span>
        </div>
      </div>
    </Panel>
    </div>
  );
}

/** One line of a step's result, short enough for a row. */
function summarise(result: unknown): string {
  if (result === undefined) return 'ok';
  if (result === null) return 'null';
  const text = typeof result === 'string' ? result : JSON.stringify(result);
  return text.length > 160 ? `${text.slice(0, 160)}…` : text;
}
