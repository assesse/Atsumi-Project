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
  incomingCoverage: 0.8,
  existingUniquePages: 0,
  incomingUniquePages: 5,
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
  incoming: { entryId: "entry-b", galleryId: galleryId(200), title: "B", artists: ["artist"], pageCount: 25 },
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
      existing: { entryId: "entry-a", galleryId: galleryId(100), title: "A", artists: ["artist"], pageCount: 30 },
      existingCoverage: 5 / 6,
      incomingCoverage: 1,
      existingUniquePages: 5,
      incomingUniquePages: 0,
      matchedPages: 25,
      exactPages: 25,
      longestAlignedRun: 25,
      pagePairs: Array.from({ length: 25 }, (_, index) => ({
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

  it("does not automate unsupported or predominantly low-information comparisons", () => {
    expect(buildStrictOverlapPlan(review([candidate({ relation: "partial_overlap" })]))).toBeNull();
    expect(buildStrictOverlapPlan(review([candidate({
      pagePairs: candidate().pagePairs.map((pair, index) => ({ ...pair, lowInformation: index < 6 })),
    })]))).toBeNull();
  });

  it("uses the 95% containment boundary and leaves page gaps over five for manual review", () => {
    expect(buildStrictOverlapPlan(review([candidate({ existingCoverage: 0.949 })]))).toBeNull();
    expect(buildStrictOverlapPlan(review([candidate({
      existing: { ...candidate().existing, pageCount: 19 },
    })]))).toBeNull();
  });

  it("automates near-equivalent editions and prefers an uncensored title marker", () => {
    const nearEquivalent = candidate({
      relation: "near_equivalent",
      existing: { ...candidate().existing, title: "Edition [Censored]", pageCount: 25 },
      existingCoverage: 1,
      incomingCoverage: 1,
      existingUniquePages: 0,
      incomingUniquePages: 0,
    });
    const incomingUncensored = review([nearEquivalent]);
    incomingUncensored.incoming = { ...incomingUncensored.incoming, title: "Edition [ＵＮＣＥＮＳＯＲＥＤ]" };
    expect(buildStrictOverlapPlan(incomingUncensored)?.winner).toBe("incoming");

    const unknownTitles = review([{ ...nearEquivalent, existing: { ...nearEquivalent.existing, title: "Edition" } }]);
    unknownTitles.incoming = { ...unknownTitles.incoming, title: "Edition" };
    expect(buildStrictOverlapPlan(unknownTitles)?.winner).toBe("existing");
  });

  it("does not remove a marked uncensored contained edition for a censored larger one", () => {
    const censoredIncoming = review([candidate({
      existing: { ...candidate().existing, title: "Edition [Uncensored]" },
    })]);
    censoredIncoming.incoming = { ...censoredIncoming.incoming, title: "Edition [Censored]" };
    expect(buildStrictOverlapPlan(censoredIncoming)).toBeNull();
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
