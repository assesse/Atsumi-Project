import { describe, expect, it } from "vitest";
import type { DownloadOverlapCandidate, DownloadOverlapReview } from "../api/contracts";
import { galleryId } from "../core/types";
import { buildDownloadOverlapContainmentGroup, prioritizeDownloadOverlapReviews } from "./downloadOverlapContainment";

const ref = (id: number, pages: number) => ({ entryId: `entry-${id}`, galleryId: galleryId(id), title: `Album ${id}`, artists: ["artist"], pageCount: pages });
const candidate = (id: number, reversed = false): DownloadOverlapCandidate => ({
  candidateId: `candidate-${id}`, existing: ref(id, reversed ? 155 : 9), existingFingerprint: `${id}`,
  relation: reversed ? "existing_contains_incoming" : "incoming_contains_existing", confidence: .9,
  matchedPages: 9, exactPages: 9, visualPages: 0,
  existingCoverage: reversed ? 9 / 155 : 1, incomingCoverage: reversed ? 1 : 9 / 155,
  existingUniquePages: reversed ? 146 : 0, incomingUniquePages: reversed ? 0 : 146,
  longestAlignedRun: 9, rank: 1,
  pagePairs: Array.from({ length: 9 }, (_, i) => ({
    incomingSourcePage: i + (reversed ? 1 : 83), existingSourcePage: i + (reversed ? 83 : 1),
    exactSha256: true, dHashDistance: 0, pHashDistance: 0, detailHashDistance: 0,
    edgeSimilarity: 1, visualSimilarity: 1, lowInformation: false,
  })),
});
const review = (id: number, candidates: DownloadOverlapCandidate[]): DownloadOverlapReview => ({
  reviewId: `review-${id}`, entryId: `entry-${id}`, incoming: ref(id, id === 3003760 ? 155 : 9),
  revision: 0, state: "pending", profileVersion: 1, policyVersion: 2, incomingFingerprint: `${id}`,
  candidates, createdAt: "2026-09-03", updatedAt: "2026-09-03",
});

describe("compilation review collection", () => {
  it("collects forward and reverse direct containment without adding ambiguous neighbours", () => {
    const parent = review(3003760, [candidate(3003702), { ...candidate(3116207), relation: "translation_edition" }]);
    const child = review(2775264, [candidate(3003760, true), { ...candidate(3118349), relation: "partial_overlap" }]);
    const group = buildDownloadOverlapContainmentGroup(galleryId(3003760), [parent, child]);
    expect(group?.keeper.galleryId).toBe(3003760);
    expect(group?.items.map((item) => [item.excluded.galleryId, item.action])).toEqual([
      [3003702, "remove_existing_continue"], [2775264, "remove_incoming"],
    ]);
    expect(group?.items[1]?.review.reviewId).toBe("review-2775264");
  });

  it("deduplicates the same excluded entry and skips processed or low-information edges", () => {
    const parent = review(3003760, [candidate(2775264), { ...candidate(1), decision: "existing_removed" },
      { ...candidate(2), pagePairs: candidate(2).pagePairs.map((pair) => ({ ...pair, lowInformation: true })) }]);
    const child = review(2775264, [candidate(3003760, true)]);
    expect(buildDownloadOverlapContainmentGroup(galleryId(3003760), [child, parent])?.items)
      .toMatchObject([{ action: "remove_existing_continue", excluded: { galleryId: 2775264 } }]);
    expect(buildDownloadOverlapContainmentGroup(galleryId(3003760), [{ ...parent, state: "resolved" }])).toBeNull();
  });

  it("prioritizes the compilation's own review, then reverse edges, ahead of unrelated reviews", () => {
    const parent = review(3003760, [candidate(3003702), candidate(3002803)]);
    const child = review(2775264, [candidate(3003760, true)]);
    const other = review(1, [{ ...candidate(2), relation: "translation_edition" }]);
    expect(prioritizeDownloadOverlapReviews([other, child, parent]).map((item) => item.reviewId))
      .toEqual([parent.reviewId, child.reviewId, other.reviewId]);
  });
});
