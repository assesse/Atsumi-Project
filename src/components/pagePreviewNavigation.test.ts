import { describe, expect, it } from "vitest";
import { nextPagePreviewAnchor, pagePreviewSlots } from "./pagePreviewNavigation";

describe("page preview spread navigation", () => {
  it("retains the chosen pairing at both boundaries, in either reading direction", () => {
    expect(nextPagePreviewAnchor(2, 5, true, -1)).toBe(0);
    expect(pagePreviewSlots(0, 5, true, "ltr")).toEqual([null, 1]);
    expect(pagePreviewSlots(0, 5, true, "rtl")).toEqual([1, null]);
    expect(pagePreviewSlots(5, 5, true, "ltr")).toEqual([5, null]);
    expect(pagePreviewSlots(5, 5, true, "rtl")).toEqual([null, 5]);
    expect(nextPagePreviewAnchor(0, 5, true, -1)).toBeNull();
    expect(nextPagePreviewAnchor(1, 5, true, -1)).toBeNull();
    expect(nextPagePreviewAnchor(5, 5, true, 1)).toBeNull();
  });
  it("never sends an empty slot to single-page navigation", () => {
    expect(pagePreviewSlots(1, 1, true, "ltr")).toEqual([1, null]);
    expect(pagePreviewSlots(1, 1, false, "rtl")).toEqual([1]);
    expect(nextPagePreviewAnchor(1, 5, false, -1)).toBeNull();
    expect(nextPagePreviewAnchor(1, 5, false, 1)).toBe(2);
    expect(pagePreviewSlots(null, 5, true, "ltr")).toEqual([]);
  });
});
