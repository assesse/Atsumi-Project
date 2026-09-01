import { describe, expect, it } from "vitest";
import {
  clampPagePreviewResizeBox,
  resizePagePreviewBox,
} from "./pagePreviewResize";

describe("pagePreviewResize", () => {
  const start = { left: 300, top: 200, width: 600, height: 500 };
  const viewport = { width: 1200, height: 900 };

  it("keeps the viewport center fixed while resizing both sides symmetrically", () => {
    expect(resizePagePreviewBox(start, "right", 80, 60, viewport)).toEqual({ left: 220, top: 200, width: 760, height: 500 });
    expect(resizePagePreviewBox(start, "bottom", 80, 60, viewport)).toEqual({ left: 300, top: 140, width: 600, height: 620 });
    expect(resizePagePreviewBox(start, "corner", 80, 60, viewport)).toEqual({ left: 220, top: 140, width: 760, height: 620 });
  });

  it("clamps drag sizes between 320px and the remaining viewport", () => {
    expect(resizePagePreviewBox(start, "corner", -1000, -1000, viewport)).toEqual({
      left: 440,
      top: 290,
      width: 320,
      height: 320,
    });
    expect(resizePagePreviewBox(start, "corner", 5000, 5000, viewport)).toEqual({
      left: 12,
      top: 12,
      width: 1176,
      height: 876,
    });
  });

  it("recenters and clamps an existing box inside a smaller viewport", () => {
    expect(clampPagePreviewResizeBox(
      { left: 700, top: 500, width: 700, height: 600 },
      { width: 800, height: 600 },
    )).toEqual({ left: 50, top: 12, width: 700, height: 576 });
  });
});
