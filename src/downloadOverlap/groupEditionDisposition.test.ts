import { describe, expect, it, vi } from 'vitest';
import type { DownloadOverlapDecisionRequest, DownloadOverlapReview } from '../api/contracts';
import { groupKeeperFixture } from '../test/groupKeeperFixture';
import { buildOverlapReviewGroup } from './groupReview';
import { planGroupEditionBatch, type GroupEditionIntent } from './groupEditionDisposition';
import { executeEditionBatch } from './editionBatch';
import { emptyMergeSelection } from './mergeSelection';

function prepare(keep: number[] = [3053713], baseline = 2776471, reviews = groupKeeperFixture().reviews) {
  const group = buildOverlapReviewGroup(reviews.find(r => Number(r.incoming.galleryId) === baseline)!, undefined, reviews)!;
  const intents: GroupEditionIntent[] = group.editions.filter(e => e.canDecide).map(edition => ({ edition, pages: emptyMergeSelection(), modes: {
    incoming: keep.includes(Number(edition.review.incoming.galleryId)) ? 'keep' : 'remove',
    existing: keep.includes(Number(edition.candidate.existing.galleryId)) ? 'keep' : 'remove',
  } }));
  return { group, intents, reviews };
}
const removed = (op: ReturnType<typeof planGroupEditionBatch>[number]) => op.kind === 'decision' && op.request.action === 'remove_incoming' ? op.incomingEntryId : op.existingEntryId;
function nativeModel(reviews: DownloadOverlapReview[]) {
  let rows = structuredClone(reviews); const removedEntries = new Set<string>();
  const load = vi.fn(async (id: string) => ({ ok: true as const, data: structuredClone(rows.find(r => r.reviewId === id)!) }));
  const decide = vi.fn(async (request: DownloadOverlapDecisionRequest) => {
    const r = rows.find(r => r.reviewId === request.reviewId)!, c = r.candidates.find(c => c.candidateId === request.candidateId)!;
    expect(request.expectedRevision).toBe(r.revision);
    const entry = request.action === 'remove_incoming' ? r.incoming.entryId : c.existing.entryId;
    const witness = request.action === 'remove_incoming' ? c.existing.entryId : r.incoming.entryId;
    expect(removedEntries.has(witness)).toBe(false); expect(removedEntries.has(entry)).toBe(false); removedEntries.add(entry);
    // Native exclusion reconciles all reviews referencing the excluded entry.
    rows = rows.map(row => ({ ...row, revision: row.revision + 1,
      state: row.incoming.entryId === entry ? 'cancelled' : row.state,
      candidates: row.candidates.map(candidate => candidate.existing.entryId === entry ? { ...candidate, decision: 'existing_removed' } : candidate),
    }));
    return { ok: true as const, data: { review: structuredClone(rows.find(row => row.reviewId === r.reviewId)!), cancelled: request.action === 'remove_incoming', resumed: false } };
  });
  return { removedEntries, ports: { load, decide, merge: vi.fn(), onDecision: vi.fn(), onMerge: vi.fn() } };
}

