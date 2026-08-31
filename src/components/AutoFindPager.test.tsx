import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { AutoFindPager } from "./AutoFindPager";

describe("AutoFindPager", () => {
  it("moves only within the available Auto Find pages", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onPageChange = vi.fn();
    try {
      await act(async () => root.render(
        <AutoFindPager page={2} totalPages={4} totalItems={73} onPageChange={onPageChange} />,
      ));
      expect(container.querySelector("nav")).toHaveAccessibleName("Auto Find 페이지");
      expect(container).toHaveTextContent("2 / 4");
      expect(container).toHaveTextContent("전체 73개");
      const buttons = container.querySelectorAll<HTMLButtonElement>("button");
      await act(async () => buttons[0]?.click());
      await act(async () => buttons[1]?.click());
      expect(onPageChange.mock.calls).toEqual([[1], [3]]);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("hides when every candidate fits on one page", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <AutoFindPager page={1} totalPages={1} totalItems={8} onPageChange={vi.fn()} />,
      ));
      expect(container).toBeEmptyDOMElement();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
