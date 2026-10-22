import { hasKanji, safeMetadataText } from "./japaneseReading";

export type LocalReading = { text: string; partial: boolean };
const IDLE_MS = 20000, TIMEOUT_MS = 15000, CACHE_LIMIT = 128;
type Waiter = { resolve: (result: LocalReading | undefined) => void; text: string; timer: ReturnType<typeof setTimeout>; detach: () => void };

export function createReadingService(factory: () => Worker = () => new Worker(new URL("./reading.worker.ts", import.meta.url), { type: "module" })) {
  let worker: Worker | undefined, idle: ReturnType<typeof setTimeout> | undefined, nextId = 0, cooldown = 0;
  const waiting = new Map<number, Waiter>(), cache = new Map<string, LocalReading>();
  const stop = () => { worker?.terminate(); worker = undefined; clearTimeout(idle); idle = undefined; };
  const finish = (id: number, result?: LocalReading) => {
    const row = waiting.get(id); if (!row) return;
    clearTimeout(row.timer); row.detach(); waiting.delete(id); row.resolve(result);
    if (!waiting.size) { clearTimeout(idle); idle = setTimeout(stop, IDLE_MS); }
  };
  const failed = () => { cooldown = Date.now() + 30000; stop(); for (const id of waiting.keys()) finish(id); };
  const clear = () => { for (const id of waiting.keys()) finish(id); stop(); cache.clear(); };
  const read = (text: string, signal?: AbortSignal): Promise<LocalReading | undefined> => {
    if (signal?.aborted || !safeMetadataText(text) || !hasKanji(text) || Date.now() < cooldown) return Promise.resolve(undefined);
    // There is only one foreground tooltip; a newer request supersedes an older hover.
    for (const id of waiting.keys()) finish(id);
    const previous = cache.get(text);
    if (previous) { cache.delete(text); cache.set(text, previous); return Promise.resolve(previous); }
    clearTimeout(idle);
    if (!worker) {
      try { worker = factory(); } catch { cooldown = Date.now() + 30000; return Promise.resolve(undefined); }
      worker.onerror = failed; worker.onmessageerror = failed;
      worker.onmessage = event => {
        const { id, result, failed: didFail } = event.data ?? {};
        const row = waiting.get(id); if (!row) return;
        if (didFail) { failed(); return; }
        if (!result || !safeMetadataText(result.text, 1600) || typeof result.partial !== "boolean") { finish(id); return; }
        cache.set(row.text, result);
        while (cache.size > CACHE_LIMIT) cache.delete(cache.keys().next().value!);
        finish(id, result);
      };
    }
    return new Promise(resolve => {
      const id = ++nextId, abort = () => finish(id);
      const timer = setTimeout(failed, TIMEOUT_MS);
      waiting.set(id, { resolve, text, timer, detach: () => signal?.removeEventListener("abort", abort) });
      signal?.addEventListener("abort", abort, { once: true });
      try { worker!.postMessage({ id, text }); } catch { failed(); }
    });
  };
  return { read, clear };
}

export const pixivReadings = createReadingService();
