import { useEffect, useMemo, useState } from 'react';
import { RuntimeProvider, useRuntime } from './hooks/useRuntime';
import { ToastProvider, ToastViewport } from './hooks/useToast';
import { api } from './lib/ipc';
import { usePolled } from './hooks/usePoll';
import { readStoredTheme, useTheme } from './lib/theme';
import { Sidebar, type Route } from './components/Sidebar';
import { Overview } from './screens/Overview';
import { Devices } from './screens/Devices';
import { DeviceDetail } from './screens/DeviceDetail';
import { Automation } from './screens/Automation';
import { SettingsScreen } from './screens/Settings';
import { Diagnostics } from './screens/Diagnostics';
import { MockHardware } from './screens/MockHardware';
import { Badge, InlineNotice } from './components/primitives';

export function App() {
  return (
    <ToastProvider>
      <RuntimeProvider>
        <Shell />
        <ToastViewport />
      </RuntimeProvider>
    </ToastProvider>
  );
}

function Shell() {
  const { snapshot, loading, error, refresh, demoMode } = useRuntime();
  const [route, setRoute] = useState<Route>({ name: 'overview' });

  const tick = snapshot?.generated_at_ms ?? 0;
  const mockStatus = usePolled(() => api.mockStatus(), tick, { throttleMs: 5000 });
  const rules = usePolled(() => api.listRules(), tick, { throttleMs: 5000 });
  const appInfo = usePolled(() => api.appInfo(), undefined);

  const settings = snapshot?.settings;
  const theme = settings?.theme ?? readStoredTheme();
  useTheme(theme);

  const showDiagnostics = settings?.developer_mode === true;
  const showMock = settings?.experimental_features === true && mockStatus.data !== null;

  // A screen can disappear when its setting is switched off: fall back safely.
  useEffect(() => {
    if (route.name === 'diagnostics' && !showDiagnostics) setRoute({ name: 'overview' });
    if (route.name === 'mock' && !showMock && mockStatus.data === null && !mockStatus.loading) {
      setRoute({ name: 'overview' });
    }
  }, [route.name, showDiagnostics, showMock, mockStatus.data, mockStatus.loading]);

  const titles = useMemo(() => describeRoute(route, snapshot?.devices.length ?? 0), [route, snapshot]);

  return (
    <div className="app">
      <Sidebar
        route={route}
        onNavigate={setRoute}
        deviceCount={snapshot?.devices.length ?? 0}
        ruleCount={rules.data?.length ?? 0}
        automationEnabled={snapshot?.settings.automation_enabled ?? false}
        logLevel={snapshot?.settings.log_level ?? 'info'}
        version={appInfo.data?.version ?? '—'}
        demoMode={demoMode}
        showDiagnostics={showDiagnostics}
        showMock={showMock}
      />

      <div className="main">
        <header className="topbar">
          <div className="topbar__titles">
            <h1 className="topbar__title">{titles.title}</h1>
            <p className="topbar__subtitle">{titles.subtitle}</p>
          </div>
          <div className="topbar__actions">
            {demoMode ? (
              <Badge tone="warn" title="No Tauri backend: a demo dataset is shown">
                demo data
              </Badge>
            ) : null}
            {snapshot?.settings.dry_run ? (
              <Badge tone="warn" title="Rules run, but writes are not sent to hardware">
                dry run
              </Badge>
            ) : null}
            {snapshot ? (
              <Badge tone={snapshot.has_controllable_hardware ? 'ok' : 'warn'}>
                {snapshot.has_controllable_hardware ? 'cooling available' : 'monitoring only'}
              </Badge>
            ) : null}
            <button
              type="button"
              className="btn btn--sm"
              onClick={() => void refresh()}
              disabled={loading}
            >
              Refresh
            </button>
          </div>
        </header>

        {error && snapshot ? (
          <div style={{ padding: 'var(--space-3) var(--space-5) 0' }}>
            <InlineNotice tone="error" title="The last refresh failed">
              <p>{error.message}</p>
              {error.hint ? <p className="inline-notice__hint">{error.hint}</p> : null}
            </InlineNotice>
          </div>
        ) : null}

        {route.name === 'overview' ? (
          <Overview
            onOpenDevice={(deviceId) => setRoute({ name: 'device', deviceId })}
            onNavigate={setRoute}
          />
        ) : null}

        {route.name === 'devices' ? (
          <Devices
            onOpenDevice={(deviceId) => setRoute({ name: 'device', deviceId })}
            onOpenSettings={() => setRoute({ name: 'settings' })}
          />
        ) : null}

        {route.name === 'device' && route.deviceId ? (
          <DeviceDetail
            deviceId={route.deviceId}
            onBack={() => setRoute({ name: 'devices' })}
            onOpenSettings={() => setRoute({ name: 'settings' })}
          />
        ) : null}

        {route.name === 'automation' ? <Automation /> : null}

        {route.name === 'settings' ? <SettingsScreen /> : null}

        {route.name === 'diagnostics' && showDiagnostics ? (
          <Diagnostics onOpenSettings={() => setRoute({ name: 'settings' })} />
        ) : null}

        {route.name === 'mock' && showMock ? (
          <MockHardware onOpenSettings={() => setRoute({ name: 'settings' })} />
        ) : null}
      </div>
    </div>
  );
}

function describeRoute(route: Route, deviceCount: number): { title: string; subtitle: string } {
  switch (route.name) {
    case 'overview':
      return {
        title: 'Overview',
        subtitle: 'Temperatures, loads and cooling at a glance',
      };
    case 'devices':
      return {
        title: 'Devices',
        subtitle: `${deviceCount} device${deviceCount === 1 ? '' : 's'} discovered by the enabled adapters`,
      };
    case 'device':
      return { title: 'Device detail', subtitle: route.deviceId ?? '' };
    case 'automation':
      return {
        title: 'Automation',
        subtitle: 'Curves that turn temperatures into fan and pump speeds',
      };
    case 'settings':
      return { title: 'Settings', subtitle: 'Monitoring, safety and application behaviour' };
    case 'diagnostics':
      return { title: 'Diagnostics', subtitle: 'Adapters, statistics, audit log and live events' };
    case 'mock':
      return {
        title: 'Simulated hardware',
        subtitle: 'Synthetic sensors and fans for demonstrating the control loop',
      };
    default:
      return { title: 'OpenHardwareOS', subtitle: '' };
  }
}
