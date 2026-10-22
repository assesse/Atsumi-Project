import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { PixivAccountSnapshot, PixivApi, PixivDetail, PixivDiscovery, PixivDownload, PixivDownloadOutcome, PixivProgress, PixivSearch } from "./model";

export function createPixivApi(runtime: "tauri" | "browser-mock"): PixivApi {
  const call = <T,>(command: string, args?: Record<string, unknown>): Promise<T> => runtime === "tauri"
    ? invoke<T>(command, args)
    : Promise.reject(new Error("Pixiv 연결은 Atsumi 데스크톱 앱에서 사용할 수 있습니다."));
  return {
    account: () => call<PixivAccountSnapshot>("pixiv_account_status"),
    login: () => call<PixivAccountSnapshot>("pixiv_account_login"),
    logout: () => call<PixivAccountSnapshot>("pixiv_account_logout"),
    search: (request) => call<PixivSearch>("pixiv_search", { request }),
    discover: (request) => call<PixivDiscovery>("pixiv_discovery", { request }),
    detail: (input) => call<PixivDetail>("pixiv_work", { input }),
    localDetail: (id) => call<PixivDetail>("pixiv_local_work", { id }),
    localMedia: async (id, page, detail = false) => {
      const bytes = await call<ArrayBuffer>("pixiv_local_media", { id, page, detail });
      return { url: URL.createObjectURL(new Blob([bytes])), bytes: bytes.byteLength };
    },
    pageMedia: async (id, page) => {
      let bytes: ArrayBuffer, preview = false;
      try { bytes = await call<ArrayBuffer>("pixiv_page_media", { id, page }); }
      catch (error) {
        const message = typeof error === "string" ? error : error instanceof Error ? error.message : "";
        // Only the native size classification permits a display fallback. A
        // login, permission, corruption or transport error must remain an error.
        if (!message.startsWith("PIXIV_DISPLAY_LIMIT:")) throw error;
        bytes = await call<ArrayBuffer>("pixiv_page_preview", { id, page }); preview = true;
      }
      return { url: URL.createObjectURL(new Blob([bytes])), bytes: bytes.byteLength, ...(preview ? { preview: true } : {}) };
    },
    media: (url, detail = false) => call<string>("pixiv_media", { url, detail }),
    download: (id) => call<PixivDownload>("pixiv_download", { id }),
    downloadBatch: (ids) => call<PixivDownloadOutcome[]>("pixiv_download_batch", { ids }),
    subscribeDownloads: runtime === "tauri" ? listener => listen<PixivDownloadOutcome>("pixiv-download-result", event => listener(event.payload)) : undefined,
    downloads: () => call<PixivDownload[]>("pixiv_downloads_list"),
    progress: () => call<PixivProgress[]>("pixiv_download_progress"),
    cancel: (id) => call<void>("pixiv_download_cancel", { id }),
    open: (id) => call<void>("pixiv_open", { id }),
    libraryAction: (id, action, token) => call("pixiv_library_action", { id, action, token }),
  };
}

// Small shared LRU, not an unbounded gallery-sized base64 cache. Both the native
// transport and this queue limit simultaneous images; hidden cards do not fetch.
export function createPixivMediaLoader(api: Pick<PixivApi, "media"> & Partial<Pick<PixivApi, "localMedia" | "pageMedia">>) {
  const cache = new Map<string, string>();
  const pending = new Map<string, Promise<string>>();
  const queue: (() => void)[] = [];
  let running = 0, cachedChars = 0, epoch = 0;
  const pump = () => { while (running < 4 && queue.length) { running++; queue.shift()?.(); } };
  const discard = (key: string, value: string) => { cache.delete(key); cachedChars -= value.length; if (value.startsWith("blob:")) URL.revokeObjectURL(value); };
  const load = (url: string, detail = false) => {
    const requestEpoch = epoch;
    const key = `${detail ? "detail" : "thumb"}:${requestEpoch}:${url}`;
    const found = cache.get(key);
    if (found) { cache.delete(key); cache.set(key, found); return Promise.resolve(found); }
    const existing = pending.get(key); if (existing) return existing;
    if (pending.size >= 128) return Promise.reject(new Error("이미지 작업이 많습니다. 잠시 후 다시 시도해 주세요."));
    const promise = new Promise<string>((resolve, reject) => {
      queue.push(() => {
        if (requestEpoch !== epoch) { reject(new Error("이미지 요청을 취소했습니다.")); pending.delete(key); running--; pump(); return; }
        const local = /^atsumi-pixiv:([1-9]\d{0,11}):(\d{1,2})$/.exec(url);
        const original = /^atsumi-pixiv-public:([1-9]\d{0,11}):(\d{1,2})$/.exec(url);
        const request = local ? api.localMedia?.(local[1]!, Number(local[2]), detail) ?? Promise.reject(new Error("저장된 이미지를 열 수 없습니다.")) : original ? api.pageMedia?.(original[1]!, Number(original[2])) ?? Promise.reject(new Error("원본을 열 수 없습니다.")) : api.media(url, detail);
        void request.then(result => {
          const value = typeof result === "string" ? result : result.url;
          if (requestEpoch !== epoch) { if (value.startsWith("blob:")) URL.revokeObjectURL(value); reject(new Error("이미지 요청을 취소했습니다.")); return; }
          if (value.length <= 8 * 1024 * 1024) {
            cache.set(key, value); cachedChars += value.length;
            // At most the current original and two neighbours on either side.
            while ([...cache.keys()].filter(item => item.startsWith("detail:")).length > 5) {
              const oldest = [...cache.entries()].find(([item]) => item.startsWith("detail:"));
              if (oldest) discard(oldest[0], oldest[1]);
            }
            while (cache.size > 32 || cachedChars > 16 * 1024 * 1024) {
              const first = cache.entries().next().value;
              if (!first) break; discard(first[0], first[1]);
            }
          }
          resolve(value);
        }, reject).finally(() => { pending.delete(key); running--; pump(); });
      });
    });
    pending.set(key, promise); pump(); return promise;
  };
  load.dispose = () => { epoch++; for (const [key, value] of cache) discard(key, value); };
  return load;
}
