import { useEffect, useMemo, useRef, useState } from "react";
import { getDownloadPopularity, popularityPeriods, type PopularityRanks } from "../api/downloadPopularity";
import type { GalleryId } from "../core/types";

/** Four index requests at most per cache day, never one request per album. */
export function useDownloadPopularity(enabled: boolean, ids: readonly GalleryId[]) {
  const membership = useMemo(() => [...ids].sort((a, b) => a - b).join(","), [ids]);
  const [state, setState] = useState<{ ranks: PopularityRanks; message: string }>({ ranks: {}, message: "" });
  const busy = useRef(false);
  const queued = useRef<(() => void) | null>(null);
  useEffect(() => {
    if (!enabled || !membership) return;
    let disposed = false;
    const refresh = async () => {
      if (disposed) return;
      if (busy.current) { queued.current = () => void refresh(); return; }
      busy.current = true;
      const ranks: PopularityRanks = {};
      const messages = new Set<string>();
      try {
        for (const period of popularityPeriods) {
          if (disposed) break;
          try {
            const result = await getDownloadPopularity(period);
            ranks[period] = result.ranks;
            if (result.warning) messages.add(result.warning);
          } catch { messages.add("일부 인기순을 불러오지 못했습니다. 순위 미확인 작품은 최신순으로 표시합니다."); }
        }
        if (!disposed) setState((previous) => ({ ranks: { ...previous.ranks, ...ranks }, message: [...messages].join(" ") }));
      } finally {
        busy.current = false;
        const next = queued.current; queued.current = null; next?.();
      }
    };
    const timer = window.setTimeout(() => void refresh(), 1500);
    return () => { disposed = true; window.clearTimeout(timer); };
  }, [enabled, membership]);
  return state;
}
