import { afterEach, describe, expect, it, vi } from "vitest";
import source from "../../../public/original-player/runtime.js?raw";
import metadataSource from "../../../public/original-player/recording-metadata.js?raw";

it("accepts only bounded inline saved profile rasters, never remote URLs or active markup", () => {
  const fallback = "/assets/default_profile_dark.png";
  const declaration = metadataSource.slice(metadataSource.indexOf("export function savedMetadata"), metadataSource.indexOf("export function mountRecordingMetadata")).replace("export function", "function");
  const validate = new Function("fixture", declaration + ";return savedMetadata;")({ profileImage: fallback });
  const raster = "data:image/png;base64,iVBORw0KGgo=";
  expect(validate({ title: "저장 방송", profileImage: raster }).profileImage).toBe(raster);
  for (const profileImage of [undefined, null, "https://ssl.pstatic.net/profile.png", "file:///private", "data:image/svg+xml;base64,PHN2Zz4=", "data:image/png;base64," + "A".repeat(1_500_000), "data:image/png;base64,AAAA\" onerror=bad"]) {
    expect(validate({ profileImage }).profileImage).toBe(fallback);
  }
});

const nonce = "a".repeat(32);
const mediaUrl = `http://atsumi-replay.localhost/${"b".repeat(32)}`;
const template = '<div id="player_layout"><pzp-pc-layout></pzp-pc-layout><div class="pzp-pc__progress-slider"></div></div>';
const transformed = source
  .replace("import originalVodTemplate from './accepted-vod-template.js';", `const originalVodTemplate=${JSON.stringify(template)};`)
  .replace("const sdk=await import('./player-vendor-BYg0wCyN.js');", "const sdk=providedSdk;")
  .replace("const metadata=await import('./recording-metadata.js');", "const metadata=providedMetadata;");

async function fixture(options: { top?: boolean; origin?: string; hash?: string } = {}) {
  const document = window.document.implementation.createHTMLDocument("isolated-runtime-test");
  const listeners = new Map<string, EventListener>();
  const playerListeners = new Map<string, () => void>();
  const components = new Map<string, Record<string, unknown>>();
  const parent = { postMessage: vi.fn() };
  const frame: { top?: unknown; origin: string; addEventListener: (name: string, listener: EventListener) => void; localStorage?: Storage; sessionStorage?: Storage } = {
    top: parent,
    origin: options.origin ?? "null",
    addEventListener: (name, listener) => { listeners.set(name, listener); },
  };
  if (options.top) frame.top = frame;
  const player = {
    duration: 8, currentTime: 0, paused: true, muted: false,
    language: "", shadowRoot: document.body,
    addEventListener: vi.fn((name: string, listener: () => void) => playerListeners.set(name, listener)),
    querySelector: vi.fn((name: string) => {
      if (!components.has(name)) components.set(name, {});
      return components.get(name)!;
    }),
    pause: vi.fn(() => { player.paused = true; playerListeners.get("pause")?.(); }),
    play: vi.fn(async () => { player.paused = false; playerListeners.get("play")?.(); }),
    srcObject: null as unknown,
  };
  let selectedSource: unknown = null;
  Object.defineProperty(player, "srcObject", {
    get: () => selectedSource,
    set: value => {
      selectedSource = value;
      // The adapter must suppress callbacks emitted synchronously by teardown.
      if (value === null) { playerListeners.get("error")?.(); playerListeners.get("pause")?.(); }
    },
  });
  const upgrade = vi.fn(() => player);
  const provider = vi.fn();
  class DataProvider { constructor(value: unknown) { provider(value); } }
  const sdk = { S: vi.fn(() => ({ default: { upgrade }, DataProvider })) };
  const interval = vi.fn(() => 1), clear = vi.fn();
  const recordingHeader = { update: vi.fn(), dispose: vi.fn() };
  const metadata = { mountRecordingMetadata: vi.fn(() => recordingHeader) };
  const execute = new Function("window", "document", "location", "parent", "providedSdk", "providedMetadata", "setInterval", "clearInterval", `return (async()=>{${transformed}\n})();`);
  const boot = execute(frame, document, { hash: options.hash ?? `#${nonce}` }, parent, sdk, metadata, interval, clear) as Promise<void>;
  const send = (type: string, data?: unknown, patch: Record<string, unknown> = {}) => listeners.get("message")?.({
    source: parent, origin: "http://tauri.localhost", data: { channel: "atsumi-replay-player-v1", nonce, type, data }, ...patch,
  } as unknown as Event);
  return { boot, send, frame, parent, player, playerListeners, components, provider, upgrade, sdk, clear, document, metadata, recordingHeader };
}

