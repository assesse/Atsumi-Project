import { describe, expect, it } from "vitest";
import { readNavigationCheckpoint,writeNavigationCheckpoint,type NavigationCheckpoint } from "./navigationCheckpoint";
import { galleryId } from "../core/types";
const sample=():NavigationCheckpoint=>({ version:1,savedAt:Date.now(),view:"downloads",downloadsFilter:"failed",activeTab:"tab1",tabs:[{
  id:"tab1",label:"search",displayValue:"query",queryId:"query-id",page:3,scrollTop:782,
  request:{ text:"query",includeTags:[],excludeTags:[],languages:["korean"],sort:"recent",pageSize:100 },
}] });
describe("renderer recovery navigation checkpoint",()=>{
  it("keeps parent links, detail order, minimization, and preview positions without image payloads",()=>{
    let raw="";
    const origin={view:"explore" as const,contextId:"tab1",detailId:42};
    const value:NavigationCheckpoint={...sample(),detail:{tabs:[galleryId(42),galleryId(43)],activeId:galleryId(43),minimized:true},detailOrigins:[[43,origin]],detailPositions:[[42,{scrollTop:701,previewStart:19}]]};
    value.tabs[0]!.origin=origin;
    writeNavigationCheckpoint(value,{setItem:(_,s)=>{raw=s;}});
    expect(readNavigationCheckpoint({getItem:()=>raw})).toEqual(value);
    expect(raw.length).toBeLessThan(1500);
  });
  it("retains tab/page/scroll and filters but no galleries, image bodies or mutations",()=>{
    let raw=""; const original={ ...sample(),galleries:[{ bytes:new Array(10_000).fill(0) }],downloadQueue:[42] };
    writeNavigationCheckpoint(original,{ setItem:(_,s)=>{raw=s;} });
    expect(raw.length).toBeLessThan(1000); expect(raw).not.toMatch(/galleries|bytes|downloadQueue/);
    expect(readNavigationCheckpoint({ getItem:()=>raw })).toEqual({ ...sample(),savedAt:original.savedAt });
  });
  it("rejects corruption, oversized payloads and expired navigation",()=>{
    for (const raw of ["{",JSON.stringify({ ...sample(),savedAt:1 })," ".repeat(128*1024+1)])
      expect(readNavigationCheckpoint({getItem:()=>raw})).toBeNull();
  });
  it("does not fail navigation when storage is unavailable",()=>{
    expect(()=>writeNavigationCheckpoint(sample(),{setItem:()=>{throw new Error("quota");}})).not.toThrow();
    expect(readNavigationCheckpoint({getItem:()=>{throw new Error("disabled");}})).toBeNull();
  });
});
