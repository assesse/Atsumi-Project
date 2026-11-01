import { act, useEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { XSplitPane } from "./XSplitPane";
let host:HTMLDivElement,root:Root;
const key="atsumi.x.feed-preview-ratio.v1";
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);localStorage.removeItem(key);host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();localStorage.removeItem(key);vi.restoreAllMocks();vi.unstubAllGlobals();});
const render=()=>act(async()=>root.render(<XSplitPane><div>Feed</div><aside>Preview</aside></XSplitPane>));
const separator=()=>host.querySelector<HTMLElement>('[role="separator"]')!;
const keyboard=(key:string,shiftKey=false)=>act(async()=>separator().dispatchEvent(new KeyboardEvent("keydown",{key,shiftKey,bubbles:true,cancelable:true})));
const point=(target:EventTarget,type:string,x:number,pointer=1)=>act(async()=>{const event=new MouseEvent(type,{clientX:x,button:0,buttons:type==="pointerup"?0:1,bubbles:true,cancelable:true});Object.defineProperty(event,"pointerId",{value:pointer});target.dispatchEvent(event);});
it("provides keyboard resize, bounds, persistence and reset without touching the pane content",async()=>{
  await render();const feed=host.querySelector('.x-body')!.firstElementChild,preview=host.querySelector('aside');
  await keyboard("ArrowRight");expect(separator().getAttribute("aria-valuenow")).toBe("50");expect(Number(localStorage.getItem(key))).toBe(.5);
  await keyboard("End");expect(separator().getAttribute("aria-valuenow")).toBe("78");await keyboard("ArrowRight",true);expect(separator().getAttribute("aria-valuenow")).toBe("78");
  await keyboard("Home");expect(separator().getAttribute("aria-valuenow")).toBe("22");
  await act(async()=>separator().dispatchEvent(new MouseEvent("dblclick",{bubbles:true})));expect(localStorage.getItem(key)).toBeNull();expect(separator().getAttribute("aria-valuenow")).toBe("48");
  expect(host.querySelector('.x-body')!.firstElementChild).toBe(feed);expect(host.querySelector('aside')).toBe(preview);
});
it("follows only the captured drag pointer and keeps the last applied ratio on release",async()=>{
  await render();const body=host.querySelector<HTMLElement>('.x-body')!;
  vi.spyOn(body,'getBoundingClientRect').mockReturnValue({left:100,width:1008,top:0,right:1108,height:800,bottom:800,x:100,y:0,toJSON(){}});
  vi.spyOn(separator(),'getBoundingClientRect').mockReturnValue({left:580,width:8,top:0,right:588,height:800,bottom:800,x:580,y:0,toJSON(){}});
  await point(separator(),"pointerdown",584);await point(window,"pointermove",804,2);expect(separator().getAttribute("aria-valuenow")).toBe("48");
  await point(window,"pointermove",804);expect(separator().getAttribute("aria-valuenow")).toBe("70");await point(window,"pointerup",804);
  expect(Number(localStorage.getItem(key))).toBe(.7);expect(body.classList.contains("is-resizing")).toBe(false);
  await point(window,"pointermove",304);expect(separator().getAttribute("aria-valuenow")).toBe("70");
});
it("rejects corrupt stored layout values and does not remount retained content while resizing",async()=>{
  localStorage.setItem(key,'NaN');const mounted=vi.fn(),removed=vi.fn();
  function Feed(){useEffect(()=>{mounted();return removed;},[]);return <div>Native feed slot</div>;}
  await act(async()=>root.render(<XSplitPane><Feed/><aside>Preview</aside></XSplitPane>));await keyboard("ArrowLeft");await keyboard("ArrowRight");
  expect(mounted).toHaveBeenCalledOnce();expect(removed).not.toHaveBeenCalled();expect(separator().getAttribute("aria-valuenow")).toBe("48");
});
