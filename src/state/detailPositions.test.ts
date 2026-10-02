import { beforeEach, describe, expect, it } from "vitest";
import { adjacentPreviewPages, readDetailPositions, saveDetailPosition } from "./detailPositions";
beforeEach(() => sessionStorage.clear());
describe("bounded detail navigation memory", () => {
  it("prefetches precisely two neighbours on each side, including spreads and endpoints", () => {
    expect(adjacentPreviewPages(5,5,20)).toEqual([3,4,6,7]);
    expect(adjacentPreviewPages(5,6,20)).toEqual([3,4,7,8]);
    expect(adjacentPreviewPages(1,1,2)).toEqual([2]);
    expect(adjacentPreviewPages(20,20,20)).toEqual([18,19]);
  });
  it("preserves both preview and scroll and retains at most 96 recently touched albums", () => {
    for(let id=1;id<=120;id++) saveDetailPosition(id,{previewStart:10});
    saveDetailPosition(120,{scrollTop:450});
    expect(readDetailPositions().size).toBe(96);
    expect(readDetailPositions().has(1)).toBe(false);
    expect(readDetailPositions().get(120)).toEqual({previewStart:10,scrollTop:450});
  });
});
