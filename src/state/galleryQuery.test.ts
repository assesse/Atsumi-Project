import { describe, expect, it } from "vitest";
import type { ApiError, GalleryPage, SearchSubmission } from "../api/contracts";
import { galleryId } from "../core/types";
import { galleryQueryReducer, initialGalleryQueryState } from "./galleryQuery";

const firstPage: GalleryPage = {
  page: 1,
  totalPages: 2,
  items: [{
    id: galleryId(1),
    title: "First",
    artist: "artist",
    pages: 10,
    language: "korean",
    tags: [],
    series: [],
    characters: [],
    publishedRank: 20260814,
    popularity: 0,
    thumbnailWidth: 512,
    thumbnailHeight: 512,
  }],
};

const submission: SearchSubmission = { queryId: "query-1", firstPage };
const error: ApiError = { code: "SOURCE_TIMEOUT", message: "timeout", retryable: true, action: "retry" };

describe("gallery query state", () => {
  it("projects a successful submission and its first page", () => {
    const loading = galleryQueryReducer(initialGalleryQueryState, { type: "submit.started", token: 1 });
    const ready = galleryQueryReducer(loading, { type: "submit.succeeded", token: 1, submission });

    expect(ready.phase).toBe("ready");
    expect(ready.queryId).toBe("query-1");
    expect(ready.page).toEqual(firstPage);
  });

  it("ignores a response from an older submit token", () => {
    const first = galleryQueryReducer(initialGalleryQueryState, { type: "submit.started", token: 1 });
    const latest = galleryQueryReducer(first, { type: "submit.started", token: 2 });
    const stale = galleryQueryReducer(latest, { type: "submit.succeeded", token: 1, submission });

    expect(stale).toBe(latest);
    expect(stale.phase).toBe("submitting");
  });

  it("accepts only the requested page for the active query", () => {
    const loading = galleryQueryReducer(initialGalleryQueryState, { type: "submit.started", token: 1 });
    const ready = galleryQueryReducer(loading, { type: "submit.succeeded", token: 1, submission });
    const paging = galleryQueryReducer(ready, { type: "page.started", queryId: "query-1", page: 2 });
    const wrongPage = galleryQueryReducer(paging, {
      type: "page.succeeded",
      queryId: "query-1",
      page: { ...firstPage, page: 1 },
    });
    const secondPage = { ...firstPage, page: 2 };
    const completed = galleryQueryReducer(wrongPage, {
      type: "page.succeeded",
      queryId: "query-1",
      page: secondPage,
    });

    expect(wrongPage).toBe(paging);
    expect(completed.phase).toBe("ready");
    expect(completed.page).toEqual(secondPage);
  });

  it("retains the current page when page loading fails", () => {
    const loading = galleryQueryReducer(initialGalleryQueryState, { type: "submit.started", token: 1 });
    const ready = galleryQueryReducer(loading, { type: "submit.succeeded", token: 1, submission });
    const paging = galleryQueryReducer(ready, { type: "page.started", queryId: "query-1", page: 2 });
    const failed = galleryQueryReducer(paging, {
      type: "page.failed",
      queryId: "query-1",
      page: 2,
      error,
    });

    expect(failed.phase).toBe("error");
    expect(failed.page).toEqual(firstPage);
    expect(failed.error).toEqual(error);
  });
});
