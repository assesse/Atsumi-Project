import { describe, expect, it } from "vitest";
import type { DownloadOverlapReview } from "../api/contracts";
import { galleryId } from "../core/types";
import { buildDownloadOverlapContainmentGroup } from "../state/downloadOverlapContainment";
import { buildGroupAlignment, buildOverlapReviewGroup, overlapEvidenceKey, relationDisplayPairs, visibleGroupRelations, type GroupSupplement } from "./groupReview";
const fixture = (): DownloadOverlapReview => ({
 reviewId:"r",entryId:"keeper",incoming:{entryId:"keeper",galleryId:galleryId(100),title:"Omnibus",artists:[],pageCount:12},
 revision:1,state:"pending",profileVersion:1,policyVersion:1,incomingFingerprint:"keeper-fp",createdAt:"",updatedAt:"",
 candidates:[1,2,3].map((n)=>({candidateId:"c"+n,existing:{entryId:"peer"+n,galleryId:galleryId(n),title:"Edition "+n,artists:[],pageCount:4},
 existingFingerprint:"peer-fp"+n,relation:n===3?"partial_overlap":"incoming_contains_existing",confidence:.99,
 matchedPages:n===3?3:4,exactPages:n===3?3:4,visualPages:0,existingCoverage:n===3?.75:1,incomingCoverage:(n===3?3:4)/12,
 existingUniquePages:n===3?1:0,incomingUniquePages:n===3?9:8,longestAlignedRun:n===3?3:4,rank:n,
 pagePairs:Array.from({length:n===3?3:4},(_,i)=>({incomingSourcePage:i+1+(n-1)*4,existingSourcePage:i+1,exactSha256:true,
 dHashDistance:0,pHashDistance:0,detailHashDistance:0,edgeSimilarity:1,visualSimilarity:1,lowInformation:false}))}))
});
describe("group review evidence and alignment",()=>{
 it("includes complete, uncertain and completed direct edges without inferring peer-peer matches",()=>{
  const review=fixture();review.candidates[0]!.decision="existing_removed";
  const containment=buildDownloadOverlapContainmentGroup(review.incoming.galleryId,[review])!;
  const group=buildOverlapReviewGroup(review,containment)!;
  expect(group.editions.map(x=>x.status)).toEqual(["excluded","contained","uncertain"]);
  expect(group.editions.filter(x=>x.canBatchExclude).map(x=>x.excluded.galleryId)).toEqual([2]);
  expect(group.editions.every(x=>x.review.incoming.entryId==="keeper")).toBe(true);
 });
 it("normalizes reverse reviews and prefers the keeper's own direct evidence",()=>{
  const own=fixture(),reverse=fixture();
  reverse.reviewId="reverse";reverse.incoming=own.candidates[0]!.existing;reverse.entryId=reverse.incoming.entryId;
  reverse.candidates=[{...own.candidates[0]!,existingFingerprint:own.incomingFingerprint,existing:own.incoming,relation:"existing_contains_incoming",existingCoverage:1/3,incomingCoverage:1,
    existingUniquePages:8,incomingUniquePages:0,pagePairs:own.candidates[0]!.pagePairs.map(p=>({...p,incomingSourcePage:p.existingSourcePage,existingSourcePage:p.incomingSourcePage}))}];
  const group=buildOverlapReviewGroup(own,buildDownloadOverlapContainmentGroup(own.incoming.galleryId,[own,reverse])!,[reverse])!;
  expect(group.editions).toHaveLength(3);
  expect(group.editions[0]!.keeperIsIncoming).toBe(true);
  const reverseOnly={...own,candidates:own.candidates.slice(1)};
  const reversed=buildOverlapReviewGroup(reverseOnly,buildDownloadOverlapContainmentGroup(own.incoming.galleryId,[reverseOnly,reverse])!,[reverse])!;
  const edge=reversed.editions.find(e=>e.excluded.galleryId===1)!;
  expect(edge.keeperIsIncoming).toBe(false);
  expect(buildGroupAlignment(own.incoming,[edge])[0]!.pages[edge.key]?.page).toBe(1);
 });
 it("unmatched pages from separate editions remain in different rows",()=>{
  const review=fixture();
  review.candidates=review.candidates.map(c=>({...c,relation:"partial_overlap",pagePairs:c.pagePairs.slice(0,3),matchedPages:3,
    existingUniquePages:1,incomingUniquePages:9}));
  const group=buildOverlapReviewGroup(review)!;
  const rows=buildGroupAlignment(group.keeper,group.editions);
  const extras=rows.filter(r=>!r.keeperPage);
  expect(extras).toHaveLength(3);
  expect(extras.every(row=>Object.values(row.pages).filter(Boolean).length===1)).toBe(true);
  expect(new Set(rows.map(r=>r.key)).size).toBe(rows.length);
 });
 it("preserves all keeper and source pages exactly once",()=>{
  const review=fixture(),group=buildOverlapReviewGroup(review)!;
  const rows=buildGroupAlignment(group.keeper,group.editions);
  expect(rows.flatMap(r=>r.keeperPage?[r.keeperPage]:[])).toEqual(Array.from({length:12},(_,i)=>i+1));
  for(const edge of group.editions) expect(rows.flatMap(r=>r.pages[edge.key]?[r.pages[edge.key]!.page]:[]).sort((a,b)=>a-b)).toEqual([1,2,3,4]);
 });
 it("ignores unrelated decision revisions but detects actual immutable evidence changes",()=>{
  const review=fixture(),candidate=review.candidates[0]!,key=overlapEvidenceKey(review,candidate);
  expect(overlapEvidenceKey({...review,revision:8,updatedAt:"later"},candidate)).toBe(key);
  expect(overlapEvidenceKey({...review,incomingFingerprint:"changed"},candidate)).not.toBe(key);
  expect(overlapEvidenceKey(review,{...candidate,existingFingerprint:"changed"})).not.toBe(key);
  expect(overlapEvidenceKey(review,{...candidate,pagePairs:candidate.pagePairs.slice(1)})).not.toBe(key);
  expect(overlapEvidenceKey({...review,policyVersion:2},candidate)).not.toBe(key);
 });
 it("does not add transitive edges from unrelated reviews",()=>{
  const review=fixture(),unrelated=fixture();unrelated.incoming={...unrelated.incoming,entryId:"other"};unrelated.entryId="other";unrelated.reviewId="other";
  expect(buildOverlapReviewGroup(review,undefined,[unrelated])!.editions).toHaveLength(3);
 });
 it("refuses to align a reverse edition against a different keeper fingerprint",()=>{
  const own=fixture(), reverse=fixture();reverse.reviewId="old-reverse";reverse.incoming=own.candidates[0]!.existing;reverse.entryId=reverse.incoming.entryId;
  reverse.candidates=[{...own.candidates[0]!,existing:own.incoming,existingFingerprint:"outdated-keeper"}];
  own.candidates=own.candidates.slice(1);
  const group=buildOverlapReviewGroup(own,undefined,[reverse])!;
  const edge=group.editions.find(e=>e.excluded.galleryId===1)!;
  expect(edge.keeperCompatible).toBe(false);expect(edge.canDecide).toBe(false);expect(edge.status).toBe("stale");
  expect(buildGroupAlignment(group.keeper,[edge]).every(r=>r.pages[edge.key]===undefined)).toBe(true);
 });
 it("does not enable a stale reviewed edition for batch removal",()=>{
  const review=fixture(),containment=buildDownloadOverlapContainmentGroup(review.incoming.galleryId,[review])!;
  review.state="stale";
  expect(buildOverlapReviewGroup(review,containment)!.editions.every(e=>!e.canDecide&&!e.canBatchExclude)).toBe(true);
 });
});

