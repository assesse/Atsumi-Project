import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { listen } from '@tauri-apps/api/event';
import { splitGalleryTitle } from '../../components/galleryCardLayout';
import { platformMediaApi, type MediaJob, type MediaLibraryItem, type MediaPlayback, type MediaProvider, type MediaRecordingGroup, type PlatformMediaApi } from './platformMedia';
import { PlatformMediaJobs } from './PlatformMediaJobs';
import { capturePresentation } from './mediaCaptureOutcome';
import { subscribeCardProfile } from './currentProfile';
import { WindowMotion } from '../../components/WindowMotion';
import { FluentIcon } from '../../components/FluentIcon';
import { PlatformReplayChat } from './PlatformReplayChat';
import { PlatformReplayPlayer } from './PlatformReplayPlayer';
import { emptyMediaSelection, retainMediaSelection, selectMedia } from './mediaSelection';
import { emptyMediaGroups, groupPanelPosition, readMediaGroups, readMediaMembers, readOpenMediaGroup, saveOpenMediaGroup, type MediaGroupSnapshot, type MediaMemberSnapshot } from './mediaGroupPaging';
import './PlatformMediaLibrary.css';

export interface PlatformMediaLibraryProps { provider: MediaProvider; active: boolean; privacyMode: boolean; api?: PlatformMediaApi; onNotice?: (message: string) => void }
const message = (error: unknown) => error instanceof Error ? error.message : typeof error === 'string' ? error : '작업을 마치지 못했습니다.';
const time = (seconds: number) => { const s = Math.max(0, Math.floor(seconds || 0)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };
const displayTitle = (title: string) => splitGalleryTitle(title).primary;
const size = (bytes: number) => bytes >= 1e9 ? `${(bytes / 1e9).toFixed(2)} GB` : `${(bytes / 1e6).toFixed(bytes < 1e6 ? 2 : 1)} MB`;
const savedDate = (atSeconds: number) => new Date(atSeconds * 1000).toLocaleString('ko-KR', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false });
function ChatStorageWarning({ item }: { item: MediaLibraryItem }) {
  const status = item.chatArchive;
  if (!status) return null;
  const failure = status.reasons.includes('archive_limit') ? '채팅 저장 한도에 도달했습니다.'
    : status.reasons.includes('write_failed') ? '일부 채팅을 저장하지 못했습니다.'
    : status.reasons.includes('flush_failed') ? '채팅의 최종 저장을 확인하지 못했습니다.'
    : status.reasons.includes('interrupted') ? '채팅 기록이 중단되었습니다.' : null;
  // The native first-reason latch can retain status_unknown after a later failure.
  // Uncertainty is still a warning, never proof of complete or unavailable chat.
  const unverified = status.state !== 'complete' || status.reasons.length > 0;
  if (!unverified && !status.assetsIncomplete) return null;
  const label = failure ? '채팅 일부 누락' : unverified ? '채팅 상태 확인 필요' : '이모티콘 일부 누락';
  const reason = [failure ?? (unverified ? '이 녹화의 채팅 보존 상태를 확인하지 못했습니다.' : ''), status.assetsIncomplete ? '일부 이모티콘은 문자로 표시됩니다.' : ''].filter(Boolean).join(' ');
  return <span className="platform-media-chat-warning" title={`${reason}${status.originalsRetained ? ' 원본 보존.' : ''}`} aria-label={`${label}${status.originalsRetained ? ' · 원본 보존' : ''}`}> · {label}</span>;
}
function CaptureState({ item }: { item: MediaLibraryItem }) {
  const status = capturePresentation(item);
  return <span className={`platform-media-saved-state${status ? ` is-${status.tone}` : ''}`} title={status?.detail}>{status?.label ?? '저장됨'}</span>;
}
function SavedChannel({ item, active, privacyMode, api }: { item: MediaLibraryItem; active: boolean; privacyMode: boolean; api: PlatformMediaApi }) {
  const [image, setImage] = useState<{ id: string; source: string; current: boolean } | null>(null), ref = useRef<HTMLDivElement>(null);
  const source = image?.id === item.libraryId ? image.source : null;
  useEffect(() => {
    setImage(null);
    if (!active || privacyMode || !api.profile) return;
    let gone = false, release: (() => void) | undefined;
    const load = () => { if (!gone && !release) release = subscribeCardProfile(api, item.libraryId, value => {
      if (!gone && value) setImage({ id: item.libraryId, source: value.image, current: value.current });
    }); };
    const observer = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) { observer!.disconnect(); load(); }
    }, { rootMargin: '100px' });
    if (observer && ref.current) observer.observe(ref.current); else load();
    return () => { gone = true; release?.(); observer?.disconnect(); };
  }, [item.libraryId, active, privacyMode, api]);
  return <div ref={ref} className="platform-media-channel"><span className="platform-media-channel-avatar" title={!privacyMode && source && image?.current ? '현재 채널 사진 · 녹화 당시 사진 아님' : undefined} aria-hidden="true">{!privacyMode && source ? <img src={source} alt="" onError={() => setImage(null)} /> : privacyMode ? '' : Array.from(item.channelName)[0] || '◯'}</span><span>{privacyMode ? '' : item.channelName}</span></div>;
}

