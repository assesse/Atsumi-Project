import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinterestApi } from "./api";
import { createPinterestAccountApi } from "./accountApi";
import type { PinterestDownloadOutcome } from "./types";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
  Channel: class { onmessage: (outcome: PinterestDownloadOutcome) => void = () => {}; },
}));

beforeEach(() => { mocks.invoke.mockReset(); });

describe("Pinterest session transport", () => {
  it("passes home and opaque continuation to native without credential material", async () => {
    mocks.invoke.mockResolvedValue({ pins: [], cursor: "next", generation: 3, authStatus: "signedIn" });
    await createPinterestApi("tauri").discover("", "home", "home-request");
    expect(mocks.invoke).toHaveBeenLastCalledWith("pinterest_discover", { input: "", kind: "home", requestId: "home-request", cursor: null });
    await createPinterestApi("tauri").discover("", "home", "next-request", "owned-cursor");
    expect(mocks.invoke).toHaveBeenLastCalledWith("pinterest_discover", { input: "", kind: "home", requestId: "next-request", cursor: "owned-cursor" });
  });
  it("uses source-specific native status and explicit login/logout operations", async () => {
    const api = createPinterestAccountApi("tauri");
    await api.status(); expect(mocks.invoke).toHaveBeenLastCalledWith("pinterest_account_status");
    await api.open("login"); expect(mocks.invoke).toHaveBeenLastCalledWith("pinterest_account_open", { action: "login" });
    await api.open("logout"); expect(mocks.invoke).toHaveBeenLastCalledWith("pinterest_account_open", { action: "logout" });
  });
});

describe("Pinterest batch transport", () => {
  it("submits every selected ID in one guarded invoke and forwards per-pin outcomes", async () => {
    const outcome: PinterestDownloadOutcome = { pinId: "12345", status: "failed", error: "fixture" };
    mocks.invoke.mockImplementation(async (command, args) => {
      expect(command).toBe("pinterest_download_batch");
      expect(args.pinIds).toEqual(["12345", "12346"]);
      expect(args.requestId).toBe("pinterest-batch-test");
      args.onProgress.onmessage(outcome);
      return [outcome, { pinId: "12346", status: "cancelled" }];
    });
    const progress = vi.fn();
    const result = await createPinterestApi("tauri").downloadBatch(["12345", "12346"], "pinterest-batch-test", progress);
    expect(progress).toHaveBeenCalledExactlyOnceWith(outcome);
    expect(result).toHaveLength(2);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
  });

  it("does not fall back to unguarded per-pin invokes when batch registration fails", async () => {
    mocks.invoke.mockRejectedValue("앱을 종료하는 중입니다.");
    await expect(createPinterestApi("tauri").downloadBatch(["12345"], "pinterest-batch-test", vi.fn())).rejects.toBe("앱을 종료하는 중입니다.");
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
  });

  it("does not advertise successful batch saves in a browser-only runtime", async () => {
    await expect(createPinterestApi("browser-mock").downloadBatch(["12345"], "pinterest-batch-test", vi.fn())).rejects.toThrow("데스크톱");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
