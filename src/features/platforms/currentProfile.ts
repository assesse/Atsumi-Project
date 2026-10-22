import type { PlatformMediaApi } from './platformMedia';
const valid = (value: unknown): value is string => typeof value === 'string' && value.length <= 180_000 && /^data:image\/(?:png|jpeg|gif|webp);base64,[A-Za-z0-9+/]+={0,2}$/.test(value);
type Result = { image: string; current: boolean } | null;
type Entry = { id: string; listeners: Set<(result: Result) => void>; started: boolean };
type Store = { pending: Map<string, Entry>; queue: Entry[]; running: number };
const stores = new WeakMap<PlatformMediaApi, Store>();
/** Card-only lazy reads. A corrupt local proof must not be hidden by a current image. */
export function subscribeCardProfile(api: PlatformMediaApi, id: string, listener: (result: Result) => void) {
  let store = stores.get(api);
  if (!store) { store = { pending: new Map(), queue: [], running: 0 }; stores.set(api, store); }
  const current = store;
  let entry = current.pending.get(id);
  if (!entry) {
    if (current.pending.size >= 128) { listener(null); return () => {}; }
    entry = { id, listeners: new Set(), started: false }; current.pending.set(id, entry); current.queue.push(entry);
  }
  const subscribed = entry; entry.listeners.add(listener);
  const pump = () => {
    while (current.running < 2 && current.queue.length) {
      const next = current.queue.shift()!;
      if (!next.listeners.size || current.pending.get(next.id) !== next) continue;
      next.started = true; current.running++;
      const read = async (): Promise<Result> => {
        const local = await api.profile?.(next.id) ?? null;
        if (valid(local)) return { image: local, current: false };
        // Non-null malformed proof is not an absent historical snapshot.
        if (local !== null || !next.listeners.size) return null;
        const now = await api.currentProfile?.(next.id);
        return valid(now) ? { image: now, current: true } : null;
      };
      void read().catch(() => null).then(value => { for (const callback of next.listeners) callback(value); }).finally(() => {
        if (current.pending.get(next.id) === next) current.pending.delete(next.id);
        current.running--; pump();
      });
    }
  };
  pump();
  return () => {
    subscribed.listeners.delete(listener);
    if (!subscribed.started && !subscribed.listeners.size && current.pending.get(id) === subscribed) {
      current.pending.delete(id); const at = current.queue.indexOf(subscribed); if (at >= 0) current.queue.splice(at, 1);
    }
  };
}
