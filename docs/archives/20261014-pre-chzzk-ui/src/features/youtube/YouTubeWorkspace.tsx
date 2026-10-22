import { useEffect, useLayoutEffect, useMemo, useRef, useState, type MouseEvent } from 'react';
import { createYouTubeApi, type YouTubeApi, type YouTubeSearchItem, type YouTubeSearchRequest } from './api';
import { parseYouTubeTarget, readYouTubeFavorites, saveYouTubeFavorites, targetKey, type YouTubeFavorite, type YouTubeMetadata } from './model';
import { YouTubePlayer } from './YouTubePlayer';
import { YouTubeViewingControls, type YouTubeViewingMode } from './YouTubeViewingControls';
import { splitGalleryTitle } from '../../components/galleryCardLayout';
import { PlatformLivePlayer } from '../platforms/PlatformLivePlayer';
import { PlatformMediaLibrary } from '../platforms/PlatformMediaLibrary';
import { PlatformScreenshotLibrary } from '../platforms/PlatformScreenshotLibrary';
import { platformMediaApi, type PlatformMediaApi } from '../platforms/platformMedia';
import './YouTubeWorkspace.css';
import './YouTubeMultiView.css';

export type YouTubeWorkspaceProps = { active: boolean; runtime: 'tauri' | 'browser-mock'; privacyMode: boolean; view: 'explore' | 'live' | 'favorites' | 'screenshots' | 'downloads'; onViewChange?: (view: 'explore' | 'live' | 'favorites' | 'screenshots' | 'downloads') => void; onNotice?: (message: string) => void; api?: YouTubeApi; mediaApi?: PlatformMediaApi; onWatchingChange?: (watching: boolean) => void };
type SearchTab = { id: string; label: string; request: YouTubeSearchRequest; items: YouTubeSearchItem[]; cursor?: string | null; loading: boolean; error?: string; scroll: number };
const errorText = (error: unknown) => error instanceof Error ? error.message : typeof error === 'string' ? error : '요청을 마치지 못했습니다.';
const titleInList = (value: string) => splitGalleryTitle(value).primary;
function Star({ filled }: { filled: boolean }) { return <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true"><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9Z" fill={filled ? 'currentColor' : 'none'} stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" /></svg>; }
const thumbnailCache = new Map<string, string | null>(), thumbnailPending = new Map<string, Promise<string | null>>();
let thumbnailRunning = 0; const thumbnailQueue: Array<() => void> = [];
function thumbnail(api: YouTubeApi, id: string): Promise<string | null> {
  if (thumbnailCache.has(id)) return Promise.resolve(thumbnailCache.get(id) ?? null);
  const pending = thumbnailPending.get(id); if (pending) return pending;
  const request = new Promise<string | null>(resolve => {
    const execute = () => { thumbnailRunning++; void (api.thumbnail?.(id) ?? Promise.resolve(null)).catch(() => null).then(result => {
      if (thumbnailCache.size >= 128) thumbnailCache.delete(thumbnailCache.keys().next().value!); thumbnailCache.set(id, result); resolve(result);
    }).finally(() => { thumbnailPending.delete(id); thumbnailRunning--; thumbnailQueue.shift()?.(); }); };
    if (thumbnailRunning < 4) execute(); else thumbnailQueue.push(execute);
  }); thumbnailPending.set(id, request); return request;
}
function Cover({ item, api, privacy }: { item: YouTubeMetadata; api: YouTubeApi; privacy: boolean }) {
  const [source, setSource] = useState(item.thumbnail), root = useRef<HTMLDivElement>(null);
  useEffect(() => { setSource(item.thumbnail); if (privacy || item.kind !== 'video' || item.thumbnail) return; let gone = false; const load = () => void thumbnail(api, item.id).then(value => { if (!gone && value) setSource(value); });
    if (typeof IntersectionObserver === 'undefined') load(); else { const observer = new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) { observer.disconnect(); load(); } }, { rootMargin: '200px' }); if (root.current) observer.observe(root.current); return () => { gone = true; observer.disconnect(); }; }
    return () => { gone = true; };
  }, [item.id, item.kind, item.thumbnail, api, privacy]);
  return <div className="youtube-card-cover" ref={root}>{source && !privacy ? <img src={source} alt="" /> : <span aria-hidden="true">{privacy ? '' : item.kind === 'channel' ? '◉' : '▶'}</span>}</div>;
}

