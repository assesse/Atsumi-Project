import { beforeEach, describe, expect, it, vi } from "vitest";
import { createAutoWatchApi, createAutoWatchLiveApi } from "./autoWatch";
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
beforeEach(() => { invoke.mockReset(); invoke.mockResolvedValue({ ok: true, data: undefined }); });
describe("automatic recording presentation transport", () => {
  it("routes common live controls only to the attached receiver, never the root player", async () => {
    const api = createAutoWatchLiveApi("tauri", "watch-bound");
    await api.snapshot(); await api.start({ rightsAcknowledged: true, captureChat: true }); await api.stop();
    await api.requestControl("screenshot");
    await api.confirmControl({ requestId: "same-confirmation", approve: true, rightsAcknowledged: true, captureChat: true });
    await api.ackUiAction("same-ui-action");
    expect(invoke.mock.calls).toHaveLength(6);
    for (const [command, args] of invoke.mock.calls) {
      expect(command).toBe("chzzk_auto_watch_browser"); expect(args.watchId).toBe("watch-bound");
    }
    expect(invoke.mock.calls.map(([, args]) => args.operation.kind)).toEqual(["snapshot", "start", "stop", "requestControl", "confirmControl", "ackUiAction"]);
    invoke.mockClear();
    expect(await api.open("another-channel")).toMatchObject({ ok: false });
    expect(await api.connectExtension()).toMatchObject({ ok: false }); expect(invoke).not.toHaveBeenCalled();
    await api.setViewport({ x: 0, y: 0, width: 640, height: 360, visible: true, epoch: 7 });
    expect(invoke).toHaveBeenCalledWith("chzzk_auto_watch_viewport", expect.objectContaining({ watchId: "watch-bound" }));
  });
  it("binds to the clicked recording and never starts, stops, or opens a new broadcast", async () => {
    const api = createAutoWatchApi("tauri");
    await api.open("channel", "recording-1"); await api.snapshot("watch-1");
    await api.setAudio("watch-1", false); await api.close("watch-1");
    expect(invoke.mock.calls).toEqual([
      ["chzzk_auto_watch_open", { channelId: "channel", recordingId: "recording-1" }],
      ["chzzk_auto_watch_snapshot", { watchId: "watch-1" }],
      ["chzzk_auto_watch_audio", { watchId: "watch-1", enabled: false }],
      ["chzzk_auto_watch_close", { watchId: "watch-1" }],
    ]);
  });
  it("uses increasing viewport sequences across API lifetimes and fences hides to a lease", async () => {
    const viewport = { x: 0, y: 0, width: 640, height: 360, visible: true, epoch: 4 };
    await createAutoWatchApi("tauri").setViewport("first", viewport);
    await createAutoWatchApi("tauri").setViewport("second", { ...viewport, visible: false });
    const first = invoke.mock.calls[0]![1], second = invoke.mock.calls[1]![1];
    expect(first.watchId).toBe("first"); expect(second.watchId).toBe("second");
    expect(second.viewport.requestSequence).toBeGreaterThan(first.viewport.requestSequence);
    expect(second.viewport).toMatchObject({ visible: false, epoch: 4 });
  });
  it("reports transport failures and does not invoke native commands in browser previews", async () => {
    invoke.mockRejectedValue(new Error("offline"));
    expect(await createAutoWatchApi("tauri").open("channel", "recording")).toMatchObject({ ok: false, error: { code: "AUTO_WATCH_TRANSPORT" } });
    invoke.mockClear();
    expect(await createAutoWatchApi("browser-mock").open("channel", "recording")).toMatchObject({ ok: false });
    expect(invoke).not.toHaveBeenCalled();
  });
});
