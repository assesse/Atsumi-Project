import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { attachDetailScrollSnap, detailScrollStops, measureDetailScrollStops, nextDetailScrollStop } from "./detailScrollSnap";

const cleanup: (() => void)[] = [];
beforeEach(() => vi.useFakeTimers());
afterEach(() => { for (const fn of cleanup.splice(0)) fn(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

function fixture() {
  const body = document.createElement("div"); body.style.paddingTop = "18px";
  body.innerHTML = '<div class="preview-window"><button class="preview-thumb"></button></div><section class="related-section"><article class="related-card"></article></section>';
  document.body.append(body);
  let height = 700, total = 2700, relatedTop = 518, previewTop = 1000;
  Object.defineProperties(body, { clientHeight: { get: () => height }, scrollHeight: { get: () => total } });
  vi.spyOn(body, "getBoundingClientRect").mockImplementation(() => ({ top: 100 } as DOMRect));
  vi.spyOn(body.querySelector(".related-section")!, "getBoundingClientRect").mockImplementation(() => ({ top: 100 + relatedTop - body.scrollTop } as DOMRect));
  vi.spyOn(body.querySelector(".related-card")!, "getBoundingClientRect").mockImplementation(() => ({ top: 100 + relatedTop + 40 - body.scrollTop, height: 300 } as DOMRect));
  vi.spyOn(body.querySelector(".preview-window")!, "getBoundingClientRect").mockImplementation(() => ({ top: 100 + previewTop - body.scrollTop, height: 700 } as DOMRect));
  const scroll = vi.fn((options: ScrollToOptions) => { body.scrollTop = options.top ?? body.scrollTop; });
  Object.defineProperty(body, "scrollTo", { configurable: true, value: scroll });
  const detach = attachDetailScrollSnap(body); cleanup.push(() => { detach(); body.remove(); });
  const wheel = (deltaY = 100, target: Element = body, extra: WheelEventInit = {}) => {
    const event = new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY, ...extra });
    target.dispatchEvent(event); return event;
  };
  const nativeScroll = (top: number) => { body.scrollTop = top; body.dispatchEvent(new Event("scroll")); };
  return { body, scroll, wheel, detach, nativeScroll, resize: () => { height = 900; total = 2100; relatedTop = 318; previewTop = 800; } };
}

describe("detail proximity scroll stops", () => {
  it("aligns related heading, preview center, and final scroll edge", () => {
    const stops = detailScrollStops({ viewportHeight: 700, scrollHeight: 2700, paddingTop: 18, relatedTop: 518, previewTop: 1000, previewHeight: 700 });
    expect(stops.map(p => p.top)).toEqual([0,500,1000,2000]);
    expect(nextDetailScrollStop(stops,500,1)?.top).toBe(1000);
    expect(nextDetailScrollStop(stops,500,-1)?.top).toBe(0);
    expect(nextDetailScrollStop(stops,0,-1)).toBeUndefined();
  });
  it("merges close preview/related stops and retains distant variable-height cards", () => {
    const stops = detailScrollStops({ viewportHeight: 700, scrollHeight: 3000, paddingTop: 18,
      relatedTop: 518, previewTop: 560, previewHeight: 700,
      relatedCards: [{top:558,height:300},{top:866,height:900},{top:1774,height:330}] });
    expect(stops.map(p=>p.top)).toEqual([0,500,848,1084,1756,2300]);
    expect(stops.filter(p=>p.top>=500&&p.top<=600)).toHaveLength(1);
  });
  it("clamps short content and remeasures current geometry without resetting position", () => {
    expect(detailScrollStops({viewportHeight:800,scrollHeight:800,paddingTop:18,relatedTop:200})).toEqual([{kind:"top",top:0}]);
    const f=fixture(); f.body.scrollTop=420; f.body.querySelector(".related-card")!.remove();
    expect(measureDetailScrollStops(f.body).map(p=>p.top)).toEqual([0,1000,2000]);
    expect(f.scroll).not.toHaveBeenCalled();
    f.resize(); expect(measureDetailScrollStops(f.body).map(p=>p.top)).toEqual([0,700,1200]);
  });
});

describe("native wheel with a soft idle settle", () => {
  it("never consumes a fast upward burst and allows native scrolling all the way to top", () => {
    const f=fixture(); f.nativeScroll(2000);
    for (const top of [1500,1000,500,0]) { expect(f.wheel(-500).defaultPrevented).toBe(false); f.nativeScroll(top); vi.advanceTimersByTime(30); }
    vi.advanceTimersByTime(180); expect(f.body.scrollTop).toBe(0); expect(f.scroll).not.toHaveBeenCalled();
  });
  it("settles only nearby, in the last gesture direction, after wheel and native momentum stop", () => {
    const f=fixture(); f.wheel(); f.nativeScroll(430); vi.advanceTimersByTime(150);
    f.nativeScroll(450); vi.advanceTimersByTime(150); expect(f.scroll).not.toHaveBeenCalled();
    vi.advanceTimersByTime(30); expect(f.scroll).toHaveBeenLastCalledWith({top:500,behavior:"smooth"});
    f.wheel(); f.nativeScroll(600); vi.advanceTimersByTime(180); expect(f.scroll).toHaveBeenCalledTimes(1);
    f.wheel(-10); f.nativeScroll(550); vi.advanceTimersByTime(180); expect(f.scroll).toHaveBeenCalledTimes(2); expect(f.body.scrollTop).toBe(500);
  });
  it("does not skip cards or snap over distant stops", () => {
    const f=fixture(); for (const top of [650,750,850,1150,1250,1350,1450]) {
      f.wheel(); f.nativeScroll(top); vi.advanceTimersByTime(180); expect(f.body.scrollTop).toBe(top);
    }
    expect(f.scroll).not.toHaveBeenCalled();
  });
  it("cancels an unfinished settle on fresh input without blocking that input", () => {
    const f=fixture(); f.scroll.mockImplementation(()=>{}); f.wheel(); f.nativeScroll(450); vi.advanceTimersByTime(180);
    expect(f.scroll).toHaveBeenLastCalledWith({top:500,behavior:"smooth"});
    expect(f.wheel(-100).defaultPrevented).toBe(false);
    expect(f.scroll).toHaveBeenLastCalledWith({top:450,behavior:"instant"});
    f.nativeScroll(0); vi.advanceTimersByTime(180); expect(f.body.scrollTop).toBe(0);
  });
  it("leaves editor, modifiers, horizontal and nested scroll input alone and cancels pending alignment", () => {
    const f=fixture(); const input=document.createElement("input"); f.body.append(input);
    const nested=document.createElement("div"); nested.style.overflowY="auto";
    Object.defineProperties(nested,{scrollHeight:{value:500},clientHeight:{value:100}}); f.body.append(nested);
    for (const action of [()=>f.wheel(100,input),()=>f.wheel(100,f.body,{ctrlKey:true}),()=>f.wheel(5,f.body,{deltaX:100}),()=>f.wheel(100,nested)]) {
      f.wheel(); f.nativeScroll(450); expect(action().defaultPrevented).toBe(false); vi.advanceTimersByTime(180);
    }
    expect(f.scroll).not.toHaveBeenCalled();
  });
  it("keeps scrollbar, keyboard, resize, blur and detach independent", () => {
    const f=fixture();
    for (const action of [()=>f.body.dispatchEvent(new Event("pointerdown")),()=>f.body.dispatchEvent(new KeyboardEvent("keydown",{key:"Home"})),()=>window.dispatchEvent(new Event("resize")),()=>window.dispatchEvent(new Event("blur")),()=>f.detach()]) {
      f.wheel(); f.nativeScroll(450); action(); vi.advanceTimersByTime(180);
    }
    expect(f.scroll).not.toHaveBeenCalled();
  });
  it("respects reduced motion even for tiny unconsumed trackpad input", () => {
    vi.stubGlobal("matchMedia",vi.fn(()=>({matches:true}))); const f=fixture();
    expect(f.wheel(3).defaultPrevented).toBe(false); f.nativeScroll(490); vi.advanceTimersByTime(180);
    expect(f.scroll).toHaveBeenLastCalledWith({top:500,behavior:"instant"});
  });
});
