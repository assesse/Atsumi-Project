import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent } from "react";
import { createPortal } from "react-dom";
import { CardContextMenu, isCardControl, useCardContextMenu } from "../../components/CardContextMenu";
import { splitGalleryTitle } from "../../components/galleryCardLayout";
import { normalizeImagePreviewWidth } from "../platforms/imagePreviewPreferences";
import { createPixivApi, createPixivMediaLoader } from "./api";
import { createOriginalWindow } from "./originalWindow";
import { PixivOriginalPages } from "./PixivOriginalPages";
import { PixivPageReader } from "./PixivPageReader";
import { PixivHomeHeader, PixivTagGroups } from "./PixivHomeHeader";
import { PixivDiscoverySections, appendDiscovery, discoveryWorks } from "./PixivDiscoverySections";
import { usePixivSafeOnly } from "./displayPreferences";
import { PixivMetadataText } from "./PixivMetadataText";
import { pixivReadings } from "./readingService";
import { artworkId, bookmarkKey, isPixivHome, loadBookmarks, loadHistory, localPageUrl, PIXIV_BOOKMARK_KEY, PIXIV_HISTORY_KEY, updateSelection, type PixivAccountSnapshot, type PixivApi, type PixivDetail, type PixivDiscovery, type PixivDownload, type PixivHomeSuggestions, type PixivMode, type PixivProgress, type PixivView, type PixivWork } from "./model";
import "./PixivWorkspace.css";

export type PixivWorkspaceProps = {
  active: boolean; runtime: "tauri" | "browser-mock"; privacyMode: boolean;
  view: PixivView; onNotice?: (message: string) => void; api?: PixivApi;
  onViewChange?: (view: PixivView) => void;
  previewWidth?: number;
};
type Entry = { work: PixivWork; page?: number; download?: PixivDownload };
type SearchTab = { id: string; label: string; query: string; mode: PixivMode; page: number; items: PixivWork[]; hasMore: boolean; discovery?: PixivDiscovery; homeSuggestions?: PixivHomeSuggestions; homeCursor?: string; appendBusy: boolean; appendError: string; busy: boolean; error: string; token: number; scroll: number; returnDetail?: { value: PixivDetail; page: number; view: PixivView } };
// This is a native, session-bound continuation handle, never a URL or an offset.
const validHomeCursor = (value: unknown): string | undefined => typeof value === "string" && value.length > 0 && value.length <= 96 ? value : undefined;
const appendUniqueWorks = (existing: PixivWork[], incoming: PixivWork[]) => {
  const seen = new Set(existing.map(work => work.id));
  return [...existing, ...incoming.filter(work => { if (seen.has(work.id)) return false; seen.add(work.id); return true; })];
};
const errorText = (error: unknown) => error instanceof Error ? error.message : typeof error === "string" ? error : "Pixiv에 연결하지 못했습니다. 잠시 후 다시 시도해 주세요.";
const formatBytes = (bytes: number) => bytes >= 1e9 ? `${(bytes / 1e9).toFixed(2)} GB` : `${(bytes / 1e6).toFixed(1)} MB`;
function Icon({ kind }: { kind: "bookmark" | "download" | "close" | "refresh" | "left" | "right" | "check" }) {
  const paths = { bookmark: "M6 3h12v18l-6-4-6 4V3Z", download: "M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5", close: "m6 6 12 12M6 18 18 6", refresh: "M20 8a8 8 0 1 0 0 8M20 3v5h-5", left: "m15 5-7 7 7 7", right: "m9 5 7 7-7 7", check: "m5 12 4 4L19 6" };
  return <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round"><path d={paths[kind]} /></svg>;
}
function PixivImage({ url, title, privacyMode, loader, detail = false }: { url?: string; title: string; privacyMode: boolean; loader: ReturnType<typeof createPixivMediaLoader>; detail?: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(detail), [source, setSource] = useState<string | null>(null), [failed, setFailed] = useState(false);
  useEffect(() => {
    if (detail || !host.current || typeof IntersectionObserver === "undefined") { setVisible(true); return; }
    const observer = new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) { setVisible(true); observer.disconnect(); } }, { rootMargin: "160px" });
    observer.observe(host.current); return () => observer.disconnect();
  }, [detail]);
  useEffect(() => {
    let cancelled = false; setSource(null); setFailed(false);
    if (!privacyMode && visible && url) void loader(url, detail).then(value => { if (!cancelled) setSource(value); }, () => { if (!cancelled) setFailed(true); });
    return () => { cancelled = true; };
  }, [privacyMode, visible, url, loader, detail]);
  return <div ref={host} className={`pixiv-image${privacyMode ? " is-private" : ""}`}>
    {!privacyMode && source && !failed ? <img src={source} alt={title} draggable={false} onError={() => setFailed(true)} /> : <span className={privacyMode || failed || !url ? "pixiv-image-caption" : "pixiv-image-skeleton"}>{privacyMode ? "프라이버시 모드" : failed ? "이미지를 불러오지 못했습니다" : !url ? "미리보기 없음" : ""}</span>}
  </div>;
}

