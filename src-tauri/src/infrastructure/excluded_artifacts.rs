//! Reversible folder relocation for duplicate removals that are still staging
//! downloads. This deliberately preserves download/page states and never
//! promotes a cancelled or failed download into a completed artifact.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use rusqlite::{params, TransactionBehavior};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    application::{
        ApplicationError, ArtifactLayout, ArtifactRepository, ArtifactStore, DownloadPipelineError,
        DownloadPipelineErrorCode, RepositoryError,
    },
    domain::{ArtifactManifest, ArtifactRelativePath, DownloadArtifactState, DownloadEntryId},
};

use super::SqliteRepository;

pub(super) const EXCLUDED_ARTIFACTS_SCHEMA: &str = r#"
    CREATE TABLE excluded_artifact_relocations (
        record_id TEXT PRIMARY KEY,
        entry_id TEXT NOT NULL REFERENCES download_entries(entry_id) ON DELETE RESTRICT,
        gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
        root_snapshot TEXT NOT NULL CHECK (length(root_snapshot) > 0),
        original_relative_path TEXT NOT NULL,
        excluded_relative_path TEXT NOT NULL,
        state TEXT NOT NULL CHECK (state IN ('pending_exclude', 'excluded', 'pending_restore', 'restored')),
        artifact_revision INTEGER NOT NULL CHECK (artifact_revision >= 0),
        backup_json TEXT NOT NULL CHECK (json_valid(backup_json)),
        reason TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    ) STRICT;
    CREATE UNIQUE INDEX excluded_artifact_active_entry_idx
        ON excluded_artifact_relocations(entry_id) WHERE state <> 'restored';
    CREATE INDEX excluded_artifact_gallery_idx ON excluded_artifact_relocations(gallery_id);
    DROP TRIGGER download_artifacts_relative_directory_immutable;
    CREATE TRIGGER download_artifacts_relative_directory_immutable
    BEFORE UPDATE OF relative_directory ON download_artifacts
    FOR EACH ROW
    WHEN NEW.relative_directory <> OLD.relative_directory
      AND NOT EXISTS (
        SELECT 1 FROM excluded_artifact_relocations x
        WHERE x.entry_id=OLD.entry_id AND x.root_snapshot=OLD.root_snapshot
          AND x.artifact_revision=OLD.revision AND NEW.revision=OLD.revision+1
          AND NEW.state=OLD.state
          AND ((x.state='pending_exclude' AND x.original_relative_path=OLD.relative_directory AND x.excluded_relative_path=NEW.relative_directory)
            OR (x.state='pending_restore' AND x.excluded_relative_path=OLD.relative_directory AND x.original_relative_path=NEW.relative_directory))
      )
    BEGIN
        SELECT RAISE(ABORT, 'download artifact relative_directory is immutable');
    END;
"#;

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcludedArtifactReport {
    pub moved: usize,
    pub restored: usize,
    pub gallery_ids: Vec<i64>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone)]
struct Relocation {
    record_id: String,
    entry_id: String,
    gallery_id: i64,
    root: PathBuf,
    original: ArtifactRelativePath,
    excluded: ArtifactRelativePath,
    state: String,
    artifact_revision: i64,
    backup_json: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct RelocationBackup {
    schema_version: u32,
    entry_id: String,
    gallery_id: i64,
    entry_state: String,
    artifact_state: String,
    root_snapshot: String,
    original_relative_path: String,
    excluded_relative_path: String,
    manifest: Option<ArtifactManifest>,
    // A durable logical backup of every original row, including partial-page
    // verification metadata that has no final manifest yet.
    pages: Vec<serde_json::Value>,
}

pub struct ExcludedArtifactService {
    repository: Arc<SqliteRepository>,
    store: Arc<dyn ArtifactStore>,
    gate: Mutex<()>,
}

impl ExcludedArtifactService {
    pub fn new(repository: Arc<SqliteRepository>, store: Arc<dyn ArtifactStore>) -> Self {
        Self {
            repository,
            store,
            gate: Mutex::new(()),
        }
    }

