import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,describe,expect,it,vi} from "vitest";
import {ArchivePost} from "./XArchivePost";
import {createXArchiveApi,type XArchiveApi,type XArchiveAsset,type XArchiveRecord,type XCapturePost} from "./xArchiveApi";

vi.mock("@tauri-apps/api/core",()=>({invoke:vi.fn(),convertFileSrc:(path:string,protocol:string)=>`http://${protocol}.localhost/${encodeURIComponent(path)}`}));
vi.mock("hls.js",()=>({default:class{static isSupported(){return false;}}}));
const ID="11111111-1111-4111-8111-111111111111",SECOND="22222222-2222-4222-8222-222222222222",CAPTURE="33333333-3333-4333-8333-333333333333";
const OWN="https://pbs.twimg.com/media/own?format=png&name=small",QUOTE="https://pbs.twimg.com/media/quote?format=png&name=medium";
const counts={replies:"1",reposts:"2",likes:"3",bookmarks:"4",views:"5"};
const post=():XCapturePost=>({id:"123",url:"https://x.com/reader/status/123",author:"Reader",body:"Saved body",timestamp:"2026-10-21T00:00:00Z",counts:"",context:"",quotes:[],truncated:false,
  media:[{kind:"image",url:OWN,alt:"Own observed alt"},{kind:"image",url:QUOTE,alt:"Quote observed alt"}],
  presentation:{handle:"reader",displayName:"Reader",verified:null,counts,relation:"root",conversationId:"123",order:0,mediaUrls:[OWN],
    quotePresentation:{handle:"quoted",displayName:"Quoted",verified:null,body:"Saved quote",timestamp:"",mediaUrls:[QUOTE]}},
});
const asset=(sourceUrl:string,name:string,original=true):XArchiveAsset=>({name,mime:"image/png",bytes:20,sha256:"a".repeat(64),postId:"123",kind:"image",sourceUrl,...(original?{downloadUrl:sourceUrl.replace(/name=[^&]+/,"name=orig")}:{})});
const record=(original=true):XArchiveRecord=>({version:1,id:ID,captureId:CAPTURE,accountId:"saved-account",savedAt:1792540800000,intent:"archive",document:{version:1,owner:"reader",sourceUrl:"https://x.com/reader/status/123",capturedAt:"2026-10-21T00:01:00Z",coverage:"loaded_dom_only",records:[post()],limitReached:false},assets:[asset(OWN,`asset-${ID}.png`,original),asset(QUOTE,`asset-${SECOND}.png`,original)],issues:[],selectedContentComplete:true,mediaRequested:true,htmlSha256:"b".repeat(64)});
const api=():XArchiveApi=>({...createXArchiveApi("browser-mock"),mediaUrl:vi.fn(createXArchiveApi("tauri").mediaUrl),save:vi.fn(),openSource:vi.fn(),inspect:vi.fn()});
let host:HTMLDivElement,root:Root;
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();vi.clearAllMocks();vi.unstubAllGlobals();});
const render=(saved:XArchiveRecord,adapter:XArchiveApi)=>act(async()=>root.render(<ArchivePost record={saved} post={saved.document.records[0]!} api={adapter} detail/>));

describe("original image assets retain frozen source association and local-only viewing",()=>{
  it.each([false,true])("keeps own/quote attribution and alt text with original provenance=%s",async original=>{
    const saved=record(original),adapter=api();await render(saved,adapter);
    const own=host.querySelector<HTMLImageElement>('img[alt="Own observed alt"]')!,quoted=host.querySelector<HTMLImageElement>('img[alt="Quote observed alt"]')!;
    expect(own).not.toBeNull();expect(quoted).not.toBeNull();expect(own.closest('.x-archive-quote-card')).toBeNull();expect(quoted.closest('.x-archive-quote-card')).not.toBeNull();
    expect(own.src).toBe(`http://x-archive.localhost/${ID}/asset-${ID}.png`);expect(quoted.src).toBe(`http://x-archive.localhost/${ID}/asset-${SECOND}.png`);
    expect(host.innerHTML).not.toContain("pbs.twimg.com");expect(adapter.save).not.toHaveBeenCalled();expect(adapter.inspect).not.toHaveBeenCalled();expect(adapter.openSource).not.toHaveBeenCalled();
    expect(vi.mocked(adapter.mediaUrl).mock.calls.every(([id,name])=>id===ID&&saved.assets.some(asset=>asset.name===name))).toBe(true);
    expect(saved.assets[0]?.sourceUrl).toBe(OWN);expect(saved.assets[1]?.sourceUrl).toBe(QUOTE);
  });
  it("retains two own/quote occurrences of one committed original resource",async()=>{
    const saved=record(),adapter=api(),item=saved.document.records[0]!;
    item.media=item.media.slice(0,1);item.presentation!.quotePresentation!.mediaUrls=[OWN];saved.assets=saved.assets.slice(0,1);
    await render(saved,adapter);const images=host.querySelectorAll<HTMLImageElement>('img[alt="Own observed alt"]');
    expect(images).toHaveLength(2);expect(images[0]?.src).toBe(images[1]?.src);expect(host.querySelector('.x-archive-unassigned-media')).toBeNull();
  });
  it("opens the committed original in the local viewer without refetching either source URL",async()=>{
    const saved=record(),adapter=api();await render(saved,adapter);
    const image=host.querySelector<HTMLImageElement>('img[alt="Own observed alt"]')!;
    await act(async()=>image.closest('button')!.click());
    const dialog=document.querySelector<HTMLElement>('[role="dialog"][aria-label="저장된 이미지"]')!;
    expect(dialog.querySelector('img')?.getAttribute('src')).toBe(image.src);expect(dialog.querySelector('img')?.getAttribute('alt')).toBe("Own observed alt");
    await act(async()=>dialog.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true})));
    expect(document.querySelector('[role="dialog"]')).toBeNull();expect(adapter.save).not.toHaveBeenCalled();expect(adapter.inspect).not.toHaveBeenCalled();expect(adapter.openSource).not.toHaveBeenCalled();
  });
  it.each([
    ["original_image_url_unavailable","원본 이미지 주소를 확인하지 못해 저장하지 않았습니다."],
    ["original_image_request_failed","원본 이미지를 끝까지 받지 못해 저장하지 않았습니다."],
    ["original_image_validation_failed","원본 이미지 검증에 실패해 저장하지 않았습니다."],
  ])("keeps %s visibly partial instead of presenting a saved thumbnail",async(reason,message)=>{
    const saved=record(),adapter=api();saved.assets=[];saved.selectedContentComplete=false;saved.issues=[{postId:"123",kind:"image",reason}];
    await render(saved,adapter);expect(host.textContent).toContain(message);expect(host.querySelector('.x-archive-timeline-images')).toBeNull();expect(adapter.mediaUrl).not.toHaveBeenCalled();
  });
});
