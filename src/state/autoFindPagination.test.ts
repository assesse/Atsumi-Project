import { describe, expect, it } from "vitest";
import { paginateAutoFindItems } from "./autoFindPagination";

describe("paginateAutoFindItems", () => {
  it("slices an existing Auto Find snapshot without mutating its order", () => {
    const source = Array.from({ length: 23 }, (_, index) => `candidate-${index + 1}`);

    const result = paginateAutoFindItems(source, 2, 10);

    expect(result).toEqual({
      items: source.slice(10, 20),
      page: 2,
      pageSize: 10,
      totalItems: 23,
      totalPages: 3,
      startIndex: 10,
    });
    expect(source[0]).toBe("candidate-1");
  });

  it("clamps a stale page after candidates are removed or the page size changes", () => {
    const result = paginateAutoFindItems([1, 2, 3, 4, 5], 9, 2);

    expect(result.page).toBe(3);
    expect(result.items).toEqual([5]);
  });

  it("returns a stable empty first page and normalizes invalid inputs", () => {
    expect(paginateAutoFindItems([], Number.NaN, 0)).toEqual({
      items: [],
      page: 1,
      pageSize: 1,
      totalItems: 0,
      totalPages: 1,
      startIndex: 0,
    });
  });
});
