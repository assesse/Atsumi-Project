import {act} from "react";
import {createRoot} from "react-dom/client";
import {describe,it,expect,vi} from "vitest";
import type {ApiResult,DownloadOverlapDisplayMatches} from "../api/contracts";
import {galleryId} from "../core/types";
import type {GroupDirectRelation} from "./groupReview";
import {useGroupPageEvidence,type PageEvidenceLoader} from "./useGroupPageEvidence";

const relation=(n:number,revision=1):GroupDirectRelation=>{
 const review={reviewId:"r"+n,entryId:"keeper",incoming:{entryId:"keeper",galleryId:galleryId(100),title:"Base",artists:[],pageCount:3},
 revision,state:"pending" as const,profileVersion:1,policyVersion:2,incomingFingerprint:"base",createdAt:"",updatedAt:"",candidates:[]};
 const candidate={candidateId:"c"+n,existing:{entryId:"e"+n,galleryId:galleryId(n),title:"Peer",artists:[],pageCount:3},
 existingFingerprint:"peer"+n,relation:"partial_overlap" as const,confidence:.99,matchedPages:1,exactPages:1,visualPages:0,
 existingCoverage:1/3,incomingCoverage:1/3,existingUniquePages:2,incomingUniquePages:2,longestAlignedRun:1,rank:n,pagePairs:[]};
 return {key:"r"+n+":c"+n,review,candidate,evidence:"evidence"+n};
};
const response=(r:GroupDirectRelation):ApiResult<DownloadOverlapDisplayMatches>=>({ok:true,data:{
 reviewId:r.review.reviewId,revision:r.review.revision,candidateId:r.candidate.candidateId,
 incomingFingerprint:r.review.incomingFingerprint,existingFingerprint:r.candidate.existingFingerprint,additionalPairs:[]
}});
const deferred=()=>{let resolve!:(v:ApiResult<DownloadOverlapDisplayMatches>)=>void;
 const promise=new Promise<ApiResult<DownloadOverlapDisplayMatches>>(r=>{resolve=r;});return {promise,resolve};};
function harness(load:PageEvidenceLoader){
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 let current!:ReturnType<typeof useGroupPageEvidence>;
 function Probe({relations}:{relations:GroupDirectRelation[]}){current=useGroupPageEvidence(relations,load);return null;}
 return {get current(){return current;},
 render:async(relations:GroupDirectRelation[])=>act(async()=>{root.render(<Probe relations={relations}/>);}),
 dispose:async()=>{await act(async()=>root.unmount());host.remove();}
 };
}
describe("supplemental page evidence",()=>{
 it("limits concurrent reads to two, caches completed reads, and keeps output identity stable on unrelated renders",async()=>{
  const pending=new Map<string,ReturnType<typeof deferred>>(),relations=[1,2,3,4].map(n=>relation(n));
  const load=vi.fn((request)=>{const d=deferred();pending.set(request.reviewId,d);return d.promise;});
  const h=harness(load);
  try{
   await h.render(relations);expect(load).toHaveBeenCalledTimes(2);expect(h.current.pending).toBe(true);
   await act(async()=>pending.get("r1")!.resolve(response(relations[0]!)));
   expect(load).toHaveBeenCalledTimes(3);
   await act(async()=>pending.get("r2")!.resolve(response(relations[1]!)));
   expect(load).toHaveBeenCalledTimes(4);
   await act(async()=>{pending.get("r3")!.resolve(response(relations[2]!));pending.get("r4")!.resolve(response(relations[3]!));});
   expect(h.current.pending).toBe(false);expect(Object.keys(h.current.supplements)).toHaveLength(4);
   const identity=h.current.supplements;await h.render([...relations]);
   expect(h.current.supplements).toBe(identity);expect(load).toHaveBeenCalledTimes(4);
   await h.render([relations[0]!]);await h.render(relations);expect(load).toHaveBeenCalledTimes(4);
  }finally{await h.dispose();}
 });
 it("does not expose a late response from an older revision or exceed concurrency after a scope change",async()=>{
  const old=relation(1),newer=relation(1,2),other=relation(2),a=deferred(),b=deferred(),c=deferred();
  const load=vi.fn().mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise).mockReturnValueOnce(c.promise);
  const h=harness(load);
  try{
   await h.render([old,other]);await h.render([newer]);expect(load).toHaveBeenCalledTimes(2);
   await act(async()=>a.resolve(response(old)));
   expect(h.current.supplements[newer.key]).toBeUndefined();expect(load).toHaveBeenCalledTimes(2);
   await act(async()=>b.resolve(response(other)));expect(load).toHaveBeenCalledTimes(3);
   await act(async()=>c.resolve(response(newer)));
   expect(h.current.supplements[newer.key]?.revision).toBe(2);
   expect(h.current.pending).toBe(false);
  }finally{await h.dispose();}
 });
 it("notifies the new scope when an existing in-flight response is reused",async()=>{
  const a=relation(1),b=relation(2),one=deferred(),two=deferred();
  const load=vi.fn().mockReturnValueOnce(one.promise).mockReturnValueOnce(two.promise),h=harness(load);
  try{
   await h.render([a,b]);await h.render([a]);
   await act(async()=>one.resolve(response(a)));
   expect(h.current.pending).toBe(false);expect(h.current.supplements[a.key]).toBeDefined();
   expect(load).toHaveBeenCalledTimes(2);
   await act(async()=>two.resolve(response(b)));
  }finally{await h.dispose();}
 });
 it("fails closed on malformed evidence, retries only failures and handles synchronous exceptions",async()=>{
  const r=relation(1),good=response(r);if(!good.ok)throw Error();
  const load=vi.fn().mockImplementationOnce(()=>{throw Error("offline");})
   .mockResolvedValueOnce({ok:true,data:{...good.data,existingFingerprint:"wrong"}}).mockResolvedValue(good);
  const h=harness(load);
  try{
   await h.render([r]);expect(h.current.pending).toBe(false);expect(h.current.failed).toBe(true);
   await act(async()=>h.current.retry());expect(h.current.failed).toBe(true);
   expect(h.current.supplements[r.key]).toBeUndefined();
   await act(async()=>h.current.retry());expect(h.current.failed).toBe(false);expect(h.current.pending).toBe(false);
   expect(h.current.supplements[r.key]).toEqual(good.data);expect(load).toHaveBeenCalledTimes(3);
  }finally{await h.dispose();}
 });
});
