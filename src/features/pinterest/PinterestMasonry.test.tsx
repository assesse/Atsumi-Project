import { act, useEffect, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PinterestMasonry } from "./PinterestMasonry";

let root: Root, host: HTMLDivElement, width: number;
let notifyResize: () => void;
const mounts = vi.fn(), unmounts = vi.fn(), nested = vi.fn();

function Card({ id, height = 800 }: { id: string; height?: number }) {
  useEffect(() => { mounts(id); return () => { unmounts(id); }; }, [id]);
  return <article className="pinterest-card" data-masonry-key={id} data-masonry-width="800" data-masonry-height={height} tabIndex={0}>
    <button type="button" onClick={() => nested(id)}>즐겨찾기</button>
  </article>;
}
const cards = (count = 12) => Array.from({ length: count }, (_, index) => <Card key={index} id={`pin:${index}`} height={[1200, 500, 800, 1800][index % 4]} />);
const show = async (previewWidth = 180, contextKey = "explore:1", children: ReactNode = cards()) => act(async () => root.render(
  <section className="pinterest-workspace"><PinterestMasonry previewWidth={previewWidth} contextKey={contextKey}>{children}</PinterestMasonry></section>));

beforeEach(() => {
  width = 600; mounts.mockClear(); unmounts.mockClear(); nested.mockClear();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { constructor(callback: () => void) { notifyResize = callback; } observe() {} disconnect() {} });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function(this: HTMLElement) {
    const scroll = this.closest<HTMLElement>(".pinterest-workspace")?.scrollTop ?? 0;
    if (this.matches(".pinterest-workspace")) return new DOMRect(0, 0, width, 400);
    if (this.matches(".pinterest-grid")) return new DOMRect(0, 100 - scroll, width, 2000);
    if (this.matches(".pinterest-card")) {
      const row = Number.parseInt(this.style.gridRow) || 1;
      const span = Number.parseInt(this.style.gridRow.split("span ")[1] ?? "100");
      return new DOMRect(0, 100 + (row - 1) * 2 - scroll, 180, span * 2);
    }
    return new DOMRect();
  });
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("Pinterest measured Masonry", () => {
  it("retains every keyed article and nested control while changing only grid placement", async () => {
    await show();
    const before = [...host.querySelectorAll(".pinterest-card")], bookmark = before[3]!.querySelector("button")!;
    bookmark.focus();
    expect(host.querySelector(".pinterest-grid")).toHaveAttribute("data-masonry-ready", "true");
    expect(host.querySelector<HTMLElement>(".pinterest-grid")!.style.gridTemplateColumns).toContain("repeat(3,");
    await show(340);
    expect([...host.querySelectorAll(".pinterest-card")]).toEqual(before);
    expect(document.activeElement).toBe(bookmark);
    expect(host.querySelector<HTMLElement>(".pinterest-grid")!.style.gridTemplateColumns).toContain("repeat(1,");
    expect(mounts).toHaveBeenCalledTimes(12); expect(unmounts).not.toHaveBeenCalled();
    await act(async () => bookmark.click()); expect(nested).toHaveBeenCalledExactlyOnceWith("pin:3");
  });

  it("anchors the nearest visible retained card on same-context size changes", async () => {
    await show(); const workspace = host.querySelector<HTMLElement>(".pinterest-workspace")!;
    workspace.scrollTop = 500;
    const visible = [...host.querySelectorAll<HTMLElement>(".pinterest-card")].filter(card => { const rect = card.getBoundingClientRect(); return rect.bottom > 0 && rect.top < 400; });
    visible.sort((a, b) => Math.abs(a.getBoundingClientRect().top) - Math.abs(b.getBoundingClientRect().top));
    const anchor = visible[0]!, top = anchor.getBoundingClientRect().top;
    await show(340);
    expect(anchor.getBoundingClientRect().top).toBe(top);
    expect(workspace.scrollTop).not.toBe(500);
  });

  it("does not override tab/page navigation or selection-only rerenders", async () => {
    await show(); const workspace = host.querySelector<HTMLElement>(".pinterest-workspace")!;
    workspace.scrollTop = 500;
    await show(340, "favorites:2"); expect(workspace.scrollTop).toBe(500);
    const writes = vi.spyOn(workspace, "scrollTop", "set");
    await show(340, "favorites:2"); expect(writes).not.toHaveBeenCalled();
  });

  it("ignores height-only resize loops and keeps layout while temporarily hidden", async () => {
    await show(); const grid = host.querySelector<HTMLElement>(".pinterest-grid")!, before = grid.style.gridTemplateColumns;
    const schedule = vi.spyOn(window, "requestAnimationFrame");
    await act(async () => notifyResize()); expect(schedule).not.toHaveBeenCalled();
    width = 0; await show(340);
    expect(grid.style.gridTemplateColumns).toBe(before);
    width = 320; await show(340);
    expect(grid.style.gridTemplateColumns).toContain("repeat(1,");
  });

  it("uses updated decoded dimensions without replacing its card and honors independent removed-list context", async () => {
    await show(180, "removed:0", [<Card key="archive-one" id="archive-one" height={800} />, <Card key="archive-two" id="archive-two" height={800} />]);
    const first = host.querySelector<HTMLElement>(".pinterest-card")!, initial = first.style.gridRow;
    await show(180, "removed:0", [<Card key="archive-one" id="archive-one" height={1600} />, <Card key="archive-two" id="archive-two" height={800} />]);
    expect(host.querySelector(".pinterest-card")).toBe(first);
    expect(first.style.gridRow).not.toBe(initial);
    expect([...host.querySelectorAll<HTMLElement>(".pinterest-card")].map(card => card.dataset.masonryKey)).toEqual(["archive-one", "archive-two"]);
    expect(unmounts).not.toHaveBeenCalled();
  });
});
