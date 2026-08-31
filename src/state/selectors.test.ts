import { describe, expect, it } from "vitest";
import { mockGalleries } from "../data/mockGalleries";
import { initialUiState, uiReducer } from "./uiState";
import { visibleGalleries } from "./selectors";

describe("gallery selectors", () => {
  it("understands namespace-prefixed artist searches", () => {
    const searched = uiReducer(initialUiState, {
      type: "search.commit",
      view: "explore",
      value: "artist:serein",
    });
    expect(visibleGalleries(searched, mockGalleries).map((gallery) => gallery.artist)).toEqual([
      "serein",
      "serein",
    ]);
  });

  it.each(["artist:sugoi_hi", "artist:sugoi\\_hi"])(
    "matches a spaced artist name from the structured %s token",
    (value) => {
      const searched = uiReducer(initialUiState, {
        type: "search.commit",
        view: "explore",
        value,
      });
      const gallery = { ...mockGalleries[0]!, artist: "sugoi hi" };

      expect(visibleGalleries(searched, [gallery])).toEqual([gallery]);
    },
  );

  it("ANDs multiple structured tokens instead of treating the tail as one artist value", () => {
    const searched = uiReducer(initialUiState, {
      type: "search.commit",
      view: "explore",
      value: "artist:healthyman female:ahegao",
    });
    const matching = {
      ...mockGalleries[0]!,
      artist: "healthyman",
      tags: ["female:ahegao", "full_color"],
    };
    const missingTag = { ...mockGalleries[1]!, artist: "healthyman" };

    expect(visibleGalleries(searched, [matching, missingTag])).toEqual([matching]);
  });

  it("applies negative structured tokens alongside positive tokens", () => {
    const searched = uiReducer(initialUiState, {
      type: "search.commit",
      view: "explore",
      value: "artist:healthyman female:ahegao -male:glasses",
    });
    const included = {
      ...mockGalleries[0]!,
      artist: "healthyman",
      tags: ["female:ahegao"],
    };
    const excluded = {
      ...mockGalleries[1]!,
      artist: "healthyman",
      tags: ["female:ahegao", "male:glasses"],
    };

    expect(visibleGalleries(searched, [included, excluded])).toEqual([included]);
  });

  it("keeps each view's search independently", () => {
    const downloads = uiReducer(initialUiState, { type: "navigate", view: "downloads" });
    const searched = uiReducer(downloads, {
      type: "search.commit",
      view: "downloads",
      value: "paperlane",
    });
    expect(searched.search.explore.committed).toBe("");
    expect(searched.search.downloads.committed).toBe("paperlane");
    expect(visibleGalleries(searched, mockGalleries)).toHaveLength(1);
  });

  it("finds a downloaded gallery by its numeric gallery ID", () => {
    const downloads = uiReducer(initialUiState, { type: "navigate", view: "downloads" });
    const searched = uiReducer(downloads, {
      type: "search.commit",
      view: "downloads",
      value: "4050974",
    });

    expect(visibleGalleries(searched, mockGalleries).map((gallery) => gallery.id)).toEqual([4050974]);
  });

  it("does not hide an exact seven-digit Explore ID behind the language filter", () => {
    const searched = uiReducer(initialUiState, {
      type: "search.commit",
      view: "explore",
      value: "4050974",
    });
    const englishGallery = { ...mockGalleries[2]!, language: "english" as const };

    expect(visibleGalleries(searched, [englishGallery])).toEqual([englishGallery]);
  });

  it("keeps unresolved legacy download summaries visible until their language hydrates", () => {
    const downloads = uiReducer(initialUiState, { type: "navigate", view: "downloads" });
    const japaneseOnly = uiReducer(downloads, {
      type: "search.languages",
      view: "downloads",
      languages: ["japanese"],
    });
    const unresolved = {
      ...mockGalleries[0]!,
      language: "korean" as const,
      languageKnown: false,
      download: { entryId: "legacy-entry", state: "completed" as const, progress: 100 },
    };

    expect(visibleGalleries(japaneseOnly, [unresolved])).toEqual([unresolved]);
    expect(visibleGalleries(japaneseOnly, [{ ...unresolved, languageKnown: true }])).toEqual([]);
  });

  it("understands optional group searches", () => {
    const searched = uiReducer(initialUiState, {
      type: "search.commit",
      view: "explore",
      value: "group:paper_studio",
    });
    expect(visibleGalleries(searched, mockGalleries).map((gallery) => gallery.title)).toEqual([
      "The Green Window",
    ]);
  });

  it("matches neutral tags after the display-only tag namespace is added", () => {
    const searched = uiReducer(initialUiState, {
      type: "search.commit",
      view: "explore",
      value: "tag:full_color",
    });

    expect(visibleGalleries(searched, mockGalleries).map((gallery) => gallery.title)).toEqual([
      "Archive of Rain",
      "Summer Pool Notes",
      "Platform 19",
      "Festival Letter",
    ]);
  });

  it.each([
    ["series:rain_archives", ["Archive of Rain", "The Last Tram"]],
    ["character:aoi_mizuno", ["Summer Pool Notes", "Blue Lane"]],
  ])("filters local results by %s metadata", (value, titles) => {
    const searched = uiReducer(initialUiState, {
      type: "search.commit",
      view: "explore",
      value,
    });
    expect(visibleGalleries(searched, mockGalleries).map((gallery) => gallery.title)).toEqual(titles);
  });

  it("hides quarantined albums from Auto Find and Downloads but preserves their Explore result slot", () => {
    const quarantined = {
      ...mockGalleries[0]!,
      favorite: true,
      download: { entryId: "quarantined-entry", state: "quarantined" as const, progress: 100 },
    };

    expect(visibleGalleries(initialUiState, [quarantined])).toEqual([quarantined]);

    const autoFind = uiReducer(initialUiState, { type: "navigate", view: "auto-find" });
    expect(visibleGalleries(autoFind, [quarantined])).toEqual([]);

    const downloads = uiReducer(initialUiState, { type: "navigate", view: "downloads" });
    expect(visibleGalleries(downloads, [quarantined])).toEqual([]);
  });

  it("shows the most recently added download first only in the flat all view", () => {
    const older = {
      ...mockGalleries[0]!,
      download: {
        entryId: "older-download",
        state: "completed" as const,
        progress: 100,
        createdAt: "2026-08-20T10:00:00Z",
      },
    };
    const newer = {
      ...mockGalleries[1]!,
      download: {
        entryId: "newer-download",
        state: "completed" as const,
        progress: 100,
        createdAt: "2026-08-28T10:00:00Z",
      },
    };
    const downloads = uiReducer(initialUiState, { type: "navigate", view: "downloads" });

    expect(visibleGalleries(downloads, [older, newer]).map((gallery) => gallery.id)).toEqual([
      newer.id,
      older.id,
    ]);

    const artistGrouping = uiReducer(downloads, {
      type: "grouping.set",
      view: "downloads",
      grouping: "artist",
    });
    expect(visibleGalleries(artistGrouping, [older, newer]).map((gallery) => gallery.id)).toEqual([
      older.id,
      newer.id,
    ]);
  });
});
