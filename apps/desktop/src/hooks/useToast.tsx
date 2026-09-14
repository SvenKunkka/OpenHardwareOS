import { createContext, useCallback, useContext, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';

export type ToastKind = 'success' | 'error' | 'info';

export interface Toast {
  id: number;
  kind: ToastKind;
  title: string;
  message?: string;
  hint?: string;
}

interface ToastApi {
  toasts: Toast[];
  push: (toast: Omit<Toast, 'id'>) => number;
  success: (title: string, message?: string) => number;
  info: (title: string, message?: string) => number;
  error: (title: string, message?: string, hint?: string) => number;
  dismiss: (id: number) => void;
}

const ToastContext = createContext<ToastApi | null>(null);

const LIFETIME: Record<ToastKind, number> = {
  success: 6000,
  info: 7000,
  error: 12000,
};

/**
 * The single feedback mechanism of the app: bounded, dismissible notices with an
 * `aria-live` region, so an action never fails silently and never speaks in raw
 * stack traces.
 */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextId = useRef(1);
  const timers = useRef(new Map<number, number>());

  const dismiss = useCallback((id: number) => {
    const timer = timers.current.get(id);
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timers.current.delete(id);
    }
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  const push = useCallback(
    (toast: Omit<Toast, 'id'>) => {
      const id = nextId.current;
      nextId.current += 1;
      setToasts((current) => [...current.slice(-4), { ...toast, id }]);
      const timer = window.setTimeout(() => dismiss(id), LIFETIME[toast.kind]);
      timers.current.set(id, timer);
      return id;
    },
    [dismiss],
  );

  const api = useMemo<ToastApi>(
    () => ({
      toasts,
      push,
      dismiss,
      success: (title, message) => push({ kind: 'success', title, message }),
      info: (title, message) => push({ kind: 'info', title, message }),
      error: (title, message, hint) => push({ kind: 'error', title, message, hint }),
    }),
    [toasts, push, dismiss],
  );

  return <ToastContext.Provider value={api}>{children}</ToastContext.Provider>;
}

export function useToast(): ToastApi {
  const value = useContext(ToastContext);
  if (!value) throw new Error('useToast() must be used inside <ToastProvider>.');
  return value;
}

export function ToastViewport() {
  const { toasts, dismiss } = useToast();
  return (
    <div className="toast-viewport" role="region" aria-label="Notifications">
      <div aria-live="polite" aria-atomic="false" className="toast-stack">
        {toasts.map((toast) => (
          <div key={toast.id} className={`toast toast--${toast.kind}`} role="status">
            <div className="toast__body">
              <p className="toast__title">
                <span className="toast__glyph" aria-hidden="true">
                  {toast.kind === 'error' ? '!' : toast.kind === 'success' ? '✓' : 'i'}
                </span>
                {toast.title}
              </p>
              {toast.message ? <p className="toast__message">{toast.message}</p> : null}
              {toast.hint ? <p className="toast__hint">{toast.hint}</p> : null}
            </div>
            <button
              type="button"
              className="toast__close"
              onClick={() => dismiss(toast.id)}
              aria-label={`Dismiss notification: ${toast.title}`}
            >
              ×
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
