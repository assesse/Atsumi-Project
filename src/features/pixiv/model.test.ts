import { beforeEach, describe, expect, it } from "vitest";
import { artworkId, bookmarkKey, loadBookmarks, loadHistory, PIXIV_BOOKMARK_KEY, PIXIV_HISTORY_KEY, updateSelection } from "./model";

describe("Pixiv isolated local library", () => {
  beforeEach(() => localStorage.clear());
  it("accepts only canonical artwork links and numeric ids", () => {
    expect(artworkId("150653396")).toBe("150653396");
    expect(artworkId("https://www.pixiv.net/ko/artworks/150653396?from=home")).toBe("150653396");
    for (const input of ["0", "001", "https://pixiv.net.evil.test/artworks/123", "http://pixiv.net/artworks/123", "https://x@pixiv.net/artworks/123", "https://pixiv.net:444/artworks/123", "https://pixiv.net/artworks/123/extra", "javascript:alert(1)"]) expect(artworkId(input)).toBeNull();
  });
  it("keeps single focus separate from checkbox multi-selection", () => {
    const ids = ["1", "2", "3", "4"];
    expect(updateSelection(ids, [], "1", null, { ctrl: false, shift: false })).toEqual(["1"]);
    expect(updateSelection(ids, ["1"], "1", "1", { ctrl: false, shift: false })).toEqual([]);
    expect(updateSelection(ids, ["1"], "3", "1", { ctrl: false, shift: true })).toEqual(["1", "2", "3"]);
    expect(updateSelection(ids, ["1"], "3", "1", { ctrl: true, shift: false })).toEqual(["1", "3"]);
    expect(updateSelection(ids, ["1", "3"], "1", "3", { ctrl: true, shift: false })).toEqual(["3"]);
  });
  it("separates album and page bookmarks and discards malformed storage", () => {
    expect(bookmarkKey({ id: "123" })).not.toBe(bookmarkKey({ id: "123" }, 0));
    localStorage.setItem(PIXIV_BOOKMARK_KEY, "bad json"); expect(loadBookmarks()).toEqual([]);
    localStorage.setItem(PIXIV_BOOKMARK_KEY, JSON.stringify([{ work: { id: "123", title: "Fixture", artist: "Artist" }, savedAt: 10 }, { work: { id: "x" }, page: -1 }]));
    expect(loadBookmarks()).toHaveLength(1);
  });
  it("keeps ten unique bounded search-history entries without trusting malformed storage", () => {
    localStorage.setItem(PIXIV_HISTORY_KEY, JSON.stringify([null, "", "landscape", "landscape", ...Array.from({ length: 20 }, (_, index) => `tag${index}`)]));
    expect(loadHistory()).toHaveLength(10); expect(loadHistory()[0]).toBe("landscape");
    localStorage.setItem(PIXIV_HISTORY_KEY, "invalid"); expect(loadHistory()).toEqual([]);
  });
});
