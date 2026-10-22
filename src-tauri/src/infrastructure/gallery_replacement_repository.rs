//! Metadata-only alias settlement. Never moves files or changes a target worker.
use super::*;
use crate::{application::DownloadGalleryLink, source::hitomi::GalleryReplacementProof};

fn conflict(message: &str) -> RepositoryError {
    RepositoryError::GalleryReplacementConflict(message.to_owned())
}

pub(super) fn link(
    repository: &SqliteRepository,
    descriptor: &DownloadJobDescriptor,
    proof: &GalleryReplacementProof,
) -> Result<DownloadGalleryLink, RepositoryError> {
    if proof.requested_id() != descriptor.gallery_id.get() as u64 {
        return Err(conflict("requested ID does not match the worker"));
    }
    let canonical_id = GalleryId::new(
        i64::try_from(proof.canonical_id())
            .map_err(|_| conflict("canonical ID is out of range"))?,
    )
    .map_err(domain_corruption)?;
    let mut connection = repository.connection()?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite_error)?;
    let source = read_pipeline_target(&tx, descriptor)?;
    if source.state != JobState::ResolvingMetadata {
        return Err(invalid_pipeline_state(&source, "link replacement"));
    }
    // A fully received immutable edition has already taken the local-snapshot
    // path. Anything else with an artifact is deliberately NOT migrated.
    let has_artifact: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM download_artifacts WHERE entry_id=?1)
            OR EXISTS(SELECT 1 FROM download_pages WHERE entry_id=?1)
            OR EXISTS(SELECT 1 FROM download_gallery_links WHERE target_entry_id=?1)",
            [&descriptor.entry_id],
            |row| row.get(0),
        )
        .map_err(map_sqlite_error)?;
    if has_artifact {
        return Err(conflict("existing edition/files must be preserved"));
    }
    for id in [descriptor.gallery_id.get(), canonical_id.get()] {
        ensure_identity_available(&tx, id)?;
    }
    let existing = tx.query_row(
        "SELECT j.job_id,j.entry_id,j.gallery_id,j.attempt
         FROM download_jobs j JOIN download_entries d ON d.entry_id=j.entry_id AND d.gallery_id=j.gallery_id
         WHERE d.gallery_id=?1 AND d.state=j.state
         ORDER BY CASE WHEN d.state IN ('queued','resolving_metadata','downloading','hashing','verifying','retry_wait') THEN 0
                       WHEN d.state='completed' THEN 1 WHEN d.state='review_required' THEN 2 ELSE 3 END,
                  d.updated_at DESC,d.entry_id DESC LIMIT 1",
        [canonical_id.get()], |row| Ok(DownloadJobDescriptor {
            job_id: row.get(0)?, entry_id: row.get(1)?,
            gallery_id: canonical_id, worker_attempt: row.get(3)?,
        }),
    ).optional().map_err(map_sqlite_error)?;
    let (target_descriptor, start) = if let Some(existing) = existing {
        let target = read_pipeline_target(&tx, &existing)?;
        if !target.state.is_active()
            && !matches!(target.state, JobState::Completed | JobState::ReviewRequired)
        {
            return Err(conflict(
                "canonical job is stopped; use its existing recovery/review flow",
            ));
        }
        validate_existing_identity(&tx, &existing, target.state, proof)?;
        (existing, None)
    } else {
        let target = insert_job(&tx, canonical_id)?;
        (target.clone(), Some(target))
    };
    let proof_json =
        serde_json::to_string(proof).map_err(|error| RepositoryError::Other(error.to_string()))?;
    let pages_json = serde_json::to_string(proof.page_revisions())
        .map_err(|error| RepositoryError::Other(error.to_string()))?;
    tx.execute(
        "INSERT INTO download_gallery_links(source_entry_id,target_entry_id,requested_gallery_id,canonical_gallery_id,
         created_target,
         content_sha256,proof_json,page_revisions_json,verified_at)
         VALUES(?1,?2,?3,?4,?8,?5,?6,?7,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![descriptor.entry_id,target_descriptor.entry_id,descriptor.gallery_id.get(),canonical_id.get(),
            proof.content_sha256(),proof_json,pages_json,start.is_some()],
    ).map_err(map_sqlite_error)?;
    let message = format!(
        "새 번호 #{}의 기존 작업에 연결했습니다. 이 요청의 파일을 다시 받지 않습니다.",
        canonical_id.get()
    );
    let mut projection = transition_pipeline_target(
        &tx,
        source,
        JobState::Cancelled,
        None,
        None,
        Some("SOURCE_GALLERY_LINKED"),
        Some(&message),
        "Verified replacement linked",
    )?;
    tx.execute(
        "UPDATE download_jobs SET linked_gallery_id=?2,last_error_retryable=0 WHERE job_id=?1",
        params![descriptor.job_id, canonical_id.get()],
    )
    .map_err(map_sqlite_error)?;
    tx.execute(
        "UPDATE download_attempts SET finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),
         outcome_state='cancelled',error_code='SOURCE_GALLERY_LINKED',error_message=?3
         WHERE job_id=?1 AND attempt=?2 AND finished_at IS NULL",
        params![descriptor.job_id, descriptor.worker_attempt, message],
    )
    .map_err(map_sqlite_error)?;
    projection.download.linked_gallery_id = Some(canonical_id.get());
    let target = read_pipeline_target(&tx, &target_descriptor)?
        .into_projection(Some("Existing canonical download retained"))?;
    tx.commit().map_err(map_sqlite_error)?;
    Ok(DownloadGalleryLink {
        projection,
        target,
        start,
    })
}

