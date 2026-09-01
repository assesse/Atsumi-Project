import { describe, expect, it, vi } from "vitest";
import { galleryId } from "../core/types";
import { backend } from "./backend";
import type {
  AutoFindSnapshot,
  DownloadEntry,
  DownloadOverlapReview,
  DuplicateSnapshot,
  InternalArtifactScanProgress,
  InternalDuplicateSnapshot,
  SearchRequest,
} from "./contracts";

const searchRequest = (patch: Partial<SearchRequest> = {}): SearchRequest => ({
  text: "",
  includeTags: [],
  excludeTags: [],
  languages: ["korean"],
  sort: "recent",
  pageSize: 3,
  ...patch,
});

describe("browser Danbooru search contract", () => {
  it("does not charge unlimited metadata against the anonymous two-term limit", async () => {
    await expect(backend.danbooruSearch({
      tags: "fixture_artist_1 blue_sky rating:g filetype:jpg date:2026-08-31 score:>=0",
      page: 1,
      pageSize: 50,
    })).resolves.toMatchObject({ ok: true });

    await expect(backend.danbooruSearch({
      tags: "fixture_artist_1 order:score",
      page: 1,
      pageSize: 50,
    })).resolves.toMatchObject({ ok: true });

    await expect(backend.danbooruSearch({
      tags: "fixture_artist_1 blue_sky order:score",
      page: 1,
      pageSize: 50,
    })).resolves.toMatchObject({
      ok: false,
      error: { code: "DANBOORU_TAG_LIMIT" },
    });
  });
});

describe("browser backend settings contract", () => {
  it("reports memory, disk, download, and volume usage separately", async () => {
    const original = await backend.settingsGet();
    if (!original.ok) throw new Error(original.error.message);
    const configured = await backend.settingsUpdate({ downloadRoot: "D:\\Atsumi" }, original.data.revision);
    if (!configured.ok) throw new Error(configured.error.message);
    try {
      const result = await backend.storageUsageGet();
      expect(result).toMatchObject({
        ok: true,
        data: {
          memoryCacheBytes: expect.any(Number),
          diskCache: { bytes: expect.any(Number), scanComplete: true },
          appData: { bytes: expect.any(Number), scanComplete: true },
          downloads: { bytes: expect.any(Number), scanComplete: true, volumeRoot: "D:\\" },
          volumes: [
            { root: "C:\\", totalBytes: expect.any(Number), availableBytes: expect.any(Number) },
            { root: "D:\\", totalBytes: expect.any(Number), availableBytes: expect.any(Number) },
          ],
        },
      });
    } finally {
      const latest = await backend.settingsGet();
      if (latest.ok) await backend.settingsUpdate({ downloadRoot: original.data.downloadRoot }, latest.data.revision);
    }
  });

  it("rejects values outside the approved settings ranges", async () => {
    expect(backend.runtime).toBe("browser-mock");
    const current = await backend.settingsGet();
    if (!current.ok) throw new Error(current.error.message);

    const result = await backend.settingsUpdate({ maxColumns: 5 }, current.data.revision);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.error.code).toBe("VALIDATION_ERROR");
    expect(result.error.details?.field).toBe("maxColumns");

    const arbitraryPreview = await backend.settingsUpdate(
      { previewWidth: 235 },
      current.data.revision,
    );
    expect(arbitraryPreview).toMatchObject({
      ok: false,
      error: { code: "VALIDATION_ERROR", details: { field: "previewWidth" } },
    });

    await expect(backend.settingsUpdate(
      { explorePageSize: 9 },
      current.data.revision,
    )).resolves.toMatchObject({
      ok: false,
      error: { code: "VALIDATION_ERROR", details: { field: "explorePageSize" } },
    });
  });

  it("persists the Explore page size used by new searches", async () => {
    const current = await backend.settingsGet();
    if (!current.ok) throw new Error(current.error.message);
    const nextPageSize = current.data.explorePageSize === 80 ? 70 : 80;
    const updated = await backend.settingsUpdate(
      { explorePageSize: nextPageSize },
      current.data.revision,
    );
    expect(updated).toMatchObject({ ok: true, data: { explorePageSize: nextPageSize } });
    if (!updated.ok) return;
    expect(JSON.parse(window.localStorage.getItem("atsumi.browser.settings.v1") ?? "{}"))
      .toMatchObject({ explorePageSize: nextPageSize });

    const restored = await backend.settingsUpdate(
      { explorePageSize: current.data.explorePageSize },
      updated.data.revision,
    );
    expect(restored).toMatchObject({ ok: true, data: { explorePageSize: current.data.explorePageSize } });
  });

  it("persists only supported overlap automation modes", async () => {
    const current = await backend.settingsGet();
    if (!current.ok) throw new Error(current.error.message);
    const nextMode = current.data.downloadOverlapAutoMode === "recommend" ? "off" : "recommend";
    const updated = await backend.settingsUpdate(
      { downloadOverlapAutoMode: nextMode },
      current.data.revision,
    );
    expect(updated).toMatchObject({ ok: true, data: { downloadOverlapAutoMode: nextMode } });
    if (!updated.ok) return;
    await expect(backend.settingsUpdate(
      { downloadOverlapAutoMode: "unsafe" as "off" },
      updated.data.revision,
    )).resolves.toMatchObject({
      ok: false,
      error: { code: "VALIDATION_ERROR", details: { field: "downloadOverlapAutoMode" } },
    });
    await expect(backend.settingsUpdate(
      { downloadOverlapAutoMode: current.data.downloadOverlapAutoMode },
      updated.data.revision,
    )).resolves.toMatchObject({ ok: true });
  });

  it("emits the new revision when settings change", async () => {
    const revisions: number[] = [];
    const unsubscribe = await backend.on("settings:changed", (snapshot) => revisions.push(snapshot.revision));
    const current = await backend.settingsGet();
    if (!current.ok) throw new Error(current.error.message);

    const nextColumns = current.data.maxColumns === 4 ? 3 : 4;
    const result = await backend.settingsUpdate({ maxColumns: nextColumns }, current.data.revision);
    unsubscribe();

    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(revisions).toEqual([result.data.revision]);
  });

  it("persists the visual privacy mode as a boolean setting", async () => {
    const current = await backend.settingsGet();
    if (!current.ok) throw new Error(current.error.message);
    const nextPrivacyMode = !current.data.privacyMode;

    const updated = await backend.settingsUpdate(
      { privacyMode: nextPrivacyMode },
      current.data.revision,
    );
    expect(updated).toMatchObject({ ok: true, data: { privacyMode: nextPrivacyMode } });
    if (!updated.ok) return;
    expect(JSON.parse(window.localStorage.getItem("atsumi.browser.settings.v1") ?? "{}"))
      .toMatchObject({ privacyMode: nextPrivacyMode, revision: updated.data.revision });

    const restored = await backend.settingsUpdate(
      { privacyMode: current.data.privacyMode },
      updated.data.revision,
    );
    expect(restored).toMatchObject({ ok: true, data: { privacyMode: current.data.privacyMode } });
  });

  it("persists normalized accordion keys for the next browser session", async () => {
    const current = await backend.settingsGet();
    if (!current.ok) throw new Error(current.error.message);
    const updated = await backend.settingsUpdate({
      collapsedGroupKeys: ["downloads\u001fday\u001f2026-08-25", "auto-find\u001fartist\u001fmizuno", "auto-find\u001fartist\u001fmizuno"],
    }, current.data.revision);
    expect(updated).toMatchObject({
      ok: true,
      data: {
        collapsedGroupKeys: ["auto-find\u001fartist\u001fmizuno", "downloads\u001fday\u001f2026-08-25"],
      },
    });
    if (!updated.ok) return;
    expect(JSON.parse(window.localStorage.getItem("atsumi.browser.settings.v1") ?? "{}"))
      .toMatchObject({ collapsedGroupKeys: updated.data.collapsedGroupKeys });

    const restored = await backend.settingsUpdate(
      { collapsedGroupKeys: current.data.collapsedGroupKeys },
      updated.data.revision,
    );
    expect(restored.ok).toBe(true);
  });

  it("defaults gallery projections to all and persists valid per-view choices", async () => {
    const current = await backend.settingsGet();
    if (!current.ok) throw new Error(current.error.message);
    expect(current.data.autoFindGrouping).toMatch(/^(all|day|artist)$/);
    expect(current.data.downloadsGrouping).toMatch(/^(all|day|artist)$/);

    const updated = await backend.settingsUpdate({
      autoFindGrouping: "artist",
      downloadsGrouping: "day",
    }, current.data.revision);
    expect(updated).toMatchObject({
      ok: true,
      data: { autoFindGrouping: "artist", downloadsGrouping: "day" },
    });
    if (!updated.ok) return;
    expect(JSON.parse(window.localStorage.getItem("atsumi.browser.settings.v1") ?? "{}"))
      .toMatchObject({ autoFindGrouping: "artist", downloadsGrouping: "day" });

    await expect(backend.settingsUpdate(
      { autoFindGrouping: "invalid" as "all" },
      updated.data.revision,
    )).resolves.toMatchObject({
      ok: false,
      error: { code: "VALIDATION_ERROR", details: { field: "autoFindGrouping" } },
    });
    await expect(backend.settingsUpdate({
      autoFindGrouping: current.data.autoFindGrouping,
      downloadsGrouping: current.data.downloadsGrouping,
    }, updated.data.revision)).resolves.toMatchObject({ ok: true });
  });

  it("persists detail and compact gallery modes independently per view", async () => {
    const current = await backend.settingsGet();
    if (!current.ok) throw new Error(current.error.message);

    const updated = await backend.settingsUpdate({
      exploreDisplayMode: "compact",
      autoFindDisplayMode: "detail",
      downloadsDisplayMode: "compact",
    }, current.data.revision);
    expect(updated).toMatchObject({
      ok: true,
      data: {
        exploreDisplayMode: "compact",
        autoFindDisplayMode: "detail",
        downloadsDisplayMode: "compact",
      },
    });
    if (!updated.ok) return;
    expect(JSON.parse(window.localStorage.getItem("atsumi.browser.settings.v1") ?? "{}"))
      .toMatchObject({
        exploreDisplayMode: "compact",
        autoFindDisplayMode: "detail",
        downloadsDisplayMode: "compact",
      });

    await expect(backend.settingsUpdate(
      { downloadsDisplayMode: "dense" as "detail" },
      updated.data.revision,
    )).resolves.toMatchObject({
      ok: false,
      error: { code: "VALIDATION_ERROR", details: { field: "downloadsDisplayMode" } },
    });

    await expect(backend.settingsUpdate({
      exploreDisplayMode: current.data.exploreDisplayMode,
      autoFindDisplayMode: current.data.autoFindDisplayMode,
      downloadsDisplayMode: current.data.downloadsDisplayMode,
    }, updated.data.revision)).resolves.toMatchObject({ ok: true });
  });

  it("persists normalized global search rules and rejects overlap", async () => {
    const current = await backend.settingsGet();
    if (!current.ok) throw new Error(current.error.message);
    const updated = await backend.settingsUpdate({
      searchIncludeTags: [" Female:Glasses ", "female:glasses", "webtoon"],
      searchExcludeTags: ["male:glasses"],
    }, current.data.revision);
    expect(updated).toMatchObject({
      ok: true,
      data: {
        searchIncludeTags: ["female:glasses", "webtoon"],
        searchExcludeTags: ["male:glasses"],
      },
    });
    if (!updated.ok) return;
    await expect(backend.settingsUpdate({
      searchExcludeTags: ["female:glasses"],
    }, updated.data.revision)).resolves.toMatchObject({
      ok: false,
      error: { code: "VALIDATION_ERROR", details: { field: "searchIncludeTags" } },
    });
    await expect(backend.settingsUpdate({
      searchIncludeTags: current.data.searchIncludeTags,
      searchExcludeTags: current.data.searchExcludeTags,
    }, updated.data.revision)).resolves.toMatchObject({ ok: true });
  });

  it("rejects unsafe folder templates and requires the gallery id token", async () => {
    const current = await backend.settingsGet();
    if (!current.ok) throw new Error(current.error.message);

    for (const folderNameTemplate of ["{title}", "{unknown} {id}", "{title {id}", "x\ny {id}"]) {
      const result = await backend.settingsUpdate({ folderNameTemplate }, current.data.revision);
      expect(result).toMatchObject({
        ok: false,
        error: { code: "VALIDATION_ERROR", details: { field: "folderNameTemplate" } },
      });
    }
  });

  it("keeps browser settings paths human-readable and uses fixed Rust-preview fixtures", async () => {
    const current = await backend.settingsGet();
    if (!current.ok) throw new Error(current.error.message);
    const updated = await backend.settingsUpdate(
      { downloadRoot: "\\\\?\\D:\\AD" },
      current.data.revision,
    );
    expect(updated).toMatchObject({ ok: true, data: { downloadRoot: "D:\\AD" } });
    const loaded = await backend.settingsGet();
    expect(loaded).toMatchObject({ ok: true, data: { downloadRoot: "D:\\AD" } });

    await expect(backend.folderNameTemplatePreview("[{artist}] {title} [{group}] {id}"))
      .resolves.toEqual({ ok: true, data: "[작가] 작품 제목 [그룹] 4113714" });
    await expect(backend.folderNameTemplatePreview("{title}:<{artist}>* [{group}] {id}"))
      .resolves.toEqual({ ok: true, data: "작품 제목__작가__ [그룹] 4113714" });
  });
});

