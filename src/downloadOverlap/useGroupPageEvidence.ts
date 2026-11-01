import { useEffect, useMemo, useRef, useState } from "react";
import type { ApiResult, DownloadOverlapDisplayMatchRequest, DownloadOverlapDisplayMatches } from "../api/contracts";
import type { GroupDirectRelation, GroupSupplement } from "./groupReview";

export type PageEvidenceLoader = (request: DownloadOverlapDisplayMatchRequest) => Promise<ApiResult<DownloadOverlapDisplayMatches>>;
type Entry = { result?: DownloadOverlapDisplayMatches; failed?: boolean; promise?: Promise<void> };
const requestKey=(r:GroupDirectRelation)=>r.evidence+":"+r.review.revision;

/** Local to the open dialog. Two reads at a time; no polling or full library reload. */
export function useGroupPageEvidence(relations: readonly GroupDirectRelation[], load?: PageEvidenceLoader) {
  const cache=useRef(new Map<string,Entry>()), loader=useRef(load);loader.current=load;
  const [version,setVersion]=useState(0);
  const [retryVersion,setRetryVersion]=useState(0);
  const signature=JSON.stringify(relations.map(requestKey));
  useEffect(()=>{
    if(!loader.current)return;
    let cancelled=false,next=0;
    const prior=[...cache.current.values()].flatMap(e=>e.promise?[e.promise]:[]);
    const requests=relations.filter(r=>!cache.current.has(requestKey(r)));
    const run=async()=>{
      while(!cancelled&&next<requests.length){
        const r=requests[next++]!,key=requestKey(r),entry:Entry={};
        if(cache.current.has(key))continue;
        cache.current.set(key,entry);
        entry.promise=Promise.resolve().then(async()=>{
          try{
            const response=await loader.current!({reviewId:r.review.reviewId,expectedRevision:r.review.revision,candidateId:r.candidate.candidateId});
            const value=response.ok?response.data:undefined;
            if(value&&value.reviewId===r.review.reviewId&&value.revision===r.review.revision
              &&value.candidateId===r.candidate.candidateId&&value.incomingFingerprint===r.review.incomingFingerprint
              &&value.existingFingerprint===r.candidate.existingFingerprint)entry.result=value;
            else entry.failed=true;
          }catch{entry.failed=true;}
          finally{entry.promise=undefined;if(!cancelled)setVersion(v=>v+1);}
        });
        await entry.promise;
      }
    };
    void Promise.all(prior).then(()=>Promise.all([run(),run()]));
    // A reused in-flight request still needs to notify the new visible scope.
    for(const r of relations){const pending=cache.current.get(requestKey(r))?.promise;
      if(pending)void pending.then(()=>{if(!cancelled)setVersion(v=>v+1);});}
    return()=>{cancelled=true;};
  },[signature,Boolean(load),retryVersion]);
  const supplements=useMemo(()=>{
    const values:GroupSupplement={};
    for(const r of relations){const entry=cache.current.get(requestKey(r));if(entry?.result)values[r.key]=entry.result;}
    return values;
  },[signature,version]);
  const pending=Boolean(load)&&relations.some(r=>{const e=cache.current.get(requestKey(r));return !e||Boolean(e.promise);});
  const failed=Boolean(load)&&relations.some(r=>cache.current.get(requestKey(r))?.failed);
  const retry=()=>{for(const r of relations)if(cache.current.get(requestKey(r))?.failed)cache.current.delete(requestKey(r));setRetryVersion(v=>v+1);};
  return {supplements,pending,failed,retry};
}
