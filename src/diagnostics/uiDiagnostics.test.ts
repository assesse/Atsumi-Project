import { afterEach, describe, expect, it, vi } from "vitest";
import { imageCreated, imageReleased, installUiDiagnostics, uiCounters } from "./uiDiagnostics";
afterEach(() => vi.useRealTimers());
describe("UI flight recorder", () => {
  it("does not accumulate heartbeat requests while the bridge is blocked", async () => {
    vi.useFakeTimers();
    const call = vi.fn(async (command: string) => {
      if (command === "ui_diagnostics_session") return "doc-1";
      if (command === "ui_diagnostics_pulse") return new Promise(() => undefined);
    });
    const stop = installUiDiagnostics(true, call as never);
    await vi.advanceTimersByTimeAsync(90_000);
    expect(call.mock.calls.filter(([c]) => c === "ui_diagnostics_pulse")).toHaveLength(1);
    const pulse = (call.mock.calls.find(([c]) => c === "ui_diagnostics_pulse") as unknown as [string, { pulse: object }])[1].pulse;
    expect(JSON.stringify(pulse)).not.toMatch(/search|cookie|password|https:/i);
    stop();
  });
  it("never creates a native watchdog in the browser fixture", async () => {
    vi.useFakeTimers(); const call = vi.fn(); const stop = installUiDiagnostics(false, call);
    await vi.advanceTimersByTimeAsync(90_000); expect(call).not.toHaveBeenCalled(); stop();
  });
  it("counts live image bodies without retaining the bodies or double releasing", () => {
    const initial = { ...uiCounters };
    imageCreated("blob:test-health", 1024); expect(uiCounters.thumbnailLiveBytes).toBe(initial.thumbnailLiveBytes + 1024);
    imageReleased("blob:test-health"); imageReleased("blob:test-health");
    expect(uiCounters.thumbnailLiveBytes).toBe(initial.thumbnailLiveBytes);
    expect(uiCounters.thumbnailLiveCount).toBe(initial.thumbnailLiveCount);
  });
});
