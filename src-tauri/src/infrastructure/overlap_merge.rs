//! Recoverable, copy-on-write page replacement. A pending operation never owns
//! new database page bytes: until the final SQLite commit, recovery restores the
//! untouched whole-folder backup. Donor bytes are copied, never moved/deleted.
use super::SqliteRepository;
use crate::{
    application::{
        overlap_artifact_fingerprint, ApplicationError, ArtifactRepository,
        DownloadOverlapRepository, DownloadPipelineRepository, RepositoryError,
    },
    domain::{
        ArtifactBundle, ArtifactManifest, ArtifactRelativePath, ArtifactSha256,
        DownloadArtifactState, DownloadEntryId, DownloadJobDescriptor, DownloadOverlapReview,
        DownloadOverlapReviewState, GalleryId, PageArtifact, PageArtifactState,
    },
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};
use uuid::Uuid;
#[path = "completed_pair.rs"]
mod completed_pair;

pub(super) const OVERLAP_MERGE_SCHEMA: &str = r#"
CREATE TABLE overlap_page_merges (
    merge_id TEXT PRIMARY KEY,
    review_id TEXT NOT NULL REFERENCES download_overlap_reviews(review_id),
    target_entry_id TEXT NOT NULL REFERENCES download_entries(entry_id),
    source_entry_id TEXT NOT NULL REFERENCES download_entries(entry_id),
    state TEXT NOT NULL CHECK(state IN ('preparing','swapping','committing','applied','rolled_back')),
    journal_json TEXT NOT NULL CHECK(json_valid(journal_json)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX overlap_page_merges_active_idx ON overlap_page_merges(state,target_entry_id,source_entry_id);
ALTER TABLE duplicate_candidates ADD COLUMN artifact_stale INTEGER NOT NULL DEFAULT 0 CHECK(artifact_stale IN(0,1));
CREATE TRIGGER overlap_merge_no_concurrent_insert BEFORE INSERT ON overlap_page_merges
WHEN EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state NOT IN ('applied','rolled_back')
 AND (m.target_entry_id IN(NEW.target_entry_id,NEW.source_entry_id) OR m.source_entry_id IN(NEW.target_entry_id,NEW.source_entry_id)))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_entry_guard BEFORE UPDATE ON download_entries
WHEN EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state IN ('preparing','swapping') AND OLD.entry_id IN(m.target_entry_id,m.source_entry_id))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_job_guard BEFORE UPDATE ON download_jobs
WHEN EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state IN ('preparing','swapping') AND OLD.entry_id IN(m.target_entry_id,m.source_entry_id))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_artifact_guard BEFORE UPDATE ON download_artifacts
WHEN EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state IN ('preparing','swapping') AND OLD.entry_id IN(m.target_entry_id,m.source_entry_id))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_page_guard BEFORE UPDATE ON download_pages
WHEN EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state IN ('preparing','swapping') AND OLD.entry_id IN(m.target_entry_id,m.source_entry_id))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_page_delete_guard BEFORE DELETE ON download_pages
WHEN EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state IN ('preparing','swapping') AND OLD.entry_id IN(m.target_entry_id,m.source_entry_id))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_quarantine_insert_guard BEFORE INSERT ON quarantine_records
WHEN EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state IN ('preparing','swapping') AND NEW.entry_id IN(m.target_entry_id,m.source_entry_id))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_quarantine_update_guard BEFORE UPDATE ON quarantine_records
WHEN EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state IN ('preparing','swapping') AND NEW.entry_id IN(m.target_entry_id,m.source_entry_id))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_page_quarantine_insert_guard BEFORE INSERT ON page_quarantine_records
WHEN EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state IN ('preparing','swapping') AND NEW.entry_id IN(m.target_entry_id,m.source_entry_id))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_page_quarantine_update_guard BEFORE UPDATE ON page_quarantine_records
WHEN EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state IN ('preparing','swapping') AND NEW.entry_id IN(m.target_entry_id,m.source_entry_id))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_plan_guard BEFORE UPDATE ON internal_removal_plans
WHEN EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state IN ('preparing','swapping') AND NEW.entry_id IN(m.target_entry_id,m.source_entry_id))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_relocation_insert_guard BEFORE INSERT ON excluded_artifact_relocations
WHEN EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state IN ('preparing','swapping') AND NEW.entry_id IN(m.target_entry_id,m.source_entry_id))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_relocation_update_guard BEFORE UPDATE ON excluded_artifact_relocations
WHEN EXISTS(SELECT 1 FROM overlap_page_merges m WHERE m.state IN ('preparing','swapping') AND NEW.entry_id IN(m.target_entry_id,m.source_entry_id))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_scan_guard BEFORE INSERT ON duplicate_scan_runs
WHEN NEW.state='running' AND EXISTS(SELECT 1 FROM overlap_page_merges WHERE state IN ('preparing','swapping'))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
CREATE TRIGGER overlap_merge_internal_scan_guard BEFORE INSERT ON internal_duplicate_runs
WHEN NEW.state='running' AND EXISTS(SELECT 1 FROM overlap_page_merges WHERE state IN ('preparing','swapping'))
BEGIN SELECT RAISE(ABORT,'overlap page merge is active'); END;
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadOverlapMergeSide {
    Existing,
    Incoming,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DownloadOverlapMergeRequest {
    pub review_id: String,
    pub expected_revision: u64,
    pub candidate_id: String,
    pub source_side: DownloadOverlapMergeSide,
    pub source_pages: Vec<u32>,
    #[serde(default)]
    pub exclude_source: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOverlapMergeResult {
    pub merge_id: String,
    pub source_gallery_id: GalleryId,
    pub target_gallery_id: GalleryId,
    pub replaced_pages: usize,
    pub backup_path: String,
    pub affected_review_ids: Vec<String>,
    pub source_excluded: bool,
    #[serde(skip)]
    pub(crate) resume_jobs: Vec<DownloadJobDescriptor>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Replacement {
    source_page: u32,
    target_page: u32,
    target_relative_path: String,
    original_sha256: String,
    original_size: u64,
    original_source_revision: String,
    source_sha256: String,
    source_size: u64,
    source_revision: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    merge_id: String,
    review_id: String,
    review_revision: u64,
    source_entry_id: String,
    source_gallery_id: i64,
    source_artifact_revision: u64,
    target_entry_id: String,
    target_gallery_id: i64,
    target_artifact_revision: u64,
    target_root: String,
    target_directory: String,
    operation_directory: String,
    exclude_source: bool,
    replacements: Vec<Replacement>,
    // Whole-tree digest prevents recovery from overwriting files changed outside Atsumi.
    original_tree: BTreeMap<String, String>,
    merged_tree: BTreeMap<String, String>,
}
pub struct OverlapMergeService {
    repository: Arc<SqliteRepository>,
    gate: Mutex<()>,
}
// WAL NORMAL is sufficient for ordinary resumable downloads, but a filesystem
// swap must never survive a power loss without its preceding journal commit.
struct DurableWrites {
    repository: Arc<SqliteRepository>,
    previous: i64,
}
impl DurableWrites {
    fn begin(repository: &Arc<SqliteRepository>) -> Result<Self, ApplicationError> {
        let connection = repository.connection()?;
        let previous = connection
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .map_err(sql)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(sql)?;
        Ok(Self {
            repository: repository.clone(),
            previous,
        })
    }
}
impl Drop for DurableWrites {
    fn drop(&mut self) {
        if let Ok(connection) = self.repository.connection() {
            if let Err(error) = connection.pragma_update(None, "synchronous", self.previous) {
                tracing::warn!(%error,"SQLite kept stronger durability after page merge");
            }
        }
    }
}
impl OverlapMergeService {
    pub fn new(repository: Arc<SqliteRepository>) -> Self {
        Self {
            repository,
            gate: Mutex::new(()),
        }
    }

    /// The download supervisor's overlap/finalization lock must enclose apply.
    pub fn apply(
        &self,
        request: DownloadOverlapMergeRequest,
    ) -> Result<DownloadOverlapMergeResult, ApplicationError> {
        let started = Instant::now();
        let mut checkpoint = started;
        let mut timings = Vec::new();
        let mut mark = |stage: &str| {
            let now = Instant::now();
            timings.push(serde_json::json!({"stage": stage, "elapsedMs": now.duration_since(checkpoint).as_millis()}));
            checkpoint = now;
        };
        let _gate = self
            .gate
            .lock()
            .map_err(|_| invalid("The merge service lock is unavailable"))?;
        let _durable = DurableWrites::begin(&self.repository)?;
        let review = self
            .repository
            .overlap_review_get(&request.review_id)?
            .ok_or_else(|| {
                ApplicationError::DownloadOverlapReviewNotFound(request.review_id.clone())
            })?;
        let candidate = validate_request(&review, &request)?;
        let incoming = self.bundle(&review.entry_id)?;
        let existing = self.bundle(&candidate.existing.entry_id)?;
        if overlap_artifact_fingerprint(&incoming, review.profile_version).as_deref()
            != Some(review.incoming_fingerprint.as_str())
            || overlap_artifact_fingerprint(&existing, review.profile_version).as_deref()
                != Some(candidate.existing_fingerprint.as_str())
        {
            return Err(invalid(
                "The comparison pages changed; reopen the comparison before merging",
            ));
        }
        let (source, mut target) = match request.source_side {
            DownloadOverlapMergeSide::Existing => (existing, incoming),
            DownloadOverlapMergeSide::Incoming => (incoming, existing),
        };
        let mapping = derive_mapping(
            candidate,
            &request,
            source.artifact.expected_page_count,
            target.artifact.expected_page_count,
        )?;
        let source_root = self
            .repository
            .pipeline_artifact_root(&source.artifact.entry_id)?;
        let target_root = self
            .repository
            .pipeline_artifact_root(&target.artifact.entry_id)?;
        let target_directory =
            checked_path(&target_root, target.artifact.relative_directory.as_str())?
                .canonicalize()
                .map_err(io)?;
        let source_directory =
            checked_path(&source_root, source.artifact.relative_directory.as_str())?
                .canonicalize()
                .map_err(io)?;
        if target_directory == source_directory
            || target_directory.starts_with(&source_directory)
            || source_directory.starts_with(&target_directory)
        {
            return Err(invalid("Source and target folders must be independent"));
        }
        let merge_id = format!("merge-{}", Uuid::new_v4());
        let operation_directory = format!(".atsumi-page-merges/{merge_id}");
        let mut journal = Journal {
            merge_id: merge_id.clone(),
            review_id: review.review_id.clone(),
            review_revision: review.revision,
            source_entry_id: source.artifact.entry_id.to_string(),
            source_gallery_id: source.gallery.id.get(),
            source_artifact_revision: source.artifact.revision,
            target_entry_id: target.artifact.entry_id.to_string(),
            target_gallery_id: target.gallery.id.get(),
            target_artifact_revision: target.artifact.revision,
            target_root: target_root.to_string_lossy().into_owned(),
            target_directory: target.artifact.relative_directory.to_string(),
            operation_directory,
            exclude_source: request.exclude_source,
            replacements: Vec::new(),
            original_tree: BTreeMap::new(),
            merged_tree: BTreeMap::new(),
        };
        for (source_number, target_number) in mapping {
            let from = page(&source, source_number)?;
            let to = page(&target, target_number)?;
            journal.replacements.push(Replacement {
                source_page: source_number,
                target_page: target_number,
                target_relative_path: to.relative_path.to_string(),
                original_sha256: to.sha256.as_ref().unwrap().to_string(),
                original_size: to.byte_length.unwrap(),
                original_source_revision: to.source_revision.clone().unwrap(),
                source_sha256: from.sha256.as_ref().unwrap().to_string(),
                source_size: from.byte_length.unwrap(),
                source_revision: from.source_revision.clone().unwrap(),
            });
        }
        self.claim(&journal)?;
        mark("validate_and_claim");
        let outcome = (|| {
            verify_bundle(&source_root, &source)?;
            verify_bundle(&target_root, &target)?;
            journal.original_tree = tree_digest(&target_directory)?;
            mark("verify_originals");
            let operation = checked_path(&target_root, &journal.operation_directory)?;
            create_private_directory(&operation)?;
            let staging = operation.join("merged");
            copy_tree(&target_directory, &staging)?;
            for replacement in &journal.replacements {
                let from = page(&source, replacement.source_page)?;
                let source_path = checked_path(&source_root, from.relative_path.as_str())?;
                let target_page = target
                    .pages
                    .iter_mut()
                    .find(|p| p.page_id.source_page_number.get() == replacement.target_page)
                    .unwrap();
                let within = relative_inside(
                    target_page.relative_path.as_str(),
                    target.artifact.relative_directory.as_str(),
                )?;
                copy_verified_replace(
                    &source_path,
                    &staging.join(within),
                    &replacement.source_sha256,
                    replacement.source_size,
                )?;
                target_page.sha256 = Some(ArtifactSha256::new(replacement.source_sha256.clone())?);
                target_page.byte_length = Some(replacement.source_size);
                // Preserve target remote identity; donor revision belongs only in provenance.
            }
            if target.artifact.state == DownloadArtifactState::Complete {
                let manifest = ArtifactManifest::from_bundle(&target)?;
                let path = target
                    .artifact
                    .manifest_relative_path
                    .as_ref()
                    .ok_or_else(|| invalid("Completed target manifest is missing"))?;
                let within =
                    relative_inside(path.as_str(), target.artifact.relative_directory.as_str())?;
                write_synced(
                    &staging.join(within),
                    &serde_json::to_vec_pretty(&manifest).map_err(serialization)?,
                )?;
            } else if target
                .artifact
                .manifest_relative_path
                .as_ref()
                .is_some_and(|path| target_root.join(path.as_str()).exists())
            {
                return Err(invalid(
                    "An incomplete target unexpectedly has a manifest; reconcile it first",
                ));
            }
            journal.merged_tree = tree_digest(&staging)?;
            mark("copy_replace_and_verify_staging");
            self.update_journal(&journal, "swapping")?;
            // Copying can take time. Recheck both originals immediately before the swap.
            verify_bundle(&source_root, &source)?;
            if tree_digest(&target_directory)? != journal.original_tree {
                return Err(invalid("Target files changed while staging the merge"));
            }
            mark("recheck_originals");
            rename_no_replace(&target_directory, &operation.join("original"))?;
            rename_no_replace(&staging, &target_directory)?;
            if tree_digest(&target_directory)? != journal.merged_tree {
                return Err(invalid(
                    "The swapped target failed its final whole-folder verification",
                ));
            }
            mark("swap_and_verify");
            let result = self.commit(&journal);
            mark("database_commit");
            result
        })();
        drop(mark);
        match outcome {
            Ok(result) => {
                let diagnostic = serde_json::json!({"mergeId": journal.merge_id, "totalMs": started.elapsed().as_millis(), "targetFiles": journal.original_tree.len(), "replacedPages": journal.replacements.len(), "stages": timings});
                let path = Path::new(&journal.target_root)
                    .join(&journal.operation_directory)
                    .join("timings.json");
                if let Err(error) = serde_json::to_vec_pretty(&diagnostic)
                    .map_err(serialization)
                    .and_then(|bytes| {
                        let mut file = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&path)
                            .map_err(io)?;
                        file.write_all(&bytes).map_err(io)?;
                        file.sync_all().map_err(io)
                    })
                {
                    tracing::warn!(merge_id=%journal.merge_id, %error, "merge timing diagnostic could not be saved after commit");
                }
                Ok(result)
            }
            Err(error) => {
                if let Err(recovery) = self.rollback(&journal) {
                    return Err(invalid(&format!("Merge did not commit ({error}); recovery remains pending ({recovery}). Original files are preserved in {}",journal.operation_directory)));
                }
                Err(error)
            }
        }
    }

    /// Must run before any download/artifact reconciliation on application startup.
    pub fn recover_pending(&self) -> Result<usize, ApplicationError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| invalid("The merge recovery lock is unavailable"))?;
        let _durable = DurableWrites::begin(&self.repository)?;
        let journals = {
            let connection = self.repository.connection()?;
            let mut statement=connection.prepare("SELECT journal_json FROM overlap_page_merges WHERE state NOT IN ('applied','rolled_back') ORDER BY created_at").map_err(sql)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql)?;
            rows
        };
        for json in &journals {
            self.rollback(&serde_json::from_str(json).map_err(serialization)?)?;
        }
        Ok(journals.len())
    }
    fn bundle(&self, id: &str) -> Result<ArtifactBundle, ApplicationError> {
        self.repository
            .artifact_bundle_get(&DownloadEntryId::new(id)?)?
            .ok_or_else(|| invalid("The merge artifact no longer exists"))
    }
    fn claim(&self, journal: &Journal) -> Result<(), ApplicationError> {
        let mut connection = self.repository.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        validate_live(&transaction, journal)?;
        let active:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM internal_duplicate_runs WHERE state='running') OR EXISTS(SELECT 1 FROM duplicate_scan_runs WHERE state='running') OR EXISTS(SELECT 1 FROM quarantine_records WHERE entry_id IN(?1,?2) AND state IN('pending_quarantine','pending_restore')) OR EXISTS(SELECT 1 FROM page_quarantine_records WHERE entry_id IN(?1,?2) AND state<>'restored') OR EXISTS(SELECT 1 FROM excluded_artifact_relocations WHERE entry_id IN(?1,?2) AND state<>'restored') OR EXISTS(SELECT 1 FROM internal_removal_plans WHERE entry_id IN(?1,?2) AND state='applying')",params![journal.target_entry_id,journal.source_entry_id],|row|row.get(0)).map_err(sql)?;
        if active {
            return Err(invalid(
                "Wait for duplicate scanning or artifact recovery to finish before merging",
            ));
        }
        transaction.execute("INSERT INTO overlap_page_merges(merge_id,review_id,target_entry_id,source_entry_id,state,journal_json,created_at,updated_at) VALUES(?1,?2,?3,?4,'preparing',?5,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![journal.merge_id,journal.review_id,journal.target_entry_id,journal.source_entry_id,serde_json::to_string(journal).map_err(serialization)?]).map_err(sql)?;
        transaction.commit().map_err(sql)?;
        Ok(())
    }
    fn update_journal(&self, journal: &Journal, state: &str) -> Result<(), ApplicationError> {
        self.repository.connection()?.execute("UPDATE overlap_page_merges SET state=?2,journal_json=?3,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE merge_id=?1 AND state IN('preparing','swapping')",params![journal.merge_id,state,serde_json::to_string(journal).map_err(serialization)?]).map_err(sql)?;
        Ok(())
    }
    fn rollback(&self, journal: &Journal) -> Result<(), ApplicationError> {
        {
            let connection = self.repository.connection()?;
            let row: Option<(String,String,String,String)> = connection.query_row("SELECT state,review_id,target_entry_id,source_entry_id FROM overlap_page_merges WHERE merge_id=?1",[&journal.merge_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(sql)?;
            let Some((state, review, target, source)) = row else {
                return Err(invalid(
                    "The durable merge record is missing; recovery was paused",
                ));
            };
            if state == "applied" || state == "rolled_back" {
                return Ok(());
            }
            if review != journal.review_id
                || target != journal.target_entry_id
                || source != journal.source_entry_id
                || journal.operation_directory
                    != format!(".atsumi-page-merges/{}", journal.merge_id)
                || !journal
                    .merge_id
                    .strip_prefix("merge-")
                    .is_some_and(|id| Uuid::parse_str(id).is_ok())
            {
                return Err(invalid(
                    "The merge journal identity is inconsistent; recovery was paused",
                ));
            }
            let valid:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM download_artifacts WHERE entry_id=?1 AND gallery_id=?2 AND revision=?3 AND root_snapshot=?4 AND relative_directory=?5)",params![journal.target_entry_id,journal.target_gallery_id,journal.target_artifact_revision,journal.target_root,journal.target_directory],|row|row.get(0)).map_err(sql)?;
            if !valid {
                return Err(invalid("The original artifact no longer agrees with its merge journal; recovery was paused"));
            }
        }
        let root = Path::new(&journal.target_root);
        let target = checked_path(root, &journal.target_directory)?;
        let operation = checked_path(root, &journal.operation_directory)?;
        let backup = operation.join("original");
        if backup.exists() {
            if journal.original_tree.is_empty() || tree_digest(&backup)? != journal.original_tree {
                return Err(invalid(
                    "The original merge backup could not be verified; recovery was paused",
                ));
            }
            if target.exists() {
                if journal.merged_tree.is_empty() || tree_digest(&target)? != journal.merged_tree {
                    return Err(invalid(
                        "The replaced folder changed externally; recovery will not overwrite it",
                    ));
                }
                rename_no_replace(&target, &operation.join("uncommitted-new"))?;
            }
            rename_no_replace(&backup, &target)?;
        } else if !target.exists() {
            return Err(invalid(
                "Neither original target nor backup is available; recovery was paused",
            ));
        } else if !journal.original_tree.is_empty() {
            if tree_digest(&target)? != journal.original_tree {
                return Err(invalid("The original backup is missing and the target is not the original tree; recovery was paused"));
            }
        } else {
            // A crash before the initial tree snapshot can only release its
            // lease when the untouched database checkpoint still verifies.
            verify_bundle(root, &self.bundle(&journal.target_entry_id)?)?;
        }
        self.repository.connection()?.execute("UPDATE overlap_page_merges SET state='rolled_back',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE merge_id=?1 AND state NOT IN('applied','rolled_back')",[&journal.merge_id]).map_err(sql)?;
        Ok(())
    }
    fn commit(&self, journal: &Journal) -> Result<DownloadOverlapMergeResult, ApplicationError> {
        let mut connection = self.repository.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        validate_live(&transaction, journal)?;
        if transaction.execute("UPDATE overlap_page_merges SET state='committing' WHERE merge_id=?1 AND state='swapping'",[&journal.merge_id]).map_err(sql)?!=1 { return Err(invalid("The merge operation is no longer pending")); }
        for replacement in &journal.replacements {
            let changed=transaction.execute("UPDATE download_pages SET sha256=?3,byte_length=?4,verified_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE entry_id=?1 AND source_page_number=?2 AND sha256=?5 AND byte_length=?6 AND source_revision=?7 AND state='present' AND excluded=0",params![journal.target_entry_id,replacement.target_page,replacement.source_sha256,replacement.source_size,replacement.original_sha256,replacement.original_size,replacement.original_source_revision]).map_err(sql)?;
            if changed != 1 {
                return Err(invalid("The replacement page changed before commit"));
            }
        }
        transaction
            .execute(
                "UPDATE download_artifacts SET revision=revision+1 WHERE entry_id=?1",
                [&journal.target_entry_id],
            )
            .map_err(sql)?;
        transaction.execute("UPDATE download_entries SET revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE entry_id=?1 AND state='completed'",[&journal.target_entry_id]).map_err(sql)?;
        transaction.execute("UPDATE download_jobs SET revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE entry_id=?1 AND state='completed'",[&journal.target_entry_id]).map_err(sql)?;
        for replacement in &journal.replacements {
            // Hash identity is page bytes + profile, not the album revision.
            // Keep every unchanged page, including an exact-byte replacement.
            transaction.execute(
                "DELETE FROM duplicate_page_hashes WHERE entry_id=?1 AND source_page_number=?2 AND artifact_sha256<>?3",
                params![journal.target_entry_id, replacement.target_page, replacement.source_sha256],
            ).map_err(sql)?;
            // A copied donor page has exactly the same pixels. Reuse its verified
            // hash under the target identity; absent/stale donor hashes stay misses.
            transaction.execute(
                "INSERT OR IGNORE INTO duplicate_page_hashes
                 (entry_id,gallery_id,source_page_number,profile_version,artifact_sha256,coarse_d_hash_hex,detail_d_hash_hex,p_hash_hex,mean_luma,std_dev,non_uniform_ratio,edge_density,width,height,low_information,computed_at)
                 SELECT ?1,?2,?3,profile_version,artifact_sha256,coarse_d_hash_hex,detail_d_hash_hex,p_hash_hex,mean_luma,std_dev,non_uniform_ratio,edge_density,width,height,low_information,computed_at
                 FROM duplicate_page_hashes WHERE entry_id=?4 AND source_page_number=?5 AND artifact_sha256=?6",
                params![journal.target_entry_id, journal.target_gallery_id, replacement.target_page, journal.source_entry_id, replacement.source_page, replacement.source_sha256],
            ).map_err(sql)?;
        }
        transaction.execute("UPDATE duplicate_candidates SET resolved=1,artifact_stale=1,revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE (parent_entry_id=?1 OR candidate_entry_id=?1) AND resolved=0",[&journal.target_entry_id]).map_err(sql)?;
        transaction.execute("UPDATE gallery_previews SET candidates_json='[]',algorithm_version=0,analysis_attempted_at=NULL,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE gallery_id=?1",[journal.target_gallery_id]).map_err(sql)?;
        transaction.execute("UPDATE internal_duplicate_groups SET resolved=1,revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE entry_id=?1 AND resolved=0",[&journal.target_entry_id]).map_err(sql)?;
        transaction.execute("UPDATE internal_removal_plans SET state='cancelled',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE entry_id=?1 AND state='prepared'",[&journal.target_entry_id]).map_err(sql)?;
        if journal.exclude_source {
            transaction
                .execute(
                    "DELETE FROM exploration_restored_galleries WHERE gallery_id=?1",
                    [journal.source_gallery_id],
                )
                .map_err(sql)?;
            transaction.execute("INSERT INTO duplicate_hidden_galleries(gallery_id,decision_id,created_at) VALUES(?1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(gallery_id) DO UPDATE SET decision_id=excluded.decision_id,created_at=excluded.created_at",params![journal.source_gallery_id,journal.merge_id]).map_err(sql)?;
            transaction.execute("UPDATE download_entries SET state='cancelled',revision=revision+1,review_id=NULL,review_kind=NULL,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE entry_id=?1",[&journal.source_entry_id]).map_err(sql)?;
            transaction.execute("UPDATE download_jobs SET state='cancelled',revision=revision+1,last_error_code=NULL,last_error_message=NULL,last_error_retryable=NULL,finished_at=COALESCE(finished_at,strftime('%Y-%m-%dT%H:%M:%fZ','now')),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE entry_id=?1",[&journal.source_entry_id]).map_err(sql)?;
            transaction.execute("UPDATE download_attempts SET outcome_state='cancelled',finished_at=COALESCE(finished_at,strftime('%Y-%m-%dT%H:%M:%fZ','now')) WHERE EXISTS(SELECT 1 FROM download_jobs job WHERE job.entry_id=?1 AND job.job_id=download_attempts.job_id AND job.attempt=download_attempts.attempt)",[&journal.source_entry_id]).map_err(sql)?;
        }
        let related = {
            let mut statement=transaction.prepare("SELECT DISTINCT r.review_id,r.entry_id FROM download_overlap_reviews r LEFT JOIN download_overlap_candidates c ON c.review_id=r.review_id WHERE r.state='pending' AND (r.entry_id=?1 OR c.existing_entry_id=?1 OR (?3=1 AND (r.entry_id=?2 OR c.existing_entry_id=?2)))").map_err(sql)?;
            let items = statement
                .query_map(
                    params![
                        journal.target_entry_id,
                        journal.source_entry_id,
                        journal.exclude_source
                    ],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .map_err(sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql)?;
            items
        };
        let mut resume_jobs = Vec::new();
        for (review_id, entry_id) in &related {
            transaction.execute("UPDATE download_overlap_reviews SET state=?2,revision=revision+1,resolved_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE review_id=?1",params![review_id,if journal.exclude_source && entry_id==&journal.source_entry_id {"cancelled"}else{"stale"}]).map_err(sql)?;
            if !(journal.exclude_source && entry_id == &journal.source_entry_id) {
                let eligible:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM download_entries WHERE entry_id=?1 AND state='review_required' AND review_id=?2)",params![entry_id,review_id],|row|row.get(0)).map_err(sql)?;
                if eligible {
                    resume_jobs.push(
                        super::sqlite_repository::requeue_overlap_target(&transaction, entry_id)?.1,
                    );
                }
            }
        }
        transaction.execute("UPDATE overlap_page_merges SET state='applied',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE merge_id=?1",[&journal.merge_id]).map_err(sql)?;
        transaction.commit().map_err(sql)?;
        Ok(DownloadOverlapMergeResult {
            merge_id: journal.merge_id.clone(),
            source_gallery_id: GalleryId::new(journal.source_gallery_id)?,
            target_gallery_id: GalleryId::new(journal.target_gallery_id)?,
            replaced_pages: journal.replacements.len(),
            backup_path: Path::new(&journal.target_root)
                .join(&journal.operation_directory)
                .join("original")
                .to_string_lossy()
                .into_owned(),
            affected_review_ids: related.into_iter().map(|item| item.0).collect(),
            source_excluded: journal.exclude_source,
            resume_jobs,
        })
    }
}

