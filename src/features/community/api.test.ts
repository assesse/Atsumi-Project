import { afterEach, describe, expect, it, vi } from "vitest";
import { communityApi } from "./api";
const native = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: vi.fn(() => true) }));
vi.mock("@tauri-apps/api/core", () => native);
afterEach(() => { vi.clearAllMocks(); vi.unstubAllGlobals(); native.isTauri.mockReturnValue(true); });
describe("community adapter", () => {
  it("uses public native reads separately from explicit identity/write requests", async () => {
    await communityApi.feed(null); await communityApi.work({ source: "hitomi", workId: "123" });
    expect(native.invoke.mock.calls.map(([command]) => command)).toEqual(["community_read", "community_read"]);
    await communityApi.beginWriting({ source: "danbooru", workId: "12" });
    expect(native.invoke).toHaveBeenLastCalledWith("community_write", { request: { kind: "beginWriting", source: "danbooru", workId: "12" } });
    expect(JSON.stringify(native.invoke.mock.calls)).not.toContain("token");
    await communityApi.myReviews();
    expect(native.invoke).toHaveBeenLastCalledWith("community_write", { request: { kind: "myReviews", cursor: null } });
  });
  it("does not create disposable browser identities or store tokens in localStorage", async () => {
    native.isTauri.mockReturnValue(false);
    const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ items: [], nextCursor: null }) }); vi.stubGlobal("fetch", fetchMock);
    await communityApi.feed(null);
    expect(fetchMock).toHaveBeenCalledTimes(1); expect(fetchMock.mock.calls[0]?.[0]).toContain("/rpc/community_v1_ranked_feed");
    expect(JSON.parse(fetchMock.mock.calls[0]?.[1].body)).toMatchObject({ p_order: "latest" });
    expect(fetchMock.mock.calls[0]?.[1].headers.Authorization).toBeUndefined();
    await expect(communityApi.beginWriting({ source: "hitomi", workId: "123" })).rejects.toThrow("Windows용 Atsumi");
    expect(fetchMock).toHaveBeenCalledTimes(1); expect(native.invoke).not.toHaveBeenCalled();
  });
  it("passes ranking and its opaque pagination cursor to native and browser readers", async () => {
    const cursor = { score: "3.70000000", reviewCount: 25, source: "hitomi" as const, workId: "123", order: "popular" as const, scope: "hitomi" as const };
    await communityApi.feed("hitomi", cursor, "popular");
    expect(native.invoke).toHaveBeenLastCalledWith("community_read", { request: { kind: "feed", source: "hitomi", cursor, order: "popular" } });
    native.isTauri.mockReturnValue(false);
    const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ items: [], nextCursor: null }) }); vi.stubGlobal("fetch", fetchMock);
    await communityApi.feed("danbooru", null, "worst");
    expect(JSON.parse(fetchMock.mock.calls[0]?.[1].body)).toEqual({ p_source: "danbooru", p_cursor: null, p_order: "worst" });
  });
});
