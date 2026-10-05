import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { restoreNativeCheckpoint } from "./workConsole";
import { readNavigationCheckpoint, writeNavigationCheckpoint, type NavigationCheckpoint } from "../state/navigationCheckpoint";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("./backend", () => ({ backend: { runtime: "tauri" } }));
const sample = (): NavigationCheckpoint => ({ version: 1, savedAt: Date.now(), view: "explore", downloadsFilter: "all", activeTab: null, tabs: [] });
beforeEach(() => { vi.clearAllMocks(); sessionStorage.clear(); localStorage.clear(); writeNavigationCheckpoint(sample()); });
afterEach(() => { vi.useRealTimers(); sessionStorage.clear(); localStorage.clear(); });
describe("native navigation recovery", () => {
  it("discards stale webview recovery state only when native confirms a clean session", async () => {
    sessionStorage.setItem("unrelated", "keep");
    sessionStorage.setItem("atsumi.detail-positions.v1", "old-position");
    localStorage.setItem("atsumi.danbooru-state.v1", "old-search");
    localStorage.setItem("atsumi.danbooru-search-preferences.v1", "keep-settings");
    invoke.mockResolvedValue(null);
    await restoreNativeCheckpoint();
    expect(readNavigationCheckpoint()).toBeNull();
    expect(sessionStorage.getItem("unrelated")).toBe("keep");
    expect(sessionStorage.getItem("atsumi.detail-positions.v1")).toBeNull();
    expect(localStorage.getItem("atsumi.danbooru-state.v1")).toBeNull();
    expect(localStorage.getItem("atsumi.danbooru-search-preferences.v1")).toBe("keep-settings");
  });
  it("restores a crash checkpoint", async () => {
    const recovered = { ...sample(), savedAt: Date.now() + 1000, view: "downloads" as const };
    invoke.mockResolvedValue(recovered);
    localStorage.setItem("atsumi.danbooru-state.v1", "recover-search");
    await restoreNativeCheckpoint();
    expect(readNavigationCheckpoint()).toEqual(recovered);
    expect(localStorage.getItem("atsumi.danbooru-state.v1")).toBe("recover-search");
  });
  it("preserves session recovery when the bridge fails", async () => {
    invoke.mockRejectedValue(new Error("bridge unavailable"));
    await restoreNativeCheckpoint();
    expect(readNavigationCheckpoint()).not.toBeNull();
  });
  it("does not confuse a bridge timeout with a clean exit", async () => {
    vi.useFakeTimers();
    invoke.mockReturnValue(new Promise(() => {}));
    const pending = restoreNativeCheckpoint();
    await vi.advanceTimersByTimeAsync(1500);
    await pending;
    expect(readNavigationCheckpoint()).not.toBeNull();
  });
});
