import type { ApiResult, DownloadOverlapDecisionRequest, DownloadOverlapDecisionResult, DownloadOverlapMergeRequest, DownloadOverlapMergeResult, DownloadOverlapReview } from '../api/contracts';
import { overlapEvidenceKey } from './groupReview';

export type EditionBatchOperation = {
  key: string; evidence: string; galleryId: number;
  incomingEntryId: string; existingEntryId: string;
} & ({ kind: 'decision'; request: DownloadOverlapDecisionRequest } | { kind: 'merge'; request: DownloadOverlapMergeRequest });
export type EditionBatchProgress = { completedKeys: string[]; total: number; failedKey?: string; failure?: string };
export type EditionBatchHandler = (operations: EditionBatchOperation[], onProgress: (progress: EditionBatchProgress) => void) => Promise<EditionBatchProgress>;

type Ports = {
  load: (id: string) => Promise<ApiResult<DownloadOverlapReview>>;
  decide: (request: DownloadOverlapDecisionRequest) => Promise<ApiResult<DownloadOverlapDecisionResult>>;
  merge: (request: DownloadOverlapMergeRequest) => Promise<ApiResult<DownloadOverlapMergeResult>>;
  onDecision: (operation: EditionBatchOperation, result: DownloadOverlapDecisionResult) => void;
  onMerge: (operation: EditionBatchOperation, result: DownloadOverlapMergeResult) => void;
  onReview?: (review: DownloadOverlapReview) => void;
  onProgress?: (progress: EditionBatchProgress) => void;
};

// Freeze and validate the whole intent before any mutation. A merge rewrites page
// numbers/fingerprints; never rebase another selected merge onto those new bytes.
export function orderEditionBatch(operations: readonly EditionBatchOperation[]): EditionBatchOperation[] {
  if (!operations.length) throw Error('적용할 판본이 없습니다.');
  if (new Set(operations.map(item => item.key)).size !== operations.length) throw Error('중복된 판본 선택입니다.');
  const merges = operations.filter(item => item.kind === 'merge');
  if (merges.length > 1) throw Error('여러 판본의 일부 병합은 한 판본씩 적용해 주세요. 선택은 유지됩니다.');
  const removed = new Set<string>();
  for (const item of operations) {
    if (item.kind === 'merge') removed.add(item.request.sourceSide === 'incoming' ? item.incomingEntryId : item.existingEntryId);
    else if (item.request.action === 'remove_incoming') removed.add(item.incomingEntryId);
    else if (item.request.action === 'remove_existing_continue') removed.add(item.existingEntryId);
  }
  for (const item of operations) {
    const target = item.kind === 'merge' ? (item.request.sourceSide === 'incoming' ? item.existingEntryId : item.incomingEntryId)
      : item.request.action === 'remove_incoming' ? item.existingEntryId : item.request.action === 'remove_existing_continue' ? item.incomingEntryId : undefined;
    if (target && removed.has(target)) throw Error('다른 판본의 보존 대상도 제거로 선택되어 있습니다. 남길 판본을 확인해 주세요.');
  }
  return [...operations].sort((a,b) => Number(a.kind === 'merge') - Number(b.kind === 'merge'));
}

function validateFresh(item: EditionBatchOperation, review: DownloadOverlapReview) {
  const candidate = review.candidates.find(candidate => candidate.candidateId === item.request.candidateId);
  if (!candidate || review.state !== 'pending' || candidate.decision !== undefined || overlapEvidenceKey(review,candidate) !== item.evidence)
    throw Error('판본 또는 페이지 근거가 변경되었습니다. 최신 내용을 다시 확인해 주세요.');
}

/** Every native action remains revision/fingerprint checked and independently durable.
 * Stop on the first uncertain result; keep unprocessed drafts and report exact successes. */
export async function executeEditionBatch(operations: readonly EditionBatchOperation[], ports: Ports): Promise<EditionBatchProgress> {
  const progress: EditionBatchProgress = { completedKeys: [], total: operations.length };
  let current: EditionBatchOperation | undefined;
  try {
    const ordered = orderEditionBatch(operations);
    // Catch known stale or invalid members before even the first exclusion.
    for (const item of ordered) {
      current = item;
      const result = await ports.load(item.request.reviewId);
      if (!result.ok) throw Error(result.error.message);
      ports.onReview?.(result.data); validateFresh(item,result.data);
    }
    for (const item of ordered) {
      current = item;
      const loaded = await ports.load(item.request.reviewId);
      if (!loaded.ok) throw Error(loaded.error.message);
      ports.onReview?.(loaded.data); validateFresh(item,loaded.data);
      if (item.kind === 'decision') {
        const result = await ports.decide({...item.request,expectedRevision:loaded.data.revision,actor:'human'});
        if (!result.ok) throw Error(result.error.message);
        const candidate = result.data.review.candidates.find(candidate => candidate.candidateId === item.request.candidateId);
        const confirmed = item.request.action === 'remove_incoming' ? result.data.cancelled && result.data.review.state === 'cancelled'
          : candidate?.decision === (item.request.action === 'remove_existing_continue' ? 'existing_removed' : item.request.action === 'false_positive_continue' ? 'false_positive' : 'keep_both');
        if (!confirmed) throw Error('처리 결과가 확인되지 않았습니다. 현재 상태를 다시 확인해 주세요.');
        progress.completedKeys.push(item.key);
        ports.onDecision(item,result.data);
      } else {
        const result = await ports.merge({...item.request,expectedRevision:loaded.data.revision,excludeSource:true});
        if (!result.ok) throw Error(result.error.message);
        if (!result.data.sourceExcluded) throw Error('병합 후 원본 판본 제외 결과를 다시 확인해 주세요.');
        progress.completedKeys.push(item.key);
        ports.onMerge(item,result.data);
      }
      ports.onProgress?.({...progress,completedKeys:[...progress.completedKeys]});
      await new Promise<void>(resolve => setTimeout(resolve,0));
    }
  } catch (error) {
    progress.failedKey = current?.key;
    progress.failure = (current ? '#'+current.galleryId+' · ' : '') + (error instanceof Error ? error.message : '처리 결과를 확인하지 못했습니다.');
  }
  ports.onProgress?.({...progress,completedKeys:[...progress.completedKeys]});
  return progress;
}