describe("browser backend search contract", () => {
  it("applies saved rules to the effective query without baking them into history", async () => {
    const original = await backend.settingsGet();
    if (!original.ok) throw new Error(original.error.message);
    try {
      const baselineSettings = await backend.settingsUpdate({
        searchIncludeTags: [],
        searchExcludeTags: [],
      }, original.data.revision);
      if (!baselineSettings.ok) throw new Error(baselineSettings.error.message);
      const request = searchRequest({ text: "global-rule-history-contract" });
      const baseline = await backend.searchSubmit(request);
      const ruledSettings = await backend.settingsUpdate({
        searchIncludeTags: ["female:glasses"],
        searchExcludeTags: ["full_color"],
      }, baselineSettings.data.revision);
      if (!ruledSettings.ok) throw new Error(ruledSettings.error.message);
      const ruled = await backend.searchSubmit(request);
      expect(baseline.ok && ruled.ok && baseline.data.queryId).not.toBe(ruled.ok ? ruled.data.queryId : "");
      const history = await backend.searchHistoryList(100);
      expect(history.ok && history.data.find((entry) => entry.text === request.text)).toMatchObject({
        includeTags: [],
        excludeTags: [],
        useCount: 2,
      });
    } finally {
      const latest = await backend.settingsGet();
      if (latest.ok) {
        await backend.settingsUpdate({
          searchIncludeTags: original.data.searchIncludeTags,
          searchExcludeTags: original.data.searchExcludeTags,
        }, latest.data.revision);
      }
    }
  });

  it("reuses a canonical query key and returns deterministic Recent pages", async () => {
    const first = await backend.searchSubmit(searchRequest({
      text: "  ARCHIVE  ",
      includeTags: [" FULL_COLOR ", "MYSTERY"],
      languages: ["english", "korean", "japanese", "korean"],
    }));
    const repeated = await backend.searchSubmit(searchRequest({
      text: "archive",
      includeTags: ["mystery", "full_color"],
      languages: ["japanese", "english", "korean"],
    }));

    expect(first.ok).toBe(true);
    expect(repeated.ok).toBe(true);
    if (!first.ok || !repeated.ok) return;
    expect(repeated.data.queryId).toBe(first.data.queryId);
    expect(first.data.queryId).toBe("fixture-f68ffad46ba6b7569bf724ae0776c47d");
    expect(first.data.firstPage.items.map((item) => item.title)).toEqual(["Archive of Rain"]);
  });

  it("serves later pages from the submitted query session", async () => {
    const submitted = await backend.searchSubmit(searchRequest({ pageSize: 2 }));
    if (!submitted.ok) throw new Error(submitted.error.message);

    const second = await backend.searchPageGet(submitted.data.queryId, 2, "page-request-2");
    expect(second.ok).toBe(true);
    if (!second.ok) return;
    expect(second.data.page).toBe(2);
    expect(second.data.items).toHaveLength(1);
    expect(second.data.items[0]?.id).not.toBe(submitted.data.firstPage.items[0]?.id);

    const outside = await backend.searchPageGet(submitted.data.queryId, 3, "page-request-3");
    expect(outside).toMatchObject({ ok: false, error: { code: "VALIDATION_ERROR" } });
  });

  it("returns structured errors for unknown queries and galleries", async () => {
    const page = await backend.searchPageGet("missing-query", 1, "page-request-missing");
    const detail = await backend.galleryDetailGet(galleryId(999));

    expect(page).toMatchObject({ ok: false, error: { code: "QUERY_NOT_FOUND" } });
    expect(detail).toMatchObject({ ok: false, error: { code: "SOURCE_NOT_FOUND" } });
  });

  it("returns tags and related summaries from the detail fixture", async () => {
    const result = await backend.galleryDetailGet(galleryId(4051038));

    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.data.tags).toContain("female:glasses");
    expect(result.data.series).toEqual(["rain archives"]);
    expect(result.data.characters).toEqual(["mira lane", "ren kujo"]);
    expect(result.data.related).toHaveLength(2);
    expect(result.data.related.every((item) => item.id !== result.data.id)).toBe(true);
  });

  it("searches artist and group autocomplete catalogs with namespace-aware favorites", async () => {
    const artist = await backend.tagSuggestionsSearch({ query: "miz", namespace: "artist", limit: 8 });
    const group = await backend.tagSuggestionsSearch({ query: "circle_en", namespace: "group", limit: 8 });
    const mixed = await backend.tagSuggestionsSearch({ query: "mizuryu", limit: 8 });

    expect(artist).toMatchObject({ ok: true, data: expect.arrayContaining([
      expect.objectContaining({ namespace: "artist", token: "artist:mizuno_tooru" }),
    ]) });
    expect(group).toEqual({ ok: true, data: [expect.objectContaining({
      namespace: "group",
      token: "group:circle_energy",
    })] });
    expect(mixed).toMatchObject({ ok: true, data: expect.arrayContaining([
      expect.objectContaining({ namespace: "artist", token: "artist:mizuryu_kei" }),
      expect.objectContaining({ namespace: "group", token: "group:mizuryu_kei_land" }),
    ]) });

    await backend.favoriteSet({ namespace: "artist", value: "mizuno tooru" }, true);
    const favorite = await backend.tagSuggestionsSearch({ query: "miz", namespace: "artist", limit: 8 });
    expect(favorite).toMatchObject({ ok: true, data: [
      expect.objectContaining({ token: "artist:mizuno_tooru", favorite: true }),
      expect.objectContaining({ token: "artist:mizuryu_kei", favorite: false }),
    ] });
    await backend.favoriteSet({ namespace: "artist", value: "mizuno tooru" }, false);
  });

  it.each([
    ["series:rain_archives", [galleryId(4051038), galleryId(4050754)]],
    ["character:mira_lane", [galleryId(4051038), galleryId(4050754)]],
    ["group:nocturne_circle", [galleryId(4051038), galleryId(4050754)]],
    ["group:nocturne\\_circle", [galleryId(4051038), galleryId(4050754)]],
  ])("routes atomic %s metadata tokens in the browser fixture", async (text, expectedIds) => {
    const result = await backend.searchSubmit(searchRequest({ text, pageSize: 20 }));
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.data.firstPage.items.map((item) => item.id)).toEqual(expectedIds);
  });
});

