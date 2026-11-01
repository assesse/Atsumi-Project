use super::*;
use crate::domain::{
    DownloadOverlapDisplayMatchRequest, DownloadOverlapDisplayMatches, DownloadOverlapGalleryRef,
    DownloadOverlapPagePair, DuplicatePageHash,
};
use super::super::duplicate_analyzer::compare_page_evidence;

const MAX_PAGES: usize = 4_000;
const MAX_UNMATCHED_COMPARISONS: usize = 4_000_000;
fn unavailable(message: &str) -> ApplicationError {
    ApplicationError::DownloadOverlapDecisionInvalid(message.into())
}

/// Display-only evidence: never reconcile/requeue, write hashes, decode files or
/// replace the durable monotonic evidence used for exclusion and page merging.
impl DownloadSupervisor {
    pub fn overlap_display_matches(
        &self, request: DownloadOverlapDisplayMatchRequest,
    ) -> Result<DownloadOverlapDisplayMatches, ApplicationError> {
        if request.review_id.is_empty() || request.review_id.len() > 200
            || request.candidate_id.is_empty() || request.candidate_id.len() > 200 {
            return Err(unavailable("비교 요청이 유효하지 않습니다."));
        }
        let review = self.inner.repository.overlap_review_get(&request.review_id)?
            .ok_or_else(|| ApplicationError::DownloadOverlapReviewNotFound(request.review_id.clone()))?;
        if review.revision != request.expected_revision {
            return Err(ApplicationError::RevisionConflict { resource: "downloadOverlapReview",
                expected: request.expected_revision, actual: review.revision });
        }
        if review.state == DownloadOverlapReviewState::Stale
            || review.profile_version != HashProfile::current().profile_version
            || review.policy_version != DOWNLOAD_OVERLAP_POLICY_VERSION {
            return Err(unavailable("페이지 비교 근거를 다시 확인해 주세요."));
        }
        let candidate = review.candidates.iter().find(|c| c.candidate_id == request.candidate_id)
            .ok_or_else(|| unavailable("비교할 판본이 변경되었습니다."))?;
        let incoming = cached_pages(self.inner.repository.as_ref(), &review.incoming,
            &review.incoming_fingerprint, review.profile_version)?;
        let existing = cached_pages(self.inner.repository.as_ref(), &candidate.existing,
            &candidate.existing_fingerprint, review.profile_version)?;
        let additional_pairs = recover_unordered_pairs(&incoming, &existing, &candidate.page_pairs)?;
        // Reject evidence if a decision or artifact revision changed during the read.
        let after = self.inner.repository.overlap_review_get(&request.review_id)?
            .ok_or_else(|| unavailable("비교할 판본이 변경되었습니다."))?;
        if after != review { return Err(unavailable("페이지 비교 근거가 변경되었습니다. 다시 열어 주세요.")); }
        verify_bundle(self.inner.repository.as_ref(), &review.incoming, &review.incoming_fingerprint, review.profile_version)?;
        verify_bundle(self.inner.repository.as_ref(), &candidate.existing, &candidate.existing_fingerprint, review.profile_version)?;
        Ok(DownloadOverlapDisplayMatches { review_id: review.review_id, revision: review.revision,
            candidate_id: candidate.candidate_id.clone(), incoming_fingerprint: review.incoming_fingerprint,
            existing_fingerprint: candidate.existing_fingerprint.clone(), additional_pairs })
    }
}

