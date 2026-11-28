import type { DownloadOverlapGalleryRef, DownloadOverlapPagePair, DownloadOverlapReview } from '../api/contracts';
import { planEditionDisposition, type EditionDispositions } from './editionDisposition';
import { orderEditionBatch, type EditionBatchOperation } from './editionBatch';
import { overlapEvidenceKey, type GroupDirectRelation, type GroupEdition, type OverlapReviewGroup } from './groupReview';
import { emptyMergeSelection, mergeOutputPages, type MergeSelection, type MergeSide } from './mergeSelection';

export type GroupEditionIntent = { edition: GroupEdition; modes: EditionDispositions; pages: MergeSelection; displayPairs?: DownloadOverlapPagePair[] };
const keeperSide = (edition: GroupEdition): MergeSide => edition.keeperIsIncoming ? 'incoming' : 'existing';
const peerSide = (edition: GroupEdition): MergeSide => edition.keeperIsIncoming ? 'existing' : 'incoming';

function operation(relation: GroupDirectRelation, modes: EditionDispositions, pages: MergeSelection, target: MergeSide, key: string): EditionBatchOperation {
  const { review, candidate } = relation;
  const plan = planEditionDisposition(modes, pages, target);
  const removedSide = plan.kind === 'merge' ? plan.sourceSide : plan.action === 'remove_incoming' ? 'incoming'
    : plan.action === 'remove_existing_continue' ? 'existing' : target === 'incoming' ? 'existing' : 'incoming';
  const galleryId = Number(removedSide === 'incoming' ? review.incoming.galleryId : candidate.existing.galleryId);
  const common = { key, evidence: overlapEvidenceKey(review, candidate), galleryId, incomingEntryId: review.incoming.entryId, existingEntryId: candidate.existing.entryId };
  const identity = { reviewId: review.reviewId, expectedRevision: review.revision, candidateId: candidate.candidateId };
  if (plan.kind === 'decision') return { ...common, kind: 'decision', request: { ...identity, action: plan.action } };
  const { kind: _, ...composition } = plan;
  const request = { ...identity, ...composition, excludeSource: true };
  if (!mergeOutputPages(candidate, review.incoming.pageCount, request).length) throw Error('남길 페이지를 하나 이상 선택해 주세요.');
  return { ...common, kind: 'merge', request };
}

/** The displayed baseline is an alignment reference, not a mandatory survivor.
 * Every exclusion still needs its own current direct relation to a final survivor. */
