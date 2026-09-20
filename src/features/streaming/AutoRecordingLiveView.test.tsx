import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AutoWatchApi, AutoWatchTarget } from "../../api/autoWatch";
import { createOfficialBrowserApi, emptyOfficialBrowserSnapshot, type OfficialBrowserSnapshot } from "../../api/officialBrowser";
import { AutoRecordingLiveView } from "./AutoRecordingLiveView";

const target: AutoWatchTarget = { kind: "automatic", watchId: "watch-1", channelId: "a".repeat(32), channelName: "테스트 방송", recordingId: "record-1", epoch: 7 };
const state: OfficialBrowserSnapshot = { ...emptyOfficialBrowserSnapshot("tauri"), windowOpen: true, ready: true, channelId: target.channelId, recordingId: target.recordingId, viewportEpoch: 7, status: "recording", chatCount: 42, chatStatus: "receiving",
  recordings: [{ id: target.recordingId, channelId: target.channelId, title: "테스트 방송", startedAt: 1000, updatedAt: 1000, status: "recording", mimeType: "video/mp4", outputDir: "fixture-only", segmentCount: 1, bytesWritten: 1024, durationSeconds: 3, lastError: null, segments: [] }] };
const ok = (data = state) => ({ ok: true as const, data });
function fake() {
  const api = { runtime: "tauri" as const, open: vi.fn(), snapshot: vi.fn(), setAudio: vi.fn(), setViewport: vi.fn(), close: vi.fn().mockResolvedValue({ ok: true }) } satisfies AutoWatchApi;
  const live = {
    ...createOfficialBrowserApi("browser-mock"), runtime: "tauri" as const,
    snapshot: vi.fn().mockResolvedValue(ok()), setViewport: vi.fn().mockResolvedValue({ ok: true }),
    ackUiAction: vi.fn().mockResolvedValue(ok()), requestControl: vi.fn().mockResolvedValue(ok()),
    confirmControl: vi.fn().mockResolvedValue(ok()), stop: vi.fn().mockResolvedValue(ok({ ...state, status: "stopping" })),
    start: vi.fn(), open: vi.fn(),
  };
  return { api, live };
}
let container: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers(); container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.clearAllTimers(); vi.useRealTimers(); });
const button = (text: string) => [...container.querySelectorAll("button")].find(button => button.textContent === text)!;
const intent = async (live: ReturnType<typeof fake>["live"], action: "open_settings" | "record_only", id = action) => {
  live.snapshot.mockResolvedValue(ok({ ...state, pendingUiAction: { id, action, expiresAt: Date.now() + 8000 } }));
  await act(async () => vi.advanceTimersByTimeAsync(2000));
};
describe("automatic receiver attached to the common live controller", () => {
  it("hides other tabs without closing the watch or changing playback, and retains privacy separately", async () => {
    const { api, live } = fake();
    const render = (active: boolean, privacy = false) => act(async () => root.render(
      <AutoRecordingLiveView target={target} runtime="tauri" active={active} privacy={privacy} api={api} liveApi={live} onLeave={vi.fn()} />));
    await render(true);
    await render(false);
    expect(container).toBeEmptyDOMElement();
    expect(live.setViewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false }));
    expect(live.setViewport.mock.lastCall?.[0].suspendAudio).not.toBe(true);
    await render(false, true);
    expect(live.setViewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false, suspendAudio: true }));
    await render(true);
    expect(container.querySelector('[aria-label="공식 CHZZK 플레이어와 채팅"]')).not.toBeNull();
    expect(api.close).not.toHaveBeenCalled(); expect(api.open).not.toHaveBeenCalled();
    expect(api.setAudio).not.toHaveBeenCalled(); expect(live.stop).not.toHaveBeenCalled();
    expect(live.start).not.toHaveBeenCalled(); expect(live.open).not.toHaveBeenCalled();
  });
  it("uses the real live settings and recording controls, not a reduced wrapper", async () => {
    const { api, live } = fake();
    await act(async () => root.render(<AutoRecordingLiveView target={target} runtime="tauri" privacy={false} api={api} liveApi={live} onLeave={vi.fn()} />));
    expect(container.querySelector('.auto-record-live-toolbar,.mado-native-pane')).toBeNull();
    expect(container.querySelector('[aria-label="공식 CHZZK 플레이어와 채팅"]')).not.toBeNull();
    await intent(live, "open_settings");
    expect(button("녹화 중지")).toBeEnabled();
    expect(container).toHaveTextContent("42개");
    await act(async () => button("녹화 중지").click());
    expect(live.stop).toHaveBeenCalledOnce(); expect(api.close).not.toHaveBeenCalled();
    expect(live.start).not.toHaveBeenCalled(); expect(live.open).not.toHaveBeenCalled(); expect(api.open).not.toHaveBeenCalled();
  });
  it("uses the normal screenshot confirmation and targets this same session", async () => {
    const { api, live } = fake();
    live.snapshot.mockResolvedValue(ok({ ...state, pendingControl: { id: "screenshot-id", action: "screenshot", channelId: target.channelId, expiresAt: Date.now() + 20000 } }));
    await act(async () => root.render(<AutoRecordingLiveView target={target} runtime="tauri" privacy={false} api={api} liveApi={live} onLeave={vi.fn()} />));
    expect(container.querySelector('[role="alertdialog"]')).toHaveTextContent("화면을 저장할까요?");
    await act(async () => button("저장 확인").click());
    expect(live.confirmControl).toHaveBeenCalledWith({ requestId: "screenshot-id", approve: true, rightsAcknowledged: true, captureChat: true });
    expect(api.close).not.toHaveBeenCalled();
  });
  it("record-only parks the presentation exactly once and does not stop capture", async () => {
    const { api, live } = fake(), onLeave = vi.fn();
    await act(async () => root.render(<AutoRecordingLiveView target={target} runtime="tauri" privacy={false} api={api} liveApi={live} onLeave={onLeave} />));
    await intent(live, "record_only");
    expect(api.close).toHaveBeenCalledExactlyOnceWith("watch-1"); expect(onLeave).toHaveBeenCalledOnce();
    expect(live.stop).not.toHaveBeenCalled(); expect(live.start).not.toHaveBeenCalled();
    await act(async () => root.render(null)); expect(api.close).toHaveBeenCalledTimes(1);
  });
  it("keeps the lease and recorder while privacy hides the native viewport", async () => {
    const { api, live } = fake();
    await act(async () => root.render(<AutoRecordingLiveView target={target} runtime="tauri" privacy api={api} liveApi={live} onLeave={vi.fn()} />));
    expect(container).toHaveTextContent("프라이버시 모드");
    expect(live.setViewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false, epoch: 7 }));
    expect(api.close).not.toHaveBeenCalled(); expect(live.stop).not.toHaveBeenCalled();
  });
  it("survives StrictMode effect replay without closing or reopening the recording", async () => {
    const { api, live } = fake();
    await act(async () => root.render(<StrictMode><AutoRecordingLiveView target={target} runtime="tauri" privacy={false} api={api} liveApi={live} onLeave={vi.fn()} /></StrictMode>));
    await act(async () => vi.advanceTimersByTimeAsync(2100));
    expect(api.close).not.toHaveBeenCalled(); expect(api.open).not.toHaveBeenCalled(); expect(live.open).not.toHaveBeenCalled();
    expect(container).toHaveTextContent("녹화 중");
    await act(async () => root.render(null)); expect(api.close).toHaveBeenCalledExactlyOnceWith("watch-1");
  });
  it("reports lost session state without calling it privacy or starting a replacement", async () => {
    const { api, live } = fake();
    await act(async () => root.render(<AutoRecordingLiveView target={target} runtime="tauri" privacy={false} api={api} liveApi={live} onLeave={vi.fn()} />));
    live.snapshot.mockResolvedValue({ ok: false, error: { code: "AUTO_WATCH_ENDED", message: "연결 확인 필요", retryable: true } } as never);
    await act(async () => vi.advanceTimersByTimeAsync(2100));
    expect(container).toHaveTextContent("연결 확인 필요"); expect(container).not.toHaveTextContent("프라이버시 모드");
    expect(live.start).not.toHaveBeenCalled(); expect(live.open).not.toHaveBeenCalled();
  });
  it("releases the old ID on target change without retiring its replacement", async () => {
    const { api, live } = fake();
    await act(async () => root.render(<AutoRecordingLiveView target={target} runtime="tauri" privacy={false} api={api} liveApi={live} onLeave={vi.fn()} />));
    await act(async () => root.render(<AutoRecordingLiveView target={{ ...target, watchId: "watch-2" }} runtime="tauri" privacy={false} api={api} liveApi={live} onLeave={vi.fn()} />));
    expect(api.close).toHaveBeenCalledExactlyOnceWith("watch-1");
  });
  it("does not leave a replacement screen when an old close completes late", async () => {
    const { api, live } = fake(), onLeave = vi.fn(); let resolve!: (value: { ok: boolean }) => void;
    api.close.mockReturnValueOnce(new Promise(done => { resolve = done; }));
    await act(async () => root.render(<AutoRecordingLiveView target={target} runtime="tauri" privacy={false} api={api} liveApi={live} onLeave={onLeave} />));
    await intent(live, "record_only"); live.snapshot.mockResolvedValue(ok());
    await act(async () => root.render(<AutoRecordingLiveView target={{ ...target, watchId: "watch-2" }} runtime="tauri" privacy={false} api={api} liveApi={live} onLeave={onLeave} />));
    await act(async () => resolve({ ok: true })); expect(onLeave).not.toHaveBeenCalled();
  });
});
