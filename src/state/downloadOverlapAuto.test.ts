import { describe, expect, it } from "vitest";
import type { DownloadOverlapCandidate, DownloadOverlapReview } from "../api/contracts";
import { galleryId } from "../core/types";
import {
  buildStrictOverlapPlan,
  DOWNLOAD_OVERLAP_AUTO_REASON_CODE,
  DOWNLOAD_OVERLAP_AUTO_RULE_VERSION,
} from "./downloadOverlapAuto";

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

  it("keeps a clearly larger incoming omnibus even when contained editions are marked uncensored", () => {
    const first = candidate({
      existing: { ...candidate().existing, title: "Chapter A [Uncensored]" },
      incomingCoverage: 1 / 3,
      incomingUniquePages: 40,
    });
    const second = candidate({
      candidateId: "candidate-c",
      existing: {
        entryId: "entry-c",
        galleryId: galleryId(300),
        title: "Chapter B [Uncensored]",
        artists: ["artist"],
        pageCount: 20,
      },
      existingFingerprint: "c".repeat(64),
      incomingCoverage: 1 / 3,
      incomingUniquePages: 40,
      rank: 2,
    });
    const omnibus = review([first, second]);
    omnibus.incoming = { ...omnibus.incoming, title: "Collected Edition [Censored]", pageCount: 60 };

    const plan = buildStrictOverlapPlan(omnibus);
    expect(plan?.winner).toBe("incoming");
    expect(plan?.steps).toMatchObject([
      { action: "remove_existing_continue", candidateId: "candidate-a" },
      { action: "remove_existing_continue", candidateId: "candidate-c" },
    ]);
    expect(plan?.summary).toContain("큰 합본");

    const snapshot = JSON.parse(plan!.steps[0]!.featureSnapshotJson) as {
      rule: string;
      ruleVersion: number;
      winner: string;
      decisionPath: string;
      preferenceReason: string;
      editionPreference: { incoming: string; existing: string };
      metrics: {
        containingPageCount: number;
        containedPageCount: number;
        containedUniquePages: number;
        containingPageRatio: number;
      };
    };
    expect(snapshot).toMatchObject({
      rule: DOWNLOAD_OVERLAP_AUTO_REASON_CODE,
      ruleVersion: DOWNLOAD_OVERLAP_AUTO_RULE_VERSION,
      winner: "incoming",
      decisionPath: "omnibus_containment",
      preferenceReason: "omnibus_containment",
      editionPreference: { incoming: "censored", existing: "uncensored" },
      metrics: {
        containingPageCount: 60,
        containedPageCount: 20,
        containedUniquePages: 0,
        containingPageRatio: 3,
      },
    });
  });

  it("removes a small incoming uncensored edition when one existing omnibus clearly contains it", () => {
    const pagePairs = candidate().pagePairs.slice(0, 12);
    const containingExisting = candidate({
      relation: "existing_contains_incoming",
      existing: {
        ...candidate().existing,
        title: "Collected Edition [Censored]",
        pageCount: 20,
      },
      confidence: 0.9454,
      matchedPages: 12,
      exactPages: 0,
      visualPages: 12,
      existingCoverage: 0.6,
      incomingCoverage: 1,
      existingUniquePages: 8,
      incomingUniquePages: 0,
      longestAlignedRun: 4,
      pagePairs: pagePairs.map((pair) => ({ ...pair, exactSha256: false })),
    });
    const smallUncensored = review([containingExisting]);
    smallUncensored.incoming = {
      ...smallUncensored.incoming,
      title: "Chapter A [Uncensored]",
      pageCount: 12,
    };

    const plan = buildStrictOverlapPlan(smallUncensored);
    expect(plan?.winner).toBe("existing");
    expect(plan?.steps).toMatchObject([{ action: "remove_incoming", candidateId: "candidate-a" }]);
    expect(plan?.summary).toContain("큰 합본");
  });

  it("keeps uncertain size or containment evidence on the manual-review path", () => {
    const omnibusReview = (overrides: Partial<DownloadOverlapCandidate>) => {
      const value = review([candidate({
        existing: { ...candidate().existing, title: "Chapter [Uncensored]" },
        incomingCoverage: 1 / 3,
        incomingUniquePages: 40,
        ...overrides,
      })]);
      value.incoming = { ...value.incoming, title: "Collected [Censored]", pageCount: 60 };
      return value;
    };

    expect(buildStrictOverlapPlan(omnibusReview({ existingCoverage: 0.979 }))).toBeNull();
    expect(buildStrictOverlapPlan(omnibusReview({ confidence: 0.859 }))).toBeNull();
    expect(buildStrictOverlapPlan(omnibusReview({ existingUniquePages: 1 }))).toBeNull();
    expect(buildStrictOverlapPlan(omnibusReview({
      pagePairs: candidate().pagePairs.map((pair, index, pairs) => index === pairs.length - 1
        ? { ...pair, incomingSourcePage: pairs[index - 1]!.incomingSourcePage }
        : pair),
    }))).toBeNull();
    expect(buildStrictOverlapPlan(omnibusReview({
      existing: { ...candidate().existing, title: "Chapter [Uncensored]", pageCount: 45 },
      matchedPages: 20,
      exactPages: 20,
      longestAlignedRun: 20,
    }))).toBeNull();
  });

  it("accepts the observed clear-containment floor from prior manual classifications", () => {
    const pagePairs = Array.from({ length: 12 }, (_, index) => ({
      incomingSourcePage: index + 1,
      existingSourcePage: index + 1,
      exactSha256: index < 4,
      dHashDistance: index < 4 ? 0 : 2,
      pHashDistance: index < 4 ? 0 : 3,
      detailHashDistance: index < 4 ? 0 : 3,
      edgeSimilarity: index < 4 ? 1 : 0.98,
      visualSimilarity: index < 4 ? 1 : 0.98,
      lowInformation: false,
    }));
    const observed = review([candidate({
      existing: {
        ...candidate().existing,
        title: "Chapter [Decensored]",
        pageCount: 12,
      },
      confidence: 0.8646,
      matchedPages: 12,
      exactPages: 4,
      visualPages: 8,
      existingCoverage: 1,
      incomingCoverage: 0.6,
      existingUniquePages: 0,
      incomingUniquePages: 8,
      longestAlignedRun: 4,
      pagePairs,
    })]);
    observed.incoming = {
      ...observed.incoming,
      title: "Collected Edition [Censored]",
      pageCount: 20,
    };

    const plan = buildStrictOverlapPlan(observed);
    expect(plan?.winner).toBe("incoming");
    expect(plan?.steps[0]).toMatchObject({ action: "remove_existing_continue" });
    expect(JSON.parse(plan!.steps[0]!.featureSnapshotJson)).toMatchObject({
      decisionPath: "omnibus_containment",
      metrics: {
        confidence: 0.8646,
        pageDifference: 8,
        containedUniquePages: 0,
        alignedRunRatio: 4 / 12,
        informativeMatchRatio: 1,
        monotonicPageOrder: true,
      },
    });
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

  it("leaves a mixed review manual when a same-size uncensored candidate beats the omnibus", () => {
    const contained = candidate({
      existing: { ...candidate().existing, title: "Chapter", pageCount: 20 },
      incomingCoverage: 1 / 3,
      incomingUniquePages: 40,
    });
    const competingEdition = candidate({
      candidateId: "candidate-c",
      relation: "near_equivalent",
      existing: {
        ...candidate().existing,
        entryId: "entry-c",
        galleryId: galleryId(300),
        title: "Collected Edition [Uncensored]",
        pageCount: 60,
      },
      existingFingerprint: "c".repeat(64),
      existingCoverage: 1,
      incomingCoverage: 1,
      existingUniquePages: 0,
      incomingUniquePages: 0,
      rank: 2,
    });
    const mixed = review([contained, competingEdition]);
    mixed.incoming = { ...mixed.incoming, title: "Collected Edition [Censored]", pageCount: 60 };

    expect(buildStrictOverlapPlan(mixed)).toBeNull();
  });
});
