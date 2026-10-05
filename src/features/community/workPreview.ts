import { backend } from "../../api/backend";
import type { DanbooruApi } from "../../api/featureClients";

// Small, session-only metadata cache. Concurrent reviews of one work share a
// request, and a long review feed cannot start an unbounded burst of requests.
export function createDanbooruPreviewLoader(api: Pick<DanbooruApi, "danbooruSearch">) {
  const cache = new Map<string, { promise: Promise<string | null>; expires: number }>();
  const queue: (() => void)[] = [];
  let running = 0;
  const drain = () => { while (running < 3 && queue.length) queue.shift()!(); };
  return (workId: string): Promise<string | null> => {
    const existing = cache.get(workId);
    if (existing && existing.expires > Date.now()) return existing.promise;
    const item = { promise: null as unknown as Promise<string | null>, expires: Infinity };
    item.promise = new Promise<string | null>((resolve) => {
      queue.push(() => {
        running++;
        void api.danbooruSearch({ tags: `id:${workId}`, page: 1, pageSize: 1 }).then((result) => {
          const post = result.ok ? result.data.items.find((post) => String(post.id) === workId) : null;
          const url = post?.previewUrl;
          resolve(url || null);
          item.expires = Date.now() + (url ? 10 * 60_000 : 30_000);
        }).catch(() => { item.expires = Date.now() + 30_000; resolve(null); }).finally(() => {
          running--;
          if (cache.size > 128) {
            for (const [key, entry] of cache) {
              if (entry.expires !== Infinity) cache.delete(key);
              if (cache.size <= 128) break;
            }
          }
          drain();
        });
      });
    });
    cache.set(workId, item);
    drain();
    return item.promise;
  };
}

export const loadDanbooruPreview = createDanbooruPreviewLoader(backend);
