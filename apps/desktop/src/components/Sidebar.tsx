import type { LogLevel } from '../types';

export type RouteName = 'overview' | 'devices' | 'device' | 'automation' | 'settings' | 'diagnostics' | 'mock';

export interface Route {
  name: RouteName;
  deviceId?: string;
}

function Glyph({ name }: { name: RouteName }) {
  const common = {
    width: 16,
    height: 16,
    viewBox: '0 0 16 16',
    fill: 'none',
    stroke: 'currentColor',
    strokeWidth: 1.4,
    strokeLinecap: 'round' as const,
    strokeLinejoin: 'round' as const,
    className: 'nav__glyph',
    'aria-hidden': true,
  };
  switch (name) {
    case 'overview':
      return (
        <svg {...common}>
          <rect x="1.5" y="1.5" width="5.5" height="5.5" rx="1" />
          <rect x="9" y="1.5" width="5.5" height="5.5" rx="1" />
          <rect x="1.5" y="9" width="5.5" height="5.5" rx="1" />
          <rect x="9" y="9" width="5.5" height="5.5" rx="1" />
        </svg>
      );
    case 'devices':
    case 'device':
      return (
        <svg {...common}>
          <rect x="1.5" y="3" width="13" height="10" rx="1.5" />
          <path d="M5 6h6M5 8.5h4" />
        </svg>
      );
    case 'automation':
      return (
        <svg {...common}>
          <path d="M2 12c2.5 0 3-8 5.5-8S10 12 14 12" />
          <circle cx="2" cy="12" r="1.2" />
          <circle cx="14" cy="12" r="1.2" />
        </svg>
      );
    case 'settings':
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="2.2" />
          <path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M12.6 3.4l-1.4 1.4M4.8 11.2l-1.4 1.4" />
        </svg>
      );
    case 'diagnostics':
      return (
        <svg {...common}>
          <path d="M1.5 8h3l1.5-4 2.5 8 1.5-4h4.5" />
        </svg>
      );
    case 'mock':
      return (
        <svg {...common}>
          <rect x="2.5" y="6" width="11" height="7.5" rx="1" />
          <path d="M8 6V3M5.5 3h5" />
        </svg>
      );
    default:
      return null;
  }
}

export function Sidebar({
  route,
  onNavigate,
  deviceCount,
  ruleCount,
  automationEnabled,
  logLevel,
  version,
  demoMode,
  showDiagnostics,
  showMock,
}: {
  route: Route;
  onNavigate: (route: Route) => void;
  deviceCount: number;
  ruleCount: number;
  automationEnabled: boolean;
  logLevel: LogLevel;
  version: string;
  demoMode: boolean;
  showDiagnostics: boolean;
  showMock: boolean;
}) {
  const items: { route: Route; label: string; count?: number }[] = [
    { route: { name: 'overview' }, label: 'Overview' },
    { route: { name: 'devices' }, label: 'Devices', count: deviceCount },
    { route: { name: 'automation' }, label: 'Automation', count: ruleCount },
    { route: { name: 'settings' }, label: 'Settings' },
  ];

  return (
    <nav className="sidebar" aria-label="Main">
      <div className="sidebar__brand">
        <span className="sidebar__mark" aria-hidden="true">
          OH
        </span>
        <div>
          <p className="sidebar__name">OpenHardwareOS</p>
          <p className="sidebar__version">v{version}</p>
        </div>
      </div>

      <ul className="nav">
        {items.map((item) => (
          <li key={item.route.name}>
            <button
              type="button"
              className="nav__item"
              aria-current={
                route.name === item.route.name ||
                (item.route.name === 'devices' && route.name === 'device')
                  ? 'page'
                  : undefined
              }
              onClick={() => onNavigate(item.route)}
            >
              <Glyph name={item.route.name} />
              <span>{item.label}</span>
              {item.count !== undefined ? <span className="nav__count">{item.count}</span> : null}
            </button>
          </li>
        ))}
      </ul>

      {showDiagnostics || showMock ? (
        <>
          <p className="nav__group-title">Tools</p>
          <ul className="nav">
            {showMock ? (
              <li>
                <button
                  type="button"
                  className="nav__item"
                  aria-current={route.name === 'mock' ? 'page' : undefined}
                  onClick={() => onNavigate({ name: 'mock' })}
                >
                  <Glyph name="mock" />
                  <span>Simulated hardware</span>
                </button>
              </li>
            ) : null}
            {showDiagnostics ? (
              <li>
                <button
                  type="button"
                  className="nav__item"
                  aria-current={route.name === 'diagnostics' ? 'page' : undefined}
                  onClick={() => onNavigate({ name: 'diagnostics' })}
                >
                  <Glyph name="diagnostics" />
                  <span>Diagnostics</span>
                </button>
              </li>
            ) : null}
          </ul>
        </>
      ) : null}

      <div className="sidebar__footer">
        <span>
          Automation: <strong>{automationEnabled ? 'on' : 'off'}</strong>
        </span>
        <span>
          Log level: <strong>{logLevel}</strong>
        </span>
        {demoMode ? (
          <span className="badge badge--warn badge--plain" title="Running outside Tauri">
            demo data
          </span>
        ) : null}
      </div>
    </nav>
  );
}
