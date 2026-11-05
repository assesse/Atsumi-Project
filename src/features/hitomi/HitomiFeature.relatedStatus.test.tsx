import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { backend } from "../../api/backend";
import { AppShell } from "../../app/AppShell";
import { HitomiFeature } from "./HitomiFeature";
import { ThumbnailProvider, ThumbnailClient } from "../../thumbnail";
import { mockGalleries } from "../../data/mockGalleries";
import type { GallerySummary } from "../../api/contracts";

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); localStorage.clear(); sessionStorage.clear(); });
it("passes locally loaded exclusion and duplicate decisions into related cards without extra requests", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} });
  localStorage.setItem("atsumi.tutorial.dismissed.v1", "true");
  const saved = await backend.settingsGet(); if (!saved.ok) throw Error("Missing fixture settings");
  vi.spyOn(backend, "settingsGet").mockResolvedValue({ ok: true, data: { ...saved.data, downloadOverlapAutoMode: "off", privacyMode: false, privacyOnStartup: false } });
  const ids = mockGalleries.slice(0,4).map(g => g.id);
  const summaries: GallerySummary[] = mockGalleries.slice(0,4).map(g => ({ id: g.id, title: g.title, artist: g.artist,
    pages: 1, language: g.language, tags: [], series: [], characters: [], publishedRank: 20261105, popularity: 0, thumbnailWidth: 200, thumbnailHeight: 300 }));
  vi.spyOn(backend, "searchSubmit").mockResolvedValue({ ok: true, data: { queryId: "related-status-fixture", firstPage: { page: 1, totalPages: 1, items: summaries } } });
  vi.spyOn(backend, "downloadEntriesList").mockImplementation(async req => ({ ok: true, data: { page: req.page, totalItems: 0, entries: [] } }));
  vi.spyOn(backend, "galleryDetailGet").mockResolvedValue({ ok: true, data: { ...summaries[0]!, related: summaries.slice(1), pageDimensions: [] } });
  const exclusions = vi.spyOn(backend, "explorationExclusionsList").mockResolvedValue({ ok: true, data: [
    { galleryId: ids[1]!, title: summaries[1]!.title, artist: "fixture", reasons: [{ kind: "duplicate_hidden", detail: "fixture", excludedAt: "" }] },
    { galleryId: ids[2]!, title: summaries[2]!.title, artist: "fixture", reasons: [{ kind: "manual", detail: "fixture", excludedAt: "" }] },
  ] });
  const originalSnapshot = await backend.duplicateSnapshot(); if (!originalSnapshot.ok) throw Error("Missing fixture snapshot");
  const ref = (i: number) => ({ galleryId: ids[i]!, entryId: "entry-" + ids[i], title: summaries[i]!.title, pageCount: 1 });
  const snapshot = vi.spyOn(backend, "duplicateSnapshot").mockResolvedValue({ ok: true, data: { ...originalSnapshot.data, candidates: [
    { candidateId: "fixture-pair", revision: 1, parent: ref(0), candidate: ref(3), relation: "partial", confidence: 0.9,
      matchedPages: 1, parentCoverage: 1, candidateCoverage: 1, createdAt: "", updatedAt: "" },
  ] } });
  const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
  const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  try {
    await act(async () => root.render(<ThumbnailProvider client={client}><AppShell api={backend}><HitomiFeature active>
      {bridge => bridge.workspace}</HitomiFeature></AppShell></ThumbnailProvider>));
    const flush = async () => act(async () => { await new Promise(resolve => setTimeout(resolve, 30)); });
    await flush();
    await act(async () => container.querySelector("form.search-box")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    for (let i=0; i<20 && !container.querySelector('.gallery-card[data-gallery-id="' + ids[0] + '"]'); i++) await flush();
    const card = container.querySelector('.gallery-card[data-gallery-id="' + ids[0] + '"]')!;
    expect(card, container.textContent?.slice(0,2200)).not.toBeNull();
    const before = [exclusions.mock.calls.length, snapshot.mock.calls.length];
    await act(async () => card.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, detail: 2 })));
    await flush();
    const related = (i: number) => container.querySelector('.related-card[data-gallery-id="' + ids[i] + '"]');
    expect(related(1)).toHaveAccessibleName(summaries[1]!.title + ", 중복 판정으로 제외");
    expect(related(2)).toHaveAccessibleName(summaries[2]!.title + ", 탐색에서 제외");
    expect(related(3)?.querySelector(".gallery-processing-badge")).toHaveAttribute("data-processing-state", "duplicate");
    expect([exclusions.mock.calls.length, snapshot.mock.calls.length]).toEqual(before);
  } finally { await act(async () => root.unmount()); client.dispose(); container.remove(); }
}, 20000);
