import { describe, expect, it } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import { initialUiState } from "./uiState";
import { visibleGalleries } from "./selectors";
import { autoFindFavoriteMatches, groupAutoFindGalleries, uniqueAutoFindGalleries } from "./autoFindFavorites";
import { paginateAutoFindItems } from "./autoFindPagination";

const album = (patch: Partial<Gallery> = {}): Gallery => ({
  id: galleryId(1775102), title: "Anthology", subtitle: "", artist: "akairo",
  artists: ["akairo", "dekosuke 18gou", "narita koh"], group: "circle g",
  pages: 30, score: 0, publishedAt: "2026-09-22", coverIndex: 0, language: "korean", tags: [], series: [], characters: [],
  ...patch,
});

describe("Auto Find artist and group membership", () => {
  it("groups every favorite participating artist without using the non-favorite primary artist", () => {
    const favorites = new Set(["artist:dekosuke_18gou", "artist:Narita Koh"]);
    const groups = groupAutoFindGalleries([album()], "artist", (item) => item.publishedAt, favorites, new Map());
    expect(groups.map((group) => group.label)).toEqual(["작가 · dekosuke 18gou", "작가 · narita koh"]);
    expect(groups.every((group) => group.items[0]?.id === galleryId(1775102))).toBe(true);
    const items = uniqueAutoFindGalleries(groups.flatMap((group) => group.items));
    expect(items).toHaveLength(1);
    expect(paginateAutoFindItems(items, 1, 10).totalPages).toBe(1);
  });

  it("keeps a collaborating favorite after removing the originally matched favorite", () => {
    expect(autoFindFavoriteMatches(album(), new Set(["artist:narita koh"]), "artist:dekosuke 18gou"))
      .toEqual([{ namespace: "artist", value: "narita koh" }]);
    expect(autoFindFavoriteMatches(album(), new Set(), "artist:dekosuke 18gou")).toEqual([]);
  });

  it("finds group-only works and does not treat a group as the artist", () => {
    const unknown = album({ artist: "Unknown artist", artists: [] });
    const matches = autoFindFavoriteMatches(unknown, new Set(["group:circle g"]));
    expect(matches).toEqual([{ namespace: "group", value: "circle g" }]);
    expect(autoFindFavoriteMatches(unknown, new Set(["artist:circle g"]))).toEqual([]);
    expect(groupAutoFindGalleries([unknown], "artist", () => undefined, new Set(["group:circle g"]), new Map())[0]?.label)
      .toBe("그룹 · circle g");
  });

  it("treats artists and groups as independent OR conditions, including other circles", () => {
    const favorites = new Set(["artist:narita koh", "group:circle g"]);
    expect(autoFindFavoriteMatches(album({ group: "another circle" }), favorites)).toEqual([{ namespace: "artist", value: "narita koh" }]);
    expect(autoFindFavoriteMatches(album(), favorites)).toHaveLength(2);
    expect(autoFindFavoriteMatches(album({ artists: ["akairo"] }), new Set(["artist:narita koh"]), "artist:narita koh")).toEqual([]);
  });

  it("preserves authoritative group matches not exposed by a single-group summary", () => {
    expect(autoFindFavoriteMatches(album(), new Set(["group:second circle"]), ["artist:akairo", "group:second circle"]))
      .toEqual([{ namespace: "group", value: "second circle" }]);
  });

  it("supports legacy artist evidence without inventing splits or duplicate memberships", () => {
    const legacy = album({ artists: undefined, artist: "a & b" });
    expect(autoFindFavoriteMatches(legacy, new Set(["artist:narita koh"]), "artist:narita koh")).toHaveLength(1);
    expect(autoFindFavoriteMatches(legacy, new Set(["artist:a"]))).toEqual([]);
    expect(autoFindFavoriteMatches(album({ artists: ["Narita_Koh", "narita koh"] }), new Set(["artist:NARITA KOH"]))).toHaveLength(1);
  });

  it("does not duplicate daily/all pagination and supports participant name searches", () => {
    const item = album({ favorite: true });
    const groups = groupAutoFindGalleries([item], "day", (gallery) => gallery.publishedAt,
      new Set(["artist:narita koh", "group:circle g"]), new Map());
    expect(groups).toHaveLength(1);
    for (const query of ["artist:narita_koh", "narita", "group:circle_g"]) {
      const state = { ...initialUiState, view: "auto-find" as const,
        search: { ...initialUiState.search, "auto-find": { ...initialUiState.search["auto-find"], committed: query } } };
      expect(visibleGalleries(state, [item])).toEqual([item]);
      state.search["auto-find"].committed = `-${query}`;
      expect(visibleGalleries(state, [item])).toEqual([]);
    }
  });
});
