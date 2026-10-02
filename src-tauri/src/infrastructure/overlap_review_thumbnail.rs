//! Read-only access to the exact artifacts referenced by a saved comparison.
//! Quarantine state is never relaxed for ordinary gallery/artifact thumbnails.
use std::path::PathBuf;

use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::{
    application::{verified_overlap_pages, ArtifactRepository},
    domain::{
        ArtifactRelativePath, DownloadArtifactState, DownloadEntryId, PageArtifact,
        PageArtifactState,
    },
    thumbnail::{OverlapReviewSide, ThumbnailFailureCode, ThumbnailKey, ThumbnailResolveError},
};

use super::SqliteRepository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewPage {
    pub root: PathBuf,
    /// An authorized read descriptor, not a persisted page/state mutation.
    pub page: PageArtifact,
    artifact_revision: u64,
    fingerprint: String,
}

pub(super) fn changed() -> ThumbnailResolveError {
    ThumbnailResolveError::new(
        ThumbnailFailureCode::EvidenceChanged,
        "판정 당시 원본이 변경되어 이 비교의 미리보기를 표시할 수 없습니다.",
        false,
    )
}

pub(super) fn unavailable() -> ThumbnailResolveError {
    ThumbnailResolveError::new(
        ThumbnailFailureCode::EvidenceUnavailable,
        "판정 당시 원본 또는 보관 위치를 찾을 수 없습니다.",
        false,
    )
}

fn database_error(_: impl std::fmt::Display) -> ThumbnailResolveError {
    ThumbnailResolveError::temporarily_unavailable("검토 이미지 정보를 읽지 못했습니다.")
}

