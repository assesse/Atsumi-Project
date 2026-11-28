import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {BrowserApp,browserApi,initialBrowserState,type BrowserSnapshot} from "./BrowserApp";
let root:Root,host:HTMLDivElement,change:(s:BrowserSnapshot)=>void;
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);vi.stubGlobal("ResizeObserver",class{observe(){}disconnect(){}});host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
function api(state=initialBrowserState){return {window:vi.fn().mockResolvedValue(undefined),preferences:vi.fn().mockResolvedValue(undefined),libraryQuery:vi.fn().mockResolvedValue({items:[],hasMore:false,nextOffset:0}),libraryUpdate:vi.fn().mockResolvedValue(false),snapshot:vi.fn().mockResolvedValue(state),tab:vi.fn().mockResolvedValue(undefined),network:vi.fn().mockResolvedValue(undefined),importConfig:vi.fn().mockResolvedValue(undefined),bounds:vi.fn().mockResolvedValue(undefined),subscribe:vi.fn().mockImplementation(async fn=>{change=fn;return()=>{}}),shortcuts:vi.fn().mockResolvedValue(()=>{})} as typeof browserApi;}
const render=(a:typeof browserApi)=>act(async()=>root.render(<BrowserApp api={a}/>));
const click=(label:string)=>act(async()=>{const target=[...host.querySelectorAll("button")].find(n=>n.getAttribute("aria-label")===label||[...n.childNodes].filter(c=>!(c instanceof HTMLElement&&c.classList.contains("fluent"))).map(c=>c.textContent).join("")===label)!;expect(target).toBeTruthy();target.click();});
const configured:BrowserSnapshot={...initialBrowserState,profiles:[{id:"jp",name:"JP-FREE.conf",countryHint:"JP"},{id:"us",name:"US-FREE.conf",countryHint:"US"}],network:{...initialBrowserState.network,profile:"JP-FREE.conf",profileId:"jp"}};
it("requires a config before enabling VPN and exposes human setup",async()=>{const a=api();await render(a);await click("Proton VPN 연결 설정");expect((host.querySelector('[role=switch][aria-label="Proton VPN"]') as HTMLButtonElement).disabled).toBe(true);expect(host.textContent).toContain("무료 계정 만들기");await click("설정 파일 가져오기");expect(a.importConfig).toHaveBeenCalledTimes(1);expect(a.network).not.toHaveBeenCalled();});
it("shows native confirmed connection instead of equating click with success",async()=>{const a=api(configured);await render(a);await click("Proton VPN 연결 설정");await click("Proton VPN");expect(a.network).toHaveBeenCalledWith(true,false);expect(host.textContent).not.toContain("VPN 연결됨");await act(async()=>change({...configured,network:{...configured.network,enabled:true,phase:"connected",country:"JP",ip:"203.0.113.1"}}));expect(host.textContent).toContain("VPN 연결됨 · 일본");expect(host.textContent).toContain("203.0.113.1");});
it("keeps a failed VPN selected and offers retry or explicit off",async()=>{const a=api({...configured,network:{...configured.network,enabled:true,phase:"error",error:"연결 실패"}});await render(a);await click("브라우저 연결 설정");expect(host.textContent).toContain("다시 연결");await click("Proton VPN");expect(a.network).toHaveBeenCalledWith(false,false);});
it("does not change networking during browser downloads",async()=>{const a=api({...configured,downloads:1});await render(a);await click("Proton VPN 연결 설정");for(const button of host.querySelectorAll<HTMLButtonElement>('[role=switch]'))expect(button.disabled).toBe(true);expect(a.network).not.toHaveBeenCalled();});
it("SNI switch is independent from VPN and new tabs preserve existing tab identity",async()=>{const a=api(configured);await render(a);await click("Proton VPN 연결 설정");await click("SNI 파편화");expect(a.network).toHaveBeenCalledWith(false,true);await click("새 탭");expect(a.tab).toHaveBeenCalledWith("new",undefined,undefined);});
it("connection failures remain actionable",async()=>{const a=api(configured);vi.mocked(a.network).mockRejectedValue("설정 확인 필요");await render(a);await click("Proton VPN 연결 설정");await click("Proton VPN");expect(host.querySelector('[role=alert]')?.textContent).toContain("설정 확인 필요");expect((host.querySelector('[aria-label="Proton VPN"]') as HTMLButtonElement).disabled).toBe(false);});

