import { describe, expect, it } from "vitest";
import { alignPageSizeToColumns } from "./pageSizeAlignment";

describe("alignPageSizeToColumns", () => {
  it("rounds a configured target up to a complete final row", () => {
    expect(alignPageSizeToColumns(50, 3, 200)).toBe(51);
    expect(alignPageSizeToColumns(50, 4, 200)).toBe(52);
    expect(alignPageSizeToColumns(60, 6, 100)).toBe(60);
  });

  it("uses the largest complete row when the source limit blocks rounding up", () => {
    expect(alignPageSizeToColumns(100, 6, 100)).toBe(96);
    expect(alignPageSizeToColumns(200, 3, 200)).toBe(198);
  });

  it("normalizes invalid geometry without exceeding the source limit", () => {
    expect(alignPageSizeToColumns(Number.NaN, 0, 100)).toBe(1);
    expect(alignPageSizeToColumns(10, 200, 100)).toBe(100);
  });
});