describe('group edition disposition planning', () => {
  it.each([2776471, 2776792, 3053713])('can keep #%i regardless of the displayed baseline', async keep => {
    for (const baseline of [2776471, 2776792, 3053713]) {
      const f = prepare([keep], baseline), operations = planGroupEditionBatch(f.group, f.intents), model = nativeModel(f.reviews);
      expect(operations).toHaveLength(2);
      expect(operations.map(removed).sort()).toEqual(f.reviews.map(r => r.incoming.entryId).filter(id => id !== 'entry-' + keep).sort());
      const result = await executeEditionBatch(operations, model.ports);
      expect(result.failure).toBeUndefined(); expect(result.completedKeys).toHaveLength(2);
      expect(model.removedEntries.has('entry-' + keep)).toBe(false);
    }
  });
  it('rejects all removed editions and leaves pair guards intact', () => {
    const f = prepare([]); expect(() => planGroupEditionBatch(f.group, f.intents)).toThrow('남길 판본을 하나 이상');
  });
  it('excludes the baseline only once when multiple other editions remain', () => {
    const f = prepare([2776792, 3053713]); const operations = planGroupEditionBatch(f.group, f.intents);
    expect(operations).toHaveLength(1); expect(operations[0]!.galleryId).toBe(2776471);
  });
  it('does not infer a removable peer from two unrelated baseline comparisons', () => {
    const reviews = groupKeeperFixture().reviews;
    for (const r of reviews) r.candidates = r.candidates.filter(c => r.incoming.galleryId === 2776471 || c.existing.galleryId === 2776471);
    const f = prepare([3053713], 2776471, reviews);
    expect(() => planGroupEditionBatch(f.group, f.intents)).toThrow('직접 비교 근거가 없습니다');
  });
  it('rejects outdated peer fingerprints even if a direct comparison exists', () => {
    const reviews = groupKeeperFixture().reviews;
    for (const r of reviews) if (r.incoming.galleryId !== 2776471) for (const c of r.candidates) if (c.existing.galleryId !== 2776471) c.existingFingerprint = 'outdated';
    const f = prepare([3053713], 2776471, reviews);
    expect(() => planGroupEditionBatch(f.group, f.intents)).toThrow('직접 비교 근거가 없습니다');
  });
  it('uses a reverse direct relation when the survivor has no pending incoming review', async () => {
    const reviews = groupKeeperFixture().reviews.filter(r => r.incoming.galleryId !== 3053713);
    const f = prepare([3053713], 2776471, reviews), operations = planGroupEditionBatch(f.group, f.intents);
    expect(operations.every(o => o.kind === 'decision' && o.request.action === 'remove_incoming')).toBe(true);
    const model = nativeModel(f.reviews); expect((await executeEditionBatch(operations, model.ports)).failure).toBeUndefined();
  });
  it('retains the first success and retries only the unapplied exclusion', async () => {
    const f = prepare(), operations = planGroupEditionBatch(f.group, f.intents), model = nativeModel(f.reviews), original = model.ports.decide.getMockImplementation()!;
    model.ports.decide.mockImplementation(async request => { if (model.removedEntries.size) throw Error('temporary I/O failure'); return original(request); });
    const result = await executeEditionBatch(operations, model.ports);
    expect(result.completedKeys).toEqual([operations[0]!.key]); expect(result.failure).toContain('#2776471');
    model.ports.decide.mockImplementation(original);
    const retry = await executeEditionBatch(operations.filter(o => !result.completedKeys.includes(o.key)), model.ports);
    expect(retry.failure).toBeUndefined(); expect(model.removedEntries).toEqual(new Set(['entry-2776471', 'entry-2776792']));
  });
  it('preflights a changed survivor before excluding any edition', async () => {
    const f = prepare(), operations = planGroupEditionBatch(f.group, f.intents), model = nativeModel(f.reviews);
    const load = model.ports.load.getMockImplementation()!;
    model.ports.load.mockImplementation(async id => { const result = await load(id); result.data.incomingFingerprint = 'changed'; return result; });
    const result = await executeEditionBatch(operations, model.ports);
    expect(result.completedKeys).toEqual([]); expect(model.ports.decide).not.toHaveBeenCalled();
  });
  it('preserves the existing single partial merge with the baseline kept', () => {
    const f = prepare([2776471]); f.intents[0]!.modes.existing = 'partial'; f.intents[0]!.pages.existing = [1, 3];
    const operations = planGroupEditionBatch(f.group, f.intents);
    expect(operations.map(o => o.kind)).toEqual(['decision', 'merge']);
    const merge = operations[1]!; expect(merge.kind === 'merge' && merge.request.selectedPages?.existing).toEqual([1, 3]);
  });
  it('retains partial selections and refuses compound retargeting before any write', () => {
    const f = prepare(); f.intents[0]!.modes.existing = 'partial'; f.intents[0]!.pages.existing = [2];
    expect(() => planGroupEditionBatch(f.group, f.intents)).toThrow('일부 병합을 먼저');
    expect(f.intents[0]!.pages.existing).toEqual([2]);
  });
});
