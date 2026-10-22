import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SoopWorkspace, type SoopWorkspaceProps } from "./SoopWorkspace";
import type { SoopApi, SoopChannel } from "../../api/soop";
import { SOOP_FAVORITES_KEY } from "./soopFavorites";
import type { MediaRef } from "../platforms/platformMedia";
import type { SoopAutoRecordApi } from "../../api/soopAutoRecord";
import workspaceStyles from "./SoopWorkspace.css?raw";

const playerLifecycle = vi.hoisted(() => ({ open: vi.fn(), close: vi.fn() }));
// Exercise workspace mount/identity contracts without opening native media sessions.
vi.mock("../platforms/PlatformLivePlayer", async () => {
  const { useEffect } = await import("react");
  return { PlatformLivePlayer: ({ reference, title, privacyMode, active, videoVisible }: { reference: MediaRef; title: string; privacyMode: boolean; active: boolean; videoVisible: boolean }) => {
    useEffect(() => { playerLifecycle.open(reference); return () => { playerLifecycle.close(reference); }; }, [reference.provider, reference.kind, reference.externalId]);
    return <div data-testid="native-live-player" data-provider={reference.provider} data-kind={reference.kind} data-external-id={reference.externalId} data-private={privacyMode} data-active={active} data-video-visible={videoVisible} aria-label={privacyMode ? "프라이버시 모드" : title} />;
  } };
});
vi.mock("../platforms/PlatformMediaLibrary", () => ({ PlatformMediaLibrary: () => <div data-testid="media-library" /> }));
vi.mock("../platforms/PlatformScreenshotLibrary", () => ({ PlatformScreenshotLibrary: ({ provider, active }: { provider: string; active: boolean }) => <section data-testid="screenshot-library" data-provider={provider} hidden={!active} /> }));

const LIVE: SoopChannel = { channelId: "aflol", name: "LCK_KR", title: "테스트 방송", image: null, status: "live", viewers: 12 };
let root: Root, container: HTMLDivElement;
beforeEach(() => { vi.useFakeTimers(); playerLifecycle.open.mockClear(); playerLifecycle.close.mockClear(); localStorage.clear(); container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); });
function setup(extra: Partial<SoopWorkspaceProps> = {}) {
  const api = { inspect: vi.fn<SoopApi["inspect"]>().mockResolvedValue(LIVE) };
  const autoRecordApi: SoopAutoRecordApi = { snapshot: vi.fn().mockResolvedValue({ revision: 0, channels: [], error: null, ready: true }), set: vi.fn() };
  let props: SoopWorkspaceProps = { active: true, runtime: "tauri", privacyMode: false, view: "live", api, autoRecordApi, ...extra };
  return { api, render: async (next: Partial<SoopWorkspaceProps> = {}) => { props = { ...props, ...next }; await act(async () => root.render(<SoopWorkspace {...props} />)); } };
}
async function openDrawer() { if (!container.querySelector(".soop-connection-panel")) await act(async () => container.querySelector<HTMLButtonElement>(".soop-bottom-handle")!.click()); }
async function input(value: string, index = 1) {
  await openDrawer();
  await act(async () => { const element = container.querySelector<HTMLInputElement>(`[aria-label="${index}번 SOOP 채널 링크 또는 ID"]`)!; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(element, value); element.dispatchEvent(new Event("input", { bubbles: true })); });
}
const click = async (selector: string) => { if (!container.querySelector(selector)) await openDrawer(); await act(async () => container.querySelector<HTMLButtonElement>(selector)!.click()); };
const inspect = async () => act(async () => vi.advanceTimersByTimeAsync(350));