fn verify_bundle(repository: &dyn DownloadOverlapRepository, gallery: &DownloadOverlapGalleryRef,
    fingerprint: &str, profile: u32) -> Result<crate::domain::ArtifactBundle, ApplicationError> {
    if gallery.page_count == 0 || gallery.page_count as usize > MAX_PAGES {
        return Err(unavailable("이 판본의 추가 대응 확인 범위를 초과했습니다."));
    }
    let bundle = repository.pipeline_artifact_bundle(&DownloadEntryId::new(gallery.entry_id.clone())?)?
        .ok_or_else(|| unavailable("페이지 비교 원본을 찾을 수 없습니다."))?;
    if bundle.gallery.id != gallery.gallery_id || bundle.artifact.expected_page_count != gallery.page_count
        || overlap_artifact_fingerprint(&bundle, profile).as_deref() != Some(fingerprint) {
        return Err(unavailable("원본 페이지가 변경되어 추가 대응을 확인할 수 없습니다."));
    }
    Ok(bundle)
}
fn cached_pages(repository: &dyn DownloadOverlapRepository, gallery: &DownloadOverlapGalleryRef,
    fingerprint: &str, profile: u32) -> Result<Vec<DuplicatePageHash>, ApplicationError> {
    let bundle = verify_bundle(repository, gallery, fingerprint, profile)?;
    let pages = verified_overlap_pages(&bundle).ok_or_else(|| unavailable("검증된 페이지 근거가 없습니다."))?;
    let mut seen = HashSet::new();
    let mut hashes = Vec::with_capacity(pages.len());
    for page in pages {
        let number = page.page_id.source_page_number;
        if !seen.insert(number) || number.get() > gallery.page_count { return Err(unavailable("페이지 구성이 변경되었습니다.")); }
        let sha = page.sha256.as_ref().ok_or_else(|| unavailable("페이지 지문이 없습니다."))?;
        let cached = repository.overlap_page_hash_get(&gallery.entry_id, number, profile, sha.as_str())?
            .ok_or_else(|| unavailable("저장된 페이지 지문이 없어 추가 대응을 확인할 수 없습니다."))?;
        if cached.entry_id != gallery.entry_id || cached.gallery_id != gallery.gallery_id
            || cached.source_page_number != number || cached.profile_version != profile
            || cached.artifact_sha256 != *sha {
            return Err(unavailable("저장된 페이지 지문이 원본과 다릅니다."));
        }
        hashes.push(cached);
    }
    hashes.sort_by_key(|p| p.source_page_number);
    Ok(hashes)
}