describe("browser backend favorites and automation contract", () => {
  it("persists normalized favorites and only records submitted non-empty searches", async () => {
    const enabled = await backend.favoriteSet({ namespace: "artist", value: "  History Artist  " }, true);
    await backend.favoriteSet({ namespace: "series", value: " Rain Archives " }, true);
    await backend.favoriteSet({ namespace: "character", value: " Mira Lane " }, true);
    expect(enabled).toMatchObject({
      ok: true,
      data: { enabled: true, favorite: { namespace: "artist", value: "history artist", revision: 0 } },
    });
    const favorites = await backend.favoritesList();
    expect(favorites).toMatchObject({ ok: true, data: expect.arrayContaining([
      expect.objectContaining({ namespace: "artist", value: "history artist" }),
      expect.objectContaining({ namespace: "series", value: "rain archives" }),
      expect.objectContaining({ namespace: "character", value: "mira lane" }),
    ]) });

    await backend.searchSubmit(searchRequest());
    await backend.searchSubmit(searchRequest({
      text: "history-contract",
      includeTags: ["female:glasses"],
      excludeTags: ["male:suit"],
      languages: ["english", "korean"],
      sort: "popular_week",
      pageSize: 17,
    }));
    await backend.searchSubmit(searchRequest({
      text: " HISTORY-CONTRACT ",
      includeTags: ["FEMALE:GLASSES"],
      excludeTags: ["MALE:SUIT"],
      languages: ["korean", "english"],
      sort: "popular_week",
      pageSize: 17,
    }));
    const history = await backend.searchHistoryList(100);
    expect(history.ok).toBe(true);
    if (!history.ok) return;
    const entry = history.data.find((item) => item.text === "history-contract");
    expect(entry).toMatchObject({
      includeTags: ["female:glasses"],
      excludeTags: ["male:suit"],
      languages: ["korean", "english"],
      sort: "popular_week",
      pageSize: 17,
      useCount: 2,
    });
    expect(history.data.some((item) => !item.text && !item.includeTags.length && !item.excludeTags.length)).toBe(false);

    await backend.favoriteSet({ namespace: "artist", value: "history artist" }, false);
    await backend.favoriteSet({ namespace: "series", value: "rain archives" }, false);
    await backend.favoriteSet({ namespace: "character", value: "mira lane" }, false);
    const removed = await backend.favoritesList();
    expect(removed.ok && removed.data.some((item) => item.value === "history artist")).toBe(false);
  });

  it("preserves partial candidates on cancel and excludes them from later explicit refreshes", async () => {
    vi.useFakeTimers();
    const events: string[] = [];
    const unsubscribe = await backend.on("auto-find:changed", (run) => events.push(run.state));
    let seededDownloadEntryId: string | undefined;
    try {
      await backend.favoriteSet({ namespace: "artist", value: "serein" }, true);
      await backend.favoriteSet({ namespace: "artist", value: "mizuno" }, true);
      const seededDownload = await backend.downloadQueueAdd([galleryId(4051038)], "auto-find-downloaded-gallery");
      if (seededDownload.ok) seededDownloadEntryId = seededDownload.data[0]?.entryId;

      const started = await backend.autoFindRefresh();
      expect(started).toMatchObject({ ok: true, data: { state: "running", totalFavorites: 2, historyMode: "include_all_history" } });
      await vi.advanceTimersByTimeAsync(60);
      const partial = await backend.autoFindSnapshot();
      expect(partial).toMatchObject({
        ok: true,
        data: {
          run: { state: "running", completedFavorites: 1 },
          candidates: [expect.objectContaining({ id: galleryId(4050754), artist: "serein" })],
        },
      });

      const cancelled = await backend.autoFindCancel();
      expect(cancelled).toMatchObject({ ok: true, data: { state: "cancelled" } });
      await vi.advanceTimersByTimeAsync(500);
      const preserved = await backend.autoFindSnapshot();
      expect(preserved).toMatchObject({
        ok: true,
        data: { run: { state: "cancelled" }, candidates: [{ id: galleryId(4050754) }] },
      });

      await backend.autoFindRefresh();
      await vi.advanceTimersByTimeAsync(120);
      const completed = await backend.autoFindSnapshot();
      expect(completed).toMatchObject({ ok: true, data: { run: { state: "completed", completedFavorites: 2 } } });
      if (!completed.ok) return;
      expect(completed.data.candidates.map((candidate) => candidate.id)).not.toContain(galleryId(4051038));

      const excludedId = completed.data.candidates[0]!.id;
      const excluded = await backend.autoFindExclude([excludedId], "focused browser contract test");
      expect(excluded).toMatchObject({ ok: true, data: { excludedGalleryIds: [excludedId] } });
      await backend.autoFindRefresh();
      await vi.advanceTimersByTimeAsync(120);
      const refreshed = await backend.autoFindSnapshot();
      expect(refreshed.ok && refreshed.data.candidates.some((candidate) => candidate.id === excludedId)).toBe(false);
      const managed = await backend.explorationExclusionsList();
      expect(managed.ok).toBe(true);
      if (!managed.ok) return;
      expect(managed.data.find((item) => item.galleryId === excludedId)).toMatchObject({
        galleryId: excludedId,
        reasons: [{ kind: "manual", detail: "focused browser contract test" }],
      });
      const restored = await backend.explorationExclusionsRestore([excludedId]);
      expect(restored).toMatchObject({
        ok: true,
        data: { restoredGalleryIds: [excludedId] },
      });
      if (restored.ok) {
        expect(restored.data.snapshot.candidates.some((candidate) => candidate.id === excludedId)).toBe(true);
      }
      const afterRestore = await backend.explorationExclusionsList();
      expect(afterRestore.ok && afterRestore.data.some((item) => item.galleryId === excludedId)).toBe(false);
      expect(events).toContain("cancelled");
      expect(events).toContain("completed");
    } finally {
      unsubscribe();
      if (seededDownloadEntryId) await backend.downloadCancel([seededDownloadEntryId]);
      await backend.favoriteSet({ namespace: "artist", value: "serein" }, false);
      await backend.favoriteSet({ namespace: "artist", value: "mizuno" }, false);
      vi.useRealTimers();
    }
  });

  it("clears only cache or exploration state and keeps download records", async () => {
    await backend.favoriteSet({ namespace: "artist", value: "reset fixture" }, true);
    await backend.searchSubmit(searchRequest({ text: "reset fixture" }));
    const queued = await backend.downloadQueueAdd([galleryId(7_199_991)], "reset-keeps-download");
    if (!queued.ok) throw new Error(queued.error.message);

    expect(await backend.thumbnailCacheClear()).toMatchObject({
      ok: true,
      data: {
        successEntriesRemoved: 0,
        negativeEntriesRemoved: 0,
      },
    });
    const reset = await backend.explorationDataReset({
      confirmation: "RESET_EXPLORATION_DATA",
    });
    expect(reset).toMatchObject({
      ok: true,
      data: {
        favoritesRemoved: expect.any(Number),
        searchHistoryRemoved: expect.any(Number),
      },
    });
    expect(await backend.favoritesList()).toEqual({ ok: true, data: [] });
    expect(await backend.searchHistoryList(100)).toEqual({ ok: true, data: [] });
    expect(await backend.downloadEntriesList({
      query: queued.data[0]!.entryId,
      page: 1,
      pageSize: 20,
    })).toMatchObject({ ok: true, data: { totalItems: 1 } });
    await backend.downloadCancel([queued.data[0]!.entryId]);
  });
});

