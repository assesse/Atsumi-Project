import { afterEach, describe, expect, it, vi } from "vitest";
import { imageCreated, imageReleased, installUiDiagnostics, markUi, uiCounters } from "./uiDiagnostics";
afterEach(() => vi.useRealTimers());
describe("UI flight recorder", () => {
  it("records frontend failures without messages, rejection reasons or URLs, and stops on disposal", async () => {
    vi.useFakeTimers();
    const call = vi.fn(async () => undefined);
    const add = vi.spyOn(window, "addEventListener"), remove = vi.spyOn(window, "removeEventListener");
    const stop = installUiDiagnostics(true, call as never);
    // Call our installed listener, not Vitest's own fatal-error handler.
    const onError = add.mock.calls.find(([name]) => name === "error")![1] as EventListener;
    const error = new ErrorEvent("error", { message: "secret-cookie", filename: "https://private.invalid/?token=secret", error: new Error("private title") });
    onError(error);
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    window.dispatchEvent(new Event("unhandledrejection"));
    await vi.advanceTimersByTimeAsync(1);
    markUi("react_error");
    await vi.advanceTimersByTimeAsync(1);
    const records = call.mock.calls as unknown as Array<[string, { mark?: string }]>;
    expect(records.some(([, args]) => args?.mark === "frontend_error")).toBe(true);
    expect(records.some(([, args]) => args?.mark === "unhandled_rejection")).toBe(true);
    expect(records.some(([, args]) => args?.mark === "react_error")).toBe(true);
    expect(JSON.stringify(records)).not.toMatch(/secret|private|https:/);
    stop(); const count = call.mock.calls.length;
    expect(remove).toHaveBeenCalledWith("error", onError, true);
    onError(error); window.dispatchEvent(new Event("unhandledrejection")); markUi("react_error");
    await vi.advanceTimersByTimeAsync(10_000);
    expect(call).toHaveBeenCalledTimes(count);
    add.mockRestore(); remove.mockRestore();
  });
  it("bounds repeated error reporting even if the native bridge stops answering", async () => {
    vi.useFakeTimers();
    const call = vi.fn((_command: string, args?: { mark?: string }) => args?.mark === "frontend_error" ? new Promise(() => undefined) : Promise.resolve("epoch"));
    const stop = installUiDiagnostics(true, call as never);
    for (let i = 0; i < 1000; i++) window.dispatchEvent(new Event("error"));
    await vi.advanceTimersByTimeAsync(10_000);
    window.dispatchEvent(new Event("error"));
    expect(call.mock.calls.filter(([, args]) => args?.mark === "frontend_error")).toHaveLength(1);
    stop();
  });
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
