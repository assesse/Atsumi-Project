// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import source from "../../../src-tauri/src/streaming/browser_quality.js?raw";
const moduleName = "node:vm";
const { runInNewContext } = await import(moduleName);
const CHANNEL = "b3e262a2795f17734c149afc738ad250";
type Track = { id: string; label: string; width: number; height: number; selected: boolean; videoBitrate?: number };
const track = (height: number, selected = false): Track => ({ id: `${height}`, label: `${height}p`, width: height * 16 / 9, height, selected });
function fixture(tracks: Track[], options: { active?: boolean; deny?: string; filter?: (t: Track) => boolean; noSettle?: string; url?: string; ambiguous?: boolean; autoRecording?: boolean } = {}) {
  const hooks = new Map<string, () => void>();
  const host: Record<string, unknown> = { isConnected: true };
  const current = tracks.find(t => t.selected) ?? tracks[0]!;
  const video = { isConnected: true, ended: false, paused: false, seeking: false, readyState: 4,
    videoWidth: current.width, videoHeight: current.height, clientWidth: 1280, clientHeight: 720,
    closest: () => host };
  const select = vi.fn(async (id: string) => {
    tracks.forEach(t => { t.selected = t.id === id; });
    if (id === options.noSettle) return;
    const chosen = tracks.find(t => t.id === id)!;
    video.videoWidth = chosen.width; video.videoHeight = chosen.height;
  });
  const dispatch = vi.fn((_event: string, { track: t }: { track: Track }) => t.id !== options.deny);
  const pane = { selectVideoTrack: select, $dispatch: dispatch, filter: options.filter ?? (() => true) };
  const player = { querySelector: () => pane, videoTracks: tracks, shadowRoot: { contains: (v: unknown) => v === video }, srcObject: {} };
  host.__reactFiber$fixture = { memoizedState: { memoizedState: player, next: options.ambiguous ? {
    memoizedState: { ...player, querySelector: () => ({ ...pane }) },
  } : null } };
  const storage = new Map<string, string>();
  let active = options.active ?? false;
  const page: Record<string, unknown> = {
    location: new URL(options.url ?? `https://chzzk.naver.com/live/${CHANNEL}`),
    localStorage: { setItem: (key: string, value: string) => storage.set(key, value) },
    __atsumiEncodedCapture: { getStatus: () => ({ active }) },
    __atsumiAutoReceiver: { isPreparingRecording: () => options.autoRecording === true },
    addEventListener: (name: string, fn: () => void) => hooks.set(name, fn),
  };
  page.top = page;
  const document = { querySelectorAll: () => [video] };
  runInNewContext(source, { window: page, document, Date, setTimeout, clearTimeout, setInterval, clearInterval });
  const api = page.__atsumiQuality as undefined | { prepare(): Promise<boolean>; canStart(): boolean; useStandardQuality(): void; getStatus(): { status: string; height: number | null } };
  return { api, select, dispatch, storage, video, tracks, setActive: (v: boolean) => { active = v; }, hide: () => hooks.get("pagehide")?.() };
}
beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(1_000_000); });
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); });
describe("official live quality preference", () => {
  it("shares the live 1080p preference with a cold grid-free automatic receiver", () => {
    const f = fixture([track(480, true)], { autoRecording: true });
    expect(JSON.parse(f.storage.get("live-player-video-track")!)).toEqual({ label: "1080p", width: 1920, height: 1080 });
  });
  it("does not reopen a grid prompt after the official standard-quality choice", async () => {
    const f = fixture([track(480, true), track(1080)], { autoRecording: true, deny: "1080" });
    f.api!.useStandardQuality(); f.select.mockClear(); f.dispatch.mockClear();
    await vi.advanceTimersByTimeAsync(90_000);
    expect(f.select).not.toHaveBeenCalled(); expect(f.dispatch).not.toHaveBeenCalled();
    expect(f.api!.canStart()).toBe(true); expect(await f.api!.prepare()).toBe(true);
    expect(f.api!.getStatus()).toEqual({ status: "native", height: 480 });
  });
  it("prefers 1080p over both lower quality and 2160p without fetching another stream", async () => {
    const f = fixture([track(480, true), track(2160), track(1080)]);
    await vi.advanceTimersByTimeAsync(1000);
    expect(f.select).toHaveBeenCalledExactlyOnceWith("1080");
    expect(f.api?.getStatus()).toEqual({ status: "ready", height: 1080 });
    expect(JSON.parse(f.storage.get("live-player-video-track")!)).toEqual({ label: "1080p", width: 1920, height: 1080 });
  });
  it("falls back to the highest provided quality, excluding automatic tracks", async () => {
    const abr = { ...track(2160), id: "auto", label: "ABR" };
    const f = fixture([track(360, true), track(720), abr]);
    await vi.advanceTimersByTimeAsync(1000);
    expect(f.select).toHaveBeenCalledExactlyOnceWith("720");
  });
  it("respects the native availability filter", async () => {
    const f = fixture([track(480, true), track(720), track(1080)], { filter: t => t.height < 1080 });
    await vi.advanceTimersByTimeAsync(1000);
    expect(f.select).toHaveBeenCalledExactlyOnceWith("720");
  });
  it("does not bypass a rejected native quality event and selects the next option", async () => {
    const f = fixture([track(480, true), track(720), track(1080)], { deny: "1080" });
    const ready = f.api!.prepare();
    await vi.advanceTimersByTimeAsync(2400);
    expect(await ready).toBe(true);
    expect(f.select).toHaveBeenCalledExactlyOnceWith("720");
    expect(f.dispatch).toHaveBeenCalledWith("change", { track: f.tracks[2] });
  });
  it("requires decoded resolution, not only selectedIndex, before arming recording", async () => {
    const f = fixture([track(480, true), track(720), track(1080)], { noSettle: "1080" });
    expect(f.api?.canStart()).toBe(false);
    const ready = f.api!.prepare();
    await vi.advanceTimersByTimeAsync(6200);
    expect(await ready).toBe(true);
    expect(f.select.mock.calls.map(c => c[0])).toEqual(["1080", "720"]);
    expect(f.api?.getStatus().height).toBe(720);
  });
  it("never upgrades an active original-stream recording", async () => {
    const f = fixture([track(480, true), track(1080)], { active: true });
    await vi.advanceTimersByTimeAsync(20_000);
    expect(f.select).not.toHaveBeenCalled();
    f.setActive(false);
    await vi.advanceTimersByTimeAsync(2000);
    expect(f.select).toHaveBeenCalledExactlyOnceWith("1080");
  });
  it("leaves playback unchanged when the upstream player reference is ambiguous", async () => {
    const f = fixture([track(480, true), track(1080)], { ambiguous: true });
    expect(await f.api?.prepare()).toBe(true);
    expect(f.select).not.toHaveBeenCalled();
  });
  it("does not enter login, chat-only or other-origin pages", () => {
    for (const url of ["https://nid.naver.com/login", `https://chzzk.naver.com/live/${CHANNEL}/chat`, "https://example.com/"]) {
      const f = fixture([track(480, true), track(1080)], { url });
      expect(f.api).toBeUndefined(); expect(f.storage.size).toBe(0);
    }
  });
  it("stops selection after page teardown", async () => {
    const f = fixture([track(480, true), track(1080)], { active: true });
    f.hide(); f.setActive(false);
    await vi.advanceTimersByTimeAsync(20_000);
    expect(f.select).not.toHaveBeenCalled();
  });
});
