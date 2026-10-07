import { describe, expect, it } from "vitest";
import { fitWindowToWorkArea, type PlacementMonitor } from "./windowWorkArea";
const fullHd = (scaleFactor = 1): PlacementMonitor => ({ workArea: { position: { x: 0, y: 0 }, size: { width: 1920, height: 1040 } }, scaleFactor });
const saved = { x: 100, y: 100, width: 1280, height: 820, maximized: false };
const frame = { width: 16, height: 39 };
describe("native physical window restoration", () => {
  it("does not grow by the titlebar and borders on every launch", () => {
    let next = saved;
    for (let i = 0; i < 10; i++) {
      const result = fitWindowToWorkArea(next, [fullHd()], fullHd(), frame);
      expect(result.inner).toEqual({ width: 1264, height: 781 });
      next = { ...next, ...result.position, ...result.outer };
      expect(next).toEqual(saved);
    }
  });
  it.each([1, 1.25, 1.5, 2])("fits the taskbar work area at scale %s, including oversized saved windows", scale => {
    const monitor = fullHd(scale);
    const result = fitWindowToWorkArea({ ...saved, width: 3840, height: 2160 }, [monitor], monitor, { width: 16 * scale, height: 39 * scale });
    expect(result.position.x + result.outer.width).toBeLessThanOrEqual(1920);
    expect(result.position.y + result.outer.height).toBeLessThanOrEqual(1040);
    expect(result.minimum.height).toBeLessThanOrEqual(result.inner.height);
    expect(result.minimum.width).toBeLessThanOrEqual(result.inner.width);
  });
  it("recovers a disconnected monitor placement onto the current monitor", () => {
    const result = fitWindowToWorkArea({ ...saved, x: 3900, y: -1800 }, [fullHd()], fullHd(), frame);
    expect(result.position).toEqual({ x: 640, y: 0 });
  });
  it("retains a secondary monitor, including negative screen coordinates", () => {
    const secondary = { ...fullHd(), workArea: { position: { x: -1920, y: 40 }, size: { width: 1920, height: 1040 } } };
    const result = fitWindowToWorkArea({ ...saved, x: -1700, y: 60 }, [fullHd(), secondary], fullHd(), frame);
    expect(result.position).toEqual({ x: -1700, y: 60 });
  });
  it("centres the first launch and fits a small VM instead of clipping onboarding", () => {
    const monitor = { ...fullHd(), workArea: { position: { x: 0, y: 0 }, size: { width: 1024, height: 728 } } };
    const result = fitWindowToWorkArea({ ...saved, x: null, y: null }, [monitor], monitor, frame);
    expect(result.outer).toEqual({ width: 1024, height: 728 });
    expect(result.position).toEqual({ x: 0, y: 0 });
  });
});