describe("browser backend download contract", () => {
  it("does not expose a fake local storage folder in browser review mode", async () => {
    const result = await backend.artifactOpenFolder("browser-entry-folder");
    expect(result).toMatchObject({
      ok: false,
      error: {
        code: "ARTIFACT_FOLDER_UNAVAILABLE_IN_BROWSER",
        retryable: false,
        action: "none",
      },
    });
  });

  it("persists idempotent cancellation and retries the same entry", async () => {
    const gallery = galleryId(7_100_000);
    const queued = await backend.downloadQueueAdd([gallery], "queue-cancel-retry-request");
    if (!queued.ok) throw new Error(queued.error.message);
    const entry = queued.data[0]!;

    const cancelled = await backend.downloadCancel([entry.entryId]);
    const cancelReplay = await backend.downloadCancel([entry.entryId]);
    expect(cancelled).toMatchObject({
      ok: true,
      data: [{ entryId: entry.entryId, state: "cancelled", revision: 1 }],
    });
    expect(cancelReplay).toEqual(cancelled);
    await expect(backend.downloadEntriesList({
      state: "cancelled",
      query: entry.entryId,
      page: 1,
      pageSize: 20,
    })).resolves.toMatchObject({
      ok: true,
      data: { totalItems: 1, entries: [{ entryId: entry.entryId, state: "cancelled" }] },
    });

    const retried = await backend.downloadRetry([entry.entryId]);
    expect(retried).toEqual({
      ok: true,
      data: [{ jobId: `browser-fixture-${entry.entryId}`, reused: false }],
    });
    const current = await backend.downloadEntriesList({
      query: entry.entryId,
      page: 1,
      pageSize: 20,
    });
    expect(current).toMatchObject({
      ok: true,
      data: { entries: [{ entryId: entry.entryId, state: "queued", revision: 2 }] },
    });
    await backend.downloadCancel([entry.entryId]);
  });

  it("does not let an old fixture timer advance a retried attempt", async () => {
    vi.useFakeTimers();
    const unsubscribe = await backend.on("download:changed", () => undefined);
    try {
      const gallery = galleryId(7_100_101);
      const queued = await backend.downloadQueueAdd([gallery], "queue-stale-browser-worker");
      if (!queued.ok) throw new Error(queued.error.message);
      const entry = queued.data[0]!;

      await vi.advanceTimersByTimeAsync(75);
      await backend.downloadCancel([entry.entryId]);
      const retried = await backend.downloadRetry([entry.entryId]);
      expect(retried).toMatchObject({ ok: true, data: [{ reused: false }] });

      await vi.advanceTimersByTimeAsync(150);
      const whileOldCompletionFires = await backend.downloadEntriesList({
        query: entry.entryId,
        page: 1,
        pageSize: 20,
      });
      expect(whileOldCompletionFires).toMatchObject({
        ok: true,
        data: {
          entries: [{
            entryId: entry.entryId,
            state: "resolving_metadata",
            attempt: 2,
          }],
        },
      });

      await vi.advanceTimersByTimeAsync(75);
      const currentAttemptCompletion = await backend.downloadEntriesList({
        query: entry.entryId,
        page: 1,
        pageSize: 20,
      });
      expect(currentAttemptCompletion).toMatchObject({
        ok: true,
        data: {
          entries: [{
            entryId: entry.entryId,
            state: "interrupted",
            attempt: 2,
            errorCode: "DOWNLOAD_FOUNDATION_UNAVAILABLE",
          }],
        },
      });
    } finally {
      unsubscribe();
      vi.useRealTimers();
    }
  });

  it("replays the original entries for the same request ID and normalized gallery set", async () => {
    const firstGallery = galleryId(7_100_002);
    const secondGallery = galleryId(7_100_001);
    const first = await backend.downloadQueueAdd(
      [firstGallery, secondGallery, firstGallery],
      " queue-replay-request ",
    );
    expect(await backend.appActiveWorkSnapshot()).toMatchObject({
      ok: true,
      data: { downloads: { activeCount: 2 } },
    });
    expect(first.ok).toBe(true);
    if (!first.ok) return;
    const browserState = backend as unknown as { downloadEntries: Map<string, DownloadEntry> };
    const current = first.data[0]!;
    browserState.downloadEntries.set(current.entryId, {
      ...current,
      state: "downloading",
      progress: 47,
    });
    const replay = await backend.downloadQueueAdd(
      [secondGallery, firstGallery],
      "queue-replay-request",
    );

    expect(replay.ok).toBe(true);
    if (!replay.ok) return;
    expect(first.data.map((entry) => entry.galleryId)).toEqual([secondGallery, firstGallery]);
    expect(replay.data).toEqual(first.data);
  });

  it("rejects reuse of a request ID for a different normalized gallery set", async () => {
    const firstGallery = galleryId(7_100_011);
    const secondGallery = galleryId(7_100_012);
    const queued = await backend.downloadQueueAdd([firstGallery], "queue-conflict-request");
    const conflict = await backend.downloadQueueAdd([firstGallery, secondGallery], "queue-conflict-request");

    expect(queued.ok).toBe(true);
    expect(conflict).toMatchObject({
      ok: false,
      error: { code: "IDEMPOTENCY_CONFLICT", details: { requestId: "queue-conflict-request" } },
    });
  });

  it("validates the original gallery input length before deduplication", async () => {
    const repeated = Array.from({ length: 201 }, () => galleryId(7_100_015));
    const result = await backend.downloadQueueAdd(repeated, "queue-too-many-request");

    expect(result).toMatchObject({
      ok: false,
      error: {
        code: "VALIDATION_ERROR",
        details: { field: "galleries", reason: "must contain at most 200 IDs" },
      },
    });
  });

  it("reuses an existing entry in each of the six active states for a new request ID", async () => {
    const states: DownloadEntry["state"][] = [
      "queued",
      "resolving_metadata",
      "downloading",
      "hashing",
      "verifying",
      "retry_wait",
    ];
    const browserState = backend as unknown as { downloadEntries: Map<string, DownloadEntry> };

    for (const [index, state] of states.entries()) {
      const shared = galleryId(7_100_021 + index);
      const first = await backend.downloadQueueAdd([shared], `queue-active-first-${state}`);
      if (!first.ok) throw new Error(first.error.message);
      const entry = first.data[0]!;
      browserState.downloadEntries.set(entry.entryId, { ...entry, state, progress: 23 });

      const second = await backend.downloadQueueAdd([shared], `queue-active-second-${state}`);
      expect(second).toMatchObject({
        ok: true,
        data: [{ entryId: entry.entryId, galleryId: shared, state, progress: 23 }],
      });
    }
  });

  it("filters and paginates queued entries deterministically", async () => {
    const query = "710003";
    const galleries = [
      galleryId(7_100_033),
      galleryId(7_100_031),
      galleryId(7_100_034),
      galleryId(7_100_032),
    ];
    const queued = await backend.downloadQueueAdd(galleries, "queue-list-request");
    if (!queued.ok) throw new Error(queued.error.message);

    const first = await backend.downloadEntriesList({ state: "queued", query, page: 1, pageSize: 2 });
    const second = await backend.downloadEntriesList({ state: "queued", query, page: 2, pageSize: 2 });
    const completed = await backend.downloadEntriesList({ state: "completed", query, page: 1, pageSize: 2 });

    expect(first.ok).toBe(true);
    expect(second.ok).toBe(true);
    expect(completed.ok).toBe(true);
    if (!first.ok || !second.ok || !completed.ok) return;
    expect(first.data.totalItems).toBe(4);
    expect(first.data.entries.map((entry) => entry.galleryId)).toEqual([galleryId(7_100_031), galleryId(7_100_032)]);
    expect(second.data.entries.map((entry) => entry.galleryId)).toEqual([galleryId(7_100_033), galleryId(7_100_034)]);
    expect(completed.data).toMatchObject({ totalItems: 0, entries: [] });
  });

  it("projects locally known gallery metadata with each download entry", async () => {
    const knownGallery = galleryId(4_050_642);
    const queued = await backend.downloadQueueAdd([knownGallery], "queue-library-projection");
    if (!queued.ok) throw new Error(queued.error.message);

    const result = await backend.downloadLibraryPageList({
      query: "blue lane",
      page: 1,
      pageSize: 20,
    });

    expect(result).toMatchObject({
      ok: true,
      data: {
        totalItems: 1,
        items: [{
          gallery: {
            id: knownGallery,
            title: "Blue Lane",
            artist: "mizuno",
            pages: 48,
            language: "english",
            publishedRank: 20260809,
          },
          download: {
            entryId: queued.data[0]!.entryId,
            galleryId: knownGallery,
          },
        }],
      },
    });
    await backend.downloadCancel([queued.data[0]!.entryId]);
  });

  it("projects one canonical latest entry for a gallery with download history", async () => {
    const target = galleryId(7_100_041);
    const browserState = backend as unknown as { downloadEntries: Map<string, DownloadEntry> };
    const first = await backend.downloadQueueAdd([target], "queue-library-canonical-first");
    if (!first.ok) throw new Error(first.error.message);
    const older = first.data[0]!;
    browserState.downloadEntries.set(older.entryId, {
      ...older,
      state: "completed",
      progress: 100,
      createdAt: "2026-08-01T00:00:00.000Z",
      updatedAt: "2026-08-01T00:00:00.000Z",
    });

    const second = await backend.downloadQueueAdd([target], "queue-library-canonical-second");
    if (!second.ok) throw new Error(second.error.message);
    const newer = second.data[0]!;
    browserState.downloadEntries.set(newer.entryId, {
      ...newer,
      createdAt: "2026-08-02T00:00:00.000Z",
      updatedAt: "2026-08-02T00:00:00.000Z",
    });

    const result = await backend.downloadLibraryPageList({
      query: String(target),
      page: 1,
      pageSize: 20,
    });
    expect(result).toMatchObject({
      ok: true,
      data: {
        totalItems: 1,
        items: [{ download: { entryId: newer.entryId, galleryId: target } }],
      },
    });
  });

  it("rejects list queries longer than 500 UTF-8 bytes after normalization", async () => {
    const result = await backend.downloadEntriesList({
      query: `  ${"가".repeat(167)}  `,
      page: 1,
      pageSize: 20,
    });

    expect(result).toMatchObject({
      ok: false,
      error: {
        code: "VALIDATION_ERROR",
        details: { field: "query", reason: "must be at most 500 bytes" },
      },
    });
  });

  it("ends the review fixture at interrupted without manufacturing a completed artifact", async () => {
    vi.useFakeTimers();
    const states: DownloadEntry["state"][] = [];
    const unsubscribe = await backend.on("download:changed", (entry) => states.push(entry.state));
    try {
      const gallery = galleryId(7_100_099);
      const queued = await backend.downloadQueueAdd([gallery], "queue-safe-fixture-request");
      expect(queued).toMatchObject({ ok: true, data: [{ galleryId: gallery, revision: 0, state: "queued" }] });

      await vi.advanceTimersByTimeAsync(225);

      expect(states).toEqual(["resolving_metadata", "interrupted"]);
      const interrupted = await backend.downloadEntriesList({
        state: "interrupted",
        query: String(gallery),
        page: 1,
        pageSize: 20,
      });
      expect(interrupted).toMatchObject({
        ok: true,
        data: {
          entries: [{
            galleryId: gallery,
            revision: 2,
            state: "interrupted",
            progress: 0,
            attempt: 1,
            errorCode: "DOWNLOAD_FOUNDATION_UNAVAILABLE",
          }],
        },
      });
    } finally {
      unsubscribe();
      vi.useRealTimers();
    }
  });
});

