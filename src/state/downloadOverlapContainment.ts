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

// Review objects are immutable. A changed decision/revision creates a new
// object; scrolling and queue progress reuse the already evaluated evidence.
const edgesByReview = new WeakMap<DownloadOverlapReview, DownloadOverlapContainmentItem[]>();
function containmentEdges(review: DownloadOverlapReview): DownloadOverlapContainmentItem[] {
  const cached = edgesByReview.get(review);
  if (cached) return cached;
  const edges: DownloadOverlapContainmentItem[] = [];
  if (review.state === "pending") for (const candidate of review.candidates) {
    if (candidate.decision !== undefined) continue;
    const evaluation = strictCandidateEvaluation(review, candidate);
    if (evaluation?.decisionPath !== "complete_containment") continue;
    const keeperIsIncoming = evaluation.winner === "incoming";
    const kept = keeperIsIncoming ? review.incoming : candidate.existing;
    const excluded = keeperIsIncoming ? candidate.existing : review.incoming;
    if (kept.galleryId === excluded.galleryId) continue;
    edges.push({ key: review.reviewId + ":" + candidate.candidateId, review, candidate,
      excluded, keeperIsIncoming, action: keeperIsIncoming ? "remove_existing_continue" : "remove_incoming" });
  }
  edgesByReview.set(review, edges);
  return edges;
}

/** One pass over direct, verified containment edges; no transitive inference. */
export function buildDownloadOverlapContainmentGroups(reviews: readonly DownloadOverlapReview[]): DownloadOverlapContainmentGroup[] {
  const groups = new Map<GalleryId, { keeper: DownloadOverlapGalleryRef; items: Map<string, DownloadOverlapContainmentItem> }>();
  for (const review of reviews) for (const item of containmentEdges(review)) {
    const keeper = item.keeperIsIncoming ? review.incoming : item.candidate.existing;
    let group = groups.get(keeper.galleryId);
    if (!group) { group = { keeper, items: new Map() }; groups.set(keeper.galleryId, group); }
    const known = group.items.get(item.excluded.entryId);
    if (!known || (item.keeperIsIncoming && !known.keeperIsIncoming)) group.items.set(item.excluded.entryId, item);
  }
  return [...groups.values()].map(group => ({ keeper: group.keeper, items: [...group.items.values()].sort((a,b) =>
    Number(b.keeperIsIncoming) - Number(a.keeperIsIncoming) || b.excluded.pageCount - a.excluded.pageCount
    || Number(a.excluded.galleryId) - Number(b.excluded.galleryId)) }));
}

export function buildDownloadOverlapContainmentGroup(keeperGalleryId: GalleryId, reviews: readonly DownloadOverlapReview[]): DownloadOverlapContainmentGroup | null {
  return buildDownloadOverlapContainmentGroups(reviews).find(group => group.keeper.galleryId === keeperGalleryId) ?? null;
}

/** Larger compilations with more directly contained editions come first. */
export function prioritizeDownloadOverlapReviews(reviews: readonly DownloadOverlapReview[]): DownloadOverlapReview[] {
  const contained = new Map(buildDownloadOverlapContainmentGroups(reviews).map(group =>
    [group.keeper.galleryId, new Set(group.items.map(item => item.excluded.entryId))]));
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

/** Yield between reviews when warming a large inventory; later renders reuse evidence. */
export async function prepareDownloadOverlapContainment(reviews: readonly DownloadOverlapReview[], cancelled: () => boolean = () => false) {
  let started = performance.now();
  for (const review of reviews) {
    if (cancelled()) return;
    containmentEdges(review);
    if (performance.now() - started >= 5) { await new Promise<void>(resolve => setTimeout(resolve, 0)); started = performance.now(); }
  }
}
