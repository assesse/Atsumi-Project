import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LiveChannelPicker } from "./LiveChannelPicker";
import { mergeLiveChannels, type LiveChannelsApi } from "../../api/liveChannels";
import type { AutoRecordingApi, AutoRecordingEntry } from "../../api/autoRecording";

const A = "a".repeat(32), B = "b".repeat(32), C = "c".repeat(32), D = "d".repeat(32);
const ok = <T,>(data: T) => ({ ok: true as const, data });
const recording: AutoRecordingEntry = { channelId: A, channelName: "예약 방송", enabled: true, status: "recording", recordingId: "rec-a", checkedAt: 0, message: null };
let container: HTMLDivElement, root: Root;
beforeEach(() => { vi.useFakeTimers(); container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); });
function setup(inputs = ["", "", "", ""], multiple = false, privacy = false) {
  const api = { snapshot: vi.fn().mockResolvedValue(ok([{ channelId: A, channelName: "저장한 이름" }, { channelId: B, channelName: "즐겨찾기 방송" }])), set: vi.fn().mockResolvedValue(ok([{ channelId: B, channelName: "즐겨찾기 방송" }])), profile: vi.fn().mockResolvedValue(ok({ channelName: "", image: "data:image/png;base64,fixture" })) } satisfies LiveChannelsApi;
  const auto = { snapshot: vi.fn<AutoRecordingApi["snapshot"]>().mockResolvedValue(ok({ channels: [recording], captureChat: true, error: null })), add: vi.fn(), update: vi.fn() } satisfies AutoRecordingApi;
  const onInputs = vi.fn();
  return { api, auto, onInputs, render: () => act(async () => root.render(<LiveChannelPicker runtime="tauri" inputs={inputs} multiple={multiple} privacy={privacy} disabled={false} onInputs={onInputs} api={api} autoApi={auto} />)) };
}
const click = async (name: string) => act(async () => container.querySelector<HTMLButtonElement>('[aria-label="' + name + '"]')!.click());
describe("live channel selector", () => {
  it("aligns both icon controls in the same action row and labels reservations consistently", async () => {
    const view = setup();
    view.auto.snapshot.mockResolvedValue(ok({ channels: [{ ...recording, status: "waiting", recordingId: null }], captureChat: true, error: null }));
    await view.render();
    expect(container.querySelectorAll('.live-channel-actions')).toHaveLength(2);
    for (const actions of container.querySelectorAll('.live-channel-actions')) {
      expect(actions.querySelectorAll('button > svg')).toHaveLength(2);
      for (const icon of actions.querySelectorAll('svg')) expect(icon.getAttribute('viewBox')).toBe('0 0 24 24');
    }
    const labels = [...container.querySelectorAll('.live-channel-choice small')].map(label => label.textContent);
    expect(labels).toEqual(['녹화 예약 됨', '예약 꺼짐']);
    view.auto.update.mockResolvedValue(ok({ channels: [{ ...recording, status: "disabled", enabled: false, recordingId: null }], captureChat: true, error: null }));
    await click("예약 방송 자동 녹화 끄기");
    expect([...container.querySelectorAll('.live-channel-choice small')].map(label => label.textContent)).toEqual(['예약 꺼짐', '예약 꺼짐']);
  });
  it("toggles automatic recording with the same icon without altering favorites or playback", async () => {
    const view = setup(); await view.render();
    view.auto.update.mockResolvedValue(ok({ channels: [{ ...recording, enabled: false, status: "disabled" }], captureChat: true, error: null }));
    await click("예약 방송 자동 녹화 끄기");
    expect(view.auto.update).toHaveBeenCalledExactlyOnceWith(A, { action: "enabled", enabled: false });
    expect(container.querySelector('[aria-label="예약 방송 자동 녹화 켜기"]')).toHaveAttribute("aria-pressed", "false");
    view.auto.add.mockResolvedValue(ok({ channels: [recording, { ...recording, channelId: B, channelName: "즐겨찾기 방송", status: "waiting", recordingId: null }], captureChat: true, error: null }));
    await click("즐겨찾기 방송 자동 녹화 켜기");
    expect(view.auto.add).toHaveBeenCalledExactlyOnceWith(B);
    expect(view.api.set).not.toHaveBeenCalled(); expect(view.onInputs).not.toHaveBeenCalled();
  });
  it("refreshes live/offline presence every 30 seconds, independently from reservation state", async () => {
    const view = setup();
    view.api.profile.mockResolvedValueOnce(ok({ channelName: "", image: null, liveStatus: "live" })).mockResolvedValueOnce(ok({ channelName: "", image: null, liveStatus: "offline" }));
    await view.render();
    expect(container.querySelectorAll('.live-channel-presence.is-live')).toHaveLength(1);
    expect(container.querySelectorAll('.live-channel-presence.is-offline')).toHaveLength(1);
    view.api.profile.mockResolvedValue(ok({ channelName: "", image: null, liveStatus: "offline" }));
    await act(async () => vi.advanceTimersByTimeAsync(30_000));
    expect(view.api.profile).toHaveBeenCalledTimes(4);
    expect(container.querySelectorAll('.live-channel-presence.is-live')).toHaveLength(0);
  });
  it("adds a favorite to a new dynamic row when fewer than four broadcasts are selected", async () => {
    const view = setup([A], true); await view.render(); await click("즐겨찾기 방송 선택");
    expect(view.onInputs).toHaveBeenCalledExactlyOnceWith([A, B]);
  });
  it("keeps available favorites during reservation initialization without raising an error", async () => {
    const view = setup();
    view.auto.snapshot.mockResolvedValue({ ok: false, error: { code: "BROWSER_INITIALIZING", message: "준비 중", retryable: true } });
    await view.render();
    await act(async () => vi.advanceTimersByTimeAsync(250));
    expect(container.querySelectorAll(".live-channel-row")).toHaveLength(2);
    expect(container.querySelector(".recording-notice.is-loading")).toHaveTextContent("저장한 채널 준비 중");
    expect(container.querySelector('[role="alert"]')).toBeNull();
    view.auto.snapshot.mockResolvedValue(ok({ channels: [recording], captureChat: true, error: null }));
    await act(async () => vi.advanceTimersByTimeAsync(3000));
    expect(container.querySelector(".recording-notice")).toBeNull();
  });
  it("deduplicates favorites and reservations and selects without changing recording", async () => {
    const view = setup(); await view.render();
    expect(container.querySelectorAll(".live-channel-row")).toHaveLength(2); expect(container).toHaveTextContent("녹화 중");
    await click("예약 방송 선택"); expect(view.onInputs).toHaveBeenCalledExactlyOnceWith([A, "", "", ""]);
    expect(view.api.set).not.toHaveBeenCalled(); expect(view.auto.add).not.toHaveBeenCalled(); expect(view.auto.update).not.toHaveBeenCalled();
  });
  it("removing a favorite leaves its reservation and recording visible", async () => {
    const view = setup(); await view.render(); await click("예약 방송 즐겨찾기 해제");
    expect(view.api.set).toHaveBeenCalledExactlyOnceWith(A, false);
    expect(container.querySelectorAll(".live-channel-row")).toHaveLength(2); expect(container).toHaveTextContent("녹화 중");
    expect(view.auto.update).not.toHaveBeenCalled();
  });
  it("hides inactive unfavorited legacy rows without deleting their saved reservation", async () => {
    const view = setup();
    view.auto.snapshot.mockResolvedValue(ok({ channels: [recording, { ...recording, channelId: C, channelName: "옛 예약", enabled: false, status: "disabled", recordingId: null }, { ...recording, channelId: D, channelName: "마무리 방송", enabled: false, status: "stopping" }], captureChat: true, error: null }));
    await view.render();
    expect(container.querySelectorAll(".live-channel-row")).toHaveLength(3);
    expect(container).not.toHaveTextContent("옛 예약");
    expect(container).toHaveTextContent("마무리 방송");
    expect(view.auto.update).not.toHaveBeenCalled();
    expect(view.api.set).not.toHaveBeenCalled();
  });
  it("shows automatic recording failures instead of calling them a normal reservation", async () => {
    const view = setup();
    view.auto.snapshot.mockResolvedValue(ok({ channels: [{ ...recording, status: "attention", recordingId: null, message: "영상 준비에 실패했습니다." }], captureChat: true, error: null }));
    await view.render();
    expect(container.querySelector('.live-channel-choice small.is-problem')).toHaveTextContent("녹화 확인 필요");
    expect(container.querySelector('.live-channel-choice small.is-problem')).toHaveAttribute("title", "영상 준비에 실패했습니다.");
  });
  it("keeps hidden legacy reservations removable in the management popup and returns focus without stopping", async () => {
    const view = setup();
    const legacy = { ...recording, channelId: C, channelName: "옛 예약", enabled: false, status: "disabled", recordingId: null };
    view.auto.snapshot.mockResolvedValue(ok({ channels: [recording, legacy], captureChat: true, error: null }));
    await view.render();
    expect(container).not.toHaveTextContent("옛 예약");
    const opener = container.querySelector<HTMLButtonElement>('[aria-label="자동 녹화 관리"]')!;
    await act(async () => { opener.focus(); opener.click(); });
    const dialog = document.querySelector('dialog[aria-label="자동 녹화 관리"]')!;
    expect(dialog).toHaveAttribute("open");
    expect(dialog).toHaveAttribute("data-native-preserve-video", "true");
    expect(dialog).toHaveTextContent("옛 예약");
    expect(dialog.querySelector('[data-tour="chzzk-auto-register"]')).toBeNull();
    expect(dialog.querySelector('.auto-record-stop')).toHaveTextContent("녹화 중지");
    view.auto.update.mockResolvedValue(ok({ channels: [recording], captureChat: true, error: null }));
    await act(async () => dialog.querySelector<HTMLButtonElement>('[aria-label="옛 예약 등록 해제"]')!.click());
    expect(view.auto.update).toHaveBeenCalledExactlyOnceWith(C, { action: "remove" });
    expect(dialog).not.toHaveTextContent("옛 예약");
    await act(async () => dialog.querySelector<HTMLButtonElement>('[aria-label="자동 녹화 관리 닫기"]')!.click());
    expect(document.querySelector('dialog[aria-label="자동 녹화 관리"]')).toBeNull();
    expect(document.activeElement).toBe(opener);
    expect(view.auto.update).toHaveBeenCalledTimes(1);
    expect(view.onInputs).not.toHaveBeenCalled();
  });
  it("does not reshuffle a reservation when its favorite switch changes", async () => {
    const view = setup();
    view.auto.snapshot.mockResolvedValue(ok({ channels: [{ ...recording, status: "waiting" }], captureChat: true, error: null }));
    await view.render();
    const names = () => [...container.querySelectorAll('.live-channel-choice')].map(button => button.getAttribute('aria-label'));
    const before = names();
    await click("예약 방송 즐겨찾기 해제");
    expect(names()).toEqual(before);
    expect(container.querySelector('[aria-label="예약 방송 즐겨찾기 추가"]')).toHaveAttribute('aria-pressed', 'false');
  });

  it("treats a favorite action during initialization as preparation, not a red error", async () => {
    const view = setup(); await view.render();
    view.api.set.mockResolvedValueOnce({ ok: false, error: { code: "BROWSER_INITIALIZING", message: "준비 중", retryable: true } });
    await click("예약 방송 즐겨찾기 해제");
    await act(async () => vi.advanceTimersByTimeAsync(250));
    expect(container.querySelector(".recording-notice.is-loading")).toHaveTextContent("저장한 채널 준비 중");
    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(container.querySelectorAll(".live-channel-row")).toHaveLength(2);
  });
  it("fills a free Mado slot without discarding the other selected channels", async () => {
    const view = setup([C, "", D, ""], true); await view.render(); await click("예약 방송 선택");
    expect(view.onInputs).toHaveBeenCalledExactlyOnceWith([C, A, D, ""]);
  });
  it("deselects an existing channel selected by URL instead of duplicating it", async () => {
    const view = setup(["https://chzzk.naver.com/live/" + A, B, "", ""], true); await view.render(); await click("예약 방송 선택");
    expect(view.onInputs).toHaveBeenCalledExactlyOnceWith(["", B, "", ""]);
  });
  it("hides identity but retains recording status in privacy mode", async () => {
    const view = setup(["", "", "", ""], false, true); await view.render();
    expect(container.innerHTML).not.toContain(A); expect(container).not.toHaveTextContent("예약 방송"); expect(container).toHaveTextContent("녹화 중");
    expect(view.api.profile).not.toHaveBeenCalled(); expect(container.querySelector("img")).toBeNull();
  });
  it("combines portraits, names and statuses into favorite banners without fetching again on each status poll", async () => {
    const view = setup(); await view.render();
    expect(container.querySelectorAll('.live-channel-choice .live-channel-avatar img')).toHaveLength(2);
    expect(container.querySelectorAll('.live-channel-row.is-favorite')).toHaveLength(2);
    expect(container.querySelector('.live-channel-choice .live-channel-identity')).toHaveTextContent('예약 방송방송 확인 중녹화 중');
    expect(view.api.profile).toHaveBeenCalledTimes(2);
    await act(async () => vi.advanceTimersByTimeAsync(9000));
    expect(view.api.profile).toHaveBeenCalledTimes(2);
  });
  it("keeps channel selection available while portraits are loading or unavailable", async () => {
    const view = setup(); view.api.profile.mockReturnValue(new Promise(() => {})); await view.render();
    await click("예약 방송 선택"); expect(view.onInputs).toHaveBeenCalledExactlyOnceWith([A, "", "", ""]);
    expect(container.querySelectorAll('.live-channel-avatar')).toHaveLength(2);
  });
  it("accepts verified GIF portraits as well as static images but never remote or SVG sources", async () => {
    const view = setup(); view.api.profile.mockResolvedValueOnce(ok({ channelName: "", image: "data:image/gif;base64,fixture" }));
    view.api.profile.mockResolvedValueOnce(ok({ channelName: "", image: "data:image/svg+xml;base64,untrusted" }));
    await view.render();
    expect(container.querySelectorAll('img')).toHaveLength(1);
    expect(container.querySelector('img')?.src).toBe('data:image/gif;base64,fixture');
  });
  it("fences a stale poll after removing a bookmark", async () => {
    const view = setup(); await view.render();
    let resolve!: (value: unknown) => void;
    view.api.snapshot.mockReturnValueOnce(new Promise(done => { resolve = done; }));
    await act(async () => vi.advanceTimersByTimeAsync(3000));
    await click("예약 방송 즐겨찾기 해제");
    await act(async () => resolve(ok([{ channelId: A, channelName: "이전 값" }])));
    expect(container.querySelector('[aria-label="예약 방송 즐겨찾기 추가"]')).toHaveAttribute("aria-pressed", "false");
  });
  it("puts recording channels first without turning favorites into reservations", () => {
    const result = mergeLiveChannels([{ channelId: B, channelName: "ㄱ" }, { channelId: A, channelName: "ㄴ" }], [recording]);
    expect(result.map(r => r.channelId)).toEqual([A, B]); expect(result[1]!.scheduled).toBeUndefined(); expect(result[0]!.favorite).toBe(true);
  });
});