const pagePair=(a:number,b:number):DownloadOverlapReview["candidates"][number]["pagePairs"][number]=>({
 incomingSourcePage:a,existingSourcePage:b,exactSha256:true,dHashDistance:0,pHashDistance:0,
 detailHashDistance:0,edgeSimilarity:1,visualSimilarity:1,lowInformation:false
});
const relatedFixture=()=>{
 const own=fixture();own.incoming.pageCount=100;
 own.candidates=own.candidates.slice(0,2).map(c=>({...c,relation:"partial_overlap" as const,
  existing:{...c.existing,pageCount:105},pagePairs:[],matchedPages:0}));
 own.candidates[0]!.pagePairs=[pagePair(7,7),pagePair(8,8),pagePair(9,9),pagePair(58,58)];
 own.candidates[1]!.pagePairs=[pagePair(7,9),pagePair(8,10),...Array.from({length:15},(_,i)=>pagePair(83+i,84+i))];
 const peer:DownloadOverlapReview={...own,reviewId:"peer-review",entryId:"peer1",incoming:own.candidates[0]!.existing,
  incomingFingerprint:own.candidates[0]!.existingFingerprint,
  candidates:[{...own.candidates[1]!,candidateId:"peer-match",pagePairs:[pagePair(1,2)]}]};
 return {own,peer,group:buildOverlapReviewGroup(own,undefined,[peer])!};
};

