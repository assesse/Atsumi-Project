import { describe, it, expect } from "vitest";
import type { DownloadOverlapCandidate } from "../api/contracts";
import { galleryId } from "../core/types";
import { emptyMergeSelection, selectMergePages, mergeOutputPages } from "./mergeSelection";
const candidate: DownloadOverlapCandidate = {
  candidateId:"c",existing:{entryId:"a",galleryId:galleryId(1),title:"A",artists:[],pageCount:5},existingFingerprint:"a",
  relation:"partial_overlap",confidence:.99,matchedPages:3,exactPages:3,visualPages:0,existingCoverage:.6,incomingCoverage:.6,
  existingUniquePages:2,incomingUniquePages:2,longestAlignedRun:2,rank:1,
  pagePairs:[1,2,5].map((n)=>({existingSourcePage:n,incomingSourcePage:n,exactSha256:true,dHashDistance:0,pHashDistance:0,detailHashDistance:0,edgeSimilarity:1,visualSimilarity:1,lowInformation:false})),
};
describe("mixed merge choices",()=>{
  it("Ctrl toggles and choosing the other side replaces exactly its paired choice",()=>{
    let s=selectMergePages(candidate,emptyMergeSelection(),"existing",1);
    s=selectMergePages(candidate,s,"incoming",1);
    expect(s).toEqual({existing:[],incoming:[1]});
    s=selectMergePages(candidate,s,"existing",3);
    s=selectMergePages(candidate,s,"incoming",3);
    expect(s).toEqual({existing:[3],incoming:[1,3]}); // unmatched ≠ counterparts
    s=selectMergePages(candidate,s,"incoming",1);
    expect(s.incoming).toEqual([3]);
  });
  it("Shift adds the inclusive range and clears only actual counterparts",()=>{
    expect(selectMergePages(candidate,{existing:[1,2,3],incoming:[]},"incoming",5,{side:"incoming",page:1},true))
      .toEqual({existing:[3],incoming:[1,2,3,4,5]});
  });
  it("keeps destination order and inserts selected unique pages at the next matched anchor",()=>{
    const result=mergeOutputPages(candidate,5,{sourceSide:"incoming",selectedPages:{existing:[2],incoming:[1,3]}});
    expect(result).toEqual([{side:"incoming",page:1},{side:"existing",page:2},{side:"existing",page:3},{side:"existing",page:4},{side:"incoming",page:3},{side:"existing",page:5}]);
  });
  it("rejects both counterparts, duplicate IDs and malformed evidence",()=>{
    expect(()=>mergeOutputPages(candidate,5,{sourceSide:"incoming",selectedPages:{existing:[1],incoming:[1]}})).toThrow();
    expect(()=>mergeOutputPages(candidate,5,{sourceSide:"incoming",selectedPages:{existing:[],incoming:[1,1]}})).toThrow();
    expect(()=>mergeOutputPages({...candidate,matchedPages:2},5,{sourceSide:"incoming",selectedPages:emptyMergeSelection()})).toThrow();
  });
});
