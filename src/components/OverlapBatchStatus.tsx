import type { EditionBatchProgress } from '../downloadOverlap/editionBatch';
import './OverlapBatchStatus.css';
export function OverlapBatchStatus({progress,onReview}:{progress:EditionBatchProgress;onReview:()=>void}) {
 const done=progress.completedKeys.length;
 return <aside className="overlap-batch-status" aria-label="판본 적용 진행 상황">
  <span role="status">{progress.failure?'판본 처리 확인 필요':'판본 적용 중'} · {done}/{progress.total}</span>
  <button type="button" className="text-button" onClick={onReview}>검토 보기</button>
  <progress max={Math.max(1,progress.total)} value={done} aria-label="판본 처리 진행률" />
 </aside>;
}
