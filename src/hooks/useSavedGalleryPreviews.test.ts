import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BackendClient, BackendEventMap } from "../api/backend";
import type { ApiResult, GalleryPreviewSetRequest } from "../api/contracts";
import { galleryId, type ArtistPreview, type Gallery, type GalleryId, type GalleryPreview } from "../core/types";
import { mockGalleries } from "../data/mockGalleries";
import { useSavedGalleryPreviews } from "./useSavedGalleryPreviews";

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(() => vi.unstubAllGlobals());

const completedGallery: Gallery = {
  ...mockGalleries[0]!,
  artist: " Preview Artist ",
  download: { entryId: "saved-preview-entry", state: "completed", progress: 100 },
};

const savedPreview = (overrides: Partial<GalleryPreview> = {}): GalleryPreview => ({
  galleryId: completedGallery.id,
  mode: "automatic",
  sourcePage: 2,
  manualSourcePage: null,
  entryId: "saved-preview-entry",
  width: 800,
  height: 1200,
  candidates: [2, 4, 7],
  algorithmVersion: 1,
  updatedAt: "2026-09-07T00:00:00Z",
  ...overrides,
});

const galleryMap = (...galleries: Gallery[]): ReadonlyMap<GalleryId, Gallery> =>
  new Map(galleries.map((gallery) => [gallery.id, gallery]));

function mockBackend() {
  const listeners = new Map<string, (payload: unknown) => void>();
  const on = vi.fn(async (event: string, handler: (payload: unknown) => void) => {
    listeners.set(event, handler);
    return () => { listeners.delete(event); };
  });
  const galleryPreviewList = vi.fn(async (_ids: GalleryId[]): Promise<ApiResult<GalleryPreview[]>> => ({ ok: true, data: [] }));
  const artistPreviewList = vi.fn(async (_artists: string[]): Promise<ApiResult<ArtistPreview[]>> => ({ ok: true, data: [] }));
  const galleryPreviewSet = vi.fn(async (_request: GalleryPreviewSetRequest): Promise<ApiResult<GalleryPreview>> => ({ ok: true, data: savedPreview() }));
  return {
    backend: { on, galleryPreviewList, artistPreviewList, galleryPreviewSet } as unknown as BackendClient,
    galleryPreviewList,
    artistPreviewList,
    galleryPreviewSet,
    on,
    emit<K extends "gallery-preview:updated" | "artist-preview:updated">(event: K, payload: BackendEventMap[K]) {
      const listener = listeners.get(event);
      if (!listener) throw new Error(`Missing test listener for ${event}`);
      listener(payload);
    },
  };
}