type GroupReadIntent = 'poll' | 'refresh' | 'moreGroups' | 'members' | 'moreMembers';
type GroupView = { provider: MediaProvider; groups: MediaGroupSnapshot; members: MediaMemberSnapshot | null };
export function PlatformMediaLibrary({ provider, active, privacyMode, api = platformMediaApi, onNotice }: PlatformMediaLibraryProps) {
  const groupPanelId = useId();
  const [jobs, setJobs] = useState<MediaJob[]>([]), [flatItems, setItems] = useState<MediaLibraryItem[]>([]);
  const grouped = !!api.groups && !!api.groupMembers;
  const [groupView, setGroupView] = useState<GroupView>(() => ({ provider, groups: emptyMediaGroups(), members: null }));
  const groupViewRef = useRef(groupView);
  const commitGroupView = (next: GroupView) => { groupViewRef.current = next; setGroupView(next); };
  const [openChoice, setOpenChoice] = useState(() => ({ provider, key: readOpenMediaGroup(provider) }));
  const openKey = openChoice.provider === provider ? openChoice.key : readOpenMediaGroup(provider);
  const openGroupRef = useRef({ provider, key: openKey }); openGroupRef.current = { provider, key: openKey };
  const [deletedIds, setDeletedIds] = useState<Set<string>>(() => new Set());
  const groups = groupView.provider === provider ? groupView.groups : emptyMediaGroups();
  const currentGroup = groups.entries.find((entry): entry is MediaRecordingGroup => entry.type === 'recordingGroup' && entry.key === openKey);
  const members = currentGroup && groupView.members?.group.key === openKey ? groupView.members : null;
  const items = useMemo(() => grouped ? groups.entries.flatMap(entry => entry.type === 'video' ? [entry.item] : entry.key === openKey && members ? members.items : []).filter(item => !deletedIds.has(item.libraryId)) : flatItems,
    [grouped, groups.entries, openKey, members, deletedIds, flatItems]);
  const [groupReading, setGroupReading] = useState(false), [memberError, setMemberError] = useState('');
  const groupFlight = useRef(false), pendingGroupRead = useRef<GroupReadIntent | null>(null), groupLifetime = useRef(0), expansionEpoch = useRef(0);
  const groupReadRef = useRef<(intent: GroupReadIntent) => Promise<void>>(async () => undefined);
  const groupPanel = useRef<HTMLDivElement>(null), groupHeadings = useRef(new Map<string, HTMLButtonElement>());
  const [panelStyle, setPanelStyle] = useState<ReturnType<typeof groupPanelPosition> | null>(null);
  const [loaded, setLoaded] = useState(false), [error, setError] = useState(''), [refreshError, setRefreshError] = useState(''), [busy, setBusy] = useState(false);
  const refreshEpoch = useRef(0), mutationActive = useRef(false);
  const [cursor, setCursor] = useState<string | null>(null), [selection, setSelection] = useState(emptyMediaSelection), pagesLoaded = useRef(1), openRequest = useRef(0);
  const visibleIds = useMemo(() => items.map(item => item.libraryId), [items]);
  const visibleSelection = useMemo(() => retainMediaSelection(selection, visibleIds), [selection, visibleIds]);
  const selected = visibleSelection.ids;
  const setSelected = (update: string[] | ((previous: string[]) => string[])) => setSelection(previous => {
    const current = retainMediaSelection(previous, visibleIds);
    return { ...current, ids: typeof update === 'function' ? update(current.ids) : update };
  });
  useEffect(() => { setSelection(previous => retainMediaSelection(previous, visibleIds)); }, [visibleIds]);
  const [confirm, setConfirm] = useState(false), [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const [playback, setPlayback] = useState<MediaPlayback | null>(null), playbackRef = useRef<MediaPlayback | null>(null);
  const video = useRef<HTMLVideoElement>(null), surface = useRef<HTMLDivElement>(null), [paused, setPaused] = useState(true), [ended, setEnded] = useState(false);
  const [position, setPosition] = useState(0), [duration, setDuration] = useState(0), [volume, setVolume] = useState(1), [trayMuted, setTrayMuted] = useState(false);
  const [muted, setMuted] = useState(false), [playbackError, setPlaybackError] = useState('');
  const [controls, setControls] = useState(true), hideTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const library = useRef<HTMLElement>(null), contextMenu = useRef<HTMLDivElement>(null), confirmation = useRef<HTMLDivElement>(null);
  const chatApi = useMemo(() => ({ page: (id: string, ms: number, cursor: string | null, limit: number) => api.replayChat ? api.replayChat(id, ms, cursor ?? undefined, limit) : Promise.reject(new Error('채팅 읽기를 지원하지 않습니다.')), asset: api.replayChatAsset }), [api]);
  const hasChat = playback?.item.reference.kind === 'live' || playback?.item.chatArchive != null;
  useEffect(() => { playbackRef.current = playback; }, [playback]);
  useEffect(() => () => { openRequest.current++; refreshEpoch.current++; if (playbackRef.current) void api.close(playbackRef.current.token).catch(() => undefined); if (hideTimer.current) clearTimeout(hideTimer.current); }, [api]);
  useEffect(() => { let gone = false; let unlisten: (() => void) | undefined; void listen<boolean>('platform-media:tray-muted', event => setTrayMuted(event.payload)).then(fn => { if (gone) fn(); else unlisten = fn; }).catch(() => undefined); return () => { gone = true; unlisten?.(); }; }, []);
  useEffect(() => { if (video.current) { video.current.muted = privacyMode || trayMuted || muted; video.current.volume = volume; } }, [privacyMode, trayMuted, muted, volume, playback]);
  useEffect(() => { if (privacyMode && video.current) { video.current.pause(); setPaused(true); } }, [privacyMode, playback?.token]);
  const chooseGroup = (key: string | null) => {
    expansionEpoch.current++;
    const previousKey = openGroupRef.current.key;
    openGroupRef.current = { provider, key };
    setOpenChoice({ provider, key }); saveOpenMediaGroup(provider, key);
    setMemberError(''); setPanelStyle(null);
    commitGroupView({ ...groupViewRef.current, members: null });
    if (key) void groupReadRef.current('members');
    else if (previousKey) groupHeadings.current.get(previousKey)?.focus({ preventScroll: true });
  };
  groupReadRef.current = async intent => {
    if (!active || !grouped || !api.groups || !api.groupMembers) return;
    if (mutationActive.current) return;
    // Coalesce explicit user intents only. Polls never queue or recursively retry.
    if (groupFlight.current) { if (intent !== 'poll') pendingGroupRead.current = intent; return; }
    groupFlight.current = true; setGroupReading(true);
    const lifetime = groupLifetime.current, epoch = refreshEpoch.current, expansion = expansionEpoch.current;
    const current = () => lifetime === groupLifetime.current && epoch === refreshEpoch.current;
    try {
      if (intent !== 'members' && intent !== 'moreMembers') {
        const previous = groupViewRef.current.groups;
        const result = await readMediaGroups(api.groups, provider, previous, previous.pages + (intent === 'moreGroups' ? 1 : 0), current);
        if (!current()) return;
        if (result.status === 'changed') setRefreshError('목록이 변경되어 갱신을 기다리고 있습니다. 다시 확인해 주세요.');
        else if (result.status === 'ready') {
          commitGroupView({ ...groupViewRef.current, groups: result.snapshot }); setRefreshError(''); setLoaded(true);
        }
        try { const nextJobs = await api.jobs(provider); if (current()) setJobs(nextJobs); }
        catch (cause) { if (current()) setRefreshError(message(cause)); }
      }
      if (!current() || expansion !== expansionEpoch.current) return;
      const key = openGroupRef.current.provider === provider ? openGroupRef.current.key : null;
      const entry = groupViewRef.current.groups.entries.find(value => value.type === 'recordingGroup' && value.key === key);
      if (!key || !entry) { setDeletedIds(new Set()); return; }
      const memberCurrent = () => current() && expansion === expansionEpoch.current && openGroupRef.current.provider === provider && openGroupRef.current.key === key;
      const prior = groupViewRef.current.members;
      const pages = (prior?.group.key === key ? prior.pages : 1) + (intent === 'moreMembers' ? 1 : 0);
      try {
        const result = await readMediaMembers(api.groupMembers, provider, key, pages, memberCurrent);
        if (!memberCurrent()) return;
        if (result.status === 'ready') {
          // The fresh summary and every loaded member page enter the UI together.
          const previous = groupViewRef.current;
          commitGroupView({ ...previous, groups: { ...previous.groups, entries: previous.groups.entries.map(value => value.key === key ? result.snapshot.group : value) }, members: result.snapshot });
          setMemberError(''); setDeletedIds(new Set());
        } else if (result.status === 'missing') {
          const previous = groupViewRef.current;
          commitGroupView({ ...previous, groups: { ...previous.groups, revision: '', entries: previous.groups.entries.filter(value => value.key !== key) }, members: null });
          openGroupRef.current = { provider, key: null }; setOpenChoice({ provider, key: null }); saveOpenMediaGroup(provider, null);
          setMemberError(''); setDeletedIds(new Set());
        } else if (result.status === 'changed') setMemberError('녹화 파일 목록이 변경됐습니다. 다시 확인해 주세요.');
      } catch (cause) { if (memberCurrent()) setMemberError(message(cause)); }
    } catch (cause) { if (current()) { setRefreshError(message(cause)); setLoaded(true); } }
    finally {
      groupFlight.current = false;
      if (lifetime === groupLifetime.current) setGroupReading(false);
      const pending = pendingGroupRead.current; pendingGroupRead.current = null;
      if (pending) void groupReadRef.current(pending);
    }
  };
  useEffect(() => {
    groupLifetime.current++;
    setGroupReading(false);
    if (!grouped) return;
    if (groupViewRef.current.provider !== provider) {
      commitGroupView({ provider, groups: emptyMediaGroups(), members: null });
      const key = readOpenMediaGroup(provider); openGroupRef.current = { provider, key }; setOpenChoice({ provider, key });
      setLoaded(false); setDeletedIds(new Set()); setMemberError(''); setRefreshError(''); setSelection(emptyMediaSelection());
    }
    if (!active) return;
    void groupReadRef.current('refresh');
    const timer = setInterval(() => void groupReadRef.current('poll'), 3000);
    return () => { groupLifetime.current++; pendingGroupRead.current = null; clearInterval(timer); };
  }, [active, api, provider, grouped]);
  useEffect(() => {
    if (!active || !grouped || !currentGroup || !openKey) { setPanelStyle(null); return; }
    const position = () => {
      const heading = groupHeadings.current.get(openKey);
      if (heading) setPanelStyle(groupPanelPosition(heading.getBoundingClientRect(), window.innerWidth, window.innerHeight));
    };
    position(); window.addEventListener('resize', position); document.addEventListener('scroll', position, true);
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(position);
    const heading = groupHeadings.current.get(openKey); if (heading) observer?.observe(heading);
    const work = library.current?.querySelector('.platform-media-work'), header = library.current?.querySelector(':scope > header');
    if (work) observer?.observe(work); if (header) observer?.observe(header);
    return () => { observer?.disconnect(); window.removeEventListener('resize', position); document.removeEventListener('scroll', position, true); };
  }, [active, grouped, openKey, currentGroup?.key, groups.entries, jobs]);
  useEffect(() => {
    if (!active || grouped) return;
    let cancelled = false, running = false;
    const refresh = async () => { if (running) return; running = true; const epoch = refreshEpoch.current; try {
      const [nextJobs, page] = await Promise.all([api.jobs(provider), api.library(provider)]);
      if (!cancelled && epoch === refreshEpoch.current) { setJobs(nextJobs); setItems(old => { if (!page.cursor || pagesLoaded.current === 1) return page.items; const current = new Map(old.map(item => [item.libraryId, item])); for (const item of page.items) current.set(item.libraryId, item); return [...current.values()].sort((a, b) => b.completedAt - a.completedAt); }); if (pagesLoaded.current === 1) setCursor(page.cursor ?? null); setLoaded(true); setRefreshError(''); }
    } catch (cause) { if (!cancelled && epoch === refreshEpoch.current) { setRefreshError(message(cause)); setLoaded(true); } } finally { running = false; } };
    void refresh(); const timer = setInterval(() => void refresh(), 3000); return () => { cancelled = true; clearInterval(timer); };
  }, [active, api, provider, grouped]);
  const close = async () => { openRequest.current++; const current = playbackRef.current; video.current?.pause(); setPlayback(null); playbackRef.current = null; setBusy(false); setPlaybackError(''); if (current) await api.close(current.token).catch(() => undefined); };
  const play = async (item: MediaLibraryItem) => { if (privacyMode) { onNotice?.('프라이버시 모드를 끄면 재생할 수 있습니다.'); return; } const request = ++openRequest.current; setBusy(true); setError(''); setPlaybackError(''); try { const next = await api.open(item.libraryId); if (request !== openRequest.current) { await api.close(next.token); return; } const old = playbackRef.current; video.current?.pause(); playbackRef.current = next; setSelected([]); setEnded(false); setPlayback(next); setPosition(0); setDuration(item.duration); setControls(true); if (old) await api.close(old.token); } catch (cause) { if (request === openRequest.current) (playbackRef.current ? setPlaybackError : setError)(message(cause)); } finally { if (request === openRequest.current) setBusy(false); } };
  const run = async (action: () => Promise<unknown>) => { refreshEpoch.current++; mutationActive.current = true; setBusy(true); setError(''); try { await action(); setJobs(await api.jobs(provider)); } catch (cause) { setError(message(cause)); } finally { refreshEpoch.current++; mutationActive.current = false; setBusy(false); if (grouped) void groupReadRef.current('refresh'); } };
  const clearSelection = () => { setSelection(emptyMediaSelection()); if (document.activeElement instanceof HTMLElement && (library.current?.contains(document.activeElement) || groupPanel.current?.contains(document.activeElement))) document.activeElement.blur(); };
  const select = (id: string, ctrl: boolean, shift: boolean, node: HTMLElement) => {
    setSelection(previous => selectMedia(previous, id, visibleIds, { ctrl, shift }));
    if (!shift && selected.includes(id) && (ctrl || selected.length === 1)) node.blur();
  };
  const remove = async () => { const targets = items.filter(item => selected.includes(item.libraryId)); setConfirm(false); await run(async () => { for (const item of targets) { await api.delete(item.libraryId, item.revision); setItems(old => old.filter(value => value.libraryId !== item.libraryId)); if (grouped) setDeletedIds(old => new Set([...old, item.libraryId])); setSelected(old => old.filter(id => id !== item.libraryId)); } }); };
  const showControls = () => { setControls(true); if (hideTimer.current) clearTimeout(hideTimer.current); hideTimer.current = setTimeout(() => { if (!video.current?.paused && !video.current?.ended) setControls(false); }, 2200); };
  const startPlayback = () => { if (video.current && !privacyMode) void video.current.play().catch(() => setPlaybackError('재생 버튼을 다시 눌러 주세요.')); };
  const togglePlayback = () => { if (video.current?.paused) startPlayback(); else video.current?.pause(); showControls(); };
  const replay = () => { if (video.current && !privacyMode) { video.current.currentTime = 0; setEnded(false); startPlayback(); } };
  const seek = (seconds: number) => { if (video.current && !privacyMode) { const limit = Number.isFinite(video.current.duration) ? video.current.duration : duration; const next = Math.max(0, Math.min(limit, seconds)); video.current.currentTime = next; setPosition(next); setEnded(false); showControls(); } };
  const fullscreen = () => { const action = document.fullscreenElement ? document.exitFullscreen?.() : surface.current?.requestFullscreen?.(); void action?.catch(() => setPlaybackError('전체화면으로 전환하지 못했습니다.')); };
  useEffect(() => {
    if (!active) { setMenu(null); return; }
    const own = () => confirmation.current ?? surface.current ?? contextMenu.current;
    const foreignOverlay = () => document.documentElement.dataset.tutorialOpen === 'true' || [...document.querySelectorAll<HTMLElement>('dialog[open], [aria-modal="true"], [data-card-context-menu]')].some(node => node !== own() && !own()?.contains(node) && !node.closest('[hidden],[inert]') && getComputedStyle(node).display !== 'none');
    const key = (event: KeyboardEvent) => {
      if (event.defaultPrevented || foreignOverlay()) return;
      const overlay = own();
      if (event.key === 'Escape') { if (surface.current?.querySelector('.platform-replay-chat [role="dialog"], .platform-replay-chat-settings, .platform-replay-settings')) return; event.preventDefault(); event.stopPropagation(); if (confirm) setConfirm(false); else if (menu) setMenu(null); else if (playbackRef.current && document.fullscreenElement) fullscreen(); else if (playbackRef.current) void close(); else if (selected.length) clearSelection(); else if (grouped && openKey) chooseGroup(null); else clearSelection(); return; }
      if (overlay) {
        if (event.key === 'Tab') { const controls = [...overlay.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled),[tabindex="0"]')]; const first = controls[0], last = controls.at(-1); if (first && (event.shiftKey ? document.activeElement === first : document.activeElement === last)) { event.preventDefault(); (event.shiftKey ? last : first)?.focus(); } }
        if (menu && ['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) { const entries = [...overlay.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')]; const current = entries.indexOf(document.activeElement as HTMLButtonElement); const next = event.key === 'Home' ? 0 : event.key === 'End' ? entries.length - 1 : (current + (event.key === 'ArrowDown' ? 1 : -1) + entries.length) % entries.length; event.preventDefault(); entries[next]?.focus(); }
        if (playbackRef.current && !confirm && !menu && !privacyMode && !event.isComposing && !event.ctrlKey && !event.metaKey && !event.altKey && !(event.target instanceof Element && event.target.closest('input,textarea,select,button,[contenteditable],.platform-replay-chat'))) {
          const key = event.key.toLowerCase();
          if ([' ', 'k', 'arrowleft', 'arrowright', 'm', 'f'].includes(key)) {
            event.preventDefault(); event.stopPropagation();
            if (key === ' ' || key === 'k') togglePlayback();
            else if (key === 'arrowleft' || key === 'arrowright') seek((video.current?.currentTime ?? 0) + (key === 'arrowleft' ? -5 : 5));
            else if (key === 'm') setMuted(value => !value);
            else fullscreen();
          }
        }
        return;
      }
      if (busy || event.target instanceof Element && event.target.closest('input,textarea,select,button,[contenteditable]')) return;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') { event.preventDefault(); if (event.shiftKey) clearSelection(); else setSelection({ ids: visibleIds, anchorId: visibleIds[0] ?? null }); }
      else if (event.key === 'Delete' && selected.length) { event.preventDefault(); setConfirm(true); }
      else if (event.key === 'Enter' && selected.length === 1) { event.preventDefault(); const item = items.find(item => item.libraryId === selected[0]); if (item) void play(item); }
    };
    window.addEventListener('keydown', key, true); return () => window.removeEventListener('keydown', key, true);
  }, [active, selected, items, confirm, menu, busy, playback, privacyMode, duration, grouped, openKey]);
  useEffect(() => {
    if (!active || (!confirm && !menu && !playback)) return;
    const previous = document.activeElement as HTMLElement | null;
    const overlay = confirm ? confirmation.current : playback ? surface.current : contextMenu.current;
    (playback && !confirm ? overlay : overlay?.querySelector<HTMLElement>('button:not(:disabled)'))?.focus({ preventScroll: true });
    return () => { if (previous?.isConnected && !previous.closest('[hidden],[inert]')) previous.focus({ preventScroll: true }); };
  }, [active, confirm, !!menu, playback?.token]);
  const fileCard = (item: MediaLibraryItem) => <article key={item.libraryId} data-library-id={item.libraryId} tabIndex={0} className={selected.includes(item.libraryId) ? 'is-focused' : ''} onClick={event => { if (event.detail > 1) return; if (event.ctrlKey || event.metaKey || event.shiftKey) event.preventDefault(); select(item.libraryId, event.ctrlKey || event.metaKey, event.shiftKey, event.currentTarget); }} onDoubleClick={() => void play(item)} onContextMenu={event => { event.preventDefault(); if (!selected.includes(item.libraryId)) setSelected([item.libraryId]); setMenu({ id: item.libraryId, x: Math.max(8, Math.min(event.clientX, window.innerWidth - 210)), y: Math.max(8, Math.min(event.clientY, window.innerHeight - 170)) }); }}>
    <SavedChannel item={item} active={active} privacyMode={privacyMode} api={api} />
    <div className="platform-media-file-content">
      <span className="platform-media-file-top"><span>{time(item.duration)}</span>{selected.length > 1 ? <span className="platform-media-check" aria-hidden="true">{selected.includes(item.libraryId) ? '✓' : ''}</span> : null}</span>
      <strong>{privacyMode ? '프라이버시 모드' : displayTitle(item.title)}</strong>
      <span className="platform-media-file-bottom"><time>{privacyMode ? '' : savedDate(item.completedAt)}</time><span>{size(item.bytes)} <CaptureState item={item} /><ChatStorageWarning item={item} /></span></span>
    </div>
  </article>;
  const shownGroup = members?.group ?? currentGroup;
  const hasEntries = grouped ? groups.entries.length > 0 : items.length > 0;
  const selectedItem = selected.length === 1 ? items.find(item => item.libraryId === selected[0]) : undefined;
  const savedCount = grouped ? groups.entries.reduce((total, entry) => total + (entry.type === 'recordingGroup' ? entry.fileCount : 1), 0) : items.length;
  return <section ref={library} className="platform-media-library" data-media-provider={provider} hidden={!active} aria-label="영상 보관함">
    <header className="platform-media-library-heading"><div><h1>녹화 보관함</h1><p>저장 영상 {savedCount.toLocaleString('ko-KR')}개</p></div><button disabled={busy || !selectedItem} onClick={() => selectedItem && void run(() => api.openFolder(selectedItem.libraryId))}>저장 폴더 열기</button></header>
    <PlatformMediaJobs jobs={jobs} privacyMode={privacyMode} stale={!!refreshError} busy={busy} onCancel={job => void run(() => api.cancel(job.jobId, job.revision))} onRetry={job => void run(() => api.retry(job.jobId, job.revision))} />
    {selected.length > 1 ? <div className="media-selection" role="toolbar" aria-label="선택한 녹화 작업"><span>{selected.length}개 선택</span><button disabled={busy} onClick={clearSelection}>선택 해제</button><button className="danger" disabled={busy} onClick={() => setConfirm(true)}>삭제</button></div> : null}
    <div className="platform-media-library-layout"><div className="platform-media-library-browse">
    {!loaded ? <p role="status"><span className="platform-media-spinner" /> 보관함을 불러오는 중</p> : hasEntries ? <div className="platform-media-grid">{grouped ? groups.entries.map(entry => {
      if (entry.type === 'video') return deletedIds.has(entry.item.libraryId) ? null : fileCard(entry.item);
      const group = entry.key === openKey && members ? members.group : entry;
      return <section key={group.key} className="platform-media-group" data-group-key={group.key}><button className="platform-media-group-heading" ref={node => { if (node) groupHeadings.current.set(group.key, node); else groupHeadings.current.delete(group.key); }} aria-expanded={group.key === openKey} aria-controls={group.key === openKey ? groupPanelId : undefined} onClick={event => { event.stopPropagation(); if (event.detail <= 1) chooseGroup(group.key === openKey ? null : group.key); }} onDoubleClick={event => { event.preventDefault(); event.stopPropagation(); }} onContextMenu={event => { event.preventDefault(); event.stopPropagation(); }}>
        <SavedChannel item={group.representative} active={active} privacyMode={privacyMode} api={api} /><span className="platform-media-group-copy"><strong>{privacyMode ? '프라이버시 모드' : displayTitle(group.representative.title)}</strong><small>파일 {group.fileCount}개 · {size(group.totalBytes)}</small></span><span className="platform-media-group-chevron" aria-hidden="true"><FluentIcon glyph="\uE70D" /></span>
      </button></section>;
    }) : items.map(fileCard)}</div> : !error && !refreshError ? <p className="platform-media-empty">저장한 영상이 없습니다.</p> : null}
    {grouped ? groups.cursor ? <button disabled={busy || groupReading} onClick={() => void groupReadRef.current('moreGroups')}>더 보기</button> : null : cursor ? <button disabled={busy} onClick={() => void run(async () => { const page = await api.library(provider, cursor); pagesLoaded.current++; setItems(old => [...old, ...page.items.filter(item => !old.some(value => value.libraryId === item.libraryId))]); setCursor(page.cursor ?? null); })}>더 보기</button> : null}
    {error ? <p role="alert" className="platform-media-error">{error}</p> : null}
    {refreshError ? <p role="alert" className="platform-media-error">{refreshError}{grouped ? <button disabled={groupReading || busy} onClick={() => void groupReadRef.current('refresh')}>다시 확인</button> : null}</p> : null}
    </div>{selectedItem ? <aside className="platform-media-library-detail" aria-label="선택한 녹화 정보"><button className="platform-media-x" aria-label="녹화 정보 닫기" onClick={clearSelection}><FluentIcon glyph="\uE711" /></button><span>저장 영상</span><h2>{privacyMode ? '프라이버시 모드' : selectedItem.title}</h2><p>{time(selectedItem.duration)} · {size(selectedItem.bytes)}</p><button className="platform-media-detail-play" disabled={busy || privacyMode} onClick={() => void play(selectedItem)}>재생</button><button className="danger" disabled={busy} onClick={() => setConfirm(true)}>삭제</button></aside> : <div className="platform-media-detail-placeholder" aria-hidden="true" />}</div>
    {active && grouped ? createPortal(<WindowMotion key={privacyMode ? 'private' : 'visible'} show={!!shownGroup && !!openKey && !!panelStyle}>{shownGroup && openKey && panelStyle ? <div ref={groupPanel} id={groupPanelId} data-media-provider={provider} role="region" aria-label="묶인 녹화 파일" className="platform-media-group-panel" style={panelStyle}>
      <div className="platform-media-group-panel-toolbar"><span>{members?.items.filter(item => !deletedIds.has(item.libraryId)).length ?? 0} / {shownGroup.fileCount}개 파일</span>{groupReading ? <span className="platform-media-spinner" role="status" aria-label="녹화 파일 갱신 중" /> : null}<button className="platform-media-x" aria-label="녹화 묶음 닫기" onClick={() => chooseGroup(null)}>×</button></div>
      <div className="platform-media-group-panel-scroll">{!members && !memberError ? <p role="status">녹화 파일을 불러오는 중</p> : null}{memberError ? <div role="alert" className="platform-media-error">{memberError}<button disabled={groupReading || busy} onClick={() => void groupReadRef.current('members')}>다시 확인</button></div> : null}<div className="platform-media-group-files">{members?.items.filter(item => !deletedIds.has(item.libraryId)).map(fileCard)}</div>{members?.cursor ? <button disabled={busy || groupReading} onClick={() => void groupReadRef.current('moreMembers')}>파일 더 보기</button> : null}</div>
    </div> : null}</WindowMotion>, document.body) : null}
    {menu ? <><div className="platform-media-dismiss" onClick={() => setMenu(null)} /><div ref={contextMenu} role="menu" className="platform-media-menu" style={{ left: menu.x, top: menu.y }}><button role="menuitem" disabled={busy} onClick={() => { const item = items.find(item => item.libraryId === menu.id); setMenu(null); if (item) void play(item); }}>재생</button><button role="menuitem" disabled={busy} onClick={() => { const id = menu.id; setMenu(null); void run(() => api.openFolder(id)); }}>저장 폴더 열기</button><button role="menuitem" disabled={busy} onClick={() => { setMenu(null); setConfirm(true); }}>삭제</button></div></> : null}
    <WindowMotion show={confirm}>{confirm ? <div className="platform-media-confirm-backdrop"><div ref={confirmation} role="alertdialog" aria-modal={active} aria-label="영상 삭제" className="platform-media-confirm"><button className="platform-media-x" aria-label="닫기" onClick={() => setConfirm(false)}>×</button><h2>{selected.length}개 영상을 삭제할까요?</h2><p>저장 파일이 삭제됩니다.</p><button className="danger" disabled={busy} onClick={() => void remove()}>삭제</button></div></div> : null}</WindowMotion>
    <WindowMotion show={!!playback}>{playback ? <div className={`platform-media-player-backdrop${privacyMode ? ' is-private' : ''}`}><div ref={surface} role="dialog" tabIndex={-1} aria-modal={active} aria-label="저장 영상 재생" className={`platform-media-player${hasChat ? ' has-chat' : ''}${controls || paused || ended || playbackError ? ' show-controls' : ''}`}>
      <PlatformReplayPlayer provider={provider} url={playback.url} videoRef={video} active={active} privacyMode={privacyMode}
        title={<>{privacyMode ? '프라이버시 모드' : playback.item.title}<ChatStorageWarning item={playback.item} /></>}
        paused={paused} ended={ended} position={position} duration={duration} volume={volume} muted={muted} busy={busy} error={playbackError}
        mediaEvents={{ onPlay: () => { setPaused(false); setEnded(false); setPlaybackError(''); showControls(); }, onPause: () => setPaused(true), onEnded: () => { setEnded(true); setControls(true); }, onLoadedMetadata: event => { const value = event.currentTarget.duration; if (Number.isFinite(value)) setDuration(value); }, onTimeUpdate: event => setPosition(event.currentTarget.currentTime), onError: () => setPlaybackError('저장 영상을 재생하지 못했습니다. 파일 상태를 확인해 주세요.') }}
        onShowControls={showControls} onLeaveControls={() => { if (!paused && !ended) setControls(false); }} onToggle={togglePlayback} onReplay={replay}
        onSeek={seek} onVolume={value => { setVolume(value); setMuted(false); }} onMute={() => setMuted(value => !value)}
        onFullscreen={fullscreen} onRetry={() => void play(playback.item)} onClose={!hasChat ? () => void close() : undefined} />
      {hasChat ? <PlatformReplayChat provider={provider} libraryId={playback.item.libraryId} currentTime={position} active={active} privacyMode={privacyMode} onClose={() => void close()} api={chatApi} /> : null}
    </div></div> : null}</WindowMotion>
  </section>;
}
