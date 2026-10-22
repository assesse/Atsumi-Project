import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { fitReplayLayout, useReplayLayout } from "./RecordingReplayLayout";

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });
describe("replay picture-first layout", () => {
  it("removes the old artificial 1600px/920px limits and uses a large screen", () => {
    const layout = fitReplayLayout(3440, 1440, 16 / 9);
    expect(layout.width).toBeGreaterThan(2500);
    expect(layout.videoWidth).toBeGreaterThan(2400);
    expect(layout.videoHeight).toBe(1402);
    expect(layout.stacked).toBe(false);
  });
  it("does not keep a 918px stage around a 541px picture", () => {
    const layout = fitReplayLayout(1351, 958, 16 / 9);
    expect(layout.videoWidth).toBeGreaterThan(961.75);
    expect(layout.stacked ? layout.height - layout.chatSize : layout.height).toBeCloseTo(layout.videoHeight, 7);
  });
  it("puts chat below a landscape picture in a narrow/tall window", () => {
    const layout = fitReplayLayout(600, 900, 16 / 9);
    expect(layout.stacked).toBe(true);
    expect(layout.videoWidth).toBe(586);
    expect(layout.height).toBeCloseTo(layout.videoHeight + layout.chatSize);
    expect(layout.chatSize).toBeGreaterThanOrEqual(160);
    expect(layout.height + 2 * (layout.padding + layout.border)).toBe(900);
  });
  it("keeps a readable side rail in a short wide window and for portrait recordings", () => {
    expect(fitReplayLayout(1280, 720, 16 / 9).stacked).toBe(false);
    const portrait = fitReplayLayout(1920, 1080, 9 / 16);
    expect(portrait.stacked).toBe(false);
    expect(portrait.videoHeight).toBe(1042);
    expect(portrait.chatSize).toBe(353);
  });
  it("never crops, stretches, overflows, or hides chat across viewport/aspect combinations", () => {
    for (const width of [320, 600, 780, 1280, 1920, 3440]) for (const height of [240, 480, 720, 1080, 1600]) {
      for (const aspect of [.05, 9 / 16, 4 / 3, 16 / 9, 21 / 9, 20]) for (const expanded of [false, true]) {
        const layout = fitReplayLayout(width, height, aspect, expanded);
        expect(layout.videoWidth / layout.videoHeight).toBeCloseTo(aspect, 8);
        expect(layout.width + 2 * (layout.padding + layout.border)).toBeLessThanOrEqual(width + .001);
        expect(layout.height + 2 * (layout.padding + layout.border)).toBeLessThanOrEqual(height + .001);
        expect(layout.videoWidth).toBeLessThanOrEqual(layout.width + .001);
        expect(layout.videoHeight).toBeLessThanOrEqual(layout.height + .001);
        expect(layout.chatSize).toBeGreaterThan(0);
        expect(expanded ? layout.padding + layout.border : 1).toBe(expanded ? 0 : 1);
      }
    }
  });
  it("falls back safely before metadata and for non-finite dimensions", () => {
    expect(fitReplayLayout(NaN, Infinity, NaN)).toEqual(fitReplayLayout(1024, 768, 16 / 9));
    expect(fitReplayLayout(1280, 720, 0)).toEqual(fitReplayLayout(1280, 720, 16 / 9));
  });
  it("coalesces resize events and cancels work/listeners when replay closes", async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    let callback: FrameRequestCallback | undefined;
    const schedule = vi.fn((next: FrameRequestCallback) => { callback = next; return 7; });
    const cancel = vi.fn();
    vi.stubGlobal("requestAnimationFrame", schedule); vi.stubGlobal("cancelAnimationFrame", cancel);
    const element = document.createElement("div"), root = createRoot(element);
    function Surface() { const layout = useReplayLayout(16 / 9, false); return <div data-layout={layout.stacked ? "stacked" : "side"} />; }
    await act(async () => root.render(<Surface />));
    await act(async () => { for (let i = 0; i < 30; i++) window.dispatchEvent(new Event("resize")); });
    expect(schedule).toHaveBeenCalledTimes(1);
    await act(async () => callback!(0));
    await act(async () => window.dispatchEvent(new Event("resize")));
    expect(schedule).toHaveBeenCalledTimes(2);
    await act(async () => root.unmount());
    expect(cancel).toHaveBeenCalledWith(7);
    window.dispatchEvent(new Event("resize"));
    expect(schedule).toHaveBeenCalledTimes(2);
  });
});
