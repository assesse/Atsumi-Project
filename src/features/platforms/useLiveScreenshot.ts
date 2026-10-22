import { useLayoutEffect, useRef, useState, type RefObject } from 'react';
import type { PlatformScreenshotsApi } from './platformScreenshots';

const MAX_BYTES = 16 * 1024 * 1024;
const CHUNK = 128 * 1024;
const expired = () => new Error('스크린샷 요청이 만료됐습니다. 다시 시도해 주세요.');
type Context = { token: string | null; identity: string; enabled: boolean; api: PlatformScreenshotsApi };
type Flight = { context: Context; epoch: number; deadline: number; cancelled: boolean; captureId?: string; aborted?: boolean };
export type ScreenshotPreview = { url: string; id: string; token: string; identity: string; api: PlatformScreenshotsApi; epoch: number; undoing: boolean; error: string; undoDeadline: number };

/** Capture only the already decoded frame: this must run before the first await. */
export function freezeLiveFrame(video: HTMLVideoElement | null) {
  if (!video || video.readyState < 2 || video.videoWidth < 1 || video.videoHeight < 1 || video.videoWidth > 4096 || video.videoHeight > 4096 || video.videoWidth * video.videoHeight > 16 * 1024 * 1024) throw new Error('현재 저장할 수 있는 영상이 없습니다.');
  const canvas = document.createElement('canvas');
  canvas.width = video.videoWidth; canvas.height = video.videoHeight;
  try {
    const context = canvas.getContext('2d');
    if (!context) throw new Error('스크린샷을 만들 수 없습니다.');
    context.drawImage(video, 0, 0);
    return canvas;
  } catch (error) { canvas.width = 0; canvas.height = 0; throw error; }
}

function encode(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('스크린샷 생성 시간이 초과됐습니다.')), 5000);
    try { canvas.toBlob(blob => { clearTimeout(timer); blob ? resolve(blob) : reject(new Error('영상이 스크린샷을 허용하지 않습니다.')); }, 'image/png'); }
    catch (error) { clearTimeout(timer); reject(error); }
  });
}
function base64(bytes: Uint8Array) {
  let binary = '';
  for (let index = 0; index < bytes.length; index += 8192) binary += String.fromCharCode(...bytes.subarray(index, index + 8192));
  return btoa(binary);
}
function beforeDeadline<T>(promise: Promise<T>, deadline: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(expired()), Math.max(0, deadline - Date.now()));
    promise.then(value => { clearTimeout(timer); resolve(value); }, error => { clearTimeout(timer); reject(error); });
  });
}
const message = (error: unknown) => error instanceof DOMException && error.name === 'SecurityError' ? '현재 영상에서 스크린샷을 만들 수 없습니다.' : typeof error === 'string' ? error : error instanceof Error ? error.message : '스크린샷 저장에 실패했습니다. 다시 시도해 주세요.';

