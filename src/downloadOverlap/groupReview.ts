import type { DownloadOverlapCandidate, DownloadOverlapGalleryRef, DownloadOverlapPagePair, DownloadOverlapReview, DownloadOverlapDisplayMatches } from "../api/contracts";
import type { DownloadOverlapContainmentGroup, DownloadOverlapContainmentItem } from "../state/downloadOverlapContainment";
import { strictCandidateEvaluation } from "../state/downloadOverlapAuto";

export type GroupEdition = DownloadOverlapContainmentItem & {
  status: "contained" | "uncertain" | "excluded" | "kept" | "different" | "stale";
  keeperCompatible: boolean;
  canDecide: boolean;
  canBatchExclude: boolean;
  evidence: string;
};
export type OverlapReviewGroup = { keeper: DownloadOverlapGalleryRef; editions: GroupEdition[]; confirmed: boolean; relations: GroupDirectRelation[] };

/** Revision can advance after another candidate's decision. Selection follows the
 * actual immutable evidence, never the ordinal or an unrelated review revision. */
export function overlapEvidenceKey(review: DownloadOverlapReview, candidate: DownloadOverlapCandidate): string {
  return JSON.stringify([review.reviewId, review.entryId, review.incoming.entryId, review.incoming.galleryId,
    review.incomingFingerprint, review.incoming.pageCount, review.profileVersion, review.policyVersion,
    candidate.candidateId, candidate.existing.entryId, candidate.existing.galleryId,
    candidate.existingFingerprint, candidate.existing.pageCount, candidate.relation,
    candidate.matchedPages, candidate.existingUniquePages, candidate.incomingUniquePages, candidate.pagePairs]);
}

export function buildOverlapReviewGroup(
  review: DownloadOverlapReview, containment?: DownloadOverlapContainmentGroup,
  related: readonly DownloadOverlapReview[] = [],
): OverlapReviewGroup | null {
  const keeper = containment?.keeper ?? review.incoming;
  const keeperFingerprint = review.incoming.entryId === keeper.entryId ? review.incomingFingerprint
    : review.candidates.find((item) => item.existing.entryId === keeper.entryId)?.existingFingerprint
      ?? containment?.items.map((item) => item.keeperIsIncoming ? item.review.incomingFingerprint : item.candidate.existingFingerprint)[0];
  const reviews = new Map<string, DownloadOverlapReview>();
  for (const item of [...related, ...(containment?.items.map((item) => item.review) ?? []), review]) {
    if ((reviews.get(item.reviewId)?.revision ?? -1) <= item.revision) reviews.set(item.reviewId, item);
  }
  const eligibleKeys = new Set(containment?.items.map((item) => item.key));
  const editions = new Map<string, GroupEdition>();
  for (const source of reviews.values()) for (const candidate of source.candidates) {
    const keeperIsIncoming = source.incoming.entryId === keeper.entryId;
    if (!keeperIsIncoming && candidate.existing.entryId !== keeper.entryId) continue;
    const excluded = keeperIsIncoming ? candidate.existing : source.incoming;
    if (excluded.entryId === keeper.entryId || excluded.galleryId === keeper.galleryId) continue;
    const evaluation = strictCandidateEvaluation(source, candidate);
    const contained = evaluation?.decisionPath === "complete_containment"
      && evaluation.winner === (keeperIsIncoming ? "incoming" : "existing");
    const sourceKeeper = keeperIsIncoming ? source.incoming : candidate.existing;
    const keeperCompatible = (keeperIsIncoming ? source.incomingFingerprint : candidate.existingFingerprint) === keeperFingerprint
      && sourceKeeper.pageCount === keeper.pageCount;
    const canDecide = keeperCompatible && source.state === "pending" && candidate.decision === undefined;
    const removedIncoming = !keeperIsIncoming && source.state === "cancelled"
      && source.decisions?.some((decision) => decision.action === "remove_incoming"
        && (!decision.candidateId || decision.candidateId === candidate.candidateId));
    const status: GroupEdition["status"] = keeperIsIncoming && candidate.decision === "existing_removed" || removedIncoming
      ? "excluded" : candidate.decision === "keep_both" ? "kept"
      : candidate.decision === "false_positive" ? "different"
      : source.state !== "pending" || !keeperCompatible ? "stale" : contained ? "contained" : "uncertain";
    const key = source.reviewId + ":" + candidate.candidateId;
    const item: GroupEdition = { key, review: source, candidate, excluded, keeperIsIncoming,
      action: keeperIsIncoming ? "remove_existing_continue" : "remove_incoming",
      status, keeperCompatible, canDecide, canBatchExclude: canDecide && contained && eligibleKeys.has(key),
      evidence: overlapEvidenceKey(source, candidate) };
    const known = editions.get(excluded.entryId);
    if (!known || (keeperIsIncoming && !known.keeperIsIncoming)) editions.set(excluded.entryId, item);
  }
  if (editions.size < 2) return null;

  const known = new Map([[keeper.entryId,{fingerprint:keeperFingerprint,count:keeper.pageCount}],
    ...[...editions.values()].filter(e=>e.keeperCompatible).map(e=>[e.excluded.entryId,{fingerprint:e.keeperIsIncoming?e.candidate.existingFingerprint:e.review.incomingFingerprint,count:e.excluded.pageCount}] as const)]);
  const relations = new Map<string,GroupDirectRelation>();
  for(const source of reviews.values())for(const candidate of source.candidates){
    const left=known.get(source.incoming.entryId),right=known.get(candidate.existing.entryId);
    if(!left||!right||source.state==="stale"||candidate.decision==="false_positive"
      ||left.fingerprint!==source.incomingFingerprint||right.fingerprint!==candidate.existingFingerprint
      ||left.count!==source.incoming.pageCount||right.count!==candidate.existing.pageCount)continue;
    const key=source.reviewId+":"+candidate.candidateId;
    const identity=[source.incoming.entryId,candidate.existing.entryId].sort().join("|");
    const prior=relations.get(identity);
    if(!prior||source.incoming.entryId===keeper.entryId)relations.set(identity,{key,review:source,candidate,evidence:overlapEvidenceKey(source,candidate)});
  }

  return { keeper, relations: [...relations.values()], confirmed: Boolean(containment) || [...editions.values()].some((item) => item.status === "contained"),
    editions: [...editions.values()].sort((a, b) => Number(a.excluded.galleryId) - Number(b.excluded.galleryId)) };
}


