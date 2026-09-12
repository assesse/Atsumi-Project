import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiResult } from "../../api/contracts";
import type { ReplayApi } from "../../api/replay";
import {
  claimOfficialBrowserViewport, createOfficialBrowserApi, emptyOfficialBrowserSnapshot, hiddenOfficialBrowserViewport,
  type BrowserRecording, type OfficialBrowserApi, type OfficialBrowserSnapshot, type OfficialBrowserViewport,
} from "../../api/officialBrowser";
import { measureOfficialBrowserViewport, OfficialBrowserPanel } from "./OfficialBrowserPanel";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const success = <T,>(data: T): ApiResult<T> => ({ ok: true, data });
const ready = (patch: Partial<OfficialBrowserSnapshot> = {}): OfficialBrowserSnapshot => ({
  ...emptyOfficialBrowserSnapshot("tauri"), windowOpen: true, ready: true,
  status: "ready", channelId: "a".repeat(32), ...patch,
});
const recordingFixture = (patch: Partial<BrowserRecording> = {}): BrowserRecording => ({
  id: "official-recording-1", channelId: "a".repeat(32), title: "공식 방송 테스트",
  startedAt: 1_800_000_000_000, updatedAt: 1_800_000_030_000,
  status: "recording", mimeType: "video/webm", outputDir: "C:\\Recordings\\official-1",
  segmentCount: 1, bytesWritten: 1024 * 1024, durationSeconds: 30, lastError: null,
  segments: [{ index: 0, file: "segment-000000.webm", bytes: 1024 * 1024, durationSeconds: 30 }],
  ...patch,
});
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => { resolve = complete; });
  return { promise, resolve };
};
const fakeApi = () => ({
  runtime: "tauri" as const,
  open: vi.fn<OfficialBrowserApi["open"]>().mockResolvedValue(success(ready())),
  snapshot: vi.fn<OfficialBrowserApi["snapshot"]>().mockResolvedValue(success(ready())),
  start: vi.fn<OfficialBrowserApi["start"]>().mockResolvedValue(success(ready({ status: "recording", recordingId: "official-recording-1", recordings: [recordingFixture()] }))),
  stop: vi.fn<OfficialBrowserApi["stop"]>().mockResolvedValue(success(ready({ status: "stopping", recordingId: "official-recording-1", recordings: [recordingFixture()] }))),
  connectExtension: vi.fn<OfficialBrowserApi["connectExtension"]>().mockResolvedValue(success(ready({ extensionStatus: "연결됨" }))),
  login: vi.fn<OfficialBrowserApi["login"]>().mockResolvedValue(success(ready({ loginStatus: "login_pending" }))),
  logout: vi.fn<OfficialBrowserApi["logout"]>().mockResolvedValue(success(ready({ loginStatus: "signed_out" }))),
  openInstaller: vi.fn<OfficialBrowserApi["openInstaller"]>().mockResolvedValue(success(undefined)),
  setViewport: vi.fn<OfficialBrowserApi["setViewport"]>().mockResolvedValue(success(undefined)),
  confirmControl: vi.fn<OfficialBrowserApi["confirmControl"]>().mockResolvedValue(success(ready())),
  requestControl: vi.fn<OfficialBrowserApi["requestControl"]>().mockResolvedValue(success(ready())),
  ackUiAction: vi.fn<OfficialBrowserApi["ackUiAction"]>().mockResolvedValue(success(ready())),
  openFolder: vi.fn<OfficialBrowserApi["openFolder"]>().mockResolvedValue(success(undefined)),
  openSegment: vi.fn<OfficialBrowserApi["openSegment"]>().mockResolvedValue(success(undefined)),
  openMerged: vi.fn<OfficialBrowserApi["openMerged"]>().mockResolvedValue(success(undefined)),
  retryMerge: vi.fn<OfficialBrowserApi["retryMerge"]>().mockResolvedValue(success(ready())),
});

