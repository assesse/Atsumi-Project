import { describe, expect, it } from "vitest";
import { spotlightRect, tourPlacement } from "./tourGeometry";
describe("guided tour geometry", () => {
  it("clips the highlight to the visible viewport", () => {
    expect(spotlightRect({ left: -40, top: -10, width: 300, height: 1200 }, 960, 640)).toEqual({ left: 4, top: 4, width: 262, height: 632 });
    expect(spotlightRect({ left: 1000, top: 0, width: 100, height: 30 }, 960, 640)).toBeNull();
  });
  it.each([[960, 640], [1280, 820], [1920, 1080], [360, 640]])("keeps the bubble inside %ix%i", (width, height) => {
    for (const target of [{ left: 8, top: 8, width: 180, height: 40 }, { left: width - 48, top: 8, width: 40, height: 40 }, { left: 100, top: height - 50, width: 160, height: 40 }, null]) {
      const result = tourPlacement(target, { width, height }, { width: 360, height: 320 });
      expect(result.bubble.left).toBeGreaterThanOrEqual(16); expect(result.bubble.top).toBeGreaterThanOrEqual(16);
      expect(result.bubble.left + result.bubble.width).toBeLessThanOrEqual(width - 16);
      expect(result.bubble.top + result.bubble.height).toBeLessThanOrEqual(height - 16);
      expect(result.line === null).toBe(target === null);
    }
  });
  it("leaves the target uncovered when there is side space", () => {
    const result = tourPlacement({ left: 8, top: 300, width: 180, height: 40 }, { width: 960, height: 640 }, { width: 360, height: 320 });
    expect(result.bubble.left).toBeGreaterThan(188);
  });
});
