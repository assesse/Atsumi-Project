import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { XArchiveLibrary, XArchiveReader, XArchiveTools } from "./XArchivePanel";
import { createXArchiveApi, isXArchiveLocalUrl, isXArchiveProgress, type XArchiveApi, type XArchiveAsset, type XArchiveDraft, type XArchiveRecord, type XArchiveSummary, type XCapturePost, type XArchiveProgress } from "./xArchiveApi";
import { auditLayouts, layoutBrowser, type LayoutScene } from "../../test/offlineLayoutBrowser";
import css from "./XArchivePanel.css?raw";

vi.mock("@tauri-apps/api/core",()=>({invoke:vi.fn(),convertFileSrc:(path:string,protocol:string)=>`http://${protocol}.localhost/${encodeURIComponent(path)}`}));
const hls=vi.hoisted(()=>({supported:false,instances:[] as Array<{config:{xhrSetup:(xhr:XMLHttpRequest,url:string)=>void;fetchSetup:(context:{url:string},options:RequestInit)=>Request};loadSource:ReturnType<typeof vi.fn>;destroy:ReturnType<typeof vi.fn>}>}));
vi.mock("hls.js",()=>({default:class{
  static Events={ERROR:"error",MANIFEST_PARSED:"parsed"};static isSupported(){return hls.supported;}
  loadSource=vi.fn();destroy=vi.fn();attachMedia=vi.fn();on=vi.fn();
  constructor(readonly config:typeof hls.instances[number]["config"]){hls.instances.push(this);}
}}));
const ID="11111111-1111-4111-8111-111111111111",ID2="22222222-2222-4222-8222-222222222222",CAPTURE="33333333-3333-4333-8333-333333333333";
const IMAGE=`asset-${ID}.jpg`,VIDEO=`asset-${ID2}.mp4`,HLS=`asset-${CAPTURE}.m3u8`,VTT=`asset-${ID2}.vtt`;
const post=(id="10",options:Partial<XCapturePost>={}):XCapturePost=>({id,url:`https://x.com/reader/status/${id}`,author:`Reader ${id}`,body:`본문 ${id}`,timestamp:"2026-10-20T00:00:00Z",counts:"좋아요 1개",context:"",quotes:[],media:[{kind:"image",url:"https://pbs.twimg.com/media/sample.jpg",alt:"sample"}],truncated:false,...options});
const draft=():XArchiveDraft=>({captureId:CAPTURE,accountId:"saved-account",document:{version:1,owner:"reader",sourceUrl:"https://x.com/reader/status/10",capturedAt:"2026-10-20T00:01:00Z",coverage:"loaded_dom_only",records:[post(),post("20"),post("30",{media:[]})],limitReached:false}});
const asset=(name=IMAGE,options:Partial<XArchiveAsset>={}):XArchiveAsset=>({name,mime:"image/jpeg",bytes:1024,sha256:"a".repeat(64),postId:"10",kind:"image",sourceUrl:"https://pbs.twimg.com/media/sample.jpg",...options});
const record=(options:Partial<XArchiveRecord>={}):XArchiveRecord=>({version:1,id:ID,captureId:CAPTURE,accountId:"saved-account",savedAt:1792454520000,intent:"archive",document:{...draft().document,records:[post()]},assets:[asset()],issues:[],selectedContentComplete:true,mediaRequested:true,htmlSha256:"b".repeat(64),...options});
const summary=(id=ID):XArchiveSummary=>({id,sourceUrl:"https://x.com/reader/status/10",savedAt:1792454520000,capturedAt:"2026-10-20T00:01:00Z",posts:1,assets:1,issues:0,intent:"archive",selectedContentComplete:true});
const mock=():XArchiveApi=>({inspect:vi.fn().mockResolvedValue(draft()),release:vi.fn().mockResolvedValue(true),save:vi.fn<XArchiveApi["save"]>().mockImplementation(async request=>record({intent:request.intent,mediaRequested:request.includeMedia,document:{...draft().document,records:draft().document.records.filter(post=>request.selectedIds.includes(post.id))},assets:request.includeMedia?[asset()]:[]})),cancel:vi.fn().mockResolvedValue(true),list:vi.fn().mockResolvedValue([summary()]),read:vi.fn().mockResolvedValue(record()),openFolder:vi.fn().mockResolvedValue(undefined),openSource:vi.fn().mockResolvedValue(undefined),mediaUrl:vi.fn(createXArchiveApi("tauri").mediaUrl)});
const mediaMock=(saved=record()):XArchiveApi=>{const api=mock();vi.mocked(api.list).mockResolvedValue([{...summary(saved.id),intent:"media",accountId:saved.accountId}]);vi.mocked(api.read).mockResolvedValue({...saved,intent:"media"});return api;};
const deferred=<T,>()=>{let resolve!:(value:T)=>void,reject!:(reason:unknown)=>void;const promise=new Promise<T>((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};};
let host:HTMLDivElement,root:Root;
beforeEach(()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);
  hls.supported=false;hls.instances=[];
  vi.spyOn(HTMLMediaElement.prototype,"play").mockResolvedValue();vi.spyOn(HTMLMediaElement.prototype,"pause").mockImplementation(()=>{});vi.spyOn(HTMLMediaElement.prototype,"load").mockImplementation(()=>{});
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.useRealTimers();vi.restoreAllMocks();vi.clearAllMocks();vi.unstubAllGlobals();});
const render=(api:XArchiveApi,options:{contextKey?:string;generation?:number|null;active?:boolean;privacyMode?:boolean;disabled?:boolean}={})=>act(async()=>root.render(<XArchiveTools runtime="browser-mock" api={api} pane="preview" generation={1} contextKey="one" {...options}/>));
const button=(label:string)=>[...document.querySelectorAll<HTMLButtonElement>("button")].find(item=>item.getAttribute("aria-label")===label||item.textContent===label)!;
const click=(label:string)=>act(async()=>button(label).click());
const checkbox=(id:string)=>document.querySelector<HTMLInputElement>(`input[aria-label$="#${id}"]`)!;
const renderLibrary=(api:XArchiveApi,options:{active?:boolean;privacyMode?:boolean}={})=>act(async()=>root.render(<XArchiveLibrary runtime="browser-mock" api={api} {...options}/>));
const openLibraryFirst=()=>act(async()=>host.querySelector<HTMLButtonElement>('.x-archive-timeline-open')!.click());
const presentation=(options:Partial<NonNullable<XCapturePost["presentation"]>>={}):NonNullable<XCapturePost["presentation"]>=>({
  handle:"reader",displayName:"우엥",verified:null,counts:{replies:"4",reposts:"456",likes:"3.5천",bookmarks:"1.4천",views:"85.5만"},relation:"root",conversationId:"10",order:0,...options,
});
const contextMenu=async(target:Element,coordinates={clientX:200,clientY:180})=>{
  const event=new MouseEvent("contextmenu",{bubbles:true,cancelable:true,...coordinates});
  await act(async()=>target.dispatchEvent(event));return event;
};
const libraryRows=(count:number)=>{
  const api=mock(),ids=Array.from({length:count},(_,index)=>`11111111-1111-4111-8111-${String(index+1).padStart(12,"0")}`);
  vi.mocked(api.list).mockResolvedValue(ids.map(id=>({...summary(id),intent:"media",accountId:"saved-account"})));
  vi.mocked(api.read).mockImplementation(async id=>record({id,intent:"media"}));return{api,ids};
};
const mediaViewport=(height=500)=>{
  const geometry={height,rowHeight:180};
  vi.spyOn(HTMLElement.prototype,"clientHeight","get").mockImplementation(function(this:HTMLElement){return this.classList.contains("x-archive-timeline")?geometry.height:0;});
  vi.spyOn(HTMLElement.prototype,"scrollHeight","get").mockImplementation(function(this:HTMLElement){return this.classList.contains("x-archive-timeline")?Math.ceil(this.querySelectorAll(".x-archive-media-tile").length/3)*geometry.rowHeight:0;});
  return geometry;
};
const scrollLibrary=(top:number)=>act(async()=>{const viewport=host.querySelector<HTMLElement>('.x-archive-timeline')!;viewport.scrollTop=top;viewport.dispatchEvent(new Event("scroll"));});

it("fills the measured media viewport in bounded batches without decoding the entire library",async()=>{
  mediaViewport();const{api,ids}=libraryRows(20);await renderLibrary(api);expect(api.read).not.toHaveBeenCalled();await click("미디어");
  expect(vi.mocked(api.read).mock.calls.map(call=>call[0])).toEqual(ids.slice(0,8));expect(host.querySelectorAll('.x-archive-media-tile')).toHaveLength(8);
  const viewport=host.querySelector<HTMLElement>('.x-archive-timeline')!;expect(viewport.scrollHeight).toBeGreaterThan(viewport.clientHeight);expect(button("더 보기")).not.toBeDisabled();
  expect(host.querySelector('.x-archive-preview')).not.toBeNull();expect(api.inspect).not.toHaveBeenCalled();expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
});
it("loads near the bottom automatically and coalesces duplicate scroll events",async()=>{
  mediaViewport();const{api,ids}=libraryRows(20);await renderLibrary(api);await click("미디어");
  await scrollLibrary(20);expect(vi.mocked(api.read).mock.calls.map(call=>call[0])).toEqual(ids.slice(0,16));
  // 16 tiles occupy 1080 px: this position is well away from the next threshold.
  await scrollLibrary(21);expect(api.read).toHaveBeenCalledTimes(16);
  await act(async()=>{const viewport=host.querySelector<HTMLElement>('.x-archive-timeline')!;viewport.scrollTop=580;for(let index=0;index<10;index++)viewport.dispatchEvent(new Event("scroll"));});
  expect(vi.mocked(api.read).mock.calls.map(call=>call[0])).toEqual(ids);expect(button("더 보기")).toBeUndefined();
  await scrollLibrary(1000);expect(api.read).toHaveBeenCalledTimes(20);
});
it("fills a larger resized view and resumes a previously zero-height view without polling",async()=>{
  const geometry=mediaViewport(0),{api}=libraryRows(20);await renderLibrary(api);await click("미디어");expect(api.read).toHaveBeenCalledTimes(4);
  geometry.height=500;await act(async()=>window.dispatchEvent(new Event("resize")));expect(api.read).toHaveBeenCalledTimes(8);
  geometry.height=900;await act(async()=>window.dispatchEvent(new Event("resize")));expect(api.read).toHaveBeenCalledTimes(16);
  geometry.height=250;await act(async()=>window.dispatchEvent(new Event("resize")));expect(api.read).toHaveBeenCalledTimes(16);
});
it("maintains at most two underlying manifest reads during automatic viewport fill",async()=>{
  mediaViewport(900);const{api,ids}=libraryRows(12),requests=new Map<string,ReturnType<typeof deferred<XArchiveRecord>>>();let running=0,maximum=0;
  vi.mocked(api.read).mockImplementation(id=>{const pending=deferred<XArchiveRecord>();requests.set(id,pending);running++;maximum=Math.max(maximum,running);return pending.promise.finally(()=>{running--;});});
  await renderLibrary(api);await click("미디어");expect(api.read).toHaveBeenCalledTimes(2);
  for(let batch=0;batch<6;batch++)await act(async()=>{for(const[id,pending]of[...requests]){requests.delete(id);pending.resolve(record({id,intent:"media"}));}});
  expect(maximum).toBe(2);expect(vi.mocked(api.read).mock.calls.map(call=>call[0])).toEqual(ids);expect(running).toBe(0);expect(button("더 보기")).toBeUndefined();
});
it("stops auto-fill on a failed item and preserves explicit retry and more controls",async()=>{
  mediaViewport();const{api,ids}=libraryRows(12);vi.mocked(api.read).mockImplementation(async id=>{if(id===ids[0])throw new Error("손상된 항목");return record({id,intent:"media"});});
  await renderLibrary(api);await click("미디어");expect(api.read).toHaveBeenCalledTimes(4);expect(button("더 보기")).not.toBeDisabled();
  await act(async()=>window.dispatchEvent(new Event("resize")));await scrollLibrary(1000);expect(api.read).toHaveBeenCalledTimes(4);
  await click("더 보기");expect(api.read).toHaveBeenCalledTimes(8);expect(host.textContent).toContain("손상된 항목");
  vi.mocked(api.read).mockImplementation(async id=>record({id,intent:"media"}));await click("다시 불러오기");
  expect(api.read).toHaveBeenCalledWith(ids[0]);expect(host.textContent).not.toContain("손상된 항목");expect(api.read).toHaveBeenCalledTimes(13);
});
it("ends empty lists and asset-less finite lists without a read or resize loop",async()=>{
  mediaViewport();const empty=libraryRows(0);await renderLibrary(empty.api);await click("미디어");await act(async()=>window.dispatchEvent(new Event("resize")));expect(empty.api.read).not.toHaveBeenCalled();
  const{api,ids}=libraryRows(9);vi.mocked(api.read).mockImplementation(async id=>record({id,intent:"media",assets:[]}));await renderLibrary(api);
  expect(vi.mocked(api.read).mock.calls.map(call=>call[0])).toEqual(ids);expect(host.querySelectorAll('.x-archive-media-tile')).toHaveLength(0);expect(button("더 보기")).toBeUndefined();
  await act(async()=>window.dispatchEvent(new Event("resize")));expect(api.read).toHaveBeenCalledTimes(9);
});
it.each([{privacyMode:true},{active:false}])("does no additional auto-fill after library lifecycle concealment %o",async options=>{
  mediaViewport();const{api}=libraryRows(20);await renderLibrary(api);await click("미디어");expect(api.read).toHaveBeenCalledTimes(8);
  await renderLibrary(api,options);await act(async()=>{window.dispatchEvent(new Event("resize"));document.dispatchEvent(new Event("visibilitychange"));});
  expect(api.read).toHaveBeenCalledTimes(8);expect(host.querySelector('.x-archive-library-view')).toBeNull();expect(api.inspect).not.toHaveBeenCalled();
});
it("does not auto-read while the document is hidden and resumes measured fill on visibility return",async()=>{
  const geometry=mediaViewport();let visible=false;vi.spyOn(document,"visibilityState","get").mockImplementation(()=>visible?"visible":"hidden");
  const{api}=libraryRows(20);await renderLibrary(api);await click("미디어");expect(api.read).toHaveBeenCalledTimes(4);
  geometry.height=900;await act(async()=>window.dispatchEvent(new Event("resize")));expect(api.read).toHaveBeenCalledTimes(4);
  visible=true;await act(async()=>document.dispatchEvent(new Event("visibilitychange")));expect(api.read).toHaveBeenCalledTimes(16);
});

