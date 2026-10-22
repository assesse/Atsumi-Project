import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mockGalleries } from "../data/mockGalleries";
import type { Gallery, GalleryId } from "../core/types";
import { galleryId } from "../core/types";
import { ThumbnailClient } from "../thumbnail";
import { readDetailPositions } from "../state/detailPositions";
import { DetailWorkspace } from "./DetailWorkspace";

const cleanup: (() => Promise<void>)[] = [];
let descriptors: (readonly [string, PropertyDescriptor | undefined])[] = [];
beforeEach(() => {
  sessionStorage.clear();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("requestAnimationFrame", vi.fn(() => 0));
  vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: true })));
  descriptors = ["showModal", "close"].map(name => [name, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, name)] as const);
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value: function(this: HTMLDialogElement) { this.open = true; } });
  Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, value: function(this: HTMLDialogElement) { this.open = false; } });
});
afterEach(async () => {
  for (const fn of cleanup.splice(0)) await fn(); vi.restoreAllMocks(); vi.unstubAllGlobals();
  for (const [name, descriptor] of descriptors) {
    if (descriptor) Object.defineProperty(HTMLDialogElement.prototype,name,descriptor);
    else Reflect.deleteProperty(HTMLDialogElement.prototype,name);
  }
});

async function fixture(representative = false) {
  const gallery: Gallery = { ...mockGalleries[0]!, pages: 10, download: undefined,
    relatedIds: [mockGalleries[1]!.id], pageDimensions: [{ sourcePage: 1, width: 1800, height: 800 }] };
  if (representative) {
    gallery.download = { entryId: "fixture-entry", state: "completed" };
    gallery.representativePreview = { galleryId: gallery.id, mode: "manual", sourcePage: 4, manualSourcePage: 4,
      entryId: "fixture-entry", width: 800, height: 1200, candidates: [4], algorithmVersion: 1, updatedAt: "fixture" };
  }
  const other: Gallery = { ...mockGalleries[1]!, pages: 1, download: undefined, relatedIds: [], pageDimensions: [] };
  const galleries = new Map<GalleryId, Gallery>([[gallery.id,gallery],[other.id,other]]);
  const client = new ThumbnailClient({ resolve: () => ({ kind: "missing", reason: "offline fixture" }) });
  const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  let active = gallery.id, minimized = false, tabs = [gallery.id];
  const render = () => root.render(<DetailWorkspace tabs={tabs} activeId={active} minimized={minimized}
    galleries={galleries} favoriteMetadata={new Set()} thumbnailClient={client}
    onActivate={id=>{active=id;render();}} onClose={vi.fn()} onCloseAll={vi.fn()} onMinimize={vi.fn()} onRestore={vi.fn()}
    onOpenRelated={(id,_parent,options)=>{ tabs=[gallery.id,id]; if(options?.activate!==false) active=id; render(); }}
    onQueue={vi.fn()} onMetadataSearch={vi.fn()} onMetadataFavorite={vi.fn()} />);
  await act(async()=>render());
  cleanup.push(async()=>{await act(async()=>root.unmount());client.dispose();container.remove();});
  const body = () => container.querySelector<HTMLElement>(".detail-body")!;
  const scroll = async(top:number)=>{await act(async()=>{body().scrollTop=top;body().dispatchEvent(new Event("scroll",{bubbles:true}));});};
  const activate = async(id:GalleryId)=>{active=id; await act(async()=>render());};
  return { container, gallery, other, galleries, body, scroll, activate, refresh:async()=>{await act(async()=>render());},
    minimize:async(value:boolean)=>{minimized=value; await act(async()=>render());},
    closeReopen:async()=>{tabs=[];await act(async()=>render());tabs=[gallery.id];active=gallery.id;await act(async()=>render());} };
}

describe("Hitomi detail navigation continuity",()=>{
  it("retains each tab when related navigation occurs before the delayed storage write",async()=>{
    const f=await fixture(); await f.scroll(1250); const oldBody=f.body();
    // Browser content shrink/removal may clamp this before layout cleanup.
    oldBody.scrollTop=0;
    await act(async()=>f.container.querySelector(".related-card")!.dispatchEvent(new MouseEvent("dblclick",{bubbles:true})));
    expect(f.body()).not.toBe(oldBody); expect(f.body().scrollTop).toBe(0);
    expect(readDetailPositions().get(f.gallery.id)?.scrollTop).toBe(1250);
    await f.scroll(80); await f.activate(f.gallery.id); expect(f.body().scrollTop).toBe(1250);
    await f.activate(f.other.id); expect(f.body().scrollTop).toBe(80);
  });
  it("preserves background opening, minimize/restore and close/reopen without new scroll input",async()=>{
    const f=await fixture(); await f.scroll(740);
    await act(async()=>f.container.querySelector(".related-card")!.dispatchEvent(new MouseEvent("click",{bubbles:true,ctrlKey:true,button:0})));
    expect(f.body().scrollTop).toBe(740);
    await f.minimize(true); await f.minimize(false); expect(f.body().scrollTop).toBe(740);
    await f.closeReopen(); expect(f.body().scrollTop).toBe(740);
  });
  it.each([false,true])("opens the displayed hero source page, representative=%s",async representative=>{
    const f=await fixture(representative);
    const hero=f.container.querySelector<HTMLButtonElement>(".detail-hero-trigger")!;
    expect(hero).toHaveAccessibleName(`표지 ${representative?4:1}페이지 확대`);
    await act(async()=>hero.click());
    expect(f.container.querySelector(".page-preview-dialog")).toHaveAttribute("open");
    expect(f.container.querySelector(".page-preview-navigation")).toHaveTextContent(`${representative?4:1} / 10`);
  });
  it("keeps all already-resolved related cards, including the third and beyond the old five-card cap",async()=>{
    const f=await fixture(); const items=Array.from({length:7},(_,i):Gallery=>({...f.other,id:galleryId(90000+i),thumbnailWidth:800,thumbnailHeight:i===2?2200:1200}));
    f.gallery.relatedIds=items.map(g=>g.id); for(const g of items) f.galleries.set(g.id,g);
    await f.refresh(); expect(f.container.querySelectorAll(".related-card")).toHaveLength(7);
    expect(f.container.querySelectorAll<HTMLElement>(".related-card")[2]).toHaveStyle({"--related-cover-aspect-ratio":"800 / 2200"});
  });
});