describe("browser backend duplicate review contract", () => {
  it("persists scan progress, cancellation, deterministic evidence, and revision-CAS decisions", async () => {
    vi.useFakeTimers();
    const events: string[] = [];
    const unsubscribe = await backend.on("duplicate:changed", (run) => events.push(`${run.state}:${run.revision}`));
    try {
      const initial = await backend.duplicateSnapshot();
      expect(initial).toMatchObject({
        ok: true,
        data: {
          profile: { profileVersion: 1, dHashBits: 1024, pHashBits: 64 },
          candidates: [],
        },
      });

      const started = await backend.duplicateScanStart();
      expect(started).toMatchObject({ ok: true, data: { state: "running", totalArtifacts: 2, totalPairs: 1 } });
      await vi.advanceTimersByTimeAsync(45);
      expect(await backend.duplicateSnapshot()).toMatchObject({
        ok: true,
        data: { run: { state: "running", hashedArtifacts: 1, comparedPairs: 0 } },
      });
      const cancelled = await backend.duplicateScanCancel();
      expect(cancelled).toMatchObject({ ok: true, data: { state: "cancelled" } });
      await vi.advanceTimersByTimeAsync(100);
      expect(await backend.duplicateSnapshot()).toMatchObject({
        ok: true,
        data: { run: { state: "cancelled" }, candidates: [] },
      });

      await backend.duplicateScanStart();
      await vi.advanceTimersByTimeAsync(100);
      const complete = await backend.duplicateSnapshot();
      expect(complete).toMatchObject({
        ok: true,
        data: {
          run: { state: "completed", hashedArtifacts: 2, comparedPairs: 1, candidatesFound: 1 },
          candidates: [{
            candidateId: "browser-duplicate-archive-tram",
            relation: "contains",
            confidence: 0.94,
          }],
        },
      });
      expect(events).toEqual(expect.arrayContaining(["cancelled:2", "completed:2"]));

      const reviewResult = await backend.duplicateReviewGet("browser-duplicate-archive-tram");
      if (!reviewResult.ok) throw new Error(reviewResult.error.message);
      expect(reviewResult.data).toMatchObject({
        evidence: expect.arrayContaining([
          expect.objectContaining({ kind: "exact_sha256" }),
          expect.objectContaining({ kind: "visual_hash" }),
          expect.objectContaining({ kind: "sequence_alignment" }),
        ]),
        pagePairs: [
          expect.objectContaining({
            parentSourcePage: 1,
            candidateSourcePage: 3,
            exactSha256: true,
            detailHashDistance: 0,
            edgeSimilarity: 1,
          }),
          expect.any(Object),
          expect.any(Object),
        ],
      });

      const stale = await backend.duplicateDecisionApply({
        candidateId: reviewResult.data.candidate.candidateId,
        expectedRevision: 99,
        action: "hide_parent",
      });
      expect(stale).toMatchObject({
        ok: false,
        error: {
          code: "REVISION_CONFLICT",
          details: { resource: "duplicateCandidate", expectedRevision: 99, actualRevision: 0 },
        },
      });

      const linked = await backend.duplicateDecisionApply({
        candidateId: reviewResult.data.candidate.candidateId,
        expectedRevision: 0,
        action: "series_link",
        seriesName: "Rain sequence",
      });
      if (!linked.ok) throw new Error(linked.error.message);
      expect(linked.data).toMatchObject({
        candidate: { revision: 1 },
        seriesGroups: [{ name: "Rain sequence", members: [{ galleryId: galleryId(4051038) }, { galleryId: galleryId(4050754) }] }],
        decisions: [{ action: "series_link", candidateRevision: 1 }],
      });
      const groupId = linked.data.seriesGroups[0]!.seriesGroupId;

      const unlinked = await backend.duplicateDecisionApply({
        candidateId: linked.data.candidate.candidateId,
        expectedRevision: 1,
        action: "series_unlink",
        targetGalleryId: galleryId(4051038),
        seriesGroupId: groupId,
      });
      if (!unlinked.ok) throw new Error(unlinked.error.message);
      expect(unlinked.data.seriesGroups[0]?.members.map((member) => member.galleryId)).toEqual([galleryId(4050754)]);

      const excluded = await backend.duplicateDecisionApply({
        candidateId: unlinked.data.candidate.candidateId,
        expectedRevision: 2,
        action: "exclude_pair",
      });
      if (!excluded.ok) throw new Error(excluded.error.message);
      expect(excluded.data.decisions.at(-1)).toMatchObject({ action: "exclude_pair", candidateRevision: 3 });
      expect(await backend.duplicateSnapshot()).toMatchObject({ ok: true, data: { candidates: [] } });
      const exclusionsAfterPairDecision = await backend.explorationExclusionsList();
      if (!exclusionsAfterPairDecision.ok) throw new Error(exclusionsAfterPairDecision.error.message);
      expect(exclusionsAfterPairDecision.data.some((item) =>
        item.galleryId === excluded.data.candidate.parent.galleryId
        || item.galleryId === excluded.data.candidate.candidate.galleryId,
      )).toBe(false);

      const hiddenCandidate = await backend.duplicateDecisionApply({
        candidateId: excluded.data.candidate.candidateId,
        expectedRevision: 3,
        action: "hide_candidate",
      });
      if (!hiddenCandidate.ok) throw new Error(hiddenCandidate.error.message);
      const exclusionsAfterHide = await backend.explorationExclusionsList();
      if (!exclusionsAfterHide.ok) throw new Error(exclusionsAfterHide.error.message);
      expect(exclusionsAfterHide.data.find((item) =>
        item.galleryId === hiddenCandidate.data.candidate.parent.galleryId,
      )).toBeUndefined();
      expect(exclusionsAfterHide.data.find((item) =>
        item.galleryId === hiddenCandidate.data.candidate.candidate.galleryId,
      )).toMatchObject({ reasons: [{ kind: "duplicate_hidden" }] });
      const hiddenParent = await backend.duplicateDecisionApply({
        candidateId: hiddenCandidate.data.candidate.candidateId,
        expectedRevision: 4,
        action: "hide_parent",
      });
      expect(hiddenParent).toMatchObject({ ok: true, data: { candidate: { revision: 5 } } });
      const visibleDownloads = await backend.downloadEntriesList({ page: 1, pageSize: 200 });
      if (!visibleDownloads.ok) throw new Error(visibleDownloads.error.message);
      expect(visibleDownloads.data.entries.some((entry) =>
        entry.galleryId === galleryId(4051038) || entry.galleryId === galleryId(4050754),
      )).toBe(false);

      await backend.favoriteSet({ namespace: "artist", value: "serein" }, true);
      await backend.autoFindRefresh();
      await vi.advanceTimersByTimeAsync(100);
      const autoFind = await backend.autoFindSnapshot();
      if (!autoFind.ok) throw new Error(autoFind.error.message);
      expect(autoFind.data.candidates.some((candidate) =>
        candidate.id === galleryId(4051038) || candidate.id === galleryId(4050754),
      )).toBe(false);
    } finally {
      await backend.favoriteSet({ namespace: "artist", value: "serein" }, false);
      await backend.explorationExclusionsRestore([galleryId(4051038), galleryId(4050754)]);
      unsubscribe();
      vi.useRealTimers();
    }
  });
});

