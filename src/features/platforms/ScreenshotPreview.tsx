import { useLayoutEffect, useRef, useState } from 'react';
import { createPreviewLifetime } from './screenshotPreviewLifetime';

type Preview = { id: string; url: string; undoing: boolean; error: string; undoDeadline: number };
/** One presentation clock per saved capture; never deletes a file on dismissal. */
export function ScreenshotPreview({ preview, onDismiss, onUndo, className = '' }: {
  preview: Preview; onDismiss(id: string): void; onUndo(): void; className?: string;
}) {
  const [expired, setExpired] = useState(false);
  const [width, setWidth] = useState(480);
  const surface = useRef<HTMLDivElement>(null), bitmap = useRef<HTMLImageElement>(null);
  const clock = useRef<ReturnType<typeof createPreviewLifetime> | null>(null);
  const owner = useRef({ preview, onDismiss }); owner.current = { preview, onDismiss };
  const fit = () => {
    const parent = surface.current?.parentElement, image = bitmap.current;
    if (!parent) return;
    const ratio = image?.naturalWidth && image.naturalHeight ? image.naturalWidth / image.naturalHeight : 16 / 9;
    const height = parent.clientHeight, top = Math.max(8, Math.min(64, height * .12));
    setWidth(Math.max(1, Math.min(480, parent.clientWidth - 24, Math.min(320, height - top - 12) * ratio)));
  };
  useLayoutEffect(() => {
    const parent = surface.current?.parentElement;
    if (!parent) return;
    fit();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(fit); observer.observe(parent);
    return () => observer.disconnect();
  }, [preview.id]);
  useLayoutEffect(() => {
    setExpired(false);
    const id = preview.id;
    const timer = createPreviewLifetime(
      { now: () => performance.now(), set: (callback, ms) => window.setTimeout(callback, ms), clear: id => window.clearTimeout(id) },
      preview.undoDeadline,
      { expire: () => setExpired(true), dismiss: () => { if (owner.current.preview.id === id) owner.current.onDismiss(id); } },
    );
    clock.current = timer;
    timer.block(preview.undoing || !!preview.error);
    return () => { timer.dispose(); if (clock.current === timer) clock.current = null; };
  }, [preview.id, preview.undoDeadline]);
  useLayoutEffect(() => { clock.current?.block(preview.undoing || !!preview.error); }, [preview.undoing, preview.error]);
  const unavailable = expired || preview.undoing || !!preview.error;
  const label = preview.error ? '저장 취소 실패 · 스크린샷 목록에서 확인해 주세요.'
    : preview.undoing ? '취소 중…' : expired ? '취소 시간이 지났습니다. 스크린샷 목록에서 관리해 주세요.' : '스크린샷 저장 취소';
  return <div ref={surface} style={{ width }} className={'platform-live-capture-preview ' + className} role="group" aria-label="방금 저장한 스크린샷"
    data-platform-native-occlusion="true" onKeyDown={event => event.stopPropagation()}
    onPointerEnter={() => clock.current?.hover(true)} onPointerLeave={() => clock.current?.hover(false)}
    onFocusCapture={() => clock.current?.focus(true)}
    onBlurCapture={event => { if (!(event.relatedTarget instanceof Node) || !event.currentTarget.contains(event.relatedTarget)) clock.current?.focus(false); }}>
    <img ref={bitmap} src={preview.url} onLoad={fit} alt="방금 저장한 스크린샷" />
    <button type="button" className="platform-live-capture-dismiss" aria-label="스크린샷 미리보기 닫기" title="스크린샷 미리보기 닫기"
      aria-disabled={preview.undoing} onClick={() => { if (!preview.undoing) onDismiss(preview.id); }}>
      <svg aria-hidden="true" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M6 6l12 12M18 6L6 18" /></svg>
    </button>
    <button type="button" className="platform-live-capture-cancel" aria-label={label} title={label}
      aria-disabled={unavailable} aria-busy={preview.undoing}
      onClick={() => { if (!unavailable && performance.now() < preview.undoDeadline) onUndo(); }}>
      취소
    </button>
    <span className="platform-live-capture-sr" role={preview.error ? 'alert' : 'status'}>{preview.error || (expired ? label : preview.undoing ? label : '스크린샷 저장됨')}</span>
  </div>;
}

