import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { BackendClient } from "../api/backend";
import type { DetailOriginalPrepared, DetailOriginalPrepareRequest } from "../api/contracts";
import type { Gallery } from "../core/types";
import { mockGalleries } from "../data/mockGalleries";
import { ThumbnailClient } from "../thumbnail";
import { ProgressiveDetailHero } from "./ProgressiveDetailHero";

const gallery = { ...mockGalleries[0]!, pageDimensions: [{ sourcePage: 1, width: 720, height: 1080 }] };
const media = (requestId: string) => ({
  requestId, galleryId: gallery.id, sourcePage: 1 as const,
  mediaUrl: `http://detail-original.localhost/${requestId}`,
  contentType: "image/webp" as const, width: 720, height: 1080,
});

const renderHero = async (backend: Partial<BackendClient>) => {
  const client = new ThumbnailClient({ resolve: () => ({ kind: "image" as const, url: "blob:cover", width: 512, height: 512 }) });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(<ProgressiveDetailHero gallery={gallery} client={client} backend={backend as BackendClient} />);
    await Promise.resolve();
    await Promise.resolve();
  });
  return { client, container, root };
};

describe("ProgressiveDetailHero", () => {
  it("switches the original to the representative page and ignores a late page-one response", async () => {
    const completed: Gallery = {
      ...gallery,
      download: { entryId: "hero-representative", state: "completed", progress: 100 },
      pageDimensions: [
        { sourcePage: 1, width: 720, height: 1080 },
        { sourcePage: 3, width: 1600, height: 900 },
      ],
    };
    const selected: Gallery = {
      ...completed,
      representativePreview: {
        galleryId: gallery.id, mode: "manual", sourcePage: 3, manualSourcePage: 3,
        entryId: "hero-representative", width: 1600, height: 900,
        candidates: [1, 3], algorithmVersion: 1, updatedAt: "2026-09-07T00:00:00Z",
      },
    };
    const pending = new Map<string, (value: { ok: true; data: DetailOriginalPrepared }) => void>();
    const prepare = vi.fn((request: DetailOriginalPrepareRequest) => new Promise<{ ok: true; data: DetailOriginalPrepared }>((resolve) => {
      pending.set(request.requestId, resolve);
    }));
    const dispose = vi.fn(async () => ({ ok: true as const, data: true }));
    const backend = { detailOriginalPrepare: prepare, detailOriginalDispose: dispose } as unknown as BackendClient;
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const render = (currentGallery: Gallery) => root.render(
      <ProgressiveDetailHero gallery={currentGallery} pageDimension={gallery.pageDimensions[0]} client={client} backend={backend} />,
    );
    try {
      await act(async () => render(completed));
      const pageOneRequest = prepare.mock.calls[0]![0];
      expect(pageOneRequest).toMatchObject({ sourcePage: 1, entryId: "hero-representative" });
      await act(async () => render(selected));
      const representativeRequest = prepare.mock.calls[1]![0];
      expect(representativeRequest).toMatchObject({ sourcePage: 3, entryId: "hero-representative" });
      expect(container.querySelector(".detail-cover")).toHaveAttribute("data-thumbnail-kind", "artifact-page");
      expect(container.querySelector(".detail-hero")).toHaveStyle({ aspectRatio: "1600 / 900" });
      await act(async () => pending.get(representativeRequest.requestId)?.({
        ok: true,
        data: { ...media(representativeRequest.requestId), sourcePage: 3, width: 1600, height: 900 },
      }));
      await act(async () => container.querySelector<HTMLImageElement>(".detail-hero-original")?.dispatchEvent(new Event("load")));
      await act(async () => pending.get(pageOneRequest.requestId)?.({ ok: true, data: media(pageOneRequest.requestId) }));
      expect(container.querySelector(".detail-hero-original")).toHaveAttribute("src", media(representativeRequest.requestId).mediaUrl);
      expect(container.querySelector(".detail-hero-original")).toHaveClass("is-ready");
      expect(dispose).toHaveBeenCalledWith(pageOneRequest.requestId);

      await act(async () => render({ ...selected, download: { ...selected.download!, entryId: "replacement-entry" } }));
      expect(prepare.mock.calls[2]![0]).toMatchObject({ sourcePage: 1, entryId: "replacement-entry" });
      expect(container.querySelector(".detail-cover")).toHaveAttribute("data-thumbnail-kind", "gallery-cover");
      expect(container.querySelector(".detail-hero-original")).toBeNull();
      expect(container.querySelector(".detail-hero")).toHaveStyle({ aspectRatio: "720 / 1080" });
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });

  it("shows the thumbnail immediately and prepares one frontend-created request ID", async () => {
    const prepare = vi.fn(async (request) => ({ ok: true as const, data: media(request.requestId) }));
    const dispose = vi.fn(async () => ({ ok: true as const, data: true }));
    const fixture = await renderHero({ detailOriginalPrepare: prepare, detailOriginalDispose: dispose });
    expect(fixture.container.querySelector(".detail-cover")).toBeTruthy();
    expect(prepare).toHaveBeenCalledTimes(1);
    const request = prepare.mock.calls[0]![0] as { requestId: string; galleryId: string; sourcePage: number };
    expect(request).toMatchObject({ galleryId: gallery.id, sourcePage: 1 });
    expect(request.requestId).toMatch(/^[0-9a-f-]{36}$/);
    expect(fixture.container.querySelector(".detail-hero")).toHaveAttribute("data-original-state", "prepared");
    await act(async () => fixture.root.unmount());
    expect(dispose).toHaveBeenCalledWith(request.requestId);
    fixture.client.dispose(); fixture.container.remove();
  });

  it("uses the Windows custom protocol URL and only onLoad displays the original", async () => {
    const prepare = vi.fn(async (request) => ({ ok: true as const, data: media(request.requestId) }));
    const fixture = await renderHero({ detailOriginalPrepare: prepare, detailOriginalDispose: vi.fn(async () => ({ ok: true as const, data: true })) });
    const original = fixture.container.querySelector<HTMLImageElement>(".detail-hero-original")!;
    expect(original).toHaveAttribute("src", expect.stringMatching(/^http:\/\/detail-original\.localhost\/[0-9a-f-]{36}$/));
    expect(original).not.toHaveClass("is-ready");
    await act(async () => original.dispatchEvent(new Event("load")));
    expect(original).toHaveClass("is-ready");
    expect(fixture.container.querySelector(".detail-hero")).toHaveAttribute("data-original-state", "displayed");
    await act(async () => fixture.root.unmount());
    fixture.client.dispose(); fixture.container.remove();
  });

  it("keeps the thumbnail on command or image failure without automatic retry", async () => {
    const prepare = vi.fn(async () => ({ ok: false as const, error: { code: "DETAIL_ORIGINAL_SOURCE_FAILED", message: "failed", retryable: false } }));
    const first = await renderHero({ detailOriginalPrepare: prepare, detailOriginalDispose: vi.fn(async () => ({ ok: true as const, data: true })) });
    expect(first.container.querySelector(".detail-cover")).toBeTruthy();
    expect(first.container.querySelector(".detail-hero")).toHaveAttribute("data-original-state", "failed");
    await act(async () => Promise.resolve());
    expect(prepare).toHaveBeenCalledTimes(1);
    await act(async () => first.root.unmount()); first.client.dispose(); first.container.remove();

    const imagePrepare = vi.fn(async (request) => ({ ok: true as const, data: media(request.requestId) }));
    const dispose = vi.fn(async () => ({ ok: true as const, data: true }));
    const second = await renderHero({ detailOriginalPrepare: imagePrepare, detailOriginalDispose: dispose });
    const image = second.container.querySelector<HTMLImageElement>(".detail-hero-original")!;
    await act(async () => image.dispatchEvent(new Event("error")));
    expect(second.container.querySelector(".detail-cover")).toBeTruthy();
    expect(second.container.querySelector(".detail-hero")).toHaveAttribute("data-original-state", "failed");
    await act(async () => Promise.resolve());
    expect(imagePrepare).toHaveBeenCalledTimes(1);
    await act(async () => second.root.unmount()); second.client.dispose(); second.container.remove();
  });

  it("disposes a late prepared result after its hero unmounts", async () => {
    let resolve: ((value: { ok: true; data: ReturnType<typeof media> }) => void) | undefined;
    const prepare = vi.fn((_request: { requestId: string }) => new Promise<{ ok: true; data: ReturnType<typeof media> }>((done) => { resolve = done; }));
    const dispose = vi.fn(async () => ({ ok: true as const, data: true }));
    const fixture = await renderHero({ detailOriginalPrepare: prepare, detailOriginalDispose: dispose });
    const requestId = (prepare.mock.calls[0]![0] as { requestId: string }).requestId;
    await act(async () => fixture.root.unmount());
    await act(async () => resolve?.({ ok: true, data: media(requestId) }));
    expect(dispose).toHaveBeenCalledWith(requestId);
    fixture.client.dispose(); fixture.container.remove();
  });

  it("times out once, disposes the request, and does not resurrect a late result", async () => {
    vi.useFakeTimers();
    let resolve: ((value: { ok: true; data: ReturnType<typeof media> }) => void) | undefined;
    const prepare = vi.fn((_request: { requestId: string }) => new Promise<{ ok: true; data: ReturnType<typeof media> }>((done) => { resolve = done; }));
    const dispose = vi.fn(async () => ({ ok: true as const, data: true }));
    try {
      const fixture = await renderHero({ detailOriginalPrepare: prepare, detailOriginalDispose: dispose });
      const requestId = (prepare.mock.calls[0]![0] as { requestId: string }).requestId;
      await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
      expect(fixture.container.querySelector(".detail-hero")).toHaveAttribute("data-original-state", "failed");
      expect(dispose).toHaveBeenCalledWith(requestId);
      await act(async () => resolve?.({ ok: true, data: media(requestId) }));
      expect(fixture.container.querySelector(".detail-hero-original")).toBeNull();
      expect(prepare).toHaveBeenCalledTimes(1);
      await act(async () => fixture.root.unmount());
      fixture.client.dispose(); fixture.container.remove();
    } finally {
      vi.useRealTimers();
    }
  });
});
