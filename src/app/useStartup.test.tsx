import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { useStartup } from "./useStartup";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const request = vi.mocked(invoke);
let root: Root;
let container: HTMLDivElement;
let state: ReturnType<typeof useStartup>;
function Probe({ ready, runtime = "tauri" }: { ready: boolean; runtime?: "tauri" | "browser-mock" }) {
  state = useStartup(runtime, ready);
  return <button onClick={() => void state.cancel()}>cancel</button>;
}
async function render(ready: boolean, runtime?: "tauri" | "browser-mock") {
  await act(async () => root.render(<Probe ready={ready} runtime={runtime} />));
}
async function advance(ms: number) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }

describe("startup readiness", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    request.mockReset();
    request.mockResolvedValue({ phase: "ready", elapsedMs: 500 });
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(performance.now()), 16));
    vi.stubGlobal("cancelAnimationFrame", window.clearTimeout.bind(window));
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });
  it("never counts a loading settings shell as interactive", async () => {
    await render(false);
    await advance(300);
    expect(request).toHaveBeenCalledWith("app_startup_frame", { settingsReady: false });
    expect(request).not.toHaveBeenCalledWith("app_startup_frame", { settingsReady: true });
    expect(state.backgroundReady).toBe(false);
  });
  it("waits for committed frames and native acknowledgement before bulk hydration", async () => {
    await render(true);
    expect(state.backgroundReady).toBe(false);
    await advance(32);
    expect(request).toHaveBeenCalledWith("app_startup_frame", { settingsReady: true });
    expect(state.backgroundReady).toBe(false);
    await advance(150);
    expect(state.backgroundReady).toBe(true);
  });
  it("keeps failure visible and allows explicit cancellation", async () => {
    request.mockImplementation(async (command) => command === "app_startup_cancel" ? true : { phase: "failed", elapsedMs: 700 });
    await render(false);
    expect(state.phase).toBe("failed");
    await act(async () => { await state.cancel(); });
    expect(state.phase).toBe("cancelling");
    expect(state.backgroundReady).toBe(false);
  });
  it("does not touch desktop IPC in the browser mock", async () => {
    await render(true, "browser-mock");
    await advance(500);
    expect(state.phase).toBe("ready");
    expect(state.backgroundReady).toBe(true);
    expect(request).not.toHaveBeenCalled();
  });
  it("cancels pending frame callbacks on unmount", async () => {
    await render(true);
    await act(async () => root.unmount());
    root = createRoot(container);
    request.mockClear();
    await advance(500);
    expect(request).not.toHaveBeenCalled();
  });
  it("reports each frame once even when the window visibility changes repeatedly", async () => {
    await render(true);
    await advance(200);
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await advance(200);
    expect(request.mock.calls.filter(([cmd, args]) => cmd === "app_startup_frame" && (args as { settingsReady: boolean }).settingsReady)).toHaveLength(1);
    expect(request.mock.calls.filter(([cmd, args]) => cmd === "app_startup_frame" && !(args as { settingsReady: boolean }).settingsReady)).toHaveLength(1);
  });
});
