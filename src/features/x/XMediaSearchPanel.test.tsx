import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { XMediaSearchPanel } from "./XMediaSearchPanel";
import type { XMediaSearchApi, XSearchSnapshot, XProviderResult } from "./xMediaSearchApi";
import css from "./XMediaSearchPanel.css?raw";
const ID="11111111-1111-4111-8111-111111111111",NEXT="22222222-2222-4222-8222-222222222222";
const snap=(change:Partial<XSearchSnapshot>={}):XSearchSnapshot=>({source:"surface",archiveId:null,assetName:null,localContext:null,originalPageState:"unavailable",sourcePostUrl:null,id:ID,generation:7,accountId:"legacy",feed:"foryou",pane:"feed",kind:"image",provider:"saucenao",phase:"consent",imageUrl:"https://pbs.twimg.com/media/Observed?format=jpg&name=small",previewData:null,message:"",candidates:[],...change});
const deferred=<T,>()=>{let resolve!:(value:T)=>void;let reject!:(error:unknown)=>void;const promise=new Promise<T>((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};};
function mockApi(value:XSearchSnapshot|null=snap()){
  let callback:((value:XSearchSnapshot|null)=>void)|undefined;const stop=vi.fn();
  const api:XMediaSearchApi={status:vi.fn().mockResolvedValue(value),subscribe:vi.fn(async handler=>{callback=handler;return stop;}),confirm:vi.fn().mockResolvedValue(snap({phase:"searching"})),resume:vi.fn().mockResolvedValue(snap({phase:"searching"})),cancel:vi.fn().mockResolvedValue(undefined),resetConsent:vi.fn().mockResolvedValue(undefined),openOriginal:vi.fn().mockResolvedValue(undefined),openLensSettings:vi.fn().mockResolvedValue(undefined),refreshLensResults:vi.fn().mockResolvedValue(value),startArchive:vi.fn().mockResolvedValue(undefined),creatorLinks:vi.fn(),openCreatorLink:vi.fn().mockResolvedValue(undefined),openResult:vi.fn().mockResolvedValue(undefined)};
  return{api,stop,emit:(next:XSearchSnapshot|null)=>act(async()=>callback?.(next)),emitNow:(next:XSearchSnapshot|null)=>callback?.(next)};
}
let host:HTMLDivElement,root:Root,notice=vi.fn<(message:string)=>void>();
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);notice=vi.fn<(message:string)=>void>();});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.unstubAllGlobals();});
const render=(api:XMediaSearchApi,props:Partial<ComponentProps<typeof XMediaSearchPanel>>={})=>act(async()=>root.render(<XMediaSearchPanel runtime="browser-mock" active privacyMode={false} accountId="legacy" generation={7} feed="foryou" onNotice={notice} api={api} {...props}/>));
const button=(label:string)=>[...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find(node=>node.textContent===label||node.getAttribute("aria-label")===label)!;
const click=(label:string)=>act(async()=>button(label).click());
const dialog=()=>document.querySelector<HTMLElement>('[role="dialog"]');
const dual=(sauce:Partial<XProviderResult>={},lens:Partial<XProviderResult>={},change:Partial<XSearchSnapshot>={}):XSearchSnapshot=>{
  const providers:XProviderResult[]=[{provider:"saucenao",phase:"searching",message:"",candidates:[],originalPageState:"available",...sauce},{provider:"lens",phase:"searching",message:"",candidates:[],originalPageState:"available",...lens}];
  const phase=providers.some(p=>p.phase==="searching")?"searching":providers.some(p=>p.phase==="verification")?"verification":providers.every(p=>p.phase==="failed")?"failed":"complete";
  return snap({phase,providers,...change});
};
it('shows the completed low-similarity source explanation and keeps the other provider usable without resubmitting',async()=>{
  const message='유사도가 낮은 결과만 있어 원본 사이트에서 숨겨졌습니다. 검색 화면에서 펼쳐 볼 수 있습니다.';
  const candidate={title:'Actual X source',creator:null,similarity:null,thumbnailData:null,sourceMetadata:[],links:[{label:'X',url:'https://x.com/NASA'}]};
  const fixture=mockApi(dual({phase:'complete',message},{phase:'complete',candidates:[candidate]}));await render(fixture.api);
  const sauce=dialog()!.querySelector('[aria-label="SauceNAO 검색 결과"]')!;
  expect(sauce).toHaveTextContent(message);expect(sauce).not.toHaveTextContent('검색 실패');expect(sauce.querySelector('[role="alert"]')).toBeNull();
  await click('SauceNAO 원본 검색 화면');expect(fixture.api.openOriginal).toHaveBeenCalledWith(ID,'saucenao');
  await click('Actual X source');expect(fixture.api.openResult).toHaveBeenCalledWith(ID,0,0,'lens');expect(fixture.api.confirm).not.toHaveBeenCalled();
});
it("opens indexed sources through the title and compact inline destination without separate link cards",async()=>{
  const candidate={title:"Observed work",creator:null,similarity:null,thumbnailData:null,sourceMetadata:[],links:[{label:"Source",url:"https://example.org/work"},{label:"Publisher",url:"https://publisher.example.org/item"}]};
  const fixture=mockApi(dual({phase:"complete"},{phase:"complete",candidates:[candidate]}));await render(fixture.api);
  const row=dialog()!.querySelector('.x-media-search-candidates li')!;
  expect(row.querySelector('.x-media-search-links')).toBeNull();
  const title=row.querySelector<HTMLButtonElement>('.x-media-search-title')!;
  expect(title).toHaveTextContent("Observed work");expect(title.querySelector('small')).toHaveTextContent('example.org');
  await act(async()=>title.click());expect(fixture.api.openResult).toHaveBeenLastCalledWith(ID,0,0,'lens');
  await act(async()=>row.querySelector<HTMLButtonElement>('.x-media-search-destination')!.click());expect(fixture.api.openResult).toHaveBeenLastCalledWith(ID,0,1,'lens');
});
it("refreshes only terminal Lens results and keeps the accepted refresh through late terminal events",async()=>{
  const candidate=(title:string)=>({title,creator:null,similarity:null,links:[{label:"source",url:"https://example.org/work"}],thumbnailData:null,sourceMetadata:[]});
  const original=dual({phase:"complete",candidates:[candidate("Sauce stays")]},{phase:"complete",candidates:[candidate("Blurred Lens")]});
  const fixture=mockApi(original);
  vi.mocked(fixture.api.refreshLensResults).mockResolvedValue(dual({phase:"complete",candidates:[candidate("Wrong Sauce")]},{phase:"complete",candidates:[candidate("Refreshed Lens")]}));
  await render(fixture.api);
  await click("Google Lens 흐림 설정");
  expect(fixture.api.openLensSettings).toHaveBeenCalledWith(ID);
  await click("Google Lens 결과 새로고침");
  expect(fixture.api.refreshLensResults).toHaveBeenCalledWith(ID);
  expect(dialog()?.textContent).toContain("Refreshed Lens");
  expect(dialog()?.textContent).toContain("Sauce stays");
  expect(dialog()?.textContent).not.toContain("Wrong Sauce");
  await fixture.emit(original);
  expect(dialog()?.textContent).toContain("Refreshed Lens");
  expect(fixture.api.confirm).not.toHaveBeenCalled();
});
it("keeps old results on Lens refresh failure, supports a closed page and retires a late refresh on privacy change",async()=>{
  const original=dual({phase:"complete"},{phase:"complete",originalPageState:"closed",candidates:[{title:"old Lens",creator:null,similarity:null,links:[],thumbnailData:null,sourceMetadata:[]}]});
  const fixture=mockApi(original);await render(fixture.api);
  expect(button("Google Lens 흐림 설정").disabled).toBe(true);
  expect(button("Google Lens 결과 새로고침").disabled).toBe(false);
  vi.mocked(fixture.api.refreshLensResults).mockRejectedValueOnce(new Error("private provider text"));
  await click("Google Lens 결과 새로고침");expect(dialog()?.textContent).toContain("old Lens");expect(dialog()?.textContent).not.toContain("private provider text");
  const pending=deferred<XSearchSnapshot>();vi.mocked(fixture.api.refreshLensResults).mockReturnValueOnce(pending.promise);
  await click("Google Lens 결과 새로고침");
  await render(fixture.api,{privacyMode:true});
  await act(async()=>pending.resolve(dual({phase:"complete"},{phase:"complete"})));
  expect(dialog()).toBeNull();expect(fixture.api.cancel).toHaveBeenCalledWith(ID);
});
it("disables Lens refresh while another provider is searching and after expiry",async()=>{
  const fixture=mockApi(dual({}, {phase:"complete"}));await render(fixture.api);
  expect(button("Google Lens 흐림 설정").disabled).toBe(false);
  expect(button("Google Lens 결과 새로고침").disabled).toBe(true);
  await fixture.emit(dual({phase:"complete"},{phase:"complete",originalPageState:"expired"}));
  expect(button("Google Lens 흐림 설정").disabled).toBe(true);
  expect(button("Google Lens 결과 새로고침").disabled).toBe(true);
});
it("closes the active search for a new supported navigation even when X generation is unchanged",async()=>{
  const fixture=mockApi(dual({phase:"complete"},{phase:"complete"}));
  await render(fixture.api,{navigationRequestId:"retained"});expect(dialog()).not.toBeNull();
  await render(fixture.api,{navigationRequestId:"retained"});expect(fixture.api.cancel).not.toHaveBeenCalled();
  await render(fixture.api,{navigationRequestId:"new-navigation"});
  expect(dialog()).toBeNull();expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
  await fixture.emit(dual({phase:"complete"},{phase:"complete"}));expect(dialog()).toBeNull();
  await fixture.emit(dual({phase:"complete"},{phase:"complete"},{id:NEXT}));expect(dialog()).not.toBeNull();
  await render(fixture.api,{navigationRequestId:"new-navigation"});expect(dialog()).not.toBeNull();
});
it("renders independently completed results while the other provider searches and keeps both original pages reachable",async()=>{
  const result={title:"First result",creator:null,similarity:53.75,thumbnailData:null,sourceMetadata:[],links:[{label:"Pixiv",url:"https://www.pixiv.net/artworks/123"}]};
  const fixture=mockApi(dual({phase:"complete",candidates:[result]}));await render(fixture.api);
  expect(dialog()?.querySelectorAll('.x-media-search-provider')).toHaveLength(2);expect(dialog()).toHaveTextContent("First result");expect(dialog()).toHaveTextContent("검색 중");
  await click("First result");expect(fixture.api.openResult).toHaveBeenCalledExactlyOnceWith(ID,0,0,"saucenao");
  await click("Google Lens 원본 검색 화면");expect(fixture.api.openOriginal).toHaveBeenCalledExactlyOnceWith(ID,"lens");
  await fixture.emit(dual({phase:"complete",candidates:[result]},{phase:"complete"}));
  expect(dialog()).toHaveTextContent("First result");expect(dialog()).toHaveTextContent("검색 결과 없음");expect(fixture.api.confirm).not.toHaveBeenCalled();
});
it("retains failed and empty provider browsers independently and updates retained-page lifetime after completion",async()=>{
  const fixture=mockApi(dual({phase:"failed",message:"서비스 오류"},{phase:"complete"}));await render(fixture.api);
  expect(dialog()).toHaveTextContent("검색 실패");expect(dialog()).toHaveTextContent("검색 결과 없음");
  await click("SauceNAO 원본 검색 화면");await click("Google Lens 원본 검색 화면");
  expect(fixture.api.openOriginal).toHaveBeenNthCalledWith(1,ID,"saucenao");expect(fixture.api.openOriginal).toHaveBeenNthCalledWith(2,ID,"lens");
  await fixture.emit(dual({phase:"failed",message:"ignored replacement",originalPageState:"expired"},{phase:"complete"}));
  expect(button("SauceNAO 원본 검색 화면")).toBeDisabled();expect(button("Google Lens 원본 검색 화면")).not.toBeDisabled();expect(dialog()).toHaveTextContent("서비스 오류");
});
it("resumes only the provider awaiting verification without re-uploading or hiding the other results",async()=>{
  const fixture=mockApi(dual({phase:"complete"},{phase:"verification"}));vi.mocked(fixture.api.resume).mockResolvedValue(dual({phase:"complete"}));await render(fixture.api);
  await click("확인 완료 후 계속");expect(fixture.api.resume).toHaveBeenCalledExactlyOnceWith(ID,"lens");expect(fixture.api.confirm).not.toHaveBeenCalled();expect(dialog()).toHaveTextContent("검색 결과 없음");
});
it("does not query publishers from unrelated result candidates when no original publisher is known",async()=>{
  const fixture=mockApi(dual({phase:"complete",candidates:[{title:"Weak match",creator:null,similarity:51,thumbnailData:null,sourceMetadata:[],links:[{label:"X",url:"https://x.com/other/status/123"}]}]},{phase:"complete"}));await render(fixture.api);
  expect(fixture.api.creatorLinks).not.toHaveBeenCalled();expect(dialog()?.querySelector('.x-media-search-creator')).toBeNull();
});

it("keeps downloaded media search independent from X login and cancels on archive context changes",async()=>{
  const fixture=mockApi(snap({source:"archive",archiveId:ID,assetName:"image.jpg",localContext:ID,accountId:null,generation:null,feed:null,pane:"archive",imageUrl:null,previewData:"data:image/jpeg;base64,/9j/2Q=="}));
  await render(fixture.api,{accountId:null,generation:null,localContext:ID});
  expect(dialog()).toHaveTextContent("다운로드한 파일");expect(dialog()?.querySelector('img')?.alt).toBe("검색할 다운로드 이미지");
  await render(fixture.api,{accountId:null,generation:null,localContext:NEXT});
  expect(dialog()).toBeNull();expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
});
it.each(["complete","failed"] as const)("opens the retained provider page for %s without repeating upload or losing preview metadata",async phase=>{
  const fixture=mockApi(snap({phase,originalPageState:"available",message:"서비스 응답",candidates:[{title:"Observed",creator:null,similarity:50.25,thumbnailData:"data:image/jpeg;base64,/9j/2Q==",sourceMetadata:[{label:"Material",value:"Original"}],links:[]}]}));
  await render(fixture.api);
  expect(dialog()?.querySelector('.x-media-search-result-image')?.getAttribute('src')).toBe("data:image/jpeg;base64,/9j/2Q==");
  expect(dialog()).toHaveTextContent("Material");expect(dialog()).toHaveTextContent("Original");expect(dialog()).toHaveTextContent("50.25%");
  await click("SauceNAO 원본 검색 화면");expect(fixture.api.openOriginal).toHaveBeenCalledExactlyOnceWith(ID);
  expect(fixture.api.confirm).not.toHaveBeenCalled();expect(fixture.api.resume).not.toHaveBeenCalled();
});
it("automatically shows original publisher links in a separate group without a lookup button",async()=>{
  const fixture=mockApi(snap({phase:"complete",sourcePostUrl:"https://x.com/fixture/status/12345"}));
  vi.mocked(fixture.api.creatorLinks).mockResolvedValue({profileUrl:"https://x.com/fixture",observedAt:"2026-10-24T12:00:00Z",message:"Long provider explanation",links:[{label:"Pixiv",url:"https://www.pixiv.net/users/123"}]});
  await render(fixture.api);
  expect(fixture.api.creatorLinks).toHaveBeenCalledExactlyOnceWith(ID);
  expect(button("게시자 프로필 링크 확인")).toBeUndefined();
  const group=dialog()?.querySelector('.x-media-search-creator');
  expect(group).toHaveTextContent("게시자 프로필");expect(group).toHaveTextContent("@fixture");expect(group).not.toHaveTextContent("Long provider explanation");
  expect(dialog()?.querySelector('.x-media-search-candidates')).toBeNull();
  await act(async()=>group!.querySelector<HTMLButtonElement>('button')!.click());
  expect(fixture.api.openCreatorLink).toHaveBeenCalledExactlyOnceWith(ID,0);
  await fixture.emit(snap({phase:"complete",sourcePostUrl:"https://x.com/fixture/status/12345",originalPageState:"closed"}));
  expect(fixture.api.creatorLinks).toHaveBeenCalledOnce();expect(group).toHaveTextContent("Pixiv");
});
it("does not block consent or results while automatic publisher links are pending, and discards late results after close",async()=>{
  const fixture=mockApi(snap({sourcePostUrl:"https://x.com/fixture/status/12345"}));
  const pending=deferred<Awaited<ReturnType<XMediaSearchApi['creatorLinks']>>>();vi.mocked(fixture.api.creatorLinks).mockReturnValue(pending.promise);
  await render(fixture.api);expect(button("확인 후 전송")).not.toBeDisabled();
  await click("확인 후 전송");expect(fixture.api.confirm).toHaveBeenCalledExactlyOnceWith(ID,false);
  await fixture.emit(snap({phase:"complete",sourcePostUrl:"https://x.com/fixture/status/12345",originalPageState:"available"}));
  expect(button("SauceNAO 원본 검색 화면")).not.toBeDisabled();expect(fixture.api.creatorLinks).toHaveBeenCalledOnce();
  await click("검색 창 닫기");
  await act(async()=>pending.resolve({profileUrl:"https://x.com/fixture",observedAt:"2026-10-24T12:00:00Z",message:"Late",links:[]}));expect(dialog()).toBeNull();
});
it("keeps profile failures separate from successful provider results and never queries result candidates",async()=>{
  const fixture=mockApi(snap({phase:"complete",sourcePostUrl:"https://x.com/fixture/status/12345",candidates:[{title:"Result",creator:null,similarity:50,thumbnailData:null,sourceMetadata:[],links:[{label:"Other",url:"https://x.com/unrelated/status/54321"}]}]}));
  vi.mocked(fixture.api.creatorLinks).mockRejectedValue(new Error("private diagnostic"));await render(fixture.api);
  expect(dialog()).toHaveTextContent("프로필 링크를 불러오지 못함");expect(dialog()).toHaveTextContent("Result");
  expect(dialog()).not.toHaveTextContent("private diagnostic");expect(button("Result")).not.toBeDisabled();
  expect(fixture.api.creatorLinks).toHaveBeenCalledExactlyOnceWith(ID);expect(button("이 X 게시자의 프로필 링크 확인")).toBeUndefined();
});

it("shows both providers and exact selection before any transmission with an unchecked remember choice",async()=>{
  const fixture=mockApi();await render(fixture.api);
  expect(dialog()?.closest('[data-native-overlay="true"]')).not.toBeNull();expect(dialog()).toHaveTextContent("SauceNAO");expect(dialog()).toHaveTextContent("Google Lens");expect(dialog()).toHaveTextContent("한 장을 두 서비스에 보내 검색");
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
it.each(['consent','searching','complete','failed'] as const)("labels the exact thumbnail as a thumbnail, not a decoded video frame, in %s",async phase=>{
  const value=snap({kind:'thumbnail',provider:'lens',phase,imageUrl:null,previewData:'data:image/jpeg;base64,/9j/2Q=='}),fixture=mockApi(value);await render(fixture.api);
  expect(dialog()?.querySelector('img')?.getAttribute('src')).toBe(value.previewData);
  expect(dialog()?.querySelector('img')?.alt).toBe(phase==='consent'?'검색할 영상 썸네일':'검색에 사용한 영상 썸네일');
  expect(dialog()?.querySelector('.x-media-search-selection')).toHaveTextContent('썸네일');
  expect(fixture.api.confirm).not.toHaveBeenCalled();expect(fixture.api.openResult).not.toHaveBeenCalled();
});
it.each(["searching","verification","complete","failed"] as const)("keeps the actual submitted frame visible in %s, even when consent was remembered",async phase=>{
  const previewData="data:image/jpeg;base64,/9j/2Q==",fixture=mockApi(snap({kind:"video",provider:"lens",imageUrl:null,previewData,phase}));
  await render(fixture.api);
  expect(dialog()?.querySelector('img[alt="검색에 사용한 영상 장면"]')?.getAttribute('src')).toBe(previewData);
  expect(dialog()?.querySelectorAll('img')).toHaveLength(1);expect(dialog()?.querySelector('iframe,video,a')).toBeNull();
  expect(fixture.api.confirm).not.toHaveBeenCalled();expect(fixture.api.resume).not.toHaveBeenCalled();expect(fixture.api.openResult).not.toHaveBeenCalled();
  await click('검색 창 닫기');expect(document.querySelector(`img[src="${previewData}"]`)).toBeNull();expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
});
it.each(['/goto','/url'])("describes an unresolved Lens %s link instead of presenting Google as the source",async path=>{
  const url=`https://www.google.com${path}?url=CAESopaqueObservedToken`,fixture=mockApi(snap({kind:"video",provider:"lens",imageUrl:null,previewData:"data:image/jpeg;base64,/9j/2Q==",phase:"complete",candidates:[{thumbnailData:null,sourceMetadata:[],title:"Observed candidate",creator:null,similarity:null,links:[{label:"www.google.com",url}]}]}));
  await render(fixture.api);
  const link=dialog()?.querySelector<HTMLButtonElement>('.x-media-search-title');
  expect(link).toHaveTextContent('Observed candidate');expect(link).toHaveTextContent('google.com · 주소 확인 필요');
  expect(link).not.toHaveTextContent('www.google.com');expect(link?.title).toBe(url);
  await act(async()=>link!.click());expect(fixture.api.openResult).toHaveBeenCalledExactlyOnceWith(ID,0,0);
});
it("shows the actual resolved destination and keeps native indexed opening",async()=>{
  const url='https://www.pixiv.net/artworks/12345',fixture=mockApi(snap({kind:'video',imageUrl:null,previewData:'data:image/jpeg;base64,/9j/2Q==',provider:'lens',phase:'complete',candidates:[{title:'Public illustration',creator:null,similarity:null,thumbnailData:null,sourceMetadata:[],links:[{label:'www.pixiv.net',url}]}]}));
  await render(fixture.api);const link=dialog()!.querySelector<HTMLButtonElement>('.x-media-search-title')!;
  expect(link).toHaveTextContent('www.pixiv.net');expect(link).not.toHaveTextContent('Google Lens');expect(link.title).toBe(url);
  await act(async()=>link.click());expect(fixture.api.openResult).toHaveBeenCalledExactlyOnceWith(ID,0,0);
});
it("keeps an unresolved wrapped result visible and points to its retained search without retrying or exposing private errors",async()=>{
  const fixture=mockApi(snap({kind:"video",provider:"lens",imageUrl:null,previewData:"data:image/jpeg;base64,/9j/2Q==",phase:"complete",candidates:[{thumbnailData:null,sourceMetadata:[],title:"Retained candidate",creator:null,similarity:null,links:[{label:"www.google.com",url:"https://www.google.com/goto?url=observed"}]}]}));
  vi.mocked(fixture.api.openResult).mockRejectedValue("SEARCH_RESULT_REDIRECT_UNRESOLVED");await render(fixture.api);
  const link=dialog()?.querySelector<HTMLButtonElement>('.x-media-search-title');await act(async()=>link!.click());
  expect(dialog()).toHaveTextContent("Retained candidate");expect(dialog()).toHaveTextContent("열린 검색 화면에서 링크를 확인");
  expect(dialog()).not.toHaveTextContent("SEARCH_RESULT_REDIRECT_UNRESOLVED");expect(fixture.api.openResult).toHaveBeenCalledOnce();expect(fixture.api.confirm).not.toHaveBeenCalled();
});
it("renders truthful creator candidates and fractional similarity, opening links solely by exact indices",async()=>{
  const value=snap({phase:"complete",candidates:[{thumbnailData:null,sourceMetadata:[],title:"<script>not HTML</script>",creator:null,similarity:83.25,links:[{label:"Source",url:"https://example.org/art"}]},{thumbnailData:null,sourceMetadata:[],title:"Second",creator:"Explicit author",similarity:null,links:[]}]}),fixture=mockApi(value);await render(fixture.api);
  expect(dialog()).toHaveTextContent("83.25%");expect(dialog()?.querySelector('.x-media-search-score')).toHaveAttribute("title","서비스의 유사도 점수 · 원작자 확인 아님");expect(dialog()?.querySelector("script,a,img,iframe")).toBeNull();
  expect(dialog()?.querySelectorAll("li")[0]).not.toHaveTextContent("작가 후보");expect(dialog()?.querySelectorAll("li")[1]).toHaveTextContent("Explicit author");
  await click("<script>not HTML</script>");expect(fixture.api.openResult).toHaveBeenCalledExactlyOnceWith(ID,0,0);expect(fixture.api.confirm).not.toHaveBeenCalled();
  await click("검색 전송 동의 초기화");expect(fixture.api.resetConsent).toHaveBeenCalledOnce();expect(notice).toHaveBeenCalledWith("다음 검색부터 전송 동의를 다시 확인합니다.");
});
it.each([snap({phase:"complete"}),snap({phase:"failed",imageUrl:null,message:"인증이 필요합니다. 자동으로 통과하지 않습니다."}),snap({phase:"searching",message:"결과 확인 중"})])("uses native empty/failure/searching messages without claiming completion: %j",async value=>{
  const fixture=mockApi(value);await render(fixture.api);expect(dialog()).toHaveTextContent(value.message||"검색 결과 없음");expect(fixture.api.confirm).not.toHaveBeenCalled();
});
it("does not let an initial late status response overwrite a newer event",async()=>{
  const fixture=mockApi(),status=deferred<XSearchSnapshot|null>();vi.mocked(fixture.api.status).mockReturnValue(status.promise);await render(fixture.api);
  await fixture.emit(snap({phase:"searching",message:"Live search"}));await act(async()=>status.resolve(snap()));expect(dialog()).toHaveTextContent("Live search");expect(button("확인 후 전송")).toBeUndefined();
});
it("keeps native events ahead of stale confirm responses and ignores phase regression",async()=>{
  const fixture=mockApi(),confirm=deferred<XSearchSnapshot|null>();vi.mocked(fixture.api.confirm).mockReturnValue(confirm.promise);await render(fixture.api);await click("확인 후 전송");
  await fixture.emit(snap({phase:"complete",message:"Native final"}));await act(async()=>confirm.resolve(snap({phase:"searching"})));expect(dialog()).toHaveTextContent("Native final");expect(button("검색 취소")).toBeUndefined();
  await fixture.emit(snap({phase:"consent"}));expect(dialog()).toHaveTextContent("Native final");expect(button("검색 취소")).toBeUndefined();
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
const consentSettingsError="검색 동의 설정을 저장하지 못했습니다. 다시 묻지 않기를 해제하고 다시 검색해 주세요.";
it.each([consentSettingsError,new Error(consentSettingsError)])("keeps the exact consent settings failure actionable without changing the selection or retrying: %s",async failure=>{
  const fixture=mockApi();vi.mocked(fixture.api.confirm).mockRejectedValueOnce(failure);await render(fixture.api);
  const mounted=dialog(),remember=document.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
  await act(async()=>remember.click());await click("확인 후 전송");
  expect(dialog()).toBe(mounted);expect(dialog()?.querySelector('[role="alert"]')).toHaveTextContent(consentSettingsError);expect(dialog()).toHaveTextContent(snap().imageUrl!);
  expect(remember).toBeChecked();expect(remember).not.toBeDisabled();expect(button("확인 후 전송")).not.toBeDisabled();expect(button("취소")).not.toBeDisabled();
  expect(fixture.api.confirm).toHaveBeenCalledExactlyOnceWith(ID,true);expect(fixture.api.resetConsent).not.toHaveBeenCalled();expect(fixture.api.cancel).not.toHaveBeenCalled();
  await click("취소");expect(dialog()).toBeNull();expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);expect(fixture.api.confirm).toHaveBeenCalledOnce();
});
it("lets the user explicitly uncheck remember and retry after a consent settings failure",async()=>{
  const fixture=mockApi();vi.mocked(fixture.api.confirm).mockRejectedValueOnce(consentSettingsError);await render(fixture.api);
  const remember=document.querySelector<HTMLInputElement>('input[type="checkbox"]')!;await act(async()=>remember.click());await click("확인 후 전송");
  expect(dialog()).toHaveTextContent(consentSettingsError);await act(async()=>remember.click());await click("확인 후 전송");
  expect(fixture.api.confirm).toHaveBeenNthCalledWith(1,ID,true);expect(fixture.api.confirm).toHaveBeenNthCalledWith(2,ID,false);expect(fixture.api.confirm).toHaveBeenCalledTimes(2);
  expect(dialog()).toHaveTextContent("검색 중");expect(dialog()?.querySelector('[role="alert"]')).toBeNull();expect(fixture.api.resetConsent).not.toHaveBeenCalled();
});
it.each(["private backend body",new Error("private backend body"),consentSettingsError+" private detail",new Error(consentSettingsError+" private detail"),{message:consentSettingsError}])("suppresses unknown or non-exact confirmation failures and retains explicit cancel: %j",async failure=>{
  const fixture=mockApi();vi.mocked(fixture.api.confirm).mockRejectedValueOnce(failure);await render(fixture.api);await click("확인 후 전송");
  expect(dialog()?.querySelector('[role="alert"]')?.textContent).toBe("검색을 시작하지 못했습니다. 다시 확인해 주세요.");expect(dialog()).not.toHaveTextContent("private");
  expect(document.querySelector('input[type="checkbox"]')).not.toBeChecked();expect(fixture.api.confirm).toHaveBeenCalledExactlyOnceWith(ID,false);expect(fixture.api.resetConsent).not.toHaveBeenCalled();
  await click("취소");expect(dialog()).toBeNull();expect(fixture.api.cancel).toHaveBeenCalledExactlyOnceWith(ID);
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
  expect(css).toContain("width:min(1120px,100%)");expect(css).toContain("max-height:calc(100dvh - 40px)");expect(css).toContain("overflow:auto;min-height:0");expect(css).toContain("object-fit:contain");expect(css).toContain("prefers-reduced-motion:reduce");expect(css).toContain("max-height:calc(100dvh - 16px)");
});
it("explains the bounded manual handoff and isolated persisted session without resubmitting or automating verification",async()=>{
  const fixture=mockApi(snap({phase:"verification",message:"사람 확인이 필요합니다."}));await render(fixture.api);
  expect(dialog()).toHaveTextContent("확인 필요");expect(dialog()).toHaveTextContent("사람 확인이 필요합니다.");
  expect(dialog()?.querySelector("iframe,webview,img,a,input")).toBeNull();expect(button("확인 후 전송")).toBeUndefined();expect(button("검색 전송 동의 초기화")).toBeDefined();
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
  expect(dialog()).toHaveTextContent("Native final");expect(button("검색 취소")).toBeUndefined();expect(button("확인 완료 후 계속")).toBeUndefined();expect(fixture.api.resume).toHaveBeenCalledOnce();
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
