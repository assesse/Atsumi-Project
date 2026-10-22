import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { XArchiveTools } from "./XArchivePanel";
import { XOfficialWorkspace } from "./XWorkspace";
import { isXArchiveChooseRequest, type XArchiveChooseRequest, type XSurfaceApi } from "./api";
import type { XArchiveApi, XArchiveDraft } from "./xArchiveApi";

vi.mock("../platforms/nativePlayerGeometry",()=>({
  measureNativePlayerGeometry:vi.fn((_left,_right,options)=>({...options,player:null,chat:null,occlusions:[]})),
  observeNativePlayerGeometry:vi.fn(()=>()=>{}),
}));
const choice=(overrides:Partial<XArchiveChooseRequest>={}):XArchiveChooseRequest=>({
  id:"11111111-1111-4111-8111-111111111111",generation:7,accountId:"legacy",feed:"foryou",pane:"feed",
  path:"/alice/status/234567",sourceUrl:"https://x.com/home",...overrides,
});
const draft=():XArchiveDraft=>({
  captureId:"22222222-2222-4222-8222-222222222222",accountId:"legacy",
  document:{version:1,owner:"alice",sourceUrl:"https://x.com/home",capturedAt:"2026-10-20T00:00:00Z",coverage:"loaded_dom_only",limitReached:false,
    records:["123456","234567","345678"].map(id=>({id,url:`https://x.com/alice/status/${id}`,author:"Alice",body:`본문 ${id}`,
      timestamp:"2026-10-20T00:00:00Z",counts:"",context:"",quotes:[],media:[],truncated:false}))},
});
const archive=():XArchiveApi=>({
  inspect:vi.fn().mockResolvedValue(draft()),release:vi.fn().mockResolvedValue(true),save:vi.fn(),cancel:vi.fn(),list:vi.fn().mockResolvedValue([]),
  read:vi.fn(),openFolder:vi.fn(),openSource:vi.fn(),mediaUrl:vi.fn(()=>""),});
const deferred=<T,>()=>{let resolve!:(value:T)=>void;const promise=new Promise<T>(done=>resolve=done);return{promise,resolve};};
let host:HTMLDivElement,root:Root;
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.unstubAllGlobals();});
const renderTools=(api:XArchiveApi,props:Partial<ComponentProps<typeof XArchiveTools>>={})=>act(async()=>root.render(
  <XArchiveTools runtime="browser-mock" pane="feed" generation={7} contextKey="legacy:foryou:7" showControls={false} showLibrary={false} api={api} {...props}/>));
const checkbox=(id:string)=>document.querySelector<HTMLInputElement>(`input[aria-label$="#${id}"]`);

it("accepts only a bounded main-event intent without arbitrary source, route or generation",()=>{
  expect(isXArchiveChooseRequest(choice())).toBe(true);
  for(const changed of [{id:"../file"},{generation:-1},{generation:NaN},{generation:1.1},{pane:"other"},{path:"/alice/status/234567/photo/1"},
    {path:"https://x.com/alice/status/234567"},{path:"/home"},{accountId:""},{sourceUrl:"https://evil.test/home"},
    {sourceUrl:"https://user@x.com/home"},{sourceUrl:"https://x.com:444/home"},{sourceUrl:"https://x.com/home?stale=1"},
    {sourceUrl:"https://x.com/__atsumi_surface__/nonce"},{sourceUrl:"javascript:alert(1)"}]){
    expect(isXArchiveChooseRequest({...choice(),...changed})).toBe(false);
  }
  expect(isXArchiveChooseRequest(null)).toBe(false);
});
it("freezes the requested native pane before any overlay and defaults the exact clicked post",async()=>{
  const api=archive(),pending=deferred<XArchiveDraft>();vi.mocked(api.inspect).mockReturnValueOnce(pending.promise);
  await renderTools(api);expect(api.inspect).not.toHaveBeenCalled();expect(host.querySelector("button")).toBeNull();
  await renderTools(api,{chooseRequest:choice()});
  expect(api.inspect).toHaveBeenCalledExactlyOnceWith("feed",7);expect(document.querySelector("[data-native-overlay]")).toBeNull();
  await act(async()=>pending.resolve(draft()));
  expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  expect(checkbox("123456")).not.toBeChecked();expect(checkbox("234567")).toBeChecked();expect(checkbox("345678")).not.toBeChecked();
  await act(async()=>checkbox("345678")!.click());
  await renderTools(api,{chooseRequest:{...choice()}});
  expect(api.inspect).toHaveBeenCalledOnce();expect(checkbox("234567")).toBeChecked();expect(checkbox("345678")).toBeChecked();
  expect(api.save).not.toHaveBeenCalled(); // Opening a chooser never writes.
});
it("rejects stale generation and other-pane intent before inspecting",async()=>{
  const api=archive();
  await renderTools(api,{chooseRequest:choice({generation:6})});
  await renderTools(api,{chooseRequest:choice({id:"33333333-3333-4333-8333-333333333333",pane:"preview"})});
  expect(api.inspect).not.toHaveBeenCalled();expect(document.querySelector('[role="dialog"]')).toBeNull();
});
it.each(["source","account","post"] as const)("does not open a chooser if inspected %s differs from the native intent",async(change)=>{
  const api=archive(),value=draft(),notice=vi.fn();
  if(change==="source")value.document.sourceUrl="https://x.com/i/bookmarks";
  if(change==="account")value.accountId="other";
  if(change==="post")value.document.records=value.document.records.filter(post=>post.id!=="234567");
  vi.mocked(api.inspect).mockResolvedValueOnce(value);
  await renderTools(api,{chooseRequest:choice(),onNotice:notice});
  expect(document.querySelector('[role="dialog"]')).toBeNull();expect(api.save).not.toHaveBeenCalled();
  expect(notice).toHaveBeenCalledWith("선택한 게시물이나 화면이 변경되었습니다. 다시 시도해 주세요.");
  expect(api.release).toHaveBeenCalledExactlyOnceWith(value.captureId,value.accountId);
});
it("drops late inspections after collection generation or privacy changes",async()=>{
  for(const props of [{generation:8,contextKey:"legacy:bookmarks:8"},{privacyMode:true}]){
    const api=archive(),pending=deferred<XArchiveDraft>();vi.mocked(api.inspect).mockReturnValueOnce(pending.promise);
    await renderTools(api,{chooseRequest:choice()});await renderTools(api,{chooseRequest:choice(),...props});
    await act(async()=>pending.resolve(draft()));
    expect(document.querySelector('[role="dialog"]')).toBeNull();expect(api.save).not.toHaveBeenCalled();
    await act(async()=>root.render(null));
  }
});

