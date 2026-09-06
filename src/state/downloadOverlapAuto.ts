import type {
  DownloadOverlapCandidate,
  DownloadOverlapDecisionRequest,
  DownloadOverlapGalleryRef,
  DownloadOverlapReview,
} from "../api/contracts";

export const DOWNLOAD_OVERLAP_AUTO_RULE_VERSION = 3;
export const DOWNLOAD_OVERLAP_AUTO_REASON_CODE = "balanced_overlap_v3";

export const strictOverlapThresholds = Object.freeze({
  loserCoverage: 0.95,
  maximumPageDifference: 5,
  matchedPages: 4,
  confidence: 0.9,
  alignedRunRatio: 0.75,
  informativeMatchRatio: 0.75,
});

/**
 * A separate path calibrated from accepted manual containment decisions for a
 * large omnibus. It is the only automatic rule allowed to override an
 * uncensored marker on the smaller edition. Zero unique pages on the contained
 * side and the explicit containment relation are the main safety gates; the
 * lower confidence/run thresholds account for the score penalty created by a
 * large page-count asymmetry.
 */
export const omnibusContainmentThresholds = Object.freeze({
  loserCoverage: 0.98,
  maximumContainedUniquePages: 0,
  minimumExtraPages: 8,
  minimumPageRatio: 1.5,
  matchedPages: 8,
  confidence: 0.86,
  minimumAlignedRun: 4,
  alignedRunRatio: 0.3,
  informativeMatchRatio: 0.95,
});

type StrictWinner = "incoming" | "existing";
type EditionPreference = "uncensored" | "censored" | "unknown";

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
  decisionPath: "balanced" | "omnibus_containment";
  featureSnapshotJson: string;
};

const safeRatio = (numerator: number, denominator: number): number =>
  denominator > 0 ? numerator / denominator : 0;

const editionPreference = (gallery: DownloadOverlapGalleryRef): EditionPreference => {
  const title = gallery.title.normalize("NFKC").toLowerCase();
  if ([
    "uncensored",
    "decensored",
    "uncen",
    "無修正",
    "无修正",
    "無碼",
    "无码",
  ].some((marker) => title.includes(marker))) return "uncensored";
  if (["censored", "mosaic", "モザイク", "修正版"].some((marker) => title.includes(marker))) {
    return "censored";
  }
  return "unknown";
};

const preferenceRank = (preference: EditionPreference): number =>
  preference === "uncensored" ? 1 : preference === "censored" ? -1 : 0;

