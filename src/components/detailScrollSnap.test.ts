import { afterEach, describe, expect, it, vi } from "vitest";
import { attachDetailScrollSnap, detailScrollStops, measureDetailScrollStops, nextDetailScrollStop } from "./detailScrollSnap";

const cleanup: (() => void)[] = [];
afterEach(() => { for (const fn of cleanup.splice(0)) fn(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

function fixture() {
  const body = document.createElement("div");
  body.style.paddingTop = "18px";
  body.innerHTML = '<div class="preview-window"><button class="preview-thumb"></button></div><section class="related-section"><article class="related-card"></article></section>';
  document.body.append(body);
  let height = 700, total = 2700, relatedTop = 518, previewTop = 1000;
  Object.defineProperties(body, { clientHeight: { get: () => height }, scrollHeight: { get: () => total } });
  vi.spyOn(body, "getBoundingClientRect").mockImplementation(() => ({ top: 100 } as DOMRect));
  vi.spyOn(body.querySelector(".related-section")!, "getBoundingClientRect").mockImplementation(() => ({ top: 100 + relatedTop - body.scrollTop } as DOMRect));
  vi.spyOn(body.querySelector(".preview-window")!, "getBoundingClientRect").mockImplementation(() => ({ top: 100 + previewTop - body.scrollTop, height: 700 } as DOMRect));
  const scroll = vi.fn((options: ScrollToOptions) => { body.scrollTop = options.top ?? body.scrollTop; });
  Object.defineProperty(body, "scrollTo", { configurable: true, value: scroll });
  let now = 100;
  vi.spyOn(performance, "now").mockImplementation(() => now);
  const detach = attachDetailScrollSnap(body);
  cleanup.push(() => { detach(); body.remove(); });
  const wheel = (deltaY = 100, target: Element = body, extra: WheelEventInit = {}) => {
    const event = new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY, ...extra });
    target.dispatchEvent(event);
    return event;
  };
  return { body, scroll, wheel, detach, advance: (ms = 220) => { now += ms; }, resize: () => { height = 900; total = 2100; relatedTop = 318; previewTop = 800; } };
}

describe("detail semantic scroll stops", () => {
  it("aligns related heading, preview center, and the final scroll edge", () => {
    const stops = detailScrollStops({ viewportHeight: 700, scrollHeight: 2700, paddingTop: 18, relatedTop: 518, previewTop: 1000, previewHeight: 700 });
    expect(stops).toEqual([{kind:"top",top:0},{kind:"related",top:500},{kind:"previews",top:1000},{kind:"bottom",top:2000}]);
    expect(nextDetailScrollStop(stops, 500, 1)?.top).toBe(1000);
    expect(nextDetailScrollStop(stops, 500, -1)?.top).toBe(0);
    expect(nextDetailScrollStop(stops, 2000, 1)).toBeUndefined();
    expect(nextDetailScrollStop(stops, 0, -1)).toBeUndefined();
  });
  it("clamps and combines coincident stops without reversing the wheel direction", () => {
    expect(detailScrollStops({ viewportHeight: 800, scrollHeight: 800, paddingTop: 18, relatedTop: 200, previewTop: 100, previewHeight: 300 })).toEqual([{kind:"top",top:0}]);
    const stops = detailScrollStops({ viewportHeight: 800, scrollHeight: 1600, paddingTop: 18, relatedTop: 1000, previewTop: 450, previewHeight: 600 });
    expect(stops.map(p => p.top)).toEqual([0,350,800]);
  });
  it("skips absent related galleries and preserves existing tab scroll positions on attach", () => {
    const f = fixture();
    f.body.scrollTop = 420;
    f.body.querySelector(".related-card")!.remove();
    expect(measureDetailScrollStops(f.body).map(p => p.top)).toEqual([0,1000,2000]);
    expect(f.scroll).not.toHaveBeenCalled();
  });
});

describe("detail wheel gestures", () => {
  it("visits all four stops down and up, consuming a burst only once", () => {
    const f = fixture();
    expect(f.wheel().defaultPrevented).toBe(true);
    expect(f.body.scrollTop).toBe(500);
    for (let i=0;i<8;i++) { f.advance(30); f.wheel(); }
    expect(f.scroll).toHaveBeenCalledTimes(1);
    f.advance(); f.wheel(); expect(f.body.scrollTop).toBe(1000);
    f.advance(); f.wheel(); expect(f.body.scrollTop).toBe(2000);
    f.advance(); f.wheel(); expect(f.scroll).toHaveBeenCalledTimes(3);
    f.advance(); f.wheel(-100); expect(f.body.scrollTop).toBe(1000);
    f.advance(); f.wheel(-100); expect(f.body.scrollTop).toBe(500);
    f.advance(); f.wheel(-100); expect(f.body.scrollTop).toBe(0);
    expect(f.scroll).toHaveBeenCalledWith({top:500,behavior:"smooth"});
  });
  it("does not skip a stop while its smooth movement is still running", () => {
    const f = fixture(); f.scroll.mockImplementation(() => {});
    f.wheel(); f.advance(250); f.wheel();
    expect(f.scroll).toHaveBeenCalledTimes(1);
    f.body.scrollTop = 500; f.advance(); f.wheel();
    expect(f.scroll).toHaveBeenLastCalledWith({top:1000,behavior:"smooth"});
  });
  it("remeasures the real positions after resize and reverses during movement", () => {
    const f = fixture(); f.wheel(); f.resize();
    window.dispatchEvent(new Event("resize"));
    f.wheel(); expect(f.body.scrollTop).toBe(700);
    f.scroll.mockImplementation(() => {}); f.advance(); f.wheel();
    f.body.scrollTop = 900; f.wheel(-100);
    expect(f.scroll).toHaveBeenLastCalledWith({top:700,behavior:"smooth"});
  });
  it("leaves editing, zooming, horizontal scrolling, and nested scroll panes alone", () => {
    const f = fixture();
    const input = document.createElement("input"); f.body.append(input);
    expect(f.wheel(100,input).defaultPrevented).toBe(false);
    expect(f.wheel(100,f.body,{ctrlKey:true}).defaultPrevented).toBe(false);
    expect(f.wheel(5,f.body,{deltaX:100}).defaultPrevented).toBe(false);
    const nested = document.createElement("div"); nested.style.overflowY="auto";
    Object.defineProperties(nested,{scrollHeight:{value:500},clientHeight:{value:100}}); f.body.append(nested);
    expect(f.wheel(100,nested).defaultPrevented).toBe(false);
    expect(f.scroll).not.toHaveBeenCalled();
  });
  it("keeps scrollbar, keyboard, and page-preview wheel separate and cleans up", () => {
    const f = fixture(); f.wheel();
    f.body.dispatchEvent(new Event("pointerdown")); f.body.scrollTop=750;
    f.wheel(); expect(f.body.scrollTop).toBe(1000);
    f.body.dispatchEvent(new KeyboardEvent("keydown",{key:"Home"})); f.body.scrollTop=0;
    const preview = document.createElement("dialog"); document.body.append(preview); cleanup.push(()=>preview.remove());
    expect(f.wheel(100,preview).defaultPrevented).toBe(false);
    f.detach(); expect(f.wheel().defaultPrevented).toBe(false);
  });
  it("accumulates small trackpad deltas and respects reduced motion", () => {
    vi.stubGlobal("matchMedia",vi.fn(()=>({matches:true})));
    const f = fixture();
    f.wheel(3); f.advance(20); f.wheel(3); expect(f.scroll).not.toHaveBeenCalled();
    f.advance(20); f.wheel(3); expect(f.scroll).toHaveBeenCalledWith({top:500,behavior:"instant"});
    f.advance(); f.wheel(3); f.advance(20); f.wheel(3);
    expect(f.scroll).toHaveBeenCalledTimes(1);
    f.advance(20); f.wheel(3);
    expect(f.scroll).toHaveBeenLastCalledWith({top:1000,behavior:"instant"});
  });
});
