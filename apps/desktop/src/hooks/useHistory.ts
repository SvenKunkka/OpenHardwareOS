import { useEffect, useRef, useState } from 'react';
import { api } from '../lib/ipc';
import { toNoticeError } from './useRuntime';
import type { NoticeError, Sample } from '../types';

export interface HistoryValue {
  samples: Sample[];
  loading: boolean;
  error: NoticeError | null;
}

/**
 * Fetch `get_history` for one series and re-fetch whenever `tick` changes
 * (callers pass `snapshot.generated_at_ms`, so the series follows every poll).
 * Late responses from a superseded request are discarded.
 */
export function useHistory(
  device: string | undefined,
  capability: string | undefined,
  limit: number,
  tick: number | undefined,
): HistoryValue {
  const [samples, setSamples] = useState<Sample[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<NoticeError | null>(null);
  const requestId = useRef(0);

  useEffect(() => {
    if (!device || !capability) {
      setSamples([]);
      setError(null);
      return;
    }
    const id = requestId.current + 1;
    requestId.current = id;
    let cancelled = false;
    setLoading(true);

    api
      .getHistory(device, capability, limit)
      .then((next) => {
        if (cancelled || requestId.current !== id) return;
        setSamples(next);
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
  }, [device, capability, limit, tick]);

  return { samples, loading, error };
}
