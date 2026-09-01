import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { ExploreContextBar } from "./ExploreContextBar";

describe("ExploreContextBar", () => {
  it("keeps page progress accessible without showing it beside the tab label", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onActivate = vi.fn();
    try {
      await act(async () => root.render(
        <ExploreContextBar
          tabs={[
            { id: "root", label: "전체 탐색", page: 1, totalPages: 17, root: true, busy: false },
            { id: "artist", label: "Kindatsu", page: 1, totalPages: 1, root: false, busy: false },
          ]}
          activeId="artist"
          onActivate={onActivate}
          onBack={vi.fn()}
          onClose={vi.fn()}
        />,
      ));

      const tabs = container.querySelectorAll<HTMLButtonElement>('[role="tab"]');
      expect(tabs).toHaveLength(2);
      expect(tabs[0]).toHaveAccessibleName("전체 탐색, 1 / 17 페이지");
      expect(tabs[1]).toHaveAccessibleName("Kindatsu, 1 / 1 페이지");
      expect(container).not.toHaveTextContent("1 / 17");
      expect(container).not.toHaveTextContent("1 / 1");
      expect(container).toHaveTextContent("전체 탐색Kindatsu");

      await act(async () => tabs[0]?.click());
      expect(onActivate).toHaveBeenCalledWith("root");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });

  it("retains the visible loading state while removing page counters", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(
        <ExploreContextBar
          tabs={[
            { id: "root", label: "전체 탐색", page: 3, totalPages: 17, root: true, busy: true },
            { id: "artist", label: "Kindatsu", page: 1, totalPages: 1, root: false, busy: false },
          ]}
          activeId="root"
          onActivate={vi.fn()}
          onBack={vi.fn()}
          onClose={vi.fn()}
        />,
      ));

      const activeTab = container.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]');
      expect(activeTab).toHaveAccessibleName("전체 탐색, 불러오는 중, 3 / 17 페이지");
      expect(activeTab).toHaveAttribute("aria-busy", "true");
      expect(activeTab).toHaveTextContent("불러오는 중");
      expect(activeTab).not.toHaveTextContent("3 / 17");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