fn ensure_identity_available(tx: &Transaction<'_>, id: i64) -> Result<(), RepositoryError> {
    let blocked: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM auto_find_exclusions WHERE gallery_id=?1)
         OR EXISTS(SELECT 1 FROM duplicate_hidden_galleries h WHERE h.gallery_id=?1
                   AND NOT EXISTS(SELECT 1 FROM exploration_restored_galleries r WHERE r.gallery_id=h.gallery_id))
         OR EXISTS(SELECT 1 FROM download_entries d WHERE d.gallery_id=?1 AND d.state='quarantined')
         OR EXISTS(SELECT 1 FROM download_artifacts a WHERE a.gallery_id=?1 AND a.state='quarantined')
         OR EXISTS(SELECT 1 FROM excluded_artifact_relocations x JOIN download_entries d ON d.entry_id=x.entry_id
                   WHERE d.gallery_id=?1 AND x.state IN ('pending_exclude','pending_restore'))
         OR EXISTS(SELECT 1 FROM quarantine_records q JOIN download_entries d ON d.entry_id=q.entry_id
                   WHERE d.gallery_id=?1 AND q.state IN ('pending_quarantine','pending_restore'))
         OR EXISTS(SELECT 1 FROM overlap_page_merges m JOIN download_entries d
                   ON d.entry_id IN (m.source_entry_id,m.target_entry_id)
                   WHERE d.gallery_id=?1 AND m.state NOT IN ('applied','rolled_back'))",
        [id], |row| row.get(0),
    ).map_err(map_sqlite_error)?;
    if blocked {
        return Err(conflict(
            "exclusion, quarantine or file-operation protection is active",
        ));
    }
    Ok(())
}