    /// Run before the ordinary download reconcile and after a removal decision.
    /// Every failure is retained as an unfinished saga and reported; later runs
    /// resume it without overwriting either location.
    pub fn reconcile(&self) -> Result<ExcludedArtifactReport, ApplicationError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| conflict("Excluded-folder lock was poisoned"))?;
        let mut report = self.reconcile_pending_locked()?;
        self.relocate_new_candidates(&mut report)?;
        Ok(report)
    }

    /// Complete only already journaled moves before ordinary file verification.
    /// Discovering and moving legacy excluded folders can run later in the
    /// background, without delaying startup on that inventory.
    pub fn reconcile_pending(&self) -> Result<ExcludedArtifactReport, ApplicationError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| conflict("Excluded-folder lock was poisoned"))?;
        self.reconcile_pending_locked()
    }

    fn reconcile_pending_locked(&self) -> Result<ExcludedArtifactReport, ApplicationError> {
        let mut report = ExcludedArtifactReport::default();
        for record in self.records("state IN ('pending_exclude', 'pending_restore')")? {
            let restoring = record.state == "pending_restore";
            match self.finish(&record) {
                Ok(()) => {
                    report.gallery_ids.push(record.gallery_id);
                    if restoring {
                        report.restored += 1;
                    } else {
                        report.moved += 1;
                    }
                }
                Err(error) => report_issue(&mut report, &record.entry_id, error),
            }
        }
        Ok(report)
    }

    fn relocate_new_candidates(
        &self,
        report: &mut ExcludedArtifactReport,
    ) -> Result<(), ApplicationError> {
        let candidates = {
            let connection = self.repository.connection()?;
            let mut statement = connection.prepare(
                "SELECT e.entry_id FROM download_entries e
                 JOIN download_artifacts a USING(entry_id)
                 JOIN duplicate_hidden_galleries h USING(gallery_id)
                 WHERE e.state IN ('cancelled', 'failed', 'completed')
                   AND a.state IN ('incomplete', 'complete')
                   AND NOT EXISTS (SELECT 1 FROM exploration_restored_galleries r WHERE r.gallery_id=e.gallery_id)
                   AND NOT EXISTS (SELECT 1 FROM excluded_artifact_relocations x WHERE x.entry_id=e.entry_id AND x.state <> 'restored')
                 ORDER BY e.entry_id"
            ).map_err(db_error)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(db_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?
        };
        for entry_id in candidates {
            let result = self
                .begin(&entry_id)
                .and_then(|record| self.finish(&record).map(|()| record.gallery_id));
            match result {
                Ok(gallery_id) => {
                    report.moved += 1;
                    report.gallery_ids.push(gallery_id);
                }
                Err(error) => report_issue(report, &entry_id, error),
            }
        }
        Ok(())
    }

    /// Hold the same gate through the actual exclusion reset. If a destination
    /// conflicts, `restore_action` is never called and the gallery stays hidden.
    pub fn restore_then<T>(
        &self,
        gallery_ids: &[i64],
        restore_action: impl FnOnce() -> Result<T, ApplicationError>,
    ) -> Result<T, ApplicationError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| conflict("Excluded-folder lock was poisoned"))?;
        let records = self.records("state <> 'restored'")?;
        for mut record in records
            .into_iter()
            .filter(|record| gallery_ids.contains(&record.gallery_id))
        {
            if record.state == "pending_exclude" {
                self.finish(&record)?;
            }
            if record.state != "pending_restore" {
                let connection = self.repository.connection()?;
                let revision: i64 = connection.query_row(
                    "SELECT revision FROM download_artifacts WHERE entry_id=?1 AND relative_directory=?2",
                    params![record.entry_id, record.excluded.as_str()], |row| row.get(0),
                ).map_err(db_error)?;
                // Check both paths before committing restore intent. In
                // particular, never remove a newly created original folder.
                let root = checked_root(&record.root)?;
                if checked_path(&root, &record.original)?.exists() {
                    return Err(conflict(
                        "The original album folder already exists; restore was not applied",
                    ));
                }
                connection.execute(
                    "UPDATE excluded_artifact_relocations SET state='pending_restore', artifact_revision=?1,
                     updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE record_id=?2 AND state='excluded'",
                    params![revision, record.record_id],
                ).map_err(db_error)?;
                record.artifact_revision = revision;
                record.state = "pending_restore".into();
            }
            self.finish(&record)?;
        }
        restore_action()
    }

    fn begin(&self, entry_id: &str) -> Result<Relocation, ApplicationError> {
        let id = DownloadEntryId::new(entry_id.to_owned())?;
        let bundle = self
            .repository
            .artifact_bundle_get(&id)?
            .ok_or_else(|| conflict("The excluded album no longer has an artifact"))?;
        let (root_text, entry_state, artifact_state, revision, pages) = {
            let connection = self.repository.connection()?;
            let (root, entry_state, artifact_state, revision) = connection
                .query_row(
                    "SELECT a.root_snapshot,e.state,a.state,a.revision FROM download_artifacts a
                 JOIN download_entries e USING(entry_id) WHERE a.entry_id=?1",
                    [entry_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .map_err(db_error)?;
            let mut statement = connection.prepare(
                "SELECT json_object('sourcePageNumber',source_page_number,'relativePath',relative_path,
                 'state',state,'byteLength',byte_length,'sha256',sha256,'storageFormat',storage_format,
                 'sourceRevision',source_revision,'verifiedAt',verified_at,'excluded',excluded)
                 FROM download_pages WHERE entry_id=?1 ORDER BY source_page_number"
            ).map_err(db_error)?;
            let pages = statement
                .query_map([entry_id], |row| row.get::<_, String>(0))
                .map_err(db_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_error)?
                .into_iter()
                .map(|text| serde_json::from_str(&text).map_err(json_error))
                .collect::<Result<Vec<_>, _>>()?;
            (root, entry_state, artifact_state, revision, pages)
        };
        let root = checked_root(Path::new(&root_text))?;
        let original = bundle.artifact.relative_directory.clone();
        if original.as_str().starts_with(".atsumi-") {
            return Err(conflict(
                "An internal recovery folder cannot be relocated again",
            ));
        }
        let excluded = ArtifactRelativePath::new(format!(
            ".atsumi-excluded/{entry_id}/{}",
            original.as_str()
        ))?;
        let source = checked_path(&root, &original)?;
        if !source.is_dir() {
            return Err(conflict("The excluded album folder is missing"));
        }
        if checked_path(&root, &excluded)?.exists() {
            return Err(conflict("The excluded album destination already exists"));
        }
        self.reject_nested_artifacts(entry_id, &source)?;
        let manifest_path =
            bundle
                .artifact
                .manifest_relative_path
                .clone()
                .unwrap_or(ArtifactRelativePath::new(format!(
                    "{}/manifest.json",
                    original.as_str()
                ))?);
        checked_path(&root, &manifest_path)?;
        let layout = ArtifactLayout {
            root: root.clone(),
            relative_directory: original.clone(),
            manifest_relative_path: manifest_path,
        };
        let manifest = self.store.read_manifest(&layout)?;
        if bundle.artifact.state == DownloadArtifactState::Complete {
            if manifest.as_ref() != Some(&ArtifactManifest::from_bundle(&bundle)?) {
                return Err(conflict(
                    "The completed album manifest does not match its database snapshot",
                ));
            }
        } else if manifest.is_some() {
            return Err(conflict(
                "An unfinished album has an unexpected final manifest; inspect it before moving",
            ));
        }
        let backup = RelocationBackup {
            schema_version: 1,
            entry_id: entry_id.to_owned(),
            gallery_id: bundle.gallery.id.get(),
            entry_state,
            artifact_state,
            root_snapshot: root_text.clone(),
            original_relative_path: original.as_str().into(),
            excluded_relative_path: excluded.as_str().into(),
            manifest,
            pages,
        };
        let record = Relocation {
            record_id: Uuid::new_v4().to_string(),
            entry_id: entry_id.into(),
            gallery_id: bundle.gallery.id.get(),
            root: PathBuf::from(root_text),
            original,
            excluded,
            state: "pending_exclude".into(),
            artifact_revision: revision,
            backup_json: serde_json::to_string(&backup).map_err(json_error)?,
        };
        let connection = self.repository.connection()?;
        let inserted = connection.execute(
            "INSERT INTO excluded_artifact_relocations
             (record_id,entry_id,gallery_id,root_snapshot,original_relative_path,excluded_relative_path,state,artifact_revision,backup_json,reason,created_at,updated_at)
             SELECT ?1,e.entry_id,e.gallery_id,?2,?3,?4,'pending_exclude',a.revision,?5,'duplicate_hidden',
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM download_entries e JOIN download_artifacts a USING(entry_id)
             WHERE e.entry_id=?6 AND a.revision=?7 AND a.relative_directory=?3
                AND e.state IN ('cancelled','failed','completed') AND a.state IN ('incomplete','complete')
                AND EXISTS (SELECT 1 FROM duplicate_hidden_galleries h WHERE h.gallery_id=e.gallery_id)
                AND NOT EXISTS (SELECT 1 FROM exploration_restored_galleries r WHERE r.gallery_id=e.gallery_id)",
            params![record.record_id, record.root.to_string_lossy(), record.original.as_str(), record.excluded.as_str(), record.backup_json, entry_id, revision],
        ).map_err(db_error)?;
        if inserted != 1 {
            return Err(conflict(
                "The excluded album changed before its move could begin",
            ));
        }
        Ok(record)
    }

    fn finish(&self, record: &Relocation) -> Result<(), ApplicationError> {
        let restoring = record.state == "pending_restore";
        let (from, to, complete_state) = if restoring {
            (&record.excluded, &record.original, "restored")
        } else {
            (&record.original, &record.excluded, "excluded")
        };
        let root = checked_root(&record.root)?;
        let source = checked_path(&root, from)?;
        let destination = checked_path(&root, to)?;
        match (source.exists(), destination.exists()) {
            (true, false) => {}
            (false, true) => {} // Interrupted after rename; the DB still has the old paths.
            _ => {
                return Err(conflict(
                    "Excluded-folder recovery found conflicting or missing locations",
                ))
            }
        }
        let backup: RelocationBackup =
            serde_json::from_str(&record.backup_json).map_err(json_error)?;
        let id = DownloadEntryId::new(record.entry_id.clone())?;
        let bundle = self
            .repository
            .artifact_bundle_get(&id)?
            .ok_or_else(|| conflict("The excluded album artifact disappeared"))?;
        if bundle.artifact.revision != record.artifact_revision as u64
            || bundle.artifact.relative_directory != *from
        {
            return Err(conflict(
                "The excluded album changed while its folder move was pending",
            ));
        }
        // Retain both the database saga and a synced, immutable sidecar backup.
        // No album bytes are deleted by this service.
        self.write_backup(&root, record)?;
        if source.exists() {
            self.reject_nested_artifacts(&record.entry_id, &source)?;
            self.verify_manifest(&root, from, &bundle, backup.manifest.is_some(), None)?;
            self.store.move_managed_directory(&root, from, to)?;
        }
        if let Some(mut expected) = self.expected_manifest(&bundle, backup.manifest.is_some())? {
            let mut moved = expected.clone();
            rebase_manifest(&mut moved, from, to)?;
            self.verify_manifest(&root, to, &bundle, true, Some((&expected, &moved)))?;
            rebase_manifest(&mut expected, from, to)?;
            self.store.write_manifest(&layout(&root, to)?, &expected)?;
        } else {
            self.verify_manifest(&root, to, &bundle, false, None)?;
        }
        let mut connection = self.repository.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let changed = transaction.execute(
            "UPDATE download_artifacts SET relative_directory=?1,
             manifest_relative_path=CASE WHEN manifest_relative_path IS NULL THEN NULL ELSE ?1 || substr(manifest_relative_path,length(?2)+1) END,
             revision=revision+1 WHERE entry_id=?3 AND revision=?4 AND relative_directory=?2",
            params![to.as_str(),from.as_str(),record.entry_id,record.artifact_revision],
        ).map_err(db_error)?;
        if changed != 1 {
            return Err(conflict("The artifact paths changed before move commit"));
        }
        transaction
            .execute(
                "UPDATE download_pages SET relative_path=?1 || substr(relative_path,length(?2)+1)
             WHERE entry_id=?3 AND substr(relative_path,1,length(?2)+1)=?2 || '/'",
                params![to.as_str(), from.as_str(), record.entry_id],
            )
            .map_err(db_error)?;
        // Internal-page quarantine records refer to paths inside this album.
        // Rebase both sides so their independent undo remains valid later.
        for column in ["original_relative_path", "quarantine_relative_path"] {
            transaction
                .execute(
                    &format!(
                "UPDATE page_quarantine_records SET {column}=?1 || substr({column},length(?2)+1)
                 WHERE entry_id=?3 AND substr({column},1,length(?2)+1)=?2 || '/'"
            ),
                    params![to.as_str(), from.as_str(), record.entry_id],
                )
                .map_err(db_error)?;
        }
        transaction.execute(
            "UPDATE excluded_artifact_relocations SET state=?1, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE record_id=?2 AND state=?3", params![complete_state,record.record_id,record.state],
        ).map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        tracing::info!(
            entry_id = record.entry_id,
            record_id = record.record_id,
            state = complete_state,
            "Excluded album folder relocation committed"
        );
        Ok(())
    }

    fn expected_manifest(
        &self,
        bundle: &crate::domain::ArtifactBundle,
        has_manifest: bool,
    ) -> Result<Option<ArtifactManifest>, ApplicationError> {
        if has_manifest {
            Ok(Some(ArtifactManifest::from_bundle(bundle)?))
        } else {
            Ok(None)
        }
    }

    fn verify_manifest(
        &self,
        root: &Path,
        directory: &ArtifactRelativePath,
        bundle: &crate::domain::ArtifactBundle,
        has_manifest: bool,
        accepted: Option<(&ArtifactManifest, &ArtifactManifest)>,
    ) -> Result<(), ApplicationError> {
        let layout = layout(root, directory)?;
        checked_path(root, &layout.manifest_relative_path)?;
        let actual = self.store.read_manifest(&layout)?;
        if let Some((before, after)) = accepted {
            if actual.as_ref() != Some(before) && actual.as_ref() != Some(after) {
                return Err(conflict(
                    "The moved manifest differs from both recorded recovery states",
                ));
            }
        } else if actual != self.expected_manifest(bundle, has_manifest)? {
            return Err(conflict("The album manifest changed before relocation"));
        }
        Ok(())
    }

    fn write_backup(&self, root: &Path, record: &Relocation) -> Result<(), ApplicationError> {
        let relative = ArtifactRelativePath::new(format!(
            ".atsumi-excluded/{}/.atsumi-recovery-{}.json",
            record.entry_id, record.record_id
        ))?;
        let path = checked_path(root, &relative)?;
        if path.exists() {
            if fs::read_to_string(&path).map_err(io_error)? != record.backup_json {
                return Err(conflict("The excluded-folder recovery backup was modified"));
            }
            return Ok(());
        }
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| conflict("Recovery backup parent is missing"))?,
        )
        .map_err(io_error)?;
        checked_path(root, &relative)?;
        let temporary = path.with_extension(format!("pending-{}", Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(io_error)?;
        file.write_all(record.backup_json.as_bytes())
            .map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        drop(file);
        // A partially written temporary file after interruption is never read as
        // a recovery backup. Hard-link creation refuses to replace an existing
        // backup on both Windows and Unix.
        fs::hard_link(&temporary, &path).map_err(io_error)?;
        fs::remove_file(&temporary).map_err(io_error)?;
        Ok(())
    }

    fn reject_nested_artifacts(
        &self,
        entry_id: &str,
        directory: &Path,
    ) -> Result<(), ApplicationError> {
        let connection = self.repository.connection()?;
        let mut statement = connection.prepare("SELECT root_snapshot,relative_directory FROM download_artifacts WHERE entry_id<>?1").map_err(db_error)?;
        let rows = statement
            .query_map([entry_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_error)?;
        let directory = path_key(directory);
        for row in rows {
            let (root, relative) = row.map_err(db_error)?;
            let other = path_key(&resolve_existing_ancestor(
                &PathBuf::from(root).join(relative),
            )?);
            if other == directory || other.starts_with(&(directory.clone() + "/")) {
                return Err(conflict(
                    "The album folder also contains another managed album",
                ));
            }
        }
        Ok(())
    }

    fn records(&self, condition: &str) -> Result<Vec<Relocation>, ApplicationError> {
        let connection = self.repository.connection()?;
        let mut statement = connection.prepare(&format!(
            "SELECT record_id,entry_id,gallery_id,root_snapshot,original_relative_path,excluded_relative_path,state,artifact_revision,backup_json
             FROM excluded_artifact_relocations WHERE {condition} ORDER BY created_at,record_id"
        )).map_err(db_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })
            .map_err(db_error)?;
        rows.map(|row| {
            let r = row.map_err(db_error)?;
            Ok(Relocation {
                record_id: r.0,
                entry_id: r.1,
                gallery_id: r.2,
                root: PathBuf::from(r.3),
                original: ArtifactRelativePath::new(r.4)?,
                excluded: ArtifactRelativePath::new(r.5)?,
                state: r.6,
                artifact_revision: r.7,
                backup_json: r.8,
            })
        })
        .collect()
    }
}

