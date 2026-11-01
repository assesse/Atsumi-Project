import { act, useRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { overlapReviewSizeKey, routeOverlapWheel, useOverlapReviewSize } from "./overlapReviewViewport";

function scrollFixture(innerHeight = 500) {
  const outer = document.createElement("div"); outer.className = "review-scroll";
  const inner = document.createElement("div"); outer.append(inner);
  Object.defineProperties(outer, { clientHeight:{value:500}, scrollHeight:{value:1600} });
  Object.defineProperties(inner, { clientHeight:{value:200}, scrollHeight:{value:innerHeight}, clientWidth:{value:400}, scrollWidth:{value:2000} });
  return { outer, inner };
}
const wheel = (options: WheelEventInit) => new WheelEvent("wheel", { cancelable:true, ...options });

describe("comparison wheel routing", () => {
  it("uses vertical inner scrolling without shifting horizontal position", () => {
    const {inner,outer}=scrollFixture();inner.scrollLeft=120;outer.scrollTop=200;
    const event=wheel({deltaY:90});expect(routeOverlapWheel(event,inner)).toBe(true);
    expect(inner.scrollTop).toBe(90);expect(outer.scrollTop).toBe(200);expect(inner.scrollLeft).toBe(120);expect(event.defaultPrevented).toBe(true);
  });
  it("passes unused downward and upward distance to the outer dialog", () => {
    const {inner,outer}=scrollFixture();inner.scrollTop=280;outer.scrollTop=200;
    routeOverlapWheel(wheel({deltaY:100}),inner);
    expect(inner.scrollTop).toBe(300);expect(outer.scrollTop).toBe(280);
    inner.scrollTop=20;routeOverlapWheel(wheel({deltaY:-100}),inner);
    expect(inner.scrollTop).toBe(0);expect(outer.scrollTop).toBe(200);
  });
  it("scrolls the dialog when every comparison row already fits", () => {
    const {inner,outer}=scrollFixture(200);outer.scrollTop=300;
    routeOverlapWheel(wheel({deltaY:-120}),inner);
    expect(inner.scrollTop).toBe(0);expect(outer.scrollTop).toBe(180);
  });
  it("handles both browser Shift-wheel delta shapes horizontally, without vertical movement", () => {
    const {inner,outer}=scrollFixture();inner.scrollTop=20;outer.scrollTop=200;
    routeOverlapWheel(wheel({shiftKey:true,deltaY:120}),inner);
    routeOverlapWheel(wheel({shiftKey:true,deltaX:90}),inner);
    expect(inner.scrollLeft).toBe(210);expect(inner.scrollTop).toBe(20);expect(outer.scrollTop).toBe(200);
  });
  it("uses vertical movement for unmodified horizontal input and preserves zoom gestures", () => {
    const {inner}=scrollFixture();routeOverlapWheel(wheel({deltaX:80}),inner);
    expect(inner.scrollTop).toBe(80);expect(inner.scrollLeft).toBe(0);
    for(const key of ["ctrlKey","metaKey"]) {
      const event=wheel({deltaY:80,[key]:true});expect(routeOverlapWheel(event,inner)).toBe(false);expect(event.defaultPrevented).toBe(false);
    }
    expect(inner.scrollTop).toBe(80);
  });
  it("normalizes line/page wheel units and clamps at boundaries", () => {
    const {inner}=scrollFixture();
    routeOverlapWheel(wheel({deltaY:2,deltaMode:1}),inner);expect(inner.scrollTop).toBe(32);
    routeOverlapWheel(wheel({deltaY:1,deltaMode:2,shiftKey:true}),inner);expect(inner.scrollLeft).toBe(400);
    routeOverlapWheel(wheel({deltaY:-10000,shiftKey:true}),inner);expect(inner.scrollLeft).toBe(0);
  });
});

let root: Root | undefined, container: HTMLDivElement | undefined;
let resize: (()=>void) | undefined;
function Harness({enabled=true}:{enabled?:boolean}) {
  const ref=useRef<HTMLDialogElement>(null);useOverlapReviewSize(ref,enabled);
  return <dialog ref={ref} open />;
}
async function mount(enabled=true) {
  if(!container){container=document.createElement("div");document.body.append(container);root=createRoot(container);}
  await act(async()=>root!.render(<Harness enabled={enabled}/>));
  return container.querySelector("dialog")!;
}
function setup() {
  vi.useFakeTimers();
  vi.stubGlobal("ResizeObserver",class{constructor(callback:()=>void){resize=callback;}observe(){}disconnect(){}});
}
afterEach(async()=>{
  if(root)await act(async()=>root!.unmount());
  root=undefined;container?.remove();container=undefined;resize=undefined;
  localStorage.removeItem(overlapReviewSizeKey);vi.useRealTimers();vi.unstubAllGlobals();vi.restoreAllMocks();
});
describe("review dialog size preference",()=>{
  it("saves a native resize and restores it on a new mount",async()=>{
    setup();const dialog=await mount();
    dialog.style.width="1510px";dialog.style.height="850px";
    resize?.();await vi.advanceTimersByTimeAsync(210);
    expect(JSON.parse(localStorage.getItem(overlapReviewSizeKey)!)).toEqual({width:1510,height:850});
    await act(async()=>root!.unmount());root=undefined;container?.remove();container=undefined;
    const reopened=await mount();expect(reopened.style.width).toBe("1510px");expect(reopened.style.height).toBe("850px");
  });
  it("flushes a just-resized size when closing immediately",async()=>{
    setup();const dialog=await mount();dialog.style.width="1400px";dialog.style.height="790px";
    resize?.();await mount(false);
    expect(JSON.parse(localStorage.getItem(overlapReviewSizeKey)!)).toEqual({width:1400,height:790});
    expect(dialog.style.width).toBe("");expect(dialog.style.height).toBe("");
    await mount(true);expect(dialog.style.width).toBe("1400px");expect(dialog.style.height).toBe("790px");
  });
  it("does not replace normal size with maximize or viewport-clamped bounds",async()=>{
    setup();localStorage.setItem(overlapReviewSizeKey,JSON.stringify({width:1700,height:900}));
    const dialog=await mount();dialog.classList.add("is-expanded");
    vi.spyOn(dialog,"getBoundingClientRect").mockReturnValue({width:1920,height:1080} as DOMRect);
    resize?.();await vi.advanceTimersByTimeAsync(210);
    expect(JSON.parse(localStorage.getItem(overlapReviewSizeKey)!)).toEqual({width:1700,height:900});
    dialog.classList.remove("is-expanded");
    vi.mocked(dialog.getBoundingClientRect).mockReturnValue({width:700,height:650} as DOMRect);
    resize?.();await vi.advanceTimersByTimeAsync(210);
    expect(JSON.parse(localStorage.getItem(overlapReviewSizeKey)!)).toEqual({width:1700,height:900});
  });
  it.each(['invalid','{"width":"800","height":600}','{"width":null,"height":600}','{"width":90000,"height":600}'])("ignores malformed stored size %s",async(value)=>{
    setup();localStorage.setItem(overlapReviewSizeKey,value);const dialog=await mount();
    expect(dialog.style.width).toBe("");expect(dialog.style.height).toBe("");
  });
  it("does not save untouched default dimensions",async()=>{
    setup();await mount();resize?.();await vi.advanceTimersByTimeAsync(210);
    expect(localStorage.getItem(overlapReviewSizeKey)).toBeNull();
  });
});