let container: HTMLDivElement;
let root: Root;
const button = (text: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent === text)!;
const help = (label: string) => container.querySelector<HTMLButtonElement>(`button[aria-label="${label} 도움말"]`)!;
const render = async (api: OfficialBrowserApi, options: { active?: boolean; view?: "live" | "recordings"; privacyMode?: boolean; replayApi?: ReplayApi } = {}) => {
  await act(async () => { root.render(<OfficialBrowserPanel runtime={api.runtime} api={api} active={options.active ?? true} view={options.view ?? "live"} privacyMode={options.privacyMode} replayApi={options.replayApi} />); });
};
const enterChannel = async (value = " https://chzzk.naver.com/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ") => {
  await act(async () => {
    const input = container.querySelector<HTMLInputElement>("#official-browser-channel")!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
};
const consent = async () => { await act(async () => container.querySelector<HTMLInputElement>(".official-browser-consent input")!.click()); };
let nextUiIntent = 0;
const deliverUiAction = async (api: ReturnType<typeof fakeApi>, action: "open_settings" | "toggle_focus" | "exit_focus", snapshot = ready(), id = `ui-${++nextUiIntent}`) => {
  api.snapshot.mockResolvedValueOnce(success({ ...snapshot, pendingUiAction: { id, action, expiresAt: Date.now() + 8000 } }));
  await act(async () => vi.advanceTimersByTimeAsync(2000));
  expect(api.ackUiAction).toHaveBeenCalledWith(id);
};
const openSettings = async (api: ReturnType<typeof fakeApi>, snapshot = ready()) => {
  await deliverUiAction(api, "open_settings", snapshot);
  expect(button("설정 닫기")).toBeEnabled();
};
const closeSettings = async () => {
  await act(async () => button("설정 닫기").click());
  await act(async () => vi.advanceTimersByTimeAsync(20));
};
const expectNoLiveToolbar = () => {
  expect(container.querySelector(".official-browser-heading,.official-browser-quickbar,.official-browser-actions,.official-browser-consent,.official-browser-settings")).toBeNull();
  expect(button("고화질 연결")).toBeUndefined();
  expect(button("녹화 시작")).toBeUndefined();
  expect(button("녹화 중지")).toBeUndefined();
  expect(button("넓게 보기")).toBeUndefined();
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(performance.now()), 16));
  vi.stubGlobal("cancelAnimationFrame", (id: number) => window.clearTimeout(id));
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("OfficialBrowserPanel", () => {
  it("explains the site's first-view quality choice without starting playback or recording", async () => {
    const api = fakeApi();
    api.snapshot.mockResolvedValue(success(ready({ ready: false, status: "waiting" })));
    await render(api);
    expectNoLiveToolbar();
    await openSettings(api, ready({ ready: false, status: "waiting" }));
    expect(container).not.toHaveTextContent("설치없이 일반 화질 시청");
    await act(async () => help("시청 시작").focus());
    expect(container.querySelector('[role="tooltip"]')).toHaveTextContent("설치없이 일반 화질 시청");
    await consent();
    expect(button("녹화 시작")).toBeDisabled();
    expect(api.start).not.toHaveBeenCalled();
    api.snapshot.mockResolvedValue(success(ready()));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(container).not.toHaveTextContent("설치없이 일반 화질 시청");
    expect(button("녹화 시작")).toBeEnabled();
  });

  it("opens the supplied channel without automatically recording and connects extensions explicitly", async () => {
    const api = fakeApi();
    api.snapshot.mockResolvedValue(success(emptyOfficialBrowserSnapshot("tauri")));
    await render(api);
    expect(container.querySelector('.official-browser-settings')).toHaveAttribute('open');
    expect(container.querySelector('.official-browser-settings')?.closest('.official-browser-stage')).not.toBeNull();
    expect(button("시청 시작")).toBeDisabled();
    await enterChannel();
    await act(async () => button("시청 시작").click());
    expect(api.open).toHaveBeenCalledExactlyOnceWith("https://chzzk.naver.com/live/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    expectNoLiveToolbar();
    api.snapshot.mockResolvedValue(success(ready()));
    await openSettings(api);
    expect(button("고화질 연결").closest("details")).toBeNull();
    expect(container).toHaveTextContent("녹화 준비됨");
    expect(api.start).not.toHaveBeenCalled();
    expect(button("녹화 시작")).toBeDisabled();
    await act(async () => button("고화질 연결").click());
    expect(api.connectExtension).toHaveBeenCalledTimes(1);
    expect(container).toHaveTextContent("확장 · 연결됨");
    await act(async () => help("고화질 연결").focus());
    expect(container.querySelector('[role="tooltip"]')).toHaveTextContent("설치만으로 연결되지는 않습니다.");
  });

  it("requires rights, locks duplicate start/stop requests, and shows file finalization", async () => {
    const api = fakeApi();
    const startResult = deferred<ApiResult<OfficialBrowserSnapshot>>();
    api.start.mockReturnValue(startResult.promise);
    await render(api);
    await openSettings(api);
    await act(async () => button("녹화 시작").click());
    expect(api.start).not.toHaveBeenCalled();
    await consent();
    await act(async () => {
      const start = button("녹화 시작");
      start.click();
      start.click();
    });
    expect(api.start).toHaveBeenCalledExactlyOnceWith({ rightsAcknowledged: true, captureChat: true });
    expect(button("시작 중…")).toBeDisabled();
    await act(async () => startResult.resolve(success(ready({ status: "recording", recordingId: "official-recording-1", recordings: [recordingFixture()] }))));
    expect(container).toHaveTextContent("녹화 중");
    await act(async () => help("녹화 상태").focus());
    expect(container).toHaveTextContent("00:00:30");
    await act(async () => help("녹화 상태").blur());
    expect(container.querySelector("#official-browser-channel")).toBeDisabled();
    const stopped = deferred<ApiResult<OfficialBrowserSnapshot>>();
    api.stop.mockReturnValue(stopped.promise);
    await act(async () => {
      const stop = button("녹화 중지");
      stop.click();
      stop.click();
    });
    expect(api.stop).toHaveBeenCalledTimes(1);
    expect(button("저장 중…")).toBeDisabled();
    await act(async () => stopped.resolve(success(ready({ recordings: [recordingFixture({ status: "stopped" })] }))));
    await act(async () => help("녹화 상태").focus());
    expect(container).toHaveTextContent("저장 완료");
    await act(async () => help("녹화 상태").blur());
    expect(button("녹화 중지")).toBeUndefined();
    await act(async () => help("녹화").focus());
    expect(container.querySelector('[role="tooltip"]')).toHaveTextContent("과거 탐색·배속 변경·채널 변경");
  });

  it("recovers polling errors and never lets an old poll undo an open action", async () => {
    const api = fakeApi();
    api.snapshot.mockResolvedValueOnce({ ok: false, error: { code: "NOT_READY", message: "공식 연결 확인 실패", retryable: true } });
    await render(api);
    expect(container).toHaveTextContent("공식 연결 확인 실패");
    await consent();
    expect(button("녹화 시작")).toBeDisabled();
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(container).not.toHaveTextContent("공식 연결 확인 실패");
    await openSettings(api);
    expect(button("녹화 시작")).toBeEnabled();

    const oldPoll = deferred<ApiResult<OfficialBrowserSnapshot>>();
    api.snapshot.mockReturnValueOnce(oldPoll.promise);
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    await enterChannel();
    await act(async () => button("시청 시작").click());
    await act(async () => oldPoll.resolve(success(emptyOfficialBrowserSnapshot("tauri"))));
    expect(container).toHaveTextContent("녹화 준비됨");
    expectNoLiveToolbar();
  });

  it("shows host compatibility/action failures without claiming recording started", async () => {
    const api = fakeApi();
    api.snapshot.mockResolvedValue(success(ready({ ready: false, status: "error", error: "공식 창에서 로그인과 재생을 확인해 주세요." })));
    await render(api);
    expect(container).toHaveTextContent("공식 창에서 로그인과 재생을 확인해 주세요.");
    await openSettings(api, ready({ ready: false, status: "error", error: "공식 창에서 로그인과 재생을 확인해 주세요." }));
    await consent();
    expect(button("녹화 시작")).toBeDisabled();
    api.snapshot.mockResolvedValue(success(ready()));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    api.start.mockResolvedValue({ ok: false, error: { code: "CAPTURE_UNAVAILABLE", message: "이 플레이어는 화면 녹화를 지원하지 않습니다.", retryable: false } });
    await act(async () => button("녹화 시작").click());
    expect(container).toHaveTextContent("이 플레이어는 화면 녹화를 지원하지 않습니다.");
    expect(container).not.toHaveTextContent("공식 화면 녹화를 요청했습니다.");
    expect(button("녹화 중지")).toBeUndefined();
  });

  it("pauses only observation in other modes and ignores late responses after deactivation", async () => {
    const api = fakeApi();
    await render(api, { active: false });
    await act(async () => vi.advanceTimersByTimeAsync(6000));
    expect(api.snapshot).not.toHaveBeenCalled();
    expect(container).toBeEmptyDOMElement();
    api.snapshot.mockResolvedValueOnce(success(ready({ status: "recording", recordingId: "official-recording-1", recordings: [recordingFixture()] })));
    await render(api);
    expect(container).toHaveTextContent("녹화 중");
    const oldPoll = deferred<ApiResult<OfficialBrowserSnapshot>>();
    api.snapshot.mockReturnValueOnce(oldPoll.promise);
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    await render(api, { active: false });
    await act(async () => { oldPoll.resolve(success(ready())); await vi.advanceTimersByTimeAsync(6000); });
    expect(api.snapshot).toHaveBeenCalledTimes(2);
    expect(api.stop).not.toHaveBeenCalled();
    expect(api.start).not.toHaveBeenCalled();
    expect(container).toBeEmptyDOMElement();
  });

  it("lists recovered recordings separately and opens only selected completed files by id/index", async () => {
    const api = fakeApi();
    api.snapshot.mockResolvedValue(success(ready({ recordings: [recordingFixture({
      status: "interrupted", lastError: "이전 실행 중 녹화가 중단되었습니다.", segmentCount: 300,
      partial: { index: 300, file: "partial.webm", bytes: 10 },
    })] })));
    await render(api, { view: "recordings", privacyMode: true });
    expect(container.querySelector('.official-browser-recordings')).toHaveTextContent("녹화 기록");
    expect(container).toHaveTextContent("마무리되지 않은 파일이 폴더에 보존");
    expect(container).toHaveTextContent("최신 1개 파일");
    expect(container).not.toHaveTextContent("공식 방송 테스트");
    expect(container).not.toHaveTextContent("segment-000000.webm");
    expect(container).not.toHaveTextContent("C:\\Recordings");
    expect(container.querySelector(".official-browser-originals")).not.toHaveAttribute("open");
    await act(async () => container.querySelector<HTMLElement>(".official-browser-originals summary")!.click());
    await act(async () => button("파일 열기").click());
    expect(api.openSegment).toHaveBeenCalledExactlyOnceWith("official-recording-1", 0);
    await act(async () => button("녹화 폴더 열기").click());
    expect(api.openFolder).toHaveBeenCalledExactlyOnceWith("official-recording-1");
    expect(api.start).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
  });
  it("opens a merged archive once by ID while preserving its interrupted recording and chat warning", async () => {
    const api = fakeApi();
    const opening = deferred<ApiResult<void>>();
    const token = "a".repeat(32);
    const recorded = recordingFixture({ status: "interrupted", captureChat: true, chatStatus: "partial", lastError: "마지막 조각은 미완성입니다.", merge: { status: "complete", segmentCount: 1, updatedAt: 10, file: `merged-${token}.webm`, timelineFile: `merged-${token}.timeline.jsonl`, bytes: 1024, durationSeconds: 30 } });
    api.snapshot.mockResolvedValue(success(ready({ recordings: [recorded] })));
    api.openMerged.mockReturnValueOnce(opening.promise);
    await render(api, { view: "recordings" });
    expect(button("외부 플레이어로 열기")).toBeEnabled();
    expect(api.openMerged).not.toHaveBeenCalled();
    expect(container.querySelector(".official-browser-files")).toHaveTextContent("마지막 조각은 미완성입니다.");
    expect(container.querySelector(".official-browser-saved-chat-warning")).not.toBeNull();
    expect(container.querySelector(".official-browser-originals")).not.toHaveAttribute("open");
    await act(async () => { const open = button("외부 플레이어로 열기"); open.click(); open.click(); });
    expect(api.openMerged).toHaveBeenCalledExactlyOnceWith(recorded.id);
    expect(button("여는 중…")).toBeDisabled();
    expect(button("파일 열기")).toBeDisabled();
    await act(async () => opening.resolve(success(undefined)));
    expect(button("외부 플레이어로 열기")).toBeEnabled();
    expect(api.start).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
  });

  it("makes in-app replay the primary completed-archive action without stopping another live recording", async () => {
    const api = fakeApi();
    const token = "a".repeat(32);
    const saved = recordingFixture({ id: "saved", title: "이전 저장본", status: "stopped", merge: { status: "complete", segmentCount: 1, updatedAt: 10, file: `merged-${token}.webm`, timelineFile: `merged-${token}.timeline.jsonl`, bytes: 1024, durationSeconds: 30 } });
    const live = recordingFixture({ id: "live", title: "진행 중인 방송" });
    api.snapshot.mockResolvedValue(success(ready({ status: "recording", recordingId: live.id, recordings: [saved, live] })));
    const replayApi: ReplayApi = {
      open: vi.fn().mockResolvedValue(success({ token, recordingId: saved.id, title: saved.title, durationSeconds: 30, mimeType: "video/webm", chatStatus: "disabled", indexState: "ready", syncQuality: "receive_time_approximate", manualOffsetSeconds: 0, warnings: [] })),
      chatAt: vi.fn(), chatPage: vi.fn(), setOffset: vi.fn(), close: vi.fn().mockResolvedValue(success(undefined)),
      timeline: vi.fn().mockResolvedValue(success({ bucketSeconds: 30, buckets: [], indexState: "ready", viewerMetricStatus: "not_recorded" })),
      mediaUrl: () => `http://atsumi-replay.localhost/${token}`,
    };
    await render(api, { view: "recordings", replayApi });
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>(".official-browser-recording-select")].find((entry) => entry.textContent?.includes(saved.title))!.click());
    expect(button("앱에서 다시보기")).toHaveClass("official-browser-primary");
    await act(async () => button("앱에서 다시보기").click());
    expect(replayApi.open).toHaveBeenCalledExactlyOnceWith(saved.id);
    expect(document.body).toHaveTextContent("다른 녹화가 진행 중입니다");
    expect(document.querySelector("video")).not.toBeNull();
    expect(api.stop).not.toHaveBeenCalled();
    expect(api.open).not.toHaveBeenCalled();
    expect(api.openMerged).not.toHaveBeenCalled();
    expect(api.setViewport.mock.calls.at(-1)?.[0].visible).toBe(false);
  });

  it("retries only the selected failed merge and accepts queued then completed native snapshots", async () => {
    const api = fakeApi();
    const retry = deferred<ApiResult<OfficialBrowserSnapshot>>();
    const recorded = recordingFixture({ status: "stopped", merge: { status: "blocked", segmentCount: 1, updatedAt: 1, lastError: "병합 도구가 없습니다." } });
    api.snapshot.mockResolvedValue(success(ready({ recordings: [recorded] })));
    api.retryMerge.mockReturnValueOnce(retry.promise);
    await render(api, { view: "recordings" });
    expect(container).toHaveTextContent("병합 도구가 없습니다.");
    await act(async () => { const retryButton = button("병합 다시 시도"); retryButton.click(); retryButton.click(); });
    expect(api.retryMerge).toHaveBeenCalledExactlyOnceWith(recorded.id);
    expect(button("요청 중…")).toBeDisabled();
    const queued = { ...recorded, merge: { status: "queued" as const, segmentCount: 1, updatedAt: 2 } };
    await act(async () => retry.resolve(success(ready({ recordings: [queued] }))));
    expect(container.querySelector(".official-browser-playback")).toHaveTextContent("병합 대기");
    expect(container).not.toHaveTextContent("병합 도구가 없습니다.");
    expect(button("외부 플레이어로 열기")).toBeUndefined();
    const token = "b".repeat(32);
    api.snapshot.mockResolvedValue(success(ready({ recordings: [{ ...recorded, merge: { status: "complete", segmentCount: 1, updatedAt: 3, file: `merged-${token}.webm`, timelineFile: `merged-${token}.timeline.jsonl`, bytes: 1024, durationSeconds: 30 } }] })));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(button("외부 플레이어로 열기")).toBeEnabled();
    expect(api.openMerged).not.toHaveBeenCalled();
  });

  it("does not replace a failed merge with success after a failed retry or late inactive response", async () => {
    const api = fakeApi();
    const recorded = recordingFixture({ status: "stopped", merge: { status: "failed", segmentCount: 1, updatedAt: 1, lastError: "조각 형식이 다릅니다." } });
    api.snapshot.mockResolvedValue(success(ready({ recordings: [recorded] })));
    api.retryMerge.mockResolvedValueOnce({ ok: false, error: { code: "MERGE", message: "재시도 요청을 처리하지 못했습니다.", retryable: true } });
    await render(api, { view: "recordings" });
    await act(async () => button("병합 다시 시도").click());
    expect(container).toHaveTextContent("재시도 요청을 처리하지 못했습니다.");
    expect(container).toHaveTextContent("조각 형식이 다릅니다.");
    expect(button("외부 플레이어로 열기")).toBeUndefined();
    expect(button("병합 다시 시도")).toBeEnabled();
    const late = deferred<ApiResult<OfficialBrowserSnapshot>>();
    api.retryMerge.mockReturnValueOnce(late.promise);
    await act(async () => button("병합 다시 시도").click());
    await render(api, { active: false, view: "recordings" });
    await act(async () => late.resolve(success(ready({ recordings: [] }))));
    expect(container).toBeEmptyDOMElement();
    await render(api, { view: "recordings" });
    expect(container).toHaveTextContent("조각 형식이 다릅니다.");
    expect(api.stop).not.toHaveBeenCalled();
  });

  it("preserves stored chat warnings in library labels without assuming legacy chat was saved", async () => {
    const api = fakeApi();
    api.snapshot.mockResolvedValue(success(ready({ recordings: [
      recordingFixture({ id: "chat-failed", title: "채팅 실패 기록", status: "stopped", captureChat: true, chatStatus: "storage_failed", chatCount: 42, startedAt: 3 }),
      recordingFixture({ id: "chat-partial", title: "채팅 누락 기록", status: "stopped", captureChat: true, chatStatus: "partial", chatCount: 12, startedAt: 2 }),
      recordingFixture({ id: "legacy", title: "이전 기록", status: "stopped", startedAt: 1 }),
    ] })));
    await render(api, { view: "recordings" });
    const records = [...container.querySelectorAll<HTMLButtonElement>(".official-browser-recording-select")];
    expect(records[0]!.title).toContain("채팅 저장 실패"); expect(records[0]!.title).toContain("42개 기록");
    expect(records[1]!.querySelector('[role="img"]')).toHaveAccessibleName(expect.stringContaining("채팅 일부 누락 가능"));
    expect(records[2]!.title).toBe("이 기록의 채팅 저장 상태는 확인되지 않았습니다.");
    expect(records[2]!.querySelector(".official-browser-saved-chat-warning")).toBeNull();
    await act(async () => records[2]!.click());
    expect(container.querySelector(".official-browser-files .official-browser-muted")).toHaveAttribute("title", "이 기록의 채팅 저장 상태는 확인되지 않았습니다.");
    expect(container.querySelector(".official-browser-files .official-browser-saved-chat-warning")).toBeNull();
    expect(container).not.toHaveTextContent("채팅 저장 완료");
  });

  it("keeps the browser preview empty and all native controls disabled without invoking Tauri", async () => {
    const api = createOfficialBrowserApi("browser-mock");
    await render(api);
    expect(container).toHaveTextContent("데스크톱 앱 필요");
    await act(async () => help("녹화 상태").focus());
    expect(container).toHaveTextContent("녹화 보관함 · 0개");
    expect(container.querySelectorAll("button:enabled:not(.official-browser-help-trigger)")).toHaveLength(0);
    await act(async () => vi.advanceTimersByTimeAsync(10_000));
    expect(await api.snapshot()).toEqual(success(emptyOfficialBrowserSnapshot("browser-mock")));
    await api.open("a".repeat(32));
    await api.start({ rightsAcknowledged: true, captureChat: true });
    await api.stop();
    await api.connectExtension();
    await api.login();
    await api.logout();
    await api.openInstaller();
    await api.setViewport(hiddenOfficialBrowserViewport);
    await api.confirmControl({ requestId: "unused", approve: true, rightsAcknowledged: true, captureChat: true });
    await api.openFolder("unused");
    await api.openSegment("unused", 0);
    expect((await api.openMerged("unused")).ok).toBe(false);
    expect((await api.retryMerge("unused")).ok).toBe(false);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("uses explicit login and confirms logout without inspecting account data", async () => {
    const api = fakeApi();
    await render(api);
    await openSettings(api);
    expect(api.login).not.toHaveBeenCalled();
    expect(api.logout).not.toHaveBeenCalled();
    await act(async () => button("로그인").click());
    expect(api.login).toHaveBeenCalledTimes(1);
    expect(container).toHaveTextContent("로그인·인증 창 열림");
    await act(async () => button("로그아웃").click());
    expect(container.querySelector('[role="alertdialog"]')).not.toBeNull();
    expect(document.activeElement).toBe(button("취소"));
    await act(async () => button("취소").dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true })));
    expect(document.activeElement).toBe(button("로그아웃 확인"));
    await act(async () => button("로그아웃 확인").dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true })));
    expect(document.activeElement).toBe(button("취소"));
    expect(api.logout).not.toHaveBeenCalled();
    await act(async () => button("취소").click());
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expect(document.activeElement).toBe(button("로그아웃"));
    await consent();
    await act(async () => button("로그아웃").click());
    await act(async () => button("로그아웃 확인").click());
    expect(api.logout).toHaveBeenCalledTimes(1);
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expect(container).toHaveTextContent("로그아웃됨");
    expect(container.querySelector('.official-browser-consent input')).not.toBeChecked();
    expect(api.stop).not.toHaveBeenCalled();
  });

  it("blocks account changes during recording and opens only an explicit installer guide", async () => {
    const api = fakeApi();
    await render(api);
    await openSettings(api);
    expect(button("전체 보기")).toBeUndefined();
    expect(button("간단히 보기")).toBeUndefined();
    await act(async () => button("Chrome").click());
    expect(api.openInstaller).toHaveBeenCalledWith("chrome");
    await act(async () => button("Edge").click());
    expect(api.openInstaller).toHaveBeenCalledWith("edge");
    await consent();
    await act(async () => button("녹화 시작").click());
    expect(button("로그인")).toBeDisabled();
    expect(button("로그아웃")).toBeDisabled();
    expect(button("Chrome")).toBeDisabled();
    expect(button("Edge")).toBeDisabled();
    expect(api.login).not.toHaveBeenCalled();
    expect(api.logout).not.toHaveBeenCalled();
  });

  it("keeps an existing official view free of app toolbars and opens settings only on a native intent", async () => {
    const api = fakeApi();
    await render(api);
    expectNoLiveToolbar();
    expect(container.querySelector('[role="dialog"],[role="alertdialog"]')).toBeNull();
    await openSettings(api);
    expect(button("고화질 연결").closest('[aria-modal="true"]')).not.toBeNull();
    expect(container.querySelector('.official-browser-extension-status')?.closest('[aria-modal="true"]')).not.toBeNull();
    expect(container).not.toHaveTextContent("독립적인 보안 검증을 마친 브라우저와 같다고 보장하지 않습니다.");
    await act(async () => help("로그인").focus());
    expect(container.querySelector('[role="tooltip"]')).toHaveTextContent("독립적인 보안 검증을 마친 브라우저와 같다고 보장하지 않습니다.");
    await act(async () => help("로그인").blur());
    await closeSettings();
    expectNoLiveToolbar();
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(button("설정 닫기")).toBeUndefined();
    expect(api.open).not.toHaveBeenCalled();
    expect(api.start).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
  });

  it("keeps all parent controls out of the live stage across ready, recording and stopping states", async () => {
    const api = fakeApi();
    await render(api);
    const stage = container.querySelector(".official-browser-stage");
    for (const status of ["ready", "recording", "stopping"] as const) {
      api.snapshot.mockResolvedValue(success(ready({ status, videoWidth: 1920, videoHeight: 1080,
        extensionStatus: "연결됨", ...(status === "ready" ? {} : { recordingId: "official-recording-1", recordings: [recordingFixture()] }) })));
      await act(async () => vi.advanceTimersByTimeAsync(2000));
      expectNoLiveToolbar();
      expect(container.querySelector(".official-browser-resolution,.official-browser-extension-status")).toBeNull();
      expect(container.querySelector(".official-browser-stage")).toBe(stage);
      expect(container.querySelector('[aria-modal="true"]')).toBeNull();
    }
    expect(api.start).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
    expect(api.connectExtension).not.toHaveBeenCalled();
  });

  it("masks native paint for settings without hiding media, traps focus and restores the unchanged stage", async () => {
    const api = fakeApi();
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      return this.classList.contains("official-browser-stage") ? new DOMRect(100, 180, 800, 500) : new DOMRect(0, 0, 1200, 1000);
    });
    await render(api);
    const stage = container.querySelector(".official-browser-stage");
    const initial = api.setViewport.mock.calls.at(-1)?.[0];
    expect(initial?.visible).toBe(true);
    api.setViewport.mockClear();
    await openSettings(api);
    expect(container.querySelector(".official-browser-stage")).toBe(stage);
    expect(api.setViewport.mock.calls.at(-1)?.[0]).toMatchObject({ ...initial, visible: true, occluded: true });
    expect(document.activeElement).toBe(button("설정 닫기"));
    const dialog = button("설정 닫기").closest<HTMLElement>('[aria-modal="true"]')!;
    const inputs = [...dialog.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), summary, select:not(:disabled), textarea:not(:disabled), a[href]')];
    const first = inputs[0]!, last = inputs.at(-1)!;
    await act(async () => { first.focus(); first.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true })); });
    expect(document.activeElement).toBe(last);
    await act(async () => last.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true })));
    expect(document.activeElement).toBe(first);
    await act(async () => dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    await act(async () => vi.advanceTimersByTimeAsync(20));
    expect(button("설정 닫기")).toBeUndefined();
    expectNoLiveToolbar();
    expect(container.querySelector(".official-browser-stage")).toBe(stage);
    expect(api.setViewport.mock.calls.at(-1)?.[0]).toEqual(initial);
    expect(api.setViewport.mock.calls.every(([viewport]) => viewport.visible)).toBe(true);
    expect(api.start).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
    expect(api.open).not.toHaveBeenCalled();
  });

  it("keeps one viewport lease through settings, nested logout and recording confirmation", async () => {
    const api = fakeApi();
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      return this.classList.contains("official-browser-stage") ? new DOMRect(100, 180, 800, 500) : new DOMRect(0, 0, 1200, 1000);
    });
    await render(api);
    api.setViewport.mockClear();
    await openSettings(api);
    await act(async () => button("로그아웃").click());
    expect(api.setViewport.mock.lastCall?.[0]).toMatchObject({ visible: true, occluded: true, width: 800, height: 500 });
    await act(async () => button("취소").click());
    await closeSettings();
    api.snapshot.mockResolvedValueOnce(success(ready({ pendingControl: { id: "masked-confirm", action: "record_start", channelId: "a".repeat(32), expiresAt: Date.now() + 20000 } })));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(button("시작 확인")).toBeDisabled();
    expect(api.setViewport.mock.lastCall?.[0]).toMatchObject({ visible: true, occluded: true, width: 800, height: 500 });
    await act(async () => button("취소").click());
    await act(async () => vi.advanceTimersByTimeAsync(20));
    expect(api.setViewport.mock.lastCall?.[0]).toMatchObject({ visible: true, occluded: false });
    expect(api.setViewport.mock.calls.every(([viewport]) => viewport.visible)).toBe(true);
    expect(api.start).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled(); expect(api.logout).not.toHaveBeenCalled();
  });

  it.each(["toggle_focus", "exit_focus"] as const)("ignores removed single-player %s actions", async (action) => {
    const api = fakeApi();
    api.snapshot.mockResolvedValue(success(ready({ pendingUiAction: { id: "removed-focus", action, expiresAt: Date.now() + 8000 } })));
    await render(api);
    const stage = container.querySelector(".official-browser-stage");
    await act(async () => vi.advanceTimersByTimeAsync(6000));
    expect(container.querySelector(".official-browser-stage")).toBe(stage);
    expect(container.querySelector(".official-browser-panel")).not.toHaveClass("is-focused");
    expect(api.ackUiAction).not.toHaveBeenCalled();
    expectNoLiveToolbar();
  });

  it("defers settings intents while a trusted recording confirmation is open", async () => {
    const api = fakeApi();
    const pendingUiAction = { id: "settings-after-confirm", action: "open_settings" as const, expiresAt: Date.now() + 8000 };
    const pendingControl = { id: "record-before-settings", action: "record_start" as const, channelId: "a".repeat(32), expiresAt: Date.now() + 20000 };
    api.snapshot.mockResolvedValue(success(ready({ pendingUiAction, pendingControl })));
    api.confirmControl.mockResolvedValue(success(ready({ pendingUiAction })));
    await render(api);
    expect(container.querySelector('[role="alertdialog"]')).toHaveTextContent("녹화를 시작할까요?");
    expect(button("설정 닫기")).toBeUndefined();
    expect(document.activeElement).toBe(button("취소"));
    expect(api.ackUiAction).not.toHaveBeenCalled();
    expect(api.start).not.toHaveBeenCalled();
    await act(async () => button("취소").click());
    expect(api.confirmControl).toHaveBeenCalledExactlyOnceWith({ requestId: pendingControl.id, approve: false, rightsAcknowledged: false, captureChat: true });
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expect(button("설정 닫기")).toBeEnabled();
    expect(document.activeElement).toBe(button("설정 닫기"));
    expect(api.ackUiAction).toHaveBeenCalledExactlyOnceWith(pendingUiAction.id);
    expect(api.start).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
  });

  it("only acknowledges expired settings intent without changing the UI", async () => {
    const api = fakeApi();
    await render(api);
    api.ackUiAction.mockClear();
    const pendingUiAction = { id: "expired-settings", action: "open_settings" as const, expiresAt: Date.now() - 1 };
    api.snapshot.mockResolvedValue(success(ready({ pendingUiAction })));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(api.ackUiAction).toHaveBeenCalledExactlyOnceWith(pendingUiAction.id);
    expect(container.querySelector(".official-browser-panel")).not.toHaveClass("is-focused");
    expect(button("설정 닫기")).toBeUndefined();
    expectNoLiveToolbar();
    await act(async () => vi.advanceTimersByTimeAsync(6000));
    expect(api.ackUiAction).toHaveBeenCalledTimes(1);
    expect(container.querySelector(".official-browser-panel")).not.toHaveClass("is-focused");
    expect(api.open).not.toHaveBeenCalled();
    expect(api.start).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
    expect(api.connectExtension).not.toHaveBeenCalled();
  });

  it("shows bounded native account text and actual video size without inferring grid quality", async () => {
    const api = fakeApi();
    api.snapshot.mockResolvedValue(success(ready({ loginStatus: "브라우저 세션 유지 · 로그인 여부는 공식 화면에서 확인", videoWidth: 1920, videoHeight: 1080, videoPaused: false, extensionStatus: "미연결" })));
    await render(api);
    expectNoLiveToolbar();
    await openSettings(api, ready({ loginStatus: "브라우저 세션 유지 · 로그인 여부는 공식 화면에서 확인", videoWidth: 1920, videoHeight: 1080, videoPaused: false, extensionStatus: "미연결" }));
    expect(container).toHaveTextContent("브라우저 세션 유지 · 로그인 여부는 공식 화면에서 확인");
    expect(container).toHaveTextContent("1920×1080");
    expect(container).toHaveTextContent("확장 · 미연결");
    expect(container).not.toHaveTextContent("그리드 연결됨");
    api.snapshot.mockResolvedValue(success(ready({ loginStatus: "가".repeat(201), videoWidth: -1, videoHeight: 1080 })));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(container).toHaveTextContent("계정 상태는 공식 페이지에서 확인");
    expect(container).toHaveTextContent("영상 확인 전");
    expect(container).not.toHaveTextContent("가".repeat(201));
  });

  it("keeps visual readiness equal to real start permission through extension connection and recovery", async () => {
    const api = fakeApi();
    await render(api);
    await openSettings(api);
    const start = button("녹화 시작");
    expect(start).toBeDisabled();
    expect(start).toHaveAttribute("data-ready", "false");
    expect(start).toHaveAccessibleDescription("아래 저장 권한을 확인해 주세요.");
    expect(container.querySelector('.official-browser-consent')?.closest('details')).toBeNull();
    await consent();
    expect(start).toBeEnabled();
    expect(start).toHaveAttribute("data-ready", "true");
    const extension = deferred<ApiResult<OfficialBrowserSnapshot>>();
    api.connectExtension.mockReturnValue(extension.promise);
    await act(async () => button("고화질 연결").click());
    expect(start).toBeDisabled();
    expect(start).toHaveAttribute("data-ready", "false");
    await act(async () => extension.resolve(success(ready({ extensionStatus: "연결 중" }))));
    expect(start).toBeDisabled();
    await act(async () => start.click());
    expect(api.start).not.toHaveBeenCalled();
    api.snapshot.mockResolvedValue(success(ready({ extensionStatus: "확장 로드 완료 · 공식 페이지 감지 확인 중", error: "고화질은 실제 수신 해상도를 확인해 주세요." })));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(start).toBeEnabled();
    expect(start).toHaveAttribute("data-ready", "true");
    await act(async () => start.click());
    expect(api.start).toHaveBeenCalledExactlyOnceWith({ rightsAcknowledged: true, captureChat: true });
    expect(button("녹화 중지")).toBeEnabled();
    expect(container).toHaveTextContent("탐색·배속·채널 변경 시 녹화가 중단됩니다.");
  });

  it("keeps explanations out of the default UI and exposes help on hover, focus and Escape", async () => {
    const api = fakeApi();
    await render(api);
    expect(container.querySelector('[role="tooltip"]')).toBeNull();
    expect(container).not.toHaveTextContent(/WEBVIEW2|HLS|재인코딩|원본 무손실/i);
    await openSettings(api);
    await act(async () => help("녹화").dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(container.querySelector('[role="tooltip"]')).toHaveTextContent("버튼을 눌러야 녹화가 시작됩니다.");
    await act(async () => help("녹화").dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body })));
    expect(container.querySelector('[role="tooltip"]')).toBeNull();
    await act(async () => help("녹화").focus());
    expect(help("녹화")).toHaveAccessibleDescription(/다른 메뉴로 이동하거나 창을 최소화해도 녹화는 계속됩니다/);
    await act(async () => help("녹화").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(container.querySelector('[role="tooltip"]')).toBeNull();
    expect(document.activeElement).toBe(help("녹화"));
    expect(api.start).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
  });

  it("keeps native paint hidden while help is used inside settings and restores it only after closing settings", async () => {
    const api = fakeApi();
    let tooltipTop = 100;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.classList.contains("official-browser-stage")) return new DOMRect(0, 300, 800, 500);
      if (this.classList.contains("official-browser-tooltip")) return new DOMRect(16, tooltipTop, 360, 120);
      return new DOMRect(16, 240, 22, 22);
    });
    await render(api);
    expect(api.setViewport.mock.calls.at(-1)?.[0].visible).toBe(true);
    await openSettings(api);
    expect(api.setViewport.mock.calls.at(-1)?.[0]).toMatchObject({ visible: true, occluded: true });
    const baseline = api.setViewport.mock.calls.length;
    await act(async () => help("녹화").focus());
    await act(async () => vi.advanceTimersByTimeAsync(20));
    expect(container.querySelector('[role="tooltip"]')).toHaveAttribute("data-native-overlay", "false");
    expect(api.setViewport).toHaveBeenCalledTimes(baseline);
    await act(async () => help("녹화").blur());
    tooltipTop = 250;
    for (const dismiss of ["blur", "Escape", "scroll"] as const) {
      await act(async () => { help("녹화").blur(); help("녹화").focus(); });
      expect(container.querySelector('[role="tooltip"]')).toHaveAttribute("data-native-overlay", "true");
      expect(api.setViewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: true, occluded: true }));
      await act(async () => {
        if (dismiss === "blur") help("녹화").blur();
        else if (dismiss === "Escape") help("녹화").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
        else document.dispatchEvent(new Event("scroll"));
      });
      await act(async () => vi.advanceTimersByTimeAsync(20));
      expect(container.querySelector('[role="tooltip"]')).toBeNull();
      expect(button("설정 닫기")).toBeEnabled();
      expect(api.setViewport.mock.calls.at(-1)?.[0]).toMatchObject({ visible: true, occluded: true });
      const settled = api.setViewport.mock.calls.length;
      await act(async () => vi.advanceTimersByTimeAsync(100));
      expect(api.setViewport).toHaveBeenCalledTimes(settled);
    }
    await closeSettings();
    expect(api.setViewport.mock.calls.at(-1)?.[0].visible).toBe(true);
    expect(api.stop).not.toHaveBeenCalled();
  });

  it.each(["page_connected", "waiting_socket", "receiving", "observer_unavailable", "queue_overflow", "connection_gap", "partial"])("labels actual official page chat state %s separately from recording", async (chatStatus) => {
    const api = fakeApi();
    api.snapshot.mockResolvedValue(success(ready({ recordingId: "official-recording-1", status: "recording", recordings: [recordingFixture()], chatStatus, chatCount: 123 })));
    await render(api);
    expect(container.querySelector('.official-browser-live-stats')).toBeNull();
    await openSettings(api, ready({ recordingId: "official-recording-1", status: "recording", recordings: [recordingFixture()], chatStatus, chatCount: 123 }));
    await act(async () => help("녹화 상태").focus());
    expect(container.querySelector('[role="tooltip"]')).toHaveTextContent("채팅");
    expect(container.querySelector('[role="tooltip"]')).toHaveTextContent("123개");
    expect(container.querySelector('[role="tooltip"]')).not.toHaveTextContent(chatStatus);
    expect(container).toHaveTextContent("녹화 중");
    expect(api.stop).not.toHaveBeenCalled();
  });

  it("routes S through a trusted permission dialog without directly starting or saving", async () => {
    const api = fakeApi();
    api.requestControl.mockResolvedValue(success(ready({ pendingControl: { id: "shortcut-shot", action: "screenshot", channelId: "a".repeat(32), expiresAt: Date.now() + 20000 } })));
    await render(api);
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true, cancelable: true })));
    expect(api.requestControl).toHaveBeenCalledExactlyOnceWith("screenshot");
    expect(button("저장 확인")).toBeDisabled();
    expect(document.activeElement).toBe(button("취소"));
    expect(api.confirmControl).not.toHaveBeenCalled(); expect(api.start).not.toHaveBeenCalled();
    await act(async () => container.querySelector('[role="alertdialog"]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(api.confirmControl).toHaveBeenCalledExactlyOnceWith({ requestId: "shortcut-shot", approve: false, rightsAcknowledged: false, captureChat: true });
  });
  it("ignores screenshot shortcuts in inputs, editable content, IME and modified or repeated keys", async () => {
    const api = fakeApi(); await render(api);
    const input = document.createElement("input"); container.append(input);
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true })));
    const editable = document.createElement("div"); editable.setAttribute("contenteditable", ""); container.append(editable);
    await act(async () => editable.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true })));
    for (const patch of [{ repeat: true }, { isComposing: true }, { ctrlKey: true }, { altKey: true }, { shiftKey: true }, { metaKey: true }])
      await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "s", bubbles: true, ...patch })));
    expect(api.requestControl).not.toHaveBeenCalled();
  });
  it("has no upper controls or replacement focus mode before a native player is ready", async () => {
    const api = fakeApi(); api.snapshot.mockResolvedValue(success(ready({ ready: false, status: "loading" })));
    await render(api);
    expect(container.querySelector(".official-browser-heading,.official-browser-controls")).toBeNull();
    expect(container.querySelector(".official-browser-stage")).not.toBeNull();
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(container.querySelector(".official-browser-panel")).not.toHaveClass("is-focused");
    expectNoLiveToolbar();
    expect(api.stop).not.toHaveBeenCalled();
  });
  it("consumes native settings intent once and never applies a stale ACK snapshot", async () => {
    const api = fakeApi(), ack = deferred<ApiResult<OfficialBrowserSnapshot>>();
    await render(api);
    api.ackUiAction.mockClear().mockReturnValue(ack.promise);
    api.snapshot.mockResolvedValue(success(ready({ pendingUiAction: { id: "settings-once", action: "open_settings", expiresAt: Date.now() + 8000 }, status: "recording", recordingId: "official-recording-1", recordings: [recordingFixture()] })));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(api.ackUiAction).toHaveBeenCalledExactlyOnceWith("settings-once");
    expect(container.querySelector(".official-browser-panel")).not.toHaveClass("is-focused");
    await act(async () => ack.resolve(success(emptyOfficialBrowserSnapshot("tauri"))));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(container.querySelector(".official-browser-panel")).not.toHaveClass("is-focused");
    expect(button("설정 닫기")).toBeEnabled();
    expect(container).toHaveTextContent("녹화 중"); expect(api.ackUiAction).toHaveBeenCalledTimes(1);
  });
  it("keeps the original live stage structure unchanged as recording details grow", async () => {
    const api = fakeApi();
    await render(api);
    const stage = container.querySelector('.official-browser-stage');
    expect(container.querySelector('.official-browser-heading')).toBeNull();
    expect(container.querySelector('.official-browser-controls')).toBeNull();
    api.snapshot.mockResolvedValue(success(ready({ recordingId: "official-recording-1", status: "recording", recordings: [recordingFixture()] })));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(container.querySelector('.official-browser-stage')).toBe(stage);
    expect(container.querySelector('.official-browser-live-stats')).toBeNull();
    expect(container.querySelector('.official-browser-library-hint')).toBeNull();
    api.snapshot.mockResolvedValue(success(ready({ recordingId: "official-recording-1", status: "recording", recordings: [recordingFixture({ title: "긴 채널 제목 ".repeat(100), segmentCount: 10000, bytesWritten: 1024 ** 4, durationSeconds: 360000 })], chatStatus: "queue_overflow", chatCount: 99999999 })));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(container.querySelector('.official-browser-stage')).toBe(stage);
    expect(container).not.toHaveTextContent("긴 채널 제목");
    expect(container.querySelector('.official-browser-focus-alert')).toHaveTextContent('채팅 기록에 누락');
    expect(api.stop).not.toHaveBeenCalled();
  });

  it("treats remote record start as an expiring request, focuses Cancel and requires explicit permission", async () => {
    const api = fakeApi();
    const pendingControl = { id: "intent-start", action: "record_start" as const, channelId: "a".repeat(32), expiresAt: Date.now() + 20000 };
    api.snapshot.mockResolvedValue(success(ready({ pendingControl })));
    await render(api);
    const dialog = container.querySelector('[role="alertdialog"]')!;
    expect(dialog).toHaveTextContent("녹화를 시작할까요?");
    expect(document.activeElement).toBe(button("취소"));
    expect(button("시작 확인")).toBeDisabled();
    await act(async () => dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(api.start).not.toHaveBeenCalled();
    expect(api.confirmControl).not.toHaveBeenCalled();
    await act(async () => dialog.querySelector<HTMLInputElement>('input')!.click());
    expect(button("시작 확인")).toBeEnabled();
    const response = deferred<ApiResult<OfficialBrowserSnapshot>>();
    api.confirmControl.mockReturnValue(response.promise);
    await act(async () => { button("시작 확인").click(); button("처리 중…")?.click(); });
    expect(api.confirmControl).toHaveBeenCalledExactlyOnceWith({ requestId: "intent-start", approve: true, rightsAcknowledged: true, captureChat: true });
    expect(api.start).not.toHaveBeenCalled();
    await act(async () => response.resolve(success(ready({ pendingControl: null, recordingId: "official-recording-1", status: "recording", recordings: [recordingFixture()] }))));
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expectNoLiveToolbar();
    expect(container).toHaveTextContent("녹화 중");
  });

  it.each(["screenshot", "record_stop"] as const)("confirms or cancels %s without authorizing another action", async (action) => {
    const api = fakeApi();
    const pendingControl = { id: `intent-${action}`, action, channelId: "a".repeat(32), expiresAt: Date.now() + 20000 };
    api.snapshot.mockResolvedValue(success(ready({ pendingControl, ...(action === "record_stop" ? { recordingId: "official-recording-1", status: "recording", recordings: [recordingFixture()] } : {}) })));
    await render(api);
    expect(document.activeElement).toBe(button("취소"));
    if (action === "screenshot") {
      expect(button("저장 확인")).toBeDisabled();
      await act(async () => container.querySelector<HTMLInputElement>('.official-browser-control-rights input')!.click());
      await act(async () => button("저장 확인").click());
      expect(api.confirmControl).toHaveBeenCalledExactlyOnceWith({ requestId: "intent-screenshot", approve: true, rightsAcknowledged: true, captureChat: true });
    } else {
      expect(button("중지 확인")).toBeEnabled();
      await act(async () => button("취소").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
      expect(api.confirmControl).toHaveBeenCalledExactlyOnceWith({ requestId: "intent-record_stop", approve: false, rightsAcknowledged: false, captureChat: true });
    }
    expect(api.start).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
  });

  it("dismisses expired or channel-mismatched control requests without file or recording actions", async () => {
    const api = fakeApi();
    const pendingControl = { id: "intent-expire", action: "screenshot" as const, channelId: "a".repeat(32), expiresAt: Date.now() + 3000 };
    api.snapshot.mockResolvedValue(success(ready({ pendingControl })));
    await render(api);
    expect(container.querySelector('[role="alertdialog"]')).not.toBeNull();
    await act(async () => vi.advanceTimersByTimeAsync(3001));
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    api.snapshot.mockResolvedValue(success(ready({ pendingControl: { ...pendingControl, id: "other-channel", channelId: "b".repeat(32), expiresAt: Date.now() + 20000 } })));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expect(api.confirmControl).not.toHaveBeenCalled();
    expect(api.start).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
  });

  it("masks an existing native child for a control confirmation and restores it after cancellation", async () => {
    const api = fakeApi();
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      return this.classList.contains("official-browser-stage") ? new DOMRect(100, 180, 800, 500) : new DOMRect(0, 0, 1200, 1000);
    });
    await render(api);
    expect(api.setViewport.mock.calls.at(-1)?.[0].visible).toBe(true);
    api.snapshot.mockResolvedValue(success(ready({ pendingControl: { id: "intent-cancel", action: "screenshot", channelId: "a".repeat(32), expiresAt: Date.now() + 20000 } })));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(api.setViewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: true, occluded: true }));
    expect(document.activeElement).toBe(button("취소"));
    await act(async () => button("취소").click());
    await act(async () => vi.advanceTimersByTimeAsync(20));
    expect(api.confirmControl).toHaveBeenCalledWith({ requestId: "intent-cancel", approve: false, rightsAcknowledged: false, captureChat: true });
    expect(api.setViewport.mock.calls.at(-1)?.[0].visible).toBe(true);
    expect(api.start).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
  });

  it("docks and clips the whole official page; hides for overlays, privacy, routes and unmount without stopping", async () => {
    const api = fakeApi();
    let rect = new DOMRect(100, 180, 800, 500);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      return this.classList.contains("official-browser-stage") ? rect : new DOMRect(0, 0, 1200, 1000);
    });
    await render(api);
    expect(api.setViewport).toHaveBeenLastCalledWith({ x: 100, y: 180, width: 800, height: 500, visible: true, occluded: false, epoch: 0, clip: { x: 0, y: 0, width: 800, height: 500 } });
    rect = new DOMRect(100, -20, 800, 500);
    const beforeScroll = api.setViewport.mock.calls.length;
    await act(async () => {
      document.dispatchEvent(new Event("scroll"));
      document.dispatchEvent(new Event("scroll"));
      await vi.advanceTimersByTimeAsync(20);
    });
    expect(api.setViewport).toHaveBeenCalledTimes(beforeScroll + 1);
    expect(api.setViewport).toHaveBeenLastCalledWith({ x: 100, y: -20, width: 800, height: 500, visible: true, occluded: false, epoch: 0, clip: { x: 0, y: 20, width: 800, height: 480 } });
    const menu = document.createElement("div");
    menu.setAttribute("role", "menu");
    await act(async () => { document.body.append(menu); });
    expect(api.setViewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: true, occluded: true }));
    await act(async () => { menu.remove(); await vi.advanceTimersByTimeAsync(20); });
    expect(api.setViewport.mock.lastCall?.[0].visible).toBe(true);
    await render(api, { privacyMode: true });
    expect(api.setViewport).toHaveBeenLastCalledWith(expect.objectContaining(hiddenOfficialBrowserViewport));
    await render(api);
    expect(api.setViewport.mock.lastCall?.[0].visible).toBe(true);
    await openSettings(api);
    await act(async () => button("로그아웃").click());
    expect(api.setViewport).toHaveBeenLastCalledWith(expect.objectContaining({ visible: true, occluded: true }));
    await act(async () => button("취소").click());
    expect(api.setViewport.mock.lastCall?.[0]).toMatchObject({ visible: true, occluded: true });
    await closeSettings();
    expect(api.setViewport.mock.lastCall?.[0].visible).toBe(true);
    await render(api, { view: "recordings" });
    expect(api.setViewport).toHaveBeenLastCalledWith(expect.objectContaining(hiddenOfficialBrowserViewport));
    expect(container.querySelector('.official-browser-stage')).toBeNull();
    await render(api);
    expect(api.setViewport.mock.lastCall?.[0].visible).toBe(true);
    await act(async () => root.render(null));
    expect(api.setViewport).toHaveBeenLastCalledWith(expect.objectContaining(hiddenOfficialBrowserViewport));
    expect(api.stop).not.toHaveBeenCalled();
    expect(api.open).not.toHaveBeenCalled();
  });

  it("surfaces viewport failure and retries a changed layout without exposing thrown details", async () => {
    const api = fakeApi();
    api.setViewport.mockResolvedValue({ ok: false, error: { code: "LAYOUT", message: "시청 영역 연결 실패", retryable: true } });
    await render(api);
    expect(container).toHaveTextContent("시청 영역 연결 실패");
    api.setViewport.mockResolvedValue(success(undefined));
    await act(async () => { window.dispatchEvent(new Event("resize")); await vi.advanceTimersByTimeAsync(20); });
    expect(container).not.toHaveTextContent("시청 영역 연결 실패");
  });

  it("echoes native document epochs and reattaches unchanged bounds after a reload", async () => {
    const api = fakeApi();
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      return this.classList.contains("official-browser-stage") ? new DOMRect(100, 180, 800, 500) : new DOMRect(0, 0, 1200, 1000);
    });
    api.snapshot.mockResolvedValue(success(ready({ viewportEpoch: 7 })));
    await render(api);
    expect(api.setViewport.mock.lastCall?.[0]).toMatchObject({ visible: true, epoch: 7, width: 800 });
    const layoutWrites = api.setViewport.mock.calls.length;
    api.snapshot.mockImplementation(async () => success(ready({ viewportEpoch: 7 })));
    await act(async () => { window.dispatchEvent(new Event("resize")); document.dispatchEvent(new Event("scroll")); await vi.advanceTimersByTimeAsync(6000); });
    expect(api.setViewport).toHaveBeenCalledTimes(layoutWrites);
    api.snapshot.mockResolvedValue(success(ready({ viewportEpoch: 8 })));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(api.setViewport.mock.lastCall?.[0]).toMatchObject({ visible: true, epoch: 8, width: 800 });
    await render(api, { privacyMode: true });
    expect(api.setViewport).toHaveBeenLastCalledWith(expect.objectContaining(hiddenOfficialBrowserViewport));
    expect(api.open).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
  });
});

