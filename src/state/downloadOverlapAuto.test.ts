import { describe, expect, it } from "vitest";
import type { DownloadOverlapCandidate, DownloadOverlapReview } from "../api/contracts";
import { galleryId } from "../core/types";
import { buildStrictOverlapPlan } from "./downloadOverlapAuto";

const candidate = (overrides: Partial<DownloadOverlapCandidate> = {}): DownloadOverlapCandidate => ({
  candidateId: "candidate-a",
  existing: { entryId: "entry-a", galleryId: galleryId(100), title: "A", artists: ["artist"], pageCount: 20 },
  existingFingerprint: "a".repeat(64),
  relation: "incoming_contains_existing",
  confidence: 0.99,
  matchedPages: 20,
  exactPages: 20,
  visualPages: 0,
  existingCoverage: 1,
  incomingCoverage: 2 / 3,
  existingUniquePages: 0,
  incomingUniquePages: 10,
  longestAlignedRun: 20,
  rank: 1,
  pagePairs: Array.from({ length: 20 }, (_, index) => ({
    incomingSourcePage: index + 1,
    existingSourcePage: index + 1,
    exactSha256: true,
    dHashDistance: 0,
    pHashDistance: 0,
    detailHashDistance: 0,
    edgeSimilarity: 1,
    visualSimilarity: 1,
    lowInformation: false,
  })),
  ...overrides,
});

const review = (candidates: DownloadOverlapCandidate[]): DownloadOverlapReview => ({
  reviewId: "review-a",
  entryId: "entry-b",
  incoming: { entryId: "entry-b", galleryId: galleryId(200), title: "B", artists: ["artist"], pageCount: 30 },
  revision: 4,
  state: "pending",
  profileVersion: 1,
  policyVersion: 1,
  incomingFingerprint: "b".repeat(64),
  candidates,
  createdAt: "2026-08-31T00:00:00Z",
  updatedAt: "2026-08-31T00:00:00Z",
});

describe("buildStrictOverlapPlan", () => {
  it("keeps an incoming edition only when it safely contains the existing edition", () => {
    const plan = buildStrictOverlapPlan(review([candidate()]));
    expect(plan?.winner).toBe("incoming");
    expect(plan?.steps).toMatchObject([{ action: "remove_existing_continue", candidateId: "candidate-a" }]);
  });

  it("removes an incoming edition when one existing edition safely contains it", () => {
    const containingExisting = candidate({
      relation: "existing_contains_incoming",
      existing: { entryId: "entry-a", galleryId: galleryId(100), title: "A", artists: ["artist"], pageCount: 40 },
      existingCoverage: 0.75,
      incomingCoverage: 1,
      existingUniquePages: 10,
      incomingUniquePages: 0,
      matchedPages: 30,
      exactPages: 30,
      longestAlignedRun: 30,
      pagePairs: Array.from({ length: 30 }, (_, index) => ({
        incomingSourcePage: index + 1,
        existingSourcePage: index + 1,
        exactSha256: true,
        dHashDistance: 0,
        pHashDistance: 0,
        detailHashDistance: 0,
        edgeSimilarity: 1,
        visualSimilarity: 1,
        lowInformation: false,
      })),
    });
    expect(buildStrictOverlapPlan(review([containingExisting]))?.steps[0]?.action).toBe("remove_incoming");
  });

  it("does not automate near-equivalent or low-information comparisons", () => {
    expect(buildStrictOverlapPlan(review([candidate({ relation: "near_equivalent" })]))).toBeNull();
    expect(buildStrictOverlapPlan(review([candidate({
      pagePairs: candidate().pagePairs.map((pair, index) => ({ ...pair, lowInformation: index < 3 })),
    })]))).toBeNull();
  });

  it("requires exact evidence and a meaningful number of additional pages", () => {
    expect(buildStrictOverlapPlan(review([candidate({ exactPages: 17 })]))).toBeNull();
    expect(buildStrictOverlapPlan(review([candidate({ incomingUniquePages: 9 })]))).toBeNull();
  });

  it("removes multiple existing candidates only when incoming strictly wins every direct edge", () => {
    const second = candidate({
      candidateId: "candidate-c",
      existing: { entryId: "entry-c", galleryId: galleryId(300), title: "C", artists: ["artist"], pageCount: 20 },
      existingFingerprint: "c".repeat(64),
      rank: 2,
    });
    expect(buildStrictOverlapPlan(review([candidate(), second]))?.steps).toHaveLength(2);
    expect(buildStrictOverlapPlan(review([candidate(), { ...second, relation: "near_equivalent" }]))).toBeNull();
  });
});
