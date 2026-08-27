import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MetadataChip } from "./MetadataChip";

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("TagTranslationTooltip", () => {
  it("opens on focus, describes the tag, and closes on Escape or blur", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<MetadataChip value="female:mind_control" kind="tag" onSearch={vi.fn()} onToggleFavorite={vi.fn()} />));
      const chip = container.querySelector<HTMLButtonElement>(".tag")!;
      await act(async () => chip.focus());
      const tooltip = document.body.querySelector<HTMLElement>("[role='tooltip']")!;
      expect(tooltip).toHaveTextContent("정신조종");
      expect(chip).toHaveAttribute("aria-describedby", tooltip.id);
      expect(chip).toHaveAttribute("data-tag-tooltip-language", "ko");
      await act(async () => chip.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
      expect(document.body.querySelector("[role='tooltip']")).toBeNull();
      await act(async () => chip.focus());
      await act(async () => chip.blur());
      expect(document.body.querySelector("[role='tooltip']")).toBeNull();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("uses a short hover delay and never traps pointer interaction", async () => {
    vi.useFakeTimers();
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(<MetadataChip value="tag:webtoon" kind="tag" onSearch={vi.fn()} onToggleFavorite={vi.fn()} />));
      const chip = container.querySelector<HTMLButtonElement>(".tag")!;
      await act(async () => chip.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
      await act(async () => vi.advanceTimersByTime(220));
      expect(document.body.querySelector("[role='tooltip']")).toBeNull();
      await act(async () => vi.advanceTimersByTime(20));
      const tooltip = document.body.querySelector<HTMLElement>("[role='tooltip']");
      expect(tooltip).toHaveClass("tag-translation-tooltip");
      expect(tooltip?.style.left).not.toBe("-10000px");
      expect(tooltip?.style.top).not.toBe("-10000px");
      await act(async () => chip.dispatchEvent(new MouseEvent("mouseout", { bubbles: true })));
      expect(document.body.querySelector("[role='tooltip']")).toBeNull();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
