import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { galleryId } from "../core/types";
import { ThumbnailClient, sourcePageThumbnailKey } from "../thumbnail";
import { mockGalleries } from "../data/mockGalleries";
import { GalleryThumbnail } from "./GalleryThumbnail";

const coverKey = {
  kind: "gallery-cover" as const,
  galleryId: galleryId(4051038),
  sourceKey: "opaque-cover-key",
  fallback: { kind: "fixture-sheet-cell" as const, index: 0 },
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("GalleryThumbnail", () => {
  it("renders a coordinator URL with intrinsic dimensions, containment and eager priority", async () => {
    const client = new ThumbnailClient({
      resolve: () => ({
        kind: "image",
        url: "https://images.example.test/cover.jpg",
        width: 720,
        height: 1080,
      }),
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <GalleryThumbnail
        thumbnailKey={coverKey}
        consumer="downloads"
        priority="visible"
        client={client}
        sizing="intrinsic"
        alt="Archive of Rain 표지"
      />,
    ));

    const media = container.querySelector<HTMLElement>(".gallery-thumbnail");
    const image = media?.querySelector<HTMLImageElement>("img");
    expect(media).toHaveStyle({ aspectRatio: "720 / 1080" });
    expect(media).toHaveAttribute("data-thumbnail-consumer", "downloads");
    expect(media).toHaveAttribute("data-thumbnail-priority", "visible");
    expect(image).toHaveAttribute("width", "720");
    expect(image).toHaveAttribute("height", "1080");
    expect(image).toHaveAttribute("loading", "eager");
    expect(image).toHaveAttribute("alt", "Archive of Rain 표지");
    expect(image).toHaveStyle({ objectFit: "contain" });

    await act(async () => root.unmount());
    container.remove();
  });

  it("reserves a supplied expected ratio before and after asynchronous resolution", async () => {
    let finish: ((asset: { kind: "image"; url: string; width: number; height: number }) => void) | undefined;
    const pending = new Promise<{ kind: "image"; url: string; width: number; height: number }>((resolve) => {
      finish = resolve;
    });
    const client = new ThumbnailClient({ resolve: () => pending });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <GalleryThumbnail
        thumbnailKey={coverKey}
        consumer="explore"
        priority="visible"
        client={client}
        sizing="intrinsic"
        expectedAspectRatio={{ width: 720, height: 1080 }}
        alt="비율 예약 표지"
      />,
    ));

    const media = container.querySelector<HTMLElement>(".gallery-thumbnail");
    expect(media).toHaveStyle({ aspectRatio: "720 / 1080" });
    // The decoded handle intentionally has different dimensions: the expected
    // ratio must reserve the layout through both states.
    finish?.({ kind: "image", url: "https://images.example.test/reserved.jpg", width: 1000, height: 1000 });
    await act(async () => { await pending; });
    expect(media).toHaveStyle({ aspectRatio: "720 / 1080" });

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("clips a 3x2 fixture sheet to a square cell without stretching the cell", async () => {
    const gallery = { ...mockGalleries[0]!, coverIndex: 1 };
    const pageKey = sourcePageThumbnailKey(gallery, 5);
    const client = new ThumbnailClient({
      resolve: ({ key }) => ({
        kind: "sprite",
        url: "/mock-gallery-sheet.png",
        sheetWidth: 1536,
        sheetHeight: 1024,
        columns: 3,
        rows: 2,
        cell: key.fallback?.index ?? 0,
      }),
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <GalleryThumbnail
        thumbnailKey={pageKey}
        consumer="review"
        priority="prefetch"
        client={client}
        alt="비교 페이지"
      />,
    ));

    const image = container.querySelector<HTMLImageElement>(".thumbnail-image--sprite");
    expect(image).toHaveAttribute("width", "1536");
    expect(image).toHaveAttribute("height", "1024");
    expect(image).toHaveAttribute("loading", "lazy");
    expect(image).toHaveStyle({ width: "300%", height: "200%", left: "-200%", top: "-100%" });

    await act(async () => root.unmount());
    container.remove();
  });

  it("shares a decoded-image failure with subscribers and reports it to the adapter", async () => {
    const displayFailed = vi.fn();
    const release = vi.fn();
    const client = new ThumbnailClient({
      resolve: () => ({
        kind: "image",
        url: "https://images.example.test/broken.jpg",
        width: 512,
        height: 512,
      }),
      displayFailed,
      release,
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <GalleryThumbnail
        thumbnailKey={coverKey}
        consumer="detail"
        priority="critical"
        client={client}
        alt="깨진 표지"
      />,
    ));
    const image = container.querySelector<HTMLImageElement>("img");
    await act(async () => image?.dispatchEvent(new Event("error")));

    expect(container.querySelector(".gallery-thumbnail")).toHaveAttribute("data-thumbnail-state", "error");
    expect(container.querySelector(".thumbnail-fallback--error")).toHaveAccessibleName(
      "깨진 표지 이미지를 안전하게 처리할 수 없음",
    );
    expect(displayFailed).toHaveBeenCalledWith(
      expect.objectContaining({ key: coverKey, consumer: "detail", priority: "critical" }),
      "Resolved thumbnail could not be decoded",
    );
    expect(release).toHaveBeenCalledWith(
      expect.objectContaining({ key: coverKey }),
      expect.objectContaining({ kind: "image", url: "https://images.example.test/broken.jpg" }),
    );

    await act(async () => root.unmount());
    container.remove();
  });

  it("defers offscreen work and promotes a near-viewport thumbnail to visible", async () => {
    let triggerIntersection: ((visible: boolean) => void) | undefined;
    const observe = vi.fn();
    const disconnect = vi.fn();
    class TestIntersectionObserver {
      static lastRoot: Element | Document | null = null;
      readonly root: Element | Document | null;
      readonly rootMargin = "600px 0px";
      readonly thresholds = [0.01];

      constructor(callback: IntersectionObserverCallback, options?: IntersectionObserverInit) {
        this.root = options?.root ?? null;
        TestIntersectionObserver.lastRoot = this.root;
        triggerIntersection = (visible) => callback(
          [{ isIntersecting: visible } as IntersectionObserverEntry],
          this as unknown as IntersectionObserver,
        );
      }

      observe = observe;
      unobserve = vi.fn();
      disconnect = disconnect;
      takeRecords = () => [];
    }
    vi.stubGlobal("IntersectionObserver", TestIntersectionObserver);
    const resolve = vi.fn(() => ({ kind: "missing" as const, reason: "fixture" }));
    const client = new ThumbnailClient({ resolve });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <div className="gallery-viewport">
        <GalleryThumbnail
          thumbnailKey={coverKey}
          consumer="explore"
          priority="prefetch"
          client={client}
          alt="화면 밖 표지"
        />
      </div>,
    ));

    expect(resolve).not.toHaveBeenCalled();
    expect(observe).toHaveBeenCalledWith(container.querySelector(".gallery-thumbnail"));
    expect(TestIntersectionObserver.lastRoot).toBe(container.querySelector(".gallery-viewport"));
    expect(container.querySelector(".gallery-thumbnail")).toHaveAttribute("data-thumbnail-state", "deferred");
    expect(container.querySelector(".gallery-thumbnail")).toHaveAttribute("data-thumbnail-priority", "prefetch");

    await act(async () => triggerIntersection?.(true));

    expect(resolve).toHaveBeenCalledOnce();
    expect(resolve).toHaveBeenCalledWith(expect.objectContaining({
      key: coverKey,
      consumer: "explore",
      priority: "visible",
    }));
    expect(container.querySelector(".gallery-thumbnail")).toHaveAttribute("data-thumbnail-priority", "visible");
    expect(container.querySelector(".gallery-thumbnail")).toHaveAttribute("data-thumbnail-state", "missing");
    expect(disconnect).not.toHaveBeenCalled();

    await act(async () => {
      triggerIntersection?.(false);
      await Promise.resolve();
    });
    expect(container.querySelector(".gallery-thumbnail")).toHaveAttribute("data-thumbnail-state", "deferred");
    expect(container.querySelector(".gallery-thumbnail")).toHaveAttribute("data-thumbnail-priority", "prefetch");
    expect(client.getSnapshot(coverKey)).toEqual({
      status: "resolved",
      asset: { kind: "missing", reason: "fixture" },
    });

    await act(async () => root.unmount());
    client.dispose();
    expect(disconnect).toHaveBeenCalledOnce();
    container.remove();
  });

  it("uses the Detail scroll root instead of activating an offscreen prefetch immediately", async () => {
    const observe = vi.fn();
    class TestIntersectionObserver {
      static root: Element | Document | null = null;
      constructor(_callback: IntersectionObserverCallback, options?: IntersectionObserverInit) {
        TestIntersectionObserver.root = options?.root ?? null;
      }
      observe = observe;
      unobserve = vi.fn();
      disconnect = vi.fn();
      takeRecords = () => [];
    }
    vi.stubGlobal("IntersectionObserver", TestIntersectionObserver);
    const resolve = vi.fn(() => ({ kind: "missing" as const, reason: "fixture" }));
    const client = new ThumbnailClient({ resolve });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const pageKey = { kind: "source-page" as const, galleryId: galleryId(4051038), page: 3 };
    await act(async () => root.render(
      <div data-thumbnail-scroll-root>
        <GalleryThumbnail thumbnailKey={pageKey} consumer="detail" priority="prefetch" client={client} alt="상세 화면 밖 페이지" />
      </div>,
    ));
    expect(resolve).not.toHaveBeenCalled();
    expect(TestIntersectionObserver.root).toBe(container.querySelector("[data-thumbnail-scroll-root]"));
    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });
});
