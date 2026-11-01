import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createXMediaSearchApi, isXPageEvidence, type XPageEvidence as Evidence } from "./xMediaSearchApi";
import { SourceText, XPageEvidence } from "./XPageEvidence";
const id="11111111-1111-4111-8111-111111111111",url="https://example.org/item";
const evidence=(requestedUrl=url):Evidence=>({requestedUrl,url:requestedUrl,observedAt:"2026-10-27T13:00:00Z",limited:false,ambiguous:false,fields:[{label:"상품 번호",value:"ITEM-042",source:"json-ld[0].sku"}]});
let host:HTMLDivElement,root:Root,mounted:boolean;
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);mounted=true;});
afterEach(async()=>{if(mounted)await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
const render=(node:ReactNode)=>act(async()=>root.render(node));
const open=()=>act(async()=>{const details=host.querySelector("details")!;details.open=true;details.dispatchEvent(new Event("toggle"));});
const click=(label:string)=>act(async()=>{const button=[...host.querySelectorAll("button")].find(node=>node.textContent===label)!;button.click();});
it("reads only the explicitly expanded current indexed source and preserves literal field text",async()=>{
  const read=vi.fn().mockResolvedValue(evidence()),api={...createXMediaSearchApi("browser-mock"),pageEvidence:read};
  await render(<XPageEvidence api={api} jobId={id} provider="lens" candidateIndex={2} links={[{label:"Source",url}]} phrase="ITEM-042"/>);
  expect(read).not.toHaveBeenCalled();await open();
  expect(host.textContent).toContain("ITEM-042");expect(read).toHaveBeenCalledExactlyOnceWith(id,2,0,"lens");
  expect(host.querySelector("mark")?.textContent).toBe("ITEM-042");expect(host.querySelector("dt")?.title).toBe("json-ld[0].sku");
});
it("switches exact source indices and does not relabel one source's information as another",async()=>{
  const second="https://other.example.org/detail",read=vi.fn().mockResolvedValueOnce(evidence()).mockResolvedValueOnce({...evidence(second),fields:[]}),api={...createXMediaSearchApi("browser-mock"),pageEvidence:read};
  await render(<XPageEvidence api={api} jobId={id} candidateIndex={0} links={[{label:"One",url},{label:"Two",url:second}]} phrase=""/>);await open();
  expect(host.textContent).toContain("ITEM-042");await click("2 · other.example.org");
  expect(host.textContent).toContain("읽어 낸 정보 없음");expect(host.textContent).not.toContain("ITEM-042");expect(read).toHaveBeenLastCalledWith(id,0,1,undefined);
});
it("discards late data after changing jobs or unmounting the disclosure",async()=>{
  let finish!:(value:Evidence)=>void;const read=vi.fn().mockImplementation(()=>new Promise<Evidence>(resolve=>{finish=resolve;})),api={...createXMediaSearchApi("browser-mock"),pageEvidence:read};
  const props={api,jobId:id,candidateIndex:0,links:[{label:"One",url}],phrase:""};
  await render(<XPageEvidence {...props}/>);await open();expect(read).toHaveBeenCalledOnce();
  const oldFinish=finish;await render(<XPageEvidence {...props} jobId="22222222-2222-4222-8222-222222222222"/>);
  await act(async()=>oldFinish(evidence()));expect(host.textContent).not.toContain("ITEM-042");
  await act(async()=>root.unmount());mounted=false;await act(async()=>finish(evidence()));expect(host.textContent).not.toContain("ITEM-042");
});
it("resets the selected source and waits for expansion when refreshed links replace a result",async()=>{
  const second="https://other.example.org/detail",next="https://new.example.org/item";
  const read=vi.fn().mockResolvedValueOnce(evidence()).mockResolvedValueOnce(evidence(second)).mockResolvedValueOnce(evidence(next));
  const api={...createXMediaSearchApi("browser-mock"),pageEvidence:read},props={api,jobId:id,candidateIndex:0,phrase:""};
  await render(<XPageEvidence {...props} links={[{label:"One",url},{label:"Two",url:second}]}/>);await open();await click("2 · other.example.org");
  expect(read).toHaveBeenCalledTimes(2);
  await render(<XPageEvidence {...props} links={[{label:"New",url:next}]}/>);
  expect(host.querySelector("details")?.open).toBe(false);expect(read).toHaveBeenCalledTimes(2);
  await open();expect(read).toHaveBeenLastCalledWith(id,0,0,undefined);expect(read).toHaveBeenCalledTimes(3);
  expect(host.textContent).toContain("ITEM-042");
});
it("keeps failures retryable and does not display data for an unexpected source",async()=>{
  const read=vi.fn().mockRejectedValueOnce(new Error("unavailable")).mockResolvedValueOnce(evidence("https://wrong.example.org/")),api={...createXMediaSearchApi("browser-mock"),pageEvidence:read};
  await render(<XPageEvidence api={api} jobId={id} candidateIndex={0} links={[{label:"One",url}]} phrase=""/>);await open();
  expect(host.textContent).toContain("페이지 정보를 읽지 못했습니다.");await click("다시 확인");
  expect(read).toHaveBeenCalledTimes(2);expect(host.textContent).not.toContain("ITEM-042");expect(host.textContent).toContain("페이지 정보를 읽지 못했습니다.");
});
it("highlights literal Unicode and punctuation without treating text as HTML or a regex",async()=>{
  await render(<SourceText text={'한글 a.*b <img src=x onerror=alert(1)> A.*B'} phrase="a.*b"/>);
  expect(host.querySelectorAll("mark")).toHaveLength(2);expect(host.querySelector("img")).toBeNull();expect(host.textContent).toContain("<img src=x");
});
it("rejects oversized, executable or structurally unbounded native evidence replies",()=>{
  expect(isXPageEvidence(evidence())).toBe(true);
  for(const change of [{url:"javascript:alert(1)"},{fields:Array(25).fill(evidence().fields[0])},{fields:[{...evidence().fields[0],value:"x".repeat(2049)}]},{observedAt:"bad"},{extra:true}])expect(isXPageEvidence({...evidence(),...change})).toBe(false);
});