fn recover_unordered_pairs(incoming: &[DuplicatePageHash], existing: &[DuplicatePageHash],
    anchors: &[DownloadOverlapPagePair]) -> Result<Vec<DownloadOverlapPagePair>, ApplicationError> {
    let mut used_incoming = HashSet::new(); let mut used_existing = HashSet::new();
    for p in anchors {
        if !used_incoming.insert(p.incoming_source_page) || !used_existing.insert(p.existing_source_page)
            || !incoming.iter().any(|h| h.source_page_number.get() == p.incoming_source_page)
            || !existing.iter().any(|h| h.source_page_number.get() == p.existing_source_page) {
            return Err(unavailable("기존 페이지 대응이 유효하지 않습니다."));
        }
    }
    let left: Vec<_> = incoming.iter().filter(|p| !used_incoming.contains(&p.source_page_number.get())).collect();
    let right: Vec<_> = existing.iter().filter(|p| !used_existing.contains(&p.source_page_number.get())).collect();
    if left.len().saturating_mul(right.len()) > MAX_UNMATCHED_COMPARISONS {
        return Err(unavailable("추가 대응 확인 범위를 초과했습니다."));
    }
    let _permit = super::super::image_work_budget::acquire(None);
    let profile = HashProfile::current();
    let mut candidates = Vec::new();
    for (i, a) in left.iter().enumerate() { for (j, b) in right.iter().enumerate() {
        if a.low_information || b.low_information { continue; }
        if let Some(p) = compare_page_evidence(a, b, &profile) { candidates.push((i, j, p)); }
    }}
    let choose = |indexes: Vec<usize>| -> Option<usize> {
        let mut sorted = indexes;
        sorted.sort_by(|a,b| candidates[*b].2.exact_sha256.cmp(&candidates[*a].2.exact_sha256)
            .then_with(|| candidates[*b].2.visual_similarity.total_cmp(&candidates[*a].2.visual_similarity)));
        let first = *sorted.first()?; let best = &candidates[first].2;
        if !best.exact_sha256 && best.visual_similarity < 0.94 { return None; }
        if let Some(second) = sorted.get(1).map(|i| &candidates[*i].2) {
            if (best.exact_sha256 && second.exact_sha256)
                || (!best.exact_sha256 && best.visual_similarity - second.visual_similarity < 0.03) { return None; }
        }
        Some(first)
    };
    let mut by_left = vec![Vec::new(); left.len()]; let mut by_right = vec![Vec::new(); right.len()];
    for (n,(i,j,_)) in candidates.iter().enumerate() { by_left[*i].push(n); by_right[*j].push(n); }
    let best_left: Vec<_> = by_left.into_iter().map(&choose).collect();
    let best_right: Vec<_> = by_right.into_iter().map(choose).collect();
    let mut pairs = Vec::new();
    for (i, selected) in best_left.iter().enumerate() {
        let Some(n) = selected else { continue; }; let (_,j,p) = &candidates[*n];
        if best_right[*j] == Some(*n) {
            debug_assert_eq!(candidates[*n].0, i);
            pairs.push(DownloadOverlapPagePair { incoming_source_page:p.parent_source_page,
                existing_source_page:p.candidate_source_page, exact_sha256:p.exact_sha256,
                d_hash_distance:p.d_hash_distance,p_hash_distance:p.p_hash_distance,
                detail_hash_distance:p.detail_hash_distance,edge_similarity:p.edge_similarity,
                visual_similarity:p.visual_similarity,low_information:p.low_information });
        }
    }
    pairs.sort_by_key(|p| p.incoming_source_page);
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ArtifactSha256, GalleryId, SourcePageNumber};

    fn page(gallery: i64, number: u32, scene: u64) -> DuplicatePageHash {
        let mut random = scene.wrapping_add(1).wrapping_mul(0x9e3779b97f4a7c15);
        let mut bytes = [0u8;128];
        for byte in &mut bytes { random ^= random << 13; random ^= random >> 7; random ^= random << 17; *byte = random as u8; }
        DuplicatePageHash { entry_id:format!("entry-{gallery}"), gallery_id:GalleryId::new(gallery).unwrap(),
            source_page_number:SourcePageNumber::new(number).unwrap(),profile_version:1,
            artifact_sha256:ArtifactSha256::new(format!("{scene:064x}")).unwrap(),
            coarse_d_hash:u64::from_le_bytes(bytes[0..8].try_into().unwrap()),p_hash:u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            detail_d_hash_hex:bytes.iter().map(|b|format!("{b:02x}")).collect(),mean_luma:127.,std_dev:40.,
            non_uniform_ratio:0.7,edge_density:0.2,width:1000,height:1414,low_information:false }
    }
    fn anchor(a:u32,b:u32)->DownloadOverlapPagePair { DownloadOverlapPagePair {
        incoming_source_page:a,existing_source_page:b,exact_sha256:true,d_hash_distance:0,
        p_hash_distance:0,detail_hash_distance:0,edge_similarity:1.,visual_similarity:1.,low_information:false
    }}
    #[test]
    fn restores_moved_page_without_replacing_monotonic_anchors() {
        let a=vec![page(1,1,1),page(1,2,2),page(1,3,3)];
        let b=vec![page(2,1,3),page(2,2,1),page(2,3,2)];
        let old=vec![anchor(1,2),anchor(2,3)];let snapshot=old.clone();
        let recovered=recover_unordered_pairs(&a,&b,&old).unwrap();
        assert_eq!(old,snapshot);assert_eq!(recovered.len(),1);
        assert_eq!((recovered[0].incoming_source_page,recovered[0].existing_source_page),(3,1));
    }
    #[test]
    fn repeated_or_blank_pages_are_not_guessed() {
        let a=vec![page(1,1,3)];let b=vec![page(2,1,3),page(2,2,3)];
        assert!(recover_unordered_pairs(&a,&b,&[]).unwrap().is_empty());
        assert!(recover_unordered_pairs(&b,&a,&[]).unwrap().is_empty());
        let mut blank=page(2,1,3);blank.low_information=true;
        assert!(recover_unordered_pairs(&a,&[blank],&[]).unwrap().is_empty());
    }
    #[test]
    fn strong_reencoded_match_is_retained_but_already_used_page_is_not_reused() {
        let a=page(1,1,7);let mut b=page(2,1,7);
        b.artifact_sha256=ArtifactSha256::new(format!("{:064x}",70)).unwrap();b.coarse_d_hash^=1;
        let pairs=recover_unordered_pairs(&[a.clone()],&[b.clone()],&[]).unwrap();
        assert_eq!(pairs.len(),1);assert!(!pairs[0].exact_sha256);
        assert!(recover_unordered_pairs(&[a],&[b],&[anchor(1,1)]).unwrap().is_empty());
    }
    #[test]
    fn invalid_anchor_and_work_budget_fail_closed() {
        let a=vec![page(1,1,3)];let b=vec![page(2,1,3)];
        assert!(recover_unordered_pairs(&a,&b,&[anchor(1,1),anchor(1,1)]).is_err());
        assert!(recover_unordered_pairs(&a,&b,&[anchor(1,2)]).is_err());
        let big:Vec<_>=(1..=2001).map(|n|page(1,n,u64::from(n))).collect();
        assert!(recover_unordered_pairs(&big,&big,&[]).is_err());
    }
    #[test]
    fn a_similar_runner_up_stays_ambiguous_even_if_both_pass_strict_matching() {
        let a=page(1,1,7);let mut b=page(2,1,7);let mut c=page(2,2,7);
        b.artifact_sha256=ArtifactSha256::new(format!("{:064x}",70)).unwrap();
        c.artifact_sha256=ArtifactSha256::new(format!("{:064x}",71)).unwrap();c.coarse_d_hash^=1;
        assert!(recover_unordered_pairs(&[a],&[b,c],&[]).unwrap().is_empty());
    }

    #[test]
    #[ignore = "explicit read-only local regression; never used by normal CI"]
    fn recorded_user_pages_readonly() {
        let path=std::env::var("ATSUMI_OVERLAP_PROBE_DB").expect("explicit read-only database path");
        let db=rusqlite::Connection::open_with_flags(path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        db.execute_batch("PRAGMA query_only=ON; PRAGMA busy_timeout=3000;").unwrap();
        let read=|id:i64|->Vec<DuplicatePageHash>{
            db.prepare("SELECT entry_id,gallery_id,source_page_number,profile_version,artifact_sha256,coarse_d_hash_hex,detail_d_hash_hex,p_hash_hex,mean_luma,std_dev,non_uniform_ratio,edge_density,width,height,low_information FROM duplicate_page_hashes WHERE gallery_id=?1 AND profile_version=1 ORDER BY source_page_number").unwrap()
              .query_map([id],|r|Ok(DuplicatePageHash {entry_id:r.get(0)?,gallery_id:GalleryId::new(r.get(1)?).unwrap(),
                source_page_number:SourcePageNumber::new(r.get(2)?).unwrap(),profile_version:r.get(3)?,
                artifact_sha256:ArtifactSha256::new(r.get::<_,String>(4)?).unwrap(),
                coarse_d_hash:u64::from_str_radix(&r.get::<_,String>(5)?,16).unwrap(),detail_d_hash_hex:r.get(6)?,
                p_hash:u64::from_str_radix(&r.get::<_,String>(7)?,16).unwrap(),mean_luma:r.get(8)?,std_dev:r.get(9)?,
                non_uniform_ratio:r.get(10)?,edge_density:r.get(11)?,width:r.get(12)?,height:r.get(13)?,low_information:r.get(14)?}))
              .unwrap().collect::<Result<_,_>>().unwrap()
        };
        let mut results=Vec::new();
        for incoming in [3591810,2540189] {
            let candidate:String=db.query_row("SELECT c.candidate_id FROM download_overlap_candidates c JOIN download_overlap_reviews r USING(review_id) WHERE r.incoming_gallery_id=?1 AND c.existing_gallery_id=3344963 LIMIT 1",[incoming],|r|r.get(0)).unwrap();
            let anchors:Vec<_>=db.prepare("SELECT incoming_source_page,existing_source_page,exact_sha256,d_hash_distance,p_hash_distance,detail_hash_distance,edge_similarity,visual_similarity,low_information FROM download_overlap_page_pairs WHERE candidate_id=?1 ORDER BY pair_index").unwrap().query_map([candidate],|r|Ok(DownloadOverlapPagePair {
                incoming_source_page:r.get(0)?,existing_source_page:r.get(1)?,exact_sha256:r.get(2)?,
                d_hash_distance:r.get(3)?,p_hash_distance:r.get(4)?,detail_hash_distance:r.get(5)?,
                edge_similarity:r.get(6)?,visual_similarity:r.get(7)?,low_information:r.get(8)?,
            })).unwrap().collect::<Result<_,_>>().unwrap();
            let recovered=recover_unordered_pairs(&read(incoming),&read(3344963),&anchors).unwrap();
            for (a,b) in [(9,8),(58,61)] { assert!(recovered.iter().any(|p|p.incoming_source_page==a&&p.existing_source_page==b),"{incoming}: {a}->{b}"); }
            if incoming==2540189 { assert!(anchors.iter().any(|p|p.incoming_source_page==1&&p.existing_source_page==2)); }
            else { for a in 83..=97 {assert!(anchors.iter().any(|p|p.incoming_source_page==a&&p.existing_source_page==a+1));} }
            results.push(serde_json::json!({"incoming":incoming,"existing":3344963,"additionalPairs":recovered}));
        }
        assert_eq!(db.total_changes(),0);
        println!("READONLY_PAGE_MATCH_PROBE={}",serde_json::to_string(&results).unwrap());
    }
}
