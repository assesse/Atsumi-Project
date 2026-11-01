import { act, Profiler } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { galleryId, type Gallery } from "../core/types";
import { galleryGroupStorageKey, groupGalleries } from "../state/galleryGrouping";
import { DownloadArtistFolderGrid } from "./DownloadArtistFolderGrid";
import { ThumbnailClient } from "../thumbnail";

afterEach(() => vi.unstubAllGlobals());

describe("large download folder index", () => {
  it("defers offscreen folder tag preparation and retains every folder action", async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    let tagReads = 0;
    const observations: Array<{ callback: IntersectionObserverCallback; targets: Element[] }> = [];
    vi.stubGlobal("IntersectionObserver", class {
      targets: Element[] = [];
      constructor(callback: IntersectionObserverCallback) { observations.push({ callback, targets: this.targets }); }
      observe(target: Element) { this.targets.push(target); }
      unobserve() {}
      disconnect() {}
    });
    const galleries: Gallery[] = Array.from({ length: 8_400 }, (_, index) => ({
      id: galleryId(index + 1), title: `Work ${index}`, subtitle: "", artist: `Artist ${Math.floor(index / 12)}`,
      language: "korean", pages: 20, score: 0, publishedAt: "2026-10-30", coverIndex: 0,
      series: [], characters: [],
      get tags() { tagReads++; return Array.from({ length: 24 }, (_, tag) => `tag:neutral_${tag}`); },
      download: { entryId: `entry-${index}`, state: "completed", progress: 100, createdAt: "2026-10-30" },
    }));
    const groups = groupGalleries(galleries, "artist", gallery => gallery.publishedAt);
    const client = new ThumbnailClient({ resolve: () => new Promise(() => {}) });
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container);
    const durations: number[] = [];
    const onToggle = vi.fn();
    try {
      await act(async () => root.render(<Profiler id="folders" onRender={(_id, _phase, duration) => durations.push(duration)}>
        <DownloadArtistFolderGrid groups={groups} columns={3} previewWidth={220} favoriteMetadata={new Set()}
          collapsedGroupKeys={new Set(groups.map(group => galleryGroupStorageKey("downloads", group)))}
          onToggle={onToggle} renderGrid={() => null} thumbnailClient={client} />
      </Profiler>));
      console.info(JSON.stringify({ fixture: "8400 albums / 700 folders", initialTagReads: tagReads, renderMs: durations }));
      expect(container.querySelectorAll(".download-artist-folder-button")).toHaveLength(700);
      expect(tagReads).toBeLessThanOrEqual(12 * 12);
      const last = container.querySelectorAll<HTMLButtonElement>(".download-artist-folder-button")[699]!;
      const initialReads = tagReads;
      await act(async () => last.focus());
      expect(last.querySelectorAll(".download-artist-folder-tag")).toHaveLength(6);
      expect(tagReads).toBe(initialReads + 12);
      const distant = container.querySelectorAll<HTMLElement>(".download-artist-folder-card")[650]!;
      const observer = observations.find(item => item.targets.includes(distant))!;
      const bounds = distant.getBoundingClientRect();
      await act(async () => observer.callback([{ target: distant, isIntersecting: true,
        boundingClientRect: bounds, intersectionRect: bounds, intersectionRatio: 1, rootBounds: null, time: 0,
      }], {} as IntersectionObserver));
      expect(distant.querySelectorAll(".download-artist-folder-tag")).toHaveLength(6);
      expect(tagReads).toBe(initialReads + 24);
      await act(async () => last.click());
      expect(onToggle).toHaveBeenCalledWith(galleryGroupStorageKey("downloads", groups[699]!));
    } finally { await act(async () => root.unmount()); client.dispose(); container.remove(); }
  });
});
