import { useEffect, useLayoutEffect, useRef, useState, type MouseEvent } from "react";
import type { SoopApi, SoopSearchItem, SoopSearchKind, SoopSearchRequest } from "../../api/soop";
import { platformMediaApi, type PlatformMediaApi } from "../platforms/platformMedia";
import "./SoopDiscovery.css";
import { splitGalleryTitle } from "../../components/galleryCardLayout";

type Tab = { id: string; request: SoopSearchRequest; title: string; items: SoopSearchItem[]; page: number; requestedPage: number; more: boolean; loading: boolean; error: string; generation: number };
type SavedVideo = Pick<SoopSearchItem, "id" | "kind" | "title" | "channelId" | "channelName" | "duration" | "restricted" | "thumbnailKey">;
const VIDEO_FAVORITES = "atsumi:soop:video-favorites:v1";
const HISTORY = "atsumi:soop:search-history:v1";
const labels = { live: "라이브", video: "영상", channel: "채널" };
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

// A small bounded image queue leaves native slots available to user searches.
let imageWorkers = 0;
const imageQueue: (() => void)[] = [];
function scheduleImage(task: () => Promise<void>) { const start = () => { imageWorkers++; void task().finally(() => { imageWorkers--; imageQueue.shift()?.(); }); }; if (imageWorkers < 2) start(); else imageQueue.push(start); }
function Thumbnail({ item, api, privacy }: { item: SoopSearchItem; api: SoopApi; privacy: boolean }) {
  const [url, setUrl] = useState("");
  useEffect(() => {
    let cancelled = false; let objectUrl = ""; setUrl("");
    if (!privacy && (item.thumbnailKey || item.kind === "video") && api.thumbnail) scheduleImage(async () => {
      if (cancelled) return;
      try { const bytes = await api.thumbnail!(item.thumbnailKey ?? "", item.kind === "video" ? item.id : undefined); if (!cancelled) { objectUrl = URL.createObjectURL(new Blob([bytes], { type: "image/jpeg" })); setUrl(objectUrl); } } catch { /* A missing thumbnail never discards the search result. */ }
    });
    return () => { cancelled = true; if (objectUrl) URL.revokeObjectURL(objectUrl); };
  }, [api, item.thumbnailKey, item.id, item.kind, privacy]);
  return <div className="soop-result-image">{url && !privacy ? <img src={url} alt="" /> : <span aria-hidden="true">{privacy ? "" : item.kind === "channel" ? "○" : "▷"}</span>}</div>;
}

