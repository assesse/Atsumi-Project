import { describe, expect, it } from "vitest";
import type { DownloadOverlapCandidate } from "../api/contracts";
import { galleryId } from "../core/types";
import {
  buildDownloadOverlapAlignment,
  formatPageRanges,
  uniquePagesForSide,
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
});