fn validate_request<'a>(
    review: &'a DownloadOverlapReview,
    request: &DownloadOverlapMergeRequest,
) -> Result<&'a crate::domain::DownloadOverlapCandidate, ApplicationError> {
    if review.revision != request.expected_revision {
        return Err(ApplicationError::RevisionConflict {
            resource: "overlap review",
            expected: request.expected_revision,
            actual: review.revision,
        });
    }
    if review.state != DownloadOverlapReviewState::Pending {
        return Err(invalid("Only an active comparison can be merged"));
    }
    if request.source_pages.is_empty()
        || request.source_pages.len() > 200
        || request.source_pages.iter().any(|page| *page == 0)
        || request.source_pages.iter().collect::<BTreeSet<_>>().len() != request.source_pages.len()
    {
        return Err(invalid("Select unique valid pages on exactly one side"));
    }
    let candidate = review
        .candidates
        .iter()
        .find(|item| item.candidate_id == request.candidate_id && item.decision.is_none())
        .ok_or_else(|| invalid("The selected comparison is no longer pending"))?;
    if review.entry_id == candidate.existing.entry_id
        || review.incoming.gallery_id == candidate.existing.gallery_id
    {
        return Err(invalid("An album cannot be merged into itself"));
    }
    Ok(candidate)
}
fn derive_mapping(
    candidate: &crate::domain::DownloadOverlapCandidate,
    request: &DownloadOverlapMergeRequest,
    source_count: u32,
    target_count: u32,
) -> Result<Vec<(u32, u32)>, ApplicationError> {
    let pairs = candidate
        .page_pairs
        .iter()
        .map(|pair| match request.source_side {
            DownloadOverlapMergeSide::Incoming => {
                (pair.incoming_source_page, pair.existing_source_page)
            }
            DownloadOverlapMergeSide::Existing => {
                (pair.existing_source_page, pair.incoming_source_page)
            }
        })
        .collect::<Vec<_>>();
    if pairs.iter().any(|(source, target)| {
        *source == 0 || *source > source_count || *target == 0 || *target > target_count
    }) {
        return Err(invalid(
            "Stored page correspondence is outside the current album bounds",
        ));
    }
    let mut output = Vec::new();
    let mut targets = BTreeSet::new();
    for source in &request.source_pages {
        let matching = pairs
            .iter()
            .filter(|pair| pair.0 == *source)
            .collect::<Vec<_>>();
        if matching.len() != 1
            || pairs.iter().filter(|pair| pair.1 == matching[0].1).count() != 1
            || !targets.insert(matching[0].1)
        {
            return Err(invalid("Every selected source page must have exactly one unique stored target correspondence"));
        }
        output.push(**matching.first().unwrap());
    }
    if request.exclude_source {
        let source_pages = pairs.iter().map(|pair| pair.0).collect::<BTreeSet<_>>();
        let unique_targets = pairs.iter().map(|pair| pair.1).collect::<BTreeSet<_>>();
        if pairs.len() != source_count as usize
            || source_pages != (1..=source_count).collect()
            || unique_targets.len() != pairs.len()
        {
            return Err(invalid(
                "The source has unmatched pages and cannot be excluded by this merge",
            ));
        }
    }
    Ok(output)
}
fn validate_live(connection: &Connection, journal: &Journal) -> Result<(), ApplicationError> {
    completed_pair::check_origin(connection, &journal.review_id)?;
    let review: Option<(u64, String)> = connection
        .query_row(
            "SELECT revision,state FROM download_overlap_reviews WHERE review_id=?1",
            [&journal.review_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    if review != Some((journal.review_revision, "pending".into())) {
        return Err(invalid("The overlap review changed before merging"));
    }
    for (entry, revision, gallery) in [
        (
            &journal.source_entry_id,
            journal.source_artifact_revision,
            journal.source_gallery_id,
        ),
        (
            &journal.target_entry_id,
            journal.target_artifact_revision,
            journal.target_gallery_id,
        ),
    ] {
        let eligible:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM download_entries d JOIN download_artifacts a ON a.entry_id=d.entry_id WHERE d.entry_id=?1 AND d.gallery_id=?2 AND a.revision=?3 AND ((d.state='completed' AND a.state='complete') OR (d.state='review_required' AND a.state='incomplete' AND d.review_kind='gallery_duplicate' AND EXISTS(SELECT 1 FROM download_overlap_reviews r WHERE r.review_id=d.review_id AND r.state='pending'))) AND NOT EXISTS(SELECT 1 FROM duplicate_hidden_galleries h WHERE h.gallery_id=d.gallery_id AND NOT EXISTS(SELECT 1 FROM exploration_restored_galleries x WHERE x.gallery_id=h.gallery_id)))",params![entry,gallery,revision],|row|row.get(0)).map_err(sql)?;
        if !eligible {
            return Err(invalid(
                "The source or target is no longer a live verified album",
            ));
        }
    }
    Ok(())
}
fn page(bundle: &ArtifactBundle, number: u32) -> Result<&PageArtifact, ApplicationError> {
    bundle
        .pages
        .iter()
        .find(|page| {
            page.page_id.source_page_number.get() == number
                && page.state == PageArtifactState::Present
                && !page.excluded
                && page.sha256.is_some()
                && page.byte_length.is_some()
                && page.source_revision.is_some()
        })
        .ok_or_else(|| invalid("A selected page is not verified"))
}
fn verify_bundle(root: &Path, bundle: &ArtifactBundle) -> Result<(), ApplicationError> {
    for item in &bundle.pages {
        let item = page(bundle, item.page_id.source_page_number.get())?;
        let path = checked_path(root, item.relative_path.as_str())?;
        verify_file(
            &path,
            item.sha256.as_ref().unwrap().as_str(),
            item.byte_length.unwrap(),
        )?;
    }
    if bundle.artifact.state == DownloadArtifactState::Complete {
        let expected = ArtifactManifest::from_bundle(bundle)?;
        let path = checked_path(
            root,
            bundle
                .artifact
                .manifest_relative_path
                .as_ref()
                .unwrap()
                .as_str(),
        )?;
        let actual: ArtifactManifest =
            serde_json::from_reader(File::open(path).map_err(io)?).map_err(serialization)?;
        if actual != expected {
            return Err(invalid(
                "The source or target manifest does not match its verified database",
            ));
        }
    }
    Ok(())
}
fn invalid(message: &str) -> ApplicationError {
    ApplicationError::DownloadOverlapDecisionInvalid(message.into())
}
fn sql(error: rusqlite::Error) -> ApplicationError {
    RepositoryError::Other(error.to_string()).into()
}
fn io(error: std::io::Error) -> ApplicationError {
    invalid(&format!("The merge filesystem operation failed: {error}"))
}
fn serialization(error: serde_json::Error) -> ApplicationError {
    invalid(&format!(
        "The merge journal or manifest is invalid: {error}"
    ))
}
fn relative_inside<'a>(path: &'a str, directory: &str) -> Result<&'a str, ApplicationError> {
    path.strip_prefix(directory)
        .and_then(|suffix| suffix.strip_prefix('/'))
        .filter(|suffix| !suffix.is_empty())
        .ok_or_else(|| invalid("An artifact page lies outside its album folder"))
}
fn checked_path(root: &Path, relative: &str) -> Result<PathBuf, ApplicationError> {
    let relative = ArtifactRelativePath::new(relative)?;
    let root = root.canonicalize().map_err(io)?;
    let mut path = root.clone();
    for part in Path::new(relative.as_str()).components() {
        path.push(part);
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            reject_link(&metadata)?;
            if !path.canonicalize().map_err(io)?.starts_with(&root) {
                return Err(invalid("The merge path escaped its download root"));
            }
        }
    }
    Ok(path)
}
fn reject_link(metadata: &fs::Metadata) -> Result<(), ApplicationError> {
    let mut link = metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        link |= metadata.file_attributes() & 0x400 != 0;
    }
    if link {
        Err(invalid(
            "Merge folders cannot contain symlinks or reparse points",
        ))
    } else {
        Ok(())
    }
}
fn create_private_directory(path: &Path) -> Result<(), ApplicationError> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Missing merge parent"))?;
    if !parent.exists() {
        fs::create_dir(parent).map_err(io)?;
    }
    reject_link(&fs::symlink_metadata(parent).map_err(io)?)?;
    fs::create_dir(path).map_err(io)
}
fn tree_digest(root: &Path) -> Result<BTreeMap<String, String>, ApplicationError> {
    fn visit(
        base: &Path,
        path: &Path,
        out: &mut BTreeMap<String, String>,
    ) -> Result<(), ApplicationError> {
        let metadata = fs::symlink_metadata(path).map_err(io)?;
        reject_link(&metadata)?;
        if metadata.is_dir() {
            if path != base {
                out.insert(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    "directory".into(),
                );
            }
            for item in fs::read_dir(path).map_err(io)? {
                visit(base, &item.map_err(io)?.path(), out)?;
            }
        } else if metadata.is_file() {
            out.insert(
                path.strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                digest_file(path)?,
            );
        } else {
            return Err(invalid("Merge folders contain an unsupported file type"));
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out)?;
    Ok(out)
}
fn digest_file(path: &Path) -> Result<String, ApplicationError> {
    let mut file = File::open(path).map_err(io)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(io)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn verify_file(path: &Path, sha: &str, size: u64) -> Result<(), ApplicationError> {
    let metadata = fs::symlink_metadata(path).map_err(io)?;
    reject_link(&metadata)?;
    if !metadata.is_file() || metadata.len() != size || digest_file(path)? != sha {
        return Err(invalid("A merge page failed its stored SHA-256/size check"));
    }
    Ok(())
}
fn copy_tree(source: &Path, target: &Path) -> Result<(), ApplicationError> {
    reject_link(&fs::symlink_metadata(source).map_err(io)?)?;
    fs::create_dir(target).map_err(io)?;
    for item in fs::read_dir(source).map_err(io)? {
        let item = item.map_err(io)?;
        let metadata = fs::symlink_metadata(item.path()).map_err(io)?;
        reject_link(&metadata)?;
        let destination = target.join(item.file_name());
        if metadata.is_dir() {
            copy_tree(&item.path(), &destination)?;
        } else if metadata.is_file() {
            fs::copy(item.path(), &destination).map_err(io)?;
            OpenOptions::new()
                .write(true)
                .open(&destination)
                .map_err(io)?
                .sync_all()
                .map_err(io)?;
        } else {
            return Err(invalid("Unsupported artifact file"));
        }
    }
    Ok(())
}
fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), ApplicationError> {
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)
}
fn copy_verified_replace(
    source: &Path,
    target: &Path,
    sha: &str,
    size: u64,
) -> Result<(), ApplicationError> {
    verify_file(source, sha, size)?;
    fs::copy(source, target).map_err(io)?;
    OpenOptions::new()
        .write(true)
        .open(target)
        .map_err(io)?
        .sync_all()
        .map_err(io)?;
    verify_file(target, sha, size)
}
fn rename_no_replace(source: &Path, target: &Path) -> Result<(), ApplicationError> {
    if target.exists() {
        return Err(invalid(
            "A merge recovery destination already exists; no files were overwritten",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH},
        };
        let source = source
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let target = target
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|error| invalid(&format!("The recoverable folder rename failed: {error}")))?;
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, target).map_err(io)?;
        if let Some(parent) = source.parent() {
            File::open(parent).map_err(io)?.sync_all().map_err(io)?;
        }
        if let Some(parent) = target.parent() {
            File::open(parent).map_err(io)?.sync_all().map_err(io)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        application::{DownloadQueueAddOutcome, DownloadRepository},
        domain::DownloadOverlapRelation,
    };
    struct Fixture {
        root: tempfile::TempDir,
        repository: Arc<SqliteRepository>,
        service: OverlapMergeService,
        incoming: String,
        existing: String,
    }

    #[test]
    fn completed_pair_retention_preserves_jobs_and_distinct_ground_truth() {
        for action in ["keep_both_continue", "false_positive_continue"] {
            let f = Fixture::completed_pair();
            let request = f
                .service
                .prepare_completed_decision(f.completed_decision(action))
                .unwrap();
            let result = f.service.decide_completed_pair(request).unwrap();
            assert!(!result.resumed && !result.cancelled);
            assert_eq!(result.review.state, DownloadOverlapReviewState::Resolved);
            let c = f.repository.connection().unwrap();
            let jobs: i64 = c
                .query_row(
                    "SELECT count(*) FROM download_jobs WHERE state='completed'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(jobs, 2);
            assert_eq!(
                c.query_row("SELECT action FROM download_overlap_decisions", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
                action
            );
            assert_eq!(
                c.query_row("SELECT count(*) FROM duplicate_pair_exclusions", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap(),
                1
            );
            assert_eq!(
                c.query_row("SELECT count(*) FROM duplicate_hidden_galleries", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            drop(c);
            assert!(f
                .service
                .prepare_completed_decision(f.completed_decision(action))
                .is_err());
        }
    }

    #[test]
    fn completed_pair_removal_excludes_only_selected_album_without_requeue() {
        for (action, selected) in [("remove_existing_continue", 101), ("remove_incoming", 102)] {
            let f = Fixture::completed_pair();
            let before_a = f.bytes(101, 1);
            let before_b = f.bytes(102, 1);
            let request = f
                .service
                .prepare_completed_decision(f.completed_decision(action))
                .unwrap();
            f.service.decide_completed_pair(request).unwrap();
            let c = f.repository.connection().unwrap();
            assert_eq!(
                c.query_row(
                    "SELECT gallery_id FROM duplicate_hidden_galleries",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                selected
            );
            assert_eq!(
                c.query_row(
                    "SELECT gallery_id FROM download_entries WHERE state='cancelled'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                selected
            );
            assert_eq!(
                c.query_row(
                    "SELECT count(*) FROM download_entries WHERE state='completed'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
            assert_eq!(f.bytes(101, 1), before_a);
            assert_eq!(f.bytes(102, 1), before_b);
        }
    }

    #[test]
    fn completed_pair_merge_uses_saved_coordinates_and_keeps_completed_target() {
        let f = Fixture::completed_pair();
        f.seed_hashes();
        let source = f.bytes(101, 2);
        let extra = f.bytes(102, 2);
        let request = DownloadOverlapMergeRequest {
            review_id: "duplicate:classic".into(),
            expected_revision: 0,
            candidate_id: "classic".into(),
            source_side: DownloadOverlapMergeSide::Existing,
            source_pages: vec![1, 2],
            exclude_source: true,
        };
        let request = f.service.prepare_completed_merge(request).unwrap();
        let result = f.service.apply(request).unwrap();
        assert_eq!(result.source_gallery_id.get(), 101);
        assert_eq!(result.target_gallery_id.get(), 102);
        assert!(result.resume_jobs.is_empty());
        assert!(result.source_excluded);
        assert_eq!(f.bytes(102, 3), source);
        assert_eq!(f.bytes(102, 2), extra);
        let c = f.repository.connection().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT state FROM download_entries WHERE gallery_id=102",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "completed"
        );
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM duplicate_page_hashes WHERE gallery_id=102",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            3
        );
        assert_eq!(
            c.query_row(
                "SELECT resolved FROM duplicate_candidates WHERE candidate_id='classic'",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap(),
            true
        );
    }

    #[test]
    fn completed_pair_rejects_concurrent_changed_evidence_and_missing_files() {
        let f = Fixture::completed_pair();
        let req = f
            .service
            .prepare_completed_decision(f.completed_decision("remove_incoming"))
            .unwrap();
        f.repository
            .connection()
            .unwrap()
            .execute(
                "UPDATE duplicate_candidates SET revision=revision+1 WHERE candidate_id='classic'",
                [],
            )
            .unwrap();
        assert!(f.service.decide_completed_pair(req).is_err());
        assert_eq!(
            f.repository
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM duplicate_hidden_galleries", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let f = Fixture::completed_pair();
        let req = f
            .service
            .prepare_completed_decision(f.completed_decision("remove_incoming"))
            .unwrap();
        fs::remove_file(f.root.path().join("album-102/0001.webp")).unwrap();
        assert!(f.service.decide_completed_pair(req).is_err());
    }

    #[test]
    fn completed_pair_rejects_wrong_candidate_and_non_human_decision_before_materializing() {
        let f = Fixture::completed_pair();
        let mut request = f.completed_decision("remove_incoming");
        request.candidate_id = Some("different".into());
        assert!(f.service.prepare_completed_decision(request).is_err());
        let mut request = f.completed_decision("remove_incoming");
        request.actor = crate::domain::DownloadOverlapDecisionActor::Automation;
        assert!(f.service.prepare_completed_decision(request).is_err());
        assert_eq!(
            f.repository
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM download_overlap_reviews", [], |r| r
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            1
        );
    }
    impl Fixture {
        fn completed_pair() -> Self {
            let f = Self::new(2, 3);
            {
                let c = f.repository.connection().unwrap();
                c.execute("UPDATE download_entries SET state='completed',review_id=NULL,review_kind=NULL WHERE entry_id=?1", [&f.incoming]).unwrap();
                c.execute(
                    "UPDATE download_jobs SET state='completed' WHERE entry_id=?1",
                    [&f.incoming],
                )
                .unwrap();
                c.execute("UPDATE download_artifacts SET state='complete',manifest_relative_path='album-101/manifest.json',manifest_schema_version=1,writer_version='fixture',completed_at='now' WHERE entry_id=?1", [&f.incoming]).unwrap();
                c.execute(
                    "UPDATE download_overlap_reviews SET state='resolved' WHERE review_id='review'",
                    [],
                )
                .unwrap();
                c.execute("INSERT INTO duplicate_scan_runs(run_id,revision,state,profile_version,total_artifacts,hashed_artifacts,total_pairs,compared_pairs,candidates_found,started_at,updated_at) VALUES('pair-run',0,'completed',1,2,2,1,1,1,'now','now')", []).unwrap();
                c.execute("INSERT INTO duplicate_candidates(candidate_id,revision,last_seen_run_id,profile_version,parent_gallery_id,parent_entry_id,candidate_gallery_id,candidate_entry_id,relation,confidence,matched_pages,parent_coverage,candidate_coverage,created_at,updated_at) VALUES('classic',0,'pair-run',1,101,?1,102,?2,'contains',0.99,2,1.0,0.666666,'now','now')", params![f.incoming,f.existing]).unwrap();
                for (a, b) in [(1, 1), (2, 3)] {
                    c.execute("INSERT INTO duplicate_page_pairs(candidate_id,parent_source_page,candidate_source_page,exact_sha256,d_hash_distance,p_hash_distance,detail_hash_distance,edge_similarity,visual_similarity,low_information) VALUES('classic',?1,?2,0,1,1,1,0.99,0.99,0)", params![a,b]).unwrap();
                }
            }
            let bundle = f.service.bundle(&f.incoming).unwrap();
            fs::write(
                f.root.path().join("album-101/manifest.json"),
                serde_json::to_vec(&ArtifactManifest::from_bundle(&bundle).unwrap()).unwrap(),
            )
            .unwrap();
            f
        }

        fn completed_decision(
            &self,
            action: &str,
        ) -> crate::domain::DownloadOverlapDecisionRequest {
            serde_json::from_value(serde_json::json!({"reviewId":"duplicate:classic", "expectedRevision":0,"candidateId":"classic","action":action})).unwrap()
        }
        fn new(incoming_count: u32, existing_count: u32) -> Self {
            let root = tempfile::tempdir().unwrap();
            let repository = Arc::new(SqliteRepository::open_in_memory().unwrap());
            let service = OverlapMergeService::new(repository.clone());
            let mut fixture = Self {
                root,
                repository,
                service,
                incoming: String::new(),
                existing: String::new(),
            };
            fixture.incoming = fixture.seed(101, incoming_count, false);
            fixture.existing = fixture.seed(102, existing_count, true);
            let incoming = fixture.service.bundle(&fixture.incoming).unwrap();
            let existing = fixture.service.bundle(&fixture.existing).unwrap();
            {
                let connection = fixture.repository.connection().unwrap();
                connection.execute("INSERT INTO download_overlap_reviews(review_id,entry_id,incoming_gallery_id,revision,state,profile_version,policy_version,incoming_fingerprint,created_at,updated_at) VALUES('review',?1,101,0,'pending',1,2,?2,'now','now')",params![fixture.incoming,overlap_artifact_fingerprint(&incoming,1).unwrap()]).unwrap();
                connection.execute("UPDATE download_entries SET review_id='review',review_kind='gallery_duplicate' WHERE entry_id=?1",[&fixture.incoming]).unwrap();
                connection.execute("INSERT INTO download_overlap_candidates(candidate_id,review_id,existing_entry_id,existing_gallery_id,existing_fingerprint,relation,confidence,matched_pages,exact_pages,visual_pages,existing_coverage,incoming_coverage,existing_unique_pages,incoming_unique_pages,longest_aligned_run,rank) VALUES('candidate','review',?1,102,?2,?3,0.99,2,0,2,?4,?5,?6,?7,2,1)",params![fixture.existing,overlap_artifact_fingerprint(&existing,1).unwrap(),if incoming_count>existing_count {"incoming_contains_existing"}else{"existing_contains_incoming"},2.0/existing_count as f64,2.0/incoming_count as f64,existing_count-2,incoming_count-2]).unwrap();
                for (index, incoming, existing) in [(0, 1, 1), (1, incoming_count, existing_count)]
                {
                    connection.execute("INSERT INTO download_overlap_page_pairs(candidate_id,pair_index,incoming_source_page,existing_source_page,exact_sha256,d_hash_distance,p_hash_distance,detail_hash_distance,edge_similarity,visual_similarity,low_information) VALUES('candidate',?1,?2,?3,0,1,1,1,0.99,0.99,0)",params![index,incoming,existing]).unwrap();
                }
            }
            fixture
        }
        fn seed(&self, id: i64, count: u32, complete: bool) -> String {
            let DownloadQueueAddOutcome::Added(queued) = self
                .repository
                .download_queue_add(&format!("fixture-{id}"), &[GalleryId::new(id).unwrap()])
                .unwrap()
            else {
                panic!("new queue");
            };
            let entry = queued.entries[0].entry_id.to_string();
            let folder = format!("album-{id}");
            fs::create_dir(self.root.path().join(&folder)).unwrap();
            {
                let connection = self.repository.connection().unwrap();
                connection.execute("INSERT INTO galleries(gallery_id,revision,title,source_page_count) VALUES(?1,0,'merge test',?2)",params![id,count]).unwrap();
                let state = if complete {
                    "completed"
                } else {
                    "review_required"
                };
                connection
                    .execute(
                        "UPDATE download_entries SET state=?2,progress=100 WHERE entry_id=?1",
                        params![entry, state],
                    )
                    .unwrap();
                connection.execute("UPDATE download_jobs SET state=?2,completed_units=?3,total_units=?3 WHERE entry_id=?1",params![entry,state,count]).unwrap();
                connection.execute("INSERT INTO download_artifacts(entry_id,gallery_id,revision,relative_directory,expected_page_count,state,root_snapshot) VALUES(?1,?2,0,?3,?4,?5,?6)",params![entry,id,folder,count,if complete {"complete"}else{"incomplete"},self.root.path().to_string_lossy()]).unwrap();
                for number in 1..=count {
                    let pixels = image::RgbaImage::from_pixel(
                        2,
                        2,
                        image::Rgba([id as u8, number as u8, 0, 255]),
                    );
                    let mut bytes = Vec::new();
                    image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
                        .encode(pixels.as_raw(), 2, 2, image::ExtendedColorType::Rgba8)
                        .unwrap();
                    let relative = format!("{folder}/{number:04}.webp");
                    fs::write(self.root.path().join(&relative), &bytes).unwrap();
                    connection.execute("INSERT INTO download_pages(entry_id,gallery_id,source_page_number,relative_path,state,byte_length,sha256,storage_format,source_revision,verified_at) VALUES(?1,?2,?3,?4,'present',?5,?6,'webp',?7,'now')",params![entry,id,number,relative,bytes.len(),format!("{:x}",Sha256::digest(&bytes)),format!("remote-{id}-{number}")]).unwrap();
                }
                if complete {
                    connection.execute("UPDATE download_artifacts SET manifest_relative_path=?2,manifest_schema_version=1,writer_version='fixture',completed_at='now' WHERE entry_id=?1",params![entry,format!("{folder}/manifest.json")]).unwrap();
                }
            }
            if complete {
                let bundle = self.service.bundle(&entry).unwrap();
                fs::write(
                    self.root.path().join(&folder).join("manifest.json"),
                    serde_json::to_vec(&ArtifactManifest::from_bundle(&bundle).unwrap()).unwrap(),
                )
                .unwrap();
            }
            entry
        }
        fn request(
            &self,
            side: DownloadOverlapMergeSide,
            exclude: bool,
        ) -> DownloadOverlapMergeRequest {
            DownloadOverlapMergeRequest {
                review_id: "review".into(),
                expected_revision: 0,
                candidate_id: "candidate".into(),
                source_side: side,
                source_pages: vec![1],
                exclude_source: exclude,
            }
        }
        fn bytes(&self, id: i64, page: u32) -> Vec<u8> {
            fs::read(self.root.path().join(format!("album-{id}/{page:04}.webp"))).unwrap()
        }
        fn seed_hashes(&self) {
            self.repository.connection().unwrap().execute(
                "INSERT INTO duplicate_page_hashes(entry_id,gallery_id,source_page_number,profile_version,artifact_sha256,coarse_d_hash_hex,detail_d_hash_hex,p_hash_hex,mean_luma,std_dev,non_uniform_ratio,edge_density,width,height,low_information,computed_at)
                 SELECT entry_id,gallery_id,source_page_number,1,sha256,?1,?2,?1,100,40,0.8,0.5,2,2,0,'original-cache' FROM download_pages",
                params!["0".repeat(16), "1".repeat(256)],
            ).unwrap();
        }
        fn pending(&self) -> Journal {
            let source = self.service.bundle(&self.incoming).unwrap();
            let target = self.service.bundle(&self.existing).unwrap();
            let id = format!("merge-{}", Uuid::new_v4());
            let mut journal = Journal {
                merge_id: id.clone(),
                review_id: "review".into(),
                review_revision: 0,
                source_entry_id: self.incoming.clone(),
                source_gallery_id: 101,
                source_artifact_revision: 0,
                target_entry_id: self.existing.clone(),
                target_gallery_id: 102,
                target_artifact_revision: 0,
                target_root: self.root.path().to_string_lossy().into_owned(),
                target_directory: "album-102".into(),
                operation_directory: format!(".atsumi-page-merges/{id}"),
                exclude_source: true,
                replacements: vec![],
                original_tree: tree_digest(&self.root.path().join("album-102")).unwrap(),
                merged_tree: BTreeMap::new(),
            };
            assert_eq!(source.artifact.state, DownloadArtifactState::Incomplete);
            assert_eq!(target.artifact.state, DownloadArtifactState::Complete);
            self.service.claim(&journal).unwrap();
            let operation = self.root.path().join(&journal.operation_directory);
            create_private_directory(&operation).unwrap();
            copy_tree(
                &self.root.path().join("album-102"),
                &operation.join("merged"),
            )
            .unwrap();
            fs::write(operation.join("merged/0001.webp"), b"new staged bytes").unwrap();
            journal.merged_tree = tree_digest(&operation.join("merged")).unwrap();
            self.service.update_journal(&journal, "swapping").unwrap();
            journal
        }
    }
    #[test]
    fn merge_preserves_unchanged_hashes_and_remaps_verified_donor_hashes_in_both_directions() {
        for side in [
            DownloadOverlapMergeSide::Incoming,
            DownloadOverlapMergeSide::Existing,
        ] {
            let fixture = if side == DownloadOverlapMergeSide::Incoming {
                Fixture::new(2, 3)
            } else {
                Fixture::new(3, 2)
            };
            fixture.seed_hashes();
            let result = fixture.service.apply(fixture.request(side, true)).unwrap();
            let connection = fixture.repository.connection().unwrap();
            let count: i64 = connection
                .query_row(
                    "SELECT count(*) FROM duplicate_page_hashes WHERE gallery_id=?1",
                    [result.target_gallery_id.get()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                count, 3,
                "all target pages should retain usable hashes after replacing one page"
            );
            let stale:i64 = connection.query_row("SELECT count(*) FROM duplicate_page_hashes h JOIN download_pages p ON h.entry_id=p.entry_id AND h.source_page_number=p.source_page_number WHERE h.artifact_sha256<>p.sha256",[],|row|row.get(0)).unwrap();
            assert_eq!(stale, 0);
            let dates: i64 = connection
                .query_row(
                    "SELECT count(*) FROM duplicate_page_hashes WHERE computed_at='original-cache'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(dates, 5, "no recomputation, donor entries remain intact");
            let timing: serde_json::Value = serde_json::from_slice(
                &fs::read(
                    Path::new(&result.backup_path)
                        .parent()
                        .unwrap()
                        .join("timings.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(timing["stages"].as_array().unwrap().len(), 6);
        }
    }

    #[test]
    fn merge_never_reuses_stale_donor_hash_and_leaves_unselected_pages_cached() {
        let fixture = Fixture::new(2, 3);
        fixture.seed_hashes();
        fixture.repository.connection().unwrap().execute("UPDATE duplicate_page_hashes SET artifact_sha256=?1 WHERE entry_id=?2 AND source_page_number=1",params!["f".repeat(64),fixture.incoming]).unwrap();
        fixture
            .service
            .apply(fixture.request(DownloadOverlapMergeSide::Incoming, true))
            .unwrap();
        let connection = fixture.repository.connection().unwrap();
        let pages:Vec<u32>=connection.prepare("SELECT source_page_number FROM duplicate_page_hashes WHERE entry_id=?1 ORDER BY source_page_number").unwrap().query_map([&fixture.existing],|row|row.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
        assert_eq!(pages, vec![2, 3]);
    }

    #[test]
    fn merges_into_complete_album_preserves_extra_pages_originals_and_target_remote_revision() {
        let fixture = Fixture::new(2, 3);
        let before = fixture.bytes(102, 1);
        let extra = fixture.bytes(102, 2);
        let source = fixture.bytes(101, 1);
        fs::write(
            fixture.root.path().join("album-102/notes.txt"),
            b"user notes",
        )
        .unwrap();
        let result = fixture
            .service
            .apply(fixture.request(DownloadOverlapMergeSide::Incoming, true))
            .unwrap();
        assert_eq!(fixture.bytes(102, 1), source);
        assert_eq!(fixture.bytes(102, 2), extra);
        assert_eq!(fixture.bytes(101, 1), source);
        assert_eq!(
            fs::read(Path::new(&result.backup_path).join("0001.webp")).unwrap(),
            before
        );
        assert_eq!(
            fs::read(Path::new(&result.backup_path).join("notes.txt")).unwrap(),
            b"user notes"
        );
        let bundle = fixture.service.bundle(&fixture.existing).unwrap();
        verify_bundle(fixture.root.path(), &bundle).unwrap();
        assert_eq!(
            bundle.pages[0].source_revision.as_deref(),
            Some("remote-102-1")
        );
        assert_eq!(bundle.artifact.revision, 1);
        assert!(result.source_excluded);
        assert!(result.resume_jobs.is_empty());
        let connection = fixture.repository.connection().unwrap();
        let state: String = connection
            .query_row(
                "SELECT state FROM download_entries WHERE entry_id=?1",
                [&fixture.incoming],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "cancelled");
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM duplicate_hidden_galleries WHERE gallery_id=101",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        let json: String = connection
            .query_row(
                "SELECT journal_json FROM overlap_page_merges WHERE merge_id=?1",
                [&result.merge_id],
                |row| row.get(0),
            )
            .unwrap();
        let journal: Journal = serde_json::from_str(&json).unwrap();
        assert_eq!(journal.replacements[0].source_revision, "remote-101-1");
        drop(connection);
        fixture.service.rollback(&journal).unwrap();
        assert_eq!(
            fixture.bytes(102, 1),
            source,
            "applied operations are never rolled back by startup recovery"
        );
    }
    #[test]
    fn merges_into_incoming_staging_and_durably_requeues_with_replacement_checkpoint() {
        let fixture = Fixture::new(3, 2);
        let extra = fixture.bytes(101, 2);
        let donor = fixture.bytes(102, 1);
        let result = fixture
            .service
            .apply(fixture.request(DownloadOverlapMergeSide::Existing, true))
            .unwrap();
        assert_eq!(fixture.bytes(101, 1), donor);
        assert_eq!(fixture.bytes(101, 2), extra);
        assert_eq!(fixture.bytes(102, 1), donor);
        assert!(!fixture.root.path().join("album-101/manifest.json").exists());
        let bundle = fixture.service.bundle(&fixture.incoming).unwrap();
        assert_eq!(bundle.artifact.state, DownloadArtifactState::Incomplete);
        assert_eq!(
            bundle.pages[0].source_revision.as_deref(),
            Some("remote-101-1")
        );
        assert_eq!(result.resume_jobs.len(), 1);
        assert_eq!(result.resume_jobs[0].entry_id, fixture.incoming);
        assert_eq!(result.resume_jobs[0].worker_attempt, 2);
        let connection = fixture.repository.connection().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM download_entries WHERE entry_id=?1",
                    [&fixture.incoming],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "queued"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM download_overlap_reviews WHERE review_id='review'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "stale"
        );
        drop(connection);
        use crate::application::{
            ArtifactStore, DownloadArtifactPlan, DownloadSourcePage, ExistingPageVerification,
        };
        let descriptor = result.resume_jobs[0].clone();
        fixture.repository.pipeline_begin(&descriptor).unwrap();
        let prepared = fixture
            .repository
            .pipeline_prepare(&DownloadArtifactPlan {
                descriptor,
                gallery: bundle.gallery.clone(),
                source_revision: "unchanged remote gallery".into(),
                root_snapshot: fixture.root.path().to_path_buf(),
                relative_directory: bundle.artifact.relative_directory.clone(),
                manifest_relative_path: ArtifactRelativePath::new("album-101/manifest.json")
                    .unwrap(),
                source_pages: bundle
                    .pages
                    .iter()
                    .map(|page| DownloadSourcePage {
                        source_page_number: page.page_id.source_page_number,
                        source_revision: page.source_revision.clone().unwrap(),
                    })
                    .collect(),
            })
            .unwrap();
        let store = super::super::FilesystemArtifactStore::new();
        let layout = store
            .prepare_existing_layout(fixture.root.path(), &prepared.relative_directory)
            .unwrap();
        assert_eq!(prepared.checkpoints.len(), 3);
        for checkpoint in &prepared.checkpoints {
            let result = store
                .verify_existing_page(
                    &layout,
                    checkpoint.page.source_page_number,
                    &format!("remote-101-{}", checkpoint.page.source_page_number.get()),
                    Some(&checkpoint.page),
                )
                .unwrap();
            assert!(
                matches!(result, ExistingPageVerification::Verified(_)),
                "merged checkpoints must not redownload or move into recovery"
            );
        }
        assert_eq!(fixture.bytes(101, 1), donor);
    }
    #[test]
    fn rejects_stale_ambiguous_unmatched_and_uncontained_exclusion_before_touching_files() {
        let fixture = Fixture::new(3, 2);
        let original = tree_digest(&fixture.root.path().join("album-102")).unwrap();
        let request = fixture.request(DownloadOverlapMergeSide::Incoming, true);
        assert!(fixture
            .service
            .apply(request.clone())
            .unwrap_err()
            .to_string()
            .contains("unmatched"));
        let mut request = fixture.request(DownloadOverlapMergeSide::Existing, false);
        request.expected_revision = 999;
        assert!(matches!(
            fixture.service.apply(request),
            Err(ApplicationError::RevisionConflict { .. })
        ));
        let mut request = fixture.request(DownloadOverlapMergeSide::Incoming, false);
        request.source_pages = vec![2];
        assert!(fixture.service.apply(request).is_err());
        let mut review = fixture
            .repository
            .overlap_review_get("review")
            .unwrap()
            .unwrap();
        let candidate = &mut review.candidates[0];
        candidate.relation = DownloadOverlapRelation::PartialOverlap;
        candidate.page_pairs.push(candidate.page_pairs[0].clone());
        assert!(derive_mapping(
            candidate,
            &fixture.request(DownloadOverlapMergeSide::Incoming, false),
            3,
            2
        )
        .is_err());
        assert_eq!(
            tree_digest(&fixture.root.path().join("album-102")).unwrap(),
            original
        );
        assert_eq!(
            fixture
                .repository
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM overlap_page_merges", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn recovery_restores_original_at_every_precommit_swap_boundary() {
        for step in 0..=2 {
            let fixture = Fixture::new(2, 3);
            let original = tree_digest(&fixture.root.path().join("album-102")).unwrap();
            let journal = fixture.pending();
            let operation = fixture.root.path().join(&journal.operation_directory);
            if step >= 1 {
                rename_no_replace(
                    &fixture.root.path().join("album-102"),
                    &operation.join("original"),
                )
                .unwrap();
            }
            if step >= 2 {
                rename_no_replace(
                    &operation.join("merged"),
                    &fixture.root.path().join("album-102"),
                )
                .unwrap();
            }
            assert_eq!(fixture.service.recover_pending().unwrap(), 1);
            assert_eq!(
                tree_digest(&fixture.root.path().join("album-102")).unwrap(),
                original
            );
            assert_eq!(fixture.service.recover_pending().unwrap(), 0);
            assert_eq!(
                fixture
                    .repository
                    .connection()
                    .unwrap()
                    .query_row(
                        "SELECT count(*) FROM duplicate_hidden_galleries",
                        [],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
        }
    }
    #[test]
    fn recovery_pauses_on_modified_backup_or_mismatched_journal_and_does_not_overwrite() {
        let fixture = Fixture::new(2, 3);
        let mut journal = fixture.pending();
        let operation = fixture.root.path().join(&journal.operation_directory);
        rename_no_replace(
            &fixture.root.path().join("album-102"),
            &operation.join("original"),
        )
        .unwrap();
        fs::write(operation.join("original/0001.webp"), b"external change").unwrap();
        assert!(fixture.service.recover_pending().is_err());
        assert_eq!(
            fs::read(operation.join("original/0001.webp")).unwrap(),
            b"external change"
        );
        journal.target_directory = "album-101".into();
        assert!(fixture
            .service
            .rollback(&journal)
            .unwrap_err()
            .to_string()
            .contains("journal"));
        assert!(fixture.root.path().join("album-101").exists());
    }
    #[test]
    fn active_merge_fences_page_mutation_job_retry_and_artifact_enumeration() {
        let fixture = Fixture::new(2, 3);
        fixture.pending();
        let connection = fixture.repository.connection().unwrap();
        for entry in [&fixture.incoming, &fixture.existing] {
            assert!(connection
                .execute(
                    "UPDATE download_pages SET excluded=1 WHERE entry_id=?1",
                    [entry]
                )
                .is_err());
            assert!(connection
                .execute(
                    "UPDATE download_entries SET state='queued' WHERE entry_id=?1",
                    [entry]
                )
                .is_err());
            assert!(connection
                .execute(
                    "UPDATE download_artifacts SET revision=revision+1 WHERE entry_id=?1",
                    [entry]
                )
                .is_err());
        }
        drop(connection);
        assert!(fixture
            .repository
            .pipeline_artifact_bundles()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn source_integrity_failure_rolls_back_without_excluding_donor() {
        let fixture = Fixture::new(2, 3);
        let original = tree_digest(&fixture.root.path().join("album-102")).unwrap();
        fs::write(
            fixture.root.path().join("album-101/0001.webp"),
            b"corrupted donor",
        )
        .unwrap();
        assert!(fixture
            .service
            .apply(fixture.request(DownloadOverlapMergeSide::Incoming, true))
            .is_err());
        assert_eq!(
            tree_digest(&fixture.root.path().join("album-102")).unwrap(),
            original
        );
        assert_eq!(
            fixture
                .repository
                .connection()
                .unwrap()
                .query_row("SELECT state FROM overlap_page_merges", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "rolled_back"
        );
    }
    #[test]
    fn missing_backup_after_swap_keeps_operation_pending_and_repeated_restore_is_safe() {
        let fixture = Fixture::new(2, 3);
        let journal = fixture.pending();
        let operation = fixture.root.path().join(&journal.operation_directory);
        rename_no_replace(
            &fixture.root.path().join("album-102"),
            &operation.join("original"),
        )
        .unwrap();
        rename_no_replace(
            &operation.join("merged"),
            &fixture.root.path().join("album-102"),
        )
        .unwrap();
        // Simulate an unavailable backup by relocating it inside this fixture,
        // never deleting the original bytes even in the failure injection.
        rename_no_replace(
            &operation.join("original"),
            &operation.join("temporarily-unavailable"),
        )
        .unwrap();
        assert!(fixture
            .service
            .recover_pending()
            .unwrap_err()
            .to_string()
            .contains("backup is missing"));
        rename_no_replace(
            &operation.join("temporarily-unavailable"),
            &operation.join("original"),
        )
        .unwrap();
        rename_no_replace(
            &fixture.root.path().join("album-102"),
            &operation.join("uncommitted-new"),
        )
        .unwrap();
        rename_no_replace(
            &operation.join("original"),
            &fixture.root.path().join("album-102"),
        )
        .unwrap();
        assert_eq!(
            fixture.service.recover_pending().unwrap(),
            1,
            "crash after restoring original but before journal acknowledgement is idempotent"
        );
    }
    #[test]
    fn exclusion_failure_rolls_back_files_pages_and_hidden_state_atomically() {
        let fixture = Fixture::new(2, 3);
        let original = tree_digest(&fixture.root.path().join("album-102")).unwrap();
        fixture.repository.connection().unwrap().execute_batch("CREATE TRIGGER merge_fail_exclusion BEFORE INSERT ON duplicate_hidden_galleries BEGIN SELECT RAISE(ABORT,'injected exclusion commit failure'); END;").unwrap();
        assert!(fixture
            .service
            .apply(fixture.request(DownloadOverlapMergeSide::Incoming, true))
            .unwrap_err()
            .to_string()
            .contains("injected exclusion"));
        assert_eq!(
            tree_digest(&fixture.root.path().join("album-102")).unwrap(),
            original
        );
        verify_bundle(
            fixture.root.path(),
            &fixture.service.bundle(&fixture.existing).unwrap(),
        )
        .unwrap();
        let connection = fixture.repository.connection().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM download_entries WHERE entry_id=?1",
                    [&fixture.incoming],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "review_required"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM duplicate_hidden_galleries",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT state FROM overlap_page_merges", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "rolled_back"
        );
    }
    #[test]
    fn excluding_source_rejects_out_of_bounds_unselected_correspondence() {
        let fixture = Fixture::new(2, 3);
        fixture
            .repository
            .connection()
            .unwrap()
            .execute(
                "UPDATE download_overlap_page_pairs SET existing_source_page=99 WHERE pair_index=1",
                [],
            )
            .unwrap();
        assert!(fixture
            .service
            .apply(fixture.request(DownloadOverlapMergeSide::Incoming, true))
            .unwrap_err()
            .to_string()
            .contains("bounds"));
        assert_eq!(
            fixture
                .repository
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM overlap_page_merges", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