export type GroupPage = { page: number; pair?: DownloadOverlapPagePair; matched?: boolean; reordered?: boolean; counterparts?: string[] };
export type GroupAlignmentRow = { key: string; keeperPage?: number; keeperMatched?: boolean; keeperReordered?: boolean; keeperCounterparts?: string[]; pages: Record<string, GroupPage | undefined> };
export type GroupDirectRelation = { key: string; review: DownloadOverlapReview; candidate: DownloadOverlapCandidate; evidence: string };
export type GroupSupplement = Record<string, DownloadOverlapDisplayMatches | undefined>;


export function visibleGroupRelations(group: OverlapReviewGroup, editions: readonly GroupEdition[]): GroupDirectRelation[] {
  const ids=new Set([group.keeper.entryId,...editions.filter(e=>e.keeperCompatible).map(e=>e.excluded.entryId)]);
  const byPair=new Map<string,GroupDirectRelation>();
  for(const relation of [...group.relations,...editions.filter(e=>e.keeperCompatible).map(e=>({key:e.key,review:e.review,candidate:e.candidate,evidence:e.evidence}))]) {
    const a=relation.review.incoming.entryId,b=relation.candidate.existing.entryId;
    if(ids.has(a)&&ids.has(b))byPair.set([a,b].sort().join("|"),relation);
  }
  return [...byPair.values()].sort((a,b)=>a.key.localeCompare(b.key));
}

