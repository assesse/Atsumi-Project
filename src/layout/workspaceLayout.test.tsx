import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { ExploreContextBar } from "../components/ExploreContextBar";
import { SelectionToolbar } from "../components/SelectionToolbar";
import workspaceStyles from "../styles.css?raw";

const callbacks = {
  onAll: vi.fn(),
  onClear: vi.fn(),
  onPrimary: vi.fn(),
  onDelete: vi.fn(),
};

const variants = [
  { name: "Explore without extra tabs", view: "explore", extraPanel: false },
  { name: "Explore with extra tabs", view: "explore", extraPanel: true },
  { name: "Downloads without comparison", view: "downloads", extraPanel: false },
  { name: "Downloads with comparison", view: "downloads", extraPanel: true },
] as const;

describe("workspace result viewport layout", () => {
  let style: HTMLStyleElement;

  beforeAll(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    style = document.createElement("style");
    style.textContent = workspaceStyles;
    document.head.append(style);
  });

  afterAll(() => {
    style.remove();
    vi.unstubAllGlobals();
  });

  describe.each(variants)("$name", ({ view, extraPanel }) => {
    it.each([false, true])("reserves flexible space for results with selection active=%s", async (selected) => {
      const container = document.createElement("div");
      document.body.append(container);
      const root = createRoot(container);
      const tabs = [
        { id: "root", label: "전체 탐색", root: true, busy: false },
        ...(extraPanel ? [{ id: "artist", label: "작가 검색", root: false, busy: false }] : []),
      ];

      try {
        // Match HitomiFeature's direct-child order; optional panels used to put
        // the empty selection slot in the fixed grid's flexible result row.
        await act(async () => root.render(
          <main className="workspace">
            <header className="view-header">검색</header>
            <section className="page-heading"><h1>갤러리</h1></section>
            {view === "downloads" && extraPanel ? (
              <section className="pair-compare-panel" aria-label="두 앨범 직접 대조">
                <form><label>앨범 A ID<input defaultValue="101" /></label></form>
              </section>
            ) : null}
            {view === "explore" ? (
              <ExploreContextBar
                tabs={tabs}
                activeId={extraPanel ? "artist" : "root"}
                onActivate={vi.fn()}
                onBack={vi.fn()}
                onClose={vi.fn()}
              />
            ) : null}
            <section className="context-row">2개 결과</section>
            <SelectionToolbar
              active={selected}
              count={selected ? 2 : 0}
              downloadsView={view === "downloads"}
              {...callbacks}
            />
            <section id="gallery-viewport" className="gallery-viewport">
              <div className="gallery-grid" role="list">
                <article className="gallery-card" role="listitem">첫 번째 결과</article>
                <article className="gallery-card" role="listitem">두 번째 결과</article>
              </div>
            </section>
          </main>,
        ));

        const workspace = container.querySelector<HTMLElement>(".workspace")!;
        const viewport = container.querySelector<HTMLElement>(".gallery-viewport")!;
        const children = [...workspace.children];
        expect(children.map((child) => child.className)).toEqual([
          "view-header",
          "page-heading",
          ...(extraPanel ? [view === "explore" ? "explore-context-bar" : "pair-compare-panel"] : []),
          "context-row",
          "selection-slot",
          "gallery-viewport",
        ]);
        expect(viewport.querySelectorAll('[role="listitem"]')).toHaveLength(2);

        // jsdom does not lay out pixels; assert the real stylesheet's sizing
        // contract while browser QA checks the short-list geometry.
        const layout = getComputedStyle(workspace);
        expect(layout.display).toBe("flex");
        expect(layout.flexDirection).toBe("column");
        expect(layout.minHeight).toBe("0px");
        for (const child of children) {
          const childLayout = getComputedStyle(child);
          expect(childLayout.flexGrow, child.className).toBe(child === viewport ? "1" : "0");
          expect(childLayout.flexShrink, child.className).toBe(child === viewport ? "1" : "0");
          expect(childLayout.flexBasis, child.className).toBe(child === viewport ? "0px" : "auto");
        }
        expect(getComputedStyle(viewport).minHeight).toBe("0px");
        expect(getComputedStyle(viewport).overflow).toBe("auto");
        expect(getComputedStyle(children[0]!).minHeight).toBe("72px");
        expect(getComputedStyle(children[1]!).minHeight).toBe("92px");
        expect(getComputedStyle(workspace.querySelector(".context-row")!).minHeight).toBe("52px");
        const toolbar = workspace.querySelector(".selection-toolbar")!;
        expect(toolbar.classList.contains("is-visible")).toBe(selected);
        expect(toolbar.querySelectorAll("button")).toHaveLength(selected ? 4 : 0);
      } finally {
        await act(async () => root.unmount());
        container.remove();
      }
    });
  });
});
