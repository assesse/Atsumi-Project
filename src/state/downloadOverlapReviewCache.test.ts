import {describe,it,expect,vi} from 'vitest';
import {DownloadOverlapReviewCache} from './downloadOverlapReviewCache';
import type {ApiResult,DownloadOverlapReview} from '../api/contracts';
const review=(revision:number)=>({reviewId:'r',revision,state:'pending'} as DownloadOverlapReview);
const ok=(revision:number):ApiResult<DownloadOverlapReview>=>({ok:true,data:review(revision)});
describe('revision scoped overlap inventory',()=>{
 it('coalesces simultaneous inventory and automatic reads, and does not poll unchanged evidence',async()=>{
 let resolve!:(v:ApiResult<DownloadOverlapReview>)=>void;
 const load=vi.fn(()=>new Promise<ApiResult<DownloadOverlapReview>>(r=>{resolve=r;}));
 const cache=new DownloadOverlapReviewCache(load);const a=cache.get('r',3),b=cache.get('r',3);
 expect(a).toBe(b);resolve(ok(3));await a;
 for(let i=0;i<1000;i++)await cache.get('r',3);
 expect(load).toHaveBeenCalledTimes(1);
 const changed=cache.get('r',4);resolve(ok(4));await changed;expect(load).toHaveBeenCalledTimes(2);
 });
 it('does not replace newer evidence with a late old response or cache failures',async()=>{
 const resolvers:((v:ApiResult<DownloadOverlapReview>)=>void)[]=[];
 const load=vi.fn(()=>new Promise<ApiResult<DownloadOverlapReview>>(r=>resolvers.push(r)));
 const cache=new DownloadOverlapReviewCache(load);const old=cache.get('r',1);const next=cache.get('r',2);
 resolvers[1]!(ok(2));await next;resolvers[0]!(ok(1));await old;expect(cache.peek('r')?.revision).toBe(2);
 cache.invalidate(['r']);const failed=cache.get('r');resolvers[2]!({ok:false,error:{code:'FAILED',message:'fixture',retryable:true}});await failed;
 expect(cache.peek('r')).toBeUndefined();const retry=cache.get('r');resolvers[3]!(ok(3));await retry;
 expect(load).toHaveBeenCalledTimes(4);
 });
 it('reconciles explicitly requested revisions even when a display snapshot is cached',async()=>{
 const resolvers:((v:ApiResult<DownloadOverlapReview>)=>void)[]=[];
 const load=vi.fn((_id:string,_reconcile:boolean)=>new Promise<ApiResult<DownloadOverlapReview>>(r=>resolvers.push(r)));
 const cache=new DownloadOverlapReviewCache(load);
 const snapshot=cache.get('r',1);const authoritative=cache.get('r',1,true);
 expect(authoritative).not.toBe(snapshot);expect(cache.get('r',1,true)).toBe(authoritative);
 expect(load.mock.calls).toEqual([['r',false],['r',true]]);
 resolvers[1]!(ok(2));await authoritative;resolvers[0]!(ok(1));await snapshot;
 expect(cache.peek('r')?.revision).toBe(2);
 const reconcileAgain=cache.get('r',2,true);expect(load).toHaveBeenCalledTimes(3);
 resolvers[2]!(ok(3));await reconcileAgain;expect(cache.peek('r')?.revision).toBe(3);
 });
 it('keeps a mutation result newer than a pending display response',async()=>{
 let resolve!:(v:ApiResult<DownloadOverlapReview>)=>void;
 const cache=new DownloadOverlapReviewCache(()=>new Promise(r=>{resolve=r;}));
 const pending=cache.get('r');cache.store(review(4));resolve(ok(3));await pending;
 expect(cache.peek('r')?.revision).toBe(4);
 });
});
