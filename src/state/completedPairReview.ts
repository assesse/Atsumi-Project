import type { DownloadOverlapReview, DuplicateReview } from "../api/contracts";

export const completedPairReviewId = (candidateId: string) => `duplicate:${candidateId}`;
export const isCompletedPairReview = (id: string) => id.startsWith("duplicate:") || id.startsWith("completed-pair:");

/** A = saved parent, B = saved candidate; preserve source page coordinates. */
export function completedPairReview(review: DuplicateReview): DownloadOverlapReview {
  const pair = review.candidate;
  const ref = (value: typeof pair.parent) => ({ ...value, artists: value.artist ? [value.artist] : [] });
  const pages = review.pagePairs.map((page) => ({ ...page, incomingSourcePage: page.candidateSourcePage, existingSourcePage: page.parentSourcePage }));
  const exact = pages.filter((page) => page.exactSha256).length;
  let run = 0, longest = 0;
  pages.forEach((page, index) => {
    const previous = pages[index - 1];
    run = previous && previous.incomingSourcePage + 1 === page.incomingSourcePage && previous.existingSourcePage + 1 === page.existingSourcePage ? run + 1 : 1;
    longest = Math.max(longest, run);
  });
  const resolved = review.resolved || review.decisions.some((decision) => ["hide_parent", "hide_candidate", "exclude_pair"].includes(decision.action));
  const last = [...review.decisions].reverse().find((d) => ["hide_parent", "hide_candidate", "exclude_pair"].includes(d.action));
  return {
    reviewId: completedPairReviewId(pair.candidateId), entryId: pair.candidate.entryId,
    incoming: ref(pair.candidate), revision: pair.revision, state: review.artifactStale ? "stale" : last?.action === "hide_candidate" ? "cancelled" : resolved ? "resolved" : "pending",
    decisions: last ? [{ candidateId: pair.candidateId, action: last.action === "hide_parent" ? "remove_existing_continue" : last.action === "hide_candidate" ? "remove_incoming" : "keep_both_continue", actor: "human", createdAt: last.createdAt }] : [],
    profileVersion: 1, policyVersion: 2, incomingFingerprint: "", createdAt: pair.createdAt, updatedAt: pair.updatedAt,
    candidates: [{
      candidateId: pair.candidateId, existing: ref(pair.parent), existingFingerprint: "",
      relation: pair.relation === "exact" ? "near_equivalent" : pair.relation === "contains"
        ? pair.parent.pageCount >= pair.candidate.pageCount ? "existing_contains_incoming" : "incoming_contains_existing"
        : pair.relation === "translation_visual" ? "translation_edition" : "partial_overlap",
      confidence: pair.confidence, matchedPages: pair.matchedPages, exactPages: exact, visualPages: Math.max(0, pair.matchedPages - exact),
      existingCoverage: pair.parentCoverage, incomingCoverage: pair.candidateCoverage,
      existingUniquePages: Math.max(0, pair.parent.pageCount - pair.matchedPages), incomingUniquePages: Math.max(0, pair.candidate.pageCount - pair.matchedPages),
      longestAlignedRun: longest, rank: 1, pagePairs: pages,
      decision: last?.action === "hide_parent" ? "existing_removed" : last?.action === "exclude_pair" ? "keep_both" : undefined,
    }],
  };
}
