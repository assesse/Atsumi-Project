import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import { galleryGroupStorageKey, groupGalleries } from "../state/galleryGrouping";
import { GalleryCoverSessionRetainer, ThumbnailClient, type ThumbnailAsset, type ThumbnailRequest } from "../thumbnail";
import { DownloadArtistFolderGrid, savedArtistPreviewGalleries } from "./DownloadArtistFolderGrid";

const downloadedGallery = (id: number, artist: string, createdAt: string, tags: string[] = []): Gallery => ({
  id: galleryId(id),
  title: `Work ${id}`,
  subtitle: "",
  artist,
  pages: 20,
  score: 0,
  publishedAt: "2025-01-01",
  coverIndex: 0,
  language: "korean",
  tags,
  series: [],
  characters: [],
  thumbnailWidth: 400,
  thumbnailHeight: 600,
  download: {
    entryId: `entry-${id}`,
    state: "completed",
    progress: 100,
    createdAt,
  },
});

const thumbnailClient = () => new ThumbnailClient({
  resolve: () => ({
    kind: "image",
    url: "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==",
    width: 400,
    height: 600,
  }),
});

const bounds = (left: number, top: number, width: number, height: number): DOMRect => ({
  x: left,
  y: top,
  left,
  top,
  width,
  height,
  right: left + width,
  bottom: top + height,
  toJSON: () => ({}),
});

const mockFolderBounds = (button: HTMLButtonElement) => {
  const previewStack = button.querySelector<HTMLElement>(".download-artist-folder-preview-stack")!;
  const preview = button.querySelector<HTMLElement>(".download-artist-folder-preview")!;
  button.getBoundingClientRect = () => bounds(100, 80, 500, 330);
  previewStack.getBoundingClientRect = () => bounds(100, 80, 220, 330);
  preview.getBoundingClientRect = () => bounds(120, 105, 180, 280);
  return previewStack;
};

