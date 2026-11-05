import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { Gallery } from "../core/types";
import { mockGalleries } from "../data/mockGalleries";
import { DownloadProgressContext, DownloadProgressStore } from "../state/downloadProgress";
import { downloadStatus } from "../state/downloadStatus";
import { ThumbnailClient } from "../thumbnail";
import { DetailWorkspace } from "./DetailWorkspace";
import { GalleryCard } from "./GalleryCard";

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); sessionStorage.clear(); });
const related: Gallery = { ...mockGalleries[1]!, pages: 1, relatedIds: [], download: undefined };
const parent: Gallery = { ...mockGalleries[0]!, pages: 1, relatedIds: [related.id], download: undefined };
const callbacks = { onSelect: vi.fn(), onOpenDetail: vi.fn(), onOpenArtifact: vi.fn(), onOpenReview: vi.fn(),
  onStatusDetail: vi.fn(), onMetadataSearch: vi.fn(), onMetadataFavorite: vi.fn() };

it.each(["duplicate-excluded", "manual-excluded", "quarantined", "candidate", "new"] as const)(
  "related cards share Explore presentation for %s and remain navigable", async (kind) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
  const excluded = kind.endsWith("excluded");
  const label = kind === "duplicate-excluded" ? "중복 판정으로 제외" : "탐색에서 제외";
  const gallery: Gallery = { ...related, download: kind === "quarantined"
    ? { entryId: "test", state: "quarantined" } : excluded ? { entryId: "test", state: "completed" } : undefined };
  const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container); const open = vi.fn();
  const galleries = new Map([[parent.id, parent], [gallery.id, gallery]]);
  try {
    await act(async () => root.render(<>
      <GalleryCard gallery={gallery} view="explore" displayMode="detail" favoriteMetadata={new Set()} selected={false} selectionContext={false}
        explorationExcluded={excluded} explorationExcludedLabel={label} duplicateCandidateCount={kind === "candidate" ? 2 : 0}
        thumbnailClient={client} {...callbacks} />
      <DetailWorkspace tabs={[parent.id]} activeId={parent.id} minimized={false} galleries={galleries}
        favoriteMetadata={new Set()} thumbnailClient={client}
        duplicateExcludedGalleryIds={new Set(kind === "duplicate-excluded" ? [gallery.id] : [])}
        explorationExcludedGalleryIds={new Set(excluded ? [gallery.id] : [])}
        duplicateCandidateCounts={new Map(kind === "candidate" ? [[gallery.id, 2]] : [])}
        onActivate={vi.fn()} onClose={vi.fn()} onCloseAll={vi.fn()} onMinimize={vi.fn()} onRestore={vi.fn()}
        onOpenRelated={open} onQueue={vi.fn()} onMetadataSearch={vi.fn()} onMetadataFavorite={vi.fn()} />
    </>));
    const card = container.querySelector<HTMLElement>(".related-card")!;
    const explore = container.querySelector(".gallery-card")!;
    expect(card.getAttribute("data-processing-tone")).toBe(explore.getAttribute("data-processing-tone"));
    expect(card.querySelector(".gallery-processing-badge")?.getAttribute("data-processing-state"))
      .toBe(explore.querySelector(".gallery-processing-badge")?.getAttribute("data-processing-state"));
    if (excluded) {
      expect(card).toHaveAccessibleName(gallery.title + ", " + label);
      expect(card).toHaveAttribute("data-processing-excluded", "true");
      expect(card).not.toHaveAttribute("data-processing-muted");
      expect(card.querySelector(".gallery-processing-badge")).toHaveAttribute("data-processing-state", "excluded");
    } else if (kind === "candidate") {
      expect(card).toHaveAccessibleName(gallery.title + ", 중복 후보 2개");
      expect(card.querySelectorAll(".gallery-processing-badge")).toHaveLength(1);
    } else if (kind === "new") {
      expect(card.querySelector(".gallery-processing-badge")).toBeNull();
    }
    await act(async () => card.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })));
    expect(open).toHaveBeenLastCalledWith(gallery.id, parent.id, { activate: false });
    await act(async () => card.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, detail: 2 })));
    expect(open).toHaveBeenLastCalledWith(gallery.id, parent.id);
    await act(async () => card.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" })));
    expect(open).toHaveBeenCalledTimes(3);
  } finally { await act(async () => root.unmount()); client.dispose(); container.remove(); }
});

it("restores an open related card and handles every live phase without remounting its thumbnail", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
  const gallery: Gallery = { ...related, download: { entryId: "entry", state: "completed", revision: 1 } };
  const store = new DownloadProgressStore();
  const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
  const subscribe = vi.spyOn(client, "subscribe");
  const container = document.createElement("div"); const root = createRoot(container);
  const render = (excluded: boolean) => root.render(<DownloadProgressContext.Provider value={store}>
    <DetailWorkspace tabs={[parent.id]} activeId={parent.id} minimized={false}
      galleries={new Map([[parent.id, parent], [gallery.id, gallery]])}
      favoriteMetadata={new Set()} thumbnailClient={client} explorationExcludedGalleryIds={new Set(excluded ? [gallery.id] : [])}
      onActivate={vi.fn()} onClose={vi.fn()} onCloseAll={vi.fn()} onMinimize={vi.fn()} onRestore={vi.fn()}
      onOpenRelated={vi.fn()} onQueue={vi.fn()} onMetadataSearch={vi.fn()} onMetadataFavorite={vi.fn()} />
    </DownloadProgressContext.Provider>);
  try {
    await act(async () => render(true));
    const card = container.querySelector(".related-card")!;
    const cover = card.querySelector(".related-cover");
    const subscriptions = subscribe.mock.calls.length;
    expect(card.querySelector("[data-processing-state=excluded]")).not.toBeNull();
    await act(async () => render(false));
    expect(card.querySelector("[data-processing-state=completed]")).not.toBeNull();
    expect(card).toHaveAttribute("data-processing-muted", "true");
    let revision = 1;
    for (const state of Object.keys(downloadStatus) as (keyof typeof downloadStatus)[]) {
      await act(async () => { store.apply({ galleryId: gallery.id, entryId: "entry", revision: ++revision, state }, gallery.download); });
      expect(card.querySelector(".gallery-processing-badge")).toHaveAccessibleName(gallery.title + ", " + downloadStatus[state].label);
      expect(card.querySelector(".related-cover")).toBe(cover);
    }
    await act(async () => { store.apply({ galleryId: gallery.id, entryId: "entry", revision: ++revision, state: "review_required", reviewKind: "gallery_duplicate" }, gallery.download); });
    expect(card.querySelector("[data-processing-state=duplicate]")).not.toBeNull();
    await act(async () => { store.apply({ galleryId: gallery.id, entryId: "entry", revision: 1, state: "completed" }, gallery.download); });
    expect(card.querySelector("[data-processing-state=duplicate]")).not.toBeNull();
    expect(subscribe.mock.calls.length).toBe(subscriptions);
  } finally { await act(async () => root.unmount()); store.clear(); client.dispose(); }
});
