import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, expect, it, vi } from 'vitest';
import { backend } from '../../api/backend';
import { AppShell } from '../../app/AppShell';
import { HitomiFeature } from './HitomiFeature';
import { ThumbnailProvider,ThumbnailClient } from '../../thumbnail';
import { pairFixture } from '../../test/overlapReviewFixture';
import type { ApiResult, DownloadEntry, DownloadOverlapDecisionResult } from '../../api/contracts';
import { galleryId } from '../../core/types';

const deferred=<T,>()=>{let resolve!:(v:T)=>void;const promise=new Promise<T>(done=>{resolve=done;});return {promise,resolve};};
const disposers:(()=>Promise<void>)[]=[];
afterEach(async()=>{for(const dispose of disposers.splice(0))await dispose();vi.restoreAllMocks();vi.unstubAllGlobals();localStorage.clear();});

it.each([{firstFails:true,background:false},{firstFails:false,background:false},{firstFails:true,background:true},{firstFails:false,background:true}])('keeps the dialog flow stable through delayed work: %j',async({firstFails,background})=>{
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT',true);
  vi.stubGlobal('ResizeObserver',class{observe(){}unobserve(){}disconnect(){}});
  vi.stubGlobal('IntersectionObserver',class{observe(){}unobserve(){}disconnect(){}});
  vi.spyOn(window,'confirm').mockReturnValue(true);
  localStorage.setItem('atsumi.tutorial.dismissed.v1','true');
  const saved=await backend.settingsGet();if(!saved.ok)throw Error('fixture settings missing');
  vi.spyOn(backend,'settingsGet').mockResolvedValue({ok:true,data:{...saved.data,downloadOverlapAutoMode:'off',privacyMode:false,privacyOnStartup:false}});
  const reviews=[pairFixture('first'),pairFixture('second')];
  reviews[1]!.incoming={...reviews[1]!.incoming,galleryId:galleryId(201),title:'Second current review'};
  const entries=reviews.map((r):DownloadEntry=>({entryId:r.entryId,galleryId:r.incoming.galleryId,revision:1,state:'review_required',reviewKind:'gallery_duplicate',reviewId:r.reviewId,updatedAt:new Date().toISOString()}));
  vi.spyOn(backend,'downloadEntriesList').mockImplementation(async req=>({ok:true,data:{page:req.page,totalItems:entries.length,entries:entries.filter(e=>!req.state||e.state===req.state)}}));
  vi.spyOn(backend,'downloadOverlapReviewGet').mockImplementation(async id=>({ok:true,data:structuredClone(reviews.find(r=>r.reviewId===id)!)}));
  vi.spyOn(backend,'downloadOverlapDisplayMatches').mockImplementation(async req=>{const r=reviews.find(r=>r.reviewId===req.reviewId)!;return {ok:true,data:{reviewId:r.reviewId,candidateId:req.candidateId,revision:r.revision,incomingFingerprint:r.incomingFingerprint,existingFingerprint:r.candidates[0]!.existingFingerprint,additionalPairs:[]}};});
  const gates=[deferred<ApiResult<DownloadOverlapDecisionResult>>(),deferred<ApiResult<DownloadOverlapDecisionResult>>()];
  const decision=vi.spyOn(backend,'downloadOverlapDecisionApply').mockImplementation(req=>gates[req.reviewId==='first'?0:1]!.promise);
  const thumbnails=new ThumbnailClient({resolve:()=>({kind:'missing',reason:'fixture'})});
  const container=document.createElement('div');document.body.append(container);const root=createRoot(container);
  disposers.push(async()=>{await act(async()=>root.unmount());thumbnails.dispose();container.remove();});
  await act(async()=>root.render(<ThumbnailProvider client={thumbnails}><AppShell api={backend}><HitomiFeature active>{bridge=><>{bridge.workspace}<button onClick={bridge.onActivity}>fixture activity</button></>}</HitomiFeature></AppShell></ThumbnailProvider>));
  const flush=async()=>act(async()=>{await new Promise(r=>setTimeout(r,30));});
  const button=(text:string)=>[...container.querySelectorAll<HTMLButtonElement>('button')].find(b=>b.textContent?.trim().endsWith(text));
  const click=async(text:string)=>{expect(button(text),text+" buttons="+[...container.querySelectorAll("button")].map(b=>b.textContent).join(" | ")).toBeDefined();await act(async()=>button(text)!.click());};
  await flush();await click('fixture activity');await click('자동분류 검토');await click('검토 필요 목록');await flush();
  const dialog=container.querySelector<HTMLDialogElement>('dialog.download-overlap-dialog')!;
  expect(dialog).not.toBeNull();expect(dialog.textContent).toContain('New edition');
  await click('1개 적용');await flush();
  expect(decision).toHaveBeenCalledTimes(1);
  expect(container.querySelector('dialog.download-overlap-dialog')).toBe(dialog);
  expect(dialog.textContent).toContain('Second current review');expect(dialog.open).toBe(true);
  expect(button('1개 적용')).toBeEnabled();expect(dialog.textContent).toContain('적용 중 1개');
  await click('1개 적용');await flush();
  expect(decision).toHaveBeenCalledTimes(1);expect(dialog.textContent).toContain('적용 중 2개');
  if(background){
    await act(async()=>dialog.querySelector<HTMLButtonElement>('button[aria-label="닫기"]')!.click());
    expect(container.querySelector('dialog.download-overlap-dialog[open]')).toBeNull();
  }
  // A failed result must retain the original review and selected draft.
  const finish=(index:number)=>{
    const completed={...reviews[index]!,state:'resolved' as const,revision:5,candidates:reviews[index]!.candidates.map(c=>({...c,decision:'keep_both' as const}))};
    reviews[index]=completed;
    return {ok:true as const,data:{review:completed,cancelled:false,resumed:true}};
  };
  await act(async()=>gates[0]!.resolve(firstFails ? {ok:false,error:{code:'FIXTURE_FAILURE',message:'Synthetic verification failure',retryable:false,action:'none'}} : finish(0)));await flush();
  expect(decision).toHaveBeenCalledTimes(2);
  expect(Boolean(container.querySelector('dialog.download-overlap-dialog[open]'))).toBe(!background);
  if(firstFails)expect(dialog.textContent).toContain('재검토 1개');
  await act(async()=>gates[1]!.resolve(finish(1)));await flush();
  if(firstFails){
    if(background){
      expect(container.querySelector('dialog.download-overlap-dialog[open]')).toBeNull();
      await click('검토 계속');await flush();
    }
    expect(dialog.textContent).toContain('New edition');expect(dialog.textContent).toContain('Synthetic verification failure');
    expect(container.querySelector('dialog.download-overlap-dialog')).toBe(dialog);expect(dialog.open).toBe(true);
    expect(button('1개 적용')).toBeEnabled();
  }else{
    for(let attempt=0;dialog.isConnected && dialog.open && attempt<50;attempt++)await flush();
    expect(container.querySelector("dialog.download-overlap-dialog[open]")).toBeNull();expect(container.textContent).toContain('수동 검토를 마쳤습니다.');
  }

},20000);
