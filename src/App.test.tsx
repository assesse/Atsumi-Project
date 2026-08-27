import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { backend, type BackendEventMap } from "./api/backend";
import type {
  AppActiveWorkSnapshot,
  DownloadEntry,
  DownloadPage,
  GalleryPage,
  InternalArtifactScanProgress,
  InternalDuplicateReview,
  InternalDuplicateSnapshot,
  InternalScanRun,
} from "./api/contracts";
import { galleryId } from "./core/types";
import { mockGalleries } from "./data/mockGalleries";
import { browserFixtureThumbnailAdapter, ThumbnailClient, ThumbnailProvider } from "./thumbnail";

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

describe("App Phase 3A backend flow", () => {
  beforeEach(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    class TestResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    vi.stubGlobal("ResizeObserver", TestResizeObserver);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(Date.now()), 0));
    vi.stubGlobal("cancelAnimationFrame", (id: number) => window.clearTimeout(id));
  });

  afterEach(() => {
    testThumbnailClient.dispose();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
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
    const toggle = container.querySelector<HTMLButtonElement>('button[aria-label="개인정보 보호 모드"]');
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
    vi.spyOn(backend, "downloadEntriesList").mockImplementation(() => new Promise(() => undefined));
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
        refreshedFirst.dispatchEvent(new KeyboardEvent("keydown", { key: "?", code: "Slash", shiftKey: true, bubbles: true }));
        await settle();
      });
      const shortcuts = container.querySelector<HTMLDialogElement>(".keyboard-shortcuts-dialog");
      expect(shortcuts).toHaveAttribute("open");
      expect(shortcuts).toHaveTextContent("이전 화면");
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

  it("does not search while typing and faithfully replays a structured history request", async () => {
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
    expect(search).toHaveBeenLastCalledWith({ ...replayRequest, languages: ["korean", "english"] });

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
    expect(search.mock.calls).toHaveLength(callsBeforeMetadata + 2);

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
      expect(search.mock.calls).toHaveLength(callsBeforeRepeat + 2);
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
      expect(container).toHaveTextContent("전부 접기");
      await act(async () => {
        clickButtonContaining(container, "전부 접기");
        await settle();
      });
      expect(container.querySelectorAll(".gallery-groups[data-group-view='downloads'] .gallery-group-toggle[aria-expanded='true']")).toHaveLength(0);
      expect(container).toHaveTextContent("전부 펼치기");
      await act(async () => {
        clickButtonContaining(container, "전부 펼치기");
        await settle();
      });
      expect(container.querySelector(".gallery-groups[data-group-view='downloads'] .gallery-group-toggle[aria-expanded='true']")).not.toBeNull();
      await act(async () => {
        clickButtonContaining(container, "작가별");
        await settle();
      });
      expect(container.querySelector(".gallery-groups[data-group-view='downloads'] .gallery-group-toggle")).not.toBeNull();
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

  it("recovers a failed snapshot, scans and cancels explicitly, then reviews real evidence with CAS reload", async () => {
    await backend.downloadQueueAdd(
      [galleryId(4051038), galleryId(4050754)],
      "app-duplicate-review-downloads",
    );
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
    const decision = vi.spyOn(backend, "duplicateDecisionApply");
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
    expect(container.querySelector(".review-summary")).toHaveTextContent("신뢰도 94%");
    expect(container.textContent).toContain("브라우저 검토 fixture");
    expect(container.textContent).toContain("원본 페이지 번호를 보존한 순서 정렬");
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
      clickButtonContaining(container, "38p 포괄 작품 유지 · 24p 귀속 작품 숨기기");
      await settle();
    });
    expect(container.textContent).toContain("다른 창에서 판정이 변경되어 최신 근거와 이력을 다시 불러왔습니다.");

    await act(async () => {
      clickButtonContaining(container, "38p 포괄 작품 유지 · 24p 귀속 작품 숨기기");
      await settle();
    });
    expect(container.querySelector(".decision-history")).toHaveTextContent("귀속 작품 숨김");
    expect(container.textContent).toContain("자동으로 파일을 삭제하지 않으며");
    expect(quarantine).not.toHaveBeenCalled();

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
});
