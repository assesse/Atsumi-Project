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
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
