import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { XWorkEvidence } from "./XWorkEvidence";
import { isXWorkEvidence, type XMediaSearchApi, type XWorkEvidence as Evidence, type XWorkContext } from "./xMediaSearchApi";
let host:HTMLDivElement,root:Root;
const report=():Evidence=>({reportId:"97ea02a0-b626-40c2-ab6b-5cf601e81ae2",works:[{title:"Declared Work",url:"https://shop.example.com/item",rootUrl:"https://example.com/source",origin:"게시자 등록 링크",observedAt:"2026-10-28T00:00:00Z",codes:[{label:"품번",value:"CODE-0042",source:"detail:tr[1]"}],sale:[{label:"판매 상태",value:"https://schema.org/InStock",source:"json-ld[0].offers[0].availability"}],ambiguous:false}],examined:2,unavailable:0,limited:false});
const context=():XWorkContext=>({group:0,status:"candidate",kind:"work",association:"same_page",pageStatus:"read",pageTitle:"Declared Work",claims:[{provider:"saucenao",position:1,title:"Declared Work",url:"https://shop.example.com/item",similarity:87.66,creator:"Declared author",metadata:[{label:"Source",value:"Catalog"}]}],reasons:["검색 결과가 가리키는 동일 페이지의 명시된 정보입니다."]});
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
const render=(api:XMediaSearchApi,jobId="job",sourceKey="source",busy=false)=>act(async()=>root.render(<XWorkEvidence api={api} jobId={jobId} sourceKey={sourceKey} phrase="CODE" busy={busy}/>));
it("reads work codes automatically without expanding page details and opens a native-owned report index",async()=>{
  const workEvidence=vi.fn().mockResolvedValue(report()),openWorkEvidence=vi.fn().mockResolvedValue(undefined),api={workEvidence,openWorkEvidence} as unknown as XMediaSearchApi;
  await render(api);expect(workEvidence).toHaveBeenCalledExactlyOnceWith("job");expect(host.textContent).toContain("CODE-0042");expect(host.querySelector("details")).toBeNull();expect(host.querySelector("mark")?.textContent).toBe("CODE");
  await act(async()=>host.querySelector("button")!.click());expect(openWorkEvidence).toHaveBeenCalledExactlyOnceWith("job",report().reportId,0);
});
it("discards late source evidence after a result refresh",async()=>{
  let resolve!:(value:Evidence)=>void;
  const workEvidence=vi.fn().mockReturnValueOnce(new Promise<Evidence>(yes=>resolve=yes)).mockResolvedValueOnce({...report(),works:[],examined:0,unavailable:2});const api={workEvidence} as unknown as XMediaSearchApi;
  await render(api);await render(api,"job","new-result");await act(async()=>resolve(report()));expect(host.textContent).not.toContain("CODE-0042");expect(host.textContent).toContain("2개 읽기 실패");expect(host.textContent).not.toContain("코드를 찾지 못했습니다");
});
it("distinguishes unreadable pages and partial scans from examined pages without codes",async()=>{
  const api={workEvidence:vi.fn().mockResolvedValue({...report(),works:[],examined:1,unavailable:2,limited:true})} as unknown as XMediaSearchApi;
  await render(api);expect(host.textContent).toContain("확인한 페이지에서 명시된 작품 코드를 찾지 못했습니다");expect(host.textContent).toContain("2개 읽기 실패");expect(host.textContent).toContain("일부 정보만 확인됨");
});
it("does not navigate during an existing search action",async()=>{const openWorkEvidence=vi.fn(),api={workEvidence:vi.fn().mockResolvedValue(report()),openWorkEvidence} as unknown as XMediaSearchApi;await render(api,"job","source",true);expect(host.querySelector("button")).toBeDisabled();await act(async()=>host.querySelector("button")!.click());expect(openWorkEvidence).not.toHaveBeenCalled();});
it("validates bounded literal fields, URLs, counts and immutable report IDs",()=>{
  expect(isXWorkEvidence(report())).toBe(true);
  expect(isXWorkEvidence({...report(),sourceStatus:"partial"})).toBe(true);expect(isXWorkEvidence({...report(),sourceStatus:"invented"})).toBe(false);
  for(const invalid of [{...report(),reportId:"fake"},{...report(),examined:25},{...report(),examined:24,unavailable:1},{...report(),works:[{...report().works[0],url:"javascript:alert(1)"}]},{...report(),works:[{...report().works[0],codes:[{label:"Code",value:"x".repeat(257),source:"declared"}]}]},{...report(),extra:true}])expect(isXWorkEvidence(invalid)).toBe(false);
});
it("does not present an unreadable source thread as an exhaustive work-code search",async()=>{const api={workEvidence:vi.fn().mockResolvedValue({...report(),works:[],sourceStatus:"unavailable"})} as unknown as XMediaSearchApi;await render(api);expect(host.textContent).toContain("게시물·타래의 등록 링크를 확인하지 못했습니다");});
it("opens a proven source thread through its native report index",async()=>{const value=report();value.works[0]!.rootUrl="https://x.com/author/status/123456";value.works[0]!.origin="게시자 타래 등록 링크";const openWorkEvidence=vi.fn().mockResolvedValue(undefined),api={workEvidence:vi.fn().mockResolvedValue(value),openWorkEvidence} as unknown as XMediaSearchApi;await render(api);await act(async()=>host.querySelector<HTMLButtonElement>('button.x-work-evidence-origin')!.click());expect(openWorkEvidence).toHaveBeenCalledExactlyOnceWith("job",value.reportId,0,true);});
it("accepts and explains a source-declared work title when no code or sale fields exist",async()=>{
  const value={...report(),works:[{...report().works[0]!,title:"A Literal Film Title",titleEvidence:{label:"작품명",value:"A Literal Film Title",source:"json-ld[0].name"},codes:[],sale:[]}]};
  expect(isXWorkEvidence(value)).toBe(true);
  const openWorkEvidence=vi.fn().mockResolvedValue(undefined),api={workEvidence:vi.fn().mockResolvedValue(value),openWorkEvidence} as unknown as XMediaSearchApi;
  await render(api);expect(host.textContent).toContain("A Literal Film Title");expect(host.textContent).toContain("품번·판매 정보 미확인");expect(host.textContent).not.toContain("코드를 찾지 못했습니다");
  await act(async()=>host.querySelector("button")!.click());expect(openWorkEvidence).toHaveBeenCalledExactlyOnceWith("job",value.reportId,0);
});
it("rejects mismatched, ambiguous, null, unbounded or extended title-evidence payloads",()=>{
  const work={...report().works[0]!,title:"A Literal Film Title",titleEvidence:{label:"작품명",value:"A Literal Film Title",source:"json-ld[0].name"},codes:[],sale:[]};
  for(const changed of [{titleEvidence:null},{titleEvidence:{...work.titleEvidence,value:"Another work"}},{titleEvidence:{...work.titleEvidence,source:""}},{title:"X".repeat(513),titleEvidence:{...work.titleEvidence,value:"X".repeat(513)}},{titleEvidence:{...work.titleEvidence,extra:"untrusted"}},{ambiguous:true}])expect(isXWorkEvidence({...report(),works:[{...work,...changed}]})).toBe(false);
  expect(isXWorkEvidence(report())).toBe(true);
});
it("keeps unreadable candidates and provider evidence without claiming a confirmed original",async()=>{
  const value=report();value.works[0]={...value.works[0]!,codes:[],sale:[],context:{...context(),status:"supported",pageStatus:"unavailable",association:"unverified",pageTitle:null}};
  expect(isXWorkEvidence(value)).toBe(true);
  await render({workEvidence:vi.fn().mockResolvedValue(value)} as unknown as XMediaSearchApi);
  expect(host.textContent).toContain("Declared Work");expect(host.textContent).toContain("페이지 읽기 실패 · 검색 후보 유지");expect(host.textContent).toContain("87.66%");expect(host.textContent).not.toContain("작품명 확인");
});
it("keeps conflicting product fields in an explicitly unlinked disclosure",async()=>{
  const value=report();value.works[0]!.context={...context(),status:"conflict",association:"conflict",kind:"product",pageTitle:"Unrelated product"};
  await render({workEvidence:vi.fn().mockResolvedValue(value)} as unknown as XMediaSearchApi);
  expect(host.textContent).toContain("정보 충돌 · 연결 보류");expect(host.textContent).toContain("Unrelated product");
  const details=host.querySelector<HTMLDetailsElement>(".x-work-unlinked")!;expect(details.open).toBe(false);expect(details.textContent).toContain("CODE-0042");expect(host.querySelectorAll(".x-work-source > dl")).toHaveLength(0);
});
it("groups associated sources while retaining immutable native indices and per-page values",async()=>{
  const value=report();value.works[0]!.context=context();value.works.push({...value.works[0]!,title:"Second source",url:"https://other.example/book",codes:[{label:"ISBN",value:"9781718504448",source:"json-ld[0].isbn"}],context:{...context(),association:"identifier",pageTitle:"Second source",claims:[]}});
  const openWorkEvidence=vi.fn().mockResolvedValue(undefined);
  await render({workEvidence:vi.fn().mockResolvedValue(value),openWorkEvidence} as unknown as XMediaSearchApi);
  expect(host.querySelectorAll(".x-work-group")).toHaveLength(1);expect(host.querySelectorAll(".x-work-source")).toHaveLength(2);
  expect(host.querySelectorAll(".x-work-source")[0]!.textContent).not.toContain("9781718504448");
  await act(async()=>host.querySelectorAll<HTMLButtonElement>("button.x-media-search-title")[1]!.click());expect(openWorkEvidence).toHaveBeenCalledWith("job",value.reportId,1);
});
it("rejects invalid relevance payloads and excessive claims",()=>{
  const valid={...report(),works:[{...report().works[0]!,context:context()}]};expect(isXWorkEvidence(valid)).toBe(true);
  for(const invalid of [{group:48},{status:"confirmed"},{association:"similarity"},{pageStatus:"invented"},{claims:Array.from({length:17},()=>context().claims[0])},{claims:[{...context().claims[0],similarity:Infinity}]},{claims:[{...context().claims[0],url:"javascript:alert(1)"}]},{extra:true}])expect(isXWorkEvidence({...valid,works:[{...valid.works[0],context:{...context(),...invalid}}]})).toBe(false);
});
