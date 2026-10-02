import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Gallery } from "../../core/types";
import { libraryApi, snapshotFor, targetFor, targetKey, type LibraryApi, type Request, type Response, type Summary, type Target } from "./api";
import "./personalLibrary.css";

type LibraryState = {
  api: LibraryApi; summary: Summary | null; index: ReadonlyMap<string, Summary["keys"][number]>;
  pending: boolean; error: string | null; reload(): Promise<void>;
  mutate(request: Request): Promise<Response | null>;
  save(gallery: Gallery, page: number, enabled: boolean): Promise<boolean>;
  membership(target: Target, collectionId: string, enabled: boolean): Promise<boolean>;
};
const LibraryContext = createContext<LibraryState | null>(null);
export const usePersonalLibrary = () => useContext(LibraryContext);

export function PersonalLibraryProvider({ children, api = libraryApi, notify }: { children: ReactNode; api?: LibraryApi; notify?: (message: string) => void }) {
  const [summary, setSummary] = useState<Summary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const busy = useRef(false), mounted = useRef(false);
  const accept = useCallback((response: Response) => {
    if (mounted.current) setSummary((current) => current && current.revision > response.summary.revision ? current : response.summary);
  }, []);
  const reload = useCallback(async () => {
    try { const response = await api({ action: "summary" }); accept(response); if (mounted.current) setError(null); }
    catch { if (mounted.current) setError("내 즐겨찾기를 불러오지 못했습니다. 기존 기록은 유지됩니다."); }
  }, [api, accept]);
  useEffect(() => { mounted.current = true; void reload(); return () => { mounted.current = false; }; }, [reload]);
  const mutate = useCallback(async (request: Request) => {
    if (busy.current) return null;
    busy.current = true; setPending(true); setError(null);
    try { const response = await api(request); accept(response); return response; }
    catch (cause) {
      const message = cause instanceof Error ? cause.message : typeof cause === "string" ? cause : "저장하지 못했습니다. 다시 시도해 주세요.";
      if (mounted.current) setError(message); notify?.(message); return null;
    } finally { busy.current = false; if (mounted.current) setPending(false); }
  }, [api, accept, notify]);
  const save = useCallback(async (gallery: Gallery, page: number, enabled: boolean) => {
    const response = await mutate({ action: "bookmark_set", target: targetFor(gallery, page), enabled, snapshot: enabled ? snapshotFor(gallery) : null });
    if (response) notify?.(`${page ? `${page}페이지를` : "앨범을"} 즐겨찾기${enabled ? "에 저장" : "에서 해제"}했습니다.`);
    return Boolean(response);
  }, [mutate, notify]);
  const membership = useCallback(async (target: Target, collectionId: string, enabled: boolean) => Boolean(await mutate({ action: "membership_set", target, collectionId, enabled })), [mutate]);
  const index = useMemo(() => new Map(summary?.keys.map((key) => [targetKey(key), key]) ?? []), [summary]);
  return <LibraryContext.Provider value={{ api, summary, index, pending, error, reload, mutate, save, membership }}>{children}</LibraryContext.Provider>;
}
