import { describe, expect, it, vi } from "vitest";
import type {
  ApiResult,
  DanbooruDownloadRecord,
  DanbooruPost,
} from "./contracts";
import { createDanbooruApi, type DanbooruApi } from "./featureClients";

const post: DanbooruPost = {
  id: 123,
  createdAt: "2026-09-10T00:00:00Z",
  rating: "g",
  score: 10,
  favoriteCount: 2,
  imageWidth: 1200,
  imageHeight: 800,
  fileExt: "jpg",
  fileSize: 2048,
  artists: ["sample_artist"],
  copyrights: [],
  characters: [],
  tags: ["blue_sky"],
  hasChildren: false,
};

const createTransport = (runtime: DanbooruApi["runtime"]) => {
  const calls: unknown[][] = [];
  const downloads: DanbooruDownloadRecord[] = [];
  const api: DanbooruApi = {
    runtime,
    async danbooruSearch(request) {
      calls.push([this, "search", request]);
      return { ok: true, data: { items: [post], page: request.page, hasMore: false } };
    },
    async danbooruRandom() {
      calls.push([this, "random"]);
      return { ok: true, data: post };
    },
    async danbooruRelated(request) {
      calls.push([this, "related", request]);
      return { ok: true, data: { siblings: [], children: [], pools: [] } };
    },
    async danbooruAutocomplete(query, limit) {
      calls.push([this, "autocomplete", query, limit]);
      return { ok: true, data: [] };
    },
    async danbooruDownload(postId) {
      calls.push([this, "download", postId]);
      const record = { post, fileName: `${postId}.jpg`, downloadedAt: post.createdAt, bytes: post.fileSize };
      downloads.push(record);
      return { ok: true, data: record };
    },
    async danbooruDownloadsList(request) {
      calls.push([this, "downloads", request]);
      return { ok: true, data: { items: [...downloads], page: request.page, total: downloads.length, totalPages: 1 } };
    },
  };
  const client = {
    ...api,
    get searchSubmit(): never { throw new Error("Hitomi search must not be accessed"); },
    get settingsGet(): never { throw new Error("App settings must not be accessed"); },
    get on(): never { throw new Error("Shared event subscriptions must not be accessed"); },
  };
  return { client, calls };
};

describe("Danbooru feature API", () => {
  it.each(["tauri", "browser-mock"] as const)("limits the %s surface and preserves transport ownership", async (runtime) => {
    const { client, calls } = createTransport(runtime);
    const api = createDanbooruApi(client);
    expect(api.runtime).toBe(runtime);
    expect(Object.keys(api).sort()).toEqual([
      "danbooruAutocomplete", "danbooruDownload", "danbooruDownloadsList",
      "danbooruRandom", "danbooruRelated", "danbooruSearch", "runtime",
    ]);
    expect(calls).toEqual([]);

    const { danbooruSearch, danbooruRandom, danbooruRelated, danbooruAutocomplete, danbooruDownload, danbooruDownloadsList } = api;
    const searchRequest = { tags: "blue_sky", page: 2, pageSize: 30 };
    const relatedRequest = { postId: post.id, hasChildren: false };
    const downloadsRequest = { query: "", page: 1, pageSize: 30 };
    expect(await danbooruSearch(searchRequest)).toEqual({ ok: true, data: { items: [post], page: 2, hasMore: false } });
    expect(await danbooruRandom()).toEqual({ ok: true, data: post });
    await danbooruRelated(relatedRequest);
    await danbooruAutocomplete("blue", 8);
    const saved = await danbooruDownload(post.id);
    expect(saved.ok).toBe(true);
    expect(await danbooruDownloadsList(downloadsRequest)).toEqual({
      ok: true,
      data: { items: saved.ok ? [saved.data] : [], page: 1, total: 1, totalPages: 1 },
    });
    expect(calls).toEqual([
      [client, "search", searchRequest],
      [client, "random"],
      [client, "related", relatedRequest],
      [client, "autocomplete", "blue", 8],
      [client, "download", post.id],
      [client, "downloads", downloadsRequest],
    ]);
  });

  it("dispatches through the current transport method and preserves failures", async () => {
    const { client, calls } = createTransport("browser-mock");
    const api = createDanbooruApi(client);
    const failure: ApiResult<DanbooruPost> = {
      ok: false,
      error: { code: "temporarilyUnavailable", message: "Try again", retryable: true },
    };
    const replacement = vi.spyOn(client, "danbooruRandom").mockResolvedValue(failure);
    expect(await api.danbooruRandom()).toBe(failure);
    expect(replacement).toHaveBeenCalledOnce();
    expect(replacement.mock.contexts).toEqual([client]);
    expect(calls).toEqual([]);
    const error = new Error("Transport disconnected");
    replacement.mockRejectedValueOnce(error);
    await expect(api.danbooruRandom()).rejects.toBe(error);
  });
});