fn insert_job(
    tx: &Transaction<'_>,
    id: GalleryId,
) -> Result<DownloadJobDescriptor, RepositoryError> {
    let descriptor = DownloadJobDescriptor {
        job_id: format!("job-{}", Uuid::new_v4()),
        entry_id: format!("entry-{}", Uuid::new_v4()),
        gallery_id: id,
        worker_attempt: 1,
    };
    tx.execute(
        "INSERT INTO download_entries(entry_id,gallery_id,revision,state,progress,created_at,updated_at)
         VALUES(?1,?2,0,'queued',0,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![descriptor.entry_id,id.get()],
    ).map_err(map_sqlite_error)?;
    tx.execute(
        "INSERT INTO download_jobs(job_id,request_id,entry_id,gallery_id,revision,state,completed_units,total_units,attempt,created_at,updated_at)
         VALUES(?1,?2,?3,?4,0,'queued',0,1,1,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![descriptor.job_id,format!("download-{}",Uuid::new_v4()),descriptor.entry_id,id.get()],
    ).map_err(map_sqlite_error)?;
    tx.execute("INSERT INTO download_attempts(job_id,attempt,started_at) VALUES(?1,1,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        [&descriptor.job_id]).map_err(map_sqlite_error)?;
    Ok(descriptor)
}

fn validate_existing_identity(
    tx: &Transaction<'_>,
    descriptor: &DownloadJobDescriptor,
    state: JobState,
    proof: &GalleryReplacementProof,
) -> Result<(), RepositoryError> {
    let artifact: Option<(i64, String, Option<String>)> = tx
        .query_row(
            "SELECT a.expected_page_count,a.state,g.source_revision FROM download_artifacts a
         JOIN galleries g ON g.gallery_id=a.gallery_id WHERE a.entry_id=?1 AND a.gallery_id=?2",
            params![descriptor.entry_id, descriptor.gallery_id.get()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(map_sqlite_error)?;
    let Some((count, artifact_state, revision)) = artifact else {
        if matches!(
            state,
            JobState::Queued | JobState::ResolvingMetadata | JobState::RetryWait
        ) {
            return validate_pins(tx, &descriptor.entry_id, proof.page_revisions());
        }
        return Err(conflict("canonical artifact is missing"));
    };
    if count != proof.page_revisions().len() as i64
        || !matches!(artifact_state.as_str(), "incomplete" | "complete")
        || (state == JobState::Completed && artifact_state != "complete")
    {
        return Err(conflict("canonical edition page count/state differs"));
    }
    let mut statement = tx.prepare(
        "SELECT source_page_number,state,source_revision,excluded,
            byte_length IS NOT NULL AND byte_length>0 AND length(sha256)=64 AND storage_format='webp' AND verified_at IS NOT NULL
         FROM download_pages WHERE entry_id=?1 ORDER BY source_page_number",
    ).map_err(map_sqlite_error)?;
    let pages = statement
        .query_map([&descriptor.entry_id], |r| {
            Ok((
                r.get::<_, usize>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, Option<bool>>(4)?.unwrap_or(false),
            ))
        })
        .map_err(map_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite_error)?;
    if pages.len() != proof.page_revisions().len() {
        return Err(conflict("canonical checkpoint count differs"));
    }
    let mut complete = true;
    for (index, (number, page_state, revision, excluded, verified)) in pages.iter().enumerate() {
        if *number != index + 1 || *excluded {
            return Err(conflict("canonical page identity/exclusion differs"));
        }
        if page_state == "present" {
            if !verified || revision.as_deref() != Some(&proof.page_revisions()[index]) {
                return Err(conflict("canonical source-page checkpoint differs"));
            }
        } else {
            complete = false;
            if page_state != "pending" {
                return Err(conflict("canonical page requires recovery"));
            }
        }
    }
    if !complete
        && (state == JobState::Completed || revision.as_deref() != Some(proof.canonical_revision()))
    {
        return Err(conflict(
            "partially received canonical edition cannot be proven identical",
        ));
    }
    validate_pins(tx, &descriptor.entry_id, proof.page_revisions())
}

fn validate_pins(
    tx: &Transaction<'_>,
    entry_id: &str,
    pages: &[String],
) -> Result<(), RepositoryError> {
    let mut statement = tx
        .prepare("SELECT page_revisions_json FROM download_gallery_links WHERE target_entry_id=?1")
        .map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([entry_id], |r| r.get::<_, String>(0))
        .map_err(map_sqlite_error)?;
    for row in rows {
        let expected: Vec<String> = serde_json::from_str(&row.map_err(map_sqlite_error)?)
            .map_err(|_| conflict("stored replacement evidence is invalid"))?;
        if expected != pages {
            return Err(conflict(
                "canonical content changed after replacement verification",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_prepared_identity(
    tx: &Transaction<'_>,
    plan: &DownloadArtifactPlan,
) -> Result<(), RepositoryError> {
    let pages = plan
        .source_pages
        .iter()
        .enumerate()
        .map(|(index, page)| {
            if page.source_page_number.get() as usize != index + 1 {
                return Err(conflict("source page order differs"));
            }
            Ok(page.source_revision.clone())
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_pins(tx, &plan.descriptor.entry_id, &pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::hitomi::{parse_galleryinfo_script, verify_gallery_replacement};
    use std::sync::{Arc, Barrier};

    fn proof(old: u64) -> GalleryReplacementProof {
        let metadata = parse_galleryinfo_script(&format!(
            r#"var galleryinfo={{"id":20,"title":"Alias fixture","files":[{{"name":"1.png","hash":"{}"}},{{"name":"2.png","hash":"{}"}}]}};"#,
            "a".repeat(64),"b".repeat(64),
        )).unwrap();
        verify_gallery_replacement(old, &metadata, 20, &metadata).unwrap()
    }

    fn queue(repo: &SqliteRepository, id: i64, request: &str) -> DownloadQueueRecord {
        match repo
            .download_queue_add(request, &[GalleryId::new(id).unwrap()])
            .unwrap()
        {
            DownloadQueueAddOutcome::Added(record) => record,
            _ => panic!("unexpected idempotency conflict"),
        }
    }

    fn plan(descriptor: &DownloadJobDescriptor) -> DownloadArtifactPlan {
        let proof = proof(10);
        DownloadArtifactPlan {
            descriptor: descriptor.clone(),
            gallery: Gallery::new(
                descriptor.gallery_id,
                1,
                GalleryMetadata::new("Alias fixture", None, None, 2).unwrap(),
            ),
            source_revision: proof.canonical_revision().into(),
            root_snapshot: PathBuf::from("C:/isolated-alias-fixture"),
            relative_directory: ArtifactRelativePath::new(format!(
                "gallery-{}",
                descriptor.gallery_id.get()
            ))
            .unwrap(),
            manifest_relative_path: ArtifactRelativePath::new(format!(
                "gallery-{}/manifest.json",
                descriptor.gallery_id.get()
            ))
            .unwrap(),
            source_pages: proof
                .page_revisions()
                .iter()
                .enumerate()
                .map(|(i, revision)| DownloadSourcePage {
                    source_page_number: SourcePageNumber::new(i as u32 + 1).unwrap(),
                    source_revision: revision.clone(),
                })
                .collect(),
        }
    }

    fn prepared(repo: &SqliteRepository, descriptor: &DownloadJobDescriptor, received: bool) {
        repo.pipeline_begin(descriptor).unwrap();
        let plan = plan(descriptor);
        repo.pipeline_prepare(&plan).unwrap();
        if received {
            for page in &plan.source_pages {
                repo.pipeline_page_verified(
                    descriptor,
                    &StoredPage {
                        source_page_number: page.source_page_number,
                        relative_path: ArtifactRelativePath::new(format!(
                            "{}/{:04}.webp",
                            plan.relative_directory.as_str(),
                            page.source_page_number.get()
                        ))
                        .unwrap(),
                        byte_length: 16,
                        sha256: ArtifactSha256::new("c".repeat(64)).unwrap(),
                        storage_format: ArtifactStorageFormat::Webp,
                        source_revision: page.source_revision.clone(),
                        verified_at: "2026-10-16T00:00:00Z".into(),
                    },
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn replacement_reuses_active_target_without_restarting_or_revising_it() {
        let repo = SqliteRepository::open_in_memory().unwrap();
        let target = queue(&repo, 20, "canonical").jobs.remove(0);
        prepared(&repo, &target, true);
        repo.pipeline_stage(&target, JobState::Hashing, "fixture")
            .unwrap();
        let before = repo
            .connection()
            .unwrap()
            .query_row(
                "SELECT revision,attempt,state FROM download_jobs WHERE job_id=?1",
                [&target.job_id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();
        let old = queue(&repo, 10, "old").jobs.remove(0);
        repo.pipeline_begin(&old).unwrap();
        let linked = repo.pipeline_link_gallery(&old, &proof(10)).unwrap();
        assert!(linked.start.is_none());
        assert_eq!(linked.target.download.entry_id, target.entry_id);
        assert_eq!(linked.projection.download.linked_gallery_id, Some(20));
        assert_eq!(linked.projection.download.state, JobState::Cancelled);
        assert_eq!(linked.projection.download.progress, Some(0.0));
        let after = repo
            .connection()
            .unwrap()
            .query_row(
                "SELECT revision,attempt,state FROM download_jobs WHERE job_id=?1",
                [&target.job_id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(before, after);
        assert!(matches!(
            repo.download_retry(&[DownloadEntryId::new(old.entry_id.clone()).unwrap()])
                .unwrap(),
            DownloadMutationOutcome::InvalidState { .. }
        ));
        let entries = repo
            .download_entries_list(&DownloadListRequest {
                state: None,
                query: None,
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(
            entries
                .entries
                .iter()
                .find(|entry| entry.entry_id.as_str() == old.entry_id)
                .unwrap()
                .linked_gallery_id,
            Some(20)
        );
        let library = repo
            .download_library_page_list(&DownloadListRequest {
                state: None,
                query: None,
                page: 1,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(
            library
                .items
                .iter()
                .find(|item| item.download.gallery_id.get() == 10)
                .unwrap()
                .download
                .linked_gallery_id,
            Some(20)
        );
    }

    #[test]
    fn replacement_reuses_completed_checkpoint_and_direct_queue_never_redownloads_it() {
        let repo = SqliteRepository::open_in_memory().unwrap();
        let target = queue(&repo, 20, "canonical").jobs.remove(0);
        prepared(&repo, &target, true);
        repo.pipeline_stage(&target, JobState::Hashing, "fixture")
            .unwrap();
        repo.pipeline_stage(&target, JobState::Verifying, "fixture")
            .unwrap();
        let mut bundle = repo
            .pipeline_artifact_bundle(&DownloadEntryId::new(target.entry_id.clone()).unwrap())
            .unwrap()
            .unwrap();
        bundle.artifact.state = DownloadArtifactState::Complete;
        bundle.artifact = bundle
            .artifact
            .with_manifest(
                plan(&target).manifest_relative_path,
                1,
                "isolated-test",
                1,
                "2026-10-16T00:00:00Z",
            )
            .unwrap();
        let manifest = ArtifactManifest::from_bundle(&bundle).unwrap();
        repo.pipeline_complete(&target, &manifest, &plan(&target).manifest_relative_path)
            .unwrap();
        let old = queue(&repo, 10, "old").jobs.remove(0);
        repo.pipeline_begin(&old).unwrap();
        let linked = repo.pipeline_link_gallery(&old, &proof(10)).unwrap();
        assert!(linked.start.is_none());
        assert_eq!(linked.target.download.state, JobState::Completed);
        assert_eq!(queue(&repo, 20, "canonical-again").jobs.len(), 0);
        assert_eq!(
            repo.pipeline_artifact_bundle(&DownloadEntryId::new(target.entry_id.clone()).unwrap())
                .unwrap()
                .unwrap()
                .pages,
            bundle.pages
        );
    }

    #[test]
    fn replacement_retains_review_required_and_does_not_fabricate_completion() {
        let repo = SqliteRepository::open_in_memory().unwrap();
        let target = queue(&repo, 20, "canonical-review").jobs.remove(0);
        prepared(&repo, &target, true);
        {
            // Isolated persisted review fixture; the decision itself belongs to
            // the unchanged overlap review service, never to alias settlement.
            let c = repo.connection().unwrap();
            c.execute(
                "UPDATE download_entries SET state='review_required',progress=99,review_kind='gallery_duplicate',review_id='canonical-review' WHERE entry_id=?1",
                [&target.entry_id],
            )
            .unwrap();
            c.execute("UPDATE download_jobs SET state='review_required',last_error_code='DOWNLOAD_OVERLAP_REVIEW',last_error_message='Pending decision' WHERE entry_id=?1",[&target.entry_id]).unwrap();
        }
        let old = queue(&repo, 10, "old").jobs.remove(0);
        repo.pipeline_begin(&old).unwrap();
        let linked = repo.pipeline_link_gallery(&old, &proof(10)).unwrap();
        assert!(linked.start.is_none());
        assert_eq!(linked.target.download.state, JobState::ReviewRequired);
        let direct=queue(&repo,20,"direct-while-reviewing");
        assert!(direct.jobs.is_empty());
        assert_eq!(direct.entries[0].review_id.as_deref(),Some("canonical-review"));
        assert_eq!(direct.entries[0].review_kind,Some(DownloadReviewKind::GalleryDuplicate));
        assert_eq!(direct.entries[0].entry_id.as_str(),target.entry_id);
        assert_eq!(linked.target.download.progress, Some(99.0));
        assert_eq!(
            linked.target.download.error_code.as_deref(),
            Some("DOWNLOAD_OVERLAP_REVIEW")
        );
        let artifact = repo
            .pipeline_artifact_bundle(&DownloadEntryId::new(target.entry_id).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(artifact.artifact.state, DownloadArtifactState::Incomplete);
        assert_eq!(artifact.pages.len(), 2);
        let c = repo.connection().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM download_attempts WHERE job_id=?1",
                [target.job_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        assert_eq!(
            c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
    }

    #[test]
    fn replacement_old_and_canonical_admission_race_creates_only_one_target_job() {
        let repo = Arc::new(SqliteRepository::open_in_memory().unwrap());
        let old = queue(&repo, 10, "old").jobs.remove(0);
        repo.pipeline_begin(&old).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let handle = {
            let repo = repo.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                queue(&repo, 20, "direct")
            })
        };
        barrier.wait();
        let linked = repo.pipeline_link_gallery(&old, &proof(10)).unwrap();
        let direct = handle.join().unwrap();
        assert_eq!(usize::from(linked.start.is_some()) + direct.jobs.len(), 1);
        assert_eq!(
            repo.connection()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM download_jobs WHERE gallery_id=20",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            direct.entries[0].entry_id.as_str(),
            linked.target.download.entry_id
        );
    }

    #[test]
    fn replacement_content_pin_rejects_change_before_any_artifact_is_created() {
        let repo = SqliteRepository::open_in_memory().unwrap();
        let old = queue(&repo, 10, "old").jobs.remove(0);
        repo.pipeline_begin(&old).unwrap();
        let target = repo
            .pipeline_link_gallery(&old, &proof(10))
            .unwrap()
            .start
            .unwrap();
        repo.pipeline_begin(&target).unwrap();
        let mut changed = plan(&target);
        changed.source_pages[0].source_revision = "hitomi-page-v1:changed".into();
        assert!(repo.pipeline_prepare(&changed).is_err());
        assert_eq!(
            repo.connection()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM download_artifacts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        repo.pipeline_prepare(&plan(&target)).unwrap();
    }

    #[test]
    fn replacement_preserves_partial_old_files_and_exclusion_protection() {
        for old_partial in [true, false] {
            let repo = SqliteRepository::open_in_memory().unwrap();
            let old = queue(&repo, 10, "old").jobs.remove(0);
            if old_partial {
                prepared(&repo, &old, false);
                repo.pipeline_fail(&old, "TEST", "fixture", true).unwrap();
                let jobs = match repo
                    .download_retry(&[DownloadEntryId::new(old.entry_id.clone()).unwrap()])
                    .unwrap()
                {
                    DownloadMutationOutcome::Applied(jobs) => jobs,
                    _ => panic!("retry"),
                };
                let next = repo.pipeline_descriptors_for_jobs(&jobs).unwrap().remove(0);
                repo.pipeline_begin(&next).unwrap();
                assert!(repo.pipeline_link_gallery(&next, &proof(10)).is_err());
                assert_eq!(
                    repo.connection()
                        .unwrap()
                        .query_row(
                            "SELECT COUNT(*) FROM download_artifacts WHERE entry_id=?1",
                            [&old.entry_id],
                            |r| r.get::<_, i64>(0)
                        )
                        .unwrap(),
                    1
                );
            } else {
                repo.pipeline_begin(&old).unwrap();
                repo.connection()
                    .unwrap()
                    .execute(
                        "INSERT INTO auto_find_exclusions VALUES(20,'manual','now')",
                        [],
                    )
                    .unwrap();
                assert!(repo.pipeline_link_gallery(&old, &proof(10)).is_err());
            }
            assert_eq!(
                repo.connection()
                    .unwrap()
                    .query_row("SELECT COUNT(*) FROM download_gallery_links", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(
                repo.connection()
                    .unwrap()
                    .query_row(
                        "SELECT COUNT(*) FROM download_jobs WHERE gallery_id=20",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
        }
    }

    #[test]
    fn replacement_cancelled_or_stale_worker_cannot_link_and_created_target_can_resume() {
        let repo = SqliteRepository::open_in_memory().unwrap();
        let old = queue(&repo, 10, "old").jobs.remove(0);
        repo.pipeline_begin(&old).unwrap();
        let mut stale = old.clone();
        stale.worker_attempt += 1;
        assert!(repo.pipeline_link_gallery(&stale, &proof(10)).is_err());
        let target = repo
            .pipeline_link_gallery(&old, &proof(10))
            .unwrap()
            .start
            .unwrap();
        // Simulate the existing startup state recovery, with no real app/DB.
        repo.connection().unwrap().execute_batch("UPDATE download_entries SET state='interrupted' WHERE state='queued';UPDATE download_jobs SET state='interrupted' WHERE state='queued';").unwrap();
        let resumed = repo.pipeline_resume_interrupted().unwrap();
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].entry_id, target.entry_id);
        let cancelled = queue(&repo, 30, "cancelled").jobs.remove(0);
        repo.pipeline_begin(&cancelled).unwrap();
        repo.download_cancel(&[DownloadEntryId::new(cancelled.entry_id.clone()).unwrap()])
            .unwrap();
        assert!(repo.pipeline_link_gallery(&cancelled, &proof(30)).is_err());
    }

    #[test]
    fn replacement_canonical_changed_checkpoint_or_stopped_job_is_not_restarted() {
        for state in ["changed", "failed", "quarantined"] {
            let repo = SqliteRepository::open_in_memory().unwrap();
            let target = queue(&repo, 20, "canonical").jobs.remove(0);
            prepared(&repo, &target, true);
            if state == "changed" {
                repo.connection().unwrap().execute("UPDATE download_pages SET source_revision='another-edition' WHERE entry_id=?1 AND source_page_number=1",[&target.entry_id]).unwrap();
            } else if state == "failed" {
                repo.pipeline_fail(&target, "TEST", "leave stopped", false)
                    .unwrap();
            } else {
                // Isolated fixture state only; no real quarantine/move operation.
                let c = repo.connection().unwrap();
                c.execute(
                    "UPDATE download_entries SET state='quarantined' WHERE entry_id=?1",
                    [&target.entry_id],
                )
                .unwrap();
                c.execute(
                    "UPDATE download_jobs SET state='quarantined' WHERE entry_id=?1",
                    [&target.entry_id],
                )
                .unwrap();
            }
            let old = queue(&repo, 10, "old").jobs.remove(0);
            repo.pipeline_begin(&old).unwrap();
            assert!(repo.pipeline_link_gallery(&old, &proof(10)).is_err());
            let c = repo.connection().unwrap();
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM download_jobs WHERE gallery_id=20",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
            assert_eq!(
                c.query_row("SELECT COUNT(*) FROM download_gallery_links", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
    }
}
