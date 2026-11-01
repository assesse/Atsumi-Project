import type { DownloadOverlapCandidate, DownloadOverlapGalleryRef, DownloadOverlapReview } from "../api/contracts";
import type { GalleryId } from "../core/types";
import { strictCandidateEvaluation } from "./downloadOverlapAuto";

export type DownloadOverlapContainmentItem = {
  key: string;
  review: DownloadOverlapReview;
  candidate: DownloadOverlapCandidate;
  action: "remove_existing_continue" | "remove_incoming";
  excluded: DownloadOverlapGalleryRef;
  keeperIsIncoming: boolean;
};

export type DownloadOverlapContainmentGroup = {
  keeper: DownloadOverlapGalleryRef;
  items: DownloadOverlapContainmentItem[];
};

/** Collect only direct, verified containment edges, including reverse reviews.
 * No existing-to-existing relationship is inferred from a common neighbour. */
export function buildDownloadOverlapContainmentGroup(
  keeperGalleryId: GalleryId,
  reviews: readonly DownloadOverlapReview[],
): DownloadOverlapContainmentGroup | null {
  const byExcludedEntry = new Map<string, DownloadOverlapContainmentItem>();
  let keeper: DownloadOverlapGalleryRef | undefined;
  for (const review of reviews) {
    if (review.state !== "pending") continue;
    for (const candidate of review.candidates) {
      if (candidate.decision !== undefined) continue;
      const evaluation = strictCandidateEvaluation(review, candidate);
      if (evaluation?.decisionPath !== "complete_containment") continue;
      const keeperIsIncoming = evaluation.winner === "incoming";
      const kept = keeperIsIncoming ? review.incoming : candidate.existing;
      const excluded = keeperIsIncoming ? candidate.existing : review.incoming;
      if (kept.galleryId !== keeperGalleryId || excluded.galleryId === keeperGalleryId) continue;
      keeper ??= kept;
      const item: DownloadOverlapContainmentItem = {
        key: `${review.reviewId}:${candidate.candidateId}`,
        review, candidate, excluded, keeperIsIncoming,
        action: keeperIsIncoming ? "remove_existing_continue" : "remove_incoming",
      };
      const known = byExcludedEntry.get(excluded.entryId);
      // Prefer a removal in the keeper's own review, so its remaining candidates
      // and revision advance together. A reverse edge is still usable on its own.
      if (!known || (keeperIsIncoming && !known.keeperIsIncoming)) byExcludedEntry.set(excluded.entryId, item);
    }
  }
  if (!keeper || !byExcludedEntry.size) return null;
  const items = [...byExcludedEntry.values()].sort((a, b) =>
    Number(b.keeperIsIncoming) - Number(a.keeperIsIncoming)
    || b.excluded.pageCount - a.excluded.pageCount
    || Number(a.excluded.galleryId) - Number(b.excluded.galleryId));
  return { keeper, items };
}

/** Larger compilations with more directly contained editions come first. */
export function prioritizeDownloadOverlapReviews(reviews: readonly DownloadOverlapReview[]): DownloadOverlapReview[] {
  const contained = new Map<GalleryId, Set<string>>();
  for (const review of reviews) {
    for (const candidate of review.candidates) {
      if (review.state !== "pending" || candidate.decision !== undefined) continue;
      const result = strictCandidateEvaluation(review, candidate);
      if (result?.decisionPath !== "complete_containment") continue;
      const keeper = result.winner === "incoming" ? review.incoming : candidate.existing;
      const excluded = result.winner === "incoming" ? candidate.existing : review.incoming;
      const ids = contained.get(keeper.galleryId) ?? new Set<string>();
      ids.add(excluded.entryId);
      contained.set(keeper.galleryId, ids);
    }
  }
  const score = (review: DownloadOverlapReview) => Math.max(
    contained.get(review.incoming.galleryId)?.size ?? 0,
    ...review.candidates.filter((candidate) => candidate.decision === undefined)
      .map((candidate) => contained.get(candidate.existing.galleryId)?.size ?? 0),
  );
  return [...reviews].sort((a, b) => score(b) - score(a)
    || (contained.get(b.incoming.galleryId)?.size ?? 0) - (contained.get(a.incoming.galleryId)?.size ?? 0)
    || a.createdAt.localeCompare(b.createdAt)
    || a.reviewId.localeCompare(b.reviewId));
}

/** Advance evidence revisions without changing the submitted batch's membership,
 * order or item keys. Processed rows remain visible; new candidates never get
 * silently added to the user's selection. The apply path revalidates every edge. */
export function refreshContainmentGroupReview(
  group: DownloadOverlapContainmentGroup | undefined,
  review: DownloadOverlapReview,
): DownloadOverlapContainmentGroup | undefined {
  if (!group) return group;
  let changed = false;
  const items = group.items.map((item) => {
    if (item.review.reviewId !== review.reviewId || item.review.entryId !== review.entryId
      || review.revision < item.review.revision) return item;
    const candidate = review.candidates.find((next) => next.candidateId === item.candidate.candidateId
      && next.existing.entryId === item.candidate.existing.entryId);
    if (!candidate) return item;
    changed = true;
    return { ...item, review, candidate, excluded: item.keeperIsIncoming ? candidate.existing : review.incoming };
  });
  if (!changed) return group;
  const keeper = review.incoming.entryId === group.keeper.entryId
    ? review.incoming
    : review.candidates.find((candidate) => candidate.existing.entryId === group.keeper.entryId)?.existing ?? group.keeper;
  return { keeper, items };
}
