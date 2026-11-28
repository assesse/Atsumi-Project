import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { BackendClient } from "../api/backend";
import type { ApiResult, DetailOriginalPrepared, DetailOriginalPrepareRequest } from "../api/contracts";
import type { Gallery } from "../core/types";
import { mockGalleries } from "../data/mockGalleries";
import { ThumbnailClient, artifactPageThumbnailKey, galleryCoverThumbnailKey, sourcePageThumbnailKey, type ThumbnailAsset, type ThumbnailCoordinatorAdapter, type ThumbnailKey } from "../thumbnail";
import { OriginalPageWindow } from "./OriginalPageWindow";
import { ProgressivePagePreview } from "./ProgressivePagePreview";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const remoteGallery: Gallery = { ...mockGalleries[0]!, download: undefined };
const cleanups: Array<() => Promise<void>> = [];
function fixture(resolve: ThumbnailCoordinatorAdapter["resolve"] = request => ({
  kind: "image", url: `https://preview.test/${request.key.kind}/${"page" in request.key ? request.key.page : 1}.webp`, width: 600, height: 900,
})) {
  const adapter = { resolve: vi.fn(resolve) };
  const client = new ThumbnailClient(adapter);
  const pending: Array<ReturnType<typeof deferred<ApiResult<DetailOriginalPrepared>>> & { request: DetailOriginalPrepareRequest }> = [];
  const prepare = vi.fn<BackendClient["detailOriginalPrepare"]>(request => {
    const next = { ...deferred<ApiResult<DetailOriginalPrepared>>(), request };
    pending.push(next);
    return next.promise;
  });
  const dispose = vi.fn<BackendClient["detailOriginalDispose"]>(async () => ({ ok: true, data: true }));
  const backend = { detailOriginalPrepare: prepare, detailOriginalDispose: dispose } as unknown as BackendClient;
  const host = document.createElement("div");
  document.body.append(host);
  const root: Root = createRoot(host);
  const releases: Array<() => void> = [];
  cleanups.push(async () => {
    await act(async () => root.unmount());
    releases.forEach(release => release());
    client.dispose();
    host.remove();
  });
  return {
    host, root, client, adapter, backend, prepare, dispose, pending,
    warm: async (key: ThumbnailKey) => {
      releases.push(client.subscribe({ key, consumer: "detail", priority: "visible" }, () => undefined));
      await Promise.resolve();
    },
    render: async (page = 18, gallery = remoteGallery, onDimensionResolved = vi.fn()) => {
      await act(async () => root.render(<ProgressivePagePreview gallery={gallery} page={page} client={client} backend={backend} onDimensionResolved={onDimensionResolved} />));
    },
    finish: async (index = 0) => {
      const item = pending[index]!;
      await act(async () => item.resolve({ ok: true, data: { ...item.request, mediaUrl: `http://original.test/${item.request.requestId}`, width: 1800, height: 2700, contentType: "image/webp" } }));
      return host.querySelector<HTMLImageElement>(".page-preview-original")!;
    },
  };
}
afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) await cleanup();
  vi.restoreAllMocks();
});

