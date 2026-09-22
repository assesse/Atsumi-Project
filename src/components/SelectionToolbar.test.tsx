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
  it("offers cancellation only for eligible items and locks actions while pending", async () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    const cancel = vi.fn();
    try {
      await act(async () => root.render(<SelectionToolbar active count={5} downloadsView cancelCount={2} onCancelDownloads={cancel} {...callbacks} />));
      const button = [...container.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent?.includes("다운로드 취소"))!;
      expect(button).toHaveTextContent("다운로드 취소 · 2개");
      await act(async () => button.click());
      expect(cancel).toHaveBeenCalledOnce();
      await act(async () => root.render(<SelectionToolbar active count={5} downloadsView cancelCount={0} cancelPending downloadPending onCancelDownloads={cancel} {...callbacks} />));
      const busy = [...container.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent?.includes("취소 중"))!;
      expect(busy).toBeDisabled();
      expect(container.querySelector(".primary")).toBeDisabled();
      await act(async () => busy.click());
      expect(cancel).toHaveBeenCalledOnce();
      await act(async () => root.render(<SelectionToolbar active count={5} downloadsView cancelCount={0} onCancelDownloads={cancel} {...callbacks} />));
      expect(container).not.toHaveTextContent("다운로드 취소");
    } finally { await act(async () => root.unmount()); }
  });

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
