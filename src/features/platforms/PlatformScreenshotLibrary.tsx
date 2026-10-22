import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { FluentIcon } from '../../components/FluentIcon';
import { WindowMotion } from '../../components/WindowMotion';
import { splitGalleryTitle } from '../../components/galleryCardLayout';
import { emptyMediaSelection, retainMediaSelection, selectMedia } from './mediaSelection';
import type { MediaProvider } from './platformMedia';
import { platformScreenshotsApi, type PlatformScreenshotItem, type PlatformScreenshotsApi } from './platformScreenshots';
import './PlatformScreenshotLibrary.css';

export interface PlatformScreenshotLibraryProps {
  provider: MediaProvider;
  active: boolean;
  privacyMode: boolean;
  api?: PlatformScreenshotsApi;
  onNotice?: (message: string) => void;
}
const failure = (cause: unknown) => cause instanceof Error ? cause.message : typeof cause === 'string' ? cause : '스크린샷을 불러오지 못했습니다.';
const title = (item: PlatformScreenshotItem) => splitGalleryTitle(item.title).primary || '스크린샷';
const date = (at: number) => new Date(at).toLocaleString('ko-KR', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });
const validImage = (value: string, full: boolean) => value.length <= (full ? 23_000_000 : 1_400_000) && /^data:image\/(?:png|jpeg|webp);base64,[A-Za-z0-9+/]+={0,2}$/.test(value);

// Independent from CHZZK: only two local image IPCs, including during tab changes.
// Queued work is removed on unmount/viewport exit; no historical image cache.
let imageReads = 0;
const imageQueue: Array<() => void> = [];
function imageRead(work: () => Promise<string>, done: (value: string) => void, failed: () => void) {
  let cancelled = false;
  const start = () => {
    if (cancelled) return;
    imageReads++;
    void Promise.resolve().then(() => cancelled ? '' : work()).then(value => { if (!cancelled) done(value); }, () => { if (!cancelled) failed(); }).finally(() => {
      imageReads--;
      while (imageReads < 2 && imageQueue.length) imageQueue.shift()!();
    });
  };
  if (imageReads < 2) start(); else imageQueue.push(start);
  return () => { cancelled = true; const index = imageQueue.indexOf(start); if (index >= 0) imageQueue.splice(index, 1); };
}
function SavedImage({ item, api, enabled, full = false }: { item: PlatformScreenshotItem; api: PlatformScreenshotsApi; enabled: boolean; full?: boolean }) {
  const target = useRef<HTMLDivElement>(null);
  const [near, setNear] = useState(full || typeof IntersectionObserver === 'undefined');
  const [result, setResult] = useState<{ id: string; value: string; error: boolean } | null>(null);
  useEffect(() => {
    if (full || !enabled || typeof IntersectionObserver === 'undefined') return;
    const observer = new IntersectionObserver(entries => { for (const entry of entries) if (entry.target === target.current) setNear(entry.isIntersecting); }, { rootMargin: '180px' });
    if (target.current) observer.observe(target.current);
    return () => observer.disconnect();
  }, [full, enabled]);
  useEffect(() => {
    setResult(null);
    if (!enabled || !near) return;
    return imageRead(() => api.image(item.provider, item.id, full), value => setResult({ id: item.id, value: validImage(value, full) ? value : '', error: !validImage(value, full) }), () => setResult({ id: item.id, value: '', error: true }));
  }, [api, enabled, near, item.provider, item.id, full]);
  const current = enabled && near && result?.id === item.id ? result : null;
  return <div ref={target} className={full ? 'platform-shot-original' : 'platform-shot-thumbnail'} aria-busy={enabled && near && !current}>
    {current?.value ? <img src={current.value} alt={full ? '저장된 스크린샷' : ''} decoding="async" onError={() => setResult({ id: item.id, value: '', error: true })} /> : !enabled ? null : current?.error ? <span role={full ? 'alert' : undefined}>이미지를 열 수 없습니다.</span> : near ? <span className="platform-shot-spinner" role="status" aria-label="불러오는 중" /> : null}
  </div>;
}

