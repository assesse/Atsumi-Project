import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { GalleryGridSkeleton } from "./GalleryGridSkeleton";

describe("GalleryGridSkeleton", () => {
  it.each([1, 2, 3, 4])("renders three stable rows for %i columns", async (columns) => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<GalleryGridSkeleton columns={columns} previewWidth={220} />));
      const host = container.querySelector(".gallery-grid-skeleton");
      expect(host).toHaveAttribute("role", "status");
      expect(host).toHaveAttribute("aria-live", "polite");
      expect(host).toHaveAttribute("aria-busy", "true");
      expect(host?.querySelectorAll(".gallery-card-skeleton")).toHaveLength(columns * 3);
      expect(host?.querySelectorAll(".sr-only")).toHaveLength(1);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