describe("official browser viewport coordinator", () => {
  const viewport: OfficialBrowserViewport = { x: 10, y: 20, width: 800, height: 450, visible: true };
  it.each(["VIEWPORT_BUSY", "VIEWPORT_CLIP_FAILED"])("retries %s at most twice, without an unbounded failure loop", async (code) => {
    const api = fakeApi();
    api.setViewport.mockResolvedValue({ ok: false, error: { code, message: "잠시 후 다시 배치", retryable: true } });
    const owner = claimOfficialBrowserViewport(api);
    owner.update(viewport);
    await act(async () => {});
    owner.update(viewport); // Identical observer rerenders must not reset budget.
    await act(async () => vi.advanceTimersByTimeAsync(5000));
    expect(api.setViewport).toHaveBeenCalledTimes(3);
    await act(async () => vi.advanceTimersByTimeAsync(5000));
    expect(api.setViewport).toHaveBeenCalledTimes(3);
    owner.release();
  });

  it("cancels retry on hide/new ownership and ignores a stale ACK after immediate privacy hide", async () => {
    const api = fakeApi();
    const stale = deferred<ApiResult<void>>();
    api.setViewport.mockReturnValueOnce(stale.promise);
    const report = vi.fn();
    const owner = claimOfficialBrowserViewport(api, report);
    owner.update({ ...viewport, epoch: 4 });
    owner.update({ ...viewport, width: 900, epoch: 4 });
    owner.update({ ...hiddenOfficialBrowserViewport, epoch: 4 });
    expect(api.setViewport).toHaveBeenCalledTimes(2); // Hide did not wait for ACK.
    expect(api.setViewport).toHaveBeenLastCalledWith({ ...hiddenOfficialBrowserViewport, epoch: 4 });
    await act(async () => {});
    report.mockClear();
    await act(async () => stale.resolve({ ok: false, error: { code: "VIEWPORT_BUSY", message: "old failure", retryable: true } }));
    await act(async () => vi.advanceTimersByTimeAsync(1000));
    expect(api.setViewport).toHaveBeenCalledTimes(2);
    expect(report).not.toHaveBeenCalled();
    owner.update({ ...hiddenOfficialBrowserViewport, epoch: 4 });
    expect(api.setViewport).toHaveBeenCalledTimes(2); // Stale ACK did not undo hide cache.
    api.setViewport.mockResolvedValueOnce({ ok: false, error: { code: "VIEWPORT_CLIP_FAILED", message: "retry", retryable: true } });
    owner.update(viewport);
    await act(async () => {});
    owner.release();
    const next = claimOfficialBrowserViewport(api);
    next.update({ ...viewport, width: 1000 });
    await act(async () => vi.advanceTimersByTimeAsync(1000));
    expect(api.setViewport.mock.calls.map(([value]) => value.width)).toEqual([800, 0, 800, 0, 1000]);
    next.release();
  });

  it("retries the latest geometry after a transient failure and never retries other errors", async () => {
    const api = fakeApi();
    api.setViewport.mockResolvedValueOnce({ ok: false, error: { code: "VIEWPORT_BUSY", message: "busy", retryable: true } });
    const owner = claimOfficialBrowserViewport(api);
    owner.update(viewport);
    await act(async () => {});
    owner.update({ ...viewport, width: 1000 });
    await act(async () => vi.advanceTimersByTimeAsync(1000));
    expect(api.setViewport.mock.calls.map(([value]) => value.width)).toEqual([800, 1000]);
    api.setViewport.mockResolvedValueOnce({ ok: false, error: { code: "VIEWPORT_STALE", message: "stale", retryable: true } });
    owner.update({ ...viewport, width: 1100 });
    await act(async () => vi.advanceTimersByTimeAsync(1000));
    expect(api.setViewport).toHaveBeenCalledTimes(3);
    owner.release();
  });

  it("keeps only the latest layout and prevents an old owner from hiding a reopened child", async () => {
    const api = fakeApi();
    const first = deferred<ApiResult<void>>();
    api.setViewport.mockReturnValueOnce(first.promise);
    const oldReport = vi.fn();
    const owner = claimOfficialBrowserViewport(api, oldReport);
    owner.update(viewport);
    owner.update({ ...viewport, width: 900 });
    owner.update({ ...viewport, width: 1000 });
    expect(api.setViewport).toHaveBeenCalledTimes(1);
    owner.release();
    const next = claimOfficialBrowserViewport(api);
    next.update({ ...viewport, width: 1100 });
    owner.release();
    await act(async () => first.resolve(success(undefined)));
    expect(api.setViewport.mock.calls.map(([value]) => value.width)).toEqual([800, 0, 1100]);
    expect(oldReport).not.toHaveBeenCalled();
    next.update({ ...viewport, width: 1100 });
    expect(api.setViewport).toHaveBeenCalledTimes(3);
    next.release();
    await act(async () => {});
    expect(api.setViewport).toHaveBeenLastCalledWith(expect.objectContaining(hiddenOfficialBrowserViewport));
  });

  it("preserves the page viewport and computes a stage-local scroll-parent intersection", () => {
    const parent = document.createElement("div");
    const stage = document.createElement("div");
    parent.style.overflow = "auto";
    parent.style.overflowX = "auto";
    parent.style.overflowY = "auto";
    parent.append(stage); container.append(parent);
    vi.spyOn(parent, "getBoundingClientRect").mockReturnValue(new DOMRect(50, 40, 400, 300));
    vi.spyOn(stage, "getBoundingClientRect").mockReturnValue(new DOMRect(20, 10, 800, 500));
    Object.defineProperties(parent, { clientWidth: { value: 400 }, clientHeight: { value: 300 } });
    expect(measureOfficialBrowserViewport(stage)).toEqual({ x: 20, y: 10, width: 800, height: 500, visible: true, clip: { x: 30, y: 30, width: 400, height: 300 } });
    parent.style.display = "none";
    expect(measureOfficialBrowserViewport(stage)).toEqual(hiddenOfficialBrowserViewport);
  });
});

