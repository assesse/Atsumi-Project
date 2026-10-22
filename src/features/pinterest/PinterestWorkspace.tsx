import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent } from "react";
import { CardContextMenu, isCardControl, useCardContextMenu } from "../../components/CardContextMenu";
import { splitGalleryTitle } from "../../components/galleryCardLayout";
import { normalizeImagePreviewWidth } from "../platforms/imagePreviewPreferences";
import { createPinterestApi, pinterestError, pinterestRequestId } from "./api";
import { loadPinterestFavorites, savePinterestFavorites } from "./storage";
import { createPinterestBrowserApi, type PinterestBrowserApi } from "./browserApi";
import { PinterestOfficialSearch } from "./PinterestOfficialSearch";
import { createPinterestAccountApi, type PinterestAccountApi, type PinterestAccountSnapshot } from "./accountApi";
import { PinterestAccountControls } from "./PinterestAccountControls";
import { usePinterestBoards } from "./usePinterestBoards";
import { PinterestMasonry } from "./PinterestMasonry";
import { pinterestImageDimensions, type PinterestImageDimensions } from "./pinterestMasonryLayout";
import { normalizePinterestInput, pinterestFavoriteKey, selectPinterestCard, type PinterestApi, type PinterestDiscovery, type PinterestDiscoveryKind, type PinterestDownload, type PinterestDownloadOutcome, type PinterestFavorite, type PinterestPin, type PinterestRemoved, type PinterestView } from "./types";
import "./PinterestWorkspace.css";

export type PinterestWorkspaceProps = {
  active: boolean;
  runtime: "tauri" | "browser-mock";
  privacyMode: boolean;
  view: PinterestView;
  previewWidth?: number;
  onNotice?: (message: string) => void;
  api?: PinterestApi;
  browserApi?: PinterestBrowserApi;
  accountApi?: PinterestAccountApi;
};

type DetailTab = PinterestFavorite & { key: string };
const detailTab = (item: PinterestFavorite): DetailTab => ({ ...item, key: pinterestFavoriteKey(item.pin.id, item.page) });
const detailState = (item: DetailTab) => ({ key: item.key, pin: item.pin, page: item.page ?? 0, preview: true });

function Icon({ kind }: { kind: "bookmark" | "download" | "close" | "left" | "right" | "open" }) {
  const paths = { bookmark: "M7 3h10v18l-5-4-5 4V3Z", download: "M12 3v12m-5-5 5 5 5-5M5 17v4h14v-4", close: "m6 6 12 12M18 6 6 18", left: "m15 5-7 7 7 7", right: "m9 5 7 7-7 7", open: "M14 3h7v7m0-7L10 14M10 3H3v18h18v-7" };
  return <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={paths[kind]} /></svg>;
}

// Only bounded, local data URLs enter the view. Do not widen the app's remote-image CSP.
const imageCache = new WeakMap<PinterestApi, Map<string, string>>();
let imageWorkers = 0;
const imageQueue: (() => void)[] = [];
async function imageSlot<T>(task: () => Promise<T>): Promise<T> {
  if (imageWorkers >= 2) await new Promise<void>(resolve => imageQueue.push(resolve));
  imageWorkers++;
  try { return await task(); }
  finally { imageWorkers--; imageQueue.shift()?.(); }
}

function PinImage({ api, pin, page = 0, thumbnail = true, load = true, removedToken, onDimensions }: { api: PinterestApi; pin: PinterestPin; page?: number; thumbnail?: boolean; load?: boolean; removedToken?: string; onDimensions?: (value: PinterestImageDimensions) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(!thumbnail);
  const [image, setImage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    if (visible || !thumbnail) return;
    if (typeof IntersectionObserver === "undefined") { setVisible(true); return; }
    const observer = new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) { setVisible(true); observer.disconnect(); } }, { rootMargin: "160px" });
    if (ref.current) observer.observe(ref.current);
    return () => observer.disconnect();
  }, [thumbnail, visible]);
  useEffect(() => {
    if (!load || !visible || pin.media.length === 0) return;
    const key = `${removedToken ?? pin.id}:${page}:${thumbnail}`, cached = imageCache.get(api)?.get(key);
    setError(null); setImage(cached ?? null);
    if (cached) return;
    let cancelled = false, started = false;
    const requestId = pinterestRequestId();
    void imageSlot(async () => {
      if (cancelled) return;
      started = true;
      try {
        const data = removedToken ? await api.preview(pin.id, page, requestId, thumbnail, removedToken) : await api.preview(pin.id, page, requestId, thumbnail);
        if (cancelled) return;
        if (!/^data:image\/(?:jpeg|png|webp);base64,[A-Za-z0-9+/=]+$/.test(data) || data.length > 45_000_000) throw new Error("이미지 응답 형식이 올바르지 않습니다.");
        const cache = imageCache.get(api) ?? new Map<string, string>();
        cache.set(key, data);
        // Full previews are much larger than card thumbnails; retain at most two.
        const large = [...cache.keys()].filter(item => item.endsWith(":false"));
        while (large.length > 2) cache.delete(large.shift()!);
        while (cache.size > 48) cache.delete(cache.keys().next().value!);
        while ([...cache.values()].reduce((total, item) => total + item.length, 0) > 32_000_000) cache.delete(cache.keys().next().value!);
        imageCache.set(api, cache); setImage(data);
      } catch (cause) { if (!cancelled) setError(pinterestError(cause)); }
    });
    return () => { cancelled = true; if (started) void api.cancel(requestId).catch(() => {}); };
  }, [api, load, page, pin.id, pin.media.length, removedToken, revision, thumbnail, visible]);
  return <div ref={ref} className="pinterest-image">
    {image ? <img src={image} alt="" decoding="async" onLoad={event => { const dimensions = pinterestImageDimensions(event.currentTarget.naturalWidth, event.currentTarget.naturalHeight); if (dimensions) onDimensions?.(dimensions); }} /> : error ? <div className="pinterest-image-error"><span>이미지를 불러오지 못했습니다</span><button type="button" title={error} onClick={event => { event.stopPropagation(); setRevision(value => value + 1); }}>다시 시도</button></div>
      : pin.media.length === 0 ? <span>표시할 이미지가 없습니다</span> : <span className="spinner" role="status" aria-label="이미지 불러오는 중" />}
  </div>;
}

