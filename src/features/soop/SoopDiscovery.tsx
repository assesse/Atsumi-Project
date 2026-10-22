import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type MouseEvent } from "react";
import type { SoopApi, SoopHomePage, SoopSearchItem, SoopSearchKind, SoopSearchRequest } from "../../api/soop";
import { platformMediaApi, type PlatformMediaApi } from "../platforms/platformMedia";
import "./SoopDiscovery.css";
import { splitGalleryTitle } from "../../components/galleryCardLayout";
import { SoopVodPlayer } from "./SoopVodPlayer";
import type { SoopVodApi } from "../../api/soopVod";
import { scheduleSoopImage } from "./soopImageQueue";
import { SoopPortrait } from "./SoopPortrait";

type Tab = { id: string; request: SoopSearchRequest; title: string; items: SoopSearchItem[]; page: number; requestedPage: number; more: boolean; loading: boolean; error: string; warnings?: string[]; generation: number };
type SavedVideo = Pick<SoopSearchItem, "id" | "kind" | "title" | "channelId" | "channelName" | "duration" | "restricted" | "thumbnailKey">;
const VIDEO_FAVORITES = "atsumi:soop:video-favorites:v1";
const HISTORY = "atsumi:soop:search-history:v1";
const labels = { all: "통합검색", live: "라이브", video: "영상", channel: "채널" };
const sectionLabels = { streamers: "관련 스트리머", live: "LIVE", related: "관련 콘텐츠", catch: "Catch" };
function audienceLabel(item: SoopSearchItem): string {
  if (item.kind === "channel" || item.views == null || !Number.isFinite(item.views) || item.views < 0) return "";
  return item.kind === "live" ? ` · 시청 ${item.views.toLocaleString()}명` : ` · 조회 ${item.views.toLocaleString()}회`;
}
function read<T>(key: string, fallback: T): T { try { return JSON.parse(localStorage.getItem(key) || "null") ?? fallback; } catch { return fallback; } }
function initialFavorites(): SavedVideo[] {
  const raw = read<unknown>(VIDEO_FAVORITES, []);
  return Array.isArray(raw) ? raw.filter(v => v && v.kind === "video" && /^\d{1,20}$/.test(v.id) && typeof v.title === "string" && typeof v.channelId === "string" && typeof v.channelName === "string").slice(0, 5000) : [];
}
const errorText = (e: unknown) => typeof e === "string" ? e : e instanceof Error ? e.message : "요청을 완료하지 못했습니다.";

function Thumbnail({ item, api, privacy, active }: { item: SoopSearchItem; api: SoopApi; privacy: boolean; active: boolean }) {
  return item.kind === "channel" ? <SoopPortrait api={api} channelId={item.channelId} active={active} privacy={privacy} className="soop-result-image soop-channel-portrait" /> : <MediaThumbnail item={item} api={api} privacy={privacy} active={active} />;
}
function MediaThumbnail({ item, api, privacy, active }: { item: SoopSearchItem; api: SoopApi; privacy: boolean; active: boolean }) {
  const [url, setUrl] = useState("");
  const target = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let cancelled = false; let objectUrl = ""; setUrl("");
    let release: (() => void) | undefined;
    const load = () => { if (cancelled || release) return;
    if (active && !privacy && (item.thumbnailKey || item.kind === "video") && api.thumbnail) release = scheduleSoopImage(async () => {
      if (cancelled) return;
      try { const bytes = await api.thumbnail!(item.thumbnailKey ?? "", item.kind === "video" ? item.id : undefined); if (!cancelled) { objectUrl = URL.createObjectURL(new Blob([bytes], { type: "image/jpeg" })); setUrl(objectUrl); } } catch { /* A missing thumbnail never discards the search result. */ }
    });
    };
    const observer = !active || privacy || typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) { observer!.disconnect(); load(); } }, { rootMargin: '100px' });
    if (observer && target.current) observer.observe(target.current); else load();
    return () => { cancelled = true; release?.(); observer?.disconnect(); if (objectUrl) URL.revokeObjectURL(objectUrl); };
  }, [api, item.thumbnailKey, item.id, item.kind, item.channelId, privacy, active]);
  return <div ref={target} className="soop-result-image">{url && !privacy ? <img src={url} alt="" /> : <span aria-hidden="true">{privacy ? "" : item.kind === "channel" ? "○" : "▷"}</span>}</div>;
}

