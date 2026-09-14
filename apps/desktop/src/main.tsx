import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { runIpcProbe } from './lib/ipcProbe';
import './styles/theme.css';
import './styles/app.css';

const container = document.getElementById('root');
if (!container) {
  throw new Error('The #root element is missing from index.html.');
}

/**
 * The entry point an `--ipc-selftest` run of the real app calls from the backend, via
 * `eval` in this webview. It is inert unless the backend asks: nothing here runs on a
 * normal launch.
 */
declare global {
  interface Window {
    __ohmIpcProbe?: () => Promise<unknown>;
  }
}

window.__ohmIpcProbe = () => runIpcProbe();

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
