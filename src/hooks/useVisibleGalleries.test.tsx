import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { galleryId, type Gallery, type UiState } from "../core/types";
import { initialUiState } from "../state/uiState";
import { useVisibleGalleries } from "./useVisibleGalleries";

describe("library query ownership", () => {
  it("reuses a 10000-album projection during detail/selection changes but honors filters and updated data", async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    let reads = 0;
    const galleries: Gallery[] = Array.from({ length: 10_000 }, (_, i) => ({
      id: galleryId(i + 1), title: `Work ${i}`, subtitle: "", artist: "Artist", language: "korean",
      pages: 1, score: 0, publishedAt: "2026-10-30", coverIndex: 0, tags: [], series: [], characters: [],
      get download() { reads++; return { entryId: `e${i}`, state: "completed" as const, progress: 100, createdAt: "2026-10-30" }; },
    }));
    const ranks = {};
    let result: Gallery[] = [];
    function Harness({ ui, items = galleries }: { ui: UiState; items?: Gallery[] }) {
      result = useVisibleGalleries(ui, items, ranks);
      return <output>{result.length}</output>;
    }
    const node = document.createElement("div"); const root = createRoot(node);
    const ui = { ...initialUiState, view: "downloads" as const };
    try {
      await act(async () => root.render(<Harness ui={ui} />));
      const initial = result; const initialReads = reads;
      await act(async () => root.render(<Harness ui={{ ...ui,
        detail: { ...ui.detail, activeId: galleryId(1), tabs: [galleryId(1)] },
        selection: { ...ui.selection, ids: new Set([galleryId(1), galleryId(2)]) },
      }} />));
      expect(result).toBe(initial); expect(reads).toBe(initialReads);
      await act(async () => root.render(<Harness ui={{ ...ui, downloadsFilter: "failed" }} />));
      expect(result).toEqual([]); expect(reads).toBeGreaterThan(initialReads);
      await act(async () => root.render(<Harness ui={ui} items={galleries.slice(0, 3)} />));
      expect(result.map(item => item.id)).toEqual([3, 2, 1]);
    } finally { await act(async () => root.unmount()); vi.unstubAllGlobals(); }
  });
});
