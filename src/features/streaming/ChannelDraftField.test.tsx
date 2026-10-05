import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, describe, it, expect, vi } from "vitest";
import { ConnectionSetup } from "./ConnectionSetup";
import { LiveChannelPicker } from "./LiveChannelPicker";
import type { LiveChannelsApi, LiveChannelProfile, LiveFavorite } from "../../api/liveChannels";
import type { AutoRecordingApi } from "../../api/autoRecording";
import type { ApiResult } from "../../api/contracts";

const A = "a".repeat(32), B = "b".repeat(32);
const ok = <T,>(data: T) => ({ ok: true as const, data });
let host: HTMLDivElement, root: Root;
beforeEach(() => { vi.useFakeTimers(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); });
function setup() {
  let favorites: LiveFavorite[] = [];
  const api = {
    snapshot: vi.fn<LiveChannelsApi["snapshot"]>(async () => ok(favorites)),
    set: vi.fn<LiveChannelsApi["set"]>(async (id, favorite) => { favorites = favorite ? [{ channelId: id, channelName: "채널 A" }] : []; return ok(favorites); }),
    profile: vi.fn<LiveChannelsApi["profile"]>(async id => ok({ channelName: id === A ? "채널 A" : "채널 B", image: "data:image/png;base64,fixture" })),
  };
  const autoApi = { snapshot: vi.fn<AutoRecordingApi["snapshot"]>().mockResolvedValue(ok({ channels: [], captureChat: true, error: null })), add: vi.fn(), update: vi.fn() };
  const connect = vi.fn(), onInput = vi.fn();
  const render = (inputs = [A, "", "", ""], privacy = false) => act(async () => root.render(<StrictMode><ConnectionSetup runtime="tauri" privacy={privacy} channelsApi={api} autoApi={autoApi}
    mode="mado" onMode={vi.fn()} modeDisabled={false} inputs={inputs} onInput={onInput} disabled={false} pending={false} onConnect={connect}
    auth="signed_in" accountDisabled={false} onLogin={vi.fn()} onLogout={vi.fn()} onGrid={vi.fn()} gridDisabled={false} gridStatus="" onInstaller={vi.fn()} installerDisabled={false}
    channelPicker={<LiveChannelPicker runtime="tauri" inputs={inputs} multiple privacy={privacy} disabled={false} onInputs={vi.fn()} />} /></StrictMode>));
  return { api, autoApi, connect, onInput, render };
}
const delay = () => act(async () => vi.advanceTimersByTimeAsync(250));
const row = (index = 0) => host.querySelectorAll<HTMLElement>(".connection-channel-row")[index]!;

