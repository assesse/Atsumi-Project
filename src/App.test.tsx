import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { backend, type BackendEventMap } from "./api/backend";
import type {
  AppActiveWorkSnapshot,
  ApiResult,
  DownloadEntry,
  DownloadLibraryPage,
  DownloadOverlapAutomationHistoryItem,
  DownloadOverlapMergeResult,
  DownloadOverlapReview,
  DownloadPage,
  GalleryDetail,
  GalleryPage,
  InternalArtifactScanProgress,
  InternalDuplicateReview,
  InternalDuplicateSnapshot,
  InternalScanRun,
} from "./api/contracts";
import { galleryId } from "./core/types";
import { mockGalleries } from "./data/mockGalleries";
import { browserFixtureThumbnailAdapter, ThumbnailClient, ThumbnailProvider } from "./thumbnail";
import { isTutorialDismissed, setTutorialDismissed } from "./tutorial/tutorialPreference";

const testThumbnailClient = new ThumbnailClient(browserFixtureThumbnailAdapter);
const TestApp = () => <ThumbnailProvider client={testThumbnailClient}><App /></ThumbnailProvider>;

const settle = (delay = 20) => new Promise((resolve) => window.setTimeout(resolve, delay));

const explorePage = (page: number, totalPages = 20): GalleryPage => ({
  page,
  totalPages,
  items: [{
    id: galleryId(9_000_000 + page),
    title: `Explore page ${page}`,
    artist: "paging fixture",
    pages: 1,
    language: "korean",
    tags: [],
    series: [],
    characters: [],
    publishedRank: 20260820,
    popularity: 0,
    thumbnailWidth: 512,
    thumbnailHeight: 768,
  }],
});

const selectionFixturePage = (): GalleryPage => ({
  page: 1,
  totalPages: 1,
  items: [mockGalleries[0]!, mockGalleries[3]!].map(({ download: _download, ...gallery }) => ({
    ...gallery,
    publishedRank: Number(gallery.publishedAt.replaceAll("-", "")),
    popularity: gallery.score,
    thumbnailWidth: gallery.thumbnailWidth ?? 512,
    thumbnailHeight: gallery.thumbnailHeight ?? 768,
  })),
});

const libraryPageFromEntries = (page: DownloadPage): DownloadLibraryPage => ({
  page: page.page,
  totalItems: page.totalItems,
  items: page.entries.map((download) => {
    const gallery = mockGalleries.find((candidate) => candidate.id === download.galleryId);
    return {
      gallery: {
        id: download.galleryId,
        ...(gallery ? {
          title: gallery.title,
          artist: gallery.artist,
          ...(gallery.group ? { group: gallery.group } : {}),
          pages: gallery.pages,
          language: gallery.language,
          publishedRank: Number(gallery.publishedAt.replaceAll("-", "")),
        } : {}),
      },
      download,
    };
  }),
});

