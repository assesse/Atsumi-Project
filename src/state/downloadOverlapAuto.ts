import type {
  DownloadOverlapCandidate,
  DownloadOverlapDecisionRequest,
  DownloadOverlapReview,
} from "../api/contracts";

export const DOWNLOAD_OVERLAP_AUTO_RULE_VERSION = 1;
export const DOWNLOAD_OVERLAP_AUTO_REASON_CODE = "strict_extra_pages_v1";

export const strictOverlapThresholds = Object.freeze({
  loserCoverage: 0.995,
  matchedPages: 10,
  winnerUniquePages: 10,
  winnerUniqueRatio: 0.1,
  alignedRunRatio: 0.8,
  informativeMatchRatio: 0.9,
  exactMatchRatio: 0.9,
});

type StrictWinner = "incoming" | "existing";

export type StrictOverlapDecisionStep = {
  action: Extract<
    DownloadOverlapDecisionRequest["action"],
    "remove_existing_continue" | "remove_incoming"
  >;
  candidateId: string;
  featureSnapshotJson: string;
};

export type StrictOverlapPlan = {
  winner: StrictWinner;
  steps: StrictOverlapDecisionStep[];
  summary: string;
};

type CandidateEvaluation = {
  winner: StrictWinner;
  featureSnapshotJson: string;
};

const safeRatio = (numerator: number, denominator: number): number =>
  denominator > 0 ? numerator / denominator : 0;

const strictCandidateEvaluation = (
  review: DownloadOverlapReview,
  candidate: DownloadOverlapCandidate,
): CandidateEvaluation | null => {
  let winner: StrictWinner;
  let loserCoverage: number;
  let loserUniquePages: number;
  let winnerUniquePages: number;
  let loserPageCount: number;
  let winnerPageCount: number;

  if (candidate.relation === "incoming_contains_existing") {
    winner = "incoming";
    loserCoverage = candidate.existingCoverage;
    loserUniquePages = candidate.existingUniquePages;
    winnerUniquePages = candidate.incomingUniquePages;
    loserPageCount = candidate.existing.pageCount;
    winnerPageCount = review.incoming.pageCount;
  } else if (candidate.relation === "existing_contains_incoming") {
    winner = "existing";
    loserCoverage = candidate.incomingCoverage;
    loserUniquePages = candidate.incomingUniquePages;
    winnerUniquePages = candidate.existingUniquePages;
    loserPageCount = review.incoming.pageCount;
    winnerPageCount = candidate.existing.pageCount;
  } else {
    return null;
  }

  const alignedRunRatio = safeRatio(candidate.longestAlignedRun, candidate.matchedPages);
  const informativeMatches = candidate.pagePairs.filter((pair) => !pair.lowInformation).length;
  const informativeMatchRatio = safeRatio(informativeMatches, candidate.matchedPages);
  const exactMatchRatio = safeRatio(candidate.exactPages, candidate.matchedPages);
  const requiredWinnerUniquePages = Math.max(
    strictOverlapThresholds.winnerUniquePages,
    Math.ceil(loserPageCount * strictOverlapThresholds.winnerUniqueRatio),
  );

  if (
    loserCoverage < strictOverlapThresholds.loserCoverage
    || loserUniquePages !== 0
    || candidate.matchedPages < strictOverlapThresholds.matchedPages
    || winnerUniquePages < requiredWinnerUniquePages
    || winnerPageCount <= loserPageCount
    || alignedRunRatio < strictOverlapThresholds.alignedRunRatio
    || informativeMatchRatio < strictOverlapThresholds.informativeMatchRatio
    || exactMatchRatio < strictOverlapThresholds.exactMatchRatio
  ) {
    return null;
  }

  return {
    winner,
    featureSnapshotJson: JSON.stringify({
      rule: DOWNLOAD_OVERLAP_AUTO_REASON_CODE,
      ruleVersion: DOWNLOAD_OVERLAP_AUTO_RULE_VERSION,
      reviewId: review.reviewId,
      reviewRevision: review.revision,
      candidateId: candidate.candidateId,
      incomingGalleryId: review.incoming.galleryId,
      existingGalleryId: candidate.existing.galleryId,
      incomingFingerprint: review.incomingFingerprint,
      existingFingerprint: candidate.existingFingerprint,
      relation: candidate.relation,
      winner,
      metrics: {
        confidence: candidate.confidence,
        matchedPages: candidate.matchedPages,
        exactPages: candidate.exactPages,
        loserCoverage,
        loserUniquePages,
        winnerUniquePages,
        loserPageCount,
        winnerPageCount,
        alignedRunRatio,
        informativeMatchRatio,
        exactMatchRatio,
      },
      thresholds: {
        ...strictOverlapThresholds,
        requiredWinnerUniquePages,
      },
    }),
  };
};

/**
 * Produces an all-or-nothing plan. A multi-candidate review is automatic only
 * when the incoming edition strictly dominates every unresolved existing
 * candidate. Existing-vs-existing comparisons are not inferred transitively.
 */
export const buildStrictOverlapPlan = (
  review: DownloadOverlapReview,
): StrictOverlapPlan | null => {
  if (review.state !== "pending") return null;
  const pending = review.candidates
    .filter((candidate) => candidate.decision === undefined)
    .sort((left, right) => left.rank - right.rank);
  if (pending.length === 0) return null;

  const evaluated = pending.map((candidate) => ({
    candidate,
    result: strictCandidateEvaluation(review, candidate),
  }));
  if (evaluated.some(({ result }) => result === null)) return null;

  const incomingWinsAll = evaluated.every(({ result }) => result?.winner === "incoming");
  if (incomingWinsAll) {
    return {
      winner: "incoming",
      steps: evaluated.map(({ candidate, result }) => ({
        action: "remove_existing_continue",
        candidateId: candidate.candidateId,
        featureSnapshotJson: result!.featureSnapshotJson,
      })),
      summary: pending.length === 1
        ? "신규 앨범 B가 기존 앨범 A의 실질 페이지를 모두 포함하고 추가 페이지가 충분합니다."
        : `신규 앨범 B가 미처리 후보 ${pending.length}개를 각각 엄격한 기준으로 포함합니다.`,
    };
  }

  const only = evaluated.length === 1 ? evaluated[0] : undefined;
  if (only?.result?.winner === "existing") {
    return {
      winner: "existing",
      steps: [{
        action: "remove_incoming",
        candidateId: only.candidate.candidateId,
        featureSnapshotJson: only.result.featureSnapshotJson,
      }],
      summary: "기존 앨범 A가 신규 앨범 B의 실질 페이지를 모두 포함하고 추가 페이지가 충분합니다.",
    };
  }

  return null;
};
