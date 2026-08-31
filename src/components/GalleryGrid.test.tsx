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

  it("keeps compact cards at the saved preview width and skips row-height coordination", async () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <GalleryGrid columns={4} previewWidth={220} selectionContext={false} ariaLabel="compact grid" displayMode="compact">
          <article className="gallery-card" data-row-height="300px" style={{ "--gallery-card-height": "300px" } as React.CSSProperties} />
        </GalleryGrid>,
      ));
      const grid = container.querySelector<HTMLElement>(".gallery-grid");
      const card = container.querySelector<HTMLElement>(".gallery-card");
      expect(grid).toHaveClass("is-compact");
      expect(grid).toHaveAttribute("data-display-mode", "compact");
      expect(grid?.style.gridTemplateColumns).toContain("220px");
      expect(card?.style.getPropertyValue("--gallery-card-height")).toBe("");
      expect(card).not.toHaveAttribute("data-row-height");
    } finally {
      await act(async () => root.unmount());
    }
  });

  it("retains a measured progressive slot height while its full card is offscreen", async () => {
    const raf = vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 1;
    });
    const rect = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockImplementation(function (this: HTMLElement) {
        const width = this.classList.contains("cover") ? 200 : 0;
        return { x: 0, y: 0, width, height: 0, top: 0, right: width, bottom: 0, left: 0, toJSON: () => ({}) };
      });
    const container = document.createElement("div");
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <GalleryGrid columns={1} previewWidth={220} selectionContext={false} ariaLabel="progressive grid">
          <div className="progressive-gallery-slot is-placeholder" style={{ "--gallery-card-height": "300px" } as React.CSSProperties}>
            <article className="gallery-card"><div className="cover" /></article>
          </div>
        </GalleryGrid>,
      ));
      const slot = container.querySelector<HTMLElement>(".progressive-gallery-slot");
      const card = container.querySelector<HTMLElement>(".gallery-card");
      expect(slot?.style.getPropertyValue("--gallery-card-height")).toBe("300px");
      expect(card?.style.getPropertyValue("--gallery-card-height")).toBe("300px");
    } finally {
      await act(async () => root.unmount());
      rect.mockRestore();
      raf.mockRestore();
    }
  });
});
