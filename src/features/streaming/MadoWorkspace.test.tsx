import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { createMultiviewApi, emptyMultiview, multiviewEntries, normalizeMultiviewChannel, type MultiviewApi, type MultiviewSnapshot } from "../../api/multiview";
import { MadoWorkspace, readMadoLayout } from "./MadoWorkspace";
import { createOfficialBrowserApi, emptyOfficialBrowserSnapshot } from "../../api/officialBrowser";
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
beforeEach(() => { localStorage.clear(); vi.mocked(invoke).mockResolvedValue(success(emptyOfficialBrowserSnapshot("tauri"))); });
beforeEach(() => { vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.useFakeTimers(); vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => setTimeout(() => cb(0), 16)); vi.stubGlobal("cancelAnimationFrame", clearTimeout); container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
const button = (text: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find((element) => element.textContent === text)!;
const enter = async (index: number, text: string) => act(async () => {
  const input = container.querySelector<HTMLInputElement>(`#mado-channel-${index}`)!;
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, text); input.dispatchEvent(new Event("input", { bubbles: true }));
});
describe("마도 배치", () => {
  it("forces an account recheck and displays failures without changing the Mado panes", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple()));
    const account = { ...createOfficialBrowserApi("tauri"), snapshot: vi.fn().mockResolvedValue(success({ ...emptyOfficialBrowserSnapshot("tauri"), authStatus: "unknown" })) };
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} officialApi={account} />));
    await act(async () => button("연결 설정").click());
    const panes = [...container.querySelectorAll('.mado-native-slot')];
    const fresh = deferred<ReturnType<typeof success>>();
    account.snapshot.mockReturnValueOnce(fresh.promise);
    await act(async () => button("상태 확인").click());
    expect(account.snapshot).toHaveBeenLastCalledWith(true);
    expect(button("확인 중…")).toBeDisabled();
    await act(async () => fresh.resolve(success({ ...emptyOfficialBrowserSnapshot("tauri"), authStatus: "unknown", authError: "로그인 상태 조회 실패" })));
    expect(container).toHaveTextContent("로그인 상태 조회 실패");
    expect(button("상태 확인")).toBeEnabled();
    expect([...container.querySelectorAll('.mado-native-slot')]).toEqual(panes);
    expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
  });

  it("opens login and logs out directly during Mado viewing without changing its layout", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple()));
    const account = { ...createOfficialBrowserApi("tauri"),
      snapshot: vi.fn().mockResolvedValue(success({ ...emptyOfficialBrowserSnapshot("tauri"), authStatus: "signed_out" })),
      login: vi.fn().mockResolvedValue(success({ ...emptyOfficialBrowserSnapshot("tauri"), authStatus: "signed_in" })),
      logout: vi.fn().mockResolvedValue(success({ ...emptyOfficialBrowserSnapshot("tauri"), authStatus: "signed_out" })),
    };
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} officialApi={account} />));
    await act(async () => button("연결 설정").click());
    const panes = [...container.querySelectorAll('.mado-native-slot')];
    expect(panes).toHaveLength(8);
    expect(button("로그인")).toBeEnabled();
    await act(async () => button("로그인").click());
    expect(account.login).toHaveBeenCalledOnce();
    expect(button("로그아웃")).toBeEnabled();
    await act(async () => button("로그아웃").click());
    expect(account.logout).toHaveBeenCalledOnce();
    expect(container).toHaveTextContent("로그아웃됨");
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expect([...container.querySelectorAll('.mado-native-slot')]).toEqual(panes);
    expect(api.close).not.toHaveBeenCalled(); expect(api.configure).not.toHaveBeenCalled();
  });

  it.each(["recording", "account"])("blocks account actions while %s is busy, not just because Mado is active", async (reason) => {
    const snapshot = multiple(); if (reason === "recording") snapshot.panes[0]!.recordingStatus = "recording";
    const api = fake(); api.snapshot.mockResolvedValue(success(snapshot));
    const account = { ...createOfficialBrowserApi("tauri"), snapshot: vi.fn().mockResolvedValue(success({ ...emptyOfficialBrowserSnapshot("tauri"), authStatus: "unknown", accountBusy: reason === "account" })), login: vi.fn(), logout: vi.fn() };
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} officialApi={account} />));
    await act(async () => button("연결 설정").click());
    expect(button("로그인")).toBeDisabled(); expect(button("로그아웃")).toBeDisabled();
    await act(async () => { button("로그인").click(); button("로그아웃").click(); });
    expect(account.login).not.toHaveBeenCalled(); expect(account.logout).not.toHaveBeenCalled();
  });

  it("names URL fields accessibly without visible numbered captions and uses the requested layout names", async () => {
    const api = fake();
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(button("4화면4챗")).toHaveAttribute("aria-pressed", "true");
    expect(button("1화면4챗")).toHaveAttribute("aria-pressed", "false");
    for (const [index, input] of [...container.querySelectorAll<HTMLInputElement>('.connection-channel-inputs input')].entries()) {
      expect(input).toHaveAccessibleName(`방송 ${index + 1} 주소 또는 ID`);
      expect(input.placeholder).toBe("https://chzzk.naver.com/live/채널ID");
      expect(input.labels?.[0]?.querySelector('span')).toHaveClass("connection-input-label-hidden");
    }
  });
  it("keeps collapsed help out of keyboard navigation and reveals it below the heading without remounting", async () => {
    const api = fake();
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const help = container.querySelector<HTMLButtonElement>('.connection-help')!;
    const reveal = container.querySelector('.connection-help-reveal')!;
    const content = container.querySelector('.connection-help-content')!;
    expect(reveal).toHaveAttribute("aria-hidden", "true"); expect(reveal).toHaveAttribute("inert");
    expect(button("Chrome")).toBeDisabled();
    await act(async () => help.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(help).toHaveAttribute("aria-expanded", "true"); expect(reveal).not.toHaveAttribute("inert");
    expect(reveal.previousElementSibling).toHaveClass("connection-setup-heading");
    expect(button("Chrome")).toBeEnabled();
    await act(async () => help.focus());
    expect(help).toHaveAccessibleDescription(/주소를 입력하고 시청을 시작하세요/);
    await act(async () => help.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(document.activeElement).toBe(help); expect(reveal).toHaveAttribute("inert");
    expect(reveal).toHaveAttribute("aria-hidden", "true"); expect(button("Chrome")).toBeDisabled();
    expect(container.querySelector('.connection-help-content')).toBe(content);
    expect(container.querySelector('[aria-label="연결 설정"][role="dialog"]')).not.toBeNull();
    expect(api.configure).not.toHaveBeenCalled();
  });
  it.each([false, true])("keeps video and chat panes free of duplicate outside headers and audio controls (one video: %s)", async (oneVideo) => {
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple(oneVideo)));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(container.querySelector('.mado-pair > .mado-pane-label, .mado-lead-inner > .mado-pane-label')).toBeNull();
    expect(container.querySelectorAll('.mado-pane-label, .mado-audio, .mado-pane-name')).toHaveLength(0);
    expect(container.querySelectorAll('.mado-chat-cell')).toHaveLength(4);
    expect([...container.querySelectorAll('.mado-chat-cell')].every(cell => cell.children.length === 1 && cell.firstElementChild?.classList.contains('mado-native-slot'))).toBe(true);
    expect(container.querySelectorAll('.mado-native-slot.is-video')).toHaveLength(oneVideo ? 1 : 4);
  });
  it("keeps chat failure and gap warnings in the workspace toolbar even after recording stops", async () => {
    const api = fake(); let state = multiple();
    state.panes[0] = { ...state.panes[0]!, recordingStatus: "stopped", recordingId: null, chatStatus: "storage_failed", chatCount: 42 };
    api.snapshot.mockImplementation(async () => success(state));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    const slots = [...container.querySelectorAll(".mado-native-slot")];
    const warning = container.querySelector<HTMLElement>(".mado-chat-warning")!;
    expect(warning).toHaveTextContent("⚠"); expect(warning).toHaveAttribute("tabindex", "0");
    expect(warning.title).toContain("채팅 저장 실패"); expect(warning.title).toContain("42개 기록");
    expect(warning).toHaveAccessibleName(warning.title);
    expect(warning.closest(".mado-toolbar")).not.toBeNull();
    expect(warning).toHaveTextContent("1. 도서관 1");
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
    await act(async () => button("연결 설정").click());
    expect(button("일반모드")).toBeDisabled(); expect(button("시청 시작")).toBeDisabled();
    await act(async () => root.render(null));
    expect(api.close).not.toHaveBeenCalled(); expect(leave).not.toHaveBeenCalled();
    expect(api.setViewport.mock.calls.slice(-state.panes.length).every(([, viewport]) => !viewport.visible)).toBe(true);
  });
  it("targets the sole video for app-level S and keeps saving behind confirmation", async () => {
    const api = fake(), state = multiple(true); api.snapshot.mockResolvedValue(success(state));
    api.requestControl.mockImplementation(async (paneId, action) => success({ ...state, pendingControl: { paneId, id: "key-save", action, channelId: A, expiresAt: Date.now() + 20000 } }));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true })));
    expect(api.requestControl).toHaveBeenCalledExactlyOnceWith("video-0", "screenshot", 7);
    expect(button("저장 확인")).toBeEnabled(); expect(api.confirmControl).not.toHaveBeenCalled();
    await act(async () => container.querySelector('[role="alertdialog"]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(api.confirmControl.mock.calls[0]?.[0]).toMatchObject({ paneId: "video-0", approve: false, rightsAcknowledged: false });
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
  it("leaves each native player's audio and S controls in charge without an outside duplicate", async () => {
    const api = fake(); const state = multiple(); api.snapshot.mockResolvedValue(success(state));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(container.querySelector('.mado-audio, .mado-pane-name')).toBeNull();
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true })));
    expect(api.requestControl).not.toHaveBeenCalled();
    expect(api.setPaneAudio).not.toHaveBeenCalled(); expect(api.setAudio).not.toHaveBeenCalled();
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy onLeave={() => {}} api={api} />));
    expect(container).not.toHaveTextContent("도서관 1");
  });
  it("does not show another confirmation for a native per-pane recording intent", async () => {
    const api = fake(); const state = multiple();
    state.pendingControl = { paneId: "video-1", id: "record-second", action: "record_start", channelId: B, expiresAt: Date.now() + 20000 };
    api.snapshot.mockResolvedValue(success(state));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expect(api.confirmControl).not.toHaveBeenCalled();
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
    expect(button("시청 시작")).toBeDisabled(); expect(api.configure).not.toHaveBeenCalled();
    await enter(0, A); await act(async () => button("시청 시작").click());
    expect(api.configure).toHaveBeenCalledWith([{ channelId: A, video: true, chat: true }]);
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
  });
  it("keeps a denied recording-active transition visible instead of pretending to open panes", async () => {
    const api = fake(); api.configure.mockResolvedValue({ ok: false, error: { code: "RECORDING_ACTIVE", message: "녹화를 먼저 중지해 주세요.", retryable: true } });
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await enter(0, A); await act(async () => button("시청 시작").click());
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("녹화를 먼저 중지");
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(0);
  });
  it("closes the expected epoch before leaving and hides panes for privacy", async () => {
    const api = fake(), leave = vi.fn(); api.snapshot.mockResolvedValue(success(active()));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy onLeave={leave} api={api} />));
    expect(api.setViewport.mock.calls.every(([, viewport]) => viewport.visible === false && viewport.epoch === 7)).toBe(true);
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={leave} api={api} />));
    await act(async () => button("연결 설정").click());
    await act(async () => button("일반모드").click());
    expect(api.close).not.toHaveBeenCalled();
    await act(async () => button("시청 시작").click());
    expect(api.close).toHaveBeenCalledWith(7); expect(leave).toHaveBeenCalledOnce();
    expect(invoke).toHaveBeenCalledWith("chzzk_browser_open", { input: A });
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
    await enter(0, A); await act(async () => button("시청 시작").click());
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
    await act(async () => initial.resolve(success(emptyMultiview())));
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
    expect(container.querySelector(".connection-setup")).toBeNull();
  });
  it("restores channel drafts and the selected video when reopening an existing one-video layout", async () => {
    const api = fake();
    api.snapshot.mockResolvedValue(success({ active: true, epoch: 9, audioOwner: B, panes: [A, B, C, D].flatMap((channelId) => [
      ...(channelId === B ? [{ paneId: `restored-${channelId}-video`, channelId, kind: "video" as const, status: "ready" }] : []),
      { paneId: `restored-${channelId}-chat`, channelId, kind: "chat" as const, status: "ready" },
    ]) }));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(container.querySelector(".mado-surfaces")).toHaveClass("is-chats");
    await act(async () => button("연결 설정").click());
    expect([0, 1, 2, 3].map((index) => container.querySelector<HTMLInputElement>(`#mado-channel-${index}`)!.value)).toEqual([A, B, C, D]);
    expect(button("1화면4챗")).toHaveAttribute("aria-pressed", "true");
    expect(container.querySelector<HTMLSelectElement>(".connection-setup select")?.value).toBe("1");
    expect(api.configure).not.toHaveBeenCalled();
  });
  it("styles the one-video selector as a connection field and only applies its choice on connect", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(success(multiple(true)));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await act(async () => button("연결 설정").click());
    const panes = [...container.querySelectorAll(".mado-native-slot")];
    const select = container.querySelector<HTMLSelectElement>(".mado-broadcast-select select")!;
    expect(select).toHaveAccessibleName("영상 방송");
    expect(select.value).toBe("0");
    expect([...select.options].map(option => option.textContent)).toEqual(["방송 1", "방송 2", "방송 3", "방송 4"]);
    expect(select.nextElementSibling).toHaveAttribute("aria-hidden", "true");
    await act(async () => { select.value = "2"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(select.value).toBe("2");
    expect([...container.querySelectorAll(".mado-native-slot")]).toEqual(panes);
    expect(api.configure).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
    await act(async () => button("시청 시작").click());
    expect(api.configure).toHaveBeenCalledExactlyOnceWith([
      { channelId: A, video: false, chat: true }, { channelId: B, video: false, chat: true },
      { channelId: C, video: true, chat: true }, { channelId: D, video: false, chat: true },
    ]);
  });
  it("does not overwrite a channel draft edited before the initial snapshot arrives", async () => {
    const api = fake(), initial = deferred<Awaited<ReturnType<MultiviewApi["snapshot"]>>>();
    api.snapshot.mockReturnValue(initial.promise);
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    await enter(0, B);
    await act(async () => initial.resolve(success(active())));
    expect(container.querySelector<HTMLInputElement>("#mado-channel-0")!.value).toBe(B);
    expect(container.querySelector("#mado-channel-0")).not.toBeNull();
    expect(api.configure).not.toHaveBeenCalled();
  });
  it("closes a configure result that arrives after unmount without changing the new workspace", async () => {
    const oldApi = fake(), newerApi = fake(), configured = deferred<Awaited<ReturnType<MultiviewApi["configure"]>>>();
    oldApi.configure.mockReturnValue(configured.promise);
    newerApi.snapshot.mockResolvedValue(success({ ...active(), epoch: 9 }));
    await act(async () => root.render(<MadoWorkspace key="old" runtime="tauri" privacy={false} onLeave={() => {}} api={oldApi} />));
    await enter(0, A); await act(async () => button("시청 시작").click());
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
    await act(async () => button("연결 설정").click());
    await act(async () => button("일반모드").click());
    await act(async () => button("시청 시작").click());
    await act(async () => root.render(<MadoWorkspace key="new" runtime="tauri" privacy={false} onLeave={leave} api={newerApi} />));
    await act(async () => closed.resolve(success(emptyMultiview())));
    expect(leave).not.toHaveBeenCalled();
    expect(button("연결 설정")).toBeEnabled();
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
  });
  it("resets a pending action on API replacement and ignores its stale screenshot response", async () => {
    const oldApi = fake(), newerApi = fake(), capture = deferred<Awaited<ReturnType<MultiviewApi["requestControl"]>>>();
    oldApi.snapshot.mockResolvedValue(success(active())); oldApi.requestControl.mockReturnValue(capture.promise);
    newerApi.snapshot.mockResolvedValue(success({ ...active(), epoch: 9, panes: active().panes.map((pane) => ({ ...pane, audioEnabled: true })) }));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={oldApi} />));
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true })));
    expect(oldApi.requestControl).toHaveBeenCalledOnce();
    await act(async () => button("연결 설정").click());
    expect(button("일반모드")).toBeDisabled();
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={newerApi} />));
    await act(async () => button("연결 설정").click());
    expect(button("일반모드")).toBeEnabled();
    await act(async () => capture.resolve(success(active())));
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(2);
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
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
  it.each(["privacy", "unmount"] as const)("does not revive panes from an old animation frame after immediate popup masking and %s", async (transition) => {
    const api = fake(), state = multiple(); api.snapshot.mockResolvedValue(success(state));
    const pendingFrames = new Map<number, FrameRequestCallback>();
    let nextFrame = 0;
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      const id = ++nextFrame; pendingFrames.set(id, callback); return id;
    });
    const cancelFrame = vi.fn((id: number) => { pendingFrames.delete(id); });
    vi.stubGlobal("cancelAnimationFrame", cancelFrame);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 400, 300));
    await act(async () => root.render(<MadoWorkspace runtime="tauri" privacy={false} onLeave={() => {}} api={api} />));
    expect(container.querySelectorAll(".mado-native-slot")).toHaveLength(8);
    expect(api.setViewport.mock.calls.slice(-8).every(([, viewport]) => viewport.visible && !viewport.occluded)).toBe(true);
    await act(async () => window.dispatchEvent(new Event("resize")));
    const savedFrames = [...pendingFrames.entries()];
    expect(savedFrames).toHaveLength(8);
    const modal = document.createElement("div"), surface = document.createElement("div");
    modal.setAttribute("role", "alertdialog"); modal.dataset.nativePreserveVideo = "true";
    surface.dataset.nativeDialogSurface = "true";
    Object.defineProperty(surface, "getBoundingClientRect", { value: () => new DOMRect(80, 60, 120, 100) });
    modal.append(surface);
    try {
      api.setViewport.mockClear();
      await act(async () => document.body.append(modal));
      // Popup mutation must preempt a pending normal frame without shrinking
      // any of the four video/chat pairs or abandoning its cancellation ID.
      expect(api.setViewport.mock.calls).toHaveLength(8);
      for (const [, viewport] of api.setViewport.mock.calls) {
        expect(viewport).toMatchObject({ visible: true, occluded: true, preserveBackground: true, width: 400, height: 300 });
        expect(viewport.occlusions).toHaveLength(1);
      }
      for (const [id] of savedFrames) expect(cancelFrame).toHaveBeenCalledWith(id);
      await act(async () => root.render(transition === "privacy"
        ? <MadoWorkspace runtime="tauri" privacy onLeave={() => {}} api={api} /> : null));
      expect(api.setViewport.mock.calls.slice(-8).every(([, viewport]) => !viewport.visible)).toBe(true);
      await act(async () => modal.remove());
      api.setViewport.mockClear();
      // Force canceled callbacks to run as if already queued by the browser.
      // An old non-private closure must not send even a masked visible update.
      await act(async () => { for (const [, callback] of savedFrames) callback(32); });
      expect(api.setViewport).not.toHaveBeenCalled();
      expect(api.close).not.toHaveBeenCalled(); expect(api.confirmControl).not.toHaveBeenCalled();
    } finally { modal.remove(); }
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
