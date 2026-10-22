import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { XMediaSearchPanel } from "./XMediaSearchPanel";
import type { XMediaSearchApi, XSearchSnapshot } from "./xMediaSearchApi";
import css from "./XMediaSearchPanel.css?raw";
const ID="11111111-1111-4111-8111-111111111111",NEXT="22222222-2222-4222-8222-222222222222";
const snap=(change:Partial<XSearchSnapshot>={}):XSearchSnapshot=>({id:ID,generation:7,accountId:"legacy",feed:"foryou",pane:"feed",kind:"image",provider:"saucenao",phase:"consent",imageUrl:"https://pbs.twimg.com/media/Observed?format=jpg&name=small",previewData:null,message:"",candidates:[],...change});
const deferred=<T,>()=>{let resolve!:(value:T)=>void;let reject!:(error:unknown)=>void;const promise=new Promise<T>((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};};
function mockApi(value:XSearchSnapshot|null=snap()){
  let callback:((value:XSearchSnapshot|null)=>void)|undefined;const stop=vi.fn();
  const api:XMediaSearchApi={status:vi.fn().mockResolvedValue(value),subscribe:vi.fn(async handler=>{callback=handler;return stop;}),confirm:vi.fn().mockResolvedValue(snap({phase:"searching"})),resume:vi.fn().mockResolvedValue(snap({phase:"searching"})),cancel:vi.fn().mockResolvedValue(undefined),resetConsent:vi.fn().mockResolvedValue(undefined),openResult:vi.fn().mockResolvedValue(undefined)};
  return{api,stop,emit:(next:XSearchSnapshot|null)=>act(async()=>callback?.(next)),emitNow:(next:XSearchSnapshot|null)=>callback?.(next)};
}
let host:HTMLDivElement,root:Root,notice=vi.fn<(message:string)=>void>();
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);notice=vi.fn<(message:string)=>void>();});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.unstubAllGlobals();});
const render=(api:XMediaSearchApi,props:Partial<ComponentProps<typeof XMediaSearchPanel>>={})=>act(async()=>root.render(<XMediaSearchPanel runtime="browser-mock" active privacyMode={false} accountId="legacy" generation={7} feed="foryou" onNotice={notice} api={api} {...props}/>));
const button=(label:string)=>[...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find(node=>node.textContent===label||node.getAttribute("aria-label")===label)!;
const click=(label:string)=>act(async()=>button(label).click());
const dialog=()=>document.querySelector<HTMLElement>('[role="dialog"]');

it("shows both providers and exact selection before any transmission with an unchecked remember choice",async()=>{
  const fixture=mockApi();await render(fixture.api);
  expect(dialog()?.closest('[data-native-overlay="true"]')).not.toBeNull();expect(dialog()).toHaveTextContent("SauceNAO에 이미지 주소");expect(dialog()).toHaveTextContent("Google Lens에 현재 장면 한 장");
  expect(document.querySelector('input[type="checkbox"]')).not.toBeChecked();expect(dialog()).toHaveTextContent(snap().imageUrl!);
  expect(dialog()?.querySelector("img,iframe,a")).toBeNull();expect(fixture.api.confirm).not.toHaveBeenCalled();expect(fixture.api.openResult).not.toHaveBeenCalled();
  await click("확인 후 전송");expect(fixture.api.confirm).toHaveBeenCalledExactlyOnceWith(ID,false);expect(dialog()).toHaveTextContent("검색 중");
});
it("transmits remembered consent only on explicit confirmation and never infers it from opening or closing",async()=>{
  const fixture=mockApi();await render(fixture.api);await act(async()=>document.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());await click("확인 후 전송");
  expect(fixture.api.confirm).toHaveBeenCalledWith(ID,true);await click("검색 취소");expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);expect(dialog()).toBeNull();
  await fixture.emit(snap({id:NEXT}));expect(document.querySelector('input[type="checkbox"]')).not.toBeChecked();await click("취소");expect(fixture.api.confirm).toHaveBeenCalledOnce();
});
it("shows only a validated in-memory current-frame preview and never embeds provider or result pages",async()=>{
  const value=snap({kind:"video",provider:"lens",imageUrl:null,previewData:"data:image/jpeg;base64,/9j/2Q=="}),fixture=mockApi(value);await render(fixture.api);
  expect(dialog()?.querySelector("img")?.getAttribute("src")).toBe(value.previewData);expect(dialog()?.querySelector("iframe,video,a")).toBeNull();expect(fixture.api.confirm).not.toHaveBeenCalled();
});
it("renders truthful creator candidates and fractional similarity, opening links solely by exact indices",async()=>{
  const value=snap({phase:"complete",candidates:[{title:"<script>not HTML</script>",creator:null,similarity:83.25,links:[{label:"Source",url:"https://example.org/art"}]},{title:"Second",creator:"Explicit author",similarity:null,links:[]}]}),fixture=mockApi(value);await render(fixture.api);
  expect(dialog()).toHaveTextContent("83.3%");expect(dialog()).toHaveTextContent("원작자임을 보증하지 않습니다");expect(dialog()?.querySelector("script,a,img,iframe")).toBeNull();
  expect(dialog()?.querySelectorAll("li")[0]).not.toHaveTextContent("작가 후보");expect(dialog()?.querySelectorAll("li")[1]).toHaveTextContent("작가 후보: Explicit author");
  await click("Sourceexample.org");expect(fixture.api.openResult).toHaveBeenCalledExactlyOnceWith(ID,0,0);expect(fixture.api.confirm).not.toHaveBeenCalled();
  await click("검색 전송 동의 초기화");expect(fixture.api.resetConsent).toHaveBeenCalledOnce();expect(notice).toHaveBeenCalledWith("다음 검색부터 전송 동의를 다시 확인합니다.");
});
it.each([snap({phase:"complete"}),snap({phase:"failed",imageUrl:null,message:"인증이 필요합니다. 자동으로 통과하지 않습니다."}),snap({phase:"searching",message:"결과 확인 중"})])("uses native empty/failure/searching messages without claiming completion: %j",async value=>{
  const fixture=mockApi(value);await render(fixture.api);expect(dialog()).toHaveTextContent(value.message||"일치하는 후보를 찾지 못했습니다.");expect(fixture.api.confirm).not.toHaveBeenCalled();
});
it("does not let an initial late status response overwrite a newer event",async()=>{
  const fixture=mockApi(),status=deferred<XSearchSnapshot|null>();vi.mocked(fixture.api.status).mockReturnValue(status.promise);await render(fixture.api);
  await fixture.emit(snap({phase:"searching",message:"Live search"}));await act(async()=>status.resolve(snap()));expect(dialog()).toHaveTextContent("Live search");expect(button("확인 후 전송")).toBeUndefined();
});
it("keeps native events ahead of stale confirm responses and ignores phase regression",async()=>{
  const fixture=mockApi(),confirm=deferred<XSearchSnapshot|null>();vi.mocked(fixture.api.confirm).mockReturnValue(confirm.promise);await render(fixture.api);await click("확인 후 전송");
  await fixture.emit(snap({phase:"complete",message:"Native final"}));await act(async()=>confirm.resolve(snap({phase:"searching"})));expect(dialog()).toHaveTextContent("Native final");
  await fixture.emit(snap({phase:"consent"}));expect(dialog()).toHaveTextContent("Native final");
});
it("closes immediately, cancels once, and rejects late events or confirmation from the dismissed request",async()=>{
  const fixture=mockApi(),confirm=deferred<XSearchSnapshot|null>();vi.mocked(fixture.api.confirm).mockReturnValue(confirm.promise);await render(fixture.api);await click("확인 후 전송");await click("검색 창 닫기");
  expect(dialog()).toBeNull();await act(async()=>confirm.resolve(snap({phase:"searching"})));await fixture.emit(snap({phase:"complete"}));expect(dialog()).toBeNull();expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
  await fixture.emit(snap({id:NEXT}));expect(dialog()).not.toBeNull();await fixture.emit(snap());expect(dialog()).not.toHaveTextContent("Native final");expect(document.querySelector('input[type="checkbox"]')).not.toBeChecked();
});
it.each([{active:false},{privacyMode:true},{accountId:"a".repeat(32)},{generation:8},{feed:"bookmarks"}])("immediately conceals and cancels an old context without touching retained source panes: %j",async change=>{
  const source=document.createElement("div");source.innerHTML='<div data-source="feed">Feed preserved</div><div data-source="preview">Preview preserved</div>';document.body.append(source);const original=source.innerHTML;
  const fixture=mockApi();await render(fixture.api);await render(fixture.api,change);expect(dialog()).toBeNull();expect(fixture.api.cancel).toHaveBeenCalledWith(ID);expect(source.innerHTML).toBe(original);
  await fixture.emit(snap());expect(dialog()).toBeNull();source.remove();
});
it("does not subscribe, poll or show private data when inactive or without an account/generation",async()=>{
  for(const props of[{active:false},{privacyMode:true},{accountId:null},{generation:null}]){const fixture=mockApi();await render(fixture.api,props);expect(fixture.api.subscribe).not.toHaveBeenCalled();expect(fixture.api.status).not.toHaveBeenCalled();expect(dialog()).toBeNull();}
});
it("cleans up a late subscription and cancels a late initial status on unmount",async()=>{
  const first=mockApi(),sub=deferred<()=>void>();vi.mocked(first.api.subscribe).mockReturnValue(sub.promise);await render(first.api);await act(async()=>root.render(null));await act(async()=>sub.resolve(first.stop));expect(first.stop).toHaveBeenCalledOnce();expect(first.api.status).not.toHaveBeenCalled();
  const second=mockApi(),status=deferred<XSearchSnapshot|null>();vi.mocked(second.api.status).mockReturnValue(status.promise);await render(second.api);await act(async()=>root.render(null));await act(async()=>status.resolve(snap()));expect(second.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);expect(dialog()).toBeNull();
});
it("does not send duplicate confirmations on burst activation and preserves explicit failure/retry",async()=>{
  const fixture=mockApi(),pending=deferred<XSearchSnapshot|null>();vi.mocked(fixture.api.confirm).mockReturnValueOnce(pending.promise);await render(fixture.api);
  await act(async()=>{const submit=button("확인 후 전송");submit.click();submit.click();});expect(fixture.api.confirm).toHaveBeenCalledOnce();
  await act(async()=>pending.reject(new Error("do not expose arbitrary provider body")));expect(dialog()).toHaveTextContent("검색을 시작하지 못했습니다");expect(dialog()).not.toHaveTextContent("arbitrary provider");await click("확인 후 전송");expect(fixture.api.confirm).toHaveBeenCalledTimes(2);
});
it("traps modal keyboard focus and Escape cancels only the current search",async()=>{
  const fixture=mockApi();await render(fixture.api);const close=button("검색 창 닫기"),last=button("확인 후 전송");expect(document.activeElement).toBe(close);
  await act(async()=>close.dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",shiftKey:true,bubbles:true,cancelable:true})));expect(document.activeElement).toBe(last);
  await act(async()=>last.dispatchEvent(new KeyboardEvent("keydown",{key:"Tab",bubbles:true,cancelable:true})));expect(document.activeElement).toBe(close);
  await act(async()=>close.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));expect(dialog()).toBeNull();expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
});
it("consumes modal Escape before document capture/native-preview routing and does not steal unrelated focus on cleanup",async()=>{
  const globalClose=vi.fn(),other=document.createElement("button");other.textContent="Outside";document.body.append(other);
  document.addEventListener("keydown",globalClose,true);const fixture=mockApi();await render(fixture.api);
  await act(async()=>button("검색 창 닫기").dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));
  expect(globalClose).not.toHaveBeenCalled();expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
  document.removeEventListener("keydown",globalClose,true);
  await fixture.emit(snap({id:NEXT}));other.focus();await render(fixture.api,{privacyMode:true});expect(document.activeElement).toBe(other);other.remove();
});
it.each(["body","html","document","window","root","backdrop"])("owns Escape after native-child focus falls back to the main %s without closing retained panes",async targetName=>{
  const fixture=mockApi(),previewClose=vi.fn();await render(fixture.api);
  const rootTarget=document.createElement("div");rootTarget.id="root-fallback";host.append(rootTarget);
  const target=targetName==="body"?document.body:targetName==="html"?document.documentElement:targetName==="document"?document:targetName==="window"?window:targetName==="root"?rootTarget:document.querySelector('[data-native-overlay="true"]')!;
  (document.activeElement as HTMLElement).blur();expect(document.activeElement).toBe(document.body);
  document.addEventListener("keydown",previewClose,true);
  try{
    const escape=new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true});
    await act(async()=>target.dispatchEvent(escape));
    expect(escape.defaultPrevented).toBe(true);expect(dialog()).toBeNull();expect(previewClose).not.toHaveBeenCalled();expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
    expect(fixture.api.confirm).not.toHaveBeenCalled();expect(fixture.api.resume).not.toHaveBeenCalled();
  }finally{document.removeEventListener("keydown",previewClose,true);rootTarget.remove();}
});
it("yields root Escape to a newer visible competing modal, then resumes when that modal closes",async()=>{
  const fixture=mockApi();await render(fixture.api);const search=dialog();
  const other=document.createElement("div");other.setAttribute("role","alertdialog");other.textContent="Foreground confirmation";document.body.append(other);
  try{
    const blocked=new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true});await act(async()=>document.body.dispatchEvent(blocked));
    expect(blocked.defaultPrevented).toBe(false);expect(dialog()).toBe(search);expect(fixture.api.cancel).not.toHaveBeenCalled();
    other.remove();await act(async()=>document.body.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));
    expect(dialog()).toBeNull();expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
  }finally{other.remove();}
});
it.each(["body","search-button","exit-button"])("yields %s Escape to an earlier-DOM native top-layer dialog",async targetName=>{
  const exit=document.createElement("dialog"),exitButton=document.createElement("button");exitButton.textContent="Exit cancellation";exit.append(exitButton);host.append(exit);
  const fixture=mockApi();await render(fixture.api);host.prepend(exit);exit.open=true;
  try{
    const target=targetName==="body"?document.body:targetName==="search-button"?button("검색 창 닫기"):exitButton;
    const escape=new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true});await act(async()=>target.dispatchEvent(escape));
    expect(escape.defaultPrevented).toBe(false);expect(fixture.api.cancel).not.toHaveBeenCalled();expect(document.querySelector('.x-media-search-dialog')).not.toBeNull();
    exit.open=false;await act(async()=>document.body.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
  }finally{exit.remove();}
});
it("does not intercept a key specifically targeted inside another modal even if its DOM precedes the search portal",async()=>{
  const other=document.createElement("div"),control=document.createElement("button");other.setAttribute("role","dialog");other.setAttribute("aria-modal","true");other.append(control);host.append(other);
  const fixture=mockApi();await render(fixture.api);host.prepend(other);
  try{const event=new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true});await act(async()=>control.dispatchEvent(event));expect(event.defaultPrevented).toBe(false);expect(fixture.api.cancel).not.toHaveBeenCalled();}finally{other.remove();}
});
it.each(['hidden','inert','aria-hidden="true"','style="display:none"','style="visibility:hidden"','style="opacity:0"'])("ignores a concealed competing modal while owning root Escape: %s",async attrs=>{
  const fixture=mockApi();await render(fixture.api);const wrapper=document.createElement("div");wrapper.innerHTML=`<div ${attrs}><div role="dialog" aria-modal="true">Concealed</div></div>`;document.body.append(wrapper);
  try{await act(async()=>document.body.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);expect(document.querySelector('.x-media-search-dialog')).toBeNull();}finally{wrapper.remove();}
});
it.each([{isComposing:true},{keyCode:229},{ctrlKey:true},{altKey:true},{metaKey:true},{shiftKey:true}])("does not cancel via either capture or bubbling for composition/modified Escape: %j",async options=>{
  const fixture=mockApi();await render(fixture.api);const escape=new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true,...options});
  await act(async()=>button("검색 창 닫기").dispatchEvent(escape));expect(escape.defaultPrevented).toBe(false);expect(fixture.api.cancel).not.toHaveBeenCalled();expect(dialog()).not.toBeNull();
});
it("respects an already-consumed Escape and removes its root listener on privacy/unmount",async()=>{
  const fixture=mockApi();await render(fixture.api);const consumed=new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true});consumed.preventDefault();
  await act(async()=>button("검색 창 닫기").dispatchEvent(consumed));expect(fixture.api.cancel).not.toHaveBeenCalled();expect(dialog()).not.toBeNull();
  await render(fixture.api,{privacyMode:true});expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
  const after=new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true});await act(async()=>window.dispatchEvent(after));expect(after.defaultPrevented).toBe(false);
  await act(async()=>root.render(null));await act(async()=>document.body.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));expect(fixture.api.cancel).toHaveBeenCalledOnce();
});
it("does not re-open or cancel a replacement request from a retired root Escape closure",async()=>{
  const fixture=mockApi();await render(fixture.api);await fixture.emit(snap({id:NEXT}));expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
  await act(async()=>{document.body.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true}));document.body.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true}));});
  expect(dialog()).toBeNull();expect(fixture.api.cancel).toHaveBeenCalledTimes(2);expect(fixture.api.cancel).toHaveBeenLastCalledWith(NEXT);await fixture.emit(snap());expect(dialog()).toBeNull();
});
it("fences the rendered request ID when replacement arrives before the React modal commit",async()=>{
  const fixture=mockApi();await render(fixture.api);const oldClose=button("검색 창 닫기");
  await act(async()=>{fixture.emitNow(snap({id:NEXT}));oldClose.click();document.body.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true}));});
  expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);expect(dialog()).not.toBeNull();expect(button("검색 창 닫기")).not.toBe(oldClose);
  await act(async()=>document.body.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));expect(fixture.api.cancel).toHaveBeenCalledTimes(2);expect(fixture.api.cancel).toHaveBeenLastCalledWith(NEXT);
});
it("bounds the dialog and scrolling content for narrow/short screens without cropping previews",()=>{
  expect(css).toContain("width:min(640px,100%)");expect(css).toContain("max-height:calc(100dvh - 40px)");expect(css).toContain("overflow:auto;min-height:0");expect(css).toContain("object-fit:contain");expect(css).toContain("prefers-reduced-motion:reduce");expect(css).toContain("max-height:calc(100dvh - 16px)");
});
it("explains the bounded manual handoff and isolated persisted session without resubmitting or automating verification",async()=>{
  const fixture=mockApi(snap({phase:"verification",message:"사람 확인이 필요합니다."}));await render(fixture.api);
  expect(dialog()).toHaveTextContent("사람 확인이 필요합니다");expect(dialog()).toHaveTextContent("최대 10분");expect(dialog()).toHaveTextContent("미디어를 다시 전송하지 않습니다");
  expect(dialog()).toHaveTextContent("X와 분리된 로컬 프로필");expect(dialog()).toHaveTextContent("만료되면 다시 확인");expect(dialog()).toHaveTextContent("‘다시 묻지 않기’와는 별개");
  expect(dialog()?.querySelector("iframe,webview,img,a,input")).toBeNull();expect(button("확인 후 전송")).toBeUndefined();expect(button("검색 전송 동의 초기화")).toBeUndefined();
  expect(fixture.api.resume).not.toHaveBeenCalled();expect(fixture.api.confirm).not.toHaveBeenCalled();expect(fixture.api.resetConsent).not.toHaveBeenCalled();
  await click("확인 완료 후 계속");expect(fixture.api.resume).toHaveBeenCalledExactlyOnceWith(ID);expect(fixture.api.confirm).not.toHaveBeenCalled();expect(dialog()).toHaveTextContent("검색 중");
});
it("retains native panes and the same dialog on verification transitions without taking focus back from the provider",async()=>{
  const outside=document.createElement("button");outside.textContent="Provider focus surrogate";document.body.append(outside);
  const fixture=mockApi(snap({phase:"searching"}));await render(fixture.api);const mounted=dialog();outside.focus();
  await act(async()=>window.dispatchEvent(new Event("blur")));await fixture.emit(snap({phase:"verification"}));await render(fixture.api);
  expect(dialog()).toBe(mounted);expect(document.activeElement).toBe(outside);expect(fixture.api.cancel).not.toHaveBeenCalled();expect(fixture.api.resume).not.toHaveBeenCalled();
  await act(async()=>window.dispatchEvent(new Event("focus")));expect(fixture.api.resume).not.toHaveBeenCalled();expect(fixture.api.confirm).not.toHaveBeenCalled();
  expect(fixture.api.status).toHaveBeenCalledOnce();outside.remove();
});
it("allows explicit verification cycles but never regresses to media consent or resumes on phase changes",async()=>{
  const fixture=mockApi(snap({phase:"searching"}));await render(fixture.api);
  await fixture.emit(snap({phase:"verification"}));await fixture.emit(snap({phase:"consent"}));expect(button("확인 후 전송")).toBeUndefined();expect(button("확인 완료 후 계속")).toBeDefined();
  await click("확인 완료 후 계속");await fixture.emit(snap({phase:"verification",message:"확인이 아직 필요합니다."}));expect(dialog()).toHaveTextContent("확인이 아직 필요합니다");
  expect(fixture.api.resume).toHaveBeenCalledOnce();await click("확인 완료 후 계속");expect(fixture.api.resume).toHaveBeenCalledTimes(2);expect(fixture.api.confirm).not.toHaveBeenCalled();
});
it("serializes resume bursts, keeps failure visible, and retries only on another explicit continue",async()=>{
  const fixture=mockApi(snap({phase:"verification"})),pending=deferred<XSearchSnapshot|null>();vi.mocked(fixture.api.resume).mockReturnValueOnce(pending.promise);await render(fixture.api);
  await act(async()=>{const resume=button("확인 완료 후 계속");resume.click();resume.click();});expect(fixture.api.resume).toHaveBeenCalledOnce();expect(button("확인 완료 후 계속")).toBeDisabled();expect(button("검색 취소")).not.toBeDisabled();
  await act(async()=>pending.reject(new Error("private provider body")));expect(dialog()).toHaveTextContent("검색을 계속하지 못했습니다");expect(dialog()).not.toHaveTextContent("private provider");expect(fixture.api.resume).toHaveBeenCalledOnce();
  await click("확인 완료 후 계속");expect(fixture.api.resume).toHaveBeenCalledTimes(2);
});
it("keeps newer native verification or terminal events ahead of a delayed resume response",async()=>{
  const fixture=mockApi(snap({phase:"verification"})),pending=deferred<XSearchSnapshot|null>();vi.mocked(fixture.api.resume).mockReturnValueOnce(pending.promise);await render(fixture.api);await click("확인 완료 후 계속");
  await fixture.emit(snap({phase:"complete",message:"Native final"}));await act(async()=>pending.resolve(snap({phase:"searching"})));await fixture.emit(snap({phase:"verification"}));
  expect(dialog()).toHaveTextContent("Native final");expect(button("확인 완료 후 계속")).toBeUndefined();expect(fixture.api.resume).toHaveBeenCalledOnce();
});
it.each([{privacyMode:true},{accountId:"a".repeat(32)},{generation:8},{feed:"bookmarks"}])("cancels manual verification and discards a late resume on context invalidation: %j",async change=>{
  const fixture=mockApi(snap({phase:"verification"})),pending=deferred<XSearchSnapshot|null>();vi.mocked(fixture.api.resume).mockReturnValue(pending.promise);await render(fixture.api);await click("확인 완료 후 계속");await render(fixture.api,change);
  await act(async()=>pending.resolve(snap({phase:"searching"})));expect(dialog()).toBeNull();expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);expect(fixture.api.confirm).not.toHaveBeenCalled();
});
it("keeps Escape and cancel active during verification and ignores a late provider resume",async()=>{
  const fixture=mockApi(snap({phase:"verification"})),pending=deferred<XSearchSnapshot|null>();vi.mocked(fixture.api.resume).mockReturnValue(pending.promise);await render(fixture.api);await click("확인 완료 후 계속");
  await act(async()=>button("검색 취소").dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));expect(dialog()).toBeNull();
  await act(async()=>pending.resolve(snap({phase:"searching"})));await fixture.emit(snap({phase:"verification"}));expect(dialog()).toBeNull();expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
});
it("uses native expiry or provider close as authoritative instead of restarting verification",async()=>{
  const fixture=mockApi(snap({phase:"verification"}));await render(fixture.api);
  await fixture.emit(snap({phase:"failed",imageUrl:null,message:"수동 확인 시간이 만료되었습니다."}));expect(dialog()).toHaveTextContent("수동 확인 시간이 만료되었습니다");expect(button("확인 완료 후 계속")).toBeUndefined();expect(fixture.api.resume).not.toHaveBeenCalled();
  await fixture.emit(null);expect(dialog()).toBeNull();expect(fixture.api.confirm).not.toHaveBeenCalled();
});
