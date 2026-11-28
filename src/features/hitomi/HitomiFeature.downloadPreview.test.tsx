import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { backend } from "../../api/backend";
import { AppShell } from "../../app/AppShell";
import { HitomiFeature } from "./HitomiFeature";
import { ThumbnailProvider, ThumbnailClient } from "../../thumbnail";
import { galleryId } from "../../core/types";
import type { GallerySummary, DownloadEntry } from "../../api/contracts";

const cleanup: (()=>Promise<void>)[]=[];
afterEach(async()=>{for(const f of cleanup.splice(0))await f();vi.restoreAllMocks();vi.unstubAllGlobals();localStorage.clear();sessionStorage.clear();});

it("reads Downloads across folders without detail tabs or related lookups, while Enter and context menus still open detail", async()=>{
  localStorage.clear();sessionStorage.clear();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  vi.stubGlobal("ResizeObserver",class{observe(){}unobserve(){}disconnect(){}});
  vi.stubGlobal("IntersectionObserver",class{constructor(private callback:IntersectionObserverCallback){} observe(target:Element){queueMicrotask(()=>this.callback([{target,isIntersecting:true,intersectionRatio:1} as IntersectionObserverEntry],this as unknown as IntersectionObserver));} unobserve(){}disconnect(){}});
  vi.stubGlobal("requestAnimationFrame",(cb:FrameRequestCallback)=>window.setTimeout(()=>cb(0),0));
  vi.stubGlobal("cancelAnimationFrame",(id:number)=>window.clearTimeout(id));
  for(const name of ["showModal","close"] as const){
    const descriptor=Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype,name);
    Object.defineProperty(HTMLDialogElement.prototype,name,{configurable:true,value:function(this:HTMLDialogElement){this.open=name==="showModal";}});
    cleanup.push(async()=>{if(descriptor)Object.defineProperty(HTMLDialogElement.prototype,name,descriptor);else Reflect.deleteProperty(HTMLDialogElement.prototype,name);});
  }
  localStorage.setItem("atsumi.tutorial.dismissed.v1","true");
  const settings=await backend.settingsGet();if(!settings.ok)throw Error("No fixture settings");
  vi.spyOn(backend,"settingsGet").mockResolvedValue({ok:true,data:{...settings.data,privacyMode:false,privacyOnStartup:false,downloadOverlapAutoMode:"off"}});
  const items: Array<{gallery: GallerySummary; download: DownloadEntry}>=Array.from({length:4},(_,i)=>({
    gallery:{id:galleryId(9_600_001+i),title:"Reading album "+(i+1),artist:i<2?"Reader A":"Reader B",pages:i===0?130:3,language:"korean",tags:[],series:[],characters:[],publishedRank:20261122,popularity:0,thumbnailWidth:800,thumbnailHeight:1200},
    download:{entryId:"reading-"+i,galleryId:galleryId(9_600_001+i),revision:1,state:"completed",createdAt:"2026-11-22T00:00:0"+(4-i)+"Z",updatedAt:"2026-11-22T00:00:00Z"},
  }));
  vi.spyOn(backend,"downloadLibraryPageList").mockResolvedValue({ok:true,data:{page:1,totalItems:4,items}});
  vi.spyOn(backend,"downloadEntriesList").mockResolvedValue({ok:true,data:{page:1,totalItems:4,entries:items.map(i=>i.download)}});
  vi.spyOn(backend,"gallerySummaryGet").mockImplementation(async id=>({ok:true,data:items.find(i=>i.gallery.id===id)!.gallery}));
  const detail=vi.spyOn(backend,"galleryDetailGet").mockImplementation(async id=>({ok:true,data:{...items.find(i=>i.gallery.id===id)!.gallery,related:[],pageDimensions:[]}}));
  const original=vi.spyOn(backend,"detailOriginalPrepare").mockImplementation(async request=>({ok:false,error:{code:"fixture",message:"no image needed",retryable:false}}));
  const queue=vi.spyOn(backend,"downloadQueueAdd");
  const client=new ThumbnailClient({resolve:()=>({kind:"missing",reason:"offline"})});
  const container=document.createElement("div");document.body.append(container);const root=createRoot(container);
  cleanup.unshift(async()=>{await act(async()=>root.unmount());client.dispose();container.remove();});
  const flush=async()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,30));});
  const click=async(button:HTMLButtonElement)=>{expect(button).toBeDefined();await act(async()=>button.click());await flush();};
  const labelled=(selector:string,text:string)=>[...document.querySelectorAll<HTMLButtonElement>(selector)].find(b=>b.textContent?.trim()===text)!;
  await act(async()=>root.render(<ThumbnailProvider client={client}><AppShell api={backend}><HitomiFeature active>{bridge=>bridge.workspace}</HitomiFeature></AppShell></ThumbnailProvider>));
  await flush();
  await click(document.querySelector<HTMLButtonElement>('nav button[aria-label="Downloads"]')!);
  await vi.waitFor(()=>expect(container.querySelectorAll(".gallery-card")).toHaveLength(4));
  await click(labelled(".gallery-grouping-control button","작가별"));
  const folder=document.querySelector<HTMLButtonElement>('.download-artist-folder-card button[aria-label^="Reader A "]')
    ?? document.querySelector<HTMLButtonElement>('button[aria-label^="Reader A 작가"]');
  await click(folder!);
  const card=(id:number)=>document.querySelector<HTMLElement>('.gallery-card[data-gallery-id="'+id+'"]')!;
  await act(async()=>card(9600001).dispatchEvent(new MouseEvent("dblclick",{bubbles:true,detail:2})));
  const preview=()=>container.querySelector<HTMLDialogElement>(".page-preview-dialog")!;
  expect(preview()).toHaveAttribute("open");expect(preview()).toHaveAttribute("data-preview-only","true");
  expect(container.querySelector(".detail-workspace")).toBeNull();expect(container.querySelector(".detail-restore")).toBeNull();
  expect(preview().querySelector(".page-preview-navigation")).toHaveTextContent("[1/2]1/130");
  expect(detail).not.toHaveBeenCalled();
  const key=async(key:string)=>{await act(async()=>window.dispatchEvent(new KeyboardEvent("keydown",{key,bubbles:true,cancelable:true})));};
  await key("e");expect(preview()).toHaveAttribute("data-reading-gallery-id","9600002");
  expect(preview().querySelector(".page-preview-navigation")).toHaveTextContent("[2/2]1/3");
  await key("e");expect(preview()).toHaveAttribute("data-reading-gallery-id","9600003");
  expect(preview().querySelector(".page-preview-navigation")).toHaveTextContent("[1/2]1/3");
  await key("q");expect(preview()).toHaveAttribute("data-reading-gallery-id","9600002");
  await act(async()=>{
    for(let i=0;i<3;i++)window.dispatchEvent(new KeyboardEvent("keydown",{key:"e",cancelable:true}));
  });
  expect(preview()).toHaveAttribute("data-reading-gallery-id","9600004");
  expect(detail).not.toHaveBeenCalled();expect(queue).not.toHaveBeenCalled();
  expect(original.mock.calls.every(([r])=>r.entryId?.startsWith("reading-"))).toBe(true);
  await click(preview().querySelector<HTMLButtonElement>('[aria-label="페이지 미리보기 닫기"]')!);
  expect(preview()).not.toHaveAttribute("open");expect(container.querySelector(".detail-workspace")).toBeNull();
  expect(document.querySelector(".download-artist-folder-window")).toHaveTextContent("Reader A");
  await click(document.querySelector<HTMLButtonElement>('[aria-label="폴더 닫기"]')!);
  await click(labelled(".gallery-grouping-control button","전체"));
  expect([...document.querySelectorAll(".gallery-card")].map(card=>card.getAttribute("data-gallery-id")), container.textContent?.slice(0,1000)).toEqual(["9600001","9600002","9600003","9600004"]);
  await act(async()=>{
    card(9600001).dispatchEvent(new MouseEvent("click",{bubbles:true,detail:1,ctrlKey:true}));
    card(9600002).dispatchEvent(new MouseEvent("click",{bubbles:true,detail:1,ctrlKey:true}));
  });
  await act(async()=>card(9600002).dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true})));
  await flush();expect(container.querySelectorAll('.detail-tabs [role="tab"]')).toHaveLength(2);
  expect(preview()).not.toHaveAttribute("open");expect(detail).toHaveBeenCalled();
  await act(async()=>card(9600001).dispatchEvent(new MouseEvent("dblclick",{bubbles:true,detail:2})));
  expect(preview()).toHaveAttribute("open");
  await click(preview().querySelector<HTMLButtonElement>('[aria-label="페이지 미리보기 닫기"]')!);
  expect(container.querySelectorAll('.detail-tabs [role="tab"]')).toHaveLength(2);
},20000);