fn rebase_manifest(
    manifest: &mut ArtifactManifest,
    from: &ArtifactRelativePath,
    to: &ArtifactRelativePath,
) -> Result<(), ApplicationError> {
    let prefix = format!("{}/", from.as_str());
    for page in &mut manifest.pages {
        let suffix = page
            .relative_path
            .strip_prefix(&prefix)
            .ok_or_else(|| conflict("A manifest page escapes its album folder"))?;
        page.relative_path = ArtifactRelativePath::new(format!("{}/{suffix}", to.as_str()))?
            .as_str()
            .into();
    }
    Ok(())
}

fn layout(
    root: &Path,
    directory: &ArtifactRelativePath,
) -> Result<ArtifactLayout, ApplicationError> {
    Ok(ArtifactLayout {
        root: root.to_owned(),
        relative_directory: directory.clone(),
        manifest_relative_path: ArtifactRelativePath::new(format!(
            "{}/manifest.json",
            directory.as_str()
        ))?,
    })
}

fn checked_root(root: &Path) -> Result<PathBuf, ApplicationError> {
    if !root.is_absolute() || !root.is_dir() {
        return Err(conflict("The recorded download root is unavailable"));
    }
    let mut ancestor = PathBuf::new();
    for component in root.components() {
        ancestor.push(component.as_os_str());
        if ancestor.is_absolute() {
            reject_link(&ancestor)?;
        }
    }
    root.canonicalize().map_err(io_error)
}

