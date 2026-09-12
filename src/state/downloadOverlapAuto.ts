import type {
  DownloadOverlapCandidate,
  DownloadOverlapDecisionRequest,
  DownloadOverlapGalleryRef,
  DownloadOverlapReview,
} from "../api/contracts";

export const DOWNLOAD_OVERLAP_AUTO_RULE_VERSION = 5;
export const DOWNLOAD_OVERLAP_AUTO_REASON_CODE = "balanced_overlap_v5";

export const strictOverlapThresholds = Object.freeze({
  loserCoverage: 0.95,
  maximumPageDifference: 5,
  matchedPages: 4,
  confidence: 0.9,
  alignedRunRatio: 0.75,
  informativeMatchRatio: 0.75,
});

/**
 * Complete directional containment is stronger evidence than an edition-title
 * marker. The contained side must have every page matched, no unique page, and
 * a monotonic mapping. This path intentionally scales the minimum match/run
 * counts for short works and lowers only the confidence floor that is penalized
 * by large page-count asymmetry.
 */
export const completeContainmentThresholds = Object.freeze({
  loserCoverage: 1,
  maximumContainedUniquePages: 0,
  matchedPages: 4,
  confidence: 0.85,
  minimumAlignedRun: 2,
  informativeMatchRatio: 0.75,
});

export const DOWNLOAD_OVERLAP_AUTO_HELP = [
  "완전 포함: 작은 판본의 모든 페이지 대응 · 고유 페이지 0 · 순서 일치 · 신뢰도 85% 이상.",
  "일반 판본: 포함률 95% 이상 · 페이지 차이 5장 이하 · 신뢰도 90% 이상.",
  "정보성·연속 일치 기준도 충족해야 합니다. 3장 이하 작품은 정확한 SHA-256 일치가 필요합니다.",
  "포함 판본을 우선합니다. 거의 동일하면 제목 표식(무검열 > 미표시 > 검열) → 페이지 수 → 기존본 순으로 보존합니다.",
  "자동 모드는 횟수 제한 없이 재검증 후 적용합니다. 완료본은 격리, 신규·대기본은 취소·제외 처리합니다. 영구 삭제하지 않습니다.",
].join("\n");

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
  pendingCandidateCount: number;
  remainingCandidateCount: number;
  isPartial: boolean;
};

export type CandidateEvaluation = {
  winner: StrictWinner;
  decisionPath: "balanced" | "complete_containment";
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

export const strictCandidateEvaluation = (
  review: DownloadOverlapReview,
  candidate: DownloadOverlapCandidate,
): CandidateEvaluation | null => {
  if (candidate.existing.galleryId === review.incoming.galleryId
    || candidate.existing.entryId === review.incoming.entryId) return null;
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
  const requiredCompleteMatches = Math.min(
    completeContainmentThresholds.matchedPages,
    containedPageCount,
  );
  const requiredCompleteAlignedRun = Math.min(
    completeContainmentThresholds.minimumAlignedRun,
    candidate.matchedPages,
  );
  const completeContainment = containmentWinner !== null
    && containmentLoserCoverage >= completeContainmentThresholds.loserCoverage
    && containedUniquePages <= completeContainmentThresholds.maximumContainedUniquePages
    && candidate.matchedPages === containedPageCount
    && candidate.pagePairs.length === candidate.matchedPages
    && candidate.matchedPages >= requiredCompleteMatches
    && candidate.confidence >= completeContainmentThresholds.confidence
    && candidate.longestAlignedRun >= requiredCompleteAlignedRun
    && informativeMatchRatio >= completeContainmentThresholds.informativeMatchRatio
    && (containedPageCount > 3 || candidate.exactPages === candidate.matchedPages)
    && monotonicPageOrder;

  if (!completeContainment && (
    pageDifference > strictOverlapThresholds.maximumPageDifference
    || candidate.matchedPages < requiredMatchedPages
    || candidate.confidence < strictOverlapThresholds.confidence
    || alignedRunRatio < strictOverlapThresholds.alignedRunRatio
    || informativeMatchRatio < strictOverlapThresholds.informativeMatchRatio
    || (smallerPageCount <= 3 && candidate.exactPages !== candidate.matchedPages)
  )) return null;

  let winner: StrictWinner;
  let loserCoverage: number;
  let preferenceReason: "containment" | "complete_containment" | "uncensored" | "page_count" | "stable_existing";
  let decisionPath: CandidateEvaluation["decisionPath"] = "balanced";

  if (completeContainment) {
    winner = containmentWinner;
    loserCoverage = containmentLoserCoverage;
    preferenceReason = "complete_containment";
    decisionPath = "complete_containment";
  } else if (candidate.relation === "incoming_contains_existing") {
    winner = "incoming";
    loserCoverage = candidate.existingCoverage;
    preferenceReason = "containment";
  } else if (candidate.relation === "existing_contains_incoming") {
    winner = "existing";
    loserCoverage = candidate.incomingCoverage;
    preferenceReason = "containment";
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
        completeContainment: {
          ...completeContainmentThresholds,
          requiredMatchedPages: requiredCompleteMatches,
          requiredAlignedRun: requiredCompleteAlignedRun,
        },
      },
    }),
    decisionPath,
  };
};

/**
 * Each step is justified by one direct comparison. Prefer a surviving existing
 * witness before removing any candidates; otherwise remove only candidates the
 * incoming edition safely replaces and leave uncertain comparisons pending.
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
  const witness = evaluated.find(({ result }) => result?.winner === "existing");
  if (witness?.result) {
    return {
      winner: "existing",
      steps: [{
        action: "remove_incoming",
        candidateId: witness.candidate.candidateId,
        featureSnapshotJson: witness.result.featureSnapshotJson,
      }],
      pendingCandidateCount: pending.length,
      remainingCandidateCount: 0,
      isPartial: false,
      summary: witness.result.decisionPath === "complete_containment"
        ? `기존 #${witness.candidate.existing.galleryId} 유지 · 신규 B 제외 (완전 포함)`
        : `기존 #${witness.candidate.existing.galleryId} 유지 · 신규 B 제외`,
    };
  }

  const incomingWins = evaluated.filter(({ result }) => result?.winner === "incoming");
  if (incomingWins.length) {
    const completeContainmentCount = incomingWins.filter(
      ({ result }) => result?.decisionPath === "complete_containment",
    ).length;
    return {
      winner: "incoming",
      steps: incomingWins.map(({ candidate, result }) => ({
        action: "remove_existing_continue",
        candidateId: candidate.candidateId,
        featureSnapshotJson: result!.featureSnapshotJson,
      })),
      pendingCandidateCount: pending.length,
      remainingCandidateCount: pending.length - incomingWins.length,
      isPartial: incomingWins.length < pending.length,
      summary: incomingWins.length < pending.length
        ? `신규 B 유지 · 기존 ${incomingWins.length}개 제외 · ${pending.length - incomingWins.length}개 직접 검토`
        : `신규 B 유지 · ${pending.length === 1 ? `기존 #${incomingWins[0]!.candidate.existing.galleryId}` : `기존 ${pending.length}개`} 제외${completeContainmentCount === pending.length ? " (완전 포함)" : ""}`,
    };
  }

  return null;
};