export function YouTubeWorkspace({ active, runtime, privacyMode, view, onViewChange, onNotice, api: providedApi, mediaApi = platformMediaApi, onWatchingChange }: YouTubeWorkspaceProps) {
  const api = useMemo(() => providedApi ?? createYouTubeApi(runtime), [providedApi, runtime]);
  const [input, setInput] = useState(''), [filter, setFilter] = useState<YouTubeSearchRequest['filter']>('all'), [sort, setSort] = useState<YouTubeSearchRequest['sort']>('relevance');
  const [tabs, setTabs] = useState<SearchTab[]>([]), [tabId, setTabId] = useState(''), [history, setHistory] = useState<string[]>([]), [showHistory, setShowHistory] = useState(false);
  const [favorites, setFavorites] = useState<YouTubeFavorite[]>(() => { try { return readYouTubeFavorites(localStorage); } catch { return []; } });
  const [category, setCategory] = useState<'video' | 'channel'>('video'), [selected, setSelected] = useState<string[]>([]), anchor = useRef<string | null>(null);
  const [details, setDetails] = useState<YouTubeMetadata[]>([]), [detailId, setDetailId] = useState(''), [playingIds, setPlayingIds] = useState<string[]>([]);
  const [playbackNotice, setPlaybackNotice] = useState('');
  const [viewingControlsOpen, setViewingControlsOpen] = useState(false), [viewingMode, setViewingMode] = useState<YouTubeViewingMode>('all'), [viewingLead, setViewingLead] = useState('');
  const [menu, setMenu] = useState<{ item: YouTubeMetadata; x: number; y: number } | null>(null), [error, setError] = useState(''), [busy, setBusy] = useState(false);
  const listRef = useRef<HTMLDivElement>(null), alive = useRef(true), favoriteScroll = useRef({ video: 0, channel: 0 });
  const workspaceRef = useRef<HTMLElement>(null), menuRef = useRef<HTMLDivElement>(null), detailRef = useRef<HTMLDivElement>(null);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const current = tabs.find(tab => tab.id === tabId), detail = details.find(item => targetKey(item) === detailId), playing = details.filter(item => playingIds.includes(targetKey(item)));
  const isWatching = playing.length > 0;
  const effectiveLead = playingIds.includes(viewingLead) ? viewingLead : playing[0] ? targetKey(playing[0]) : '';
  const selectedViewing = view === 'live' && viewingMode === 'selected';
  const viewingCellCount = playing.filter(item => !selectedViewing || item.isLive || targetKey(item) === effectiveLead).length;
  useEffect(() => { if (!active || view !== 'live') setViewingControlsOpen(false); }, [active, view]);
  useEffect(() => { onWatchingChange?.(isWatching); }, [isWatching, onWatchingChange]);
  useEffect(() => () => { onWatchingChange?.(false); }, [onWatchingChange]);
  const showFavorites = view === 'favorites';
  const archiveView = view === 'downloads' || view === 'screenshots';
  const list = showFavorites ? favorites.filter(item => category === 'channel' ? item.kind === 'channel' : item.kind !== 'channel') : current?.items ?? [];
  useEffect(() => { setShowHistory(false); setSelected([]); setMenu(null); }, [view, category]);
  useEffect(() => { setSelected([]); setMenu(null); }, [tabId]);
  useLayoutEffect(() => {
    if (active && !archiveView && view !== 'live' && listRef.current) listRef.current.scrollTop = showFavorites ? favoriteScroll.current[category] : current?.scroll ?? 0;
  }, [active, view, tabId, showFavorites, category]);
  const updateTab = (id: string, update: Partial<SearchTab>) => { if (alive.current) setTabs(old => old.map(tab => tab.id === id ? { ...tab, ...update } : tab)); };
  const search = async (request: YouTubeSearchRequest, label: string, background = false) => {
    const match = tabs.find(tab => JSON.stringify(tab.request) === JSON.stringify(request));
    if (!background && view === 'favorites') onViewChange?.('explore');
    if (match && !match.error) { if (!background) setTabId(match.id); return; }
    const id = match?.id ?? crypto.randomUUID(); if (match) updateTab(id, { loading: true, error: undefined }); else setTabs(old => [...old, { id, label, request, items: [], loading: true, scroll: 0 }]); if (!background) setTabId(id); setError('');
    try { if (!api.search) throw new Error('검색 기능을 사용할 수 없습니다.'); const result = await api.search(request); updateTab(id, { items: result.items, cursor: result.cursor, loading: false }); }
    catch (cause) { updateTab(id, { error: errorText(cause), loading: false }); }
  };
  const open = (item: YouTubeMetadata, background = false) => {
    setMenu(null);
    if (!background) setSelected([]);
    if (item.kind === 'channel') { void search({ query: '', channel: item.url, filter: 'all', sort: 'relevance' }, item.title, background); return; }
    const key = targetKey(item); setDetails(old => old.some(value => targetKey(value) === key) ? old : [...old, item]); if (!background) setDetailId(key); setMenu(null);
  };
  const submit = async (value = input) => {
    setShowHistory(false); if (!value.trim()) return;
    setHistory(old => [value.trim(), ...old.filter(item => item !== value.trim())].slice(0, 10));
    const target = parseYouTubeTarget(value);
    if (target?.kind === 'channel') { await search({ query: '', channel: target.url, filter: 'all', sort: 'relevance' }, target.id); return; }
    if (target) { setBusy(true); try { const item = await api.metadata(target); open(item); } catch (cause) { setError(errorText(cause)); } finally { setBusy(false); } return; }
    await search({ query: value.trim(), filter, sort }, value.trim());
  };
  const favorite = (item: YouTubeMetadata) => { setFavorites(old => { const exists = old.some(value => targetKey(value) === targetKey(item)); if (!exists && old.length >= 100) { setError('즐겨찾기는 100개까지 저장할 수 있습니다. 기존 항목을 해제한 뒤 추가해 주세요.'); return old; } const next = exists ? old.filter(value => targetKey(value) !== targetKey(item)) : [{ ...item, savedAt: Date.now() }, ...old]; try { saveYouTubeFavorites(localStorage, next); return next; } catch { setError('즐겨찾기를 저장하지 못했습니다.'); return old; } }); };
  const download = async (items: YouTubeMetadata[]) => {
    setBusy(true); setMenu(null); try { const videos = items.filter(item => item.kind === 'video'); if (!videos.length) throw new Error('저장할 영상을 선택해 주세요.'); if (videos.some(item => item.isLive)) throw new Error('라이브를 재생한 뒤 녹화 버튼을 눌러 주세요.');
      if (mediaApi.downloadBatch) await mediaApi.downloadBatch(videos.map(item => ({ provider: 'youtube', kind: 'video', externalId: item.id })), crypto.randomUUID());
      else for (const item of videos) await mediaApi.download({ provider: 'youtube', kind: 'video', externalId: item.id }, crypto.randomUUID());
      onNotice?.('다운로드를 대기 큐에 추가했습니다.');
    } catch (cause) { setError(errorText(cause)); } finally { setBusy(false); }
  };
  const external = async (item: YouTubeMetadata) => { try { await api.openExternal(item.url); } catch (cause) { setError(errorText(cause)); } };
  const select = (item: YouTubeMetadata, event: MouseEvent) => { const key = targetKey(item); if (event.shiftKey && anchor.current) { const a = list.findIndex(item => targetKey(item) === anchor.current), b = list.findIndex(item => targetKey(item) === key); setSelected(a < 0 || b < 0 ? [key] : list.slice(Math.min(a, b), Math.max(a, b) + 1).map(targetKey)); } else if (event.ctrlKey || event.metaKey) { if (selected.includes(key)) (event.currentTarget as HTMLElement).blur(); setSelected(old => old.includes(key) ? old.filter(value => value !== key) : [...old, key]); } else if (selected.length === 1 && selected[0] === key) { setSelected([]); (event.currentTarget as HTMLElement).blur(); } else setSelected([key]); anchor.current = key; };
  const closeDetail = (key: string) => { setDetails(old => old.filter(item => targetKey(item) !== key)); setPlayingIds(old => old.filter(id => id !== key)); setDetailId(currentId => currentId === key ? details.find(item => targetKey(item) !== key) ? targetKey(details.find(item => targetKey(item) !== key)!) : '' : currentId); };
  const startPlayback = (key: string) => {
    if (privacyMode || playingIds.includes(key)) return;
    if (playingIds.length >= 4) { setPlaybackNotice('동시에 네 개까지 재생할 수 있습니다. 시청 중인 영상 하나를 종료해 주세요.'); return; }
    setPlaybackNotice('');
    setPlayingIds(old => old.includes(key) || old.length >= 4 ? old : [...old, key]);
  };
  const togglePlayback = (key: string) => { if (playingIds.includes(key)) { setPlayingIds(old => old.filter(id => id !== key)); setPlaybackNotice(''); } else startPlayback(key); };
  useEffect(() => { setPlaybackNotice(''); }, [detailId]);
  useEffect(() => { if (!active || archiveView || view === 'live') { setMenu(null); return; }
    const key = (event: KeyboardEvent) => {
      if (event.defaultPrevented || document.fullscreenElement) return;
      const top = [...document.querySelectorAll<HTMLElement>('[role="dialog"][aria-modal="true"]')].filter(node => !node.closest('[hidden]')).at(-1);
      if (top && top !== detailRef.current) return;
      const overlay = menu ? menuRef.current : detailId ? detailRef.current : null;
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); if (menu) setMenu(null); else if (detailId) closeDetail(detailId); else { setSelected([]); if (document.activeElement instanceof HTMLElement && workspaceRef.current?.contains(document.activeElement)) document.activeElement.blur(); } return; }
      if (overlay) { const elements = [...overlay.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled),iframe,[tabindex="0"]')].filter(node => !node.closest('[hidden]')); if (event.key === 'Tab' && elements.length) { const current = elements.indexOf(document.activeElement as HTMLElement); if (event.shiftKey && current <= 0) { event.preventDefault(); elements.at(-1)?.focus(); } else if (!event.shiftKey && (current < 0 || current === elements.length - 1)) { event.preventDefault(); elements[0]?.focus(); } } else if (menu && ['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) { event.preventDefault(); const current = elements.indexOf(document.activeElement as HTMLElement); elements[event.key === 'Home' ? 0 : event.key === 'End' ? elements.length - 1 : (current + (event.key === 'ArrowDown' ? 1 : -1) + elements.length) % elements.length]?.focus(); } return; }
      if (event.target instanceof Element && event.target.closest('input,textarea,select,button')) return;
      if (event.key === 'Enter' && selected.length) { event.preventDefault(); const selectedItems = list.filter(item => selected.includes(targetKey(item))); if (event.ctrlKey) void download(selectedItems); else selectedItems.forEach((item, index) => open(item, index !== 0)); }
    }; window.addEventListener('keydown', key); return () => window.removeEventListener('keydown', key);
  }, [active, view, selected, list, detailId, menu]);
  useEffect(() => { if (!active || archiveView || view === 'live') return; const overlay = menu ? menuRef.current : detailId ? detailRef.current : null; if (!overlay) return; const previous = document.activeElement; overlay.querySelector<HTMLElement>('button:not(:disabled)')?.focus(); return () => { if (previous instanceof HTMLElement && previous.isConnected && !previous.closest('[hidden]')) previous.focus(); }; }, [active, view, !!menu, detailId]);
  return <div className="youtube-surface">
    <section ref={workspaceRef} className="youtube-workspace" hidden={!active || archiveView || view === 'live'} aria-label="YouTube" data-tour="youtube-content">
      <h1>{view === 'favorites' ? '즐겨찾기' : '탐색'}</h1>
      {view === 'explore' ? <form className="youtube-search" data-tour="youtube-input" onSubmit={event => { event.preventDefault(); void submit(); }}>
        <div className="youtube-search-entry"><input aria-label="YouTube 검색" placeholder="검색어 또는 YouTube 주소" value={input} onChange={event => setInput(event.target.value)} onFocus={() => setShowHistory(true)} /><button type="button" aria-label="검색어 지우기" onClick={() => setInput('')}>×</button>
          {showHistory && history.length ? <div className="youtube-history">{history.map(value => <div key={value}><button type="button" onClick={() => { setInput(value); void submit(value); }}>{value}</button><button type="button" aria-label={`${value} 기록 삭제`} onClick={() => setHistory(old => old.filter(item => item !== value))}>×</button></div>)}<button type="button" onClick={() => setHistory([])}>기록 모두 지우기</button></div> : null}
        </div><select aria-label="검색 종류" value={filter} onChange={event => { setFilter(event.target.value as typeof filter); if (event.target.value !== 'all') setSort('relevance'); }}><option value="all">전체</option><option value="video">영상</option><option value="channel">채널</option><option value="live">라이브</option></select><select aria-label="정렬" value={sort} onChange={event => { setSort(event.target.value as typeof sort); if (event.target.value === 'popular') setFilter('all'); }}><option value="relevance">관련성</option><option value="popular">인기순</option></select><button type="submit" disabled={busy}>{busy ? <span className="youtube-spinner" /> : '검색'}</button>
      </form> : null}
      {showFavorites ? <div className="youtube-categories" role="group" aria-label="즐겨찾기 종류"><button aria-pressed={category === 'video'} onClick={() => setCategory('video')}>영상</button><button aria-pressed={category === 'channel'} onClick={() => setCategory('channel')}>채널</button></div> : null}
      {!showFavorites ? <div className="youtube-search-tabs">{tabs.map(tab => <div key={tab.id} className={tab.id === tabId ? 'is-active' : ''}><button onClick={() => setTabId(tab.id)}>{tab.loading ? <span className="youtube-spinner" /> : null}{privacyMode ? '탐색' : tab.label}</button><button aria-label="검색 탭 닫기" onClick={() => { setTabs(old => old.filter(value => value.id !== tab.id)); if (tabId === tab.id) setTabId(tabs.find(value => value.id !== tab.id)?.id ?? ''); }}>×</button></div>)}</div> : null}
      {selected.length > 1 ? <div className="youtube-selection"><span>{selected.length}개 선택</span><button disabled={busy} onClick={() => void download(list.filter(item => selected.includes(targetKey(item))))}>다운로드</button><button onClick={() => list.filter(item => selected.includes(targetKey(item))).forEach((item, index) => open(item, index !== 0))}>열기</button><button aria-label="선택 해제" onClick={() => { setSelected([]); (document.activeElement as HTMLElement)?.blur(); }}>×</button></div> : null}
      {error || (!showFavorites && current?.error) ? <p role="alert" className="youtube-error">{error || current?.error}</p> : null}
      <div ref={listRef} className="youtube-results" onScroll={event => { if (!active || archiveView || view === 'live') return; if (showFavorites) favoriteScroll.current[category] = event.currentTarget.scrollTop; else if (current) updateTab(current.id, { scroll: event.currentTarget.scrollTop }); }}>
      {list.length ? <div className="youtube-card-grid">{list.map(item => { const key = targetKey(item), saved = favorites.some(value => targetKey(value) === key); return <article key={key} tabIndex={0} className={`youtube-card${selected.includes(key) ? ' is-focused' : ''}`} onClick={event => select(item, event)} onDoubleClick={() => open(item)} onContextMenu={event => { event.preventDefault(); setSelected([key]); setMenu({ item, x: Math.min(event.clientX, window.innerWidth - 215), y: Math.min(event.clientY, window.innerHeight - 220) }); }}>
        <Cover item={item} api={api} privacy={privacyMode} />{selected.length > 1 ? <span className="youtube-card-check">{selected.includes(key) ? '✓' : ''}</span> : null}
        <button className="youtube-favorite" aria-label={saved ? '즐겨찾기 해제' : '즐겨찾기'} aria-pressed={saved} onClick={event => { event.stopPropagation(); favorite(item); }} onDoubleClick={event => event.stopPropagation()}><Star filled={saved} /></button>
        <div className="youtube-card-text"><strong>{privacyMode ? '프라이버시 모드' : titleInList(item.title)}</strong>{item.authorUrl ? <button className="youtube-author" onClick={event => { event.stopPropagation(); const target = parseYouTubeTarget(item.authorUrl!); if (target) open({ ...target, title: item.author, author: '' }, event.ctrlKey); }} onContextMenu={event => { event.preventDefault(); event.stopPropagation(); const target = parseYouTubeTarget(item.authorUrl!); if (target) setMenu({ item: { ...target, title: item.author, author: '' }, x: Math.max(8, Math.min(event.clientX, window.innerWidth - 215)), y: Math.max(8, Math.min(event.clientY, window.innerHeight - 200)) }); }} onDoubleClick={event => event.stopPropagation()}>{privacyMode ? '' : item.author}</button> : <span>{privacyMode ? '' : item.author || (item.kind === 'channel' ? '채널' : '')}</span>}<small>{privacyMode ? '' : [(item as YouTubeSearchItem).isLive ? '라이브' : (item as YouTubeSearchItem).duration, (item as YouTubeSearchItem).views].filter(Boolean).join(' · ')}</small></div>
      </article>; })}</div> : current?.loading && !showFavorites ? <p role="status" className="youtube-empty"><span className="youtube-spinner" /></p> : <p className="youtube-empty">{showFavorites ? '즐겨찾기한 항목이 없습니다.' : current ? '검색 결과가 없습니다.' : '검색어로 영상이나 채널을 찾아보세요.'}</p>}
      {current?.cursor && !showFavorites ? <button className="youtube-more" disabled={current.loading} onClick={() => { const tab = current; updateTab(tab.id, { loading: true, error: undefined }); void api.search?.({ ...tab.request, cursor: tab.cursor! }).then(result => updateTab(tab.id, { items: [...tab.items, ...result.items.filter(item => !tab.items.some(value => targetKey(value) === targetKey(item)))], cursor: result.cursor, loading: false })).catch(cause => updateTab(tab.id, { loading: false, error: errorText(cause) })); }}>{current.loading ? <span className="youtube-spinner" /> : '더 보기'}</button> : null}
      </div>
      {menu ? <><div className="platform-media-dismiss" onClick={() => setMenu(null)} /><div ref={menuRef} role="menu" className="platform-media-menu" style={{ left: menu.x, top: menu.y }}><button role="menuitem" onClick={() => open(menu.item)}>열기</button><button role="menuitem" onClick={() => { favorite(menu.item); setMenu(null); }}>즐겨찾기</button>{menu.item.kind === 'video' ? <button role="menuitem" disabled={busy} onClick={() => void download([menu.item])}>다운로드</button> : null}<button role="menuitem" onClick={() => { void external(menu.item); setMenu(null); }}>YouTube에서 열기</button></div></> : null}
    </section>
    <PlatformMediaLibrary provider="youtube" active={active && view === 'downloads'} privacyMode={privacyMode} api={mediaApi} onNotice={onNotice} />
    <PlatformScreenshotLibrary provider="youtube" active={active && view === 'screenshots'} privacyMode={privacyMode} onNotice={onNotice} />
    {view === 'live' && !playing.length ? <section className="youtube-workspace youtube-viewing-empty" hidden={!active} aria-label="YouTube 시청"><div><span aria-hidden="true">▷</span><h2>{privacyMode ? '프라이버시 모드' : '영상을 시청해 보세요'}</h2><p>{privacyMode ? '프라이버시 모드를 끈 뒤 아래 손잡이를 눌러주세요.' : '아래 손잡이를 누르거나 끌어올려 시청할 영상을 고르세요.'}</p><button type="button" onClick={() => setViewingControlsOpen(true)}>방송 보기 열기</button></div><svg viewBox="0 0 24 64" aria-hidden="true"><path d="M12 2v56m-7-7 7 7 7-7" /></svg></section> : null}
    {details.length ? <div className={`youtube-detail-backdrop${view === 'live' ? ' youtube-viewing' : ''}`} hidden={!active || archiveView || (view === 'live' ? !playing.length : !detailId)}>
      <div ref={detailRef} role={view === 'live' ? 'region' : 'dialog'} aria-modal={view !== 'live' ? active && !!detailId && !archiveView : undefined} aria-label={view === 'live' ? 'YouTube 시청' : '영상 상세'} className="youtube-detail">
        {view !== 'live' ? <header><div className="youtube-detail-tabs">{details.map(item => <div key={targetKey(item)} className={targetKey(item) === detailId ? 'is-active' : ''}><button onClick={() => setDetailId(targetKey(item))}>{privacyMode ? '영상' : item.title}</button><button aria-label="상세 탭 닫기" onClick={() => closeDetail(targetKey(item))}>×</button></div>)}</div><button className="youtube-detail-minimize" aria-label="상세 내리기" onClick={() => setDetailId('')}>−</button></header> : null}
        {detail && view !== 'live' ? <div className="youtube-detail-actions"><strong>{privacyMode ? '프라이버시 모드' : detail.title}</strong><button aria-label="즐겨찾기" aria-pressed={favorites.some(item => targetKey(item) === detailId)} onClick={() => favorite(detail)}><Star filled={favorites.some(item => targetKey(item) === detailId)} /></button><button disabled={busy || detail.kind !== 'video' || !!(detail as YouTubeSearchItem).isLive} onClick={() => void download([detail])}>다운로드</button><button disabled={privacyMode} onClick={() => togglePlayback(detailId)}>{playingIds.includes(detailId) ? '시청 종료' : '재생'}</button></div> : null}
        {detail?.warning && view !== 'live' ? <p className="youtube-error" role="status">{detail.warning}</p> : null}
        {playbackNotice && view !== 'live' ? <p className="youtube-detail-notice youtube-error" role="alert">{playbackNotice}</p> : null}
        {playing.length ? <div className={`youtube-playing-grid${playing.length > 1 ? ' is-multiple' : ''}`} data-count={viewingCellCount} data-mode={selectedViewing ? 'selected' : 'all'} hidden={view !== 'live' && !playingIds.includes(detailId)}>{playing.map(item => {
          const key = targetKey(item), videoVisible = !selectedViewing || key === effectiveLead, backgroundVod = !item.isLive && !videoVisible;
          return <div className="youtube-playing-cell" key={key} data-video-visible={videoVisible} data-background-vod={backgroundVod} aria-hidden={backgroundVod || undefined} inert={backgroundVod || undefined}>{item.isLive ? <PlatformLivePlayer reference={{ provider: 'youtube', kind: 'live', externalId: item.id }} active={active && !archiveView && (view === 'live' || (!!detailId && playingIds.includes(detailId)))} videoVisible={videoVisible} privacyMode={privacyMode} title={item.title} /> : <YouTubePlayer item={item} privacyMode={privacyMode} onExternal={() => void external(item)} />}</div>;
        })}</div> : null}
        {detail && !playingIds.includes(detailId) && view !== 'live' ? <div className="youtube-detail-preview"><Cover item={detail} api={api} privacy={privacyMode} /><button type="button" className="youtube-preview-play" aria-label="영상 재생" title="영상 재생" disabled={privacyMode} onClick={() => startPlayback(detailId)}><svg viewBox="0 0 32 32" aria-hidden="true" focusable="false"><path d="M11 7 25 16 11 25Z" /></svg></button></div> : null}
      </div>
    </div> : null}
    {details.length && !detailId && active && !archiveView && view !== 'live' ? <button className="youtube-restore" onClick={() => setDetailId(playingIds[0] || targetKey(details[0]!))}>열린 영상 {details.length}</button> : null}
    <YouTubeViewingControls active={active && view === 'live'} open={viewingControlsOpen} privacyMode={privacyMode} busy={busy} noticeError={error} items={details} playingIds={playingIds} favoriteIds={favorites.map(targetKey)} mode={viewingMode} lead={effectiveLead} onOpenChange={setViewingControlsOpen}
      onApply={(ids, mode, lead) => { setPlayingIds(ids); setViewingMode(mode); setViewingLead(lead); setError(''); }} onStop={() => { setPlayingIds([]); setViewingLead(''); }} onExplore={() => onViewChange?.('explore')} onCloseItem={closeDetail} onFavorite={favorite} onDownload={item => void download([item])} />
  </div>;
}