export function screenshotShortcut(event: KeyboardEvent, surface: HTMLElement): boolean {
  if (!(event.code === 'KeyS' || event.key.toLowerCase() === 's') || event.defaultPrevented || event.repeat || event.isComposing || event.keyCode === 229 || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false;
  if (event.target instanceof Element && event.target.closest('input,textarea,select,button,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="combobox"],[role="menu"]')) return false;
  return ![...surface.ownerDocument.querySelectorAll<HTMLElement>('dialog[open],[role="dialog"],[role="alertdialog"],[aria-modal="true"]')].some(dialog => {
    // A floating YouTube detail is this player's own container, not an overlay
    // blocking it. Hidden retained dialogs must not steal the shortcut either.
    if (dialog.contains(surface) || dialog.closest('[hidden],[inert],[aria-hidden="true"]')) return false;
    const style = getComputedStyle(dialog), bounds = dialog.getBoundingClientRect();
    return style.display !== 'none' && style.visibility !== 'hidden' && bounds.width > 0 && bounds.height > 0;
  });
}

export function useLiveScreenshot(context: Context, video: RefObject<HTMLVideoElement | null>) {
  const current = useRef(context); current.current = context;
  const epoch = useRef(0), mounted = useRef(false), flight = useRef<Flight | null>(null);
  const previewRef = useRef<ScreenshotPreview | null>(null);
  const [busy, setBusy] = useState(false), [error, setError] = useState(''), [preview, setPreview] = useState<ScreenshotPreview | null>(null);
  const abort = (ticket: Flight) => {
    if (ticket.captureId && !ticket.aborted) { ticket.aborted = true; void ticket.context.api.abort(ticket.context.token!, ticket.captureId).catch(() => undefined); }
  };
  const clearPreview = (expectedId?: string) => {
    if (expectedId && previewRef.current?.id !== expectedId) return;
    if (previewRef.current) URL.revokeObjectURL(previewRef.current.url);
    previewRef.current = null;
    if (mounted.current) setPreview(null);
  };
  const matches = (owner: Context, generation: number) => mounted.current && epoch.current === generation && current.current.enabled && current.current.token === owner.token && current.current.identity === owner.identity && current.current.api === owner.api;
  const valid = (ticket: Flight) => flight.current === ticket && !ticket.cancelled && matches(ticket.context, ticket.epoch) && Date.now() < ticket.deadline;
  const check = (ticket: Flight) => { if (!valid(ticket)) throw expired(); };
  useLayoutEffect(() => {
    mounted.current = true; ++epoch.current; setBusy(false); setError(''); clearPreview();
    return () => {
      mounted.current = false; ++epoch.current;
      const pending = flight.current; flight.current = null;
      if (pending) { pending.cancelled = true; abort(pending); }
      clearPreview();
    };
  }, [context.token, context.identity, context.enabled, context.api]);

  const save = async () => {
    const owner = current.current;
    if (!owner.enabled || !owner.token || flight.current || previewRef.current?.undoing) return;
    const ticket: Flight = { context: owner, epoch: epoch.current, deadline: Date.now() + 19_000, cancelled: false };
    const undoDeadline = performance.now() + 45_000;
    flight.current = ticket; setBusy(true); setError(''); clearPreview();
    let canvas: HTMLCanvasElement | undefined;
    try {
      canvas = freezeLiveFrame(video.current);
      const width = canvas.width, height = canvas.height;
      const blob = await encode(canvas); check(ticket);
      canvas.width = 0; canvas.height = 0;
      if (!blob.size || blob.size > MAX_BYTES || blob.type !== 'image/png') throw new Error('스크린샷 파일이 너무 크거나 올바르지 않습니다.');
      const begin = owner.api.begin(owner.token, crypto.randomUUID(), blob.size, width, height).then(value => {
        ticket.captureId = value.captureId;
        if (!valid(ticket)) abort(ticket);
        return value;
      });
      await beforeDeadline(begin, ticket.deadline); check(ticket);
      const bytes = new Uint8Array(await beforeDeadline(blob.arrayBuffer(), ticket.deadline)); check(ticket);
      for (let offset = 0, index = 0; offset < bytes.length; offset += CHUNK, index++) {
        check(ticket);
        await beforeDeadline(owner.api.chunk(owner.token, ticket.captureId!, index, base64(bytes.subarray(offset, offset + CHUNK))), ticket.deadline);
      }
      check(ticket);
      const saved = await beforeDeadline(owner.api.finish(owner.token, ticket.captureId!), ticket.deadline); check(ticket);
      const next: ScreenshotPreview = { url: URL.createObjectURL(blob), id: saved.id, token: owner.token, identity: owner.identity, api: owner.api, epoch: ticket.epoch, undoing: false, error: '', undoDeadline };
      previewRef.current = next; setPreview(next);
    } catch (cause) {
      abort(ticket);
      if (matches(owner, ticket.epoch) && !ticket.cancelled) setError(message(cause));
    } finally {
      if (canvas) { canvas.width = 0; canvas.height = 0; }
      if (flight.current === ticket) { flight.current = null; if (matches(owner, ticket.epoch)) setBusy(false); }
    }
  };
  const undo = async () => {
    const shown = previewRef.current;
    if (!shown || shown.undoing || shown.error || performance.now() >= shown.undoDeadline || !matches({ ...current.current, token: shown.token, identity: shown.identity, api: shown.api }, shown.epoch)) return;
    const pending = { ...shown, undoing: true }; previewRef.current = pending; setPreview(pending);
    try {
      await beforeDeadline(shown.api.undo(shown.token, shown.id), Date.now() + 10_000);
      if (previewRef.current === pending && matches(current.current, shown.epoch)) clearPreview();
    } catch {
      if (previewRef.current === pending && matches(current.current, shown.epoch)) {
        const failed = { ...pending, undoing: false, error: '취소를 확인하지 못했습니다. 스크린샷 목록에서 확인해 주세요.' };
        previewRef.current = failed; setPreview(failed);
        setError(failed.error);
      }
    }
  };
  const visible = context.enabled && preview?.token === context.token && preview.identity === context.identity && preview.api === context.api ? preview : null;
  return { busy: context.enabled && busy, error: context.enabled ? error : '', preview: visible, save, undo, dismiss: clearPreview, clearError: () => { setError(''); if (previewRef.current?.error) clearPreview(); } };
}