describe("official browser transport", () => {
  it("sends hides ahead of visible ACKs, fences each request and drops an older queued show", async () => {
    const nativeInvoke = vi.mocked(invoke);
    const first = deferred<ApiResult<void>>();
    nativeInvoke.mockReturnValueOnce(first.promise).mockResolvedValue(success(undefined));
    const api = createOfficialBrowserApi("tauri");
    const show = api.setViewport({ x: 1, y: 2, width: 800, height: 500, visible: true, epoch: 7 });
    await act(async () => {});
    const queued = api.setViewport({ x: 1, y: 2, width: 900, height: 500, visible: true, epoch: 7 });
    const hide = api.setViewport({ ...hiddenOfficialBrowserViewport, epoch: 7 });
    expect(nativeInvoke).toHaveBeenCalledTimes(2);
    const sent = nativeInvoke.mock.calls.map(([, args]) => (args as { viewport: OfficialBrowserViewport }).viewport);
    expect(sent[0]).toMatchObject({ visible: true, epoch: 7, requestSequence: expect.any(Number) });
    expect(sent[1]).toMatchObject({ visible: false, epoch: 7 });
    expect(sent[1]!.requestSequence).toBeGreaterThan(sent[0]!.requestSequence!);
    await hide;
    await act(async () => first.resolve(success(undefined)));
    await Promise.all([show, queued]);
    expect(nativeInvoke).toHaveBeenCalledTimes(2);
    await api.setViewport({ x: 1, y: 2, width: 1000, height: 500, visible: true, epoch: 7 });
    expect(nativeInvoke).toHaveBeenLastCalledWith("chzzk_browser_set_viewport", { viewport: expect.objectContaining({ width: 1000, epoch: 7 }) });
  });

  it("uses only explicit official-browser commands and sanitizes transport exceptions", async () => {
    const nativeInvoke = vi.mocked(invoke);
    nativeInvoke.mockResolvedValue(success(ready()));
    const api = createOfficialBrowserApi("tauri");
    await api.open("channel");
    await api.snapshot();
    await api.start({ rightsAcknowledged: true, captureChat: false });
    await api.stop();
    await api.connectExtension();
    await api.login();
    await api.logout();
    await api.openInstaller();
    await api.setViewport(hiddenOfficialBrowserViewport);
    await api.openFolder("recording");
    await api.openSegment("recording", 3);
    await api.openMerged("recording");
    await api.retryMerge("recording");
    expect(nativeInvoke.mock.calls).toEqual([
      ["chzzk_browser_open", { input: "channel" }], ["chzzk_browser_snapshot", {}],
      ["chzzk_browser_start", { rightsAcknowledged: true, captureChat: false }], ["chzzk_browser_stop", {}],
      ["chzzk_browser_connect_extension", {}], ["chzzk_browser_login", {}], ["chzzk_browser_logout", {}],
      ["chzzk_browser_open_installer", { browser: "chrome" }],
      ["chzzk_browser_set_viewport", { viewport: expect.objectContaining({ ...hiddenOfficialBrowserViewport, requestSequence: expect.any(Number), epoch: expect.any(Number) }) }], ["chzzk_browser_open_folder", { recordingId: "recording" }],
      ["chzzk_browser_open_segment", { recordingId: "recording", index: 3 }],
      ["chzzk_browser_open_merged", { recordingId: "recording" }],
      ["chzzk_browser_retry_merge", { recordingId: "recording" }],
    ]);
    nativeInvoke.mockRejectedValueOnce(new Error("secret-cookie=do-not-show"));
    const result = await api.snapshot();
    expect(result.ok).toBe(false);
    expect(JSON.stringify(result)).not.toContain("secret-cookie");
  });

  it("does not let slow account/window work delay a viewport hide across API lifetimes", async () => {
    const nativeInvoke = vi.mocked(invoke);
    const opening = deferred<ApiResult<OfficialBrowserSnapshot>>();
    nativeInvoke.mockImplementation(async (command) => command === "chzzk_browser_open" ? opening.promise : success(undefined));
    const first = createOfficialBrowserApi("tauri");
    const second = createOfficialBrowserApi("tauri");
    const open = first.open("channel");
    const layout = second.setViewport(hiddenOfficialBrowserViewport);
    await act(async () => {});
    expect(nativeInvoke).toHaveBeenCalledTimes(2);
    await act(async () => opening.resolve(success(ready())));
    await Promise.all([open, layout]);
    expect(nativeInvoke.mock.calls.map(([command]) => command)).toEqual(["chzzk_browser_set_viewport", "chzzk_browser_open"]);
  });
});