describe("group-wide page identity",()=>{
 it("groups directly matched pages absent from the keeper without inventing a keeper page",()=>{
  const {group}=relatedFixture(),[b,c]=group.editions;
  const rows=buildGroupAlignment(group.keeper,group.editions,group.relations);
  const row=rows.find(r=>r.pages[b!.key]?.page===1)!;
  expect(row.keeperPage).toBeUndefined();
  expect(row.pages[c!.key]).toMatchObject({page:2,matched:true});
  expect(row.pages[b!.key]).toMatchObject({page:1,matched:true,pair:undefined});
 });
 it("retains a common 15-page block even though a third edition has none of it",()=>{
  const {group}=relatedFixture(),[b,c]=group.editions;
  const rows=buildGroupAlignment(group.keeper,group.editions,group.relations);
  for(let n=83;n<=97;n++){
   const row=rows.find(r=>r.keeperPage===n)!;
   expect(row.keeperMatched).toBe(true);
   expect(row.pages[b!.key]).toBeUndefined();
   expect(row.pages[c!.key]).toMatchObject({page:n+1,matched:true});
  }
 });
 it("adds unordered display matches while keeping durable merge evidence and page counts untouched",()=>{
  const {group}=relatedFixture(),[b,c]=group.editions,before=JSON.stringify(group);
  const supplements:GroupSupplement={
   [c!.key]:{reviewId:c!.review.reviewId,revision:c!.review.revision,candidateId:c!.candidate.candidateId,
    incomingFingerprint:c!.review.incomingFingerprint,existingFingerprint:c!.candidate.existingFingerprint,
    additionalPairs:[pagePair(9,8),pagePair(58,61)]}
  };
  const rows=buildGroupAlignment(group.keeper,group.editions,group.relations,supplements);
  const nine=rows.find(r=>r.keeperPage===9)!;
  expect(nine.keeperReordered).toBe(true);
  expect(nine.pages[b!.key]).toMatchObject({page:9,matched:true});
  expect(nine.pages[c!.key]).toMatchObject({page:8,matched:true,reordered:true,pair:undefined});
  expect(rows.find(r=>r.keeperPage===58)?.pages[c!.key]).toMatchObject({page:61,matched:true});
  for(const edition of group.editions){
   const pages=rows.flatMap(r=>r.pages[edition.key]?[r.pages[edition.key]!.page]:[]);
   expect(pages.length).toBe(edition.excluded.pageCount);
   expect(new Set(pages).size).toBe(pages.length);
  }
  expect(JSON.stringify(group)).toBe(before);
 });
 it("does not infer a missing keeper-to-peer match through another edition",()=>{
  const {own,peer}=relatedFixture();
  own.candidates[0]!.pagePairs=[pagePair(9,9)];
  own.candidates[1]!.pagePairs=[];
  peer.candidates[0]!.pagePairs=[pagePair(9,8)];
  const group=buildOverlapReviewGroup(own,undefined,[peer])!,[b,c]=group.editions;
  const rows=buildGroupAlignment(group.keeper,group.editions,group.relations);
  const base=rows.find(r=>r.keeperPage===9)!;
  expect(base.pages[b!.key]?.page).toBe(9);
  expect(base.pages[c!.key]).toBeUndefined();
  const independent=rows.find(r=>r.pages[c!.key]?.page===8)!;
  expect(independent.pages[c!.key]?.matched).toBe(true);
  expect(independent.pages[c!.key]?.counterparts).toEqual(["#1 9p"]);
 });
 it("rejects stale, wrong-fingerprint, reused and low-confidence supplemental pairs as a whole",()=>{
  const {group}=relatedFixture(),c=group.editions[1]!;
  const relation={key:c.key,review:c.review,candidate:c.candidate,evidence:c.evidence};
  const good={reviewId:c.review.reviewId,revision:c.review.revision,candidateId:c.candidate.candidateId,
   incomingFingerprint:c.review.incomingFingerprint,existingFingerprint:c.candidate.existingFingerprint,
   additionalPairs:[pagePair(9,8)]};
  for(const bad of [
   {...good,revision:2},{...good,existingFingerprint:"new"},
   {...good,additionalPairs:[pagePair(9,9)]},
   {...good,additionalPairs:[{...pagePair(9,8),lowInformation:true}]},
   {...good,additionalPairs:[{...pagePair(9,8),exactSha256:false,visualSimilarity:.7}]},
   {...good,additionalPairs:[pagePair(101,8)]}
  ])expect(relationDisplayPairs(relation,bad)).toBe(c.candidate.pagePairs);
 });
 it("ignores peer evidence from a previous artifact revision and requests only visible pairs",()=>{
  const {own,peer}=relatedFixture();peer.incomingFingerprint="old";
  const group=buildOverlapReviewGroup(own,undefined,[peer])!;
  expect(group.relations).toHaveLength(2);
  expect(visibleGroupRelations(group,[group.editions[0]!])).toHaveLength(1);
 });
});