pub(super) fn load_review_page(
    repository: &SqliteRepository,
    key: &ThumbnailKey,
) -> Result<ReviewPage, ThumbnailResolveError> {
    let ThumbnailKey::OverlapReviewPage {
        review_id,
        candidate_id,
        review_revision,
        side,
        source_page,
    } = key
    else {
        return Err(unavailable());
    };
    key.validate().map_err(|_| unavailable())?;
    let (entry_id, expected, profile, policy) = {
        let connection = repository.connection().map_err(database_error)?;
        let row = connection.query_row(
            "SELECT r.revision, r.profile_version, r.policy_version,
                    r.entry_id, r.incoming_fingerprint, c.existing_entry_id, c.existing_fingerprint
             FROM download_overlap_reviews r JOIN download_overlap_candidates c ON c.review_id=r.review_id
             WHERE r.review_id=?1 AND c.candidate_id=?2",
            params![review_id, candidate_id],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u32>(1)?, row.get::<_, u32>(2)?,
                row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?)),
        ).optional().map_err(database_error)?.ok_or_else(unavailable)?;
        if row.0 != *review_revision {
            return Err(changed());
        }
        match side {
            OverlapReviewSide::Incoming => (row.3, row.4, row.1, row.2),
            OverlapReviewSide::Existing => (row.5, row.6, row.1, row.2),
        }
    };
    let entry_id = DownloadEntryId::new(entry_id).map_err(|_| unavailable())?;
    let mut bundle = repository
        .artifact_bundle_get(&entry_id)
        .map_err(database_error)?
        .ok_or_else(unavailable)?;
    if bundle.artifact.state == DownloadArtifactState::MissingArtifacts {
        return Err(unavailable());
    }

    let (root, quarantine) = {
        let connection = repository.connection().map_err(database_error)?;
        let (root, revision, moving): (String, u64, bool) = connection.query_row(
            "SELECT a.root_snapshot, a.revision,
                EXISTS(SELECT 1 FROM excluded_artifact_relocations x WHERE x.entry_id=a.entry_id AND x.state IN ('pending_exclude','pending_restore'))
                OR EXISTS(SELECT 1 FROM overlap_page_merges m WHERE a.entry_id IN (m.target_entry_id,m.source_entry_id) AND m.state NOT IN ('applied','rolled_back'))
             FROM download_artifacts a WHERE a.entry_id=?1",
            [entry_id.as_str()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).map_err(database_error)?;
        if revision != bundle.artifact.revision {
            return Err(changed());
        }
        if moving {
            return Err(ThumbnailResolveError::temporarily_unavailable(
                "원본 파일 이동 또는 병합이 끝난 뒤 다시 확인해 주세요.",
            ));
        }
        let quarantine: Option<(String, String, String)> = connection.query_row(
            "SELECT original_relative_path,quarantine_relative_path,state FROM quarantine_records
             WHERE entry_id=?1 AND state IN ('pending_quarantine','quarantined','pending_restore')",
            [entry_id.as_str()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional().map_err(database_error)?;
        if quarantine
            .as_ref()
            .is_some_and(|record| record.2 != "quarantined")
        {
            return Err(ThumbnailResolveError::temporarily_unavailable(
                "격리 파일 이동이 끝난 뒤 다시 확인해 주세요.",
            ));
        }
        if root.trim().is_empty() {
            return Err(unavailable());
        }
        (PathBuf::from(root), quarantine)
    };

    if bundle.artifact.state == DownloadArtifactState::Quarantined {
        let (original, destination, _) = quarantine.as_ref().ok_or_else(unavailable)?;
        if original != bundle.artifact.relative_directory.as_str()
            || !destination.starts_with(".atsumi-quarantine/")
        {
            return Err(unavailable());
        }
        for page in &mut bundle.pages {
            if page.excluded {
                continue;
            }
            if page.state != PageArtifactState::Quarantined {
                return Err(changed());
            }
            let suffix = page
                .relative_path
                .as_str()
                .strip_prefix(original.as_str())
                .and_then(|suffix| suffix.strip_prefix('/'))
                .ok_or_else(unavailable)?;
            page.relative_path = ArtifactRelativePath::new(format!("{destination}/{suffix}"))
                .map_err(|_| unavailable())?;
            // Only this detached read descriptor is normalized. The database,
            // download state, original paths and quarantine journal stay intact.
            page.state = PageArtifactState::Present;
        }
    } else if quarantine.is_some() {
        return Err(changed());
    }

    let mut pages = verified_overlap_pages(&bundle).ok_or_else(changed)?;
    pages.sort_by_key(|page| page.page_id.source_page_number);
    // Use the policy stored with this review, including older reviews. This is
    // the same fingerprint recipe as download_overlap::overlap_artifact_fingerprint.
    let mut digest = Sha256::new();
    digest.update(b"atsumi-download-overlap\0");
    digest.update(profile.to_le_bytes());
    digest.update(policy.to_le_bytes());
    for page in &pages {
        digest.update(page.page_id.source_page_number.get().to_le_bytes());
        digest.update([0]);
        digest.update(
            page.source_revision
                .as_ref()
                .ok_or_else(changed)?
                .as_bytes(),
        );
        digest.update([0]);
        digest.update(
            page.sha256
                .as_ref()
                .ok_or_else(changed)?
                .as_str()
                .as_bytes(),
        );
        digest.update([0xff]);
    }
    if format!("{:x}", digest.finalize()) != expected {
        if let Some((root, page, artifact_revision)) =
            super::overlap_merge::review_backup_page(repository, &entry_id, &expected, *source_page)
                .map_err(database_error)?
        {
            return Ok(ReviewPage {
                root,
                page,
                artifact_revision,
                fingerprint: expected,
            });
        }
        return Err(changed());
    }
    let page = pages
        .into_iter()
        .find(|page| page.page_id.source_page_number.get() == *source_page)
        .ok_or_else(unavailable)?
        .clone();
    Ok(ReviewPage {
        root,
        page,
        artifact_revision: bundle.artifact.revision,
        fingerprint: expected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        application::{overlap_artifact_fingerprint, DownloadQueueAddOutcome, DownloadRepository},
        domain::GalleryId,
        infrastructure::{CompositeThumbnailResolver, FilesystemArtifactStore, ThumbnailDiskCache},
        thumbnail::{
            CancellationToken, FixtureThumbnailResolver, ThumbnailConsumer, ThumbnailCoordinator,
            ThumbnailCoordinatorConfig, ThumbnailPriority, ThumbnailRequestDto, ThumbnailResolver,
        },
    };
    use image::{codecs::webp::WebPEncoder, ExtendedColorType, ImageEncoder};
    use std::{fs, sync::Arc, time::Duration};

    struct Fixture {
        temp: tempfile::TempDir,
        repository: Arc<SqliteRepository>,
        existing: String,
        incoming: String,
    }

    impl Fixture {
        fn new(excluded_incoming: bool) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let repository = Arc::new(SqliteRepository::open_in_memory().unwrap());
            let mut bytes = Vec::new();
            WebPEncoder::new_lossless(&mut bytes)
                .write_image(&[30, 70, 90, 255], 1, 1, ExtendedColorType::Rgba8)
                .unwrap();
            let seed = |gallery_id: i64, directory: &str, cancelled: bool| {
                let DownloadQueueAddOutcome::Added(queued) = repository
                    .download_queue_add(
                        &format!("review-fixture-{gallery_id}"),
                        &[GalleryId::new(gallery_id).unwrap()],
                    )
                    .unwrap()
                else {
                    panic!("new fixture download")
                };
                let entry_id = queued.entries[0].entry_id.to_string();
                let connection = repository.connection().unwrap();
                connection.execute("INSERT INTO galleries(gallery_id,revision,title,source_page_count) VALUES(?1,0,'review fixture',1)", [gallery_id]).unwrap();
                let state = if cancelled { "cancelled" } else { "completed" };
                connection
                    .execute(
                        "UPDATE download_entries SET state=?1 WHERE entry_id=?2",
                        params![state, entry_id],
                    )
                    .unwrap();
                connection
                    .execute(
                        "UPDATE download_jobs SET state=?1 WHERE entry_id=?2",
                        params![state, entry_id],
                    )
                    .unwrap();
                connection.execute("INSERT INTO download_artifacts(entry_id,gallery_id,revision,relative_directory,expected_page_count,state,manifest_relative_path,manifest_schema_version,writer_version,hash_profile_version,completed_at,root_snapshot)
                    VALUES(?1,?2,0,?3,1,?4,?5,1,'fixture',1,'now',?6)",
                    params![entry_id,gallery_id,directory,if cancelled {"incomplete"} else {"complete"},format!("{directory}/manifest.json"),temp.path().to_string_lossy()]).unwrap();
                connection.execute("INSERT INTO download_pages(entry_id,gallery_id,source_page_number,relative_path,state,byte_length,sha256,storage_format,source_revision,verified_at)
                    VALUES(?1,?2,1,?3,'present',?4,?5,'webp','fixture-revision','now')",
                    params![entry_id,gallery_id,format!("{directory}/0001.webp"),bytes.len(),format!("{:x}", Sha256::digest(&bytes))]).unwrap();
                fs::create_dir_all(temp.path().join(directory)).unwrap();
                fs::write(temp.path().join(directory).join("0001.webp"), &bytes).unwrap();
                entry_id
            };
            let existing = seed(101, "album-101", false);
            let incoming = seed(
                202,
                if excluded_incoming {
                    ".atsumi-excluded/incoming/album-202"
                } else {
                    "album-202"
                },
                excluded_incoming,
            );
            let fingerprint = |entry: &str| {
                overlap_artifact_fingerprint(
                    &repository
                        .artifact_bundle_get(&DownloadEntryId::new(entry).unwrap())
                        .unwrap()
                        .unwrap(),
                    1,
                )
                .unwrap()
            };
            let existing_fingerprint = fingerprint(&existing);
            let incoming_fingerprint = fingerprint(&incoming);
            {
                let connection = repository.connection().unwrap();
                connection.execute("INSERT INTO download_overlap_reviews(review_id,entry_id,incoming_gallery_id,revision,state,profile_version,policy_version,incoming_fingerprint,created_at,updated_at)
                    VALUES('review-1',?1,202,1,?2,1,2,?3,'now','now')",
                    params![incoming,if excluded_incoming {"cancelled"} else {"resolved"},incoming_fingerprint]).unwrap();
                connection.execute("INSERT INTO download_overlap_candidates(candidate_id,review_id,existing_entry_id,existing_gallery_id,existing_fingerprint,relation,confidence,matched_pages,exact_pages,visual_pages,existing_coverage,incoming_coverage,existing_unique_pages,incoming_unique_pages,longest_aligned_run,rank)
                    VALUES('candidate-1','review-1',?1,101,?2,'near_equivalent',1,1,1,0,1,1,0,0,1,1)", params![existing,existing_fingerprint]).unwrap();
            }
            Self {
                temp,
                repository,
                existing,
                incoming,
            }
        }

        fn key(&self, side: OverlapReviewSide) -> ThumbnailKey {
            ThumbnailKey::OverlapReviewPage {
                review_id: "review-1".into(),
                candidate_id: "candidate-1".into(),
                review_revision: 1,
                side,
                source_page: 1,
            }
        }

        fn resolver(&self) -> CompositeThumbnailResolver {
            CompositeThumbnailResolver::new(
                Arc::new(FixtureThumbnailResolver::new()),
                self.repository.clone(),
                self.repository.clone(),
                Arc::new(FilesystemArtifactStore::new()),
            )
            .with_review_repository(self.repository.clone())
            .with_disk_cache(Arc::new(ThumbnailDiskCache::new(
                self.temp.path().join("cache"),
                1024 * 1024,
            )))
        }

        fn quarantine_existing(&self) -> PathBuf {
            let destination = ".atsumi-quarantine/record-1/album-101";
            fs::create_dir_all(self.temp.path().join(".atsumi-quarantine/record-1")).unwrap();
            fs::rename(
                self.temp.path().join("album-101"),
                self.temp.path().join(destination),
            )
            .unwrap();
            let connection = self.repository.connection().unwrap();
            connection.execute("INSERT INTO quarantine_records(record_id,entry_id,original_relative_path,quarantine_relative_path,reason,state,created_at)
                VALUES('record-1',?1,'album-101',?2,'review fixture','quarantined','now')", params![self.existing,destination]).unwrap();
            connection.execute("UPDATE download_artifacts SET state='quarantined',revision=revision+1 WHERE entry_id=?1", [&self.existing]).unwrap();
            connection
                .execute(
                    "UPDATE download_pages SET state='quarantined' WHERE entry_id=?1",
                    [&self.existing],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE download_entries SET state='quarantined' WHERE entry_id=?1",
                    [&self.existing],
                )
                .unwrap();
            self.temp.path().join(destination).join("0001.webp")
        }

        fn state(&self) -> (String, String, String, String, i64) {
            self.repository.connection().unwrap().query_row(
                "SELECT e.state,a.state,a.relative_directory,p.state,(SELECT count(*) FROM download_overlap_automation_acknowledgements)
                 FROM download_entries e JOIN download_artifacts a USING(entry_id) JOIN download_pages p USING(entry_id) WHERE e.entry_id=?1",
                [&self.existing], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
            ).unwrap()
        }
    }

    #[test]
    fn quarantined_review_pages_are_read_without_restore_or_general_access() {
        let fixture = Fixture::new(false);
        let path = fixture.quarantine_existing();
        let before = fixture.state();
        let resolver = fixture.resolver();
        assert!(resolver
            .resolve(
                &fixture.key(OverlapReviewSide::Existing),
                &CancellationToken::new()
            )
            .is_ok());
        assert!(resolver
            .resolve(
                &fixture.key(OverlapReviewSide::Incoming),
                &CancellationToken::new()
            )
            .is_ok());
        assert_eq!(
            resolver
                .resolve(
                    &ThumbnailKey::artifact_page(&fixture.existing, 1).unwrap(),
                    &CancellationToken::new()
                )
                .unwrap_err()
                .code,
            ThumbnailFailureCode::NotFound
        );
        assert_eq!(fixture.state(), before);
        assert!(path.is_file());
        assert!(!fixture.temp.path().join("album-101").exists());
    }

    #[test]
    fn cancelled_excluded_pages_and_old_policy_reviews_remain_readable() {
        let fixture = Fixture::new(true);
        assert!(fixture
            .resolver()
            .resolve(
                &fixture.key(OverlapReviewSide::Incoming),
                &CancellationToken::new()
            )
            .is_ok());
        let bundle = fixture
            .repository
            .artifact_bundle_get(&DownloadEntryId::new(&fixture.incoming).unwrap())
            .unwrap()
            .unwrap();
        let page = &bundle.pages[0];
        let mut digest = Sha256::new();
        digest.update(b"atsumi-download-overlap\0");
        digest.update(1_u32.to_le_bytes());
        digest.update(1_u32.to_le_bytes());
        digest.update(1_u32.to_le_bytes());
        digest.update([0]);
        digest.update(b"fixture-revision");
        digest.update([0]);
        digest.update(page.sha256.as_ref().unwrap().as_str().as_bytes());
        digest.update([0xff]);
        fixture.repository.connection().unwrap().execute("UPDATE download_overlap_reviews SET policy_version=1,incoming_fingerprint=?1 WHERE review_id='review-1'", [format!("{:x}",digest.finalize())]).unwrap();
        assert!(fixture
            .resolver()
            .resolve(
                &fixture.key(OverlapReviewSide::Incoming),
                &CancellationToken::new()
            )
            .is_ok());
    }

    #[test]
    fn rejects_wrong_review_candidate_page_and_changed_bundle() {
        let fixture = Fixture::new(false);
        let resolver = fixture.resolver();
        let invalid_keys = [
            ("review-1", "unrelated", 1),
            ("unrelated", "candidate-1", 1),
            ("review-1", "candidate-1", 2),
        ];
        for (review_id, candidate_id, source_page) in invalid_keys {
            let key = ThumbnailKey::OverlapReviewPage {
                review_id: review_id.into(),
                candidate_id: candidate_id.into(),
                review_revision: 1,
                side: OverlapReviewSide::Existing,
                source_page,
            };
            assert_eq!(
                resolver
                    .resolve(&key, &CancellationToken::new())
                    .unwrap_err()
                    .code,
                ThumbnailFailureCode::EvidenceUnavailable
            );
        }
        fixture
            .repository
            .connection()
            .unwrap()
            .execute(
                "UPDATE download_pages SET sha256=?1 WHERE entry_id=?2",
                params!["0".repeat(64), fixture.existing],
            )
            .unwrap();
        assert_eq!(
            resolver
                .resolve(
                    &fixture.key(OverlapReviewSide::Existing),
                    &CancellationToken::new()
                )
                .unwrap_err()
                .code,
            ThumbnailFailureCode::EvidenceChanged
        );
    }

    #[test]
    fn memory_and_disk_caches_cannot_hide_missing_or_changed_review_originals() {
        let fixture = Fixture::new(false);
        let path = fixture.quarantine_existing();
        let cache = Arc::new(ThumbnailDiskCache::new(
            fixture.temp.path().join("checked-cache"),
            1024 * 1024,
        ));
        let coordinator = ThumbnailCoordinator::new(
            Arc::new(fixture.resolver().with_disk_cache(cache.clone())),
            ThumbnailCoordinatorConfig {
                request_start_interval: Duration::ZERO,
                ..ThumbnailCoordinatorConfig::default()
            },
        )
        .unwrap();
        let request = || ThumbnailRequestDto {
            key: fixture.key(OverlapReviewSide::Existing),
            consumer: ThumbnailConsumer::Review,
            priority: ThumbnailPriority::Critical,
        };
        assert!(coordinator.request(request()).unwrap().recv().is_ok());
        assert_eq!(cache.usage().entries, 1);
        assert!(coordinator.request(request()).unwrap().recv().is_ok());
        fs::write(&path, b"changed source").unwrap();
        assert_eq!(
            coordinator
                .request(request())
                .unwrap()
                .recv()
                .unwrap_err()
                .code,
            ThumbnailFailureCode::EvidenceChanged
        );
        fs::remove_file(&path).unwrap();
        assert_eq!(
            coordinator
                .request(request())
                .unwrap()
                .recv()
                .unwrap_err()
                .code,
            ThumbnailFailureCode::EvidenceUnavailable
        );
    }

    #[test]
    fn rejects_stale_review_revision_and_pending_or_escaping_quarantine_paths() {
        let fixture = Fixture::new(false);
        fixture.quarantine_existing();
        let resolver = fixture.resolver();
        let mut key = fixture.key(OverlapReviewSide::Existing);
        if let ThumbnailKey::OverlapReviewPage {
            review_revision, ..
        } = &mut key
        {
            *review_revision = 0;
        }
        assert_eq!(
            resolver
                .resolve(&key, &CancellationToken::new())
                .unwrap_err()
                .code,
            ThumbnailFailureCode::EvidenceChanged
        );
        let key = fixture.key(OverlapReviewSide::Existing);
        fixture
            .repository
            .connection()
            .unwrap()
            .execute(
                "UPDATE quarantine_records SET state='pending_restore' WHERE record_id='record-1'",
                [],
            )
            .unwrap();
        assert_eq!(
            resolver
                .resolve(&key, &CancellationToken::new())
                .unwrap_err()
                .code,
            ThumbnailFailureCode::TemporarilyUnavailable
        );
        fixture.repository.connection().unwrap().execute("UPDATE quarantine_records SET state='quarantined',quarantine_relative_path='.atsumi-quarantine/../other' WHERE record_id='record-1'", []).unwrap();
        assert_eq!(
            resolver
                .resolve(&key, &CancellationToken::new())
                .unwrap_err()
                .code,
            ThumbnailFailureCode::EvidenceUnavailable
        );
    }
}
