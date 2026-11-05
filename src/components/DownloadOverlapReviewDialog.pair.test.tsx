import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { pairFixture } from '../test/overlapReviewFixture';
import { ThumbnailClient } from '../thumbnail';
import { DownloadOverlapReviewDialog } from './DownloadOverlapReviewDialog';

const cleanups:(()=>Promise<void>)[]=[];
afterEach(async()=>{for(const cleanup of cleanups.splice(0))await cleanup();vi.restoreAllMocks();});
async function mount(review=pairFixture()) {
  const client=new ThumbnailClient({resolve:()=>({kind:'missing',reason:'fixture'})});
  const container=document.createElement('div');document.body.append(container);const root=createRoot(container);
  const onDecision=vi.fn(),onMerge=vi.fn();
  const render=async(value=review,busy=false)=>act(async()=>root.render(<DownloadOverlapReviewDialog open review={value} decisionPending={busy} previewWidth={220} thumbnailClient={client} onClose={vi.fn()} onRetry={vi.fn()} onDecision={onDecision} onMergePages={onMerge}/>));
  await render();
  cleanups.push(async()=>{await act(async()=>root.unmount());client.dispose();container.remove();});
  const page=(id:number,n:number)=>container.querySelector<HTMLElement>(`[aria-label^="#${id} ${n}페이지"]`)!;
  const button=(text:string)=>[...container.querySelectorAll<HTMLButtonElement>('button')].find(b=>b.textContent===text)!;
  const click=async(text:string)=>act(async()=>button(text).click());
  const select=async(id:number,n:number,shiftKey=false)=>act(async()=>page(id,n).dispatchEvent(new MouseEvent('click',{bubbles:true,ctrlKey:!shiftKey,shiftKey})));
  const mode=async(id:number,text:string)=>act(async()=>[...container.querySelectorAll<HTMLButtonElement>(`[aria-label="#${id} 남길 범위"] button`)].find(b=>b.textContent===text)!.click());
  return {container,render,page,button,click,select,mode,onDecision,onMerge};
}
describe('single pair uses the current edition review',()=>{
  it('shows two horizontal edition rows, compact evidence and modern controls',async()=>{
    const f=await mount();
    expect(f.container.querySelector('[aria-label="두 판본 검토"]')).not.toBeNull();
    expect(f.container.querySelectorAll('.overlap-group-grid-row')).toHaveLength(2);
    expect(f.container.querySelector('.download-overlap-metrics')).toBeNull();
    expect(f.container.querySelectorAll('.download-overlap-page-filter')).toHaveLength(2);
    expect(f.button('일치 보기')).toHaveAttribute('aria-pressed','true');
    expect(f.button('추가 보기')).toHaveAttribute('aria-pressed','true');
    expect(f.container.querySelector('.is-gap')).toBeNull();
    expect(f.container.textContent).toContain('Ctrl과 Shift를 이용해 원하는 페이지를 선택할 수 있습니다.');
    expect(f.container.querySelector('[aria-label="#100 비교"]')).toHaveAttribute('aria-pressed','true');
  });
  it('filters both classes without clearing hidden Ctrl selections or reassigning pages',async()=>{
    const f=await mount();vi.spyOn(window,'confirm').mockReturnValue(true);
    await f.select(100,3);await f.click('일치 보기');
    expect(f.container.querySelectorAll('.is-matched')).toHaveLength(0);
    expect(f.page(200,5)).toHaveClass('is-unique');
    await f.click('추가 보기');
    expect(f.container.textContent).toContain('표시할 페이지 없음');
    expect(f.container.querySelector('.overlap-group-selection-count[role="status"]')).toHaveTextContent('1장 선택');
    expect(f.button('중복 아님')).toBeDisabled();
    await f.click('적용');
    expect(f.onMerge).toHaveBeenCalledExactlyOnceWith({reviewId:'pair',candidateId:'pair-candidate',expectedRevision:4,sourceSide:'existing',sourcePages:[3],selectedPages:{existing:[3],incoming:[]},targetSelectionOnly:false,excludeSource:true});
    await f.click('일치 보기');
    expect(f.page(100,3)).toHaveAttribute('data-merge-state','source');
    expect(f.page(200,3)).toHaveAttribute('data-merge-state','target');
  });
  it('keeps the Shift anchor across hidden matches and selects the whole source range',async()=>{
    const f=await mount();await f.select(200,1);await f.click('일치 보기');await f.select(200,6,true);
    await f.click('일치 보기');
    for(let p=1;p<=6;p++)expect(f.page(200,p)).toHaveAttribute('data-merge-state','source');
    expect(f.container.querySelector('[aria-label="#200 남길 범위"]')).toHaveAttribute('data-mode','partial');
    expect(f.container.querySelector('.overlap-group-selection-count[role="status"]')).toHaveTextContent('6장 선택');
    await f.click('선택 해제');expect(f.container.querySelectorAll('[data-merge-state="source"]')).toHaveLength(0);
    expect(f.onMerge).not.toHaveBeenCalled();
  });
  it('preserves the draft across the next review and back, without resetting modes or filters',async()=>{
    const review=pairFixture(),f=await mount(review);
    await f.mode(100,'제거');await f.select(100,2);await f.click('추가 보기');
    await f.render(pairFixture('next'));await f.render(review);
    expect(f.page(100,2)).toHaveAttribute('data-merge-state','source');
    expect(f.container.querySelector('[aria-label="#100 남길 범위"]')).toHaveAttribute('data-mode','partial');
    expect(f.button('추가 보기')).toHaveAttribute('aria-pressed','false');
    await f.select(100,4,true);
    for(let p=2;p<=4;p++)expect(f.page(100,p)).toHaveAttribute('data-merge-state','source');
  });
  it('invalidates the selected proof on a real fingerprint change instead of merging stale pages',async()=>{
    const review=pairFixture(),f=await mount(review);await f.select(100,1);
    await f.render({...review,revision:5,incomingFingerprint:'changed'});await f.click('적용');
    expect(f.container.textContent).toContain('다시 선택');expect(f.onMerge).not.toHaveBeenCalled();expect(f.onDecision).not.toHaveBeenCalled();
  });
  it('switches matching sides exclusively and blocks actions while the current pair is processing',async()=>{
    const r=pairFixture(),f=await mount(r);await f.select(100,1);await f.select(200,1);
    expect(f.page(100,1)).toHaveAttribute('data-merge-state','target');expect(f.page(200,1)).toHaveAttribute('data-merge-state','source');
    await f.render(r,true);expect(f.button('적용')).toBeDisabled();
    await f.select(200,2);expect(f.page(200,2)).not.toHaveAttribute('data-merge-state','source');
  });
  it('keeps remove, keep and false-positive distinct and scoped to the selected candidate',async()=>{
    const f=await mount(),confirm=vi.spyOn(window,'confirm').mockReturnValue(false);
    await f.mode(100,'제거');await f.click('적용');expect(f.onDecision).not.toHaveBeenCalled();
    confirm.mockReturnValue(true);await f.click('적용');
    await f.mode(100,'남기기');await f.mode(200,'제거');await f.click('적용');
    await f.mode(200,'남기기');await f.click('적용');await f.click('중복 아님');
    for(const action of ['remove_existing_continue','remove_incoming','keep_both_continue','false_positive_continue'])expect(f.onDecision).toHaveBeenCalledWith({reviewId:'pair',candidateId:'pair-candidate',expectedRevision:4,action});
    await f.mode(100,'제거');await f.mode(200,'제거');await f.click('적용');
    expect(f.onDecision).toHaveBeenCalledTimes(4);expect(f.container.textContent).toContain('남길 판본을 하나 이상');
  });
  it('rejects incomplete persisted mappings even if the summary claims full containment',async()=>{
    const review=pairFixture();review.candidates[0]!.pagePairs.pop();
    const f=await mount(review);await f.select(100,1);
    expect(f.container.textContent).toContain('페이지 대응 다시 확인');expect(f.container.querySelectorAll('[data-merge-state="source"]')).toHaveLength(0);
    expect(f.onMerge).not.toHaveBeenCalled();
  });
});