it("replaces the saved-body default menu with one black external action without opening anything",async()=>{
  const api=mock();await renderLibrary(api);
  const event=await contextMenu(host.querySelector('.x-archive-body')!);
  expect(event.defaultPrevented).toBe(true);
  const menu=document.querySelector('.x-archive-context-menu')!;
  expect(menu).toHaveAttribute("role","menu");expect(menu.querySelectorAll('[role="menuitem"]')).toHaveLength(1);
  expect(menu.textContent).toBe("외부 브라우저에서 열기");
  expect(api.openSource).not.toHaveBeenCalled();expect(api.inspect).not.toHaveBeenCalled();expect(api.save).not.toHaveBeenCalled();
  expect(host.querySelector('.x-archive-preview .x-archive-reader')).toBeNull();
  await click("외부 브라우저에서 열기");
  expect(api.openSource).toHaveBeenCalledExactlyOnceWith(ID,"10");expect(document.querySelector('[role="menu"]')).toBeNull();
});
it("suppresses only the default menu on local library headers, margins and empty preview",async()=>{
  const api=mock();await renderLibrary(api);
  for(const selector of [".x-archive-library-view",".x-archive-timeline-heading",'[role="tablist"]',".x-archive-timeline",".x-archive-preview"]){
    const event=await contextMenu(host.querySelector(selector)!);
    expect(event.defaultPrevented).toBe(true);expect(document.querySelector('[role="menu"]')).toBeNull();
  }
  expect(api.openSource).not.toHaveBeenCalled();
  const outside=document.createElement("button");document.body.append(outside);
  try{expect((await contextMenu(outside)).defaultPrevented).toBe(false);}
  finally{outside.remove();}
});
it("suppresses the default menu on an empty local list without inventing a post action",async()=>{
  const api=mock();vi.mocked(api.list).mockResolvedValueOnce([]);await renderLibrary(api);
  const empty=host.querySelector(".x-archive-empty")!;
  expect(empty).toHaveTextContent("보관된 항목이 없습니다.");
  expect((await contextMenu(empty)).defaultPrevented).toBe(true);expect(document.querySelector('[role="menu"]')).toBeNull();
  expect(api.openSource).not.toHaveBeenCalled();expect(api.read).not.toHaveBeenCalled();
});
it("dismisses a post menu on blank-area right-click without binding the blank area to that post",async()=>{
  const api=mock();await renderLibrary(api);await openLibraryFirst();
  await contextMenu(host.querySelector(".x-archive-preview .x-archive-body")!);
  expect(document.querySelector('[role="menu"]')).not.toBeNull();
  expect((await contextMenu(host.querySelector(".x-archive-timeline-heading")!)).defaultPrevented).toBe(true);
  expect(document.querySelector('[role="menu"]')).toBeNull();expect(api.openSource).not.toHaveBeenCalled();
  expect(host.querySelector(".x-archive-preview .x-archive-reader")).not.toBeNull();
});
it("suppresses the default menu in standalone reader information and margins while retaining exact post menus",async()=>{
  const api=mock();await act(async()=>root.render(<XArchiveReader record={record()} api={api}/>));
  for(const selector of [".x-archive-reader",".x-archive-reader-info",".x-archive-reply-composer"]){
    expect((await contextMenu(host.querySelector(selector)!)).defaultPrevented).toBe(true);
    expect(document.querySelector('[role="menu"]')).toBeNull();
  }
  await contextMenu(host.querySelector(".x-archive-body")!);await click("외부 브라우저에서 열기");
  expect(api.openSource).toHaveBeenCalledExactlyOnceWith(ID,"10");
});
it("opens the exact saved reply from its right-reader menu, not the root or live cache",async()=>{
  const api=mock(),saved=record({assets:[],document:{...draft().document,records:[post("10",{presentation:presentation()}),post("20",{presentation:presentation({relation:"reply",order:1})})]}});
  await act(async()=>root.render(<XArchiveReader record={saved} api={api}/>));
  await contextMenu(host.querySelector('[data-post-id="20"] .x-archive-body')!);await click("외부 브라우저에서 열기");
  expect(api.openSource).toHaveBeenCalledExactlyOnceWith(ID,"20");
  expect(api.inspect).not.toHaveBeenCalled();expect(api.read).not.toHaveBeenCalled();expect(api.save).not.toHaveBeenCalled();
});
it("keeps only one local menu and retains normal body-to-preview navigation",async()=>{
  const api=mock();await renderLibrary(api);await openLibraryFirst();
  await contextMenu(host.querySelector('.x-archive-timeline .x-archive-body')!);
  await contextMenu(host.querySelector('.x-archive-preview .x-archive-body')!);
  expect(document.querySelectorAll('.x-archive-context-menu')).toHaveLength(1);
  await act(async()=>window.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));
  expect(document.querySelector('.x-archive-context-menu')).toBeNull();
  expect(host.querySelector('.x-archive-preview .x-archive-reader')).not.toBeNull();
  await click("미리보기 닫기");await openLibraryFirst();expect(host.querySelector('.x-archive-preview .x-archive-reader')).not.toBeNull();
  expect(api.openSource).not.toHaveBeenCalled();
});
it.each(["pointerdown","scroll","resize","blur"])("dismisses a local menu on outside %s without an external action",async type=>{
  const api=mock();await renderLibrary(api);await contextMenu(host.querySelector('.x-archive-body')!);
  await act(async()=>window.dispatchEvent(new Event(type,{bubbles:true})));
  expect(document.querySelector('.x-archive-context-menu')).toBeNull();expect(api.openSource).not.toHaveBeenCalled();
});
it("restores the invoking button on Escape without closing the preview",async()=>{
  const api=mock();await renderLibrary(api);
  const source=host.querySelector<HTMLButtonElement>('.x-archive-timeline-open')!;source.focus();
  await contextMenu(source,{clientX:0,clientY:0});expect(document.activeElement).toBe(button("외부 브라우저에서 열기"));
  const escape=new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true});
  await act(async()=>document.activeElement!.dispatchEvent(escape));
  expect(escape.defaultPrevented).toBe(true);expect(document.activeElement).toBe(source);expect(api.openSource).not.toHaveBeenCalled();
});
it("uses the same menu on media tiles while preserving ordinary selection",async()=>{
  const api=mediaMock();await renderLibrary(api);await click("미디어");
  const tile=host.querySelector<HTMLButtonElement>('.x-archive-media-tile')!;
  await contextMenu(tile);expect(host.querySelector('.x-archive-preview .x-archive-reader')).toBeNull();
  await click("외부 브라우저에서 열기");expect(api.openSource).toHaveBeenCalledExactlyOnceWith(ID,"10");
  await act(async()=>tile.click());expect(host.querySelector('.x-archive-preview .x-archive-reader')).not.toBeNull();
});
it("gives the image menu Escape priority then preserves local image navigation and close",async()=>{
  const api=mock(),saved=record({assets:[asset(),asset(`asset-${ID2}.jpg`,{sourceUrl:"https://pbs.twimg.com/media/second.jpg"})]});
  await act(async()=>root.render(<XArchiveReader record={saved} api={api}/>));await click("이미지 1 크게 보기");
  const viewer=document.querySelector('.x-archive-image-viewer')!;
  await contextMenu(viewer.querySelector("img")!);expect(document.querySelector('.x-archive-context-menu')).not.toBeNull();
  await act(async()=>button("외부 브라우저에서 열기").dispatchEvent(new KeyboardEvent("keydown",{key:"d",bubbles:true,cancelable:true})));
  expect(viewer.querySelector("img")).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${IMAGE}`);
  await act(async()=>button("외부 브라우저에서 열기").dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));
  expect(document.querySelector('.x-archive-context-menu')).toBeNull();expect(document.querySelector('.x-archive-image-viewer')).toBe(viewer);
  await act(async()=>button("이미지 닫기").dispatchEvent(new KeyboardEvent("keydown",{key:"d",bubbles:true,cancelable:true})));
  expect(viewer.querySelector("img")).toHaveAttribute("src",`http://x-archive.localhost/${ID}/asset-${ID2}.jpg`);
  await act(async()=>button("이미지 닫기").dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));
  expect(document.querySelector('.x-archive-image-viewer')).toBeNull();expect(api.openSource).not.toHaveBeenCalled();
});
it.each([{privacyMode:true},{active:false}])("removes a local menu and rejects its old button on lifecycle change %o",async options=>{
  const api=mock();await renderLibrary(api);await contextMenu(host.querySelector('.x-archive-body')!);
  const stale=button("외부 브라우저에서 열기");await renderLibrary(api,options);await act(async()=>stale.click());
  expect(document.querySelector('.x-archive-context-menu')).toBeNull();expect(api.openSource).not.toHaveBeenCalled();
});
it.each(["record","api"])("drops the stale menu after its %s is replaced",async change=>{
  const api=mock(),saved=record();await act(async()=>root.render(<XArchiveReader record={saved} api={api}/>));
  await contextMenu(host.querySelector('.x-archive-body')!);const stale=button("외부 브라우저에서 열기"),replacement=mock();
  await act(async()=>root.render(<XArchiveReader record={change==="record"?record({id:ID2}):saved} api={change==="api"?replacement:api}/>));
  await act(async()=>stale.click());expect(document.querySelector('.x-archive-context-menu')).toBeNull();
  expect(api.openSource).not.toHaveBeenCalled();expect(replacement.openSource).not.toHaveBeenCalled();
});
it.each(["removed","hidden"])("refuses a menu action when its original source becomes %s",async mode=>{
  const api=mock();await renderLibrary(api);const body=host.querySelector<HTMLElement>('.x-archive-body')!;
  await contextMenu(body);
  const parent=body.parentElement!,next=body.nextSibling;
  if(mode==="removed")body.remove();else parent.hidden=true;
  await click("외부 브라우저에서 열기");expect(api.openSource).not.toHaveBeenCalled();
  if(mode==="removed")parent.insertBefore(body,next);else parent.hidden=false;
});
it.each(["https://evil.test/reader/status/10","https://x.com/reader/status/20","https://x.com/reader/status/10?x=1"])("does not expose an opener for an invalid stored source %s",async url=>{
  const api=mock();await act(async()=>root.render(<XArchiveReader record={record({document:{...draft().document,records:[post("10",{url})]}})} api={api}/>));
  const event=await contextMenu(host.querySelector('.x-archive-body')!);
  expect(event.defaultPrevented).toBe(true);expect(document.querySelector('.x-archive-context-menu')).toBeNull();expect(api.openSource).not.toHaveBeenCalled();
});
it("clamps the measured black menu within the viewport rather than clipping it",async()=>{
  const width=vi.spyOn(HTMLElement.prototype,"offsetWidth","get").mockReturnValue(232),height=vi.spyOn(HTMLElement.prototype,"offsetHeight","get").mockReturnValue(50);
  try{
    await renderLibrary(mock());await contextMenu(host.querySelector('.x-archive-body')!,{clientX:100000,clientY:100000});
    const menu=document.querySelector<HTMLElement>('.x-archive-context-menu')!;
    expect(menu.style.left).toBe(`${innerWidth-240}px`);expect(menu.style.top).toBe(`${innerHeight-58}px`);
    expect(css).toContain("background:#000;color:#e7e9ea;border:1px solid #2f3336;border-radius:12px");
  }finally{width.mockRestore();height.mockRestore();}
});
it("discards a late external-open error when the archive owner changes",async()=>{
  const api=mock(),pending=deferred<void>();vi.mocked(api.openSource).mockReturnValueOnce(pending.promise);
  await renderLibrary(api);await contextMenu(host.querySelector('.x-archive-body')!);await click("외부 브라우저에서 열기");
  await renderLibrary(api,{active:false});await renderLibrary(api);await act(async()=>pending.reject(new Error("이전 외부 열기 오류")));
  expect(host.textContent).not.toContain("이전 외부 열기 오류");expect(document.querySelector('.x-archive-context-menu')).toBeNull();
});

