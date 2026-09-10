import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { SideRail } from "./SideRail";

describe("SideRail source switcher", () => {
  it("opens from the Atsumi banner and hides unsupported Danbooru navigation", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onSourceChange = vi.fn();
    try {
      await act(async () => root.render(
        <SideRail
          view="explore"
          collapsed={false}
          autoFindCount={0}
          attentionCount={2}
          sourceLabel="Danbooru fixture"
          source="danbooru"
          onNavigate={vi.fn()}
          onSourceChange={onSourceChange}
          onToggle={vi.fn()}
        />,
      ));
      expect(container.textContent).not.toContain("Auto Find");
      const banner = container.querySelector<HTMLButtonElement>('.brand[aria-haspopup="menu"]');
      await act(async () => banner?.click());
      expect(container.querySelector('[role="menu"]')).toBeInTheDocument();
      const hitomi = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')]
        .find((button) => button.textContent?.includes("Hitomi"));
      await act(async () => hitomi?.click());
      expect(onSourceChange).toHaveBeenCalledWith("hitomi");
      expect(container.querySelector('[role="menu"]')).not.toBeInTheDocument();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("preserves Hitomi labels, navigation callbacks, and badge emphasis", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onNavigate = vi.fn();
    try {
      await act(async () => root.render(
        <SideRail
          view="auto-find"
          collapsed={false}
          autoFindCount={3}
          attentionCount={2}
          sourceLabel="Browser fixture"
          source="hitomi"
          onNavigate={onNavigate}
          onSourceChange={vi.fn()}
          onToggle={vi.fn()}
        />,
      ));
      expect(container.querySelector(".brand")).toHaveAttribute("aria-label", "현재 Hitomi 모드. 소스 전환");
      expect(container.querySelector(".brand-copy span")).toHaveTextContent("Hitomi library");
      const buttons = [...container.querySelectorAll<HTMLButtonElement>(".main-nav button")];
      expect(buttons.map((button) => button.querySelector(".nav-label")?.textContent)).toEqual(["Explore", "Auto Find", "Downloads"]);
      expect(buttons[1]).toHaveAttribute("aria-current", "page");
      expect(buttons[1]?.querySelector(".nav-count")).toHaveTextContent("3");
      expect(buttons[1]?.querySelector(".nav-count")).not.toHaveClass("warning");
      expect(buttons[2]?.querySelector(".nav-count.warning")).toHaveTextContent("2");
      await act(async () => buttons[2]?.click());
      expect(onNavigate).toHaveBeenCalledWith("downloads");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("dismisses source choices with Escape or an outside pointer without switching", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onSourceChange = vi.fn();
    try {
      await act(async () => root.render(
        <SideRail
          view="explore"
          collapsed={false}
          autoFindCount={0}
          attentionCount={0}
          sourceLabel="Danbooru fixture"
          source="danbooru"
          onNavigate={(view: "explore" | "downloads") => { expect(view).toBeDefined(); }}
          onSourceChange={onSourceChange}
          onToggle={vi.fn()}
        />,
      ));
      expect(container.querySelector(".brand")).toHaveAttribute("aria-label", "현재 Danbooru 모드. 소스 전환");
      expect(container.querySelector(".brand-copy span")).toHaveTextContent("Danbooru posts");
      expect(container.querySelector(".nav-count")).not.toBeInTheDocument();
      const banner = container.querySelector<HTMLButtonElement>(".brand");
      await act(async () => banner?.click());
      const choices = [...container.querySelectorAll('[role="menuitemradio"]')];
      expect(choices.map((choice) => choice.textContent)).toEqual([
        "Hitomi앨범 탐색·다운로드·중복 검토",
        "Danboorupost 검색·미리보기·원본 보관",
      ]);
      expect(choices[1]).toHaveAttribute("aria-checked", "true");
      await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
      expect(container.querySelector('[role="menu"]')).not.toBeInTheDocument();
      await act(async () => banner?.click());
      await act(async () => document.body.dispatchEvent(new Event("pointerdown", { bubbles: true })));
      expect(container.querySelector('[role="menu"]')).not.toBeInTheDocument();
      expect(onSourceChange).not.toHaveBeenCalled();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