it("explains every Proton choice and opens downloads in a new tab",async()=>{
  const a=api({...initialBrowserState,tabs:[{id:"kept",url:"https://example.com/",title:"기존 페이지",loading:false}],active:"kept"});
  await render(a);await click("브라우저 연결 설정");
  for(const text of ["WireGuard 설정","Windows","VPN Accelerator","Port Forwarding","무료(Free)","부하(%)","생성 → 다운로드",".conf"])expect(host.textContent).toContain(text);
  await click("Proton 다운로드 열기");
  expect(a.tab).toHaveBeenCalledWith("new",undefined,"https://account.protonvpn.com/downloads");
  expect(a.tab).not.toHaveBeenCalledWith("close",expect.anything());
  expect(a.network).not.toHaveBeenCalled();
});
it("keeps import failures recoverable and never enables a tunnel implicitly",async()=>{
  const a=api();vi.mocked(a.importConfig).mockRejectedValue("WireGuard .conf 파일을 선택해 주세요.");
  await render(a);await click("브라우저 연결 설정");await click("설정 파일 가져오기");
  expect(host.querySelector("[role=alert]")?.textContent).toContain(".conf");
  expect(a.network).not.toHaveBeenCalled();
  expect(host.querySelector<HTMLButtonElement>('[aria-label="Proton VPN"]')?.disabled).toBe(true);
});
it("blocks replacement as well as toggles while a browser download is active",async()=>{
  const a=api({...configured,downloads:2});await render(a);await click("브라우저 연결 설정");
  expect([...host.querySelectorAll("button")].find(b=>b.textContent?.includes("국가 추가"))?.disabled).toBe(true);
  expect(host.textContent).toContain("다운로드 2개가 끝나면");
});
it("preserves configured profile and opens help only on request",async()=>{
  const a=api(configured);await render(a);await click("브라우저 연결 설정");
  expect(host.querySelector(".browser-setup-guide")).toBeNull();
  await click("Proton 처음 연결하기");expect(host.querySelector(".browser-setup-guide")).not.toBeNull();
  await click("연결 설정 닫기");await click("브라우저 연결 설정");
  expect(host.textContent).toContain("JP-FREE.conf");
  expect(host.querySelector(".browser-setup-guide")).not.toBeNull();
});
it("shortcuts still select existing tabs and return from settings",async()=>{
  const a=api({...configured,tabs:[{id:"a",url:"https://example.com/",title:"A",loading:false},{id:"b",url:"https://example.org/",title:"B",loading:false}],active:"a"});
  await render(a);await click("브라우저 연결 설정");
  await act(async()=>window.dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",ctrlKey:true,shiftKey:true,bubbles:true})));
  expect(a.tab).toHaveBeenCalledWith("select","b");
  expect(host.querySelector(".browser-network-panel")).toBeNull();
  await act(async()=>window.dispatchEvent(new KeyboardEvent("keydown",{key:"l",ctrlKey:true,bubbles:true})));
  expect(document.activeElement).toBe(host.querySelector('[aria-label="주소 또는 검색어"]'));
});
it("hides native page bounds during settings and restores them on close",async()=>{
  vi.useFakeTimers();
  const rect={x:0,y:108,width:1200,height:700};
  const geometry=vi.spyOn(HTMLElement.prototype,"getBoundingClientRect").mockReturnValue(rect as DOMRect);
  const a=api({...configured,tabs:[{id:"a",url:"https://example.com/",title:"A",loading:false}],active:"a"});
  try{
    await render(a);await act(async()=>vi.advanceTimersByTime(30));
    expect(a.bounds).toHaveBeenLastCalledWith(0,108,1200,700);
    await click("브라우저 연결 설정");await act(async()=>vi.advanceTimersByTime(30));
    expect(a.bounds).toHaveBeenLastCalledWith(0,108,0,0);
    await click("연결 설정 닫기");await act(async()=>vi.advanceTimersByTime(30));
    expect(a.bounds).toHaveBeenLastCalledWith(0,108,1200,700);
  }finally{geometry.mockRestore();vi.useRealTimers();}
});

it("uses one native shortcut path when WebView2 also delivers a DOM keydown",async()=>{
  vi.stubGlobal("isTauri",true);
  let nativeShortcut:(key:string)=>void=()=>{};
  const a=api({...configured,tabs:[{id:"a",url:"https://example.com/",title:"A",loading:false},{id:"b",url:"about:blank",title:"B",loading:false}],active:"b"});
  vi.mocked(a.shortcuts).mockImplementation(async handler=>{nativeShortcut=handler;return()=>{};});
  await render(a);
  await act(async()=>{window.dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",ctrlKey:true,bubbles:true}));nativeShortcut("next");});
  expect(a.tab).toHaveBeenCalledTimes(1);
  expect(a.tab).toHaveBeenCalledWith("select","a");
});

it("switches a connected country in one action and keeps the confirmed location until native updates",async()=>{
  const a=api({...configured,network:{...configured.network,enabled:true,phase:"connected",country:"JP",ip:"203.0.113.1"}});
  await render(a);await click("브라우저 연결 설정");
  const select=host.querySelector<HTMLSelectElement>('[aria-label="VPN 국가"]')!;
  expect(select.options).toHaveLength(2);
  await act(async()=>{select.value="us";select.dispatchEvent(new Event("change",{bubbles:true}));});
  expect(a.network).toHaveBeenCalledExactlyOnceWith(true,false,"us");
  expect(host.textContent).toContain("VPN 연결됨 · 일본");
  expect(host.textContent).not.toContain("VPN 연결됨 · 미국");
  await act(async()=>change({...configured,network:{...configured.network,profileId:"us",profile:"US-FREE.conf",enabled:true,phase:"connecting",country:null}}));
  expect(select.disabled).toBe(true);
  expect(host.textContent).toContain("VPN 연결 중");
});
it("country selection while off never enables VPN implicitly",async()=>{
  const a=api(configured);await render(a);await click("브라우저 연결 설정");
  const select=host.querySelector<HTMLSelectElement>('[aria-label="VPN 국가"]')!;
  await act(async()=>{select.value="us";select.dispatchEvent(new Event("change",{bubbles:true}));});
  expect(a.network).toHaveBeenCalledExactlyOnceWith(false,false,"us");
});
it("locks the country selector during downloads and allows adding without replacing the active country",async()=>{
  const a=api({...configured,network:{...configured.network,enabled:true,phase:"connected",country:"JP"}});
  await render(a);await click("브라우저 연결 설정");await click("국가 추가");
  expect(a.importConfig).toHaveBeenCalledTimes(1);expect(a.network).not.toHaveBeenCalled();
  expect(host.querySelector<HTMLSelectElement>('[aria-label="VPN 국가"]')?.value).toBe("jp");
  await act(async()=>change({...configured,downloads:1}));
  expect(host.querySelector<HTMLSelectElement>('[aria-label="VPN 국가"]')?.disabled).toBe(true);
});

const browsing:BrowserSnapshot={...configured,passwordSavingAvailable:true,tabs:[{id:"a",url:"https://example.com/",title:"Example",loading:false,canGoBack:true,canGoForward:false}],active:"a"};
const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,15));});
it("uses the compact header as window chrome without dragging its controls",async()=>{
 const a=api(browsing);await render(a);expect(host.querySelector(".browser-brand strong")).toBeNull();
 await click("최소화");expect(a.window).toHaveBeenLastCalledWith("minimize");
 await click("최대화");expect(a.window).toHaveBeenLastCalledWith("maximize");
 await act(async()=>change({...browsing,maximized:true}));await click("이전 크기로");expect(a.window).toHaveBeenLastCalledWith("maximize");
 await act(async()=>host.querySelector(".browser-drag-space")!.dispatchEvent(new MouseEvent("mousedown",{button:0,detail:1,bubbles:true})));expect(a.window).toHaveBeenLastCalledWith("drag");
 await act(async()=>host.querySelector(".browser-drag-space")!.dispatchEvent(new MouseEvent("mousedown",{button:0,detail:2,bubbles:true})));expect(a.window).toHaveBeenLastCalledWith("maximize");
 const count=vi.mocked(a.window).mock.calls.length;await act(async()=>host.querySelector('[aria-label="새 탭"]')!.dispatchEvent(new MouseEvent("mousedown",{button:0,detail:1,bubbles:true})));expect(a.window).toHaveBeenCalledTimes(count);
});
it("history opens via Ctrl+H and a history item navigates the existing tab",async()=>{
 const a=api(browsing);vi.mocked(a.libraryQuery).mockResolvedValue({items:[{id:"v1",url:"https://example.org/page",title:"Visited page",visitedAt:1794750000000}],hasMore:false,nextOffset:1});await render(a);
 await act(async()=>window.dispatchEvent(new KeyboardEvent("keydown",{key:"h",ctrlKey:true,bubbles:true})));await settle();
 expect(a.libraryQuery).toHaveBeenCalledWith("history","",0);expect(host.textContent).toContain("Visited page");
 await act(async()=>host.querySelector<HTMLButtonElement>(".browser-library-open")!.click());expect(a.tab).toHaveBeenLastCalledWith("go",undefined,"https://example.org/page");expect(host.querySelector(".browser-library")).toBeNull();
});
it("history clearing requires its local confirmation and keeps browsing untouched",async()=>{
 const a=api(browsing);await render(a);await click("방문 기록");await settle();await click("기록 삭제");expect(host.querySelector('[role=alertdialog]')).not.toBeNull();expect(a.libraryUpdate).not.toHaveBeenCalledWith("history","clear",undefined);
 await click("취소");expect(a.libraryUpdate).not.toHaveBeenCalledWith("history","clear",undefined);await click("기록 삭제");await click("모두 삭제");expect(a.libraryUpdate).toHaveBeenCalledWith("history","clear",undefined);expect(a.tab).not.toHaveBeenCalled();
});
it("closed tab shortcut restores instead of opening a new blank tab",async()=>{
 const a=api({...browsing,recentlyClosed:2});await render(a);await act(async()=>window.dispatchEvent(new KeyboardEvent("keydown",{key:"T",ctrlKey:true,shiftKey:true,bubbles:true})));expect(a.tab).toHaveBeenCalledExactlyOnceWith("restore");
});
it("bookmarks use the current URL and do not submit anything to the page",async()=>{
 const a=api(browsing);await render(a);await click("북마크 추가");expect(a.libraryUpdate).toHaveBeenCalledWith("bookmarks","add",undefined,"https://example.com/","Example");expect(a.tab).not.toHaveBeenCalled();
});
it("password preference only sends a boolean to the native browser",async()=>{
 const a=api(browsing);await render(a);await click("브라우저 설정");expect(host.querySelector('input[type=password]')).toBeNull();await click("비밀번호 저장 제안");expect(a.preferences).toHaveBeenCalledWith({savePasswords:false,autofill:true,restoreTabs:true});expect(a.tab).not.toHaveBeenCalled();
});
it("a late history response cannot replace another library section",async()=>{
 const a=api(browsing);let finish:(page:any)=>void=()=>{};vi.mocked(a.libraryQuery).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;})).mockResolvedValue({items:[{id:"b1",url:"https://example.org/",title:"Saved bookmark",visitedAt:1794750000000}],hasMore:false,nextOffset:1});await render(a);await click("방문 기록");await settle();
 await act(async()=>host.querySelector<HTMLButtonElement>('.browser-library-tabs button:nth-child(2)')!.click());await settle();await act(async()=>finish({items:[{id:"old",url:"https://example.net/",title:"Stale history",visitedAt:1794750000000}],hasMore:false,nextOffset:1}));expect(host.textContent).toContain("Saved bookmark");expect(host.textContent).not.toContain("Stale history");
});
