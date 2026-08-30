import { describe, expect, it, vi } from "vitest";
import type { ApiResult, GalleryPage } from "../api/contracts";
import { galleryId } from "../core/types";
import { ExplorePageSession } from "./explorePageSession";

const page = (pageNumber: number, totalPages = 20): GalleryPage => ({
  page: pageNumber,
  totalPages,
  items: [{
    id: galleryId(pageNumber),
    title: `Page ${pageNumber}`,
    artist: "artist",
    pages: 1,
    language: "korean",
    tags: [],
    series: [],
    characters: [],
    publishedRank: 20260820,
    popularity: 0,
    thumbnailWidth: 512,
    thumbnailHeight: 512,
  }],
});

const flush = async () => {
  await Promise.resolve();
  await Promise.resolve();
};

describe("ExplorePageSession", () => {
  it("loads page 3 once, then prefetches page 4 before page 2", async () => {
    const calls: number[] = [];
    const fetchPage = vi.fn(async (_queryId: string, pageNumber: number): Promise<ApiResult<GalleryPage>> => {
      calls.push(pageNumber);
      return { ok: true, data: page(pageNumber) };
    });
    const session = new ExplorePageSession({ fetchPage });
    session.start("query-a", page(1));

    const third = await session.open(3);
    await flush();

    expect(third.status).toBe("ready");
    expect(calls.slice(0, 3)).toEqual([3, 4, 2]);
    expect(fetchPage.mock.calls.filter(([, value]) => value === 3)).toHaveLength(1);
    expect(fetchPage.mock.calls.filter(([, value]) => value === 4)).toHaveLength(1);
    expect(fetchPage.mock.calls.filter(([, value]) => value === 2)).toHaveLength(1);
  });

  it("reuses a prefetched page and restores page-specific scroll without another call", async () => {
    const fetchPage = vi.fn(async (_queryId: string, pageNumber: number): Promise<ApiResult<GalleryPage>> => ({ ok: true, data: page(pageNumber) }));
    const session = new ExplorePageSession({ fetchPage });
    session.start("query-a", page(1));
    await session.open(3);
    await flush();
    session.recordScroll(3, 417);

    const fourth = await session.open(4);
    await flush();
    const callsAfterFourth = fetchPage.mock.calls.length;
    session.recordScroll(4, 88);
    const thirdAgain = await session.open(3);

    expect(fourth.status).toBe("ready");
    expect(fetchPage.mock.calls.filter(([, value]) => value === 4)).toHaveLength(1);
    expect(fetchPage.mock.calls.length).toBe(callsAfterFourth);
    expect(thirdAgain).toMatchObject({ status: "ready", source: "cache", scrollTop: 417 });
    expect(fetchPage.mock.calls.filter(([, value]) => value === 3)).toHaveLength(1);
  });

  it("coalesces a prefetch promoted to foreground", async () => {
    const completions = new Map<number, (result: ApiResult<GalleryPage>) => void>();
    const fetchPage = vi.fn((_queryId: string, pageNumber: number) => new Promise<ApiResult<GalleryPage>>((resolve) => {
      completions.set(pageNumber, resolve);
    }));
    const promotePage = vi.fn();
    const session = new ExplorePageSession({ fetchPage, promotePage });
    session.start("query-a", page(3));
    session.prefetchAdjacent();
    await flush();
    const opening = session.open(4);

    expect(fetchPage).toHaveBeenCalledTimes(2);
    expect(fetchPage.mock.calls[0]?.[1]).toBe(4);
    expect(promotePage).toHaveBeenCalledWith("query-a", 4);
    completions.get(4)?.({ ok: true, data: page(4) });
    await opening;
    completions.get(2)?.({ ok: true, data: page(2) });
    await flush();
    expect(fetchPage.mock.calls.filter(([, value]) => value === 4)).toHaveLength(1);
  });

  it("does not prefetch adjacent pages after a foreground failure", async () => {
    const fetchPage = vi.fn(async (): Promise<ApiResult<GalleryPage>> => ({
      ok: false,
      error: { code: "SOURCE_TIMEOUT", message: "timeout", retryable: true, action: "retry" },
    }));
    const session = new ExplorePageSession({ fetchPage });
    session.start("query-a", page(1));

    const result = await session.open(3);

    expect(result.status).toBe("failed");
    expect(fetchPage).toHaveBeenCalledTimes(1);
  });

  it("retries a cached prefetch failure only when it becomes a direct foreground request", async () => {
    const transient = { code: "SOURCE_TIMEOUT", message: "timeout", retryable: true, action: "retry" } as const;
    const fetchPage = vi.fn(async (_queryId: string, pageNumber: number): Promise<ApiResult<GalleryPage>> => {
      if (pageNumber === 4 && fetchPage.mock.calls.filter(([, value]) => value === 4).length === 1) {
        return { ok: false, error: transient };
      }
      return { ok: true, data: page(pageNumber) };
    });
    const session = new ExplorePageSession({ fetchPage });
    session.start("query-a", page(3));
    session.prefetchAdjacent();
    await flush();

    session.prefetchAdjacent();
    await flush();
    expect(fetchPage.mock.calls.filter(([, value]) => value === 4)).toHaveLength(1);

    await expect(session.open(4)).resolves.toMatchObject({ status: "ready", source: "network" });
    expect(fetchPage.mock.calls.filter(([, value]) => value === 4)).toHaveLength(2);
  });

  it("does not auto-loop a permanent prefetch failure", async () => {
    const fetchPage = vi.fn(async (_queryId: string, pageNumber: number): Promise<ApiResult<GalleryPage>> => (
      pageNumber === 4
        ? { ok: false, error: { code: "SOURCE_NOT_FOUND", message: "gone", retryable: false, action: "none" } }
        : { ok: true, data: page(pageNumber) }
    ));
    const session = new ExplorePageSession({ fetchPage });
    session.start("query-a", page(3));
    session.prefetchAdjacent();
    await flush();
    session.prefetchAdjacent();
    await flush();

    expect(fetchPage.mock.calls.filter(([, value]) => value === 4)).toHaveLength(1);
  });

  it("ignores an old query response after a new search starts", async () => {
    let complete: ((result: ApiResult<GalleryPage>) => void) | undefined;
    const fetchPage = vi.fn((_queryId: string, _pageNumber: number, _requestId: string) =>
      new Promise<ApiResult<GalleryPage>>((resolve) => { complete = resolve; }));
    const cancelPage = vi.fn();
    const session = new ExplorePageSession({ fetchPage, cancelPage });
    session.start("query-a", page(1));
    const old = session.open(2);
    await flush();
    const requestId = fetchPage.mock.calls[0]?.[2];
    session.start("query-b", page(1));
    expect(cancelPage).toHaveBeenCalledWith(requestId);
    complete?.({ ok: true, data: page(2) });

    await expect(old).resolves.toEqual({ status: "stale" });
    expect(session.cachedPageNumbers()).toEqual([1]);
  });

  it("keeps only the current plus two adjacent settled pages across twenty pages", async () => {
    const released: number[] = [];
    const session = new ExplorePageSession({
      fetchPage: async (_queryId, pageNumber) => ({ ok: true, data: page(pageNumber) }),
      warmPage: (loaded) => () => { released.push(loaded.page); },
    });
    session.start("query-a", page(1));

    for (let pageNumber = 2; pageNumber <= 20; pageNumber += 1) {
      await session.open(pageNumber);
      await flush();
      expect(session.cachedPageNumbers().length).toBeLessThanOrEqual(5);
    }

    expect(session.cachedPageNumbers()).toEqual([18, 19, 20]);
    expect(released.length).toBeGreaterThan(0);
  });

  it("parks without discarding cached pages or scroll and makes cancelled work stale", async () => {
    const pending = new Map<number, (result: ApiResult<GalleryPage>) => void>();
    const fetchPage = vi.fn((_queryId: string, pageNumber: number, _requestId: string) => {
      if (pageNumber === 5) {
        return new Promise<ApiResult<GalleryPage>>((resolve) => pending.set(pageNumber, resolve));
      }
      return Promise.resolve<ApiResult<GalleryPage>>({ ok: true, data: page(pageNumber) });
    });
    const cancelPage = vi.fn();
    const warmReleases = new Map<number, ReturnType<typeof vi.fn>>();
    const warmPage = vi.fn((loaded: GalleryPage) => {
      const release = vi.fn();
      warmReleases.set(loaded.page, release);
      return release;
    });
    const retainedRelease = vi.fn();
    const retainPage = vi.fn(() => retainedRelease);
    const session = new ExplorePageSession({ fetchPage, cancelPage, warmPage, retainPage });
    session.start("query-a", page(3));
    session.recordScroll(3, 417);
    session.prefetchAdjacent();
    await vi.waitFor(() => expect(session.cachedPageNumbers()).toEqual([2, 3, 4]));

    const opening = session.open(5);
    await flush();
    const requestId = fetchPage.mock.calls.find(([, pageNumber]) => pageNumber === 5)?.[2];

    session.park();
    session.park();

    expect(retainPage).toHaveBeenCalledOnce();
    expect(retainPage).toHaveBeenCalledWith(page(3));
    expect(warmReleases.get(2)).toHaveBeenCalledOnce();
    expect(warmReleases.get(4)).toHaveBeenCalledOnce();
    expect(cancelPage).toHaveBeenCalledOnce();
    expect(cancelPage).toHaveBeenCalledWith(requestId);
    expect(session.cachedPageNumbers()).toEqual([2, 3, 4]);
    expect(session.scrollFor(3)).toBe(417);
    expect(retainedRelease).not.toHaveBeenCalled();

    pending.get(5)?.({ ok: true, data: page(5) });
    await expect(opening).resolves.toEqual({ status: "stale" });
    expect(session.cachedPageNumbers()).toEqual([2, 3, 4]);
  });

  it("resumes cached warmups without fetching and keeps the retained page until explicitly released", async () => {
    const fetchPage = vi.fn(async (_queryId: string, pageNumber: number): Promise<ApiResult<GalleryPage>> => ({
      ok: true,
      data: page(pageNumber),
    }));
    const warmPage = vi.fn(() => vi.fn());
    const retainedRelease = vi.fn();
    const session = new ExplorePageSession({
      fetchPage,
      warmPage,
      retainPage: () => retainedRelease,
    });
    session.start("query-a", page(3));
    session.recordScroll(3, 92);
    session.prefetchAdjacent();
    await vi.waitFor(() => expect(session.cachedPageNumbers()).toEqual([2, 3, 4]));
    const fetchesBeforePark = fetchPage.mock.calls.length;
    const warmupsBeforePark = warmPage.mock.calls.length;

    session.park();
    session.prefetchAdjacent();
    expect(fetchPage).toHaveBeenCalledTimes(fetchesBeforePark);

    session.resume();
    session.resume();

    expect(fetchPage).toHaveBeenCalledTimes(fetchesBeforePark);
    expect(warmPage).toHaveBeenCalledTimes(warmupsBeforePark + 2);
    expect(retainedRelease).not.toHaveBeenCalled();

    const restored = await session.open(3);
    expect(restored).toMatchObject({ status: "ready", source: "cache", scrollTop: 92 });
    expect(fetchPage).toHaveBeenCalledTimes(fetchesBeforePark);

    session.releaseRetainedPage();
    session.releaseRetainedPage();
    expect(retainedRelease).toHaveBeenCalledOnce();
  });

  it("does not prefetch an uncached adjacent page while parked", async () => {
    const fetchPage = vi.fn(async (_queryId: string, pageNumber: number): Promise<ApiResult<GalleryPage>> => ({
      ok: true,
      data: page(pageNumber),
    }));
    const session = new ExplorePageSession({ fetchPage });
    session.start("query-a", page(1));
    session.park();

    session.prefetchAdjacent();
    await flush();
    expect(fetchPage).not.toHaveBeenCalled();

    session.resume();
    session.prefetchAdjacent();
    await vi.waitFor(() => expect(fetchPage).toHaveBeenCalledOnce());
    expect(fetchPage).toHaveBeenCalledWith("query-a", 2, expect.any(String));
  });

  it("releases a parked page lease when a session is restarted or cleared", () => {
    const releases: Array<ReturnType<typeof vi.fn>> = [];
    const session = new ExplorePageSession({
      fetchPage: async (_queryId, pageNumber) => ({ ok: true, data: page(pageNumber) }),
      retainPage: () => {
        const release = vi.fn();
        releases.push(release);
        return release;
      },
    });
    session.start("query-a", page(1));
    session.park();

    session.start("query-b", page(2));
    expect(releases[0]).toHaveBeenCalledOnce();
    expect(session.cachedPageNumbers()).toEqual([2]);

    session.park();
    session.clear();
    expect(releases[1]).toHaveBeenCalledOnce();
    expect(session.cachedPageNumbers()).toEqual([]);
  });
});