/// Check every existing component before the store is allowed to create missing
/// destination parents. Canonicalization alone can follow junctions outside root.
fn checked_path(root: &Path, relative: &ArtifactRelativePath) -> Result<PathBuf, ApplicationError> {
    let mut candidate = root.to_owned();
    for component in Path::new(relative.as_str()).components() {
        candidate.push(component.as_os_str());
        match fs::symlink_metadata(&candidate) {
            Ok(_) => {
                reject_link(&candidate)?;
                let actual = candidate.canonicalize().map_err(io_error)?;
                if actual == root || !actual.starts_with(root) {
                    return Err(conflict(
                        "The album path escapes its recorded download root",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    Ok(candidate)
}

fn reject_link(path: &Path) -> Result<(), ApplicationError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    let mut linked = metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        linked |= metadata.file_attributes() & 0x400 != 0;
    }
    if linked {
        return Err(conflict(
            "A symlink or junction prevents safe album relocation",
        ));
    }
    Ok(())
}

fn path_key(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized
        .strip_prefix("//?/")
        .unwrap_or(&normalized)
        .trim_end_matches('/')
        .to_lowercase()
}

// Resolve aliases before comparing other managed locations. A quarantined
// artifact's original directory may be absent, so resolve its nearest existing
// ancestor and append the missing tail without losing `..` semantics.
fn resolve_existing_ancestor(path: &Path) -> Result<PathBuf, ApplicationError> {
    if !path.is_absolute() {
        return Err(conflict(
            "Another managed album has an unresolved download root",
        ));
    }
    let mut ancestor = path.to_owned();
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                for component in suffix.into_iter().rev() {
                    match component.to_str() {
                        Some(".") => {}
                        Some("..") => {
                            if !resolved.pop() {
                                return Err(conflict(
                                    "Another managed album path escapes its filesystem root",
                                ));
                            }
                        }
                        _ => resolved.push(component),
                    }
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let component = ancestor
                    .components()
                    .next_back()
                    .ok_or_else(|| conflict("Another managed album location cannot be resolved"))?
                    .as_os_str()
                    .to_owned();
                if !ancestor.pop() {
                    return Err(conflict(
                        "Another managed album location cannot be resolved",
                    ));
                }
                suffix.push(component);
            }
            Err(error) => return Err(io_error(error)),
        }
    }
}

fn report_issue(report: &mut ExcludedArtifactReport, entry_id: &str, error: ApplicationError) {
    tracing::warn!(entry_id, error=%error, "Excluded album folder relocation requires recovery");
    report.issues.push(format!("{entry_id}: {error}"));
}

fn conflict(message: &str) -> ApplicationError {
    DownloadPipelineError::new(
        DownloadPipelineErrorCode::QuarantineConflict,
        message,
        false,
    )
    .into()
}
fn db_error(error: rusqlite::Error) -> ApplicationError {
    RepositoryError::Other(format!(
        "Excluded-folder database operation failed: {error}"
    ))
    .into()
}
fn io_error(error: std::io::Error) -> ApplicationError {
    DownloadPipelineError::new(
        DownloadPipelineErrorCode::Filesystem,
        format!("Excluded-folder filesystem operation failed: {error}"),
        true,
    )
    .into()
}
fn json_error(error: serde_json::Error) -> ApplicationError {
    conflict(&format!(
        "Excluded-folder recovery record is invalid: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        application::{AutomationRepository, DownloadQueueAddOutcome, DownloadRepository},
        domain::GalleryId,
        infrastructure::FilesystemArtifactStore,
    };
    use tempfile::TempDir;

    struct Fixture {
        temporary: TempDir,
        repository: Arc<SqliteRepository>,
        service: ExcludedArtifactService,
    }

    impl Fixture {
        fn new() -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let repository = Arc::new(SqliteRepository::open_in_memory().unwrap());
            let service = ExcludedArtifactService::new(
                repository.clone(),
                Arc::new(FilesystemArtifactStore::new()),
            );
            Self {
                temporary,
                repository,
                service,
            }
        }

        fn seed(&self, gallery_id: i64, state: &str) -> String {
            self.seed_in(gallery_id, state, self.temporary.path())
        }

        fn seed_in(&self, gallery_id: i64, state: &str, download_root: &Path) -> String {
            let gallery = GalleryId::new(gallery_id).unwrap();
            let DownloadQueueAddOutcome::Added(queued) = self
                .repository
                .download_queue_add(&format!("fixture-{gallery_id}"), &[gallery])
                .unwrap()
            else {
                panic!("new download")
            };
            let entry_id = queued.entries[0].entry_id.to_string();
            let directory = format!("album-{gallery_id}");
            fs::create_dir(download_root.join(&directory)).unwrap();
            fs::write(
                download_root.join(&directory).join("0001.webp"),
                b"original page bytes",
            )
            .unwrap();
            {
                let connection = self.repository.connection().unwrap();
                connection.execute("INSERT INTO galleries(gallery_id,revision,title,source_page_count) VALUES(?1,0,'fixture',1)", [gallery_id]).unwrap();
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
                connection.execute(
                    "INSERT INTO download_artifacts(entry_id,gallery_id,revision,relative_directory,expected_page_count,state,manifest_relative_path,root_snapshot)
                     VALUES(?1,?2,0,?3,1,'incomplete',?4,?5)",
                    params![entry_id,gallery_id,directory,format!("{directory}/manifest.json"),download_root.to_string_lossy()],
                ).unwrap();
                connection.execute(
                    "INSERT INTO download_pages(entry_id,gallery_id,source_page_number,relative_path,state,byte_length,sha256,storage_format,source_revision,verified_at)
                     VALUES(?1,?2,1,?3,'present',19,?4,'webp','fixture-revision','2026-09-01T00:00:00Z')",
                    params![entry_id,gallery_id,format!("{directory}/0001.webp"),"a".repeat(64)],
                ).unwrap();
                connection.execute("INSERT INTO duplicate_hidden_galleries(gallery_id,decision_id,created_at) VALUES(?1,?2,'2026-09-01T00:00:00Z')",params![gallery_id,format!("hide-{gallery_id}")]).unwrap();
                if state == "completed" {
                    connection.execute("UPDATE download_artifacts SET state='complete',manifest_schema_version=1,writer_version='fixture',completed_at='2026-09-01T00:00:00Z' WHERE entry_id=?1",[&entry_id]).unwrap();
                }
            }
            if state == "completed" {
                let bundle = self
                    .repository
                    .artifact_bundle_get(&DownloadEntryId::new(entry_id.clone()).unwrap())
                    .unwrap()
                    .unwrap();
                let manifest = ArtifactManifest::from_bundle(&bundle).unwrap();
                self.service
                    .store
                    .write_manifest(
                        &layout(download_root, &bundle.artifact.relative_directory).unwrap(),
                        &manifest,
                    )
                    .unwrap();
            }
            entry_id
        }

        fn artifact(&self, entry_id: &str) -> (String, String, String) {
            self.repository.connection().unwrap().query_row(
                "SELECT e.state,a.state,a.relative_directory FROM download_entries e JOIN download_artifacts a USING(entry_id) WHERE e.entry_id=?1",
                [entry_id],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
            ).unwrap()
        }
    }

    #[test]
    fn duplicate_staging_moves_and_restores_without_changing_download_states() {
        let fixture = Fixture::new();
        for (id, state) in [(101, "cancelled"), (102, "failed")] {
            let entry = fixture.seed(id, state);
            let report = fixture.service.reconcile().unwrap();
            assert!(report.issues.is_empty(), "{:?}", report.issues);
            assert_eq!(report.moved, 1);
            let artifact = fixture.artifact(&entry);
            assert_eq!((&*artifact.0, &*artifact.1), (state, "incomplete"));
            assert!(!fixture
                .temporary
                .path()
                .join(format!("album-{id}"))
                .exists());
            assert_eq!(
                fs::read(fixture.temporary.path().join(&artifact.2).join("0001.webp")).unwrap(),
                b"original page bytes"
            );
            assert!(!fixture
                .temporary
                .path()
                .join(&artifact.2)
                .join("manifest.json")
                .exists());
            assert_eq!(fixture.service.reconcile().unwrap().moved, 0);
            fixture
                .service
                .restore_then(&[id], || {
                    fixture
                        .repository
                        .exploration_exclusions_restore(&[GalleryId::new(id).unwrap()])
                        .map_err(Into::into)
                })
                .unwrap();
            assert_eq!(
                fixture.artifact(&entry),
                (state.into(), "incomplete".into(), format!("album-{id}"))
            );
            assert!(fixture
                .temporary
                .path()
                .join(format!("album-{id}/0001.webp"))
                .exists());
            assert_eq!(fixture.service.reconcile().unwrap().moved, 0);
        }
    }

    #[test]
    fn completed_manifest_paths_follow_round_trip() {
        let fixture = Fixture::new();
        let entry = fixture.seed(103, "completed");
        let before = fixture
            .repository
            .artifact_bundle_get(&DownloadEntryId::new(entry.clone()).unwrap())
            .unwrap()
            .unwrap();
        let original_manifest = ArtifactManifest::from_bundle(&before).unwrap();
        let report = fixture.service.reconcile().unwrap();
        assert!(report.issues.is_empty(), "{:?}", report.issues);
        let moved = fixture
            .repository
            .artifact_bundle_get(&DownloadEntryId::new(entry.clone()).unwrap())
            .unwrap()
            .unwrap();
        let actual = fixture
            .service
            .store
            .read_manifest(
                &layout(fixture.temporary.path(), &moved.artifact.relative_directory).unwrap(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(actual, ArtifactManifest::from_bundle(&moved).unwrap());
        assert_eq!(moved.artifact.state, DownloadArtifactState::Complete);
        fixture.service.restore_then(&[103], || Ok(())).unwrap();
        let restored = fixture
            .repository
            .artifact_bundle_get(&DownloadEntryId::new(entry).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            ArtifactManifest::from_bundle(&restored).unwrap(),
            original_manifest
        );
    }

    #[test]
    fn interrupted_rename_resumes_and_keeps_the_recovery_backup() {
        let fixture = Fixture::new();
        let entry = fixture.seed(104, "cancelled");
        let record = fixture.service.begin(&entry).unwrap();
        fixture
            .service
            .write_backup(&checked_root(&record.root).unwrap(), &record)
            .unwrap();
        fixture
            .service
            .store
            .move_managed_directory(&record.root, &record.original, &record.excluded)
            .unwrap();
        // Simulate process exit between filesystem rename and SQLite commit.
        let restarted = ExcludedArtifactService::new(
            fixture.repository.clone(),
            Arc::new(FilesystemArtifactStore::new()),
        );
        let report = restarted.reconcile().unwrap();
        assert_eq!(report.moved, 1);
        assert!(report.issues.is_empty(), "{:?}", report.issues);
        let backup = fixture.temporary.path().join(format!(
            ".atsumi-excluded/{entry}/.atsumi-recovery-{}.json",
            record.record_id
        ));
        assert_eq!(fs::read_to_string(backup).unwrap(), record.backup_json);
        assert_eq!(fixture.artifact(&entry).2, record.excluded.as_str());
    }

    #[test]
    fn restore_conflict_never_clears_the_exclusion_or_overwrites_new_files() {
        let fixture = Fixture::new();
        let entry = fixture.seed(105, "failed");
        assert!(fixture.service.reconcile().unwrap().issues.is_empty());
        fs::create_dir(fixture.temporary.path().join("album-105")).unwrap();
        fs::write(
            fixture.temporary.path().join("album-105/new.webp"),
            b"new file",
        )
        .unwrap();
        let called = std::sync::atomic::AtomicBool::new(false);
        assert!(fixture
            .service
            .restore_then(&[105], || {
                called.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
            .is_err());
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(
            fs::read(fixture.temporary.path().join("album-105/new.webp")).unwrap(),
            b"new file"
        );
        assert!(fixture.artifact(&entry).2.starts_with(".atsumi-excluded/"));
    }

    #[test]
    fn manually_excluded_and_restored_galleries_are_not_moved() {
        let fixture = Fixture::new();
        let entry = fixture.seed(106, "completed");
        fixture
            .repository
            .exploration_exclusions_restore(&[GalleryId::new(106).unwrap()])
            .unwrap();
        let manual_entry = fixture.seed(109, "completed");
        {
            let connection = fixture.repository.connection().unwrap();
            connection
                .execute(
                    "DELETE FROM duplicate_hidden_galleries WHERE gallery_id=109",
                    [],
                )
                .unwrap();
            connection.execute("INSERT INTO auto_find_exclusions(gallery_id,reason,created_at) VALUES(109,'manual','2026-09-01T00:00:00Z')",[]).unwrap();
        }
        let report = fixture.service.reconcile().unwrap();
        assert_eq!(report.moved, 0);
        assert!(report.issues.is_empty());
        assert_eq!(fixture.artifact(&entry).2, "album-106");
        assert_eq!(fixture.artifact(&manual_entry).2, "album-109");
    }

    #[test]
    fn nested_managed_album_blocks_parent_relocation() {
        let fixture = Fixture::new();
        fixture.seed(107, "cancelled");
        fixture.seed_in(
            108,
            "cancelled",
            &fixture.temporary.path().join("album-107"),
        );
        let report = fixture.service.reconcile().unwrap();
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.contains("another managed album")));
        assert!(fixture
            .temporary
            .path()
            .join("album-107/0001.webp")
            .exists());
    }

    #[test]
    fn interrupted_restore_finishes_before_ordinary_verification() {
        let fixture = Fixture::new();
        let entry = fixture.seed(110, "failed");
        assert!(fixture.service.reconcile().unwrap().issues.is_empty());
        let mut record = fixture
            .service
            .records("state='excluded'")
            .unwrap()
            .remove(0);
        record.state = "pending_restore".into();
        record.artifact_revision += 1;
        fixture.repository.connection().unwrap().execute(
            "UPDATE excluded_artifact_relocations SET state='pending_restore',artifact_revision=?1 WHERE record_id=?2",
            params![record.artifact_revision,record.record_id],
        ).unwrap();
        fixture
            .service
            .store
            .move_managed_directory(&record.root, &record.excluded, &record.original)
            .unwrap();
        let report = fixture.service.reconcile_pending().unwrap();
        assert_eq!(report.restored, 1);
        assert!(report.issues.is_empty(), "{:?}", report.issues);
        assert_eq!(
            fixture.artifact(&entry),
            ("failed".into(), "incomplete".into(), "album-110".into())
        );
        assert!(fixture
            .temporary
            .path()
            .join("album-110/0001.webp")
            .exists());
    }

    #[test]
    fn aliased_root_cannot_hide_a_nested_managed_album() {
        let fixture = Fixture::new();
        fixture.seed(112, "cancelled");
        fs::create_dir(fixture.temporary.path().join("alias-anchor")).unwrap();
        fixture.seed_in(
            113,
            "cancelled",
            &fixture.temporary.path().join("alias-anchor/../album-112"),
        );
        let report = fixture.service.reconcile().unwrap();
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.contains("another managed album")));
        assert_eq!(
            fs::read(fixture.temporary.path().join("album-112/0001.webp")).unwrap(),
            b"original page bytes"
        );
    }

    #[test]
    fn immutable_path_trigger_rejects_changes_without_an_exact_pending_saga() {
        let fixture = Fixture::new();
        let entry = fixture.seed(111, "cancelled");
        let connection = fixture.repository.connection().unwrap();
        assert!(connection.execute(
            "UPDATE download_artifacts SET relative_directory='other',revision=revision+1 WHERE entry_id=?1",[&entry],
        ).is_err());
        drop(connection);
        let record = fixture.service.begin(&entry).unwrap();
        let connection = fixture.repository.connection().unwrap();
        assert!(connection.execute(
            "UPDATE download_artifacts SET relative_directory=?1,revision=revision+1,state='complete' WHERE entry_id=?2",
            params![record.excluded.as_str(),entry],
        ).is_err());
        assert!(connection.execute(
            "UPDATE download_artifacts SET relative_directory='other',revision=revision+1 WHERE entry_id=?1",[&entry],
        ).is_err());
    }
}
