import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SettingsPatch } from "../api/contracts";
import { useSettingsAutosave } from "./useSettingsAutosave";

let root: Root, value: ReturnType<typeof useSettingsAutosave>;
const deferred = () => { let resolve!: (ok: boolean) => void; const promise = new Promise<boolean>(done => { resolve = done; }); return { promise, resolve }; };
const mount = async (save: (patch: SettingsPatch) => Promise<boolean>) => {
  function Probe() { value = useSettingsAutosave(save); return null; }
  await act(async () => root.render(<Probe />));
};
beforeEach(() => { vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.useFakeTimers(); root = createRoot(document.createElement("div")); });
afterEach(async () => { await act(async () => root.unmount()); vi.useRealTimers(); vi.unstubAllGlobals(); });

describe("settings autosave", () => {
  it("never saves untouched defaults and coalesces a slider drag", async () => {
    const save = vi.fn(async () => true); await mount(save);
    expect(save).not.toHaveBeenCalled();
    await act(async () => { value.enqueue({ previewWidth: 190 }, 180); value.enqueue({ previewWidth: 280 }, 180); await vi.advanceTimersByTimeAsync(179); });
    expect(save).not.toHaveBeenCalled();
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(save).toHaveBeenCalledExactlyOnceWith({ previewWidth: 280 });
    expect(value.status).toBe("saved");
  });
  it("serializes slow writes without losing later edits and flushes before close", async () => {
    const first = deferred();
    const save = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue(true); await mount(save);
    await act(async () => value.enqueue({ privacyOnStartup: false }));
    await act(async () => { value.enqueue({ previewWidth: 280 }, 180); value.enqueue({ explorePageSize: 80 }, 180); });
    expect(save).toHaveBeenCalledTimes(1);
    expect(value.unsaved()).toEqual({ privacyOnStartup: false, previewWidth: 280, explorePageSize: 80 });
    await act(async () => { const closed = value.flush(); first.resolve(true); await closed; });
    expect(save.mock.calls).toEqual([[{ privacyOnStartup: false }], [{ previewWidth: 280, explorePageSize: 80 }]]);
    expect(value.unsaved()).toEqual({});
  });
  it("keeps failed values, does not loop retries, and retries the newest value", async () => {
    const first = deferred();
    const save = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue(true); await mount(save);
    await act(async () => value.enqueue({ explorePageSize: 80 }));
    await act(async () => { value.enqueue({ explorePageSize: 90, privacyOnStartup: false }); first.resolve(false); });
    expect(value.status).toBe("error");
    expect(value.unsaved()).toEqual({ explorePageSize: 90, privacyOnStartup: false });
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(save).toHaveBeenCalledTimes(1);
    await act(async () => { await value.flush(); });
    expect(save).toHaveBeenLastCalledWith({ explorePageSize: 90, privacyOnStartup: false });
    expect(value.status).toBe("saved");
  });
});
