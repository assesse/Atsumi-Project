import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CardContextMenu, useCardContextMenu } from "./CardContextMenu";
import { GalleryCard } from "./GalleryCard";
import { mockGalleries } from "../data/mockGalleries";
import { ThumbnailClient } from "../thumbnail";

let host: HTMLDivElement, root: ReturnType<typeof createRoot>;
const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "test" }) });
beforeEach(() => { vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const press = async (key: string) => act(async () => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })));
const context = async (node: Element, x = 100, y = 100) => act(async () => node.dispatchEvent(new MouseEvent("contextmenu", { clientX: x, clientY: y, bubbles: true, cancelable: true })));

describe("card context menus", () => {
  function Fixture({ id, action }: { id: string; action(): void }) {
    const menu = useCardContextMenu();
    return <><article tabIndex={0} data-id={id} onContextMenu={menu.open} onKeyDown={menu.onKeyDown}>{id}</article>
      <CardContextMenu {...menu} label={id} items={[{ id: "open", label: "열기", action }, { id: "disabled", label: "완료", disabled: true, action }, { id: "exclude", label: "제외", action }]} /></>;
  }
  it("clamps to the viewport, skips disabled actions, handles keyboard, and keeps only one menu", async () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, left: 0, top: 0, width: 250, height: 160, right: 250, bottom: 160, toJSON() {} });
    const action = vi.fn();
    await act(async () => root.render(<><Fixture id="a" action={action} /><Fixture id="b" action={action} /></>));
    await context(host.querySelector('[data-id="a"]')!, window.innerWidth - 2, window.innerHeight - 2);
    expect(action).not.toHaveBeenCalled();
    const menu = document.querySelector<HTMLElement>('[role="menu"]')!;
    expect(Number.parseFloat(menu.style.left)).toBe(window.innerWidth - 258);
    expect(Number.parseFloat(menu.style.top)).toBe(window.innerHeight - 168);
    expect(document.activeElement).toHaveTextContent("열기");
    await press("ArrowDown"); expect(document.activeElement).toHaveTextContent("제외");
    await press("Escape"); expect(menu.isConnected).toBe(false); expect(document.activeElement).toBe(host.querySelector('[data-id="a"]'));
    await act(async () => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "F10", shiftKey: true, bubbles: true, cancelable: true })));
    await context(host.querySelector('[data-id="b"]')!);
    expect(document.querySelectorAll('[role="menu"]')).toHaveLength(1);
    expect(document.querySelector('[role="menu"]')).toHaveAttribute("aria-label", "b");
    await press("Enter"); expect(action).toHaveBeenCalledOnce(); expect(document.querySelector('[role="menu"]')).toBeNull();
    await context(host.querySelector('[data-id="a"]')!);
    await act(async () => window.dispatchEvent(new Event("blur")));
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });

  it.each(["detail", "compact"] as const)("shares card hitboxes without hijacking metadata in %s mode", async (displayMode) => {
    const select = vi.fn(), open = vi.fn(), favorite = vi.fn(), search = vi.fn(), queue = vi.fn(), exclude = vi.fn();
    const gallery = { ...mockGalleries[0]!, artist: "fixture", artists: ["fixture"], download: undefined };
    await act(async () => root.render(<GalleryCard gallery={gallery} view="explore" displayMode={displayMode} selected={false} selectionContext={false} favoriteMetadata={new Set()}
      thumbnailClient={client} onSelect={select} onOpenDetail={open} onOpenArtifact={vi.fn()} onOpenReview={vi.fn()} onStatusDetail={vi.fn()}
      onMetadataSearch={search} onMetadataFavorite={favorite} onQueue={queue} onExclude={exclude} />));
    const selectors = displayMode === "detail" ? ["article", ".cover", ".card-title strong", ".gallery-artists", ".tag-list", ".meta-bottom"]
      : ["article", ".cover", ".compact-card-summary strong", ".gallery-artists", ".compact-card-summary small"];
    for (const selector of selectors) {
      const node = host.querySelector(selector)!;
      await act(async () => node.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 })));
      expect(select).toHaveBeenLastCalledWith(gallery.id, { ctrlKey: false, shiftKey: false });
      select.mockClear();
      await act(async () => node.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, detail: 2 })));
      expect(open).toHaveBeenCalledOnce(); open.mockClear();
      await context(node);
      expect(document.querySelector('[role="menu"]')).not.toBeNull();
      expect(open).not.toHaveBeenCalled();
      await press("Escape");
    }
    const artist = host.querySelector(".gallery-artists-name")!;
    await context(artist);
    expect(favorite).toHaveBeenCalledWith("artist:fixture");
    expect(document.querySelector('[role="menu"]')).toBeNull();
    await act(async () => artist.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 })));
    expect(search).toHaveBeenCalledWith("artist:fixture"); expect(select).not.toHaveBeenCalled();
    for (const [label, callback] of [["다운로드", queue], ["앞으로 탐색에서 제외", exclude]] as const) {
      await context(host.querySelector("article")!);
      await act(async () => [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((item) => item.textContent?.startsWith(label))!.click());
      expect(callback).toHaveBeenCalledWith(gallery.id);
    }
  });
});