afterEach(() => vi.restoreAllMocks());

describe("original player runtime isolation and lifecycle", () => {
  it("updates saved broadcast metadata without reloading or pausing the video", async () => {
    const f = await fixture(); await f.boot;
    const recording = { title: "저장된 방송 제목", recordedAt: 1789272000000 };
    f.send("init", { url: mediaUrl, duration: 8, recording });
    expect(f.metadata.mountRecordingMetadata).toHaveBeenCalledWith(f.player, recording);
    f.send("metadata", { ...recording, title: "수정된 제목" });
    expect(f.recordingHeader.update).toHaveBeenCalledWith({ ...recording, title: "수정된 제목" });
    expect(f.upgrade).toHaveBeenCalledTimes(1);
    expect(f.player.pause).not.toHaveBeenCalled();
    f.send("dispose");
    expect(f.recordingHeader.dispose).toHaveBeenCalledOnce();
  });
  it.each([{ top: true }, { origin: "http://tauri.localhost" }, { hash: "#invalid" }])("refuses non-opaque/unscoped hosts before SDK initialization: %j", async options => {
    const f = await fixture(options);
    await expect(f.boot).rejects.toThrow("opaque, scoped frame");
    expect(f.sdk.S).not.toHaveBeenCalled();
    expect(f.frame.localStorage).toBeUndefined();
    expect(f.parent.postMessage).not.toHaveBeenCalled();
  });

  it("uses only bounded per-frame memory storage", async () => {
    const f = await fixture(); await f.boot;
    const storage = f.frame.localStorage!;
    storage.setItem("key", "value");
    expect(storage.getItem("key")).toBe("value");
    expect(f.frame.sessionStorage!.getItem("key")).toBeNull();
    storage.setItem("long", "x".repeat(65537));
    expect(storage.getItem("long")).toBeNull();
    storage.setItem("k".repeat(257), "x");
    expect(storage.length).toBe(1);
    for (let i = 0; i < 110; i++) storage.setItem(`entry-${i}`, "bounded");
    expect(storage.length).toBe(100);
    const second = await fixture(); await second.boot;
    expect(second.frame.localStorage!.length).toBe(0);
  });

  it("mounts the original SDK from one authorized token without fabricated dimensions", async () => {
    const f = await fixture(); await f.boot;
    f.send("init", { url: mediaUrl, duration: 8, privacy: false });
    expect(f.upgrade).toHaveBeenCalledTimes(1);
    const track = f.provider.mock.calls[0]![0].videoTracks[0];
    expect(track).toMatchObject({ src: mediaUrl, id: "local", duration: 8 });
    expect(track).not.toHaveProperty("width");
    expect(track).not.toHaveProperty("height");
    f.send("init", { url: mediaUrl, duration: 8 });
    expect(f.upgrade).toHaveBeenCalledTimes(1);
  });

  it.each(["https://evil.example/video.mp4", `${mediaUrl}?file=C:/private`, `${mediaUrl}#fragment`, "file:///private.mp4", "http://atsumi-replay.localhost/not-a-token"]) ("does not initialize unauthorized media %s", async url => {
    const f = await fixture(); await f.boot;
    f.send("init", { url, duration: 8 });
    expect(f.upgrade).not.toHaveBeenCalled();
  });

  it("retains initial privacy and suppresses all disposal callbacks", async () => {
    const f = await fixture(); await f.boot;
    f.send("init", { url: mediaUrl, duration: 8, privacy: true });
    f.send("toggle");
    expect(f.player.play).not.toHaveBeenCalled();
    // An original native player control cannot start playback in privacy mode.
    await f.player.play();
    expect(f.player.paused).toBe(true);
    f.parent.postMessage.mockClear();
    f.send("dispose");
    expect(f.player.srcObject).toBeNull();
    expect(f.clear).toHaveBeenCalledWith(1);
    expect(f.parent.postMessage).not.toHaveBeenCalled();
    f.send("privacy", false);
    f.send("toggle");
    expect(f.parent.postMessage).not.toHaveBeenCalled();
    expect(f.player.play).toHaveBeenCalledTimes(1);
  });

  it("rejects a matching nonce from another source or origin", async () => {
    const f = await fixture(); await f.boot;
    f.send("init", { url: mediaUrl, duration: 8 }, { source: {} });
    f.send("init", { url: mediaUrl, duration: 8 }, { origin: "https://chzzk.naver.com" });
    expect(f.upgrade).not.toHaveBeenCalled();
  });

  it("synchronizes original viewmode and inner fullscreen visual properties without click feedback", async () => {
    const f = await fixture(); await f.boot;
    f.send("presentation", { wide: true, fullscreen: true });
    f.send("init", { url: mediaUrl, duration: 8 });
    const wide = f.components.get("pzp-pc-viewmode-button")!;
    const fullscreen = f.components.get("pzp-fullscreen-button")!;
    expect(wide.checked).toBe(true);
    expect(fullscreen.fullscreen).toBe(true);
    expect(f.components.has("pzp-pc-fullscreen-button")).toBe(false);
    const updateWide = vi.fn();
    Object.defineProperty(wide, "checked", { get: () => true, set: updateWide });
    f.parent.postMessage.mockClear();
    f.send("presentation", { wide: true, fullscreen: true });
    expect(updateWide).not.toHaveBeenCalled();
    expect(f.parent.postMessage).not.toHaveBeenCalled();
    f.send("presentation", { wide: "false", fullscreen: false });
    expect(fullscreen.fullscreen).toBe(true);
    f.send("presentation", { wide: false, fullscreen: false });
    expect(updateWide).toHaveBeenCalledWith(false);
    expect(fullscreen.fullscreen).toBe(false);
  });

  it("preserves partial viewer samples and identifies legacy message counts", async () => {
    const f = await fixture(); await f.boot;
    f.send("init", { url: mediaUrl, duration: 8 });
    f.send("metrics", { bucketSeconds: 4, participantCounts: false, chatPaths: [], viewerPaths: [], buckets: [
      { startSeconds: 0, viewerCount: 100, viewerCoverageSeconds: 2, uniqueSenderCount: null, chatCount: 7 },
      { startSeconds: 4, viewerCount: 200, viewerCoverageSeconds: 4, uniqueSenderCount: null, chatCount: 8 },
    ] });
    const marks = f.document.querySelectorAll(".viewer-partial");
    expect(marks).toHaveLength(1);
    expect(marks[0]!.getAttribute("cx")).toBe("250");
    expect(marks[0]!.getAttribute("cy")).toBe("18");
    const slider = f.document.querySelector(".pzp-pc__progress-slider")!;
    vi.spyOn(slider, "getBoundingClientRect").mockReturnValue({ left: 0, width: 1000 } as DOMRect);
    slider.dispatchEvent(new MouseEvent("pointermove", { clientX: 250 }));
    const tip = f.document.querySelector(".atsumi-replay-metric-tip")!;
    expect(tip.textContent).toContain("채팅 수 7개");
    expect(tip.textContent).toContain("평균 시청자 100명 · 일부 기록");
    expect(tip.textContent).not.toContain("채팅 참여자");
    f.send("metrics", null);
    expect(f.document.querySelectorAll(".viewer-partial")).toHaveLength(0);
  });
});