it("separates whole-post attachments from explicitly downloaded media without changing archives",async()=>{
  const api=mock(),body=record(),media=record({id:ID2,intent:"media"}),before=JSON.stringify([body,media]);
  vi.mocked(api.list).mockResolvedValue([summary(),{...summary(ID2),intent:"media"}]);
  vi.mocked(api.read).mockImplementation(async id=>id===ID?body:media);
  await renderLibrary(api);expect(api.read).toHaveBeenCalledExactlyOnceWith(ID);
  expect(host.querySelectorAll('.x-archive-timeline-post[data-archive-id]')).toHaveLength(1);
  await click("미디어");expect(host.querySelectorAll('.x-archive-media-tile')).toHaveLength(1);
  expect(host.querySelector('.x-archive-media-tile img')).toHaveAttribute("src",`http://x-archive.localhost/${ID2}/${IMAGE}`);
  expect(JSON.stringify([body,media])).toBe(before);expect(api.save).not.toHaveBeenCalled();
});
it("keeps archived attachments in their reader but not in the media-download tab",async()=>{
  const api=mock();await renderLibrary(api);await openLibraryFirst();await click("미디어");
  expect(host.querySelector('.x-archive-media-tile')).toBeNull();expect(host.textContent).toContain("다운로드한 미디어가 없습니다.");
  expect(host.querySelector('.x-archive-preview img')).not.toBeNull();expect(api.read).toHaveBeenCalledOnce();
});
it("filters recorded owners independently of login and preserves removed or unknown account access",async()=>{
  const api=mock(),rows=[{...summary(),accountId:"alpha"},{...summary(ID2),accountId:"removed-owner"},summary(CAPTURE)];
  vi.mocked(api.list).mockResolvedValue(rows);vi.mocked(api.read).mockImplementation(async id=>record({id,accountId:id===ID?"alpha":id===ID2?"removed-owner":"legacy"}));
  await act(async()=>root.render(<XArchiveLibrary runtime="browser-mock" api={api} accounts={[{id:"alpha",handle:"reader"}]}/>));
  const select=host.querySelector<HTMLSelectElement>('select[aria-label="저장 계정"]')!;
  expect(select).toHaveTextContent("모든 계정@reader저장 계정 (removed-)계정 미확인");
  const choose=async(value:string)=>act(async()=>{select.value=value;select.dispatchEvent(new Event("change",{bubbles:true}));});
  await openLibraryFirst();await choose("account:removed-owner");
  expect(host.querySelectorAll('[data-archive-id]')).toHaveLength(1);expect(host.querySelector('[data-archive-id]')).toHaveAttribute("data-archive-id",ID2);
  expect(host.querySelector('.x-archive-preview .x-archive-reader')).toBeNull();
  await choose("unknown");expect(host.querySelector('[data-archive-id]')).toHaveAttribute("data-archive-id",CAPTURE);
  await choose("all");expect(host.querySelectorAll('[data-archive-id]')).toHaveLength(3);expect(api.read).toHaveBeenCalledTimes(3);expect(api.inspect).not.toHaveBeenCalled();
});
it("rejects a manifest whose account differs from the selected summary",async()=>{
  const api=mock();vi.mocked(api.list).mockResolvedValue([{...summary(),accountId:"alpha"}]);
  await renderLibrary(api);expect(host.textContent).toContain("종류나 계정이 일치하지 않습니다");expect(host.querySelector('.x-archive-body')).toBeNull();
});
it("bounds reads even when scope changes before an old read completes",async()=>{
  const api=mock(),pending=deferred<XArchiveRecord>();
  vi.mocked(api.list).mockResolvedValue([summary(),{...summary(ID2),intent:"media"}]);
  vi.mocked(api.read).mockReturnValueOnce(pending.promise).mockResolvedValue(record({id:ID2,intent:"media"}));
  await renderLibrary(api);await click("미디어");expect(api.read).toHaveBeenCalledOnce();
  await act(async()=>pending.resolve(record()));
  expect(api.read).toHaveBeenCalledTimes(2);expect(host.querySelectorAll('.x-archive-media-tile')).toHaveLength(1);expect(host.querySelector('[data-archive-id]')).toBeNull();
});
it("keeps native read lanes occupied after a UI deadline until the actual reads settle",async()=>{
  vi.useFakeTimers();const api=mock(),first=deferred<XArchiveRecord>(),second=deferred<XArchiveRecord>();
  vi.mocked(api.list).mockResolvedValue([summary(),summary(ID2),{...summary(CAPTURE),intent:"media"}]);
  vi.mocked(api.read).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise).mockResolvedValue(record({id:CAPTURE,intent:"media"}));
  await renderLibrary(api);expect(api.read).toHaveBeenCalledTimes(2);
  await act(async()=>vi.advanceTimersByTimeAsync(15001));expect(host.textContent).toContain("시간이 오래");
  await click("미디어");await act(async()=>vi.advanceTimersByTimeAsync(15001));expect(api.read).toHaveBeenCalledTimes(2);
  expect(button("다시 불러오기")).not.toBeDisabled();
  await act(async()=>first.resolve(record()));expect(api.read).toHaveBeenCalledTimes(2);
  await click("다시 불러오기");expect(api.read).toHaveBeenCalledTimes(3);
  expect(host.querySelectorAll('.x-archive-media-tile')).toHaveLength(1);
  await act(async()=>second.resolve(record({id:ID2})));expect(host.querySelector('[data-archive-id]')).toBeNull();
});
it("ends a four-item UI batch after two native reads hang without freeing those native lanes",async()=>{
  vi.useFakeTimers();mediaViewport();const{api,ids}=libraryRows(8),first=deferred<XArchiveRecord>(),second=deferred<XArchiveRecord>();
  vi.mocked(api.read).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  await renderLibrary(api);await click("미디어");expect(api.read).toHaveBeenCalledTimes(2);
  await act(async()=>vi.advanceTimersByTimeAsync(15001));
  expect(host.querySelectorAll('.x-archive-card-skeleton')).toHaveLength(0);expect(host.querySelectorAll('.x-archive-timeline-failed')).toHaveLength(4);
  expect(button("더 보기")).not.toBeDisabled();expect(button("다시 불러오기")).not.toBeDisabled();expect(button("새로고침")).not.toBeDisabled();
  for(let attempt=0;attempt<3;attempt++){
    await click("다시 불러오기");await act(async()=>vi.advanceTimersByTimeAsync(15001));
    expect(api.read).toHaveBeenCalledTimes(2);expect(button("다시 불러오기")).not.toBeDisabled();
  }
  await act(async()=>{first.resolve(record({id:ids[0],intent:"media"}));second.resolve(record({id:ids[1],intent:"media"}));});
  expect(api.read).toHaveBeenCalledTimes(2);expect(host.querySelectorAll('.x-archive-media-tile')).toHaveLength(0);
  await click("다시 불러오기");expect(api.read).toHaveBeenCalledTimes(3);expect(host.querySelectorAll('.x-archive-media-tile')).toHaveLength(1);
});
it("replaces superseded lane waiters instead of starting stale reads after rapid scope changes",async()=>{
  vi.useFakeTimers();const api=mock(),first=deferred<XArchiveRecord>(),second=deferred<XArchiveRecord>();
  vi.mocked(api.list).mockResolvedValue([summary(),summary(ID2),{...summary(CAPTURE),intent:"media"}]);
  vi.mocked(api.read).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise).mockImplementation(async id=>record({id,intent:id===CAPTURE?"media":"archive"}));
  await renderLibrary(api);
  for(let change=0;change<8;change++){await click("미디어");await click("저장된 본문");}
  await click("미디어");expect(api.read).toHaveBeenCalledTimes(2);
  await act(async()=>first.resolve(record()));expect(api.read).toHaveBeenCalledTimes(3);expect(api.read).toHaveBeenLastCalledWith(CAPTURE);
  expect(host.querySelectorAll('.x-archive-media-tile')).toHaveLength(1);
  await act(async()=>{second.resolve(record({id:ID2}));await vi.advanceTimersByTimeAsync(15001);});
  expect(api.read).toHaveBeenCalledTimes(3);expect(host.textContent).not.toContain("시간이 오래");
});
it("restores expanded pages as well as scroll when returning to a library scope",async()=>{
  const api=mock(),ids=Array.from({length:9},(_,index)=>`00000000-0000-4000-8000-${String(index).padStart(12,"0")}`);
  vi.mocked(api.list).mockResolvedValue([...ids.map(summary),{...summary(ID2),intent:"media"}]);
  vi.mocked(api.read).mockImplementation(async id=>record({id,intent:id===ID2?"media":"archive"}));
  await renderLibrary(api);await click("더 보기");await click("더 보기");
  const timeline=host.querySelector<HTMLDivElement>('.x-archive-timeline')!;timeline.scrollTop=2100;
  await click("미디어");await click("저장된 본문");
  expect(host.querySelectorAll('[data-archive-id]')).toHaveLength(9);expect(timeline.scrollTop).toBe(2100);
  expect(api.read).toHaveBeenCalledTimes(10);expect(button("더 보기")).toBeUndefined();
});
it("shows real request-scoped progress and ignores stale or terminal-late events",async()=>{
  const api=mock(),work=deferred<XArchiveRecord>();let notify!:(value:XArchiveProgress)=>void;
  api.subscribeProgress=vi.fn(async handler=>{notify=handler;return vi.fn();});vi.mocked(api.save).mockReturnValue(work.promise);
  await render(api);await click("현재 화면 보관");await click("보관");
  const request=vi.mocked(api.save).mock.calls[0]![0];
  const progress=(options:Partial<XArchiveProgress>={}):XArchiveProgress=>({requestId:request.requestId,captureId:CAPTURE,accountId:"saved-account",sequence:1,stage:"downloading",processedMedia:1,totalMedia:3,downloadedBytes:1048576,totalBytes:null,partial:false,currentTransfer:null,...options});
  expect(document.querySelector('progress')).not.toHaveAttribute("value");
  await act(async()=>notify(progress()));expect(document.querySelector('progress')).not.toHaveAttribute("value");expect(document.querySelector('progress')).toHaveAttribute("max","100");expect(document.body.textContent).toContain("1/3 · 받은 1.0 MB");
  await act(async()=>{notify(progress({sequence:0,processedMedia:0}));notify(progress({sequence:2,accountId:"other",processedMedia:2}));notify(progress({sequence:2,requestId:ID2,processedMedia:2}));notify(progress({sequence:2,captureId:ID2,processedMedia:2}));});
  expect(document.querySelector('progress')).not.toHaveAttribute("value");
  await act(async()=>notify(progress({sequence:3,stage:"writing",processedMedia:3})));expect(document.querySelector('progress')).not.toHaveAttribute("value");expect(document.body.textContent).toContain("파일 검증·보관 중");
  await act(async()=>{notify(progress({sequence:4,stage:"failed"}));notify(progress({sequence:5,stage:"complete",processedMedia:3}));});expect(document.body.textContent).toContain("저장 실패");expect(document.body.textContent).not.toContain("저장 완료");
  await render(api,{contextKey:"other"});await act(async()=>notify(progress({sequence:6})));expect(document.querySelector('progress')).toBeNull();
  await act(async()=>work.resolve(record()));expect(document.querySelector('.x-archive-reader')).toBeNull();
});
it.each([{processedMedia:4,totalMedia:3},{downloadedBytes:-1},{sequence:Infinity},{sequence:1.5},{totalBytes:1,downloadedBytes:2},{stage:"invented"},{accountId:""}])("rejects invalid progress counters and stages %o",change=>{
  expect(isXArchiveProgress({requestId:ID,captureId:CAPTURE,accountId:"saved-account",sequence:1,stage:"downloading",processedMedia:1,totalMedia:3,downloadedBytes:4,totalBytes:null,partial:false,currentTransfer:null,...change})).toBe(false);
});
it("keeps chooser cumulative transfer wording stable across file boundaries without faking a whole-job percentage",async()=>{
  vi.useFakeTimers();
  const api=mock(),work=deferred<XArchiveRecord>();let notify!:(value:XArchiveProgress)=>void;
  api.subscribeProgress=vi.fn(async handler=>{notify=handler;return vi.fn();});vi.mocked(api.save).mockReturnValue(work.promise);
  await render(api);await click("현재 화면 보관");await click("보관");const request=vi.mocked(api.save).mock.calls[0]![0];
  const value:XArchiveProgress={requestId:request.requestId,captureId:CAPTURE,accountId:"saved-account",sequence:0,stage:"queued",processedMedia:0,totalMedia:1,downloadedBytes:0,totalBytes:null,partial:false,currentTransfer:null};
  await act(async()=>notify(value));expect(document.body.textContent).toContain("대기 중");expect(document.querySelector('progress')).not.toHaveAttribute("value");
  await act(async()=>notify({...value,sequence:1,stage:"downloading",downloadedBytes:2097152,currentTransfer:{receivedBytes:1048576,totalBytes:3145728}}));
  expect(document.body.textContent).toContain("0/1 · 받은 2.0 MB");expect(document.querySelector('progress')).not.toHaveAttribute("value");expect(document.querySelector('progress')).toHaveAttribute("max","100");
  await act(async()=>notify({...value,sequence:2,stage:"downloading",downloadedBytes:4194304,currentTransfer:{receivedBytes:2097152,totalBytes:null}}));
  expect(document.body.textContent).toContain("0/1 · 받은 2.0 MB");await act(async()=>vi.advanceTimersByTimeAsync(750));
  expect(document.body.textContent).toContain("0/1 · 받은 4.0 MB");
  await act(async()=>notify({...value,sequence:3,stage:"downloading",processedMedia:1,downloadedBytes:4194304}));
  await act(async()=>vi.advanceTimersByTimeAsync(750));
  expect(document.body.textContent).toContain("1/1");expect(document.querySelector('progress')).not.toHaveAttribute("value");
  await act(async()=>notify({...value,sequence:4,stage:"writing",processedMedia:1,downloadedBytes:4194304,totalBytes:4194304}));
  expect(document.querySelector('progress')).toHaveAttribute("value","99");
  await act(async()=>notify({...value,sequence:5,stage:"complete",processedMedia:1,downloadedBytes:4194304}));
  expect(document.querySelector('progress')).toHaveAttribute("value","100");await act(async()=>work.resolve(record()));
});
it("trails chooser numeric bursts at 750ms while real progress stays current and phases stay immediate",async()=>{
  vi.useFakeTimers();vi.setSystemTime(0);const api=mock(),work=deferred<XArchiveRecord>();let notify!:(value:XArchiveProgress)=>void;
  api.subscribeProgress=vi.fn(async handler=>{notify=handler;return vi.fn();});vi.mocked(api.save).mockReturnValue(work.promise);
  await render(api);await click("현재 화면 보관");await click("보관");const request=vi.mocked(api.save).mock.calls[0]![0];
  const value:XArchiveProgress={requestId:request.requestId,captureId:CAPTURE,accountId:"saved-account",sequence:1,stage:"downloading",processedMedia:0,totalMedia:1,downloadedBytes:1048576,totalBytes:10485760,partial:false,currentTransfer:null};
  await act(async()=>notify(value));
  for(let n=2;n<=7;n++){
    await act(async()=>{await vi.advanceTimersByTimeAsync(100);notify({...value,sequence:n,downloadedBytes:n*1048576,currentTransfer:n%2?null:{receivedBytes:1048576,totalBytes:2097152}});});
    expect(document.querySelector('.x-archive-save-progress [role="status"]')!.textContent).toContain('받은 1.0 MB');
    expect(document.querySelector('progress')).toHaveAttribute('value',String(n*10));expect(document.querySelector('progress')).toHaveAttribute('aria-valuetext','0/1 · 받은 1.0 MB');
    expect(vi.getTimerCount()).toBeLessThanOrEqual(2); // One text timer plus the existing UI/test scheduling timer.
  }
  await act(async()=>vi.advanceTimersByTimeAsync(149));expect(document.body.textContent).toContain('받은 1.0 MB');
  await act(async()=>vi.advanceTimersByTimeAsync(1));expect(document.body.textContent).toContain('받은 7.0 MB');expect(vi.getTimerCount()).toBeLessThanOrEqual(1);
  await act(async()=>notify({...value,sequence:8,downloadedBytes:8388608}));expect(vi.getTimerCount()).toBeLessThanOrEqual(2);
  await act(async()=>notify({...value,sequence:9,stage:'writing',downloadedBytes:8388608}));expect(document.body.textContent).toContain('파일 검증·보관 중');expect(vi.getTimerCount()).toBeLessThanOrEqual(1);
  await act(async()=>vi.advanceTimersByTimeAsync(750));expect(document.body.textContent).not.toContain('받은');await act(async()=>work.resolve(record()));
});
it("immediately shows cancellation and its final outcome instead of stale trailing download text",async()=>{
  vi.useFakeTimers();vi.setSystemTime(0);const api=mock(),work=deferred<XArchiveRecord>();let notify!:(value:XArchiveProgress)=>void;
  api.subscribeProgress=vi.fn(async handler=>{notify=handler;return vi.fn();});vi.mocked(api.save).mockReturnValue(work.promise);
  await render(api);await click('현재 화면 보관');await click('보관');const request=vi.mocked(api.save).mock.calls[0]![0];
  const value:XArchiveProgress={requestId:request.requestId,captureId:CAPTURE,accountId:'saved-account',sequence:1,stage:'downloading',processedMedia:0,totalMedia:1,downloadedBytes:1048576,totalBytes:null,partial:false,currentTransfer:null};
  await act(async()=>notify(value));await act(async()=>notify({...value,sequence:2,downloadedBytes:2097152}));expect(vi.getTimerCount()).toBeLessThanOrEqual(2);
  await click('저장 취소');expect(document.body.textContent).toContain('취소 결과를 기다리는 중');expect(vi.getTimerCount()).toBeLessThanOrEqual(1);
  await act(async()=>notify({...value,sequence:3,stage:'cancelled'}));expect(document.body.textContent).toContain('저장 취소됨');expect(document.body.textContent).not.toContain('취소 결과를');
  await act(async()=>vi.advanceTimersByTimeAsync(750));expect(document.body.textContent).toContain('저장 취소됨');await act(async()=>work.reject(new Error('저장 취소됨')));
});
it("discards chooser numeric timers on privacy or context replacement before a new save starts",async()=>{
  vi.useFakeTimers();const api=mock(),work=deferred<XArchiveRecord>(),next=deferred<XArchiveRecord>();let notify!:(value:XArchiveProgress)=>void;
  api.subscribeProgress=vi.fn(async handler=>{notify=handler;return vi.fn();});vi.mocked(api.save).mockReturnValueOnce(work.promise).mockReturnValueOnce(next.promise);
  await render(api);await click('현재 화면 보관');await click('보관');const request=vi.mocked(api.save).mock.calls[0]![0];
  const value:XArchiveProgress={requestId:request.requestId,captureId:CAPTURE,accountId:'saved-account',sequence:1,stage:'downloading',processedMedia:0,totalMedia:1,downloadedBytes:1048576,totalBytes:null,partial:false,currentTransfer:null};
  await act(async()=>notify(value));await act(async()=>notify({...value,sequence:2,downloadedBytes:2097152}));expect(vi.getTimerCount()).toBeLessThanOrEqual(2);
  await render(api,{privacyMode:true});expect(vi.getTimerCount()).toBeLessThanOrEqual(1);await act(async()=>vi.advanceTimersByTimeAsync(750));expect(document.querySelector('.x-archive-save-progress')).toBeNull();
  await render(api,{contextKey:'new'});await click('현재 화면 보관');await click('보관');expect(document.body.textContent).toContain('저장 준비 중');
  await act(async()=>{notify({...value,sequence:3,downloadedBytes:3145728});await vi.advanceTimersByTimeAsync(750);});expect(document.body.textContent).not.toContain('받은');
  await act(async()=>{work.resolve(record());next.resolve(record());});
});
it("validates the exact progress and transfer field sets including queued zero sequence",()=>{
  const valid={requestId:ID,captureId:CAPTURE,accountId:"saved-account",sequence:0,stage:"queued",processedMedia:0,totalMedia:1,downloadedBytes:4,totalBytes:null,partial:false,currentTransfer:null};
  expect(isXArchiveProgress(valid)).toBe(true);expect(isXArchiveProgress({...valid,currentTransfer:{receivedBytes:2,totalBytes:4}})).toBe(true);
  for(const currentTransfer of [undefined,[],{},1,{receivedBytes:-1,totalBytes:null},{receivedBytes:5,totalBytes:null},{receivedBytes:2,totalBytes:1},{receivedBytes:0.5,totalBytes:null},{receivedBytes:1,totalBytes:Infinity},{receivedBytes:1,totalBytes:null,url:"private"}])expect(isXArchiveProgress({...valid,currentTransfer})).toBe(false);
  expect(isXArchiveProgress({...valid,stage:["queued"]})).toBe(false);expect(isXArchiveProgress({...valid,postUrl:"private"})).toBe(false);
});
it("opens the local sidebar library inline without account state, capture or a modal",async()=>{
  const api=mock();await renderLibrary(api);
  expect(host.querySelector('[aria-label="X 로컬 저장소"]')).not.toBeNull();
  expect(document.querySelector('[role="dialog"],[data-native-overlay]')).toBeNull();
  expect(api.list).toHaveBeenCalledOnce();expect(api.inspect).not.toHaveBeenCalled();expect(api.save).not.toHaveBeenCalled();
  expect(host.querySelector('[role="feed"]')).not.toBeNull();expect(host.querySelector('.x-archive-timeline-post')).toHaveTextContent("본문 10");
  expect(host.querySelector('.x-archive-list')).toBeNull();await openLibraryFirst();
  expect(api.read).toHaveBeenCalledExactlyOnceWith(ID);expect(host.querySelector('.x-archive-reader')).not.toBeNull();
  expect(api.openFolder).not.toHaveBeenCalled();await click("보관 폴더 열기");expect(api.openFolder).toHaveBeenCalledExactlyOnceWith(ID);
  expect(host.querySelector('[role="feed"]')).not.toBeNull();await click("미리보기 닫기");expect(api.list).toHaveBeenCalledOnce();expect(host.querySelector('.x-archive-reader')).toBeNull();expect(host.querySelector('.x-archive-timeline-post')).toHaveTextContent("본문 10");
});
it.each([{privacyMode:true},{active:false}])("does not read or expose local records for hidden sidebar lifecycle %o",async options=>{
  const api=mock();await renderLibrary(api,options);expect(host.textContent).toBe("");expect(api.list).not.toHaveBeenCalled();expect(api.read).not.toHaveBeenCalled();
});
it("hides the inline reader immediately on privacy and lists afresh when restored",async()=>{
  const api=mock();await renderLibrary(api);await openLibraryFirst();
  expect(host.textContent).toContain("본문 10");await renderLibrary(api,{privacyMode:true});expect(host.textContent).toBe("");
  await renderLibrary(api);expect(api.list).toHaveBeenCalledTimes(2);expect(host.querySelector('.x-archive-reader')).toBeNull();
});
it("drops a late inline read after leaving and reentering the local sidebar tab",async()=>{
  const api=mock(),pending=deferred<XArchiveRecord>();vi.mocked(api.read).mockReturnValueOnce(pending.promise);
  await renderLibrary(api);
  await renderLibrary(api,{active:false});await renderLibrary(api);await act(async()=>pending.resolve(record()));
  expect(api.list).toHaveBeenCalledTimes(2);expect(host.querySelector('.x-archive-reader')).toBeNull();expect(host.querySelectorAll('.x-archive-timeline-post[data-archive-id]')).toHaveLength(1);
});
it("ignores a late library result after its API owner is replaced",async()=>{
  const first=mock(),second=mock(),pending=deferred<XArchiveSummary[]>();vi.mocked(first.list).mockReturnValueOnce(pending.promise);vi.mocked(second.list).mockResolvedValueOnce([]);
  await renderLibrary(first);await renderLibrary(second);await act(async()=>pending.resolve([summary()]));
  expect(host.querySelector('.x-archive-timeline-post')).toBeNull();expect(host.textContent).toContain("보관된 항목이 없습니다.");
});
it("rejects mismatched inline reads and preserves the local list for retry",async()=>{
  const api=mock();vi.mocked(api.read).mockResolvedValueOnce(record({id:ID2}));await renderLibrary(api);
  expect(host.textContent).toContain("보관 항목이 일치하지 않습니다.");expect(host.querySelector('.x-archive-reader')).toBeNull();expect(host.querySelector('.x-archive-timeline-open')).toBeNull();
  await click("다시 불러오기");expect(api.read).toHaveBeenCalledTimes(2);expect(host.querySelector('.x-archive-timeline-open')).not.toBeNull();
});
it("bounds inline list loading without automatic retries or late state replacement",async()=>{
  vi.useFakeTimers();const api=mock(),pending=deferred<XArchiveSummary[]>();vi.mocked(api.list).mockReturnValueOnce(pending.promise);
  await renderLibrary(api);expect(button("새로고침")).toBeDisabled();await act(async()=>vi.advanceTimersByTimeAsync(15001));
  expect(button("새로고침")).not.toBeDisabled();expect(host.textContent).toContain("시간이 오래");expect(api.list).toHaveBeenCalledOnce();
  await act(async()=>pending.resolve([summary()]));expect(host.querySelector('.x-archive-timeline-post')).toBeNull();expect(host.textContent).not.toContain("보관된 항목이 없습니다.");
});
it("does not let a late inline folder error escape into another tab visit",async()=>{
  const api=mock(),pending=deferred<void>();vi.mocked(api.openFolder).mockReturnValueOnce(pending.promise);
  await renderLibrary(api);await openLibraryFirst();await click("보관 폴더 열기");
  await renderLibrary(api,{active:false});await renderLibrary(api);await act(async()=>pending.reject(new Error("이전 폴더 오류")));
  expect(host.textContent).not.toContain("이전 폴더 오류");expect(host.querySelector('.x-archive-reader')).toBeNull();
});
it("hydrates only four timeline captures at a time with at most two concurrent local reads",async()=>{
  const api=mock(),ids=Array.from({length:9},(_,index)=>`00000000-0000-4000-8000-${String(index).padStart(12,"0")}`);
  const first=deferred<XArchiveRecord>(),second=deferred<XArchiveRecord>();
  vi.mocked(api.list).mockResolvedValue(ids.map(summary));
  vi.mocked(api.read).mockImplementation(async id=>record({id})).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  await renderLibrary(api);expect(api.read).toHaveBeenCalledTimes(2);expect(host.querySelectorAll('.x-archive-card-skeleton')).toHaveLength(4);expect(button("더 보기")).toBeDisabled();
  await act(async()=>{first.resolve(record({id:ids[0]}));second.resolve(record({id:ids[1]}));});
  expect(api.read).toHaveBeenCalledTimes(4);expect(host.querySelectorAll('[data-archive-id]')).toHaveLength(4);
  expect(vi.mocked(api.read).mock.calls.map(([id])=>id)).toEqual(ids.slice(0,4));
  await click("더 보기");expect(api.read).toHaveBeenCalledTimes(8);expect(host.querySelectorAll('[data-archive-id]')).toHaveLength(8);
  await click("더 보기");expect(api.read).toHaveBeenCalledTimes(9);expect(host.querySelectorAll('[data-archive-id]')).toHaveLength(9);expect(button("더 보기")).toBeUndefined();
});
it("does not start subsequent hydration reads after the library becomes hidden",async()=>{
  const api=mock(),pending=deferred<XArchiveRecord>();
  vi.mocked(api.list).mockResolvedValue([summary(),summary(ID2),summary(CAPTURE),summary("44444444-4444-4444-8444-444444444444")]);
  vi.mocked(api.read).mockReturnValue(pending.promise);
  await renderLibrary(api);expect(api.read).toHaveBeenCalledTimes(2);await renderLibrary(api,{privacyMode:true});
  await act(async()=>pending.resolve(record()));expect(api.read).toHaveBeenCalledTimes(2);expect(host.textContent).toBe("");
});
it("renders actual local thumbnails and collapsed provenance with a visible partial marker",async()=>{
  const api=mock();vi.mocked(api.read).mockResolvedValueOnce(record({selectedContentComplete:false,issues:[{postId:"10",kind:"image",reason:"media_request_failed"}]}));
  await renderLibrary(api);
  expect(host.querySelector('.x-archive-timeline-images img')).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${IMAGE}`);
  expect(host.querySelector('.x-archive-partial')).toHaveTextContent("일부 보관");expect(host.querySelector('.x-archive-provenance-details')).not.toHaveAttribute("open");
  expect(host.querySelector('.x-archive-provenance-details')).toHaveTextContent("저장할 때 불러온 내용만");expect(host.querySelector('a,iframe,video')).toBeNull();
});
it("does not resolve a remote thumbnail fallback or preload HLS in the saved timeline",async()=>{
  hls.supported=true;const api=mock();vi.mocked(api.read).mockResolvedValueOnce(record({assets:[asset(HLS,{kind:"video_hls",mime:"application/vnd.apple.mpegurl"}),asset(IMAGE)]}));
  vi.mocked(api.mediaUrl).mockImplementation((id,name)=>name===IMAGE?"https://pbs.twimg.com/media/sample.jpg":createXArchiveApi("tauri").mediaUrl(id,name));
  await renderLibrary(api);expect(host.querySelector('img,video')).toBeNull();expect(hls.instances).toHaveLength(0);expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
  await click("저장 영상 재생");expect(hls.instances).toHaveLength(1);expect(hls.instances[0]!.loadSource).toHaveBeenCalledWith(`http://x-archive.localhost/${ID}/${HLS}`);
});
it("keeps each capture separate even when source URL and contents match",async()=>{
  const api=mock();vi.mocked(api.list).mockResolvedValueOnce([summary(),summary(ID2)]);vi.mocked(api.read).mockImplementation(async id=>record({id}));
  await renderLibrary(api);expect(host.querySelectorAll('[data-archive-id]')).toHaveLength(2);expect(host.querySelectorAll('.x-archive-timeline-open')).toHaveLength(2);
});
it("returns from a saved post to the same local timeline position without rereading",async()=>{
  const api=mock();await renderLibrary(api);host.querySelector<HTMLDivElement>('.x-archive-timeline')!.scrollTop=320;
  await openLibraryFirst();expect(host.querySelector('.x-archive-reader')).not.toBeNull();await click("미리보기 닫기");
  expect(host.querySelector<HTMLDivElement>('.x-archive-timeline')!.scrollTop).toBe(320);expect(api.list).toHaveBeenCalledOnce();expect(api.read).toHaveBeenCalledOnce();
});
it("keeps the feed and preview mounted across body/media tabs and preserves each scroll position",async()=>{
  const api=mock();vi.mocked(api.list).mockResolvedValue([summary(),{...summary(ID2),intent:"media"}]);vi.mocked(api.read).mockImplementation(async id=>record({id,intent:id===ID2?"media":"archive"}));
  await renderLibrary(api);const timeline=host.querySelector<HTMLDivElement>('.x-archive-timeline')!;timeline.scrollTop=320;
  await openLibraryFirst();expect(host.querySelector('[role="feed"]')).not.toBeNull();expect(host.querySelector('.x-archive-reader')).not.toBeNull();expect(timeline.scrollTop).toBe(320);
  await click("미디어");expect(host.querySelector('[aria-selected="true"]')).toHaveTextContent("미디어");expect(host.querySelector('.x-archive-reader')).not.toBeNull();expect(host.querySelector('.x-archive-media-grid img')).toHaveAttribute("src",`http://x-archive.localhost/${ID2}/${IMAGE}`);
  timeline.scrollTop=98;await click("저장된 본문");expect(timeline.scrollTop).toBe(320);await click("미디어");expect(timeline.scrollTop).toBe(98);
  expect(api.read).toHaveBeenCalledTimes(2);expect(api.list).toHaveBeenCalledOnce();expect(api.inspect).not.toHaveBeenCalled();
});
it("navigates local tabs with unmodified arrow keys but preserves modified shortcuts",async()=>{
  await renderLibrary(mock());const body=button("저장된 본문");body.focus();
  const modified=new KeyboardEvent("keydown",{key:"ArrowRight",ctrlKey:true,bubbles:true,cancelable:true});
  await act(async()=>body.dispatchEvent(modified));expect(modified.defaultPrevented).toBe(false);expect(body).toHaveAttribute("aria-selected","true");
  await act(async()=>body.dispatchEvent(new KeyboardEvent("keydown",{key:"ArrowRight",bubbles:true,cancelable:true})));
  expect(button("미디어")).toHaveAttribute("aria-selected","true");expect(document.activeElement).toBe(button("미디어"));
  await act(async()=>button("미디어").dispatchEvent(new KeyboardEvent("keydown",{key:"Home",bubbles:true})));
  expect(body).toHaveAttribute("aria-selected","true");expect(document.activeElement).toBe(body);
});
it("selects media into the right preview without fetching source content or creating players",async()=>{
  hls.supported=true;const api=mediaMock(record({assets:[asset(HLS,{kind:"video_hls",mime:"application/vnd.apple.mpegurl"})]}));
  await renderLibrary(api);await click("미디어");expect(host.querySelector('.x-archive-media-tile')).not.toBeNull();expect(hls.instances).toHaveLength(0);
  await act(async()=>host.querySelector<HTMLButtonElement>('.x-archive-media-tile')!.click());
  expect(host.querySelector('.x-archive-preview .x-archive-reader')).not.toBeNull();expect(host.querySelector('.x-archive-media-grid')).not.toBeNull();expect(hls.instances).toHaveLength(0);expect(api.inspect).not.toHaveBeenCalled();expect(api.read).toHaveBeenCalledOnce();
});
it("pauses only another playing video in this local library when a preview video starts",async()=>{
  const api=mock();vi.mocked(api.read).mockResolvedValueOnce(record({assets:[asset(VIDEO,{kind:"video",mime:"video/mp4"})]}));await renderLibrary(api);await openLibraryFirst();
  await act(async()=>host.querySelector<HTMLButtonElement>('.x-archive-timeline .x-archive-play')!.click());
  await act(async()=>host.querySelector<HTMLButtonElement>('.x-archive-preview .x-archive-play')!.click());
  const left=host.querySelector<HTMLVideoElement>('.x-archive-timeline video')!,right=host.querySelector<HTMLVideoElement>('.x-archive-preview video')!,outside=document.createElement("video");
  document.body.append(outside);left.muted=true;right.muted=false;
  Object.defineProperty(left,"paused",{configurable:true,value:false});Object.defineProperty(right,"paused",{configurable:true,value:false});Object.defineProperty(outside,"paused",{configurable:true,value:false});
  const leftPause=vi.fn(),rightPause=vi.fn(),outsidePause=vi.fn();left.pause=leftPause;right.pause=rightPause;outside.pause=outsidePause;
  await act(async()=>right.dispatchEvent(new Event("play",{bubbles:false})));
  expect(leftPause).toHaveBeenCalledOnce();expect(rightPause).not.toHaveBeenCalled();expect(outsidePause).not.toHaveBeenCalled();expect(left.muted).toBe(true);expect(right.muted).toBe(false);
  outside.remove();
});
it("renders structured historical profile, quote, icon counters and a disabled reply composer without active social actions",async()=>{
  const api=mock(),saved=record({assets:[],document:{...draft().document,records:[post("10",{presentation:presentation({quote:{id:"42",url:"https://x.com/quoted/status/42",handle:"quoted",displayName:"냥",verified:true,body:"인용된 본문",timestamp:"2026-10-19T12:00:00Z"}})})]}});
  await act(async()=>root.render(<XArchiveReader record={saved} api={api}/>));
  expect(host.querySelector('.x-archive-author')).toHaveTextContent("우엥@reader");expect(host.querySelector('.x-archive-quote-card')).toHaveTextContent("냥@quoted");expect(host.querySelector('.x-archive-quote-card')).toHaveTextContent("인용된 본문");
  expect(host.querySelector('.x-archive-reactions [aria-label^="마음에 들어요"]')).toHaveTextContent("3.5천");
  expect(host.querySelector('.x-archive-post-date')).toHaveTextContent("85.5만 조회수");
  expect(host.querySelector('.x-archive-reactions button,.x-archive-reactions [tabindex]')).toBeNull();expect(host.querySelector('.x-archive-reply-composer button')).toBeDisabled();
  expect(host.querySelector('a,iframe,input,textarea,form')).toBeNull();expect(host.textContent).not.toContain("캡처 당시 ·");expect(host.querySelector('summary')).toHaveAttribute("aria-label","보관 정보");
  expect(api.inspect).not.toHaveBeenCalled();expect(api.save).not.toHaveBeenCalled();expect(api.mediaUrl).not.toHaveBeenCalled();
});
it("uses observed historical time labels without advancing them with the current clock",async()=>{
  vi.useFakeTimers();vi.setSystemTime(new Date("2036-01-01T00:00:00Z"));
  const saved=record({assets:[],document:{...draft().document,records:[
    post("10",{presentation:presentation({timestampLabel:"오후 12:29 · 2026년 10월 20일",quote:{id:"42",url:"https://x.com/quoted/status/42",handle:"quoted",displayName:"냥",verified:null,body:"인용",timestamp:"2026-10-19T16:00:00Z",timestampLabel:"8시간"}})}),
    post("20",{presentation:presentation({relation:"reply",order:1,timestampLabel:"44분"})}),
  ]}});
  await act(async()=>root.render(<XArchiveReader record={saved} api={mock()}/>));
  expect(host.querySelector('.x-archive-post-date time')).toHaveTextContent("오후 12:29 · 2026년 10월 20일");
  expect(host.querySelector('.x-archive-quote-author time')).toHaveTextContent("8시간");expect(host.querySelector('.x-archive-replies time')).toHaveTextContent("44분");
});
it.each([
  ["26만","26만 조회수"],["26만\n 조회수","26만 조회수"],["26K Views","26K Views"],["表示回数 26万","表示回数 26万"],
])("does not duplicate a source-localized view-count label %s",async(views,expected)=>{
  const source=presentation({counts:{...presentation().counts,views}}),saved=record({assets:[],document:{...draft().document,records:[post("10",{presentation:source})]}});
  await act(async()=>root.render(<XArchiveReader record={saved} api={mock()}/>));
  expect(host.querySelector('.x-archive-post-date>span')?.textContent).toBe(expected);
  expect(source.counts.views).toBe(views);
});
it("shows a captured emoji-only name and a display-only quote without fabricating identity or live controls",async()=>{
  const avatarName=`asset-${CAPTURE}.webp`,avatarSource="https://pbs.twimg.com/profile_images/42/quote.jpg",imageSource="https://pbs.twimg.com/media/sample.jpg";
  const saved=record({assets:[asset(),asset(avatarName,{kind:"avatar",sourceUrl:avatarSource})],document:{...draft().document,records:[post("10",{
    author:"🍫\n@reader",body:"",quotes:["legacy flat quote","second untouched quote"],media:[{kind:"image",url:imageSource,alt:"観測された画像"},{kind:"avatar",url:avatarSource,alt:""}],
    presentation:presentation({displayName:"🍫",mediaUrls:[],quotePresentation:{handle:"quoted",displayName:"asa",verified:true,body:"観測された本文\n二行目",timestamp:"",timestampLabel:"10월 19일",avatarUrl:avatarSource,mediaUrls:[imageSource]}}),
  })]}});
  const api=mock();await act(async()=>root.render(<XArchiveReader record={saved} api={api}/>));
  expect(host.querySelector('.x-archive-author>strong')).toHaveTextContent("🍫");
  expect(host.querySelector('.x-archive-quote-author')).toHaveTextContent("asa@quoted10월 19일");expect(host.querySelector('.x-archive-quote-author>svg')).not.toBeNull();
  expect(host.querySelector('.x-archive-quote-author img')).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${avatarName}`);
  expect(host.querySelector('.x-archive-post>header img')).toBeNull();
  expect(host.querySelectorAll(`img[src$="/${IMAGE}"]`)).toHaveLength(1);expect(host.querySelector('[aria-label="함께 보관된 미디어"]')).toBeNull();
  expect(host.querySelector('.x-archive-quote-card .x-archive-body')?.textContent).toBe("観測された本文\n二行目");
  expect(host.querySelector('.x-archive-legacy-quote')).toHaveTextContent("second untouched quote");
  expect(host.textContent).not.toContain("legacy flat quote");expect(host.querySelector("a,iframe,script,input")).toBeNull();
  expect(api.inspect).not.toHaveBeenCalled();expect(api.read).not.toHaveBeenCalled();expect(api.save).not.toHaveBeenCalled();
  expect(saved.document.records[0]!.presentation!.quote).toBeUndefined();
});
it.each([true,false,undefined])("respects only the stored media/quote order and preserves legacy fallback (%s)",async mediaBeforeQuote=>{
  const videoSource="https://video.twimg.com/clip/video.mp4",posterSource="https://pbs.twimg.com/media/sample.jpg";
  const saved=record({assets:[asset(VIDEO,{kind:"video",mime:"video/mp4",sourceUrl:videoSource}),asset(IMAGE,{kind:"poster",sourceUrl:posterSource})],document:{...draft().document,records:[post("10",{
    media:[{kind:"video",url:videoSource,alt:"",poster:posterSource},{kind:"poster",url:posterSource,alt:""}],
    presentation:presentation({mediaBeforeQuote,mediaUrls:[videoSource,posterSource],quotePresentation:{handle:"quoted",displayName:"asa",verified:null,body:"인용",timestamp:"",mediaUrls:[]}}),
  })]}});
  const api=mock();await act(async()=>root.render(<XArchiveReader record={saved} api={api}/>));
  const quote=host.querySelector('.x-archive-quote-card')!,video=host.querySelector('.x-archive-video')!;
  expect(Boolean(video.compareDocumentPosition(quote)&Node.DOCUMENT_POSITION_FOLLOWING)).toBe(mediaBeforeQuote===true);
  expect(host.querySelectorAll('.x-archive-video')).toHaveLength(1);expect(host.querySelectorAll(`img[src$="/${IMAGE}"]`)).toHaveLength(1);
  expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
});
it("does not fetch missing display-only quote assets or grant authority to its observed handle",async()=>{
  const api=mock(),saved=record({assets:[],document:{...draft().document,records:[post("10",{
    presentation:presentation({quotePresentation:{handle:"unknown",displayName:"<script>x</script>",verified:null,body:"보관된 텍스트",timestamp:"",avatarUrl:"https://pbs.twimg.com/profile_images/42/missing.jpg",mediaUrls:["https://pbs.twimg.com/media/missing.jpg"]}}),
  })]}});
  await act(async()=>root.render(<XArchiveReader record={saved} api={api}/>));
  expect(host.querySelector('.x-archive-quote-card')).toHaveTextContent("<script>x</script>@unknown");expect(host.querySelector('img,iframe,video,script,a')).toBeNull();
  expect(api.mediaUrl).not.toHaveBeenCalled();expect(api.inspect).not.toHaveBeenCalled();expect(api.save).not.toHaveBeenCalled();
});
it("renders only proven loaded replies in the reply section and retains legacy or unrelated records separately",async()=>{
  const saved=record({assets:[],document:{...draft().document,records:[
    post("30",{presentation:presentation({relation:"reply",conversationId:"10",order:2,displayName:"둘째 답글"})}),
    post("40",{presentation:presentation({relation:"reply",conversationId:"999",order:1,displayName:"다른 대화"})}),
    post("20",{presentation:presentation({relation:"reply",conversationId:"10",order:1,displayName:"첫째 답글"})}),
    post("10",{presentation:presentation()}),post("50",{author:"이전 형식 항목"}),
  ]}});
  await act(async()=>root.render(<XArchiveReader record={saved} api={mock()}/>));
  expect(host.querySelector('.x-archive-reader>.x-archive-post')).toHaveAttribute("data-post-id","10");
  expect([...host.querySelectorAll('.x-archive-replies>[data-post-id]')].map(node=>node.getAttribute("data-post-id"))).toEqual(["20","30"]);
  expect(host.querySelector('.x-archive-replies')).not.toHaveTextContent("다른 대화");expect(host.querySelector('[aria-label="함께 보관된 게시물"]')).toHaveTextContent("이전 형식 항목");
  expect(saved.document.records[0]?.id).toBe("30");
});
it("defaults archive selection to the root and only its proven loaded replies, not unrelated neighbors",async()=>{
  const api=mock(),value=draft();value.document.records=[
    post("10",{presentation:presentation()}),post("20",{presentation:presentation({relation:"reply",conversationId:"10",order:1})}),
    post("30",{presentation:presentation({relation:"unknown",conversationId:undefined,order:2})}),
  ];vi.mocked(api.inspect).mockResolvedValueOnce(value);await render(api);await click("현재 화면 보관");
  expect(checkbox("10")).toBeChecked();expect(checkbox("20")).toBeChecked();expect(checkbox("30")).not.toBeChecked();
});
it.each(["missing","different-source"])("does not expand chooser selection without an exact proved source root (%s)",async mode=>{
  const api=mock(),value=draft();value.document.sourceUrl=mode==="different-source"?"https://x.com/reader/status/40":value.document.sourceUrl;
  value.document.records=[post("10",{presentation:mode==="missing"?null:presentation()}),post("20",{presentation:presentation({relation:"reply",conversationId:"10",order:1})})];
  vi.mocked(api.inspect).mockResolvedValueOnce(value);await render(api);await click("현재 화면 보관");expect(checkbox("10")).toBeChecked();expect(checkbox("20")).not.toBeChecked();
});
it("keeps quote avatar and media inside their quote with no duplication or remote fallback",async()=>{
  const avatarName=`asset-${CAPTURE}.webp`,avatarSource="https://pbs.twimg.com/profile_images/42/avatar_normal.jpg",imageSource="https://pbs.twimg.com/media/sample.jpg";
  const saved=record({assets:[asset(),asset(avatarName,{kind:"avatar",sourceUrl:avatarSource})],document:{...draft().document,records:[post("10",{quotes:["legacy first quote","second legacy quote"],presentation:presentation({quote:{id:"42",url:"https://x.com/quoted/status/42",handle:"quoted",displayName:"냥",verified:null,body:"인용",timestamp:"",avatarUrl:avatarSource,mediaUrls:[imageSource]}})})]}});
  await act(async()=>root.render(<XArchiveReader record={saved} api={mock()}/>));
  expect(host.querySelector('.x-archive-quote-author img')).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${avatarName}`);
  expect(host.querySelector('.x-archive-quote-card .x-archive-timeline-images img')).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${IMAGE}`);
  expect(host.querySelectorAll(`img[src$="/${IMAGE}"]`)).toHaveLength(1);expect(host.querySelector('.x-archive-post-content>.x-archive-timeline-images')).toBeNull();
  expect(host.querySelector('.x-archive-legacy-quote')).toHaveTextContent("second legacy quote");expect(host.textContent).not.toContain("legacy first quote");
  expect(host.querySelector('button button')).toBeNull();expect([...host.querySelectorAll("img")].every(node=>node.src.startsWith("http://x-archive.localhost/"))).toBe(true);
});
it("preserves an observed image and avatar in both a post and its self-quote while sharing committed local files",async()=>{
  const avatarName=`asset-${CAPTURE}.webp`,avatarSource="https://pbs.twimg.com/profile_images/42/avatar_normal.jpg",imageSource="https://pbs.twimg.com/media/sample.jpg";
  const saved=record({assets:[asset(),asset(avatarName,{kind:"avatar",sourceUrl:avatarSource})],document:{...draft().document,records:[post("10",{presentation:presentation({
    avatarUrl:avatarSource,mediaUrls:[imageSource],
    quote:{id:"42",url:"https://x.com/reader/status/42",handle:"reader",displayName:"우엥",verified:null,body:"이전 게시물",timestamp:"",avatarUrl:avatarSource,mediaUrls:[imageSource]},
  })})]}});
  const api=mock();await act(async()=>root.render(<XArchiveReader record={saved} api={api}/>));
  expect(host.querySelector('.x-archive-post>header img')).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${avatarName}`);
  expect(host.querySelector('.x-archive-quote-author img')).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${avatarName}`);
  expect(host.querySelectorAll(`img[src$="/${IMAGE}"]`)).toHaveLength(2);expect(host.querySelectorAll(`img[src$="/${avatarName}"]`)).toHaveLength(2);
  expect(host.querySelector('.x-archive-post-content>.x-archive-timeline-images img')).not.toBeNull();
  expect(host.querySelector('[aria-label="함께 보관된 미디어"]')).toBeNull();
  expect(api.read).not.toHaveBeenCalled();expect(api.inspect).not.toHaveBeenCalled();expect(saved.assets).toHaveLength(2);
});
it("treats an explicit empty own-media list as no root media rather than inheriting quote files",async()=>{
  const saved=record({document:{...draft().document,records:[post("10",{presentation:presentation({mediaUrls:[],quote:{id:"42",url:"https://x.com/quoted/status/42",handle:"quoted",displayName:"냥",verified:null,body:"인용",timestamp:"",mediaUrls:["https://pbs.twimg.com/media/sample.jpg"]}})})]}});
  await act(async()=>root.render(<XArchiveReader record={saved} api={mock()}/>));
  expect(host.querySelectorAll(`img[src$="/${IMAGE}"]`)).toHaveLength(1);expect(host.querySelector('.x-archive-post-content>.x-archive-timeline-images')).toBeNull();
});
it("retains linkless quote media in an unattributed local group when own media is explicitly empty",async()=>{
  const saved=record({document:{...draft().document,records:[post("10",{quotes:["주소가 확인되지 않은 인용"],presentation:presentation({mediaUrls:[],quote:undefined})})]}});
  const api=mock();await act(async()=>root.render(<XArchiveReader record={saved} api={api}/>));
  const group=host.querySelector('[aria-label="함께 보관된 미디어"]')!;
  expect(group).not.toBeNull();expect(group.querySelector("img")).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${IMAGE}`);
  expect(host.querySelectorAll(`img[src$="/${IMAGE}"]`)).toHaveLength(1);
  expect(host.querySelector('.x-archive-post-content>.x-archive-timeline-images,.x-archive-quote-card img')).toBeNull();
  expect(host.querySelector('.x-archive-legacy-quote')).toHaveTextContent("주소가 확인되지 않은 인용");
  expect(group.textContent).not.toContain("미디어");expect(api.read).not.toHaveBeenCalled();expect(api.inspect).not.toHaveBeenCalled();
});
it("retains an unidentified second quote's media separately while preserving proved shared root and quote occurrences",async()=>{
  const otherImage=`asset-${ID2}.jpg`,firstSource="https://pbs.twimg.com/media/sample.jpg",otherSource="https://pbs.twimg.com/media/unknown-quote.jpg";
  const saved=record({assets:[asset(),asset(otherImage,{sourceUrl:otherSource})],document:{...draft().document,records:[post("10",{
    quotes:["식별된 첫 인용","식별할 수 없는 두 번째 인용"],
    media:[{kind:"image",url:firstSource,alt:"shared"},{kind:"image",url:otherSource,alt:"unidentified"}],
    presentation:presentation({mediaUrls:[firstSource],quote:{id:"42",url:"https://x.com/quoted/status/42",handle:"quoted",displayName:"냥",verified:null,body:"식별된 인용",timestamp:"",mediaUrls:[firstSource]}}),
  })]}});
  await act(async()=>root.render(<XArchiveReader record={saved} api={mock()}/>));
  const group=host.querySelector('[aria-label="함께 보관된 미디어"]')!;
  expect(group.querySelectorAll("img")).toHaveLength(1);expect(group.querySelector("img")).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${otherImage}`);
  expect(host.querySelectorAll(`img[src$="/${IMAGE}"]`)).toHaveLength(2);expect(host.querySelectorAll(`img[src$="/${otherImage}"]`)).toHaveLength(1);
  expect(host.querySelector('.x-archive-quote-card img')).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${IMAGE}`);
  expect(host.querySelector('.x-archive-post-content>.x-archive-timeline-images img')).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${IMAGE}`);
});
it("shows unattributed videos and unrelated saved posters without rendering their segments or preloading HLS",async()=>{
  hls.supported=true;
  const videoSource="https://video.twimg.com/clip/master.m3u8",posterSource="https://pbs.twimg.com/media/unresolved-poster.jpg";
  const saved=record({assets:[asset(HLS,{kind:"video_hls",mime:"application/vnd.apple.mpegurl",sourceUrl:videoSource}),asset(IMAGE,{kind:"poster",sourceUrl:posterSource}),asset(VIDEO,{kind:"video_segment",mime:"video/mp4",sourceUrl:"https://video.twimg.com/clip/1.m4s"})],document:{...draft().document,records:[post("10",{
    media:[{kind:"video",url:videoSource,alt:""},{kind:"unavailable_video",url:"",alt:"",poster:posterSource}],quotes:["인용"],presentation:presentation({mediaUrls:[]}),
  })]}});
  await act(async()=>root.render(<XArchiveReader record={saved} api={mock()}/>));
  const group=host.querySelector('[aria-label="함께 보관된 미디어"]')!;
  expect(group.querySelectorAll(".x-archive-video")).toHaveLength(1);expect(group.querySelector("img")).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${IMAGE}`);
  expect(group.querySelector("video")).toBeNull();expect(hls.instances).toHaveLength(0);expect(host.querySelector(`img[src$="/${VIDEO}"]`)).toBeNull();
});
it("keeps unrelated poster tiles separate and only uses a video's exact observed poster association",async()=>{
  hls.supported=true;const videoSource="https://video.twimg.com/clip/master.m3u8",posterSource="https://pbs.twimg.com/media/another-video-poster.jpg";
  const saved=record({assets:[asset(HLS,{kind:"video_hls",mime:"application/vnd.apple.mpegurl",sourceUrl:videoSource}),asset(IMAGE,{kind:"poster",sourceUrl:posterSource})],document:{...draft().document,records:[post("10",{
    media:[{kind:"video",url:videoSource,alt:""},{kind:"unavailable_video",url:"",alt:"",poster:posterSource}],presentation:presentation({mediaUrls:[]}),
  })]}});
  const api=mediaMock(saved);await renderLibrary(api);await click("미디어");
  const grid=host.querySelector('.x-archive-media-grid')!;
  expect(grid.querySelectorAll(".x-archive-media-tile")).toHaveLength(2);expect(grid.querySelector('.x-archive-tile-video')!.parentElement!.querySelector("img")).toBeNull();
  expect(grid.querySelector("img")).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${IMAGE}`);expect(hls.instances).toHaveLength(0);
  const linkedApi=mediaMock(record({...saved,document:{...saved.document,records:[post("10",{media:[{kind:"video",url:videoSource,alt:"",poster:posterSource}],presentation:presentation({mediaUrls:[videoSource]})})]}}));
  await renderLibrary(linkedApi);
  expect(host.querySelectorAll(".x-archive-media-grid .x-archive-media-tile")).toHaveLength(1);
  expect(host.querySelector('.x-archive-media-grid .x-archive-tile-video')!.parentElement!.querySelector("img")).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${IMAGE}`);
});
it("counts a saved poster as visible media without inventing a playable missing video",async()=>{
  const posterSource="https://pbs.twimg.com/ext_tw_video_thumb/10/pu/img/saved.jpg";
  const saved=record({assets:[asset(IMAGE,{kind:"poster",sourceUrl:posterSource})],selectedContentComplete:false,issues:[{postId:"10",kind:"video",reason:"video_url_not_observed"}],document:{...draft().document,records:[post("10",{
    media:[{kind:"unavailable_video",url:"",poster:posterSource,alt:""},{kind:"poster",url:posterSource,alt:"저장된 포스터"}],
  })]}});
  const api=mediaMock(saved);await renderLibrary(api);await click("미디어");
  const tile=host.querySelector<HTMLButtonElement>('.x-archive-media-grid .x-archive-media-tile')!;
  expect(tile.querySelector("img")).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${IMAGE}`);
  expect(host.textContent).not.toContain("불러온 항목에 저장된 미디어가 없습니다.");
  expect(tile.querySelector('.x-archive-tile-video')).toBeNull();await act(async()=>tile.click());
  expect(host.querySelector('.x-archive-preview .x-archive-issues')).not.toBeNull();
  expect(host.querySelector("video")).toBeNull();expect(hls.instances).toHaveLength(0);
  expect(api.inspect).not.toHaveBeenCalled();expect(api.save).not.toHaveBeenCalled();
});
it("opens only local image pixels in a focus-trapped viewer and restores the clicked image focus",async()=>{
  await act(async()=>root.render(<XArchiveReader record={record()} api={mock()}/>));button("이미지 1 크게 보기").focus();await click("이미지 1 크게 보기");
  const viewer=document.querySelector('[role="dialog"][aria-label="저장된 이미지"]')!;
  expect(viewer.querySelector("img")).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${IMAGE}`);expect(document.activeElement).toBe(button("이미지 닫기"));
  await act(async()=>viewer.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true})));
  expect(document.querySelector('.x-archive-image-viewer')).toBeNull();expect(document.activeElement).toBe(button("이미지 1 크게 보기"));
});
it("fits the image viewport without replacing the captured rendition or mutating archive metadata",async()=>{
  const source="https://pbs.twimg.com/media/captured.jpg?format=jpg&name=360x360";
  const saved=record({assets:[asset(IMAGE,{sourceUrl:source})],document:{...draft().document,records:[post("10",{media:[{kind:"image",url:source,alt:"captured thumbnail"}]})]}});
  const before=JSON.stringify(saved),api=mock();
  await act(async()=>root.render(<XArchiveReader record={saved} api={api}/>));await click("이미지 1 크게 보기");
  const image=document.querySelector<HTMLImageElement>('.x-archive-image-viewer>img')!;
  expect(image).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${IMAGE}`);
  expect(image.hasAttribute("srcset")).toBe(false);expect(JSON.stringify(saved)).toBe(before);
  expect(api.inspect).not.toHaveBeenCalled();expect(api.save).not.toHaveBeenCalled();
  const style=document.createElement("style");style.textContent=css;document.head.append(style);
  try{
    const rules=Array.from(style.sheet!.cssRules).filter((rule):rule is CSSStyleRule=>"selectorText"in rule);
    const viewerRule=rules.find(rule=>rule.selectorText===".x-archive-image-viewer")!;
    const imageRule=rules.find(rule=>rule.selectorText===".x-archive-image-viewer>img")!;
    expect(viewerRule.style.getPropertyValue("box-sizing")).toBe("border-box");
    expect(imageRule.style.getPropertyValue("width")).toBe("100%");
    expect(imageRule.style.getPropertyValue("height")).toBe("100%");
    expect(imageRule.style.getPropertyValue("object-fit")).toBe("contain");
    expect(parseFloat(imageRule.style.getPropertyValue("min-width"))).toBe(0);
    expect(parseFloat(imageRule.style.getPropertyValue("min-height"))).toBe(0);
  }finally{style.remove();}
});
it("keeps image-only posts keyboard discoverable for the right preview",async()=>{
  const api=mock();vi.mocked(api.read).mockResolvedValueOnce(record({document:{...draft().document,records:[post("10",{body:""})]}}));await renderLibrary(api);
  expect(host.querySelector('.x-archive-timeline-open')).toBeNull();expect(host.querySelector('.x-archive-author-open')).not.toBeNull();await click("Reader 10 보관 게시물 열기");
  expect(host.querySelector('.x-archive-preview .x-archive-reader')).not.toBeNull();expect(host.querySelector('[role="feed"]')).not.toBeNull();
});
it("opens an empty-body combining-mark author's local preview through its visible saved timestamp",async()=>{
  const api=mock(),saved=record({document:{...draft().document,records:[post("10",{author:"َ",body:"",presentation:presentation({displayName:"َ",handle:"pecelnya",timestampLabel:"10월 19일"})})]}});
  const before=JSON.stringify(saved);vi.mocked(api.read).mockResolvedValueOnce(saved);await renderLibrary(api);
  expect(host.querySelector('.x-archive-timeline-open')).toBeNull();
  const timestamp=button("@pecelnya 보관 게시물 열기 · 10월 19일");
  expect(timestamp.tagName).toBe("BUTTON");expect(timestamp).toHaveTextContent("10월 19일");
  expect(timestamp.closest("time")).toHaveAttribute("dateTime",saved.document.records[0]!.timestamp);
  timestamp.focus();expect(document.activeElement).toBe(timestamp);await act(async()=>timestamp.click());
  expect(host.querySelector('.x-archive-preview .x-archive-post')).toHaveAttribute("data-post-id","10");
  expect(host.querySelector('.x-archive-preview .x-archive-timestamp-open')).toBeNull();
  expect(host.querySelector('.x-archive-reactions button')).toBeNull();expect(host.querySelector("a[href],iframe")).toBeNull();
  expect(api.read).toHaveBeenCalledExactlyOnceWith(ID);expect(api.inspect).not.toHaveBeenCalled();expect(api.save).not.toHaveBeenCalled();
  expect(JSON.stringify(saved)).toBe(before);
});
it("does not hijack Ctrl/Alt/Meta or IME image-viewer keys",async()=>{
  const second=`asset-${ID2}.jpg`,saved=record({assets:[asset(),asset(second,{sourceUrl:"https://pbs.twimg.com/media/second.jpg"})]});
  await act(async()=>root.render(<XArchiveReader record={saved} api={mock()}/>));await click("이미지 1 크게 보기");
  const viewer=document.querySelector('.x-archive-image-viewer')!;
  for(const options of [{ctrlKey:true},{altKey:true},{metaKey:true},{isComposing:true}]){
    const event=new KeyboardEvent("keydown",{key:"d",bubbles:true,cancelable:true,...options});
    await act(async()=>viewer.dispatchEvent(event));expect(event.defaultPrevented).toBe(false);expect(viewer.querySelector("img")).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${IMAGE}`);
  }
  await act(async()=>viewer.dispatchEvent(new KeyboardEvent("keydown",{key:"d",bubbles:true})));expect(viewer.querySelector("img")).toHaveAttribute("src",`http://x-archive.localhost/${ID}/${second}`);
});
it("never loads a structured quote avatar absent from its committed manifest",async()=>{
  const saved=record({assets:[],document:{...draft().document,records:[post("10",{presentation:presentation({quote:{id:"42",url:"https://x.com/quoted/status/42",handle:"quoted",displayName:"냥",verified:null,body:"텍스트",timestamp:"",avatarUrl:"https://untrusted.example/avatar.png",mediaUrls:["https://untrusted.example/image.jpg"]}})})]}});
  const api=mock();await act(async()=>root.render(<XArchiveReader record={saved} api={api}/>));expect(host.querySelector("img,iframe,video,script,a")).toBeNull();expect(api.mediaUrl).not.toHaveBeenCalled();
});

