import type { DownloadOverlapReview } from '../api/contracts';
import { galleryId } from '../core/types';

export const pairFixture = (id = 'pair'): DownloadOverlapReview => ({
  reviewId:id, entryId:id+'-incoming', revision:4, state:'pending', profileVersion:1, policyVersion:1,
  incomingFingerprint:id+'-incoming-fp', createdAt:'now', updatedAt:'now',
  incoming:{entryId:id+'-incoming',galleryId:galleryId(200),title:'New edition',artists:['fixture'],pageCount:6},
  candidates:[{
    candidateId:id+'-candidate', existing:{entryId:id+'-existing',galleryId:galleryId(100),title:'Owned edition',artists:['fixture'],pageCount:4},
    existingFingerprint:id+'-existing-fp', relation:'incoming_contains_existing',confidence:1,matchedPages:4,exactPages:4,visualPages:0,
    existingCoverage:1,incomingCoverage:4/6,existingUniquePages:0,incomingUniquePages:2,longestAlignedRun:4,rank:1,
    pagePairs:Array.from({length:4},(_,i)=>({incomingSourcePage:i+1,existingSourcePage:i+1,exactSha256:true,dHashDistance:0,pHashDistance:0,detailHashDistance:0,edgeSimilarity:1,visualSimilarity:1,lowInformation:false})),
  }],
});
