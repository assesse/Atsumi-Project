import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { getAutostartStatus, setAutostartEnabled, type AutostartStatus } from "./autostart";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));

const status: AutostartStatus = {
  supported: true,
  enabled: false,
  launchMode: "development",
  needsRepair: false,
  disabledByWindows: false,
};

afterEach(() => vi.resetAllMocks());

describe("Windows autostart transport", () => {
  it("never simulates an OS registration in a browser preview", async () => {
    vi.mocked(isTauri).mockReturnValue(false);
    expect(await getAutostartStatus()).toMatchObject({ ok: true, data: { supported: false, enabled: false } });
    expect(await setAutostartEnabled(true)).toMatchObject({ ok: false, error: { code: "AUTOSTART_UNSUPPORTED" } });
    expect(invoke).not.toHaveBeenCalled();
  });

  it("reads native state without enabling startup", async () => {
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockResolvedValue(status);
    expect(await getAutostartStatus()).toEqual({ ok: true, data: status });
    expect(invoke).toHaveBeenCalledExactlyOnceWith("autostart_status_get", undefined);
  });

  it.each([true, false])("sends only the requested enabled=%s value to the native owner", async (enabled) => {
    vi.mocked(isTauri).mockReturnValue(true);
    const next = { ...status, enabled };
    vi.mocked(invoke).mockResolvedValue(next);
    expect(await setAutostartEnabled(enabled)).toEqual({ ok: true, data: next });
    expect(invoke).toHaveBeenCalledExactlyOnceWith("autostart_enabled_set", { enabled });
  });

  it("preserves external Windows disable and stale-target flags", async () => {
    vi.mocked(isTauri).mockReturnValue(true);
    const blocked = { ...status, enabled: true, disabledByWindows: true, needsRepair: true };
    vi.mocked(invoke).mockResolvedValue(blocked);
    expect(await getAutostartStatus()).toEqual({ ok: true, data: blocked });
  });

  it("returns actionable native errors without a false success", async () => {
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockRejectedValue("시작 프로그램 바로가기를 저장하지 못했습니다.");
    expect(await setAutostartEnabled(true)).toMatchObject({
      ok: false,
      error: { code: "AUTOSTART_UPDATE_FAILED", message: "시작 프로그램 바로가기를 저장하지 못했습니다.", retryable: true },
    });
    vi.mocked(invoke).mockRejectedValue(new Error("private implementation error"));
    expect(await getAutostartStatus()).toMatchObject({
      ok: false,
      error: { code: "AUTOSTART_READ_FAILED", message: expect.stringContaining("다시 시도") },
    });
  });
});