export function SoopDiscovery({ api, active, privacyMode, legacyFavoritesView = false, onLive, onFavoriteChannel, isChannelFavorite, onNotice, mediaApi = platformMediaApi }: {
  api: SoopApi; active: boolean; privacyMode: boolean; legacyFavoritesView?: boolean;
  onLive: (item: SoopSearchItem) => void; onFavoriteChannel: (item: SoopSearchItem) => void; isChannelFavorite: (id: string) => boolean;
  onNotice?: (message: string) => void; mediaApi?: PlatformMediaApi;
}) {
  const [query, setQuery] = useState(""); const [kind, setKind] = useState<SoopSearchKind | "saved">(legacyFavoritesView ? "saved" : "live");
  const [savedDraft, setSavedDraft] = useState(""), [savedQuery, setSavedQuery] = useState("");
  const savedOnly = kind === "saved";
  useEffect(() => { if (legacyFavoritesView) setKind("saved"); }, [legacyFavoritesView]);
  const [history, setHistory] = useState<string[]>(() => { const v = read<unknown>(HISTORY, []); return Array.isArray(v) ? v.filter((s): s is string => typeof s === "string").slice(0, 10) : []; });
  const [historyOpen, setHistoryOpen] = useState(false);
  const [tabs, setTabs] = useState<Tab[]>([]); const tabsRef = useRef(tabs); tabsRef.current = tabs;
  const [selectedTab, setSelectedTab] = useState(""); const [selected, setSelected] = useState<string[]>([]); const anchor = useRef<string | null>(null);
  const [favorites, setFavorites] = useState<SavedVideo[]>(initialFavorites);
  const [detail, setDetail] = useState<SoopSearchItem | null>(null);
  const [menu, setMenu] = useState<{ item: SoopSearchItem; x: number; y: number } | null>(null);
  const [pending, setPending] = useState<string[]>([]); const [operationError, setOperationError] = useState(""); const mounted = useRef(true);
  const surface = useRef<HTMLElement>(null); const scrolls = useRef(new Map<string, number>());
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { setMenu(null); }, [active, selectedTab, savedOnly, privacyMode]);
  useEffect(() => { setSelected([]); anchor.current = null; setDetail(null); }, [savedOnly]);
  const key = (item: SoopSearchItem) => `${item.kind}:${item.id}`;
  const currentTab = tabs.find(tab => tab.id === selectedTab);
  useLayoutEffect(() => {
    const scroller = surface.current?.closest<HTMLElement>(".soop-workspace");
    if (!scroller) return;
    const scrollKey = savedOnly ? "saved-videos" : selectedTab;
    scroller.scrollTop = scrolls.current.get(scrollKey) ?? 0;
    const remember = () => scrolls.current.set(scrollKey, scroller.scrollTop);
    scroller.addEventListener("scroll", remember, { passive: true });
    return () => { remember(); scroller.removeEventListener("scroll", remember); };
  }, [selectedTab, savedOnly]);
  function switchTab(id: string) {
    const scroller = surface.current?.closest<HTMLElement>(".soop-workspace");
    if (scroller) scrolls.current.set(savedOnly ? "saved-videos" : selectedTab, scroller.scrollTop);
    setSelectedTab(id); setSelected([]); anchor.current = null;
  }
  const savedNeedle = savedQuery.trim().toLocaleLowerCase();
  const items: SoopSearchItem[] = savedOnly ? favorites.filter(item => !savedNeedle || `${item.title} ${item.channelName}`.toLocaleLowerCase().includes(savedNeedle)).map(item => ({ ...item, thumbnailKey: item.thumbnailKey ?? null, views: null, restricted: item.restricted ?? true })) : currentTab?.items ?? [];
  function storeHistory(next: string[]) { setHistory(next); try { localStorage.setItem(HISTORY, JSON.stringify(next)); } catch { /* Search history is optional. */ } }
  async function load(id: string, request: SoopSearchRequest, append: boolean, generation: number) {
    if (!api.search) return;
    setTabs(old => old.map(tab => tab.id === id ? { ...tab, loading: true, error: "", generation, requestedPage: request.page } : tab));
    try {
      const page = await api.search(request);
      if (!mounted.current) return;
      setTabs(old => old.map(tab => tab.id === id && tab.generation === generation ? { ...tab, items: append ? [...new Map([...tab.items, ...page.items].map(item => [key(item), item])).values()] : page.items, page: page.page, more: page.hasMore, loading: false } : tab));
    } catch (error) { if (mounted.current) setTabs(old => old.map(tab => tab.id === id && tab.generation === generation ? { ...tab, loading: false, error: errorText(error) } : tab)); }
  }
  function search(request: SoopSearchRequest, background = false) {
    if (!request.keyword.trim() && !request.channelId) return;
    const id = JSON.stringify([request.kind, request.channelId ?? "", request.keyword.trim()]);
    const old = tabsRef.current.find(tab => tab.id === id);
    if (!background) { setKind(request.kind); switchTab(id); }
    setHistoryOpen(false);
    if (!request.channelId) storeHistory([request.keyword.trim(), ...history.filter(s => s !== request.keyword.trim())].slice(0, 10));
    if (old && (old.items.length || old.loading)) return;
    const generation = (old?.generation ?? 0) + 1;
    if (!old) setTabs(prev => [...prev, { id, request, title: `${request.keyword || request.channelId} · ${labels[request.kind]}`, items: [], page: 0, requestedPage: 1, more: false, loading: true, error: "", generation }]);
    void load(id, request, false, generation);
  }
  function favorite(item: SoopSearchItem) {
    if (item.kind !== "video") { onFavoriteChannel(item); return; }
    const next = favorites.some(v => v.id === item.id) ? favorites.filter(v => v.id !== item.id) : [...favorites, { id: item.id, kind: item.kind, title: item.title, channelId: item.channelId, channelName: item.channelName, duration: item.duration, restricted: item.restricted, thumbnailKey: item.thumbnailKey }];
    try { localStorage.setItem(VIDEO_FAVORITES, JSON.stringify(next)); setFavorites(next); } catch { onNotice?.("즐겨찾기를 저장하지 못했습니다."); }
  }
  const saved = (item: SoopSearchItem) => item.kind === "video" ? favorites.some(v => v.id === item.id) : isChannelFavorite(item.channelId);
  async function download(targets: SoopSearchItem[]) {
    setOperationError("");
    const batch = targets.filter(item => item.kind === "video" && !item.restricted && !pending.includes(key(item)));
    if (batch.length > 1 && mediaApi.downloadBatch) {
      const ids = batch.map(key); setPending(old => [...old, ...ids]);
      try {
        await mediaApi.downloadBatch(batch.map(item => ({ provider: "soop", kind: "video", externalId: item.id })), crypto.randomUUID());
        onNotice?.(`${batch.length}개 영상을 대기 큐에 추가했습니다.`);
      } catch (error) { if (mounted.current) setOperationError(errorText(error)); }
      finally { if (mounted.current) setPending(old => old.filter(id => !ids.includes(id))); }
      return;
    }
    for (const item of targets.filter(v => v.kind === "video" && !v.restricted)) {
      const id = key(item); if (pending.includes(id)) continue;
      setPending(old => [...old, id]);
      try { await mediaApi.download({ provider: "soop", kind: "video", externalId: item.id }, crypto.randomUUID()); onNotice?.("대기 큐에 추가했습니다."); }
      catch (error) { if (mounted.current) setOperationError(errorText(error)); }
      finally { if (mounted.current) setPending(old => old.filter(v => v !== id)); }
    }
  }
  function select(item: SoopSearchItem, event: MouseEvent<HTMLElement>) {
    const id = key(item);
    if (event.shiftKey && anchor.current) { const keys = items.map(key); const first = keys.indexOf(anchor.current); const last = keys.indexOf(id); if (first >= 0 && last >= 0) setSelected(keys.slice(Math.min(first, last), Math.max(first, last) + 1)); }
    else if (event.ctrlKey || event.metaKey) { setSelected(old => old.includes(id) ? old.filter(v => v !== id) : [...old, id]); anchor.current = id; }
    else { const deselect = selected.length === 1 && selected[0] === id; setSelected(deselect ? [] : [id]); anchor.current = deselect ? null : id; if (deselect) event.currentTarget.blur(); }
  }
  function creator(item: SoopSearchItem, background: boolean) { search({ kind: "video", keyword: item.channelName, channelId: item.channelId, page: 1 }, background); }
  function open(item: SoopSearchItem) { setSelected([]); anchor.current = null; if (item.kind === "channel") creator(item, false); else setDetail(item); }
  function closeTab(id: string) { const next = tabs.filter(v => v.id !== id); setTabs(next); if (selectedTab === id) setSelectedTab(next.at(-1)?.id ?? ""); setSelected([]); }
  const visible = active && !privacyMode;
  useEffect(() => {
    if (!visible || (!menu && !detail)) return;
    const previous = document.activeElement as HTMLElement | null;
    const dialog = surface.current?.querySelector<HTMLElement>(menu ? ".soop-context-menu" : ".soop-detail");
    dialog?.querySelector<HTMLElement>("button:not(:disabled)")?.focus();
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setMenu(null); setDetail(null); }
      if (event.key === "Tab" && dialog) {
        const controls = [...dialog.querySelectorAll<HTMLElement>("button:not(:disabled),a[href],input,select,[tabindex='0']")];
        const first = controls[0], last = controls.at(-1);
        if (first && (event.shiftKey ? document.activeElement === first : document.activeElement === last)) { event.preventDefault(); (event.shiftKey ? last : first)?.focus(); }
      }
    };
    window.addEventListener("keydown", keydown, true);
    return () => { window.removeEventListener("keydown", keydown, true); if (previous?.isConnected && active) previous.focus({ preventScroll: true }); };
  }, [visible, menu, detail, active]);
  return <section ref={surface} className="soop-discovery" aria-label="SOOP 탐색">
      <form className="soop-search" data-tour="soop-search" onSubmit={event => { event.preventDefault(); if (kind === "saved") { setSavedQuery(savedDraft); setSelected([]); anchor.current = null; } else search({ kind, keyword: query, page: 1 }); }}>
        <div className="soop-search-input"><input aria-label="SOOP 검색어" placeholder={savedOnly ? "저장한 영상 검색" : "방송·영상·채널 검색"} value={savedOnly ? savedDraft : query} disabled={privacyMode} onChange={event => savedOnly ? setSavedDraft(event.target.value) : setQuery(event.target.value)} onFocus={() => setHistoryOpen(true)} onKeyDown={event => { if (event.key === "Escape") setHistoryOpen(false); }} />
          {!savedOnly && historyOpen && history.length && !privacyMode ? <div className="soop-search-history" onMouseLeave={() => setHistoryOpen(false)}>{history.map(item => <div key={item}><button type="button" onClick={() => { setQuery(item); search({ kind, keyword: item, page: 1 }); }}>{item}</button><button type="button" aria-label={`${item} 기록 삭제`} onClick={() => storeHistory(history.filter(v => v !== item))}>×</button></div>)}<button type="button" onClick={() => storeHistory([])}>기록 모두 지우기</button></div> : null}
        </div>
        <select aria-label="SOOP 검색 종류" value={kind} onChange={event => { setKind(event.target.value as SoopSearchKind | "saved"); setHistoryOpen(false); }}>{Object.entries(labels).map(([value, label]) => <option key={value} value={value}>{label}</option>)}<option value="saved">저장한 영상</option></select>
        <button type="submit" disabled={privacyMode || (!savedOnly && !query.trim())}>검색</button>
      </form>
      <div className="soop-search-tabs" role="tablist" hidden={savedOnly}>{tabs.map(tab => <div key={tab.id} className={tab.id === selectedTab ? "is-current" : ""}><button type="button" role="tab" aria-selected={tab.id === selectedTab} onClick={() => switchTab(tab.id)}>{privacyMode ? "탐색" : tab.title}{tab.loading ? <span className="soop-spinner" aria-label="불러오는 중" /> : null}</button><button type="button" aria-label="탐색 닫기" onClick={() => closeTab(tab.id)}>×</button></div>)}</div>
    {operationError && !privacyMode ? <p role="alert">{operationError}</p> : null}
    {currentTab?.error && !savedOnly ? <div role="alert">{currentTab.error}<button type="button" onClick={() => void load(currentTab.id, { ...currentTab.request, page: currentTab.requestedPage }, currentTab.requestedPage > 1, currentTab.generation + 1)}>다시 시도</button></div> : null}
    {selected.length > 1 ? <div className="soop-multi-bar"><span>{selected.length}개 선택</span><button type="button" onClick={() => void download(items.filter(v => selected.includes(key(v))))} disabled={!items.some(v => selected.includes(key(v)) && v.kind === "video" && !v.restricted)}>다운로드</button><button type="button" onClick={() => { setSelected([]); anchor.current = null; (document.activeElement as HTMLElement | null)?.blur(); }}>선택 해제</button></div> : null}
    <div className="soop-results">{items.map(item => <article key={key(item)} className={`soop-result${selected.includes(key(item)) ? " is-focused" : ""}`} tabIndex={visible ? 0 : -1} aria-label={privacyMode ? "콘텐츠" : item.title}
      onClick={event => select(item, event)} onDoubleClick={() => visible && open(item)} onContextMenu={event => { event.preventDefault(); if (visible) setMenu({ item, x: Math.min(event.clientX, innerWidth - 210), y: Math.min(event.clientY, innerHeight - 210) }); }}
      onKeyDown={event => { if (!visible || event.target !== event.currentTarget) return; if (event.key === "Enter") { event.preventDefault(); if (event.ctrlKey) void download(selected.includes(key(item)) && selected.length > 1 ? items.filter(value => selected.includes(key(value))) : [item]); else open(item); } }}>
      <Thumbnail item={item} api={api} privacy={privacyMode} />
      {selected.length > 1 ? <span className="soop-result-check" aria-label={selected.includes(key(item)) ? "선택됨" : "선택 안 됨"}>{selected.includes(key(item)) ? "✓" : ""}</span> : null}
      <button type="button" className={`soop-result-favorite${saved(item) ? " is-saved" : ""}`} aria-label={saved(item) ? "즐겨찾기 해제" : "즐겨찾기 추가"} aria-pressed={saved(item)} disabled={privacyMode} onClick={event => { event.stopPropagation(); favorite(item); }} onDoubleClick={event => event.stopPropagation()} onContextMenu={event => event.stopPropagation()}>{saved(item) ? "★" : "☆"}</button>
      <div className="soop-result-copy"><strong title={privacyMode ? "" : item.title}>{privacyMode ? "콘텐츠를 숨겼습니다." : splitGalleryTitle(item.title).primary}</strong><button type="button" disabled={privacyMode} onClick={event => { event.stopPropagation(); creator(item, event.ctrlKey || event.metaKey); }} onDoubleClick={event => event.stopPropagation()} onContextMenu={event => event.stopPropagation()}>{privacyMode ? "" : item.channelName}</button><small>{privacyMode ? "" : `${labels[item.kind]}${item.duration ? ` · ${item.duration}` : ""}${audienceLabel(item)}${item.restricted ? " · 이용 제한" : ""}`}</small></div>
    </article>)}</div>
    {!items.length && (savedOnly || !currentTab?.loading) ? <p className="soop-discovery-empty">{privacyMode ? "프라이버시 모드" : savedOnly ? savedQuery ? "검색 결과가 없습니다." : "영상의 별표를 눌러 저장하세요." : !currentTab ? "검색어를 입력해 방송과 영상을 찾아보세요." : currentTab.error ? "" : "검색 결과가 없습니다."}</p> : null}
    {!savedOnly && currentTab?.more ? <button type="button" className="soop-load-more" disabled={currentTab.loading} onClick={() => void load(currentTab.id, { ...currentTab.request, page: currentTab.page + 1 }, true, currentTab.generation + 1)}>{currentTab.loading ? <span className="soop-spinner" aria-label="불러오는 중" /> : "더 보기"}</button> : null}
    {menu && visible ? <><div className="soop-context-scrim" onPointerDown={() => setMenu(null)} /><div className="soop-context-menu" role="menu" style={{ left: menu.x, top: menu.y }} onKeyDown={event => { if (event.key === "Escape") setMenu(null); }}>
      <button role="menuitem" onClick={() => { open(menu.item); setMenu(null); }}>열기</button><button role="menuitem" onClick={() => { favorite(menu.item); setMenu(null); }}>{saved(menu.item) ? "즐겨찾기 해제" : "즐겨찾기 추가"}</button>
      {menu.item.kind === "video" ? <button role="menuitem" disabled={menu.item.restricted} onClick={() => { void download([menu.item]); setMenu(null); }}>다운로드</button> : null}<button role="menuitem" onClick={() => { creator(menu.item, false); setMenu(null); }}>채널 영상</button>
    </div></> : null}
    {detail && visible ? <div className="soop-detail-shade" onClick={() => setDetail(null)}><section className="soop-detail" role="dialog" aria-label="영상 상세" onClick={event => event.stopPropagation()} onKeyDown={event => { if (event.key === "Escape") setDetail(null); }}><button className="soop-detail-close" aria-label="상세 닫기" onClick={() => setDetail(null)}>×</button><Thumbnail item={detail} api={api} privacy={false} /><h2>{detail.title}</h2><p>{detail.channelName}{detail.duration ? ` · ${detail.duration}` : ""}</p><div className="soop-detail-actions"><button onClick={() => favorite(detail)} aria-pressed={saved(detail)}>{saved(detail) ? "★" : "☆"} 즐겨찾기</button>{detail.kind === "live" ? <button onClick={() => { onLive(detail); setDetail(null); }}>시청</button> : <button disabled={detail.restricted || pending.includes(key(detail))} onClick={() => void download([detail])}>{pending.includes(key(detail)) ? <span className="soop-spinner" aria-label="대기 큐에 추가 중" /> : "다운로드"}</button>}<button onClick={() => { creator(detail, false); setDetail(null); }}>채널 영상</button></div>{detail.restricted ? <p>로그인 또는 이용 권한이 필요한 콘텐츠입니다.</p> : null}</section></div> : null}
  </section>;
}
