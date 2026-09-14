import { describe, expect, it } from "vitest";
import type { DownloadEntry, DownloadLibraryPage, GalleryDetail, GalleryPage, GallerySummary } from "../api/contracts";
import { galleryId, type Gallery } from "../core/types";
import { mergeDownloadEntries, mergeDownloadLibraryPage, mergeGalleryDetail, mergeGalleryPage, projectGallerySummary } from "./galleryProjection";

const summary = (idValue: number, title = `Gallery ${idValue}`): GallerySummary => ({
  id: galleryId(idValue),
  title,
  artist: "serein",
  pages: 24,
  language: "japanese",
  tags: ["female:glasses"],
  series: ["rain archives"],
  characters: ["mira lane"],
  publishedRank: 20260814,
  popularity: 91,
  thumbnailKey: `fixture-${idValue}`,
  thumbnailWidth: 512,
  thumbnailHeight: 512,
});

describe("gallery API projection", () => {
  it("carries participant artists without changing the primary artist or sharing source arrays", () => {
    const artists = ["serein", "favorite collaborator", "another artist"];
    const projected = projectGallerySummary({ ...summary(1), artists });
    expect(projected.artist).toBe("serein");
    expect(projected.artists).toEqual(artists);
    expect(projected.artists).not.toBe(artists);
    expect(projected.favorite).toBeUndefined();
  });

  it("keeps known participants through legacy summaries, empty lists and queue events", () => {
    const existing = projectGallerySummary({ ...summary(1), artists: ["serein", "collaborator"] });
    for (const incoming of [summary(1), { ...summary(1), artists: [] }]) {
      const merged = mergeGalleryPage(new Map([[existing.id, existing]]), {
        page: 1, totalPages: 1, items: [incoming],
      }).galleries;
      const queued = mergeDownloadEntries(merged, [{
        entryId: "artists-queue", galleryId: existing.id, revision: 1, state: "queued",
      }]);
      expect(queued.get(existing.id)?.artists).toEqual(existing.artists);
    }
    expect(projectGallerySummary(summary(2))).not.toHaveProperty("artists");
  });

  it("hydrates full participants from details and related summaries", () => {
    const merged = mergeGalleryDetail(new Map(), {
      ...summary(1), artists: ["serein", "collaborator"], pageDimensions: [],
      related: [{ ...summary(2), artists: ["serein", "another collaborator"] }],
    });
    expect(merged.get(galleryId(1))?.artists).toEqual(["serein", "collaborator"]);
    expect(merged.get(galleryId(2))?.artists).toEqual(["serein", "another collaborator"]);
  });

  it("restores locally saved participants on cold download lists and preserves later source enrichment", () => {
    const page: DownloadLibraryPage = {
      page: 1, totalItems: 1, items: [{
        gallery: { id: galleryId(1), artist: "serein", artists: ["serein", "saved collaborator"] },
        download: { entryId: "saved-artists", galleryId: galleryId(1), revision: 1, state: "completed" },
      }],
    };
    const loaded = mergeDownloadLibraryPage(new Map(), page).galleries;
    expect(loaded.get(galleryId(1))?.artists).toEqual(["serein", "saved collaborator"]);
    const enriched = mergeGalleryPage(loaded, {
      page: 1, totalPages: 1,
      items: [{ ...summary(1), artists: ["serein", "saved collaborator", "fresh collaborator"] }],
    }).galleries;
    expect(mergeDownloadLibraryPage(enriched, page).galleries.get(galleryId(1))?.artists)
      .toEqual(["serein", "saved collaborator", "fresh collaborator"]);
  });

  it("projects summary metadata while preserving an existing download projection", () => {
    const existing: Gallery = {
      ...projectGallerySummary(summary(1, "Old title")),
      download: { entryId: "entry-1", state: "downloading", progress: 42 },
    };

    const projected = projectGallerySummary(summary(1, "Fresh title"), existing);

    expect(projected).toMatchObject({
      title: "Fresh title",
      language: "japanese",
      publishedAt: "2026-08-14",
      score: 91,
      download: { entryId: "entry-1", state: "downloading", progress: 42 },
    });
  });

  it("accepts legacy summaries without newer search metadata", () => {
    const existing = projectGallerySummary(summary(1, "Existing title"));
    const legacySummary = (idValue: number, title: string) => ({
      id: galleryId(idValue),
      title,
      artist: "legacy artist",
      pages: 12,
    }) as unknown as GallerySummary;
    const page: GalleryPage = {
      page: 1,
      totalPages: 1,
      items: [legacySummary(1, "Updated legacy title"), legacySummary(2, "New legacy title")],
    };

    const projected = mergeGalleryPage(new Map([[existing.id, existing]]), page).galleries;

    expect(projected.get(galleryId(1))).toMatchObject({
      title: "Updated legacy title",
      language: "japanese",
      tags: ["female:glasses"],
      series: ["rain archives"],
      characters: ["mira lane"],
      publishedAt: "2026-08-14",
      score: 91,
      thumbnailKey: "fixture-1",
      thumbnailWidth: 512,
      thumbnailHeight: 512,
    });
    expect(projected.get(galleryId(2))).toMatchObject({
      title: "New legacy title",
      language: "korean",
      tags: [],
      series: [],
      characters: [],
      publishedAt: "0000-00-00",
      score: 0,
    });
    expect(projected.get(galleryId(2))).not.toHaveProperty("thumbnailWidth");
    expect(projected.get(galleryId(2))).not.toHaveProperty("thumbnailHeight");
  });

  it("merges only the current search page IDs without discarding prior galleries", () => {
    const initial = new Map([[galleryId(1), projectGallerySummary(summary(1))]]);
    const page: GalleryPage = { page: 2, totalPages: 2, items: [summary(2), summary(3)] };

    const result = mergeGalleryPage(initial, page);

    expect(result.ids).toEqual([galleryId(2), galleryId(3)]);
    expect([...result.galleries.keys()]).toEqual([galleryId(1), galleryId(2), galleryId(3)]);
  });

  it("preserves hydrated page dimensions when a later search summary refreshes the gallery", () => {
    const dimensions = [{ sourcePage: 1, width: 720, height: 1080 }];
    const hydrated: Gallery = {
      ...projectGallerySummary(summary(1, "Hydrated title")),
      pageDimensions: dimensions,
    };
    const page: GalleryPage = {
      page: 1,
      totalPages: 1,
      items: [summary(1, "Refreshed title")],
    };

    const refreshed = mergeGalleryPage(new Map([[hydrated.id, hydrated]]), page).galleries;

    expect(refreshed.get(hydrated.id)?.title).toBe("Refreshed title");
    expect(refreshed.get(hydrated.id)?.pageDimensions).toBe(dimensions);
  });

  it("preserves another open detail's page dimensions when it appears as a related summary", () => {
    const dimensions = [{ sourcePage: 1, width: 1600, height: 900 }];
    const openRelated: Gallery = {
      ...projectGallerySummary(summary(2, "Open related detail")),
      pageDimensions: dimensions,
    };
    const incoming: GalleryDetail = {
      ...summary(1),
      related: [summary(2, "Related summary refresh")],
      pageDimensions: [{ sourcePage: 1, width: 720, height: 1080 }],
    };

    const merged = mergeGalleryDetail(new Map([[openRelated.id, openRelated]]), incoming);

    expect(merged.get(openRelated.id)?.title).toBe("Related summary refresh");
    expect(merged.get(openRelated.id)?.pageDimensions).toBe(dimensions);
  });

  it("hydrates related summaries and projects queue snapshots onto the same galleries", () => {
    const detail: GalleryDetail = { ...summary(1), related: [summary(2), summary(3)], pageDimensions: [{ sourcePage: 1, width: 720, height: 1080 }] };
    const hydrated = mergeGalleryDetail(new Map(), detail);
    const entries: DownloadEntry[] = [{
      entryId: "entry-1",
      galleryId: galleryId(1),
      revision: 0,
      state: "queued",
      progress: 0,
    }];
    const queued = mergeDownloadEntries(hydrated, entries);

    expect(queued.get(galleryId(1))?.relatedIds).toEqual([galleryId(2), galleryId(3)]);
    expect(queued.get(galleryId(2))?.title).toBe("Gallery 2");
    expect(queued.get(galleryId(1))?.download).toEqual({
      entryId: "entry-1",
      revision: 0,
      state: "queued",
      progress: 0,
    });
  });

  it("projects download library summaries without discarding existing hydrated metadata", () => {
    const hydrated: Gallery = {
      ...projectGallerySummary(summary(1)),
      tags: ["favorite-tag"],
      series: ["series-one"],
      characters: ["character-one"],
    };
    const page: DownloadLibraryPage = {
      page: 1,
      totalItems: 1,
      items: [{
        gallery: {
          id: galleryId(1),
          title: "Fresh title",
          artist: "fresh artist",
          pages: 24,
          language: "japanese",
          publishedRank: 20260831,
          tags: ["older-cached-tag"],
        },
        download: {
          entryId: "entry-library",
          galleryId: galleryId(1),
          revision: 4,
          state: "completed",
          progress: 100,
        },
      }],
    };

    const projected = mergeDownloadLibraryPage(new Map([[hydrated.id, hydrated]]), page).galleries;

    expect(projected.get(hydrated.id)).toMatchObject({
      title: "Fresh title",
      artist: "fresh artist",
      publishedAt: "2026-08-31",
      download: {
        entryId: "entry-library",
        state: "completed",
        progress: 100,
      },
      tags: ["favorite-tag"],
      tagsKnown: true,
      series: ["series-one"],
      characters: ["character-one"],
    });
  });

  it("restores saved tags on a cold list load and distinguishes absent data from known empty tags", () => {
    const ids = [81, 82, 83].map(galleryId);
    const page: DownloadLibraryPage = {
      page: 1,
      totalItems: 3,
      items: ids.map((id, index) => ({
        gallery: {
          id, title: `Saved ${id}`, artist: "saved artist",
          ...(index === 0 ? { tags: ["female:glasses", "full_color"] } : index === 1 ? { tags: [] } : {}),
        },
        download: { entryId: `saved-${id}`, galleryId: id, revision: 1, state: "completed" },
      })),
    };
    // A download event may arrive before the list. Its placeholder is not an
    // authoritative empty tag list and must still accept the saved metadata.
    const placeholders = mergeDownloadEntries(new Map(), page.items.map((item) => item.download));
    const loaded = mergeDownloadLibraryPage(placeholders, page).galleries;
    expect(loaded.get(ids[0]!)).toMatchObject({ tags: ["female:glasses", "full_color"], tagsKnown: true });
    expect(loaded.get(ids[1]!)).toMatchObject({ tags: [], tagsKnown: true });
    expect(loaded.get(ids[2]!)).toMatchObject({ tags: [], tagsKnown: false });

    const enriched = mergeGalleryPage(loaded, {
      page: 1, totalPages: 1, items: [{ ...summary(83), tags: ["fresh-tag"] }],
    }).galleries;
    const reloaded = mergeDownloadLibraryPage(enriched, page).galleries;
    expect(reloaded.get(ids[2]!)).toMatchObject({ tags: ["fresh-tag"], tagsKnown: true });
    const cleared = mergeGalleryPage(reloaded, {
      page: 1, totalPages: 1, items: [{ ...summary(81), tags: [] }],
    }).galleries;
    expect(mergeDownloadLibraryPage(cleared, page).galleries.get(ids[0]!))
      .toMatchObject({ tags: [], tagsKnown: true });
  });

  it("marks a legacy download language unknown until local detail metadata hydrates it", () => {
    const page: DownloadLibraryPage = {
      page: 1,
      totalItems: 1,
      items: [{
        gallery: { id: galleryId(77), title: "Legacy local album", artist: "local artist" },
        download: {
          entryId: "entry-legacy",
          galleryId: galleryId(77),
          revision: 0,
          state: "completed",
          progress: 100,
        },
      }],
    };

    const summarized = mergeDownloadLibraryPage(new Map(), page).galleries;
    expect(summarized.get(galleryId(77))?.languageKnown).toBe(false);

    const hydrated = mergeGalleryDetail(summarized, {
      ...summary(77),
      related: [],
      pageDimensions: [],
    });
    expect(hydrated.get(galleryId(77))?.language).toBe("japanese");
    expect(hydrated.get(galleryId(77))?.languageKnown).not.toBe(false);
  });

  it("does not let an older list snapshot overwrite a newer event projection", () => {
    const projected = projectGallerySummary(summary(1));
    const current: Gallery = {
      ...projected,
      download: {
        entryId: "entry-1",
        revision: 5,
        state: "downloading",
        progress: 72,
        attempt: 2,
      },
    };

    const merged = mergeDownloadEntries(new Map([[current.id, current]]), [{
      entryId: "entry-1",
      galleryId: current.id,
      revision: 4,
      state: "queued",
      progress: 0,
      attempt: 2,
    }]);

    expect(merged.get(current.id)).toBe(current);
  });

  it("retains the download timeline when a newer event omits immutable list metadata", () => {
    const current = projectGallerySummary(summary(6));
    const first = mergeDownloadEntries(new Map([[current.id, current]]), [{
      entryId: "entry-timeline",
      galleryId: current.id,
      revision: 1,
      state: "queued",
      progress: 0,
      createdAt: "2026-08-20T10:00:00Z",
      updatedAt: "2026-08-20T10:00:00Z",
    }]);
    const second = mergeDownloadEntries(first, [{
      entryId: "entry-timeline",
      galleryId: current.id,
      revision: 2,
      state: "downloading",
      progress: 40,
    }]);

    expect(second.get(current.id)?.download).toMatchObject({
      createdAt: "2026-08-20T10:00:00Z",
      updatedAt: "2026-08-20T10:00:00Z",
    });
  });

  it("preserves the typed download overlap review identity from list hydration", () => {
    const current = projectGallerySummary(summary(7));
    const merged = mergeDownloadEntries(new Map([[current.id, current]]), [{
      entryId: "entry-overlap",
      galleryId: current.id,
      revision: 3,
      state: "review_required",
      progress: 100,
      reviewKind: "gallery_duplicate",
      reviewId: "review-overlap",
    }]);
    expect(merged.get(current.id)?.download).toMatchObject({
      state: "review_required",
      reviewKind: "gallery_duplicate",
      reviewId: "review-overlap",
    });
  });
});
