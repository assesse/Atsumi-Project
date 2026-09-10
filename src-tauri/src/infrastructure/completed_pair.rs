//! Completed-library pairs share overlap evidence and merge journals without
//! ever converting a completed download into a queued/review-required job.
use super::*;
use crate::application::DuplicateRepository;
use crate::domain::{
    DownloadOverlapDecisionAction as Action, DownloadOverlapDecisionActor,
    DownloadOverlapDecisionRequest, DownloadOverlapDecisionResult, DuplicateRelation,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

pub(super) fn check_origin(
    connection: &Connection,
    review_id: &str,
) -> Result<Option<(String, u64)>, ApplicationError> {
    let Some(encoded) = review_id.strip_prefix("completed-pair:") else {
        return Ok(None);
    };
    let mut parts = encoded.split(':');
    let candidate = String::from_utf8(
        URL_SAFE_NO_PAD
            .decode(parts.next().unwrap_or(""))
            .map_err(|_| invalid("Invalid completed-pair identity"))?,
    )
    .map_err(|_| invalid("Invalid completed-pair identity"))?;
    let expected: u64 = parts
        .next()
        .unwrap_or("")
        .parse()
        .map_err(|_| invalid("Invalid completed-pair revision"))?;
    let row:Option<(u64,bool,bool)>=connection.query_row("SELECT revision,resolved,artifact_stale FROM duplicate_candidates WHERE candidate_id=?1",[&candidate],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
    let Some((revision, resolved, stale)) = row else {
        return Err(invalid("The source comparison no longer exists"));
    };
    if revision != expected {
        return Err(ApplicationError::RevisionConflict {
            resource: "duplicateCandidate",
            expected,
            actual: revision,
        });
    }
    if resolved || stale {
        return Err(invalid("This comparison was already processed or its pages changed; compare the two albums again"));
    }
    Ok(Some((candidate, expected)))
}

impl OverlapMergeService {
    pub fn prepare_completed_decision(
        &self,
        mut request: DownloadOverlapDecisionRequest,
    ) -> Result<DownloadOverlapDecisionRequest, ApplicationError> {
        if request.actor != DownloadOverlapDecisionActor::Human
            || request.candidate_id.as_deref() != request.review_id.strip_prefix("duplicate:")
        {
            return Err(invalid(
                "Select the current completed pair for an explicit human decision",
            ));
        }
        let review = self.prepare_completed_pair(&request.review_id, request.expected_revision)?;
        request.review_id = review.review_id;
        request.expected_revision = review.revision;
        request.candidate_id = Some(review.candidates[0].candidate_id.clone());
        Ok(request)
    }

    pub fn prepare_completed_merge(
        &self,
        mut request: DownloadOverlapMergeRequest,
    ) -> Result<DownloadOverlapMergeRequest, ApplicationError> {
        if !request.review_id.starts_with("duplicate:") {
            return Ok(request);
        }
        if Some(request.candidate_id.as_str()) != request.review_id.strip_prefix("duplicate:") {
            return Err(invalid("Select the current completed pair before merging"));
        }
        let review = self.prepare_completed_pair(&request.review_id, request.expected_revision)?;
        request.review_id = review.review_id;
        request.expected_revision = review.revision;
        request.candidate_id = review.candidates[0].candidate_id.clone();
        Ok(request)
    }
    /// Materialize an explicit user action, not a new download or automatic job.
    pub fn prepare_completed_pair(
        &self,
        virtual_id: &str,
        expected: u64,
    ) -> Result<DownloadOverlapReview, ApplicationError> {
        let candidate_id = virtual_id
            .strip_prefix("duplicate:")
            .ok_or_else(|| invalid("Not a completed-pair comparison"))?;
        let old = self
            .repository
            .duplicate_review_get(candidate_id)?
            .ok_or_else(|| ApplicationError::DuplicateCandidateNotFound(candidate_id.into()))?;
        if old.candidate.revision != expected {
            return Err(ApplicationError::RevisionConflict {
                resource: "duplicateCandidate",
                expected,
                actual: old.candidate.revision,
            });
        }
        let pair = &old.candidate;
        let available = self
            .repository
            .duplicate_selected_bundles(&[pair.parent.gallery_id, pair.candidate.gallery_id])?;
        let existing = available
            .iter()
            .find(|b| b.artifact.entry_id.as_str() == pair.parent.entry_id)
            .ok_or_else(|| invalid("Album A must still be a completed, visible album"))?;
        let incoming = available
            .iter()
            .find(|b| b.artifact.entry_id.as_str() == pair.candidate.entry_id)
            .ok_or_else(|| invalid("Album B must still be a completed, visible album"))?;
        if old.page_pairs.is_empty()
            || existing.artifact.expected_page_count != pair.parent.page_count
            || incoming.artifact.expected_page_count != pair.candidate.page_count
        {
            return Err(invalid(
                "The stored page correspondence changed; compare the two albums again",
            ));
        }
        let profile = crate::domain::HashProfile::current().profile_version;
        let fingerprint = |bundle: &ArtifactBundle| {
            overlap_artifact_fingerprint(bundle, profile)
                .ok_or_else(|| invalid("The completed album has invalid page checkpoints"))
        };
        let incoming_fingerprint = fingerprint(incoming)?;
        let existing_fingerprint = fingerprint(existing)?;
        let review_id = format!(
            "completed-pair:{}:{expected}:{}",
            URL_SAFE_NO_PAD.encode(candidate_id),
            Uuid::new_v4()
        );
        let overlap_candidate_id = format!("{review_id}:A");
        let mut connection = self.repository.connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        check_origin(&tx, &review_id)?;
        let relation = match pair.relation {
            DuplicateRelation::Exact => "near_equivalent",
            DuplicateRelation::Contains if pair.parent.page_count >= pair.candidate.page_count => {
                "existing_contains_incoming"
            }
            DuplicateRelation::Contains => "incoming_contains_existing",
            DuplicateRelation::Partial => "partial_overlap",
            DuplicateRelation::TranslationVisual => "translation_edition",
        };
        let exact = old.page_pairs.iter().filter(|p| p.exact_sha256).count();
        let mut longest = 0;
        let mut run = 0;
        let mut previous = None;
        for page in &old.page_pairs {
            let coordinates = (page.parent_source_page, page.candidate_source_page);
            if coordinates.0 == 0
                || coordinates.0 > pair.parent.page_count
                || coordinates.1 == 0
                || coordinates.1 > pair.candidate.page_count
            {
                return Err(invalid(
                    "A stored page number is outside the completed album",
                ));
            }
            run = if previous
                .is_some_and(|p: (u32, u32)| p.0 + 1 == coordinates.0 && p.1 + 1 == coordinates.1)
            {
                run + 1
            } else {
                1
            };
            longest = longest.max(run);
            previous = Some(coordinates);
        }
        tx.execute("INSERT INTO download_overlap_reviews(review_id,entry_id,incoming_gallery_id,revision,state,profile_version,policy_version,incoming_fingerprint,created_at,updated_at) VALUES(?1,?2,?3,0,'pending',?4,2,?5,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![review_id,pair.candidate.entry_id,pair.candidate.gallery_id.get(),profile,incoming_fingerprint]).map_err(sql)?;
        tx.execute("INSERT INTO download_overlap_candidates(candidate_id,review_id,existing_entry_id,existing_gallery_id,existing_fingerprint,relation,confidence,matched_pages,exact_pages,visual_pages,existing_coverage,incoming_coverage,existing_unique_pages,incoming_unique_pages,longest_aligned_run,rank) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,1)",params![overlap_candidate_id,review_id,pair.parent.entry_id,pair.parent.gallery_id.get(),existing_fingerprint,relation,pair.confidence,pair.matched_pages,exact as u32,pair.matched_pages.saturating_sub(exact as u32),pair.parent_coverage,pair.candidate_coverage,pair.parent.page_count.saturating_sub(pair.matched_pages),pair.candidate.page_count.saturating_sub(pair.matched_pages),longest]).map_err(sql)?;
        for (index, p) in old.page_pairs.iter().enumerate() {
            tx.execute("INSERT INTO download_overlap_page_pairs(candidate_id,pair_index,incoming_source_page,existing_source_page,exact_sha256,d_hash_distance,p_hash_distance,detail_hash_distance,edge_similarity,visual_similarity,low_information) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![overlap_candidate_id,index as u32,p.candidate_source_page,p.parent_source_page,p.exact_sha256,p.d_hash_distance,p.p_hash_distance,p.detail_hash_distance,p.edge_similarity,p.visual_similarity,p.low_information]).map_err(sql)?;
        }
        // No download_entries.review_id/state update: startup/automation inventory
        // only follows linked review-required jobs, never these completed pairs.
        tx.commit().map_err(sql)?;
        drop(connection);
        self.repository
            .overlap_review_get(&review_id)?
            .ok_or_else(|| invalid("Completed comparison disappeared"))
    }

    pub fn decide_completed_pair(
        &self,
        request: DownloadOverlapDecisionRequest,
    ) -> Result<DownloadOverlapDecisionResult, ApplicationError> {
        if request.actor != DownloadOverlapDecisionActor::Human {
            return Err(invalid(
                "Completed-pair comparisons require an explicit user decision",
            ));
        }
        let review = self
            .repository
            .overlap_review_get(&request.review_id)?
            .ok_or_else(|| invalid("Completed comparison not found"))?;
        if review.revision != request.expected_revision
            || review.state != DownloadOverlapReviewState::Pending
        {
            return Err(invalid("Completed comparison has changed"));
        }
        let candidate = review
            .candidates
            .first()
            .ok_or_else(|| invalid("Completed comparison has no candidate"))?;
        if request.candidate_id.as_deref() != Some(candidate.candidate_id.as_str()) {
            return Err(invalid("The selected completed-pair candidate changed"));
        }
        let mut verified_revisions = Vec::new();
        for (id, expected) in [
            (&review.entry_id, &review.incoming_fingerprint),
            (
                &candidate.existing.entry_id,
                &candidate.existing_fingerprint,
            ),
        ] {
            let bundle = self.bundle(id)?;
            verified_revisions.push((id.clone(), bundle.artifact.revision));
            if overlap_artifact_fingerprint(&bundle, review.profile_version).as_ref()
                != Some(expected)
            {
                return Err(invalid("The album changed; compare the two albums again"));
            }
            // A decision moves no page bytes and deletes nothing. Do not rehash
            // entire completed albums merely to retain/exclude them. The merge
            // path still verifies bytes before copying and committing.
            let root = self
                .repository
                .pipeline_artifact_root(&bundle.artifact.entry_id)?;
            let manifest = bundle
                .artifact
                .manifest_relative_path
                .as_ref()
                .ok_or_else(|| invalid("Completed album manifest is missing"))?;
            let actual: ArtifactManifest = serde_json::from_reader(
                File::open(checked_path(&root, manifest.as_str())?).map_err(io)?,
            )
            .map_err(serialization)?;
            if actual != ArtifactManifest::from_bundle(&bundle)? {
                return Err(invalid("The completed album manifest changed"));
            }
            for p in &bundle.pages {
                let metadata =
                    fs::metadata(checked_path(&root, p.relative_path.as_str())?).map_err(io)?;
                if !metadata.is_file() || Some(metadata.len()) != p.byte_length {
                    return Err(invalid("A completed page is missing or its size changed"));
                }
            }
        }
        let mut connection = self.repository.connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let (classic_id, revision) = check_origin(&tx, &review.review_id)?
            .ok_or_else(|| invalid("Not a completed-pair comparison"))?;
        let current: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM download_overlap_reviews WHERE review_id=?1 AND revision=?2 AND state='pending')", params![review.review_id, request.expected_revision], |r| r.get(0)).map_err(sql)?;
        if !current {
            return Err(invalid("The completed comparison changed before saving"));
        }
        for (id, revision) in verified_revisions {
            let same: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM download_artifacts WHERE entry_id=?1 AND revision=?2)",params![id,revision],|r|r.get(0)).map_err(sql)?;
            if !same {
                return Err(invalid("The album changed while checking the decision"));
            }
        }
        let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM quarantine_records WHERE entry_id IN(?1,?2) AND state IN('pending_quarantine','pending_restore')) OR EXISTS(SELECT 1 FROM page_quarantine_records WHERE entry_id IN(?1,?2) AND state<>'restored') OR EXISTS(SELECT 1 FROM excluded_artifact_relocations WHERE entry_id IN(?1,?2) AND state<>'restored') OR EXISTS(SELECT 1 FROM internal_removal_plans WHERE entry_id IN(?1,?2) AND state='applying')",params![review.entry_id,candidate.existing.entry_id],|r|r.get(0)).map_err(sql)?;
        if active {
            return Err(invalid(
                "Wait for album restoration or page maintenance to finish",
            ));
        }
        let live:bool=tx.query_row("SELECT count(*)=2 FROM download_entries e JOIN download_artifacts a ON a.entry_id=e.entry_id WHERE e.entry_id IN(?1,?2) AND e.state='completed' AND a.state='complete' AND NOT EXISTS(SELECT 1 FROM duplicate_hidden_galleries h WHERE h.gallery_id=e.gallery_id) AND NOT EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state NOT IN('applied','rolled_back') AND e.entry_id IN(m.source_entry_id,m.target_entry_id))",params![review.entry_id,candidate.existing.entry_id],|r|r.get(0)).map_err(sql)?;
        if !live {
            return Err(invalid(
                "Wait for active album work to finish or reload this comparison",
            ));
        }
        let decision_id = format!("completed-decision-{}", Uuid::new_v4());
        let (legacy_action, removed) = match request.action {
            Action::RemoveExistingContinue => ("hide_parent", Some(&candidate.existing)),
            Action::RemoveIncoming => ("hide_candidate", Some(&review.incoming)),
            Action::KeepBothContinue | Action::FalsePositiveContinue => ("exclude_pair", None),
        };
        if let Some(removed) = removed {
            tx.execute("INSERT INTO duplicate_hidden_galleries(gallery_id,decision_id,created_at) VALUES(?1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(gallery_id) DO UPDATE SET decision_id=excluded.decision_id,created_at=excluded.created_at",params![removed.gallery_id.get(),decision_id]).map_err(sql)?;
            tx.execute(
                "DELETE FROM exploration_restored_galleries WHERE gallery_id=?1",
                [removed.gallery_id.get()],
            )
            .map_err(sql)?;
            tx.execute("UPDATE download_entries SET state='cancelled',revision=revision+1,review_id=NULL,review_kind=NULL,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE entry_id=?1",[&removed.entry_id]).map_err(sql)?;
            tx.execute("UPDATE download_jobs SET state='cancelled',revision=revision+1,finished_at=COALESCE(finished_at,strftime('%Y-%m-%dT%H:%M:%fZ','now')),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE entry_id=?1",[&removed.entry_id]).map_err(sql)?;
            tx.execute("UPDATE download_attempts SET outcome_state='cancelled',finished_at=COALESCE(finished_at,strftime('%Y-%m-%dT%H:%M:%fZ','now')) WHERE EXISTS(SELECT 1 FROM download_jobs j WHERE j.entry_id=?1 AND j.job_id=download_attempts.job_id AND j.attempt=download_attempts.attempt)", [&removed.entry_id]).map_err(sql)?;
        } else {
            tx.execute("INSERT INTO duplicate_pair_exclusions(parent_gallery_id,candidate_gallery_id,decision_id,created_at) VALUES(?1,?2,?3,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(parent_gallery_id,candidate_gallery_id) DO UPDATE SET decision_id=excluded.decision_id,created_at=excluded.created_at",params![candidate.existing.gallery_id.get(),review.incoming.gallery_id.get(),decision_id]).map_err(sql)?;
            let (left, right) = if review.incoming_fingerprint < candidate.existing_fingerprint {
                (
                    &review.incoming_fingerprint,
                    &candidate.existing_fingerprint,
                )
            } else {
                (
                    &candidate.existing_fingerprint,
                    &review.incoming_fingerprint,
                )
            };
            tx.execute("INSERT OR REPLACE INTO download_overlap_pair_policies(left_fingerprint,right_fingerprint,profile_version,policy_version,decision,created_at) VALUES(?1,?2,?3,?4,?5,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![left,right,review.profile_version,review.policy_version,if request.action==Action::KeepBothContinue {"keep_both"}else{"false_positive"}]).map_err(sql)?;
        }
        tx.execute("UPDATE duplicate_candidates SET revision=revision+1,resolved=1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE candidate_id=?1 AND revision=?2",params![classic_id,revision]).map_err(sql)?;
        tx.execute("INSERT INTO duplicate_decisions(decision_id,candidate_id,candidate_revision,action,target_gallery_id,created_at) VALUES(?1,?2,?3,?4,?5,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![decision_id,classic_id,revision+1,legacy_action,removed.map(|g|g.gallery_id.get())]).map_err(sql)?;
        // Preserve the distinct overlap ground-truth label, not just legacy exclude_pair.
        tx.execute("INSERT INTO download_overlap_decisions(decision_id,review_id,review_revision,candidate_id,action,actor,created_at) VALUES(?1,?2,1,?3,?4,'human',strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![decision_id,review.review_id,candidate.candidate_id,request.action.as_str()]).map_err(sql)?;
        tx.execute(
            "UPDATE download_overlap_candidates SET decision=?2 WHERE candidate_id=?1",
            params![
                candidate.candidate_id,
                match request.action {
                    Action::KeepBothContinue => Some("keep_both"),
                    Action::FalsePositiveContinue => Some("false_positive"),
                    Action::RemoveExistingContinue => Some("existing_removed"),
                    _ => None,
                }
            ],
        )
        .map_err(sql)?;
        tx.execute("UPDATE download_overlap_reviews SET revision=revision+1,state=?2,resolved_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE review_id=?1",params![review.review_id,if request.action==Action::RemoveIncoming {"cancelled"}else{"resolved"}]).map_err(sql)?;
        tx.commit().map_err(sql)?;
        drop(connection);
        Ok(DownloadOverlapDecisionResult {
            review: self
                .repository
                .overlap_review_get(&review.review_id)?
                .ok_or_else(|| invalid("Completed comparison disappeared after saving"))?,
            resumed: false,
            cancelled: false,
        })
    }
}