it("inspects the native document before any modal can hide it",async()=>{
  const api=mock(),pending=deferred<XArchiveDraft>();vi.mocked(api.inspect).mockReturnValueOnce(pending.promise);await render(api);await click("현재 화면 보관");
  expect(api.inspect).toHaveBeenCalledWith("preview",1);expect(document.querySelector('[aria-modal="true"]')).toBeNull();expect(document.body.textContent).toContain("화면 읽는 중");
  await act(async()=>pending.resolve(draft()));expect(document.querySelector('[role="dialog"][data-native-overlay]')).toBeNull();expect(document.querySelector('[data-native-overlay="true"] [role="dialog"]')).not.toBeNull();
  expect(checkbox("10")).toBeChecked();expect(checkbox("20")).not.toBeChecked();
});
it("releases only the exact chooser draft on close and never cancels a job",async()=>{
  const api=mock();await render(api);await click("현재 화면 보관");expect(api.release).not.toHaveBeenCalled();
  await click("보관 창 닫기");expect(api.release).toHaveBeenCalledExactlyOnceWith(CAPTURE,"saved-account");
  await act(async()=>root.render(null));expect(api.release).toHaveBeenCalledOnce();expect(api.cancel).not.toHaveBeenCalled();expect(api.save).not.toHaveBeenCalled();
});
it.each(["unmount","privacy","inactive","context","api"])("returns a held chooser lease to its original API on %s",async change=>{
  const api=mock(),replacement=mock();await render(api);await click("현재 화면 보관");
  if(change==="unmount")await act(async()=>root.render(null));
  else await render(change==="api"?replacement:api,change==="privacy"?{privacyMode:true}:change==="inactive"?{active:false}:change==="context"?{contextKey:"two"}:{});
  expect(api.release).toHaveBeenCalledExactlyOnceWith(CAPTURE,"saved-account");expect(replacement.release).not.toHaveBeenCalled();expect(api.cancel).not.toHaveBeenCalled();
});
it.each(["unmount","context","generation"])("releases a discarded late inspection after %s without opening a chooser",async change=>{
  const api=mock(),pending=deferred<XArchiveDraft>();vi.mocked(api.inspect).mockReturnValueOnce(pending.promise);
  await render(api);await click("현재 화면 보관");
  if(change==="unmount")await act(async()=>root.render(null));else await render(api,change==="context"?{contextKey:"two"}:{generation:2});
  await act(async()=>pending.resolve(draft()));
  expect(api.release).toHaveBeenCalledExactlyOnceWith(CAPTURE,"saved-account");expect(document.querySelector('[role="dialog"]')).toBeNull();expect(api.cancel).not.toHaveBeenCalled();
});
it("releases an expired inspection without releasing a newer open capture",async()=>{
  vi.useFakeTimers();const api=mock(),pending=deferred<XArchiveDraft>(),newer={...draft(),captureId:ID2};
  vi.mocked(api.inspect).mockReturnValueOnce(pending.promise).mockResolvedValueOnce(newer);
  await render(api);await click("현재 화면 보관");await act(async()=>vi.advanceTimersByTimeAsync(15001));
  await click("현재 화면 보관");expect(checkbox("10")).toBeChecked();await act(async()=>pending.resolve(draft()));
  expect(api.release).toHaveBeenCalledExactlyOnceWith(CAPTURE,"saved-account");expect(checkbox("10")).toBeChecked();
  await click("보관 창 닫기");expect(api.release).toHaveBeenNthCalledWith(2,ID2,"saved-account");
});
it("retains a failed chooser capture for retry and releases it once only after verified success",async()=>{
  const api=mock();vi.mocked(api.save).mockRejectedValueOnce(new Error("실패"));await render(api);await click("현재 화면 보관");
  await click("보관");expect(api.release).not.toHaveBeenCalled();expect(checkbox("10")).toBeChecked();
  await click("보관");expect(api.save).toHaveBeenCalledTimes(2);expect(vi.mocked(api.save).mock.calls.map(call=>call[0].captureId)).toEqual([CAPTURE,CAPTURE]);
  expect(api.release).toHaveBeenCalledExactlyOnceWith(CAPTURE,"saved-account");await click("보관 창 닫기");expect(api.release).toHaveBeenCalledOnce();
});
it("retains a chooser when the saved result identity is invalid",async()=>{
  const api=mock();vi.mocked(api.save).mockResolvedValueOnce(record({captureId:ID2}));await render(api);await click("현재 화면 보관");await click("보관");
  expect(api.release).not.toHaveBeenCalled();expect(checkbox("10")).toBeChecked();await click("보관 창 닫기");expect(api.release).toHaveBeenCalledExactlyOnceWith(CAPTURE,"saved-account");
});
it("does not cancel a reserved save or release a newer chooser when its old save finishes late",async()=>{
  const api=mock(),pending=deferred<XArchiveRecord>(),newer={...draft(),captureId:ID2};vi.mocked(api.save).mockReturnValueOnce(pending.promise);
  await render(api);await click("현재 화면 보관");await click("보관");await render(api,{contextKey:"two"});
  expect(api.release).toHaveBeenCalledExactlyOnceWith(CAPTURE,"saved-account");expect(api.cancel).not.toHaveBeenCalled();
  vi.mocked(api.inspect).mockResolvedValueOnce(newer);await click("현재 화면 보관");await act(async()=>pending.resolve(record()));
  expect(api.release).toHaveBeenCalledOnce();expect(checkbox("10")).toBeChecked();expect(document.querySelector('.x-archive-reader')).toBeNull();
  await click("보관 창 닫기");expect(api.release).toHaveBeenNthCalledWith(2,ID2,"saved-account");
});
it("does not retry cleanup or turn a verified save into a failure when release IPC fails",async()=>{
  const api=mock();vi.mocked(api.release).mockRejectedValueOnce(new Error("cleanup unavailable"));await render(api);await click("현재 화면 보관");await click("보관");
  expect(document.body.textContent).toContain("선택한 내용 보관됨");expect(document.body.textContent).not.toContain("cleanup unavailable");
  expect(api.release).toHaveBeenCalledOnce();expect(api.save).toHaveBeenCalledOnce();
});
it("does not inspect twice while the first read is pending",async()=>{
  const api=mock(),pending=deferred<XArchiveDraft>();vi.mocked(api.inspect).mockReturnValueOnce(pending.promise);await render(api);await click("현재 화면 보관");await click("미디어 다운로드");expect(api.inspect).toHaveBeenCalledOnce();await act(async()=>pending.resolve(draft()));
});
it("drops an inspection that belongs to a previous account context",async()=>{
  const api=mock(),pending=deferred<XArchiveDraft>();vi.mocked(api.inspect).mockReturnValueOnce(pending.promise);await render(api);await click("현재 화면 보관");await render(api,{contextKey:"two"});await act(async()=>pending.resolve(draft()));expect(document.querySelector('[role="dialog"]')).toBeNull();
});
it("drops an inspection if its source generation has changed",async()=>{
  const api=mock(),pending=deferred<XArchiveDraft>();vi.mocked(api.inspect).mockReturnValueOnce(pending.promise);await render(api);await click("현재 화면 보관");await render(api,{generation:2});await act(async()=>pending.resolve(draft()));expect(document.querySelector('[role="dialog"]')).toBeNull();expect(button("현재 화면 보관")).not.toBeDisabled();
});
it("sends only selected frozen post IDs, not new HTML or URLs",async()=>{
  const api=mock();await render(api);await click("현재 화면 보관");await act(async()=>{checkbox("10").click();checkbox("20").click();});await click("보관");
  expect(api.save).toHaveBeenCalledExactlyOnceWith({requestId:expect.stringMatching(/^[a-f0-9-]{36}$/),captureId:CAPTURE,selectedIds:["20"],intent:"archive",includeMedia:true});
});
it("retains all selections after a save failure and does not retry automatically",async()=>{
  const api=mock();vi.mocked(api.save).mockRejectedValueOnce(new Error("파일 저장 실패"));await render(api);await click("현재 화면 보관");await act(async()=>checkbox("20").click());await click("보관");
  expect(checkbox("10")).toBeChecked();expect(checkbox("20")).toBeChecked();expect(document.body.textContent).toContain("파일 저장 실패");expect(api.save).toHaveBeenCalledOnce();
});
it("does not present a foreign save result as the requested archive",async()=>{
  const api=mock();vi.mocked(api.save).mockResolvedValueOnce(record({captureId:ID2}));await render(api);await click("현재 화면 보관");await click("보관");expect(document.body.textContent).toContain("저장 결과가 선택한 보관 내용과 일치하지 않습니다.");expect(checkbox("10")).toBeChecked();expect(document.querySelector('.x-archive-reader')).toBeNull();
});
it("keeps the save attempt single and prevents closing from pretending cancellation",async()=>{
  const api=mock(),pending=deferred<XArchiveRecord>();vi.mocked(api.save).mockReturnValueOnce(pending.promise);await render(api);await click("현재 화면 보관");await click("보관");
  expect(button("보관 창 닫기")).toBeDisabled();expect(button("저장 중…")).toBeDisabled();await click("보관 창 닫기");expect(api.cancel).not.toHaveBeenCalled();
  await act(async()=>document.querySelector('[role="dialog"]')!.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true})));expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  await act(async()=>pending.resolve(record()));expect(api.save).toHaveBeenCalledOnce();expect(button("보관 창 닫기")).not.toBeDisabled();
});
it("sends explicit cancellation with the active request UUID and waits for final save state",async()=>{
  const api=mock(),pending=deferred<XArchiveRecord>();vi.mocked(api.save).mockReturnValueOnce(pending.promise);await render(api);await click("현재 화면 보관");await click("보관");await click("저장 취소");
  const id=vi.mocked(api.save).mock.calls[0]![0].requestId;expect(api.cancel).toHaveBeenCalledWith(id);expect(button("저장 취소")).toBeDisabled();expect(button("보관 창 닫기")).toBeDisabled();expect(document.body.textContent).toContain("취소 결과를 기다리는 중");
  await act(async()=>pending.reject(new Error("보관 작업이 취소되었습니다.")));expect(button("보관 창 닫기")).not.toBeDisabled();expect(checkbox("10")).toBeChecked();
});
it("reports committed save success if cancellation lost the publication race",async()=>{
  const api=mock(),pending=deferred<XArchiveRecord>();vi.mocked(api.save).mockReturnValueOnce(pending.promise);vi.mocked(api.cancel).mockResolvedValueOnce(false);await render(api);await click("현재 화면 보관");await click("보관");await click("저장 취소");await act(async()=>pending.resolve(record()));
  expect(document.body.textContent).toContain("선택한 내용 보관됨");expect(document.body.textContent).not.toContain("취소 완료");
});
it("distinguishes media download and disables posts with no downloadable media",async()=>{
  const api=mock();await render(api);await click("미디어 다운로드");expect(checkbox("30")).toBeDisabled();await click("다운로드");expect(api.save).toHaveBeenCalledWith(expect.objectContaining({intent:"media",includeMedia:true,selectedIds:["10"]}));
});
it("keeps an unavailable image selectable so the native save records its explicit missing-resource issue",async()=>{
  const api=mock(),value=draft();value.document.records[0]!.media=[{kind:"unavailable_image",url:"",alt:""}];vi.mocked(api.inspect).mockResolvedValueOnce(value);await render(api);await click("미디어 다운로드");expect(checkbox("10")).not.toBeDisabled();expect(checkbox("10")).toBeChecked();
});
it("can save text only without presenting media as downloaded",async()=>{
  const api=mock();vi.mocked(api.save).mockResolvedValueOnce(record({mediaRequested:false,assets:[]}));await render(api);await click("현재 화면 보관");await act(async()=>document.querySelector<HTMLInputElement>('.x-archive-include input')!.click());await click("보관");expect(api.save).toHaveBeenCalledWith(expect.objectContaining({includeMedia:false}));expect(document.body.textContent).toContain("미디어 파일은 저장하지 않았습니다.");
});
it("identifies a photo-only selection and successful image download without claiming body or comments",async()=>{
  const api=mock(),value=draft();
  value.document.sourceUrl="https://x.com/reader/status/10/photo/1";
  value.document.records=[Object.assign(post("10",{author:"",body:"",timestamp:"",counts:"",context:"",quotes:[]}),{photoOnly:true})];
  vi.mocked(api.inspect).mockResolvedValueOnce(value);
  vi.mocked(api.save).mockResolvedValueOnce(record({intent:"media",document:value.document}));
  await render(api);await click("미디어 다운로드");
  expect(document.body.textContent).toContain("현재 사진만 보관 · 본문·댓글 미포함");
  expect(checkbox("10").getAttribute("aria-label")).toBe("현재 사진 #10");
  expect(document.body.textContent).not.toContain("접힌 본문");
  await click("다운로드");
  expect(document.body.textContent).toContain("선택한 사진 보관됨");
  expect(document.body.textContent).toContain("현재 사진만 보관 · 본문·댓글 미포함");
  expect(document.body.textContent).not.toContain("시각 정보 없음");
});
it("cannot save a photo-only snapshot without its media file",async()=>{
  const api=mock(),value=draft();
  value.document.sourceUrl="https://x.com/reader/status/10/photo/1";
  value.document.records=[post("10",{author:"",body:"",timestamp:"",counts:"",context:"",quotes:[],photoOnly:true})];
  vi.mocked(api.inspect).mockResolvedValueOnce(value);
  vi.mocked(api.save).mockResolvedValueOnce(record({document:value.document}));
  await render(api);await click("현재 화면 보관");
  const include=document.querySelector<HTMLInputElement>('.x-archive-include input')!;
  expect(include).toBeChecked();expect(include).toBeDisabled();
  await click("보관");
  expect(api.save).toHaveBeenCalledWith(expect.objectContaining({includeMedia:true,selectedIds:["10"]}));
});
it("does not mislabel related or timeline records as verified comments",async()=>{
  await render(mock());await click("현재 화면 보관");expect(checkbox("20").getAttribute("aria-label")).toContain("불러온 항목");expect(document.body.textContent).toContain("전체 댓글이 아닙니다");
});
it("lists and reads archives without native inspection, source login or remote fetching",async()=>{
  const api=mock();await render(api,{generation:null,disabled:true});await click("로컬 보관함");expect(api.list).toHaveBeenCalledOnce();expect(api.inspect).not.toHaveBeenCalled();await act(async()=>document.querySelector<HTMLButtonElement>('.x-archive-list>button')!.click());expect(api.read).toHaveBeenCalledWith(ID);expect(api.inspect).not.toHaveBeenCalled();
});
it("preserves multiple captures of the same source as separate selectable rows",async()=>{
  const api=mock();vi.mocked(api.list).mockResolvedValueOnce([summary(),summary(ID2)]);await render(api);await click("로컬 보관함");expect(document.querySelectorAll('.x-archive-list>button')).toHaveLength(2);
});
it("opens only the selected local archive folder by explicit request",async()=>{
  const api=mock();await render(api);await click("로컬 보관함");await act(async()=>document.querySelector<HTMLButtonElement>('.x-archive-list>button')!.click());expect(api.openFolder).not.toHaveBeenCalled();await click("보관 폴더 열기");expect(api.openFolder).toHaveBeenCalledExactlyOnceWith(ID);expect(api.inspect).not.toHaveBeenCalled();
});
it("drops late local reads after the modal closes",async()=>{
  const api=mock(),pending=deferred<XArchiveRecord>();vi.mocked(api.read).mockReturnValueOnce(pending.promise);await render(api);await click("로컬 보관함");await act(async()=>document.querySelector<HTMLButtonElement>('.x-archive-list>button')!.click());await click("보관 창 닫기");await act(async()=>pending.resolve(record()));expect(document.querySelector('[role="dialog"]')).toBeNull();
});
it("rejects mismatched archive IDs instead of displaying another record",async()=>{
  const api=mock();vi.mocked(api.read).mockResolvedValueOnce(record({id:ID2}));await render(api);await click("로컬 보관함");await act(async()=>document.querySelector<HTMLButtonElement>('.x-archive-list>button')!.click());expect(document.body.textContent).toContain("보관 항목이 일치하지 않습니다.");expect(document.querySelector('.x-archive-reader')).toBeNull();
});
it("bounds read loading and ignores late completion after a deadline",async()=>{
  vi.useFakeTimers();const api=mock(),pending=deferred<XArchiveDraft>();vi.mocked(api.inspect).mockReturnValueOnce(pending.promise);await render(api);await click("현재 화면 보관");await act(async()=>vi.advanceTimersByTimeAsync(15001));expect(button("현재 화면 보관")).not.toBeDisabled();expect(document.body.textContent).toContain("시간이 오래");await act(async()=>pending.resolve(draft()));expect(document.querySelector('[role="dialog"]')).toBeNull();
});
it.each([{privacyMode:true},{active:false}])("conceals all local content and does no work when disabled by lifecycle %o",async options=>{
  const api=mock();await render(api,options);expect(document.querySelector('.x-archive-tools')).toBeNull();expect(api.list).not.toHaveBeenCalled();expect(api.inspect).not.toHaveBeenCalled();
});
it("closes visible archive content immediately when privacy is enabled",async()=>{
  const api=mock();await render(api);await click("현재 화면 보관");await render(api,{privacyMode:true});expect(document.querySelector('[role="dialog"]')).toBeNull();expect(document.body.textContent).not.toContain("본문 10");
});
it("restores keyboard focus and traps Tab within the active dialog",async()=>{
  await render(mock());button("현재 화면 보관").focus();await click("현재 화면 보관");expect(document.activeElement).toBe(button("보관 창 닫기"));
  await act(async()=>button("보관 창 닫기").dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",shiftKey:true,bubbles:true})));expect(document.activeElement).toBe(button("보관"));
  await act(async()=>button("보관").dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",bubbles:true})));expect(document.activeElement).toBe(button("보관 창 닫기"));
  await act(async()=>button("보관 창 닫기").dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true})));expect(document.activeElement).toBe(button("현재 화면 보관"));
});
it("normalizes displayed author whitespace without modifying captured text or timestamps",async()=>{
  const captured=post("10",{author:"Duckets\n @Franzferdinan57\n ·\n 2시간",body:"첫째 줄\n둘째 줄"}),saved=record({document:{...draft().document,records:[captured]}});
  await act(async()=>root.render(<XArchiveReader record={saved} api={mock()}/>));
  expect(host.querySelector('.x-archive-post header strong')?.textContent).toBe("Duckets @Franzferdinan57 · 2시간");
  expect(host.querySelector('.x-archive-post-date time')?.textContent).toContain("2026");
  expect(host.querySelector('.x-archive-body')?.textContent).toBe("첫째 줄\n둘째 줄");
  expect(captured.author).toBe("Duckets\n @Franzferdinan57\n ·\n 2시간");expect(captured.timestamp).toBe("2026-10-20T00:00:00Z");
});
it("normalizes selection names and checkbox labels while preserving selection IDs",async()=>{
  const api=mock(),value=draft();value.document.records[0]!.author="Reader\n\t @reader";vi.mocked(api.inspect).mockResolvedValueOnce(value);
  await render(api);await click("현재 화면 보관");
  expect(checkbox("10")).toHaveAttribute("aria-label","본문 Reader @reader #10");
  expect(document.querySelector('.x-archive-choice strong')?.textContent).toBe("본문 · Reader @reader");
  expect(checkbox("10")).toBeChecked();expect(value.document.records[0]!.author).toBe("Reader\n\t @reader");
});
it("includes the named reader scroll region in the dialog keyboard focus cycle",async()=>{
  const api=mock();vi.mocked(api.read).mockResolvedValueOnce(record({assets:[]}));await render(api);await click("로컬 보관함");await act(async()=>document.querySelector<HTMLButtonElement>('.x-archive-list>button')!.click());
  const region=document.querySelector<HTMLElement>('.x-archive-reader')!;
  expect(region).toHaveAttribute("role","region");expect(region).toHaveAttribute("aria-label","보관된 게시물 내용");expect(region).toHaveAttribute("tabindex","0");
  region.focus();expect(document.activeElement).toBe(region);
  const info=region.querySelector<HTMLElement>('summary')!;info.focus();
  await act(async()=>info.dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",bubbles:true})));expect(document.activeElement).toBe(button("보관 창 닫기"));
});

it("renders escaped frozen text, capture time and partial status, never source HTML",async()=>{
  const saved=record({selectedContentComplete:false,issues:[{postId:"10",kind:"text",reason:"capture_text_truncated"}],document:{...draft().document,records:[post("10",{body:'<img src="https://evil.example/x" onerror="alert(1)">',quotes:['<script>alert(1)</script>']})]}});
  await act(async()=>root.render(<XArchiveReader record={saved} api={mock()}/>));expect(host.textContent).toContain('<script>alert(1)</script>');expect(host.querySelector('script,iframe,a')).toBeNull();expect(host.textContent).toContain("일부 내용만 보관됨");expect(host.textContent).toContain("2026");expect(host.textContent).toContain("저장할 때 불러온 내용만");
  expect([...host.querySelectorAll('img,video')].every(node=>node.getAttribute('src')?.startsWith('http://x-archive.localhost/'))).toBe(true);
});
it("does not fall back to a remote media source when a local URL is rejected",async()=>{
  const api=mock();vi.mocked(api.mediaUrl).mockReturnValue("https://pbs.twimg.com/media/sample.jpg");await act(async()=>root.render(<XArchiveReader record={record()} api={api}/>));expect(host.querySelector('img,video,iframe')).toBeNull();
});
it("rejects uncommitted/path-traversal media names before calling the protocol",async()=>{
  const api=mock();await act(async()=>root.render(<XArchiveReader record={record({assets:[asset("../secret.jpg"),asset(IMAGE,{sha256:"invalid"})]})} api={api}/>));expect(api.mediaUrl).not.toHaveBeenCalled();expect(host.querySelector('img')).toBeNull();
});
it("starts local video only on explicit play and never autoplays while opening an archive",async()=>{
  const saved=record({assets:[asset(VIDEO,{kind:"video",mime:"video/mp4"})]});await act(async()=>root.render(<XArchiveReader record={saved} api={mock()}/>));expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();expect(host.querySelector('video')).toBeNull();await click("저장 영상 재생");expect(host.querySelector('video')).toHaveAttribute('controls');expect(host.querySelector('video')?.getAttribute('src')).toBe(`http://x-archive.localhost/${ID}/${VIDEO}`);
});
it("renders only the root HLS player and refuses every non-manifest local or remote request",async()=>{
  hls.supported=true;const saved=record({assets:[asset(HLS,{kind:"video_hls",mime:"application/vnd.apple.mpegurl"}),asset(VIDEO,{kind:"video_segment",mime:"video/mp4"}),asset(VTT,{kind:"video_segment",mime:"text/vtt"})]});
  await act(async()=>root.render(<XArchiveReader record={saved} api={mock()}/>));expect(document.querySelectorAll('.x-archive-video')).toHaveLength(1);await click("저장 영상 재생");expect(hls.instances).toHaveLength(1);
  const instance=hls.instances[0]!,xhr={abort:vi.fn(),withCredentials:true} as unknown as XMLHttpRequest;
  expect(instance.loadSource).toHaveBeenCalledWith(`http://x-archive.localhost/${ID}/${HLS}`);
  instance.config.xhrSetup(xhr,`http://x-archive.localhost/${ID}/${VIDEO}`);expect(xhr.withCredentials).toBe(false);
  instance.config.xhrSetup(xhr,`http://x-archive.localhost/${ID}/${VTT}`);expect(xhr.withCredentials).toBe(false);
  expect(()=>instance.config.xhrSetup(xhr,"https://video.twimg.com/example.mp4")).toThrow("비로컬");expect(xhr.abort).toHaveBeenCalled();
  expect(()=>instance.config.fetchSetup({url:`http://x-archive.localhost/${ID2}/${HLS}`},{})).toThrow("비로컬");
  await act(async()=>root.render(null));expect(instance.destroy).toHaveBeenCalledOnce();
});
it.each([IMAGE,VIDEO,HLS,VTT])("keeps separate validated archive and asset path segments for Tauri media %s",name=>{
  const url=createXArchiveApi("tauri").mediaUrl(ID,name);
  expect(url).toBe(`http://x-archive.localhost/${ID}/${name}`);
  expect(isXArchiveLocalUrl(url,ID,new Set([name]))).toBe(true);
});
it.each([`http://x-archive.localhost/${ID}/${VTT}`,`http://x-archive.localhost/${ID2}/${VTT}`,"https://video.twimg.com/subtitles/amplify_video/123/0/sample.vtt"])("rejects subtitles outside the committed local manifest %s",url=>{
  expect(isXArchiveLocalUrl(url,ID,new Set([HLS]))).toBe(false);
});
it.each(["https://evil.example/x","file:///C:/secret.jpg","data:image/png;base64,AA==",`http://x-archive.localhost/${ID}/${IMAGE}?secret=1`,`http://x-archive.localhost/${ID}/../${IMAGE}`,`http://x-archive.localhost/${ID}/%2e%2e/${IMAGE}`,`http://x-archive.localhost/${ID}%2F${IMAGE}`,`http://x-archive.localhost/${ID2}/${IMAGE}`])("rejects non-owned archive URL %s",url=>{expect(isXArchiveLocalUrl(url,ID,new Set([IMAGE]))).toBe(false);});
it("has no live-site calls in unsupported runtime and validates protocol identifiers",async()=>{
  const api=createXArchiveApi("browser-mock");await expect(api.inspect("feed",1)).rejects.toThrow("데스크톱");await expect(api.release(CAPTURE,"saved-account")).rejects.toThrow("데스크톱");await expect(api.list()).rejects.toThrow("데스크톱");await expect(api.openSource(ID,"10")).rejects.toThrow("데스크톱");expect(api.mediaUrl("../../private",IMAGE)).toBe("");expect(api.mediaUrl(ID,"../secret")).toBe("");expect(invoke).not.toHaveBeenCalled();
});
it("uses agreed archive commands and read argument names",async()=>{
  const api=createXArchiveApi("tauri"),request={requestId:ID2,captureId:CAPTURE,selectedIds:["10"],intent:"archive" as const,includeMedia:true};
  await api.inspect("preview",2);await api.save(request);await api.cancel(ID2);await api.list();await api.read(ID);await api.openFolder(ID);await api.openSource(ID,"10");
  expect(invoke).toHaveBeenNthCalledWith(1,"x_archive_inspect",{pane:"preview",generation:2});expect(invoke).toHaveBeenNthCalledWith(2,"x_archive_save",{request});expect(invoke).toHaveBeenNthCalledWith(3,"x_archive_cancel",{requestId:ID2});expect(invoke).toHaveBeenNthCalledWith(4,"x_archive_list");expect(invoke).toHaveBeenNthCalledWith(5,"x_archive_read",{archiveId:ID});expect(invoke).toHaveBeenNthCalledWith(6,"x_archive_open_folder",{archiveId:ID});expect(invoke).toHaveBeenNthCalledWith(7,"x_archive_open_source",{archiveId:ID,postId:"10"});
  await api.release(CAPTURE,"saved-account");expect(invoke).toHaveBeenNthCalledWith(8,"x_archive_release",{captureId:CAPTURE,accountId:"saved-account"});
});

describe.skipIf(!layoutBrowser)("offline archive UI fixtures (not actual persistence or download proof)",()=>{
  it.each([[360,640],[960,640],[1920,1080]])("fits captured small and large images in a bounded lightbox at %ix%i",async(width,height)=>{
    const second=`asset-${ID2}.jpg`,saved=record({assets:[asset(),asset(second,{sourceUrl:"https://pbs.twimg.com/media/second.jpg"})]});
    await act(async()=>root.render(<XArchiveReader record={saved} api={mock()}/>));await click("이미지 1 크게 보기");
    const viewer=document.querySelector('.x-archive-image-viewer')!.outerHTML;
    const scenes:LayoutScene[]=[
      ["small-portrait",292,360],["small-square",360,360],["small-landscape",900,423],
      ["large-portrait",2400,3200],["large-landscape",3200,1800],
    ].map(([name,imageWidth,imageHeight])=>{
      // Synthetic pixels retain explicit intrinsic dimensions. Production
      // still uses exactly the committed asset URL, never a guessed rendition.
      const source="data:image/svg+xml,"+encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="${imageWidth}" height="${imageHeight}" viewBox="0 0 ${imageWidth} ${imageHeight}"><rect width="${imageWidth}" height="${imageHeight}" fill="#243447"/><path d="M0 0 ${imageWidth} ${imageHeight}M${imageWidth} 0 0 ${imageHeight}" stroke="#8b98a5" stroke-width="8"/><rect x="4" y="4" width="${Number(imageWidth)-8}" height="${Number(imageHeight)-8}" fill="none" stroke="#e7a65c" stroke-width="8"/><circle cx="${Number(imageWidth)/2}" cy="${Number(imageHeight)/2}" r="${Math.min(Number(imageWidth),Number(imageHeight))/5}" fill="#496479"/></svg>`);
      return{name:`lightbox-${name}`,html:viewer.replace(`http://x-archive.localhost/${ID}/${IMAGE}`,source)};
    });
    const results=await auditLayouts(scenes,"html,body{margin:0;background:#000;font:14px sans-serif}"+css,width,height,`x-archive-lightbox-${width}`);
    expect(results.flatMap(result=>result.issues.map(issue=>result.name+": "+issue))).toEqual([]);
  },60000);
  it.each([[360,640],[960,640],[1920,1080]])("keeps archive controls reachable at %ix%i",async(width,height)=>{
    const api=mock();await render(api);await click("현재 화면 보관");
    const choose=document.querySelector('.x-archive-backdrop')!.outerHTML;
    const reading=renderToStaticMarkup(<XArchiveReader record={record({assets:[],selectedContentComplete:false,issues:[{postId:"10",kind:"text",reason:"capture_text_truncated"}],document:{...draft().document,records:[post("10",{body:"긴 보관 본문 ".repeat(600)})]}})} api={api}/>);
    vi.mocked(api.read).mockResolvedValue(record({assets:[],selectedContentComplete:false,document:{...draft().document,records:[post("10",{body:"긴 타임라인 본문 ".repeat(100)})]}}));await renderLibrary(api);
    const timeline=`<div style="height:100dvh;display:flex;min-height:0;background:#000">${host.innerHTML}</div>`;
    const richApi=mock();vi.mocked(richApi.read).mockResolvedValue(record({assets:[],document:{...draft().document,records:[
      post("10",{body:"이거 어케 불리는거지… 머리를 물에 담그고 있어야 합니까?\n물에 적신 채로 방치만 하면 되나?",presentation:presentation({quote:{id:"42",url:"https://x.com/quoted/status/42",handle:"quoted",displayName:"냥",verified:true,body:"두피 3분 불리고 샴푸하란 분 누굽니까.\n물로 두피 충분히 적셔주고 잘 말려주세요.",timestamp:"2026-10-20T00:00:00Z"}})}),
      post("20",{body:"와 이렇게나 많은 조언이\n감사합니다",presentation:presentation({relation:"reply",order:1})}),
      post("30",{body:"물이 닿는 시간도 중요하죠.",presentation:presentation({handle:"second",displayName:"다른 작성자",relation:"reply",order:2})}),
    ]}}));await renderLibrary(richApi);await openLibraryFirst();
    const split=`<div style="height:100dvh;display:flex;min-height:0;background:#000">${host.innerHTML}</div>`;
    const unresolvedApi=mock();vi.mocked(unresolvedApi.read).mockResolvedValue(record({document:{...draft().document,records:[post("10",{quotes:["주소가 확인되지 않은 인용"],presentation:presentation({mediaUrls:[]})})]}}));
    await renderLibrary(unresolvedApi);await openLibraryFirst();
    // Static geometry only: replace the test's nonexistent local-protocol image
    // with a synthetic fixture, never relax production local URL validation.
    const syntheticImage="data:image/svg+xml,"+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360"><rect width="640" height="360" fill="#243447"/><circle cx="465" cy="95" r="36" fill="#8b98a5"/><path d="M0 360 190 120 380 360M200 360 400 170 640 360" fill="#496479"/></svg>');
    const unattributed=`<div style="height:100dvh;display:flex;min-height:0;background:#000">${host.innerHTML.replaceAll(`http://x-archive.localhost/${ID}/${IMAGE}`,syntheticImage)}</div>`;
    const liveApi=mock(),videoSource="https://video.twimg.com/clip/video.mp4",posterSource="https://pbs.twimg.com/media/sample.jpg",avatarSource="https://pbs.twimg.com/profile_images/42/quote.jpg",avatarName=`asset-${CAPTURE}.webp`;
    vi.mocked(liveApi.read).mockResolvedValue(record({assets:[asset(VIDEO,{kind:"video",mime:"video/mp4",sourceUrl:videoSource}),asset(IMAGE,{kind:"poster",sourceUrl:posterSource}),asset(avatarName,{kind:"avatar",sourceUrl:avatarSource})],document:{...draft().document,records:[
      post("10",{author:"🍫",body:"",quotes:["보이는 인용"],media:[{kind:"video",url:videoSource,alt:"",poster:posterSource},{kind:"poster",url:posterSource,alt:""},{kind:"avatar",url:avatarSource,alt:""}],presentation:presentation({displayName:"🍫",mediaBeforeQuote:true,mediaUrls:[videoSource,posterSource],counts:{...presentation().counts,views:"26만\n 조회수"},quotePresentation:{handle:"quoted",displayName:"asa",verified:true,body:'짧은 대답, 단 3단어, 가족 싸움을 불러일으킬 수 있는:\n\n"꼭 결혼해야 해?"',timestamp:"",timestampLabel:"10월 19일",avatarUrl:avatarSource,mediaUrls:[]}})}),
      post("20",{author:"So",body:"Je comprend ton point de vue",presentation:presentation({handle:"reply",displayName:"So",relation:"reply",order:1,timestampLabel:"3시간"})}),
    ]}}));await renderLibrary(liveApi);await click("🍫 보관 게시물 열기");
    const portrait="data:image/svg+xml,"+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="900" height="1600"><rect width="900" height="1600" fill="#5f8a3d"/><path d="m0 400 900 500v400L0 800Z" fill="#604c34"/><circle cx="580" cy="260" r="110" fill="#a8bc78"/><path d="M100 1400 800 700" stroke="#ddd1ab" stroke-width="90"/></svg>');
    const linklessPortrait=`<div style="height:100dvh;display:flex;min-height:0;background:#000">${host.innerHTML.replaceAll(`http://x-archive.localhost/${ID}/${IMAGE}`,portrait).replaceAll(`http://x-archive.localhost/${ID}/${avatarName}`,syntheticImage)}</div>`;
    const emptyApi=mock();vi.mocked(emptyApi.read).mockResolvedValue(record({assets:[],document:{...draft().document,records:[post("10",{author:"َ",body:"",presentation:presentation({displayName:"َ",handle:"pecelnya",timestampLabel:"10월 19일"})})]}}));await renderLibrary(emptyApi);
    const emptyTimestamp=`<div style="height:100dvh;display:flex;min-height:0;background:#000">${host.innerHTML}</div>`;
    const scenes:LayoutScene[]=[{name:"selection",html:choose},{name:"reader",html:`<div class="x-archive-backdrop"><div class="x-archive-dialog" role="dialog"><header><h2>로컬 보관함</h2><button>×</button></header>${reading}</div></div>`},{name:"saved-timeline",html:timeline},{name:"saved-conversation-split",html:split},{name:"saved-unattributed-media",html:unattributed},{name:"saved-linkless-portrait",html:linklessPortrait},{name:"saved-empty-body-timestamp",html:emptyTimestamp}];
    const results=await auditLayouts(scenes,"html,body{margin:0;background:#211d28;font:14px sans-serif}*{box-sizing:border-box}"+css,width,height,`x-archive-${width}`);expect(results.flatMap(result=>result.issues.map(issue=>result.name+": "+issue))).toEqual([]);
  },60000);
});
