import { beforeEach, describe, expect, it, vi } from "vitest";
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
import { soopAutoRecordApi } from "./soopAutoRecord";

describe("SOOP explicit automatic recording transport", () => {
  beforeEach(() => { invoke.mockReset(); });
  it("only reads reservations when polling", async () => {
    const snapshot = { revision: 4, ready: true, channels: [], error: null };
    invoke.mockResolvedValue(snapshot);
    expect(await soopAutoRecordApi.snapshot()).toBe(snapshot);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("soop_auto_record_snapshot");
  });
  it("carries the explicit desired state and CAS revision", async () => {
    invoke.mockResolvedValue({ revision: 5, ready: true, channels: [], error: null });
    await soopAutoRecordApi.set("fixture", "채널", false, 4);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("soop_auto_record_set", { channelId: "fixture", name: "채널", enabled: false, expectedRevision: 4 });
  });
  it("does not silently replay a stale mutation", async () => {
    invoke.mockRejectedValue(new Error("예약 상태가 변경됐습니다."));
    await expect(soopAutoRecordApi.set("fixture", "채널", true, 1)).rejects.toThrow("예약 상태가 변경됐습니다.");
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});
