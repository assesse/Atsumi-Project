import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { galleryId } from "../core/types";
import type { QueueRow, QueueSnapshot } from "../api/workConsole";
import { queueProgress, QueueSummary, WorkQueuePanel } from "./WorkQueuePanel";
const row=(id:number):QueueRow=>({entryId:`entry-${id}`,galleryId:galleryId(id),title:`앨범 ${id}`,artist:"fixture",state:"hashing",progress:80,sequence:7,updatedAt:"",errorCode:null});
const snapshot=():QueueSnapshot=>({queriedAt:"",counts:{completed:4,review_required:2,failed:1,hashing:2,cancelled:1},globalActive:2,totalRows:2,page:1,pageSize:100,offset:0,items:[row(1),row(2)],batches:[],etaSeconds:null,recentCompleted:0,lastProgressAt:null});
describe("minimal download queue",()=>{
 it("settles the indicator for review, interrupted and excluded work without calling them successful",()=>{
  const settled = {...snapshot(), counts:{completed:4, review_required:2, interrupted:1, failed:1, cancelled:1, quarantined:1}, globalActive:0};
  expect(queueProgress(settled)).toMatchObject({percent:100,completed:6,review:2,failed:2,cancelled:2});
  expect(queueProgress({...settled,counts:{completed:1000,verifying:1}}).percent).toBe(99);
  expect(queueProgress(null).percent).toBe(0);
 });
 it("shows only the stage bar with concise hover labels and preserves completion semantics",async()=>{
  expect(queueProgress(snapshot())).toEqual({total:10,active:2,completed:5,review:2,failed:1,cancelled:1,percent:80});
  const host=document.createElement("div"),root=createRoot(host);
  try {
   await act(async()=>root.render(<QueueSummary snapshot={snapshot()}/>));
   expect(host.textContent).toBe("");
   expect(host.querySelector('[title="완료 5개 (취소·격리 1개 포함)"]')).toBeTruthy();
   expect(host.querySelector('.tone-muted')).toBeNull();
   expect(host.querySelector('[title="해시 2개"]')).toHaveClass("is-running");
   expect(host.querySelector('[title="검토 필요 2개"]')).not.toHaveClass("is-running");
   await act(async()=>root.render(<QueueSummary snapshot={snapshot()} error="offline"/>));
   expect(host.querySelector('[role="alert"]')).toHaveTextContent("마지막 확인값");
  } finally {await act(async()=>root.unmount());}
 });
 it("contains only current rows and selected cancellation, leaving later arrivals alone",async()=>{
  const cancel=vi.fn(async()=>"취소 완료"),refresh=vi.fn(),host=document.createElement("div"),root=createRoot(host);
  const render=(s:QueueSnapshot)=>root.render(<WorkQueuePanel snapshot={s} query={{}} error={null} onQuery={vi.fn()} onRefresh={refresh} onCancelEntries={cancel} onOpen={vi.fn()}/>);
  try {
   await act(async()=>render(snapshot()));
   expect(host.querySelector('.queue-summary')).toBeNull();
   expect(host.querySelectorAll("select,details,dialog")).toHaveLength(0);
   expect(host.querySelectorAll("button")).toHaveLength(3);
   expect(host.textContent).not.toMatch(/상태 확인|요청 이후|완료 포함|전체 취소|예상/);
   await act(async()=> (host.querySelectorAll('input[type="checkbox"]')[1] as HTMLInputElement).click());
   await act(async()=>render({...snapshot(),totalRows:3,items:[row(1),row(2),row(3)]}));
   await act(async()=> (host.querySelector(".queue-actions button") as HTMLButtonElement).click());
   expect(cancel).toHaveBeenCalledExactlyOnceWith(["entry-2"]);
   expect(refresh).toHaveBeenCalledOnce();
  }finally{await act(async()=>root.unmount());}
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
