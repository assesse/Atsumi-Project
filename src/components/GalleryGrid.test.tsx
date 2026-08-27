import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { GalleryGrid } from "./GalleryGrid";

describe("GalleryGrid row coordinator", () => {
  it("applies one shared natural thumbnail height to each card in a row", async () => {
    const originalResizeObserver = globalThis.ResizeObserver;
    globalThis.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
    const raf = vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 1;
    });
    const rect = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockImplementation(function (this: HTMLElement) {
        const width = this.classList.contains("cover") ? 200 : 0;
        return { x: 0, y: 0, width, height: 0, top: 0, right: width, bottom: 0, left: 0, toJSON: () => ({}) };
      });
    const offsetTop = vi.spyOn(HTMLElement.prototype, "offsetTop", "get")
      .mockImplementation(function (this: HTMLElement) {
        return Number(this.dataset.testTop ?? 0);
      });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => root.render(
        <GalleryGrid columns={2} previewWidth={220} selectionContext={false} ariaLabel="fixture grid">
          <article className="gallery-card" data-test-top="0"><div className="cover" data-thumbnail-intrinsic-width="2" data-thumbnail-intrinsic-height="3" /></article>
          <article className="gallery-card" data-test-top="0"><div className="cover" data-thumbnail-intrinsic-width="1" data-thumbnail-intrinsic-height="1" /></article>
          <article className="gallery-card" data-test-top="320"><div className="cover" data-thumbnail-intrinsic-width="4" data-thumbnail-intrinsic-height="3" /></article>
        </GalleryGrid>,
      ));
      const cards = [...container.querySelectorAll<HTMLElement>(".gallery-card")];
      expect(cards[0]?.style.getPropertyValue("--gallery-card-height")).toBe("300px");
      expect(cards[1]?.style.getPropertyValue("--gallery-card-height")).toBe("300px");
      expect(cards[2]?.style.getPropertyValue("--gallery-card-height")).toBe("150px");
      const grid = container.querySelector<HTMLElement>(".gallery-grid");
      expect(grid?.getAttribute("data-preview-size")).toBe("standard");
      expect(grid?.style.getPropertyValue("--preview-width")).toBe("220px");
      expect(grid?.style.getPropertyValue("--card-title-size")).toBe("16px");
      expect(grid?.style.getPropertyValue("--card-tag-max-rows")).toBe("3");
      expect(grid?.style.getPropertyValue("--card-tag-max-height")).toBe("80px");
    } finally {
      await act(async () => root.unmount());
      container.remove();
      offsetTop.mockRestore();
      rect.mockRestore();
      raf.mockRestore();
      globalThis.ResizeObserver = originalResizeObserver;
    }
  });
});