export function SoopDiscovery({ api, active, privacyMode, legacyFavoritesView = false, onLive, onFavoriteChannel, isChannelFavorite, onNotice, mediaApi = platformMediaApi, previewWidth = 320, vodApi }: {
  api: SoopApi; active: boolean; privacyMode: boolean; legacyFavoritesView?: boolean;
  onLive: (item: SoopSearchItem) => void; onFavoriteChannel: (item: SoopSearchItem) => void; isChannelFavorite: (id: string) => boolean;
  onNotice?: (message: string) => void; mediaApi?: PlatformMediaApi; previewWidth?: number; vodApi?: SoopVodApi;
}) {
  const [query, setQuery] = useState(""); const [kind, setKind] = useState<SoopSearchKind | "saved">(legacyFavoritesView ? "saved" : "all");
  const [home, setHome] = useState<SoopHomePage | null>(null), [homeLoading, setHomeLoading] = useState(false), [homeError, setHomeError] = useState("");
  const [homeRetry, setHomeRetry] = useState(0);
  const homeAttempted = useRef(false);
  const [savedDraft, setSavedDraft] = useState(""), [savedQuery, setSavedQuery] = useState("");
  const savedOnly = kind === "saved";
  useEffect(() => { if (legacyFavoritesView) setKind("saved"); }, [legacyFavoritesView]);
  const [history, setHistory] = useState<string[]>(() => { const v = read<unknown>(HISTORY, []); return Array.isArray(v) ? v.filter((s): s is string => typeof s === "string").slice(0, 10) : []; });
  const [historyOpen, setHistoryOpen] = useState(false);
  const [tabs, setTabs] = useState<Tab[]>([]); const tabsRef = useRef(tabs); tabsRef.current = tabs;
  // Query ids survive reopening; request identities must never be recycled with a closed tab.
  const requestGeneration = useRef(0);
  const [selectedTab, setSelectedTab] = useState(""); const [selected, setSelected] = useState<string[]>([]); const anchor = useRef<string | null>(null);
  const [favorites, setFavorites] = useState<SavedVideo[]>(initialFavorites);
  const [detail, setDetail] = useState<SoopSearchItem | null>(null);
  const [playingVod, setPlayingVod] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ item: SoopSearchItem; x: number; y: number } | null>(null);
  const [pending, setPending] = useState<string[]>([]); const [operationError, setOperationError] = useState(""); const mounted = useRef(true);
  const surface = useRef<HTMLElement>(null); const scrolls = useRef(new Map<string, number>());
  const scrollContext = useRef({ active: false, key: "" });
  scrollContext.current = { active: active && !privacyMode, key: savedOnly ? "saved-videos" : selectedTab };
  function rememberScroll(scrollKey = scrollContext.current.key) {
    if (!scrollContext.current.active || scrollContext.current.key !== scrollKey || !surface.current || surface.current.closest('[hidden], [inert], [aria-hidden="true"]')) return;
    const scroller = surface.current.closest<HTMLElement>(".soop-workspace");
    if (scroller) scrolls.current.set(scrollKey, scroller.scrollTop);
  }
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { setMenu(null); }, [active, selectedTab, savedOnly, privacyMode]);
  useEffect(() => { setSelected([]); anchor.current = null; }, [savedOnly]);
  const key = (item: SoopSearchItem) => `${item.section ?? ""}:${item.kind}:${item.id}`;
  const mediaKey = (item: SoopSearchItem) => `${item.kind}:${item.id}`;
  const currentTab = tabs.find(tab => tab.id === selectedTab);
  const showHome = !savedOnly && !currentTab;
  useEffect(() => {
    if (!active || privacyMode || !api.home || homeAttempted.current) return;
    homeAttempted.current = true;
    let cancelled = false, settled = false; setHomeLoading(true); setHomeError("");
    void api.home().then(value => { if (!cancelled) setHome(value); }).catch(error => { if (!cancelled) setHomeError(errorText(error)); }).finally(() => { settled = true; if (!cancelled) setHomeLoading(false); });
    return () => { cancelled = true; if (!settled) homeAttempted.current = false; };
  }, [api, active, privacyMode, homeRetry]);
  useLayoutEffect(() => {
    const scroller = surface.current?.closest<HTMLElement>(".soop-workspace");
    if (!scrollContext.current.active || !scroller || surface.current?.closest('[hidden], [inert], [aria-hidden="true"]')) return;
    const scrollKey = savedOnly ? "saved-videos" : selectedTab;
    scroller.scrollTop = scrolls.current.get(scrollKey) ?? 0;
    const remember = () => rememberScroll(scrollKey);
    scroller.addEventListener("scroll", remember, { passive: true });
    // The parent may already have hidden/clamped this shared scroller during
    // cleanup. Keep the last visible event, never sample that hidden position.
    return () => { scroller.removeEventListener("scroll", remember); };
  }, [active, privacyMode, selectedTab, savedOnly]);
  function switchTab(id: string) {
    rememberScroll();
    setSelectedTab(id); setSelected([]); anchor.current = null;
  }
  const savedNeedle = savedQuery.trim().toLocaleLowerCase();
  const items: SoopSearchItem[] = savedOnly ? favorites.filter(item => !savedNeedle || `${item.title} ${item.channelName}`.toLocaleLowerCase().includes(savedNeedle)).map(item => ({ ...item, thumbnailKey: item.thumbnailKey ?? null, views: null, restricted: item.restricted ?? true })) : currentTab?.items ?? home?.sections.flatMap(section => section.items) ?? [];
  const groups = showHome && home ? home.sections : currentTab?.request.kind === "all" && !savedOnly
    ? items.some(item => item.section)
      ? (Object.keys(sectionLabels) as (keyof typeof sectionLabels)[]).map(section => ({ title: sectionLabels[section], section, items: items.filter(item => item.section === section) })).filter(group => group.items.length)
      : (["channel", "live", "video"] as const).map(itemKind => ({ title: labels[itemKind], items: items.filter(item => item.kind === itemKind) })).filter(group => group.items.length)
    : [{ title: "", items }];
  const displayedItems = groups.flatMap(group => group.items);
  function storeHistory(next: string[]) { setHistory(next); try { localStorage.setItem(HISTORY, JSON.stringify(next)); } catch { /* Search history is optional. */ } }
  async function load(id: string, request: SoopSearchRequest, append: boolean) {
    if (!api.search) return;
    const generation = ++requestGeneration.current;
    setTabs(old => old.map(tab => tab.id === id ? { ...tab, loading: true, error: "", generation, requestedPage: request.page } : tab));
    try {
      const page = await api.search(request);
      if (!mounted.current) return;
      setTabs(old => old.map(tab => tab.id === id && tab.generation === generation ? { ...tab, items: append ? [...new Map([...tab.items, ...page.items].map(item => [key(item), item])).values()] : page.items, page: page.page, more: page.hasMore, warnings: append ? [...new Set([...(tab.warnings ?? []), ...(page.warnings ?? [])])] : page.warnings, loading: false } : tab));
    } catch (error) { if (mounted.current) setTabs(old => old.map(tab => tab.id === id && tab.generation === generation ? { ...tab, loading: false, error: errorText(error) } : tab)); }
  }
  function search(request: SoopSearchRequest, background = false) {
    if (!request.keyword.trim() && !request.channelId) { switchTab(""); return; }
    const id = JSON.stringify([request.kind, request.channelId ?? "", request.keyword.trim()]);
    const old = tabsRef.current.find(tab => tab.id === id);
    if (!background) { setKind(request.kind); switchTab(id); }
    setHistoryOpen(false);
    if (!request.channelId) storeHistory([request.keyword.trim(), ...history.filter(s => s !== request.keyword.trim())].slice(0, 10));
    if (old && (old.items.length || old.loading)) return;
    if (!old) setTabs(prev => [...prev, { id, request, title: `${request.keyword || request.channelId} · ${labels[request.kind]}`, items: [], page: 0, requestedPage: 1, more: false, loading: true, error: "", generation: 0 }]);
    void load(id, request, false);
  }
  function favorite(item: SoopSearchItem) {
    if (item.kind !== "video") { onFavoriteChannel(item); return; }
    const next = favorites.some(v => v.id === item.id) ? favorites.filter(v => v.id !== item.id) : [...favorites, { id: item.id, kind: item.kind, title: item.title, channelId: item.channelId, channelName: item.channelName, duration: item.duration, restricted: item.restricted, thumbnailKey: item.thumbnailKey }];
    try { localStorage.setItem(VIDEO_FAVORITES, JSON.stringify(next)); setFavorites(next); } catch { onNotice?.("즐겨찾기를 저장하지 못했습니다."); }
  }
  const saved = (item: SoopSearchItem) => item.kind === "video" ? favorites.some(v => v.id === item.id) : isChannelFavorite(item.channelId);
  async function download(targets: SoopSearchItem[]) {
    setOperationError("");
    const batch = [...new Map(targets.filter(item => item.kind === "video" && !item.restricted && !pending.includes(mediaKey(item))).map(item => [mediaKey(item), item])).values()];
    if (batch.length > 1 && mediaApi.downloadBatch) {
      const ids = batch.map(mediaKey); setPending(old => [...old, ...ids]);
      try {
        await mediaApi.downloadBatch(batch.map(item => ({ provider: "soop", kind: "video", externalId: item.id })), crypto.randomUUID());
        onNotice?.(`${batch.length}개 영상을 대기 큐에 추가했습니다.`);
      } catch (error) { if (mounted.current) setOperationError(errorText(error)); }
      finally { if (mounted.current) setPending(old => old.filter(id => !ids.includes(id))); }
      return;
    }
    for (const item of batch) {
      const id = mediaKey(item); if (pending.includes(id)) continue;
      setPending(old => [...old, id]);
      try { await mediaApi.download({ provider: "soop", kind: "video", externalId: item.id }, crypto.randomUUID()); onNotice?.("대기 큐에 추가했습니다."); }
      catch (error) { if (mounted.current) setOperationError(errorText(error)); }
      finally { if (mounted.current) setPending(old => old.filter(v => v !== id)); }
    }
  }
  function select(item: SoopSearchItem, event: MouseEvent<HTMLElement>) {
    const id = key(item);
    if (event.shiftKey && anchor.current) { const keys = displayedItems.map(key); const first = keys.indexOf(anchor.current); const last = keys.indexOf(id); if (first >= 0 && last >= 0) setSelected(keys.slice(Math.min(first, last), Math.max(first, last) + 1)); }
    else if (event.ctrlKey || event.metaKey) { setSelected(old => old.includes(id) ? old.filter(v => v !== id) : [...old, id]); anchor.current = id; }
    else { const deselect = selected.length === 1 && selected[0] === id; setSelected(deselect ? [] : [id]); anchor.current = deselect ? null : id; if (deselect) event.currentTarget.blur(); }
  }
  function creator(item: SoopSearchItem, background: boolean) { search({ kind: "video", keyword: item.channelName, channelId: item.channelId, page: 1 }, background); }
  function closeDetail() { setPlayingVod(null); setDetail(null); }
  function open(item: SoopSearchItem) { setSelected([]); anchor.current = null; if (item.kind === "channel") creator(item, false); else { if (detail?.id !== item.id || detail.kind !== item.kind) setPlayingVod(null); setDetail(item); } }
  function closeTab(id: string) { const next = tabs.filter(v => v.id !== id); setTabs(next); if (selectedTab === id) setSelectedTab(next.at(-1)?.id ?? ""); setSelected([]); }
  const visible = active && !privacyMode;
  useEffect(() => {
    if (!visible || (!menu && !detail)) return;
    const previous = document.activeElement as HTMLElement | null;
    const dialog = surface.current?.querySelector<HTMLElement>(menu ? ".soop-context-menu" : ".soop-detail");
    dialog?.querySelector<HTMLElement>("button:not(:disabled)")?.focus();
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        // A focused React splitter still belongs to the native fullscreen view.
        // Its owner exits fullscreen first; the next Escape may close detail.
        if (!menu && dialog?.querySelector('.soop-vod-pair[data-platform-native-fullscreen="true"]')) return;
        event.preventDefault(); event.stopPropagation(); setMenu(null); closeDetail();
      }
      if (event.key === "Tab" && dialog) {
        const controls = [...dialog.querySelectorAll<HTMLElement>("button:not(:disabled),a[href],input,select,[tabindex='0']")];
        const first = controls[0], last = controls.at(-1);
        if (first && (event.shiftKey ? document.activeElement === first : document.activeElement === last)) { event.preventDefault(); (event.shiftKey ? last : first)?.focus(); }
      }
    };
    window.addEventListener("keydown", keydown, true);
    return () => { window.removeEventListener("keydown", keydown, true); if (previous?.isConnected && active) previous.focus({ preventScroll: true }); };
  }, [visible, menu, detail, active]);
  return <section ref={surface} className="soop-discovery" aria-label="SOOP 탐색" style={{ "--soop-preview-width": `${[240, 320, 400].includes(previewWidth) ? previewWidth : 320}px` } as CSSProperties}>
      <form className="soop-search" data-tour="soop-search" onSubmit={event => { event.preventDefault(); if (kind === "saved") { setSavedQuery(savedDraft); setSelected([]); anchor.current = null; } else search({ kind, keyword: query, page: 1 }); }}>
        <div className="soop-search-input"><input aria-label="SOOP 검색어" placeholder={savedOnly ? "저장한 영상 검색" : "방송·영상·채널 검색"} value={privacyMode ? "" : savedOnly ? savedDraft : query} disabled={privacyMode} onChange={event => savedOnly ? setSavedDraft(event.target.value) : setQuery(event.target.value)} onFocus={() => setHistoryOpen(true)} onKeyDown={event => { if (event.key === "Escape") setHistoryOpen(false); }} />
          {!savedOnly && historyOpen && history.length && !privacyMode ? <div className="soop-search-history" onMouseLeave={() => setHistoryOpen(false)}>{history.map(item => <div key={item}><button type="button" onClick={() => { setQuery(item); search({ kind, keyword: item, page: 1 }); }}>{item}</button><button type="button" aria-label={`${item} 기록 삭제`} onClick={() => storeHistory(history.filter(v => v !== item))}>×</button></div>)}<button type="button" onClick={() => storeHistory([])}>기록 모두 지우기</button></div> : null}
        </div>
        <select aria-label="SOOP 검색 종류" value={kind} onChange={event => { setKind(event.target.value as SoopSearchKind | "saved"); setHistoryOpen(false); }}>{Object.entries(labels).map(([value, label]) => <option key={value} value={value}>{label}</option>)}<option value="saved">저장한 영상</option></select>
        <button type="submit" disabled={privacyMode}>검색</button>
      </form>
      <div className="soop-search-tabs" role="tablist" hidden={savedOnly}><div className={showHome ? "is-current" : ""}><button type="button" role="tab" aria-selected={showHome} onClick={() => switchTab("")}>홈</button></div>{tabs.map(tab => <div key={tab.id} className={tab.id === selectedTab ? "is-current" : ""}><button type="button" role="tab" aria-selected={tab.id === selectedTab} onClick={() => switchTab(tab.id)}>{privacyMode ? "탐색" : tab.title}{tab.loading ? <span className="soop-spinner" aria-label="불러오는 중" /> : null}</button><button type="button" aria-label="탐색 닫기" onClick={() => closeTab(tab.id)}>×</button></div>)}</div>
    {showHome && !privacyMode ? <><div className="soop-home-status"><span>{homeLoading ? <span className="soop-spinner" aria-label="메인 불러오는 중" /> : null}</span><button type="button" aria-label="메인 새로고침" disabled={homeLoading} onClick={() => { homeAttempted.current = false; setHomeRetry(value => value + 1); }}>↻</button></div>{homeError ? <p role="alert">{homeError}</p> : home?.notice ? <p className="soop-feed-notice">{home.notice}</p> : null}</> : null}
    {operationError && !privacyMode ? <p role="alert">{operationError}</p> : null}
    {currentTab?.error && !savedOnly && !privacyMode ? <div role="alert">{currentTab.error}<button type="button" onClick={() => void load(currentTab.id, { ...currentTab.request, page: currentTab.requestedPage }, currentTab.requestedPage > 1)}>다시 시도</button></div> : null}
    {!savedOnly && !privacyMode ? currentTab?.warnings?.map(warning => <p role="alert" key={warning}>{warning}</p>) : null}
    {selected.length > 1 ? <div className="soop-multi-bar"><span>{selected.length}개 선택</span><button type="button" onClick={() => void download(displayedItems.filter(v => selected.includes(key(v))))} disabled={!displayedItems.some(v => selected.includes(key(v)) && v.kind === "video" && !v.restricted)}>다운로드</button><button type="button" onClick={() => { setSelected([]); anchor.current = null; (document.activeElement as HTMLElement | null)?.blur(); }}>선택 해제</button></div> : null}
    {groups.map((group, groupIndex) => <section className="soop-result-section" data-section={"section" in group ? String(group.section) : undefined} key={`${group.title}:${groupIndex}`}>{group.title && !privacyMode ? <h2>{group.title}</h2> : null}<div className="soop-results">{group.items.map(item => <article key={key(item)} data-kind={item.kind} className={`soop-result${selected.includes(key(item)) ? " is-focused" : ""}`} tabIndex={visible ? 0 : -1} aria-label={privacyMode ? "콘텐츠" : item.title}
      onClick={event => { if (visible) select(item, event); }} onDoubleClick={() => visible && open(item)} onContextMenu={event => { event.preventDefault(); if (visible) setMenu({ item, x: Math.min(event.clientX, innerWidth - 210), y: Math.min(event.clientY, innerHeight - 210) }); }}
      onKeyDown={event => { if (!visible || event.target !== event.currentTarget) return; if (event.key === "Enter") { event.preventDefault(); if (event.ctrlKey) void download(selected.includes(key(item)) && selected.length > 1 ? displayedItems.filter(value => selected.includes(key(value))) : [item]); else open(item); } }}>
      <div className="soop-card-preview"><Thumbnail item={item} api={api} privacy={privacyMode} active={active} />
        {!privacyMode && item.kind === "live" && item.views != null && Number.isFinite(item.views) && item.views >= 0 ? <span className="soop-viewer-badge"><i aria-hidden="true" />시청 {item.views.toLocaleString()}명</span> : null}
        {!privacyMode && item.kind === "video" && item.duration ? <span className="soop-duration-badge">{item.duration}</span> : null}
      </div>
      {selected.length > 1 ? <span className="soop-result-check" aria-label={selected.includes(key(item)) ? "선택됨" : "선택 안 됨"}>{selected.includes(key(item)) ? "✓" : ""}</span> : null}
      <button type="button" className={`soop-result-favorite${saved(item) ? " is-saved" : ""}`} aria-label={saved(item) ? "즐겨찾기 해제" : "즐겨찾기 추가"} aria-pressed={saved(item)} disabled={privacyMode} onClick={event => { event.stopPropagation(); favorite(item); }} onDoubleClick={event => event.stopPropagation()} onContextMenu={event => event.stopPropagation()}>{saved(item) ? "★" : "☆"}</button>
      <div className="soop-result-copy">
        {item.kind !== "channel" ? <SoopPortrait api={api} channelId={item.channelId} active={active} privacy={privacyMode} /> : null}
        <div className="soop-result-text">
        {item.kind === "channel" && item.section !== "streamers" ? <strong title={privacyMode ? "" : item.title}>{privacyMode ? "콘텐츠를 숨겼습니다." : splitGalleryTitle(item.title).primary}</strong> : null}
        <button type="button" disabled={privacyMode} onClick={event => { event.stopPropagation(); creator(item, event.ctrlKey || event.metaKey); }} onDoubleClick={event => event.stopPropagation()} onContextMenu={event => event.stopPropagation()}>{privacyMode ? "" : item.kind === "channel" && item.section !== "streamers" ? `@${item.channelId}` : item.channelName}</button>
        {item.kind !== "channel" ? <strong title={privacyMode ? "" : item.title}>{privacyMode ? "콘텐츠를 숨겼습니다." : splitGalleryTitle(item.title).primary}</strong> : null}
        <small>{privacyMode ? "" : item.kind === "channel" ? item.favoriteCount != null ? `즐겨찾기 ${item.favoriteCount.toLocaleString()}명` : "채널" : `${item.section === "catch" ? "Catch" : labels[item.kind]}${item.kind === "video" ? audienceLabel(item) : ""}${item.restricted ? " · 이용 제한" : ""}`}</small>
        {!privacyMode && item.tags?.length ? <div className="soop-card-tags">{item.tags.slice(0, 8).map((tag, index) => <span key={`${tag}:${index}`}>{tag}</span>)}</div> : null}</div>
      </div>
    </article>)}</div></section>)}
    {!items.length && (savedOnly || (!currentTab?.loading && !homeLoading)) ? <p className="soop-discovery-empty">{privacyMode ? "프라이버시 모드" : savedOnly ? savedQuery ? "검색 결과가 없습니다." : "영상의 별표를 눌러 저장하세요." : !currentTab ? homeError ? "" : "메인 콘텐츠를 불러오세요." : currentTab.error ? "" : "검색 결과가 없습니다."}</p> : null}
    {!savedOnly && currentTab?.more ? <button type="button" className="soop-load-more" disabled={currentTab.loading} onClick={() => void load(currentTab.id, { ...currentTab.request, page: currentTab.page + 1 }, true)}>{currentTab.loading ? <span className="soop-spinner" aria-label="불러오는 중" /> : "더 보기"}</button> : null}
    {menu && visible ? <><div className="soop-context-scrim" onPointerDown={() => setMenu(null)} /><div className="soop-context-menu" role="menu" style={{ left: menu.x, top: menu.y }} onKeyDown={event => { if (event.key === "Escape") setMenu(null); }}>
      <button role="menuitem" onClick={() => { open(menu.item); setMenu(null); }}>열기</button><button role="menuitem" onClick={() => { favorite(menu.item); setMenu(null); }}>{saved(menu.item) ? "즐겨찾기 해제" : "즐겨찾기 추가"}</button>
      {menu.item.kind === "video" ? <button role="menuitem" disabled={menu.item.restricted} onClick={() => { void download([menu.item]); setMenu(null); }}>다운로드</button> : null}<button role="menuitem" onClick={() => { creator(menu.item, false); setMenu(null); }}>채널 영상</button>
    </div></> : null}
    {detail ? <div className="soop-detail-shade" hidden={!visible} inert={!visible} onClick={closeDetail}>
      <section className={`soop-detail${playingVod === detail.id && detail.kind === "video" ? " is-playing-vod" : ""}`} role="dialog" aria-modal="true" aria-label="영상 상세" onClick={event => event.stopPropagation()} onKeyDown={event => { if (event.key === "Escape") closeDetail(); }}>
        <button type="button" className="soop-detail-close" aria-label="상세 닫기" data-platform-native-occlusion="true" onClick={closeDetail}>×</button>
        {detail.kind === "video" && playingVod === detail.id ? <div className="soop-detail-vod"><SoopVodPlayer key={detail.id} videoId={detail.id} title={detail.title} active={active} privacyMode={privacyMode} api={vodApi} /></div> : <div className="soop-detail-preview"><Thumbnail item={detail} api={api} privacy={privacyMode} active={active} />{detail.kind === "video" ? <button type="button" className="soop-detail-play" aria-label="원본 영상 재생" disabled={!visible} onClick={() => { if (visible) setPlayingVod(detail.id); }}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m8 5 11 7-11 7Z" /></svg><span>재생</span></button> : null}</div>}
        <h2>{privacyMode ? "영상" : detail.title}</h2><p>{privacyMode ? "" : `${detail.channelName}${detail.duration ? ` · ${detail.duration}` : ""}`}</p>
        <div className="soop-detail-actions"><button type="button" disabled={!visible} onClick={() => favorite(detail)} aria-pressed={saved(detail)}>{saved(detail) ? "★" : "☆"} 즐겨찾기</button>{detail.kind === "live" ? <button type="button" disabled={!visible} onClick={() => { onLive(detail); closeDetail(); }}>시청</button> : <button type="button" disabled={!visible || detail.restricted || pending.includes(mediaKey(detail))} onClick={() => void download([detail])}>{pending.includes(mediaKey(detail)) ? <span className="soop-spinner" aria-label="대기 큐에 추가 중" /> : "다운로드"}</button>}<button type="button" disabled={!visible} onClick={() => { creator(detail, false); closeDetail(); }}>채널 영상</button></div>
        {detail.restricted && !privacyMode ? <p>로그인 또는 이용 권한이 필요한 콘텐츠입니다.</p> : null}
      </section>
    </div> : null}
  </section>;
}