export function PlatformScreenshotLibrary(props: PlatformScreenshotLibraryProps) {
  // Provider changes dispose all selection/preview/late-reply state, not playback.
  return <ScreenshotScope key={props.provider} {...props} />;
}
function ScreenshotScope({ provider, active, privacyMode, api = platformScreenshotsApi, onNotice }: PlatformScreenshotLibraryProps) {
  const [items, setItems] = useState<PlatformScreenshotItem[]>([]), [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(false), [loaded, setLoaded] = useState(false), [error, setError] = useState('');
  const [selection, setSelection] = useState(emptyMediaSelection), [viewing, setViewing] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null), [confirm, setConfirm] = useState<string[] | null>(null);
  const [deleting, setDeleting] = useState(false), [folderBusy, setFolderBusy] = useState(false);
  const root = useRef<HTMLElement>(null), viewer = useRef<HTMLDivElement>(null), context = useRef<HTMLDivElement>(null), confirmation = useRef<HTMLDivElement>(null);
  const alive = useRef(false), epoch = useRef(0), reading = useRef(false), mutation = useRef(false), folderFlight = useRef(false);
  const enabled = useRef(active && !privacyMode); enabled.current = active && !privacyMode;
  const pages = useRef(1), pendingRead = useRef(false), readRef = useRef<(append?: boolean) => Promise<void>>(async () => undefined);
  const state = useRef({ items, cursor }); state.current = { items, cursor };
  const visibleIds = useMemo(() => items.map(item => item.id), [items]);
  const selected = selection.ids, multi = selected.length > 1;
  const viewed = items.find(item => item.id === viewing);
  const canAct = active && !privacyMode && !deleting;
  const clear = () => { setSelection(emptyMediaSelection()); if (document.activeElement instanceof HTMLElement && root.current?.contains(document.activeElement)) document.activeElement.blur(); };
  readRef.current = async (append = false) => {
    if (!enabled.current) return;
    if (mutation.current) { pendingRead.current = true; return; }
    if (reading.current) { pendingRead.current = true; return; }
    const previous = state.current;
    if (append && !previous.cursor) return;
    reading.current = true; const request = epoch.current;
    const current = () => alive.current && enabled.current && request === epoch.current && !mutation.current;
    setLoading(true); setError('');
    try {
      let nextCursor: string | null = append ? previous.cursor : null;
      const nextItems = append ? previous.items.slice() : [];
      const known = new Set(nextItems.map(item => item.id)), visited = new Set<string>();
      const count = append ? 1 : pages.current;
      let fetched = 0;
      for (let page = 0; page < count; page++) {
        const next = await api.list(provider, nextCursor, 24);
        if (!current()) return;
        for (const item of next.items) if (item.provider === provider && !known.has(item.id)) { known.add(item.id); nextItems.push(item); }
        fetched++;
        nextCursor = next.cursor ?? null;
        if (!nextCursor) break;
        if (visited.has(nextCursor)) throw new Error('목록 위치가 변경됐습니다. 새로고침해 주세요.');
        visited.add(nextCursor);
      }
      if (!current()) return;
      state.current = { items: nextItems, cursor: nextCursor ?? null };
      setItems(nextItems); setCursor(nextCursor ?? null); setLoaded(true);
      pages.current = append ? pages.current + fetched : Math.max(1, fetched);
      setSelection(previousSelection => retainMediaSelection(previousSelection, nextItems.map(item => item.id)));
      setViewing(previousId => nextItems.some(item => item.id === previousId) ? previousId : null);
    } catch (cause) { if (current()) { setError(failure(cause)); setLoaded(true); } }
    finally {
      reading.current = false;
      if (alive.current && request === epoch.current) setLoading(false);
      if (pendingRead.current) { pendingRead.current = false; if (enabled.current && alive.current) void readRef.current(); }
    }
  };
  useEffect(() => { alive.current = true; return () => { alive.current = false; epoch.current++; pendingRead.current = false; }; }, []);
  useEffect(() => {
    epoch.current++; setLoading(false); setMenu(null); setConfirm(null); setViewing(null);
    if (privacyMode) setSelection(emptyMediaSelection());
    if (active && !privacyMode) void readRef.current();
    return () => { epoch.current++; pendingRead.current = false; };
  }, [active, privacyMode, api]);
  const open = (id: string) => { if (canAct) { setMenu(null); setViewing(id); } };
  const folder = async (id?: string) => {
    if (!id || !canAct || folderFlight.current) return;
    folderFlight.current = true; setFolderBusy(true); setMenu(null); const request = epoch.current;
    try { await api.folder(provider, id); }
    catch (cause) { if (alive.current && enabled.current && request === epoch.current) setError(failure(cause)); }
    finally { folderFlight.current = false; if (alive.current) setFolderBusy(false); }
  };
  const remove = async () => {
    if (!canAct || !confirm || mutation.current) return;
    const ids = confirm.slice(); mutation.current = true; epoch.current++; setLoading(false); setDeleting(true); setError('');
    const request = epoch.current;
    try {
      const result = await api.delete(provider, ids);
      if (!alive.current || !enabled.current || request !== epoch.current) return;
      const deleted = new Set(result.deleted.filter(id => ids.includes(id)));
      const remaining = state.current.items.filter(item => !deleted.has(item.id));
      state.current = { ...state.current, items: remaining }; setItems(remaining);
      setSelection(previous => retainMediaSelection(previous, remaining.map(item => item.id)));
      setViewing(previous => previous && deleted.has(previous) ? null : previous); setConfirm(null);
      if (result.failed.length) setError(`${result.failed.length}개 삭제 실패 · ${result.failed[0]?.error ?? '파일을 확인해 주세요.'}`);
      else if (deleted.size) onNotice?.(`${deleted.size}개 삭제했습니다.`);
    } catch (cause) { if (alive.current && enabled.current && request === epoch.current) setError(failure(cause)); }
    finally { mutation.current = false; if (alive.current) { setDeleting(false); if (pendingRead.current && enabled.current) { pendingRead.current = false; void readRef.current(); } } }
  };
  const move = (direction: number) => { const index = items.findIndex(item => item.id === viewing); const next = items[index + direction]; if (next) setViewing(next.id); };
  useEffect(() => {
    if (!active || privacyMode) return;
    const own = () => confirmation.current ?? viewer.current ?? context.current;
    const foreign = () => document.documentElement.dataset.tutorialOpen === 'true' || [...document.querySelectorAll<HTMLElement>('dialog[open],[aria-modal="true"],[data-card-context-menu]')].some(node => node !== own() && !own()?.contains(node) && !node.closest('[hidden],[inert]') && getComputedStyle(node).display !== 'none');
    const key = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || foreign()) return;
      const overlay = own();
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); if (confirm) { if (!deleting) setConfirm(null); } else if (menu) setMenu(null); else if (viewing) setViewing(null); else clear(); return; }
      if (overlay) {
        if (event.key === 'Tab') { const controls = [...overlay.querySelectorAll<HTMLElement>('button:not(:disabled),[tabindex="0"]')]; const first = controls[0], last = controls.at(-1); if (first && (event.shiftKey ? document.activeElement === first : document.activeElement === last)) { event.preventDefault(); (event.shiftKey ? last : first)?.focus(); } }
        if (menu && ['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) { const buttons = [...overlay.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')]; const current = buttons.indexOf(document.activeElement as HTMLButtonElement); event.preventDefault(); buttons[event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (current + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length]?.focus(); }
        if (viewing && !confirm && !menu && !event.ctrlKey && !event.metaKey && !event.altKey && ['ArrowLeft', 'ArrowRight', 'a', 'd'].includes(event.key)) { event.preventDefault(); move(event.key === 'ArrowLeft' || event.key === 'a' ? -1 : 1); }
        return;
      }
      if (deleting || event.target instanceof Element && event.target.closest('input,textarea,select,button,[contenteditable]')) return;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') { event.preventDefault(); if (event.shiftKey) clear(); else setSelection({ ids: visibleIds, anchorId: visibleIds[0] ?? null }); }
      else if (event.key === 'Delete' && selected.length) { event.preventDefault(); setConfirm(selected.slice()); }
      else if (event.key === 'Enter' && selected.length === 1 && !(event.target instanceof Element && event.target.closest('[data-screenshot-id]'))) { event.preventDefault(); open(selected[0]!); }
    };
    window.addEventListener('keydown', key, true); return () => window.removeEventListener('keydown', key, true);
  }, [active, privacyMode, confirm, menu, viewing, items, selected, deleting]);
  useEffect(() => {
    if (!active || privacyMode || (!confirm && !menu && !viewing)) return;
    const previous = document.activeElement as HTMLElement | null;
    const overlay = confirmation.current ?? viewer.current ?? context.current;
    overlay?.querySelector<HTMLElement>('button:not(:disabled)')?.focus({ preventScroll: true });
    return () => { if (previous?.isConnected && !previous.closest('[hidden],[inert]')) previous.focus({ preventScroll: true }); };
  }, [active, privacyMode, confirm, !!menu, viewing]);
  const overlays = active && !privacyMode ? <>
    {menu ? <><div className="platform-shot-dismiss" onPointerDown={() => setMenu(null)} /><div ref={context} role="menu" data-card-context-menu className="platform-shot-menu" style={{ left: menu.x, top: menu.y }}>
      <button role="menuitem" disabled={deleting} onClick={() => open(menu.id)}>열기</button><button role="menuitem" disabled={deleting || folderBusy} onClick={() => void folder(menu.id)}>폴더 열기</button><button role="menuitem" className="danger" disabled={deleting} onClick={() => { setConfirm(selected.includes(menu.id) ? selected.slice() : [menu.id]); setMenu(null); }}>삭제</button>
    </div></> : null}
    <WindowMotion show={!!viewed}>{viewed ? <div className="platform-shot-backdrop"><div ref={viewer} role="dialog" aria-modal="true" aria-label="스크린샷 보기" className="platform-shot-viewer" tabIndex={-1}>
      <header><div><strong>{viewed.channelName}</strong><span>{title(viewed)}</span></div><button className="platform-shot-x" aria-label="스크린샷 닫기" onClick={() => setViewing(null)}><FluentIcon glyph="\uE711" /></button></header>
      <SavedImage key={viewed.id} item={viewed} api={api} enabled={active && !privacyMode} full />
      <footer><button disabled={items[0]?.id === viewed.id} aria-label="이전 스크린샷" onClick={() => move(-1)}><FluentIcon glyph="\uE76B" /></button><span>{date(viewed.createdAt)} · {viewed.width}×{viewed.height}</span><button disabled={items.at(-1)?.id === viewed.id} aria-label="다음 스크린샷" onClick={() => move(1)}><FluentIcon glyph="\uE76C" /></button></footer>
    </div></div> : null}</WindowMotion>
    <WindowMotion show={!!confirm}>{confirm ? <div className="platform-shot-backdrop"><div ref={confirmation} role="alertdialog" aria-modal="true" aria-label="스크린샷 삭제" className="platform-shot-confirm">
      <button className="platform-shot-x" aria-label="닫기" disabled={deleting} onClick={() => setConfirm(null)}><FluentIcon glyph="\uE711" /></button><h2>{confirm.length}개 스크린샷을 삭제할까요?</h2><p>저장 파일이 삭제됩니다.</p>{error ? <p role="alert">{error}</p> : null}<footer><button disabled={deleting} onClick={() => setConfirm(null)}>취소</button><button className="danger" disabled={deleting} aria-busy={deleting} onClick={() => void remove()}>{deleting ? <span className="platform-shot-spinner" aria-label="삭제 중" /> : '삭제'}</button></footer>
    </div></div> : null}</WindowMotion>
  </> : null;
  return <section ref={root} className="platform-screenshot-library" data-media-provider={provider} hidden={!active} aria-label="스크린샷 목록" tabIndex={-1}>
    <header><div className="platform-shot-heading-copy"><h1>스크린샷</h1><p>{items.length.toLocaleString('ko-KR')}개</p></div><div><button disabled={!canAct || loading} aria-busy={loading} onClick={() => void readRef.current()}>{loading ? <span className="platform-shot-spinner" aria-label="불러오는 중" /> : '새로고침'}</button><button disabled={!canAct || folderBusy || selected.length !== 1} aria-busy={folderBusy} onClick={() => void folder(selected[0])}>저장 폴더 열기</button></div></header>
    {error && !privacyMode ? <p className="platform-shot-error" role="alert">{error}</p> : null}
    {multi && !privacyMode ? <div className="platform-shot-selection"><span>{selected.length}개 선택</span><button disabled={deleting} onClick={clear}>선택 해제</button><button className="danger" disabled={deleting} onClick={() => setConfirm(selected.slice())}>삭제</button></div> : null}
    <div className="platform-shot-grid">{items.map(item => <article key={item.id} data-screenshot-id={item.id} tabIndex={0} aria-label={privacyMode ? '스크린샷' : `${item.channelName} · ${title(item)}`} className={selected.includes(item.id) ? 'is-focused' : ''}
      onClick={event => { if (!canAct || event.detail > 1) return; const next = selectMedia(selection, item.id, visibleIds, { ctrl: event.ctrlKey || event.metaKey, shift: event.shiftKey }); setSelection(next); if (!next.ids.includes(item.id)) event.currentTarget.blur(); }}
      onKeyDown={event => { if (!canAct || event.defaultPrevented || event.nativeEvent.isComposing) return; if (event.key === ' ' && !event.altKey) { event.preventDefault(); setSelection(previous => selectMedia(previous, item.id, visibleIds, { ctrl: true, shift: event.shiftKey })); } else if (event.key === 'Enter' && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey) { event.preventDefault(); open(item.id); } }}
      onDoubleClick={event => { if (!event.ctrlKey && !event.metaKey && !event.shiftKey) open(item.id); }}
      onContextMenu={event => { event.preventDefault(); if (!canAct) return; if (!selected.includes(item.id)) setSelection({ ids: [item.id], anchorId: item.id }); setMenu({ id: item.id, x: Math.max(8, Math.min(event.clientX, window.innerWidth - 208)), y: Math.max(8, Math.min(event.clientY, window.innerHeight - 144)) }); }}>
      <SavedImage item={item} api={api} enabled={active && !privacyMode} />{multi && !privacyMode ? <span className="platform-shot-check" aria-hidden="true">{selected.includes(item.id) ? '✓' : ''}</span> : null}
      <div className="platform-shot-card-caption"><strong title={privacyMode ? undefined : title(item)}>{privacyMode ? '프라이버시 모드' : item.channelName || title(item)}</strong><time>{privacyMode ? '' : date(item.createdAt)}</time><small>{privacyMode ? '' : `${item.width}×${item.height} · ${(item.bytes / 1_000_000).toFixed(1)} MB`}</small></div>
    </article>)}</div>
    {!items.length && !loading && loaded && !error && !privacyMode ? <div className="platform-shot-empty"><strong>저장된 스크린샷이 없습니다.</strong><p>라이브 플레이어의 카메라 버튼으로 저장할 수 있습니다.</p></div> : null}
    {cursor ? <button className="platform-shot-more" disabled={!canAct || loading} onClick={() => void readRef.current(true)}>더 보기</button> : null}
    {overlays ? createPortal(overlays, document.body) : null}
  </section>;
}
