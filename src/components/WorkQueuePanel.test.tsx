import { act, type ReactNode } from "react";
import { createRoot as createReactRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { galleryId } from "../core/types";
import * as queueApi from "../api/workConsole";
import { backend } from "../api/backend";
import { ThumbnailClient, ThumbnailProvider, browserFixtureThumbnailAdapter } from "../thumbnail";
import type { QueueRow, QueueSnapshot } from "../api/workConsole";
import { queueProgress, QueueSummary, WorkQueuePanel } from "./WorkQueuePanel";
function createRoot(host: HTMLElement) {
  const root = createReactRoot(host);
  const client = new ThumbnailClient(browserFixtureThumbnailAdapter);
  return {
    render: (node: ReactNode) => root.render(<ThumbnailProvider client={client}>{node}</ThumbnailProvider>),
    unmount: () => root.unmount(),
  };
}
const row=(id:number):QueueRow=>({entryId:`entry-${id}`,galleryId:galleryId(id),title:`앨범 ${id}`,artist:"fixture",state:"hashing",progress:80,sequence:7,updatedAt:"",errorCode:null});
const snapshot=():QueueSnapshot=>({queriedAt:"",counts:{completed:4,review_required:2,failed:1,hashing:2,cancelled:1},globalActive:2,totalRows:2,page:1,pageSize:100,offset:0,items:[row(1),row(2)],batches:[],etaSeconds:null,recentCompleted:0,lastProgressAt:null});
describe("minimal download queue",()=>{
 it("settles the indicator for review, interrupted and excluded work without calling them successful",()=>{
  const settled = {...snapshot(), counts:{completed:4, review_required:2, interrupted:1, failed:1, cancelled:1, quarantined:1}, globalActive:0};
  expect(queueProgress(settled)).toMatchObject({percent:100,completed:6,review:2,failed:2,cancelled:2});
  expect(queueProgress({...settled,counts:{completed:1000,verifying:1}}).percent).toBe(99);
  expect(queueProgress(null).percent).toBe(0);
 });
 it("aggregates working stages without hiding review or failure outcomes",async()=>{
  expect(queueProgress(snapshot())).toEqual({total:10,active:2,completed:5,review:2,failed:1,cancelled:1,percent:80});
  const host=document.createElement("div"),root=createRoot(host);
  try {
   await act(async()=>root.render(<QueueSummary snapshot={snapshot()}/>));
   expect(host.querySelector('.queue-summary-heading')).toHaveTextContent("전체 작업5 / 10 완료");
   expect(host.querySelector('.queue-summary-legend')).toHaveTextContent("완료 5작업 중 2대기 0검토 필요 2실패·중단 1");
   expect(host.querySelector('[title="완료 5개 (취소·격리 1개 포함)"]')).toBeTruthy();
   expect(host.querySelector('.queue-summary')).toHaveClass("is-running");
   expect(host.querySelector('.queue-progress-track .tone-active')).toHaveAttribute("title", "작업 중 2개 · 후처리 2개");
   expect(host.querySelector('.queue-progress-track .tone-active')).toHaveStyle({width:"20%"});
   expect(host.querySelector('[title="검토 필요 2개"]')).not.toHaveClass("is-running");
   expect([...host.querySelectorAll<HTMLElement>('.queue-progress-segment')].reduce((sum,node)=>sum+parseFloat(node.style.width),0)).toBe(100);
   await act(async()=>root.render(<QueueSummary snapshot={snapshot()} error="offline"/>));
   expect(host.querySelector('.queue-summary')).not.toHaveClass("is-running");
   expect(host.querySelector('[role="alert"]')).toHaveTextContent("마지막 확인값");
  } finally {await act(async()=>root.unmount());}
 });
 it("keeps the same segments through zero counts so boundaries transition instead of popping",async()=>{
  const host=document.createElement("div"),root=createRoot(host);
  const render=(counts:QueueSnapshot["counts"])=>act(async()=>root.render(<QueueSummary snapshot={{...snapshot(),counts}}/>));
  try {
   await render({queued:3,retry_wait:1});
   const segments=[...host.querySelectorAll('.queue-progress-segment')];
   expect(segments).toHaveLength(5);
   expect(segments[1]).toHaveAttribute("aria-hidden","true");
   expect(host.querySelector('.queue-summary')).not.toHaveClass("is-running");
   expect(segments[2]).toHaveAttribute("title","대기 4개 · 재시도 1개");
   await render({resolving_metadata:1,downloading:1,hashing:1,verifying:1});
   [...host.querySelectorAll('.queue-progress-segment')].forEach((segment,index)=>expect(segment).toBe(segments[index]));
   expect(segments[1]).toHaveAttribute("aria-hidden","false");
   expect(segments[1]).toHaveStyle({width:"100%"});
   expect(segments[1]).toHaveAttribute("title","작업 중 4개 · 정보 확인 1개 · 다운로드 1개 · 후처리 2개");
   expect(host.querySelector('.queue-summary')).toHaveClass("is-running");
   await render({completed:2,cancelled:1,quarantined:1});
   [...host.querySelectorAll('.queue-progress-segment')].forEach((segment,index)=>expect(segment).toBe(segments[index]));
   expect((segments[0] as HTMLElement).style.width).toBe("100%");
   expect((segments[1] as HTMLElement).style.width).toBe("0%");
   expect(host.querySelector('.queue-summary')).toHaveClass("is-finished");
   expect(host.querySelector('.queue-summary')).not.toHaveClass("is-running");
  } finally {await act(async()=>root.unmount());}
 });
 it("does not invent progress for empty, loading, or very large queues",async()=>{
  const host=document.createElement("div"),root=createRoot(host);
  try {
   await act(async()=>root.render(<QueueSummary snapshot={null}/>));
   expect(host.querySelector('.queue-progress-track')).toHaveAttribute("aria-busy","true");
   expect(host.querySelector('.queue-summary-heading')).toHaveTextContent("확인 중…");
   await act(async()=>root.render(<QueueSummary snapshot={{...snapshot(),counts:{}}}/>));
   expect(host.querySelector('.queue-summary-heading')).toHaveTextContent("작업 없음");
   expect(host.querySelector('.queue-summary')).not.toHaveClass("is-running","is-finished");
   await act(async()=>root.render(<QueueSummary snapshot={{...snapshot(),counts:{completed:999999,hashing:1}}}/>));
   expect(parseFloat(host.querySelector<HTMLElement>('.queue-progress-track .tone-active')!.style.width)).toBeCloseTo(0.0001,8);
   expect(host.querySelector('.queue-summary')).toHaveClass("is-running");
   expect(host.querySelector('.queue-summary')).not.toHaveClass("is-finished");
  } finally {await act(async()=>root.unmount());}
 });
 it("flashes only newly completed counts and respects reduced motion and stale data",async()=>{
  const descriptor=Object.getOwnPropertyDescriptor(HTMLElement.prototype,"animate");
  const cancel=vi.fn(),animate=vi.fn(()=>({cancel}));
  Object.defineProperty(HTMLElement.prototype,"animate",{configurable:true,value:animate});
  const reduced=vi.fn(()=>({matches:false}));
  vi.stubGlobal("matchMedia",reduced);
  const host=document.createElement("div"),root=createRoot(host);
  const render=(completed:number,error?:string,scoped=false,total=10)=>act(async()=>root.render(
   <QueueSummary snapshot={{...snapshot(),counts:{completed,hashing:total-completed}}} error={error} scoped={scoped}/>));
  try {
   await render(1);
   expect(animate).not.toHaveBeenCalled();
   await render(2);
   expect(animate).toHaveBeenCalledTimes(2);
   expect(animate.mock.instances[0]).toBe(host.querySelector('.queue-completion-edge'));
   await render(2);
   expect(animate).toHaveBeenCalledTimes(2);
   await render(3,"offline");
   expect(cancel).toHaveBeenCalledTimes(2);
   await render(4);
   await render(5,undefined,true);
   await render(6,undefined,true,11);
   expect(animate).toHaveBeenCalledTimes(2);
   reduced.mockReturnValue({matches:true});
   await render(7,undefined,true,11);
   expect(animate).toHaveBeenCalledTimes(2);
   reduced.mockReturnValue({matches:false});
   await render(11,undefined,true,11);
   expect(animate).toHaveBeenCalledTimes(4);
   expect(host.querySelector('.queue-summary')).toHaveClass("is-finished");
   expect(host.querySelector('.queue-summary')).not.toHaveClass("is-running");
  } finally {
   await act(async()=>root.unmount());
   if(descriptor) Object.defineProperty(HTMLElement.prototype,"animate",descriptor);
   else delete (HTMLElement.prototype as Partial<HTMLElement>).animate;
   vi.unstubAllGlobals();
  }
 });
 it("contains only current rows and selected cancellation, leaving later arrivals alone",async()=>{
  const cancel=vi.fn(async()=>"취소 완료"),refresh=vi.fn(),host=document.createElement("div"),root=createRoot(host);
  const render=(s:QueueSnapshot)=>root.render(<WorkQueuePanel snapshot={s} query={{}} error={null} onQuery={vi.fn()} onRefresh={refresh} onCancelEntries={cancel} onOpen={vi.fn()}/>);
  try {
   await act(async()=>render(snapshot()));
   expect(host.querySelector('.queue-summary')).toBeNull();
   expect(host.querySelectorAll("select,details,dialog")).toHaveLength(0);
   expect(host.querySelectorAll(".gallery-thumbnail")).toHaveLength(0);
   expect(host.querySelectorAll(".queue-item-status")).toHaveLength(2);
   expect(host.querySelector(".queue-item-number")).toHaveTextContent("#1");
   expect(host.querySelectorAll("button")).toHaveLength(2);
   expect(host.textContent).not.toMatch(/상태 확인|요청 이후|완료 포함|전체 취소|예상/);
   await act(async()=> (host.querySelectorAll('input[type="checkbox"]')[1] as HTMLInputElement).click());
   await act(async()=>render({...snapshot(),totalRows:3,items:[row(1),row(2),row(3)]}));
   await act(async()=> (host.querySelector(".queue-actions button") as HTMLButtonElement).click());
   expect(cancel).toHaveBeenCalledExactlyOnceWith(["entry-2"]);
   expect(refresh).toHaveBeenCalledOnce();
  }finally{await act(async()=>root.unmount());}
 });
 it("selects rows and modified titles by entry identity, and opens a plain title", async () => {
  const open = vi.fn(), host = document.createElement("div"), root = createRoot(host);
  const click = (node: Element, keys: MouseEventInit = {}) => act(async () => {
    node.dispatchEvent(new MouseEvent("click", { bubbles: true, ...keys }));
  });
  try {
    await act(async () => root.render(<WorkQueuePanel snapshot={{ ...snapshot(), totalRows: 5, items: [1,2,3,4,5].map(row) }}
      query={{}} error={null} onQuery={vi.fn()} onRefresh={vi.fn()} onCancelEntries={vi.fn()} onOpen={open} />));
    const rows = host.querySelectorAll(".queue-item");
    const count = (n: number) => expect(host.querySelector(".queue-actions")).toHaveTextContent("선택 " + n + "개");
    await click(rows[0]!); count(1);
    await click(rows[2]!.querySelector(".queue-item-number")!, { ctrlKey:true }); count(2);
    await click(rows[4]!, { ctrlKey:true, shiftKey:true }); count(4);
    await click(rows[3]!.querySelector("button")!, { shiftKey:true }); count(2);
    await click(rows[4]!.querySelector("input")!); count(3);
    expect(open).not.toHaveBeenCalled();
    await click(rows[1]!.querySelector("strong")!); expect(open).toHaveBeenCalledExactlyOnceWith(galleryId(2));
  } finally { await act(async () => root.unmount()); }
 });
 it("selects across virtual windows and ignores a stale range after a new click", async () => {
  const host = document.createElement("div"), root = createRoot(host), cancel = vi.fn(async () => "ok");
  let finish!: (ids:string[])=>void;
  const range = vi.spyOn(queueApi, "getQueueSelectionRange").mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const render = (start:number) => act(async () => root.render(<WorkQueuePanel snapshot={{...snapshot(),offset:start,totalRows:220,
    items:Array.from({length:100},(_,i)=>row(start+i+1))}} query={{offset:start}} error={null} onQuery={vi.fn()} onRefresh={vi.fn()} onCancelEntries={cancel} onOpen={vi.fn()} />));
  const click = (index:number,shiftKey=false) => act(async () => {
    host.querySelectorAll(".queue-item")[index]!.dispatchEvent(new MouseEvent("click",{bubbles:true,shiftKey}));
  });
  try {
    await render(0); await click(0); await render(100); await click(3,true);
    expect(range).toHaveBeenCalledWith({offset:100},"entry-1","entry-104");
    expect(host.querySelector(".queue-actions button")).toBeDisabled();
    await click(7); await act(async () => finish(["entry-1","entry-104"]));
    expect(host.querySelector(".queue-actions")).toHaveTextContent("선택 1개");
    await render(0); await click(0,true);
    const ids=Array.from({length:108},(_,i)=>"entry-"+(i+1));
    await act(async () => finish(ids));
    await act(async () => { (host.querySelector(".queue-actions button") as HTMLButtonElement).click(); });
    expect(cancel).toHaveBeenCalledExactlyOnceWith(ids);
  } finally { range.mockRestore(); await act(async () => root.unmount()); }
 });
 it("loads an uncached title on hover and drops a late response after the pointer moves", async () => {
  const host = document.createElement("div"), root = createRoot(host);
  const sample = await backend.gallerySummaryGet(galleryId(4051038));
  if (!sample.ok) throw Error("missing fixture");
  const metadata = { ok:true as const, data:{ ...sample.data, id:galleryId(77), title:"확인된 제목" } };
  let finish!: (value: typeof metadata) => void;
  const resolve = vi.spyOn(backend,"gallerySummaryGet").mockImplementationOnce(() => new Promise(done => { finish = done; })).mockResolvedValue(metadata);
  const client = new ThumbnailClient(browserFixtureThumbnailAdapter);
  vi.useFakeTimers();
  try {
    await act(async () => root.render(<ThumbnailProvider client={client}><WorkQueuePanel snapshot={{...snapshot(),items:[{...row(77),title:"#77"},row(78)]}}
      query={{}} error={null} onQuery={vi.fn()} onRefresh={vi.fn()} onCancelEntries={vi.fn()} onOpen={vi.fn()}/></ThumbnailProvider>));
    expect(resolve).not.toHaveBeenCalled();
    const titles=host.querySelectorAll(".queue-item-title");
    const hover=async (i:number)=>act(async()=>{titles[i]!.dispatchEvent(new MouseEvent("mouseover",{bubbles:true}));});
    await hover(0);
    await act(async()=>{await vi.advanceTimersByTimeAsync(200);});
    expect(resolve).toHaveBeenCalledExactlyOnceWith(galleryId(77));
    await hover(1);
    await act(async()=>finish(metadata));
    expect(document.querySelector(".queue-album-preview")).toHaveTextContent("앨범 78");
    expect(titles[0]).toHaveTextContent("#77");
    await hover(0);
    await act(async()=>{await vi.advanceTimersByTimeAsync(200);});
    expect(titles[0]).toHaveTextContent("확인된 제목");
    expect(document.querySelector(".queue-album-preview")).toHaveTextContent("확인된 제목");
  } finally { await act(async()=>root.unmount()); vi.useRealTimers(); resolve.mockRestore(); }
 });
 it("keeps covers in a single hover or keyboard preview and preserves row size across album settings", async () => {
  const host = document.createElement("div"), root = createRoot(host);
  document.body.appendChild(host);
  const render = (previewWidth: number) => act(async () => root.render(<WorkQueuePanel
    snapshot={snapshot()} query={{}} previewWidth={previewWidth} error={null}
    onQuery={vi.fn()} onRefresh={vi.fn()} onCancelEntries={vi.fn()} onOpen={vi.fn()} />));
  const tooltip = () => document.querySelector<HTMLElement>(".queue-album-preview");
  try {
    await render(160);
    const list = host.querySelector<HTMLElement>(".queue-items")!;
    const row = list.querySelector<HTMLElement>(".queue-item")!;
    const number = row.querySelector(".queue-item-number")!;
    expect(list.querySelector(".gallery-thumbnail")).toBeNull();
    expect(tooltip()).toBeNull();
    await act(async () => { number.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })); });
    expect(tooltip()).toHaveStyle({ width: "160px" });
    expect(tooltip()?.querySelector("strong")).toHaveTextContent("앨범 1");
    expect(document.querySelectorAll(".gallery-thumbnail")).toHaveLength(1);
    const rowHeight = row.style.height;
    await render(360);
    expect(tooltip()).toHaveStyle({ width: "360px" });
    expect(row.style.height).toBe(rowHeight);
    expect(list.querySelector(".gallery-thumbnail")).toBeNull();
    await act(async () => { number.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: list })); });
    expect(tooltip()).toBeNull();
    const other = list.querySelectorAll<HTMLButtonElement>(".queue-item-title")[1]!;
    await act(async () => other.focus());
    expect(tooltip()?.id).toBe(other.getAttribute("aria-describedby"));
    expect(tooltip()?.querySelector("strong")).toHaveTextContent("앨범 2");
    await act(async () => other.blur());
    expect(tooltip()).toBeNull();
    await act(async () => other.focus());
    await act(async () => { list.dispatchEvent(new Event("scroll")); });
    expect(tooltip()).toBeNull();
    await act(async () => { number.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })); });
    expect(tooltip()).toBeNull();
    await act(async () => { number.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 5 })); });
    expect(tooltip()?.querySelector("strong")).toHaveTextContent("앨범 1");
  } finally { await act(async () => root.unmount()); host.remove(); }
 });
 it("requests overlapping windows without page controls",async()=>{
  const query=vi.fn(),host=document.createElement("div"),root=createRoot(host);
  try{
   await act(async()=>root.render(<WorkQueuePanel snapshot={{...snapshot(),totalRows:1000}} query={{offset:0}} error={null} onQuery={query} onRefresh={vi.fn()} onCancelEntries={vi.fn()} onOpen={vi.fn()}/>));
   const list=host.querySelector(".queue-items") as HTMLDivElement;
   await act(async()=>{list.scrollTop=62*80;list.dispatchEvent(new Event("scroll",{bubbles:true}));});
   expect(query).toHaveBeenCalledWith({offset:25});
  }finally{await act(async()=>root.unmount());}
 });
});

it('separates waiting from active postprocessing without changing durable job states',async()=>{
 const host=document.createElement('div'),root=createRoot(host);
 const value={...snapshot(),counts:{hashing:18},postprocessingCounts:{waiting:15,hashing:1,comparing:1,unknown:1}};
 try{ await act(async()=>root.render(<QueueSummary snapshot={value}/>));
 expect(host.querySelector('.queue-summary-legend')).toHaveTextContent('작업 중 2대기 16');
 expect(host.querySelector('.tone-active')).toHaveAttribute('title','작업 중 2개 · 해시 계산 중 1개 · 중복 비교 중 1개');
 expect(host.querySelector('.tone-waiting')?.getAttribute('title')).toContain('후처리 대기 15개');
 expect(queueProgress(value).active).toBe(18);
 }finally{await act(async()=>root.unmount());}
});