export function PixivWorkspace({ active, runtime, privacyMode, view: outerView, onNotice, onViewChange, api: injected, previewWidth = 180 }: PixivWorkspaceProps) {
  const safeOnly = usePixivSafeOnly(), appliedSafeOnly = useRef(safeOnly);
  const requestSafeOnly = useRef(safeOnly); requestSafeOnly.current = safeOnly;
  const [discoveryOverride, setDiscoveryOverride] = useState(false);
  const view = discoveryOverride ? "explore" : outerView;
  const visibleView = useRef({ active, view, privacyMode }); visibleView.current = { active, view, privacyMode };
  useEffect(() => { if (!active || privacyMode) pixivReadings.clear(); return () => pixivReadings.clear(); }, [active, privacyMode]);
  useEffect(() => setDiscoveryOverride(false), [outerView]);
  const workspace = useRef<HTMLElement>(null);
  const tabRows = useRef(new Map<string, SearchTab>()), currentTab = useRef("");
  const [tabButtons, setTabButtons] = useState<{ id: string; label: string; busy: boolean }[]>([]);
  const api = useMemo(() => injected ?? createPixivApi(runtime), [injected, runtime]);
  const [account, setAccount] = useState<PixivAccountSnapshot | null>(null), [accountPending, setAccountPending] = useState(false);
  const accountRevision = useRef<number | null>(null), accountRequest = useRef(0), accountActionInFlight = useRef(false);
  const accountValue = useRef<PixivAccountSnapshot | null>(null);
  const [mediaRevision, setMediaRevision] = useState(0);
  const loader = useMemo(() => createPixivMediaLoader(api), [api, mediaRevision]);
  const originalPool = useMemo(() => createOriginalWindow(api), [api, mediaRevision]);
  // Locally saved originals have no account authorization lifetime. Remote session
  // invalidation must not revoke a local reader's URLs or reset its reading context.
  const localLoader = useMemo(() => createPixivMediaLoader(api), [api]);
  const localOriginalPool = useMemo(() => createOriginalWindow(api), [api]);
  useEffect(() => () => loader.dispose(), [loader]);
  useEffect(() => () => originalPool.clear(), [originalPool]);
  useEffect(() => () => localLoader.dispose(), [localLoader]);
  useEffect(() => () => localOriginalPool.clear(), [localOriginalPool]);
  const [query, setQuery] = useState(""), [mode, setMode] = useState<PixivMode>("home");
  const [localQueries, setLocalQueries] = useState({ favorites: "", downloads: "" });
  const [history, setHistory] = useState(loadHistory), [historyOpen, setHistoryOpen] = useState(false);
  const [committed, setCommitted] = useState({ query: "", mode: "home" as PixivMode });
  const [homeSuggestions, setHomeSuggestions] = useState<PixivHomeSuggestions>();
  const [searchSuggestions, setSearchSuggestions] = useState<PixivHomeSuggestions>();
  const [homeDiscovery, setHomeDiscovery] = useState<PixivDiscovery>();
  const homeSession = useRef<string | null>(null);
  const [works, setWorks] = useState<PixivWork[]>([]), [page, setPage] = useState(1), [hasMore, setHasMore] = useState(false);
  const [busy, setBusy] = useState(false), [error, setError] = useState("");
  const [appendBusy, setAppendBusy] = useState(false), [appendError, setAppendError] = useState("");
  const [bookmarks, setBookmarks] = useState(loadBookmarks), [category, setCategory] = useState<"albums" | "pages">("albums");
  const [downloads, setDownloads] = useState<PixivDownload[]>([]), [progress, setProgress] = useState<PixivProgress[]>([]);
  const [removeEntries, setRemoveEntries] = useState<Entry[]>([]), [libraryBusy, setLibraryBusy] = useState(false);
  const [removed, setRemoved] = useState<{ id: string; token: string }[]>([]);
  const [selected, setSelected] = useState<string[]>([]), anchor = useRef<string | null>(null);
  const [detail, setDetail] = useState<PixivDetail | null>(null), [detailLoading, setDetailLoading] = useState(false), [detailError, setDetailError] = useState("");
  const detailPool = detail?.pages[0]?.previewUrl.startsWith("atsumi-pixiv:") ? localOriginalPool : originalPool;
  const [detailInput, setDetailInput] = useState<string | null>(null), [detailPage, setDetailPage] = useState(0);
  const [readerOpen, setReaderOpen] = useState(false);
  const [readerInfoOpen, setReaderInfoOpen] = useState(false);
  const [related, setRelated] = useState<PixivDiscovery>();
  const [relatedBusy, setRelatedBusy] = useState(false), [relatedError, setRelatedError] = useState("");
  const [relatedRetry, setRelatedRetry] = useState(0);
  const relatedEpoch = useRef(0), readerScrolls = useRef(new Map<string, number>());
  const readerOpener = useRef<HTMLElement | null>(null);
  const [detailTabs, setDetailTabs] = useState<{ id: string; title: string; page: number }[]>([]), [confirmCloseDetails, setConfirmCloseDetails] = useState(false);
  const [detailClosing, setDetailClosing] = useState(false), closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const generation = useRef(0), detailGeneration = useRef(0), initialized = useRef(false), mounted = useRef(true);
  const storageWarningShown = useRef(false);
  const downloaded = useMemo(() => new Set(downloads.map(row => row.work.id)), [downloads]);
  const incomplete = useMemo(() => new Set(downloads.filter(row => row.issue).map(row => row.work.id)), [downloads]);
  const replaceDownloads = useCallback((rows: PixivDownload[]) => {
    setDownloads(rows.filter(row => !row.removedToken));
    setRemoved(rows.flatMap(row => row.removedToken ? [{ id: row.work.id, token: row.removedToken }] : []));
  }, []);
  const [contextEntry, setContextEntry] = useState<Entry | null>(null), context = useCardContextMenu();
  const closeContext = useRef(context.close); closeContext.current = context.close;
  // The shared close callback follows its anchor. Opening the menu must not
  // look like a filter change and immediately dismiss that same menu.
  useEffect(() => { setSelected([]); anchor.current = null; closeContext.current(); }, [category, localQueries.favorites, localQueries.downloads]);
  const closeButton = useRef<HTMLButtonElement>(null), priorFocus = useRef<HTMLElement | null>(null);
  const notice = useCallback((message: string) => { setError(message); onNotice?.(message); }, [onNotice]);
  const updateTabButtons = () => setTabButtons([...tabRows.current.values()].map(({ id, label, busy, appendBusy }) => ({ id, label, busy: busy || appendBusy })));
  const displayTab = useCallback((tab: SearchTab, preserveDraft = false) => {
    currentTab.current = tab.id; setWorks(tab.items); setPage(tab.page); setHasMore(tab.hasMore); setBusy(tab.busy); setError(tab.error);
    setCommitted({ query: tab.query, mode: tab.mode }); setHomeSuggestions(tab.homeSuggestions);
    setHomeDiscovery(tab.discovery);
    setAppendBusy(tab.appendBusy); setAppendError(tab.appendError);
    if (!preserveDraft) { setQuery(tab.mode === "artist" ? "" : tab.query); setMode(tab.mode === "artist" ? "latest" : tab.mode); }
    setSelected([]); anchor.current = null;
    const beforeRestore = workspace.current?.scrollTop;
    requestAnimationFrame(() => { if (workspace.current && currentTab.current === tab.id && workspace.current.scrollTop === beforeRestore) workspace.current.scrollTop = tab.scroll; });
  }, []);

  useEffect(() => { mounted.current = true; return () => { mounted.current = false; generation.current++; detailGeneration.current++; clearTimeout(closeTimer.current); }; }, []);
  useEffect(() => { try { localStorage.setItem(PIXIV_BOOKMARK_KEY, JSON.stringify(bookmarks)); storageWarningShown.current = false; } catch { if (!storageWarningShown.current) { storageWarningShown.current = true; onNotice?.("즐겨찾기를 이 PC에 저장하지 못했습니다. 저장 공간 또는 브라우저 설정을 확인해 주세요."); } } }, [bookmarks, onNotice]);
  useEffect(() => { try { localStorage.setItem(PIXIV_HISTORY_KEY, JSON.stringify(history)); } catch { /* Search remains usable when optional history storage is unavailable. */ } }, [history]);
  const refreshDownloads = useCallback(async () => {
    const token = ++generation.current; setBusy(true); setError("");
    try { const result = await api.downloads(); if (token === generation.current) replaceDownloads(result); }
    catch (reason) { if (token === generation.current) setError(errorText(reason)); }
    finally { if (token === generation.current) setBusy(false); }
  }, [api, replaceDownloads]);
  const search = useCallback(async (searchQuery: string, searchMode: PixivMode, nextPage = 1, label?: string, background = false, origin?: SearchTab["returnDetail"], options?: { refresh?: boolean; preserveDraft?: boolean }) => {
    // Searching now resolves an account lease. Masked/hidden views must not
    // initialize that native profile through the search path either.
    if (!visibleView.current.active || visibleView.current.privacyMode) return;
    setHistoryOpen(false);
    if (searchMode === "latest" && searchQuery.trim() && searchQuery.trim().length <= 120) setHistory(rows => [searchQuery.trim(), ...rows.filter(row => row !== searchQuery.trim())].slice(0, 10));
    const id = `${searchMode}:${searchQuery.trim()}`;
    const previous = tabRows.current.get(currentTab.current);
    if (previous && workspace.current) previous.scroll = workspace.current.scrollTop;
    const existing = tabRows.current.get(id);
    if (existing && (isPixivHome(searchMode) || existing.page === nextPage) && !existing.error && !options?.refresh) { if (!background) displayTab(existing, options?.preserveDraft); return; }
    if (!existing && tabRows.current.size >= 24) { onNotice?.("열린 탐색을 닫은 뒤 다시 검색해 주세요."); return; }
    const tab: SearchTab = existing ?? { id, label: label ?? (searchMode === "home" ? "추천" : searchMode === "illustration" ? "일러스트" : searchMode === "manga" ? "만화" : searchMode === "daily" ? "일간 랭킹" : searchMode === "weekly" ? "주간 랭킹" : searchQuery), query: searchQuery.trim(), mode: searchMode, page: nextPage, items: [], hasMore: false, appendBusy: false, appendError: "", busy: false, error: "", token: 0, scroll: 0, returnDetail: origin };
    const token = ++tab.token; tab.busy = true; tab.error = ""; tab.appendBusy = false; tab.appendError = "";
    if (isPixivHome(searchMode)) { tab.homeCursor = undefined; tab.hasMore = false; }
    tabRows.current.set(id, tab); updateTabButtons();
    if (!background) displayTab(tab, options?.preserveDraft);
    try {
      const discovery = isPixivHome(searchMode) && api.discover ? await api.discover({ surface: searchMode, ...(requestSafeOnly.current ? { safeOnly: true } : {}) }) : undefined;
      const result = discovery ? { items: discoveryWorks(discovery), page: discovery.page, hasMore: Boolean(discovery.cursor), homeCursor: discovery.cursor, homeSuggestions: discovery.suggestions }
        : await api.search({ query: searchQuery.trim(), mode: isPixivHome(searchMode) ? "home" : searchMode, page: nextPage, ...(requestSafeOnly.current ? { safeOnly: true } : {}) });
      if (token !== tab.token || !mounted.current || tabRows.current.get(id) !== tab) return;
      tab.discovery = discovery;
      tab.items = isPixivHome(searchMode) ? appendUniqueWorks([], result.items).filter(work => searchMode === "manga" ? work.type === "manga" : searchMode === "illustration" ? work.type !== "manga" : true) : result.items; tab.page = result.page;
      tab.homeCursor = isPixivHome(searchMode) ? validHomeCursor(result.homeCursor) : undefined;
      tab.hasMore = result.hasMore && (!isPixivHome(searchMode) || Boolean(tab.homeCursor)); tab.homeSuggestions = result.homeSuggestions; tab.scroll = 0;
      if (result.homeSuggestions) setSearchSuggestions(result.homeSuggestions);
    } catch (reason) { if (token === tab.token) tab.error = errorText(reason); }
    finally { if (token === tab.token && mounted.current) { tab.busy = false; updateTabButtons(); if (currentTab.current === id && visibleView.current.active && !visibleView.current.privacyMode && visibleView.current.view === "explore") displayTab(tab, true); } }
  }, [api, displayTab, onNotice]);
  const appendHome = useCallback(async () => {
    const tab = tabRows.current.get(currentTab.current), snapshot = accountValue.current;
    if (!visibleView.current.active || visibleView.current.privacyMode || visibleView.current.view !== "explore" ||
      api.account && (!snapshot || snapshot.authenticated === null || snapshot.busy || accountActionInFlight.current) ||
      !tab || !isPixivHome(tab.mode) || tab.busy || tab.appendBusy || !tab.hasMore || !tab.homeCursor) return;
    const revision = accountRevision.current, token = ++tab.token, cursor = tab.homeCursor;
    tab.appendBusy = true; tab.appendError = ""; setAppendBusy(true); setAppendError(""); updateTabButtons();
    const current = () => mounted.current && tabRows.current.get(tab.id) === tab && token === tab.token && accountRevision.current === revision;
    const visible = () => currentTab.current === tab.id && visibleView.current.active && !visibleView.current.privacyMode && visibleView.current.view === "explore";
    try {
      const discovery = tab.discovery && api.discover ? await api.discover({ surface: tab.mode, cursor, page: tab.page + 1, ...(requestSafeOnly.current ? { safeOnly: true } : {}) }) : undefined;
      const result = discovery ? { items: discoveryWorks(discovery), page: discovery.page, hasMore: Boolean(discovery.cursor), homeCursor: discovery.cursor }
        : await api.search({ query: "", mode: "home", page: tab.page + 1, homeCursor: cursor, ...(requestSafeOnly.current ? { safeOnly: true } : {}) });
      const latestAccount = accountValue.current;
      if (!current() || !visibleView.current.active || visibleView.current.privacyMode || visibleView.current.view !== "explore" ||
        api.account && (!latestAccount || latestAccount.authenticated === null || latestAccount.busy || accountActionInFlight.current)) return;
      if (tab.discovery && discovery) tab.discovery = appendDiscovery(tab.discovery, discovery);
      tab.items = appendUniqueWorks(tab.items, result.items).filter(work => tab.mode === "manga" ? work.type === "manga" : tab.mode === "illustration" ? work.type !== "manga" : true); tab.page = result.page;
      tab.homeCursor = validHomeCursor(result.homeCursor); tab.hasMore = result.hasMore && Boolean(tab.homeCursor);
      // Suggestions, selected IDs, Shift anchor and the user's current scroll all
      // belong to the retained page. Appending must not call displayTab().
      if (visible()) { setWorks(tab.items); setHomeDiscovery(tab.discovery); setPage(tab.page); setHasMore(tab.hasMore); }
    } catch (reason) { if (current()) tab.appendError = errorText(reason); }
    finally {
      if (current()) {
        tab.appendBusy = false; updateTabButtons();
        if (visible()) { setAppendBusy(false); setAppendError(tab.appendError); }
      }
    }
  }, [api]);
  const cancelAppend = () => {
    const tab = tabRows.current.get(currentTab.current); if (!tab?.appendBusy) return;
    tab.token++; tab.appendBusy = false; tab.appendError = ""; updateTabButtons(); setAppendBusy(false); setAppendError("");
  };
  const invalidateRemoteResults = useCallback((message: string) => {
      relatedEpoch.current++; setRelated(undefined); setRelatedBusy(false); setRelatedError(""); readerScrolls.current.clear();
      setHomeDiscovery(undefined); setSearchSuggestions(undefined);
      // Keep tabs, drafts and local library records, but never reuse another session's remote results.
      for (const tab of tabRows.current.values()) {
        tab.token++; tab.items = []; tab.discovery = undefined; tab.busy = false; tab.appendBusy = false; tab.appendError = ""; tab.homeCursor = undefined; tab.error = message; tab.hasMore = false; tab.homeSuggestions = undefined; tab.returnDetail = undefined;
      }
      setTabButtons([...tabRows.current.values()].map(({ id, label, busy }) => ({ id, label, busy })));
      if (visibleView.current.view === "explore") { setWorks([]); setHasMore(false); setBusy(false); setAppendBusy(false); setAppendError(""); setError(message); setSelected([]); setHomeSuggestions(undefined); }
      detailGeneration.current++;
      if (!detail?.pages[0]?.previewUrl.startsWith("atsumi-pixiv:")) {
        setDetail(null); setDetailInput(null); setDetailLoading(false); setReaderOpen(false); setDetailTabs([]);
      }
      setMediaRevision(value => value + 1);
  }, [detail]);
  useEffect(() => {
    if (appliedSafeOnly.current === safeOnly) return;
    appliedSafeOnly.current = safeOnly;
    invalidateRemoteResults("표시 설정이 변경되었습니다. 다시 검색해 주세요.");
  }, [safeOnly, invalidateRemoteResults]);
  const applyAccount = useCallback((snapshot: PixivAccountSnapshot) => {
    if (!mounted.current) return;
    if (accountRevision.current !== null && snapshot.revision < accountRevision.current) return;
    setAccount(snapshot);
    if (accountRevision.current !== null && (accountRevision.current !== snapshot.revision || accountValue.current?.authenticated !== snapshot.authenticated)) {
      pixivReadings.clear();
      invalidateRemoteResults("로그인 상태가 변경되었습니다. 다시 검색해 주세요.");
    }
    accountRevision.current = snapshot.revision;
    accountValue.current = snapshot;
  }, [invalidateRemoteResults]);
  const accountHandler = useRef(applyAccount); accountHandler.current = applyAccount;
  const accountFailure = () => accountHandler.current({ authenticated: null, open: accountValue.current?.open ?? false, busy: false, revision: accountValue.current?.revision ?? 0, error: "Pixiv 로그인 상태를 확인하지 못했습니다. 잠시 후 다시 시도해 주세요." });
  useEffect(() => {
    if (!active || privacyMode || !api.account) return;
    let alive = true, inFlight = false;
    const poll = () => {
      if (inFlight || accountActionInFlight.current) return;
      inFlight = true; const token = ++accountRequest.current;
      void api.account!().then(snapshot => { if (alive && token === accountRequest.current) accountHandler.current(snapshot); }, () => { if (alive && token === accountRequest.current) accountFailure(); }).finally(() => { inFlight = false; });
    };
    poll(); const timer = window.setInterval(poll, 2500);
    return () => { alive = false; accountRequest.current++; clearInterval(timer); };
  }, [active, privacyMode, api]);
  const accountAction = async (action: "login" | "logout") => {
    if (accountPending || privacyMode) return;
    const operation = api[action]; if (!operation) return;
    const token = ++accountRequest.current; accountActionInFlight.current = true; setAccountPending(true);
    try { const snapshot = await operation(); if (mounted.current && token === accountRequest.current) accountHandler.current(snapshot); }
    catch { if (mounted.current && token === accountRequest.current) accountFailure(); }
    finally { accountActionInFlight.current = false; if (mounted.current) setAccountPending(false); }
  };
  useEffect(() => {
    if (!active || privacyMode || view !== "explore") {
      for (const tab of tabRows.current.values()) if (tab.appendBusy) { tab.token++; tab.appendBusy = false; tab.appendError = ""; }
      setAppendBusy(false); setAppendError(""); updateTabButtons();
    }
    if (!active) { generation.current++; detailGeneration.current++; clearTimeout(closeTimer.current); setDetailClosing(false); setBusy(false); setDetailInput(null); setDetail(null); context.close(); return; }
    if (view === "downloads") void refreshDownloads();
    else if (view === "explore" && !privacyMode) { const tab = tabRows.current.get(currentTab.current); if (tab) displayTab(tab, true); }
    setSelected([]); anchor.current = null;
    return () => { generation.current++; setBusy(false); };
  // Context close intentionally excluded: the anchor must not refetch results.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, view, privacyMode, refreshDownloads, search, displayTab]);
  const accountReady = !api.account || Boolean(account && account.authenticated !== null && !account.busy && !accountPending);
  useEffect(() => {
    if (!active || view !== "explore" || privacyMode) return;
    if (!accountReady) { homeSession.current = null; return; }
    const session = `${api.account ? `${account?.revision}:${account?.authenticated}` : "public"}:${safeOnly}`;
    const firstVisit = !initialized.current;
    const sessionChanged = homeSession.current !== session;
    homeSession.current = session;
    if (firstVisit) {
      initialized.current = true;
      // Waiting for account proof must not erase text already being typed.
      if (!currentTab.current) void search("", "home", 1, undefined, false, undefined, { preserveDraft: true });
    } else if (sessionChanged && isPixivHome(tabRows.current.get(currentTab.current)?.mode ?? "latest") && !query.trim()) {
      void search("", tabRows.current.get(currentTab.current)!.mode, 1, undefined, false, undefined, { refresh: true, preserveDraft: true });
    }
  }, [active, view, privacyMode, accountReady, account?.revision, account?.authenticated, api, query, search, safeOnly]);
  useEffect(() => {
    const token = ++relatedEpoch.current;
    setRelated(undefined); setRelatedError(""); setRelatedBusy(false);
    if (!active || privacyMode || !accountReady || !readerOpen || !detail || !api.discover) return;
    // Local albums stay offline. Related browsing is optional, never a
    // prerequisite for viewing a downloaded page.
    if (detail.pages[0]?.previewUrl.startsWith("atsumi-pixiv:")) return;
    setRelatedBusy(true);
    void api.discover({ surface: "related", workId: detail.work.id, ...(safeOnly ? { safeOnly: true } : {}) }).then(result => {
      if (token === relatedEpoch.current && mounted.current) setRelated(result);
    }, () => { if (token === relatedEpoch.current && mounted.current) setRelatedError("관련 작품을 불러오지 못했습니다."); }).finally(() => {
      if (token === relatedEpoch.current && mounted.current) setRelatedBusy(false);
    });
    return () => { relatedEpoch.current++; };
  }, [active, privacyMode, accountReady, account?.revision, readerOpen, detail, api, safeOnly, relatedRetry]);
  const appendRelated = async () => {
    if (!related?.cursor || !detail || !api.discover || relatedBusy || privacyMode || !accountReady) return;
    const token = relatedEpoch.current; setRelatedBusy(true); setRelatedError("");
    try {
      const result = await api.discover({ surface: "related", workId: detail.work.id, cursor: related.cursor, page: related.page + 1, ...(safeOnly ? { safeOnly: true } : {}) });
      if (token === relatedEpoch.current && mounted.current) setRelated(current => current ? appendDiscovery(current, result) : current);
    } catch { if (token === relatedEpoch.current && mounted.current) setRelatedError("관련 작품을 더 불러오지 못했습니다."); }
    finally { if (token === relatedEpoch.current && mounted.current) setRelatedBusy(false); }
  };
  useEffect(() => {
    if (!active || runtime !== "tauri" && !injected) return;
    let alive = true, inFlight = false;
    const poll = () => { if (inFlight) return; inFlight = true; void api.progress().then(rows => { if (alive) setProgress(rows); }, () => {}).finally(() => { inFlight = false; }); };
    poll(); const timer = window.setInterval(poll, 900); return () => { alive = false; clearInterval(timer); };
  }, [active, api, injected, runtime]);
  useEffect(() => { let alive = true; void api.downloads().then(rows => { if (alive) replaceDownloads(rows); }, () => {}); return () => { alive = false; }; }, [api, replaceDownloads]);
  useEffect(() => {
    let alive = true, stop: (() => void) | undefined;
    void api.subscribeDownloads?.(outcome => {
      if (!alive) return;
      if (outcome.saved) { const saved = outcome.saved; setDownloads(rows => [saved, ...rows.filter(row => row.work.id !== saved.work.id)]); }
      setProgress(rows => rows.filter(row => row.id !== outcome.id));
    }).then(unsubscribe => { if (alive) stop = unsubscribe; else unsubscribe(); }).catch(() => { /* Batch invoke still returns every completed item. */ });
    return () => { alive = false; stop?.(); };
  }, [api]);

  const entries = useMemo<Entry[]>(() => {
    const rows: Entry[] = view === "explore" ? works.map(work => ({ work })) : view === "downloads" ? downloads.map(download => ({ work: download.work, download })) : bookmarks.filter(item => category === "albums" ? item.page === undefined : item.page !== undefined);
    const term = view === "explore" ? "" : localQueries[view].trim().normalize("NFKC").toLocaleLowerCase();
    return term ? rows.filter(({ work }) => [work.id, work.title, work.artist, ...work.tags].some(value => value.normalize("NFKC").toLocaleLowerCase().includes(term))) : rows;
  }, [works, downloads, bookmarks, category, view, localQueries]);
  const selectionEntries = readerOpen && related ? discoveryWorks(related).map(work => ({ work } as Entry)) : entries;
  const ids = selectionEntries.map(entry => bookmarkKey(entry.work, entry.page));
  const favorite = (work: PixivWork, atPage?: number) => bookmarks.some(entry => bookmarkKey(entry.work, entry.page) === bookmarkKey(work, atPage));
  const toggleFavorite = (work: PixivWork, atPage?: number) => setBookmarks(current => {
    const key = bookmarkKey(work, atPage);
    if (current.some(entry => bookmarkKey(entry.work, entry.page) === key)) return current.filter(entry => bookmarkKey(entry.work, entry.page) !== key);
    if (current.length >= 2000) { onNotice?.("즐겨찾기는 최대 2,000개까지 저장할 수 있습니다."); return current; }
    const pagePreview = atPage !== undefined && detail?.work.id === work.id ? detail.pages[atPage]?.previewUrl : undefined;
    return [{ work: pagePreview?.startsWith("https:") ? { ...work, thumbnailUrl: pagePreview } : work, page: atPage, savedAt: Date.now() }, ...current];
  });
  const download = async (work: PixivWork) => {
    if (downloaded.has(work.id)) { if (incomplete.has(work.id)) notice("저장된 파일을 확인해 주세요. 다시 받으려면 보관함에서 기존 항목을 먼저 제거해 주세요."); return !incomplete.has(work.id); }
    if (work.type === "ugoira") { notice("움직이는 작품은 아직 저장을 지원하지 않습니다. 원문에서 확인해 주세요."); return false; }
    setError(""); setProgress(rows => rows.some(row => row.id === work.id) ? rows : [...rows, { id: work.id, completedPages: 0, totalPages: work.pageCount, bytes: 0, cancelled: false }]);
    try { const saved = await api.download(work.id); if (!mounted.current) return false; setDownloads(rows => [saved, ...rows.filter(row => row.work.id !== saved.work.id)]); onNotice?.(`${work.title} 저장 완료`); return true; }
    catch (reason) { if (mounted.current) notice(errorText(reason)); return false; }
    finally { if (mounted.current) setProgress(rows => rows.filter(row => row.id !== work.id)); }
  };
  const openOriginal = (id: string) => { void api.open(id).catch(reason => notice(errorText(reason))); };
  const removeSaved = async () => {
    setLibraryBusy(true);
    const completed: { id: string; token: string }[] = [];
    try {
      for (const entry of removeEntries) {
        const result = await api.libraryAction(entry.work.id, "remove");
        if (result.token) { completed.push({ id: entry.work.id, token: result.token }); setDownloads(rows => rows.filter(row => row.work.id !== entry.work.id)); setRemoveEntries(rows => rows.filter(row => row.work.id !== entry.work.id)); }
      }
      setSelected([]); setRemoveEntries([]); loader.dispose(); localLoader.dispose();
    } catch (reason) { notice(errorText(reason)); }
    finally { if (completed.length) { setRemoved(rows => [...rows, ...completed]); loader.dispose(); localLoader.dispose(); } setLibraryBusy(false); }
  };
  const undoRemove = async () => {
    setLibraryBusy(true);
    try { for (const entry of removed) { const result = await api.libraryAction(entry.id, "restore", entry.token); if (result.record) { const record = result.record; setDownloads(rows => [record, ...rows.filter(row => row.work.id !== entry.id)]); } setRemoved(rows => rows.filter(row => row.token !== entry.token)); } }
    catch (reason) { notice(errorText(reason)); }
    finally { setLibraryBusy(false); }
  };
  const openDetail = async (workOrInput: PixivWork | string, atPage = 0) => {
    setReaderOpen(false);
    setReaderInfoOpen(false); setConfirmCloseDetails(false);
    clearTimeout(closeTimer.current); setDetailClosing(false);
    const input = typeof workOrInput === "string" ? workOrInput : workOrInput.id;
    const id = artworkId(input);
    if (id) setDetailTabs(rows => rows.some(row => row.id === id) ? rows : [...rows, { id, title: typeof workOrInput === "string" ? `#${id}` : workOrInput.title, page: atPage }].slice(-24));
    const token = ++detailGeneration.current;
    if (!detailInput) priorFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    readerOpener.current = priorFocus.current;
    setDetailInput(input); setDetail(null); setDetailLoading(true); setDetailError(""); setDetailPage(atPage); setSelected([]);
    try { const result = id && downloaded.has(id) ? await api.localDetail(id) : await api.detail(input); if (token !== detailGeneration.current) return; setDetail(result); setDetailPage(Math.min(atPage, Math.max(0, result.pages.length - 1))); setDetailTabs(rows => rows.map(row => row.id === result.work.id ? { ...row, title: result.work.title } : row)); setReaderOpen(result.work.type !== "ugoira" && result.pages.length > 0); }
    catch (reason) { if (token === detailGeneration.current) setDetailError(errorText(reason)); }
    finally { if (token === detailGeneration.current) setDetailLoading(false); }
  };
  const closeDetail = useCallback((immediate = false) => {
    setReaderOpen(false);
    detailGeneration.current++; clearTimeout(closeTimer.current);
    const finish = () => { setDetailClosing(false); setDetailInput(null); setDetail(null); setDetailLoading(false); priorFocus.current?.focus({ preventScroll: true }); };
    if (immediate || window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) finish();
    else { setDetailClosing(true); closeTimer.current = setTimeout(finish, 140); }
  }, []);
  const closeAllDetails = () => { if (detailTabs.length > 1) setConfirmCloseDetails(true); else { setDetailTabs([]); closeDetail(); } };
  const dismissCloseConfirmation = () => {
    setConfirmCloseDetails(false);
    // The auto-focused confirmation button is removed; explicitly return to
    // the current overlay rather than leaving Tab on the background search.
    (document.querySelector<HTMLButtonElement>('.pixiv-reader [aria-label="미리보기 닫기"]') ?? closeButton.current)?.focus({ preventScroll: true });
  };
  const closeDetailTab = (id: string) => {
    const remaining = detailTabs.filter(row => row.id !== id); setDetailTabs(remaining);
    if (detailInput === id) { const next = remaining.at(-1); if (next) void openDetail(next.id, next.page); else closeDetail(); }
  };
  const closeSearchTab = (id: string) => {
    const tab = tabRows.current.get(id); if (!tab) return; tab.token++; tabRows.current.delete(id); updateTabButtons();
    if (id !== currentTab.current) return;
    const previous = [...tabRows.current.values()].at(-1);
    if (previous) displayTab(previous); else { currentTab.current = ""; setWorks([]); setHomeDiscovery(undefined); setBusy(false); setAppendBusy(false); setAppendError(""); setError(""); setHasMore(false); setHomeSuggestions(undefined); setCommitted({ query: "", mode: "home" }); }
    if (tab.returnDetail) { setDiscoveryOverride(tab.returnDetail.view === "explore" && outerView !== "explore"); onViewChange?.(tab.returnDetail.view); setDetail(tab.returnDetail.value); setDetailInput(tab.returnDetail.value.work.id); setDetailPage(tab.returnDetail.page); setDetailLoading(false); setDetailError(""); setReaderOpen(tab.returnDetail.value.work.type !== "ugoira" && tab.returnDetail.value.pages.length > 0); }
  };
  const exploreRelated = (value: string, nextMode: PixivMode, label: string, background = false) => {
    const origin = detail ? { value: detail, page: detailPage, view } : undefined;
    if (!background) { closeDetail(true); setDiscoveryOverride(true); onViewChange?.("explore"); }
    void search(value, nextMode, 1, label, background, origin);
  };
  const downloadSelection = () => {
    const chosen = [...new Map(selectionEntries.filter(entry => selected.includes(bookmarkKey(entry.work, entry.page)) && !downloaded.has(entry.work.id) && !progress.some(row => row.id === entry.work.id) && entry.work.type !== "ugoira").map(entry => [entry.work.id, entry.work])).values()];
    if (!chosen.length) return;
    setProgress(rows => [...rows, ...chosen.filter(work => !rows.some(row => row.id === work.id)).map(work => ({ id: work.id, completedPages: 0, totalPages: work.pageCount, bytes: 0, cancelled: false, queued: true }))]);
    void api.downloadBatch(chosen.map(work => work.id)).then(outcomes => {
      if (!mounted.current) return;
      const saved = outcomes.flatMap(outcome => outcome.saved ? [outcome.saved] : []);
      setDownloads(rows => [...saved, ...rows.filter(row => !saved.some(item => item.work.id === row.work.id))]);
      const cancelled = outcomes.filter(outcome => outcome.error?.includes("취소"));
      const failed = outcomes.filter(outcome => outcome.error && !outcome.error.includes("취소"));
      if (failed.length) notice(`${failed.length}개 저장 실패 · ${failed[0]?.error}`); else onNotice?.(cancelled.length ? `${saved.length}개 저장 · ${cancelled.length}개 취소 완료` : `${saved.length}개 저장 완료`);
    }, reason => notice(errorText(reason))).finally(() => { if (mounted.current) setProgress(rows => rows.filter(row => !chosen.some(work => row.id === work.id))); });
  };
  const openSelection = (entry: Entry) => {
    const chosen = selected.length > 1 ? [...new Map(selectionEntries.filter(item => selected.includes(bookmarkKey(item.work, item.page))).map(item => [item.work.id, item])).values()].slice(0, 24) : [entry];
    setDetailTabs(rows => [...new Map([...rows, ...chosen.map(item => ({ id: item.work.id, title: item.work.title, page: item.page ?? 0 }))].map(item => [item.id, item])).values()].slice(-24));
    const first = chosen[0]; if (first) void openDetail(first.work, first.page);
  };
  useEffect(() => {
    if (!detailInput) return;
    const keyboard = (event: KeyboardEvent) => {
      const ownDialog = closeButton.current?.closest('[role="dialog"]');
      if (detailClosing || !active || document.documentElement.dataset.tutorialOpen === "true" || document.querySelector('dialog[open], [data-card-context-menu]')) return;
      if ([...document.querySelectorAll<HTMLElement>('[aria-modal="true"]')].some(node => node !== ownDialog && !ownDialog?.contains(node) && !node.closest('[hidden],[inert]') && getComputedStyle(node).display !== "none")) return;
      if (event.key === "Escape") { event.preventDefault(); if (confirmCloseDetails) setConfirmCloseDetails(false); else closeDetailTab(detailInput); return; }
      if (confirmCloseDetails) return;
      if (event.target instanceof Element && event.target.matches("input,textarea,select,[contenteditable]")) return;
      if (["a", "A", "ArrowLeft"].includes(event.key)) { event.preventDefault(); setDetailPage(value => Math.max(0, value - 1)); }
      if (["d", "D", "ArrowRight"].includes(event.key)) { event.preventDefault(); setDetailPage(value => Math.min(Math.max(0, (detail?.pages.length ?? 1) - 1), value + 1)); }
      if (["q", "Q", "e", "E"].includes(event.key) && detailTabs.length > 1) { event.preventDefault(); const index = detailTabs.findIndex(row => row.id === detailInput); const next = detailTabs[(index + (event.key.toLowerCase() === "q" ? -1 : 1) + detailTabs.length) % detailTabs.length]; if (next) void openDetail(next.id, next.page); }
      if (event.key === "Tab") {
        const buttons = [...(closeButton.current?.closest("[role=dialog]")?.querySelectorAll<HTMLElement>("button:not(:disabled),a[href],[tabindex='0']") ?? [])];
        const first = buttons[0], last = buttons[buttons.length - 1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
      }
    };
    window.addEventListener("keydown", keyboard, true); return () => window.removeEventListener("keydown", keyboard, true);
  }, [detailInput, detail?.pages.length, closeDetail, detailTabs, downloaded, confirmCloseDetails, detailClosing, active]);
  useEffect(() => { if (detailInput) closeButton.current?.focus(); }, [detailInput]);
  useEffect(() => { if (detailInput) setDetailTabs(rows => rows.map(row => row.id === detailInput && row.page !== detailPage ? { ...row, page: detailPage } : row)); }, [detailPage, detailInput]);
  useEffect(() => {
    if (privacyMode || !detail || !active) { originalPool.clear(); localOriginalPool.clear(); }
    if (!detail || !active) setReaderOpen(false);
  }, [detail, privacyMode, originalPool, localOriginalPool, active]);
  const select = (event: MouseEvent<HTMLElement>, entry: Entry) => {
    if (isCardControl(event.target)) return;
    const key = bookmarkKey(entry.work, entry.page);
    const next = updateSelection(ids, selected, key, anchor.current, { ctrl: event.ctrlKey || event.metaKey, shift: event.shiftKey });
    setSelected(next);
    if (!next.length) event.currentTarget.blur();
    if (!event.shiftKey) anchor.current = key;
  };
  const renderEntry = (entry: Entry) => {
    const key = bookmarkKey(entry.work, entry.page), checked = selected.includes(key), saved = favorite(entry.work, entry.page);
    return <article key={key} className={`pixiv-card${checked ? " is-focused" : ""}`} tabIndex={0} aria-label={privacyMode ? "작품" : entry.work.title} onClick={event => select(event, entry)}
      onDoubleClick={event => { if (!isCardControl(event.target)) void openDetail(entry.work, entry.page); }}
      onContextMenu={event => { if (!isCardControl(event.target)) { setContextEntry(entry); context.open(event); } }}
      onKeyDown={event => { if (context.onKeyDown(event)) { setContextEntry(entry); return; } if (event.target !== event.currentTarget) return; if (event.key === "Enter") { event.preventDefault(); if (event.ctrlKey || event.metaKey) { if (selected.length > 1) downloadSelection(); else void download(entry.work); } else openSelection(entry); } if (event.key === " ") { event.preventDefault(); setSelected(current => current.includes(key) ? current.filter(value => value !== key) : [...current, key]); } }}>
      <PixivImage url={downloaded.has(entry.work.id) ? localPageUrl(entry.work.id, entry.page ?? 0) : entry.work.thumbnailUrl} title={entry.work.title} privacyMode={privacyMode} loader={downloaded.has(entry.work.id) ? localLoader : loader} />
      {selected.length > 1 && <span className="pixiv-card-check" role="checkbox" aria-checked={checked}>{checked && <Icon kind="check" />}</span>}
      <button className={`pixiv-bookmark${saved ? " is-saved" : ""}`} type="button" aria-label={saved ? "즐겨찾기 해제" : "즐겨찾기"} aria-pressed={saved} title={saved ? "즐겨찾기 해제" : "즐겨찾기"} onClick={() => toggleFavorite(entry.work, entry.page)}><Icon kind="bookmark" /></button>
      {downloaded.has(entry.work.id) && !incomplete.has(entry.work.id) && <span className="pixiv-saved-marker" title="저장 완료" aria-label="저장 완료"><Icon kind="check" /></span>}
      {incomplete.has(entry.work.id) && <span className="pixiv-saved-marker" title="파일 확인 필요" aria-label="파일 확인 필요">!</span>}
      <div className="pixiv-card-info"><PixivMetadataText as="strong" text={entry.work.title} kind="title" private={privacyMode}>{privacyMode ? "작품" : splitGalleryTitle(entry.work.title).primary}</PixivMetadataText><PixivMetadataText as="button" text={entry.work.artist} kind="artist" private={privacyMode} className="pixiv-artist" disabled={!/^[1-9]\d{0,11}$/.test(entry.work.artistId)} onClick={event => exploreRelated(entry.work.artistId, "artist", entry.work.artist, event.ctrlKey || event.metaKey)} /><small>{entry.page !== undefined ? `${entry.page + 1}쪽` : entry.work.type === "ugoira" ? "움직이는 작품" : `${entry.work.pageCount}쪽`}{entry.download ? ` · ${formatBytes(entry.download.bytes)}` : entry.work.bookmarkCount > 0 ? ` · ♥ ${entry.work.bookmarkCount.toLocaleString()}` : ""}</small></div>
    </article>;
  };
  const renderDiscovery = (value: PixivDiscovery) => <PixivDiscoverySections value={value} privacyMode={privacyMode} renderWork={work => renderEntry({ work })}
    renderImage={(url, name) => <PixivImage url={url} title={name} privacyMode={privacyMode} loader={loader} />}
    onTag={tag => exploreRelated(tag, "latest", tag)} onCreator={(id, name, background) => exploreRelated(id, "artist", name, background)} />;
  if (!active) return null;
  const detailWork = detail?.work;
  const detailActions = detailWork && <div className="pixiv-detail-actions">
    <button type="button" aria-pressed={favorite(detailWork)} onClick={() => toggleFavorite(detailWork)}><Icon kind="bookmark" />앨범</button>
    <button type="button" className="pixiv-save-button" disabled={downloaded.has(detailWork.id) || detailWork.type === "ugoira" || progress.some(row => row.id === detailWork.id && row.cancelled)} onClick={() => { if (progress.some(row => row.id === detailWork.id)) { void api.cancel(detailWork.id).then(() => setProgress(rows => rows.map(row => row.id === detailWork.id ? { ...row, cancelled: true } : row)), reason => notice(errorText(reason))); } else void download(detailWork); }}><Icon kind={downloaded.has(detailWork.id) ? "check" : progress.some(row => row.id === detailWork.id) ? "close" : "download"} />{downloaded.has(detailWork.id) ? "저장 완료" : progress.some(row => row.id === detailWork.id && row.cancelled) ? "취소 중" : progress.some(row => row.id === detailWork.id) ? "다운로드 취소" : "다운로드"}</button>
    <button type="button" onClick={() => openOriginal(detailWork.id)}>웹에서 열기</button>
  </div>;
  const workTabs = detailTabs.length > 1 && <nav className="pixiv-search-tabs pixiv-detail-tabs" aria-label="열린 작품">{detailTabs.map(tab => <div className={tab.id === detailInput ? "is-active" : ""} key={tab.id}><PixivMetadataText as="button" text={tab.title} kind="title" private={privacyMode} onClick={() => void openDetail(tab.id, tab.page)}>{privacyMode ? "작품" : splitGalleryTitle(tab.title).primary}</PixivMetadataText><button type="button" aria-label={`${privacyMode ? "작품" : tab.title} 닫기`} onClick={() => closeDetailTab(tab.id)}><Icon kind="close" /></button></div>)}</nav>;
  return <section ref={workspace} className="pixiv-workspace" aria-label="Pixiv" style={{ "--pixiv-preview-width": `${normalizeImagePreviewWidth(previewWidth)}px` } as CSSProperties}>
    <header className="pixiv-toolbar">
    {view === "explore" ? <form className="pixiv-search" data-tour="pixiv-input" onSubmit={event => { event.preventDefault(); const id = artworkId(query); if (id) void openDetail(id); else if (/^https?:/i.test(query.trim())) setError("Pixiv 작품 링크 또는 작품 번호를 입력해 주세요."); else void search(query, !query.trim() && mode === "latest" ? "home" : mode); }}>
      <div className="pixiv-search-input" onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setHistoryOpen(false); }}><input aria-label="Pixiv 검색어 또는 작품 링크" placeholder="태그, 작품 링크 또는 번호" value={query} onFocus={() => setHistoryOpen(true)} onChange={event => { setQuery(event.target.value); if (event.target.value.trim() && !artworkId(event.target.value)) setMode("latest"); }} onKeyDown={event => { if (event.key === "Escape") setHistoryOpen(false); }} />
        {historyOpen && !privacyMode && <div className="pixiv-search-panel" aria-label="Pixiv 검색 태그 패널">
          {history.length > 0 && <div className="pixiv-history" aria-label="최근 검색 기록">{history.map(item => <div key={item}><PixivMetadataText as="button" text={item} kind="tag" hints={searchSuggestions?.tagTranslations} onClick={() => { setQuery(item); setMode("latest"); void search(item, "latest"); }} /><button type="button" aria-label={`${item} 기록 삭제`} onClick={() => setHistory(rows => rows.filter(row => row !== item))}><Icon kind="close" /></button></div>)}<button type="button" className="pixiv-history-clear" onClick={() => setHistory([])}>기록 모두 지우기</button></div>}
          <PixivTagGroups searchPanel suggestions={searchSuggestions} authenticated={accountReady && account?.authenticated === true} onSearch={tag => void search(tag, "latest")} />
          {!searchSuggestions && !history.length && <p className="pixiv-panel-empty">태그나 작품 번호로 검색해 보세요.</p>}
        </div>}
      </div>
      <select aria-label="Pixiv 검색 정렬" value={mode} onChange={event => { const next = event.target.value as PixivMode; setMode(next); if (next !== "latest") setQuery(""); }}><option value="home">추천 작품</option><option value="illustration">일러스트</option><option value="manga">만화</option><option value="latest">최신 작품</option><option value="daily">일간 랭킹</option><option value="weekly">주간 랭킹</option></select>
      <button type={busy ? "button" : "submit"} onClick={busy ? () => { const tab = tabRows.current.get(currentTab.current); if (tab) { tab.token++; tab.busy = false; tab.error = "검색을 취소했습니다."; updateTabButtons(); } setBusy(false); } : undefined}>{busy ? <><span className="pixiv-spinner" />취소</> : "검색"}</button>
    </form> : <form className="pixiv-search pixiv-local-search" onSubmit={event => event.preventDefault()}><div className="pixiv-search-input"><input type="search" aria-label={view === "favorites" ? "Pixiv 즐겨찾기 검색" : "저장한 Pixiv 작품 검색"} placeholder="제목, 작가, 태그 또는 작품 번호" value={localQueries[view]} onChange={event => { const value = event.target.value; setLocalQueries(rows => ({ ...rows, [view]: value })); setSelected([]); }} /></div>{view === "downloads" && <button type="button" aria-label="보관함 새로고침" onClick={() => void refreshDownloads()} disabled={busy}>{busy ? <span className="pixiv-spinner" /> : <Icon kind="refresh" />}</button>}</form>}
      {api.account && <div className="pixiv-account-controls" aria-label="Pixiv 계정">
        <span role="status">{accountPending || account?.busy ? "확인 중" : account?.authenticated === true ? "로그인됨" : account?.authenticated === false ? "로그아웃됨" : "로그인 상태 확인 필요"}</span>
        <button type="button" disabled={privacyMode || accountPending || account?.busy || account?.open || account?.authenticated === true} onClick={() => void accountAction("login")}>로그인</button>
        <button type="button" disabled={privacyMode || accountPending || account?.busy || account?.open || account?.authenticated !== true} onClick={() => void accountAction("logout")}>로그아웃</button>
      </div>}
    </header>
    {api.account && account?.error && <div className="pixiv-account-error" role="alert">{account.error}</div>}
    {view === "favorites" && <div className="pixiv-segments" role="group" aria-label="즐겨찾기 종류"><button type="button" aria-pressed={category === "albums"} onClick={() => setCategory("albums")}>앨범</button><button type="button" aria-pressed={category === "pages"} onClick={() => setCategory("pages")}>페이지</button></div>}
    {view === "explore" && tabButtons.length > 0 && <nav className="pixiv-search-tabs" aria-label="Pixiv 탐색 탭">{tabButtons.map(tab => <div className={tab.id === currentTab.current ? "is-active" : ""} key={tab.id}><PixivMetadataText as="button" text={tab.label} kind={tabRows.current.get(tab.id)?.mode === "artist" ? "artist" : "tag"} hints={searchSuggestions?.tagTranslations} private={privacyMode} onClick={() => { const current = tabRows.current.get(currentTab.current); if (current && workspace.current) current.scroll = workspace.current.scrollTop; const target = tabRows.current.get(tab.id); if (target) displayTab(target); }}>{tab.busy && <span className="pixiv-spinner" />}{tab.label}</PixivMetadataText><button type="button" aria-label={`${tab.label} 닫기`} onClick={() => closeSearchTab(tab.id)}><Icon kind="close" /></button></div>)}</nav>}
    {view === "explore" && <nav className="pixiv-home-categories" aria-label="Pixiv 작품 분류">
      {(["home", "illustration", "manga"] as const).map(category => <button key={category} type="button" aria-current={committed.mode === category ? "page" : undefined} disabled={privacyMode || !accountReady} onClick={() => void search("", category)}>{category === "home" ? "홈" : category === "illustration" ? "일러스트" : "만화"}</button>)}
      {homeDiscovery && isPixivHome(committed.mode) && <button className="pixiv-home-refresh" type="button" aria-label="추천 작품 새로고침" disabled={busy || !accountReady || privacyMode} onClick={() => void search("", committed.mode, 1, undefined, false, undefined, { refresh: true, preserveDraft: true })}><Icon kind="refresh" /></button>}
    </nav>}
    {view === "explore" && isPixivHome(committed.mode) && !privacyMode && (homeDiscovery
      ? <PixivTagGroups suggestions={homeSuggestions} authenticated={accountReady && account?.authenticated === true} onSearch={tag => void search(tag, "latest")} />
      : <PixivHomeHeader suggestions={homeSuggestions} authenticated={accountReady && account?.authenticated === true} disabled={busy || !accountReady} retained={Boolean(currentTab.current)} onSearch={tag => { void search(tag, "latest"); }} onRefresh={() => { void search("", committed.mode, 1, undefined, false, undefined, { refresh: true, preserveDraft: true }); }} />)}
    {error && <div className="pixiv-message" role="alert">{error}</div>}
    {removed.length > 0 && view === "downloads" && <div className="pixiv-message" role="status">복원할 수 있는 작품 {removed.length}개<button type="button" disabled={libraryBusy} onClick={() => void undoRemove()}>{libraryBusy ? <span className="pixiv-spinner" /> : "되돌리기"}</button></div>}
    {removeEntries.length > 0 && createPortal(<div className="pixiv-detail-backdrop"><div className="pixiv-library-confirm" role="alertdialog" aria-modal="true" aria-label="보관함에서 제거" onKeyDown={event => { if (event.key === "Escape" && !libraryBusy) setRemoveEntries([]); if (event.key === "Tab") { const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")]; if (event.shiftKey && event.target === buttons[0]) { event.preventDefault(); buttons.at(-1)?.focus(); } else if (!event.shiftKey && event.target === buttons.at(-1)) { event.preventDefault(); buttons[0]?.focus(); } } }}><strong>작품 {removeEntries.length}개를 보관함에서 제거할까요?</strong><p>파일은 복원용 폴더에 남고, 즐겨찾기는 유지됩니다.</p><button type="button" disabled={libraryBusy} onClick={() => void removeSaved()}>{libraryBusy ? <span className="pixiv-spinner" /> : "제거"}</button><button type="button" autoFocus disabled={libraryBusy} onClick={() => setRemoveEntries([])}>취소</button></div></div>, document.body)}
    {progress.length > 0 && <div className="pixiv-progress-list" aria-label="Pixiv 다운로드 진행">{progress.map(row => <div key={row.id}><span>#{row.id} · {row.cancelled ? "취소 중" : row.queued ? "대기 중" : row.totalPages > 0 && row.completedPages >= row.totalPages ? "저장 마무리 중" : `다운로드 중 ${row.completedPages}/${row.totalPages}`}</span><progress max={Math.max(row.totalPages, 1)} value={row.completedPages} /><button type="button" disabled={row.cancelled} onClick={() => { void api.cancel(row.id).then(() => setProgress(rows => rows.map(item => item.id === row.id ? { ...item, cancelled: true } : item)), reason => notice(errorText(reason))); }}>취소</button></div>)}</div>}
    {selected.length > 1 && <div className="pixiv-selection-bar"><span>{selected.length}개 선택</span>{view === "downloads" ? <button type="button" onClick={() => setRemoveEntries(entries.filter(entry => selected.includes(bookmarkKey(entry.work, entry.page))))}>보관함에서 제거</button> : <button type="button" onClick={downloadSelection}>다운로드</button>}{entries.some(entry => selected.includes(bookmarkKey(entry.work, entry.page)) && progress.some(row => row.id === entry.work.id)) && <button type="button" onClick={() => { for (const entry of entries.filter(entry => selected.includes(bookmarkKey(entry.work, entry.page)))) void api.cancel(entry.work.id).catch(reason => notice(errorText(reason))); }}>다운로드 취소</button>}<button type="button" onClick={() => setSelected([])}>선택 해제</button></div>}
    <div data-tour="pixiv-content" aria-busy={busy}>{view === "explore" && isPixivHome(committed.mode) && homeDiscovery ? renderDiscovery(homeDiscovery) : <div className="pixiv-grid">{entries.map(renderEntry)}</div>}</div>
    {!entries.length && <div className="pixiv-empty" role="status">{view === "explore" && privacyMode ? "프라이버시 모드를 해제하면 작품을 불러옵니다." : busy || view === "explore" && api.account && !account ? <><span className="pixiv-spinner" />불러오는 중</> : error || view === "explore" && !accountReady ? "" : view === "favorites" ? "즐겨찾기한 작품이 없습니다." : view === "downloads" ? "저장한 작품이 없습니다." : committed.mode === "home" ? currentTab.current ? "추천 작품이 없습니다." : "추천 작품을 다시 불러오거나 검색어를 입력해 보세요." : "검색 결과가 없습니다."}</div>}
    {view === "explore" && isPixivHome(committed.mode) && !privacyMode && currentTab.current && (hasMore || appendBusy || appendError) && <div className="pixiv-home-more" aria-label="추천 작품 추가 불러오기">
      {appendError && <p role="alert">{appendError}</p>}
      <button type="button" disabled={busy || !accountReady} aria-label={appendBusy ? "추천 작품 추가 불러오기 취소" : appendError ? "추천 작품 추가 불러오기 다시 시도" : "추천 작품 더 불러오기"} onClick={() => { if (appendBusy) cancelAppend(); else void appendHome(); }}>{appendBusy ? <><span className="pixiv-spinner" />취소</> : appendError ? "다시 시도" : "더 불러오기"}</button>
    </div>}
    {view === "explore" && !isPixivHome(committed.mode) && entries.length > 0 && <nav className="pixiv-pagination" aria-label="Pixiv 결과 페이지"><button type="button" disabled={busy || page <= 1} onClick={() => void search(committed.query, committed.mode, page - 1)}>이전</button><span>{page}</span><button type="button" disabled={busy || !hasMore} onClick={() => void search(committed.query, committed.mode, page + 1)}>다음</button></nav>}
    <CardContextMenu anchor={context.anchor} close={context.close} label={contextEntry?.work.title ?? "작품"} items={contextEntry ? [
      { id: "open", label: "상세 보기", shortcut: "Enter", action: () => { void openDetail(contextEntry.work, contextEntry.page); } },
      { id: "favorite", label: favorite(contextEntry.work, contextEntry.page) ? "즐겨찾기 해제" : "즐겨찾기", action: () => toggleFavorite(contextEntry.work, contextEntry.page) },
      { id: "download", label: incomplete.has(contextEntry.work.id) ? "파일 확인 필요" : downloaded.has(contextEntry.work.id) ? "저장 완료" : progress.some(row => row.id === contextEntry.work.id) ? "다운로드 취소" : "원본 다운로드", disabled: contextEntry.work.type === "ugoira" || downloaded.has(contextEntry.work.id), action: () => { if (progress.some(row => row.id === contextEntry.work.id)) void api.cancel(contextEntry.work.id).catch(reason => notice(errorText(reason))); else void download(contextEntry.work); } },
      { id: "source", label: "Pixiv에서 열기", action: () => openOriginal(contextEntry.work.id) },
      ...(downloaded.has(contextEntry.work.id) ? [
        { id: "folder", label: "저장 폴더 열기", action: () => { void api.libraryAction(contextEntry.work.id, "open").catch(reason => notice(errorText(reason))); } },
        { id: "remove", label: "보관함에서 제거", action: () => setRemoveEntries([contextEntry]) },
      ] : []),
    ] : []} />
    {detailInput && !readerOpen && createPortal(<div className={`pixiv-detail-backdrop${detailClosing ? " is-closing" : ""}`} onMouseDown={event => { if (event.target === event.currentTarget) closeAllDetails(); }}><section className="pixiv-detail" role="dialog" aria-modal="true" aria-label="Pixiv 작품 상세" data-gallery-shortcuts-suspended>
      <header><div><PixivMetadataText as="strong" text={detailWork?.title ?? "작품 상세"} kind="title" private={privacyMode} />{detailWork && <PixivMetadataText as="button" text={detailWork.artist} kind="artist" private={privacyMode} className="pixiv-artist" disabled={!/^[1-9]\d{0,11}$/.test(detailWork.artistId)} onClick={event => exploreRelated(detailWork.artistId, "artist", detailWork.artist, event.ctrlKey || event.metaKey)} />}</div><button type="button" className="pixiv-icon-button" ref={closeButton} aria-label="상세 닫기" onClick={closeAllDetails}><Icon kind="close" /></button></header>
      {workTabs}
      {detailLoading ? <div className="pixiv-empty" role="status"><span className="pixiv-spinner" />불러오는 중</div> : detailError ? <div className="pixiv-message" role="alert">{detailError}<button type="button" onClick={() => void openDetail(detailInput)}>다시 시도</button></div> : detailWork && <>
        <div className="pixiv-detail-image">{detailWork.type === "ugoira" ? <PixivImage url={detailWork.thumbnailUrl} title={detailWork.title} privacyMode={privacyMode} loader={loader} detail /> : detail && <div className="pixiv-detail-page-open" role="button" tabIndex={0} aria-label="페이지 이미지 열기" onClick={event => { if (!(event.target instanceof Element && event.target.closest("button"))) setReaderOpen(true); }} onKeyDown={event => { if (event.target === event.currentTarget && (event.key === "Enter" || event.key === " ")) { event.preventDefault(); event.stopPropagation(); setReaderOpen(true); } }}><PixivOriginalPages detail={detail} pages={[detailPage]} pool={detailPool} privacyMode={privacyMode} /></div>}
          {(detail?.pages.length ?? 0) > 1 && <><button className="pixiv-page-arrow is-left" type="button" aria-label="이전 페이지" disabled={detailPage === 0} onClick={() => setDetailPage(value => value - 1)}><Icon kind="left" /></button><button className="pixiv-page-arrow is-right" type="button" aria-label="다음 페이지" disabled={detailPage >= (detail?.pages.length ?? 1) - 1} onClick={() => setDetailPage(value => value + 1)}><Icon kind="right" /></button></>}
          <span className="pixiv-page-number">{detailWork.type === "ugoira" ? "움직이는 작품 · 미리보기" : `${detailPage + 1} / ${detail?.pages.length ?? 0}`}</span>
        </div>
        <footer>{error && <div className="pixiv-message" role="alert">{error}</div>}{detailActions}
          {detailWork.type === "ugoira" && <p>움직이는 작품 재생·저장은 아직 지원하지 않습니다.</p>}
          {!privacyMode && <div className="pixiv-tags">{detailWork.tags.slice(0, 20).map(tag => <PixivMetadataText as="button" text={tag} kind="tag" hints={detailWork.tagTranslations} key={tag} onClick={event => exploreRelated(tag, "latest", tag, event.ctrlKey || event.metaKey)} />)}</div>}
        </footer>
      </>}
    </section></div>, document.body)}
    {readerOpen && detail && <PixivPageReader key={detail.work.id} detail={detail} initialPage={detailPage} initialPinned pool={detailPool} privacyMode={privacyMode} openerElement={readerOpener.current}
      initialScroll={readerScrolls.current.get(detail.work.id) ?? 0} onScrollPosition={position => { readerScrolls.current.set(detail.work.id, position); while (readerScrolls.current.size > 24) readerScrolls.current.delete(readerScrolls.current.keys().next().value!); }}
      related={!privacyMode && api.discover && !detail.pages[0]?.previewUrl.startsWith("atsumi-pixiv:") ? <>
        {selected.length > 1 && <div className="pixiv-selection-bar"><span>{selected.length}개 선택</span><button type="button" onClick={downloadSelection}>다운로드</button><button type="button" onClick={() => setSelected([])}>선택 해제</button></div>}
        {related && renderDiscovery(related)}
        {relatedError && <p className="pixiv-discovery-issues" role="alert">{relatedError}<button type="button" disabled={relatedBusy} onClick={() => related ? void appendRelated() : setRelatedRetry(value => value + 1)}>다시 시도</button></p>}
        {relatedBusy ? <div className="pixiv-related-loading" role="status"><span className="pixiv-spinner" />관련 작품 불러오는 중</div> : related?.cursor && <button type="button" className="pixiv-related-more" onClick={() => void appendRelated()}>관련 작품 더 보기</button>}
      </> : undefined}
      navigation={workTabs}
      toolbar={<>
        {detailActions}
        {!privacyMode && <div className="pixiv-reader-work-info"><PixivMetadataText as="button" text={detail.work.artist} kind="artist" className="pixiv-artist" disabled={!/^[1-9]\d{0,11}$/.test(detail.work.artistId)} onClick={event => exploreRelated(detail.work.artistId, "artist", detail.work.artist, event.ctrlKey || event.metaKey)} />{detail.work.tags.length > 0 && <button type="button" className="pixiv-reader-tags-toggle" aria-expanded={readerInfoOpen} onClick={() => setReaderInfoOpen(value => !value)}>태그</button>}</div>}
        {!privacyMode && readerInfoOpen && <div className="pixiv-tags">{detail.work.tags.slice(0, 20).map(tag => <PixivMetadataText as="button" text={tag} kind="tag" hints={detail.work.tagTranslations} key={tag} onClick={event => exploreRelated(tag, "latest", tag, event.ctrlKey || event.metaKey)} />)}</div>}
        {error && <div className="pixiv-message" role="alert">{error}</div>}
      </>}
      isFavorite={page => favorite(detail.work, page)} onFavorite={page => toggleFavorite(detail.work, page)} onPageChange={setDetailPage}
      onCycleWork={detailTabs.length > 1 ? direction => { const index = detailTabs.findIndex(row => row.id === detailInput); const next = detailTabs[(index + direction + detailTabs.length) % detailTabs.length]; if (next) void openDetail(next.id, next.page); } : undefined}
      onBeforeClose={() => { if (detailTabs.length > 1) { setConfirmCloseDetails(true); return false; } return true; }}
      onClose={() => { setDetailTabs([]); closeDetail(true); }} />}
    {detailInput && confirmCloseDetails && createPortal(<div className="pixiv-confirm-backdrop"><div className="pixiv-close-confirm" role="alertdialog" aria-modal="true" aria-label="열린 작품 닫기" onKeyDown={event => {
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); dismissCloseConfirmation(); }
      if (event.key === "Tab") { const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button")]; if (event.shiftKey && event.target === buttons[0]) { event.preventDefault(); buttons.at(-1)?.focus(); } else if (!event.shiftKey && event.target === buttons.at(-1)) { event.preventDefault(); buttons[0]?.focus(); } }
    }}><strong>작품 {detailTabs.length}개를 닫을까요?</strong><button type="button" onClick={() => { setConfirmCloseDetails(false); setDetailTabs([]); closeDetail(true); }}>닫기</button><button type="button" autoFocus onClick={dismissCloseConfirmation}>취소</button></div></div>, document.body)}
  </section>;
}
