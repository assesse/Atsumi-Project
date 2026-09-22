import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "../../App";
import { backend } from "../../api/backend";
import type { ApiResult, DownloadEntry, GallerySummary } from "../../api/contracts";
import { galleryId } from "../../core/types";
import { ThumbnailClient, ThumbnailProvider } from "../../thumbnail";
import { setTutorialDismissed } from "../../tutorial/tutorialPreference";

const settle = () => new Promise((resolve) => window.setTimeout(resolve, 30));

describe("album cancellation without session activity", () => {
  beforeEach(() => {
    window.localStorage.clear();
    setTutorialDismissed(true);
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(Date.now()), 0));
    vi.stubGlobal("cancelAnimationFrame", (id: number) => window.clearTimeout(id));
  });
  afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); window.localStorage.clear(); });

  const prepare = async () => {
    const items: GallerySummary[] = [1, 2, 3].map((index) => ({
      id: galleryId(9_100_000 + index), title: `Cancellation fixture ${index}`, artist: "fixture", pages: 1,
      language: "korean", tags: [], series: [], characters: [], publishedRank: 20260922, popularity: 0, thumbnailWidth: 512, thumbnailHeight: 768,
    }));
    let entries: DownloadEntry[] = items.map((gallery, index) => ({
      entryId: `cancellation-fixture-${index}`, galleryId: gallery.id, revision: 1,
      state: index === 0 ? "queued" : index === 1 ? "hashing" : "completed",
    }));
    vi.spyOn(backend, "downloadLibraryPageList").mockImplementation(async () => ({ ok: true,
      data: { page: 1, totalItems: items.length, items: items.map((gallery, index) => ({ gallery, download: entries[index]! })) },
    }));
    vi.spyOn(backend, "downloadEntriesList").mockImplementation(async () => ({ ok: true, data: { page: 1, totalItems: entries.length, entries } }));
    vi.spyOn(backend, "searchSubmit").mockResolvedValue({ ok: true, data: { queryId: "cancel-fixture", firstPage: { page: 1, totalPages: 1, items } } });
    vi.spyOn(backend, "galleryDetailGet").mockImplementation(async (id) => ({ ok: true,
      data: { ...items.find((item) => item.id === id)!, related: [], pageDimensions: [] },
    }));
    let finish: (() => void) | undefined;
    const cancel = vi.spyOn(backend, "downloadCancel").mockImplementation((ids) => new Promise<ApiResult<DownloadEntry[]>>((resolve) => {
      finish = () => {
        entries = entries.map((entry) => ids.includes(entry.entryId) ? { ...entry, revision: entry.revision + 1, state: "cancelled" } : entry);
        resolve({ ok: true, data: entries.filter((entry) => ids.includes(entry.entryId)) });
      };
    }));
    const quarantine = vi.spyOn(backend, "downloadQuarantine");
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => { root.render(<ThumbnailProvider client={client}><App /></ThumbnailProvider>); await settle(); });
    await act(async () => { await settle(); });
    return { container, items, cancel, quarantine, finish: () => { if (!finish) throw new Error("No pending cancellation"); finish(); },
      entries: () => entries,
      dispose: async () => { await act(async () => root.unmount()); client.dispose(); container.remove(); },
    };
  };

  const explore = async (container: HTMLElement) => {
    await act(async () => { container.querySelector<HTMLButtonElement>('button[type="submit"][aria-label="검색"]')!.click(); await settle(); });
  };

  it("cancels an old queued download from Floating Detail even when activity is empty", async () => {
    const fixture = await prepare();
    const { container, items, cancel } = fixture;
    try {
      await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="활동 기록"]')!.click());
      expect(container.querySelector("#activity-session-panel .activity-item")).toBeNull();
      await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="활동 기록 닫기"]')!.click());
      await explore(container);
      const card = container.querySelector<HTMLElement>(`[data-gallery-id="${items[0]!.id}"]`)!;
      await act(async () => { card.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true })); await settle(); });
      const button = container.querySelector<HTMLButtonElement>('.detail-title-actions [aria-label="다운로드 취소"]')!;
      expect(button).toBeEnabled();
      await act(async () => button.click());
      expect(cancel).toHaveBeenCalledWith(["cancellation-fixture-0"]);
      const busy = container.querySelector<HTMLButtonElement>('[aria-label="다운로드 취소 중"]')!;
      expect(busy).toBeDisabled();
      await act(async () => busy.click());
      expect(cancel).toHaveBeenCalledOnce();
      await act(async () => { fixture.finish(); await settle(); });
      expect(container.querySelector('.detail-title-actions [aria-label="다운로드"]')).toBeEnabled();
      expect(fixture.quarantine).not.toHaveBeenCalled();
    } finally { await fixture.dispose(); }
  });

  it.each(["Explore", "Downloads"])("cancels only active members of a mixed selection in %s", async (view) => {
    const fixture = await prepare();
    const { container, items, cancel } = fixture;
    try {
      if (view === "Explore") await explore(container);
      else await act(async () => { container.querySelector<HTMLButtonElement>('[aria-label="Downloads"]')!.click(); await settle(); });
      for (let index = 0; index < items.length; index++) {
        const card = container.querySelector<HTMLElement>(`[data-gallery-id="${items[index]!.id}"]`)!;
        expect(card).not.toBeNull();
        await act(async () => card.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1, ctrlKey: index > 0 })));
      }
      const button = [...container.querySelectorAll<HTMLButtonElement>(".selection-toolbar button")].find((item) => item.textContent?.includes("다운로드 취소"))!;
      expect(button).toHaveTextContent("다운로드 취소 · 2개");
      await act(async () => button.click());
      expect(cancel).toHaveBeenCalledWith(["cancellation-fixture-0", "cancellation-fixture-1"]);
      expect(button).toBeDisabled();
      await act(async () => button.click());
      expect(cancel).toHaveBeenCalledOnce();
      await act(async () => { fixture.finish(); await settle(); });
      expect(fixture.entries().map((entry) => entry.state)).toEqual(["cancelled", "cancelled", "completed"]);
      expect(container.querySelector(".selection-toolbar")).not.toHaveTextContent("다운로드 취소");
      expect(fixture.quarantine).not.toHaveBeenCalled();
    } finally { await fixture.dispose(); }
  });
});