describe("browser backend internal duplicate contract", () => {
  it("persists exact source-page evidence and keeps quarantine undoable with revision checks", async () => {
    vi.useFakeTimers();
    const events: string[] = [];
    const artifactEvents: InternalArtifactScanProgress[] = [];
    const unsubscribe = await backend.on("internal-duplicate:changed", (run) => events.push(`${run.state}:${run.revision}`));
    const unsubscribeArtifact = await backend.on("internal-duplicate:artifact-progress", (progress) => artifactEvents.push(progress));
    try {
      const empty = await backend.internalDuplicateScanStart({ entryIds: [] });
      expect(empty).toMatchObject({ ok: false, error: { code: "VALIDATION_ERROR" } });
      const started = await backend.internalDuplicateScanStart({
        entryIds: ["browser-artifact-4051038"],
      });
      expect(started).toMatchObject({ ok: true, data: { state: "running", totalArtifacts: 1 } });
      const sameSelection = await backend.internalDuplicateScanStart({
        entryIds: ["browser-artifact-4051038"],
      });
      if (!started.ok) throw new Error(started.error.message);
      expect(sameSelection).toMatchObject({ ok: true, data: { runId: started.data.runId } });
      const otherSelection = await backend.internalDuplicateScanStart({
        entryIds: ["browser-artifact-other"],
      });
      expect(otherSelection).toMatchObject({ ok: false, error: { code: "OPERATION_ACTIVE" } });
      const hashing = await backend.internalDuplicateActiveArtifact();
      expect(hashing).toMatchObject({
        ok: true,
        data: {
          runId: started.data.runId,
          entryId: "browser-artifact-4051038",
          artifactIndex: 1,
          totalArtifacts: 1,
          sequence: 1,
          stage: "hashing",
          progressPercent: 0,
        },
      });

      await vi.advanceTimersByTimeAsync(35);
      const comparing = await backend.internalDuplicateActiveArtifact();
      expect(comparing).toMatchObject({
        ok: true,
        data: {
          runId: started.data.runId,
          sequence: 2,
          stage: "comparing",
          processedPages: 24,
          comparedPairs: 138,
          totalPairs: 276,
          progressPercent: 65,
        },
      });

      await vi.advanceTimersByTimeAsync(25);
      const finalizing = await backend.internalDuplicateActiveArtifact();
      expect(finalizing).toMatchObject({
        ok: true,
        data: {
          runId: started.data.runId,
          sequence: 3,
          stage: "finalizing",
          comparedPairs: 276,
          progressPercent: 99,
        },
      });

      await vi.advanceTimersByTimeAsync(30);
      const snapshot = await backend.internalDuplicateSnapshot();
      if (!snapshot.ok) throw new Error(snapshot.error.message);
      expect(snapshot.data).toMatchObject({
        run: { state: "completed", scannedArtifacts: 1, groupsFound: 3 },
        groups: [
          { relation: "exact", pages: [{ sourcePage: 2 }, { sourcePage: 8 }] },
          { relation: "translation_visual", pages: [{ sourcePage: 14 }, { sourcePage: 20 }] },
          { relation: "translation_visual", pages: [{ sourcePage: 15 }, { sourcePage: 21 }] },
        ],
      });
      expect(events).toEqual(expect.arrayContaining(["running:0", "completed:2"]));
      expect(artifactEvents.map((event) => [event.sequence, event.stage])).toEqual([
        [1, "hashing"],
        [2, "comparing"],
        [3, "finalizing"],
      ]);
      await expect(backend.internalDuplicateActiveArtifact()).resolves.toEqual({ ok: true, data: null });

      const group = snapshot.data.groups[0]!;
      const stale = await backend.internalRemovalPlan({
        entryId: group.entryId,
        selections: [{
          groupId: group.groupId,
          expectedRevision: 99,
          keepSourcePage: 2,
          removeSourcePages: [8],
        }],
      });
      expect(stale).toMatchObject({ ok: false, error: { code: "REVISION_CONFLICT" } });

      const prepared = await backend.internalRemovalPlan({
        entryId: group.entryId,
        selections: [{
          groupId: group.groupId,
          expectedRevision: group.revision,
          keepSourcePage: 2,
          removeSourcePages: [8],
        }],
      });
      if (!prepared.ok) throw new Error(prepared.error.message);
      expect(prepared.data).toMatchObject({ filesToQuarantine: 1, bytesToQuarantine: 512_000 });
      const applied = await backend.internalRemovalApply({ plan: prepared.data, reason: "test review" });
      if (!applied.ok) throw new Error(applied.error.message);
      expect(applied.data.records).toEqual([
        expect.objectContaining({ sourcePage: 8, state: "quarantined" }),
      ]);
      expect(applied.data.review.groups.some((item) => item.groupId === group.groupId)).toBe(false);

      const restored = await backend.internalRemovalUndo({
        recordIds: applied.data.records.map((record) => record.recordId),
      });
      if (!restored.ok) throw new Error(restored.error.message);
      expect(restored.data.review.groups).toEqual(expect.arrayContaining([
        expect.objectContaining({ groupId: group.groupId, resolved: false }),
      ]));
      expect(restored.data.records).toEqual([
        expect.objectContaining({ sourcePage: 8, state: "restored" }),
      ]);
    } finally {
      unsubscribeArtifact();
      unsubscribe();
      vi.useRealTimers();
    }
  });

  it("clears active artifact progress on cancellation and suppresses queued updates", async () => {
    vi.useFakeTimers();
    const artifactEvents: InternalArtifactScanProgress[] = [];
    const unsubscribe = await backend.on("internal-duplicate:artifact-progress", (progress) => artifactEvents.push(progress));
    try {
      const started = await backend.internalDuplicateScanStart({ entryIds: ["browser-artifact-cancel"] });
      expect(started).toMatchObject({ ok: true, data: { state: "running" } });
      await vi.advanceTimersByTimeAsync(35);
      expect(artifactEvents.at(-1)).toMatchObject({ stage: "comparing", sequence: 2 });

      const cancelled = await backend.internalDuplicateScanCancel();
      expect(cancelled).toMatchObject({ ok: true, data: { state: "cancelled" } });
      await expect(backend.internalDuplicateActiveArtifact()).resolves.toEqual({ ok: true, data: null });

      const eventCountAfterCancel = artifactEvents.length;
      await vi.advanceTimersByTimeAsync(100);
      expect(artifactEvents).toHaveLength(eventCountAfterCancel);
      const snapshot = await backend.internalDuplicateSnapshot();
      expect(snapshot).toMatchObject({ ok: true, data: { run: { state: "cancelled" } } });
    } finally {
      unsubscribe();
      vi.useRealTimers();
    }
  });

  it("advances multi-artifact progress to each selected entry in deterministic order", async () => {
    vi.useFakeTimers();
    const artifactEvents: InternalArtifactScanProgress[] = [];
    const unsubscribe = await backend.on("internal-duplicate:artifact-progress", (progress) => artifactEvents.push(progress));
    try {
      const started = await backend.internalDuplicateScanStart({
        entryIds: ["browser-artifact-z", "browser-artifact-a"],
      });
      expect(started).toMatchObject({ ok: true, data: { state: "running", totalArtifacts: 2 } });
      await expect(backend.internalDuplicateActiveArtifact()).resolves.toMatchObject({
        ok: true,
        data: { entryId: "browser-artifact-a", artifactIndex: 1, totalArtifacts: 2, sequence: 1 },
      });

      await vi.advanceTimersByTimeAsync(85);
      await expect(backend.internalDuplicateActiveArtifact()).resolves.toMatchObject({
        ok: true,
        data: {
          entryId: "browser-artifact-z",
          artifactIndex: 2,
          totalArtifacts: 2,
          sequence: 4,
          stage: "hashing",
        },
      });
      expect(artifactEvents.map((event) => [event.entryId, event.artifactIndex, event.sequence])).toEqual([
        ["browser-artifact-a", 1, 1],
        ["browser-artifact-a", 1, 2],
        ["browser-artifact-a", 1, 3],
        ["browser-artifact-z", 2, 4],
      ]);

      await vi.advanceTimersByTimeAsync(80);
      await expect(backend.internalDuplicateActiveArtifact()).resolves.toEqual({ ok: true, data: null });
    } finally {
      unsubscribe();
      vi.useRealTimers();
    }
  });
});

describe("browser backend detail original contract", () => {
  it("uses the caller request ID, returns a terminal media result, and disposes idempotently", async () => {
    const requestId = "550e8400-e29b-41d4-a716-446655440000";
    const prepared = await backend.detailOriginalPrepare({
      requestId,
      galleryId: galleryId(4051038),
      sourcePage: 1,
    });
    expect(prepared).toMatchObject({
      ok: true,
      data: {
        requestId,
        sourcePage: 1,
        mediaUrl: "/mock-gallery-sheet.png",
        contentType: "image/png",
      },
    });
    await expect(backend.detailOriginalDispose(requestId)).resolves.toEqual({ ok: true, data: true });
    await expect(backend.detailOriginalDispose(requestId)).resolves.toEqual({ ok: true, data: true });
    await expect(backend.detailOriginalPrepare({ requestId: "not-a-uuid", galleryId: galleryId(4051038), sourcePage: 1 }))
      .resolves.toMatchObject({ ok: false, error: { code: "VALIDATION_ERROR" } });
  });

  it("allows later pages only for the matching completed local entry", async () => {
    const state = backend as unknown as { downloadEntries: Map<string, DownloadEntry> };
    const entryId = "browser-local-original";
    const previous = state.downloadEntries.get(entryId);
    const requestId = "550e8400-e29b-41d4-a716-446655440001";
    const gallery = galleryId(4051038);
    try {
      state.downloadEntries.set(entryId, {
        entryId,
        galleryId: gallery,
        revision: 1,
        state: "completed",
        progress: 100,
      });

      await expect(backend.detailOriginalPrepare({
        requestId,
        galleryId: gallery,
        sourcePage: 7,
        entryId,
      })).resolves.toMatchObject({
        ok: true,
        data: { requestId, galleryId: gallery, sourcePage: 7, contentType: "image/png" },
      });
      await expect(backend.detailOriginalPrepare({
        requestId,
        galleryId: galleryId(4050974),
        sourcePage: 7,
        entryId,
      })).resolves.toMatchObject({ ok: false, error: { code: "DETAIL_ORIGINAL_UNAVAILABLE" } });

      state.downloadEntries.set(entryId, {
        ...state.downloadEntries.get(entryId)!,
        state: "failed",
      });
      await expect(backend.detailOriginalPrepare({
        requestId,
        galleryId: gallery,
        sourcePage: 7,
        entryId,
      })).resolves.toMatchObject({ ok: false, error: { code: "DETAIL_ORIGINAL_UNAVAILABLE" } });
      await expect(backend.detailOriginalPrepare({
        requestId,
        galleryId: gallery,
        sourcePage: 2,
      })).resolves.toMatchObject({ ok: false, error: { code: "VALIDATION_ERROR" } });
    } finally {
      if (previous) state.downloadEntries.set(entryId, previous);
      else state.downloadEntries.delete(entryId);
    }
  });
});

