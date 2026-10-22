import { useEffect, useRef, useState } from "react";
import { pinterestError, pinterestRequestId } from "./api";
import type { PinterestApi, PinterestDiscovery } from "./types";

/** Remote account data is session-only, separate from persistent local favorites. */
export function usePinterestBoards(api: PinterestApi, active: boolean, generation: number | null) {
  const [path, setPath] = useState("");
  const [pages, setPages] = useState<Record<string, PinterestDiscovery>>({});
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const pending = useRef<string | null>(null);
  const epoch = useRef(0);
  const current = useRef({ active, generation }); current.current = { active, generation };
  const cancel = () => { const id = pending.current; pending.current = null; setLoading(false); if (id) void api.cancel(id).catch(() => {}); };
  useEffect(() => { epoch.current++; cancel(); setPages({}); setPath(""); setError(null); }, [api, generation]);
  useEffect(() => {
    if (!active) cancel();
    return () => { const id = pending.current; pending.current = null; if (id) void api.cancel(id).catch(() => {}); };
  }, [api, active]);
  const load = async (more = false) => {
    if (!active || generation === null || pending.current) return;
    const previous = pages[path], cursor = more ? previous?.cursor : undefined;
    if (more && !cursor) return;
    const id = pinterestRequestId(), started = epoch.current;
    pending.current = id; setLoading(true); setError(null);
    try {
      const result = cursor ? await api.discover(path, path ? "profile" : "saved", id, cursor) : await api.discover(path, path ? "profile" : "saved", id);
      if (pending.current !== id || started !== epoch.current || !current.current.active || current.current.generation !== generation) return;
      if (result.generation !== generation || result.authStatus !== "signedIn") throw new Error("계정 상태가 변경되었습니다. 로그인 상태를 확인해 주세요.");
      const pins = [...new Map([...(more ? previous?.pins ?? [] : []), ...result.pins].map(pin => [pin.id, pin])).values()].slice(0, 1200);
      const boards = [...new Map([...(more ? previous?.boards ?? [] : []), ...result.boards].map(board => [board.path, board])).values()].slice(0, 1000);
      const stopped = result.cursor === cursor && !!cursor || pins.length >= 1200 || boards.length >= 1000;
      setPages(value => ({ ...value, [path]: { ...result, pins, boards, cursor: stopped ? null : result.cursor, limited: result.limited || stopped } }));
    } catch (cause) { if (pending.current === id && started === epoch.current) setError(pinterestError(cause)); }
    finally { if (pending.current === id) { pending.current = null; setLoading(false); } }
  };
  const latestLoad = useRef(load); latestLoad.current = load;
  useEffect(() => { if (active && generation !== null && !pages[path] && !error) void latestLoad.current(); }, [active, generation, path, pages, error]);
  const open = (target: string) => {
    if (target && !pages[""]?.boards.some(board => board.path === target)) return;
    if (target && !pages[target] && Object.keys(pages).length >= 33) { setError("보드 32개를 열었습니다. 새 계정 세션에서 계속 탐색해 주세요."); return; }
    epoch.current++; cancel(); setPath(target); setError(null);
  };
  return { path, root: pages[""], result: pages[path], loading, error, open, reload: () => load(), more: () => load(true) };
}