export function planGroupEditionBatch(group: OverlapReviewGroup, intents: readonly GroupEditionIntent[]): EditionBatchOperation[] {
  if (!intents.length) throw Error('적용할 판본이 없습니다.');
  const baselineMode = intents[0]!.modes[keeperSide(intents[0]!.edition)];
  for (const { edition, modes, pages, displayPairs = [] } of intents) {
    if (!edition.canDecide || !edition.keeperCompatible || edition.evidence !== overlapEvidenceKey(edition.review, edition.candidate))
      throw Error('#' + edition.excluded.galleryId + ' · 판본 또는 페이지 근거가 변경되었습니다.');
    if (modes[keeperSide(edition)] !== baselineMode) throw Error('기준 판본의 선택을 다시 확인해 주세요.');
    for (const side of ['incoming', 'existing'] as const) if (modes[side] === 'partial' && !pages[side].length)
      throw Error('#' + (side === keeperSide(edition) ? group.keeper.galleryId : edition.excluded.galleryId) + ' · 일부만 남길 페이지를 선택해 주세요.');
    const originalPairs = new Set(edition.candidate.pagePairs.map(p => p.incomingSourcePage + ':' + p.existingSourcePage));
    if (displayPairs.some(p => !originalPairs.has(p.incomingSourcePage + ':' + p.existingSourcePage)
      && ((modes.incoming === 'partial' && pages.incoming.includes(p.incomingSourcePage)) || (modes.existing === 'partial' && pages.existing.includes(p.existingSourcePage)))))
      throw Error('순서가 다른 선택 페이지는 병합할 수 없습니다. 선택은 유지됩니다.');
  }
  if (baselineMode !== 'remove' || intents.length === 1) return orderEditionBatch(intents.map(({ edition, modes, pages }) =>
    operation(edition, modes, pages, keeperSide(edition), edition.key)));

  const retained = intents.filter(i => i.modes[peerSide(i.edition)] === 'keep');
  const partial = intents.filter(i => i.modes[peerSide(i.edition)] === 'partial');
  if (!retained.length && !partial.length) throw Error('남길 판본을 하나 이상 선택해 주세요.');
  // Retargeting a partial merge can invalidate other selected relations and retry
  // identities. Keep it separate from full-edition exclusions, before any writes.
  if (partial.length) throw Error('기준 판본도 제거할 때는 일부 병합을 먼저 한 판본씩 적용해 주세요. 선택은 유지됩니다.');
  const targets = retained;

  const known = new Map<string, { gallery: DownloadOverlapGalleryRef; fingerprint: string }>();
  for (const item of group.editions.filter(e => e.keeperCompatible)) {
    known.set(group.keeper.entryId, { gallery: group.keeper, fingerprint: item.keeperIsIncoming ? item.review.incomingFingerprint : item.candidate.existingFingerprint });
    known.set(item.excluded.entryId, { gallery: item.excluded, fingerprint: item.keeperIsIncoming ? item.candidate.existingFingerprint : item.review.incomingFingerprint });
  }
  const reviews = new Map<string, DownloadOverlapReview>();
  for (const review of [...(group.reviews ?? []), ...group.relations.map(r => r.review), ...group.editions.map(e => e.review)])
    if ((reviews.get(review.reviewId)?.revision ?? -1) <= review.revision) reviews.set(review.reviewId, review);
  const direct: GroupDirectRelation[] = [];
  for (const review of reviews.values()) for (const candidate of review.candidates) {
    const incoming = known.get(review.incoming.entryId), existing = known.get(candidate.existing.entryId);
    if (review.state !== 'pending' || candidate.decision !== undefined || !incoming || !existing
      || incoming.fingerprint !== review.incomingFingerprint || existing.fingerprint !== candidate.existingFingerprint
      || incoming.gallery.pageCount !== review.incoming.pageCount || existing.gallery.pageCount !== candidate.existing.pageCount
      || incoming.gallery.galleryId !== review.incoming.galleryId || existing.gallery.galleryId !== candidate.existing.galleryId) continue;
    direct.push({ review, candidate, key: review.reviewId + ':' + candidate.candidateId, evidence: overlapEvidenceKey(review, candidate) });
  }
  const link = (source: DownloadOverlapGalleryRef) => {
    for (const target of targets) {
      const targetId = target.edition.excluded.entryId;
      const relation = direct.filter(r => (r.review.incoming.entryId === targetId && r.candidate.existing.entryId === source.entryId)
        || (r.candidate.existing.entryId === targetId && r.review.incoming.entryId === source.entryId))
        // Keep the survivor's review open across the entire batch where available.
        .sort((a, b) => Number(b.review.incoming.entryId === targetId) - Number(a.review.incoming.entryId === targetId) || a.key.localeCompare(b.key))[0];
      if (relation) return { target, relation };
    }
    throw Error('#' + source.galleryId + '와 남길 판본의 직접 비교 근거가 없습니다. 다시 대조해 주세요.');
  };
  const make = (source: DownloadOverlapGalleryRef, sourceIntent?: GroupEditionIntent): EditionBatchOperation => {
    const { target, relation } = link(source);
    const targetSide: MergeSide = relation.review.incoming.entryId === target.edition.excluded.entryId ? 'incoming' : 'existing';
    const sourceSide: MergeSide = targetSide === 'incoming' ? 'existing' : 'incoming';
    const modes: EditionDispositions = { incoming: 'keep', existing: 'keep' };
    modes[sourceSide] = 'remove';
    const pages = emptyMergeSelection();
    // The survivor's original baseline pair owns the single baseline exclusion.
    return operation(relation, modes, pages, targetSide, sourceIntent?.edition.key ?? target.edition.key);
  };
  const operations = intents.filter(i => i.modes[peerSide(i.edition)] !== 'keep' && !targets.includes(i)).map(i => make(i.edition.excluded, i));
  operations.push(make(group.keeper));
  return orderEditionBatch(operations);
}