describe("browser backend active-work exit contract", () => {
  it("aggregates only running managed work and keeps progress out of the fingerprint", async () => {
    const state = backend as unknown as {
      downloadEntries: Map<string, DownloadEntry>;
      autoFind: AutoFindSnapshot;
      duplicateSnapshotState: DuplicateSnapshot;
      internalSnapshotState: InternalDuplicateSnapshot;
    };
    const savedDownloads = state.downloadEntries;
    const savedAutoFind = state.autoFind;
    const savedDuplicate = state.duplicateSnapshotState;
    const savedInternal = state.internalSnapshotState;
    const now = "2026-08-23T00:00:00.000Z";
    const firstDownload: DownloadEntry = {
      entryId: "entry-b",
      galleryId: galleryId(8_000_002),
      revision: 0,
      state: "downloading",
      progress: 10,
    };
    const secondDownload: DownloadEntry = {
      entryId: "entry-a",
      galleryId: galleryId(8_000_001),
      revision: 0,
      state: "queued",
    };
    try {
      state.downloadEntries = new Map([[firstDownload.entryId, firstDownload], [secondDownload.entryId, secondDownload]]);
      state.autoFind = {
        candidates: [], cutoffEvidence: [], truncations: [],
        run: {
          runId: "auto-running", revision: 0, state: "running",
          totalFavorites: 7, completedFavorites: 3, candidatesFound: 128,
          startedAt: now, updatedAt: now, historyMode: "include_all_history",
        },
      };
      state.duplicateSnapshotState = {
        profile: savedDuplicate.profile,
        candidates: [],
        run: {
          runId: "duplicate-running", revision: 0, state: "running",
          totalArtifacts: 80, hashedArtifacts: 12, totalPairs: 3_160,
          comparedPairs: 340, candidatesFound: 4, startedAt: now, updatedAt: now,
        },
      };
      state.internalSnapshotState = {
        groups: [], quarantineRecords: [], skips: [],
        run: {
          runId: "internal-running", revision: 0, state: "running",
          totalArtifacts: 20, scannedArtifacts: 4, totalPages: 100,
          comparedPairs: 50, groupsFound: 3, algorithmVersion: 4,
          skippedArtifacts: 1, skippedPages: 500, startedAt: now, updatedAt: now,
        },
      };

      const first = await backend.appActiveWorkSnapshot();
      expect(first).toMatchObject({
        ok: true,
        data: {
          downloads: { activeCount: 2 },
          autoFind: { runId: "auto-running", completedFavorites: 3, totalFavorites: 7, candidatesFound: 128 },
          duplicateScan: { runId: "duplicate-running", hashedArtifacts: 12, totalArtifacts: 80, comparedPairs: 340, totalPairs: 3_160 },
          internalDuplicateScan: { runId: "internal-running", scannedArtifacts: 4, totalArtifacts: 20, skippedArtifacts: 1, groupsFound: 3 },
        },
      });
      if (!first.ok) return;

      state.downloadEntries = new Map([
        [secondDownload.entryId, secondDownload],
        [firstDownload.entryId, { ...firstDownload, progress: 95 }],
      ]);
      state.autoFind = {
        ...state.autoFind,
        run: state.autoFind.run ? { ...state.autoFind.run, completedFavorites: 6, candidatesFound: 250 } : undefined,
      };
      const progressChanged = await backend.appActiveWorkSnapshot();
      expect(progressChanged).toMatchObject({ ok: true, data: { workSetFingerprint: first.data.workSetFingerprint } });

      state.downloadEntries.set("entry-c", { ...secondDownload, entryId: "entry-c", galleryId: galleryId(8_000_003) });
      const identityChanged = await backend.appActiveWorkSnapshot();
      expect(identityChanged.ok && identityChanged.data.workSetFingerprint).not.toBe(first.data.workSetFingerprint);

      state.downloadEntries.delete("entry-c");
      if (state.autoFind.run) state.autoFind = { ...state.autoFind, run: { ...state.autoFind.run, runId: "auto-restarted" } };
      const runChanged = await backend.appActiveWorkSnapshot();
      expect(runChanged.ok && runChanged.data.workSetFingerprint).not.toBe(first.data.workSetFingerprint);
    } finally {
      state.downloadEntries = savedDownloads;
      state.autoFind = savedAutoFind;
      state.duplicateSnapshotState = savedDuplicate;
      state.internalSnapshotState = savedInternal;
    }
  });

  it("rejects unconfirmed or stale quit requests and accepts a confirmed current work set", async () => {
    const state = backend as unknown as { downloadEntries: Map<string, DownloadEntry> };
    const savedDownloads = state.downloadEntries;
    try {
      state.downloadEntries = new Map([[
        "quit-entry",
        { entryId: "quit-entry", galleryId: galleryId(8_100_001), revision: 0, state: "verifying" },
      ]]);
      const snapshot = await backend.appActiveWorkSnapshot();
      if (!snapshot.ok) throw new Error(snapshot.error.message);
      const currentWork = {
        workSetFingerprint: snapshot.data.workSetFingerprint,
        downloads: snapshot.data.downloads,
      };

      await expect(backend.appQuit({
        expectedWorkSetFingerprint: snapshot.data.workSetFingerprint,
        confirmActiveWork: false,
      })).resolves.toMatchObject({
        ok: true,
        data: { accepted: false, reason: "active_work_confirmation_required", snapshot: currentWork },
      });
      await expect(backend.appQuit({
        expectedWorkSetFingerprint: "stale-fingerprint",
        confirmActiveWork: true,
      })).resolves.toMatchObject({
        ok: true,
        data: { accepted: false, reason: "active_work_changed", snapshot: currentWork },
      });
      await expect(backend.appQuit({
        expectedWorkSetFingerprint: snapshot.data.workSetFingerprint,
        confirmActiveWork: true,
      })).resolves.toMatchObject({ ok: true, data: { accepted: true } });
    } finally {
      state.downloadEntries = savedDownloads;
    }
  });

  it("excludes completed, failed, cancelled, interrupted, and review-only work", async () => {
    const state = backend as unknown as {
      downloadEntries: Map<string, DownloadEntry>;
      autoFind: AutoFindSnapshot;
      duplicateSnapshotState: DuplicateSnapshot;
      internalSnapshotState: InternalDuplicateSnapshot;
    };
    const saved = {
      downloads: state.downloadEntries,
      autoFind: state.autoFind,
      duplicate: state.duplicateSnapshotState,
      internal: state.internalSnapshotState,
    };
    const terminalDownloads = ["completed", "failed", "cancelled", "interrupted", "review_required"] as const;
    try {
      state.downloadEntries = new Map(terminalDownloads.map((downloadState, index) => [
        `terminal-${index}`,
        { entryId: `terminal-${index}`, galleryId: galleryId(8_200_000 + index), revision: 0, state: downloadState },
      ]));
      state.autoFind = { candidates: [], cutoffEvidence: [], truncations: [], run: { ...saved.autoFind.run!, state: "completed" } };
      state.duplicateSnapshotState = { ...saved.duplicate, run: saved.duplicate.run ? { ...saved.duplicate.run, state: "failed" } : undefined };
      state.internalSnapshotState = { ...saved.internal, run: saved.internal.run ? { ...saved.internal.run, state: "cancelled" } : undefined };
      const snapshot = await backend.appActiveWorkSnapshot();
      expect(snapshot).toMatchObject({ ok: true, data: { downloads: { activeCount: 0 } } });
      if (!snapshot.ok) return;
      expect(snapshot.data.autoFind).toBeUndefined();
      expect(snapshot.data.duplicateScan).toBeUndefined();
      expect(snapshot.data.internalDuplicateScan).toBeUndefined();
      await expect(backend.appQuit({
        expectedWorkSetFingerprint: snapshot.data.workSetFingerprint,
        confirmActiveWork: false,
      })).resolves.toMatchObject({ ok: true, data: { accepted: true } });
    } finally {
      state.downloadEntries = saved.downloads;
      state.autoFind = saved.autoFind;
      state.duplicateSnapshotState = saved.duplicate;
      state.internalSnapshotState = saved.internal;
    }
  });

  it("keeps browser download-overlap review fixtures terminal and revision checked", async () => {
    const review = await backend.downloadOverlapReviewGet("browser-overlap-contract");
    expect(review).toMatchObject({
      ok: true,
      data: {
        reviewId: "browser-overlap-contract",
        revision: 0,
        state: "pending",
        candidates: [
          { relation: "near_equivalent" },
          { relation: "incoming_contains_existing" },
          { relation: "existing_contains_incoming" },
          { relation: "partial_overlap" },
        ],
      },
    });
    if (!review.ok) return;
    const firstCandidate = review.data.candidates[0];
    if (!firstCandidate) throw new Error("browser overlap fixture must include candidates");
    await expect(backend.downloadOverlapDecisionApply({
      reviewId: review.data.reviewId,
      expectedRevision: review.data.revision,
      action: "remove_existing_continue",
      candidateId: firstCandidate.candidateId,
      actor: "automation",
      reasonCode: "balanced_overlap_v2",
      ruleVersion: 2,
      featureSnapshotJson: "[]",
    })).resolves.toMatchObject({
      ok: false,
      error: { code: "VALIDATION_ERROR", details: { field: "request.featureSnapshotJson" } },
    });
    await expect(backend.downloadOverlapDecisionApply({
      reviewId: review.data.reviewId,
      expectedRevision: 99,
      action: "keep_both_continue",
      candidateId: firstCandidate.candidateId,
    })).resolves.toMatchObject({ ok: false, error: { code: "REVISION_CONFLICT" } });
    await expect(backend.downloadOverlapDecisionApply({
      reviewId: review.data.reviewId,
      expectedRevision: 0,
      action: "false_positive_continue",
      candidateId: firstCandidate.candidateId,
    })).resolves.toMatchObject({
      ok: true,
      data: { resumed: false, cancelled: false, review: { revision: 1, state: "pending" } },
    });
    const secondCandidate = review.data.candidates[1]!;
    await expect(backend.downloadOverlapDecisionApply({
      reviewId: review.data.reviewId,
      expectedRevision: 1,
      action: "remove_existing_continue",
      candidateId: secondCandidate.candidateId,
    })).resolves.toMatchObject({
      ok: true,
      data: { resumed: false, cancelled: false, review: { revision: 2, state: "pending" } },
    });
    for (const [index, candidate] of review.data.candidates.slice(2).entries()) {
      await expect(backend.downloadOverlapDecisionApply({
        reviewId: review.data.reviewId,
        expectedRevision: index + 2,
        action: "keep_both_continue",
        candidateId: candidate.candidateId,
      })).resolves.toMatchObject({
        ok: true,
        data: {
          resumed: index === 1,
          cancelled: false,
          review: { state: index === 1 ? "resolved" : "pending" },
        },
      });
    }

    const removable = await backend.downloadOverlapReviewGet("browser-overlap-remove-incoming");
    if (!removable.ok) throw new Error("browser overlap fixture missing");
    const browserState = backend as unknown as {
      downloadEntries: Map<string, DownloadEntry>;
    };
    browserState.downloadEntries.set(removable.data.entryId, {
      entryId: removable.data.entryId,
      galleryId: removable.data.incoming.galleryId,
      revision: 0,
      state: "review_required",
      progress: 100,
      reviewKind: "gallery_duplicate",
      reviewId: removable.data.reviewId,
    });
    await expect(backend.downloadOverlapDecisionApply({
      reviewId: removable.data.reviewId,
      expectedRevision: removable.data.revision,
      action: "remove_incoming",
    })).resolves.toMatchObject({
      ok: true,
      data: { resumed: false, cancelled: true, review: { state: "cancelled" } },
    });
    await expect(backend.downloadEntriesList({ page: 1, pageSize: 200 })).resolves.toMatchObject({
      ok: true,
      data: {
        entries: expect.not.arrayContaining([
          expect.objectContaining({ entryId: removable.data.entryId }),
        ]),
      },
    });
    await expect(backend.downloadRetry([removable.data.entryId])).resolves.toMatchObject({
      ok: false,
      error: {
        code: "INVALID_DOWNLOAD_STATE",
        retryable: false,
        details: { reason: "duplicate_excluded" },
      },
    });
    await backend.explorationExclusionsRestore([removable.data.incoming.galleryId]);
    await expect(backend.downloadRetry([removable.data.entryId])).resolves.toMatchObject({
      ok: true,
      data: [{ reused: false }],
    });
  });

  it("cancels only a chained staging review when it is removed from a newer overlap review", async () => {
    const state = backend as unknown as {
      downloadEntries: Map<string, DownloadEntry>;
      downloadOverlapReviews: Map<string, DownloadOverlapReview>;
    };
    const currentResult = await backend.downloadOverlapReviewGet("browser-overlap-chain-current");
    const chainedResult = await backend.downloadOverlapReviewGet("browser-overlap-chain-existing");
    if (!currentResult.ok || !chainedResult.ok) throw new Error("browser overlap fixture missing");
    const selected = currentResult.data.candidates[0]!;
    const chained: DownloadOverlapReview = {
      ...chainedResult.data,
      entryId: selected.existing.entryId,
    };
    state.downloadOverlapReviews.set(chained.reviewId, chained);
    state.downloadEntries.set(selected.existing.entryId, {
      entryId: selected.existing.entryId,
      galleryId: selected.existing.galleryId,
      revision: 0,
      state: "review_required",
      progress: 100,
      reviewKind: "gallery_duplicate",
      reviewId: chained.reviewId,
    });

    const applied = await backend.downloadOverlapDecisionApply({
      reviewId: currentResult.data.reviewId,
      expectedRevision: currentResult.data.revision,
      action: "remove_existing_continue",
      candidateId: selected.candidateId,
    });
    expect(applied).toMatchObject({
      ok: true,
      data: { resumed: false, cancelled: false, review: { state: "pending" } },
    });
    expect(state.downloadEntries.get(selected.existing.entryId)).toMatchObject({
      state: "cancelled",
      reviewKind: undefined,
      reviewId: undefined,
    });
    expect(state.downloadOverlapReviews.get(chained.reviewId)).toMatchObject({ state: "cancelled" });
    expect(state.downloadOverlapReviews.get(currentResult.data.reviewId)).toMatchObject({ state: "pending" });
    const exclusions = await backend.explorationExclusionsList();
    if (!exclusions.ok) throw new Error(exclusions.error.message);
    expect(exclusions.data).toEqual(expect.arrayContaining([
      expect.objectContaining({
        galleryId: selected.existing.galleryId,
        reasons: expect.arrayContaining([expect.objectContaining({ kind: "duplicate_hidden" })]),
      }),
    ]));
    await expect(backend.downloadRetry([selected.existing.entryId])).resolves.toMatchObject({
      ok: false,
      error: { code: "INVALID_DOWNLOAD_STATE", details: { reason: "duplicate_excluded" } },
    });
    await expect(backend.downloadEntriesList({ page: 1, pageSize: 200 })).resolves.toMatchObject({
      ok: true,
      data: {
        entries: expect.not.arrayContaining([
          expect.objectContaining({ entryId: selected.existing.entryId }),
        ]),
      },
    });
    await backend.explorationExclusionsRestore([selected.existing.galleryId]);
    await expect(backend.downloadRetry([selected.existing.entryId])).resolves.toMatchObject({
      ok: true,
      data: [{ reused: false }],
    });
  });

  it("removes a failed candidate while resolving a peer already quarantined elsewhere", async () => {
    const state = backend as unknown as {
      downloadEntries: Map<string, DownloadEntry>;
      downloadOverlapReviews: Map<string, DownloadOverlapReview>;
    };
    const fixture = await backend.downloadOverlapReviewGet("browser-overlap-stale-peer");
    if (!fixture.ok) throw new Error("browser overlap fixture missing");
    const base = fixture.data.candidates[0]!;
    const quarantinedEntryId = "browser-overlap-already-quarantined";
    const failedEntryId = "browser-overlap-failed-existing";
    const quarantinedCandidate = {
      ...base,
      candidateId: "browser-overlap-quarantined-candidate",
      existing: {
        ...base.existing,
        entryId: quarantinedEntryId,
        galleryId: galleryId(7_200_001),
      },
    };
    const failedCandidate = {
      ...base,
      candidateId: "browser-overlap-failed-candidate",
      existing: {
        ...base.existing,
        entryId: failedEntryId,
        galleryId: galleryId(7_200_002),
      },
    };
    const review: DownloadOverlapReview = {
      ...fixture.data,
      candidates: [quarantinedCandidate, failedCandidate],
    };
    state.downloadOverlapReviews.set(review.reviewId, review);
    state.downloadEntries.set(review.entryId, {
      entryId: review.entryId,
      galleryId: review.incoming.galleryId,
      revision: 0,
      state: "review_required",
      progress: 100,
      reviewKind: "gallery_duplicate",
      reviewId: review.reviewId,
    });
    state.downloadEntries.set(quarantinedEntryId, {
      entryId: quarantinedEntryId,
      galleryId: quarantinedCandidate.existing.galleryId,
      revision: 1,
      state: "quarantined",
      progress: 100,
    });
    state.downloadEntries.set(failedEntryId, {
      entryId: failedEntryId,
      galleryId: failedCandidate.existing.galleryId,
      revision: 2,
      state: "failed",
      progress: 100,
    });

    await expect(backend.downloadOverlapDecisionApply({
      reviewId: review.reviewId,
      expectedRevision: review.revision,
      action: "remove_existing_continue",
      candidateId: failedCandidate.candidateId,
    })).resolves.toMatchObject({
      ok: true,
      data: {
        resumed: true,
        cancelled: false,
        review: {
          state: "resolved",
          candidates: [
            { candidateId: quarantinedCandidate.candidateId, decision: "existing_removed" },
            { candidateId: failedCandidate.candidateId, decision: "existing_removed" },
          ],
        },
      },
    });
    expect(state.downloadEntries.get(failedEntryId)).toMatchObject({ state: "cancelled" });
    const exclusions = await backend.explorationExclusionsList();
    if (!exclusions.ok) throw new Error(exclusions.error.message);
    expect(exclusions.data.some((item) => item.galleryId === failedCandidate.existing.galleryId)).toBe(true);
    expect(exclusions.data.some((item) => item.galleryId === quarantinedCandidate.existing.galleryId)).toBe(false);
    await expect(backend.downloadEntriesList({ page: 1, pageSize: 200 })).resolves.toMatchObject({
      ok: true,
      data: {
        entries: expect.not.arrayContaining([
          expect.objectContaining({ entryId: failedEntryId }),
        ]),
      },
    });
    await backend.explorationExclusionsRestore([failedCandidate.existing.galleryId]);
  });
});
