import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { BackendClient } from "../api/backend";
import type { DetailOriginalPrepareRequest } from "../api/contracts";
import type { Gallery } from "../core/types";
import { mockGalleries } from "../data/mockGalleries";
import { ThumbnailClient } from "../thumbnail";
import { ProgressivePagePreview } from "./ProgressivePagePreview";

describe("ProgressivePagePreview", () => {
  it("replaces a completed gallery thumbnail with its prepared local original", async () => {
    const gallery: Gallery = {
      ...mockGalleries[0]!,
      download: { entryId: "completed-entry", state: "completed", progress: 100 },
    };
    const prepare = vi.fn(async (request: { requestId: string; galleryId: Gallery["id"]; sourcePage: number; entryId?: string }) => ({
      ok: true as const,
      data: {
        requestId: request.requestId,
        galleryId: request.galleryId,
        sourcePage: request.sourcePage,
        mediaUrl: `http://detail-original.localhost/${request.requestId}`,
        contentType: "image/webp" as const,
        width: 1800,
        height: 2400,
      },
    }));
    const dispose = vi.fn(async () => ({ ok: true as const, data: true }));
    const backend = { detailOriginalPrepare: prepare, detailOriginalDispose: dispose } as unknown as BackendClient;
    const client = new ThumbnailClient({
      resolve: () => ({ kind: "image" as const, url: "https://images.example.test/thumbnail.webp", width: 600, height: 800 }),
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <ProgressivePagePreview gallery={gallery} page={7} client={client} backend={backend} />,
      ));
      await act(async () => Promise.resolve());
      expect(prepare).toHaveBeenCalledWith(expect.objectContaining({
        galleryId: gallery.id,
        sourcePage: 7,
        entryId: "completed-entry",
      }));
      expect(container.querySelector(".page-preview-media")).toHaveAttribute("data-original-source", "local-artifact");
      const original = container.querySelector<HTMLImageElement>(".page-preview-original")!;
      expect(original.src).toContain("detail-original.localhost");
      expect(original.alt).toBe("");
      await act(async () => original.dispatchEvent(new Event("load")));
      expect(original).toHaveClass("is-ready");
    } finally {
      await act(async () => root.unmount());
      expect(dispose).toHaveBeenCalled();
      client.dispose();
      container.remove();
    }
  });

  it("keeps incomplete galleries on the bounded thumbnail path", async () => {
    const gallery: Gallery = { ...mockGalleries[0]!, download: { entryId: "active-entry", state: "downloading" } };
    const prepare = vi.fn();
    const backend = { detailOriginalPrepare: prepare, detailOriginalDispose: vi.fn() } as unknown as BackendClient;
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "test fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <ProgressivePagePreview
          gallery={gallery}
          page={3}
          expectedDimension={{ width: 1600, height: 900 }}
          client={client}
          backend={backend}
        />,
      ));
      expect(prepare).not.toHaveBeenCalled();
      const media = container.querySelector<HTMLElement>(".page-preview-media")!;
      expect(media).toHaveAttribute("data-original-source", "thumbnail");
      expect(media).toHaveAttribute("data-page-orientation", "landscape");
      expect(media).toHaveStyle({ aspectRatio: "1600 / 900" });
      expect(container.querySelector(".page-preview-original")).toBeNull();
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });

  it("reports the resolved source dimensions so the dialog can replace its metadata fallback", async () => {
    const gallery: Gallery = { ...mockGalleries[0]!, download: undefined };
    const backend = { detailOriginalPrepare: vi.fn(), detailOriginalDispose: vi.fn() } as unknown as BackendClient;
    const client = new ThumbnailClient({
      resolve: () => ({ kind: "image" as const, url: "https://images.example.test/resolved.webp", width: 720, height: 1080 }),
    });
    const onDimensionResolved = vi.fn();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <ProgressivePagePreview
          gallery={gallery}
          page={4}
          client={client}
          backend={backend}
          onDimensionResolved={onDimensionResolved}
        />,
      ));
      await act(async () => Promise.resolve());
      expect(onDimensionResolved).toHaveBeenCalledWith({
        galleryId: gallery.id,
        page: 4,
        width: 720,
        height: 1080,
      });
      expect(container.querySelector(".page-preview-media")).toHaveAttribute("data-page-orientation", "portrait");
      expect(container.querySelector<HTMLElement>(".page-preview-media")).toHaveStyle({ aspectRatio: "2 / 3" });
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });

  it("disposes a rejected preparation and keeps the source thumbnail fallback", async () => {
    const gallery: Gallery = {
      ...mockGalleries[0]!,
      download: { entryId: "completed-entry", state: "completed", progress: 100 },
    };
    const prepare = vi.fn(async (_request: DetailOriginalPrepareRequest) => Promise.reject(new Error("transport unavailable")));
    const dispose = vi.fn(async () => ({ ok: true as const, data: true }));
    const backend = { detailOriginalPrepare: prepare, detailOriginalDispose: dispose } as unknown as BackendClient;
    const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "test fixture" }) });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <ProgressivePagePreview gallery={gallery} page={11} client={client} backend={backend} />,
      ));
      await act(async () => Promise.resolve());
      const requestId = prepare.mock.calls[0]?.[0]?.requestId;
      expect(dispose).toHaveBeenCalledWith(requestId);
      expect(container.querySelector(".page-preview-media")).toHaveAttribute("data-original-state", "failed");
      expect(container.querySelector(".page-preview-fallback")).not.toBeNull();
      expect(container.querySelector(".page-preview-original")).toBeNull();
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });
});