describe("DownloadArtistFolderGrid", () => {
  it.each([[2, 3], [3, 4], [4, 5]])("preloads every folder's %i-column fan without opening, hovering, or entering the viewport", async (columns, count) => {
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      disconnect() {}
    });
    const items = ["Alpha", "Beta"].flatMap((artist, artistIndex) => Array.from({ length: 6 }, (_, index) => (
      downloadedGallery(701 + artistIndex * 100 + index, artist, `2026-09-0${index + 1}T10:00:00Z`)
    )));
    items[4] = {
      ...items[4]!,
      representativePreview: {
        galleryId: galleryId(705), mode: "manual", sourcePage: 7, manualSourcePage: 7,
        entryId: "entry-705", width: 400, height: 600, candidates: [7], algorithmVersion: 1, updatedAt: "2026-09-07T10:00:00Z",
      },
    };
    const groups = groupGalleries(items, "artist", (gallery) => gallery.download?.createdAt);
    const saved = new Map([
      ["alpha", [705, 702, 706, 701, 704, 703].map(galleryId)],
      ["beta", [803, 801, 806, 804, 802, 805].map(galleryId)],
    ]);
    const expected = [...saved.values()].flatMap((ids) => ids.slice(0, count));
    const requestId = (request: ThumbnailRequest): number => request.key.kind === "artifact-page"
      ? Number(request.key.entryId.replace("entry-", ""))
      : Number(request.key.galleryId);
    const resolvers = new Map<number, (asset: ThumbnailAsset) => void>();
    const resolve = vi.fn((request: ThumbnailRequest) => new Promise<ThumbnailAsset>((done) => {
      resolvers.set(requestId(request), done);
    }));
    const client = new ThumbnailClient({ resolve });
    const subscribe = vi.spyOn(client, "subscribe");
    const retainer = new GalleryCoverSessionRetainer(client);
    const onPreviewItems = vi.fn();
    const renderGrid = vi.fn(() => null);
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const props = {
      columns, previewWidth: 220, favoriteMetadata: new Set<string>(),
      collapsedGroupKeys: new Set(groups.map((group) => galleryGroupStorageKey("downloads", group))),
      onToggle: vi.fn(), onPreviewItems, renderGrid, thumbnailClient: client, coverRetainer: retainer,
    };

    try {
      await act(async () => root.render(<DownloadArtistFolderGrid {...props} groups={groups} previewGalleryIdsByArtist={saved} />));
      expect(resolve.mock.calls.map(([request]) => requestId(request))).toEqual(expected.slice(0, 4));
      expect(resolve.mock.calls[0]?.[0].key).toMatchObject({ kind: "artifact-page", entryId: "entry-705", page: 7 });
      expect(resolve.mock.calls.every(([request]) => request.priority === "prefetch")).toBe(true);
      expect(retainer.size).toBe(0);
      expect(onPreviewItems).not.toHaveBeenCalled();
      expect(renderGrid).not.toHaveBeenCalled();
      expect(container.querySelectorAll(".download-artist-folder-preview")).toHaveLength(2);
      expect(document.body.querySelector(".download-artist-folder-preview-overlay")).toBeNull();

      await act(async () => root.render(
        <DownloadArtistFolderGrid {...props}
          groups={groups.map((group) => ({ ...group, items: group.items.map((item) => ({ ...item, tags: ["new_tag"] })) }))}
          previewGalleryIdsByArtist={new Map(saved)}
        />,
      ));
      expect(subscribe).toHaveBeenCalledTimes(4);

      for (const [index, id] of expected.entries()) {
        await act(async () => resolvers.get(id)?.({ kind: "image", url: `blob:https://atsumi.test/${id}`, width: 400, height: 600 }));
        expect(resolve).toHaveBeenCalledTimes(Math.min(expected.length, 4 + index + 1));
      }
      expect(resolve.mock.calls.map(([request]) => requestId(request))).toEqual(expected);
      expect(retainer.size).toBe(expected.length);
      expect(renderGrid).not.toHaveBeenCalled();
      expect(onPreviewItems).not.toHaveBeenCalled();
    } finally {
      await act(async () => root.unmount());
      retainer.clear();
      client.dispose();
      container.remove();
      vi.unstubAllGlobals();
    }
  });

  it("cancels pending background subscriptions and stops queued covers on unmount", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      disconnect() {}
    });
    const groups = groupGalleries(["Alpha", "Beta"].flatMap((artist, artistIndex) => Array.from({ length: 3 }, (_, index) => (
      downloadedGallery(901 + artistIndex * 100 + index, artist, "2026-09-07T10:00:00Z")
    ))), "artist", (gallery) => gallery.download?.createdAt);
    const resolvers: Array<(asset: ThumbnailAsset) => void> = [];
    const resolve = vi.fn(() => new Promise<ThumbnailAsset>((done) => { resolvers.push(done); }));
    const cancel = vi.fn();
    const client = new ThumbnailClient({ resolve, cancel });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => root.render(
        <DownloadArtistFolderGrid groups={groups} columns={2} previewWidth={220} favoriteMetadata={new Set()}
          collapsedGroupKeys={new Set(groups.map((group) => galleryGroupStorageKey("downloads", group)))}
          onToggle={vi.fn()} renderGrid={() => null} thumbnailClient={client}
        />,
      ));
      expect(resolve).toHaveBeenCalledTimes(4);
      await act(async () => root.unmount());
      await vi.advanceTimersByTimeAsync(400);
      expect(cancel).toHaveBeenCalledTimes(4);
      for (const done of resolvers) done({ kind: "missing" });
      await Promise.resolve();
      await Promise.resolve();
      expect(resolve).toHaveBeenCalledTimes(4);
    } finally {
      client.dispose();
      container.remove();
      vi.useRealTimers();
      vi.unstubAllGlobals();
    }
  });

  it("keeps saved candidate order and replaces only missing or filtered works", () => {
    const items = [
      downloadedGallery(101, "Mizuno", "2026-09-01T10:00:00Z"),
      downloadedGallery(102, "Mizuno", "2026-09-03T10:00:00Z"),
      downloadedGallery(103, "Mizuno", "2026-09-02T10:00:00Z"),
    ];
    expect(savedArtistPreviewGalleries(items, [galleryId(101), galleryId(999), galleryId(103), galleryId(101)], 3)
      .map((item) => item.id)).toEqual([101, 103, 102]);
    expect(savedArtistPreviewGalleries(items, [galleryId(101), galleryId(103)], 1)
      .map((item) => item.id)).toEqual([101]);
  });

  it("previews the newest covers at their original size only while hovering the folder preview", async () => {
    const groups = groupGalleries([
      downloadedGallery(101, "Mizuno", "2026-09-01T10:00:00Z", ["female:glasses", "full_color"]),
      downloadedGallery(102, "Mizuno", "2026-09-03T10:00:00Z", ["female:glasses", "sole_female"]),
      downloadedGallery(103, "Mizuno", "2026-09-02T10:00:00Z", ["female:glasses", "full_color"]),
      downloadedGallery(104, "Mizuno", "2026-08-31T10:00:00Z", ["full_color"]),
      downloadedGallery(201, "Serein", "2026-09-04T10:00:00Z"),
    ], "artist", (gallery) => gallery.download?.createdAt);
    const collapsed = new Set(groups.map((group) => galleryGroupStorageKey("downloads", group)));
    const onToggle = vi.fn();
    const onPreviewItems = vi.fn();
    const onStopPreviewItems = vi.fn();
    const client = thumbnailClient();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <DownloadArtistFolderGrid
        groups={groups}
        columns={2}
        previewWidth={220}
        favoriteMetadata={new Set(["full_color"])}
        collapsedGroupKeys={collapsed}
        onToggle={onToggle}
        onPreviewItems={onPreviewItems}
        onStopPreviewItems={onStopPreviewItems}
        renderGrid={() => null}
        thumbnailClient={client}
      />,
    ));

    const grid = container.querySelector<HTMLElement>(".download-artist-folder-grid");
    const mizuno = [...container.querySelectorAll<HTMLElement>(".download-artist-folder-card")]
      .find((card) => card.textContent?.includes("Mizuno"));
    const button = mizuno!.querySelector<HTMLButtonElement>(".download-artist-folder-button")!;
    const copy = button.querySelector<HTMLElement>(".download-artist-folder-copy")!;
    const previewStack = mockFolderBounds(button);
    expect(grid?.style.gridTemplateColumns).toBe("repeat(2, minmax(0, 1fr))");
    expect(grid).toHaveAttribute("data-preview-size", "standard");
    expect(button).toHaveAttribute("aria-expanded", "false");
    expect(button).toHaveAccessibleName("Mizuno 작가 폴더, 4개 작품, 열기");
    expect(mizuno?.querySelector(".download-artist-folder-type")).toHaveTextContent("작가 폴더");
    expect(mizuno?.querySelector(".download-artist-folder-latest")).toBeNull();
    expect(mizuno?.querySelector(".download-artist-folder-tags")).toHaveTextContent("full color3");
    expect(mizuno?.querySelector(".download-artist-folder-tag.is-favorite")).toHaveTextContent("★full color3");
    expect(mizuno?.querySelectorAll(".download-artist-folder-preview")).toHaveLength(1);

    await act(async () => {
      copy.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    });
    expect(document.body.querySelector(".download-artist-folder-preview-overlay")).toBeNull();
    expect(onPreviewItems).not.toHaveBeenCalled();

    await act(async () => {
      copy.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: previewStack }));
    });
    expect([...document.body.querySelectorAll<HTMLElement>(".download-artist-folder-overlay-preview")]
      .map((cover) => cover.dataset.galleryId)).toEqual(["102", "103", "101"]);
    const overlay = document.body.querySelector<HTMLElement>(".download-artist-folder-preview-overlay");
    expect(overlay).toHaveAttribute("data-preview-count", "3");
    expect(overlay?.style.getPropertyValue("--folder-preview-width")).toBe("180px");
    expect(overlay?.style.getPropertyValue("--folder-preview-height")).toBe("280px");
    expect(overlay?.style.width).toBe("564px");
    expect(overlay?.style.height).toBe("280px");
    expect(onPreviewItems).toHaveBeenCalledWith([galleryId(102), galleryId(103), galleryId(101), galleryId(104)]);
    expect(onPreviewItems).toHaveBeenCalledTimes(1);
    expect(container.querySelector(".download-artist-folder-contents")).toBeNull();

    await act(async () => {
      previewStack.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: copy }));
    });
    expect(document.body.querySelector(".download-artist-folder-preview-overlay")).toBeNull();
    expect(onStopPreviewItems).toHaveBeenCalledWith([galleryId(102), galleryId(103), galleryId(101), galleryId(104)]);
    expect(onPreviewItems).toHaveBeenCalledTimes(1);

    await act(async () => previewStack.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    expect(document.body.querySelector(".download-artist-folder-preview-overlay")).not.toBeNull();
    await act(async () => button.click());
    expect(onToggle).toHaveBeenCalledWith(expect.stringContaining("downloads\u001fartist\u001fmizuno"));
    expect(onStopPreviewItems).toHaveBeenCalled();
    expect(document.body.querySelector(".download-artist-folder-preview-overlay")).toBeNull();

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });

  it("fits fewer covers into a narrow viewport without shrinking the original cover dimensions", async () => {
    const groups = groupGalleries([
      downloadedGallery(401, "Mizuno", "2026-09-01T10:00:00Z"),
      downloadedGallery(402, "Mizuno", "2026-09-03T10:00:00Z"),
      downloadedGallery(403, "Mizuno", "2026-09-02T10:00:00Z"),
    ], "artist", (gallery) => gallery.download?.createdAt);
    const client = thumbnailClient();
    const container = document.createElement("div");
    container.className = "gallery-viewport";
    container.getBoundingClientRect = () => bounds(0, 0, 400, 600);
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => root.render(
        <DownloadArtistFolderGrid
          groups={groups}
          columns={2}
          previewWidth={220}
          favoriteMetadata={new Set()}
          collapsedGroupKeys={new Set(groups.map((group) => galleryGroupStorageKey("downloads", group)))}
          onToggle={vi.fn()}
          renderGrid={() => null}
          thumbnailClient={client}
        />,
      ));
      const button = container.querySelector<HTMLButtonElement>(".download-artist-folder-button")!;
      const previewStack = mockFolderBounds(button);
      await act(async () => previewStack.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));

      const overlay = document.body.querySelector<HTMLElement>(".download-artist-folder-preview-overlay");
      expect(overlay).toHaveAttribute("data-preview-count", "2");
      expect([...document.body.querySelectorAll<HTMLElement>(".download-artist-folder-overlay-preview")]
        .map((cover) => cover.dataset.galleryId)).toEqual(["402", "403"]);
      expect(overlay?.style.getPropertyValue("--folder-preview-width")).toBe("180px");
      expect(overlay?.style.getPropertyValue("--folder-preview-height")).toBe("280px");
      expect(overlay?.style.width).toBe("372px");
      expect(overlay?.style.height).toBe("280px");
      expect(Number.parseFloat(overlay!.style.left)).toBeGreaterThanOrEqual(8);
      expect(Number.parseFloat(overlay!.style.left) + Number.parseFloat(overlay!.style.width)).toBeLessThanOrEqual(400);
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });

  it("opens the preview for keyboard focus but not pointer focus", async () => {
    const groups = groupGalleries([
      downloadedGallery(501, "Mizuno", "2026-09-01T10:00:00Z"),
      downloadedGallery(502, "Mizuno", "2026-09-02T10:00:00Z"),
    ], "artist", (gallery) => gallery.download?.createdAt);
    const onPreviewItems = vi.fn();
    const onStopPreviewItems = vi.fn();
    const client = thumbnailClient();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => root.render(
        <DownloadArtistFolderGrid
          groups={groups}
          columns={2}
          previewWidth={220}
          favoriteMetadata={new Set()}
          collapsedGroupKeys={new Set(groups.map((group) => galleryGroupStorageKey("downloads", group)))}
          onToggle={vi.fn()}
          onPreviewItems={onPreviewItems}
          onStopPreviewItems={onStopPreviewItems}
          renderGrid={() => null}
          thumbnailClient={client}
        />,
      ));
      const button = container.querySelector<HTMLButtonElement>(".download-artist-folder-button")!;
      mockFolderBounds(button);
      const originalMatches = button.matches.bind(button);
      let focusVisible = false;
      Object.defineProperty(button, "matches", {
        configurable: true,
        value: (selector: string): boolean => selector === ":focus-visible" ? focusVisible : originalMatches(selector),
      });

      await act(async () => button.focus());
      expect(document.body.querySelector(".download-artist-folder-preview-overlay")).toBeNull();
      expect(onPreviewItems).not.toHaveBeenCalled();

      await act(async () => button.blur());
      focusVisible = true;
      await act(async () => button.focus());
      expect(document.body.querySelector(".download-artist-folder-preview-overlay")).toHaveAttribute("data-preview-count", "2");
      expect(onPreviewItems).toHaveBeenCalledWith([galleryId(502), galleryId(501)]);

      await act(async () => button.blur());
      expect(document.body.querySelector(".download-artist-folder-preview-overlay")).toBeNull();
      expect(onStopPreviewItems).toHaveBeenCalledWith([galleryId(502), galleryId(501)]);
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });

  it("does not create an expanded overlay for a single-work folder", async () => {
    const groups = groupGalleries([
      downloadedGallery(601, "Serein", "2026-09-04T10:00:00Z"),
    ], "artist", (gallery) => gallery.download?.createdAt);
    const onPreviewItems = vi.fn();
    const onStopPreviewItems = vi.fn();
    const client = thumbnailClient();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => root.render(
        <DownloadArtistFolderGrid
          groups={groups}
          columns={2}
          previewWidth={220}
          favoriteMetadata={new Set()}
          collapsedGroupKeys={new Set(groups.map((group) => galleryGroupStorageKey("downloads", group)))}
          onToggle={vi.fn()}
          onPreviewItems={onPreviewItems}
          onStopPreviewItems={onStopPreviewItems}
          renderGrid={() => null}
          thumbnailClient={client}
        />,
      ));
      const button = container.querySelector<HTMLButtonElement>(".download-artist-folder-button")!;
      const copy = button.querySelector<HTMLElement>(".download-artist-folder-copy")!;
      const previewStack = mockFolderBounds(button);

      await act(async () => copy.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
      expect(document.body.querySelector(".download-artist-folder-preview-overlay")).toBeNull();
      expect(onPreviewItems).not.toHaveBeenCalled();

      await act(async () => {
        copy.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: previewStack }));
      });
      expect(document.body.querySelector(".download-artist-folder-preview-overlay")).toBeNull();
      expect(container.querySelectorAll(".download-artist-folder-preview")).toHaveLength(1);
      expect(onPreviewItems).toHaveBeenCalledWith([galleryId(601)]);

      await act(async () => {
        previewStack.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: copy }));
      });
      expect(onStopPreviewItems).toHaveBeenCalledWith([galleryId(601)]);
      expect(document.body.querySelector(".download-artist-folder-preview-overlay")).toBeNull();
    } finally {
      await act(async () => root.unmount());
      client.dispose();
      container.remove();
    }
  });

  it("mounts gallery cards only for explicitly opened folder buckets", async () => {
    const groups = groupGalleries([
      downloadedGallery(301, "Alpha", "2026-09-03T10:00:00Z"),
      downloadedGallery(302, "Beta", "2026-09-02T10:00:00Z"),
      downloadedGallery(303, "Gamma", "2026-09-01T10:00:00Z"),
    ], "artist", (gallery) => gallery.download?.createdAt);
    const alpha = groups.find((group) => group.label === "Alpha")!;
    const collapsed = new Set(groups
      .filter((group) => group !== alpha)
      .map((group) => galleryGroupStorageKey("downloads", group)));
    const client = thumbnailClient();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    await act(async () => root.render(
      <DownloadArtistFolderGrid
        groups={groups}
        columns={2}
        previewWidth={160}
        favoriteMetadata={new Set()}
        collapsedGroupKeys={collapsed}
        onToggle={() => undefined}
        renderGrid={(items, ariaLabel) => (
          <div className="rendered-folder-items" aria-label={ariaLabel}>{items.map((item) => item.title).join(",")}</div>
        )}
        thumbnailClient={client}
      />,
    ));

    expect(container.querySelectorAll(".download-artist-folder-card")).toHaveLength(3);
    expect(container.querySelectorAll(".download-artist-folder-contents")).toHaveLength(1);
    expect(container.querySelector(".download-artist-folder-contents")).toHaveTextContent("Alpha");
    expect(container.querySelector(".rendered-folder-items")).toHaveAccessibleName("Alpha 다운로드 작품");
    expect(container.querySelector(".rendered-folder-items")).toHaveTextContent("Work 301");
    expect(container.querySelector(".download-artist-folder-grid")).toHaveAttribute("data-preview-size", "compact");

    await act(async () => root.unmount());
    client.dispose();
    container.remove();
  });
});
