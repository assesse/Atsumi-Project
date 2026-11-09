import {act} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {XCollectionControls} from "./XCollectionControls";
import type {XSurfaceApi} from "./api";
let root:Root,host:HTMLDivElement;
beforeEach(()=>{localStorage.clear();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();localStorage.clear();vi.restoreAllMocks();vi.unstubAllGlobals();});
const api=()=>({collectionView:vi.fn().mockResolvedValue(undefined)}) as unknown as XSurfaceApi;
const draw=(provided:XSurfaceApi,feed:"likes"|"bookmarks"="likes",enabled=true,generation=1)=>act(async()=>root.render(<XCollectionControls key={feed} api={provided} feed={feed} enabled={enabled} generation={generation} onError={vi.fn()}/>));
const click=(text:string)=>act(async()=>[...host.querySelectorAll("button")].find(n=>n.textContent===text)!.click());
it("defaults to original and remembers independent collection preferences",async()=>{
 const a=api();await draw(a);expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","original",expect.any(Number));
 await click("카드 보기");expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","posts",expect.any(Number));
 await act(async()=>{const select=host.querySelector("select")!;select.value="media";select.dispatchEvent(new Event("change",{bubbles:true}));});
 expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","media",expect.any(Number));
 await draw(a,"bookmarks");expect(a.collectionView).toHaveBeenLastCalledWith(1,"bookmarks","original",expect.any(Number));
 await draw(a);expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","media",expect.any(Number));
 await click("기본 보기");await click("카드 보기");expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","media",expect.any(Number));
});
it("never changes a hidden/private/loading native surface and rebinds to the current generation",async()=>{
 const a=api();await draw(a,"likes",false);expect(a.collectionView).not.toHaveBeenCalled();
 expect([...host.querySelectorAll("button")].every(n=>n.disabled)).toBe(true);
 await draw(a,"likes",true,9);await click("카드 보기");
 expect(a.collectionView).toHaveBeenLastCalledWith(9,"likes","posts",expect.any(Number));
 await draw(a,"likes",true,10);expect(a.collectionView).toHaveBeenLastCalledWith(10,"likes","posts",expect.any(Number));
});
it("survives invalid/unavailable presentation storage",async()=>{
 localStorage.setItem("atsumi.x.collection-view.v1.likes",'{"cards":true,"basis":"invalid"}');
 const a=api();await draw(a);expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","original",expect.any(Number));
 vi.spyOn(Storage.prototype,"setItem").mockImplementation(()=>{throw Error("denied");});
 await click("카드 보기");expect(a.collectionView).toHaveBeenLastCalledWith(1,"likes","posts",expect.any(Number));
});
