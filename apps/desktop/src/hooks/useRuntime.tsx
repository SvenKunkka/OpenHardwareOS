import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import type { ReactNode } from 'react';
import { BackendError, api, isDemoMode, subscribeRuntime } from '../lib/ipc';
import { decodeEvent } from '../lib/events';
import { useToast } from './useToast';
import type { ActivityEntry, NoticeError, RuntimeSnapshot } from '../types';

export interface RuntimeValue {
  /** Latest pushed or polled snapshot, or null before the first load. */
  snapshot: RuntimeSnapshot | null;
  /** True until the first snapshot arrives. */
  loading: boolean;
  /** Populated when the initial load or a refresh failed. */
  error: NoticeError | null;
  /** Running in a plain browser against the demo backend. */
  demoMode: boolean;
  /** Last ~200 decoded runtime events, newest first. */
  events: ActivityEntry[];
  /** Re-poll the backend and update the snapshot. */
  refresh: () => Promise<RuntimeSnapshot | null>;
  /** Subscribe to new runtime events (used by the diagnostics feed). */
  subscribeEvents: (handler: (entry: ActivityEntry) => void) => () => void;
  /**
   * Run an action, report failures through the toast mechanism and refresh the
   * snapshot afterwards. Returns `undefined` when the action failed.
   */
  perform: <T>(
    label: string,
    action: () => Promise<T>,
    options?: { refresh?: boolean; success?: string },
  ) => Promise<T | undefined>;
}

const RuntimeContext = createContext<RuntimeValue | null>(null);

export function toNoticeError(error: unknown): NoticeError {
  if (error instanceof BackendError) {
    return {
      code: error.code,
      message: error.message,
      hint: error.hint,
      unsupported: error.unsupported,
    };
  }
  if (error instanceof Error) return { code: 'unknown', message: error.message };
  return { code: 'unknown', message: String(error) };
}

const MAX_EVENTS = 200;

/**
 * The single runtime data source: subscribes to `snapshot` and `runtime-event`
 * once for the whole app, keeps the snapshot in React state and exposes
 * `refresh()` plus an action wrapper.
 */
export function RuntimeProvider({ children }: { children: ReactNode }) {
  const [snapshot, setSnapshot] = useState<RuntimeSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<NoticeError | null>(null);
  const [events, setEvents] = useState<ActivityEntry[]>([]);
  const toast = useToast();
  const eventHandlers = useRef(new Set<(entry: ActivityEntry) => void>());
  const toastRef = useRef(toast);
  toastRef.current = toast;

  const pushEvent = useCallback((entry: ActivityEntry) => {
    setEvents((current) => [entry, ...current].slice(0, MAX_EVENTS));
    for (const handler of eventHandlers.current) handler(entry);
  }, []);

  const refresh = useCallback(async (): Promise<RuntimeSnapshot | null> => {
    try {
      const next = await api.getSnapshot();
      setSnapshot(next);
      setError(null);
      return next;
    } catch (cause) {
      setError(toNoticeError(cause));
      return null;
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let disposed = false;

    void refresh();

    let unsubscribe: (() => void) | undefined;
    void subscribeRuntime(
      (next) => {
        if (disposed) return;
        setSnapshot(next);
        setError(null);
        setLoading(false);
      },
      (event) => {
        if (disposed) return;
        pushEvent(decodeEvent(event));
      },
    )
      .then((off) => {
        if (disposed) off();
        else unsubscribe = off;
      })
      .catch((cause: unknown) => {
        if (!disposed) setError(toNoticeError(cause));
      });

    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [refresh, pushEvent]);

  const subscribeEvents = useCallback((handler: (entry: ActivityEntry) => void) => {
    eventHandlers.current.add(handler);
    return () => {
      eventHandlers.current.delete(handler);
    };
  }, []);

  const perform = useCallback<RuntimeValue['perform']>(
    async (label, action, options) => {
      try {
        const result = await action();
        if (options?.success) {
          toastRef.current.success(options.success);
        } else {
          toastRef.current.success(label);
        }
        if (options?.refresh !== false) await refresh();
        return result;
      } catch (cause) {
        const notice = toNoticeError(cause);
        toastRef.current.error(`${label} failed`, notice.message, notice.hint);
        return undefined;
      }
    },
    [refresh],
  );

  const value = useMemo<RuntimeValue>(
    () => ({
      snapshot,
      loading,
      error,
      demoMode: isDemoMode,
      events,
      refresh,
      subscribeEvents,
      perform,
    }),
    [snapshot, loading, error, events, refresh, subscribeEvents, perform],
  );

  return <RuntimeContext.Provider value={value}>{children}</RuntimeContext.Provider>;
}

export function useRuntime(): RuntimeValue {
  const value = useContext(RuntimeContext);
  if (!value) throw new Error('useRuntime() must be used inside <RuntimeProvider>.');
  return value;
}