const strictCandidateEvaluation = (
  review: DownloadOverlapReview,
  candidate: DownloadOverlapCandidate,
): CandidateEvaluation | null => {
  const incomingPageCount = review.incoming.pageCount;
  const existingPageCount = candidate.existing.pageCount;
  const pageDifference = Math.abs(incomingPageCount - existingPageCount);
  const smallerPageCount = Math.min(incomingPageCount, existingPageCount);
  const requiredMatchedPages = Math.min(strictOverlapThresholds.matchedPages, smallerPageCount);
  const alignedRunRatio = safeRatio(candidate.longestAlignedRun, candidate.matchedPages);
  const informativeMatches = candidate.pagePairs.filter((pair) => !pair.lowInformation).length;
  const informativeMatchRatio = safeRatio(informativeMatches, candidate.matchedPages);
  const monotonicPageOrder = candidate.pagePairs.every((pair, index, pairs) => {
    const previous = pairs[index - 1];
    return !previous
      || (pair.incomingSourcePage > previous.incomingSourcePage
        && pair.existingSourcePage > previous.existingSourcePage);
  });
  const incomingPreference = editionPreference(review.incoming);
  const existingPreference = editionPreference(candidate.existing);
  const incomingPreferenceRank = preferenceRank(incomingPreference);
  const existingPreferenceRank = preferenceRank(existingPreference);

  const containmentWinner: StrictWinner | null =
    candidate.relation === "incoming_contains_existing" && incomingPageCount > existingPageCount
      ? "incoming"
      : candidate.relation === "existing_contains_incoming" && existingPageCount > incomingPageCount
        ? "existing"
        : null;
  const containmentLoserCoverage = containmentWinner === "incoming"
    ? candidate.existingCoverage
    : containmentWinner === "existing"
      ? candidate.incomingCoverage
      : 0;
  const containedUniquePages = containmentWinner === "incoming"
    ? candidate.existingUniquePages
    : containmentWinner === "existing"
      ? candidate.incomingUniquePages
      : Math.max(candidate.existingUniquePages, candidate.incomingUniquePages);
  const containingPageCount = Math.max(incomingPageCount, existingPageCount);
  const containedPageCount = Math.min(incomingPageCount, existingPageCount);
  const containingPageRatio = safeRatio(containingPageCount, containedPageCount);
  const omnibusContainment = containmentWinner !== null
    && containmentLoserCoverage >= omnibusContainmentThresholds.loserCoverage
    && containedUniquePages <= omnibusContainmentThresholds.maximumContainedUniquePages
    && pageDifference >= omnibusContainmentThresholds.minimumExtraPages
    && containingPageRatio >= omnibusContainmentThresholds.minimumPageRatio
    && candidate.matchedPages >= omnibusContainmentThresholds.matchedPages
    && candidate.confidence >= omnibusContainmentThresholds.confidence
    && candidate.longestAlignedRun >= omnibusContainmentThresholds.minimumAlignedRun
    && alignedRunRatio >= omnibusContainmentThresholds.alignedRunRatio
    && informativeMatchRatio >= omnibusContainmentThresholds.informativeMatchRatio
    && monotonicPageOrder;

  if (!omnibusContainment && (
    pageDifference > strictOverlapThresholds.maximumPageDifference
    || candidate.matchedPages < requiredMatchedPages
    || candidate.confidence < strictOverlapThresholds.confidence
    || alignedRunRatio < strictOverlapThresholds.alignedRunRatio
    || informativeMatchRatio < strictOverlapThresholds.informativeMatchRatio
    || (smallerPageCount <= 3 && candidate.exactPages !== candidate.matchedPages)
  )) return null;

  let winner: StrictWinner;
  let loserCoverage: number;
  let preferenceReason: "containment" | "omnibus_containment" | "uncensored" | "page_count" | "stable_existing";
  let decisionPath: CandidateEvaluation["decisionPath"] = "balanced";

  if (omnibusContainment) {
    winner = containmentWinner;
    loserCoverage = containmentLoserCoverage;
    preferenceReason = "omnibus_containment";
    decisionPath = "omnibus_containment";
  } else if (candidate.relation === "incoming_contains_existing") {
    winner = "incoming";
    loserCoverage = candidate.existingCoverage;
    preferenceReason = "containment";
    // Do not automatically discard a known uncensored edition in favour of a
    // known censored one. The evidence stays pending for human review.
    if (incomingPreferenceRank < existingPreferenceRank) return null;
  } else if (candidate.relation === "existing_contains_incoming") {
    winner = "existing";
    loserCoverage = candidate.incomingCoverage;
    preferenceReason = "containment";
    if (existingPreferenceRank < incomingPreferenceRank) return null;
  } else if (candidate.relation === "near_equivalent") {
    loserCoverage = Math.min(candidate.existingCoverage, candidate.incomingCoverage);
    if (incomingPreferenceRank !== existingPreferenceRank) {
      winner = incomingPreferenceRank > existingPreferenceRank ? "incoming" : "existing";
      preferenceReason = "uncensored";
    } else if (incomingPageCount !== existingPageCount) {
      winner = incomingPageCount > existingPageCount ? "incoming" : "existing";
      preferenceReason = "page_count";
    } else {
      winner = "existing";
      preferenceReason = "stable_existing";
    }
  } else {
    return null;
  }

  if (loserCoverage < strictOverlapThresholds.loserCoverage) return null;

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
      decisionPath,
      preferenceReason,
      editionPreference: {
        incoming: incomingPreference,
        existing: existingPreference,
      },
      metrics: {
        confidence: candidate.confidence,
        matchedPages: candidate.matchedPages,
        exactPages: candidate.exactPages,
        loserCoverage,
        pageDifference,
        incomingPageCount,
        existingPageCount,
        containingPageCount,
        containedPageCount,
        containedUniquePages,
        containingPageRatio,
        alignedRunRatio,
        informativeMatchRatio,
        monotonicPageOrder,
      },
      thresholds: {
        ...strictOverlapThresholds,
        requiredMatchedPages,
        omnibusContainment: omnibusContainmentThresholds,
      },
    }),
    decisionPath,
  };
};

/**
 * Produces an all-or-nothing plan. Multi-candidate cleanup is automatic only
 * when the incoming edition wins every direct comparison. Mixed candidate
 * graphs remain pending because existing-vs-existing edges are not inferred.
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
    const omnibusCount = evaluated.filter(({ result }) => result?.decisionPath === "omnibus_containment").length;
    return {
      winner: "incoming",
      steps: evaluated.map(({ candidate, result }) => ({
        action: "remove_existing_continue",
        candidateId: candidate.candidateId,
        featureSnapshotJson: result!.featureSnapshotJson,
      })),
      summary: omnibusCount === pending.length
        ? pending.length === 1
          ? "신규 앨범 B가 기존 앨범 A를 명확히 포함하는 큰 합본으로 판정됐습니다. 작은 판본의 무검열 표식보다 합본 구성을 우선합니다."
          : `신규 앨범 B가 작은 기존 판본 ${pending.length}개를 명확히 포함하는 큰 합본으로 판정됐습니다. 작은 판본의 무검열 표식보다 합본 구성을 우선합니다.`
        : pending.length === 1
          ? "신규 앨범 B가 기존 앨범 A와 95% 이상 일치하며 더 보존할 판본으로 판정됐습니다."
          : `신규 앨범 B가 미처리 후보 ${pending.length}개와 각각 안전하게 일치하며 더 보존할 판본으로 판정됐습니다.`,
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
      summary: only.result.decisionPath === "omnibus_containment"
        ? "기존 앨범 A가 신규 앨범 B를 명확히 포함하는 큰 합본으로 판정됐습니다. 작은 판본의 무검열 표식보다 합본 구성을 우선합니다."
        : "기존 앨범 A가 신규 앨범 B와 95% 이상 일치하며 더 보존할 판본으로 판정됐습니다.",
    };
  }

  return null;
};
