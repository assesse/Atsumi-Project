import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,describe,expect,it,vi} from "vitest";
import {ArchivePost,ArchiveMediaTiles} from "./XArchivePost";
import {ArchiveSearchContext,type ArchiveSearchAction} from "./XArchiveSearchContext";
import type {XArchiveApi,XArchiveAsset,XArchiveRecord,XCapturePost} from "./xArchiveApi";

const ID="11111111-1111-4111-8111-111111111111",SECOND="22222222-2222-4222-8222-222222222222";
const asset=(id=ID,kind="image"):XArchiveAsset=>({name:`asset-${id}.${kind==="video"?"mp4":"jpg"}`,mime:kind==="video"?"video/mp4":"image/jpeg",bytes:123,sha256:"a".repeat(64),postId:"10",kind,sourceUrl:`https://pbs.twimg.com/media/${id}.jpg`});
const make=(assets:XArchiveAsset[]):XArchiveRecord=>{
  const post:XCapturePost={id:"10",url:"https://x.com/reader/status/10",author:"Reader",body:"Saved text",timestamp:"",counts:"",context:"",quotes:[],media:assets.map(item=>({kind:item.kind,url:item.sourceUrl,alt:item.name})),truncated:false};
  return{version:1,id:ID,captureId:SECOND,accountId:"legacy",savedAt:1,intent:"media",document:{version:1,owner:"reader",sourceUrl:post.url,capturedAt:"",coverage:"loaded_dom_only",records:[post],limitReached:false},assets,issues:[],selectedContentComplete:true,mediaRequested:true,htmlSha256:"b".repeat(64)};
};
const api={mediaUrl:(id:string,name:string)=>`http://x-archive.localhost/${id}/${name}`,openSource:vi.fn().mockResolvedValue(undefined)} as unknown as XArchiveApi;
let host:HTMLDivElement,root:Root;
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();vi.clearAllMocks();});
const context=async(element:Element)=>act(async()=>element.dispatchEvent(new MouseEvent("contextmenu",{bubbles:true,cancelable:true,clientX:40,clientY:60})));
const action=()=>[...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(item=>item.textContent==="작가·원본 찾기"||item.textContent==="썸네일로 검색 (기본)")!;
describe("downloaded-media exact source search",()=>{
  it("keeps the clicked second asset through reader and enlarged image without a live account",async()=>{
    const record=make([asset(),asset(SECOND)]),search=vi.fn<ArchiveSearchAction>().mockResolvedValue(undefined);
    await act(async()=>root.render(<ArchiveSearchContext.Provider value={search}><ArchivePost record={record} post={record.document.records[0]!} api={api}/></ArchiveSearchContext.Provider>));
    await context(host.querySelector('[aria-label="이미지 2 크게 보기"]')!);
    await act(async()=>action().click());
    expect(search).toHaveBeenLastCalledWith(record,record.document.records[0],record.assets[1]);
    await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="이미지 2 크게 보기"]')!.click());
    await context(document.querySelector('.x-archive-image-viewer img')!);
    await act(async()=>action().click());
    expect(search).toHaveBeenLastCalledWith(record,record.document.records[0],record.assets[1]);
  });
  it("searches the exact stored poster before playback, without reading or starting the video",async()=>{
    const video=asset(ID,"video"),poster=asset(SECOND,"poster"),record=make([video,poster]),search=vi.fn<ArchiveSearchAction>().mockResolvedValue(undefined);
    record.document.records[0]!.media[0]!.poster=poster.sourceUrl;
    await act(async()=>root.render(<ArchiveSearchContext.Provider value={search}><ArchiveMediaTiles record={record} api={api} onOpen={vi.fn()}/></ArchiveSearchContext.Provider>));
    await context(host.querySelector('button')!);
    expect(action().textContent).toBe("썸네일로 검색 (기본)");
    await act(async()=>action().click());
    expect(search).toHaveBeenCalledWith(record,record.document.records[0],poster);
    expect(host.querySelector('video')).toBeNull();
  });
  it("never substitutes another image when a saved video has no poster",async()=>{
    const record=make([asset(ID,"video"),asset(SECOND)]),search=vi.fn<ArchiveSearchAction>().mockResolvedValue(undefined);
    await act(async()=>root.render(<ArchiveSearchContext.Provider value={search}><ArchiveMediaTiles record={record} api={api} onOpen={vi.fn()}/></ArchiveSearchContext.Provider>));
    await context(host.querySelector('button')!);
    expect(action()).toBeUndefined();expect(search).not.toHaveBeenCalled();
  });
  it("revokes a menu when the context owner disappears",async()=>{
    const record=make([asset()]),search=vi.fn<ArchiveSearchAction>().mockResolvedValue(undefined);
    const render=(enabled:boolean)=><ArchiveSearchContext.Provider value={enabled?search:null}><ArchivePost record={record} post={record.document.records[0]!} api={api}/></ArchiveSearchContext.Provider>;
    await act(async()=>root.render(render(true)));await context(host.querySelector('[aria-label="이미지 1 크게 보기"]')!);
    const stale=action();await act(async()=>root.render(render(false)));await act(async()=>stale.click());
    expect(search).not.toHaveBeenCalled();expect(document.querySelector('[role="menu"]')).toBeNull();
  });
});