function mountHook(backend: BackendClient) {
  let latest: ReturnType<typeof useSavedGalleryPreviews> | undefined;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  function Harness({ galleries }: { galleries: ReadonlyMap<GalleryId, Gallery> }) {
    latest = useSavedGalleryPreviews(backend, galleries);
    return null;
  }
  return {
    get current() {
      if (!latest) throw new Error("Hook has not rendered");
      return latest;
    },
    async render(galleries: ReadonlyMap<GalleryId, Gallery>) {
      await act(async () => root.render(createElement(Harness, { galleries })));
    },
    async unmount() {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

describe("useSavedGalleryPreviews", () => {
  it("reads saved selections once without refetching on hover or detail metadata rerenders", async () => {
    const backend = mockBackend();
    const preview = savedPreview();
    backend.galleryPreviewList.mockResolvedValue({ ok: true, data: [preview] });
    backend.artistPreviewList.mockResolvedValue({
      ok: true,
      data: [{ artist: "PREVIEW ARTIST", galleryIds: [completedGallery.id], updatedAt: preview.updatedAt }],
    });
    const incomplete: Gallery = {
      ...mockGalleries[1]!,
      download: { entryId: "still-downloading", state: "downloading", progress: 50 },
    };
    const fixture = mountHook(backend.backend);
    try {
      await fixture.render(galleryMap(completedGallery, incomplete));
      expect(backend.galleryPreviewList).toHaveBeenCalledExactlyOnceWith([completedGallery.id]);
      expect(backend.artistPreviewList).toHaveBeenCalledExactlyOnceWith(["preview artist"]);
      expect(fixture.current.previews.get(completedGallery.id)).toEqual(preview);
      expect(fixture.current.artistGalleryIds.get("preview artist")).toEqual([completedGallery.id]);

      await fixture.render(galleryMap(incomplete, {
        ...completedGallery,
        tags: ["full_color", "female:glasses"],
        pageDimensions: [{ sourcePage: 1, width: 720, height: 1080 }],
      }));
      await fixture.render(galleryMap({
        ...completedGallery,
        title: "Hydrated detail title",
        favorite: !completedGallery.favorite,
        representativePreview: preview,
      }, { ...incomplete, download: { ...incomplete.download!, progress: 70 } }));
      expect(backend.galleryPreviewList).toHaveBeenCalledTimes(1);
      expect(backend.artistPreviewList).toHaveBeenCalledTimes(1);
      expect(backend.galleryPreviewSet).not.toHaveBeenCalled();
      expect(backend.on).toHaveBeenCalledTimes(2);
    } finally {
      await fixture.unmount();
    }
  });

  it("applies gallery and artist selections emitted when a download completes", async () => {
    const backend = mockBackend();
    const fixture = mountHook(backend.backend);
    const preview = savedPreview({ sourcePage: 4, updatedAt: "2026-09-07T00:01:00Z" });
    try {
      await fixture.render(galleryMap({
        ...completedGallery,
        download: { ...completedGallery.download!, state: "downloading", progress: 99 },
      }));
      expect(backend.galleryPreviewList).not.toHaveBeenCalled();
      expect(backend.artistPreviewList).not.toHaveBeenCalled();
      await act(async () => {
        backend.emit("gallery-preview:updated", preview);
        backend.emit("artist-preview:updated", {
          artist: " Preview Artist ",
          galleryIds: [completedGallery.id],
          updatedAt: preview.updatedAt,
        });
      });
      expect(fixture.current.previews.get(completedGallery.id)).toEqual(preview);
      expect(fixture.current.artistGalleryIds.get("preview artist")).toEqual([completedGallery.id]);

      await fixture.render(galleryMap(completedGallery));
      expect(backend.galleryPreviewList).toHaveBeenCalledExactlyOnceWith([completedGallery.id]);
      expect(fixture.current.previews.get(completedGallery.id)).toEqual(preview);
      expect(fixture.current.artistGalleryIds.get("preview artist")).toEqual([completedGallery.id]);
      expect(backend.galleryPreviewSet).not.toHaveBeenCalled();
    } finally {
      await fixture.unmount();
    }
  });

  it("does not duplicate an in-flight gallery read when artist metadata or completed IDs are hydrated", async () => {
    const backend = mockBackend();
    let resolveInitial: ((result: ApiResult<GalleryPreview[]>) => void) | undefined;
    backend.galleryPreviewList.mockImplementationOnce(() => new Promise((resolve) => { resolveInitial = resolve; }));
    const additional: Gallery = {
      ...completedGallery,
      id: galleryId(808),
      download: { entryId: "additional-preview-entry", state: "completed", progress: 100 },
    };
    const fixture = mountHook(backend.backend);
    try {
      await fixture.render(galleryMap({ ...completedGallery, artist: "" }));
      expect(backend.galleryPreviewList).toHaveBeenCalledExactlyOnceWith([completedGallery.id]);
      expect(backend.artistPreviewList).not.toHaveBeenCalled();

      await fixture.render(galleryMap(completedGallery));
      expect(backend.galleryPreviewList).toHaveBeenCalledTimes(1);
      expect(backend.artistPreviewList).toHaveBeenCalledExactlyOnceWith(["preview artist"]);

      await fixture.render(galleryMap(additional, completedGallery));
      expect(backend.galleryPreviewList.mock.calls).toEqual([
        [[completedGallery.id]],
        [[additional.id]],
      ]);
      expect(backend.artistPreviewList).toHaveBeenCalledTimes(1);
      await act(async () => resolveInitial?.({ ok: true, data: [savedPreview()] }));
      expect(fixture.current.previews.get(completedGallery.id)).toEqual(savedPreview());
      expect(backend.galleryPreviewList).toHaveBeenCalledTimes(2);
    } finally {
      await fixture.unmount();
    }
  });

  it("applies manual saves and automatic resets directly from successful command results", async () => {
    const backend = mockBackend();
    const automatic = savedPreview();
    const manual = savedPreview({ mode: "manual", sourcePage: 7, manualSourcePage: 7, updatedAt: "2026-09-07T00:02:00Z" });
    backend.galleryPreviewList.mockResolvedValue({ ok: true, data: [automatic] });
    backend.galleryPreviewSet
      .mockResolvedValueOnce({ ok: true, data: manual })
      .mockResolvedValueOnce({ ok: true, data: automatic });
    const fixture = mountHook(backend.backend);
    try {
      await fixture.render(galleryMap(completedGallery));
      await act(async () => fixture.current.save(completedGallery.id, 7));
      expect(backend.galleryPreviewSet).toHaveBeenLastCalledWith({ galleryId: completedGallery.id, sourcePage: 7 });
      expect(fixture.current.previews.get(completedGallery.id)).toEqual(manual);

      await act(async () => fixture.current.save(completedGallery.id, null));
      expect(backend.galleryPreviewSet).toHaveBeenLastCalledWith({ galleryId: completedGallery.id, sourcePage: null });
      expect(fixture.current.previews.get(completedGallery.id)).toEqual(automatic);
      expect(backend.galleryPreviewList).toHaveBeenCalledTimes(1);
      expect(backend.artistPreviewList).toHaveBeenCalledTimes(1);
    } finally {
      await fixture.unmount();
    }
  });

  it("keeps newer manual and artist events when an older initial list resolves afterward", async () => {
    const backend = mockBackend();
    let resolveInitial: ((result: ApiResult<GalleryPreview[]>) => void) | undefined;
    backend.galleryPreviewList.mockImplementationOnce(() => new Promise((resolve) => { resolveInitial = resolve; }));
    const initial = savedPreview();
    const manual = savedPreview({ mode: "manual", sourcePage: 7, manualSourcePage: 7, updatedAt: "2026-09-07T00:03:00Z" });
    const newerArtistIds = [completedGallery.id, galleryId(808)];
    backend.artistPreviewList.mockResolvedValue({
      ok: true,
      data: [{ artist: "preview artist", galleryIds: [completedGallery.id], updatedAt: initial.updatedAt }],
    });
    const fixture = mountHook(backend.backend);
    try {
      await fixture.render(galleryMap(completedGallery));
      expect(backend.galleryPreviewList).toHaveBeenCalledTimes(1);
      expect(fixture.current.previews.size).toBe(0);
      await act(async () => {
        backend.emit("gallery-preview:updated", manual);
        backend.emit("artist-preview:updated", {
          artist: "PREVIEW ARTIST", galleryIds: newerArtistIds, updatedAt: manual.updatedAt,
        });
      });
      expect(fixture.current.previews.get(completedGallery.id)).toEqual(manual);
      await fixture.render(galleryMap({ ...completedGallery, pageDimensions: [{ sourcePage: 7, width: 800, height: 1200 }] }));
      expect(backend.galleryPreviewList).toHaveBeenCalledTimes(1);

      await act(async () => resolveInitial?.({ ok: true, data: [initial] }));
      expect(fixture.current.previews.get(completedGallery.id)).toEqual(manual);
      expect(fixture.current.artistGalleryIds.get("preview artist")).toEqual(newerArtistIds);
      expect(backend.galleryPreviewList).toHaveBeenCalledTimes(1);
      expect(backend.artistPreviewList).toHaveBeenCalledTimes(1);
    } finally {
      await fixture.unmount();
    }
  });
});
