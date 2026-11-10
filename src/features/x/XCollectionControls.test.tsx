import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {XCollectionControls} from "./XCollectionControls";
import { XCollectionSettings } from "./XCollectionSettings";
import { saveXCollectionCardWidth } from "./collectionPreferences";
import type {XSurfaceApi} from "./api";
let root:Root,host:HTMLDivElement;
beforeEach(()=>{localStorage.clear();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();localStorage.clear();vi.restoreAllMocks();vi.unstubAllGlobals();});
const api=()=>({collectionView:vi.fn().mockResolvedValue(undefined)}) as unknown as XSurfaceApi;
const draw=(provided:XSurfaceApi,feed:"likes"|"bookmarks"="likes",enabled=true,generation=1)=>act(async()=>root.render(<XCollectionControls key={feed} api={provided} feed={feed} enabled={enabled} generation={generation} onError={vi.fn()}/>));
const click=(text:string)=>act(async()=>[...host.querySelectorAll("button")].find(n=>n.textContent===text)!.click());
it("defaults to original and remembers independent collection preferences",async()=>{
 const a=api();await draw(a);expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","original",expect.any(Number),220);
 await click("카드 보기");expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","posts",expect.any(Number),220);
 await act(async()=>{const select=host.querySelector("select")!;select.value="media";select.dispatchEvent(new Event("change",{bubbles:true}));});
 expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","media",expect.any(Number),220);
 await draw(a,"bookmarks");expect(a.collectionView).toHaveBeenLastCalledWith(1,"bookmarks","original",expect.any(Number),220);
 await draw(a);expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","media",expect.any(Number),220);
 await click("기본 보기");await click("카드 보기");expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","media",expect.any(Number),220);
});
it("never changes a hidden/private/loading native surface and rebinds to the current generation",async()=>{
 const a=api();await draw(a,"likes",false);expect(a.collectionView).not.toHaveBeenCalled();
 expect([...host.querySelectorAll("button")].every(n=>n.disabled)).toBe(true);
 await draw(a,"likes",true,9);await click("카드 보기");
 expect(a.collectionView).toHaveBeenLastCalledWith(9,"likes","posts",expect.any(Number),220);
 await draw(a,"likes",true,10);expect(a.collectionView).toHaveBeenLastCalledWith(10,"likes","posts",expect.any(Number),220);
});
it("survives invalid/unavailable presentation storage",async()=>{
 localStorage.setItem("atsumi.x.collection-view.v1.likes",'{"cards":true,"basis":"invalid"}');
 const a=api();await draw(a);expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","original",expect.any(Number),220);
 vi.spyOn(Storage.prototype,"setItem").mockImplementation(()=>{throw Error("denied");});
 await click("카드 보기");expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","posts",expect.any(Number),220);
});

it("settings update the mounted collection, retain old sizes and never reset its view",async()=>{
 localStorage.setItem("atsumi.x.collection-view.v1.likes",JSON.stringify({cards:true,basis:"media",width:300}));
 const a=api();
 await act(async()=>root.render(<><XCollectionControls api={a} feed="likes" enabled generation={1} onError={vi.fn()}/><XCollectionSettings/></>));
 expect(host.querySelector('.x-collection-view-controls input')).toBeNull();
 const likes=host.querySelector<HTMLInputElement>('[aria-label="마음에 들어요 카드 크기"]')!;
 const bookmarks=host.querySelector<HTMLInputElement>('[aria-label="북마크 카드 크기"]')!;
 expect(likes.value).toBe("300");expect(bookmarks.value).toBe("220");
 expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","media",expect.any(Number),300);
 await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(likes,"360");likes.dispatchEvent(new Event("input",{bubbles:true}));});
 expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","media",expect.any(Number),360);
 expect(bookmarks.value).toBe("220");
 await click("기본 보기");await click("카드 보기");
 expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","media",expect.any(Number),360);
 await draw(a,"bookmarks");await click("카드 보기");expect(a.collectionView).toHaveBeenLastCalledWith(1,"bookmarks","posts",expect.any(Number),220);
 await draw(a);expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","media",expect.any(Number),360);
});
it("retains committed size and reports settings save failures",async()=>{
 await act(async()=>root.render(<XCollectionSettings/>));
 vi.spyOn(Storage.prototype,"setItem").mockImplementation(()=>{throw Error("quota");});
 const input=host.querySelector<HTMLInputElement>('input')!;
 await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(input,"280");input.dispatchEvent(new Event("input",{bubbles:true}));});
 expect(input.value).toBe("220");expect(host.querySelector('[role=alert]')).toHaveTextContent('저장하지 못했습니다');
});
it("does not dispatch native changes while hidden when a settings width changes",async()=>{
 const a=api();await draw(a,"likes",false);
 await act(async()=>{saveXCollectionCardWidth("likes",280);});expect(a.collectionView).not.toHaveBeenCalled();
 await draw(a,"likes",true,12);expect(a.collectionView).toHaveBeenLastCalledWith(12,"likes","original",expect.any(Number),280);
});
it.each([null,-1,159,361,220.5,"300"])("discards an invalid saved size %s without losing the view",async width=>{
 localStorage.setItem("atsumi.x.collection-view.v1.likes",JSON.stringify({cards:true,basis:"media",width}));
 const a=api();await draw(a);expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","media",expect.any(Number),220);
});