export function relationDisplayPairs(relation: GroupDirectRelation, supplemental?: DownloadOverlapDisplayMatches): DownloadOverlapPagePair[] {
  const { review, candidate } = relation;
  if (!supplemental || supplemental.reviewId !== review.reviewId || supplemental.revision !== review.revision
    || supplemental.candidateId !== candidate.candidateId || supplemental.incomingFingerprint !== review.incomingFingerprint
    || supplemental.existingFingerprint !== candidate.existingFingerprint) return candidate.pagePairs;
  const left = new Set(candidate.pagePairs.map(p=>p.incomingSourcePage)), right = new Set(candidate.pagePairs.map(p=>p.existingSourcePage));
  const valid: DownloadOverlapPagePair[] = [];
  for (const p of supplemental.additionalPairs) {
    if (!Number.isSafeInteger(p.incomingSourcePage) || p.incomingSourcePage<1 || p.incomingSourcePage>review.incoming.pageCount
      || !Number.isSafeInteger(p.existingSourcePage) || p.existingSourcePage<1 || p.existingSourcePage>candidate.existing.pageCount
      || left.has(p.incomingSourcePage) || right.has(p.existingSourcePage) || p.lowInformation
      || !Number.isFinite(p.visualSimilarity) || (!p.exactSha256 && p.visualSimilarity<.94)) return candidate.pagePairs;
    left.add(p.incomingSourcePage);right.add(p.existingSourcePage);valid.push(p);
  }
  return [...candidate.pagePairs,...valid];
}

/** Each member must directly match the keeper; keeper-less columns require all-pair evidence.
 * An active merge target never decides the visual status of other editions. */
