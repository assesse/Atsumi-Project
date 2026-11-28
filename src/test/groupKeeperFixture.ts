import type { DownloadOverlapReview } from '../api/contracts';
import { galleryId } from '../core/types';
import { pairFixture } from './overlapReviewFixture';

// Synthetic pixels/pages, with the reported three gallery identities and direct edges.
export function groupKeeperFixture() {
  const ids = [2776471, 2776792, 3053713];
  const galleries = ids.map(id => ({ entryId: 'entry-' + id, galleryId: galleryId(id), title: 'Edition ' + id, artists: ['fixture'], pageCount: 6 }));
  const reviews: DownloadOverlapReview[] = galleries.map((incoming, i) => ({
    ...pairFixture('review-' + ids[i]), incoming, entryId: incoming.entryId, incomingFingerprint: 'fp-' + ids[i],
    candidates: galleries.filter(g => g !== incoming).map((existing, rank) => ({
      ...pairFixture().candidates[0]!, candidateId: ids[i] + '-' + existing.galleryId, existing,
      existingFingerprint: 'fp-' + existing.galleryId, relation: 'translation_edition',
      existingUniquePages: 2, incomingUniquePages: 2, existingCoverage: 4 / 6, rank,
    })),
  }));
  return { ids, galleries, reviews, review: reviews[0]! };
}