describe("numbered channel drafts", () => {
  it("reserves skeleton profiles beside the number, never below the input", async () => {
    const view = setup(); await view.render(["", "", "", ""]);
    for (const item of host.querySelectorAll(".connection-channel-row")) {
      expect([...item.children].map(child => child.className)).toEqual(["connection-channel-number", "connection-channel-profile is-skeleton", "connection-channel-draft", "connection-channel-favorite"]);
      expect(item.querySelector(".connection-channel-avatar")).not.toBeNull();
      expect(item.querySelector(".connection-channel-profile")).toHaveAttribute("aria-label", "채널 미선택");
      expect(item.querySelector(".connection-channel-draft .connection-channel-profile")).toBeNull();
    }
    await delay(); expect(view.api.profile).not.toHaveBeenCalled();
    await view.render(); await delay();
    expect(row().querySelector(".connection-channel-profile")).not.toHaveClass("is-skeleton");
    await act(async () => row().querySelector("img")!.dispatchEvent(new Event("error")));
    expect(row().querySelector("img")).toBeNull();
    expect(row().querySelector(".connection-channel-avatar")).toHaveTextContent("채");
  });
  it("shows profiles after a short debounce, without search or manual status controls", async () => {
    const view = setup(); await view.render();
    expect([...host.querySelectorAll('.connection-channel-number')].map(node => node.textContent)).toEqual(["1", "2", "3", "4"]);
    expect(host.querySelector('[aria-label="채널 검색"]')).toBeNull();
    expect([...host.querySelectorAll('button')].some(button => button.textContent === "상태 확인")).toBe(false);
    expect(row().querySelector('[aria-busy=true]')).not.toBeNull();
    expect(view.api.profile).not.toHaveBeenCalled();
    await delay();
    expect(row()).toHaveTextContent("채널 A");
    expect(row().querySelector("img")).toHaveAttribute("src", "data:image/png;base64,fixture");
    expect(view.api.profile).toHaveBeenCalledExactlyOnceWith(A);
    expect(view.connect).not.toHaveBeenCalled();
  });
  it("shares favorite state between an input and saved channel without changing recording", async () => {
    const view = setup(); await view.render(); await delay();
    const star = row().querySelector<HTMLButtonElement>('button')!;
    await act(async () => star.click());
    expect(view.api.set).toHaveBeenCalledExactlyOnceWith(A, true);
    expect(star).toHaveAttribute('aria-pressed', 'true');
    const saved = host.querySelector<HTMLButtonElement>('.live-channel-star')!;
    expect(saved).toHaveAttribute('aria-pressed', 'true');
    await act(async () => saved.click());
    expect(star).toHaveAttribute('aria-pressed', 'false');
    expect(view.api.profile).toHaveBeenCalledTimes(1);
    expect(view.connect).not.toHaveBeenCalled(); expect(view.autoApi.add).not.toHaveBeenCalled(); expect(view.autoApi.update).not.toHaveBeenCalled();
  });
  it("does not display a late response for the previous address", async () => {
    const view = setup(); let resolve!: (result: ApiResult<LiveChannelProfile>) => void;
    view.api.profile.mockImplementation(async id => id === A ? new Promise(done => { resolve = done; }) : ok({ channelName: "채널 B", image: null }));
    await view.render(); await delay();
    await view.render([B, "", "", ""]); await delay();
    await act(async () => resolve(ok({ channelName: "이전 채널 A", image: "data:image/png;base64,old" })));
    expect(row()).toHaveTextContent("채널 B"); expect(row()).not.toHaveTextContent("이전 채널 A"); expect(row().querySelector("img")).toBeNull();
  });
  it("deduplicates matching slots and avoids lookups for invalid addresses", async () => {
    const view = setup(); await view.render([A, A, "https://chzzk.naver.com.evil.test/" + B, "partial"]); await delay();
    expect(view.api.profile).toHaveBeenCalledExactlyOnceWith(A);
    expect(row(0)).toHaveTextContent("채널 A"); expect(row(1)).toHaveTextContent("채널 A");
    expect(row(2).querySelector("button")).toBeDisabled(); expect(row(3).querySelector("button")).toBeDisabled();
  });
  it("respects privacy without loading or revealing profiles", async () => {
    const view = setup(); await view.render([A, "", "", ""], true); await delay();
    expect(view.api.profile).not.toHaveBeenCalled(); expect(row().querySelector("img")).toBeNull();
    expect(row()).toHaveTextContent("프라이버시 모드"); expect(row()).not.toHaveTextContent("채널 A");
    await view.render([A, "", "", ""]); await delay();
    expect(row()).toHaveTextContent("채널 A");
    await view.render([A, "", "", ""], true);
    expect(row()).not.toHaveTextContent("채널 A"); expect(row().querySelector("img")).toBeNull();
  });
  it("keeps connecting available if profile lookup fails, and rejects unsafe portraits", async () => {
    const view = setup(); view.api.profile.mockResolvedValueOnce({ ok: false, error: { code: "OFFLINE", message: "offline", retryable: true } });
    await view.render(); await delay();
    expect(row()).toHaveTextContent("채널 정보를 불러오지 못했습니다.");
    expect(host.querySelector('button[type=submit]')).toBeEnabled();
    view.api.profile.mockResolvedValue(ok({ channelName: "채널 B", image: "data:image/svg+xml;base64,untrusted" }));
    await view.render([B, "", "", ""]); await delay();
    expect(row()).toHaveTextContent("채널 B"); expect(row().querySelector("img")).toBeNull();
  });
  it("does not optimistically mark a failed favorite write as saved", async () => {
    const view = setup(); await view.render(); await delay();
    view.api.set.mockResolvedValue({ ok: false, error: { code: "OFFLINE", message: "즐겨찾기 저장 실패", retryable: true } });
    await act(async () => row().querySelector<HTMLButtonElement>('button')!.click());
    expect(row().querySelector("button")).toHaveAttribute('aria-pressed', 'false');
    expect(host.querySelector('[role=alert]')).toHaveTextContent("즐겨찾기 저장 실패");
  });
});
