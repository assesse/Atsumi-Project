import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { XOfficialWorkspace as XWorkspace, XWorkspace as ApplicationXWorkspace } from "./XWorkspace";
import { createXSurfaceApi, type XSurfaceApi, type XSurfaceSnapshot, type XSnapshot } from "./api";
import { observeNativePlayerGeometry } from "../platforms/nativePlayerGeometry";
import type { XArchiveApi } from "./xArchiveApi";
vi.mock("../platforms/nativePlayerGeometry",()=>({
  measureNativePlayerGeometry:vi.fn((_left,_right,options)=>({...options,player:{x:220,y:60,width:600,height:900},chat:{x:820,y:60,width:900,height:900},occlusions:[]})),
  observeNativePlayerGeometry:vi.fn(()=>()=>{}),
}));
const account=(open=false):XSnapshot=>({open,status:"signedIn",generation:1,active:"one",accounts:[{id:"one",handle:"alice"},{id:"two",handle:"bob"}]});
const surface=(feed="foryou",options:Partial<XSurfaceSnapshot>={}):XSurfaceSnapshot=>({generation:1,feed,state:"ready",preview:false,previewState:"empty",error:null,
  previews:options.preview?[{id:"first",path:"/alice/status/123456",title:"미리보기",state:options.previewState??"ready"}]:[],
  activePreviewId:options.preview?"first":null,...options});
