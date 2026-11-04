import { afterEach, describe, expect, it, vi } from "vitest";
import { createRecordingUpdateGuard } from "./recordingUpdateGuard";

const native = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: native.invoke }));
afterEach(() => vi.resetAllMocks());

describe("database-independent update reservation", () => {
  it("uses bootstrap commands rather than the database-backed streaming service", async () => {
    const guard = createRecordingUpdateGuard("tauri");
    await guard.acquire();
    await guard.release();
    expect(native.invoke.mock.calls).toEqual([["app_update_reserve"], ["app_update_release"]]);
  });
  it("preserves active-work and initialization errors without treating them as success", async () => {
    native.invoke.mockRejectedValue("앱 데이터 준비·정리가 끝난 뒤 업데이트를 설치해 주세요.");
    await expect(createRecordingUpdateGuard("tauri").acquire()).rejects.toThrow("앱 데이터 준비·정리");
    expect(native.invoke).toHaveBeenCalledOnce();
  });
  it("does not reach the real updater from browser previews", async () => {
    await createRecordingUpdateGuard("browser-mock").acquire();
    await createRecordingUpdateGuard("browser-mock").release();
    expect(native.invoke).not.toHaveBeenCalled();
  });
});
