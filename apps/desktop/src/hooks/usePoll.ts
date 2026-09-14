import { useEffect, useRef, useState } from 'react';
import { toNoticeError } from './useRuntime';
import type { NoticeError } from '../types';

/**
 * Run an async fetcher on mount and again whenever `tick` changes, optionally
 * throttled. Used for the small side queries (rule outcomes, audit log, mock
 * status) that are not part of the pushed snapshot.
 */
export function usePolled<T>(
  fetcher: () => Promise<T>,
  tick: number | undefined,
  options: { throttleMs?: number; enabled?: boolean } = {},
): { data: T | null; error: NoticeError | null; loading: boolean; reload: () => void } {
  const { throttleMs = 0, enabled = true } = options;
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<NoticeError | null>(null);
  const [loading, setLoading] = useState(enabled);
  const [nonce, setNonce] = useState(0);

  const fetcherRef = useRef(fetcher);
  fetcherRef.current = fetcher;
  const lastRun = useRef(0);
  const requestId = useRef(0);
  /** Set by `reload()`: an explicit refresh is never swallowed by the throttle. */
  const forced = useRef(false);

  useEffect(() => {
    if (!enabled) return;
    const now = Date.now();
    const isForced = forced.current;
    forced.current = false;
    if (
      !isForced &&
      tick !== undefined &&
      throttleMs > 0 &&
      now - lastRun.current < throttleMs &&
      data !== null
    ) {
      return;
    }
    lastRun.current = now;
    const id = requestId.current + 1;
    requestId.current = id;
    let cancelled = false;
    setLoading(true);
    fetcherRef
      .current()
      .then((result) => {
        if (cancelled || requestId.current !== id) return;
        setData(result);
        setError(null);
      })
      .catch((cause: unknown) => {
        if (cancelled || requestId.current !== id) return;
        setError(toNoticeError(cause));
      })
      .finally(() => {
        if (cancelled || requestId.current !== id) return;
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // `data` is intentionally not a dependency: it only gates throttling.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tick, enabled, throttleMs, nonce]);

  return {
    data,
    error,
    loading,
    reload: () => {
      forced.current = true;
      setNonce((value) => value + 1);
    },
  };
}
