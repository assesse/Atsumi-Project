import type { SoopApi } from "../../api/soop";

// SOOP covers and portraits share admission. This state never services another provider.
let workers = 0;
const queue: (() => void)[] = [];
export function scheduleSoopImage(task: () => Promise<void>): (() => void) | undefined {
  const start = () => {
    workers++;
    void Promise.resolve().then(task).catch(() => {}).finally(() => { workers--; queue.shift()?.(); });
  };
  if (workers < 2) start();
  else if (queue.length < 128) queue.push(start);
  else return undefined;
  return () => { const at = queue.indexOf(start); if (at >= 0) queue.splice(at, 1); };
}

type Entry = { image: string; expires: number; pending: boolean; started: boolean; listeners: Set<(image: string) => void>; cancel?: () => void };
const caches = new WeakMap<SoopApi["inspect"], Map<string, Entry>>();
export function subscribeSoopPortrait(api: SoopApi, channelId: string, listener: (image: string) => void): () => void {
  if (!/^[A-Za-z0-9_-]{1,40}$/.test(channelId)) return () => {};
  const id = channelId.toLowerCase();
  let cache = caches.get(api.inspect);
  if (!cache) { cache = new Map(); caches.set(api.inspect, cache); }
  let entry = cache.get(id);
  if (entry && !entry.pending && entry.expires <= Date.now()) { cache.delete(id); entry = undefined; }
  if (entry) { cache.delete(id); cache.set(id, entry); }
  if (!entry) {
    while (cache.size >= 128) {
      // Mounted subscribers already own the delivered image. They must not pin
      // settled cache entries forever as the user scrolls through a long feed.
      const candidate = [...cache].find(([, value]) => !value.pending);
      if (!candidate) return () => {};
      cache.delete(candidate[0]);
    }
    entry = { image: "", expires: 0, pending: true, started: false, listeners: new Set() };
    cache.set(id, entry);
  }
  const current = entry;
  current.listeners.add(listener);
  if (!current.pending) listener(current.image);
  else if (!current.cancel) {
    current.cancel = scheduleSoopImage(async () => {
      if (!current.listeners.size) return;
      current.started = true;
      try {
        const result = await api.inspect(id);
        if (result?.channelId.toLowerCase() === id && result.image && result.image.length <= 180_000
          && /^data:image\/(?:png|jpeg|gif|webp);base64,[A-Za-z0-9+/]+={0,2}$/.test(result.image)) current.image = result.image;
      } catch { /* A failed portrait is not a failed content card. */ }
      current.pending = false;
      current.expires = Date.now() + (current.image ? 600_000 : 30_000);
      current.listeners.forEach(notify => notify(current.image));
    });
    if (!current.cancel) { current.pending = false; current.expires = Date.now() + 1_000; }
  }
  return () => {
    current.listeners.delete(listener);
    if (current.pending && !current.started && !current.listeners.size) {
      current.cancel?.();
      if (cache!.get(id) === current) cache!.delete(id);
    }
  };
}
