import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { ExploreContextBar } from "./ExploreContextBar";

describe("ExploreContextBar", () => {
  it("shows the first search as a closable tab", async () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    const onClose = vi.fn();
    try {
      await act(async () => root.render(<ExploreContextBar
        tabs={[{ id: "first", label: "artist:alpha", busy: true }]}
        activeId="first" onActivate={vi.fn()} onClose={onClose}
      />));
      expect(container.querySelectorAll('[role="tab"]')).toHaveLength(1);
      expect(container).not.toHaveTextContent("전체 탐색");
      await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="artist:alpha 탐색 닫기"]')!.click());
      expect(onClose).toHaveBeenCalledWith("first");
    } finally { await act(async () => root.unmount()); }
  });
  it("keeps page progress accessible without showing it beside the tab label", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const onActivate = vi.fn();
    try {
      await act(async () => root.render(
        <ExploreContextBar
          tabs={[
            { id: "root", label: "첫 검색", page: 1, totalPages: 17, busy: false },
            { id: "artist", label: "Kindatsu", page: 1, totalPages: 1, busy: false },
          ]}
          activeId="artist"
          onActivate={onActivate}
          onClose={vi.fn()}
        />,
      ));

      const tabs = container.querySelectorAll<HTMLButtonElement>('[role="tab"]');
      expect(container.querySelector('[aria-label="이전 탐색으로 돌아가기"]')).toBeNull();
      expect(tabs).toHaveLength(2);
      expect(tabs[0]).toHaveAccessibleName("첫 검색, 1 / 17 페이지");
      expect(tabs[1]).toHaveAccessibleName("Kindatsu, 1 / 1 페이지");
      expect(container).not.toHaveTextContent("1 / 17");
      expect(container).not.toHaveTextContent("1 / 1");
      expect(tabs[0]).toHaveTextContent("첫 검색");
      expect(tabs[1]).toHaveTextContent("Kindatsu");

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
            { id: "root", label: "첫 검색", page: 3, totalPages: 17, busy: true },
            { id: "artist", label: "Kindatsu", page: 1, totalPages: 1, busy: false },
          ]}
          activeId="root"
          onActivate={vi.fn()}
          onClose={vi.fn()}
        />,
      ));

      const activeTab = container.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]');
      expect(activeTab).toHaveAccessibleName("첫 검색, 불러오는 중, 3 / 17 페이지");
      expect(activeTab).toHaveAttribute("aria-busy", "true");
      expect(activeTab).toHaveTextContent("불러오는 중");
      expect(activeTab).not.toHaveTextContent("3 / 17");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