const mock=():XSurfaceApi=>({
  status:vi.fn().mockResolvedValue(account()),account:vi.fn().mockResolvedValue(account(true)),select:vi.fn().mockResolvedValue({...account(),active:"two"}),
  suspend:vi.fn().mockResolvedValue(undefined),surface:vi.fn().mockImplementation(async feed=>surface(feed)),
  surfaceStatus:vi.fn().mockResolvedValue(surface()),viewport:vi.fn().mockResolvedValue(undefined),closePreview:vi.fn().mockResolvedValue(surface()),
  selectPreview:vi.fn().mockResolvedValue(surface()),dismissMenus:vi.fn().mockResolvedValue(undefined),
});
let host:HTMLDivElement,root:Root;
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.useRealTimers();vi.restoreAllMocks();vi.unstubAllGlobals();});
type View="explore"|"likes"|"bookmarks"|"local";
const render=(api:XSurfaceApi,active=true,privacyMode=false,view:View="explore",extra:{accountPortalTarget?:HTMLElement;archiveApi?:XArchiveApi}={})=>act(async()=>root.render(<XWorkspace api={api} active={active} privacyMode={privacyMode} view={view} runtime="browser-mock" onNotice={vi.fn()} {...extra}/>));
const button=(label:string)=>[...host.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent===label||b.getAttribute("aria-label")===label)!;
const click=(label:string)=>act(async()=>button(label).click());
const deferred=<T,>()=>{let resolve!:(value:T)=>void;let reject!:(cause:unknown)=>void;const promise=new Promise<T>((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};};
it("exports the official split as the real application workspace",()=>{expect(ApplicationXWorkspace).toBe(XWorkspace);});
it("reports unsupported browser runtime, never pretends a projection is the live feed",async()=>{await expect(createXSurfaceApi("browser-mock").surface("foryou",1,false)).rejects.toThrow("데스크톱");});
it("uses real official surfaces instead of copying posts, media, or engagement counts",async()=>{
  const api=mock();await render(api);expect(api.surface).toHaveBeenCalledWith("foryou",expect.any(Number),false);
  expect(host.querySelector('[aria-label="X 공식 타임라인"]')).not.toBeNull();
  expect(host.querySelector("article,img,video")).toBeNull(); // Native child owns all original X media/actions.
  expect(host.textContent).not.toContain("외부 브라우저에서 재생");
});
it("has no empty-preview arrow or original-page buttons",async()=>{await render(mock());expect(host.textContent).toContain("게시물을 선택하면");expect(host.textContent).not.toContain("↗");expect(host.textContent).not.toContain("원문");});
it("leaves the native home strip alone and follows sidebar views without a duplicate top toolbar",async()=>{
  const api=mock();await render(api);expect(host.querySelector(".x-feeds")).toBeNull();
  for(const label of ["추천","팔로잉","가입된 X 커뮤니티"])expect(host.textContent).not.toContain(label);
  expect(host.querySelector(".x-official-toolbar,.x-toolbar,.x-collections")).toBeNull();
  for(const [view,feed] of [["likes","likes"],["bookmarks","bookmarks"],["explore","foryou"]] as const){
    await render(api,true,false,view);expect(api.surface).toHaveBeenLastCalledWith(feed,expect.any(Number),false);
  }
});
it("requests cached home on return, not a refresh that loses original tab and scroll",async()=>{const api=mock();await render(api);await render(api,true,false,"likes");await render(api);expect(api.surface).toHaveBeenLastCalledWith("foryou",expect.any(Number),false);});
it("only explicit floating-menu refresh requests a new official document",async()=>{const api=mock();await render(api);expect(button("새로고침")).toBeUndefined();await click("계정");await click("새로고침");expect(api.surface).toHaveBeenLastCalledWith("foryou",expect.any(Number),true);});
it("shows a skeleton during command and remote-document loading",async()=>{
  const api=mock();let resolve!:(v:XSurfaceSnapshot)=>void;vi.mocked(api.surface).mockReturnValueOnce(new Promise(r=>resolve=r));await render(api);
  expect(host.querySelector('[aria-label="피드 불러오는 중"]')).not.toBeNull();
  await act(async()=>resolve(surface("foryou",{state:"loading"})));expect(host.querySelector('[aria-label="피드 불러오는 중"]')).not.toBeNull();
});
it("renders full preview slot, close and loading skeleton without fabricated social buttons",async()=>{
  const api=mock();vi.mocked(api.surface).mockResolvedValue(surface("foryou",{preview:true,previewState:"loading"}));await render(api);
  expect(host.querySelector('[aria-label="미리보기 불러오는 중"]')).not.toBeNull();expect(button("미리보기 닫기")).not.toBeNull();expect(host.querySelector('[aria-label="X 원본 미리보기"]')).not.toBeNull();expect(host.querySelector('[role="tab"] span')?.textContent).toBe("미리보기");
  await click("미리보기 닫기");expect(api.closePreview).toHaveBeenCalledExactlyOnceWith(1,"first");expect(host.textContent).toContain("게시물을 선택하면");
});
it("does not start site work while private or inactive",async()=>{const api=mock();await render(api,false);await render(api,true,true);expect(api.surface).not.toHaveBeenCalled();expect(api.account).not.toHaveBeenCalled();expect(button("X 로그인")).toBeDisabled();});
it("revokes native content and late responses when privacy is enabled",async()=>{
  const api=mock();let resolve!:(v:XSurfaceSnapshot)=>void;vi.mocked(api.surface).mockReturnValueOnce(new Promise(r=>resolve=r));await render(api);await render(api,true,true);
  await act(async()=>resolve(surface("foryou",{preview:true,previewState:"ready"})));
  expect(api.suspend).toHaveBeenCalled();expect(host.querySelector(".x-preview")).toBeNull();expect(host.textContent).not.toContain("@alice");
});
it("does not let a late earlier tab replace a newer tab",async()=>{
  const api=mock();let resolve!:(v:XSurfaceSnapshot)=>void;vi.mocked(api.surface).mockReturnValueOnce(new Promise(r=>resolve=r));await render(api);await render(api,true,false,"likes");
  await act(async()=>resolve(surface("foryou",{preview:true,previewState:"ready"})));expect(api.surface).toHaveBeenLastCalledWith("likes",expect.any(Number),false);expect(host.querySelector(".x-preview")).not.toHaveClass("has-selection");
});
it("uses increasing intent orders across collection changes",async()=>{const api=mock();await render(api);await render(api,true,false,"likes");await render(api);const orders=vi.mocked(api.surface).mock.calls.map(c=>c[1]);expect(orders[1]).toBeGreaterThan(orders[0]!);expect(orders[2]).toBeGreaterThan(orders[1]!);});
it("hides native panes beneath the account menu and marks it as an occlusion",async()=>{
  await render(mock());await click("계정");expect(host.querySelector('[data-native-overlay="true"][role="menu"]')).not.toBeNull();
});
it("does not steal Escape or focus from a foreground search dialog",async()=>{
  await render(mock());await click("계정");
  const dialog=document.createElement("div"),control=document.createElement("button");
  dialog.setAttribute("role","dialog");dialog.setAttribute("aria-modal","true");dialog.append(control);document.body.append(dialog);
  try{
    control.focus();const escape=new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true});
    await act(async()=>control.dispatchEvent(escape));
    expect(escape.defaultPrevented).toBe(false);expect(document.activeElement).toBe(control);
    expect(host.querySelector('[role="menu"]')).not.toBeNull();
  }finally{dialog.remove();}
});
it.each(["body","document","window"])("yields account-menu Escape on %s to a foreground modal after native focus loss",async targetName=>{
  const api=mock();await render(api);await click("계정");
  const dialog=document.createElement("div");dialog.setAttribute("role","dialog");dialog.setAttribute("aria-modal","true");document.body.append(dialog);
  try{
    const target=targetName==="body"?document.body:targetName==="document"?document:window;
    const escape=new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true});await act(async()=>target.dispatchEvent(escape));
    expect(escape.defaultPrevented).toBe(false);expect(host.querySelector('[role="menu"]')).not.toBeNull();expect(api.closePreview).not.toHaveBeenCalled();
  }finally{dialog.remove();}
});
it.each(['hidden','inert','aria-hidden="true"','style="display:none"','style="visibility:hidden"','style="opacity:0"'])("does not let a hidden modal block normal account-menu Escape: %s",async attrs=>{
  await render(mock());await click("계정");const wrapper=document.createElement("div");wrapper.innerHTML=`<div ${attrs}><div role="dialog" aria-modal="true">Concealed</div></div>`;document.body.append(wrapper);
  try{
    const escape=new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true});await act(async()=>document.body.dispatchEvent(escape));
    expect(escape.defaultPrevented).toBe(true);expect(host.querySelector('[role="menu"]')).toBeNull();expect(document.activeElement).toBe(button("계정"));
  }finally{wrapper.remove();}
});
it("portals only the floating account control into the left footer and keeps refresh inside its menu",async()=>{
  const target=document.createElement("div");document.body.append(target);
  try{
    await render(mock(),true,false,"explore",{accountPortalTarget:target});
    expect(host.querySelector('[aria-label="계정"]')).toBeNull();
    const anchor=target.querySelector<HTMLButtonElement>('[aria-label="계정"]')!;
    expect(anchor).not.toBeNull();expect(target.querySelector('[aria-label="새로고침"]')).toBeNull();
    expect(target.querySelector('[aria-label="현재 화면 보관"],[aria-label="미디어 다운로드"]')).toBeNull();
    await act(async()=>anchor.click());
    const menu=target.querySelector('[data-native-overlay="true"][role="menu"]')!;
    expect(menu).not.toBeNull();
    expect(menu.querySelector('[aria-label="@alice"]')).toHaveAttribute("aria-checked","true");
    expect(menu.querySelector('[aria-label="@bob"]')).toHaveAttribute("aria-checked","false");
    expect(menu.textContent).toContain("계정 추가");expect(menu.textContent).toContain("현재 계정 정보 복구");
    expect(menu.querySelector('[aria-label="새로고침"]')??[...menu.querySelectorAll("button")].find(node=>node.textContent==="새로고침")).toBeTruthy();
    await act(async()=>window.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape"})));
    expect(target.querySelector('[role="menu"]')).toBeNull();expect(document.activeElement).toBe(anchor);
    await render(mock(),true,true,"explore",{accountPortalTarget:target});
    expect(target.textContent).not.toContain("@alice");expect(target.querySelector('[role="menu"]')).toBeNull();
  }finally{await act(async()=>root.render(null));target.remove();}
});
const archiveMock=():XArchiveApi=>({
  inspect:vi.fn(),release:vi.fn().mockResolvedValue(true),save:vi.fn(),cancel:vi.fn(),list:vi.fn().mockResolvedValue([]),read:vi.fn(),openFolder:vi.fn(),openSource:vi.fn(),mediaUrl:vi.fn().mockReturnValue(""),
});
it("opens local storage inline without live X authentication, a native document or a dialog",async()=>{
  const api=mock(),archiveApi=archiveMock();vi.mocked(api.status).mockRejectedValue(new Error("offline"));
  await render(api,true,false,"local",{archiveApi});
  expect(host.querySelector('[aria-label="X 로컬 저장소"]')).not.toBeNull();
  expect(host.querySelector('[role="dialog"],.x-native-feed,.x-native-preview')).toBeNull();
  expect(archiveApi.list).toHaveBeenCalledOnce();expect(api.surface).not.toHaveBeenCalled();expect(api.account).not.toHaveBeenCalled();
  expect(host.textContent).not.toContain("offline");
});
it("drops late native results after entering local storage and reopens the selected collection on return",async()=>{
  const api=mock(),archiveApi=archiveMock(),opening=deferred<XSurfaceSnapshot>();
  vi.mocked(api.surface).mockReturnValueOnce(opening.promise);await render(api);
  await render(api,true,false,"local",{archiveApi});
  await act(async()=>opening.resolve(surface("foryou",{preview:true,previewState:"ready"})));
  expect(host.querySelector(".x-preview,.x-native-feed")).toBeNull();expect(archiveApi.list).toHaveBeenCalledOnce();
  await render(api,true,false,"bookmarks",{archiveApi});
  expect(host.querySelector('[aria-label="X 로컬 저장소"]')).toBeNull();
  expect(api.surface).toHaveBeenLastCalledWith("bookmarks",expect.any(Number),false);
});
it.each([{active:false,private:false},{active:true,private:true}])("never reads local data under hidden lifecycle %o",async mode=>{
  const api=mock(),archiveApi=archiveMock();
  await render(api,mode.active,mode.private,"local",{archiveApi});
  expect(archiveApi.list).not.toHaveBeenCalled();expect(api.surface).not.toHaveBeenCalled();
});
it("switches saved accounts in app and resets the old native document state",async()=>{
  const api=mock();await render(api);vi.mocked(api.status).mockResolvedValue({...account(),active:"two"});await click("계정");await click("@bob");expect(api.select).toHaveBeenCalledWith("two");expect(host.textContent).toContain("@bob");expect(api.account).not.toHaveBeenCalled();
});
it("opens login only by explicit action and resumes after native login closes",async()=>{
  vi.useFakeTimers();const api=mock();await render(api);await click("계정");await click("현재 계정 정보 복구");expect(api.account).toHaveBeenCalledWith(false);
  expect(host.textContent).toContain("공식 X 창에서 로그인");expect(button("계정")).toBeDisabled();expect(button("새로고침")).toBeUndefined();
  await act(async()=>vi.advanceTimersByTimeAsync(1300));expect(button("계정")).not.toBeDisabled();expect(api.surface).toHaveBeenCalledTimes(2);
});
it("exposes owner mismatch without clearing saved accounts or automatically changing authentication",async()=>{
  const api=mock();vi.mocked(api.surface).mockResolvedValue(surface("foryou",{state:"ownerChanged",error:"선택한 계정과 다릅니다"}));await render(api);
  expect(host.querySelector('[role="alert"]')).toHaveTextContent("선택한 계정");expect(api.select).not.toHaveBeenCalled();expect(api.account).not.toHaveBeenCalled();
});
it("caches lightweight status polling without reopening the timeline",async()=>{
  vi.useFakeTimers();const api=mock();await render(api);vi.mocked(api.surfaceStatus).mockResolvedValue(surface("foryou",{generation:2,preview:true,previewState:"ready"}));
  await act(async()=>vi.advanceTimersByTimeAsync(1300));expect(api.surface).toHaveBeenCalledOnce();expect(host.querySelector(".x-preview")).toHaveClass("has-selection");
});
it("revokes all native panes on unmount",async()=>{const api=mock();await render(api);const before=vi.mocked(api.suspend).mock.calls.length;await act(async()=>root.render(null));expect(api.suspend).toHaveBeenCalledTimes(before+1);});
it("reacts to native presentation signals without waiting for polling or accepting event payloads",async()=>{
  const api=mock();let changed=()=>{};const stop=vi.fn();api.subscribe=vi.fn(async cb=>{changed=cb;return stop;});await render(api);
  vi.mocked(api.surfaceStatus).mockResolvedValue(surface("foryou",{generation:5,preview:true,previewState:"loading"}));
  await act(async()=>changed());expect(host.querySelector('[aria-label="미리보기 불러오는 중"]')).not.toBeNull();expect(api.surface).toHaveBeenCalledOnce();
  await render(api,true,true);expect(stop).toHaveBeenCalled();
});
const portrait="data:image/jpeg;base64,YQ==";
it("shows observed current and saved-account portraits only for their matching registered handle",async()=>{
  const api=mock();vi.mocked(api.status).mockResolvedValue({...account(),presentations:{
    one:{handle:"alice",name:"앨리스",avatarData:portrait},
    two:{handle:"bob",name:"밥",avatarData:portrait},
  }});
  await render(api);
  expect(host.querySelector<HTMLImageElement>('.x-account-anchor img')?.getAttribute("src")).toBe(portrait);
  expect(host.querySelector(".x-account-anchor")).toHaveTextContent("@alice");
  await click("계정");
  expect(host.querySelector('[aria-label="@alice"]')).toHaveTextContent("앨리스");
  expect(host.querySelector('[aria-label="@alice"] img')).not.toBeNull();
  expect(host.querySelector('[aria-label="@bob"] img')).not.toBeNull();
  expect(host.querySelector('[aria-label="@bob"]')).toHaveTextContent("밥");
});
it("never displays another handle's cached portrait or a remote image URL",async()=>{
  const api=mock();vi.mocked(api.status).mockResolvedValue({...account(),presentations:{
    one:{handle:"mallory",name:"잘못된 계정",avatarData:portrait},
    two:{handle:"bob",name:"밥",avatarData:"https://remote.invalid/avatar.jpg"},
  }});
  await render(api);expect(host.querySelector(".x-account-anchor img")).toBeNull();
  expect(host.textContent).not.toContain("잘못된 계정");
  await click("계정");expect(host.querySelector('[aria-label="@alice"] img')).toBeNull();
  expect(host.querySelector('[aria-label="@bob"] img')).toBeNull();
});
it("removes account portraits and display names when privacy becomes active",async()=>{
  const api=mock();vi.mocked(api.status).mockResolvedValue({...account(),presentations:{
    one:{handle:"alice",name:"비공개 이름",avatarData:portrait},
  }});
  await render(api);expect(host.querySelector(".x-account-anchor img")).not.toBeNull();
  await click("계정");await render(api,true,true);
  expect(host.querySelector("img")).toBeNull();expect(host.textContent).not.toContain("비공개 이름");
  expect(host.textContent).not.toContain("@alice");
});
it("refreshes validated account presentation after a native event without reopening the feed",async()=>{
  const api=mock();let changed=()=>{};api.subscribe=vi.fn(async cb=>{changed=cb;return()=>{};});
  await render(api);const statusBefore=vi.mocked(api.status).mock.calls.length;
  vi.mocked(api.status).mockResolvedValue({...account(),presentations:{
    one:{handle:"alice",name:"새 표시 이름",avatarData:portrait},
  }});
  await act(async()=>changed());
  expect(vi.mocked(api.status).mock.calls.length).toBeGreaterThan(statusBefore);
  expect(host.querySelector<HTMLImageElement>('.x-account-anchor img')?.getAttribute("src")).toBe(portrait);
  await click("계정");expect(host.querySelector('[aria-label="@alice"]')).toHaveTextContent("새 표시 이름");
  expect(api.surface).toHaveBeenCalledOnce();
});
it("does not place post or media saving controls in React headers or the account footer",async()=>{
  const api=mock();vi.mocked(api.surface).mockResolvedValue(surface("foryou",{preview:true,previewState:"ready"}));
  await render(api);
  expect(host.querySelector(".x-official-toolbar,.x-sidebar-actions,.x-archive-actions")).toBeNull();
  for(const label of ["현재 화면 보관","미디어 다운로드","미리보기 보관","미리보기 미디어 다운로드"]){
    expect(button(label)).toBeUndefined();
  }
  expect(button("미리보기 닫기")).toBeDefined();
});
it("preserves a failed account switch error while restoring the existing account view",async()=>{
  const api=mock();await render(api);vi.mocked(api.select).mockRejectedValueOnce(new Error("계정 저장 오류"));await click("계정");await click("@bob");
  expect(host.querySelector('[role=alert]')).toHaveTextContent("계정 저장 오류");expect(host.textContent).toContain("@alice");
});
it("awaits initial reader suspension before account status or native surface work",async()=>{
  const api=mock(),closing=deferred<void>();vi.mocked(api.suspend).mockReturnValueOnce(closing.promise);await render(api);
  try{expect(api.suspend).toHaveBeenCalledOnce();expect(api.status).not.toHaveBeenCalled();expect(api.surface).not.toHaveBeenCalled();expect(host.querySelector('[aria-label="피드 불러오는 중"]')).not.toBeNull();}
  finally{await act(async()=>closing.resolve());}
  expect(api.status).toHaveBeenCalledOnce();expect(api.surface).toHaveBeenCalledOnce();
});
it("serializes StrictMode teardown before admitting the surviving setup",async()=>{
  const api=mock(),closing=deferred<void>();vi.mocked(api.suspend).mockReturnValueOnce(closing.promise);
  await act(async()=>root.render(<StrictMode><XWorkspace api={api} active privacyMode={false} runtime="browser-mock" onNotice={vi.fn()}/></StrictMode>));
  try{expect(api.status).not.toHaveBeenCalled();expect(api.surface).not.toHaveBeenCalled();}
  finally{await act(async()=>closing.resolve());}
  expect(api.suspend).toHaveBeenCalledTimes(3);expect(api.status).toHaveBeenCalledOnce();expect(api.surface).toHaveBeenCalledOnce();
});
it("waits for a prior component's suspension even when the next instance has a new API object",async()=>{
  const old=mock(),next=mock(),closing=deferred<void>();await render(old);vi.mocked(old.suspend).mockReturnValueOnce(closing.promise);
  await act(async()=>root.render(null));await render(next);
  try{expect(next.suspend).not.toHaveBeenCalled();expect(next.status).not.toHaveBeenCalled();expect(next.surface).not.toHaveBeenCalled();}
  finally{await act(async()=>closing.resolve());}
  expect(next.status).toHaveBeenCalledOnce();expect(next.surface).toHaveBeenCalledOnce();
});
it("does not reopen after rapid privacy return until outstanding native closure finishes",async()=>{
  const api=mock(),closing=deferred<void>();await render(api);vi.mocked(api.suspend).mockReturnValueOnce(closing.promise);
  await render(api,true,true);await render(api,true,false);
  try{expect(api.surface).toHaveBeenCalledOnce();}
  finally{await act(async()=>closing.resolve());}
  expect(api.surface).toHaveBeenCalledTimes(2);
});
it("fails closed when initial suspension rejects instead of starting another native document",async()=>{
  const api=mock();vi.mocked(api.suspend).mockRejectedValueOnce(new Error("이전 X 화면 정리 실패"));await render(api);
  expect(api.status).not.toHaveBeenCalled();expect(api.surface).not.toHaveBeenCalled();expect(host.querySelector('[role="alert"]')).toHaveTextContent("정리 실패");
});
it.each(["login","switch"])("awaits old native closure before explicit %s",async kind=>{
  const api=mock(),closing=deferred<void>();await render(api);vi.mocked(api.suspend).mockReturnValueOnce(closing.promise);
  await click("계정");await click(kind==="login"?"현재 계정 정보 복구":"@bob");
  try{expect(api.account).not.toHaveBeenCalled();expect(api.select).not.toHaveBeenCalled();}
  finally{await act(async()=>closing.resolve());}
  if(kind==="login")expect(api.account).toHaveBeenCalledWith(false);else expect(api.select).toHaveBeenCalledWith("two");
});
it("does not admit account work if privacy wins while native cleanup is pending",async()=>{
  const api=mock(),closing=deferred<void>();await render(api);vi.mocked(api.suspend).mockReturnValueOnce(closing.promise);
  await click("계정");await click("현재 계정 정보 복구");await render(api,true,true);await act(async()=>closing.resolve());
  expect(api.account).not.toHaveBeenCalled();expect(host.textContent).not.toContain("@alice");expect(host.querySelector(".x-native-feed")).toBeNull();
});
it("ignores older surface generations from a delayed status response",async()=>{
  vi.useFakeTimers();const api=mock();await render(api);
  vi.mocked(api.surfaceStatus).mockResolvedValueOnce(surface("foryou",{generation:3,preview:true,previewState:"ready"}));
  await act(async()=>vi.advanceTimersByTimeAsync(1300));
  vi.mocked(api.surfaceStatus).mockResolvedValueOnce(surface("foryou",{generation:2,preview:false}));
  await act(async()=>vi.advanceTimersByTimeAsync(1300));expect(host.querySelector(".x-preview")).toHaveClass("has-selection");
});
it.each([false,true])("ignores a late preview-close result from an earlier collection (failure=%s)",async failure=>{
  const api=mock(),closing=deferred<XSurfaceSnapshot>();vi.mocked(api.surface).mockResolvedValueOnce(surface("foryou",{preview:true,previewState:"ready"}));
  await render(api);vi.mocked(api.closePreview).mockReturnValueOnce(closing.promise);await click("미리보기 닫기");await render(api,true,false,"likes");
  await act(async()=>{if(failure)closing.reject(new Error("이전 미리보기 오류"));else closing.resolve(surface("foryou",{generation:9,preview:true,previewState:"ready"}));});
  expect(api.surface).toHaveBeenLastCalledWith("likes",expect.any(Number),false);expect(host.querySelector(".x-preview")).not.toHaveClass("has-selection");expect(host.textContent).not.toContain("이전 미리보기 오류");
});
it("ignores geometry callbacks after their observer has been disposed",async()=>{
  const updates:Array<()=>void>=[];vi.mocked(observeNativePlayerGeometry).mockImplementation((_elements,update)=>{updates.push(update);return()=>{};});
  const api=mock();await render(api);const stale=updates[0]!;await render(api,true,false,"likes");const count=vi.mocked(api.viewport).mock.calls.length;
  await act(async()=>stale());expect(api.viewport).toHaveBeenCalledTimes(count);
});
it("remeasures preview creation and readiness without concealing the playing feed",async()=>{
  const api=mock();let changed=()=>{};api.subscribe=vi.fn(async cb=>{changed=cb;return()=>{};});await render(api);
  const before=vi.mocked(api.viewport).mock.calls.length;
  vi.mocked(api.surfaceStatus).mockResolvedValueOnce(surface("foryou",{generation:2,preview:true,previewState:"loading"}));
  await act(async()=>changed());
  vi.mocked(api.surfaceStatus).mockResolvedValueOnce(surface("foryou",{generation:2,preview:true,previewState:"ready"}));
  await act(async()=>changed());
  const updates=vi.mocked(api.viewport).mock.calls.slice(before);expect(updates.length).toBeGreaterThan(0);
  expect(updates.every(([,viewport])=>viewport.visible&&viewport.player!==null)).toBe(true);
});
it("still sends hidden geometry on actual workspace unmount",async()=>{
  const api=mock();await render(api);await act(async()=>root.render(null));
  expect(api.viewport).toHaveBeenLastCalledWith(1,expect.objectContaining({visible:false,player:null,chat:null}));
});
it("conceals rather than resets retained feed and preview sessions across local and platform views",async()=>{
  const api=mock(),archiveApi=archiveMock();let ids=['one','two'],activeId='two',generation=1;
  const sourceDocument={scrollTop:1234,route:'/home'};
  vi.mocked(api.suspend).mockImplementation(async retain=>{if(!retain){ids=[];sourceDocument.scrollTop=0;}generation++;});
  vi.mocked(api.surface).mockImplementation(async(feed,_order,refresh)=>{
    if(refresh)sourceDocument.scrollTop=0;
    return surface(feed,{generation:++generation,preview:ids.length>0,previewState:ids.length?'ready':'empty',activePreviewId:ids.length?activeId:null,
      previews:ids.map(id=>({id,path:`/${id}/status/123456`,title:id,state:'ready'}))});
  });
  await render(api);expect(host.querySelectorAll('[role=tab]')).toHaveLength(2);
  await render(api,true,false,'local',{archiveApi});expect(api.suspend).toHaveBeenLastCalledWith(true);
  await render(api);expect(host.querySelectorAll('[role=tab]')).toHaveLength(2);
  expect(host.querySelector('[role=tab][aria-selected=true]')).toHaveTextContent('two');
  await render(api,false);expect(api.suspend).toHaveBeenLastCalledWith(true);
  await render(api);expect(host.querySelectorAll('[role=tab]')).toHaveLength(2);expect(sourceDocument.scrollTop).toBe(1234);
  expect(vi.mocked(api.surface).mock.calls.every(([, ,refresh])=>!refresh)).toBe(true);
  expect(vi.mocked(api.suspend).mock.calls.every(([retain])=>retain===true)).toBe(true);
  // Privacy is an explicit invalidation, not an ordinary hidden tab.
  await render(api,true,true);expect(api.suspend).toHaveBeenLastCalledWith(false);
  await render(api);expect(host.querySelectorAll('[role=tab]')).toHaveLength(0);expect(sourceDocument.scrollTop).toBe(0);
});
it("still invalidates the retained session for explicit account operations",async()=>{
  const api=mock();await render(api);await click('계정');await click('@bob');
  expect(vi.mocked(api.suspend).mock.calls.some(([retain])=>retain===false)).toBe(true);
  expect(api.select).toHaveBeenCalledWith('two');
});
const retained=(activeId="two",generation=3,ids=["one","two","three"])=>surface("foryou",{
  generation,preview:ids.length>0,previewState:ids.length?"ready":"empty",activePreviewId:ids.length?activeId:null,
  previews:ids.map(id=>({id,path:`/${id}/status/123456`,title:`@${id} · 게시물`,state:"ready"})),
});
it("selects retained preview ids without reopening the feed",async()=>{
  const api=mock();vi.mocked(api.surface).mockResolvedValueOnce(retained());vi.mocked(api.selectPreview).mockResolvedValueOnce(retained("one",4));
  await render(api);
  const left=host.querySelector(".x-native-feed"),right=host.querySelector(".x-native-preview");
  await click("@one · 게시물");expect(api.selectPreview).toHaveBeenCalledExactlyOnceWith(3,"one");
  expect(host.querySelector('[role=tab][aria-selected=true]')).toHaveTextContent("@one");
  expect(host.querySelector(".x-native-feed")).toBe(left);expect(host.querySelector(".x-native-preview")).toBe(right);
  expect(api.surface).toHaveBeenCalledOnce();
});
it("sends same-active reselection to native so a pending source-open cannot override the latest user intent",async()=>{
  const api=mock();vi.mocked(api.surface).mockResolvedValueOnce(retained());
  vi.mocked(api.selectPreview).mockResolvedValueOnce(retained());await render(api);
  await click("@two · 게시물");
  expect(api.selectPreview).toHaveBeenCalledExactlyOnceWith(3,"two");
  expect(host.querySelector('[role=tab][aria-selected=true]')).toHaveTextContent("@two");
  expect(host.querySelectorAll('[role=tab]')).toHaveLength(3);expect(api.surface).toHaveBeenCalledOnce();
  expect(api.closePreview).not.toHaveBeenCalled();
});
it("closes inactive, active and last tabs using the native retained selection without a confirmation dialog",async()=>{
  const api=mock();vi.mocked(api.surface).mockResolvedValueOnce(retained());
  vi.mocked(api.closePreview).mockResolvedValueOnce(retained("two",4,["one","two"]))
    .mockResolvedValueOnce(retained("one",5,["one"])).mockResolvedValueOnce(retained("",6,[]));
  await render(api);await click("@three · 게시물 닫기");
  expect(api.closePreview).toHaveBeenLastCalledWith(3,"three");expect(host.querySelectorAll('[role=tab]')).toHaveLength(2);
  expect(host.querySelector('[role=tab][aria-selected=true]')).toHaveTextContent("@two");
  await click("@two · 게시물 닫기");expect(api.closePreview).toHaveBeenLastCalledWith(4,"two");
  expect(host.querySelector('[role=tab][aria-selected=true]')).toHaveTextContent("@one");
  await click("@one · 게시물 닫기");expect(api.closePreview).toHaveBeenLastCalledWith(5,"one");
  expect(host.querySelector('[role=tablist],[role=dialog]')).toBeNull();expect(host.textContent).toContain("게시물을 선택하면");
  expect(api.surface).toHaveBeenCalledOnce();
});
it("admits one preview operation at a time without concealing the unrelated feed",async()=>{
  const api=mock(),selection=deferred<XSurfaceSnapshot>();vi.mocked(api.surface).mockResolvedValueOnce(retained());
  vi.mocked(api.selectPreview).mockReturnValueOnce(selection.promise);await render(api);const before=vi.mocked(api.viewport).mock.calls.length;
  await click("@one · 게시물");expect(button("@three · 게시물")).toBeDisabled();await click("@three · 게시물 닫기");
  expect(api.closePreview).not.toHaveBeenCalled();expect(api.selectPreview).toHaveBeenCalledOnce();
  expect(vi.mocked(api.viewport).mock.calls.slice(before).every(([,value])=>value.visible)).toBe(true);
  await act(async()=>selection.resolve(retained("one",4)));expect(button("@three · 게시물")).not.toBeDisabled();
});
it.each(["privacy","local","collection","inactive"] as const)("drops a delayed preview selection after %s transition",async transition=>{
  const api=mock(),selection=deferred<XSurfaceSnapshot>();vi.mocked(api.surface).mockResolvedValueOnce(retained());
  vi.mocked(api.selectPreview).mockReturnValueOnce(selection.promise);await render(api);await click("@one · 게시물");
  if(transition==="collection")vi.mocked(api.surface).mockResolvedValueOnce(surface("likes",{generation:4}));
  await render(api,transition!=="inactive",transition==="privacy",transition==="local"?"local":transition==="collection"?"likes":"explore",{archiveApi:archiveMock()});
  await act(async()=>selection.resolve(retained("one",90)));
  expect(host.querySelector('.x-preview-tabs [role=tab]')).toBeNull();expect(host.textContent).not.toContain("@one · 게시물");
});
it("does not replace a newly opened native tab with an older in-flight selection result or stale error",async()=>{
  const api=mock(),selection=deferred<XSurfaceSnapshot>();let changed=()=>{};
  api.subscribe=vi.fn(async cb=>{changed=cb;return()=>{};});
  vi.mocked(api.surface).mockResolvedValueOnce(retained());vi.mocked(api.selectPreview).mockReturnValueOnce(selection.promise);
  await render(api);await click("@one · 게시물");
  vi.mocked(api.surfaceStatus).mockResolvedValueOnce(retained("four",9,["one","two","three","four"]));await act(async()=>changed());
  await act(async()=>selection.resolve(retained("one",4)));
  expect(host.querySelector('[role=tab][aria-selected=true]')).toHaveTextContent("@four");expect(host.querySelectorAll('[role=tab]')).toHaveLength(4);
});
it("keeps a first background tab reachable while leaving the right document empty until explicit selection",async()=>{
  const api=mock();vi.mocked(api.surface).mockResolvedValueOnce({...retained("",3,["one"]),preview:false,previewState:"empty",activePreviewId:null});
  vi.mocked(api.selectPreview).mockResolvedValueOnce(retained("one",4,["one"]));await render(api);
  expect(host.querySelector(".x-preview")).toHaveClass("has-selection");expect(button("@one · 게시물")).toHaveAttribute("tabindex","0");
  expect(button("@one · 게시물")).toHaveAttribute("aria-selected","false");expect(host.textContent).toContain("게시물을 선택하면");
  expect(host.querySelector('[role=tabpanel]')).toBeNull();expect(api.selectPreview).not.toHaveBeenCalled();
  await click("@one · 게시물");expect(api.selectPreview).toHaveBeenCalledExactlyOnceWith(3,"one");
  expect(host.querySelector('[role=tabpanel]')).not.toBeNull();
});
it("adds a background tab without selecting it or disturbing the existing preview",async()=>{
  const api=mock();let changed=()=>{};api.subscribe=vi.fn(async cb=>{changed=cb;return()=>{};});
  vi.mocked(api.surface).mockResolvedValueOnce(retained("one",3,["one"]));await render(api);
  vi.mocked(api.surfaceStatus).mockResolvedValueOnce(retained("one",4,["one","two"]));await act(async()=>changed());
  expect(host.querySelectorAll('[role=tab]')).toHaveLength(2);expect(host.querySelector('[role=tab][aria-selected=true]')).toHaveTextContent("@one");
  expect(api.selectPreview).not.toHaveBeenCalled();expect(api.surface).toHaveBeenCalledOnce();
});