export function buildGroupAlignment(keeper: DownloadOverlapGalleryRef, editions: readonly GroupEdition[],
  relations: readonly GroupDirectRelation[] = [], supplements: GroupSupplement = {}): GroupAlignmentRow[] {
  type Node = { id: string; entry: string; page: number; gallery: number; edition?: GroupEdition };
  type Edge = { left: Node; right: Node; pair: DownloadOverlapPagePair; moved: boolean; primary: boolean };
  const nodes = new Map<string,Node>();
  const sourceEntries = new Map<string,DownloadOverlapGalleryRef>([[keeper.entryId,keeper]]);
  const byEntry = new Map<string,Node[]>();
  const make = (gallery: DownloadOverlapGalleryRef, edition?: GroupEdition) => {
    const pages = Array.from({length:gallery.pageCount},(_,i):Node=>({id:gallery.entryId+":"+ (i+1),entry:gallery.entryId,page:i+1,gallery:Number(gallery.galleryId),edition}));
    for(const n of pages)nodes.set(n.id,n);byEntry.set(gallery.entryId,pages);sourceEntries.set(gallery.entryId,gallery);
  };
  make(keeper);for(const e of editions)if(e.keeperCompatible)make(e.excluded,e);
  const direct = new Map<string,Edge>();
  const edgeKey = (a:string,b:string)=>a<b?a+"|"+b:b+"|"+a;
  const all = new Map(relations.map(r=>[r.key,r]));
  for(const e of editions)if(e.keeperCompatible)all.set(e.key,{key:e.key,review:e.review,candidate:e.candidate,evidence:e.evidence});
  for(const relation of all.values()) {
    const {review,candidate}=relation;
    if(!sourceEntries.has(review.incoming.entryId)||!sourceEntries.has(candidate.existing.entryId))continue;
    const original=new Set(candidate.pagePairs.map(p=>p.incomingSourcePage+":"+p.existingSourcePage));
    const pairs=relationDisplayPairs(relation,supplements[relation.key]);
    const seenLeft=new Set<number>(),seenRight=new Set<number>();
    for(const p of pairs) {
      if(seenLeft.has(p.incomingSourcePage)||seenRight.has(p.existingSourcePage))continue;
      const a=nodes.get(review.incoming.entryId+":"+p.incomingSourcePage),b=nodes.get(candidate.existing.entryId+":"+p.existingSourcePage);
      if(!a||!b||a.entry===b.entry)continue;
      seenLeft.add(a.page);seenRight.add(b.page);
      const moved=!original.has(p.incomingSourcePage+":"+p.existingSourcePage)
        && candidate.pagePairs.some(anchor=>(anchor.incomingSourcePage-p.incomingSourcePage)*(anchor.existingSourcePage-p.existingSourcePage)<0);
      const edge={left:a,right:b,pair:p,moved,primary:original.has(p.incomingSourcePage+":"+p.existingSourcePage)};
      const key=edgeKey(a.id,b.id),known=direct.get(key);
      if(!known||(!known.primary&&edge.primary))direct.set(key,edge);
    }
  }
  const parent=new Map([...nodes.keys()].map(id=>[id,id]));
  const members=new Map([...nodes.values()].map(n=>[n.id,[n]]));
  const root=(id:string):string=>{let r=id;while(parent.get(r)!==r)r=parent.get(r)!;return r;};
  const edges=[...direct.values()].sort((a,b)=>Number(b.primary)-Number(a.primary)
    || Number(b.left.entry===keeper.entryId||b.right.entry===keeper.entryId)-Number(a.left.entry===keeper.entryId||a.right.entry===keeper.entryId)
    || Number(b.pair.exactSha256)-Number(a.pair.exactSha256)||b.pair.visualSimilarity-a.pair.visualSimilarity||edgeKey(a.left.id,a.right.id).localeCompare(edgeKey(b.left.id,b.right.id)));
  for(const edge of edges) {
    const a=root(edge.left.id),b=root(edge.right.id);if(a===b)continue;
    const left=members.get(a)!,right=members.get(b)!;
    if(left.some(x=>right.some(y=>x.entry===y.entry)))continue;
    const combined=[...left,...right],base=combined.find(n=>n.entry===keeper.entryId);
    const proven=base ? combined.every(n=>n===base||direct.has(edgeKey(base.id,n.id)))
      : left.every(x=>right.every(y=>direct.has(edgeKey(x.id,y.id))));
    if(!proven)continue;
    parent.set(b,a);members.set(a,[...left,...right]);members.delete(b);
  }
  const support=new Map<string,{reordered:boolean;counterparts:Set<string>}>();
  for(const edge of direct.values())for(const [a,b] of [[edge.left,edge.right],[edge.right,edge.left]] as const) {
    const value=support.get(a.id)??{reordered:false,counterparts:new Set<string>()};
    value.reordered ||= edge.moved;value.counterparts.add("#"+b.gallery+" "+b.page+"p");support.set(a.id,value);
  }
  const originalPairsByEdition = new Map(editions.map(e => [e.key, new Map(e.candidate.pagePairs.map(p => [e.keeperIsIncoming ? p.existingSourcePage : p.incomingSourcePage, p]))]));
  const result=new Map<string,GroupAlignmentRow>();
  for(const [id,group] of members) {
    const base=group.find(n=>n.entry===keeper.entryId);
    const row:GroupAlignmentRow={key:base?"keeper:"+base.page:"extra:"+group.map(n=>n.id).sort()[0],keeperPage:base?.page,pages:{}};
    for(const n of group) {
      const s=support.get(n.id);
      if(n.entry===keeper.entryId){row.keeperMatched=Boolean(s);row.keeperReordered=s?.reordered;row.keeperCounterparts=s?[...s.counterparts].sort():[];}
      else {
        const e=n.edition!;
        const pair=originalPairsByEdition.get(e.key)?.get(n.page);
        row.pages[e.key]={page:n.page,pair,matched:Boolean(s),reordered:s?.reordered,counterparts:s?[...s.counterparts].sort():[]};
      }
    }
    result.set(id,row);
  }
  const before=new Map<string,number>();
  for(const pages of byEntry.values()) {
    let next=keeper.pageCount+1;
    for(let i=pages.length-1;i>=0;i--){const id=root(pages[i]!.id),row=result.get(id)!;
      if(row.keeperPage)next=row.keeperPage;else before.set(id,Math.min(before.get(id)??next,next));}
  }
  return [...result].sort(([a,x],[b,y])=>{
    const px=x.keeperPage??before.get(a)??keeper.pageCount+1,py=y.keeperPage??before.get(b)??keeper.pageCount+1;
    return px-py||Number(Boolean(x.keeperPage))-Number(Boolean(y.keeperPage))||x.key.localeCompare(y.key,undefined,{numeric:true});
  }).map(([,row])=>row);
}

export const groupEditionStatus: Record<GroupEdition["status"], string> = {
  contained: "완전 포함", uncertain: "검토 필요", excluded: "제외 완료",
  kept: "둘 다 보존", different: "다른 판본", stale: "다시 확인",
};