describe("SOOP workspace", () => {
  it("normalizes the removed favorites route to Explore without deleting favorites or starting a player", async () => {
    const stored = JSON.stringify([{ id: "123", kind: "video", title: "저장된 영상", channelId: "aflol", channelName: "LCK", duration: "1:00", restricted: false }]);
    localStorage.setItem("atsumi:soop:video-favorites:v1", stored);
    localStorage.setItem(SOOP_FAVORITES_KEY, JSON.stringify([{ channelId: "aflol", name: "LCK" }]));
    const onViewChange = vi.fn(); const view = setup({ view: "favorites", onViewChange });
    await view.render();
    expect(onViewChange).toHaveBeenCalledExactlyOnceWith("explore");
    expect(container.querySelector('[aria-label="SOOP 검색 종류"]')).toHaveValue("saved");
    expect(container.querySelector(".soop-result")).toHaveTextContent("저장된 영상");
    expect(container.querySelector(".soop-channel-panel")).toBeNull();
    expect(container.querySelector(".soop-connection-panel")).toBeNull();
    expect(localStorage.getItem("atsumi:soop:video-favorites:v1")).toBe(stored);
    expect(playerLifecycle.open).not.toHaveBeenCalled();
    await view.render({ view: "explore" });
    expect(container.querySelector('[aria-label="SOOP 검색 종류"]')).toHaveValue("saved");
    await view.render({ view: "live" }); await openDrawer();
    expect(container.querySelector(".soop-favorite")).toHaveTextContent("LCK");
  });
  it("keeps a reservation visible after unstar and uses a separate fixed record toggle", async () => {
    localStorage.setItem(SOOP_FAVORITES_KEY, JSON.stringify([{ channelId: "aflol", name: "LCK_KR" }]));
    const scheduled = { channelId: "aflol", name: "LCK_KR", enabled: true, status: "idle" as const, liveStatus: "live" as const, jobId: null, checkedAt: 1, error: null };
    const autoRecordApi = { snapshot: vi.fn().mockResolvedValue({ revision: 2, channels: [scheduled], ready: true, error: null }), set: vi.fn() };
    const view = setup({ autoRecordApi }); await view.render(); await openDrawer();
    const record = container.querySelector<HTMLButtonElement>(".soop-record-toggle")!;
    const row = container.querySelector(".soop-favorite")!;
    expect(record).toHaveAttribute("aria-pressed", "true"); expect(row).toHaveTextContent("방송 중 · 녹화 예약 됨");
    await click(".soop-favorite .soop-star"); expect(container.querySelector(".soop-favorite")).toBe(row);
    expect(container.querySelector(".soop-record-toggle")).toBe(record); expect(container.querySelector(".soop-favorite .soop-star")).toHaveAttribute("aria-pressed", "false");
    expect(autoRecordApi.set).not.toHaveBeenCalled();
    await view.render({ active: false, privacyMode: true }); expect(autoRecordApi.set).not.toHaveBeenCalled();
  });
  it("does not create a reservation by favoriting; acknowledged toggle keeps centered controls", async () => {
    const autoRecordApi = { snapshot: vi.fn().mockResolvedValue({ revision: 1, channels: [], ready: true, error: null }), set: vi.fn().mockResolvedValue({ revision: 2, channels: [{ channelId: "aflol", name: "LCK_KR", enabled: true, status: "idle", liveStatus: "live", jobId: null, checkedAt: 1, error: null }], ready: true, error: null }) };
    const view = setup({ autoRecordApi }); await view.render(); await input("aflol"); await inspect(); await click(".soop-channel-row .soop-star");
    expect(autoRecordApi.set).not.toHaveBeenCalled(); expect(container.querySelector(".soop-favorite")).toHaveTextContent("예약 꺼짐");
    const record = container.querySelector(".soop-record-toggle"), star = container.querySelector(".soop-favorite .soop-star");
    await click(".soop-record-toggle"); expect(autoRecordApi.set).toHaveBeenCalledExactlyOnceWith("aflol", "LCK_KR", true, 1);
    expect(container.querySelector(".soop-record-toggle")).toBe(record); expect(container.querySelector(".soop-favorite .soop-star")).toBe(star);
    expect(record).toHaveAttribute("aria-pressed", "true");
    expect(workspaceStyles).toMatch(/\.soop-workspace \.soop-record-toggle\{[^}]*place-items:center[^}]*width:38px;height:38px/);
  });
  it("preserves surviving row order when a reserved channel is unstarred and starred again", async () => {
    localStorage.setItem(SOOP_FAVORITES_KEY, JSON.stringify([{ channelId: "aflol", name: "LCK_KR" }, { channelId: "second", name: "Second" }]));
    const reservation = { channelId: "aflol", name: "LCK_KR", enabled: true, status: "idle", liveStatus: "live", jobId: null, checkedAt: 1, error: null };
    const autoRecordApi = { snapshot: vi.fn().mockResolvedValue({ revision: 2, channels: [reservation], ready: true, error: null }), set: vi.fn() };
    const view = setup({ autoRecordApi }); await view.render(); await openDrawer();
    const before = [...container.querySelectorAll(".soop-favorite")];
    const firstRecord = before[0]!.querySelector(".soop-record-toggle");
    expect(before).toHaveLength(2);
    await click(".soop-favorite:first-child .soop-star");
    expect([...container.querySelectorAll(".soop-favorite")]).toEqual(before);
    expect(container.querySelector(".soop-record-toggle")).toBe(firstRecord);
    expect(before[0]!.querySelector(".soop-star")).toHaveAttribute("aria-pressed", "false");
    await click(".soop-favorite:first-child .soop-star");
    expect([...container.querySelectorAll(".soop-favorite")]).toEqual(before);
    expect(before[0]!.querySelector(".soop-star")).toHaveAttribute("aria-pressed", "true");
    expect(autoRecordApi.set).not.toHaveBeenCalled();
    await input("third");
    // A newly added channel appends without reordering the existing union.
    view.api.inspect.mockResolvedValueOnce({ ...LIVE, channelId: "third", name: "Third" });
    await inspect(); await click(".soop-channel-row .soop-star");
    expect([...container.querySelectorAll(".soop-favorite")].slice(0, 2)).toEqual(before);
    expect(container.querySelectorAll(".soop-favorite")).toHaveLength(3);
  });
  it("reports watching lifetime independently of active view, privacy or drawer", async () => {
    const onWatchingChange = vi.fn(); const view = setup({ onWatchingChange }); await view.render();
    expect(onWatchingChange).toHaveBeenLastCalledWith(false);
    await input("aflol"); await inspect(); await click(".soop-start"); expect(onWatchingChange).toHaveBeenLastCalledWith(true);
    const player = container.querySelector('[data-testid="native-live-player"]'); onWatchingChange.mockClear();
    await openDrawer(); await view.render({ view: "downloads", active: false, privacyMode: true });
    expect(onWatchingChange).not.toHaveBeenCalled(); expect(container.querySelector('[data-testid="native-live-player"]')).toBe(player);
    await view.render({ view: "live", active: true, privacyMode: false }); await click(".soop-stop");
    expect(onWatchingChange).toHaveBeenCalledExactlyOnceWith(false);
  });
  it("begins empty and never loads a player or remote metadata on startup", async () => {
    const view = setup(); await view.render();
    expect(container).toHaveTextContent("방송을 시청해 보세요");
    expect(container.querySelector(".soop-connection-panel")).toBeNull();
    expect(container.querySelector('[data-testid="native-live-player"]')).toBeNull();
    expect(playerLifecycle.open).not.toHaveBeenCalled();
    expect(view.api.inspect).not.toHaveBeenCalled();
    expect(container.querySelector('[data-tour="soop-input"]')).not.toBeNull();
    expect(container.querySelector('[data-tour="soop-content"]')).not.toBeNull();
  });
  it("normalizes the channel then opens only after an explicit watch action", async () => {
    const view = setup(); await view.render(); await input("https://play.sooplive.co.kr/aflol/123"); await inspect();
    expect(view.api.inspect).toHaveBeenCalledExactlyOnceWith("aflol");
    expect(container).toHaveTextContent("LCK_KR");
    expect(container.querySelector('[data-testid="native-live-player"]')).toBeNull();
    await click(".soop-start");
    expect(playerLifecycle.open).toHaveBeenCalledExactlyOnceWith({ provider: "soop", kind: "live", externalId: "aflol" });
    expect(container.querySelector('[data-testid="native-live-player"]')).toHaveAttribute("aria-label", "테스트 방송");
    expect(container.querySelector("iframe")).toBeNull();
    await click(".soop-stop"); expect(container.querySelector('[data-testid="native-live-player"]')).toBeNull();
    expect(playerLifecycle.close).toHaveBeenCalledExactlyOnceWith({ provider: "soop", kind: "live", externalId: "aflol" });
  });
  it("rejects foreign URLs without making a metadata request", async () => {
    const view = setup(); await view.render(); await input("https://evil.test/aflol"); await inspect();
    expect(view.api.inspect).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("링크 또는 ID를 확인");
  });
  it("renders a validated GIF portrait supplied by the backend", async () => {
    const view = setup(); view.api.inspect.mockResolvedValue({ ...LIVE, image: "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7" });
    await view.render(); await input("aflol"); await inspect();
    expect(container.querySelector(".soop-input-identity img")).toHaveAttribute("src", "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7");
  });
  it("keeps the native player when changing to the same channel after a metadata refresh", async () => {
    const view = setup(); await view.render(); await input("aflol"); await inspect(); await click(".soop-start");
    const player = container.querySelector('[data-testid="native-live-player"]')!;
    view.api.inspect.mockResolvedValue({ ...LIVE, viewers: 14 });
    await openDrawer(); await inspect(); await click(".soop-change");
    expect(container.querySelector('[data-testid="native-live-player"]')).toBe(player);
    expect(playerLifecycle.open).toHaveBeenCalledTimes(1);
    expect(playerLifecycle.close).not.toHaveBeenCalled();
    expect(container.querySelector(".soop-player-loading")).toBeNull();
  });
  it("does not call an offline or unknown channel live", async () => {
    const view = setup(); view.api.inspect.mockResolvedValue({ ...LIVE, status: "offline" });
    await view.render(); await input("aflol"); await inspect();
    expect(container).toHaveTextContent("방송 종료"); expect(container.querySelector(".soop-start")).toBeDisabled();
    expect(container.querySelector('[data-testid="native-live-player"]')).toBeNull();
    expect(container.querySelector(".soop-inspect")).toBeNull();
  });
  it("ignores stale metadata after the input is cleared", async () => {
    let resolve!: (channel: SoopChannel) => void;
    const view = setup(); view.api.inspect.mockReturnValue(new Promise((done) => { resolve = done; }));
    await view.render(); await input("aflol"); await inspect();
    const button = container.querySelector('[aria-label="1번 채널 입력 지우기"]'); expect(button).not.toBeNull();
    await click('[aria-label="1번 채널 입력 지우기"]');
    await act(async () => resolve(LIVE));
    expect(container.querySelector('[aria-label="1번 채널 입력 지우기"]')).toBe(button);
    expect(container.querySelector(".soop-start")).toBeDisabled();
  });
  it("ignores a response from the previously typed channel", async () => {
    let resolve!: (channel: SoopChannel) => void;
    const view = setup(); view.api.inspect.mockReturnValueOnce(new Promise((done) => { resolve = done; }));
    await view.render(); await input("aflol"); await inspect();
    await input("other"); view.api.inspect.mockResolvedValue({ ...LIVE, channelId: "other", name: "다른 채널" }); await inspect();
    await act(async () => resolve(LIVE));
    expect(container.querySelector(".soop-input-identity")).toHaveTextContent("다른 채널");
  });
  it("persists channel bookmarks without portraits and keeps the star in place", async () => {
    const view = setup(); await view.render(); await input("aflol"); await inspect();
    const star = container.querySelector('[aria-label="1번 채널 즐겨찾기 추가"]');
    await click('[aria-label="1번 채널 즐겨찾기 추가"]');
    expect(container.querySelector('[aria-label="1번 채널 즐겨찾기 해제"]')).toBe(star);
    expect(JSON.parse(localStorage.getItem(SOOP_FAVORITES_KEY)!)).toEqual([{ channelId: "aflol", name: "LCK_KR" }]);
    await click('[aria-label="1번 채널 즐겨찾기 해제"]');
    expect(JSON.parse(localStorage.getItem(SOOP_FAVORITES_KEY)!)).toEqual([]);
  });
  it("loads favorite presence without starting playback", async () => {
    localStorage.setItem(SOOP_FAVORITES_KEY, JSON.stringify([{ channelId: "aflol", name: "LCK" }]));
    const view = setup(); await view.render(); await openDrawer();
    expect(view.api.inspect).toHaveBeenCalledWith("aflol");
    expect(container.querySelector(".soop-favorite")).toHaveTextContent("방송 중");
    expect(container.querySelector('[data-testid="native-live-player"]')).toBeNull();
  });
  it("changes stale live state to unknown after a failed refresh", async () => {
    localStorage.setItem(SOOP_FAVORITES_KEY, JSON.stringify([{ channelId: "aflol", name: "LCK" }]));
    const view = setup(); await view.render(); await openDrawer(); view.api.inspect.mockRejectedValue(new Error("offline network"));
    await act(async () => vi.advanceTimersByTimeAsync(60_000));
    expect(container.querySelector(".soop-favorite")).toHaveTextContent("상태 미확인");
  });
  it.each([{ active: false }, { privacyMode: true }, { view: "favorites" as const }, { view: "explore" as const }, { view: "downloads" as const }, { view: "screenshots" as const }])("keeps the same player across presentation changes %j", async (next: Partial<SoopWorkspaceProps>) => {
    const view = setup(); await view.render(); await input("aflol"); await inspect(); await click(".soop-start");
    const player = container.querySelector('[data-testid="native-live-player"]'); expect(player).not.toBeNull();
    await view.render(next); expect(container.querySelector('[data-testid="native-live-player"]')).toBe(player);
    if (next.active === false) { expect(container.querySelector(".soop-workspace")).toHaveAttribute("hidden"); expect(container.querySelector(".soop-workspace")).toHaveAttribute("inert"); }
    if (next.privacyMode) expect(player).toHaveAttribute("data-private", "true");
    if (next.view) expect(container.querySelector(".soop-player-area")).toHaveAttribute("hidden");
    await view.render({ active: true, privacyMode: false, view: "live" }); expect(container.querySelector('[data-testid="native-live-player"]')).toBe(player);
    expect(playerLifecycle.open).toHaveBeenCalledTimes(1);
    expect(playerLifecycle.close).not.toHaveBeenCalled();
  });
  it("changes the native session only after explicitly selecting a different channel", async () => {
    const view = setup(); await view.render(); await input("aflol"); await inspect(); await click(".soop-start");
    const player = container.querySelector('[data-testid="native-live-player"]');
    view.api.inspect.mockResolvedValue({ ...LIVE, channelId: "other", name: "다른 채널", title: "두 번째 방송" });
    await input("other"); await inspect();
    expect(container.querySelector('[data-testid="native-live-player"]')).toBe(player);
    expect(playerLifecycle.close).not.toHaveBeenCalled();
    await click(".soop-change");
    expect(playerLifecycle.close).toHaveBeenCalledExactlyOnceWith({ provider: "soop", kind: "live", externalId: "aflol" });
    expect(playerLifecycle.open).toHaveBeenLastCalledWith({ provider: "soop", kind: "live", externalId: "other" });
    expect(container.querySelector('[data-testid="native-live-player"]')).toHaveAttribute("aria-label", "두 번째 방송");
  });
  it("does not promise authentication inside a removed official iframe", async () => {
    const view = setup(); await view.render(); await openDrawer();
    expect(container.querySelector('[role="tooltip"]')).toHaveTextContent("공개 방송만 지원");
    expect(container).not.toHaveTextContent("공식 플레이어에서 진행");
    expect(container.querySelector("iframe")).toBeNull();
  });
  it("does not fetch or reveal saved channel names while private", async () => {
    localStorage.setItem(SOOP_FAVORITES_KEY, JSON.stringify([{ channelId: "aflol", name: "PrivateName" }]));
    const view = setup({ privacyMode: true }); await view.render(); await openDrawer();
    expect(view.api.inspect).not.toHaveBeenCalled(); expect(container).not.toHaveTextContent("PrivateName");
    expect(container.querySelector('[aria-label="1번 SOOP 채널 링크 또는 ID"]')).toHaveAttribute("type", "password");
  });
  it("offers a visible retry after a failed metadata request", async () => {
    const view = setup(); view.api.inspect.mockRejectedValueOnce(new Error("연결 실패")); await view.render(); await input("aflol"); await inspect();
    expect(container.querySelector('[role="alert"]')).toHaveTextContent("연결 실패");
    await click('.soop-inline-error button'); await inspect(); expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(container).toHaveTextContent("LCK_KR");
  });

  it("uses a thin nonmodal drawer without automatic metadata or playback on open", async () => {
    const view = setup(); await view.render();
    const handle = container.querySelector<HTMLButtonElement>(".soop-bottom-handle")!;
    expect(handle).toHaveTextContent(""); expect(handle).toHaveAttribute("aria-expanded", "false");
    await openDrawer();
    const panel = container.querySelector(".soop-connection-panel")!;
    expect(panel).toHaveAttribute("aria-modal", "false");
    expect(panel.querySelector(".soop-favorites")?.compareDocumentPosition(panel.querySelector(".soop-input-area")!)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(panel.lastElementChild).toHaveClass("soop-watch-actions");
    expect(view.api.inspect).not.toHaveBeenCalled(); expect(playerLifecycle.open).not.toHaveBeenCalled();
    await click('[aria-label="방송 보기 닫기"]');
    expect(container.querySelector(".soop-connection-panel")).toBeNull();
    expect(container.querySelector(".soop-bottom-handle")).toBe(handle);
  });
  it("opens and closes by vertical drag without the following click reversing the gesture", async () => {
    const view = setup(); await view.render();
    const handle = container.querySelector<HTMLButtonElement>(".soop-bottom-handle")!;
    const pointer = async (type: string, y: number) => act(async () => {
      const event = new MouseEvent(type, { bubbles: true, clientY: y, button: 0 });
      Object.defineProperty(event, "pointerId", { value: 1 }); handle.dispatchEvent(event);
    });
    await pointer("pointerdown", 200); await pointer("pointermove", 175); await pointer("pointerup", 175);
    await act(async () => handle.click()); expect(handle).toHaveAttribute("aria-expanded", "true");
    await pointer("pointerdown", 175); await pointer("pointermove", 205); await pointer("pointerup", 205);
    await act(async () => handle.click()); expect(handle).toHaveAttribute("aria-expanded", "false");
  });
  it("adds from two to four without remounting existing native receivers", async () => {
    const view = setup(); view.api.inspect.mockImplementation(async id => ({ ...LIVE, channelId: id, name: `채널 ${id}` }));
    await view.render(); await input("one"); await click('[aria-label="방송 추가"]'); await input("two", 2); await inspect(); await click(".soop-start");
    const original = [...container.querySelectorAll('[data-testid="native-live-player"]')];
    expect(original).toHaveLength(2); expect(playerLifecycle.open).toHaveBeenCalledTimes(2);
    await openDrawer(); await click('[aria-label="방송 추가"]'); await input("three", 3); await click('[aria-label="방송 추가"]'); await input("four", 4); await inspect();
    expect(container.querySelector('[aria-label="방송 추가"]')).toBeDisabled();
    await click(".soop-change");
    const changed = [...container.querySelectorAll('[data-testid="native-live-player"]')];
    expect(changed).toHaveLength(4); expect(changed[0]).toBe(original[0]); expect(changed[1]).toBe(original[1]);
    expect(playerLifecycle.open).toHaveBeenCalledTimes(4); expect(playerLifecycle.close).not.toHaveBeenCalled();
    await openDrawer(); await click('[aria-label="방송 보기 닫기"]');
    expect(playerLifecycle.open).toHaveBeenCalledTimes(4); expect(playerLifecycle.close).not.toHaveBeenCalled();
  });
  it("selected viewing retains every chat/player lease and names the selected channel", async () => {
    const view = setup(); view.api.inspect.mockImplementation(async id => ({ ...LIVE, channelId: id, name: `채널 ${id}` }));
    await view.render(); await input("one"); await click('[aria-label="방송 추가"]'); await input("two", 2); await inspect(); await click(".soop-start");
    const original = [...container.querySelectorAll('[data-testid="native-live-player"]')];
    await openDrawer(); await inspect();
    const select = container.querySelector<HTMLSelectElement>('[aria-label="볼 영상"]')!;
    const segment = container.querySelector('.soop-mode')!;
    await act(async () => segment.querySelectorAll<HTMLButtonElement>('button')[1]!.click());
    expect(select).not.toBeDisabled(); expect(select).toHaveTextContent("채널 two");
    await act(async () => { select.value = "two"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    await click(".soop-change");
    const players = [...container.querySelectorAll('[data-testid="native-live-player"]')];
    expect(players[0]).toBe(original[0]); expect(players[1]).toBe(original[1]);
    expect(players[0]).toHaveAttribute("data-video-visible", "false"); expect(players[1]).toHaveAttribute("data-video-visible", "true");
    expect(playerLifecycle.close).not.toHaveBeenCalled(); expect(playerLifecycle.open).toHaveBeenCalledTimes(2);
  });
  it("stops, edits and restarts without leaving inputs or native session identity locked", async () => {
    const view = setup(); view.api.inspect.mockImplementation(async id => ({ ...LIVE, channelId: id, name: id }));
    await view.render(); await input("one"); await inspect(); await click(".soop-start"); await click(".soop-stop");
    expect(container.querySelector('[data-testid="native-live-player"]')).toBeNull();
    await input("two"); await inspect(); await click(".soop-start");
    expect(container.querySelector('[data-testid="native-live-player"]')).toHaveAttribute("data-external-id", "two");
    expect(playerLifecycle.close).toHaveBeenCalledTimes(1); expect(playerLifecycle.open).toHaveBeenCalledTimes(2);
  });
  it("deduplicates repeated channels and excludes ended broadcasts with an explicit notice", async () => {
    const onNotice = vi.fn(), view = setup({ onNotice });
    view.api.inspect.mockImplementation(async id => ({ ...LIVE, channelId: id, status: id === "offline" ? "offline" : "live" }));
    await view.render(); await input("one"); await click('[aria-label="방송 추가"]'); await input("one", 2); await click('[aria-label="방송 추가"]'); await input("offline", 3); await inspect(); await click(".soop-start");
    expect(playerLifecycle.open).toHaveBeenCalledTimes(1); expect(onNotice).toHaveBeenCalledWith("종료된 방송을 제외하고 시청합니다.");
  });
  it("disables hidden player shortcuts but preserves native receivers and privacy", async () => {
    const view = setup(); await view.render(); await input("aflol"); await inspect(); await click(".soop-start");
    const player = container.querySelector('[data-testid="native-live-player"]');
    await view.render({ active: false }); expect(player).toHaveAttribute("data-active", "false");
    await view.render({ active: true, view: "downloads" }); expect(player).toHaveAttribute("data-active", "false");
    await view.render({ view: "live", privacyMode: true }); expect(player).toHaveAttribute("data-private", "true");
    expect(playerLifecycle.close).not.toHaveBeenCalled();
  });
  it("declares a bounded 960x640 grid/overlay model with fixed toggles (not a desktop pixel measurement)", async () => {
    const view = setup(); container.style.width = "960px"; container.style.height = "640px"; await view.render(); await openDrawer();
    expect(container.querySelector('.soop-workspace')).toHaveClass("is-live-view");
    expect(workspaceStyles).toContain('.soop-workspace.is-live-view{padding:0;gap:0;overflow:hidden}');
    expect(workspaceStyles).toContain('inset:0 0 24px;min-height:0;min-width:0;overflow:hidden');
    expect(workspaceStyles).toContain('grid-template-columns:repeat(2,minmax(0,1fr));grid-template-rows:repeat(2,minmax(0,1fr))');
    expect(workspaceStyles).toContain('max-height:calc(100% - 40px)');
    expect(workspaceStyles).toContain('.soop-connection-scroll{min-height:0;overflow:auto');
    expect(workspaceStyles).toContain('grid-auto-rows:64px;max-height:280px');
    expect(workspaceStyles).toContain('width:76px;height:22px');
    expect(workspaceStyles).toContain('cursor:pointer;touch-action:none');
    const select = container.querySelector('[aria-label="볼 영상"]'), buttons = [...container.querySelectorAll('.soop-mode button')];
    await act(async () => (buttons[1] as HTMLButtonElement).click());
    expect(container.querySelector('[aria-label="볼 영상"]')).toBe(select);
    expect([...container.querySelectorAll('.soop-mode button')]).toEqual(buttons);
    // Four cells each have a finite 477x305 outer slot before each player's own responsive split.
    expect((960 - 6) / 2).toBe(477); expect((640 - 24 - 6) / 2).toBe(305);
  });
});