export function PinterestWorkspace({ active, runtime, privacyMode, view, previewWidth = 180, onNotice, api: providedApi, browserApi: providedBrowser, accountApi: providedAccount }: PinterestWorkspaceProps) {
  const api = useMemo(() => providedApi ?? createPinterestApi(runtime), [providedApi, runtime]);
  const browserApi = useMemo(() => providedBrowser ?? createPinterestBrowserApi(runtime), [providedBrowser, runtime]);
  const accountApi = useMemo(() => providedAccount ?? createPinterestAccountApi(runtime), [providedAccount, runtime]);
  const sessionEnabled = runtime === "tauri" || !!providedAccount;
  const sessionGeneration = useRef<number | null>(null);
  const [sessionRevision, setSessionRevision] = useState(0);
  const [accountStatus, setAccountStatus] = useState<PinterestAccountSnapshot["status"]>("unknown");
  const homeAttempted = useRef(false);
  const [officialSearch, setOfficialSearch] = useState<{ query: string; revision: number } | null>(null);
  const [officialVisible, setOfficialVisible] = useState(false);
  const [draft, setDraft] = useState("");
  const [discoveryKind, setDiscoveryKind] = useState<"keyword" | "profile">("keyword");
  const [history, setHistory] = useState<{ input: string; kind: "keyword" | "profile" }[]>([]);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [tabs, setTabs] = useState<{ key: string; input: string; kind: Exclude<PinterestDiscoveryKind, "saved">; result: PinterestDiscovery; scroll: number; page: number }[]>([]);
  const [currentTab, setCurrentTab] = useState<string | null>(null);
  const workspace = useRef<HTMLElement>(null);
  const navigationRevision = useRef(0);
  const viewPositions = useRef<Record<string, { scroll: number; page: number }>>({});
  const [filter, setFilter] = useState("");
  const [confirmArchive, setConfirmArchive] = useState<PinterestPin[]>([]);
  const [archiveBusy, setArchiveBusy] = useState(false);
  const [removedOpen, setRemovedOpen] = useState(false);
  const [batch, setBatch] = useState<{ total: number; finished: number; failed: number; cancelled: number; running: boolean } | null>(null);
  const batchCancelled = useRef(false);
  const batchRequest = useRef<string | null>(null);
  const [pins, setPins] = useState<PinterestPin[]>([]);
  const [favorites, setFavorites] = useState(loadPinterestFavorites);
  const [favoriteCategory, setFavoriteCategory] = useState<"album" | "page" | "boards">("album");
  const isBoards = view === "favorites" && favoriteCategory === "boards";
  const savedBoards = usePinterestBoards(api, active && !privacyMode && isBoards && accountStatus === "signedIn", sessionGeneration.current);
  const [downloads, setDownloads] = useState<PinterestDownload[]>([]);
  const [downloadsLoading, setDownloadsLoading] = useState(false);
  const [listRevision, setListRevision] = useState(0);
  const [listPage, setListPage] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [request, setRequest] = useState<string | null>(null);
  const requestRef = useRef<string | null>(null);
  const downloadRequests = useRef(new Map<string, string>());
  const [pending, setPending] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const anchor = useRef<string | null>(null);
  const [detail, setDetail] = useState<ReturnType<typeof detailState> | null>(null);
  const [detailTabs, setDetailTabs] = useState<DetailTab[]>([]);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [detailRevision, setDetailRevision] = useState(0);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; batchCancelled.current = true; if (requestRef.current) void api.cancel(requestRef.current).catch(() => {}); const ids = new Set(downloadRequests.current.values()); if (batchRequest.current) ids.add(batchRequest.current); for (const id of ids) void api.cancel(id).catch(() => {}); }; }, [api]);
  useEffect(() => {
    navigationRevision.current++;
    setDetail(null); setDetailTabs([]); setRemovedOpen(false); setSelected(new Set()); anchor.current = null;
    const position = viewPositions.current[`${view}:${favoriteCategory}`];
    setListPage(position?.page ?? 0);
    const initialTop = workspace.current?.scrollTop ?? 0;
    const initialNavigation = navigationRevision.current;
    const frame = requestAnimationFrame(() => {
      // A delayed view restoration must not overwrite a user scroll/tab change.
      if (workspace.current && workspace.current.scrollTop === initialTop && navigationRevision.current === initialNavigation) workspace.current.scrollTop = position?.scroll ?? 0;
    });
    return () => cancelAnimationFrame(frame);
  }, [view, active, favoriteCategory]);
  useEffect(() => {
    if (!active || view !== "downloads") return;
    let cancelled = false;
    setDownloadsLoading(true); setError(null);
    void api.downloads().then(items => { if (!cancelled) setDownloads(items); }).catch(cause => { if (!cancelled) setError(pinterestError(cause)); }).finally(() => { if (!cancelled) setDownloadsLoading(false); });
    return () => { cancelled = true; };
  }, [active, api, listRevision, view]);

  const notify = (message: string) => { onNotice?.(message); };
  const rememberListPage = (page: number, scroll = workspace.current?.scrollTop ?? 0) => {
    setListPage(page); viewPositions.current[`${view}:${favoriteCategory}`] = { page, scroll };
  };
  const cancelLoad = () => { const current = requestRef.current; requestRef.current = null; setRequest(null); if (current) void api.cancel(current).catch(() => {}); };
  const receiveAccount = (snapshot: PinterestAccountSnapshot) => {
    const previous = sessionGeneration.current;
    if (previous !== null && snapshot.generation < previous) return;
    setAccountStatus(snapshot.status);
    if (snapshot.status === "signedOut") setFavoriteCategory(category => category === "boards" ? "album" : category);
    sessionGeneration.current = snapshot.generation;
    if (previous === null) { setSessionRevision(value => value + 1); return; }
    if (previous === snapshot.generation) return;
    // Session-scoped remote results must not survive an account change. Local
    // favorites/media stay intact; native also fences in-flight publication.
    cancelLoad(); navigationRevision.current++;
    imageCache.delete(api); setTabs([]); setPins([]); setCurrentTab(null);
    setDetail(null); setDetailTabs([]); setSelected(new Set()); anchor.current = null;
    setOfficialSearch(null); setOfficialVisible(false); setError(null);
    homeAttempted.current = false; setSessionRevision(value => value + 1);
  };
  const acceptGeneration = (generation: number | undefined) => {
    if (generation === undefined) return !sessionEnabled;
    if (sessionGeneration.current !== null && generation < sessionGeneration.current) return false;
    if (sessionGeneration.current !== generation) receiveAccount({ status: "unknown", open: false, busy: false, generation });
    return true;
  };
  const load = async (input = draft, kind: Exclude<PinterestDiscoveryKind, "saved"> = discoveryKind) => {
    if (requestRef.current) { cancelLoad(); return; }
    if (kind === "home") { input = ""; homeAttempted.current = true; }
    const id = normalizePinterestInput(input);
    if ((kind !== "home" && !input.trim()) || (input.includes("://") && !id && !/^https:\/\/(?:(?:www|[a-z]{2})\.)?pinterest\.com\//.test(input))) { setError("검색어, 프로필 이름 또는 Pinterest 전체 링크를 입력해 주세요."); return; }
    if (input.startsWith("@") || (input.includes("pinterest.com/") && !id)) kind = "profile";
    setOfficialVisible(false);
    const key = `${kind}:${input.trim().toLowerCase()}`;
    const retained = !id && tabs.find(tab => tab.key === key);
    if (retained) { activateTab(retained.key); setHistoryOpen(false); return; }
    if (!id && tabs.length >= 64) { setError("탐색 탭이 64개 열려 있습니다. 사용하지 않는 탭을 닫아 주세요."); return; }
    if (kind !== "home") {
      const historyKind = kind;
      setHistory(previous => [{ input: input.trim(), kind: historyKind }, ...previous.filter(item => item.input !== input.trim() || item.kind !== historyKind)].slice(0, 10));
    }
    setHistoryOpen(false);
    const startedNavigation = navigationRevision.current;
    setTabs(previous => previous.map(tab => tab.key === currentTab ? { ...tab, scroll: workspace.current?.scrollTop ?? tab.scroll, page: listPage } : tab));
    const requestId = pinterestRequestId(); requestRef.current = requestId; setRequest(requestId); setError(null);
    try {
      const result = id ? await api.get(id, requestId) : await api.discover(input, kind, requestId);
      if (requestRef.current !== requestId || !mounted.current) return;
      if ("pins" in result) {
        // Native can discover a session transition before the account poll.
        // Defer that result and reload under the new generation, never attach it
        // to tabs from the prior account.
        const generationBefore = sessionGeneration.current;
        if (!acceptGeneration(result.generation)) return;
        if (generationBefore !== null && generationBefore !== result.generation) return;
        setTabs(previous => {
          const next = { key, input, kind, result, scroll: 0, page: 0 };
          return previous.some(tab => tab.key === key) ? previous.map(tab => tab.key === key ? next : tab) : [...previous, next];
        });
        // A request belongs to its destination tab, not whichever tab is now focused.
        if (navigationRevision.current !== startedNavigation) return;
        setCurrentTab(key); setPins(result.pins); workspace.current?.scrollTo?.({ top: 0 });
      } else {
        setPins(previous => [result, ...previous.filter(item => item.id !== result.id)].slice(0, 120));
        setCurrentTab(null);
        if (result.unsupportedVideo) notify("영상 핀은 원본 저장을 지원하지 않습니다. 제공된 표지 이미지만 확인할 수 있습니다.");
      }
      rememberListPage(0, 0);
      setSelected(new Set());
    } catch (cause) { if (requestRef.current === requestId && mounted.current) setError(pinterestError(cause)); }
    finally { if (requestRef.current === requestId && mounted.current) { requestRef.current = null; setRequest(null); } }
  };
  const loadMore = async () => {
    const tab = tabs.find(item => item.key === currentTab);
    if (!tab?.result.cursor || requestRef.current || tab.result.pins.length >= 1200) return;
    const requestId = pinterestRequestId(), startedNavigation = navigationRevision.current;
    requestRef.current = requestId; setRequest(requestId); setError(null);
    try {
      const result = await api.discover(tab.input, tab.kind, requestId, tab.result.cursor);
      if (!mounted.current || requestRef.current !== requestId || !acceptGeneration(result.generation)) return;
      if (sessionEnabled && result.generation !== tab.result.generation) return;
      const merged = [...new Map([...tab.result.pins, ...result.pins].map(pin => [pin.id, pin])).values()].slice(0, 1200);
      const next = { ...result, pins: merged, cursor: merged.length >= 1200 || result.cursor === tab.result.cursor ? null : result.cursor };
      setTabs(previous => previous.map(item => item.key === tab.key ? { ...item, result: next } : item));
      if (startedNavigation === navigationRevision.current) setPins(merged);
    } catch (cause) { if (mounted.current && requestRef.current === requestId) setError(pinterestError(cause)); }
    finally { if (mounted.current && requestRef.current === requestId) { requestRef.current = null; setRequest(null); } }
  };
  const currentLoad = useRef(load); currentLoad.current = load;
  useEffect(() => {
    if (!sessionEnabled || sessionGeneration.current === null || !active || privacyMode || view !== "explore" || homeAttempted.current || requestRef.current) return;
    homeAttempted.current = true;
    void currentLoad.current("", "home");
  }, [active, privacyMode, sessionEnabled, sessionRevision, view]);
  const toggleFavorite = (pin: PinterestPin, page: number | null) => {
    const key = pinterestFavoriteKey(pin.id, page);
    const next = favorites.some(item => pinterestFavoriteKey(item.pin.id, item.page) === key)
      ? favorites.filter(item => pinterestFavoriteKey(item.pin.id, item.page) !== key) : [{ pin, page }, ...favorites];
    if (!savePinterestFavorites(next)) { setError("즐겨찾기를 저장하지 못했습니다. 앱 저장 공간을 확인해 주세요."); return; }
    setFavorites(next);
  };
  const download = async (pin: PinterestPin) => {
    const existing = downloadRequests.current.get(pin.id);
    if (existing) { if (existing === batchRequest.current) batchCancelled.current = true; await api.cancel(existing).catch(() => {}); return false; }
    if (pin.unsupportedVideo || !pin.media.length) return false;
    const requestId = pinterestRequestId(); downloadRequests.current.set(pin.id, requestId);
    setPending(previous => new Set(previous).add(pin.id)); setError(null);
    try {
      const record = await api.download(pin.id, requestId);
      if (mounted.current) { setDownloads(previous => [record, ...previous.filter(item => item.pin.id !== pin.id)]); notify("원본 저장 완료"); }
      return true;
    } catch (cause) { if (mounted.current) setError(pinterestError(cause)); return false; }
    finally { downloadRequests.current.delete(pin.id); if (mounted.current) setPending(previous => { const next = new Set(previous); next.delete(pin.id); return next; }); }
  };
  const downloadMany = async (items: PinterestPin[]) => {
    if (batchRequest.current) return;
    const unique = [...new Map(items.filter(pin => !pin.unsupportedVideo && !downloadRequests.current.has(pin.id) && !downloads.some(record => record.pin.id === pin.id)).map(pin => [pin.id, pin])).values()].slice(0, 128);
    if (!unique.length) return;
    const requestId = pinterestRequestId(), ids = new Set(unique.map(pin => pin.id)), settled = new Set<string>();
    batchRequest.current = requestId;
    batchCancelled.current = false;
    for (const pin of unique) downloadRequests.current.set(pin.id, requestId);
    setPending(previous => new Set([...previous, ...ids])); setError(null);
    setBatch({ total: unique.length, finished: 0, failed: 0, cancelled: 0, running: true });
    const applyOutcome = (outcome: PinterestDownloadOutcome) => {
      if (batchRequest.current !== requestId || !mounted.current || !ids.has(outcome.pinId) || settled.has(outcome.pinId)) return;
      settled.add(outcome.pinId);
      downloadRequests.current.delete(outcome.pinId);
      setPending(previous => { const next = new Set(previous); next.delete(outcome.pinId); return next; });
      if (outcome.status === "completed" && outcome.record) {
        const record = outcome.record;
        setDownloads(previous => [record, ...previous.filter(item => item.pin.id !== record.pin.id)]);
      } else if (outcome.error) setError(outcome.error);
      setBatch(previous => previous ? { ...previous, finished: previous.finished + 1, failed: previous.failed + Number(outcome.status === "failed"), cancelled: previous.cancelled + Number(outcome.status === "cancelled") } : null);
    };
    try {
      // Native owns the complete accepted queue and its shutdown guard; never
      // chain per-pin invokes here, which creates an unguarded inter-item gap.
      const outcomes = await api.downloadBatch([...ids], requestId, applyOutcome);
      outcomes.forEach(applyOutcome);
    } catch (cause) {
      if (mounted.current) setError(pinterestError(cause));
      for (const pinId of ids) applyOutcome({ pinId, status: batchCancelled.current ? "cancelled" : "failed" });
    } finally {
      for (const pinId of ids) if (downloadRequests.current.get(pinId) === requestId) downloadRequests.current.delete(pinId);
      if (batchRequest.current === requestId) batchRequest.current = null;
      if (mounted.current) {
        setPending(previous => new Set([...previous].filter(id => !ids.has(id))));
        setBatch(previous => previous ? { ...previous, running: false } : null);
      }
    }
  };
  const cancelBatch = () => { batchCancelled.current = true; if (batchRequest.current) void api.cancel(batchRequest.current).catch(() => {}); };
  const libraryAction = async (action: "archive" | "restore" | "folder", id: string) => api.libraryAction(action, id, pinterestRequestId());
  const archive = async () => {
    if (archiveBusy) return;
    setArchiveBusy(true); setError(null);
    try {
      for (const pin of confirmArchive) {
        await libraryAction("archive", pin.id);
        setDownloads(previous => previous.filter(record => record.pin.id !== pin.id));
        setConfirmArchive(previous => previous.filter(item => item.id !== pin.id));
      }
      setSelected(new Set()); setConfirmArchive([]); notify("보관함에서 제외했습니다. ‘제외된 핀’에서 복원할 수 있습니다.");
    } catch (cause) { setError(pinterestError(cause)); }
    finally { setArchiveBusy(false); }
  };
  useEffect(() => {
    setDetailError(null);
    if (!detail?.pin.summaryOnly) return;
    let current = true;
    const requestId = pinterestRequestId(), id = detail.pin.id;
    void api.get(id, requestId).then(pin => {
      if (!current) return;
      setDetail(previous => previous?.pin.id === id ? { ...previous, pin } : previous);
      setDetailTabs(previous => previous.map(item => item.pin.id === id ? { ...item, pin } : item));
      setPins(previous => previous.map(item => item.id === id ? pin : item));
      setFavorites(previous => {
        const next = previous.map(item => item.pin.id === id ? { ...item, pin } : item);
        if (next.some((item, index) => item !== previous[index])) savePinterestFavorites(next);
        return next;
      });
    }).catch(cause => { if (current) setDetailError(pinterestError(cause)); });
    return () => { current = false; void api.cancel(requestId).catch(() => {}); };
  }, [api, detail?.pin.id, detail?.pin.summaryOnly, detailRevision]);
  const favoriteKeys = useMemo(() => new Set(favorites.map(item => pinterestFavoriteKey(item.pin.id, item.page))), [favorites]);
  const unfilteredEntries: PinterestFavorite[] = isBoards ? (accountStatus === "signedIn" ? savedBoards.result?.pins ?? [] : []).map(pin => ({ pin, page: null })) : view === "favorites" ? favorites.filter(item => favoriteCategory === "album" ? item.page === null : item.page !== null)
    : (view === "downloads" ? downloads.map(record => record.pin) : pins).map(pin => ({ pin, page: null }));
  const entries = view === "explore" || !filter.trim() ? unfilteredEntries : unfilteredEntries.filter(item => `${item.pin.title} ${item.pin.creatorName ?? ""} ${item.pin.id}`.toLocaleLowerCase().includes(filter.trim().toLocaleLowerCase()));
  const entryIds = entries.map(item => pinterestFavoriteKey(item.pin.id, item.page));
  const pageCount = Math.max(1, Math.ceil(entries.length / 48));
  const visiblePage = Math.min(listPage, pageCount - 1);
  const visibleEntries = entries.slice(visiblePage * 48, (visiblePage + 1) * 48);
  const downloadIds = new Set(downloads.map(item => item.pin.id));
  const selectedPins = entries.filter(item => selected.has(pinterestFavoriteKey(item.pin.id, item.page))).map(item => item.pin);
  const discovery = tabs.find(tab => tab.key === currentTab)?.result;
  const activateTab = (key: string) => {
    const tab = tabs.find(item => item.key === key); if (!tab) return;
    if (key === currentTab) { setDraft(tab.input); setDiscoveryKind(tab.kind === "home" ? "keyword" : tab.kind); return; }
    navigationRevision.current++;
    setTabs(previous => previous.map(item => item.key === currentTab ? { ...item, scroll: workspace.current?.scrollTop ?? 0, page: listPage } : item));
    setCurrentTab(key); setPins(tab.result.pins); setDraft(tab.input); setDiscoveryKind(tab.kind === "home" ? "keyword" : tab.kind); setSelected(new Set()); rememberListPage(tab.page, tab.scroll); anchor.current = null;
    requestAnimationFrame(() => { if (workspace.current) workspace.current.scrollTop = tab.scroll; });
  };
  const closeSearchTab = (key: string) => {
    const remaining = tabs.filter(item => item.key !== key); setTabs(remaining);
    if (key !== currentTab) return;
    navigationRevision.current++;
    const next = remaining[Math.min(tabs.findIndex(item => item.key === key), remaining.length - 1)];
    setCurrentTab(next?.key ?? null); setPins(next?.result.pins ?? []); setDraft(next?.input ?? ""); setDiscoveryKind(next?.kind === "profile" ? "profile" : "keyword");
    rememberListPage(next?.page ?? 0, next?.scroll ?? 0); setSelected(new Set()); anchor.current = null;
    requestAnimationFrame(() => { if (workspace.current) workspace.current.scrollTop = next?.scroll ?? 0; });
  };
  const openDetail = (item: PinterestFavorite) => { const tab = detailTab(item); setSelected(new Set()); setDetailTabs([tab]); setDetail(detailState(tab)); };
  const importOfficialPin = async (id: string) => {
    if (requestRef.current) throw new Error("진행 중인 불러오기를 마친 뒤 다시 선택해 주세요.");
    const startedNavigation = navigationRevision.current;
    const requestId = pinterestRequestId(); requestRef.current = requestId; setRequest(requestId); setError(null);
    try {
      const pin = await api.get(id, requestId);
      if (!mounted.current || requestRef.current !== requestId || navigationRevision.current !== startedNavigation) return;
      setPins(previous => [pin, ...previous.filter(item => item.id !== pin.id)].slice(0, 120));
      setCurrentTab(null); rememberListPage(0, 0); setOfficialVisible(false); openDetail({ pin, page: null });
    } finally { if (mounted.current && requestRef.current === requestId) { requestRef.current = null; setRequest(null); } }
  };
  const openDetails = (items: PinterestFavorite[]) => { if (!items.length) return; openDetail(items[0]!); setDetailTabs(items.map(detailTab)); };
  const detailSource = detailTabs.length > 1 ? detailTabs : entries.map(detailTab);
  const detailIndex = detailSource.findIndex(item => item.key === detail?.key);
  const moveDetail = (direction: number) => {
    if (!detail) return;
    const next = detailSource[detailIndex + direction];
    if (next) setDetail(detailState(next));
  };
  return <section ref={workspace} hidden={!active} className={`pinterest-workspace${privacyMode ? " is-private" : ""}`} style={{ "--pinterest-preview-width": `${normalizeImagePreviewWidth(previewWidth)}px` } as CSSProperties} aria-label="Pinterest" onScroll={event => { if (event.target === event.currentTarget) viewPositions.current[`${view}:${favoriteCategory}`] = { scroll: event.currentTarget.scrollTop, page: listPage }; }} onKeyDown={event => {
    if (!active || event.defaultPrevented || detail || removedOpen || confirmArchive.length || event.nativeEvent.isComposing || isCardControl(event.target)) return;
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") { event.preventDefault(); setSelected(new Set(entryIds)); }
    if (event.key === "Escape") { setSelected(new Set()); (document.activeElement as HTMLElement)?.blur?.(); }
    if (event.key === "Enter" && selected.size > 1) { event.preventDefault(); if (event.ctrlKey || event.metaKey) void downloadMany(selectedPins); else openDetails(entries.filter(item => selected.has(pinterestFavoriteKey(item.pin.id, item.page)))); }
  }}>
    <header className="pinterest-toolbar" data-tour="pinterest-input">
      {view === "explore" ? <button type="button" className="secondary-button" aria-label="Pinterest 홈 피드" onClick={() => void load("", "home")}>홈</button> : null}
      {view === "explore" ? <form className="pinterest-input" onSubmit={event => { event.preventDefault(); void load(); }}>
        <div className="pinterest-search-field"><input aria-label="Pinterest 검색어·프로필·핀 링크" placeholder={discoveryKind === "profile" ? "@프로필 이름 또는 보드 주소" : "검색어 또는 핀 링크"} value={draft} onChange={event => setDraft(event.target.value)} onFocus={() => setHistoryOpen(true)} onKeyDown={event => { if (event.key === "Escape") { event.stopPropagation(); setHistoryOpen(false); } }} onBlur={event => { if (!event.currentTarget.parentElement?.contains(event.relatedTarget as Node | null)) setHistoryOpen(false); }} maxLength={2048} />
          {historyOpen && history.length ? <div className="pinterest-history" aria-label="최근 탐색 기록" onBlur={event => { if (!event.currentTarget.parentElement?.contains(event.relatedTarget as Node | null)) setHistoryOpen(false); }}>{history.map(item => <div key={`${item.kind}:${item.input}`}><button type="button" onClick={() => { setDraft(item.input); setDiscoveryKind(item.kind); void load(item.input, item.kind); }}>{item.input}</button><button type="button" aria-label={`${item.input} 기록 삭제`} onClick={() => setHistory(previous => previous.filter(value => value !== item))}><Icon kind="close" /></button></div>)}<button type="button" className="pinterest-history-clear" onClick={() => setHistory([])}>기록 모두 지우기</button></div> : null}
        </div>
        <select aria-label="Pinterest 탐색 종류" value={discoveryKind} onChange={event => setDiscoveryKind(event.target.value as "keyword" | "profile")}><option value="keyword">키워드</option><option value="profile">프로필·보드</option></select>
        <button type="submit" className="primary-button" title={request ? "불러오기 취소" : "핀 불러오기"}>{request ? <><span className="spinner" /> 취소</> : "불러오기"}</button>
      </form> : <><h1>{view === "favorites" ? "즐겨찾기" : "다운로드"}</h1><input className="pinterest-local-search" aria-label="저장한 핀 검색" placeholder="제목·작가 검색" value={filter} onChange={event => { setFilter(event.target.value); rememberListPage(0); }} /></>}
      {view === "explore" && (runtime === "tauri" || providedBrowser) ? <button type="button" className="secondary-button" disabled={!draft.trim() || !!request} onClick={() => {
        setHistoryOpen(false); setError(null); setOfficialVisible(true);
        setOfficialSearch(previous => previous?.query === draft.trim() ? previous : { query: draft.trim(), revision: (previous?.revision ?? 0) + 1 });
      }}>공식 웹에서 검색</button> : null}
      {view === "favorites" ? <div className="pinterest-segment" aria-label="즐겨찾기 종류">{(["album", "page"] as const).map(category => <button type="button" key={category} aria-pressed={favoriteCategory === category} onClick={() => setFavoriteCategory(category)}>{category === "album" ? "앨범" : "페이지"}</button>)}</div> : null}
      {view === "favorites" && sessionEnabled ? <button type="button" className="secondary-button" disabled={accountStatus !== "signedIn"} aria-pressed={isBoards} onClick={() => setFavoriteCategory("boards")}>내 보드</button> : null}
      {view === "downloads" ? <button type="button" className="secondary-button" disabled={downloadsLoading} onClick={() => setListRevision(value => value + 1)}>{downloadsLoading ? <span className="spinner" /> : "새로고침"}</button> : null}
      {view === "downloads" ? <button type="button" className="secondary-button" onClick={() => setRemovedOpen(true)}>제외된 핀</button> : null}
    </header>
    <PinterestAccountControls api={accountApi} active={active && !privacyMode} enabled={sessionEnabled && !privacyMode} onSnapshot={receiveAccount} />
    {officialSearch ? <PinterestOfficialSearch api={browserApi} query={officialSearch.query} revision={officialSearch.revision} active={active && view === "explore" && officialVisible && !detail && !removedOpen && !confirmArchive.length && !historyOpen} privacyMode={privacyMode} onImport={importOfficialPin} onReturn={() => { cancelLoad(); setOfficialVisible(false); }} /> : null}
    <div hidden={view === "explore" && officialVisible}>
    {isBoards ? <div className="pinterest-saved-boards">
      <div className="pinterest-boards-toolbar">{savedBoards.path ? <button type="button" className="secondary-button" onClick={() => { savedBoards.open(""); setSelected(new Set()); rememberListPage(0, 0); }}>보드 목록</button> : null}<span>{savedBoards.path ? savedBoards.root?.boards.find(board => board.path === savedBoards.path)?.title : "내 보드"}</span><button type="button" className="secondary-button" disabled={savedBoards.loading || privacyMode || accountStatus !== "signedIn"} onClick={() => void savedBoards.reload()}>{savedBoards.loading ? <span className="spinner" /> : "새로고침"}</button></div>
      {savedBoards.error ? <p role="alert" className="pinterest-message is-error">{savedBoards.error}</p> : null}
      {!savedBoards.path && accountStatus === "signedIn" ? <div className="pinterest-boards" aria-label="내 Pinterest 보드">{savedBoards.root?.boards.filter(board => !filter.trim() || board.title.toLocaleLowerCase().includes(filter.trim().toLocaleLowerCase())).map(board => <button type="button" key={board.path} onClick={() => { savedBoards.open(board.path); setSelected(new Set()); anchor.current = null; rememberListPage(0, 0); }}>{board.title}</button>)}</div> : null}
      {savedBoards.result?.limited && !savedBoards.result.cursor ? <p className="pinterest-hint">현재 제공된 목록만 표시합니다.</p> : null}
    </div> : null}
    {view === "explore" ? <><p className="pinterest-hint">로그인하면 내 홈 피드로 탐색합니다. 핀과 페이지를 즐겨찾기하고 원본을 저장할 수 있습니다.</p>
      {tabs.length ? <nav className="pinterest-search-tabs" aria-label="Pinterest 탐색 탭">{tabs.map(tab => <span key={tab.key} className={tab.key === currentTab ? "is-active" : ""}><button type="button" aria-pressed={tab.key === currentTab} onClick={() => activateTab(tab.key)}>{tab.result.title}</button><button type="button" aria-label={`${tab.result.title} 탐색 닫기`} onClick={() => closeSearchTab(tab.key)}><Icon kind="close" /></button></span>)}</nav> : null}
      {discovery?.boards.length ? <div className="pinterest-boards" aria-label="공개 보드">{discovery.boards.map(board => <button type="button" key={board.path} disabled={!!request} onClick={() => { setDraft(board.path); setDiscoveryKind("profile"); void load(board.path, "profile"); }}>{board.title}</button>)}</div> : null}
      {discovery?.limited && !discovery.cursor ? <p className="pinterest-hint">불러온 핀 {discovery.pins.length}개 · 추가 목록이 제공되지 않았습니다.</p> : null}</> : null}
    {error ? <div role="alert" className="pinterest-message is-error">{error}<button type="button" aria-label="오류 닫기" onClick={() => setError(null)}><Icon kind="close" /></button></div> : null}
    {batch ? <div className="pinterest-batch" role="status"><span>{batch.running ? "다운로드 중" : "처리 종료"} · {batch.finished}/{batch.total}{batch.failed ? ` · 실패 ${batch.failed}` : ""}{batch.cancelled ? ` · 취소 ${batch.cancelled}` : ""}</span><progress max={Math.max(batch.total, 1)} value={batch.finished} /><button type="button" onClick={() => batch.running ? cancelBatch() : setBatch(null)}>{batch.running ? "취소" : "닫기"}</button></div> : null}
    {downloadsLoading ? <div className="pinterest-empty" data-tour="pinterest-content" role="status"><span className="spinner" /> 다운로드 목록 불러오는 중</div>
      : entries.length ? <PinterestMasonry previewWidth={previewWidth} contextKey={`${view}:${favoriteCategory}:${savedBoards.path}:${visiblePage}:${currentTab ?? ""}:${filter}:${sessionRevision}`} tour="pinterest-content">{visibleEntries.map(item => {
        const key = pinterestFavoriteKey(item.pin.id, item.page);
        return <PinCard key={key} api={api} item={item} selected={selected.has(key)} multiple={selected.size > 1} favorite={favoriteKeys.has(pinterestFavoriteKey(item.pin.id, item.page))}
          pending={pending.has(item.pin.id)} downloaded={downloadIds.has(item.pin.id)} onOpen={() => openDetail(item)}
          missingFiles={downloads.some(record => record.pin.id === item.pin.id && record.missingFiles)}
          onSelect={event => { if (isCardControl(event.target)) return; const next = selectPinterestCard(entryIds, selected, key, anchor.current, event); setSelected(next); if (!next.size) event.currentTarget.blur(); if (!event.shiftKey) anchor.current = key; }}
          onFavorite={() => toggleFavorite(item.pin, item.page)} onDownload={() => void download(item.pin)}
          onFolder={downloadIds.has(item.pin.id) ? () => void libraryAction("folder", item.pin.id).catch(cause => setError(pinterestError(cause))) : undefined}
          onArchive={view === "downloads" ? () => setConfirmArchive([item.pin]) : undefined} />;
      })}</PinterestMasonry> : isBoards ? <div className="pinterest-empty" data-tour="pinterest-content" role="status">{savedBoards.loading ? <><span className="spinner" />보드 불러오는 중</> : savedBoards.root?.boards.length && !savedBoards.path ? "보드를 선택해 저장한 핀을 확인하세요." : savedBoards.error ? "보드 목록을 불러오지 못했습니다." : savedBoards.result && !savedBoards.result.limited ? "표시할 항목이 없습니다." : "로그인한 계정의 보드를 확인합니다."}</div> : <div className="pinterest-empty" data-tour="pinterest-content"><h2>{view === "explore" ? request ? "홈·핀 불러오는 중" : "마음에 드는 핀을 찾아보세요" : view === "favorites" ? "즐겨찾기가 없습니다" : "저장한 핀이 없습니다"}</h2><p>{view === "explore" ? "로그인 후 홈 피드를 보거나, 검색어·프로필·핀 링크로 탐색하세요." : view === "favorites" ? "핀 또는 페이지의 책갈피 버튼으로 추가하세요." : "핀의 원본 저장을 선택하면 여기에 표시됩니다."}</p>{view === "explore" ? <button type="button" className="secondary-button" disabled={!!request} onClick={() => void load("", "home")}>{request ? <span className="spinner" /> : null}홈 피드 불러오기</button> : null}</div>}
    {pageCount > 1 ? <nav className="pinterest-pagination" aria-label="핀 목록 페이지"><button type="button" aria-label="이전 목록 페이지" disabled={visiblePage === 0} onClick={() => rememberListPage(visiblePage - 1)}><Icon kind="left" /></button><span>{visiblePage + 1} / {pageCount}</span><button type="button" aria-label="다음 목록 페이지" disabled={visiblePage === pageCount - 1} onClick={() => rememberListPage(visiblePage + 1)}><Icon kind="right" /></button></nav> : null}
    {view === "explore" && discovery?.cursor ? <div className="pinterest-load-more"><button type="button" className="secondary-button" disabled={!!request} onClick={() => void loadMore()}>{request ? <span className="spinner" /> : null}핀 더 불러오기</button></div> : null}
    {isBoards && savedBoards.result?.cursor ? <div className="pinterest-load-more"><button type="button" className="secondary-button" disabled={savedBoards.loading || privacyMode} onClick={() => void savedBoards.more()}>{savedBoards.loading ? <span className="spinner" /> : null}{savedBoards.path ? "핀 더 불러오기" : "보드 더 불러오기"}</button></div> : null}
    {selected.size > 1 ? <div className="pinterest-selection" role="toolbar" aria-label="선택한 핀 작업"><b>{selected.size}개 선택</b><button type="button" onClick={() => {
      const picked = entries.filter(item => selected.has(pinterestFavoriteKey(item.pin.id, item.page)));
      const localFavorites = view === "favorites" && !isBoards;
      const next = localFavorites ? favorites.filter(item => !selected.has(pinterestFavoriteKey(item.pin.id, item.page))) : [...favorites];
      if (!localFavorites) for (const item of picked) if (!next.some(value => pinterestFavoriteKey(value.pin.id, value.page) === pinterestFavoriteKey(item.pin.id, item.page))) next.push(item);
      if (savePinterestFavorites(next)) { setFavorites(next); if (localFavorites) setSelected(new Set()); } else setError("즐겨찾기를 저장하지 못했습니다.");
    }}>{view === "favorites" && !isBoards ? "즐겨찾기 해제" : "즐겨찾기 추가"}</button><button type="button" disabled={batch?.running} onClick={() => void downloadMany(selectedPins)}>원본 다운로드</button>{view === "downloads" ? <button type="button" onClick={() => setConfirmArchive(selectedPins)}>보관함에서 제외</button> : null}<button type="button" onClick={() => { setSelected(new Set()); (document.activeElement as HTMLElement)?.blur?.(); }}>선택 해제</button></div> : null}
    </div>
    {detail ? <PinDetail key={detail.key} api={api} pin={detail.pin} initialPage={detail.page} initialPreview={detail.preview} privacyMode={privacyMode} favorites={favoriteKeys}
      pending={pending.has(detail.pin.id)} downloaded={downloadIds.has(detail.pin.id)} onClose={() => setDetail(null)} onMove={moveDetail}
      canPrevious={detailIndex > 0} canNext={detailIndex >= 0 && detailIndex < detailSource.length - 1}
      error={detailError ?? error} metadataFailed={!!detailError} onRetry={() => setDetailRevision(value => value + 1)}
      tabs={detailTabs} activeTabKey={detail.key} onTab={item => setDetail(detailState(item))}
      onCloseTab={item => { const remaining = detailTabs.filter(tab => tab.key !== item.key); setDetailTabs(remaining); if (item.key === detail.key) setDetail(remaining[0] ? detailState(remaining[0]) : null); }}
      onCreator={detail.pin.creatorPath ? () => { const path = detail.pin.creatorPath!; setDetail(null); setDraft(path); setDiscoveryKind("profile"); void load(path, "profile"); } : undefined}
      onFavorite={page => toggleFavorite(detail.pin, page)} onDownload={() => void download(detail.pin)} onExternal={() => void api.openExternal(detail.pin.id).catch(cause => setError(pinterestError(cause)))} /> : null}
    {confirmArchive.length ? <div className="pinterest-detail-backdrop"><section className="pinterest-confirm" role="alertdialog" aria-modal="true" aria-label="보관함에서 제외" data-gallery-shortcuts-suspended onKeyDown={event => { event.stopPropagation(); if (event.key === "Escape" && !archiveBusy) { event.preventDefault(); setConfirmArchive([]); } if (event.key === "Tab") { const controls = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")]; const next = event.shiftKey ? controls.at(-1) : controls[0]; if (document.activeElement === (event.shiftKey ? controls[0] : controls.at(-1))) { event.preventDefault(); next?.focus(); } } }}><h2>{confirmArchive.length}개 핀을 보관함에서 제외할까요?</h2><p>원본과 즐겨찾기는 유지되며 ‘제외된 핀’에서 복원할 수 있습니다. 저장 공간은 줄어들지 않습니다.</p><footer><button type="button" autoFocus disabled={archiveBusy} onClick={() => setConfirmArchive([])}>취소</button><button type="button" disabled={archiveBusy} onClick={() => void archive()}>{archiveBusy ? <span className="spinner" /> : "제외"}</button></footer></section></div> : null}
    {removedOpen ? <RemovedPins api={api} privacyMode={privacyMode} previewWidth={previewWidth} favorites={favoriteKeys} onFavorite={toggleFavorite} onRestored={() => setListRevision(value => value + 1)} onClose={() => { setRemovedOpen(false); setListRevision(value => value + 1); }} /> : null}
  </section>;
}

function RemovedPins({ api, privacyMode, previewWidth, favorites, onFavorite, onRestored, onClose }: {
  api: PinterestApi; privacyMode: boolean; previewWidth: number; favorites: Set<string>; onFavorite: (pin: PinterestPin, page: number | null) => void; onRestored: () => void; onClose: () => void;
}) {
  const [records, setRecords] = useState<PinterestRemoved[]>([]), [loading, setLoading] = useState(true), [revision, setRevision] = useState(0);
  const [error, setError] = useState<string | null>(null), [page, setPage] = useState(0), [filter, setFilter] = useState("");
  const [selected, setSelected] = useState(new Set<string>()), [detail, setDetail] = useState<PinterestRemoved | null>(null);
  const [detailTabs, setDetailTabs] = useState<PinterestRemoved[]>([]);
  const [restoring, setRestoring] = useState(false), [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const dialog = useRef<HTMLElement>(null), anchor = useRef<string | null>(null), alive = useRef(true), stop = useRef(false);
  const restoreRequest = useRef<string | null>(null), listRequest = useRef<string | null>(null);
  useEffect(() => {
    alive.current = true;
    const previous = document.activeElement; dialog.current?.focus({ preventScroll: true });
    return () => { alive.current = false; stop.current = true; if (restoreRequest.current) void api.cancel(restoreRequest.current).catch(() => {}); if (previous instanceof HTMLElement && previous.isConnected) previous.focus({ preventScroll: true }); };
  }, [api]);
  useEffect(() => {
    let current = true;
    const id = pinterestRequestId(); listRequest.current = id; setLoading(true); setError(null);
    void api.removed(id).then(items => { if (current) setRecords(items); }).catch(cause => { if (current) setError(pinterestError(cause)); }).finally(() => { if (current) { setLoading(false); listRequest.current = null; } });
    return () => { current = false; if (listRequest.current === id) listRequest.current = null; void api.cancel(id).catch(() => {}); };
  }, [api, revision]);
  const visible = records.filter(item => `${item.record.pin.title} ${item.record.pin.creatorName ?? ""} ${item.record.pin.id}`.toLocaleLowerCase().includes(filter.trim().toLocaleLowerCase()));
  const ids = visible.map(item => item.token), pageCount = Math.max(1, Math.ceil(visible.length / 48)), currentPage = Math.min(page, pageCount - 1);
  const restore = async (items: PinterestRemoved[]) => {
    if (restoring || !items.length) return;
    stop.current = false; setRestoring(true); setError(null); setProgress({ done: 0, total: items.length });
    for (const item of items) {
      if (stop.current || !alive.current) break;
      const id = pinterestRequestId(); restoreRequest.current = id;
      try {
        await api.libraryAction("restore", item.token, id);
        onRestored();
        if (!alive.current) break;
        setRecords(previous => previous.filter(value => value.token !== item.token));
        setSelected(previous => { const next = new Set(previous); next.delete(item.token); return next; });
        setDetail(previous => previous?.token === item.token ? null : previous);
        setDetailTabs(previous => previous.filter(value => value.token !== item.token));
        setProgress(previous => previous ? { ...previous, done: previous.done + 1 } : null);
      } catch (cause) { if (alive.current) setError(pinterestError(cause)); break; }
      finally { restoreRequest.current = null; }
    }
    if (alive.current) setRestoring(false);
  };
  const cancelRestore = () => { stop.current = true; if (restoreRequest.current) void api.cancel(restoreRequest.current).catch(() => {}); };
  const open = (item: PinterestRemoved) => { setSelected(new Set()); setDetailTabs([item]); setDetail(item); };
  const detailSource = detailTabs.length > 1 ? detailTabs : visible;
  const detailIndex = detailSource.findIndex(item => item.token === detail?.token);
  return <div className="pinterest-detail-backdrop" onMouseDown={event => { if (event.target === event.currentTarget) onClose(); }}>
    <section ref={dialog} className={`pinterest-removed${privacyMode ? " is-private" : ""}`} role="dialog" aria-modal="true" aria-label="제외된 핀" tabIndex={-1} data-gallery-shortcuts-suspended onKeyDown={event => {
      event.stopPropagation(); if (detail || event.defaultPrevented || event.nativeEvent.isComposing) return;
      if (event.key === "Escape") { event.preventDefault(); onClose(); return; }
      if (event.key === "Tab") {
        const controls = [...event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled),input,[tabindex="0"]')];
        const first = controls[0], last = controls.at(-1);
        if (event.shiftKey && (document.activeElement === first || document.activeElement === dialog.current)) { event.preventDefault(); last?.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a" && !isCardControl(event.target)) { event.preventDefault(); setSelected(new Set(ids)); }
      if (event.key === "Enter" && selected.size > 1 && !isCardControl(event.target)) {
        event.preventDefault(); const items = visible.filter(item => selected.has(item.token));
        if (event.ctrlKey || event.metaKey) void restore(items);
        else if (items[0]) { open(items[0]); setDetailTabs(items); }
      }
    }}>
      <header><h2>제외된 핀</h2><button type="button" className="icon-button" aria-label="제외된 핀 닫기" title="닫기" onClick={onClose}><Icon kind="close" /></button></header>
      <div className="pinterest-removed-tools"><input className="pinterest-local-search" aria-label="제외된 핀 검색" placeholder="제목·작가 검색" value={filter} onChange={event => { setFilter(event.target.value); setPage(0); }} /><button type="button" className="secondary-button" disabled={restoring} onClick={() => loading && listRequest.current ? void api.cancel(listRequest.current).catch(() => {}) : setRevision(value => value + 1)}>{loading ? <><span className="spinner" /> 취소</> : "새로고침"}</button></div>
      <p className="pinterest-hint">원본을 보관 중입니다. 복원하면 다운로드 목록으로 돌아갑니다.</p>
      {error ? <div className="pinterest-message is-error" role="alert">{error}</div> : null}
      <div className="pinterest-removed-scroll">
        {loading ? <div className="pinterest-empty" role="status"><span className="spinner" />제외 목록 불러오는 중</div> : visible.length ? <PinterestMasonry previewWidth={previewWidth} contextKey={`removed:${currentPage}:${filter}`}>{visible.slice(currentPage * 48, (currentPage + 1) * 48).map(item => <PinCard key={item.token} api={api} item={{ pin: item.record.pin, page: null }}
          selected={selected.has(item.token)} multiple={selected.size > 1} favorite={favorites.has(pinterestFavoriteKey(item.record.pin.id, null))} pending={restoring} downloaded removedToken={item.token} missingFiles={item.record.missingFiles}
          onSelect={event => { if (isCardControl(event.target)) return; const next = selectPinterestCard(ids, selected, item.token, anchor.current, event); setSelected(next); if (!next.size) event.currentTarget.blur(); if (!event.shiftKey) anchor.current = item.token; }}
          onOpen={() => open(item)} onFavorite={() => onFavorite(item.record.pin, null)} onDownload={() => {}} onRestore={() => void restore([item])} />)}</PinterestMasonry> : <div className="pinterest-empty"><h2>{filter ? "일치하는 핀이 없습니다" : "제외된 핀이 없습니다"}</h2></div>}
      </div>
      <footer>{progress ? <span role="status">{restoring ? "복원 중" : "복원 종료"} · {progress.done}/{progress.total}</span> : <span>{records.length}개</span>}
        {pageCount > 1 ? <nav className="pinterest-pagination" aria-label="제외 목록 페이지"><button type="button" aria-label="이전 제외 목록 페이지" disabled={!currentPage} onClick={() => setPage(currentPage - 1)}><Icon kind="left" /></button><span>{currentPage + 1}/{pageCount}</span><button type="button" aria-label="다음 제외 목록 페이지" disabled={currentPage === pageCount - 1} onClick={() => setPage(currentPage + 1)}><Icon kind="right" /></button></nav> : null}
        <button type="button" className="primary-button" disabled={!restoring && !selected.size} onClick={() => restoring ? cancelRestore() : void restore(visible.filter(item => selected.has(item.token)))}>{restoring ? <><span className="spinner" /> 취소</> : `선택 복원${selected.size ? ` (${selected.size})` : ""}`}</button>
      </footer>
    </section>
    {detail ? <PinDetail key={detail.token} api={api} pin={detail.record.pin} removedToken={detail.token} initialPage={0} initialPreview privacyMode={privacyMode} favorites={favorites} pending={restoring} downloaded
      canPrevious={detailIndex > 0} canNext={detailIndex >= 0 && detailIndex < detailSource.length - 1}
      onMove={direction => { const next = detailSource[detailIndex + direction]; if (next) setDetail(next); }} onClose={() => setDetail(null)}
      onFavorite={page => onFavorite(detail.record.pin, page)} onDownload={() => {}} onExternal={() => void api.openExternal(detail.record.pin.id).catch(cause => setError(pinterestError(cause)))} onRestore={() => void restore([detail])}
      error={error} metadataFailed={false} onRetry={() => {}} tabs={detailTabs.map(item => ({ pin: item.record.pin, page: null, key: item.token }))} activeTabKey={detail.token}
      onTab={item => { const next = detailTabs.find(value => value.token === item.key); if (next) setDetail(next); }}
      onCloseTab={item => { const remaining = detailTabs.filter(value => value.token !== item.key); setDetailTabs(remaining); if (item.key === detail.token) setDetail(remaining[0] ?? null); }} /> : null}
  </div>;
}

function PinCard({ api, item, selected, multiple, favorite, pending, downloaded, onSelect, onOpen, onFavorite, onDownload, onFolder, onArchive, missingFiles, removedToken, onRestore }: {
  api: PinterestApi; item: PinterestFavorite; selected: boolean; multiple: boolean; favorite: boolean; pending: boolean; downloaded: boolean;
  onSelect: (event: MouseEvent<HTMLElement>) => void; onOpen: () => void; onFavorite: () => void; onDownload: () => void;
  onFolder?: () => void; onArchive?: () => void;
  missingFiles?: boolean;
  removedToken?: string; onRestore?: () => void;
}) {
  const menu = useCardContextMenu(), pin = item.pin;
  const media = pin.media[item.page ?? 0];
  const mediaIdentity = `${removedToken ?? pin.id}:${item.page ?? 0}:${media?.url ?? ""}`;
  const [decoded, setDecoded] = useState<{ identity: string; dimensions: PinterestImageDimensions } | null>(null);
  const metadataDimensions = pinterestImageDimensions(media?.width, media?.height);
  const dimensions = metadataDimensions ?? (decoded?.identity === mediaIdentity ? decoded.dimensions : null);
  return <><article className={`pinterest-card${selected ? " is-selected" : ""}`} tabIndex={0} aria-label={pin.title} onClick={onSelect}
    data-masonry-key={removedToken ?? pinterestFavoriteKey(pin.id, item.page)} data-masonry-width={dimensions?.width} data-masonry-height={dimensions?.height}
    onDoubleClick={event => { if (!isCardControl(event.target) && !event.ctrlKey && !event.metaKey && !event.shiftKey) onOpen(); }} onContextMenu={menu.open}
    onKeyDown={event => { if (!menu.onKeyDown(event) && event.key === "Enter" && event.target === event.currentTarget && !multiple) { event.preventDefault(); if (event.ctrlKey || event.metaKey) { if (onRestore) { if (!pending) onRestore(); } else onDownload(); } else onOpen(); } }}>
    <PinImage api={api} pin={pin} page={item.page ?? 0} removedToken={removedToken} onDimensions={value => {
      if (metadataDimensions) return;
      setDecoded(previous => previous?.identity === mediaIdentity && previous.dimensions.width === value.width && previous.dimensions.height === value.height ? previous : { identity: mediaIdentity, dimensions: value });
    }} />
    {multiple ? <span className={`pinterest-check${selected ? " is-checked" : ""}`} role="checkbox" aria-checked={selected} aria-label={`${pin.title} 선택`}>{selected ? "✓" : ""}</span> : null}
    <button type="button" className={`pinterest-bookmark${favorite ? " is-saved" : ""}`} title={favorite ? "즐겨찾기 해제" : "즐겨찾기"} aria-label={`${pin.title} ${favorite ? "즐겨찾기 해제" : "즐겨찾기"}`} aria-pressed={favorite} onClick={onFavorite}><Icon kind="bookmark" /></button>
    <div className="pinterest-card-copy"><strong title={pin.title}>{splitGalleryTitle(pin.title).primary}</strong><span>{pin.creatorName || "Pinterest"}{item.page !== null ? ` · ${item.page + 1}쪽` : pin.media.length > 1 ? ` · ${pin.media.length}쪽` : ""}</span><small>{missingFiles ? "원본 파일 없음" : removedToken ? "제외됨" : downloaded ? "저장됨" : pending ? "다운로드 중" : pin.unsupportedVideo ? "영상 핀 · 표지" : ""}</small></div>
    {onRestore ? <button type="button" className="pinterest-restore" disabled={pending} onClick={onRestore} aria-label={`${pin.title} 복원`}>{pending ? <span className="spinner" /> : "복원"}</button> : null}
  </article><CardContextMenu anchor={menu.anchor} close={menu.close} label={pin.title} items={[
    { id: "open", label: "상세 열기", shortcut: "Enter", action: onOpen },
    { id: "favorite", label: favorite ? "즐겨찾기 해제" : "즐겨찾기 추가", action: onFavorite },
    ...(onRestore ? [{ id: "restore", label: "보관함으로 복원", disabled: pending, action: onRestore }] : [{ id: "download", label: pending ? "다운로드 취소" : downloaded ? "원본 저장 완료" : "원본 다운로드", disabled: downloaded || pin.unsupportedVideo || !pin.media.length, action: onDownload }]),
    ...(onFolder ? [{ id: "folder", label: "저장 폴더 열기", action: onFolder }] : []),
    ...(onArchive ? [{ id: "archive", label: "보관함에서 제외", action: onArchive }] : []),
  ]} /></>;
}

function PinDetail({ api, pin, initialPage, initialPreview, privacyMode, favorites, pending, downloaded, canPrevious, canNext, onClose, onMove, onFavorite, onDownload, onExternal, onCreator, error, metadataFailed, onRetry, tabs, activeTabKey, onTab, onCloseTab, removedToken, onRestore }: {
  api: PinterestApi; pin: PinterestPin; initialPage: number; initialPreview: boolean; privacyMode: boolean; favorites: Set<string>; pending: boolean; downloaded: boolean;
  canPrevious: boolean; canNext: boolean; onClose: () => void; onMove: (direction: number) => void; onFavorite: (page: number | null) => void; onDownload: () => void; onExternal: () => void;
  onCreator?: () => void; error: string | null;
  metadataFailed: boolean; onRetry: () => void; tabs: DetailTab[]; activeTabKey: string; onTab: (item: DetailTab) => void; onCloseTab: (item: DetailTab) => void;
  removedToken?: string; onRestore?: () => void;
}) {
  const [page, setPage] = useState(initialPage), [preview, setPreview] = useState(initialPreview);
  const [confirmClose, setConfirmClose] = useState(false);
  const [controlsVisible, setControlsVisible] = useState(true), [controlsPinned, setControlsPinned] = useState(true);
  const hideControls = useRef<ReturnType<typeof setTimeout> | null>(null);
  const dialog = useRef<HTMLElement>(null);
  useEffect(() => { const previous = document.activeElement; dialog.current?.focus({ preventScroll: true }); return () => { if (previous instanceof HTMLElement && previous.isConnected) previous.focus({ preventScroll: true }); }; }, []);
  useEffect(() => {
    if (hideControls.current) clearTimeout(hideControls.current);
    setControlsVisible(true);
    if (preview && !controlsPinned) hideControls.current = setTimeout(() => setControlsVisible(false), 1800);
    return () => { if (hideControls.current) clearTimeout(hideControls.current); };
  }, [preview, controlsPinned]);
  const revealControls = () => {
    setControlsVisible(true);
    if (hideControls.current) clearTimeout(hideControls.current);
    if (preview && !controlsPinned) hideControls.current = setTimeout(() => setControlsVisible(false), 1800);
  };
  const navigate = (direction: number) => { if (preview) setPage(value => Math.min(pin.media.length - 1, Math.max(0, value + direction))); else onMove(direction); };
  const requestClose = () => tabs.length > 1 ? setConfirmClose(true) : onClose();
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (confirmClose || event.defaultPrevented || event.nativeEvent.isComposing || event.ctrlKey || event.altKey || event.metaKey || (event.target instanceof Element && event.target.closest("input,textarea,select"))) return;
    if (event.key === "Tab") {
      const buttons = [...(dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled),a[href]') ?? [])];
      const first = buttons[0], last = buttons[buttons.length - 1];
      if (event.shiftKey && (document.activeElement === first || document.activeElement === dialog.current)) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    }
    if (event.key === "Escape") { event.preventDefault(); requestClose(); }
    if (["ArrowLeft", "a", "A"].includes(event.key)) { event.preventDefault(); navigate(-1); }
    if (["ArrowRight", "d", "D"].includes(event.key)) { event.preventDefault(); navigate(1); }
  };
  const favorite = favorites.has(pinterestFavoriteKey(pin.id, preview ? page : null));
  return <div className="pinterest-detail-backdrop" onMouseDown={event => { if (event.target === event.currentTarget) requestClose(); }}>
    <section ref={dialog} className={`pinterest-detail${preview ? " is-preview" : ""}${privacyMode ? " is-private" : ""}${controlsVisible || controlsPinned ? " show-controls" : ""}`} role="dialog" aria-modal="true" aria-label={preview ? "Pinterest 페이지 미리보기" : "Pinterest 핀 상세"} data-gallery-shortcuts-suspended tabIndex={-1} onKeyDown={onKeyDown} onMouseMove={revealControls}>
      <header><h2>{pin.title}</h2><button type="button" className="secondary-button" aria-pressed={!preview} onClick={() => setPreview(value => !value)}>{preview ? "정보" : "미리보기"}</button><button type="button" className="icon-button" aria-label={preview ? "미리보기 닫기" : "상세 닫기"} title="닫기" onClick={requestClose}><Icon kind="close" /></button></header>
      {tabs.length > 1 ? <nav className="pinterest-search-tabs pinterest-detail-tabs" aria-label="열린 핀 상세">{tabs.map(item => <span key={item.key} className={item.key === activeTabKey ? "is-active" : ""}><button type="button" aria-pressed={item.key === activeTabKey} onClick={() => onTab(item)}>{splitGalleryTitle(item.pin.title).primary}{item.page !== null ? ` · ${item.page + 1}쪽` : ""}</button><button type="button" aria-label={`${item.pin.title}${item.page !== null ? ` ${item.page + 1}쪽` : ""} 상세 탭 닫기`} onClick={() => onCloseTab(item)}><Icon kind="close" /></button></span>)}</nav> : null}
      {error ? <div className="pinterest-message is-error" role="alert">{error}{metadataFailed ? <button type="button" className="pinterest-undo" onClick={onRetry}>다시 시도</button> : null}</div> : null}
      <div className="pinterest-detail-body"><div className="pinterest-detail-media">
        <PinImage api={api} pin={pin} page={page} thumbnail={false} removedToken={removedToken} />
        {!preview && pin.media.length ? <button type="button" className="pinterest-preview-open" onClick={() => setPreview(true)}>페이지 미리보기</button> : null}
        <button type="button" className="pinterest-prev" title="이전 (A)" aria-label="이전" disabled={preview ? page === 0 : !canPrevious} onClick={() => navigate(-1)}><Icon kind="left" /></button>
        <button type="button" className="pinterest-next" title="다음 (D)" aria-label="다음" disabled={preview ? page >= pin.media.length - 1 : !canNext} onClick={() => navigate(1)}><Icon kind="right" /></button>
        {pin.media.length > 0 ? <span className="pinterest-page-count">{page + 1} / {pin.media.length}</span> : null}
      </div>{!preview ? <aside>{onCreator ? <button type="button" className="pinterest-creator" onClick={onCreator}>{pin.creatorName || "프로필"}</button> : <span>{pin.creatorName || "Pinterest"}</span>}{pin.summaryOnly && !metadataFailed ? <span className="spinner" role="status" aria-label="상세 불러오는 중" /> : null}<p>{pin.description}</p>{pin.unsupportedVideo ? <p className="pinterest-video-notice">영상 원본 저장은 지원하지 않습니다.</p> : null}<a href={pin.sourceUrl} onClick={event => { event.preventDefault(); onExternal(); }}>Pinterest에서 열기 <Icon kind="open" /></a>
        {pin.media.length > 1 ? <div className="pinterest-pages">{pin.media.map((_, index) => <button type="button" key={index} aria-label={`${index + 1}쪽 보기`} aria-pressed={page === index} onClick={() => setPage(index)}>{index + 1}</button>)}</div> : null}</aside> : null}</div>
      <footer><button type="button" className={`icon-button${favorite ? " is-saved" : ""}`} aria-pressed={favorite} aria-label={preview ? `${page + 1}쪽 즐겨찾기` : "앨범 즐겨찾기"} title={preview ? "페이지 즐겨찾기" : "앨범 즐겨찾기"} onClick={() => onFavorite(preview ? page : null)}><Icon kind="bookmark" />{preview ? <span className="pinterest-bookmark-page">{page + 1}</span> : null}</button>
        {preview ? <button type="button" className={`secondary-button${favorites.has(pinterestFavoriteKey(pin.id, null)) ? " is-saved" : ""}`} aria-pressed={favorites.has(pinterestFavoriteKey(pin.id, null))} onClick={() => onFavorite(null)}>앨범 즐겨찾기</button> : null}
        {onRestore ? <button type="button" className="primary-button" disabled={pending} onClick={onRestore}>{pending ? <span className="spinner" /> : null}보관함으로 복원</button> : <button type="button" className="primary-button" disabled={downloaded || pin.unsupportedVideo || !pin.media.length} onClick={onDownload}>{pending ? <span className="spinner" /> : <Icon kind="download" />}{downloaded ? "저장 완료" : pending ? "취소" : "원본 저장"}</button>}</footer>
      {preview ? <button type="button" className={`pinterest-controls-pin${controlsPinned ? " is-saved" : ""}`} aria-label="미리보기 조작부 고정" aria-pressed={controlsPinned} title="조작부 고정" onClick={() => setControlsPinned(value => !value)}><svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden="true"><path d="M8 3h8m-7 0v7l-3 4h12l-3-4V3M12 14v7" /></svg></button> : null}
    </section>
    {confirmClose ? <CloseDetailsConfirmation count={tabs.length} onCancel={() => setConfirmClose(false)} onConfirm={onClose} /> : null}
  </div>;
}

function CloseDetailsConfirmation({ count, onCancel, onConfirm }: { count: number; onCancel: () => void; onConfirm: () => void }) {
  const previousFocus = useRef(document.activeElement);
  useEffect(() => () => { const previous = previousFocus.current; if (previous instanceof HTMLElement && previous.isConnected) previous.focus({ preventScroll: true }); }, []);
  return <div className="pinterest-detail-backdrop" onMouseDown={event => event.stopPropagation()}><section className="pinterest-confirm" role="alertdialog" aria-modal="true" aria-label="열린 핀 닫기" data-gallery-shortcuts-suspended onKeyDown={event => {
    event.stopPropagation();
    if (event.key === "Escape") { event.preventDefault(); onCancel(); }
    if (event.key === "Tab") {
      const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button")];
      if (document.activeElement === (event.shiftKey ? buttons[0] : buttons.at(-1))) { event.preventDefault(); (event.shiftKey ? buttons.at(-1) : buttons[0])?.focus(); }
    }
  }}><h2>열린 핀 {count}개를 닫을까요?</h2><footer><button type="button" autoFocus onClick={onCancel}>취소</button><button type="button" onClick={onConfirm}>모두 닫기</button></footer></section></div>;
}

