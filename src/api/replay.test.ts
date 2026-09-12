import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createReplayApi } from "./replay";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), convertFileSrc: vi.fn((token: string, protocol: string) => `http://${protocol}.localhost/${token}`) }));
beforeEach(() => { vi.clearAllMocks(); vi.mocked(invoke).mockResolvedValue({ ok: true, data: null }); });
describe("offline replay API", () => {
  it("sends recording IDs and opaque cursors with bounded generation-scoped requests", async () => {
    const api = createReplayApi("tauri");
    await api.open("recording-id"); await api.chatAt("token", 42.5, 7); await api.chatPage("token", "opaque", 8);
    await api.timeline("token"); await api.setOffset("token", -2.5); await api.close("token");
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["replay_open", { recordingId: "recording-id" }],
      ["replay_chat_at", { token: "token", mediaTime: 42.5, generation: 7, limit: 200 }],
      ["replay_chat_page", { token: "token", cursor: "opaque", generation: 8, limit: 200 }],
      ["replay_timeline", { token: "token" }], ["replay_set_offset", { token: "token", offsetSeconds: -2.5 }], ["replay_close", { token: "token" }],
    ]);
    expect(api.mediaUrl("token")).toBe("http://atsumi-replay.localhost/token");
    expect(convertFileSrc).toHaveBeenCalledExactlyOnceWith("token", "atsumi-replay");
  });
  it("does not invoke native commands in browser previews and handles transport failure", async () => {
    const api = createReplayApi("browser-mock");
    expect((await api.open("id")).ok).toBe(false); expect((await api.chatAt("t", 1, 1)).ok).toBe(false);
    expect(invoke).not.toHaveBeenCalled();
    vi.mocked(invoke).mockRejectedValueOnce(new Error("private filesystem path"));
    const result = await createReplayApi("tauri").open("id");
    expect(result.ok).toBe(false); expect(JSON.stringify(result)).not.toContain("private filesystem path");
  });
});
