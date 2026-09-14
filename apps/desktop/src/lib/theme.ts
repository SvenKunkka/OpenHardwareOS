/**
 * Theme handling: `system` | `dark` | `light`, applied as `[data-theme]` on the
 * root element. The choice is mirrored into localStorage so `index.html` can
 * paint the right theme before React mounts.
 */

import { useEffect, useState } from 'react';
import type { Theme } from '../types';

const STORAGE_KEY = 'ohm.theme';

export type ResolvedTheme = 'dark' | 'light';

export function systemTheme(): ResolvedTheme {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return 'dark';
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

export function resolveTheme(mode: Theme): ResolvedTheme {
  return mode === 'system' ? systemTheme() : mode;
}

export function readStoredTheme(): Theme {
  if (typeof window === 'undefined') return 'system';
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return stored === 'dark' || stored === 'light' || stored === 'system' ? stored : 'system';
  } catch {
    return 'system';
  }
}

export function applyTheme(mode: Theme): void {
  if (typeof document === 'undefined') return;
  const resolved = resolveTheme(mode);
  document.documentElement.setAttribute('data-theme', resolved);
  document.documentElement.setAttribute('data-theme-mode', mode);
  document.documentElement.style.colorScheme = resolved;
  try {
    window.localStorage.setItem(STORAGE_KEY, mode);
  } catch {
    /* storage is best-effort */
  }
}

/** Keep the document theme in sync with the settings choice. */
export function useTheme(mode: Theme): ResolvedTheme {
  const [resolved, setResolved] = useState<ResolvedTheme>(() => resolveTheme(mode));

  useEffect(() => {
    applyTheme(mode);
    setResolved(resolveTheme(mode));
    if (mode !== 'system' || typeof window.matchMedia !== 'function') return;
    const media = window.matchMedia('(prefers-color-scheme: light)');
    const onChange = () => {
      applyTheme('system');
      setResolved(systemTheme());
    };
    media.addEventListener('change', onChange);
    return () => media.removeEventListener('change', onChange);
  }, [mode]);

  return resolved;
}
