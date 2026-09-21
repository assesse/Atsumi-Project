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
  const auto = { snapshot: vi.fn().mockResolvedValue(ok({ channels: [recording], captureChat: true, error: null })), add: vi.fn(), update: vi.fn() } satisfies AutoRecordingApi;
  const onInputs = vi.fn();
  return { api, auto, onInputs, render: () => act(async () => root.render(<LiveChannelPicker runtime="tauri" inputs={inputs} multiple={multiple} privacy={privacy} disabled={false} onInputs={onInputs} api={api} autoApi={auto} />)) };
}
const click = async (name: string) => act(async () => container.querySelector<HTMLButtonElement>('[aria-label="' + name + '"]')!.click());
describe("live channel selector", () => {
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
    expect(container.querySelector('.live-channel-choice .live-channel-identity')).toHaveTextContent('예약 방송녹화 중');
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