function workspaceApi(preview=false){
  let chosen:((value:XArchiveChooseRequest)=>void)|undefined;
  const unlisten=vi.fn();
  const state={generation:7,feed:"foryou",state:"ready",preview,previewState:preview?"ready":"empty",error:null,
    previews:preview?[{id:"first",path:"/alice/status/234567",title:"미리보기",state:"ready"}]:[],activePreviewId:preview?"first":null};
  const api:XSurfaceApi={
    status:vi.fn().mockResolvedValue({status:"signedIn",generation:1,open:false,active:"legacy",accounts:[{id:"legacy",handle:"alice"}]}),
    account:vi.fn(),select:vi.fn(),suspend:vi.fn().mockResolvedValue(undefined),
    surface:vi.fn().mockImplementation(async feed=>({...state,feed})),surfaceStatus:vi.fn().mockResolvedValue(state),
    viewport:vi.fn().mockResolvedValue(undefined),closePreview:vi.fn(),selectPreview:vi.fn(),dismissMenus:vi.fn().mockResolvedValue(undefined),
    subscribeArchiveChoose:vi.fn(async callback=>{chosen=callback;return unlisten;}),
  };
  return {api,unlisten,send:async(value:XArchiveChooseRequest)=>act(async()=>chosen?.(value)),sendBurst:async(values:XArchiveChooseRequest[])=>act(async()=>values.forEach(value=>chosen?.(value)))};
}
it("workspace fences stale account/feed/generation/absent-preview events and routes current choice without global controls",async()=>{
  const api=workspaceApi(),archiveApi=archive();
  await act(async()=>root.render(<XOfficialWorkspace runtime="browser-mock" active privacyMode={false} api={api.api} archiveApi={archiveApi} onNotice={vi.fn()}/>));
  for(const stale of [{generation:6},{accountId:"other"},{feed:"bookmarks"},{pane:"preview" as const}])await api.send(choice(stale));
  expect(archiveApi.inspect).not.toHaveBeenCalled();
  await api.send(choice());
  expect(archiveApi.inspect).toHaveBeenCalledExactlyOnceWith("feed",7);
  expect(host.querySelector(".x-archive-tools")).toBeNull();expect(checkbox("234567")).toBeChecked();
});
it("workspace rejects delayed delivery after leaving a collection or hiding",async()=>{
  const api=workspaceApi(),archiveApi=archive(),render=(view:"explore"|"bookmarks"|"local",active=true)=>act(async()=>root.render(
    <XOfficialWorkspace runtime="browser-mock" view={view} active={active} privacyMode={false} api={api.api} archiveApi={archiveApi} onNotice={vi.fn()}/>));
  await render("explore");await render("bookmarks");await api.send(choice());
  await render("local");await api.send(choice());
  await render("explore",false);await api.send(choice());
  expect(archiveApi.inspect).not.toHaveBeenCalled();
  await act(async()=>root.render(null));expect(api.unlisten).toHaveBeenCalledOnce();
});
it("invalidates a pending preview inspection when the active retained id changes, even before a newer generation arrives",async()=>{
  const wrapper=workspaceApi(true),archiveApi=archive(),pending=deferred<XArchiveDraft>();let changed=()=>{};
  wrapper.api.subscribe=vi.fn(async callback=>{changed=callback;return()=>{};});
  vi.mocked(archiveApi.inspect).mockReturnValueOnce(pending.promise);
  await act(async()=>root.render(<XOfficialWorkspace runtime="browser-mock" active privacyMode={false} api={wrapper.api} archiveApi={archiveApi} onNotice={vi.fn()}/>));
  await wrapper.send(choice({pane:"preview"}));expect(archiveApi.inspect).toHaveBeenCalledExactlyOnceWith("preview",7);
  vi.mocked(wrapper.api.surfaceStatus).mockResolvedValueOnce({generation:7,feed:"foryou",state:"ready",preview:true,previewState:"ready",error:null,
    previews:[{id:"second",path:"/bob/status/456789",title:"@bob",state:"ready"}],activePreviewId:"second"});
  await act(async()=>changed());await act(async()=>pending.resolve(draft()));
  expect(document.querySelector('[role=dialog]')).toBeNull();expect(archiveApi.save).not.toHaveBeenCalled();
});
const newId=(number:number)=>`${String(number).padStart(8,"0")}-1111-4111-8111-111111111111`;
const closeChooser=()=>act(async()=>document.querySelector<HTMLButtonElement>('[aria-label="보관 창 닫기"]')!.click());
it("admits only one pane synchronously during a burst and retains admission through the dialog until dismissal",async()=>{
  const api=workspaceApi(true),archiveApi=archive(),pending=deferred<XArchiveDraft>();
  vi.mocked(archiveApi.inspect).mockReturnValueOnce(pending.promise);
  await act(async()=>root.render(<XOfficialWorkspace runtime="browser-mock" active privacyMode={false} api={api.api} archiveApi={archiveApi} onNotice={vi.fn()}/>));
  await api.sendBurst([choice(),choice({id:newId(2),pane:"preview"})]);
  expect(archiveApi.inspect).toHaveBeenCalledExactlyOnceWith("feed",7);
  expect(document.querySelectorAll('[role="dialog"]')).toHaveLength(0);
  await act(async()=>pending.resolve(draft()));
  await api.send(choice({id:newId(3),pane:"preview"}));
  expect(archiveApi.inspect).toHaveBeenCalledOnce();expect(document.querySelectorAll('[role="dialog"]')).toHaveLength(1);
  await closeChooser();await api.send(choice()); // A replay must not reopen or wedge admission.
  expect(archiveApi.inspect).toHaveBeenCalledOnce();
  await api.send(choice({id:newId(4),pane:"preview"}));
  expect(archiveApi.inspect).toHaveBeenCalledTimes(2);expect(archiveApi.inspect).toHaveBeenLastCalledWith("preview",7);
  expect(document.querySelectorAll('[role="dialog"]')).toHaveLength(1);
});
it("releases cross-pane admission after failed inspection and after a lifecycle change, without an old result unlocking a new chooser",async()=>{
  const api=workspaceApi(true),archiveApi=archive(),old=deferred<XArchiveDraft>(),current=deferred<XArchiveDraft>();
  const render=(active=true)=>act(async()=>root.render(<XOfficialWorkspace runtime="browser-mock" active={active} privacyMode={false} api={api.api} archiveApi={archiveApi} onNotice={vi.fn()}/>));
  vi.mocked(archiveApi.inspect).mockRejectedValueOnce(new Error("source unavailable"));
  await render();await api.send(choice());
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  vi.mocked(archiveApi.inspect).mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
  await api.send(choice({id:newId(2),pane:"preview"}));expect(archiveApi.inspect).toHaveBeenCalledTimes(2);
  await render(false);await render();
  await api.send(choice({id:newId(3)}));expect(archiveApi.inspect).toHaveBeenCalledTimes(3);
  await act(async()=>old.resolve(draft()));
  await api.send(choice({id:newId(4),pane:"preview"}));expect(archiveApi.inspect).toHaveBeenCalledTimes(3);
  await act(async()=>current.resolve(draft()));
  expect(document.querySelectorAll('[role="dialog"]')).toHaveLength(1);expect(checkbox("234567")).toBeChecked();
});
it("does not replace an active save or its cancel control when another pane requests a chooser",async()=>{
  const api=workspaceApi(true),archiveApi=archive(),saving=deferred<never>();
  vi.mocked(archiveApi.save).mockReturnValueOnce(saving.promise);vi.mocked(archiveApi.cancel).mockResolvedValue(true);
  await act(async()=>root.render(<XOfficialWorkspace runtime="browser-mock" active privacyMode={false} api={api.api} archiveApi={archiveApi} onNotice={vi.fn()}/>));
  await api.send(choice());
  await act(async()=>[...document.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="보관")!.click());
  await api.send(choice({id:newId(2),pane:"preview"}));
  expect(archiveApi.inspect).toHaveBeenCalledOnce();expect(archiveApi.save).toHaveBeenCalledOnce();
  expect(document.querySelector<HTMLButtonElement>('[aria-label="보관 창 닫기"]')).toBeDisabled();
  const cancel=[...document.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="저장 취소")!;
  expect(cancel).toBeEnabled();await act(async()=>cancel.click());
  expect(archiveApi.cancel).toHaveBeenCalledWith(vi.mocked(archiveApi.save).mock.calls[0]![0].requestId);
});
