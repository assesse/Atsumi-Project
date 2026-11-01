import { beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
import { isSupportedLinkRequest, subscribeSupportedLinks } from "./supportedLinkNavigation";
const request = { id: "97ea02a0-b626-40c2-ab6b-5cf601e81ae2", source: "pixiv", kind: "artist", targetId: "4912", url: "https://www.pixiv.net/users/4912" };

describe("native supported result navigation", () => {
  beforeEach(() => { vi.clearAllMocks(); });
  it("accepts only canonical target data that matches the exact platform and kind", () => {
    expect(isSupportedLinkRequest(request)).toBe(true);
    expect(isSupportedLinkRequest({ ...request, kind: "artwork", url: "https://www.pixiv.net/artworks/4912" })).toBe(true);
    expect(isSupportedLinkRequest({ ...request, source: "danbooru", kind: "post", url: "https://danbooru.donmai.us/posts/4912" })).toBe(true);
    for (const update of [{ source: "x" }, { targetId: "0" }, { targetId: "04912" }, { kind: "post" }, { url: "https://www.pixiv.net.evil.example/users/4912" }, { url: "https://www.google.com/goto?url=https://www.pixiv.net/users/4912" }, { url: "https://www.pixiv.net/users/4913" }, { url: "https://user@www.pixiv.net/users/4912" }, { url: "https://www.pixiv.net/users/4912#target" }, { arbitrary: "value" }]) expect(isSupportedLinkRequest({ ...request, ...update })).toBe(false);
  });
  it.each([
    ["pinterest", "pin", "621004236103700020", "https://www.pinterest.com/pin/621004236103700020/"],
    ["hitomi", "gallery", "424242", "https://hitomi.la/galleries/424242.html"],
    ["youtube", "video", "abcdefghijk", "https://www.youtube.com/watch?v=abcdefghijk"],
    ["youtube", "channel", "@example", "https://www.youtube.com/@example"],
    ["youtube", "playlist", "PL1234567890", "https://www.youtube.com/playlist?list=PL1234567890"],
    ["soop", "video", "123456", "https://vod.sooplive.com/player/123456"],
    ["soop", "channel", "example_1", "https://play.sooplive.com/example_1"],
    ["chzzk", "channel", "0123456789abcdef0123456789abcdef", "https://chzzk.naver.com/live/0123456789abcdef0123456789abcdef"],
    ["x", "profile", "/example", "https://x.com/example"],
    ["x", "post", "/example/status/12345/photo/1", "https://x.com/example/status/12345/photo/1"],
    ["x", "post", "/i/web/status/12345", "https://x.com/i/web/status/12345"],
  ])("validates the canonical %s %s delivery without widening external targets", (source, kind, targetId, url) => {
    const value = { ...request, source, kind, targetId, url };
    expect(isSupportedLinkRequest(value)).toBe(true);
    expect(isSupportedLinkRequest({ ...value, url: `${url}#forged` })).toBe(false);
    expect(isSupportedLinkRequest({ ...value, targetId: `${targetId}/extra` })).toBe(false);
  });
  it("recovers a pending destination after subscribing and handles duplicate event delivery only once", async () => {
    let event!: (event: { payload: unknown }) => void;
    const unlisten = vi.fn();
    mocks.listen.mockImplementation(async (_name, callback) => { event = callback; return unlisten; });
    mocks.invoke.mockImplementation(async name => name === "media_search_navigation_pending" ? request : undefined);
    const accept = vi.fn(), stop = await subscribeSupportedLinks(accept);
    await Promise.resolve();
    expect(mocks.listen).toHaveBeenCalledWith("atsumi:open-supported-link", expect.any(Function));
    expect(accept).toHaveBeenCalledOnce(); expect(accept).toHaveBeenCalledWith(request);
    event({ payload: request }); expect(accept).toHaveBeenCalledOnce();
    expect(mocks.invoke).toHaveBeenCalledWith("media_search_navigation_ack", { id: request.id });
    stop(); event({ payload: { ...request, id: "97ea02a0-b626-40c2-ab6b-5cf601e81ae3" } });
    expect(accept).toHaveBeenCalledOnce(); expect(unlisten).toHaveBeenCalledOnce();
  });
  it("does not acknowledge malformed destinations", async () => {
    let event!: (event: { payload: unknown }) => void;
    mocks.listen.mockImplementation(async (_name, callback) => { event = callback; return vi.fn(); });
    mocks.invoke.mockResolvedValue(null);
    const accept = vi.fn(), stop = await subscribeSupportedLinks(accept);
    event({ payload: { ...request, url: "https://www.google.com/goto?url=opaque" } });
    expect(accept).not.toHaveBeenCalled(); expect(mocks.invoke).not.toHaveBeenCalledWith("media_search_navigation_ack", expect.anything()); stop();
  });
  it("does not let a delayed startup snapshot override a newer clicked destination", async () => {
    let event!: (event: { payload: unknown }) => void, resolve!: (value: unknown) => void;
    mocks.listen.mockImplementation(async (_name, callback) => { event = callback; return vi.fn(); });
    mocks.invoke.mockImplementation(name => name === "media_search_navigation_pending" ? new Promise(done => { resolve = done; }) : Promise.resolve());
    const accept = vi.fn(), stop = await subscribeSupportedLinks(accept);
    const newer = { ...request, id: "97ea02a0-b626-40c2-ab6b-5cf601e81ae3", targetId: "123", url: "https://www.pixiv.net/users/123" };
    event({ payload: newer }); resolve(request); await Promise.resolve();
    expect(accept).toHaveBeenCalledExactlyOnceWith(newer); stop();
  });
});
