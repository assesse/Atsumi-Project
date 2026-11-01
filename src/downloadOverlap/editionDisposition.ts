import type { DownloadOverlapDecisionRequest, DownloadOverlapMergeRequest } from '../api/contracts';
import type { MergeSelection, MergeSide } from './mergeSelection';
export type EditionDisposition = 'keep' | 'partial' | 'remove';
export type EditionDispositions = Record<MergeSide, EditionDisposition>;
export const keepEditions = (): EditionDispositions => ({ existing:'keep', incoming:'keep' });
export type EditionPlan = {kind:'decision';action:DownloadOverlapDecisionRequest['action']} | {kind:'merge';sourceSide:MergeSide;sourcePages:number[];selectedPages:MergeSelection;targetSelectionOnly:boolean};
/** Draft modes never mutate storage. Validation precedes the existing confirmation and revision gate. */
export function planEditionDisposition(modes:EditionDispositions,pages:MergeSelection,keeperSide:MergeSide):EditionPlan {
  for(const side of ['existing','incoming'] as const)if(modes[side]==='partial'&&!pages[side].length)throw Error('일부만 남길 페이지를 선택해 주세요.');
  if(modes.existing==='remove'&&modes.incoming==='remove')throw Error('남길 판본을 하나 이상 선택해 주세요.');
  if(modes.existing==='keep'&&modes.incoming==='keep')return {kind:'decision',action:'keep_both_continue'};
  if(modes.existing!=='partial'&&modes.incoming!=='partial')return {kind:'decision',action:modes.existing==='remove'?'remove_existing_continue':'remove_incoming'};
  const target:MergeSide = modes.existing==='keep'?'existing':modes.incoming==='keep'?'incoming':modes[keeperSide]!=='remove'?keeperSide:keeperSide==='incoming'?'existing':'incoming';
  const sourceSide:MergeSide=target==='incoming'?'existing':'incoming';
  const selectedPages:MergeSelection={existing:modes.existing==='partial'?[...pages.existing]:[],incoming:modes.incoming==='partial'?[...pages.incoming]:[]};
  return {kind:'merge',sourceSide,sourcePages:selectedPages[sourceSide],selectedPages,targetSelectionOnly:modes[target]==='partial'};
}