const clickButtonContaining = (container: HTMLElement, label: string): HTMLButtonElement => {
  const button = [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find((item) => item.textContent?.includes(label));
  if (!button) throw new Error(`Button containing ${label} was not found`);
  button.click();
  return button;
};

const submitExploreSearch = async (container: HTMLElement, delay = 20): Promise<void> => {
  const button = container.querySelector<HTMLButtonElement>('button[type="submit"][aria-label="검색"]');
  if (!button) throw new Error("Explore search button was not found");
  await act(async () => {
    button.click();
    await settle(delay);
  });
};

const prepareContainmentBatchApp = async (failSecond = false) => {
  const settings = await backend.settingsGet();
  if (!settings.ok) throw new Error(settings.error.message);
  vi.spyOn(backend, "settingsGet").mockResolvedValue({
    ok: true, data: { ...settings.data, downloadOverlapAutoMode: "off" },
  });
  const galleries = [mockGalleries[0]!, mockGalleries[3]!, mockGalleries[5]!, mockGalleries[6]!];
  const refs = galleries.map((gallery, index) => ({
    entryId: `containment-batch-entry-${index}`,
    galleryId: gallery.id,
    title: index === 0 ? "Containment keeper anthology" : `Contained volume ${index}`,
    artists: [galleries[0]!.artist],
    pageCount: index === 0 ? 30 : 7 - index,
  }));
  const keeper = refs[0]!;
  const candidate = (index: number, reverse = false): DownloadOverlapReview["candidates"][number] => {
    const small = refs[index]!;
    return {
      candidateId: `containment-batch-candidate-${index}`,
      existing: reverse ? keeper : small,
      existingFingerprint: String(index).repeat(64),
      relation: reverse ? "existing_contains_incoming" : "incoming_contains_existing",
      confidence: 0.99,
      matchedPages: small.pageCount, exactPages: small.pageCount, visualPages: 0,
      existingCoverage: reverse ? small.pageCount / keeper.pageCount : 1,
      incomingCoverage: reverse ? 1 : small.pageCount / keeper.pageCount,
      existingUniquePages: reverse ? keeper.pageCount - small.pageCount : 0,
      incomingUniquePages: reverse ? 0 : keeper.pageCount - small.pageCount,
      longestAlignedRun: small.pageCount, rank: index,
      pagePairs: Array.from({ length: small.pageCount }, (_, page) => ({
        incomingSourcePage: page + 1, existingSourcePage: page + 1,
        exactSha256: true, dHashDistance: 0, pHashDistance: 0, detailHashDistance: 0,
        edgeSimilarity: 1, visualSimilarity: 1, lowInformation: false,
      })),
    };
  };
  const direct: DownloadOverlapReview = {
    reviewId: "containment-batch-direct", entryId: keeper.entryId, incoming: keeper,
    revision: 3, state: "pending", profileVersion: 1, policyVersion: 2,
    incomingFingerprint: "a".repeat(64), candidates: [candidate(1), candidate(2)],
    createdAt: "2026-09-07T00:00:00Z", updatedAt: "2026-09-07T00:00:00Z",
  };
  const reverse: DownloadOverlapReview = {
    ...direct, reviewId: "containment-batch-reverse", entryId: refs[3]!.entryId,
    incoming: refs[3]!, revision: 5, candidates: [candidate(3, true)],
  };
  const reviews = new Map([direct, reverse].map((review) => [review.reviewId, review]));
  const entries: DownloadEntry[] = refs.map((ref, index) => ({
    entryId: ref.entryId, galleryId: ref.galleryId, revision: 1, progress: 100,
    state: index === 0 || index === 3 ? "review_required" : "completed",
    ...(index === 0 || index === 3 ? {
      reviewKind: "gallery_duplicate" as const,
      reviewId: index === 0 ? direct.reviewId : reverse.reviewId,
    } : {}),
  }));
  vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({
    ok: true, data: { page: 1, totalItems: entries.length, entries },
  });
  const overlapGet = vi.spyOn(backend, "downloadOverlapReviewGet").mockImplementation(async (reviewId) => {
    const review = reviews.get(reviewId);
    if (!review) throw new Error(`Unexpected batch review ${reviewId}`);
    return { ok: true, data: review };
  });
  const appliedIds: number[] = [];
  vi.spyOn(backend, "explorationExclusionsList").mockImplementation(async () => ({
    ok: true, data: appliedIds.map((id) => ({
      galleryId: galleryId(id), title: refs.find((ref) => Number(ref.galleryId) === id)!.title,
      artist: "Containment fixture artist",
      reasons: [{ kind: "duplicate_hidden", detail: "Containment batch decision", excludedAt: "2026-09-08T00:00:00Z" }],
    })),
  }));
  const decision = vi.spyOn(backend, "downloadOverlapDecisionApply").mockImplementation(async (request) => {
    if (failSecond && appliedIds.length === 1) return {
      ok: false, error: { code: "QUARANTINE_CONFLICT", message: "Fixture destination is occupied", retryable: false },
    };
    const current = reviews.get(request.reviewId)!;
    const selected = current.candidates.find((item) => item.candidateId === request.candidateId)!;
    const removeIncoming = request.action === "remove_incoming";
    const updated: DownloadOverlapReview = {
      ...current, revision: current.revision + 7,
      ...(removeIncoming ? { state: "cancelled" as const } : {
        candidates: current.candidates.map((item) => item === selected ? { ...item, decision: "existing_removed" as const } : item),
      }),
    };
    reviews.set(updated.reviewId, updated);
    appliedIds.push(Number(removeIncoming ? current.incoming.galleryId : selected.existing.galleryId));
    return { ok: true, data: { review: updated, resumed: false, cancelled: removeIncoming } };
  });
  vi.spyOn(backend, "searchSubmit").mockResolvedValue({
    ok: true, data: { queryId: "containment-batch-explore", firstPage: {
      ...selectionFixturePage(), items: galleries.map((gallery) => ({
        ...gallery, publishedRank: Number(gallery.publishedAt.replaceAll("-", "")),
        popularity: gallery.score, thumbnailWidth: 512, thumbnailHeight: 768,
      })),
    } },
  });
  return { keeper, refs, direct, reverse, reviews, decision, overlapGet, appliedIds, entries };
};

describe("App Phase 3A backend flow", () => {
  beforeEach(() => {
    setTutorialDismissed(true);
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    class TestResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    vi.stubGlobal("ResizeObserver", TestResizeObserver);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(Date.now()), 0));
    vi.stubGlobal("cancelAnimationFrame", (id: number) => window.clearTimeout(id));
    vi.spyOn(backend, "downloadLibraryPageList").mockImplementation(async (request) => {
      const result = await backend.downloadEntriesList(request);
      return result.ok ? { ok: true, data: libraryPageFromEntries(result.data) } : result;
    });
  });

  afterEach(() => {
    testThumbnailClient.dispose();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("shows the tutorial on first launch and persists the explicit do-not-show-again choice", async () => {
    setTutorialDismissed(false);
    const previousShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
    const previousClose = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "close");
    Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
      configurable: true,
      value() { this.setAttribute("open", ""); },
    });
    Object.defineProperty(HTMLDialogElement.prototype, "close", {
      configurable: true,
      value() { this.removeAttribute("open"); },
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      const tutorial = container.querySelector<HTMLDialogElement>(".tutorial-dialog");
      expect(tutorial).toHaveAttribute("open");
      expect(tutorial).toHaveTextContent("Atsumi 시작하기");

      await act(async () => tutorial?.querySelector<HTMLInputElement>('input[type="checkbox"]')?.click());
      await act(async () => {
        [...(tutorial?.querySelectorAll<HTMLButtonElement>("button") ?? [])]
          .find((button) => button.textContent === "Atsumi 시작")?.click();
        await settle();
      });

      expect(tutorial).not.toHaveAttribute("open");
      expect(isTutorialDismissed()).toBe(true);
    } finally {
      await act(async () => root.unmount());
      container.remove();
      if (previousShowModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", previousShowModal);
      else delete (HTMLDialogElement.prototype as unknown as { showModal?: unknown }).showModal;
      if (previousClose) Object.defineProperty(HTMLDialogElement.prototype, "close", previousClose);
      else delete (HTMLDialogElement.prototype as unknown as { close?: unknown }).close;
      setTutorialDismissed(true);
    }
  });

  it("persists the global privacy toggle and scopes the preview mask to the app document", async () => {
    const current = await backend.settingsGet();
    if (!current.ok) throw new Error(current.error.message);
    if (current.data.privacyMode) {
      const reset = await backend.settingsUpdate({ privacyMode: false }, current.data.revision);
      if (!reset.ok) throw new Error(reset.error.message);
    }
    const settingsUpdate = vi.spyOn(backend, "settingsUpdate");
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(<TestApp />);
      await settle();
    });
    expect(document.documentElement.dataset.privacyMode).toBe("off");
    const toggle = container.querySelector<HTMLButtonElement>('button[aria-label="프라이버시 모드"]');
    if (!toggle) throw new Error("Privacy mode toggle was not rendered");

    await act(async () => {
      toggle.click();
      await settle();
    });
    await vi.waitFor(() => expect(document.documentElement.dataset.privacyMode).toBe("on"));
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    expect(settingsUpdate).toHaveBeenCalledWith({ privacyMode: true }, expect.any(Number));

    await act(async () => {
      toggle.click();
      await settle();
    });
    await vi.waitFor(() => expect(document.documentElement.dataset.privacyMode).toBe("off"));
    expect(toggle).toHaveAttribute("aria-pressed", "false");

    await act(async () => root.unmount());
    expect(document.documentElement).not.toHaveAttribute("data-privacy-mode");
    container.remove();
  });

  it("keeps Explore idle through sort, language, and draft edits until an explicit search", async () => {
    const searchSubmit = vi.spyOn(backend, "searchSubmit").mockImplementation(() => new Promise(() => undefined));
    const searchPageGet = vi.spyOn(backend, "searchPageGet");
    vi.spyOn(backend, "downloadLibraryPageList").mockImplementation(() => new Promise(() => undefined));
    vi.spyOn(backend, "autoFindSnapshot").mockImplementation(() => new Promise(() => undefined));
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      expect(searchSubmit).not.toHaveBeenCalled();
      expect(searchPageGet).not.toHaveBeenCalled();
      expect(container.textContent).toContain("검색을 시작해 주세요");
      expect(container.querySelector(".gallery-grid-skeleton")).toBeNull();

      const sort = container.querySelector<HTMLSelectElement>("#sort-select");
      const input = container.querySelector<HTMLInputElement>('input[aria-label="검색"]');
      if (!sort || !input) throw new Error("Explore search controls were not rendered");
      const selectValueSetter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")?.set;
      const inputValueSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      await act(async () => {
        selectValueSetter?.call(sort, "popular_week");
        sort.dispatchEvent(new Event("change", { bubbles: true }));
        container.querySelector<HTMLButtonElement>('button[aria-label="언어 필터"]')?.click();
        await settle();
      });
      const english = [...container.querySelectorAll<HTMLLabelElement>(".language-popover label")]
        .find((label) => label.textContent?.includes("영어"))
        ?.querySelector<HTMLInputElement>('input[type="checkbox"]');
      await act(async () => {
        english?.click();
        inputValueSetter?.call(input, "typing stays local");
        input.dispatchEvent(new Event("input", { bubbles: true }));
        await settle(150);
      });
      expect(searchSubmit).not.toHaveBeenCalled();
      expect(searchPageGet).not.toHaveBeenCalled();

      await submitExploreSearch(container);
      expect(searchSubmit).toHaveBeenCalledOnce();
      expect(searchSubmit).toHaveBeenCalledWith({
        text: "typing stays local",
        includeTags: [],
        excludeTags: [],
        languages: ["korean", "english"],
        sort: "popular_week",
        pageSize: 50,
      });
      expect(searchPageGet).not.toHaveBeenCalled();
      expect(container.querySelector(".gallery-grid-skeleton")).toHaveAttribute("aria-busy", "true");
      expect(container.querySelector(".loading-state")).toBeNull();

      await act(async () => {
        clickButtonContaining(container, "Downloads");
        await settle();
      });
      expect(container.querySelector(".gallery-grid-skeleton")).toHaveAttribute("aria-busy", "true");

      await act(async () => {
        clickButtonContaining(container, "Auto Find");
        await settle();
      });
      expect(container.querySelector(".gallery-grid-skeleton")).toHaveAttribute("aria-busy", "true");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("hydrates Explore after an explicit search and queues through the formal backend client", async () => {
    const search = vi.spyOn(backend, "searchSubmit").mockResolvedValue({
      ok: true,
      data: { queryId: "selection-queue", firstPage: selectionFixturePage() },
    });
    const downloadList = vi.spyOn(backend, "downloadEntriesList");
    const queue = vi.spyOn(backend, "downloadQueueAdd");
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(<TestApp />);
      await settle();
    });

    expect(search).not.toHaveBeenCalled();
    await submitExploreSearch(container);
    expect(search).toHaveBeenCalledWith(expect.objectContaining({ text: "", sort: "recent" }));
    expect(downloadList).toHaveBeenCalledWith({ page: 1, pageSize: 200 });
    expect(container.textContent).toContain("Archive of Rain");
    expect(container.textContent).toContain("브라우저 fixture");
    expect(container.textContent).not.toContain("backend fixture");

    const [firstCard, secondCard] = [...container.querySelectorAll<HTMLElement>(".gallery-grid > .gallery-card")];
    if (!firstCard || !secondCard) throw new Error("Two Explore selection fixtures were not rendered");
    const firstId = galleryId(Number(firstCard.dataset.galleryId));
    const secondId = galleryId(Number(secondCard.dataset.galleryId));
    await act(async () => {
      firstCard.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
    });
    expect(firstCard).toHaveClass("is-selected");
    expect(firstCard.querySelector(".selection-indicator")).toBeNull();
    expect(container.querySelector(".selection-toolbar")).not.toHaveClass("is-visible");
    await act(async () => {
      secondCard.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1, ctrlKey: true }));
    });
    const queueButton = container.querySelector<HTMLButtonElement>(".selection-toolbar .primary");
    expect(container.querySelector(".selection-toolbar")).toHaveClass("is-visible");
    expect(firstCard.querySelector(".selection-indicator")).not.toBeNull();
    expect(secondCard.querySelector(".selection-indicator")).not.toBeNull();
    await act(async () => {
      queueButton?.click();
      await settle();
    });

    expect(queue).toHaveBeenCalledWith(
      [firstId, secondId],
      expect.stringMatching(/^frontend-queue-\d+-\d+$/),
    );

    await act(async () => root.unmount());
    container.remove();
  });

  it("hydrates every download page so an old quarantined Explore result is still blinded", async () => {
    const pageOneEntries: DownloadEntry[] = Array.from({ length: 200 }, (_, index) => ({
      entryId: `older-download-${index}`,
      galleryId: mockGalleries[1]!.id,
      revision: index,
      state: "completed",
      progress: 100,
    }));
    const quarantined: DownloadEntry = {
      entryId: "quarantined-on-second-page",
      galleryId: mockGalleries[2]!.id,
      revision: 201,
      state: "quarantined",
      progress: 100,
    };
    const downloadList = vi.spyOn(backend, "downloadEntriesList").mockImplementation(async ({ page }) => ({
      ok: true,
      data: {
        page,
        totalItems: 201,
        entries: page === 1 ? pageOneEntries : page === 2 ? [quarantined] : [],
      },
    }));
    const { download: _download, ...quarantinedSummary } = mockGalleries[2]!;
    vi.spyOn(backend, "searchSubmit").mockResolvedValue({
      ok: true,
      data: {
        queryId: "quarantine-second-page",
        firstPage: {
          page: 1,
          totalPages: 1,
          items: [{
            ...quarantinedSummary,
            publishedRank: Number(quarantinedSummary.publishedAt.replaceAll("-", "")),
            popularity: quarantinedSummary.score,
            thumbnailWidth: quarantinedSummary.thumbnailWidth ?? 512,
            thumbnailHeight: quarantinedSummary.thumbnailHeight ?? 768,
          }],
        },
      },
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await act(async () => {
        await vi.waitFor(() => expect(downloadList).toHaveBeenCalledWith({ page: 2, pageSize: 200 }));
        await settle(100);
      });
      await submitExploreSearch(container);

      const quarantinedCard = container.querySelector<HTMLElement>(
        `[data-gallery-id="${Number(mockGalleries[2]!.id)}"]`,
      );
      await vi.waitFor(() => expect(quarantinedCard).toHaveClass("is-quarantined-blind"));
      expect(quarantinedCard).toHaveAttribute("aria-disabled", "true");
      expect(quarantinedCard).toHaveTextContent("격리된 앨범");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("blinds an Explore result whose incoming overlap edition was removed", async () => {
    const hidden = mockGalleries[2]!;
    vi.spyOn(backend, "explorationExclusionsList").mockResolvedValue({
      ok: true,
      data: [{
        galleryId: hidden.id,
        title: hidden.title,
        artist: hidden.artist,
        reasons: [{
          kind: "duplicate_hidden",
          detail: "다운로드 판본 검토에서 신규 앨범 제거",
          excludedAt: "2026-08-28T13:51:09.774Z",
        }],
      }],
    });
    vi.spyOn(backend, "searchSubmit").mockResolvedValue({
      ok: true,
      data: {
        queryId: "removed-overlap-explore",
        firstPage: {
          page: 1,
          totalPages: 1,
          items: [{
            ...hidden,
            publishedRank: Number(hidden.publishedAt.replaceAll("-", "")),
            popularity: hidden.score,
            thumbnailWidth: hidden.thumbnailWidth ?? 512,
            thumbnailHeight: hidden.thumbnailHeight ?? 768,
          }],
        },
      },
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await submitExploreSearch(container);

      const card = container.querySelector<HTMLElement>(`[data-gallery-id="${Number(hidden.id)}"]`);
      await vi.waitFor(() => expect(card).toHaveClass("is-exploration-blind"));
      expect(card).toHaveAttribute("aria-disabled", "true");
      expect(card).toHaveTextContent("중복 판정으로 제외");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("excludes an Explore card with Delete without moving files and restores its original slot with Ctrl+Z", async () => {
    const snapshot = { candidates: [], cutoffEvidence: [], truncations: [] };
    const page = selectionFixturePage();
    const selected = page.items[0]!;
    let excluded = false;
    vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({ ok: true, data: { page: 1, totalItems: 0, entries: [] } });
    vi.spyOn(backend, "searchSubmit").mockResolvedValue({ ok: true, data: { queryId: "explore-delete-slots", firstPage: page } });
    vi.spyOn(backend, "explorationExclusionsList").mockImplementation(async () => ({
      ok: true, data: excluded ? [{
        galleryId: selected.id, title: selected.title, artist: selected.artist,
        reasons: [{ kind: "manual", detail: "사용자가 Explore 탐색에서 제외함", excludedAt: "2026-09-08T00:00:00Z" }],
      }] : [],
    }));
    const exclude = vi.spyOn(backend, "autoFindExclude").mockImplementation(async (ids) => {
      excluded = true;
      return { ok: true, data: { excludedGalleryIds: ids, snapshot } };
    });
    const restore = vi.spyOn(backend, "explorationExclusionsRestore").mockImplementation(async (ids) => {
      excluded = false;
      return { ok: true, data: { restoredGalleryIds: ids, snapshot } };
    });
    const quarantine = vi.spyOn(backend, "downloadQuarantine");
    const undoQuarantine = vi.spyOn(backend, "downloadQuarantineUndo");
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => { root.render(<TestApp />); await settle(); });
      await submitExploreSearch(container);
      const cards = () => [...container.querySelectorAll<HTMLElement>(".gallery-grid > .gallery-card")];
      const originalIds = cards().map((card) => card.dataset.galleryId);
      const card = cards()[0]!;
      const input = container.querySelector<HTMLInputElement>('.view-header input[aria-label="검색"]')!;
      await act(async () => {
        card.focus();
        input.focus();
        input.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true }));
        await settle();
      });
      expect(exclude).not.toHaveBeenCalled();
      await act(async () => {
        card.focus();
        card.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true }));
        await settle();
      });
      expect(exclude).toHaveBeenCalledExactlyOnceWith([selected.id], "사용자가 Explore 탐색에서 제외함");
      await vi.waitFor(() => expect(cards()[0]).toHaveClass("is-exploration-blind"));
      expect(cards().map((item) => item.dataset.galleryId)).toEqual(originalIds);
      expect(cards()[0]).toHaveTextContent("탐색에서 제외");
      expect(cards()[0]).toHaveAttribute("aria-disabled", "true");
      expect(quarantine).not.toHaveBeenCalled();
      await act(async () => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "z", code: "KeyZ", ctrlKey: true }));
        await settle();
      });
      expect(restore).toHaveBeenCalledExactlyOnceWith([selected.id]);
      await vi.waitFor(() => expect(cards()[0]).not.toHaveClass("is-exploration-blind"));
      expect(cards().map((item) => item.dataset.galleryId)).toEqual(originalIds);
      expect(undoQuarantine).not.toHaveBeenCalled();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("collects direct and reverse containment reviews from Activity and fetches each latest revision for a selected batch", async () => {
    const fixture = await prepareContainmentBatchApp();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => { root.render(<TestApp />); await settle(); });
      await act(async () => {
        container.querySelector<HTMLButtonElement>('button[aria-label="활동 기록"]')!.click();
        await settle();
      });
      await vi.waitFor(() => expect(container.querySelector('[aria-label="합본 우선 검토"]')).toHaveTextContent("Containment keeper anthology"));
      await act(async () => {
        container.querySelector<HTMLButtonElement>('[aria-label="합본 우선 검토"] button')!.click();
        await settle();
      });
      await vi.waitFor(() => expect(container.querySelectorAll(".download-overlap-containment-item")).toHaveLength(3));
      const rows = [...container.querySelectorAll<HTMLElement>(".download-overlap-containment-item")];
      rows.forEach((row, index) => {
        expect(row).toHaveTextContent(`Contained volume ${index + 1}`);
        expect(row.querySelector('input[type="checkbox"]')).toBeChecked();
      });
      expect(container.querySelector(".download-overlap-containment-keeper")).toHaveTextContent("Containment keeper anthology");
      // Another operation advanced each review after the grouped screen loaded.
      fixture.reviews.set(fixture.direct.reviewId, { ...fixture.direct, revision: 11 });
      fixture.reviews.set(fixture.reverse.reviewId, { ...fixture.reverse, revision: 19 });
      await act(async () => {
        container.querySelector<HTMLButtonElement>(".download-overlap-containment-apply")!.click();
        await settle();
      });
      await vi.waitFor(() => expect(fixture.decision).toHaveBeenCalledTimes(3));
      expect(fixture.decision.mock.calls.map(([request]) => request)).toMatchObject([
        { reviewId: fixture.direct.reviewId, candidateId: "containment-batch-candidate-1", expectedRevision: 11, action: "remove_existing_continue", actor: "human" },
        { reviewId: fixture.direct.reviewId, candidateId: "containment-batch-candidate-2", expectedRevision: 18, action: "remove_existing_continue", actor: "human" },
        { reviewId: fixture.reverse.reviewId, candidateId: "containment-batch-candidate-3", expectedRevision: 19, action: "remove_incoming", actor: "human" },
      ]);
      expect(fixture.appliedIds).toEqual(fixture.refs.slice(1).map((ref) => Number(ref.galleryId)));
      expect(fixture.appliedIds).not.toContain(Number(fixture.keeper.galleryId));
      expect(container).toHaveTextContent("포함 앨범 3개 제외 완료");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("keeps the first containment batch exclusion after the second fails and leaves remaining editions unchanged", async () => {
    const fixture = await prepareContainmentBatchApp(true);
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => { root.render(<TestApp />); await settle(); });
      await act(async () => {
        container.querySelector<HTMLButtonElement>('button[aria-label="활동 기록"]')!.click();
        await settle();
      });
      await vi.waitFor(() => expect(container.querySelector('[aria-label="합본 우선 검토"] button')).not.toBeNull());
      await act(async () => {
        container.querySelector<HTMLButtonElement>('[aria-label="합본 우선 검토"] button')!.click();
        await settle();
      });
      await vi.waitFor(() => expect(container.querySelectorAll(".download-overlap-containment-item")).toHaveLength(3));
      await act(async () => {
        container.querySelector<HTMLButtonElement>(".download-overlap-containment-apply")!.click();
        await settle();
      });
      await vi.waitFor(() => expect(fixture.decision).toHaveBeenCalledTimes(2));
      expect(fixture.appliedIds).toEqual([Number(fixture.refs[1]!.galleryId)]);
      expect(fixture.reviews.get(fixture.direct.reviewId)!.candidates[0]!.decision).toBe("existing_removed");
      expect(fixture.reviews.get(fixture.direct.reviewId)!.candidates[1]!.decision).toBeUndefined();
      expect(fixture.reviews.get(fixture.reverse.reviewId)).toEqual(fixture.reverse);
      expect(container).toHaveTextContent("1/3개 처리 완료 · 나머지는 변경하지 않았습니다.");
      await act(async () => {
        container.querySelector<HTMLButtonElement>('.download-overlap-dialog button[aria-label="닫기"]')!.click();
        await settle();
      });
      // The gallery projection retains the first durable removal even after the
      // rest of the batch failed and the dialog fetched its current review.
      await submitExploreSearch(container);
      const first = container.querySelector(`[data-gallery-id="${fixture.refs[1]!.galleryId}"]`);
      await vi.waitFor(() => expect(first).toHaveClass("is-exploration-blind"));
      expect(container.querySelector(`[data-gallery-id="${fixture.refs[2]!.galleryId}"]`)).not.toHaveClass("is-exploration-blind");
      expect(container.querySelector(`[data-gallery-id="${fixture.refs[3]!.galleryId}"]`)).not.toHaveClass("is-exploration-blind");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("shows batch controls only while two or more gallery cards are selected", async () => {
    vi.spyOn(backend, "searchSubmit").mockResolvedValue({
      ok: true,
      data: { queryId: "selection-mode", firstPage: selectionFixturePage() },
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await submitExploreSearch(container);
      const [first, second] = [...container.querySelectorAll<HTMLElement>(".gallery-grid > .gallery-card")];
      if (!first || !second) throw new Error("Two Explore cards are required for selection mode coverage");
      const toolbar = container.querySelector<HTMLElement>(".selection-toolbar");
      const grid = container.querySelector<HTMLElement>(".gallery-grid");

      await act(async () => first.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 })));
      expect(first).toHaveClass("is-selected");
      expect(toolbar).not.toHaveClass("is-visible");
      expect(toolbar).not.toHaveTextContent("1개 선택됨");
      expect(toolbar?.querySelector("button")).toBeNull();
      expect(grid).not.toHaveClass("is-selection-context");
      expect(first.querySelector(".selection-indicator")).toBeNull();

      await act(async () => second.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1, ctrlKey: true })));
      expect(first).toHaveClass("is-selected");
      expect(second).toHaveClass("is-selected");
      expect(toolbar).toHaveClass("is-visible");
      expect(toolbar).toHaveTextContent("2개 선택됨");
      expect(toolbar?.querySelector(".primary")).not.toBeNull();
      expect(grid).toHaveClass("is-selection-context");
      expect(first.querySelector(".selection-indicator")).not.toBeNull();
      expect(second.querySelector(".selection-indicator")).not.toBeNull();

      await act(async () => second.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1, ctrlKey: true })));
      expect(first).toHaveClass("is-selected");
      expect(second).not.toHaveClass("is-selected");
      expect(toolbar).not.toHaveClass("is-visible");
      expect(grid).not.toHaveClass("is-selection-context");
      expect(first.querySelector(".selection-indicator")).toBeNull();

      await act(async () => second.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1, ctrlKey: true })));
      await act(async () => first.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 })));
      expect(first).toHaveClass("is-selected");
      expect(second).not.toHaveClass("is-selected");
      expect(toolbar).not.toHaveClass("is-visible");
      expect(grid).not.toHaveClass("is-selection-context");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("supports roving gallery navigation, selection shortcuts, refresh, search focus, view switching, and help", async () => {
    const keyboardPage = selectionFixturePage();
    keyboardPage.items = keyboardPage.items.map((item, index) => ({
      ...item,
      id: galleryId(9_100_001 + index),
      title: `Keyboard gallery ${index + 1}`,
    }));
    const search = vi.spyOn(backend, "searchSubmit").mockResolvedValue({
      ok: true,
      data: { queryId: "keyboard-gallery", firstPage: keyboardPage },
    });
    vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({
      ok: true,
      data: { page: 1, totalItems: 0, entries: [] },
    });
    const queue = vi.spyOn(backend, "downloadQueueAdd").mockImplementation(async (ids) => ({
      ok: true,
      data: ids.map((id, index) => ({
        entryId: `keyboard-queue-${id}`,
        galleryId: id,
        revision: 1,
        state: "queued" as const,
        progress: 0,
        attempt: index + 1,
      })),
    }));
    const previousShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
    const previousClose = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "close");
    Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
      configurable: true,
      value() { this.setAttribute("open", ""); },
    });
    Object.defineProperty(HTMLDialogElement.prototype, "close", {
      configurable: true,
      value() { this.removeAttribute("open"); },
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      expect(container.querySelector('button[aria-label="키보드 단축키"]')).toBeNull();
      await submitExploreSearch(container);
      const [first, second] = [...container.querySelectorAll<HTMLElement>(".gallery-grid > .gallery-card")];
      if (!first || !second) throw new Error("Two Explore cards are required for keyboard coverage");
      expect(first).toHaveAttribute("tabindex", "0");
      expect(second).toHaveAttribute("tabindex", "-1");

      await act(async () => {
        first.focus();
        first.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
        await settle();
      });
      expect(document.activeElement).toBe(second);
      expect(first).toHaveAttribute("tabindex", "-1");
      expect(second).toHaveAttribute("tabindex", "0");

      await act(async () => second.dispatchEvent(new KeyboardEvent("keydown", {
        key: "a",
        code: "KeyA",
        ctrlKey: true,
        bubbles: true,
      })));
      expect(first).toHaveClass("is-selected");
      expect(second).toHaveClass("is-selected");

      await act(async () => second.dispatchEvent(new KeyboardEvent("keydown", {
        key: "A",
        code: "KeyA",
        ctrlKey: true,
        shiftKey: true,
        bubbles: true,
      })));
      expect(first).not.toHaveClass("is-selected");
      expect(second).not.toHaveClass("is-selected");

      await act(async () => second.dispatchEvent(new KeyboardEvent("keydown", {
        key: "a",
        code: "KeyA",
        ctrlKey: true,
        bubbles: true,
      })));

      const ctrlEnter = new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true });
      await act(async () => {
        second.dispatchEvent(ctrlEnter);
        await settle();
      });
      expect(ctrlEnter.defaultPrevented).toBe(true);
      expect(queue).toHaveBeenCalledWith(
        [galleryId(Number(first.dataset.galleryId)), galleryId(Number(second.dataset.galleryId))],
        expect.stringMatching(/^frontend-queue-\d+-\d+$/),
      );
      expect(first).not.toHaveClass("is-selected");
      expect(second).not.toHaveClass("is-selected");

      await act(async () => {
        second.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", shiftKey: true, bubbles: true }));
        await settle();
      });
      expect(document.activeElement).toBe(first);
      expect(first).toHaveClass("is-selected");
      expect(second).toHaveClass("is-selected");

      await act(async () => {
        first.dispatchEvent(new KeyboardEvent("keydown", { key: "F5", bubbles: true }));
        await settle();
      });
      expect(search).toHaveBeenCalledTimes(2);
      const refreshedFirst = container.querySelector<HTMLElement>(".gallery-grid > .gallery-card");
      if (!refreshedFirst) throw new Error("The refreshed Explore card was not rendered");
      await act(async () => refreshedFirst.dispatchEvent(new KeyboardEvent("keydown", {
        key: "a",
        code: "KeyA",
        ctrlKey: true,
        bubbles: true,
      })));
      expect(container.querySelectorAll(".gallery-card.is-selected")).toHaveLength(2);

      await act(async () => {
        refreshedFirst.dispatchEvent(new KeyboardEvent("keydown", { key: "/", code: "Slash", bubbles: true }));
        await settle();
      });
      const shortcuts = container.querySelector<HTMLDialogElement>(".keyboard-shortcuts-dialog");
      expect(shortcuts).toHaveAttribute("open");
      expect(shortcuts).toHaveTextContent("이전 화면");
      expect(shortcuts).toHaveTextContent("? · /");
      expect(shortcuts).toHaveTextContent("Floating Detail");
      expect(shortcuts).toHaveTextContent("Q · E");
      expect(shortcuts).toHaveTextContent("추가 미리보기 이전·다음 묶음");
      expect(shortcuts).toHaveTextContent("PAGE PREVIEW 이전·다음 페이지");
      await act(async () => {
        container.querySelector<HTMLButtonElement>('button[aria-label="단축키 도움말 닫기"]')?.click();
        await settle();
      });
      expect(shortcuts).not.toHaveAttribute("open");

      await act(async () => refreshedFirst.dispatchEvent(new KeyboardEvent("keydown", {
        key: "f",
        code: "KeyF",
        ctrlKey: true,
        bubbles: true,
      })));
      const input = container.querySelector<HTMLInputElement>('.view-header input[aria-label="검색"]');
      expect(document.activeElement).toBe(input);
      await act(async () => input?.dispatchEvent(new KeyboardEvent("keydown", {
        key: "A",
        code: "KeyA",
        ctrlKey: true,
        shiftKey: true,
        bubbles: true,
        cancelable: true,
      })));
      expect(container.querySelectorAll(".gallery-card.is-selected")).toHaveLength(2);

      await act(async () => {
        input?.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", ctrlKey: true, bubbles: true }));
        await settle();
      });
      expect(container).toHaveTextContent("즐겨찾기 작가 자동 탐색");
      await act(async () => {
        input?.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", ctrlKey: true, shiftKey: true, bubbles: true }));
        await settle();
      });
      expect(container).toHaveTextContent("갤러리 탐색");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      if (previousShowModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", previousShowModal);
      else delete (HTMLDialogElement.prototype as unknown as { showModal?: unknown }).showModal;
      if (previousClose) Object.defineProperty(HTMLDialogElement.prototype, "close", previousClose);
      else delete (HTMLDialogElement.prototype as unknown as { close?: unknown }).close;
    }
  });

  it("undoes the latest Downloads quarantine with Ctrl+Z", async () => {
    const completedEntry = {
      entryId: "keyboard-quarantine-entry",
      galleryId: galleryId(4051038),
      revision: 1,
      state: "completed" as const,
      progress: 100,
    };
    vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({
      ok: true,
      data: { page: 1, totalItems: 1, entries: [completedEntry] },
    });
    const quarantine = vi.spyOn(backend, "downloadQuarantine").mockResolvedValue({
      ok: true,
      data: [{ ...completedEntry, revision: 2, state: "quarantined" }],
    });
    const undo = vi.spyOn(backend, "downloadQuarantineUndo").mockResolvedValue({
      ok: true,
      data: [{ ...completedEntry, revision: 3 }],
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await act(async () => {
        clickButtonContaining(container, "Downloads");
        await settle();
      });
      const card = container.querySelector<HTMLElement>('[data-gallery-id="4051038"]');
      if (!card) throw new Error("A completed Downloads card is required for quarantine undo coverage");
      await act(async () => {
        card.focus();
        card.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true }));
        await settle();
      });
      expect(quarantine).toHaveBeenCalledWith(
        [completedEntry.entryId],
        "사용자가 Downloads 화면에서 격리를 확인함",
      );

      await act(async () => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "z", code: "KeyZ", ctrlKey: true }));
        await settle();
      });
      expect(undo).toHaveBeenCalledWith([completedEntry.entryId]);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("opens the active download entry folder from Floating Detail", async () => {
    const entryId = "floating-detail-folder-entry";
    vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({
      ok: true,
      data: {
        page: 1,
        totalItems: 1,
        entries: [{
          entryId,
          galleryId: galleryId(4051038),
          revision: 3,
          state: "downloading",
          progress: 35,
          attempt: 1,
        }],
      },
    });
    const openFolder = vi.spyOn(backend, "artifactOpenFolder").mockResolvedValue({
      ok: true,
      data: null,
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await submitExploreSearch(container);
      const archive = container.querySelector<HTMLElement>('[data-gallery-id="4051038"]');
      if (!archive) throw new Error("Archive fixture card was not rendered");
      await act(async () => {
        archive.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, detail: 2 }));
        await settle();
      });

      const folderButton = container.querySelector<HTMLButtonElement>(
        '.detail-workspace [aria-label="저장 폴더 열기"]',
      );
      expect(folderButton).not.toBeNull();
      await act(async () => {
        folderButton?.click();
        await settle();
      });
      expect(openFolder).toHaveBeenCalledOnce();
      expect(openFolder).toHaveBeenCalledWith(entryId);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("projects cached Explore pages, warms adjacent pages once, and restores each page scroll position", async () => {
    const searchSubmit = vi.spyOn(backend, "searchSubmit").mockResolvedValue({
      ok: true,
      data: { queryId: "paging-query", firstPage: explorePage(1) },
    });
    const searchPageGet = vi.spyOn(backend, "searchPageGet").mockImplementation(async (_queryId, page) => ({
      ok: true,
      data: explorePage(page),
    }));
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(<TestApp />);
      await settle();
    });
    expect(searchSubmit).not.toHaveBeenCalled();
    expect(searchPageGet).not.toHaveBeenCalled();
    await submitExploreSearch(container);
    expect(searchSubmit).toHaveBeenCalledOnce();
    expect(container.textContent).toContain("Explore page 1");
    expect(container.querySelector(".pager")).toHaveTextContent("1 / 20");
    expect(searchPageGet.mock.calls.filter(([, page]) => page === 2)).toHaveLength(1);

    const viewport = container.querySelector<HTMLElement>(".gallery-viewport");
    if (!viewport) throw new Error("Explore viewport was not rendered");
    await act(async () => {
      clickButtonContaining(container, "다음");
      await settle();
    });
    await act(async () => {
      clickButtonContaining(container, "다음");
      await settle();
    });
    expect(container.textContent).toContain("Explore page 3");
    expect(searchPageGet.mock.calls.filter(([, page]) => page === 3)).toHaveLength(1);
    expect(searchPageGet.mock.calls.filter(([, page]) => page === 4)).toHaveLength(1);
    expect(searchPageGet.mock.calls.filter(([, page]) => page === 2)).toHaveLength(1);

    viewport.scrollTop = 417;
    const fourthCallsBeforeForeground = searchPageGet.mock.calls.filter(([, page]) => page === 4).length;
    await act(async () => {
      clickButtonContaining(container, "다음");
      await settle();
    });
    expect(searchPageGet.mock.calls.filter(([, page]) => page === 4)).toHaveLength(fourthCallsBeforeForeground);
    viewport.scrollTop = 88;
    const thirdCallsBeforeReturn = searchPageGet.mock.calls.filter(([, page]) => page === 3).length;
    await act(async () => {
      clickButtonContaining(container, "이전");
      await settle();
    });
    expect(searchPageGet.mock.calls.filter(([, page]) => page === 3)).toHaveLength(thirdCallsBeforeReturn);
    await vi.waitFor(() => expect(viewport.scrollTop).toBe(417));

    await submitExploreSearch(container);
    expect(searchSubmit).toHaveBeenCalledTimes(2);
    expect(container.textContent).toContain("Explore page 1");
    expect(container.querySelector(".pager")).toHaveTextContent("1 / 20");
    expect(searchPageGet.mock.calls.filter(([, page]) => page === 2)).toHaveLength(2);

    await act(async () => root.unmount());
    container.remove();
  });

  it("opens a random gallery from the source, visible Auto Find candidates, and completed downloads", async () => {
    const asSummary = (index: number) => {
      const { download: _download, subtitle: _subtitle, coverIndex: _coverIndex, favorite: _favorite, ...gallery } = mockGalleries[index]!;
      return {
        ...gallery,
        publishedRank: Number(gallery.publishedAt.replaceAll("-", "")),
        popularity: gallery.score,
        thumbnailWidth: gallery.thumbnailWidth ?? 512,
        thumbnailHeight: gallery.thumbnailHeight ?? 768,
      };
    };
    const randomSummary = asSummary(3);
    const autoFindSummary = asSummary(4);
    const filteredAutoFindSummary = {
      ...autoFindSummary,
      id: galleryId(5_300_001),
      title: "Filtered Japanese Auto Find candidate",
      language: "japanese" as const,
    };
    const completedSummary = asSummary(2);
    const excludedCompletedSummary = asSummary(5);
    const searchSubmit = vi.spyOn(backend, "searchSubmit").mockResolvedValue({
      ok: true,
      data: { queryId: "random-source-query", firstPage: { page: 1, totalPages: 1, items: [randomSummary] } },
    });
    vi.spyOn(backend, "favoritesList").mockResolvedValue({
      ok: true,
      data: [{ namespace: "artist", value: autoFindSummary.artist, revision: 1, createdAt: "2026-08-30T00:00:00Z", updatedAt: "2026-08-30T00:00:00Z" }],
    });
    vi.spyOn(backend, "explorationExclusionsList").mockResolvedValue({
      ok: true,
      data: [{
        galleryId: excludedCompletedSummary.id,
        title: excludedCompletedSummary.title,
        artist: excludedCompletedSummary.artist,
        reasons: [{ kind: "manual", detail: "사용자가 제외", excludedAt: "2026-08-30T00:00:00Z" }],
      }],
    });
    vi.spyOn(backend, "autoFindSnapshot").mockResolvedValue({
      ok: true,
      data: {
        candidates: [autoFindSummary, filteredAutoFindSummary].map((candidate) => ({
          ...candidate,
          runId: "random-auto-find-run",
          matchedFavorite: { namespace: "artist" as const, value: autoFindSummary.artist },
          discoveredAt: "2026-08-30T00:00:00Z",
        })),
        cutoffEvidence: [],
        truncations: [],
      },
    });
    vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({
      ok: true,
      data: {
        page: 1,
        totalItems: 3,
        entries: [
          { entryId: "random-completed", galleryId: completedSummary.id, revision: 1, state: "completed", progress: 100 },
          { entryId: "random-excluded", galleryId: excludedCompletedSummary.id, revision: 1, state: "completed", progress: 100 },
          { entryId: "random-active", galleryId: mockGalleries[1]!.id, revision: 1, state: "downloading", progress: 20 },
        ],
      },
    });
    vi.spyOn(Math, "random").mockReturnValue(0.99);
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle(80);
      });

      const randomButton = () => container.querySelector<HTMLButtonElement>('button[aria-label="랜덤 열기"]');
      expect(randomButton()).toBeEnabled();
      await act(async () => {
        randomButton()?.click();
        await settle(40);
      });
      expect(searchSubmit).toHaveBeenCalledWith({
        text: "",
        includeTags: [],
        excludeTags: [],
        languages: ["korean", "japanese", "chinese", "english"],
        sort: "random",
        pageSize: 1,
      });
      expect(container.querySelector(".detail-workspace")).toHaveAttribute("aria-label", `${randomSummary.title} 상세`);

      await act(async () => {
        clickButtonContaining(container, "Auto Find");
        await settle();
      });
      expect(randomButton()).toBeEnabled();
      await act(async () => {
        randomButton()?.click();
        await settle();
      });
      expect(searchSubmit).toHaveBeenCalledTimes(1);
      expect(container.querySelector(".detail-workspace")).toHaveAttribute("aria-label", `${autoFindSummary.title} 상세`);

      await act(async () => {
        clickButtonContaining(container, "Downloads");
        await settle();
      });
      expect(randomButton()).toBeEnabled();
      await act(async () => {
        randomButton()?.click();
        await settle();
      });
      expect(searchSubmit).toHaveBeenCalledTimes(1);
      expect(container.querySelector(".detail-workspace")).toHaveAttribute("aria-label", `${completedSummary.title} 상세`);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("keeps Downloads random disabled until both complete library and exclusion snapshots are ready", async () => {
    const completed = mockGalleries[2]!;
    let resolveLibrary!: (value: Awaited<ReturnType<typeof backend.downloadLibraryPageList>>) => void;
    let resolveExclusions!: (value: Awaited<ReturnType<typeof backend.explorationExclusionsList>>) => void;
    vi.spyOn(backend, "downloadLibraryPageList").mockImplementation(() => new Promise((resolve) => {
      resolveLibrary = resolve;
    }));
    vi.spyOn(backend, "explorationExclusionsList").mockImplementation(() => new Promise((resolve) => {
      resolveExclusions = resolve;
    }));

    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await act(async () => {
        clickButtonContaining(container, "Downloads");
        await settle();
      });

      const randomButton = () => container.querySelector<HTMLButtonElement>('button[aria-label="랜덤 열기"]');
      expect(randomButton()).toBeDisabled();

      await act(async () => {
        resolveExclusions({ ok: true, data: [] });
        await settle();
      });
      expect(randomButton()).toBeDisabled();

      await act(async () => {
        resolveLibrary({
          ok: true,
          data: {
            page: 1,
            totalItems: 1,
            items: [{
              gallery: {
                id: completed.id,
                title: completed.title,
                artist: completed.artist,
                pages: completed.pages,
                language: completed.language,
                publishedRank: Number(completed.publishedAt.replaceAll("-", "")),
              },
              download: {
                entryId: "complete-random-hydration",
                galleryId: completed.id,
                revision: 1,
                state: "completed",
                progress: 100,
              },
            }],
          },
        });
        await settle(40);
      });
      expect(randomButton()).toBeEnabled();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("uses the persisted Explore page size for each new search", async () => {
    const original = await backend.settingsGet();
    if (!original.ok) throw new Error(original.error.message);
    const configured = await backend.settingsUpdate(
      { explorePageSize: 80 },
      original.data.revision,
    );
    if (!configured.ok) throw new Error(configured.error.message);
    const search = vi.spyOn(backend, "searchSubmit");
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await submitExploreSearch(container);
      expect(search).toHaveBeenLastCalledWith(expect.objectContaining({ pageSize: 80 }));
    } finally {
      await act(async () => root.unmount());
      container.remove();
      const latest = await backend.settingsGet();
      if (latest.ok) {
        await backend.settingsUpdate(
          { explorePageSize: original.data.explorePageSize },
          latest.data.revision,
        );
      }
    }
  });

  it("restores parked root and child Explore contexts without repeating backend searches", async () => {
    const originalSettings = await backend.settingsGet();
    if (!originalSettings.ok) throw new Error(originalSettings.error.message);
    const contextPage = (
      scope: "Root" | "Artist",
      page: number,
      totalPages: number,
      idBase: number,
      artist: string,
    ): GalleryPage => ({
      page,
      totalPages,
      items: [{
        id: galleryId(idBase + page),
        title: `${scope} page ${page}`,
        artist,
        pages: 1,
        language: "korean",
        tags: [],
        series: [],
        characters: [],
        publishedRank: 20260820,
        popularity: 0,
        thumbnailWidth: 512,
        thumbnailHeight: 768,
      }],
    });
    const rootPage = (page: number) => contextPage("Root", page, 20, 8_100_000, "root artist");
    const artistPage = (page: number) => contextPage("Artist", page, 5, 8_200_000, "root artist");
    const searchSubmit = vi.spyOn(backend, "searchSubmit")
      .mockResolvedValueOnce({
        ok: true,
        data: { queryId: "query-root", firstPage: rootPage(1) },
      })
      .mockResolvedValueOnce({
        ok: true,
        data: { queryId: "query-artist", firstPage: artistPage(1) },
      });
    const searchPageGet = vi.spyOn(backend, "searchPageGet").mockImplementation(async (queryId, page) => {
      if (queryId === "query-root") return { ok: true, data: rootPage(page) };
      if (queryId === "query-artist") return { ok: true, data: artistPage(page) };
      throw new Error(`Unexpected Explore query ${queryId}`);
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await submitExploreSearch(container);
      await act(async () => {
        clickButtonContaining(container, "다음");
        await settle();
      });
      await act(async () => {
        clickButtonContaining(container, "다음");
        await settle();
      });

      const viewport = container.querySelector<HTMLElement>(".gallery-viewport");
      const rootCard = container.querySelector<HTMLElement>('[data-gallery-id="8100003"]');
      const rootArtist = rootCard?.querySelector<HTMLButtonElement>(".byline");
      if (!viewport || !rootCard || !rootArtist) throw new Error("Root Explore page 3 fixture was not rendered");
      expect(container).toHaveTextContent("Root page 3");
      expect(container.querySelector(".pager")).toHaveTextContent("3 / 20");
      viewport.scrollTop = 417;

      await act(async () => {
        rootArtist.click();
        await settle();
      });
      expect(searchSubmit).toHaveBeenCalledTimes(2);
      expect(container).toHaveTextContent("Artist page 1");
      expect(container.querySelector('[data-gallery-id="8100003"]')).toBeNull();
      expect(container.querySelector(".explore-context-bar")).not.toBeNull();

      await act(async () => {
        clickButtonContaining(container, "다음");
        await settle();
      });
      expect(container).toHaveTextContent("Artist page 2");
      expect(container.querySelector(".pager")).toHaveTextContent("2 / 5");
      viewport.scrollTop = 88;

      const callsBeforeSwitching = {
        submit: searchSubmit.mock.calls.length,
        page: searchPageGet.mock.calls.length,
      };
      const findContextTab = (label: string): HTMLButtonElement => {
        const tab = [...container.querySelectorAll<HTMLButtonElement>(".explore-context-bar [role='tab']")]
          .find((item) => item.textContent?.includes(label));
        if (!tab) throw new Error(`Explore context tab ${label} was not rendered`);
        return tab;
      };

      await act(async () => {
        findContextTab("전체 탐색").click();
        await settle();
      });
      expect(container).toHaveTextContent("Root page 3");
      expect(container.querySelector(".pager")).toHaveTextContent("3 / 20");
      expect(container.querySelector('[data-gallery-id="8100003"]')).not.toBeNull();
      expect(container.querySelector('[data-gallery-id="8200002"]')).toBeNull();
      expect(findContextTab("전체 탐색")).toHaveAttribute("aria-selected", "true");
      await vi.waitFor(() => expect(viewport.scrollTop).toBe(417));
      expect(searchSubmit).toHaveBeenCalledTimes(callsBeforeSwitching.submit);
      expect(searchPageGet).toHaveBeenCalledTimes(callsBeforeSwitching.page);

      await act(async () => {
        findContextTab("artist:root_artist").click();
        await settle();
      });
      expect(container).toHaveTextContent("Artist page 2");
      expect(container.querySelector(".pager")).toHaveTextContent("2 / 5");
      expect(container.querySelector('[data-gallery-id="8200002"]')).not.toBeNull();
      expect(container.querySelector('[data-gallery-id="8100003"]')).toBeNull();
      await vi.waitFor(() => expect(viewport.scrollTop).toBe(88));
      expect(searchSubmit).toHaveBeenCalledTimes(callsBeforeSwitching.submit);
      expect(searchPageGet).toHaveBeenCalledTimes(callsBeforeSwitching.page);

      await act(async () => {
        container.querySelector<HTMLButtonElement>('[aria-label="이전 탐색으로 돌아가기"]')?.click();
        await settle();
      });
      expect(container).toHaveTextContent("Root page 3");
      expect(container.querySelector(".pager")).toHaveTextContent("3 / 20");
      await vi.waitFor(() => expect(viewport.scrollTop).toBe(417));
      expect(searchSubmit).toHaveBeenCalledTimes(callsBeforeSwitching.submit);
      expect(searchPageGet).toHaveBeenCalledTimes(callsBeforeSwitching.page);

      const latestSettings = await backend.settingsGet();
      if (!latestSettings.ok) throw new Error(latestSettings.error.message);
      await act(async () => {
        const updated = await backend.settingsUpdate(
          { explorePageSize: latestSettings.data.explorePageSize === 80 ? 70 : 80 },
          latestSettings.data.revision,
        );
        if (!updated.ok) throw new Error(updated.error.message);
        await settle();
      });
      const restoredRootArtist = container.querySelector<HTMLElement>('[data-gallery-id="8100003"]')
        ?.querySelector<HTMLButtonElement>(".byline");
      if (!restoredRootArtist) throw new Error("Root artist action was not restored");
      await act(async () => {
        restoredRootArtist.click();
        await settle();
      });
      expect(container).toHaveTextContent("Artist page 2");
      expect(searchSubmit).toHaveBeenCalledTimes(callsBeforeSwitching.submit);
      expect(searchPageGet).toHaveBeenCalledTimes(callsBeforeSwitching.page);
    } finally {
      await act(async () => root.unmount());
      container.remove();
      const latestSettings = await backend.settingsGet();
      if (latestSettings.ok) {
        await backend.settingsUpdate(
          { explorePageSize: originalSettings.data.explorePageSize },
          latestSettings.data.revision,
        );
      }
    }
  });

  it("does not search while typing and replays structured history with the current page size", async () => {
    const replayRequest = {
      text: "archive",
      includeTags: ["full_color"],
      excludeTags: ["male:suit"],
      languages: ["english", "korean"] as const,
      sort: "popular_week" as const,
      pageSize: 17,
    };
    await backend.searchSubmit({ ...replayRequest, languages: [...replayRequest.languages] });
    const search = vi.spyOn(backend, "searchSubmit");
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(<TestApp />);
      await settle();
    });
    expect(search).not.toHaveBeenCalled();

    const input = container.querySelector<HTMLInputElement>('input[aria-label="검색"]');
    if (!input) throw new Error("Search input was not found");
    await act(async () => {
      input.focus();
      await settle();
    });
    const historySuggestion = [...container.querySelectorAll<HTMLButtonElement>(".suggestion")]
      .find((item) => item.textContent?.includes("archive"));
    if (!historySuggestion) throw new Error("Structured history suggestion was not found");
    await act(async () => {
      historySuggestion.click();
      await settle();
    });
    expect(search).toHaveBeenLastCalledWith({
      ...replayRequest,
      languages: ["korean", "english"],
      pageSize: 50,
    });

    const viewport = container.querySelector<HTMLElement>(".gallery-viewport");
    const metadata = container.querySelector<HTMLButtonElement>(".gallery-card .byline");
    if (!metadata || !viewport) throw new Error("gallery metadata fixture was not rendered");
    viewport.scrollTop = 245;
    const callsBeforeMetadata = search.mock.calls.length;
    await act(async () => {
      metadata.click();
      await settle();
    });
    expect(search.mock.calls).toHaveLength(callsBeforeMetadata + 1);
    expect(search).toHaveBeenLastCalledWith(expect.objectContaining({
      text: expect.stringMatching(/^artist:/),
      includeTags: [],
      excludeTags: [],
      languages: ["korean", "english"],
      sort: "popular_week",
      pageSize: 50,
    }));
    expect(viewport.scrollTop).toBe(0);
    const refreshedMetadata = container.querySelector<HTMLButtonElement>(".gallery-card .byline");
    if (!refreshedMetadata) throw new Error("Fresh gallery metadata fixture was not rendered");
    await act(async () => {
      refreshedMetadata.click();
      await settle();
    });
    expect(search.mock.calls).toHaveLength(callsBeforeMetadata + 1);
    expect(container.querySelectorAll(".explore-context-bar [role='tab']")).toHaveLength(2);

    const callsAfterReplay = search.mock.calls.length;
    const valueSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    await act(async () => {
      valueSetter?.call(input, "typing must stay local");
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await settle();
    });
    expect(search).toHaveBeenCalledTimes(callsAfterReplay);

    await act(async () => root.unmount());
    container.remove();
  });

  it("starts a new structured metadata search from selected cards, detail, and related chips", async () => {
    const search = vi.spyOn(backend, "searchSubmit");
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const expectFreshTagRequest = (includeTag = "full color") => {
      expect(search).toHaveBeenLastCalledWith(expect.objectContaining({
        text: "",
        includeTags: [includeTag],
        excludeTags: [],
        pageSize: 50,
      }));
    };

    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await submitExploreSearch(container);
      const archive = container.querySelector<HTMLElement>('[data-gallery-id="4051038"]');
      if (!archive) throw new Error("Archive fixture card was not rendered");
      await act(async () => {
        archive.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, detail: 2 }));
        await settle();
      });
      expect(container.querySelector(".detail-workspace")).not.toBeNull();

      const cardTag = [...archive.querySelectorAll<HTMLButtonElement>(".tag")]
        .find((chip) => chip.querySelector(".tag-label")?.textContent === "full color");
      if (!cardTag) throw new Error("Selected-card neutral tag was not rendered");
      await act(async () => {
        archive.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
        cardTag.click();
        await settle();
      });
      expectFreshTagRequest();
      expect(container.querySelector(".detail-workspace")).toBeNull();
      expect(container.querySelector(".detail-restore")).not.toBeNull();

      await act(async () => {
        container.querySelector<HTMLButtonElement>(".detail-restore")?.click();
        await settle();
      });
      const detailTag = [...container.querySelectorAll<HTMLButtonElement>(".detail-workspace .tags-box .tag")]
        .find((chip) => chip.querySelector(".tag-label")?.textContent === "full color");
      if (!detailTag) throw new Error("Floating Detail neutral tag was not rendered");
      await act(async () => {
        detailTag.click();
        await settle();
      });
      expectFreshTagRequest();

      await act(async () => {
        container.querySelector<HTMLButtonElement>(".detail-restore")?.click();
        await settle();
      });
      const relatedTag = [...container.querySelectorAll<HTMLButtonElement>(".detail-workspace .related-card .tag")]
        .find((chip) => chip.querySelector(".tag-label")?.textContent === "coat");
      if (!relatedTag) throw new Error("Related neutral tag was not rendered");
      const callsBeforeRepeat = search.mock.calls.length;
      await act(async () => {
        relatedTag.click();
        await settle();
      });
      expectFreshTagRequest("female:coat");
      await act(async () => {
        container.querySelector<HTMLButtonElement>(".detail-restore")?.click();
        await settle();
        [...container.querySelectorAll<HTMLButtonElement>(".detail-workspace .related-card .tag")]
          .find((chip) => chip.querySelector(".tag-label")?.textContent === "coat")?.click();
        await settle();
      });
      expect(search.mock.calls).toHaveLength(callsBeforeRepeat + 1);
      expectFreshTagRequest("female:coat");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("shows the Explore skeleton instead of the previous list while a tag search is pending", async () => {
    let resolveTagSearch: ((value: { ok: true; data: { queryId: string; firstPage: GalleryPage } }) => void) | undefined;
    const tagSearch = new Promise<{ ok: true; data: { queryId: string; firstPage: GalleryPage } }>((resolve) => {
      resolveTagSearch = resolve;
    });
    const search = vi.spyOn(backend, "searchSubmit")
      .mockResolvedValueOnce({
        ok: true,
        data: { queryId: "existing-explore-page", firstPage: selectionFixturePage() },
      })
      .mockImplementationOnce(() => tagSearch);
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await submitExploreSearch(container);
      const existingCard = container.querySelector<HTMLElement>('[data-gallery-id="4051038"]');
      const tag = [...(existingCard?.querySelectorAll<HTMLButtonElement>(".tag") ?? [])]
        .find((chip) => chip.querySelector(".tag-label")?.textContent === "full color");
      if (!existingCard || !tag) throw new Error("Existing Explore tag fixture was not rendered");

      await act(async () => {
        tag.click();
        await Promise.resolve();
      });
      await vi.waitFor(() => expect(search).toHaveBeenCalledTimes(2));
      expect(container.querySelector(".gallery-grid-skeleton")).toHaveAttribute("aria-busy", "true");
      expect(container.querySelector('[data-gallery-id="4051038"]')).toBeNull();

      const freshPage = selectionFixturePage();
      await act(async () => {
        resolveTagSearch?.({
          ok: true,
          data: {
            queryId: "full-color-page",
            firstPage: {
              ...freshPage,
              items: [{ ...freshPage.items[0]!, id: galleryId(9_000_001), title: "Fresh full color result" }],
            },
          },
        });
        await settle();
      });
      expect(container.querySelector(".gallery-grid-skeleton")).toBeNull();
      expect(container).toHaveTextContent("Fresh full color result");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("keeps series and character favorites in detail while related galleries stay compact", async () => {
    const favoriteSet = vi.spyOn(backend, "favoriteSet");
    const container = document.createElement("div");
    document.body.append(container);
    let root = createRoot(container);

    await act(async () => {
      root.render(<TestApp />);
      await settle();
    });
    await submitExploreSearch(container);
    const archiveCard = container.querySelector<HTMLElement>('[data-gallery-id="4051038"]');
    if (!archiveCard) throw new Error("Archive fixture card was not rendered");
    expect(archiveCard.querySelector('[title^="시리즈 · rain archives"]')).toBeNull();
    expect(archiveCard.querySelector('[title^="캐릭터 · mira lane"]')).toBeNull();

    await act(async () => {
      archiveCard.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, detail: 2 }));
      await settle();
    });
    const detailSeries = container.querySelector<HTMLButtonElement>('.detail-workspace [title^="rain archives"]');
    const detailCharacter = container.querySelector<HTMLButtonElement>('.detail-workspace [title^="mira lane"]');
    if (!detailSeries || !detailCharacter) throw new Error("Detail series/character chips were not rendered");
    await act(async () => {
      detailSeries.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
      detailCharacter.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
      await settle();
    });
    expect(favoriteSet).toHaveBeenCalledWith({ namespace: "series", value: "rain archives" }, true);
    expect(favoriteSet).toHaveBeenCalledWith({ namespace: "character", value: "mira lane" }, true);
    expect([...container.querySelectorAll<HTMLButtonElement>('.detail-workspace [title^="rain archives"]')]
      .every((chip) => chip.classList.contains("favorite"))).toBe(true);
    expect([...container.querySelectorAll<HTMLButtonElement>('.detail-workspace [title^="mira lane"]')]
      .every((chip) => chip.classList.contains("favorite"))).toBe(true);

    const matchingDetailChips = [...container.querySelectorAll<HTMLButtonElement>('.detail-workspace [title^="rain archives"]')];
    expect(matchingDetailChips).toHaveLength(1);
    expect(matchingDetailChips.every((chip) => chip.classList.contains("favorite"))).toBe(true);
    expect(container.querySelector(".related-card")?.textContent).not.toContain("rain archives");
    expect(container.querySelector(".related-card")?.textContent).not.toContain("mira lane");

    await act(async () => root.unmount());
    container.replaceChildren();
    root = createRoot(container);
    await act(async () => {
      root.render(<TestApp />);
      await settle();
    });
    await submitExploreSearch(container);
    expect(container.querySelector('[data-gallery-id="4051038"] [title^="시리즈 · rain archives"]')).toBeNull();
    expect(container.querySelector('[data-gallery-id="4051038"] [title^="캐릭터 · mira lane"]')).toBeNull();

    await act(async () => root.unmount());
    container.remove();
    await backend.favoriteSet({ namespace: "series", value: "rain archives" }, false);
    await backend.favoriteSet({ namespace: "character", value: "mira lane" }, false);
  });

  it("serializes rapid add then remove input for the same metadata favorite", async () => {
    const artist = mockGalleries[0]!.artist;
    const key = { namespace: "artist" as const, value: artist };
    const originalFavoriteSet = backend.favoriteSet.bind(backend);
    await originalFavoriteSet(key, false);
    let resolveFirst!: (value: Awaited<ReturnType<typeof backend.favoriteSet>>) => void;
    const favoriteSet = vi.spyOn(backend, "favoriteSet")
      .mockImplementationOnce(() => new Promise((resolve) => {
        resolveFirst = resolve;
      }))
      .mockImplementation(originalFavoriteSet);
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await submitExploreSearch(container);
      const artistChip = container.querySelector<HTMLButtonElement>(`[data-gallery-id="${mockGalleries[0]!.id}"] .byline.artist`);
      if (!artistChip) throw new Error("Artist metadata chip was not rendered");

      await act(async () => {
        artistChip.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
        artistChip.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
        await Promise.resolve();
      });
      expect(favoriteSet).toHaveBeenCalledTimes(1);
      expect(favoriteSet).toHaveBeenNthCalledWith(1, key, true);

      await act(async () => {
        resolveFirst({
          ok: true,
          data: {
            enabled: true,
            favorite: {
              ...key,
              revision: 0,
              createdAt: "2026-09-07T00:00:00Z",
              updatedAt: "2026-09-07T00:00:00Z",
            },
          },
        });
        await settle(40);
      });

      expect(favoriteSet).toHaveBeenCalledTimes(2);
      expect(favoriteSet).toHaveBeenNthCalledWith(2, key, false);
      expect(container.querySelector(`[data-gallery-id="${mockGalleries[0]!.id}"] .byline.artist`)).not.toHaveClass("favorite");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      await originalFavoriteSet(key, false);
    }
  });

  it("cancels, restores, groups, and excludes Auto Find candidates", async () => {
    await backend.favoriteSet({ namespace: "artist", value: "serein" }, true);
    await backend.favoriteSet({ namespace: "artist", value: "mizuno" }, true);
    const refresh = vi.spyOn(backend, "autoFindRefresh");
    const cancel = vi.spyOn(backend, "autoFindCancel");
    const exclude = vi.spyOn(backend, "autoFindExclude");
    const restoreExclusion = vi.spyOn(backend, "explorationExclusionsRestore");
    const container = document.createElement("div");
    document.body.append(container);
    let root = createRoot(container);

    await act(async () => {
      root.render(<TestApp />);
      await settle();
    });
    await act(async () => {
      clickButtonContaining(container, "Auto Find");
      await settle();
    });
    await act(async () => {
      clickButtonContaining(container, "즐겨찾기 작가 갱신");
      await settle(10);
    });
    expect(refresh).toHaveBeenCalledTimes(1);
    await act(async () => {
      clickButtonContaining(container, "탐색 취소");
      await settle();
    });
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("탐색 취소됨");

    await act(async () => {
      clickButtonContaining(container, "즐겨찾기 작가 갱신");
      await settle(150);
      container.querySelector<HTMLButtonElement>('button[aria-label="언어 필터"]')?.click();
      await settle();
      const english = [...container.querySelectorAll<HTMLLabelElement>(".language-popover label")]
        .find((label) => label.textContent?.includes("영어"))
        ?.querySelector<HTMLInputElement>('input[type="checkbox"]');
      english?.click();
      await settle();
    });
    expect(container.textContent).toContain("탐색 완료");
    expect(container.textContent).toContain("The Last Tram");
    expect(container.textContent).toContain("Blue Lane");

    expect(container).toHaveTextContent("전부 접기");
    expect(container).not.toHaveTextContent("후보 다운로드");

    await act(async () => root.unmount());
    container.replaceChildren();
    root = createRoot(container);
    await act(async () => {
      root.render(<TestApp />);
      await settle();
    });
    await act(async () => {
      clickButtonContaining(container, "Auto Find");
      await settle();
    });
    await act(async () => {
      clickButtonContaining(container, "작가별");
      await settle();
    });
    expect(container.querySelectorAll(".gallery-group").length).toBeGreaterThanOrEqual(1);
    expect(container.textContent).toContain("The Last Tram");

    const cardsBeforeExclude = container.querySelectorAll(".gallery-card").length;
    const firstCard = container.querySelector<HTMLDivElement>(".gallery-card");
    if (!firstCard) throw new Error("An Auto Find card is required for keyboard exclusion coverage");
    await act(async () => {
      firstCard.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
      await settle();
      firstCard.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true }));
      await settle();
    });
    expect(exclude).toHaveBeenCalledWith(
      [expect.any(Number)],
      "사용자가 Auto Find 후보 목록에서 제외함",
    );
    expect(container.querySelectorAll(".gallery-card")).toHaveLength(cardsBeforeExclude - 1);

    const excludedId = exclude.mock.calls.at(-1)?.[0][0];
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "z", code: "KeyZ", ctrlKey: true }));
      await settle();
    });
    expect(restoreExclusion).toHaveBeenCalledWith([excludedId]);
    expect(container.querySelectorAll(".gallery-card")).toHaveLength(cardsBeforeExclude);

    await act(async () => root.unmount());
    container.remove();
    await backend.favoriteSet({ namespace: "artist", value: "serein" }, false);
    await backend.favoriteSet({ namespace: "artist", value: "mizuno" }, false);
  });

  it("removes queued galleries from the Auto Find list and navigation count immediately", async () => {
    const base = mockGalleries[4]!;
    const candidateIds = [galleryId(5_200_001), galleryId(5_200_002), galleryId(5_200_003)];
    const candidates = candidateIds.map((id, index) => ({
      id,
      title: index === 1 ? "Anthology matched through favorite artist" : `Queued Auto Find candidate ${index + 1}`,
      artist: index === 1 ? "anthology editor" : base.artist,
      ...(base.group ? { group: base.group } : {}),
      pages: base.pages,
      language: index === 2 ? "japanese" as const : base.language,
      tags: [...base.tags],
      series: [...base.series],
      characters: [...base.characters],
      publishedRank: 20260901 - index,
      popularity: base.score,
      thumbnailWidth: 512,
      thumbnailHeight: 768,
      runId: "auto-find-queue-filter-run",
      matchedFavorite: { namespace: "artist" as const, value: base.artist },
      discoveredAt: `2026-09-0${index + 1}T00:00:00Z`,
    }));
    vi.spyOn(backend, "favoritesList").mockResolvedValue({
      ok: true,
      data: [{
        namespace: "artist",
        value: base.artist,
        revision: 1,
        createdAt: "2026-09-01T00:00:00Z",
        updatedAt: "2026-09-01T00:00:00Z",
      }],
    });
    vi.spyOn(backend, "autoFindSnapshot").mockResolvedValue({
      ok: true,
      data: {
        run: {
          runId: "auto-find-queue-filter-run",
          revision: 3,
          state: "completed",
          totalFavorites: 1,
          completedFavorites: 1,
          candidatesFound: 3,
          startedAt: "2026-09-01T00:00:00Z",
          updatedAt: "2026-09-01T00:01:00Z",
          finishedAt: "2026-09-01T00:01:00Z",
          historyMode: "include_all_history",
        },
        candidates,
        cutoffEvidence: [],
        truncations: [],
      },
    });
    vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({
      ok: true,
      data: { page: 1, totalItems: 0, entries: [] },
    });
    vi.spyOn(backend, "explorationExclusionsList").mockResolvedValue({ ok: true, data: [] });
    const queue = vi.spyOn(backend, "downloadQueueAdd").mockImplementation(async (ids) => ({
      ok: true,
      data: ids.map((id) => ({
        entryId: `queued-auto-find-${id}`,
        galleryId: id,
        revision: 1,
        state: "queued" as const,
        progress: 0,
      })),
    }));

    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle(60);
      });
      await act(async () => {
        clickButtonContaining(container, "Auto Find");
        await settle(30);
      });

      const autoFindNav = [...container.querySelectorAll<HTMLButtonElement>(".nav-item")]
        .find((button) => button.textContent?.includes("Auto Find"));
      expect(autoFindNav?.querySelector(".nav-count")).toHaveTextContent("2");
      expect(container).toHaveTextContent("확인된 항목 3개 · 다운로드 전 2개");
      const cards = [...container.querySelectorAll<HTMLElement>(".gallery-card")];
      expect(cards).toHaveLength(2);
      expect(container).toHaveTextContent("Anthology matched through favorite artist");
      expect(container).not.toHaveTextContent("Queued Auto Find candidate 3");

      await act(async () => {
        cards[0]?.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
        cards[1]?.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1, ctrlKey: true }));
        await settle();
      });
      const downloadSelected = container.querySelector<HTMLButtonElement>(".selection-toolbar .primary");
      if (!downloadSelected) throw new Error("Auto Find multi-selection download button was not rendered");
      await act(async () => {
        downloadSelected.click();
        await settle(30);
      });

      expect(queue).toHaveBeenCalledWith(candidateIds.slice(0, 2), expect.stringMatching(/^frontend-queue-/));
      expect(container.querySelectorAll(".gallery-card")).toHaveLength(0);
      expect(autoFindNav?.querySelector(".nav-count")).toBeNull();
      expect(container).toHaveTextContent("확인된 항목 3개 · 다운로드 전 0개");
      expect(container).toHaveTextContent("표시할 갤러리가 없습니다");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("uses compact evidence and persists Auto Find and Downloads accordion state", async () => {
    const originalSettings = await backend.settingsGet();
    if (!originalSettings.ok) throw new Error(originalSettings.error.message);
    const reset = await backend.settingsUpdate({
      collapsedGroupKeys: [],
      autoFindGrouping: "all",
      downloadsGrouping: "all",
    }, originalSettings.data.revision);
    if (!reset.ok) throw new Error(reset.error.message);
    const first = await backend.favoriteSet({ namespace: "artist", value: "serein" }, true);
    const second = await backend.favoriteSet({ namespace: "artist", value: "mizuno" }, true);
    if (!first.ok || !second.ok) throw new Error("Could not prepare Auto Find favorites");
    await backend.explorationExclusionsRestore([galleryId(4051038)]);
    const seeded = await backend.downloadQueueAdd([galleryId(4051038)], "daily-group-fixture");
    if (!seeded.ok) throw new Error(seeded.error.message);
    const settingsUpdate = vi.spyOn(backend, "settingsUpdate");
    const container = document.createElement("div");
    document.body.append(container);
    let root = createRoot(container);

    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await act(async () => {
        clickButtonContaining(container, "Auto Find");
        await settle();
      });
      await act(async () => {
        clickButtonContaining(container, "즐겨찾기 작가 갱신");
        await settle(180);
      });
      expect(container).toHaveTextContent("기간별");
      expect(container).toHaveTextContent("작가별");
      expect(container).toHaveTextContent("전체");
      const autoFindGroupingToolbar = container.querySelector(".context-left > .gallery-grouping-toolbar");
      expect(autoFindGroupingToolbar).not.toBeNull();
      expect(container.querySelector(".heading-actions .gallery-grouping-toolbar")).toBeNull();
      expect(autoFindGroupingToolbar?.querySelector("button[aria-pressed='true']")).toHaveTextContent("전체");
      await act(async () => {
        clickButtonContaining(autoFindGroupingToolbar as HTMLElement, "전체");
        await settle();
      });
      expect(container.querySelector(".gallery-groups[data-group-view='auto-find']")).toBeNull();
      expect(container.querySelector(".gallery-viewport > .gallery-grid")).not.toBeNull();
      expect(autoFindGroupingToolbar?.querySelector<HTMLButtonElement>(".gallery-groups-toggle-all")).toBeDisabled();
      await act(async () => {
        clickButtonContaining(container, "작가별");
        await settle();
      });
      await vi.waitFor(() => expect(settingsUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ autoFindGrouping: "artist" }),
        expect.any(Number),
      ));
      const firstToggle = container.querySelector<HTMLButtonElement>(".gallery-group-toggle[aria-expanded='true']");
      if (!firstToggle) throw new Error("An expanded Auto Find accordion group is required");
      await act(async () => {
        firstToggle.click();
        await settle();
      });
      expect(firstToggle).toHaveAttribute("aria-expanded", "false");
      await vi.waitFor(() => expect(settingsUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ collapsedGroupKeys: expect.arrayContaining([expect.stringContaining("auto-find\u001fartist\u001f")]) }),
        expect.any(Number),
      ));

      const persisted = await backend.settingsGet();
      if (!persisted.ok) throw new Error(persisted.error.message);
      expect(persisted.data.collapsedGroupKeys).toEqual(expect.arrayContaining([
        expect.stringContaining("auto-find\u001fartist\u001f"),
      ]));

      await act(async () => {
        clickButtonContaining(container, "Downloads");
        await settle();
      });
      expect(container).toHaveTextContent("기간별");
      const downloadsGroupingToolbar = container.querySelector(".context-left > .gallery-grouping-toolbar");
      expect(downloadsGroupingToolbar).not.toBeNull();
      expect(downloadsGroupingToolbar?.querySelectorAll("button")).toHaveLength(
        autoFindGroupingToolbar?.querySelectorAll("button").length ?? 0,
      );
      const statusFilter = container.querySelector<HTMLSelectElement>("#download-status-filter");
      expect(statusFilter).toHaveAccessibleName("다운로드 상태 필터");
      expect(statusFilter).toHaveValue("all");
      expect(statusFilter?.options).toHaveLength(5);
      expect(container.querySelector(".status-filter")).toBeNull();
      expect(downloadsGroupingToolbar?.querySelector<HTMLButtonElement>("button[aria-pressed='true']")).toHaveTextContent("전체");
      expect(container.querySelector(".gallery-group-toggle")).toBeNull();
      await act(async () => {
        clickButtonContaining(downloadsGroupingToolbar as HTMLElement, "기간별");
        await settle();
      });
      expect(container.querySelector(".gallery-group-toggle")).not.toBeNull();
      expect(container.querySelectorAll(".gallery-groups[data-group-view='downloads'] .gallery-group-toggle[aria-expanded='true']")).toHaveLength(0);
      expect(container).toHaveTextContent("전부 펼치기");
      await act(async () => {
        clickButtonContaining(container, "전부 펼치기");
        await settle();
      });
      expect(container.querySelector(".gallery-groups[data-group-view='downloads'] .gallery-group-toggle[aria-expanded='true']")).not.toBeNull();
      expect(container).toHaveTextContent("전부 접기");
      await act(async () => {
        clickButtonContaining(container, "작가별");
        await settle();
      });
      const artistFolderGrid = container.querySelector<HTMLElement>(".download-artist-folder-grid[data-group-view='downloads']");
      const artistFolder = artistFolderGrid?.querySelector(".download-artist-folder-card");
      expect(artistFolderGrid).not.toBeNull();
      expect(artistFolder?.querySelector(".download-artist-folder-button")).toHaveAttribute("aria-expanded", "false");
      expect(artistFolder?.querySelector(".download-artist-folder-preview-stack")).not.toBeNull();
      expect(artistFolder?.querySelector(".download-artist-folder-latest")).toBeNull();
      expect(artistFolder?.querySelector(".download-artist-folder-tags")).not.toBeNull();
      expect(container.querySelector(".download-artist-folder-contents")).toBeNull();
      await vi.waitFor(() => expect(settingsUpdate).toHaveBeenCalledWith(
        expect.objectContaining({ downloadsGrouping: "artist" }),
        expect.any(Number),
      ));

      await act(async () => root.unmount());
      container.replaceChildren();
      root = createRoot(container);
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await act(async () => {
        clickButtonContaining(container, "Auto Find");
        await settle();
      });
      expect(container.querySelector(".gallery-grouping-control button[aria-pressed='true']")).toHaveTextContent("작가별");
      await act(async () => {
        clickButtonContaining(container, "Downloads");
        await settle();
      });
      expect(container.querySelector(".gallery-grouping-control button[aria-pressed='true']")).toHaveTextContent("작가별");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      await backend.downloadCancel(seeded.data.map((entry) => entry.entryId));
      await backend.favoriteSet({ namespace: "artist", value: "serein" }, false);
      await backend.favoriteSet({ namespace: "artist", value: "mizuno" }, false);
      const latest = await backend.settingsGet();
      if (latest.ok) await backend.settingsUpdate({
        collapsedGroupKeys: originalSettings.data.collapsedGroupKeys,
        autoFindGrouping: originalSettings.data.autoFindGrouping,
        downloadsGrouping: originalSettings.data.downloadsGrouping,
      }, latest.data.revision);
    }
  });

  it("shows persisted artist-folder tags on cold mounts while only legacy metadata waits or fails", async () => {
    const settings = await backend.settingsGet();
    if (!settings.ok) throw new Error("settings fixture unavailable");
    vi.spyOn(backend, "settingsGet").mockResolvedValue({ ok: true, data: { ...settings.data, downloadsGrouping: "artist" } });
    const ids = [galleryId(4_610_001), galleryId(4_610_002), galleryId(4_610_003)];
    vi.mocked(backend.downloadLibraryPageList).mockResolvedValue({ ok: true, data: {
      page: 1, totalItems: ids.length, items: ids.map((id, index) => ({
        gallery: {
          id, title: `Persisted ${id}`, artist: ["Saved tags artist", "Known empty artist", "Legacy artist"][index],
          pages: 12, language: "korean", ...(index === 0 ? { tags: ["female:glasses"] } : index === 1 ? { tags: [] } : {}),
        },
        download: { entryId: `persisted-${id}`, galleryId: id, revision: 1, state: "completed", progress: 100 },
      })),
    } });
    let failPending!: () => void;
    const failure = { ok: false as const, error: { code: "UNAVAILABLE", message: "metadata unavailable", retryable: false } };
    const getSummary = vi.spyOn(backend, "gallerySummaryGet").mockResolvedValue(failure)
      .mockImplementationOnce(() => new Promise((resolve) => { failPending = () => resolve(failure); }));
    const fullDetail = vi.spyOn(backend, "galleryDetailGet");
    const container = document.createElement("div");
    document.body.append(container);
    let root = createRoot(container);
    const folder = (artist: string) => [...container.querySelectorAll<HTMLElement>(".download-artist-folder-card")]
      .find((item) => item.querySelector(".download-artist-folder-name")?.textContent === artist);
    const expectPersistedFolders = () => {
      expect(folder("Saved tags artist")?.querySelector(".download-artist-folder-tags")).toHaveTextContent("glasses1");
      expect(folder("Known empty artist")?.querySelector(".download-artist-folder-tags")).toHaveTextContent("표시할 태그가 없습니다");
      expect(folder("Legacy artist")?.querySelector(".download-artist-folder-tags")).toHaveTextContent("태그 정보 미확인");
      expect(container.querySelectorAll(".download-artist-folder-button[aria-expanded='false']")).toHaveLength(3);
      expect(container.querySelector(".download-artist-folder-contents")).toBeNull();
    };
    try {
      await act(async () => { root.render(<TestApp />); await settle(); });
      expect(getSummary).not.toHaveBeenCalled();
      await act(async () => { clickButtonContaining(container, "Downloads"); await settle(); });
      expectPersistedFolders();
      expect(getSummary.mock.calls.map(([id]) => id)).toEqual([ids[2]]);
      await act(async () => { failPending(); await settle(); });
      expectPersistedFolders();

      await act(async () => root.unmount());
      container.replaceChildren();
      root = createRoot(container);
      await act(async () => { root.render(<TestApp />); await settle(); });
      await act(async () => { clickButtonContaining(container, "Downloads"); await settle(); });
      expectPersistedFolders();
      expect(getSummary.mock.calls.map(([id]) => id)).toEqual([ids[2], ids[2]]);
      expect(fullDetail).not.toHaveBeenCalled();
    } finally {
      await act(async () => { root.unmount(); failPending?.(); });
      container.remove();
    }
  });

  it("fills only missing artist-folder tags and treats a successful empty response as loaded", async () => {
    const settings = await backend.settingsGet();
    if (!settings.ok) throw new Error("settings fixture unavailable");
    vi.spyOn(backend, "settingsGet").mockResolvedValue({ ok: true, data: { ...settings.data, downloadsGrouping: "artist" } });
    const details: GalleryDetail[] = ["Saved artist", "Filled artist", "Empty artist"].map((artist, index) => ({
      id: galleryId(4_620_001 + index), title: `Tags ${index}`, artist,
      pages: 12, language: "korean", tags: index === 0 ? ["female:glasses"] : index === 1 ? ["full_color"] : [],
      series: [], characters: [], publishedRank: 20260901, popularity: 0,
      thumbnailWidth: 400, thumbnailHeight: 600, related: [], pageDimensions: [],
    }));
    vi.mocked(backend.downloadLibraryPageList).mockResolvedValue({ ok: true, data: {
      page: 1, totalItems: details.length, items: details.map((detail, index) => ({
        gallery: {
          id: detail.id, title: detail.title, artist: detail.artist, pages: detail.pages, language: detail.language,
          ...(index === 0 ? { tags: detail.tags } : {}),
        },
        download: { entryId: `tags-${detail.id}`, galleryId: detail.id, revision: 1, state: "completed", progress: 100 },
      })),
    } });
    const pending: Array<() => void> = [];
    const getSummary = vi.spyOn(backend, "gallerySummaryGet").mockImplementation((id) => new Promise((resolve) => {
      const detail = details.find((item) => item.id === id)!;
      pending.push(() => resolve({ ok: true, data: detail }));
    }));
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const folderTags = (artist: string) => [...container.querySelectorAll<HTMLElement>(".download-artist-folder-card")]
      .find((item) => item.querySelector(".download-artist-folder-name")?.textContent === artist)
      ?.querySelector(".download-artist-folder-tags");
    try {
      await act(async () => { root.render(<TestApp />); await settle(); });
      await act(async () => { clickButtonContaining(container, "Downloads"); await settle(); });
      expect(folderTags("Saved artist")).toHaveTextContent("glasses1");
      expect(folderTags("Filled artist")).toHaveTextContent("태그 정보 미확인");
      expect(folderTags("Empty artist")).toHaveTextContent("태그 정보 미확인");
      expect(new Set(getSummary.mock.calls.map(([id]) => id))).toEqual(new Set(details.slice(1).map((detail) => detail.id)));
      await act(async () => { pending.splice(0).forEach((finish) => finish()); await settle(); });
      expect(folderTags("Filled artist")).toHaveTextContent("full color1");
      expect(folderTags("Empty artist")).toHaveTextContent("표시할 태그가 없습니다");
      await act(async () => { clickButtonContaining(container, "Explore"); await settle(); });
      await act(async () => { clickButtonContaining(container, "Downloads"); await settle(); });
      expect(folderTags("Saved artist")).toHaveTextContent("glasses1");
      expect(folderTags("Filled artist")).toHaveTextContent("full color1");
      expect(folderTags("Empty artist")).toHaveTextContent("표시할 태그가 없습니다");
      expect(getSummary).toHaveBeenCalledTimes(2);
      expect(container.querySelector(".download-artist-folder-contents")).toBeNull();
    } finally {
      await act(async () => { root.unmount(); pending.splice(0).forEach((finish) => finish()); });
      container.remove();
    }
  });

  it("loads every collapsed artist folder's tags without opening or hovering, using six shared workers", async () => {
    const settings = await backend.settingsGet();
    if (!settings.ok) throw new Error("settings fixture unavailable");
    vi.spyOn(backend, "settingsGet").mockResolvedValue({ ok: true, data: { ...settings.data, downloadsGrouping: "artist" } });
    const details: GalleryDetail[] = Array.from({ length: 16 }, (_, index) => ({
      id: galleryId(4_600_001 + index), title: `Background work ${index}`, artist: "Background artist",
      pages: 12, language: "korean", tags: index === 0 ? ["full_color"] : ["female:glasses"],
      series: [], characters: [], publishedRank: 20260901, popularity: 0,
      thumbnailWidth: 400, thumbnailHeight: 600, related: [], pageDimensions: [],
    }));
    vi.mocked(backend.downloadLibraryPageList).mockResolvedValue({ ok: true, data: {
      page: 1, totalItems: details.length, items: details.map((detail) => ({
        gallery: { id: detail.id, title: detail.title, artist: detail.artist, pages: detail.pages, language: detail.language },
        download: { entryId: `background-${detail.id}`, galleryId: detail.id, revision: 1, state: "completed", progress: 100, createdAt: "2026-09-01T00:00:00Z" },
      })),
    } });
    const pending = new Map<number, () => void>();
    let active = 0;
    let maximumActive = 0;
    const fullDetail = vi.spyOn(backend, "galleryDetailGet");
    const getDetail = vi.spyOn(backend, "gallerySummaryGet").mockImplementation((id) => {
      const detail = details.find((item) => item.id === id)!;
      active += 1;
      maximumActive = Math.max(maximumActive, active);
      return new Promise((resolve) => pending.set(id, () => {
        pending.delete(id);
        active -= 1;
        resolve({ ok: true, data: detail });
      }));
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => { root.render(<TestApp />); await settle(); });
      expect(getDetail).not.toHaveBeenCalled();
      await act(async () => { clickButtonContaining(container, "Downloads"); await settle(); });
      expect(container.querySelector(".download-artist-folder-button")).toHaveAttribute("aria-expanded", "false");
      expect(container.querySelector(".download-artist-folder-contents")).toBeNull();
      expect(getDetail).toHaveBeenCalledTimes(6);
      for (let batch = 0; batch < 4 && pending.size; batch += 1) {
        await act(async () => { [...pending.values()].forEach((finish) => finish()); await settle(); });
      }
      expect(getDetail).toHaveBeenCalledTimes(16);
      expect(maximumActive).toBe(6);
      expect(fullDetail).not.toHaveBeenCalled();
      expect(new Set(getDetail.mock.calls.map(([id]) => id)).size).toBe(16);
      expect(container.querySelector(".download-artist-folder-tags")).toHaveTextContent("full color1");
      expect(container.querySelector(".download-artist-folder-tags")).toHaveTextContent("glasses15");
      expect(container.querySelector(".download-artist-folder-contents")).toBeNull();
      await act(async () => { clickButtonContaining(container, "Explore"); await settle(); });
      await act(async () => { clickButtonContaining(container, "Downloads"); await settle(); });
      expect(getDetail).toHaveBeenCalledTimes(16);
    } finally {
      await act(async () => { root.unmount(); [...pending.values()].forEach((finish) => finish()); });
      container.remove();
    }
  });

  it("stops unopened artist-folder metadata work when leaving Downloads", async () => {
    const settings = await backend.settingsGet();
    if (!settings.ok) throw new Error("settings fixture unavailable");
    vi.spyOn(backend, "settingsGet").mockResolvedValue({ ok: true, data: { ...settings.data, downloadsGrouping: "artist" } });
    const ids = Array.from({ length: 12 }, (_, index) => galleryId(4_700_001 + index));
    vi.mocked(backend.downloadLibraryPageList).mockResolvedValue({ ok: true, data: {
      page: 1, totalItems: ids.length, items: ids.map((id) => ({
        gallery: { id, title: `Queued ${id}`, artist: "Queued artist", pages: 1, language: "korean" },
        download: { entryId: `queued-${id}`, galleryId: id, revision: 1, state: "completed", progress: 100 },
      })),
    } });
    const pending: Array<() => void> = [];
    const getDetail = vi.spyOn(backend, "gallerySummaryGet").mockImplementation(() => new Promise((resolve) => {
      pending.push(() => resolve({ ok: false, error: { code: "UNAVAILABLE", message: "fixture unavailable", retryable: false } }));
    }));
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => { root.render(<TestApp />); await settle(); });
      await act(async () => { clickButtonContaining(container, "Downloads"); await settle(); });
      expect(getDetail).toHaveBeenCalledTimes(6);
      await act(async () => { clickButtonContaining(container, "Explore"); await settle(); });
      await act(async () => { pending.splice(0).forEach((finish) => finish()); await settle(); });
      expect(getDetail).toHaveBeenCalledTimes(6);
    } finally {
      await act(async () => { root.unmount(); pending.splice(0).forEach((finish) => finish()); });
      container.remove();
    }
  });

  it("runs internal duplicate analysis only for the selected completed albums", async () => {
    const downloads: DownloadPage = {
      page: 1,
      totalItems: 3,
      entries: [
        { entryId: "selected-entry-a", galleryId: galleryId(4051038), revision: 1, state: "completed", progress: 100 },
        { entryId: "selected-entry-b", galleryId: galleryId(4050754), revision: 1, state: "completed", progress: 100 },
        { entryId: "unfinished-entry", galleryId: galleryId(4051027), revision: 1, state: "failed", progress: 40 },
      ],
    };
    const finishedRun: InternalScanRun = {
      runId: "selected-internal-run",
      revision: 1,
      state: "completed",
      totalArtifacts: 1,
      scannedArtifacts: 1,
      totalPages: 24,
      comparedPairs: 276,
      groupsFound: 0,
      algorithmVersion: 4,
      skippedArtifacts: 0,
      skippedPages: 0,
      startedAt: "2026-08-23T00:00:00.000Z",
      updatedAt: "2026-08-23T00:00:01.000Z",
      finishedAt: "2026-08-23T00:00:01.000Z",
    };
    const review: InternalDuplicateReview = {
      entryId: "selected-entry-a",
      galleryId: galleryId(4051038),
      title: "Archive of Rain",
      groups: [{
        groupId: "selected-entry-a-group",
        blockId: "selected-entry-a-block",
        sequenceIndex: 0,
        revision: 0,
        entryId: "selected-entry-a",
        galleryId: galleryId(4051038),
        relation: "exact",
        confidence: 1,
        recommendedKeepSourcePage: 1,
        pages: [1, 2].map((sourcePage) => ({
          sourcePage,
          exactSha256: true,
          visualSimilarity: 1,
          detailHashDistance: 0,
          lowInformation: false,
        })),
        resolved: false,
        createdAt: "2026-08-23T00:00:00.000Z",
        updatedAt: "2026-08-23T00:00:00.000Z",
      }],
      quarantineRecords: [],
    };
    vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({ ok: true, data: downloads });
    vi.spyOn(backend, "internalDuplicateSnapshot").mockResolvedValue({
      ok: true,
      data: { groups: review.groups, quarantineRecords: [], skips: [] },
    });
    vi.spyOn(backend, "internalDuplicateReviewGet").mockResolvedValue({ ok: true, data: review });
    const scanStart = vi.spyOn(backend, "internalDuplicateScanStart").mockResolvedValue({
      ok: true,
      data: finishedRun,
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await act(async () => {
        clickButtonContaining(container, "Downloads");
        await settle();
      });

      const scanButton = clickButtonContaining(container, "선택 앨범 내부 페이지 검사");
      expect(scanButton).toBeDisabled();
      expect(scanStart).not.toHaveBeenCalled();

      const first = container.querySelector<HTMLElement>('[data-gallery-id="4051038"]');
      const second = container.querySelector<HTMLElement>('[data-gallery-id="4050754"]');
      const unfinished = container.querySelector<HTMLElement>('[data-gallery-id="4051027"]');
      if (!first || !second || !unfinished) throw new Error("Download selection fixtures were not rendered");
      expect(first.querySelector(".internal-result-badge")).toHaveTextContent("내부 검토 1");
      expect(second.querySelector(".internal-result-badge")).toBeNull();
      expect(unfinished.querySelector(".internal-result-badge")).toBeNull();

      await act(async () => {
        first.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
        await settle();
      });
      expect(scanButton).toBeEnabled();
      expect(scanButton).toHaveTextContent("선택 앨범 내부 페이지 검사 (1)");
      await act(async () => {
        scanButton.click();
        await settle();
      });
      expect(scanStart).toHaveBeenLastCalledWith({ entryIds: ["selected-entry-a"] });

      await act(async () => {
        second.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1, ctrlKey: true }));
        await settle();
      });
      expect(scanButton).toHaveTextContent("선택 앨범 내부 페이지 검사 (2)");
      await act(async () => {
        scanButton.click();
        await settle();
      });
      expect(scanStart).toHaveBeenLastCalledWith({
        entryIds: ["selected-entry-a", "selected-entry-b"],
      });

      await act(async () => {
        unfinished.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1, ctrlKey: true }));
        await settle();
      });
      expect(scanButton).toBeDisabled();
      expect(scanButton).toHaveAttribute("title", "선택한 항목이 모두 다운로드 완료 상태여야 합니다.");
      expect(scanStart).toHaveBeenCalledTimes(2);

      await act(async () => {
        clickButtonContaining(container, "선택 해제");
        first.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
        await settle();
      });
      expect(container.textContent).not.toContain("선택 앨범 내부 결과 열기");
      const internalResultBadge = first.querySelector<HTMLButtonElement>(".internal-result-badge");
      if (!internalResultBadge) throw new Error("Per-album internal result badge was not rendered");
      await act(async () => {
        internalResultBadge.click();
        await settle();
      });
      expect(container.querySelector(".internal-review-dialog")).toHaveAttribute("open");
      await act(async () => {
        clickButtonContaining(container, "이 앨범 다시 검사");
        await settle();
      });
      expect(scanStart).toHaveBeenLastCalledWith({ entryIds: ["selected-entry-a"] });
      expect(scanStart).toHaveBeenCalledTimes(3);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it.each([false, true])("keeps Ctrl-selected merge pages and applies source exclusion only after success (failure=%s)", async (fail) => {
    const fixture = await prepareContainmentBatchApp();
    const review = { ...fixture.direct, candidates: [fixture.direct.candidates[0]!] };
    fixture.reviews.set(review.reviewId, review);
    const source = fixture.refs[1]!;
    let finish: ((result: ApiResult<DownloadOverlapMergeResult>) => void) | undefined;
    const pending = new Promise<ApiResult<DownloadOverlapMergeResult>>((resolve) => { finish = resolve; });
    const merge = vi.spyOn(backend, "downloadOverlapMerge").mockReturnValue(pending);
    const invalidate = vi.spyOn(testThumbnailClient, "invalidate");
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await act(async () => {
        clickButtonContaining(container, "Downloads");
        await settle();
      });
      const status = container.querySelector<HTMLButtonElement>(`[data-gallery-id="${fixture.keeper.galleryId}"] .status-pill`);
      expect(status).not.toBeNull();
      await act(async () => {
        status!.click();
        await settle();
      });
      const dialog = container.querySelector<HTMLDialogElement>(".download-overlap-dialog")!;
      expect(dialog).toHaveAttribute("open");
      const sourceCell = (page: number) => dialog.querySelector<HTMLElement>(`.download-overlap-page-cell[aria-label^="기존 A ${page}페이지"]`)!;
      for (const page of [3, 1]) await act(async () => {
        sourceCell(page).dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true, button: 0 }));
      });
      expect(dialog.querySelectorAll(".is-merge-source")).toHaveLength(2);
      expect(dialog.querySelectorAll(".is-merge-target")).toHaveLength(2);
      const apply = dialog.querySelector<HTMLButtonElement>(".download-overlap-merge-apply")!;
      expect(apply).toHaveTextContent("선택한 2장 병합");
      await act(async () => { apply.click(); });
      expect(merge).toHaveBeenCalledExactlyOnceWith({
        reviewId: review.reviewId, expectedRevision: review.revision,
        candidateId: review.candidates[0]!.candidateId,
        sourceSide: "existing", sourcePages: [1, 3], excludeSource: true,
      });
      expect(apply).toBeDisabled();
      expect(dialog.querySelector(".download-overlap-merge-clear")).toBeDisabled();
      await act(async () => {
        apply.click();
        sourceCell(2).dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true, button: 0 }));
      });
      expect(merge).toHaveBeenCalledOnce();
      expect(dialog.querySelectorAll(".is-merge-source")).toHaveLength(2);
      expect(invalidate).not.toHaveBeenCalled();
      expect(fixture.appliedIds).toEqual([]);

      await act(async () => {
        if (fail) {
          finish?.({ ok: false, error: { code: "DOWNLOAD_OVERLAP_MERGE_FAILED", message: "Fixture merge verification failed; original files retained", retryable: false } });
        } else {
          fixture.appliedIds.push(Number(source.galleryId));
          const index = fixture.entries.findIndex((entry) => entry.entryId === source.entryId);
          fixture.entries[index] = { ...fixture.entries[index]!, state: "cancelled", revision: 2 };
          fixture.reviews.set(review.reviewId, { ...review, state: "stale", revision: review.revision + 1 });
          finish?.({ ok: true, data: {
            mergeId: "app-merge-fixture", sourceGalleryId: source.galleryId,
            targetGalleryId: fixture.keeper.galleryId, replacedPages: 2,
            backupPath: ".atsumi-merge-backups/app-merge-fixture", affectedReviewIds: [review.reviewId], sourceExcluded: true,
          } });
        }
        await pending;
        await settle();
      });
      expect(fixture.decision).not.toHaveBeenCalled();
      if (fail) {
        expect(dialog).toHaveAttribute("open");
        expect(dialog).toHaveTextContent("Fixture merge verification failed; original files retained");
        expect(dialog.querySelectorAll(".is-merge-source")).toHaveLength(2);
        expect(apply).not.toBeDisabled();
        expect(invalidate).not.toHaveBeenCalled();
        expect(fixture.appliedIds).toEqual([]);
        await act(async () => dialog.querySelector<HTMLButtonElement>('button[aria-label="닫기"]')!.click());
      } else {
        expect(container.querySelector(".download-overlap-dialog[open]")).toBeNull();
        expect(container).toHaveTextContent("2장 병합 완료");
        expect(invalidate).toHaveBeenCalledOnce();
        const predicate = invalidate.mock.calls[0]![0];
        expect(predicate({ kind: "gallery-cover", galleryId: fixture.keeper.galleryId })).toBe(true);
        expect(predicate({ kind: "source-page", galleryId: fixture.keeper.galleryId, page: 1 })).toBe(true);
        expect(predicate({ kind: "artifact-page", entryId: fixture.keeper.entryId, page: 1 })).toBe(true);
        expect(predicate({ kind: "gallery-cover", galleryId: source.galleryId })).toBe(false);
        expect(predicate({ kind: "artifact-page", entryId: source.entryId, page: 1 })).toBe(false);
      }
      await act(async () => {
        clickButtonContaining(container, "Explore");
        await settle();
      });
      await submitExploreSearch(container);
      const sourceCard = container.querySelector(`.gallery-card[data-gallery-id="${source.galleryId}"]`)!;
      const keeperCard = container.querySelector(`.gallery-card[data-gallery-id="${fixture.keeper.galleryId}"]`)!;
      expect(sourceCard.classList.contains("is-exploration-blind")).toBe(!fail);
      expect(keeperCard).not.toHaveClass("is-exploration-blind");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("routes a typed download overlap review without looking up a global duplicate candidate", async () => {
    vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({
      ok: true,
      data: {
        page: 1,
        totalItems: 1,
        entries: [{
          entryId: "incoming-overlap-entry",
          galleryId: galleryId(4051038),
          revision: 3,
          state: "review_required",
          progress: 100,
          reviewKind: "gallery_duplicate",
          reviewId: "app-overlap-review",
        }],
      },
    });
    vi.spyOn(backend, "duplicateSnapshot").mockResolvedValue({
      ok: true,
      data: {
        profile: {
          profileVersion: 1,
          algorithmVersion: 3,
          dHashBits: 1024,
          pHashBits: 64,
          visualMatchThreshold: 0.82,
          lowInformationStdDevThreshold: 8,
        },
        candidates: [],
      },
    });
    const overlapGet = vi.spyOn(backend, "downloadOverlapReviewGet");
    const globalGet = vi.spyOn(backend, "duplicateReviewGet");
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await act(async () => {
        clickButtonContaining(container, "Downloads");
        await settle();
      });
      const status = container.querySelector<HTMLButtonElement>('[data-gallery-id="4051038"] .status-pill');
      if (!status) throw new Error("Overlap review status was not rendered");
      expect(status).toHaveAttribute("title", expect.stringContaining("다운로드 판본 중복"));
      await act(async () => {
        status.click();
        await settle();
      });
      expect(overlapGet).toHaveBeenCalledWith("app-overlap-review");
      expect(globalGet).not.toHaveBeenCalled();
      expect(container.querySelector(".download-overlap-dialog")).toHaveAttribute("open");
      expect(container.textContent).toContain("다운로드 판본 중복 검토");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("skips an ineligible overlap review and chains returned revisions for the next eligible review", async () => {
    const originalSettings = await backend.settingsGet();
    if (!originalSettings.ok) throw new Error(originalSettings.error.message);
    vi.spyOn(backend, "settingsGet").mockResolvedValue({
      ok: true,
      data: { ...originalSettings.data, downloadOverlapAutoMode: "strict_quarantine" },
    });

    const pagePairs = Array.from({ length: 20 }, (_, index) => ({
      incomingSourcePage: index + 1,
      existingSourcePage: index + 1,
      exactSha256: true,
      dHashDistance: 0,
      pHashDistance: 0,
      detailHashDistance: 0,
      edgeSimilarity: 1,
      visualSimilarity: 1,
      lowInformation: false,
    }));
    const ineligibleIncoming = mockGalleries[3]!;
    const eligibleIncoming = mockGalleries[0]!;
    const eligibleExisting = [mockGalleries[6]!, mockGalleries[5]!];
    const eligibleReview: DownloadOverlapReview = {
      reviewId: "sweep-eligible-review",
      entryId: "sweep-eligible-incoming",
      incoming: {
        entryId: "sweep-eligible-incoming",
        galleryId: eligibleIncoming.id,
        title: eligibleIncoming.title,
        artists: [eligibleIncoming.artist],
        pageCount: 25,
      },
      revision: 7,
      state: "pending",
      profileVersion: 1,
      policyVersion: 1,
      incomingFingerprint: "a".repeat(64),
      candidates: eligibleExisting.map((gallery, index) => ({
        candidateId: `sweep-eligible-candidate-${index + 1}`,
        existing: {
          entryId: `sweep-eligible-existing-${index + 1}`,
          galleryId: gallery.id,
          title: gallery.title,
          artists: [gallery.artist],
          pageCount: 20,
        },
        existingFingerprint: String(index + 1).repeat(64),
        relation: "incoming_contains_existing" as const,
        confidence: 0.99,
        matchedPages: 20,
        exactPages: 20,
        visualPages: 0,
        existingCoverage: 1,
        incomingCoverage: 0.8,
        existingUniquePages: 0,
        incomingUniquePages: 5,
        longestAlignedRun: 20,
        rank: index + 1,
        pagePairs,
      })),
      createdAt: "2026-09-04T00:00:00Z",
      updatedAt: "2026-09-04T00:00:00Z",
    };
    const ineligibleReview: DownloadOverlapReview = {
      ...eligibleReview,
      reviewId: "sweep-ineligible-review",
      entryId: "sweep-ineligible-incoming",
      incoming: {
        entryId: "sweep-ineligible-incoming",
        galleryId: ineligibleIncoming.id,
        title: ineligibleIncoming.title,
        artists: [ineligibleIncoming.artist],
        pageCount: 25,
      },
      revision: 3,
      incomingFingerprint: "f".repeat(64),
      candidates: [{
        ...eligibleReview.candidates[0]!,
        candidateId: "sweep-ineligible-candidate",
        relation: "partial_overlap",
      }],
    };
    const afterFirstDecision: DownloadOverlapReview = {
      ...eligibleReview,
      revision: 41,
      candidates: eligibleReview.candidates.map((candidate, index) => index === 0
        ? { ...candidate, decision: "existing_removed" }
        : candidate),
      updatedAt: "2026-09-04T00:00:01Z",
    };
    const afterSecondDecision: DownloadOverlapReview = {
      ...afterFirstDecision,
      revision: 42,
      state: "resolved",
      candidates: afterFirstDecision.candidates.map((candidate) => ({
        ...candidate,
        decision: "existing_removed",
      })),
      updatedAt: "2026-09-04T00:00:02Z",
      resolvedAt: "2026-09-04T00:00:02Z",
    };
    const downloadPage: DownloadPage = {
      page: 1,
      totalItems: 2,
      entries: [
        {
          entryId: ineligibleReview.entryId,
          galleryId: ineligibleReview.incoming.galleryId,
          revision: 1,
          state: "review_required",
          progress: 100,
          reviewKind: "gallery_duplicate",
          reviewId: ineligibleReview.reviewId,
        },
        {
          entryId: eligibleReview.entryId,
          galleryId: eligibleReview.incoming.galleryId,
          revision: 1,
          state: "review_required",
          progress: 100,
          reviewKind: "gallery_duplicate",
          reviewId: eligibleReview.reviewId,
        },
      ],
    };
    vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({ ok: true, data: downloadPage });
    vi.spyOn(backend, "downloadLibraryPageList").mockResolvedValue({
      ok: true,
      data: libraryPageFromEntries(downloadPage),
    });
    let eligibleCurrent = eligibleReview;
    const overlapGet = vi.spyOn(backend, "downloadOverlapReviewGet").mockImplementation(async (reviewId) => ({
      ok: true,
      data: reviewId === ineligibleReview.reviewId ? ineligibleReview : eligibleCurrent,
    }));
    const decision = vi.spyOn(backend, "downloadOverlapDecisionApply").mockImplementation(async () => {
      eligibleCurrent = eligibleCurrent.revision === eligibleReview.revision
        ? afterFirstDecision
        : afterSecondDecision;
      return { ok: true, data: { review: eligibleCurrent, resumed: eligibleCurrent.state === "resolved", cancelled: false } };
    });

    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await vi.waitFor(() => expect(decision).toHaveBeenCalledTimes(2));

      expect(overlapGet).toHaveBeenNthCalledWith(1, ineligibleReview.reviewId);
      expect(overlapGet).toHaveBeenNthCalledWith(2, eligibleReview.reviewId);
      expect(decision.mock.calls.map(([request]) => request)).toMatchObject([
        {
          reviewId: eligibleReview.reviewId,
          candidateId: eligibleReview.candidates[0]!.candidateId,
          expectedRevision: 7,
          actor: "automation",
        },
        {
          reviewId: eligibleReview.reviewId,
          candidateId: eligibleReview.candidates[1]!.candidateId,
          expectedRevision: 41,
          actor: "automation",
        },
      ]);
      expect(JSON.parse(decision.mock.calls[1]![0].featureSnapshotJson ?? "{}"))
        .toMatchObject({ reviewRevision: 41 });
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("keeps an earlier automatic overlap exclusion visible when a later candidate decision fails", async () => {
    const originalSettings = await backend.settingsGet();
    if (!originalSettings.ok) throw new Error(originalSettings.error.message);
    const settingsGet = vi.spyOn(backend, "settingsGet").mockResolvedValue({
      ok: true,
      data: { ...originalSettings.data, downloadOverlapAutoMode: "strict_quarantine" },
    });

    const incoming = mockGalleries[3]!;
    const excludedExisting = mockGalleries[6]!;
    const pendingExisting = mockGalleries[5]!;
    const pagePairs = Array.from({ length: 20 }, (_, index) => ({
      incomingSourcePage: index + 1,
      existingSourcePage: index + 1,
      exactSha256: true,
      dHashDistance: 0,
      pHashDistance: 0,
      detailHashDistance: 0,
      edgeSimilarity: 1,
      visualSimilarity: 1,
      lowInformation: false,
    }));
    const initialReview: DownloadOverlapReview = {
      reviewId: "partial-auto-overlap-review",
      entryId: "partial-auto-incoming",
      incoming: {
        entryId: "partial-auto-incoming",
        galleryId: incoming.id,
        title: incoming.title,
        artists: [incoming.artist],
        pageCount: 25,
      },
      revision: 1,
      state: "pending",
      profileVersion: 1,
      policyVersion: 1,
      incomingFingerprint: "b".repeat(64),
      candidates: [excludedExisting, pendingExisting].map((gallery, index) => ({
        candidateId: `partial-auto-candidate-${index + 1}`,
        existing: {
          entryId: `partial-auto-existing-${index + 1}`,
          galleryId: gallery.id,
          title: gallery.title,
          artists: [gallery.artist],
          pageCount: 20,
        },
        existingFingerprint: String(index + 1).repeat(64),
        relation: "incoming_contains_existing" as const,
        confidence: 0.99,
        matchedPages: 20,
        exactPages: 20,
        visualPages: 0,
        existingCoverage: 1,
        incomingCoverage: 0.8,
        existingUniquePages: 0,
        incomingUniquePages: 5,
        longestAlignedRun: 20,
        rank: index + 1,
        pagePairs,
      })),
      createdAt: "2026-09-02T00:00:00Z",
      updatedAt: "2026-09-02T00:00:00Z",
    };
    const reviewAfterFirstDecision: DownloadOverlapReview = {
      ...initialReview,
      revision: 2,
      candidates: initialReview.candidates.map((candidate, index) => index === 0
        ? { ...candidate, decision: "existing_removed" }
        : candidate),
      updatedAt: "2026-09-02T00:00:01Z",
    };
    vi.spyOn(backend, "explorationExclusionsList").mockResolvedValue({ ok: true, data: [] });
    const downloadPage: DownloadPage = {
      page: 1,
      totalItems: 3,
      entries: [
        {
          entryId: initialReview.entryId,
          galleryId: incoming.id,
          revision: 1,
          state: "review_required",
          progress: 100,
          reviewKind: "gallery_duplicate",
          reviewId: initialReview.reviewId,
        },
        {
          entryId: initialReview.candidates[0]!.existing.entryId,
          galleryId: excludedExisting.id,
          revision: 1,
          state: "failed",
          progress: 100,
        },
        {
          entryId: initialReview.candidates[1]!.existing.entryId,
          galleryId: pendingExisting.id,
          revision: 1,
          state: "completed",
          progress: 100,
        },
      ],
    };
    vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({ ok: true, data: downloadPage });
    const downloadLibrary = vi.spyOn(backend, "downloadLibraryPageList").mockResolvedValue({
      ok: true,
      data: libraryPageFromEntries(downloadPage),
    });
    const overlapGet = vi.spyOn(backend, "downloadOverlapReviewGet").mockResolvedValue({ ok: true, data: initialReview });
    const decision = vi.spyOn(backend, "downloadOverlapDecisionApply")
      .mockResolvedValueOnce({
        ok: true,
        data: { review: reviewAfterFirstDecision, resumed: false, cancelled: false },
      })
      .mockResolvedValueOnce({
        ok: false,
        error: {
          code: "PARTIAL_AUTOMATION_FIXTURE",
          message: "second candidate failed",
          retryable: true,
          action: "review",
        },
      });
    const { download: _download, ...excludedSummary } = excludedExisting;
    vi.spyOn(backend, "searchSubmit").mockResolvedValue({
      ok: true,
      data: {
        queryId: "partial-auto-exclusion-search",
        firstPage: {
          page: 1,
          totalPages: 1,
          items: [{
            ...excludedSummary,
            publishedRank: Number(excludedExisting.publishedAt.replaceAll("-", "")),
            popularity: excludedExisting.score,
            thumbnailWidth: excludedExisting.thumbnailWidth ?? 512,
            thumbnailHeight: excludedExisting.thumbnailHeight ?? 768,
          }],
        },
      },
    });

    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await vi.waitFor(() => expect(settingsGet).toHaveBeenCalled());
      await vi.waitFor(() => expect(downloadLibrary).toHaveBeenCalled());
      await vi.waitFor(() => expect(overlapGet).toHaveBeenCalled());
      await vi.waitFor(() => expect(decision).toHaveBeenCalledTimes(2));
      expect(decision).toHaveBeenNthCalledWith(1, expect.objectContaining({
        action: "remove_existing_continue",
        candidateId: initialReview.candidates[0]!.candidateId,
        actor: "automation",
      }));
      expect(decision).toHaveBeenNthCalledWith(2, expect.objectContaining({
        action: "remove_existing_continue",
        candidateId: initialReview.candidates[1]!.candidateId,
        actor: "automation",
      }));

      await submitExploreSearch(container);
      const excludedCard = container.querySelector<HTMLElement>(`[data-gallery-id="${excludedExisting.id}"]`);
      expect(excludedCard).toHaveClass("is-exploration-blind");
      expect(excludedCard).toHaveAccessibleName(expect.stringContaining("중복 판정으로 제외"));
      expect(container).toHaveTextContent("자동 분류 중단 · 직접 검토 필요 · second candidate failed");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("restores the persistent automatic-review badge and lazily supports list-only restore", async () => {
    const removedGalleryIds = [galleryId(4_051_038), galleryId(4_050_754)];
    const historyItem: DownloadOverlapAutomationHistoryItem = {
      reviewId: "persistent-auto-review",
      incomingGalleryId: galleryId(4_053_291),
      title: "새로고침 뒤 자동분류 기록",
      occurredAt: "2026-09-04T01:30:00Z",
      reviewState: "resolved",
      removeIncomingCount: 0,
      removeExistingCount: 2,
      removedGalleryIds,
    };
    let acknowledged = false;
    const historyList = vi.spyOn(backend, "downloadOverlapAutomationHistoryList").mockImplementation(async (request) => ({
      ok: true,
      data: {
        page: request.page,
        pageSize: request.pageSize,
        totalItems: 1,
        unacknowledgedItems: acknowledged ? 0 : 1,
        items: request.page === 1 ? [{
          ...historyItem,
          ...(acknowledged ? { acknowledgedAt: "2026-09-04T01:31:00Z" } : {}),
        }] : [],
      },
    }));
    const historyAcknowledge = vi.spyOn(backend, "downloadOverlapAutomationHistoryAcknowledge")
      .mockImplementation(async (reviewId) => {
        acknowledged = true;
        return {
          ok: true,
          data: { ...historyItem, reviewId, acknowledgedAt: "2026-09-04T01:31:00Z" },
        };
      });
    const restoreExclusions = vi.spyOn(backend, "explorationExclusionsRestore").mockResolvedValue({
      ok: true,
      data: {
        restoredGalleryIds: removedGalleryIds,
        snapshot: { candidates: [], cutoffEvidence: [], truncations: [] },
      },
    });
    vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({
      ok: true,
      data: { page: 1, totalItems: 0, entries: [] },
    });
    vi.spyOn(backend, "downloadLibraryPageList").mockResolvedValue({
      ok: true,
      data: { page: 1, totalItems: 0, items: [] },
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await vi.waitFor(() => expect(historyList).toHaveBeenCalledWith({ page: 1, pageSize: 1 }));
      expect(historyList).not.toHaveBeenCalledWith({ page: 1, pageSize: 50 });
      expect(container.querySelector('[aria-label="활동 기록"] .activity-count')).toHaveTextContent("1");

      await act(async () => {
        container.querySelector<HTMLButtonElement>('button[aria-label="활동 기록"]')?.click();
        await settle();
      });
      await vi.waitFor(() => expect(historyList).toHaveBeenCalledWith({ page: 1, pageSize: 50 }));
      await act(async () => {
        [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')]
          .find((button) => button.textContent?.includes("자동분류 검토"))?.click();
      });
      expect(container.querySelector("#activity-automation-panel")).toHaveTextContent("새로고침 뒤 자동분류 기록");
      expect(container.querySelector("#activity-automation-panel")).toHaveTextContent("격리된 실제 파일은 복원하지 않습니다");

      await act(async () => {
        [...container.querySelectorAll<HTMLButtonElement>("#activity-automation-panel button")]
          .find((button) => button.textContent === "목록에 복원")?.click();
        await settle();
      });
      expect(restoreExclusions).toHaveBeenCalledWith(removedGalleryIds);
      expect(historyAcknowledge).toHaveBeenCalledWith(historyItem.reviewId);
      expect(container.querySelector("#activity-automation-panel")).toHaveTextContent("확인 완료");
      expect(container.querySelector('[aria-label="활동 기록"] .activity-count')).toBeNull();
      expect(container).toHaveTextContent("격리된 실제 파일은 복원하지 않았습니다");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("hydrates, routes, and clears per-artifact internal scan progress on exact Downloads cards", async () => {
    const downloads: DownloadPage = {
      page: 1,
      totalItems: 2,
      entries: [
        { entryId: "progress-entry-a", galleryId: galleryId(4051038), revision: 1, state: "completed", progress: 100 },
        { entryId: "progress-entry-b", galleryId: galleryId(4050754), revision: 1, state: "completed", progress: 100 },
      ],
    };
    const runningRun: InternalScanRun = {
      runId: "progress-internal-run",
      revision: 0,
      state: "running",
      totalArtifacts: 2,
      scannedArtifacts: 0,
      totalPages: 48,
      comparedPairs: 0,
      groupsFound: 0,
      algorithmVersion: 4,
      skippedArtifacts: 0,
      skippedPages: 0,
      startedAt: "2026-08-25T00:00:00.000Z",
      updatedAt: "2026-08-25T00:00:00.000Z",
    };
    const completedRun: InternalScanRun = {
      ...runningRun,
      revision: 1,
      state: "completed",
      scannedArtifacts: 2,
      comparedPairs: 552,
      updatedAt: "2026-08-25T00:00:02.000Z",
      finishedAt: "2026-08-25T00:00:02.000Z",
    };
    const progressA: InternalArtifactScanProgress = {
      runId: runningRun.runId,
      sequence: 1,
      entryId: "progress-entry-a",
      galleryId: galleryId(4051038),
      artifactIndex: 1,
      totalArtifacts: 2,
      processedPages: 6,
      totalPages: 24,
      comparedPairs: 0,
      totalPairs: 276,
      progressPercent: 18,
      stage: "hashing",
    };
    const initialSnapshot: InternalDuplicateSnapshot = {
      run: runningRun,
      groups: [],
      quarantineRecords: [],
      skips: [],
    };
    const completedSnapshot: InternalDuplicateSnapshot = {
      ...initialSnapshot,
      run: completedRun,
    };
    const eventHandlers = new Map<keyof BackendEventMap, (payload: unknown) => void>();

    vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({ ok: true, data: downloads });
    vi.spyOn(backend, "internalDuplicateSnapshot")
      .mockResolvedValueOnce({ ok: true, data: initialSnapshot })
      .mockResolvedValue({ ok: true, data: completedSnapshot });
    const activeArtifact = vi.spyOn(backend, "internalDuplicateActiveArtifact")
      .mockResolvedValue({ ok: true, data: progressA });
    vi.spyOn(backend, "on").mockImplementation(async (event, handler) => {
      eventHandlers.set(event, handler as (payload: unknown) => void);
      return () => eventHandlers.delete(event);
    });

    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await act(async () => {
        clickButtonContaining(container, "Downloads");
        await settle();
      });

      await vi.waitFor(() => {
        expect(container.querySelector('[data-gallery-id="4051038"] .internal-duplicate-card-progress'))
          .toHaveTextContent("내부 검사 1/2");
      });
      expect(activeArtifact).toHaveBeenCalled();
      expect(container.querySelector('[data-gallery-id="4051038"] .internal-duplicate-card-progress'))
        .toHaveAccessibleName(expect.stringContaining("페이지 6/24"));
      expect(container.querySelector('[data-gallery-id="4050754"] .internal-duplicate-card-progress')).toBeNull();

      const progressHandler = eventHandlers.get("internal-duplicate:artifact-progress");
      const runHandler = eventHandlers.get("internal-duplicate:changed");
      if (!progressHandler || !runHandler) throw new Error("Internal duplicate event handlers were not registered");

      await act(async () => {
        progressHandler({
          ...progressA,
          sequence: 2,
          galleryId: galleryId(4050754),
          progressPercent: 44,
        } satisfies InternalArtifactScanProgress);
        await settle();
      });
      expect(container.querySelector(".internal-duplicate-card-progress")).toBeNull();

      const progressB: InternalArtifactScanProgress = {
        ...progressA,
        sequence: 3,
        entryId: "progress-entry-b",
        galleryId: galleryId(4050754),
        artifactIndex: 2,
        processedPages: 24,
        comparedPairs: 138,
        progressPercent: 78,
        stage: "comparing",
      };
      await act(async () => {
        progressHandler(progressB);
        await settle();
      });
      expect(container.querySelector('[data-gallery-id="4051038"] .internal-duplicate-card-progress')).toBeNull();
      expect(container.querySelector('[data-gallery-id="4050754"] .internal-duplicate-card-progress'))
        .toHaveAccessibleName(expect.stringContaining("비교 138/276"));

      await act(async () => {
        progressHandler({ ...progressA, sequence: 2 });
        await settle();
      });
      expect(container.querySelector('[data-gallery-id="4050754"] .internal-duplicate-card-progress'))
        .toHaveAttribute("aria-valuenow", "78");

      let resolveLateHydration: ((value: Awaited<ReturnType<typeof backend.internalDuplicateActiveArtifact>>) => void) | undefined;
      activeArtifact.mockImplementationOnce(() => new Promise((resolve) => {
        resolveLateHydration = resolve;
      }));
      const replacementRun: InternalScanRun = {
        ...runningRun,
        runId: "progress-internal-run-replacement",
        startedAt: "2026-08-25T00:01:00.000Z",
        updatedAt: "2026-08-25T00:01:00.000Z",
      };
      await act(async () => {
        runHandler(replacementRun);
        await settle();
      });
      expect(resolveLateHydration).toBeTypeOf("function");
      await act(async () => {
        progressHandler({ ...progressB, runId: replacementRun.runId, sequence: 1 });
        await settle();
      });
      expect(container.querySelector('[data-gallery-id="4050754"] .internal-duplicate-card-progress'))
        .toHaveAttribute("aria-valuenow", "78");

      await act(async () => {
        resolveLateHydration?.({ ok: true, data: null });
        await settle();
      });
      expect(container.querySelector('[data-gallery-id="4050754"] .internal-duplicate-card-progress'))
        .toHaveAttribute("aria-valuenow", "78");

      await act(async () => {
        runHandler({
          ...completedRun,
          runId: replacementRun.runId,
          startedAt: replacementRun.startedAt,
        });
        await settle();
      });
      await vi.waitFor(() => expect(container.querySelector(".internal-duplicate-card-progress")).toBeNull());
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("submits only the two manually entered album IDs without starting a library scan", async () => {
    const run = { runId: "selected-only", revision: 1, state: "completed" as const, totalArtifacts: 2, hashedArtifacts: 2, totalPairs: 1, comparedPairs: 1, candidatesFound: 0, startedAt: "now", updatedAt: "now" };
    const scan = vi.spyOn(backend, "duplicateScanStart").mockResolvedValue({ ok: true, data: run });
    vi.spyOn(backend, "duplicateSnapshot").mockResolvedValue({ ok: true, data: { profile: { profileVersion: 1, dHashBits: 1024, pHashBits: 64 } as never, candidates: [] } });
    const container=document.createElement("div"); document.body.append(container);
    const root=createRoot(container);
    try {
      await act(async () => { root.render(<TestApp />); await settle(); });
      await act(async () => { clickButtonContaining(container,"Downloads"); await settle(); });
      await act(async () => clickButtonContaining(container,"두 앨범 직접 대조"));
      const inputs=container.querySelectorAll<HTMLInputElement>(".pair-compare-panel input");
      for (const [index,value] of ["1012753","1011663"].entries()) {
        await act(async () => {
          Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(inputs[index],value);
          inputs[index]!.dispatchEvent(new Event("input",{bubbles:true}));
        });
      }
      await act(async () => { container.querySelector(".pair-compare-panel form")!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true})); await settle(); });
      expect(scan).toHaveBeenCalledExactlyOnceWith([galleryId(1012753),galleryId(1011663)]);
      expect(container.querySelector(".pair-compare-panel")).toHaveTextContent("자동 제외하지 않습니다");
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(inputs[1],"1012753");
        inputs[1]!.dispatchEvent(new Event("input",{bubbles:true}));
      });
      await act(async () => container.querySelector(".pair-compare-panel form")!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true})));
      expect(scan).toHaveBeenCalledTimes(1);
    } finally { await act(async () => root.unmount()); container.remove(); }
  });

  it("recovers a failed snapshot, scans and cancels explicitly, then reviews real evidence with CAS reload", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await backend.explorationExclusionsRestore([galleryId(4051038), galleryId(4050754)]);
    const seededDownloads = await backend.downloadQueueAdd(
      [galleryId(4051038), galleryId(4050754)],
      "app-duplicate-review-downloads",
    );
    if (!seededDownloads.ok) throw new Error(seededDownloads.error.message);
    const removedCandidateEntry = seededDownloads.data.find((entry) => entry.galleryId === galleryId(4050754));
    if (!removedCandidateEntry) throw new Error("duplicate candidate download fixture missing");
    const browserState = backend as unknown as { downloadEntries: Map<string, DownloadEntry> };
    for (const entry of seededDownloads.data) browserState.downloadEntries.set(entry.entryId, { ...entry, state: "completed" });
    const snapshot = vi.spyOn(backend, "duplicateSnapshot").mockResolvedValueOnce({
      ok: false,
      error: {
        code: "BACKEND_UNAVAILABLE",
        message: "initial duplicate snapshot unavailable",
        retryable: true,
        action: "retry",
      },
    });
    const scanStart = vi.spyOn(backend, "duplicateScanStart");
    const scanCancel = vi.spyOn(backend, "duplicateScanCancel");
    const decision = vi.spyOn(backend, "downloadOverlapDecisionApply");
    const quarantine = vi.spyOn(backend, "downloadQuarantine");
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(<TestApp />);
      await settle();
    });
    await act(async () => {
      clickButtonContaining(container, "Downloads");
      await settle();
    });
    expect(container.textContent).toContain("initial duplicate snapshot unavailable");

    await act(async () => {
      clickButtonContaining(container, "같은 작가 작품 중복 검사");
      await settle(15);
    });
    expect(scanStart).toHaveBeenCalledTimes(1);
    expect(snapshot).toHaveBeenCalledTimes(2);
    expect(container.textContent).toContain("중복 검사 중");
    await act(async () => {
      clickButtonContaining(container, "중복 검사 취소");
      await settle();
    });
    expect(scanCancel).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("중복 검사 취소됨");

    await act(async () => {
      clickButtonContaining(container, "같은 작가 작품 중복 검사");
      await settle(130);
    });
    expect(container.textContent).toContain("중복 검사 완료");
    const warning = container.querySelector<HTMLButtonElement>(
      '[data-gallery-id="4051038"] .status-pill.has-duplicate-count',
    );
    expect(warning).toHaveTextContent("1");
    expect(warning).toHaveAccessibleName(expect.stringContaining("중복 후보 1개"));

    await act(async () => {
      warning?.focus();
      warning?.click();
      await settle();
    });
    expect(container.querySelector(".review-dialog")).toHaveAttribute("open");
    expect(container.querySelector(".review-dialog")).toHaveTextContent("DOWNLOAD OVERLAP REVIEW");
    expect(container.textContent).toContain("브라우저 검토 fixture");
    expect(container.textContent).toContain("판본 페이지 정렬");
    expect(container.textContent).not.toContain("82%");
    expect(container.textContent).not.toContain("first gid");

    await act(async () => {
      container.querySelector<HTMLButtonElement>('.review-dialog button[aria-label="닫기"]')?.click();
      await settle();
    });
    expect(document.activeElement).toBe(warning);
    await act(async () => {
      warning?.click();
      await settle();
    });

    decision.mockResolvedValueOnce({
      ok: false,
      error: {
        code: "REVISION_CONFLICT",
        message: "stale",
        retryable: false,
        action: "review",
        details: { resource: "duplicateCandidate", expectedRevision: 0, actualRevision: 1 },
      },
    });
    await act(async () => {
      clickButtonContaining(container, "B 제외");
      await settle();
    });
    expect(container.textContent).toContain("다른 창에서 판정이 변경되어 최신 근거를 다시 불러왔습니다.");

    await act(async () => {
      clickButtonContaining(container, "B 제외");
      await settle();
    });
    expect(container.querySelector(".review-dialog[open]")).toBeNull();
    expect(container.textContent).toContain("파일은 영구 삭제하지 않습니다.");
    expect(quarantine).not.toHaveBeenCalled();

    await act(async () => {
      container.querySelector<HTMLButtonElement>('.review-dialog button[aria-label="닫기"]')?.click();
      await settle();
      container.querySelector<HTMLButtonElement>('button[aria-label="활동 기록"]')?.click();
      await settle();
    });
    const processedActivity = [...container.querySelectorAll<HTMLElement>("#activity-panel .activity-item")]
      .find((item) => item.textContent?.includes("The Last Tram"));
    expect(processedActivity).toHaveTextContent("중복 처리 완료 · 목록에서 제외");
    expect(processedActivity).toHaveTextContent("처리 완료");
    expect(processedActivity).not.toHaveTextContent("재시도");

    await act(async () => root.unmount());
    container.remove();
  });

  it("uses the same aggregate snapshot dialog for tray exit and rejects a stale work set", async () => {
    const current: AppActiveWorkSnapshot = {
      queriedAt: "2026-08-23T00:00:00.000Z",
      workSetFingerprint: "downloads-one",
      downloads: { activeCount: 1 },
    };
    const changed: AppActiveWorkSnapshot = {
      queriedAt: "2026-08-23T00:00:01.000Z",
      workSetFingerprint: "downloads-and-auto-find",
      downloads: { activeCount: 1 },
      autoFind: {
        runId: "auto-new",
        completedFavorites: 1,
        totalFavorites: 3,
        candidatesFound: 4,
      },
    };
    const snapshot = vi.spyOn(backend, "appActiveWorkSnapshot").mockResolvedValue({ ok: true, data: current });
    const quit = vi.spyOn(backend, "appQuit").mockResolvedValue({
      ok: true,
      data: { accepted: false, reason: "active_work_changed", snapshot: changed },
    });
    const mockBackend = backend as unknown as {
      emit(event: "app:exit-requested", payload: { source: "window_close" | "tray_menu" }): void;
    };
    const previousShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
    const previousClose = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "close");
    Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value() { this.setAttribute("open", ""); } });
    Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, value() { this.removeAttribute("open"); } });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      const snapshotCallsBeforeExitRequest = snapshot.mock.calls.length;
      await act(async () => {
        mockBackend.emit("app:exit-requested", { source: "tray_menu" });
        await settle();
      });
      expect(snapshot).toHaveBeenCalledTimes(snapshotCallsBeforeExitRequest + 1);
      expect(container.querySelector(".exit-dialog")).toHaveAttribute("open");
      expect(container).toHaveTextContent("다운로드 1개");

      const quitButton = container.querySelector<HTMLButtonElement>(".exit-dialog .quit-choice");
      await act(async () => {
        quitButton?.click();
        quitButton?.click();
        await settle();
      });
      expect(quit).toHaveBeenCalledOnce();
      expect(quit).toHaveBeenCalledWith({
        expectedWorkSetFingerprint: current.workSetFingerprint,
        confirmActiveWork: true,
      });
      expect(container).toHaveTextContent("Auto Find · 작가 1/3 · 후보 4개");
      expect(container).toHaveTextContent("진행 작업이 변경되었습니다. 내용을 확인하고 다시 선택해 주세요.");
      expect(container.querySelector(".exit-dialog")).toHaveAttribute("open");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      if (previousShowModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", previousShowModal);
      else delete (HTMLDialogElement.prototype as unknown as { showModal?: unknown }).showModal;
      if (previousClose) Object.defineProperty(HTMLDialogElement.prototype, "close", previousClose);
      else delete (HTMLDialogElement.prototype as unknown as { close?: unknown }).close;
    }
  });

  it("never quits automatically when status checks fail and arms force only after an explicit retry", async () => {
    const snapshot = vi.spyOn(backend, "appActiveWorkSnapshot").mockRejectedValue(new Error("status unavailable"));
    const quit = vi.spyOn(backend, "appQuit").mockResolvedValue({ ok: true, data: { accepted: true } });
    const mockBackend = backend as unknown as {
      emit(event: "app:exit-requested", payload: { source: "window_close" | "tray_menu" }): void;
    };
    const previousShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
    Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value() { this.setAttribute("open", ""); } });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await act(async () => {
        mockBackend.emit("app:exit-requested", { source: "window_close" });
        await settle();
      });
      expect(container.querySelector<HTMLButtonElement>(".exit-dialog .quit-choice")).toHaveTextContent("다시 확인");
      expect(quit).not.toHaveBeenCalled();

      await act(async () => {
        container.querySelector<HTMLButtonElement>(".exit-dialog .quit-choice")?.click();
        await settle();
      });
      expect(snapshot).toHaveBeenCalledTimes(2);
      expect(quit).not.toHaveBeenCalled();
      expect(container.querySelector<HTMLButtonElement>(".exit-dialog .quit-choice")).toHaveTextContent("상태 확인 없이 종료");

      await act(async () => {
        container.querySelector<HTMLButtonElement>(".exit-dialog .quit-choice")?.click();
        await settle();
      });
      expect(quit).toHaveBeenCalledOnce();
      expect(quit).toHaveBeenCalledWith({
        expectedWorkSetFingerprint: "",
        confirmActiveWork: true,
        forceWhenStatusUnknown: true,
      });
    } finally {
      await act(async () => root.unmount());
      container.remove();
      if (previousShowModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", previousShowModal);
      else delete (HTMLDialogElement.prototype as unknown as { showModal?: unknown }).showModal;
    }
  });

  it("drops a stale snapshot when appQuit cannot recheck active work and returns to explicit retry", async () => {
    const current: AppActiveWorkSnapshot = {
      queriedAt: "2026-08-23T00:00:00.000Z",
      workSetFingerprint: "known-before-quit",
      downloads: { activeCount: 1 },
    };
    const snapshot = vi.spyOn(backend, "appActiveWorkSnapshot").mockResolvedValue({ ok: true, data: current });
    const quit = vi.spyOn(backend, "appQuit").mockResolvedValue({
      ok: false,
      error: {
        code: "APP_ACTIVE_WORK_STATUS_UNAVAILABLE",
        message: "작업 상태를 다시 확인할 수 없습니다.",
        retryable: true,
        action: "retry",
      },
    });
    const mockBackend = backend as unknown as {
      emit(event: "app:exit-requested", payload: { source: "window_close" | "tray_menu" }): void;
    };
    const previousShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
    Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value() { this.setAttribute("open", ""); } });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle();
      });
      await act(async () => {
        mockBackend.emit("app:exit-requested", { source: "window_close" });
        await settle();
      });
      expect(container).toHaveTextContent("다운로드 1개");

      await act(async () => {
        container.querySelector<HTMLButtonElement>(".exit-dialog .quit-choice")?.click();
        await settle();
      });
      expect(snapshot).toHaveBeenCalledOnce();
      expect(quit).toHaveBeenCalledOnce();
      expect(container).toHaveTextContent("작업 상태를 확인할 수 없습니다.");
      expect(container).not.toHaveTextContent("다운로드 1개");
      expect(container.querySelector<HTMLButtonElement>(".exit-dialog .quit-choice")).toHaveTextContent("다시 확인");
      expect(container.querySelector<HTMLButtonElement>(".exit-dialog .quit-choice")).not.toHaveTextContent("상태 확인 없이 종료");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      if (previousShowModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", previousShowModal);
      else delete (HTMLDialogElement.prototype as unknown as { showModal?: unknown }).showModal;
    }
  });

  it("limits the Downloads all view to the configured Hitomi page size", async () => {
    const originalSettings = await backend.settingsGet();
    if (!originalSettings.ok) throw new Error(originalSettings.error.message);
    const configured = await backend.settingsUpdate({
      explorePageSize: 10,
      downloadsGrouping: "all",
    }, originalSettings.data.revision);
    if (!configured.ok) throw new Error(configured.error.message);

    const items: DownloadLibraryPage["items"] = Array.from({ length: 12 }, (_, index) => {
      const id = galleryId(6_100_000 + index);
      return {
        gallery: {
          id,
          title: `Downloaded page item ${index + 1}`,
          artist: `Downloaded artist ${Math.floor(index / 3) + 1}`,
          pages: 20 + index,
          language: "korean" as const,
          publishedRank: 20260901 - index,
        },
        download: {
          entryId: `download-pagination-${index + 1}`,
          galleryId: id,
          revision: 1,
          state: "completed" as const,
          progress: 100,
          createdAt: `2026-09-${String(12 - index).padStart(2, "0")}T00:00:00Z`,
          updatedAt: `2026-09-${String(12 - index).padStart(2, "0")}T00:00:00Z`,
        },
      };
    });
    vi.spyOn(backend, "downloadLibraryPageList").mockImplementation(async ({ page }) => ({
      ok: true,
      data: { page, totalItems: items.length, items: page === 1 ? items : [] },
    }));

    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle(80);
      });
      await act(async () => {
        clickButtonContaining(container, "Downloads");
        await settle(40);
      });

      expect(container.querySelectorAll(".gallery-card")).toHaveLength(10);
      expect(container.querySelector(".downloads-pager")).toHaveTextContent("1 / 2");
      expect(container).toHaveTextContent("Downloaded page item 1");
      expect(container).not.toHaveTextContent("Downloaded page item 12");

      const viewport = container.querySelector<HTMLElement>(".gallery-viewport");
      if (!viewport) throw new Error("gallery viewport missing");
      viewport.scrollTop = 420;
      await act(async () => {
        [...container.querySelectorAll<HTMLButtonElement>(".downloads-pager button")]
          .find((button) => button.textContent === "다음")
          ?.click();
        await settle();
      });

      expect(viewport.scrollTop).toBe(0);
      expect(container.querySelectorAll(".gallery-card")).toHaveLength(2);
      expect(container.querySelector(".downloads-pager")).toHaveTextContent("2 / 2");
      expect(container).toHaveTextContent("Downloaded page item 12");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      const latest = await backend.settingsGet();
      if (latest.ok) {
        await backend.settingsUpdate({
          explorePageSize: originalSettings.data.explorePageSize,
          downloadsGrouping: originalSettings.data.downloadsGrouping,
        }, latest.data.revision);
      }
    }
  });

  it("paginates Auto Find locally while retaining selection and resetting page focus and scroll", async () => {
    const originalSettings = await backend.settingsGet();
    if (!originalSettings.ok) throw new Error(originalSettings.error.message);
    const configured = await backend.settingsUpdate({ explorePageSize: 10 }, originalSettings.data.revision);
    if (!configured.ok) throw new Error(configured.error.message);

    const base = mockGalleries[4]!;
    const candidates = Array.from({ length: 12 }, (_, index) => ({
      id: galleryId(5_100_000 + index),
      title: `Auto Find page candidate ${index + 1}`,
      artist: base.artist,
      ...(base.group ? { group: base.group } : {}),
      pages: base.pages,
      language: base.language,
      tags: [...base.tags],
      series: [...base.series],
      characters: [...base.characters],
      publishedRank: 20260830 - index,
      popularity: base.score,
      thumbnailWidth: 512,
      thumbnailHeight: 768,
      runId: "auto-find-pagination-run",
      matchedFavorite: { namespace: "artist" as const, value: base.artist },
      discoveredAt: `2026-08-${String(30 - index).padStart(2, "0")}T00:00:00Z`,
    }));
    vi.spyOn(backend, "favoritesList").mockResolvedValue({
      ok: true,
      data: [{ namespace: "artist", value: base.artist, revision: 1, createdAt: "2026-08-01T00:00:00Z", updatedAt: "2026-08-01T00:00:00Z" }],
    });
    vi.spyOn(backend, "autoFindSnapshot").mockResolvedValue({
      ok: true,
      data: { candidates, cutoffEvidence: [], truncations: [] },
    });
    vi.spyOn(backend, "downloadEntriesList").mockResolvedValue({
      ok: true,
      data: { page: 1, totalItems: 0, entries: [] },
    });
    vi.spyOn(backend, "explorationExclusionsList").mockResolvedValue({ ok: true, data: [] });

    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle(80);
      });
      await act(async () => {
        clickButtonContaining(container, "Auto Find");
        await settle(40);
      });
      expect(container.querySelectorAll(".gallery-card")).toHaveLength(10);
      expect(container.querySelector(".auto-find-pager")).toHaveTextContent("1 / 2");

      const firstPageCards = container.querySelectorAll<HTMLElement>(".gallery-card");
      await act(async () => {
        firstPageCards[0]?.focus();
        firstPageCards[0]?.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
        firstPageCards[1]?.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1, ctrlKey: true }));
        await settle();
      });
      expect(container.querySelector(".selection-toolbar")).toHaveTextContent("2개 선택됨");
      const viewport = container.querySelector<HTMLElement>(".gallery-viewport");
      if (!viewport) throw new Error("gallery viewport missing");
      viewport.scrollTop = 420;

      await act(async () => {
        [...container.querySelectorAll<HTMLButtonElement>(".auto-find-pager button")]
          .find((button) => button.textContent === "다음")
          ?.click();
        await settle();
      });
      expect(viewport.scrollTop).toBe(0);
      expect(container.querySelectorAll(".gallery-card")).toHaveLength(2);
      expect(container.querySelector(".auto-find-pager")).toHaveTextContent("2 / 2");
      expect(container.querySelector(".selection-toolbar")).toHaveTextContent("2개 선택됨");
      expect(container.querySelector<HTMLElement>(".gallery-card")?.tabIndex).toBe(0);

      const searchInput = container.querySelector<HTMLInputElement>('input[aria-label="검색"]');
      const searchButton = container.querySelector<HTMLButtonElement>('button[type="submit"][aria-label="검색"]');
      if (!searchInput || !searchButton) throw new Error("Auto Find search controls were not rendered");
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(searchInput, "candidate");
        searchInput.dispatchEvent(new Event("input", { bubbles: true }));
        searchButton.click();
        await settle();
      });
      expect(container.querySelector(".auto-find-pager")).toHaveTextContent("1 / 2");
      expect(container.querySelectorAll(".gallery-card")).toHaveLength(10);
      expect(container.querySelectorAll(".gallery-card.is-selected")).toHaveLength(0);
    } finally {
      await act(async () => root.unmount());
      container.remove();
      const latest = await backend.settingsGet();
      if (latest.ok) {
        await backend.settingsUpdate({ explorePageSize: originalSettings.data.explorePageSize }, latest.data.revision);
      }
    }
  });

  it("switches sources from the Atsumi banner and restores the Hitomi workspace", async () => {
    window.localStorage.removeItem("atsumi.content-source.v1");
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle(50);
      });
      const openSourceMenu = async () => {
        const banner = container.querySelector<HTMLButtonElement>('.brand[aria-haspopup="menu"]');
        if (!banner) throw new Error("source banner missing");
        await act(async () => banner.click());
      };
      await openSourceMenu();
      const danbooru = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')]
        .find((button) => button.textContent?.includes("Danbooru"));
      await act(async () => {
        danbooru?.click();
        await settle(40);
      });
      expect(container).toHaveTextContent("Danbooru post 탐색");
      expect(window.localStorage.getItem("atsumi.content-source.v1")).toBe("danbooru");

      await openSourceMenu();
      const hitomi = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')]
        .find((button) => button.textContent?.includes("Hitomi"));
      await act(async () => {
        hitomi?.click();
        await settle(20);
      });
      expect(container).toHaveTextContent("갤러리 탐색");
      expect(window.localStorage.getItem("atsumi.content-source.v1")).toBe("hitomi");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("retains loaded Hitomi results and one set of app/work subscriptions during mode round trips", async () => {
    window.localStorage.removeItem("atsumi.content-source.v1");
    const page = explorePage(1, 1);
    const search = vi.spyOn(backend, "searchSubmit").mockResolvedValue({
      ok: true, data: { queryId: "retained-workspace", firstPage: page },
    });
    const pageGet = vi.spyOn(backend, "searchPageGet");
    const subscribe = vi.spyOn(backend, "on");
    const cancel = vi.spyOn(backend, "downloadCancel");
    const settingsGet = vi.spyOn(backend, "settingsGet");
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const switchMode = async (name: string) => {
      await act(async () => container.querySelector<HTMLButtonElement>('.brand[aria-haspopup="menu"]')!.click());
      const button = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')]
        .find((item) => item.querySelector("strong")?.textContent === name)!;
      await act(async () => { button.click(); await settle(30); });
    };
    try {
      await act(async () => { root.render(<TestApp />); await settle(50); });
      await submitExploreSearch(container);
      expect(container.querySelector(`[data-gallery-id="${page.items[0]!.id}"]`)).not.toBeNull();
      const downloadSubscription = subscribe.mock.calls.find(([event]) => event === "download:changed")!;
      const publishDownload = downloadSubscription[1] as (event: BackendEventMap["download:changed"]) => void;
      const countSubscriptions = (event: keyof BackendEventMap) => subscribe.mock.calls.filter(([name]) => name === event).length;
      const initialSettingsReads = settingsGet.mock.calls.length;
      const initialCounts = ["settings:changed", "app:exit-requested", "download:changed"].map((name) => countSubscriptions(name as keyof BackendEventMap));
      expect(initialCounts).toEqual([1, 1, 1]);

      await switchMode("Danbooru");
      expect(container.querySelector(".gallery-viewport")).toBeNull();
      await act(async () => {
        publishDownload({ entryId: "background-entry", galleryId: Number(page.items[0]!.id), revision: 100, state: "failed", errorMessage: "background fixture" });
        await settle();
      });
      await switchMode("Hitomi");
      const retained = container.querySelector(`[data-gallery-id="${page.items[0]!.id}"]`);
      expect(retained).toHaveTextContent("Explore page 1");
      expect(retained).toHaveTextContent("실패");
      await switchMode("Danbooru");
      await switchMode("Hitomi");

      expect(search).toHaveBeenCalledTimes(1);
      expect(pageGet).not.toHaveBeenCalled();
      expect(settingsGet).toHaveBeenCalledTimes(initialSettingsReads);
      expect(["settings:changed", "app:exit-requested", "download:changed"].map((name) => countSubscriptions(name as keyof BackendEventMap))).toEqual(initialCounts);
      expect(cancel).not.toHaveBeenCalled();
    } finally {
      await act(async () => root.unmount());
      container.remove();
      window.localStorage.removeItem("atsumi.content-source.v1");
    }
  });

  it("measures detailed Hitomi columns when its workspace first mounts after a source switch", async () => {
    const originalContentSource = window.localStorage.getItem("atsumi.content-source.v1");
    window.localStorage.setItem("atsumi.content-source.v1", "danbooru");
    const originalClientWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get() {
        return this instanceof HTMLElement && this.classList.contains("gallery-viewport") ? 1_700 : 0;
      },
    });
    const currentSettings = await backend.settingsGet();
    if (!currentSettings.ok) throw new Error(currentSettings.error.message);
    const configuredSettings = await backend.settingsUpdate({ maxColumns: 3, previewWidth: 220 }, currentSettings.data.revision);
    if (!configuredSettings.ok) throw new Error(configuredSettings.error.message);

    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(<TestApp />);
        await settle(50);
      });
      expect(container).toHaveTextContent("Danbooru post 탐색");

      const banner = container.querySelector<HTMLButtonElement>('.brand[aria-haspopup="menu"]');
      if (!banner) throw new Error("source banner missing");
      await act(async () => banner.click());
      const hitomi = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')]
        .find((button) => button.textContent?.includes("Hitomi"));
      await act(async () => {
        hitomi?.click();
        await settle(20);
      });
      await submitExploreSearch(container);

      const grid = container.querySelector<HTMLElement>('.gallery-grid[data-display-mode="detail"]');
      expect(grid?.style.gridTemplateColumns).toBe("repeat(3, minmax(0, 1fr))");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      if (originalClientWidth) Object.defineProperty(HTMLElement.prototype, "clientWidth", originalClientWidth);
      else delete (HTMLElement.prototype as unknown as { clientWidth?: number }).clientWidth;
      if (originalContentSource === null) window.localStorage.removeItem("atsumi.content-source.v1");
      else window.localStorage.setItem("atsumi.content-source.v1", originalContentSource);
      const latestSettings = await backend.settingsGet();
      if (latestSettings.ok) {
        await backend.settingsUpdate({
          maxColumns: currentSettings.data.maxColumns,
          previewWidth: currentSettings.data.previewWidth,
        }, latestSettings.data.revision);
      }
    }
  });
});
