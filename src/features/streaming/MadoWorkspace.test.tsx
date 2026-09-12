import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { createMultiviewApi, emptyMultiview, multiviewEntries, normalizeMultiviewChannel, type MultiviewApi, type MultiviewSnapshot } from "../../api/multiview";
import { MadoWorkspace, readMadoLayout } from "./MadoWorkspace";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const A = "a".repeat(32), B = "b".repeat(32), C = "c".repeat(32), D = "d".repeat(32);
const success = <T,>(data: T) => ({ ok: true as const, data });
const multiple = (oneVideo = false): MultiviewSnapshot => ({ active: true, epoch: 7, audioOwner: null, panes: [A, B, C, D].flatMap((channelId, index) => [
  ...(!oneVideo || index === 0 ? [{ paneId: `video-${index}`, channelId, channelName: `도서관 ${index + 1}`, kind: "video" as const, status: "ready", ready: true, audioEnabled: false }] : []),
  { paneId: `chat-${index}`, channelId, channelName: `도서관 ${index + 1}`, kind: "chat" as const, status: "ready" },
]) });
const deferred = <T,>() => { let resolve!: (value: T) => void; const promise = new Promise<T>((done) => { resolve = done; }); return { promise, resolve }; };
const active = (): MultiviewSnapshot => ({ active: true, epoch: 7, audioOwner: null, panes: [
  { paneId: "mado-7-a-video", channelId: A, kind: "video", status: "ready", ready: true, audioEnabled: false },
  { paneId: "mado-7-a-chat", channelId: A, kind: "chat", status: "ready" },
] });
const fake = () => ({ runtime: "tauri" as const, configure: vi.fn<MultiviewApi["configure"]>().mockResolvedValue(success(active())),
  snapshot: vi.fn<MultiviewApi["snapshot"]>().mockResolvedValue(success(emptyMultiview())),
  close: vi.fn<MultiviewApi["close"]>().mockResolvedValue(success(emptyMultiview())),
  setAudio: vi.fn<MultiviewApi["setAudio"]>().mockResolvedValue(success(active())),
  setPaneAudio: vi.fn<MultiviewApi["setPaneAudio"]>().mockResolvedValue(success(active())),
  requestControl: vi.fn<MultiviewApi["requestControl"]>().mockResolvedValue(success(active())),
  confirmControl: vi.fn<MultiviewApi["confirmControl"]>().mockResolvedValue(success(active())),
  ackUiAction: vi.fn<MultiviewApi["ackUiAction"]>().mockResolvedValue(success(active())),
  setViewport: vi.fn<MultiviewApi["setViewport"]>().mockResolvedValue(success(undefined)),
});
let container: HTMLDivElement, root: Root;
beforeEach(() => localStorage.clear());
beforeEach(() => { vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.useFakeTimers(); vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => setTimeout(() => cb(0), 16)); vi.stubGlobal("cancelAnimationFrame", clearTimeout); container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
const button = (text: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find((element) => element.textContent === text)!;
const enter = async (index: number, text: string) => act(async () => {
  const input = container.querySelector<HTMLInputElement>(`#mado-channel-${index}`)!;
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, text); input.dispatchEvent(new Event("input", { bubbles: true }));
});
describe("마도 배치", () => {
  it("keeps chat failure and gap warnings in the fixed status row even after recording stops", async () => {
    const api = fake(); let state = multiple();
    state.panes[0] = { ...state.panes[0]!, recordingStatus: "stopped", recordingId: null, chatStatus: "storage_failed", chatCount: 42 };
    api.snapshot.mockImplementation(async () => success(state));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const slots = [...container.querySelectorAll(".mado-native-slot")];
    const warning = container.querySelector<HTMLElement>(".mado-chat-warning")!;
    expect(warning).toHaveTextContent("⚠"); expect(warning).toHaveAttribute("tabindex", "0");
    expect(warning.title).toContain("채팅 저장 실패"); expect(warning.title).toContain("42개 기록");
    expect(warning).toHaveAccessibleName(warning.title);
    expect(warning.closest(".mado-pane-label")).not.toBeNull();
    state = { ...state, panes: state.panes.map((pane, index) => index === 0 ? { ...pane, chatStatus: "connection_gap" } : pane) };
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(container.querySelector<HTMLElement>(".mado-chat-warning")!.title).toContain("일부 누락 가능");
    expect([...container.querySelectorAll(".mado-native-slot")]).toEqual(slots);
    expect(api.close).not.toHaveBeenCalled();
  });
  it("rejects mismatched and expired per-pane intents without approval", async () => {
    const api = fake(); let state = multiple();
    state.pendingControl = { paneId: "video-0", id: "mismatch", action: "screenshot", channelId: B, expiresAt: Date.now() + 20000 };
    api.snapshot.mockImplementation(async () => success(state));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    state = { ...state, pendingControl: { paneId: "video-0", id: "expires", action: "screenshot", channelId: A, expiresAt: Date.now() + 2500 } };
    await act(async () => vi.advanceTimersByTimeAsync(2000)); expect(container.querySelector('[role="alertdialog"]')).not.toBeNull();
    await act(async () => vi.advanceTimersByTimeAsync(501));
    expect(container.querySelector('[role="alertdialog"]')).toBeNull(); expect(api.confirmControl).not.toHaveBeenCalled();
  });
  it("keeps recordings alive on unmount and blocks explicit configuration or leave", async () => {
    const api = fake(), leave = vi.fn(), state = multiple(); state.panes[0]!.recordingId = "test-record"; state.panes[0]!.recordingStatus = "recording";
    api.snapshot.mockResolvedValue(success(state));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={leave} api={api} />));
    expect(button("한 방송 보기")).toBeDisabled();
    await act(async () => button("배치").click()); expect(button("적용")).toBeDisabled();
    await act(async () => root.render(null));
    expect(api.close).not.toHaveBeenCalled(); expect(leave).not.toHaveBeenCalled();
    expect(api.setViewport.mock.calls.slice(-state.panes.length).every(([, viewport]) => !viewport.visible)).toBe(true);
  });
  it("targets the selected video for S and keeps saving behind confirmation", async () => {
    const api = fake(), state = multiple(); api.snapshot.mockResolvedValue(success(state));
    api.requestControl.mockImplementation(async (paneId, action) => success({ ...state, pendingControl: { paneId, id: "key-save", action, channelId: B, expiresAt: Date.now() + 20000 } }));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await act(async () => button("2. 도서관 2").click());
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true })));
    expect(api.requestControl).toHaveBeenCalledExactlyOnceWith("video-1", "screenshot", 7);
    expect(button("저장 확인")).toBeDisabled(); expect(api.confirmControl).not.toHaveBeenCalled();
    await act(async () => container.querySelector('[role="alertdialog"]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(api.confirmControl.mock.calls[0]?.[0]).toMatchObject({ paneId: "video-1", approve: false, rightsAcknowledged: false });
  });
  it("persists bounded split preferences and supports keyboard adjustment", async () => {
    localStorage.setItem("atsumi.mado.layout.v1", '{"version":1,"direction":"horizontal","horizontal":999,"vertical":-20}');
    expect(readMadoLayout()).toEqual({ version: 1, direction: "horizontal", horizontal: 76, vertical: 24 });
    localStorage.setItem("atsumi.mado.layout.v1", "broken"); expect(readMadoLayout().horizontal).toBe(60); localStorage.clear();
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple(true)));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const divider = container.querySelector('[role="separator"]')!;
    await act(async () => divider.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true })));
    expect(divider).toHaveAttribute("aria-valuenow", "62");
    await act(async () => button("위아래 배치").click()); expect(divider).toHaveAttribute("aria-orientation", "horizontal");
    await act(async () => divider.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true })));
    await act(async () => vi.advanceTimersByTimeAsync(250));
    expect(JSON.parse(localStorage.getItem("atsumi.mado.layout.v1")!)).toEqual({ version: 1, direction: "vertical", horizontal: 62, vertical: 24 });
  });
  it("hides panes only during divider dragging and restores after cancellation", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple(true)));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 800, 600));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const divider = container.querySelector('[role="separator"]')!;
    const pointer = (type: string, x: number) => { const event = new MouseEvent(type, { clientX: x, button: 0, bubbles: true }); Object.defineProperty(event, "pointerId", { value: 1 }); divider.dispatchEvent(event); };
    await act(async () => pointer("pointerdown", 480));
    expect(api.setViewport.mock.calls.slice(-5).every(([, viewport]) => !viewport.visible)).toBe(true);
    await act(async () => { for (let i = 0; i < 20; i++) pointer("pointermove", 400 + i); });
    expect(divider).toHaveAttribute("aria-valuenow", "60");
    await act(async () => vi.advanceTimersByTimeAsync(20)); expect(divider).toHaveAttribute("aria-valuenow", "52");
    await act(async () => pointer("pointercancel", 419));
    expect(api.setViewport.mock.calls.slice(-5).every(([, viewport]) => viewport.visible)).toBe(true);
    expect(api.close).not.toHaveBeenCalled(); expect(api.confirmControl).not.toHaveBeenCalled();
  });
  it("toggles each video's audio independently and displays provider names", async () => {
    const api = fake(); let state = multiple(); api.snapshot.mockImplementation(async () => success(state));
    api.setPaneAudio.mockImplementation(async (paneId, enabled) => { state = { ...state, panes: state.panes.map((pane) => pane.paneId === paneId ? { ...pane, audioEnabled: enabled } : pane) }; return success(state); });
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="1. 도서관 1 소리 켜기"]')!.click());
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="2. 도서관 2 소리 켜기"]')!.click());
    expect(api.setPaneAudio.mock.calls).toEqual([["video-0", true, 7], ["video-1", true, 7]]);
    expect(state.panes.filter((pane) => pane.audioEnabled).map((pane) => pane.paneId)).toEqual(["video-0", "video-1"]);
    expect(api.setAudio).not.toHaveBeenCalled();
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy onLeave={() => {}} api={api} />));
    expect(container).not.toHaveTextContent("도서관 1");
  });
  it("requires explicit per-pane recording permission and focuses Cancel before approval", async () => {
    const api = fake(); const state = multiple();
    state.pendingControl = { paneId: "video-1", id: "record-second", action: "record_start", channelId: B, expiresAt: Date.now() + 20000 };
    api.snapshot.mockResolvedValue(success(state));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const dialog = container.querySelector('[role="alertdialog"]')!;
    expect(dialog).toHaveTextContent("2. 도서관 2"); expect(button("시작 확인")).toBeDisabled(); expect(document.activeElement).toBe(button("취소"));
    await act(async () => dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(api.confirmControl).not.toHaveBeenCalled();
    await act(async () => dialog.querySelector<HTMLInputElement>("input")!.click());
    await act(async () => button("시작 확인").click());
    expect(api.confirmControl).toHaveBeenCalledExactlyOnceWith({ paneId: "video-1", requestId: "record-second", approve: true, rightsAcknowledged: true, captureChat: true, epoch: 7 });
  });
  it("normalizes only exact official channel identifiers and refuses credentials/query/foreign URLs", () => {
    expect(normalizeMultiviewChannel(A.toUpperCase())).toBe(A);
    expect(normalizeMultiviewChannel(`https://chzzk.naver.com/live/${A}/chat`)).toBe(A);
    for (const value of [`http://chzzk.naver.com/live/${A}`, `https://chzzk.naver.com.evil.test/live/${A}`, `https://user@chzzk.naver.com/live/${A}`, `https://chzzk.naver.com/live/${A}?extra=1`, "abc"]) expect(normalizeMultiviewChannel(value)).toBeNull();
  });
  it("creates one video and four chat panes without hidden extra video entries", () => {
    expect(multiviewEntries([A, B, C, D], "chats", 1)).toEqual([
      { channelId: A, video: false, chat: true }, { channelId: B, video: true, chat: true },
      { channelId: C, video: false, chat: true }, { channelId: D, video: false, chat: true },
    ]);
    expect(typeof multiviewEntries([A, A], "paired", 0)).toBe("string");
    expect(typeof multiviewEntries([A, "", C], "chats", 1)).toBe("string");
    expect(typeof multiviewEntries([], "paired", 0)).toBe("string");
  });
  it("does not open streams on mount and requires explicit valid configuration", async () => {
    const api = fake(); await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(api.configure).not.toHaveBeenCalled();
    await act(async () => button("적용").click()); expect(api.configure).not.toHaveBeenCalled();
    await enter(0, A); await act(async () => button("적용").click());
    expect(api.configure).toHaveBeenCalledWith([{ channelId: A, video: true, chat: true }]);
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
  });
  it("keeps a denied recording-active transition visible instead of pretending to open panes", async () => {
    const api = fake(); api.configure.mockResolvedValue({ ok: false, error: { code: "RECORDING_ACTIVE", message: "녹화를 먼저 중지해 주세요.", retryable: true } });
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await enter(0, A); await act(async () => button("적용").click());
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("녹화를 먼저 중지");
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(0);
  });
  it("closes the expected epoch before leaving and hides panes for privacy", async () => {
    const api = fake(), leave = vi.fn(); api.snapshot.mockResolvedValue(success(active()));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy onLeave={leave} api={api} />));
    expect(api.setViewport.mock.calls.every(([, viewport]) => viewport.visible === false && viewport.epoch === 7)).toBe(true);
    await act(async () => button("한 방송 보기").click()); expect(api.close).toHaveBeenCalledWith(7); expect(leave).toHaveBeenCalledOnce();
  });
  it("binds audio and close to the native epoch and assigns unique viewport sequences", async () => {
    vi.mocked(invoke).mockResolvedValue(success(undefined));
    const api = createMultiviewApi("tauri");
    await api.setAudio(A, 7); expect(invoke).toHaveBeenCalledWith("chzzk_multiview_set_audio", { channelId: A, epoch: 7 });
    await api.close(7); expect(invoke).toHaveBeenCalledWith("chzzk_multiview_close", { epoch: 7 });
    const viewport = { x: 0, y: 0, width: 100, height: 100, visible: true, epoch: 7 };
    await api.setViewport("mado-7-a-video", viewport); await api.setViewport("mado-7-a-video", { ...viewport, visible: false });
    const calls = vi.mocked(invoke).mock.calls.filter(([name]) => name === "chzzk_multiview_set_viewport");
    const sequences = calls.map(([, args]) => (args as { viewport: { requestSequence: number } }).viewport.requestSequence);
    expect(sequences).toHaveLength(2);
    expect(sequences[1]!).toBeGreaterThan(sequences[0]!);
  });
  it("does not let a delayed initial snapshot replace a newer configured layout", async () => {
    const api = fake(), initial = deferred<Awaited<ReturnType<MultiviewApi["snapshot"]>>>();
    api.snapshot.mockReturnValue(initial.promise);
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await enter(0, A); await act(async () => button("적용").click());
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
    await act(async () => initial.resolve(success(emptyMultiview())));
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
    expect(button("배치")).toHaveAttribute("aria-expanded", "false");
  });
  it("restores channel drafts and the selected video when reopening an existing one-video layout", async () => {
    const api = fake();
    api.snapshot.mockResolvedValue(success({ active: true, epoch: 9, audioOwner: B, panes: [A, B, C, D].flatMap((channelId) => [
      ...(channelId === B ? [{ paneId: `restored-${channelId}-video`, channelId, kind: "video" as const, status: "ready" }] : []),
      { paneId: `restored-${channelId}-chat`, channelId, kind: "chat" as const, status: "ready" },
    ]) }));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(container.querySelector(".mado-surfaces")).toHaveClass("is-chats");
    await act(async () => button("배치").click());
    expect([0, 1, 2, 3].map((index) => container.querySelector<HTMLInputElement>(`#mado-channel-${index}`)!.value)).toEqual([A, B, C, D]);
    expect(button("영상 하나·채팅 모아보기")).toHaveAttribute("aria-pressed", "true");
    expect([...container.querySelectorAll<HTMLInputElement>('input[name="mado-lead"]')].map((radio) => radio.checked)).toEqual([false, true, false, false]);
    expect(api.configure).not.toHaveBeenCalled();
  });
  it("does not overwrite a channel draft edited before the initial snapshot arrives", async () => {
    const api = fake(), initial = deferred<Awaited<ReturnType<MultiviewApi["snapshot"]>>>();
    api.snapshot.mockReturnValue(initial.promise);
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await enter(0, B);
    await act(async () => initial.resolve(success(active())));
    expect(container.querySelector<HTMLInputElement>("#mado-channel-0")!.value).toBe(B);
    expect(button("배치")).toHaveAttribute("aria-expanded", "true");
    expect(api.configure).not.toHaveBeenCalled();
  });
  it("closes a configure result that arrives after unmount without changing the new workspace", async () => {
    const oldApi = fake(), newerApi = fake(), configured = deferred<Awaited<ReturnType<MultiviewApi["configure"]>>>();
    oldApi.configure.mockReturnValue(configured.promise);
    newerApi.snapshot.mockResolvedValue(success({ ...active(), epoch: 9 }));
    await act(async () => root.render(<MadoWorkspace key="old" runtime="tauri" privacy={false} onLeave={() => {}} api={oldApi} />));
    await enter(0, A); await act(async () => button("적용").click());
    await act(async () => root.render(<MadoWorkspace key="new" runtime="tauri" privacy={false} onLeave={() => {}} api={newerApi} />));
    await act(async () => configured.resolve(success(active())));
    expect(oldApi.close).toHaveBeenCalledWith(7);
    expect(newerApi.close).not.toHaveBeenCalled();
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
    expect(newerApi.setViewport.mock.calls.every(([, viewport]) => viewport.epoch === 9)).toBe(true);
  });
  it("ignores a late leave completion after unmount instead of leaving a replacement workspace", async () => {
    const oldApi = fake(), newerApi = fake(), closed = deferred<Awaited<ReturnType<MultiviewApi["close"]>>>(), leave = vi.fn();
    oldApi.snapshot.mockResolvedValue(success(active())); oldApi.close.mockReturnValue(closed.promise);
    newerApi.snapshot.mockResolvedValue(success({ ...active(), epoch: 9 }));
    await act(async () => root.render(<MadoWorkspace key="old" runtime="tauri" privacy={false} onLeave={leave} api={oldApi} />));
    await act(async () => button("한 방송 보기").click());
    await act(async () => root.render(<MadoWorkspace key="new" runtime="tauri" privacy={false} onLeave={leave} api={newerApi} />));
    await act(async () => closed.resolve(success(emptyMultiview())));
    expect(leave).not.toHaveBeenCalled();
    expect(button("한 방송 보기")).toBeEnabled();
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
  });
  it("resets a pending action on API replacement and ignores its stale audio response", async () => {
    const oldApi = fake(), newerApi = fake(), audio = deferred<Awaited<ReturnType<MultiviewApi["setPaneAudio"]>>>();
    oldApi.snapshot.mockResolvedValue(success(active())); oldApi.setPaneAudio.mockReturnValue(audio.promise);
    newerApi.snapshot.mockResolvedValue(success({ ...active(), epoch: 9, panes: active().panes.map((pane) => ({ ...pane, audioEnabled: true })) }));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={oldApi} />));
    await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="1. 방송 소리 켜기"]')!.click());
    expect(button("한 방송 보기")).toBeDisabled();
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={newerApi} />));
    expect(button("한 방송 보기")).toBeEnabled();
    await act(async () => audio.resolve(success(active())));
    expect(container.querySelector<HTMLButtonElement>('button[aria-label="1. 방송 소리 끄기"]')).toHaveAttribute("aria-pressed", "true");
    expect(newerApi.setViewport.mock.calls.every(([, viewport]) => viewport.epoch === 9 || viewport.visible === false)).toBe(true);
  });
  it("retries transient multiview viewport contention twice without a permanent retry loop", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    api.setViewport.mockImplementation(async (_paneId, viewport) => viewport.visible
      ? { ok: false, error: { code: "MULTIVIEW_BUSY", message: "영역 연결을 다시 확인해 주세요.", retryable: true } } : success(undefined));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await act(async () => vi.advanceTimersByTimeAsync(500));
    expect(api.setViewport.mock.calls.filter(([, viewport]) => viewport.visible && !viewport.occluded)).toHaveLength(6);
    await act(async () => vi.advanceTimersByTimeAsync(10000));
    // The resulting trusted error popup is a new masking purpose, which has
    // its own bounded retries and fail-closed hides, never a normal re-show.
    const total = api.setViewport.mock.calls.length;
    expect(api.setViewport.mock.calls.filter(([, viewport]) => viewport.visible && !viewport.occluded)).toHaveLength(6);
    expect(api.setViewport.mock.calls.filter(([, viewport]) => viewport.occluded).length).toBeLessThanOrEqual(6);
    await act(async () => vi.advanceTimersByTimeAsync(10000));
    expect(api.setViewport).toHaveBeenCalledTimes(total);
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("영역 연결을 다시 확인");
  });
  it("sends privacy hides immediately and never retries a late visible failure", async () => {
    const api = fake(), visible = deferred<Awaited<ReturnType<MultiviewApi["setViewport"]>>>();
    api.snapshot.mockResolvedValue(success(active()));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    api.setViewport.mockImplementation(async (_paneId, viewport) => viewport.visible ? visible.promise : success(undefined));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(api.setViewport.mock.calls.filter(([, viewport]) => viewport.visible)).toHaveLength(2);
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy onLeave={() => {}} api={api} />));
    expect(api.setViewport.mock.calls.slice(-2).every(([, viewport]) => !viewport.visible)).toBe(true);
    const afterHide = api.setViewport.mock.calls.length;
    await act(async () => visible.resolve({ ok: false, error: { code: "MULTIVIEW_BUSY", message: "stale visible", retryable: true } }));
    await act(async () => vi.advanceTimersByTimeAsync(10000));
    expect(api.setViewport).toHaveBeenCalledTimes(afterHide);
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });
  it("masks every pane for a trusted modal without hiding or stopping media", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    api.setViewport.mockClear();
    const modal = document.createElement("div"); modal.setAttribute("role", "alertdialog");
    try {
      await act(async () => document.body.append(modal));
      expect(api.setViewport.mock.calls).toHaveLength(2);
      expect(api.setViewport.mock.calls.every(([, viewport]) => viewport.visible && viewport.occluded && viewport.width === 400 && viewport.height === 300)).toBe(true);
      await act(async () => { modal.remove(); await vi.advanceTimersByTimeAsync(20); });
      expect(api.setViewport.mock.calls.slice(-2).every(([, viewport]) => viewport.visible && !viewport.occluded)).toBe(true);
      expect(api.setViewport.mock.calls.every(([, viewport]) => viewport.visible)).toBe(true);
      expect(api.close).not.toHaveBeenCalled(); expect(api.setAudio).not.toHaveBeenCalled(); expect(api.setPaneAudio).not.toHaveBeenCalled();
    } finally { modal.remove(); }
  });
  it("hard-hides after a modal transport rejection before reporting its safe error", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(active()));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const rejected = new Set<string>();
    api.setViewport.mockClear().mockImplementation(async (paneId, viewport) => {
      if (viewport.occluded && !rejected.has(paneId)) { rejected.add(paneId); throw new Error("secret transport detail"); }
      return success(undefined);
    });
    const modal = document.createElement("div"); modal.setAttribute("role", "alertdialog");
    try {
      await act(async () => document.body.append(modal));
      expect(api.setViewport.mock.calls.filter(([, viewport]) => !viewport.visible)).toHaveLength(2);
      expect(container.querySelector('[role="alert"]')).toHaveTextContent("시청 영역을 표시하지 못했습니다");
      expect(container).not.toHaveTextContent("secret transport detail");
    } finally { modal.remove(); }
  });
  it("does not let late modal failure hide a newer queued restoration", async () => {
    const api = fake(), visible = deferred<Awaited<ReturnType<MultiviewApi["setViewport"]>>>(), masked = deferred<Awaited<ReturnType<MultiviewApi["setViewport"]>>>();
    api.snapshot.mockResolvedValue(success(active()));
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    api.setViewport.mockImplementation(async (_paneId, viewport) => viewport.occluded ? masked.promise : viewport.visible ? visible.promise : success(undefined));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const modal = document.createElement("div"); modal.setAttribute("role", "alertdialog");
    try {
      await act(async () => document.body.append(modal));
      expect(api.setViewport.mock.calls.filter(([, viewport]) => viewport.occluded)).toHaveLength(2);
      await act(async () => { modal.remove(); await vi.advanceTimersByTimeAsync(20); });
      await act(async () => masked.resolve({ ok: false, error: { code: "VIEWPORT_CLIP_FAILED", message: "stale mask", retryable: true } }));
      await act(async () => visible.resolve(success(undefined)));
      await act(async () => vi.advanceTimersByTimeAsync(1000));
      expect(api.setViewport.mock.calls.every(([, viewport]) => viewport.visible)).toBe(true);
      expect(api.setViewport.mock.calls.slice(-2).every(([, viewport]) => !viewport.occluded)).toBe(true);
      expect(container.querySelector('[role="alert"]')).toBeNull();
    } finally { modal.remove(); }
  });
});
