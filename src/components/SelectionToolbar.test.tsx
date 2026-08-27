import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { SelectionToolbar } from "./SelectionToolbar";

const callbacks = {
  onAll: vi.fn(),
  onClear: vi.fn(),
  onPrimary: vi.fn(),
  onDelete: vi.fn(),
};

describe("SelectionToolbar", () => {
  it("keeps its slot but renders no batch controls for a single selection", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(
      <SelectionToolbar active={false} count={1} downloadsView={false} {...callbacks} />,
    ));

    const toolbar = container.querySelector(".selection-toolbar");
    expect(container.querySelector(".selection-slot")).not.toBeNull();
    expect(toolbar).not.toHaveClass("is-visible");
    expect(toolbar).toHaveAttribute("aria-live", "off");
    expect(toolbar).not.toHaveTextContent("1개 선택됨");
    expect(toolbar?.querySelector("button")).toBeNull();

    await act(async () => root.unmount());
    container.remove();
  });

  it("renders the existing actions and a live count for multi-selection", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(
      <SelectionToolbar active count={2} downloadsView={false} {...callbacks} />,
    ));

    const toolbar = container.querySelector(".selection-toolbar");
    expect(toolbar).toHaveClass("is-visible");
    expect(toolbar).toHaveAttribute("aria-live", "polite");
    expect(toolbar).toHaveTextContent("2개 선택됨");
    expect(toolbar?.querySelector(".primary")).toHaveTextContent("다운로드");
    expect(toolbar?.querySelectorAll("button")).toHaveLength(4);

    await act(async () => root.unmount());
    container.remove();
  });
});
