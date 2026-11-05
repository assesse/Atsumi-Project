import { describe, expect, it } from 'vitest';
import { galleryId } from '../core/types';
import { nextManualReviewIndex, SerializedReviewWork, settleManualReviewSession } from './manualReviewFlow';
const session={index:0,items:['a','b','c'].map((reviewId,i)=>({reviewId,incomingGalleryId:galleryId(i+1)}))};
const deferred=()=>{let resolve!:()=>void,reject!:(e:Error)=>void;const promise=new Promise<void>((a,b)=>{resolve=a;reject=b;});return {promise,resolve,reject};};
describe('continuous manual review work',()=>{
  it('advances immediately past queued work without declaring it completed',async()=>{
    const work=new SerializedReviewWork(), gate=deferred();
    const first=work.enqueue(['a'],()=>gate.promise);
    expect(nextManualReviewIndex(session,work.ids)).toBe(1);
    expect(work.size).toBe(1);expect(work.ids).toEqual(new Set(['a']));
    expect(session.items).toHaveLength(3);
    gate.resolve();await first;expect(work.size).toBe(0);
  });
  it('serializes filesystem actions while accepting the next user choice, and continues after failure',async()=>{
    const work=new SerializedReviewWork(), gate=deferred(), order:string[]=[];
    const a=work.enqueue(['a','shared'],async()=>{order.push('a-start');await gate.promise;order.push('a-end');throw Error('verify failed');}).catch(e=>e.message);
    const b=work.enqueue(['b'],async()=>{order.push('b-start');return 'done';});
    await Promise.resolve();expect(order).toEqual(['a-start']);expect(work.size).toBe(2);
    expect(nextManualReviewIndex({...session,index:1},work.ids)).toBe(2);
    await expect(work.enqueue(['shared'],async()=>{})).rejects.toThrow('이미 적용 중');
    gate.resolve();expect(await a).toBe('verify failed');expect(await b).toBe('done');
    expect(order).toEqual(['a-start','a-end','b-start']);expect(work.ids.size).toBe(0);
  });
  it('only removes confirmed terminal reviews and keeps current selection through preceding completions',()=>{
    const current={...session,index:2};
    const settled=settleManualReviewSession(current,new Set(['a']))!;
    expect(settled.items[settled.index]?.reviewId).toBe('c');expect(settled.index).toBe(1);
    expect(settleManualReviewSession(settled,new Set(['c']))?.items[0]?.reviewId).toBe('b');
    expect(settleManualReviewSession(session,new Set(['a','b','c']))).toBeNull();
  });
  it('does not choose a pending review when all remaining choices were submitted',()=>{
    expect(nextManualReviewIndex(session,new Set(['a','b','c']))).toBe(-1);
  });
});
