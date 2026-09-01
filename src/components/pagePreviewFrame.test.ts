import { describe, expect, it } from "vitest";
import {
  pagePreviewFrame,
  pagePreviewOrientation,
  pagePreviewSpreadDimension,
} from "./pagePreviewFrame";

describe("pagePreviewFrame", () => {
  it("uses the maximum available vertical length for a portrait page", () => {
    const frame = pagePreviewFrame(
      { width: 800, height: 1200 },
      { width: 1600, height: 1000 },
    );
    expect(frame.orientation).toBe("portrait");
    expect(frame.dialogHeight).toBe(952);
    expect(frame.mediaHeight).toBe(824);
    expect(frame.mediaWidth / frame.mediaHeight).toBeCloseTo(2 / 3, 2);
    expect(frame.aspectRatio).toBe("800 / 1200");
  });

  it("becomes width-bound for an extra-wide page without changing its ratio", () => {
    const frame = pagePreviewFrame(
      { width: 2400, height: 800 },
      { width: 1600, height: 1000 },
    );
    expect(frame.orientation).toBe("landscape");
    expect(frame.dialogWidth).toBe(1552);
    expect(frame.dialogHeight).toBeLessThan(952);
    expect(frame.mediaWidth / frame.mediaHeight).toBeCloseTo(3, 2);
  });

  it("uses a stable portrait ratio while dimensions are unavailable", () => {
    const frame = pagePreviewFrame(undefined, { width: 1024, height: 768 });
    expect(frame.orientation).toBe("portrait");
    expect(frame.aspectRatio).toBe("2 / 3");
  });

  it("combines two portrait pages at a shared height for spread view", () => {
    const spread = pagePreviewSpreadDimension(
      { width: 800, height: 1200 },
      { width: 900, height: 1200 },
    );
    expect(spread).toEqual({ width: (2 / 3) + (3 / 4), height: 1 });
    const frame = pagePreviewFrame(spread, { width: 1600, height: 1000 });
    expect(frame.dialogHeight).toBe(952);
    expect(frame.mediaWidth / frame.mediaHeight).toBeCloseTo((2 / 3) + (3 / 4), 2);
  });

  it("does not claim an orientation or spread until both dimensions are valid", () => {
    expect(pagePreviewOrientation(undefined)).toBeUndefined();
    expect(pagePreviewOrientation({ width: 1600, height: 900 })).toBe("landscape");
    expect(pagePreviewSpreadDimension({ width: 800, height: 1200 }, undefined)).toBeUndefined();
  });
});
