import { describe, expect, it } from "vitest";
import type { DownloadOverlapCandidate } from "../api/contracts";
import { galleryId } from "../core/types";
import {
  buildDownloadOverlapAlignment,
  filterDownloadOverlapAlignment,
  formatPageRanges,
  uniquePagesForSide,
  type DownloadOverlapAlignmentColumn,
} from "./alignment";

const candidate = (pagePairs: DownloadOverlapCandidate["pagePairs"]): DownloadOverlapCandidate => ({
  candidateId: "candidate",
  existing: {
    entryId: "existing",
    galleryId: galleryId(1),
    title: "A",
    artists: [],
    pageCount: 11,
  },
  existingFingerprint: "fingerprint",
  relation: "incoming_contains_existing",
  confidence: 0.9,
  matchedPages: pagePairs.length,
  exactPages: 0,
  visualPages: pagePairs.length,
  existingCoverage: 1,
  incomingCoverage: 11 / 16,
  existingUniquePages: 0,
  incomingUniquePages: 5,
  longestAlignedRun: 8,
  rank: 1,
  pagePairs,
});

const pair = (existingSourcePage: number, incomingSourcePage: number) => ({
  existingSourcePage,
  incomingSourcePage,
  exactSha256: false,
  dHashDistance: 1,
  pHashDistance: 1,
  detailHashDistance: 1,
  edgeSimilarity: 0.99,
  visualSimilarity: 0.99,
  lowInformation: false,
});

describe("download overlap page alignment", () => {
  it("leaves A gaps for pages that exist only in containing album B", () => {
    const pairs = [
      ...Array.from({ length: 8 }, (_, index) => pair(index + 1, index + 1)),
      pair(9, 14), pair(10, 15), pair(11, 16),
    ];
    const columns = buildDownloadOverlapAlignment(candidate(pairs), 16);

    expect(columns).toHaveLength(16);
    expect(columns.slice(8, 13).map((column) => [column.existingPage, column.incomingPage]))
      .toEqual([[undefined, 9], [undefined, 10], [undefined, 11], [undefined, 12], [undefined, 13]]);
    expect(uniquePagesForSide(columns, "existing")).toEqual([]);
    expect(uniquePagesForSide(columns, "incoming")).toEqual([9, 10, 11, 12, 13]);
    expect(formatPageRanges(uniquePagesForSide(columns, "incoming"))).toBe("9~13p");
  });

  it("summarizes disjoint unique ranges without implying a match", () => {
    expect(formatPageRanges([1, 2, 3, 7, 9, 10])).toBe("1~3p, 7p, 9~10p");
  });

  const filterColumns: DownloadOverlapAlignmentColumn[] = [
    { key: "matched", existingPage: 1, incomingPage: 1, pair: pair(1, 1) },
    { key: "both-unmatched", existingPage: 2, incomingPage: 2 },
    { key: "only-a", existingPage: 3 },
    { key: "only-b", incomingPage: 3 },
  ];

  it.each(Array.from({ length: 8 }, (_, mask) => mask))("filters independently without changing source indices (mask %i)", (mask) => {
    const visibility = { matched: Boolean(mask & 1), unique: Boolean(mask & 2), gap: Boolean(mask & 4) };
    const before = structuredClone(filterColumns);
    const visible = filterDownloadOverlapAlignment(filterColumns, visibility);
    const expected = [
      [0, visibility.matched, visibility.matched],
      [1, visibility.unique, visibility.unique],
      [2, visibility.unique, visibility.gap],
      [3, visibility.gap, visibility.unique],
    ].filter(([, a, b]) => a || b);
    expect(visible.map(({ sourceIndex, existingVisible, incomingVisible }) => [sourceIndex, existingVisible, incomingVisible])).toEqual(expected);
    for (const item of visible) expect(item.column).toBe(filterColumns[item.sourceIndex]);
    expect(filterColumns).toEqual(before);
  });

  it("keeps the original axis when only gap decorations are hidden", () => {
    const columns = buildDownloadOverlapAlignment(candidate([pair(1, 1), pair(2, 7)]), 16);
    const visible = filterDownloadOverlapAlignment(columns, { matched: true, unique: true, gap: false });
    expect(visible.map(({ column }) => column)).toEqual(columns);
    expect(visible[1]).toMatchObject({ sourceIndex: 1, existingVisible: false, incomingVisible: true });
    expect(visible[6]).toMatchObject({ sourceIndex: 6, column: { existingPage: 2, incomingPage: 7 } });
  });

  it("handles empty input and hides all categories without manufacturing gaps", () => {
    expect(filterDownloadOverlapAlignment([], { matched: true, unique: true, gap: true })).toEqual([]);
    expect(filterDownloadOverlapAlignment(filterColumns, { matched: false, unique: false, gap: false })).toEqual([]);
  });
});
