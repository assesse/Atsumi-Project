import { useCallback, useEffect, useRef, useState } from "react";
import { getQueueSnapshot, type QueueQuery, type QueueSnapshot } from "../api/workConsole";

/** One in-flight read, five-second cadence, no download-library rehydration. */
export function useWorkQueue(enabled: boolean) {
  const [observedSince] = useState(() => new Date().toISOString());
  const [query, setQuery] = useState<QueueQuery>({ page: 1 });
  const [snapshot, setSnapshot] = useState<QueueSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const busy = useRef(false);
  const nextRefresh = useRef<(() => Promise<void>) | null>(null);
  const signature = useRef("");
  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    const refresh = async () => {
      if (busy.current) return;
      busy.current = true;
      try {
        const next = await getQueueSnapshot({ offset: query.offset ?? 0, observedSince });
        if (disposed) return;
        const stable = JSON.stringify({ ...next, queriedAt: "" });
        if (signature.current !== stable) { signature.current = stable; setSnapshot(next); }
        setError(null);
      } catch (e) { if (!disposed) setError(e instanceof Error ? e.message : "큐 상태를 읽지 못했습니다."); }
      finally { busy.current = false; if (disposed) void nextRefresh.current?.(); }
    };
    nextRefresh.current = refresh;
    void refresh();
    const timer = window.setInterval(() => void refresh(), 5000);
    return () => { disposed = true; window.clearInterval(timer); if (nextRefresh.current === refresh) nextRefresh.current = null; };
  }, [enabled, query, revision, observedSince]);
  const refresh = useCallback(() => setRevision((current) => current + 1), []);
  const changeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const changed = useCallback(() => {
    if (changeTimer.current) return;
    changeTimer.current = setTimeout(() => { changeTimer.current = undefined; refresh(); }, 750);
  }, [refresh]);
  useEffect(() => () => clearTimeout(changeTimer.current), []);
  const selectQuery = useCallback((next: QueueQuery) => { setQuery((previous) => previous.offset === next.offset ? previous : { offset: next.offset ?? 0 }); }, []);
  return { query, setQuery: selectQuery, snapshot, error, refresh, changed };
}