describe("progressive page presentation", () => {
  it("keeps the same cached preview through prepare, image load and delayed decode, with no second thumbnail request", async () => {
    const f = fixture();
    await f.warm(sourcePageThumbnailKey(remoteGallery, 18));
    await f.render();
    const preview = f.host.querySelector(".page-preview-fallback img");
    expect(preview).toHaveAttribute("src", "https://preview.test/source-page/18.webp");
    const image = await f.finish();
    const decoded = deferred<void>();
    const decode = vi.fn(() => decoded.promise);
    Object.defineProperty(image, "decode", { value: decode });
    await act(async () => image.dispatchEvent(new Event("load")));
    expect(image).not.toHaveClass("is-ready");
    expect(f.host.querySelector(".page-preview-fallback img")).toBe(preview);
    expect(f.host.querySelector(".page-preview-fallback")).not.toHaveAttribute("aria-hidden", "true");
    await act(async () => decoded.resolve());
    expect(image).toHaveClass("is-ready");
    expect(f.host.querySelector(".page-preview-fallback")).toHaveAttribute("aria-hidden", "true");
    expect(f.adapter.resolve).toHaveBeenCalledTimes(1);
    expect(decode).toHaveBeenCalledTimes(1);
    // A later display failure reveals the already mounted preview immediately.
    await act(async () => image.dispatchEvent(new Event("error")));
    expect(f.host.querySelector(".page-preview-fallback img")).toBe(preview);
    expect(f.host.querySelector(".page-preview-fallback")).not.toHaveAttribute("aria-hidden", "true");
    expect(f.host.querySelector(".page-preview-original")).toBeNull();
    expect(f.adapter.resolve).toHaveBeenCalledTimes(1);
  });

  it("joins an in-flight page preview rather than waiting for an original failure", async () => {
    const thumbnail = deferred<ThumbnailAsset>();
    const f = fixture(() => thumbnail.promise);
    await f.warm(sourcePageThumbnailKey(remoteGallery, 18));
    await f.render();
    expect(f.host.querySelector(".page-preview-fallback")).toHaveAttribute("data-thumbnail-state", "loading");
    await act(async () => thumbnail.resolve({ kind: "image", url: "https://preview.test/inflight.webp", width: 600, height: 900 }));
    expect(f.host.querySelector(".page-preview-fallback img")).toHaveAttribute("src", "https://preview.test/inflight.webp");
    expect(f.host.querySelector(".page-preview-media")).toHaveAttribute("data-original-state", "preparing");
    expect(f.adapter.resolve).toHaveBeenCalledTimes(1);
  });

  it("uses a warm cover only for page one, never for an uncached later page", async () => {
    const f = fixture();
    await f.warm(galleryCoverThumbnailKey(remoteGallery));
    await f.render(1);
    expect(f.host.querySelector(".page-preview-fallback img")).toHaveAttribute("src", "https://preview.test/gallery-cover/1.webp");
    expect(f.adapter.resolve).toHaveBeenCalledTimes(1);
    await f.render(18);
    expect(f.host.querySelector(".page-preview-fallback img")).toHaveAttribute("src", "https://preview.test/source-page/18.webp");
    expect(f.adapter.resolve).toHaveBeenCalledTimes(2);
  });

  it("prefers the exact cached first-page preview when a cover is also cached", async () => {
    const f = fixture();
    await f.warm(galleryCoverThumbnailKey(remoteGallery));
    await f.warm(sourcePageThumbnailKey(remoteGallery, 1));
    await f.render(1);
    expect(f.host.querySelector(".page-preview-fallback img")).toHaveAttribute("src", "https://preview.test/source-page/1.webp");
    expect(f.adapter.resolve).toHaveBeenCalledTimes(2);
  });

  it("keeps completed pages local even if the gallery has a different representative cover", async () => {
    const f = fixture();
    const gallery: Gallery = { ...remoteGallery, download: { state: "completed", entryId: "local-entry", progress: 100 }, representativePreview: { galleryId: remoteGallery.id, mode: "manual", sourcePage: 7, manualSourcePage: 7, entryId: "local-entry", width: 600, height: 900, candidates: [7], algorithmVersion: 1, updatedAt: "2026-11-22T00:00:00Z" } };
    await f.warm(artifactPageThumbnailKey("local-entry", 18));
    await f.render(18, gallery);
    expect(f.host.querySelector(".page-preview-fallback img")).toHaveAttribute("src", "https://preview.test/artifact-page/18.webp");
    expect(f.adapter.resolve).toHaveBeenCalledTimes(1);
    expect(f.prepare).toHaveBeenCalledWith(expect.objectContaining({ sourcePage: 18, entryId: "local-entry" }));
  });

  it("ignores an old preparation after a page change and disposes its lease", async () => {
    const f = fixture();
    await f.render(18);
    await f.render(19);
    await f.finish(0);
    expect(f.host.querySelector(".page-preview-original")).toBeNull();
    expect(f.host.querySelector(".page-preview-fallback img")).toHaveAttribute("src", "https://preview.test/source-page/19.webp");
    expect(f.dispose).toHaveBeenCalledWith(f.pending[0]!.request.requestId);
    const current = await f.finish(1);
    expect(current).toHaveAttribute("src", `http://original.test/${f.pending[1]!.request.requestId}`);
  });

  it("ignores a late decode from another album even when the source page number is the same", async () => {
    const f = fixture();
    await f.render();
    const old = await f.finish();
    const decoded = deferred<void>();
    Object.defineProperty(old, "decode", { value: () => decoded.promise });
    await act(async () => old.dispatchEvent(new Event("load")));
    const nextGallery: Gallery = { ...remoteGallery, id: mockGalleries[1]!.id };
    await f.render(18, nextGallery);
    const current = await f.finish(1);
    await act(async () => decoded.resolve());
    expect(current).not.toHaveClass("is-ready");
    expect(f.host.querySelector(".page-preview-original")).toBe(current);
    await act(async () => current.dispatchEvent(new Event("load")));
    expect(current).toHaveClass("is-ready");
  });

  it("falls back immediately on a decode failure without refetching the thumbnail", async () => {
    const f = fixture();
    await f.render();
    const image = await f.finish();
    Object.defineProperty(image, "decode", { value: () => Promise.reject(new Error("invalid image")) });
    await act(async () => image.dispatchEvent(new Event("load")));
    expect(f.host.querySelector(".page-preview-media")).toHaveAttribute("data-original-state", "failed");
    expect(f.host.querySelector(".page-preview-fallback img")).not.toBeNull();
    expect(f.adapter.resolve).toHaveBeenCalledTimes(1);
    expect(f.dispose).toHaveBeenCalledWith(f.pending[0]!.request.requestId);
  });

  it("promotes an already complete cached original without needing another load event", async () => {
    vi.spyOn(HTMLImageElement.prototype, "complete", "get").mockReturnValue(true);
    vi.spyOn(HTMLImageElement.prototype, "naturalWidth", "get").mockReturnValue(1800);
    const f = fixture();
    await f.render();
    const image = await f.finish();
    expect(image).toHaveClass("is-ready");
  });

  it("does not let a late thumbnail replace the original dimensions", async () => {
    const thumbnail = deferred<ThumbnailAsset>();
    const f = fixture(() => thumbnail.promise);
    const dimension = vi.fn();
    await f.render(18, remoteGallery, dimension);
    await f.finish();
    expect(dimension).toHaveBeenLastCalledWith({ galleryId: remoteGallery.id, page: 18, width: 1800, height: 2700 });
    const calls = dimension.mock.calls.length;
    await act(async () => thumbnail.resolve({ kind: "image", url: "https://preview.test/late.webp", width: 200, height: 300 }));
    expect(dimension).toHaveBeenCalledTimes(calls);
  });

  it("shows both cached pages in the real original-window path while preserving two concurrent original reads", async () => {
    const f = fixture();
    await f.warm(sourcePageThumbnailKey(remoteGallery, 18));
    await f.warm(sourcePageThumbnailKey(remoteGallery, 19));
    await act(async () => f.root.render(
      <OriginalPageWindow gallery={remoteGallery} pages={[18, 19]} count={31} backend={f.backend}>
        {[18, 19].map(page => <ProgressivePagePreview key={page} gallery={remoteGallery} page={page} client={f.client} backend={f.backend} />)}
      </OriginalPageWindow>,
    ));
    expect(f.pending.map(item => item.request.sourcePage)).toEqual([18, 19]);
    expect(Array.from(f.host.querySelectorAll(".page-preview-fallback img"), img => img.getAttribute("src"))).toEqual([
      "https://preview.test/source-page/18.webp", "https://preview.test/source-page/19.webp",
    ]);
    expect(f.adapter.resolve).toHaveBeenCalledTimes(2);
    await f.finish(0);
    expect(f.host.querySelector(".page-preview-original")).not.toHaveClass("is-ready");
    expect(f.host.querySelectorAll(".page-preview-fallback img")).toHaveLength(2);
  });
});
