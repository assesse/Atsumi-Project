import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AutoRecordingPanel } from "./AutoRecordingPanel";
import type { AutoRecordingSnapshot, AutoRecordingApi } from "../../api/autoRecording";

const ID = "a".repeat(32);
const data = (status = "waiting"): AutoRecordingSnapshot => ({ captureChat: true, error: null, channels: [{ channelId: ID, channelName: "등록한 채널", enabled: true, checkedAt: 1000, status, recordingId: status === "recording" ? "recording-1" : null, message: null }] });
const ok = (snapshot = data()) => ({ ok: true as const, data: snapshot });
let container: HTMLDivElement, root: Root;
beforeEach(() => { vi.useFakeTimers(); container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); });
const button = (text: string) => [...container.querySelectorAll("button")].find(button => button.textContent === text)!;
function fake() { return { snapshot: vi.fn().mockResolvedValue(ok()), add: vi.fn().mockResolvedValue(ok()), update: vi.fn().mockResolvedValue(ok()) } satisfies AutoRecordingApi; }
describe("automatic recording registration", () => {
  it("has no live-view entry even when an automatic recording is active", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(ok(data("recording")));
    await act(async () => root.render(<AutoRecordingPanel runtime="tauri" api={api} />));
    expect(button("실시간 보기")).toBeUndefined();
    expect(button("녹화 중지")).toBeEnabled();
    expect(api.update).not.toHaveBeenCalled(); expect(api.add).not.toHaveBeenCalled();
    api.snapshot.mockResolvedValue(ok(data("stopping")));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(button("실시간 보기")).toBeUndefined();
  });
  it("keeps channel identity private without adding a playback entry", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(ok(data("recording")));
    await act(async () => root.render(<AutoRecordingPanel runtime="tauri" api={api} privacy />));
    expect(container.innerHTML).not.toContain("등록한 채널");
    expect(container.innerHTML).not.toContain(ID);
    expect(button("실시간 보기")).toBeUndefined(); expect(api.update).not.toHaveBeenCalled();
  });
  it("does not stop a recording when leaving its reservation list", async () => {
    const api = fake(); api.snapshot.mockResolvedValue(ok(data("recording")));
    await act(async () => root.render(<AutoRecordingPanel runtime="tauri" api={api} />));
    await act(async () => root.render(null));
    const polls = api.snapshot.mock.calls.length;
    await act(async () => vi.advanceTimersByTimeAsync(10_000));
    expect(api.snapshot).toHaveBeenCalledTimes(polls); expect(api.update).not.toHaveBeenCalled();
  });
  it("shows persisted channels and updates live progress without a confirmation", async () => {
    const api = fake();
    await act(async () => root.render(<AutoRecordingPanel runtime="tauri" api={api} />));
    expect(container).toHaveTextContent("등록한 채널"); expect(container).toHaveTextContent("방송 대기");
    api.snapshot.mockResolvedValue(ok(data("recording")));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(container).toHaveTextContent("녹화 중");
    await act(async () => button("녹화 중지").click());
    expect(api.update).toHaveBeenCalledExactlyOnceWith(ID, { action: "stop" });
    expect(container.querySelector('[role="dialog"], [role="alertdialog"]')).toBeNull();
  });
  it("registers only the entered channel and clears the input on success", async () => {
    const api = fake();
    await act(async () => root.render(<AutoRecordingPanel runtime="tauri" api={api} />));
    const input = container.querySelector("input")!;
    await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, ` https://chzzk.naver.com/live/${ID} `); input.dispatchEvent(new Event("input", { bubbles: true })); });
    await act(async () => button("등록").click());
    expect(api.add).toHaveBeenCalledExactlyOnceWith(`https://chzzk.naver.com/live/${ID}`); expect(input.value).toBe("");
  });
  it("distinguishes preparation from saved recording and lets the user cancel it", async () => {
    const api = fake();
    api.snapshot.mockResolvedValue(ok(data("starting")));
    await act(async () => root.render(<AutoRecordingPanel runtime="tauri" api={api} />));
    expect(container).toHaveTextContent("아직 저장이 시작되지 않았습니다");
    expect(button("녹화 중지")).toBeUndefined();
    await act(async () => button("시도 중지").click());
    expect(api.update).toHaveBeenCalledExactlyOnceWith(ID, { action: "stop" });
  });
  it("keeps the failure reason after stopping or remounting without leaking source diagnostics", async () => {
    const api = fake();
    const failure = { occurredAt: 1_789_400_000_000, liveKey: "id:private-live-key", stage: "player_prepare", message: "재생 가능한 영상을 받지 못해 저장을 시작하지 못했습니다." };
    const snapshot = data("retry");
    snapshot.channels[0]!.lastFailure = failure;
    api.snapshot.mockResolvedValue(ok(snapshot));
    await act(async () => root.render(<AutoRecordingPanel runtime="tauri" api={api} privacy />));
    expect(container).toHaveTextContent("최근 저장 미시작");
    expect(container).toHaveTextContent(failure.message);
    expect(container.innerHTML).not.toContain(failure.liveKey);
    expect(container.innerHTML).not.toContain(failure.stage);
    expect(container).not.toHaveTextContent("등록한 채널");
    const stopped = { ...snapshot, channels: [{ ...snapshot.channels[0]!, status: "skipped" }] };
    api.update.mockResolvedValue(ok(stopped));
    await act(async () => button("시도 중지").click());
    expect(api.update).toHaveBeenCalledExactlyOnceWith(ID, { action: "stop" });
    expect(container).toHaveTextContent("이번 방송 건너뜀");
    expect(container).toHaveTextContent(failure.message);
    await act(async () => root.render(null));
    api.snapshot.mockResolvedValue(ok(stopped));
    await act(async () => root.render(<AutoRecordingPanel runtime="tauri" api={api} />));
    expect(container).toHaveTextContent(failure.message);
  });
  it("unregisters without deleting saved recordings and fences stale poll results", async () => {
    const api = fake();
    await act(async () => root.render(<AutoRecordingPanel runtime="tauri" api={api} />));
    let resolve!: (value: ReturnType<typeof ok>) => void;
    api.snapshot.mockReturnValue(new Promise(done => { resolve = done; }));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    api.update.mockResolvedValue(ok({ ...data(), channels: [] }));
    await act(async () => button("해제").click());
    await act(async () => resolve(ok()));
    expect(api.update).toHaveBeenCalledExactlyOnceWith(ID, { action: "remove" });
    expect(container).not.toHaveTextContent("등록한 채널");
  });
  it("removes the previous failure when the next recording has actually started", async () => {
    const api = fake();
    const preparing = data("starting");
    preparing.channels[0]!.lastFailure = { occurredAt: 1_789_400_000_000, liveKey: "id:old", stage: "receiver_create", message: "이전 시작 실패" };
    api.snapshot.mockResolvedValue(ok(preparing));
    await act(async () => root.render(<AutoRecordingPanel runtime="tauri" api={api} />));
    expect(container).toHaveTextContent("이전 시작 실패");
    api.snapshot.mockResolvedValue(ok(data("recording")));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(container).toHaveTextContent("녹화 중");
    expect(container).not.toHaveTextContent("최근 저장 미시작");
    expect(container).not.toHaveTextContent("이전 시작 실패");
    expect(api.update).not.toHaveBeenCalled();
  });
  it("protects channel identity in privacy mode and reports failed mutations", async () => {
    const api = fake();
    api.update.mockResolvedValue({ ok: false, error: { code: "FAIL", message: "저장 실패", retryable: true } });
    await act(async () => root.render(<AutoRecordingPanel runtime="tauri" api={api} privacy />));
    expect(container).not.toHaveTextContent("등록한 채널"); expect(container.innerHTML).not.toContain(ID);
    await act(async () => button("켜짐").click());
    expect(api.update).toHaveBeenCalledExactlyOnceWith(ID, { action: "enabled", enabled: false });
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("저장 실패");
  });
});
