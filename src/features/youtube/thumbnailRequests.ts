import { YouTubeThumbnailError, type YouTubeApi } from './api';

type Transport = NonNullable<YouTubeApi['thumbnail']>;
type Listener = (image: string | null) => void;
type Request = { id: string; listeners: Set<Listener>; running: boolean; attempts: number; timer?: ReturnType<typeof setTimeout> };
type Store = { cache: Map<string, { at: number; image: string }>; pending: Map<string, Request>; queue: Request[]; running: number };
// actionApi is a shallow API copy. Its transport, not its wrapper, is the owner.
const stores = new WeakMap<Transport, Store>();
const TTL = 10 * 60 * 1000;
export const isYouTubeImage = (value: unknown): value is string => typeof value === 'string' && value.length <= 180000 && /^data:image\/(?:png|jpeg|webp|gif);base64,[A-Za-z0-9+/]+={0,2}$/.test(value);

export function subscribeYouTubeThumbnail(transport: Transport, id: string, listener: Listener): () => void {
  let store = stores.get(transport);
  if (!store) { store = { cache: new Map(), pending: new Map(), queue: [], running: 0 }; stores.set(transport, store); }
  const current = store;
  const cached = current.cache.get(id);
  if (cached && Date.now() - cached.at < TTL) { listener(cached.image); return () => {}; }
  current.cache.delete(id);
  let request = current.pending.get(id);
  if (!request) {
    if (current.pending.size >= 128) { listener(null); return () => {}; }
    request = { id, listeners: new Set(), running: false, attempts: 0 };
    current.pending.set(id, request); current.queue.push(request);
  }
  const entry = request;
  entry.listeners.add(listener);
  const finish = (next: Request, image: string | null) => {
    if (current.pending.get(next.id) === next) current.pending.delete(next.id);
    if (image) {
      if (current.cache.size >= 128) current.cache.delete(current.cache.keys().next().value!);
      current.cache.set(next.id, { at: Date.now(), image });
    }
    for (const callback of next.listeners) callback(image);
    next.listeners.clear();
  };
  const pump = () => {
    while (current.running < 4 && current.queue.length) {
      const next = current.queue.shift()!;
      if (!next.listeners.size || current.pending.get(next.id) !== next) continue;
      next.running = true; next.attempts++; current.running++;
      void (async () => {
        try {
          const value = await transport(next.id);
          finish(next, isYouTubeImage(value) ? value : null);
        } catch (error) {
          if (error instanceof YouTubeThumbnailError && error.code === 'busy' && next.attempts <= 2 && next.listeners.size) {
            next.timer = setTimeout(() => {
              next.timer = undefined;
              if (next.listeners.size && current.pending.get(next.id) === next) { current.queue.push(next); pump(); }
            }, next.attempts === 1 ? 250 : 750);
          } else finish(next, null);
        } finally {
          next.running = false; current.running--; pump();
        }
      })();
    }
  };
  pump();
  return () => {
    entry.listeners.delete(listener);
    if (!entry.running && !entry.listeners.size && current.pending.get(id) === entry) {
      if (entry.timer !== undefined) clearTimeout(entry.timer);
      current.pending.delete(id);
      const at = current.queue.indexOf(entry); if (at >= 0) current.queue.splice(at, 1);
    }
  };
}
