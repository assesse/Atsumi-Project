import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { createPortal } from "react-dom";
import { nextPagePreviewAnchor, pagePreviewSlots, type PreviewReadingDirection } from "../../components/pagePreviewNavigation";
import { pagePreviewFrame, pagePreviewSpreadDimension } from "../../components/pagePreviewFrame";
import { clampPagePreviewResizeBox, pagePreviewResizeBounds, resizePagePreviewBox, type PagePreviewResizeBox, type PagePreviewResizeEdge } from "../../components/pagePreviewResize";
import type { PixivDetail } from "./model";
import type { OriginalWindow } from "./originalWindow";
import { PixivOriginalPages } from "./PixivOriginalPages";
import { PixivMetadataText } from "./PixivMetadataText";
import "./PixivPageReader.css";

const PREFERENCE_KEY = "atsumi.pixiv.reader.v1";
const readerViewport = () => ({ width: window.visualViewport?.width ?? window.innerWidth, height: window.visualViewport?.height ?? window.innerHeight });
type ResizeSession = { edge: PagePreviewResizeEdge; pointerId: number; x: number; y: number; box: PagePreviewResizeBox; handle: HTMLElement };
function preferences(): { reading: PreviewReadingDirection; pinned: boolean } {
  try { const value = JSON.parse(localStorage.getItem(PREFERENCE_KEY) ?? "{}"); return { reading: value?.reading === "rtl" ? "rtl" : "ltr", pinned: value?.pinned === true }; }
  catch { return { reading: "ltr", pinned: false }; }
}
function ReaderIcon({ kind }: { kind: "left" | "right" | "close" | "pin" | "spread" | "direction" | "bookmark" }) {
  const paths = { left: "m15 4-8 8 8 8", right: "m9 4 8 8-8 8", close: "m6 6 12 12M6 18 18 6", pin: "m8 3 8 0-1 6 3 5H6l3-5-1-6M12 14v7", spread: "M3 4h18v16H3zM12 4v16", direction: "M3 7h18m-4-4 4 4-4 4M21 17H3m4-4-4 4 4 4", bookmark: "M5 3h14v19l-7-4-7 4V3Z" };
  return <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round"><path d={paths[kind]} /></svg>;
}
export function PixivPageReader({ detail, initialPage, pool, privacyMode, openerElement, initialPinned, toolbar, navigation, related, initialScroll = 0, onScrollPosition, isFavorite, onFavorite, onPageChange, onCycleWork, onBeforeClose, onClose }: {
  detail: PixivDetail; initialPage: number; pool: OriginalWindow; privacyMode: boolean;
  openerElement?: HTMLElement | null;
  initialPinned?: boolean; toolbar?: ReactNode; navigation?: ReactNode;
  related?: ReactNode; initialScroll?: number; onScrollPosition?: (position: number) => void;
  onPageChange?: (page: number) => void; onCycleWork?: (direction: -1 | 1) => void;
  onBeforeClose?: (page: number) => boolean;
  isFavorite: (page: number) => boolean; onFavorite: (page: number) => void; onClose: (page: number) => void;
}) {
  const [anchor, setAnchor] = useState(Math.min(detail.pages.length, Math.max(1, initialPage + 1)));
  const [spread, setSpread] = useState(false), [preference, setPreference] = useState(() => ({ ...preferences(), ...(initialPinned === undefined ? {} : { pinned: initialPinned }) }));
  const [visible, setVisible] = useState(false), [closing, setClosing] = useState(false);
  const [pulse, setPulse] = useState({ side: "", sequence: 0 });
  const [viewport, setViewport] = useState(readerViewport);
  const [resizeBox, setResizeBox] = useState<PagePreviewResizeBox | null>(null);
  const resizeSession = useRef<ResizeSession | null>(null);
  const dialog = useRef<HTMLElement>(null), opener = useRef<HTMLElement | null>(null);
  const scrollBody = useRef<HTMLDivElement>(null), scrollRestored = useRef(initialScroll <= 0);
  useLayoutEffect(() => {
    const node = scrollBody.current;
    if (scrollRestored.current || !node || node.scrollHeight - node.clientHeight < initialScroll) return;
    node.scrollTop = initialScroll; scrollRestored.current = true;
  }, [initialScroll, related]);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined), closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const keyboardFocus = useRef(false), lastPointer = useRef<string | null>(null), closeRef = useRef(onClose); closeRef.current = onClose;
  const pageChangeRef = useRef(onPageChange), cycleWorkRef = useRef(onCycleWork), beforeCloseRef = useRef(onBeforeClose);
  pageChangeRef.current = onPageChange; cycleWorkRef.current = onCycleWork; beforeCloseRef.current = onBeforeClose;
  const slots = useMemo(() => pagePreviewSlots(anchor, detail.pages.length, spread, preference.reading), [anchor, detail.pages.length, spread, preference.reading]);
  const realPages = slots.filter((page): page is number => page !== null);
  const currentReal = Math.min(...realPages);
  const currentRef = useRef(currentReal - 1); currentRef.current = currentReal - 1;
  useEffect(() => { pageChangeRef.current?.(currentReal - 1); }, [currentReal]);
  const fallback = detail.pages[currentReal - 1];
  const dimensions = slots.map(page => page === null ? fallback : detail.pages[page - 1]);
  const frame = pagePreviewFrame(spread ? pagePreviewSpreadDimension(dimensions[0], dimensions[1]) : dimensions[0], viewport);
  const resizeLimits = pagePreviewResizeBounds(viewport);
  const ownsInput = useCallback(() => {
    const modals = [...document.querySelectorAll<HTMLElement>('[aria-modal="true"]')].filter(node => !node.closest('[hidden],[inert]') && getComputedStyle(node).display !== "none");
    return modals.at(-1) === dialog.current && document.documentElement.dataset.tutorialOpen !== "true" && !document.querySelector('dialog[open], [data-card-context-menu]');
  }, []);
  const showControls = useCallback(() => {
    clearTimeout(timer.current); setVisible(true);
    timer.current = setTimeout(() => {
      if (!resizeSession.current && (!keyboardFocus.current || !dialog.current?.contains(document.activeElement) || document.activeElement === dialog.current)) setVisible(false);
    }, 1800);
  }, []);
  const finishResize = useCallback((reveal = true) => {
    const session = resizeSession.current;
    if (!session) return;
    resizeSession.current = null;
    dialog.current?.classList.remove("is-edge-resizing");
    try { if (session.handle.hasPointerCapture?.(session.pointerId)) session.handle.releasePointerCapture(session.pointerId); } catch { /* Capture may have been revoked by the window. */ }
    if (reveal) showControls();
  }, [showControls]);
  const close = useCallback(() => {
    if (closeTimer.current || !ownsInput()) return;
    finishResize(false);
    if (beforeCloseRef.current?.(currentRef.current) === false) return;
    if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) { closeRef.current(currentRef.current); return; }
    setClosing(true); closeTimer.current = setTimeout(() => closeRef.current(currentRef.current), 140);
  }, [finishResize, ownsInput]);
  useEffect(() => {
    opener.current = openerElement ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    dialog.current?.focus({ preventScroll: true });
    const resize = () => { const next = readerViewport(); finishResize(false); setViewport(next); setResizeBox(current => current ? clampPagePreviewResizeBox(current, next) : null); };
    const focus = () => { if (ownsInput() && (!document.activeElement || document.activeElement === document.body)) dialog.current?.focus({ preventScroll: true }); };
    const move = (event: PointerEvent) => {
      const session = resizeSession.current;
      if (!session || event.pointerId !== session.pointerId) return;
      if (!ownsInput()) { finishResize(); return; }
      event.preventDefault();
      setResizeBox(resizePagePreviewBox(session.box, session.edge, event.clientX - session.x, event.clientY - session.y, readerViewport()));
      showControls();
    };
    const finish = (event: PointerEvent) => { if (resizeSession.current?.pointerId === event.pointerId) finishResize(); };
    const blur = () => finishResize();
    window.addEventListener("resize", resize); window.visualViewport?.addEventListener("resize", resize); window.addEventListener("focus", focus);
    window.addEventListener("pointermove", move); window.addEventListener("pointerup", finish); window.addEventListener("pointercancel", finish); window.addEventListener("blur", blur);
    return () => { finishResize(false); clearTimeout(timer.current); clearTimeout(closeTimer.current); window.removeEventListener("resize", resize); window.visualViewport?.removeEventListener("resize", resize); window.removeEventListener("focus", focus); window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", finish); window.removeEventListener("pointercancel", finish); window.removeEventListener("blur", blur); if (opener.current?.isConnected) opener.current.focus({ preventScroll: true }); };
  }, [finishResize, ownsInput, showControls]);
  const currentBox = (): PagePreviewResizeBox => {
    const rect = dialog.current?.getBoundingClientRect();
    return resizeBox ?? { left: rect?.left ?? 0, top: rect?.top ?? 0, width: rect?.width || frame.dialogWidth, height: rect?.height || frame.dialogHeight };
  };
  const beginResize = (event: ReactPointerEvent<HTMLElement>, edge: PagePreviewResizeEdge) => {
    if (closing || event.button !== 0 || event.isPrimary === false || resizeSession.current || !ownsInput()) return;
    event.preventDefault(); event.stopPropagation(); keyboardFocus.current = false;
    const box = resizePagePreviewBox(currentBox(), edge, 0, 0, readerViewport());
    resizeSession.current = { edge, pointerId: event.pointerId, x: event.clientX, y: event.clientY, box, handle: event.currentTarget };
    try { event.currentTarget.setPointerCapture?.(event.pointerId); } catch { /* Window listeners still provide bounded drag. */ }
    setResizeBox(box); dialog.current?.classList.add("is-edge-resizing"); showControls();
  };
  const resizeWithKeyboard = (event: ReactKeyboardEvent<HTMLElement>, edge: "right" | "bottom") => {
    if (closing || !ownsInput() || event.ctrlKey || event.metaKey || event.altKey) return;
    const axisKeys = edge === "right" ? ["ArrowLeft", "ArrowRight"] : ["ArrowUp", "ArrowDown"];
    if (!axisKeys.includes(event.key)) return;
    event.preventDefault(); event.stopPropagation(); keyboardFocus.current = true;
    const step = (event.shiftKey ? 48 : 12) * (event.key === axisKeys[0] ? -1 : 1);
    setResizeBox(resizePagePreviewBox(currentBox(), edge, edge === "right" ? step : 0, edge === "bottom" ? step : 0, readerViewport())); showControls();
  };
  const lostResizeCapture = (event: ReactPointerEvent<HTMLElement>) => {
    const session = resizeSession.current;
    if (session?.pointerId === event.pointerId && session.handle === event.currentTarget && !event.currentTarget.hasPointerCapture?.(event.pointerId)) finishResize();
  };
  useEffect(() => { try { localStorage.setItem(PREFERENCE_KEY, JSON.stringify(preference)); } catch { /* Optional reading preferences; never stores image data. */ } }, [preference]);
  const turn = (side: "left" | "right") => {
    const direction = (side === "left" ? -1 : 1) * (preference.reading === "rtl" ? -1 : 1) as -1 | 1;
    const next = nextPagePreviewAnchor(anchor, detail.pages.length, spread, direction);
    if (next === null) return;
    setAnchor(next); setPulse(value => ({ side, sequence: value.sequence + 1 })); showControls();
  };
  useEffect(() => {
    const keyboard = (event: KeyboardEvent) => {
      if (closing || !ownsInput()) return;
      if (event.key === "Escape") { event.preventDefault(); event.stopImmediatePropagation(); close(); return; }
      if (event.isComposing || event.keyCode === 229 || event.ctrlKey || event.metaKey || event.altKey || event.target instanceof Element && event.target.closest("input,textarea,select,[contenteditable]")) return;
      if (event.key === "Tab") {
        keyboardFocus.current = true; showControls();
        const buttons = [...(dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled), [role="separator"][tabindex="0"]') ?? [])];
        const focusOutside = !dialog.current?.contains(document.activeElement);
        if (event.shiftKey && (focusOutside || document.activeElement === buttons[0] || document.activeElement === dialog.current)) { event.preventDefault(); buttons.at(-1)?.focus(); }
        else if (!event.shiftKey && (focusOutside || document.activeElement === buttons.at(-1))) { event.preventDefault(); buttons[0]?.focus(); }
      } else if (["a", "A", "ArrowLeft", "d", "D", "ArrowRight"].includes(event.key)) {
        if (event.target instanceof Element && event.target.closest(".pixiv-reader-related")) return;
        if (event.key.startsWith("Arrow") && event.target instanceof Element && event.target.closest("[data-pixiv-resize-edge]")) return;
        event.preventDefault(); event.stopImmediatePropagation(); turn(["a", "A", "ArrowLeft"].includes(event.key) ? "left" : "right");
      } else if (["q", "Q", "e", "E"].includes(event.key) && cycleWorkRef.current) {
        event.preventDefault(); event.stopImmediatePropagation(); cycleWorkRef.current(["q", "Q"].includes(event.key) ? -1 : 1);
      }
    };
    window.addEventListener("keydown", keyboard, true); return () => window.removeEventListener("keydown", keyboard, true);
  }, [anchor, spread, preference.reading, closing, close, showControls, ownsInput]);
  const controlsVisible = preference.pinned || visible;
  return createPortal(<div className={`pixiv-reader-backdrop${closing ? " is-closing" : ""}`} onMouseDown={event => { if (event.target === event.currentTarget) close(); }}>
    <section ref={dialog} className="pixiv-reader" role="dialog" aria-modal="true" aria-label="Pixiv 페이지 미리보기" tabIndex={-1} data-gallery-shortcuts-suspended data-controls-visible={controlsVisible} data-reading={preference.reading} data-spread={spread} style={resizeBox ? { position: "fixed", ...resizeBox } : { width: frame.dialogWidth, height: frame.dialogHeight }}
      onPointerDown={() => { keyboardFocus.current = false; }} onPointerMove={event => { const point = `${event.clientX}:${event.clientY}`; if (point !== lastPointer.current) { lastPointer.current = point; showControls(); } }} onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) { keyboardFocus.current = false; showControls(); } }}>
      <div className="pixiv-reader-scroll" ref={scrollBody} onScroll={event => { if (scrollRestored.current) onScrollPosition?.(event.currentTarget.scrollTop); }} onWheel={() => { scrollRestored.current = true; }}>
      <div className="pixiv-reader-page">
      <PixivOriginalPages detail={detail} pages={slots.map(page => page === null ? null : page - 1)} pool={pool} privacyMode={privacyMode} />
      <header className="pixiv-reader-chrome pixiv-reader-header">
        <div className="pixiv-reader-title"><PixivMetadataText as="strong" text={detail.work.title} kind="title" private={privacyMode}>{privacyMode ? "Pixiv 페이지 미리보기" : detail.work.title}</PixivMetadataText><button type="button" aria-label="미리보기 닫기" onClick={close}><ReaderIcon kind="close" /></button></div>
        {toolbar && <div className="pixiv-reader-toolbar">{toolbar}</div>}
        {navigation && <div className="pixiv-reader-navigation">{navigation}</div>}
      </header>
      {(["left", "right"] as const).map(side => <button key={side} className={`pixiv-reader-chrome pixiv-reader-arrow is-${side}`} type="button" aria-label={side === "left" ? "왼쪽 페이지" : "오른쪽 페이지"} disabled={nextPagePreviewAnchor(anchor, detail.pages.length, spread, ((side === "left" ? -1 : 1) * (preference.reading === "rtl" ? -1 : 1)) as -1 | 1) === null} onClick={() => turn(side)}>
        <span key={pulse.side === side ? pulse.sequence : "idle"} className={pulse.side === side ? "is-turning" : ""}><ReaderIcon kind={side} /></span>
      </button>)}
      <footer className="pixiv-reader-chrome pixiv-reader-footer">
        <span className="pixiv-reader-page-status" aria-label="현재 쪽">{[...realPages].sort((a, b) => a - b).join("–")} / {detail.pages.length}</span>
        <div className="pixiv-reader-tools"><button type="button" aria-label="두쪽 보기" title="두쪽 보기" aria-pressed={spread} onClick={() => { setSpread(value => !value); setAnchor(currentReal); showControls(); }}><ReaderIcon kind="spread" /></button><button type="button" aria-label="오른쪽부터 읽기" title={preference.reading === "rtl" ? "오른쪽부터 읽기" : "왼쪽부터 읽기"} aria-pressed={preference.reading === "rtl"} onClick={() => { setPreference(value => ({ ...value, reading: value.reading === "ltr" ? "rtl" : "ltr" })); showControls(); }}><ReaderIcon kind="direction" /></button></div>
      </footer>
      <div className="pixiv-reader-left-tools">
        <button type="button" className="pixiv-reader-pin" aria-label="미리보기 UI 고정" title="UI 고정" aria-pressed={preference.pinned} onClick={() => { setPreference(value => ({ ...value, pinned: !value.pinned })); showControls(); }}><ReaderIcon kind="pin" /></button>
        <div className="pixiv-reader-chrome pixiv-reader-page-favorites" role="group" aria-label="페이지 즐겨찾기">{slots.map(page => page === null ? <span key="blank" className="pixiv-reader-bookmark-blank" /> : <button key={page} type="button" className="pixiv-reader-bookmark" aria-label={`${page}쪽 즐겨찾기`} aria-pressed={isFavorite(page - 1)} onClick={() => onFavorite(page - 1)}><ReaderIcon kind="bookmark" /><span>{page}</span></button>)}</div>
      </div>
      {related && <button type="button" className="pixiv-reader-related-hint" aria-label="아래 관련 작품 보기" onClick={() => scrollBody.current?.scrollTo({ top: scrollBody.current.clientHeight, behavior: window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ? "instant" : "smooth" })}>관련 작품 <span aria-hidden="true">⌄</span></button>}
      </div>
      {related && <div className="pixiv-reader-related" aria-label="작품 아래 추천">
        <div className="pixiv-reader-related-nav"><button type="button" onClick={() => { if (scrollBody.current) scrollBody.current.scrollTop = 0; }}>↑ 작품으로</button><button type="button" aria-label="추천 영역에서 미리보기 닫기" onClick={close}><ReaderIcon kind="close" /></button></div>
        {related}
      </div>}
      </div>
      {(["right", "bottom"] as const).map(edge => <div key={edge} className={`pixiv-reader-resize is-${edge}`} data-pixiv-resize-edge={edge} role="separator" tabIndex={0} aria-label={edge === "right" ? "페이지 미리보기 너비 조절" : "페이지 미리보기 높이 조절"} aria-orientation={edge === "right" ? "vertical" : "horizontal"} aria-valuemin={edge === "right" ? resizeLimits.minimumWidth : resizeLimits.minimumHeight} aria-valuemax={edge === "right" ? resizeLimits.maximumWidth : resizeLimits.maximumHeight} aria-valuenow={edge === "right" ? resizeBox?.width ?? frame.dialogWidth : resizeBox?.height ?? frame.dialogHeight} onPointerDown={event => beginResize(event, edge)} onLostPointerCapture={lostResizeCapture} onKeyDown={event => resizeWithKeyboard(event, edge)} onFocus={showControls} />)}
      <div className="pixiv-reader-resize is-corner" data-pixiv-resize-edge="corner" aria-hidden="true" onPointerDown={event => beginResize(event, "corner")} onLostPointerCapture={lostResizeCapture} />
    </section>
  </div>, document.body);
}
