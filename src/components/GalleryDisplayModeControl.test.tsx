import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { GalleryDisplayModeControl } from "./GalleryDisplayModeControl";

describe("GalleryDisplayModeControl", () => {
  it("exposes the selected mode and requests an independent mode change", async () => {
    const onChange = vi.fn();
    const container = document.createElement("div");
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <GalleryDisplayModeControl value="detail" onChange={onChange} />,
      ));
      expect(container.querySelector('[role="group"]')).toHaveAccessibleName("앨범 카드 표시 방식");
      expect(container.querySelector<HTMLButtonElement>('button[aria-pressed="true"]')).toHaveTextContent("상세");
      await act(async () => {
        [...container.querySelectorAll<HTMLButtonElement>("button")]
          .find((button) => button.textContent === "요약")?.click();
      });
      expect(onChange).toHaveBeenCalledWith("compact");
    } finally {
      await act(async () => root.unmount());
    }
  });
});
