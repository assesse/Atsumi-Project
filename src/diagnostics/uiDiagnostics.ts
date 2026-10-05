import { invoke } from "@tauri-apps/api/core";

/** Counters only: no input text, URLs, thumbnail bodies or gallery metadata. */
export const uiCounters = {
  thumbnailReceived: 0, thumbnailLiveBytes: 0, thumbnailLiveCount: 0,
  downloadReceived: 0, downloadNotifications: 0,
};
const blobSizes = new Map<string, number>();
export function imageCreated(url: string, bytes: number): void {
  imageReleased(url);
  blobSizes.set(url, bytes); uiCounters.thumbnailLiveBytes += bytes;
  uiCounters.thumbnailLiveCount = blobSizes.size;
}
export function imageReleased(url: string): void {
  uiCounters.thumbnailLiveBytes -= blobSizes.get(url) ?? 0;
  blobSizes.delete(url); uiCounters.thumbnailLiveCount = blobSizes.size;
}
type Invoke = typeof invoke;
type UiMark = "f5_results" | "frontend_ready" | "frontend_error" | "unhandled_rejection" | "react_error";
let emitMark: ((mark: UiMark) => void) | undefined;
export function markUi(mark: UiMark): void { emitMark?.(mark); }

/** Only one heartbeat may be in flight. A blocked bridge cannot create its
 * own unbounded queue. The native foreground watchdog owns recovery. */
export function installUiDiagnostics(enabled: boolean, call: Invoke = invoke): () => void {
  if (!enabled) return () => undefined;
  let disposed = false, pending = false, epoch = "", inputCount = 0;
  let markPending = false;
  const lastError = new Map<UiMark, number>();
  const sendMark = (mark: UiMark) => {
    if (disposed || markPending) return;
    const now = performance.now();
    if (now - (lastError.get(mark) ?? -Infinity) < 1000) return;
    lastError.set(mark, now); markPending = true;
    // No Error.message/stack, event filename, URL or rejection reason crosses IPC.
    void call("ui_diagnostics_mark", { mark }).catch(() => undefined).finally(() => { markPending = false; });
  };
  emitMark = sendMark;
  const onError = () => sendMark("frontend_error");
  const onRejection = () => sendMark("unhandled_rejection");
  window.addEventListener("error", onError, true);
  window.addEventListener("unhandledrejection", onRejection);
  let longTasks = 0, longestTaskMs = 0, expected = performance.now() + 2000;
  const input = () => { inputCount++; };
  window.addEventListener("pointerdown", input, true);
  window.addEventListener("keydown", input, true);
  let observer: PerformanceObserver | undefined;
  try {
    observer = new PerformanceObserver((list) => {
      for (const task of list.getEntries()) {
        longTasks++; longestTaskMs = Math.max(longestTaskMs, Math.round(task.duration));
      }
    });
    observer.observe({ type: "longtask", buffered: false });
  } catch { /* Runtime support varies. Native private-memory samples remain. */ }
  const tick = async () => {
    const lagMs = Math.max(0, Math.round(performance.now() - expected));
    expected = performance.now() + 2000;
    if (disposed || pending) return;
    pending = true;
    try {
      if (!epoch) epoch = await call<string>("ui_diagnostics_session");
      if (disposed) return;
      const memory = (performance as Performance & { memory?: { usedJSHeapSize: number } }).memory;
      const accepted = await call<boolean>("ui_diagnostics_pulse", { pulse: {
        epoch, visible: document.visibilityState === "visible", lagMs,
        jsHeapBytes: memory?.usedJSHeapSize ?? null, inputCount, longTasks, longestTaskMs,
        ...uiCounters,
      } });
      if (!accepted) epoch = "";
    } catch { /* Best effort; do not log one error per timer into the UI. */ }
    finally { pending = false; }
  };
  const timer = window.setInterval(() => { void tick(); }, 2000);
  void call("ui_diagnostics_mark", { mark: "frontend_ready" }).catch(() => undefined);
  return () => {
    disposed = true; if (emitMark === sendMark) emitMark = undefined;
    window.clearInterval(timer); observer?.disconnect();
    window.removeEventListener("pointerdown", input, true); window.removeEventListener("keydown", input, true);
    window.removeEventListener("error", onError, true); window.removeEventListener("unhandledrejection", onRejection);
  };
}
