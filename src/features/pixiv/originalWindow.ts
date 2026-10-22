import type { PixivApi, PixivDetail, PixivMedia } from "./model";

export const ORIGINAL_BYTES = 96 * 1024 * 1024;
export const PAGE_BYTES = 32 * 1024 * 1024;
export type OriginalImage = { state: "loading" | "ready" | "error"; url?: string; bytes?: number; error?: string; preview?: boolean };
export const pixivPageSource = (detail: PixivDetail, index: number) => detail.pages[index]?.previewUrl.startsWith("atsumi-pixiv:")
  ? detail.pages[index]!.previewUrl : `atsumi-pixiv-public:${detail.work.id}:${index}`;

/** Real indices only: visible first, then two original pages on each side. */
export function originalPageWindow(pages: (number | null)[], count: number) {
  const visible = [...new Set(pages.filter((page): page is number => page !== null && page >= 0 && page < count))];
  if (!visible.length) return { visible, prefetch: [] as number[] };
  const first = Math.min(...visible), last = Math.max(...visible);
  const prefetch = [first - 1, last + 1, first - 2, last + 2].filter(page => page >= 0 && page < count && !visible.includes(page));
  return { visible, prefetch };
}

const revoke = (url?: string) => { if (url?.startsWith("blob:")) URL.revokeObjectURL(url); };
function normalize(value: PixivMedia): { url: string; bytes: number; preview?: boolean } {
  if (typeof value !== "string") return value;
  // Unknown Blob sizes are charged the native maximum, never the URL length.
  const payload = value.startsWith("data:") ? value.slice(value.indexOf(",") + 1) : null;
  return { url: value, bytes: payload === null ? PAGE_BYTES : Math.ceil(payload.length * 3 / 4) };
}

/** Owns encoded originals, not decoded gallery-sized image elements. */
export function createOriginalWindow(api: Pick<PixivApi, "localMedia" | "pageMedia">) {
  const cache = new Map<string, OriginalImage>();
  const running = new Map<string, number>();
  const listeners = new Set<() => void>();
  let allowed: string[] = [], visible = new Set<string>(), queue: string[] = [];
  let epoch = 0, active = 0, revision = 0;
  const emit = () => { revision++; for (const listener of listeners) listener(); };
  const remove = (key: string) => { revoke(cache.get(key)?.url); cache.delete(key); };
  const used = () => [...cache.values()].reduce((sum, image) => sum + (image.bytes ?? 0), 0);
  const read = async (key: string) => {
    const match = /^atsumi-pixiv(-public)?:([1-9]\d{0,11}):(\d{1,2})$/.exec(key);
    if (!match) throw new Error("원본 위치를 확인할 수 없습니다.");
    return normalize(await (match[1] ? api.pageMedia(match[2]!, Number(match[3])) : api.localMedia(match[2]!, Number(match[3]), true)));
  };
  const pump = () => {
    while (active < 2 && queue.length) {
      const key = queue.shift()!;
      if (!allowed.includes(key) || running.get(key) === epoch) continue;
      const requestEpoch = epoch; running.set(key, requestEpoch); active++;
      cache.set(key, { state: "loading" });
      void read(key).then(value => {
        if (requestEpoch !== epoch || !allowed.includes(key)) { revoke(value.url); return; }
        if (!Number.isSafeInteger(value.bytes) || value.bytes < 1 || value.bytes > PAGE_BYTES) { revoke(value.url); throw new Error("원본 표시 용량을 초과했습니다."); }
        // Evict farthest speculative images first, never a currently visible page.
        for (const candidate of [...allowed].reverse()) {
          if (used() + value.bytes <= ORIGINAL_BYTES) break;
          if (!visible.has(candidate) && candidate !== key && cache.get(candidate)?.state === "ready") remove(candidate);
        }
        if (used() + value.bytes > ORIGINAL_BYTES) { revoke(value.url); cache.delete(key); return; }
        cache.set(key, { state: "ready", ...value });
      }).catch(reason => {
        const message = typeof reason === "string" ? reason : reason instanceof Error ? reason.message : "";
        // Do not reflect raw service errors, URLs or credentials into the UI.
        const safe = message.startsWith("PIXIV_DISPLAY_LIMIT:") ? "원본이 표시 가능한 크기를 초과합니다. 다운로드한 원본은 그대로 보존됩니다."
          : message.includes("로그인 상태가 변경") ? "로그인 상태가 변경되었습니다. 작품을 다시 열어 주세요."
          : message.includes("로그인 또는 열람 설정") ? "로그인 또는 열람 설정이 필요한 작품입니다."
          : message.includes("이미지 크기가 일치") ? "작품 정보와 이미지 크기가 다릅니다. 작품을 다시 열어 주세요."
          : message.includes("HTTP 429") ? "Pixiv 요청이 많습니다. 잠시 후 다시 시도해 주세요."
          : "원본을 불러오지 못했습니다.";
        if (requestEpoch === epoch && allowed.includes(key)) cache.set(key, { state: "error", error: safe });
      }).finally(() => {
        if (running.get(key) === requestEpoch) running.delete(key);
        active--; emit(); pump();
      });
    }
  };
  return {
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    snapshot: () => revision,
    get: (key: string) => cache.get(key),
    setWindow(nextVisible: string[], prefetch: string[]) {
      visible = new Set(nextVisible.slice(0, 2));
      allowed = [...new Set([...visible, ...prefetch])].slice(0, 6);
      for (const key of cache.keys()) if (!allowed.includes(key)) remove(key);
      queue = allowed.filter(key => !cache.has(key) && running.get(key) !== epoch);
      pump(); emit();
    },
    retry(key: string) {
      if (!allowed.includes(key) || running.get(key) === epoch) return;
      remove(key); queue = [key, ...queue.filter(value => value !== key)]; pump(); emit();
    },
    failedImage(key: string) {
      if (!allowed.includes(key) || cache.get(key)?.state !== "ready") return;
      remove(key); cache.set(key, { state: "error", error: "이미지를 표시하지 못했습니다." }); emit();
    },
    clear() {
      epoch++; allowed = []; visible.clear(); queue = [];
      for (const key of cache.keys()) remove(key);
      emit();
    },
  };
}
export type OriginalWindow = ReturnType<typeof createOriginalWindow>;
