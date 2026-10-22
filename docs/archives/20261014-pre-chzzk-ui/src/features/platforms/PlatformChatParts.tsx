import { useEffect, useRef, useState } from 'react';
import type { MediaChatAsset, MediaChatPart } from './platformMedia';
import './PlatformChatParts.css';

const ID = /^[a-f0-9]{64}$/;
const DATA = /^data:image\/(?:png|jpeg|gif|webp);base64,[A-Za-z0-9+/]+=*$/;
const MAX_RAM = 8 * 1024 * 1024;
export function safeChatImage(value: unknown): value is string {
  return typeof value === 'string' && value.length <= 1_400_000 && DATA.test(value);
}
/** Local opaque-ID reads only. Two workers, bounded cache, no request on privacy/inactive.
 * Live pending entries are scheduled fairly without holding IPC slots while waiting.
 * Completed replay misses never cause a network recovery request. */
export function useChatAssets(scope: string, rows: ReadonlyArray<{ parts?: MediaChatPart[] }>, enabled: boolean,
  load: ((scope: string, id: string) => Promise<MediaChatAsset | string | null>) | undefined, live = false) {
  const [images, setImages] = useState<ReadonlyMap<string, string>>(new Map());
  const cache = useRef({ scope: '', bytes: 0, images: new Map<string, string>(), attempts: new Map<string, { started: number; due: number; count: number; done: boolean }>() });
  const inFlight = useRef(0), drainLatest = useRef<() => void>(() => {});
  const ids = [...new Set(rows.flatMap(row => (row.parts ?? []).slice(0, 128)
    .flatMap(part => part.type === 'emoji' && ID.test(part.assetId) ? [part.assetId] : [])))].slice(-256).join(',');
  useEffect(() => {
    if (cache.current.scope !== scope) { cache.current = { scope, bytes: 0, images: new Map(), attempts: new Map() }; setImages(new Map()); }
    if (!enabled || !load || !scope) return;
    const current = cache.current;
    let stopped = false;
    const wanted = ids.split(',').filter(Boolean), busy = new Set<string>();
    let timer: ReturnType<typeof setTimeout> | undefined;
    for (const id of wanted) if (!current.attempts.has(id)) current.attempts.set(id, { started: Date.now(), due: 0, count: 0, done: current.images.has(id) });
    for (const id of current.attempts.keys()) if (current.attempts.size > 512 && !wanted.includes(id)) current.attempts.delete(id);
    const request = async (id: string) => {
      inFlight.current++; busy.add(id);
      const attempt = current.attempts.get(id)!;
      let result: MediaChatAsset | string | null = null;
      try { result = await load(scope, id); } catch { result = live ? { state: 'pending', dataUrl: null } : null; }
      if (!stopped) {
        const value = typeof result === 'string' ? result : result?.state === 'ready' ? result.dataUrl : null;
        attempt.count++;
        attempt.done = !(live && typeof result === 'object' && result?.state === 'pending' && Date.now() - attempt.started < 360_000);
        attempt.due = Date.now() + ([1000, 2000, 5000][attempt.count - 1] ?? 10_000);
        if (safeChatImage(value)) {
          current.images.set(id, value); current.bytes += value.length;
          while (current.bytes > MAX_RAM || current.images.size > 256) { const first = current.images.keys().next().value!; current.bytes -= current.images.get(first)!.length; current.images.delete(first); }
          setImages(new Map(current.images));
        }
      }
      busy.delete(id); inFlight.current--; drainLatest.current();
    };
    const drain = () => {
      if (timer !== undefined) clearTimeout(timer);
      if (stopped) return;
      const now = Date.now();
      const pending = wanted.filter(id => !busy.has(id) && !current.attempts.get(id)!.done).sort((a, b) => current.attempts.get(a)!.due - current.attempts.get(b)!.due);
      for (const id of pending) {
        const attempt = current.attempts.get(id)!;
        if (now - attempt.started >= 360_000) { attempt.done = true; continue; }
        if (inFlight.current >= 2) break;
        if (attempt.due > now) { timer = setTimeout(drain, attempt.due - now); break; }
        void request(id);
      }
    };
    drainLatest.current = drain;
    drainLatest.current();
    return () => { stopped = true; drainLatest.current = () => {}; if (timer !== undefined) clearTimeout(timer); };
  }, [scope, ids, enabled, load, live]);
  return cache.current.scope === scope ? images : new Map<string, string>();
}
function Emoji({ src, alt }: { src?: string; alt: string }) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [src]);
  return src && !failed && safeChatImage(src)
    ? <img className="platform-chat-emoji" src={src} alt={alt || '[이모티콘]'} title={alt || '이모티콘'} onError={() => setFailed(true)} draggable={false} />
    : <span className="platform-chat-emoji-fallback" title="이미지를 불러오지 못하면 문자로 표시합니다.">{alt || '[이모티콘]'}</span>;
}
export function PlatformChatParts({ text, parts, images }: { text: string; parts?: MediaChatPart[]; images: ReadonlyMap<string, string> }) {
  if (!parts?.length) return <>{text}</>;
  return <>{parts.slice(0, 128).map((part, index) => part.type === 'text'
    ? <span key={index}>{part.text}</span>
    : part.type === 'emoji' ? <Emoji key={index} src={ID.test(part.assetId) ? images.get(part.assetId) : undefined} alt={part.alt} /> : null)}</>;
}
