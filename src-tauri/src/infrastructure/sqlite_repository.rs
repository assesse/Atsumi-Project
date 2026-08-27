use std::{
    collections::BTreeMap,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{
    params, Connection, ErrorCode, OptionalExtension, Row, Transaction, TransactionBehavior,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    application::{
        ArtifactRepository, AutomationRepository, DownloadArtifactPlan, DownloadCheckpoint,
        DownloadMutationOutcome, DownloadOverlapRepository, DownloadPageAttempt,
        DownloadPageAttemptResult, DownloadPipelineRepository, DownloadPrepared,
        DownloadQueueAddOutcome, DownloadQueueRecord, DownloadRepository, DuplicateRepository,
        QuarantineSaga, QuarantineSagaState, RepositoryError, StateRepository, StoredPage,
        TagCatalogRepository,
    },
    domain::{
        ArtifactBundle, ArtifactManifest, ArtifactRelativePath, ArtifactSha256,
        ArtifactStorageFormat, AutoFindCandidate, AutoFindCandidateRecord, AutoFindCutoffEvidence,
        AutoFindExclusionResult, AutoFindHistoryMode, AutoFindRun, AutoFindRunState,
        AutoFindSnapshot, AutoFindTruncation, DownloadArtifact, DownloadArtifactState,
        DownloadChangedEvent, DownloadEntry, DownloadEntryId, DownloadJobDescriptor,
        DownloadJobProjection, DownloadListRequest, DownloadOverlapCandidate,
        DownloadOverlapCandidateIdentity, DownloadOverlapDecisionAction,
        DownloadOverlapDecisionApplied, DownloadOverlapDecisionApplyOutcome,
        DownloadOverlapDecisionRequest, DownloadOverlapDecisionResult, DownloadOverlapGalleryRef,
        DownloadOverlapPagePair, DownloadOverlapPairDecision, DownloadOverlapRelation,
        DownloadOverlapReview, DownloadOverlapReviewDraft, DownloadOverlapReviewState,
        DownloadPage, DownloadReviewKind, DuplicateCandidate, DuplicateCandidateRecord,
        DuplicateDecisionAction, DuplicateDecisionApplyOutcome, DuplicateDecisionHistory,
        DuplicateDecisionRequest, DuplicateEvidence, DuplicateEvidenceKind, DuplicateGalleryRef,
        DuplicatePageHash, DuplicatePagePair, DuplicateRelation, DuplicateReview, DuplicateScanRun,
        DuplicateScanState, DuplicateSnapshot, ExplorationDataResetResult, ExplorationExclusion,
        ExplorationExclusionKind, ExplorationExclusionReason, ExplorationExclusionRestoreResult,
        FavoriteKey, FavoriteMutationResult, FavoriteNamespace, FavoriteRecord,
        FixtureDownloadJobStep, Gallery, GalleryGroupingMode, GalleryId, GalleryMetadata,
        GallerySummary, HashProfile, JobEvent, JobRef, JobState, Language, PageArtifact,
        PageArtifactState, SearchHistoryEntry, SearchRequest, SearchSort, SeriesGroup,
        SettingsSnapshot, SourcePageNumber, TagCatalogEntry, TagCatalogStatus, TagNamespace,
        TagSuggestion, TagSuggestionRequest, WindowPlacementSnapshot,
        DOWNLOAD_OVERLAP_MAX_STORED_PAGE_PAIRS,
    },
};

use super::migrations::{MigrationError, MigrationRunner};

pub struct SqliteRepository {
    connection: Mutex<Connection>,
}

impl SqliteRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RepositoryError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                RepositoryError::Other(format!("could not create the database directory: {error}"))
            })?;
        }
        let existing_database = match std::fs::metadata(path) {
            Ok(metadata) => metadata.is_file() && metadata.len() > 0,
            Err(error) if error.kind() == ErrorKind::NotFound => false,
            Err(error) => {
                return Err(RepositoryError::Other(format!(
                    "could not inspect the database file: {error}"
                )))
            }
        };
        let connection = Connection::open(path).map_err(map_sqlite_error)?;
        Self::from_connection(
            connection,
            Some(FileDatabase {
                path,
                existing_database,
            }),
        )
    }

    pub fn open_in_memory() -> Result<Self, RepositoryError> {
        let connection = Connection::open_in_memory().map_err(map_sqlite_error)?;
        Self::from_connection(connection, None)
    }

    fn from_connection(
        mut connection: Connection,
        file_database: Option<FileDatabase<'_>>,
    ) -> Result<Self, RepositoryError> {
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(map_sqlite_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(map_sqlite_error)?;
        let backup_path = file_database
            .filter(|database| database.existing_database)
            .map(|database| database.path);
        let report = run_migrations_with_backup(&mut connection, backup_path)?;
        if file_database.is_some() {
            let journal_mode: String = connection
                .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
                .map_err(map_sqlite_error)?;
            if !journal_mode.eq_ignore_ascii_case("wal") {
                return Err(RepositoryError::Other(format!(
                    "SQLite refused WAL journal mode and returned {journal_mode:?}"
                )));
            }
            connection
                .execute_batch("PRAGMA synchronous = NORMAL;")
                .map_err(map_sqlite_error)?;
        }
        tracing::info!(
            schema_version = report.current_version,
            migrations_applied = ?report.applied_versions,
            "SQLite schema is ready"
        );
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub(crate) fn connection(&self) -> Result<MutexGuard<'_, Connection>, RepositoryError> {
        self.connection
            .lock()
            .map_err(|_| RepositoryError::Other("database mutex was poisoned".into()))
    }
}

#[derive(Clone, Copy)]
struct FileDatabase<'a> {
    path: &'a Path,
    existing_database: bool,
}

fn run_migrations_with_backup(
    connection: &mut Connection,
    existing_database_path: Option<&Path>,
) -> Result<super::migrations::MigrationReport, RepositoryError> {
    if let Some(database_path) = existing_database_path {
        let pending_versions =
            MigrationRunner::pending_versions(connection).map_err(map_migration_error)?;
        if let (Some(first_pending), Some(target_version)) =
            (pending_versions.first(), pending_versions.last())
        {
            let backup_path = create_pre_migration_backup(
                connection,
                database_path,
                first_pending - 1,
                *target_version,
            )?;
            tracing::info!(
                backup_file = %backup_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("pre-migration-backup.bak"),
                from_version = first_pending - 1,
                to_version = target_version,
                "Created a recoverable pre-migration database backup"
            );
        }
    }

    MigrationRunner::run(connection).map_err(map_migration_error)
}

fn create_pre_migration_backup(
    connection: &Connection,
    database_path: &Path,
    from_version: i64,
    to_version: i64,
) -> Result<PathBuf, RepositoryError> {
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            RepositoryError::MigrationBackup(format!(
                "the system clock is before the Unix epoch: {error}"
            ))
        })?
        .as_secs();
    let backup_path =
        next_pre_migration_backup_path(database_path, from_version, to_version, created_at)?;
    let backup_path_text = backup_path.to_str().ok_or_else(|| {
        RepositoryError::MigrationBackup(
            "the pre-migration backup path is not valid Unicode".into(),
        )
    })?;
    connection
        .execute("VACUUM main INTO ?1", [backup_path_text])
        .map_err(|error| {
            RepositoryError::MigrationBackup(format!(
                "could not create pre-migration backup {}: {error}",
                backup_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("snapshot.bak")
            ))
        })?;
    Ok(backup_path)
}

fn next_pre_migration_backup_path(
    database_path: &Path,
    from_version: i64,
    to_version: i64,
    created_at: u64,
) -> Result<PathBuf, RepositoryError> {
    let parent = database_path.parent().ok_or_else(|| {
        RepositoryError::MigrationBackup(
            "the database has no directory for a pre-migration backup".into(),
        )
    })?;
    let file_name = database_path.file_name().ok_or_else(|| {
        RepositoryError::MigrationBackup(
            "the database has no file name for a pre-migration backup".into(),
        )
    })?;

    for sequence in 0..10_000_u32 {
        let mut backup_name = file_name.to_os_string();
        backup_name.push(format!(
            ".pre-migration-v{from_version}-to-v{to_version}-{created_at}"
        ));
        if sequence > 0 {
            backup_name.push(format!("-{sequence}"));
        }
        backup_name.push(".bak");
        let candidate = parent.join(backup_name);
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => continue,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(candidate),
            Err(error) => {
                return Err(RepositoryError::MigrationBackup(format!(
                    "could not inspect a pre-migration backup path: {error}"
                )))
            }
        }
    }

    Err(RepositoryError::MigrationBackup(
        "could not reserve a non-overwriting pre-migration backup name".into(),
    ))
}

impl StateRepository for SqliteRepository {
    fn settings_get(&self) -> Result<SettingsSnapshot, RepositoryError> {
        let connection = self.connection()?;
        read_settings(&connection)
    }

    fn settings_compare_and_set(
        &self,
        next: &SettingsSnapshot,
        expected_revision: u64,
    ) -> Result<bool, RepositoryError> {
        let connection = self.connection()?;
        let changed = connection
            .execute(
                r#"
                    UPDATE settings
                    SET revision = ?1,
                        download_root = ?2,
                        folder_name_template = ?3,
                        max_columns = ?4,
                        preview_width = ?5,
                        related_preview_width = ?6,
                        cache_limit_gb = ?7,
                        concurrent_image_requests = ?8,
                        request_start_interval_ms = ?9,
                        auto_find_history_mode = ?10,
                        auto_find_grouping = ?11,
                        downloads_grouping = ?12,
                        privacy_mode = ?13,
                        collapsed_group_keys_json = ?14,
                        search_include_tags_json = ?15,
                        search_exclude_tags_json = ?16
                    WHERE singleton = 1 AND revision = ?17
                "#,
                params![
                    to_sql_integer(next.revision, "settings revision")?,
                    next.download_root,
                    next.folder_name_template,
                    i64::from(next.max_columns),
                    i64::from(next.preview_width),
                    i64::from(next.related_preview_width),
                    i64::from(next.cache_limit_gb),
                    i64::from(next.concurrent_image_requests),
                    to_sql_integer(next.request_start_interval_ms, "request start interval")?,
                    next.auto_find_history_mode.as_str(),
                    next.auto_find_grouping.as_str(),
                    next.downloads_grouping.as_str(),
                    next.privacy_mode,
                    serde_json::to_string(&next.collapsed_group_keys).map_err(domain_corruption)?,
                    serde_json::to_string(&next.search_include_tags).map_err(domain_corruption)?,
                    serde_json::to_string(&next.search_exclude_tags).map_err(domain_corruption)?,
                    to_sql_integer(expected_revision, "expected settings revision")?,
                ],
            )
            .map_err(map_sqlite_error)?;
        Ok(changed == 1)
    }

    fn window_placement_get(&self) -> Result<WindowPlacementSnapshot, RepositoryError> {
        let connection = self.connection()?;
        read_window_placement(&connection)
    }

    fn window_placement_compare_and_set(
        &self,
        next: &WindowPlacementSnapshot,
        expected_revision: u64,
    ) -> Result<bool, RepositoryError> {
        let connection = self.connection()?;
        let changed = connection
            .execute(
                r#"
                    UPDATE window_placement
                    SET revision = ?1,
                        x = ?2,
                        y = ?3,
                        width = ?4,
                        height = ?5,
                        maximized = ?6
                    WHERE singleton = 1 AND revision = ?7
                "#,
                params![
                    to_sql_integer(next.revision, "window placement revision")?,
                    next.x,
                    next.y,
                    i64::from(next.width),
                    i64::from(next.height),
                    next.maximized,
                    to_sql_integer(expected_revision, "expected window placement revision")?,
                ],
            )
            .map_err(map_sqlite_error)?;
        Ok(changed == 1)
    }
}

impl TagCatalogRepository for SqliteRepository {
    fn tag_catalog_status(&self) -> Result<TagCatalogStatus, RepositoryError> {
        let connection = self.connection()?;
        read_tag_catalog_status(&connection)
    }

    fn tag_catalog_record_attempt(&self) -> Result<(), RepositoryError> {
        let connection = self.connection()?;
        connection.execute("UPDATE tag_catalog_state SET last_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), last_error_code = NULL, last_error_message = NULL WHERE singleton = 1", [])
            .map_err(map_sqlite_error)?;
        Ok(())
    }

    fn tag_catalog_replace(
        &self,
        entries: &[TagCatalogEntry],
    ) -> Result<TagCatalogStatus, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        transaction
            .execute("DELETE FROM tag_catalog_entries", [])
            .map_err(map_sqlite_error)?;
        transaction
            .execute("DELETE FROM metadata_catalog_entries", [])
            .map_err(map_sqlite_error)?;
        let mut neutral = 0u64;
        let mut female = 0u64;
        let mut male = 0u64;
        let mut artist = 0u64;
        let mut group = 0u64;
        for entry in entries {
            match entry.namespace {
                TagNamespace::Artist => artist += 1,
                TagNamespace::Group => group += 1,
                TagNamespace::Tag => neutral += 1,
                TagNamespace::Female => female += 1,
                TagNamespace::Male => male += 1,
            }
            let insert = match entry.namespace {
                TagNamespace::Artist | TagNamespace::Group => {
                    "INSERT INTO metadata_catalog_entries (namespace, name, normalized_name, canonical_token, gallery_count, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))"
                }
                TagNamespace::Tag | TagNamespace::Female | TagNamespace::Male => {
                    "INSERT INTO tag_catalog_entries (namespace, name, normalized_name, canonical_token, gallery_count, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))"
                }
            };
            transaction
                .execute(
                    insert,
                    params![
                        entry.namespace.as_str(),
                        entry.name,
                        entry.normalized_name,
                        entry.canonical_token,
                        to_sql_integer(entry.gallery_count, "tag gallery count")?
                    ],
                )
                .map_err(map_sqlite_error)?;
        }
        transaction.execute(
            "UPDATE tag_catalog_state SET revision = revision + 1, entry_count = ?1, neutral_count = ?2, female_count = ?3, male_count = ?4, artist_count = ?5, group_count = ?6, last_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), last_success_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), last_error_code = NULL, last_error_message = NULL WHERE singleton = 1",
            params![to_sql_integer(entries.len() as u64, "catalog entry count")?, to_sql_integer(neutral, "neutral count")?, to_sql_integer(female, "female count")?, to_sql_integer(male, "male count")?, to_sql_integer(artist, "artist count")?, to_sql_integer(group, "group count")?],
        ).map_err(map_sqlite_error)?;
        let status = read_tag_catalog_status(&transaction)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(status)
    }

    fn tag_catalog_record_failure(&self, code: &str, message: &str) -> Result<(), RepositoryError> {
        let connection = self.connection()?;
        connection.execute("UPDATE tag_catalog_state SET last_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), last_error_code = ?1, last_error_message = ?2 WHERE singleton = 1", params![code, message]).map_err(map_sqlite_error)?;
        Ok(())
    }

    fn tag_suggestions_search(
        &self,
        request: &TagSuggestionRequest,
    ) -> Result<Vec<TagSuggestion>, RepositoryError> {
        if request.query.chars().count() < 2 || request.limit == 0 {
            return Ok(Vec::new());
        }
        let connection = self.connection()?;
        let namespace = request.namespace.map(TagNamespace::as_str);
        let needle = request.query.as_str();
        let mut statement = connection
            .prepare(
                r#"WITH suggestion_entries AS (
                SELECT namespace, name, normalized_name, canonical_token, gallery_count
                  FROM tag_catalog_entries
                UNION ALL
                SELECT namespace, name, normalized_name, canonical_token, gallery_count
                  FROM metadata_catalog_entries
              )
              SELECT e.namespace, e.name, e.canonical_token, e.gallery_count,
                EXISTS(
                  SELECT 1 FROM favorites f
                   WHERE f.namespace = CASE e.namespace
                     WHEN 'artist' THEN 'artist'
                     WHEN 'group' THEN 'group'
                     ELSE 'tag'
                   END
                     AND lower(replace(f.value, '_', ' ')) = lower(CASE e.namespace
                       WHEN 'female' THEN 'female:' || e.name
                       WHEN 'male' THEN 'male:' || e.name
                       ELSE e.name
                     END)
                )
              FROM suggestion_entries e
              WHERE instr(e.normalized_name, ?1) > 0 AND (?2 IS NULL OR e.namespace = ?2)
              ORDER BY 5 DESC, e.gallery_count DESC, e.normalized_name COLLATE NOCASE ASC,
                CASE e.namespace
                  WHEN 'artist' THEN 0 WHEN 'group' THEN 1 WHEN 'female' THEN 2
                  WHEN 'male' THEN 3 ELSE 4
                END ASC,
                e.canonical_token COLLATE NOCASE ASC LIMIT ?3"#,
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map(
                params![needle, namespace, i64::from(request.limit)],
                |row| {
                    let namespace: String = row.get(0)?;
                    Ok(TagSuggestion {
                        namespace: TagNamespace::parse(&namespace)
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        name: row.get(1)?,
                        token: row.get(2)?,
                        gallery_count: row.get::<_, i64>(3)? as u64,
                        favorite: row.get(4)?,
                    })
                },
            )
            .map_err(map_sqlite_error)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(map_sqlite_error)
    }
}

impl AutomationRepository for SqliteRepository {
    fn favorites_list(&self) -> Result<Vec<FavoriteRecord>, RepositoryError> {
        let connection = self.connection()?;
        read_favorites(&connection)
    }

    fn favorite_set(
        &self,
        key: &FavoriteKey,
        enabled: bool,
    ) -> Result<FavoriteMutationResult, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        if enabled {
            transaction
                .execute(
                    r#"
                        INSERT INTO favorites (
                            namespace, value, revision, created_at, updated_at
                        ) VALUES (
                            ?1, ?2, 0,
                            strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                            strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        )
                        ON CONFLICT(namespace, value) DO UPDATE SET
                            revision = favorites.revision + 1,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    "#,
                    params![key.namespace.as_str(), key.value],
                )
                .map_err(map_sqlite_error)?;
            let favorite = read_favorite(&transaction, key)?.ok_or_else(|| {
                RepositoryError::Corrupt("favorite upsert did not produce a row".into())
            })?;
            transaction.commit().map_err(map_sqlite_error)?;
            Ok(FavoriteMutationResult {
                enabled: true,
                favorite: Some(favorite),
            })
        } else {
            transaction
                .execute(
                    "DELETE FROM favorites WHERE namespace = ?1 AND value = ?2",
                    params![key.namespace.as_str(), key.value],
                )
                .map_err(map_sqlite_error)?;
            transaction.commit().map_err(map_sqlite_error)?;
            Ok(FavoriteMutationResult {
                enabled: false,
                favorite: None,
            })
        }
    }

    fn search_history_record(
        &self,
        request: &SearchRequest,
    ) -> Result<SearchHistoryEntry, RepositoryError> {
        let canonical = serde_json::to_vec(request)
            .map_err(|error| RepositoryError::Other(error.to_string()))?;
        let fingerprint = format!("{:x}", Sha256::digest(canonical));
        let include_tags = serde_json::to_string(&request.include_tags)
            .map_err(|error| RepositoryError::Other(error.to_string()))?;
        let exclude_tags = serde_json::to_string(&request.exclude_tags)
            .map_err(|error| RepositoryError::Other(error.to_string()))?;
        let languages = serde_json::to_string(&request.languages)
            .map_err(|error| RepositoryError::Other(error.to_string()))?;
        let connection = self.connection()?;
        connection
            .execute(
                r#"
                    INSERT INTO search_history (
                        fingerprint, text, include_tags_json, exclude_tags_json,
                        languages_json, sort, page_size, use_count, last_used_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, 1,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                    ON CONFLICT(fingerprint) DO UPDATE SET
                        use_count = search_history.use_count + 1,
                        last_used_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                "#,
                params![
                    fingerprint,
                    request.text,
                    include_tags,
                    exclude_tags,
                    languages,
                    search_sort_text(request.sort),
                    i64::from(request.page_size),
                ],
            )
            .map_err(map_sqlite_error)?;
        read_search_history_by_fingerprint(&connection, &fingerprint)?.ok_or_else(|| {
            RepositoryError::Corrupt("search history upsert did not produce a row".into())
        })
    }

    fn search_history_list(&self, limit: u32) -> Result<Vec<SearchHistoryEntry>, RepositoryError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                r#"
                    SELECT history_id, text, include_tags_json, exclude_tags_json,
                           languages_json, sort, page_size, use_count, last_used_at
                    FROM search_history
                    ORDER BY last_used_at DESC, history_id DESC
                    LIMIT ?1
                "#,
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([i64::from(limit)], stored_search_history)
            .map_err(map_sqlite_error)?;
        rows.map(|row| row.map_err(map_sqlite_error)?.try_into_domain())
            .collect()
    }

    fn auto_find_recover_interrupted(&self) -> Result<usize, RepositoryError> {
        let connection = self.connection()?;
        connection
            .execute(
                r#"
                    UPDATE auto_find_runs
                    SET revision = revision + 1,
                        state = 'failed',
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        error_code = 'AUTO_FIND_INTERRUPTED',
                        error_message = 'The previous Auto Find refresh stopped before completion'
                    WHERE state = 'running'
                "#,
                [],
            )
            .map_err(map_sqlite_error)
    }

    fn auto_find_owned_cutoffs(
        &self,
        artists: &[String],
    ) -> Result<Vec<AutoFindCutoffEvidence>, RepositoryError> {
        let connection = self.connection()?;
        artists
            .iter()
            .map(|artist| read_auto_find_owned_cutoff(&connection, artist))
            .collect()
    }

    fn auto_find_start(
        &self,
        total_favorites: u32,
        history_mode: AutoFindHistoryMode,
        cutoff_evidence: &[AutoFindCutoffEvidence],
    ) -> Result<AutoFindRun, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        if let Some(existing) = read_running_auto_find(&transaction)? {
            transaction.commit().map_err(map_sqlite_error)?;
            return Ok(existing);
        }
        let run_id = format!("auto-find-{}", Uuid::new_v4());
        transaction
            .execute(
                r#"
                    INSERT INTO auto_find_runs (
                        run_id, revision, state, total_favorites,
                        completed_favorites, candidates_found, history_mode,
                        started_at, updated_at
                    ) VALUES (
                        ?1, 0, 'running', ?2, 0, 0, ?3,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                "#,
                params![run_id, i64::from(total_favorites), history_mode.as_str()],
            )
            .map_err(map_sqlite_error)?;
        for cutoff in cutoff_evidence {
            transaction.execute(
                "INSERT INTO auto_find_run_cutoffs (run_id, artist, oldest_owned_gallery_id, qualified_owned_count, cutoff_source, policy_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![run_id, cutoff.artist, cutoff.oldest_owned_gallery_id.map(GalleryId::get), i64::from(cutoff.qualified_owned_count), cutoff.source, i64::from(cutoff.policy_version)],
            ).map_err(map_sqlite_error)?;
        }
        let run = read_auto_find_run(&transaction, &run_id)?.ok_or_else(|| {
            RepositoryError::Corrupt("Auto Find start did not produce a run".into())
        })?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(run)
    }

    fn auto_find_truncation_add(
        &self,
        run_id: &str,
        truncation: &AutoFindTruncation,
    ) -> Result<(), RepositoryError> {
        let connection = self.connection()?;
        connection.execute(
            "INSERT OR REPLACE INTO auto_find_run_truncations (run_id, artist, reason, eligible_count, candidate_limit) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, truncation.artist, truncation.reason, i64::from(truncation.eligible_count), i64::from(truncation.limit)],
        ).map_err(map_sqlite_error)?;
        Ok(())
    }

    fn auto_find_candidate_add(
        &self,
        candidate: &AutoFindCandidateRecord,
    ) -> Result<Option<AutoFindRun>, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        if !auto_find_run_is_running(&transaction, &candidate.run_id)? {
            transaction.commit().map_err(map_sqlite_error)?;
            return Ok(None);
        }
        let excluded: bool = transaction
            .query_row(
                r#"
                    SELECT EXISTS (
                        SELECT 1 FROM auto_find_exclusions WHERE gallery_id = ?1
                        UNION ALL
                        SELECT 1 FROM download_entries WHERE gallery_id = ?1
                        UNION ALL
                        SELECT 1 FROM duplicate_hidden_galleries WHERE gallery_id = ?1
                          AND NOT EXISTS (
                              SELECT 1 FROM exploration_restored_galleries
                              WHERE gallery_id = ?1
                          )
                    )
                "#,
                [candidate.gallery.id.get()],
                |row| row.get(0),
            )
            .map_err(map_sqlite_error)?;
        if excluded {
            transaction.commit().map_err(map_sqlite_error)?;
            return Ok(None);
        }
        let tags = serde_json::to_string(&candidate.gallery.tags)
            .map_err(|error| RepositoryError::Other(error.to_string()))?;
        let series = serde_json::to_string(&candidate.gallery.series)
            .map_err(|error| RepositoryError::Other(error.to_string()))?;
        let characters = serde_json::to_string(&candidate.gallery.characters)
            .map_err(|error| RepositoryError::Other(error.to_string()))?;
        let inserted = transaction
            .execute(
                r#"
                    INSERT OR IGNORE INTO auto_find_candidates (
                        run_id, gallery_id, title, artist, group_name, pages,
                        language, tags_json, series_json, characters_json,
                        published_rank, popularity,
                        thumbnail_key, thumbnail_width, thumbnail_height,
                        favorite_namespace, favorite_value, discovered_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                        ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                "#,
                params![
                    candidate.run_id,
                    candidate.gallery.id.get(),
                    candidate.gallery.title,
                    candidate.gallery.artist,
                    candidate.gallery.group,
                    i64::from(candidate.gallery.pages),
                    language_text(candidate.gallery.language),
                    tags,
                    series,
                    characters,
                    i64::from(candidate.gallery.published_rank),
                    i64::from(candidate.gallery.popularity),
                    candidate.gallery.thumbnail_key,
                    i64::from(candidate.gallery.thumbnail_width),
                    i64::from(candidate.gallery.thumbnail_height),
                    candidate.matched_favorite.namespace.as_str(),
                    candidate.matched_favorite.value,
                ],
            )
            .map_err(map_sqlite_error)?;
        if inserted == 1 {
            transaction
                .execute(
                    r#"
                        UPDATE auto_find_runs
                        SET revision = revision + 1,
                            candidates_found = candidates_found + 1,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        WHERE run_id = ?1 AND state = 'running'
                    "#,
                    [candidate.run_id.as_str()],
                )
                .map_err(map_sqlite_error)?;
        }
        let run = read_auto_find_run(&transaction, &candidate.run_id)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(run.filter(|_| inserted == 1))
    }

    fn auto_find_progress(
        &self,
        run_id: &str,
        completed_favorites: u32,
    ) -> Result<Option<AutoFindRun>, RepositoryError> {
        let connection = self.connection()?;
        connection
            .execute(
                r#"
                    UPDATE auto_find_runs
                    SET revision = revision + 1,
                        completed_favorites = MIN(total_favorites, MAX(completed_favorites, ?2)),
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    WHERE run_id = ?1 AND state = 'running'
                "#,
                params![run_id, i64::from(completed_favorites)],
            )
            .map_err(map_sqlite_error)?;
        read_auto_find_run(&connection, run_id)
    }

    fn auto_find_finish(
        &self,
        run_id: &str,
        state: AutoFindRunState,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<Option<AutoFindRun>, RepositoryError> {
        if state == AutoFindRunState::Running {
            return Err(RepositoryError::Other(
                "Auto Find finish cannot keep a run in running state".into(),
            ));
        }
        let connection = self.connection()?;
        connection
            .execute(
                r#"
                    UPDATE auto_find_runs
                    SET revision = revision + 1,
                        state = ?2,
                        completed_favorites = CASE
                            WHEN ?2 = 'completed' THEN total_favorites
                            ELSE completed_favorites
                        END,
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        error_code = ?3,
                        error_message = ?4
                    WHERE run_id = ?1 AND state = 'running'
                "#,
                params![run_id, state.as_str(), error_code, error_message],
            )
            .map_err(map_sqlite_error)?;
        read_auto_find_run(&connection, run_id)
    }

    fn auto_find_is_running(&self, run_id: &str) -> Result<bool, RepositoryError> {
        let connection = self.connection()?;
        auto_find_run_is_running(&connection, run_id)
    }

    fn auto_find_snapshot(&self) -> Result<AutoFindSnapshot, RepositoryError> {
        let connection = self.connection()?;
        read_auto_find_snapshot(&connection)
    }

    fn auto_find_exclude(
        &self,
        gallery_ids: &[GalleryId],
        reason: &str,
    ) -> Result<AutoFindExclusionResult, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        for gallery_id in gallery_ids {
            transaction
                .execute(
                    "DELETE FROM exploration_restored_galleries WHERE gallery_id = ?1",
                    [gallery_id.get()],
                )
                .map_err(map_sqlite_error)?;
            transaction
                .execute(
                    r#"
                        INSERT INTO auto_find_exclusions (gallery_id, reason, created_at)
                        VALUES (
                            ?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        )
                        ON CONFLICT(gallery_id) DO UPDATE SET reason = excluded.reason
                    "#,
                    params![gallery_id.get(), reason],
                )
                .map_err(map_sqlite_error)?;
        }
        let snapshot = read_auto_find_snapshot(&transaction)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(AutoFindExclusionResult {
            excluded_gallery_ids: gallery_ids.to_vec(),
            snapshot,
        })
    }

    fn exploration_exclusions_list(&self) -> Result<Vec<ExplorationExclusion>, RepositoryError> {
        let connection = self.connection()?;
        read_exploration_exclusions(&connection)
    }

    fn exploration_exclusions_restore(
        &self,
        gallery_ids: &[GalleryId],
    ) -> Result<ExplorationExclusionRestoreResult, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        for gallery_id in gallery_ids {
            transaction
                .execute(
                    "DELETE FROM auto_find_exclusions WHERE gallery_id = ?1",
                    [gallery_id.get()],
                )
                .map_err(map_sqlite_error)?;
            transaction
                .execute(
                    r#"
                        INSERT INTO exploration_restored_galleries (gallery_id, restored_at)
                        VALUES (?1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                        ON CONFLICT(gallery_id) DO UPDATE SET restored_at = excluded.restored_at
                    "#,
                    [gallery_id.get()],
                )
                .map_err(map_sqlite_error)?;
        }
        let snapshot = read_auto_find_snapshot(&transaction)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(ExplorationExclusionRestoreResult {
            restored_gallery_ids: gallery_ids.to_vec(),
            snapshot,
        })
    }

    fn exploration_data_reset(&self) -> Result<ExplorationDataResetResult, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let running: bool = transaction
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM auto_find_runs WHERE state='running')",
                [],
                |row| row.get(0),
            )
            .map_err(map_sqlite_error)?;
        if running {
            return Err(RepositoryError::OperationActive(
                "Auto Find must finish or be cancelled before resetting exploration data".into(),
            ));
        }
        let count = |table: &str| -> Result<u64, RepositoryError> {
            let sql = format!("SELECT COUNT(*) FROM {table}");
            let value: i64 = transaction
                .query_row(&sql, [], |row| row.get(0))
                .map_err(map_sqlite_error)?;
            u64::try_from(value)
                .map_err(|_| RepositoryError::Corrupt(format!("negative row count in {table}")))
        };
        let result = ExplorationDataResetResult {
            favorites_removed: count("favorites")?,
            search_history_removed: count("search_history")?,
            auto_find_runs_removed: count("auto_find_runs")?,
            auto_find_candidates_removed: count("auto_find_candidates")?,
            auto_find_exclusions_removed: count("auto_find_exclusions")?,
        };
        transaction
            .execute("DELETE FROM favorites", [])
            .map_err(map_sqlite_error)?;
        transaction
            .execute("DELETE FROM search_history", [])
            .map_err(map_sqlite_error)?;
        transaction
            .execute("DELETE FROM auto_find_runs", [])
            .map_err(map_sqlite_error)?;
        transaction
            .execute("DELETE FROM auto_find_exclusions", [])
            .map_err(map_sqlite_error)?;
        transaction
            .execute("DELETE FROM exploration_restored_galleries", [])
            .map_err(map_sqlite_error)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(result)
    }
}

impl DownloadRepository for SqliteRepository {
    fn download_recover_interrupted(&self) -> Result<usize, RepositoryError> {
        let mut connection = self.connection()?;
        recover_volatile_downloads(&mut connection)
    }

    fn download_queue_add(
        &self,
        request_id: &str,
        galleries: &[GalleryId],
    ) -> Result<DownloadQueueAddOutcome, RepositoryError> {
        let normalized_galleries = serde_json::to_string(
            &galleries
                .iter()
                .map(|gallery| gallery.get())
                .collect::<Vec<_>>(),
        )
        .map_err(|error| RepositoryError::Other(error.to_string()))?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;

        let existing_batch = transaction
            .query_row(
                r#"
                    SELECT normalized_galleries
                    FROM download_queue_requests
                    WHERE request_id = ?1
                "#,
                [request_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(map_sqlite_error)?;

        if let Some(existing_batch) = existing_batch {
            if existing_batch != normalized_galleries {
                transaction.commit().map_err(map_sqlite_error)?;
                return Ok(DownloadQueueAddOutcome::IdempotencyConflict);
            }

            let entries = read_request_entries(&transaction, request_id)?;
            if entries.len() != galleries.len() {
                return Err(RepositoryError::Corrupt(format!(
                    "download queue request {request_id:?} has an incomplete entry mapping"
                )));
            }
            transaction.commit().map_err(map_sqlite_error)?;
            return Ok(DownloadQueueAddOutcome::Added(DownloadQueueRecord {
                entries,
                jobs: Vec::new(),
            }));
        }

        transaction
            .execute(
                r#"
                    INSERT INTO download_queue_requests (
                        request_id, normalized_galleries
                    ) VALUES (?1, ?2)
                "#,
                params![request_id, normalized_galleries],
            )
            .map_err(map_sqlite_error)?;

        let mut jobs = Vec::new();
        for (position, gallery_id) in galleries.iter().enumerate() {
            let existing_entry_id = transaction
                .query_row(
                    r#"
                        SELECT entry_id
                        FROM download_entries
                        WHERE gallery_id = ?1
                          AND state IN (
                              'queued', 'resolving_metadata', 'downloading',
                              'hashing', 'verifying', 'retry_wait'
                          )
                        LIMIT 1
                    "#,
                    [gallery_id.get()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(map_sqlite_error)?;

            let entry_id = match existing_entry_id {
                Some(entry_id) => entry_id,
                None => {
                    let entry_id = format!("entry-{}", Uuid::new_v4());
                    let job_id = format!("job-{}", Uuid::new_v4());
                    let job_request_id = format!("download-{}", Uuid::new_v4());
                    transaction
                        .execute(
                            r#"
                                INSERT INTO download_entries (
                                    entry_id, gallery_id, revision, state, progress,
                                    created_at, updated_at
                                ) VALUES (
                                    ?1, ?2, 0, 'queued', 0.0,
                                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                                )
                            "#,
                            params![entry_id, gallery_id.get()],
                        )
                        .map_err(map_sqlite_error)?;
                    transaction
                        .execute(
                            r#"
                                INSERT INTO download_jobs (
                                    job_id, request_id, entry_id, gallery_id,
                                    revision, state, completed_units, total_units,
                                    attempt, created_at, updated_at
                                ) VALUES (
                                    ?1, ?2, ?3, ?4, 0, 'queued', 0, 1, 1,
                                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                                )
                            "#,
                            params![job_id, job_request_id, entry_id, gallery_id.get()],
                        )
                        .map_err(map_sqlite_error)?;
                    transaction
                        .execute(
                            r#"
                                INSERT INTO download_attempts (
                                    job_id, attempt, started_at
                                ) VALUES (
                                    ?1, 1,
                                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                                )
                            "#,
                            [&job_id],
                        )
                        .map_err(map_sqlite_error)?;
                    jobs.push(DownloadJobDescriptor {
                        job_id,
                        entry_id: entry_id.clone(),
                        gallery_id: *gallery_id,
                        worker_attempt: 1,
                    });
                    entry_id
                }
            };

            transaction
                .execute(
                    r#"
                        INSERT INTO download_queue_request_entries (
                            request_id, position, gallery_id, entry_id,
                            response_revision, response_state, response_progress,
                            response_review_kind, response_review_id
                        )
                        SELECT ?1, ?2, ?3, d.entry_id,
                               d.revision, d.state, d.progress, d.review_kind, d.review_id
                        FROM download_entries d
                        WHERE d.entry_id = ?4
                    "#,
                    params![
                        request_id,
                        to_sql_integer(position as u64, "queue position")?,
                        gallery_id.get(),
                        entry_id,
                    ],
                )
                .map_err(map_sqlite_error)?;
        }

        let entries = read_request_entries(&transaction, request_id)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(DownloadQueueAddOutcome::Added(DownloadQueueRecord {
            entries,
            jobs,
        }))
    }

    fn download_entries_list(
        &self,
        request: &DownloadListRequest,
    ) -> Result<DownloadPage, RepositoryError> {
        let connection = self.connection()?;
        let state = request.state.map(|state| state.to_string());
        let query = request.query.as_deref();
        let total_items = connection
            .query_row(
                r#"
                    SELECT COUNT(*)
                    FROM download_entries d
                    WHERE (?1 IS NULL OR d.state = ?1)
                      AND NOT (
                          d.state = 'cancelled'
                          AND (
                              EXISTS (
                                  SELECT 1
                                  FROM download_overlap_reviews overlap_review
                                  WHERE overlap_review.entry_id = d.entry_id
                                    AND overlap_review.state = 'cancelled'
                              )
                              OR EXISTS (
                                  SELECT 1
                                  FROM download_overlap_candidates overlap_candidate
                                  WHERE overlap_candidate.existing_entry_id = d.entry_id
                                    AND overlap_candidate.decision = 'existing_removed'
                              )
                          )
                      )
                      AND NOT EXISTS (
                          SELECT 1 FROM duplicate_hidden_galleries hidden
                          WHERE hidden.gallery_id = d.gallery_id
                            AND NOT EXISTS (
                                SELECT 1 FROM exploration_restored_galleries restored
                                WHERE restored.gallery_id = d.gallery_id
                            )
                      )
                      AND (
                          ?2 IS NULL
                          OR instr(lower(d.entry_id), ?2) > 0
                          OR instr(CAST(d.gallery_id AS TEXT), ?2) > 0
                      )
                "#,
                params![state, query],
                |row| row.get::<_, i64>(0),
            )
            .map_err(map_sqlite_error)?;
        let total_items = stored_u64(total_items, "download total items")?;
        let offset = u64::from(request.page - 1)
            .checked_mul(u64::from(request.page_size))
            .ok_or_else(|| RepositoryError::Other("download list offset overflowed".into()))?;

        let mut statement = connection
            .prepare(
                r#"
                    SELECT
                        d.entry_id, d.gallery_id, d.revision, d.state, d.progress,
                        d.review_kind, d.review_id,
                        j.attempt, j.last_error_code, j.last_error_message,
                        j.last_error_retryable, d.created_at, d.updated_at
                    FROM download_entries d
                    JOIN download_jobs j
                      ON j.entry_id = d.entry_id AND j.gallery_id = d.gallery_id
                    WHERE (?1 IS NULL OR d.state = ?1)
                      AND NOT (
                          d.state = 'cancelled'
                          AND (
                              EXISTS (
                                  SELECT 1
                                  FROM download_overlap_reviews overlap_review
                                  WHERE overlap_review.entry_id = d.entry_id
                                    AND overlap_review.state = 'cancelled'
                              )
                              OR EXISTS (
                                  SELECT 1
                                  FROM download_overlap_candidates overlap_candidate
                                  WHERE overlap_candidate.existing_entry_id = d.entry_id
                                    AND overlap_candidate.decision = 'existing_removed'
                              )
                          )
                      )
                      AND NOT EXISTS (
                          SELECT 1 FROM duplicate_hidden_galleries hidden
                          WHERE hidden.gallery_id = d.gallery_id
                            AND NOT EXISTS (
                                SELECT 1 FROM exploration_restored_galleries restored
                                WHERE restored.gallery_id = d.gallery_id
                            )
                      )
                      AND (
                          ?2 IS NULL
                          OR instr(lower(d.entry_id), ?2) > 0
                          OR instr(CAST(d.gallery_id AS TEXT), ?2) > 0
                      )
                    ORDER BY d.gallery_id ASC, d.entry_id ASC
                    LIMIT ?3 OFFSET ?4
                "#,
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map(
                params![
                    state,
                    query,
                    i64::from(request.page_size),
                    to_sql_integer(offset, "download list offset")?,
                ],
                stored_download_entry,
            )
            .map_err(map_sqlite_error)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.map_err(map_sqlite_error)?.try_into_domain()?);
        }

        Ok(DownloadPage {
            page: request.page,
            total_items,
            entries,
        })
    }

    fn download_active_count(&self) -> Result<u64, RepositoryError> {
        let connection = self.connection()?;
        let count = connection
            .query_row(
                r#"
                    SELECT COUNT(*)
                    FROM download_entries
                    WHERE state IN (
                        'queued', 'resolving_metadata', 'downloading',
                        'hashing', 'verifying', 'retry_wait'
                    )
                "#,
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(map_sqlite_error)?;
        stored_u64(count, "active download count")
    }

    fn download_active_entry_ids(&self) -> Result<Vec<DownloadEntryId>, RepositoryError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                r#"
                    SELECT entry_id
                    FROM download_entries
                    WHERE state IN (
                        'queued', 'resolving_metadata', 'downloading',
                        'hashing', 'verifying', 'retry_wait'
                    )
                    ORDER BY entry_id ASC
                "#,
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(map_sqlite_error)?;
        let mut entry_ids = Vec::new();
        for row in rows {
            entry_ids.push(
                DownloadEntryId::new(row.map_err(map_sqlite_error)?).map_err(domain_corruption)?,
            );
        }
        Ok(entry_ids)
    }

    fn download_retry(
        &self,
        entry_ids: &[DownloadEntryId],
    ) -> Result<DownloadMutationOutcome<Vec<JobRef>>, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let mut job_refs = Vec::with_capacity(entry_ids.len());

        for entry_id in entry_ids {
            let Some(target) = read_download_target(&transaction, entry_id)? else {
                return Ok(DownloadMutationOutcome::EntryNotFound(entry_id.clone()));
            };

            if target.state.is_active() {
                job_refs.push(JobRef {
                    job_id: target.job_id,
                    reused: true,
                    worker_attempt: stored_u64(target.attempt, "download attempt")?,
                });
                continue;
            }
            if !target.state.is_retryable() || !target.state.allows_transition_to(JobState::Queued)
            {
                return Ok(DownloadMutationOutcome::InvalidState {
                    entry_id: entry_id.clone(),
                    state: target.state,
                });
            }

            if let Some(active_job_id) =
                active_job_for_gallery(&transaction, target.gallery_id, target.entry_id.as_str())?
            {
                job_refs.push(JobRef {
                    job_id: active_job_id,
                    reused: true,
                    worker_attempt: stored_u64(target.attempt, "download attempt")?,
                });
                continue;
            }

            let job_revision = next_stored_revision(target.job_revision, "job revision")?;
            let entry_revision = next_stored_revision(target.entry_revision, "download revision")?;
            let attempt = next_stored_revision(target.attempt, "download attempt")?;
            let changed_jobs = transaction
                .execute(
                    r#"
                        UPDATE download_jobs
                        SET revision = ?1,
                            state = 'queued',
                            attempt = ?2,
                            completed_units = 0,
                            total_units = 1,
                            last_error_code = NULL,
                            last_error_message = NULL,
                            last_error_retryable = NULL,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                            started_at = NULL,
                            finished_at = NULL
                        WHERE job_id = ?3 AND revision = ?4 AND state = ?5
                    "#,
                    params![
                        to_sql_integer(job_revision, "job revision")?,
                        to_sql_integer(attempt, "download attempt")?,
                        target.job_id,
                        target.job_revision,
                        target.state.to_string(),
                    ],
                )
                .map_err(map_sqlite_error)?;
            let changed_entries = transaction
                .execute(
                    r#"
                        UPDATE download_entries
                        SET revision = ?1,
                            state = 'queued',
                            progress = 0,
                            review_kind = NULL,
                            review_id = NULL,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        WHERE entry_id = ?2 AND revision = ?3 AND state = ?4
                    "#,
                    params![
                        to_sql_integer(entry_revision, "download revision")?,
                        target.entry_id.as_str(),
                        target.entry_revision,
                        target.state.to_string(),
                    ],
                )
                .map_err(map_sqlite_error)?;
            if changed_jobs != 1 || changed_entries != 1 {
                return Err(RepositoryError::Other(format!(
                    "download entry {:?} changed concurrently while retrying",
                    target.entry_id.as_str()
                )));
            }
            transaction
                .execute(
                    r#"
                        INSERT INTO download_attempts (
                            job_id, attempt, started_at
                        ) VALUES (
                            ?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        )
                    "#,
                    params![target.job_id, to_sql_integer(attempt, "download attempt")?],
                )
                .map_err(map_sqlite_error)?;
            job_refs.push(JobRef {
                job_id: target.job_id,
                reused: false,
                worker_attempt: attempt,
            });
        }

        transaction.commit().map_err(map_sqlite_error)?;
        Ok(DownloadMutationOutcome::Applied(job_refs))
    }

    fn download_cancel(
        &self,
        entry_ids: &[DownloadEntryId],
    ) -> Result<DownloadMutationOutcome<Vec<DownloadEntry>>, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;

        for entry_id in entry_ids {
            let Some(target) = read_download_target(&transaction, entry_id)? else {
                return Ok(DownloadMutationOutcome::EntryNotFound(entry_id.clone()));
            };
            if target.state == JobState::Cancelled {
                continue;
            }
            if !target.state.allows_transition_to(JobState::Cancelled) {
                return Ok(DownloadMutationOutcome::InvalidState {
                    entry_id: entry_id.clone(),
                    state: target.state,
                });
            }

            let job_revision = next_stored_revision(target.job_revision, "job revision")?;
            let entry_revision = next_stored_revision(target.entry_revision, "download revision")?;
            let changed_jobs = transaction
                .execute(
                    r#"
                        UPDATE download_jobs
                        SET revision = ?1,
                            state = 'cancelled',
                            last_error_code = CASE
                                WHEN ?4 IN ('interrupted', 'failed') THEN last_error_code
                                ELSE NULL
                            END,
                            last_error_message = CASE
                                WHEN ?4 IN ('interrupted', 'failed') THEN last_error_message
                                ELSE NULL
                            END,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                            finished_at = COALESCE(
                                finished_at,
                                strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                            )
                        WHERE job_id = ?2 AND revision = ?3 AND state = ?4
                    "#,
                    params![
                        to_sql_integer(job_revision, "job revision")?,
                        target.job_id,
                        target.job_revision,
                        target.state.to_string(),
                    ],
                )
                .map_err(map_sqlite_error)?;
            let changed_entries = transaction
                .execute(
                    r#"
                        UPDATE download_entries
                        SET revision = ?1,
                            state = 'cancelled',
                            review_kind = NULL,
                            review_id = NULL,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        WHERE entry_id = ?2 AND revision = ?3 AND state = ?4
                    "#,
                    params![
                        to_sql_integer(entry_revision, "download revision")?,
                        target.entry_id.as_str(),
                        target.entry_revision,
                        target.state.to_string(),
                    ],
                )
                .map_err(map_sqlite_error)?;
            if changed_jobs != 1 || changed_entries != 1 {
                return Err(RepositoryError::Other(format!(
                    "download entry {:?} changed concurrently while cancelling",
                    target.entry_id.as_str()
                )));
            }
            transaction
                .execute(
                    r#"
                        UPDATE download_attempts
                        SET finished_at = COALESCE(
                                finished_at,
                                strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                            ),
                            outcome_state = 'cancelled',
                            error_code = NULL,
                            error_message = NULL
                        WHERE job_id = ?1 AND attempt = ?2 AND finished_at IS NULL
                    "#,
                    params![target.job_id, target.attempt],
                )
                .map_err(map_sqlite_error)?;
        }

        let mut entries = Vec::with_capacity(entry_ids.len());
        for entry_id in entry_ids {
            let target = read_download_target(&transaction, entry_id)?.ok_or_else(|| {
                RepositoryError::Corrupt(format!(
                    "cancelled download entry {:?} disappeared",
                    entry_id.as_str()
                ))
            })?;
            entries.push(target.into_download_entry()?);
        }
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(DownloadMutationOutcome::Applied(entries))
    }

    fn fixture_download_job_advance(
        &self,
        job_id: &str,
        worker_attempt: u64,
        step: FixtureDownloadJobStep,
    ) -> Result<DownloadJobProjection, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;

        let (
            entry_id,
            gallery_id,
            job_revision,
            download_revision,
            stored_job_state,
            stored_download_state,
            stored_attempt,
        ) = transaction
            .query_row(
                r#"
                    SELECT
                        j.entry_id, j.gallery_id, j.revision, d.revision,
                        j.state, d.state, j.attempt
                    FROM download_jobs j
                    JOIN download_entries d
                      ON d.entry_id = j.entry_id AND d.gallery_id = j.gallery_id
                    WHERE j.job_id = ?1
                "#,
                [job_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .map_err(map_sqlite_error)?;
        let current_attempt = stored_u64(stored_attempt, "download attempt")?;
        if current_attempt != worker_attempt {
            return Err(RepositoryError::Other(format!(
                "fixture job {job_id:?} worker attempt {worker_attempt} is stale; current attempt is {current_attempt}"
            )));
        }
        let current_job_state = stored_job_state
            .parse::<JobState>()
            .map_err(domain_corruption)?;
        let current_download_state = stored_download_state
            .parse::<JobState>()
            .map_err(domain_corruption)?;
        if current_job_state != current_download_state {
            return Err(RepositoryError::Corrupt(format!(
                "fixture job {job_id:?} and its download entry disagree on state"
            )));
        }
        if !step.follows(current_job_state) {
            return Err(RepositoryError::Other(format!(
                "fixture job {job_id:?} cannot advance from {current_job_state} to {}",
                step.state()
            )));
        }

        let job_revision = next_stored_revision(job_revision, "job revision")?;
        let download_revision = next_stored_revision(download_revision, "download revision")?;
        let state = step.state();
        let stored_state = state.to_string();
        let completed_units = step.completed_units();
        let total_units = step.total_units();
        let progress = completed_units as f64 / total_units as f64 * 100.0;
        let (last_error_code, last_error_message) = match step {
            FixtureDownloadJobStep::ResolvingMetadata => (None, None),
            FixtureDownloadJobStep::FoundationUnavailable => (
                Some("DOWNLOAD_FOUNDATION_UNAVAILABLE"),
                Some(step.message()),
            ),
        };

        let changed_jobs = transaction
            .execute(
                r#"
                    UPDATE download_jobs
                    SET revision = ?1,
                        state = ?2,
                        completed_units = ?3,
                        total_units = ?4,
                        last_error_code = ?5,
                        last_error_message = ?6,
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        started_at = CASE
                            WHEN ?2 = 'resolving_metadata'
                            THEN COALESCE(
                                started_at,
                                strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                            )
                            ELSE started_at
                        END,
                        finished_at = CASE
                            WHEN ?2 = 'interrupted'
                            THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                            ELSE NULL
                        END
                    WHERE job_id = ?7 AND revision = ?8 AND state = ?9 AND attempt = ?10
                "#,
                params![
                    to_sql_integer(job_revision, "job revision")?,
                    stored_state,
                    to_sql_integer(completed_units, "completed units")?,
                    to_sql_integer(total_units, "total units")?,
                    last_error_code,
                    last_error_message,
                    job_id,
                    to_sql_integer(job_revision - 1, "expected job revision")?,
                    current_job_state.to_string(),
                    to_sql_integer(worker_attempt, "worker attempt")?,
                ],
            )
            .map_err(map_sqlite_error)?;
        let changed_downloads = transaction
            .execute(
                r#"
                    UPDATE download_entries
                    SET revision = ?1,
                        state = ?2,
                        progress = ?3,
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    WHERE entry_id = ?4 AND revision = ?5 AND state = ?6
                "#,
                params![
                    to_sql_integer(download_revision, "download revision")?,
                    stored_state,
                    progress,
                    entry_id,
                    to_sql_integer(download_revision - 1, "expected download revision")?,
                    current_download_state.to_string(),
                ],
            )
            .map_err(map_sqlite_error)?;
        if changed_jobs != 1 || changed_downloads != 1 {
            return Err(RepositoryError::Other(format!(
                "fixture job {job_id:?} changed concurrently"
            )));
        }
        if step == FixtureDownloadJobStep::FoundationUnavailable {
            transaction
                .execute(
                    r#"
                        UPDATE download_attempts
                        SET finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                            outcome_state = 'interrupted',
                            error_code = ?1,
                            error_message = ?2
                        WHERE job_id = ?3 AND attempt = ?4
                    "#,
                    params![
                        last_error_code,
                        last_error_message,
                        job_id,
                        to_sql_integer(worker_attempt, "worker attempt")?,
                    ],
                )
                .map_err(map_sqlite_error)?;
        }
        transaction.commit().map_err(map_sqlite_error)?;

        Ok(DownloadJobProjection {
            job: JobEvent {
                job_id: job_id.to_owned(),
                gallery_id: Some(gallery_id),
                revision: job_revision,
                state,
                completed_units: Some(completed_units),
                total_units: Some(total_units),
                message: Some(step.message().to_owned()),
            },
            download: DownloadChangedEvent {
                entry_id,
                gallery_id,
                revision: download_revision,
                state,
                progress: Some(progress),
                attempt: Some(worker_attempt),
                error_code: last_error_code.map(str::to_owned),
                error_message: last_error_message.map(str::to_owned),
                review_kind: None,
                review_id: None,
            },
        })
    }
}

impl ArtifactRepository for SqliteRepository {
    fn artifact_bundle_replace(&self, bundle: &ArtifactBundle) -> Result<(), RepositoryError> {
        bundle
            .validate()
            .map_err(|error| RepositoryError::Other(error.to_string()))?;

        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;

        transaction
            .execute(
                r#"
                    INSERT INTO galleries (
                        gallery_id, revision, title, primary_artist, primary_group,
                        source_page_count
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                    ON CONFLICT (gallery_id) DO UPDATE SET
                        revision = excluded.revision,
                        title = excluded.title,
                        primary_artist = excluded.primary_artist,
                        primary_group = excluded.primary_group,
                        source_page_count = excluded.source_page_count
                "#,
                params![
                    bundle.gallery.id.get(),
                    to_sql_integer(bundle.gallery.revision, "gallery revision")?,
                    bundle.gallery.metadata.title,
                    bundle.gallery.metadata.primary_artist,
                    bundle.gallery.metadata.primary_group,
                    i64::from(bundle.gallery.metadata.source_page_count),
                ],
            )
            .map_err(map_sqlite_error)?;

        transaction
            .execute(
                "DELETE FROM owned_gallery_artists WHERE gallery_id = ?1",
                [bundle.gallery.id.get()],
            )
            .map_err(map_sqlite_error)?;
        for artist in &bundle.gallery.metadata.artists {
            transaction
                .execute(
                    "INSERT INTO owned_gallery_artists (gallery_id, artist) VALUES (?1, ?2)",
                    params![bundle.gallery.id.get(), artist],
                )
                .map_err(map_sqlite_error)?;
        }

        let entry_gallery_id = transaction
            .query_row(
                "SELECT gallery_id FROM download_entries WHERE entry_id = ?1",
                [bundle.artifact.entry_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(map_sqlite_error)?;
        match entry_gallery_id {
            Some(gallery_id) if gallery_id == bundle.gallery.id.get() => {}
            Some(_) => {
                return Err(RepositoryError::Other(
                    "download artifact gallery does not match its download entry".into(),
                ));
            }
            None => {
                return Err(RepositoryError::Other(
                    "download artifact requires an existing download entry".into(),
                ));
            }
        }

        let stored_artifact_path = transaction
            .query_row(
                "SELECT relative_directory FROM download_artifacts WHERE entry_id = ?1",
                [bundle.artifact.entry_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(map_sqlite_error)?;
        if stored_artifact_path
            .as_deref()
            .is_some_and(|path| path != bundle.artifact.relative_directory.as_str())
        {
            return Err(RepositoryError::Other(
                "download artifact relative directory is immutable".into(),
            ));
        }

        transaction
            .execute(
                r#"
                    INSERT INTO download_artifacts (
                        entry_id, gallery_id, revision, relative_directory,
                        expected_page_count, state, manifest_relative_path,
                        manifest_schema_version, writer_version,
                        hash_profile_version, completed_at, root_snapshot
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                        (SELECT download_root FROM settings WHERE singleton = 1)
                    )
                    ON CONFLICT (entry_id) DO UPDATE SET
                        gallery_id = excluded.gallery_id,
                        revision = excluded.revision,
                        expected_page_count = excluded.expected_page_count,
                        state = excluded.state,
                        manifest_schema_version = excluded.manifest_schema_version,
                        writer_version = excluded.writer_version,
                        hash_profile_version = excluded.hash_profile_version,
                        completed_at = excluded.completed_at
                "#,
                params![
                    bundle.artifact.entry_id.as_str(),
                    bundle.artifact.gallery_id.get(),
                    to_sql_integer(bundle.artifact.revision, "download artifact revision")?,
                    bundle.artifact.relative_directory.as_str(),
                    i64::from(bundle.artifact.expected_page_count),
                    bundle.artifact.state.as_str(),
                    bundle
                        .artifact
                        .manifest_relative_path
                        .as_ref()
                        .map(ArtifactRelativePath::as_str),
                    bundle.artifact.manifest_schema_version.map(i64::from),
                    bundle.artifact.writer_version,
                    i64::from(bundle.artifact.hash_profile_version),
                    bundle.artifact.completed_at,
                ],
            )
            .map_err(map_sqlite_error)?;

        transaction
            .execute(
                "DELETE FROM download_pages WHERE entry_id = ?1",
                [bundle.artifact.entry_id.as_str()],
            )
            .map_err(map_sqlite_error)?;
        for page in &bundle.pages {
            transaction
                .execute(
                    r#"
                        INSERT INTO download_pages (
                            entry_id, gallery_id, source_page_number,
                            relative_path, state, byte_length, sha256,
                            storage_format, source_revision, verified_at, excluded
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                    "#,
                    params![
                        page.entry_id.as_str(),
                        page.page_id.gallery_id.get(),
                        i64::from(page.page_id.source_page_number.get()),
                        page.relative_path.as_str(),
                        page.state.as_str(),
                        page.byte_length
                            .map(|value| to_sql_integer(value, "page byte length"))
                            .transpose()?,
                        page.sha256.as_ref().map(ArtifactSha256::as_str),
                        page.storage_format.map(ArtifactStorageFormat::as_str),
                        page.source_revision,
                        page.verified_at,
                        page.excluded,
                    ],
                )
                .map_err(map_sqlite_error)?;
        }

        transaction.commit().map_err(map_sqlite_error)
    }

    fn artifact_bundle_get(
        &self,
        entry_id: &DownloadEntryId,
    ) -> Result<Option<ArtifactBundle>, RepositoryError> {
        let connection = self.connection()?;
        let stored = connection
            .query_row(
                r#"
                    SELECT
                        g.gallery_id,
                        g.revision,
                        g.title,
                        g.primary_artist,
                        g.primary_group,
                        g.source_page_count,
                        a.revision,
                        a.relative_directory,
                        a.expected_page_count,
                        a.state,
                        a.manifest_relative_path,
                        a.manifest_schema_version,
                        a.writer_version,
                        a.hash_profile_version,
                        a.completed_at
                    FROM download_artifacts a
                    JOIN galleries g ON g.gallery_id = a.gallery_id
                    WHERE a.entry_id = ?1
                "#,
                [entry_id.as_str()],
                stored_artifact_bundle,
            )
            .optional()
            .map_err(map_sqlite_error)?;
        let Some(stored) = stored else {
            return Ok(None);
        };

        let gallery_id = GalleryId::new(stored.gallery_id).map_err(domain_corruption)?;
        let artists = read_owned_gallery_artists(&connection, gallery_id)?;
        let metadata = GalleryMetadata::new(
            stored.title,
            stored.primary_artist,
            stored.primary_group,
            stored_u32(stored.source_page_count, "gallery source page count")?,
        )
        .map(|metadata| metadata.with_artists(artists))
        .map_err(domain_corruption)?;
        let gallery = Gallery::new(
            gallery_id,
            stored_u64(stored.gallery_revision, "gallery revision")?,
            metadata,
        );
        let mut artifact = DownloadArtifact::new(
            entry_id.clone(),
            gallery_id,
            stored_u64(stored.artifact_revision, "download artifact revision")?,
            ArtifactRelativePath::new(stored.relative_directory).map_err(domain_corruption)?,
            stored_u32(stored.expected_page_count, "expected page count")?,
            stored
                .artifact_state
                .parse::<DownloadArtifactState>()
                .map_err(domain_corruption)?,
        )
        .map_err(domain_corruption)?;
        artifact.hash_profile_version =
            stored_u32(stored.hash_profile_version, "artifact hash profile version")?;
        match (
            stored.manifest_relative_path,
            stored.manifest_schema_version,
            stored.writer_version,
            stored.completed_at,
        ) {
            (Some(path), Some(schema), Some(writer), Some(completed_at)) => {
                let hash_profile_version = artifact.hash_profile_version;
                artifact = artifact
                    .with_manifest(
                        ArtifactRelativePath::new(path).map_err(domain_corruption)?,
                        stored_u32(schema, "manifest schema version")?,
                        writer,
                        hash_profile_version,
                        completed_at,
                    )
                    .map_err(domain_corruption)?;
            }
            (Some(path), None, None, None) if artifact.state != DownloadArtifactState::Complete => {
                artifact.manifest_relative_path =
                    Some(ArtifactRelativePath::new(path).map_err(domain_corruption)?);
            }
            (None, None, None, None) => {}
            _ => {
                return Err(RepositoryError::Corrupt(
                    "download artifact has incomplete manifest metadata".into(),
                ));
            }
        }

        let mut statement = connection
            .prepare(
                r#"
                    SELECT gallery_id, source_page_number, relative_path, state, byte_length,
                           sha256, storage_format, source_revision, verified_at, excluded
                    FROM download_pages
                    WHERE entry_id = ?1
                    ORDER BY source_page_number ASC
                "#,
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([entry_id.as_str()], stored_page_artifact)
            .map_err(map_sqlite_error)?;
        let mut pages = Vec::new();
        for row in rows {
            let stored = row.map_err(map_sqlite_error)?;
            let page_gallery_id = GalleryId::new(stored.gallery_id).map_err(domain_corruption)?;
            let source_page_number =
                SourcePageNumber::new(stored_u32(stored.source_page_number, "source page number")?)
                    .map_err(domain_corruption)?;
            let byte_length = stored
                .byte_length
                .map(|value| stored_u64(value, "page byte length"))
                .transpose()?;
            let mut page = PageArtifact::new(
                entry_id.clone(),
                page_gallery_id,
                source_page_number,
                ArtifactRelativePath::new(stored.relative_path).map_err(domain_corruption)?,
                stored
                    .page_state
                    .parse::<PageArtifactState>()
                    .map_err(domain_corruption)?,
                byte_length,
            )
            .map_err(domain_corruption)?;
            match (
                stored.sha256,
                stored.storage_format,
                stored.source_revision,
                stored.verified_at,
            ) {
                (Some(sha256), Some(format), Some(source_revision), Some(verified_at)) => {
                    page = page
                        .with_verification(
                            ArtifactSha256::new(sha256).map_err(domain_corruption)?,
                            format
                                .parse::<ArtifactStorageFormat>()
                                .map_err(domain_corruption)?,
                            source_revision,
                            verified_at,
                        )
                        .map_err(domain_corruption)?;
                }
                (None, None, None, None) => {}
                _ => {
                    return Err(RepositoryError::Corrupt(
                        "download page has incomplete verification metadata".into(),
                    ));
                }
            }
            pages.push(page.with_excluded(stored.excluded));
        }

        ArtifactBundle::new(gallery, artifact, pages)
            .map(Some)
            .map_err(domain_corruption)
    }
}

impl DownloadPipelineRepository for SqliteRepository {
    fn pipeline_begin(
        &self,
        descriptor: &DownloadJobDescriptor,
    ) -> Result<DownloadJobProjection, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let target = read_pipeline_target(&transaction, descriptor)?;
        if target.state != JobState::Queued {
            if target.state == JobState::ResolvingMetadata {
                let projection =
                    target.into_projection(Some("Metadata resolution is already active"));
                transaction.commit().map_err(map_sqlite_error)?;
                return projection;
            }
            return Err(invalid_pipeline_state(&target, "begin"));
        }
        let projection = transition_pipeline_target(
            &transaction,
            target,
            JobState::ResolvingMetadata,
            None,
            None,
            None,
            None,
            "Resolving gallery metadata",
        )?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(projection)
    }

    fn pipeline_prepare(
        &self,
        plan: &DownloadArtifactPlan,
    ) -> Result<DownloadPrepared, RepositoryError> {
        if plan.source_pages.is_empty() {
            return Err(RepositoryError::Other(
                "download artifact plan must contain at least one source page".into(),
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let target = read_pipeline_target(&transaction, &plan.descriptor)?;
        if target.state != JobState::ResolvingMetadata {
            return Err(invalid_pipeline_state(&target, "prepare artifact"));
        }

        let source_revision = plan.source_revision.trim();
        if source_revision.is_empty() || source_revision.len() > 512 {
            return Err(RepositoryError::Other(
                "gallery source revision must contain between 1 and 512 bytes".into(),
            ));
        }
        let existing_gallery = transaction
            .query_row(
                "SELECT revision, source_revision FROM galleries WHERE gallery_id=?1",
                [plan.gallery.id.get()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(map_sqlite_error)?;
        let gallery_revision = match existing_gallery {
            Some((revision, Some(stored_source_revision)))
                if stored_source_revision == source_revision =>
            {
                revision
            }
            Some((revision, _)) => to_sql_integer(
                next_stored_revision(revision, "gallery revision")?,
                "gallery revision",
            )?,
            None => 0,
        };

        transaction
            .execute(
                r#"
                    INSERT INTO galleries (
                        gallery_id, revision, title, primary_artist, primary_group,
                        source_page_count, source_revision
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                    ON CONFLICT (gallery_id) DO UPDATE SET
                        revision = excluded.revision,
                        title = excluded.title,
                        primary_artist = excluded.primary_artist,
                        primary_group = excluded.primary_group,
                        source_page_count = excluded.source_page_count,
                        source_revision = excluded.source_revision
                "#,
                params![
                    plan.gallery.id.get(),
                    gallery_revision,
                    plan.gallery.metadata.title,
                    plan.gallery.metadata.primary_artist,
                    plan.gallery.metadata.primary_group,
                    i64::from(plan.gallery.metadata.source_page_count),
                    source_revision,
                ],
            )
            .map_err(map_sqlite_error)?;

        transaction
            .execute(
                "DELETE FROM owned_gallery_artists WHERE gallery_id = ?1",
                [plan.gallery.id.get()],
            )
            .map_err(map_sqlite_error)?;
        for artist in &plan.gallery.metadata.artists {
            transaction
                .execute(
                    "INSERT INTO owned_gallery_artists (gallery_id, artist) VALUES (?1, ?2)",
                    params![plan.gallery.id.get(), artist],
                )
                .map_err(map_sqlite_error)?;
        }

        let previous_artifact = transaction
            .query_row(
                "SELECT revision, relative_directory, manifest_relative_path, root_snapshot FROM download_artifacts WHERE entry_id = ?1",
                [&plan.descriptor.entry_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(map_sqlite_error)?;
        let artifact_created = previous_artifact.is_none();
        let artifact_revision = previous_artifact
            .as_ref()
            .map(|(revision, _, _, _)| next_stored_revision(*revision, "artifact revision"))
            .transpose()?
            .unwrap_or(0);
        let relative_directory = previous_artifact
            .as_ref()
            .map(|(_, path, _, _)| ArtifactRelativePath::new(path).map_err(domain_corruption))
            .transpose()?
            .unwrap_or_else(|| plan.relative_directory.clone());
        let manifest_relative_path = previous_artifact
            .as_ref()
            .and_then(|(_, _, path, _)| path.as_ref())
            .map(|path| ArtifactRelativePath::new(path).map_err(domain_corruption))
            .transpose()?
            .unwrap_or_else(|| {
                ArtifactRelativePath::new(format!("{}/manifest.json", relative_directory.as_str()))
                    .expect("stored artifact directory produces a relative manifest path")
            });
        let root_snapshot = previous_artifact
            .as_ref()
            .map(|(_, _, _, root)| PathBuf::from(root))
            .unwrap_or_else(|| plan.root_snapshot.clone());
        transaction
            .execute(
                r#"
                    INSERT INTO download_artifacts (
                        entry_id, gallery_id, revision, relative_directory,
                        expected_page_count, state, manifest_relative_path,
                        hash_profile_version, root_snapshot
                    ) VALUES (?1, ?2, ?3, ?4, ?5, 'incomplete', ?6, 1, ?7)
                    ON CONFLICT (entry_id) DO UPDATE SET
                        gallery_id = excluded.gallery_id,
                        revision = excluded.revision,
                        expected_page_count = excluded.expected_page_count,
                        state = 'incomplete',
                        manifest_schema_version = NULL,
                        writer_version = NULL,
                        completed_at = NULL
                "#,
                params![
                    plan.descriptor.entry_id,
                    plan.gallery.id.get(),
                    to_sql_integer(artifact_revision, "artifact revision")?,
                    relative_directory.as_str(),
                    i64::try_from(plan.source_pages.len()).map_err(|_| {
                        RepositoryError::Other("source page count exceeds SQLite range".into())
                    })?,
                    manifest_relative_path.as_str(),
                    root_snapshot.to_string_lossy(),
                ],
            )
            .map_err(map_sqlite_error)?;

        for source_page in &plan.source_pages {
            let relative_path = ArtifactRelativePath::new(format!(
                "{}/{:04}.webp",
                relative_directory.as_str(),
                source_page.source_page_number.get()
            ))
            .map_err(|error| RepositoryError::Other(error.to_string()))?;
            transaction
                .execute(
                    r#"
                        INSERT INTO download_pages (
                            entry_id, gallery_id, source_page_number,
                            relative_path, state, excluded
                        ) VALUES (?1, ?2, ?3, ?4, 'pending', 0)
                        ON CONFLICT (entry_id, source_page_number) DO UPDATE SET
                            gallery_id = excluded.gallery_id,
                            relative_path = excluded.relative_path
                    "#,
                    params![
                        plan.descriptor.entry_id,
                        plan.gallery.id.get(),
                        i64::from(source_page.source_page_number.get()),
                        relative_path.as_str(),
                    ],
                )
                .map_err(map_sqlite_error)?;
        }

        let unexpected_pages = transaction
            .query_row(
                r#"
                    SELECT COUNT(*)
                    FROM download_pages
                    WHERE entry_id = ?1
                      AND source_page_number > ?2
                "#,
                params![
                    plan.descriptor.entry_id,
                    i64::try_from(plan.source_pages.len()).unwrap_or(i64::MAX),
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(map_sqlite_error)?;
        if unexpected_pages != 0 {
            return Err(RepositoryError::Corrupt(
                "download artifact contains source pages beyond the current gallery metadata"
                    .into(),
            ));
        }

        let total_units = u64::try_from(plan.source_pages.len())
            .map_err(|_| RepositoryError::Other("download page count overflowed".into()))?;
        let verified_units = transaction
            .query_row(
                r#"
                    SELECT COUNT(*)
                    FROM download_pages
                    WHERE entry_id = ?1
                      AND state = 'present'
                      AND byte_length IS NOT NULL
                      AND sha256 IS NOT NULL
                      AND storage_format = 'webp'
                      AND source_revision IS NOT NULL
                      AND verified_at IS NOT NULL
                      AND excluded = 0
                "#,
                [&plan.descriptor.entry_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(map_sqlite_error)?;
        let verified_units = stored_u64(verified_units, "verified page count")?;
        let projection = transition_pipeline_target(
            &transaction,
            target,
            JobState::Downloading,
            Some(verified_units),
            Some(total_units),
            None,
            None,
            "Downloading verified source pages",
        )?;

        let mut statement = transaction
            .prepare(
                r#"
                    SELECT source_page_number, relative_path, byte_length,
                           sha256, storage_format, source_revision, verified_at, excluded
                    FROM download_pages
                    WHERE entry_id = ?1
                      AND state = 'present'
                      AND byte_length IS NOT NULL
                      AND sha256 IS NOT NULL
                      AND storage_format IS NOT NULL
                      AND source_revision IS NOT NULL
                      AND verified_at IS NOT NULL
                    ORDER BY source_page_number ASC
                "#,
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([&plan.descriptor.entry_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, bool>(7)?,
                ))
            })
            .map_err(map_sqlite_error)?;
        let mut checkpoints = Vec::new();
        for row in rows {
            let row = row.map_err(map_sqlite_error)?;
            checkpoints.push(DownloadCheckpoint {
                page: StoredPage {
                    source_page_number: SourcePageNumber::new(stored_u32(
                        row.0,
                        "checkpoint source page number",
                    )?)
                    .map_err(domain_corruption)?,
                    relative_path: ArtifactRelativePath::new(row.1).map_err(domain_corruption)?,
                    byte_length: stored_u64(row.2, "checkpoint byte length")?,
                    sha256: ArtifactSha256::new(row.3).map_err(domain_corruption)?,
                    storage_format: row
                        .4
                        .parse::<ArtifactStorageFormat>()
                        .map_err(domain_corruption)?,
                    source_revision: row.5,
                    verified_at: row.6,
                },
                excluded: row.7,
            });
        }
        drop(statement);
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(DownloadPrepared {
            projection,
            checkpoints,
            relative_directory,
            manifest_relative_path,
            root_snapshot,
            artifact_created,
        })
    }

    fn pipeline_page_attempt_start(
        &self,
        attempt: &DownloadPageAttempt,
    ) -> Result<(), RepositoryError> {
        let connection = self.connection()?;
        ensure_current_pipeline_attempt(&connection, &attempt.descriptor)?;
        connection
            .execute(
                r#"
                    INSERT INTO download_page_attempts (
                        job_id, job_attempt, source_page_number,
                        candidate_index, candidate_format, started_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                    ON CONFLICT (
                        job_id, job_attempt, source_page_number, candidate_index
                    ) DO NOTHING
                "#,
                params![
                    attempt.descriptor.job_id,
                    to_sql_integer(attempt.descriptor.worker_attempt, "download attempt")?,
                    i64::from(attempt.source_page_number.get()),
                    i64::from(attempt.candidate_index),
                    attempt.candidate_format,
                ],
            )
            .map_err(map_sqlite_error)?;
        Ok(())
    }

    fn pipeline_page_attempt_finish(
        &self,
        result: &DownloadPageAttemptResult,
    ) -> Result<(), RepositoryError> {
        let connection = self.connection()?;
        ensure_current_pipeline_attempt(&connection, &result.attempt.descriptor)?;
        connection
            .execute(
                r#"
                    INSERT INTO download_page_attempts (
                        job_id, job_attempt, source_page_number,
                        candidate_index, candidate_format, started_at, finished_at,
                        outcome, error_code, error_message, bytes_received,
                        http_status, content_type, retryable
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        ?6, ?7, ?8, ?9, ?10, ?11, ?12
                    )
                    ON CONFLICT (
                        job_id, job_attempt, source_page_number, candidate_index
                    ) DO UPDATE SET
                        finished_at = excluded.finished_at,
                        outcome = excluded.outcome,
                        error_code = excluded.error_code,
                        error_message = excluded.error_message,
                        bytes_received = excluded.bytes_received,
                        candidate_format = excluded.candidate_format,
                        http_status = excluded.http_status,
                        content_type = excluded.content_type,
                        retryable = excluded.retryable
                "#,
                params![
                    result.attempt.descriptor.job_id,
                    to_sql_integer(result.attempt.descriptor.worker_attempt, "download attempt")?,
                    i64::from(result.attempt.source_page_number.get()),
                    i64::from(result.attempt.candidate_index),
                    result.attempt.candidate_format,
                    result.outcome.as_str(),
                    result.error_code,
                    result.error_message,
                    result
                        .bytes_received
                        .map(|bytes| to_sql_integer(bytes, "received page bytes"))
                        .transpose()?,
                    result.http_status.map(i64::from),
                    result.content_type,
                    i64::from(u8::from(result.retryable)),
                ],
            )
            .map_err(map_sqlite_error)?;
        Ok(())
    }

    fn pipeline_page_verified(
        &self,
        descriptor: &DownloadJobDescriptor,
        page: &StoredPage,
    ) -> Result<DownloadJobProjection, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let target = read_pipeline_target(&transaction, descriptor)?;
        if target.state != JobState::Downloading {
            return Err(invalid_pipeline_state(&target, "record a verified page"));
        }
        let changed = transaction
            .execute(
                r#"
                    UPDATE download_pages
                    SET state = 'present',
                        relative_path = ?1,
                        byte_length = ?2,
                        sha256 = ?3,
                        storage_format = ?4,
                        source_revision = ?5,
                        verified_at = ?6
                    WHERE entry_id = ?7 AND source_page_number = ?8
                "#,
                params![
                    page.relative_path.as_str(),
                    to_sql_integer(page.byte_length, "page byte length")?,
                    page.sha256.as_str(),
                    page.storage_format.as_str(),
                    page.source_revision,
                    page.verified_at,
                    descriptor.entry_id,
                    i64::from(page.source_page_number.get()),
                ],
            )
            .map_err(map_sqlite_error)?;
        if changed != 1 {
            return Err(RepositoryError::Corrupt(format!(
                "download page {} has no prepared checkpoint",
                page.source_page_number.get()
            )));
        }
        let completed_units = transaction
            .query_row(
                r#"
                    SELECT COUNT(*)
                    FROM download_pages
                    WHERE entry_id = ?1 AND state = 'present' AND excluded = 0
                      AND byte_length IS NOT NULL AND sha256 IS NOT NULL
                      AND storage_format = 'webp' AND source_revision IS NOT NULL
                      AND verified_at IS NOT NULL
                "#,
                [&descriptor.entry_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(map_sqlite_error)?;
        let projection = update_pipeline_progress(
            &transaction,
            target,
            stored_u64(completed_units, "verified page count")?,
            "Verified a downloaded source page",
        )?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(projection)
    }

    fn pipeline_stage(
        &self,
        descriptor: &DownloadJobDescriptor,
        state: JobState,
        message: &'static str,
    ) -> Result<DownloadJobProjection, RepositoryError> {
        if !matches!(state, JobState::Hashing | JobState::Verifying) {
            return Err(RepositoryError::Other(
                "pipeline stage must be hashing or verifying".into(),
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let target = read_pipeline_target(&transaction, descriptor)?;
        if !target.state.allows_transition_to(state) {
            return Err(invalid_pipeline_state(&target, "advance the pipeline"));
        }
        let projection = transition_pipeline_target(
            &transaction,
            target,
            state,
            None,
            None,
            None,
            None,
            message,
        )?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(projection)
    }

    fn pipeline_complete(
        &self,
        descriptor: &DownloadJobDescriptor,
        manifest: &ArtifactManifest,
        manifest_relative_path: &ArtifactRelativePath,
    ) -> Result<DownloadJobProjection, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let target = read_pipeline_target(&transaction, descriptor)?;
        if target.state != JobState::Verifying {
            return Err(invalid_pipeline_state(&target, "complete the artifact"));
        }
        let expected = transaction
            .query_row(
                "SELECT expected_page_count FROM download_artifacts WHERE entry_id = ?1",
                [&descriptor.entry_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(map_sqlite_error)?;
        let verified = transaction
            .query_row(
                r#"
                    SELECT COUNT(*)
                    FROM download_pages
                    WHERE entry_id = ?1 AND state = 'present' AND excluded = 0
                      AND byte_length IS NOT NULL AND sha256 IS NOT NULL
                      AND storage_format = 'webp' AND source_revision IS NOT NULL
                      AND verified_at IS NOT NULL
                "#,
                [&descriptor.entry_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(map_sqlite_error)?;
        if expected != verified
            || stored_u32(expected, "expected artifact page count")? != manifest.expected_page_count
            || manifest.pages.len() != manifest.expected_page_count as usize
        {
            return Err(RepositoryError::Other(
                "artifact cannot complete before every source page is verified".into(),
            ));
        }
        let artifact_changed = transaction
            .execute(
                r#"
                    UPDATE download_artifacts
                    SET revision = revision + 1,
                        state = 'complete',
                        manifest_relative_path = ?1,
                        manifest_schema_version = ?2,
                        writer_version = ?3,
                        hash_profile_version = ?4,
                        completed_at = ?5
                    WHERE entry_id = ?6 AND state = 'incomplete'
                "#,
                params![
                    manifest_relative_path.as_str(),
                    i64::from(manifest.schema_version),
                    manifest.writer_version,
                    i64::from(manifest.hash_profile_version),
                    manifest.completed_at,
                    descriptor.entry_id,
                ],
            )
            .map_err(map_sqlite_error)?;
        if artifact_changed != 1 {
            return Err(RepositoryError::Other(
                "artifact changed concurrently while completing".into(),
            ));
        }
        let projection = transition_pipeline_target(
            &transaction,
            target,
            JobState::Completed,
            Some(stored_u64(verified, "verified page count")?),
            Some(stored_u64(expected, "expected page count")?),
            None,
            None,
            "Download completed and artifact integrity was verified",
        )?;
        transaction
            .execute(
                r#"
                    UPDATE download_attempts
                    SET finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        outcome_state = 'completed',
                        error_code = NULL,
                        error_message = NULL
                    WHERE job_id = ?1 AND attempt = ?2
                "#,
                params![
                    descriptor.job_id,
                    to_sql_integer(descriptor.worker_attempt, "download attempt")?,
                ],
            )
            .map_err(map_sqlite_error)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(projection)
    }

    fn pipeline_fail(
        &self,
        descriptor: &DownloadJobDescriptor,
        code: &str,
        message: &str,
        retryable: bool,
    ) -> Result<Option<DownloadJobProjection>, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let target = match read_pipeline_target(&transaction, descriptor) {
            Ok(target) => target,
            Err(RepositoryError::Other(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        if !target.state.is_active() || !target.state.allows_transition_to(JobState::Failed) {
            transaction.commit().map_err(map_sqlite_error)?;
            return Ok(None);
        }
        let projection = transition_pipeline_target(
            &transaction,
            target,
            JobState::Failed,
            None,
            None,
            Some(code),
            Some(message),
            "Download stopped before artifact verification completed",
        )?;
        transaction
            .execute(
                "UPDATE download_jobs SET last_error_retryable = ?1 WHERE job_id = ?2",
                params![i64::from(u8::from(retryable)), descriptor.job_id],
            )
            .map_err(map_sqlite_error)?;
        transaction
            .execute(
                r#"
                    UPDATE download_attempts
                    SET finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        outcome_state = 'failed', error_code = ?1, error_message = ?2,
                        error_retryable = ?3
                    WHERE job_id = ?4 AND attempt = ?5
                "#,
                params![
                    code,
                    message,
                    i64::from(u8::from(retryable)),
                    descriptor.job_id,
                    to_sql_integer(descriptor.worker_attempt, "download attempt")?,
                ],
            )
            .map_err(map_sqlite_error)?;
        if code == "ARTIFACT_DESTINATION_OCCUPIED" {
            transaction
                .execute(
                    r#"
                        DELETE FROM download_artifacts
                        WHERE entry_id = ?1
                          AND NOT EXISTS (
                              SELECT 1 FROM download_pages
                              WHERE entry_id = ?1 AND state = 'present'
                          )
                    "#,
                    [&descriptor.entry_id],
                )
                .map_err(map_sqlite_error)?;
        }
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(Some(projection))
    }

    fn pipeline_resume_interrupted(&self) -> Result<Vec<DownloadJobDescriptor>, RepositoryError> {
        let entry_ids = {
            let connection = self.connection()?;
            let mut statement = connection
                .prepare(
                    r#"
                        SELECT d.entry_id
                        FROM download_entries d
                        JOIN download_artifacts a ON a.entry_id = d.entry_id
                        WHERE d.state = 'interrupted'
                        ORDER BY d.created_at ASC, d.entry_id ASC
                    "#,
                )
                .map_err(map_sqlite_error)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(map_sqlite_error)?;
            let mut entry_ids = Vec::new();
            for row in rows {
                entry_ids.push(
                    DownloadEntryId::new(row.map_err(map_sqlite_error)?)
                        .map_err(domain_corruption)?,
                );
            }
            entry_ids
        };
        if entry_ids.is_empty() {
            return Ok(Vec::new());
        }
        let job_refs = match <Self as DownloadRepository>::download_retry(self, &entry_ids)? {
            DownloadMutationOutcome::Applied(job_refs) => job_refs,
            DownloadMutationOutcome::EntryNotFound(entry_id) => {
                return Err(RepositoryError::Corrupt(format!(
                    "interrupted download entry {entry_id} disappeared during resume"
                )))
            }
            DownloadMutationOutcome::InvalidState { entry_id, state } => {
                return Err(RepositoryError::Other(format!(
                    "interrupted download entry {entry_id} changed to {state} during resume"
                )))
            }
        };
        let connection = self.connection()?;
        let mut descriptors = Vec::new();
        for job_ref in job_refs.into_iter().filter(|job_ref| !job_ref.reused) {
            let descriptor = connection
                .query_row(
                    r#"
                        SELECT job_id, entry_id, gallery_id, attempt
                        FROM download_jobs WHERE job_id = ?1
                    "#,
                    [&job_ref.job_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .map_err(map_sqlite_error)?;
            descriptors.push(DownloadJobDescriptor {
                job_id: descriptor.0,
                entry_id: descriptor.1,
                gallery_id: GalleryId::new(descriptor.2).map_err(domain_corruption)?,
                worker_attempt: stored_u64(descriptor.3, "download attempt")?,
            });
        }
        Ok(descriptors)
    }

    fn pipeline_descriptors_for_jobs(
        &self,
        jobs: &[JobRef],
    ) -> Result<Vec<DownloadJobDescriptor>, RepositoryError> {
        let connection = self.connection()?;
        let mut descriptors = Vec::new();
        for job in jobs.iter().filter(|job| !job.reused) {
            let stored = connection
                .query_row(
                    r#"
                        SELECT job_id, entry_id, gallery_id, attempt
                        FROM download_jobs WHERE job_id = ?1
                    "#,
                    [&job.job_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(map_sqlite_error)?
                .ok_or_else(|| {
                    RepositoryError::Corrupt(format!(
                        "download job {:?} disappeared before launch",
                        job.job_id
                    ))
                })?;
            let attempt = stored_u64(stored.3, "download attempt")?;
            if attempt != job.worker_attempt {
                return Err(RepositoryError::Other(format!(
                    "download job {:?} changed attempt before launch",
                    job.job_id
                )));
            }
            descriptors.push(DownloadJobDescriptor {
                job_id: stored.0,
                entry_id: stored.1,
                gallery_id: GalleryId::new(stored.2).map_err(domain_corruption)?,
                worker_attempt: attempt,
            });
        }
        Ok(descriptors)
    }

    fn pipeline_mark_artifact_issue(
        &self,
        entry_id: &DownloadEntryId,
        code: &str,
        message: &str,
    ) -> Result<Option<DownloadJobProjection>, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let Some(target) = read_download_target(&transaction, entry_id)? else {
            return Err(RepositoryError::Corrupt(format!(
                "artifact references missing download entry {entry_id}"
            )));
        };
        transaction
            .execute(
                r#"
                    UPDATE download_artifacts
                    SET revision = revision + 1, state = 'missing_artifacts'
                    WHERE entry_id = ?1
                      AND state NOT IN ('quarantined', 'missing_artifacts')
                "#,
                [entry_id.as_str()],
            )
            .map_err(map_sqlite_error)?;
        if !target.state.allows_transition_to(JobState::Failed) {
            transaction.commit().map_err(map_sqlite_error)?;
            return Ok(None);
        }
        let descriptor = DownloadJobDescriptor {
            job_id: target.job_id,
            entry_id: target.entry_id.to_string(),
            gallery_id: GalleryId::new(target.gallery_id).map_err(domain_corruption)?,
            worker_attempt: stored_u64(target.attempt, "download attempt")?,
        };
        let pipeline_target = read_pipeline_target(&transaction, &descriptor)?;
        let active_attempt = pipeline_target.state.is_active();
        let projection = transition_pipeline_target(
            &transaction,
            pipeline_target,
            JobState::Failed,
            None,
            None,
            Some(code),
            Some(message),
            "Artifact integrity requires a safe retry or manual recovery",
        )?;
        transaction
            .execute(
                "UPDATE download_jobs SET last_error_retryable = 0 WHERE job_id = ?1",
                [&descriptor.job_id],
            )
            .map_err(map_sqlite_error)?;
        if active_attempt {
            transaction
                .execute(
                    r#"
                        UPDATE download_attempts
                        SET finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                            outcome_state = 'failed', error_code = ?1,
                            error_message = ?2, error_retryable = 0
                        WHERE job_id = ?3 AND attempt = ?4
                          AND outcome_state IS NULL
                    "#,
                    params![
                        code,
                        message,
                        descriptor.job_id,
                        to_sql_integer(descriptor.worker_attempt, "download attempt")?,
                    ],
                )
                .map_err(map_sqlite_error)?;
        }
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(Some(projection))
    }

    fn pipeline_artifact_bundle(
        &self,
        entry_id: &DownloadEntryId,
    ) -> Result<Option<ArtifactBundle>, RepositoryError> {
        <Self as ArtifactRepository>::artifact_bundle_get(self, entry_id)
    }

    fn pipeline_artifact_root(
        &self,
        entry_id: &DownloadEntryId,
    ) -> Result<PathBuf, RepositoryError> {
        let connection = self.connection()?;
        let stored: String = connection
            .query_row(
                "SELECT root_snapshot FROM download_artifacts WHERE entry_id = ?1",
                [entry_id.as_str()],
                |row| row.get(0),
            )
            .map_err(map_sqlite_error)?;
        if stored.trim().is_empty() {
            return Err(RepositoryError::Other(
                "artifact root snapshot is missing".into(),
            ));
        }
        Ok(PathBuf::from(stored))
    }

    fn pipeline_artifact_bundles(&self) -> Result<Vec<ArtifactBundle>, RepositoryError> {
        let entry_ids = {
            let connection = self.connection()?;
            let mut statement = connection
                .prepare("SELECT entry_id FROM download_artifacts ORDER BY entry_id ASC")
                .map_err(map_sqlite_error)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(map_sqlite_error)?;
            let mut entry_ids = Vec::new();
            for row in rows {
                entry_ids.push(
                    DownloadEntryId::new(row.map_err(map_sqlite_error)?)
                        .map_err(domain_corruption)?,
                );
            }
            entry_ids
        };
        let mut bundles = Vec::with_capacity(entry_ids.len());
        for entry_id in entry_ids {
            if let Some(bundle) =
                <Self as ArtifactRepository>::artifact_bundle_get(self, &entry_id)?
            {
                bundles.push(bundle);
            }
        }
        Ok(bundles)
    }

    fn pipeline_quarantine_begin(&self, saga: &QuarantineSaga) -> Result<(), RepositoryError> {
        if saga.state != QuarantineSagaState::PendingQuarantine {
            return Err(RepositoryError::Other(
                "quarantine saga must begin in pending_quarantine".into(),
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let target = read_download_target(&transaction, &saga.entry_id)?
            .ok_or_else(|| RepositoryError::Other("download entry no longer exists".into()))?;
        if target.state != JobState::Completed {
            return Err(RepositoryError::Other(format!(
                "download entry cannot be quarantined from {}",
                target.state
            )));
        }
        let artifact = transaction
            .query_row(
                "SELECT relative_directory, state FROM download_artifacts WHERE entry_id = ?1",
                [saga.entry_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(map_sqlite_error)?
            .ok_or_else(|| RepositoryError::Other("download artifact no longer exists".into()))?;
        if artifact.0 != saga.original_relative_path.as_str() || artifact.1 != "complete" {
            return Err(RepositoryError::Other(
                "download artifact is not a verified complete artifact".into(),
            ));
        }
        transaction
            .execute(
                r#"
                    INSERT INTO quarantine_records (
                        record_id, entry_id, original_relative_path,
                        quarantine_relative_path, reason, state,
                        original_entry_state, original_artifact_state,
                        created_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, 'pending_quarantine',
                        'completed', 'complete',
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                "#,
                params![
                    saga.record_id,
                    saga.entry_id.as_str(),
                    saga.original_relative_path.as_str(),
                    saga.quarantine_relative_path.as_str(),
                    saga.reason,
                ],
            )
            .map_err(map_sqlite_error)?;
        transaction.commit().map_err(map_sqlite_error)
    }

    fn pipeline_quarantine_complete(
        &self,
        record_id: &str,
    ) -> Result<DownloadJobProjection, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let entry_id = transaction
            .query_row(
                "SELECT entry_id FROM quarantine_records WHERE record_id = ?1 AND state = 'pending_quarantine'",
                [record_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(map_sqlite_error)?
            .ok_or_else(|| RepositoryError::Other("quarantine operation is no longer pending".into()))?;
        let entry_id = DownloadEntryId::new(entry_id).map_err(domain_corruption)?;
        let target = read_download_target(&transaction, &entry_id)?
            .ok_or_else(|| RepositoryError::Other("download entry no longer exists".into()))?;
        if target.state != JobState::Completed {
            return Err(RepositoryError::Other(
                "download entry changed while it was being quarantined".into(),
            ));
        }
        let descriptor = DownloadJobDescriptor {
            job_id: target.job_id.clone(),
            entry_id: target.entry_id.to_string(),
            gallery_id: GalleryId::new(target.gallery_id).map_err(domain_corruption)?,
            worker_attempt: stored_u64(target.attempt, "download attempt")?,
        };
        let pipeline_target = read_pipeline_target(&transaction, &descriptor)?;
        let artifact_changed = transaction
            .execute(
                "UPDATE download_artifacts SET revision = revision + 1, state = 'quarantined' WHERE entry_id = ?1 AND state = 'complete'",
                [entry_id.as_str()],
            )
            .map_err(map_sqlite_error)?;
        if artifact_changed != 1 {
            return Err(RepositoryError::Other(
                "artifact changed while it was being quarantined".into(),
            ));
        }
        transaction
            .execute(
                "UPDATE download_pages SET state = 'quarantined' WHERE entry_id = ?1 AND state = 'present'",
                [entry_id.as_str()],
            )
            .map_err(map_sqlite_error)?;
        let projection = transition_pipeline_target(
            &transaction,
            pipeline_target,
            JobState::Quarantined,
            None,
            None,
            None,
            None,
            "Artifact moved to recoverable quarantine",
        )?;
        transaction
            .execute(
                "UPDATE quarantine_records SET state = 'quarantined' WHERE record_id = ?1 AND state = 'pending_quarantine'",
                [record_id],
            )
            .map_err(map_sqlite_error)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(projection)
    }

    fn pipeline_restore_begin(
        &self,
        entry_id: &DownloadEntryId,
    ) -> Result<QuarantineSaga, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let target = read_download_target(&transaction, entry_id)?
            .ok_or_else(|| RepositoryError::Other("download entry no longer exists".into()))?;
        if target.state != JobState::Quarantined {
            return Err(RepositoryError::Other(format!(
                "download entry cannot be restored from {}",
                target.state
            )));
        }
        let stored = transaction
            .query_row(
                r#"
                    SELECT record_id, original_relative_path,
                           quarantine_relative_path, reason
                    FROM quarantine_records
                    WHERE entry_id = ?1 AND state = 'quarantined'
                    ORDER BY created_at DESC, record_id DESC
                    LIMIT 1
                "#,
                [entry_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(map_sqlite_error)?
            .ok_or_else(|| {
                RepositoryError::Corrupt("quarantined entry has no active record".into())
            })?;
        let changed = transaction
            .execute(
                "UPDATE quarantine_records SET state = 'pending_restore' WHERE record_id = ?1 AND state = 'quarantined'",
                [&stored.0],
            )
            .map_err(map_sqlite_error)?;
        if changed != 1 {
            return Err(RepositoryError::Other(
                "quarantine record changed before restore".into(),
            ));
        }
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(QuarantineSaga {
            record_id: stored.0,
            entry_id: entry_id.clone(),
            original_relative_path: ArtifactRelativePath::new(stored.1)
                .map_err(domain_corruption)?,
            quarantine_relative_path: ArtifactRelativePath::new(stored.2)
                .map_err(domain_corruption)?,
            reason: stored.3,
            state: QuarantineSagaState::PendingRestore,
        })
    }

    fn pipeline_restore_complete(
        &self,
        record_id: &str,
    ) -> Result<DownloadJobProjection, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let entry_id = transaction
            .query_row(
                "SELECT entry_id FROM quarantine_records WHERE record_id = ?1 AND state = 'pending_restore'",
                [record_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(map_sqlite_error)?
            .ok_or_else(|| RepositoryError::Other("restore operation is no longer pending".into()))?;
        let entry_id = DownloadEntryId::new(entry_id).map_err(domain_corruption)?;
        let target = read_download_target(&transaction, &entry_id)?
            .ok_or_else(|| RepositoryError::Other("download entry no longer exists".into()))?;
        if target.state != JobState::Quarantined {
            return Err(RepositoryError::Other(
                "download entry changed while it was being restored".into(),
            ));
        }
        let descriptor = DownloadJobDescriptor {
            job_id: target.job_id.clone(),
            entry_id: target.entry_id.to_string(),
            gallery_id: GalleryId::new(target.gallery_id).map_err(domain_corruption)?,
            worker_attempt: stored_u64(target.attempt, "download attempt")?,
        };
        let pipeline_target = read_pipeline_target(&transaction, &descriptor)?;
        let artifact_changed = transaction
            .execute(
                "UPDATE download_artifacts SET revision = revision + 1, state = 'complete' WHERE entry_id = ?1 AND state = 'quarantined'",
                [entry_id.as_str()],
            )
            .map_err(map_sqlite_error)?;
        if artifact_changed != 1 {
            return Err(RepositoryError::Other(
                "artifact changed while it was being restored".into(),
            ));
        }
        transaction
            .execute(
                "UPDATE download_pages SET state = 'present' WHERE entry_id = ?1 AND state = 'quarantined'",
                [entry_id.as_str()],
            )
            .map_err(map_sqlite_error)?;
        let projection = transition_pipeline_target(
            &transaction,
            pipeline_target,
            JobState::Completed,
            None,
            None,
            None,
            None,
            "Artifact restored from quarantine",
        )?;
        transaction
            .execute(
                r#"
                    UPDATE quarantine_records
                    SET state = 'restored',
                        restored_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    WHERE record_id = ?1 AND state = 'pending_restore'
                "#,
                [record_id],
            )
            .map_err(map_sqlite_error)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(projection)
    }

    fn pipeline_pending_quarantine_sagas(&self) -> Result<Vec<QuarantineSaga>, RepositoryError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                r#"
                    SELECT record_id, entry_id, original_relative_path,
                           quarantine_relative_path, reason, state
                    FROM quarantine_records
                    WHERE state IN ('pending_quarantine', 'pending_restore')
                    ORDER BY created_at ASC, record_id ASC
                "#,
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(map_sqlite_error)?;
        let mut sagas = Vec::new();
        for row in rows {
            let row = row.map_err(map_sqlite_error)?;
            let state = match row.5.as_str() {
                "pending_quarantine" => QuarantineSagaState::PendingQuarantine,
                "pending_restore" => QuarantineSagaState::PendingRestore,
                _ => {
                    return Err(RepositoryError::Corrupt(
                        "pending quarantine query returned an invalid state".into(),
                    ))
                }
            };
            sagas.push(QuarantineSaga {
                record_id: row.0,
                entry_id: DownloadEntryId::new(row.1).map_err(domain_corruption)?,
                original_relative_path: ArtifactRelativePath::new(row.2)
                    .map_err(domain_corruption)?,
                quarantine_relative_path: ArtifactRelativePath::new(row.3)
                    .map_err(domain_corruption)?,
                reason: row.4,
                state,
            });
        }
        Ok(sagas)
    }
}

impl DownloadOverlapRepository for SqliteRepository {
    fn overlap_candidate_identities(
        &self,
        incoming_entry_id: &DownloadEntryId,
    ) -> Result<Vec<DownloadOverlapCandidateIdentity>, RepositoryError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                r#"
                    SELECT entry.entry_id, entry.gallery_id
                    FROM download_entries entry
                    JOIN download_artifacts artifact
                      ON artifact.entry_id = entry.entry_id
                     AND artifact.gallery_id = entry.gallery_id
                    WHERE entry.entry_id != ?1
                      AND (
                        (
                          entry.state = 'completed'
                          AND artifact.state = 'complete'
                          AND artifact.manifest_relative_path IS NOT NULL
                          AND artifact.manifest_schema_version IS NOT NULL
                          AND artifact.writer_version IS NOT NULL
                          AND artifact.completed_at IS NOT NULL
                        )
                        OR (
                          entry.state = 'review_required'
                          AND entry.review_kind = 'gallery_duplicate'
                          AND artifact.state = 'incomplete'
                        )
                      )
                      AND (
                        SELECT COUNT(*) FROM download_pages page
                        WHERE page.entry_id = artifact.entry_id
                      ) = artifact.expected_page_count
                      AND NOT EXISTS (
                        SELECT 1 FROM download_pages page
                        WHERE page.entry_id = artifact.entry_id
                          AND (
                            page.excluded != 0
                            OR page.state != 'present'
                            OR page.byte_length IS NULL
                            OR page.sha256 IS NULL
                            OR page.storage_format IS NULL
                            OR page.source_revision IS NULL
                            OR page.verified_at IS NULL
                          )
                      )
                    ORDER BY entry.entry_id ASC
                "#,
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([incoming_entry_id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(map_sqlite_error)?;
        let mut stored = Vec::new();
        for row in rows {
            stored.push(row.map_err(map_sqlite_error)?);
        }
        drop(statement);
        let mut result = Vec::with_capacity(stored.len());
        for (entry_id, gallery_id) in stored {
            let gallery_id = GalleryId::new(gallery_id).map_err(domain_corruption)?;
            result.push(DownloadOverlapCandidateIdentity {
                entry_id: DownloadEntryId::new(entry_id).map_err(domain_corruption)?,
                artists: read_owned_gallery_artists(&connection, gallery_id)?,
            });
        }
        Ok(result)
    }

    fn overlap_page_hash_get(
        &self,
        entry_id: &str,
        source_page_number: SourcePageNumber,
        profile_version: u32,
        artifact_sha256: &str,
    ) -> Result<Option<DuplicatePageHash>, RepositoryError> {
        <Self as DuplicateRepository>::duplicate_page_hash_get(
            self,
            entry_id,
            source_page_number,
            profile_version,
            artifact_sha256,
        )
    }

    fn overlap_page_hash_upsert(&self, hash: &DuplicatePageHash) -> Result<(), RepositoryError> {
        <Self as DuplicateRepository>::duplicate_page_hash_upsert(self, hash)
    }

    fn overlap_pair_policy_exists(
        &self,
        incoming_fingerprint: &str,
        existing_fingerprint: &str,
        profile_version: u32,
        policy_version: u32,
    ) -> Result<bool, RepositoryError> {
        let (left, right) =
            canonical_overlap_fingerprints(incoming_fingerprint, existing_fingerprint)?;
        let connection = self.connection()?;
        connection
            .query_row(
                r#"
                    SELECT EXISTS (
                        SELECT 1 FROM download_overlap_pair_policies
                        WHERE left_fingerprint = ?1 AND right_fingerprint = ?2
                          AND profile_version = ?3 AND policy_version = ?4
                    )
                "#,
                params![
                    left,
                    right,
                    i64::from(profile_version),
                    i64::from(policy_version)
                ],
                |row| row.get(0),
            )
            .map_err(map_sqlite_error)
    }

    fn overlap_review_pause(
        &self,
        descriptor: &DownloadJobDescriptor,
        draft: &DownloadOverlapReviewDraft,
    ) -> Result<DownloadJobProjection, RepositoryError> {
        if draft.entry_id.as_str() != descriptor.entry_id
            || draft.incoming.entry_id != descriptor.entry_id
            || draft.incoming.gallery_id != descriptor.gallery_id
            || draft.candidates.is_empty()
            || draft.incoming_fingerprint.len() != 64
        {
            return Err(RepositoryError::Other(
                "download overlap review draft does not match its pipeline target".into(),
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let target = read_pipeline_target(&transaction, descriptor)?;
        if target.state != JobState::Hashing {
            return Err(invalid_pipeline_state(&target, "pause for overlap review"));
        }
        transaction
            .execute(
                r#"
                    INSERT INTO download_overlap_reviews (
                        review_id, entry_id, incoming_gallery_id, revision, state,
                        profile_version, policy_version, incoming_fingerprint,
                        created_at, updated_at
                    ) VALUES (
                        ?1, ?2, ?3, 0, 'pending', ?4, ?5, ?6,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                "#,
                params![
                    draft.review_id,
                    draft.entry_id.as_str(),
                    draft.incoming.gallery_id.get(),
                    i64::from(draft.profile_version),
                    i64::from(draft.policy_version),
                    draft.incoming_fingerprint,
                ],
            )
            .map_err(map_sqlite_error)?;
        for candidate in &draft.candidates {
            if candidate.rank == 0
                || candidate.existing_fingerprint.len() != 64
                || candidate.page_pairs.is_empty()
            {
                return Err(RepositoryError::Other(
                    "download overlap candidate evidence is incomplete".into(),
                ));
            }
            transaction
                .execute(
                    r#"
                        INSERT INTO download_overlap_candidates (
                            candidate_id, review_id, existing_entry_id,
                            existing_gallery_id, existing_fingerprint, relation,
                            confidence, matched_pages, exact_pages, visual_pages,
                            existing_coverage, incoming_coverage,
                            existing_unique_pages, incoming_unique_pages,
                            longest_aligned_run, rank, decision
                        ) VALUES (
                            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                            ?11, ?12, ?13, ?14, ?15, ?16, NULL
                        )
                    "#,
                    params![
                        candidate.candidate_id,
                        draft.review_id,
                        candidate.existing.entry_id,
                        candidate.existing.gallery_id.get(),
                        candidate.existing_fingerprint,
                        candidate.relation.as_str(),
                        candidate.confidence,
                        i64::from(candidate.matched_pages),
                        i64::from(candidate.exact_pages),
                        i64::from(candidate.visual_pages),
                        candidate.existing_coverage,
                        candidate.incoming_coverage,
                        i64::from(candidate.existing_unique_pages),
                        i64::from(candidate.incoming_unique_pages),
                        i64::from(candidate.longest_aligned_run),
                        i64::from(candidate.rank),
                    ],
                )
                .map_err(map_sqlite_error)?;
            for (index, pair) in candidate
                .page_pairs
                .iter()
                .take(DOWNLOAD_OVERLAP_MAX_STORED_PAGE_PAIRS)
                .enumerate()
            {
                transaction
                    .execute(
                        r#"
                            INSERT INTO download_overlap_page_pairs (
                                candidate_id, pair_index, incoming_source_page,
                                existing_source_page, exact_sha256,
                                d_hash_distance, p_hash_distance, detail_hash_distance,
                                edge_similarity, visual_similarity, low_information
                            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                        "#,
                        params![
                            candidate.candidate_id,
                            to_sql_integer(index as u64, "overlap pair index")?,
                            i64::from(pair.incoming_source_page),
                            i64::from(pair.existing_source_page),
                            i64::from(u8::from(pair.exact_sha256)),
                            i64::from(pair.d_hash_distance),
                            i64::from(pair.p_hash_distance),
                            i64::from(pair.detail_hash_distance),
                            pair.edge_similarity,
                            pair.visual_similarity,
                            i64::from(u8::from(pair.low_information)),
                        ],
                    )
                    .map_err(map_sqlite_error)?;
            }
        }
        let mut projection = transition_pipeline_target(
            &transaction,
            target,
            JobState::ReviewRequired,
            None,
            None,
            None,
            None,
            "Download paused for edition overlap review",
        )?;
        let review_target_changed = transaction
            .execute(
                r#"
                    UPDATE download_entries
                    SET review_kind = 'gallery_duplicate', review_id = ?1
                    WHERE entry_id = ?2 AND state = 'review_required'
                "#,
                params![draft.review_id, draft.entry_id.as_str()],
            )
            .map_err(map_sqlite_error)?;
        if review_target_changed != 1 {
            return Err(RepositoryError::Other(
                "download overlap review target changed concurrently".into(),
            ));
        }
        projection.download.review_kind = Some(DownloadReviewKind::GalleryDuplicate);
        projection.download.review_id = Some(draft.review_id.clone());
        transaction
            .execute(
                r#"
                    UPDATE download_attempts
                    SET finished_at = COALESCE(
                            finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        ),
                        outcome_state = 'review_required',
                        error_code = NULL, error_message = NULL
                    WHERE job_id = ?1 AND attempt = ?2
                "#,
                params![
                    descriptor.job_id,
                    to_sql_integer(descriptor.worker_attempt, "download attempt")?,
                ],
            )
            .map_err(map_sqlite_error)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(projection)
    }

    fn overlap_review_get(
        &self,
        review_id: &str,
    ) -> Result<Option<DownloadOverlapReview>, RepositoryError> {
        let connection = self.connection()?;
        read_download_overlap_review(&connection, review_id)
    }

    fn overlap_decision_apply(
        &self,
        request: &DownloadOverlapDecisionRequest,
        verified_incoming_fingerprint: &str,
        verified_existing_fingerprints: &[(String, String)],
    ) -> Result<DownloadOverlapDecisionApplyOutcome, RepositoryError> {
        let verified_existing = verified_existing_fingerprints
            .iter()
            .cloned()
            .collect::<BTreeMap<_, _>>();
        let (projection, removed_existing_projection, resume, resumed, cancelled) = {
            let mut connection = self.connection()?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(map_sqlite_error)?;
            let stored = transaction
                .query_row(
                    r#"
                        SELECT entry_id, revision, state, incoming_fingerprint,
                               profile_version, policy_version
                        FROM download_overlap_reviews WHERE review_id = ?1
                    "#,
                    [request.review_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, i64>(5)?,
                        ))
                    },
                )
                .optional()
                .map_err(map_sqlite_error)?;
            let Some((entry_id, revision, state, incoming_fingerprint, profile, policy)) = stored
            else {
                return Ok(DownloadOverlapDecisionApplyOutcome::ReviewNotFound);
            };
            let actual_revision = stored_u64(revision, "download overlap review revision")?;
            if actual_revision != request.expected_revision {
                return Ok(DownloadOverlapDecisionApplyOutcome::RevisionConflict {
                    actual_revision,
                });
            }
            if state != "pending" {
                return Ok(DownloadOverlapDecisionApplyOutcome::InvalidCandidate);
            }
            if incoming_fingerprint != verified_incoming_fingerprint {
                return Err(RepositoryError::Other(
                    "download overlap incoming fingerprint changed before decision".into(),
                ));
            }
            if request.action != DownloadOverlapDecisionAction::RemoveIncoming {
                transaction
                    .execute(
                        r#"
                            UPDATE download_overlap_candidates
                            SET decision = 'existing_removed'
                            WHERE review_id = ?1
                              AND decision IS NULL
                              AND candidate_id != ?2
                              AND EXISTS (
                                  SELECT 1
                                  FROM download_entries existing_entry
                                  WHERE existing_entry.entry_id = download_overlap_candidates.existing_entry_id
                                    AND existing_entry.state = 'quarantined'
                              )
                        "#,
                        params![request.review_id, request.candidate_id],
                    )
                    .map_err(map_sqlite_error)?;
            }
            let pending = read_pending_overlap_candidates(&transaction, &request.review_id)?;
            if pending.is_empty()
                || pending.iter().any(|(candidate_id, fingerprint)| {
                    verified_existing.get(candidate_id) != Some(fingerprint)
                })
            {
                return Err(RepositoryError::Other(
                    "download overlap candidate fingerprint changed before decision".into(),
                ));
            }
            let profile = stored_u32(profile, "download overlap profile version")?;
            let policy = stored_u32(policy, "download overlap policy version")?;
            let decision_id = format!("overlap-decision-{}", Uuid::new_v4());
            let mut projection = None;
            let mut removed_existing_projection = None;
            let mut resume = None;
            let mut resumed = false;
            let mut cancelled = false;

            let selected = if request.action == DownloadOverlapDecisionAction::RemoveIncoming {
                None
            } else {
                let selected = match (request.candidate_id.as_deref(), pending.as_slice()) {
                    (Some(candidate_id), _)
                        if pending.iter().any(|item| item.0 == candidate_id) =>
                    {
                        pending.iter().find(|item| item.0 == candidate_id).cloned()
                    }
                    (None, [candidate]) => Some(candidate.clone()),
                    _ => None,
                };
                let Some(selected) = selected else {
                    return Ok(DownloadOverlapDecisionApplyOutcome::InvalidCandidate);
                };
                Some(selected)
            };

            match request.action {
                DownloadOverlapDecisionAction::KeepBothContinue => {
                    let (candidate_id, existing_fingerprint) = selected
                        .as_ref()
                        .expect("candidate-scoped action was validated");
                    insert_overlap_pair_policy(
                        &transaction,
                        verified_incoming_fingerprint,
                        existing_fingerprint,
                        profile,
                        policy,
                        DownloadOverlapPairDecision::KeepBoth,
                    )?;
                    transaction
                        .execute(
                            "UPDATE download_overlap_candidates SET decision='keep_both' WHERE candidate_id=?1 AND decision IS NULL",
                            [candidate_id],
                        )
                        .map_err(map_sqlite_error)?;
                }
                DownloadOverlapDecisionAction::FalsePositiveContinue => {
                    let (candidate_id, existing_fingerprint) = selected
                        .as_ref()
                        .expect("candidate-scoped action was validated");
                    insert_overlap_pair_policy(
                        &transaction,
                        verified_incoming_fingerprint,
                        existing_fingerprint,
                        profile,
                        policy,
                        DownloadOverlapPairDecision::FalsePositive,
                    )?;
                    transaction
                        .execute(
                            "UPDATE download_overlap_candidates SET decision='false_positive' WHERE candidate_id=?1 AND decision IS NULL",
                            [candidate_id],
                        )
                        .map_err(map_sqlite_error)?;
                }
                DownloadOverlapDecisionAction::RemoveExistingContinue => {
                    let (candidate_id, _) = selected
                        .as_ref()
                        .expect("candidate-scoped action was validated");
                    let existing_entry_id = transaction
                        .query_row(
                            "SELECT existing_entry_id FROM download_overlap_candidates WHERE candidate_id=?1 AND review_id=?2 AND decision IS NULL",
                            params![candidate_id, request.review_id],
                            |row| row.get::<_, String>(0),
                        )
                        .optional()
                        .map_err(map_sqlite_error)?;
                    let Some(existing_entry_id) = existing_entry_id else {
                        return Ok(DownloadOverlapDecisionApplyOutcome::InvalidCandidate);
                    };
                    let existing_entry_id =
                        DownloadEntryId::new(existing_entry_id).map_err(domain_corruption)?;
                    let existing_target = read_download_target(&transaction, &existing_entry_id)?
                        .ok_or_else(|| {
                        RepositoryError::Corrupt(
                            "download overlap candidate target disappeared".into(),
                        )
                    })?;
                    match existing_target.state {
                        JobState::Quarantined => {
                            // The supervisor moved a verified complete artifact before
                            // entering this transaction. Compensation restores it if the
                            // current review CAS below cannot be committed.
                        }
                        JobState::ReviewRequired => {
                            let Some(cancelled_projection) = cancel_chained_overlap_target(
                                &transaction,
                                existing_target,
                                &request.review_id,
                            )?
                            else {
                                return Ok(DownloadOverlapDecisionApplyOutcome::InvalidCandidate);
                            };
                            removed_existing_projection = Some(cancelled_projection);
                        }
                        JobState::Failed | JobState::Interrupted => {
                            let Some(cancelled_projection) =
                                cancel_failed_overlap_target(&transaction, existing_target)?
                            else {
                                return Ok(DownloadOverlapDecisionApplyOutcome::InvalidCandidate);
                            };
                            removed_existing_projection = Some(cancelled_projection);
                        }
                        JobState::Cancelled => {
                            if !incomplete_overlap_artifact_exists(
                                &transaction,
                                &existing_target.entry_id,
                            )? {
                                return Ok(DownloadOverlapDecisionApplyOutcome::InvalidCandidate);
                            }
                        }
                        _ => {
                            return Ok(DownloadOverlapDecisionApplyOutcome::InvalidCandidate);
                        }
                    }
                    transaction
                        .execute(
                            "UPDATE download_overlap_candidates SET decision='existing_removed' WHERE candidate_id=?1 AND decision IS NULL",
                            [candidate_id],
                        )
                        .map_err(map_sqlite_error)?;
                }
                DownloadOverlapDecisionAction::RemoveIncoming => {
                    finish_overlap_review(&transaction, &request.review_id, "cancelled")?;
                    let entry_id =
                        DownloadEntryId::new(entry_id.clone()).map_err(domain_corruption)?;
                    let target =
                        read_download_target(&transaction, &entry_id)?.ok_or_else(|| {
                            RepositoryError::Corrupt("overlap download target disappeared".into())
                        })?;
                    let descriptor = DownloadJobDescriptor {
                        job_id: target.job_id.clone(),
                        entry_id: target.entry_id.to_string(),
                        gallery_id: GalleryId::new(target.gallery_id).map_err(domain_corruption)?,
                        worker_attempt: stored_u64(target.attempt, "download attempt")?,
                    };
                    let pipeline_target = read_pipeline_target(&transaction, &descriptor)?;
                    projection = Some(transition_pipeline_target(
                        &transaction,
                        pipeline_target,
                        JobState::Cancelled,
                        None,
                        None,
                        None,
                        None,
                        "Incoming download was cancelled after overlap review",
                    )?);
                    transaction
                        .execute(
                            "UPDATE download_attempts SET finished_at=COALESCE(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), outcome_state='cancelled' WHERE job_id=?1 AND attempt=?2",
                            params![descriptor.job_id, to_sql_integer(descriptor.worker_attempt, "download attempt")?],
                        )
                        .map_err(map_sqlite_error)?;
                    cancelled = true;
                }
            }
            if request.action != DownloadOverlapDecisionAction::RemoveIncoming {
                let remaining: i64 = transaction
                    .query_row(
                        "SELECT COUNT(*) FROM download_overlap_candidates WHERE review_id=?1 AND decision IS NULL",
                        [request.review_id.as_str()],
                        |row| row.get(0),
                    )
                    .map_err(map_sqlite_error)?;
                if remaining == 0 {
                    finish_overlap_review(&transaction, &request.review_id, "resolved")?;
                    let (next_projection, next_descriptor) =
                        requeue_overlap_target(&transaction, &entry_id)?;
                    projection = Some(next_projection);
                    resume = Some(next_descriptor);
                    resumed = true;
                } else {
                    bump_pending_overlap_review(&transaction, &request.review_id)?;
                }
            }
            transaction
                .execute(
                    r#"
                        INSERT INTO download_overlap_decisions (
                            decision_id, review_id, review_revision,
                            candidate_id, action, created_at
                        ) VALUES (
                            ?1, ?2, ?3, ?4, ?5,
                            strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        )
                    "#,
                    params![
                        decision_id,
                        request.review_id,
                        to_sql_integer(request.expected_revision, "overlap review revision")?,
                        request.candidate_id,
                        request.action.as_str(),
                    ],
                )
                .map_err(map_sqlite_error)?;
            transaction.commit().map_err(map_sqlite_error)?;
            (
                projection,
                removed_existing_projection,
                resume,
                resumed,
                cancelled,
            )
        };
        let review = self
            .overlap_review_get(&request.review_id)?
            .ok_or_else(|| RepositoryError::Corrupt("applied overlap review disappeared".into()))?;
        Ok(DownloadOverlapDecisionApplyOutcome::Applied(Box::new(
            DownloadOverlapDecisionApplied {
                result: DownloadOverlapDecisionResult {
                    review,
                    resumed,
                    cancelled,
                },
                projection,
                removed_existing_projection,
                resume,
            },
        )))
    }

    fn overlap_review_requeue_stale(
        &self,
        review_id: &str,
        expected_revision: u64,
    ) -> Result<DownloadOverlapDecisionApplyOutcome, RepositoryError> {
        let (projection, resume) = {
            let mut connection = self.connection()?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(map_sqlite_error)?;
            let stored = transaction
                .query_row(
                    "SELECT entry_id, revision, state FROM download_overlap_reviews WHERE review_id=?1",
                    [review_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
                )
                .optional()
                .map_err(map_sqlite_error)?;
            let Some((entry_id, revision, state)) = stored else {
                return Ok(DownloadOverlapDecisionApplyOutcome::ReviewNotFound);
            };
            let actual_revision = stored_u64(revision, "download overlap review revision")?;
            if actual_revision != expected_revision {
                return Ok(DownloadOverlapDecisionApplyOutcome::RevisionConflict {
                    actual_revision,
                });
            }
            if state != "pending" {
                return Ok(DownloadOverlapDecisionApplyOutcome::InvalidCandidate);
            }
            finish_overlap_review(&transaction, review_id, "stale")?;
            let (projection, resume) = requeue_overlap_target(&transaction, &entry_id)?;
            transaction.commit().map_err(map_sqlite_error)?;
            (projection, resume)
        };
        let review = self
            .overlap_review_get(review_id)?
            .ok_or_else(|| RepositoryError::Corrupt("stale overlap review disappeared".into()))?;
        Ok(DownloadOverlapDecisionApplyOutcome::Applied(Box::new(
            DownloadOverlapDecisionApplied {
                result: DownloadOverlapDecisionResult {
                    review,
                    resumed: true,
                    cancelled: false,
                },
                projection: Some(projection),
                removed_existing_projection: None,
                resume: Some(resume),
            },
        )))
    }
}

impl DuplicateRepository for SqliteRepository {
    fn duplicate_artifact_bundles(&self) -> Result<Vec<ArtifactBundle>, RepositoryError> {
        let bundles = <Self as DownloadPipelineRepository>::pipeline_artifact_bundles(self)?;
        Ok(bundles
            .into_iter()
            .filter(duplicate_bundle_is_verified)
            .collect())
    }

    fn duplicate_page_hash_get(
        &self,
        entry_id: &str,
        source_page_number: SourcePageNumber,
        profile_version: u32,
        artifact_sha256: &str,
    ) -> Result<Option<DuplicatePageHash>, RepositoryError> {
        let connection = self.connection()?;
        connection
            .query_row(
                r#"
                    SELECT entry_id, gallery_id, source_page_number, profile_version,
                           artifact_sha256, coarse_d_hash_hex, detail_d_hash_hex,
                           p_hash_hex, mean_luma, std_dev, non_uniform_ratio,
                           edge_density, width, height, low_information
                    FROM duplicate_page_hashes
                    WHERE entry_id = ?1 AND source_page_number = ?2
                      AND profile_version = ?3 AND artifact_sha256 = ?4
                "#,
                params![
                    entry_id,
                    i64::from(source_page_number.get()),
                    i64::from(profile_version),
                    artifact_sha256,
                ],
                stored_duplicate_page_hash,
            )
            .optional()
            .map_err(map_sqlite_error)?
            .map(StoredDuplicatePageHash::try_into_domain)
            .transpose()
    }

    fn duplicate_page_hash_upsert(&self, hash: &DuplicatePageHash) -> Result<(), RepositoryError> {
        validate_detail_hash(&hash.detail_d_hash_hex)?;
        let connection = self.connection()?;
        connection
            .execute(
                r#"
                    INSERT INTO duplicate_page_hashes (
                        entry_id, gallery_id, source_page_number, profile_version,
                        artifact_sha256, coarse_d_hash_hex, detail_d_hash_hex,
                        p_hash_hex, mean_luma, std_dev, non_uniform_ratio,
                        edge_density, width, height, low_information, computed_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                        ?12, ?13, ?14, ?15,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                    ON CONFLICT(entry_id, source_page_number, profile_version) DO UPDATE SET
                        gallery_id = excluded.gallery_id,
                        artifact_sha256 = excluded.artifact_sha256,
                        coarse_d_hash_hex = excluded.coarse_d_hash_hex,
                        detail_d_hash_hex = excluded.detail_d_hash_hex,
                        p_hash_hex = excluded.p_hash_hex,
                        mean_luma = excluded.mean_luma,
                        std_dev = excluded.std_dev,
                        non_uniform_ratio = excluded.non_uniform_ratio,
                        edge_density = excluded.edge_density,
                        width = excluded.width,
                        height = excluded.height,
                        low_information = excluded.low_information,
                        computed_at = excluded.computed_at
                "#,
                params![
                    hash.entry_id,
                    hash.gallery_id.get(),
                    i64::from(hash.source_page_number.get()),
                    i64::from(hash.profile_version),
                    hash.artifact_sha256.as_str(),
                    format!("{:016x}", hash.coarse_d_hash),
                    hash.detail_d_hash_hex.to_ascii_lowercase(),
                    format!("{:016x}", hash.p_hash),
                    hash.mean_luma,
                    hash.std_dev,
                    hash.non_uniform_ratio,
                    hash.edge_density,
                    i64::from(hash.width),
                    i64::from(hash.height),
                    hash.low_information,
                ],
            )
            .map_err(map_sqlite_error)?;
        Ok(())
    }

    fn duplicate_recover_interrupted(&self) -> Result<usize, RepositoryError> {
        let connection = self.connection()?;
        connection
            .execute(
                r#"
                    UPDATE duplicate_scan_runs
                    SET revision = revision + 1,
                        state = 'failed',
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        error_code = 'DUPLICATE_SCAN_INTERRUPTED',
                        error_message = 'The previous duplicate scan stopped before completion'
                    WHERE state = 'running'
                "#,
                [],
            )
            .map_err(map_sqlite_error)
    }

    fn duplicate_scan_start(
        &self,
        profile_version: u32,
        total_artifacts: u32,
        total_pairs: u64,
    ) -> Result<DuplicateScanRun, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        if let Some(run) = read_running_duplicate_scan(&transaction)? {
            transaction.commit().map_err(map_sqlite_error)?;
            return Ok(run);
        }
        let run_id = format!("duplicate-scan-{}", Uuid::new_v4());
        transaction
            .execute(
                r#"
                    INSERT INTO duplicate_scan_runs (
                        run_id, revision, state, profile_version,
                        total_artifacts, hashed_artifacts, total_pairs,
                        compared_pairs, candidates_found, started_at, updated_at
                    ) VALUES (
                        ?1, 0, 'running', ?2, ?3, 0, ?4, 0, 0,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                "#,
                params![
                    run_id,
                    i64::from(profile_version),
                    i64::from(total_artifacts),
                    to_sql_integer(total_pairs, "duplicate total pair count")?,
                ],
            )
            .map_err(map_sqlite_error)?;
        let run = read_duplicate_scan(&transaction, &run_id)?.ok_or_else(|| {
            RepositoryError::Corrupt("duplicate scan start did not produce a run".into())
        })?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(run)
    }

    fn duplicate_scan_progress(
        &self,
        run_id: &str,
        hashed_artifacts: u32,
        compared_pairs: u64,
    ) -> Result<Option<DuplicateScanRun>, RepositoryError> {
        let connection = self.connection()?;
        connection
            .execute(
                r#"
                    UPDATE duplicate_scan_runs
                    SET revision = revision + 1,
                        hashed_artifacts = max(hashed_artifacts, ?1),
                        compared_pairs = max(compared_pairs, ?2),
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    WHERE run_id = ?3 AND state = 'running'
                      AND ?1 BETWEEN 0 AND total_artifacts
                      AND ?2 BETWEEN 0 AND total_pairs
                "#,
                params![
                    i64::from(hashed_artifacts),
                    to_sql_integer(compared_pairs, "duplicate compared pair count")?,
                    run_id,
                ],
            )
            .map_err(map_sqlite_error)?;
        read_duplicate_scan(&connection, run_id)
    }

    fn duplicate_candidate_replace(
        &self,
        record: &DuplicateCandidateRecord,
    ) -> Result<Option<DuplicateScanRun>, RepositoryError> {
        validate_duplicate_candidate_record(record)?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let Some(profile_version) = running_duplicate_profile(&transaction, &record.run_id)? else {
            transaction.commit().map_err(map_sqlite_error)?;
            return Ok(None);
        };
        let parent_id = record.candidate.parent.gallery_id.get();
        let candidate_id = record.candidate.candidate.gallery_id.get();
        let rejected: bool = transaction
            .query_row(
                r#"
                    SELECT EXISTS (
                        SELECT 1 FROM duplicate_hidden_galleries
                        WHERE gallery_id IN (?1, ?2)
                        UNION ALL
                        SELECT 1 FROM duplicate_pair_exclusions
                        WHERE parent_gallery_id = ?1 AND candidate_gallery_id = ?2
                        UNION ALL
                        SELECT 1 FROM duplicate_candidates
                        WHERE profile_version = ?3
                          AND parent_gallery_id = ?1 AND candidate_gallery_id = ?2
                          AND resolved = 1
                    )
                "#,
                params![parent_id, candidate_id, i64::from(profile_version)],
                |row| row.get(0),
            )
            .map_err(map_sqlite_error)?;
        if rejected {
            transaction.commit().map_err(map_sqlite_error)?;
            return Ok(None);
        }

        let existing = transaction
            .query_row(
                r#"
                    SELECT candidate_id, revision, last_seen_run_id
                    FROM duplicate_candidates
                    WHERE profile_version = ?1
                      AND parent_gallery_id = ?2 AND candidate_gallery_id = ?3
                "#,
                params![i64::from(profile_version), parent_id, candidate_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(map_sqlite_error)?;
        let (stored_candidate_id, first_seen_in_run) =
            if let Some((id, revision, last_run)) = existing {
                transaction
                    .execute(
                        r#"
                        UPDATE duplicate_candidates
                        SET revision = ?1, last_seen_run_id = ?2,
                            parent_entry_id = ?3, candidate_entry_id = ?4,
                            relation = ?5, confidence = ?6, matched_pages = ?7,
                            parent_coverage = ?8, candidate_coverage = ?9,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        WHERE candidate_id = ?10 AND resolved = 0
                    "#,
                        params![
                            next_stored_revision(revision, "duplicate candidate revision")?,
                            record.run_id,
                            record.candidate.parent.entry_id,
                            record.candidate.candidate.entry_id,
                            record.candidate.relation.as_str(),
                            record.candidate.confidence,
                            i64::from(record.candidate.matched_pages),
                            record.candidate.parent_coverage,
                            record.candidate.candidate_coverage,
                            id,
                        ],
                    )
                    .map_err(map_sqlite_error)?;
                (id, last_run != record.run_id)
            } else {
                transaction
                    .execute(
                        r#"
                        INSERT INTO duplicate_candidates (
                            candidate_id, revision, last_seen_run_id, profile_version,
                            parent_gallery_id, parent_entry_id,
                            candidate_gallery_id, candidate_entry_id,
                            relation, confidence, matched_pages,
                            parent_coverage, candidate_coverage, resolved,
                            created_at, updated_at
                        ) VALUES (
                            ?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                            ?11, ?12, 0,
                            strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                            strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        )
                    "#,
                        params![
                            record.candidate.candidate_id,
                            record.run_id,
                            i64::from(profile_version),
                            parent_id,
                            record.candidate.parent.entry_id,
                            candidate_id,
                            record.candidate.candidate.entry_id,
                            record.candidate.relation.as_str(),
                            record.candidate.confidence,
                            i64::from(record.candidate.matched_pages),
                            record.candidate.parent_coverage,
                            record.candidate.candidate_coverage,
                        ],
                    )
                    .map_err(map_sqlite_error)?;
                (record.candidate.candidate_id.clone(), true)
            };

        transaction
            .execute(
                "DELETE FROM duplicate_evidence WHERE candidate_id = ?1",
                [&stored_candidate_id],
            )
            .map_err(map_sqlite_error)?;
        transaction
            .execute(
                "DELETE FROM duplicate_page_pairs WHERE candidate_id = ?1",
                [&stored_candidate_id],
            )
            .map_err(map_sqlite_error)?;
        for evidence in &record.evidence {
            transaction
                .execute(
                    r#"
                        INSERT INTO duplicate_evidence (
                            evidence_id, candidate_id, kind, confidence,
                            matched_pages, description
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                    "#,
                    params![
                        evidence.evidence_id,
                        stored_candidate_id,
                        evidence.kind.as_str(),
                        evidence.confidence,
                        i64::from(evidence.matched_pages),
                        evidence.description,
                    ],
                )
                .map_err(map_sqlite_error)?;
        }
        for pair in &record.page_pairs {
            transaction
                .execute(
                    r#"
                        INSERT INTO duplicate_page_pairs (
                            candidate_id, parent_source_page, candidate_source_page,
                            exact_sha256, d_hash_distance, p_hash_distance,
                            detail_hash_distance, edge_similarity,
                            visual_similarity, low_information
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                    "#,
                    params![
                        stored_candidate_id,
                        i64::from(pair.parent_source_page),
                        i64::from(pair.candidate_source_page),
                        pair.exact_sha256,
                        i64::from(pair.d_hash_distance),
                        i64::from(pair.p_hash_distance),
                        i64::from(pair.detail_hash_distance),
                        pair.edge_similarity,
                        pair.visual_similarity,
                        pair.low_information,
                    ],
                )
                .map_err(map_sqlite_error)?;
        }
        if first_seen_in_run {
            transaction
                .execute(
                    r#"
                        UPDATE duplicate_scan_runs
                        SET revision = revision + 1,
                            candidates_found = candidates_found + 1,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        WHERE run_id = ?1 AND state = 'running'
                    "#,
                    [&record.run_id],
                )
                .map_err(map_sqlite_error)?;
        }
        let run = read_duplicate_scan(&transaction, &record.run_id)?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(run)
    }

    fn duplicate_scan_finish(
        &self,
        run_id: &str,
        state: DuplicateScanState,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<Option<DuplicateScanRun>, RepositoryError> {
        if state == DuplicateScanState::Running {
            return Err(RepositoryError::Other(
                "a duplicate scan cannot finish in the running state".into(),
            ));
        }
        let connection = self.connection()?;
        connection
            .execute(
                r#"
                    UPDATE duplicate_scan_runs
                    SET revision = revision + 1, state = ?1,
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        error_code = ?2, error_message = ?3
                    WHERE run_id = ?4 AND state = 'running'
                "#,
                params![state.as_str(), error_code, error_message, run_id],
            )
            .map_err(map_sqlite_error)?;
        read_duplicate_scan(&connection, run_id)
    }

    fn duplicate_scan_is_running(&self, run_id: &str) -> Result<bool, RepositoryError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM duplicate_scan_runs WHERE run_id = ?1 AND state = 'running')",
                [run_id],
                |row| row.get(0),
            )
            .map_err(map_sqlite_error)
    }

    fn duplicate_snapshot(&self) -> Result<DuplicateSnapshot, RepositoryError> {
        let connection = self.connection()?;
        read_duplicate_snapshot(&connection)
    }

    fn duplicate_review_get(
        &self,
        candidate_id: &str,
    ) -> Result<Option<DuplicateReview>, RepositoryError> {
        let connection = self.connection()?;
        read_duplicate_review(&connection, candidate_id)
    }

    fn duplicate_decision_apply(
        &self,
        request: &DuplicateDecisionRequest,
    ) -> Result<DuplicateDecisionApplyOutcome, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let Some(candidate) = read_duplicate_candidate(&transaction, &request.candidate_id)? else {
            transaction.commit().map_err(map_sqlite_error)?;
            return Ok(DuplicateDecisionApplyOutcome::CandidateNotFound);
        };
        if candidate.candidate.revision != request.expected_revision {
            let actual_revision = candidate.candidate.revision;
            transaction.commit().map_err(map_sqlite_error)?;
            return Ok(DuplicateDecisionApplyOutcome::RevisionConflict { actual_revision });
        }
        let next_revision = request.expected_revision.checked_add(1).ok_or_else(|| {
            RepositoryError::Other("duplicate candidate revision is exhausted".into())
        })?;
        let decision_id = format!("duplicate-decision-{}", Uuid::new_v4());
        let (target_gallery_id, series_group_id) = apply_duplicate_decision_side_effect(
            &transaction,
            &candidate.candidate,
            request,
            &decision_id,
        )?;
        if matches!(
            request.action,
            DuplicateDecisionAction::HideParent | DuplicateDecisionAction::HideCandidate
        ) {
            let hidden_gallery_id = target_gallery_id.ok_or_else(|| {
                RepositoryError::Corrupt(
                    "a hide decision did not identify the hidden gallery".into(),
                )
            })?;
            transaction
                .execute(
                    "DELETE FROM exploration_restored_galleries WHERE gallery_id = ?1",
                    [hidden_gallery_id],
                )
                .map_err(map_sqlite_error)?;
        }
        let changed = transaction
            .execute(
                r#"
                    UPDATE duplicate_candidates
                    SET revision = ?1,
                        resolved = CASE WHEN ?4 THEN 1 ELSE resolved END,
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    WHERE candidate_id = ?2 AND revision = ?3
                "#,
                params![
                    to_sql_integer(next_revision, "duplicate candidate revision")?,
                    request.candidate_id,
                    to_sql_integer(request.expected_revision, "expected duplicate revision")?,
                    matches!(
                        request.action,
                        DuplicateDecisionAction::HideParent
                            | DuplicateDecisionAction::HideCandidate
                            | DuplicateDecisionAction::ExcludePair
                    ),
                ],
            )
            .map_err(map_sqlite_error)?;
        if changed != 1 {
            let actual_revision = transaction
                .query_row(
                    "SELECT revision FROM duplicate_candidates WHERE candidate_id = ?1",
                    [&request.candidate_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(map_sqlite_error)?
                .map(|value| stored_u64(value, "duplicate candidate revision"))
                .transpose()?
                .unwrap_or(request.expected_revision);
            transaction.rollback().map_err(map_sqlite_error)?;
            return Ok(DuplicateDecisionApplyOutcome::RevisionConflict { actual_revision });
        }
        transaction
            .execute(
                r#"
                    INSERT INTO duplicate_decisions (
                        decision_id, candidate_id, candidate_revision, action,
                        target_gallery_id, series_group_id, created_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                "#,
                params![
                    decision_id,
                    request.candidate_id,
                    to_sql_integer(next_revision, "duplicate decision revision")?,
                    request.action.as_str(),
                    target_gallery_id,
                    series_group_id,
                ],
            )
            .map_err(map_sqlite_error)?;
        let review =
            read_duplicate_review(&transaction, &request.candidate_id)?.ok_or_else(|| {
                RepositoryError::Corrupt("decided duplicate candidate disappeared".into())
            })?;
        transaction.commit().map_err(map_sqlite_error)?;
        Ok(DuplicateDecisionApplyOutcome::Applied(Box::new(review)))
    }
}

fn duplicate_bundle_is_verified(bundle: &ArtifactBundle) -> bool {
    bundle.artifact.state == DownloadArtifactState::Complete
        && bundle.artifact.manifest_relative_path.is_some()
        && bundle.artifact.manifest_schema_version.is_some()
        && bundle.artifact.writer_version.is_some()
        && bundle.artifact.completed_at.is_some()
        && !bundle.pages.is_empty()
        && bundle.pages.iter().all(|page| {
            page.state != PageArtifactState::Quarantined
                && (page.excluded
                    || (page.state == PageArtifactState::Present
                        && page.byte_length.is_some()
                        && page.sha256.is_some()
                        && page.storage_format.is_some()
                        && page.source_revision.is_some()
                        && page.verified_at.is_some()))
        })
        && bundle.pages.iter().any(|page| {
            !page.excluded
                && page.state == PageArtifactState::Present
                && page.byte_length.is_some()
                && page.sha256.is_some()
                && page.storage_format.is_some()
                && page.source_revision.is_some()
                && page.verified_at.is_some()
        })
}

fn validate_detail_hash(value: &str) -> Result<(), RepositoryError> {
    if value.len() != 256 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RepositoryError::Other(
            "duplicate detail hash must contain exactly 256 hexadecimal characters".into(),
        ));
    }
    Ok(())
}

fn parse_u64_hex(value: &str, label: &str) -> Result<u64, RepositoryError> {
    if value.len() != 16 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RepositoryError::Corrupt(format!(
            "{label} is not a 64-bit hexadecimal value"
        )));
    }
    u64::from_str_radix(value, 16)
        .map_err(|_| RepositoryError::Corrupt(format!("{label} cannot be decoded")))
}

struct StoredDuplicatePageHash {
    entry_id: String,
    gallery_id: i64,
    source_page_number: i64,
    profile_version: i64,
    artifact_sha256: String,
    coarse_d_hash_hex: String,
    detail_d_hash_hex: String,
    p_hash_hex: String,
    mean_luma: f64,
    std_dev: f64,
    non_uniform_ratio: f64,
    edge_density: f64,
    width: i64,
    height: i64,
    low_information: bool,
}

impl StoredDuplicatePageHash {
    fn try_into_domain(self) -> Result<DuplicatePageHash, RepositoryError> {
        validate_detail_hash(&self.detail_d_hash_hex)
            .map_err(|error| RepositoryError::Corrupt(error.to_string()))?;
        Ok(DuplicatePageHash {
            entry_id: self.entry_id,
            gallery_id: GalleryId::new(self.gallery_id).map_err(domain_corruption)?,
            source_page_number: SourcePageNumber::new(stored_u32(
                self.source_page_number,
                "duplicate hash source page number",
            )?)
            .map_err(domain_corruption)?,
            profile_version: stored_u32(self.profile_version, "duplicate hash profile version")?,
            artifact_sha256: ArtifactSha256::new(self.artifact_sha256)
                .map_err(domain_corruption)?,
            coarse_d_hash: parse_u64_hex(&self.coarse_d_hash_hex, "duplicate coarse dHash")?,
            detail_d_hash_hex: self.detail_d_hash_hex,
            p_hash: parse_u64_hex(&self.p_hash_hex, "duplicate pHash")?,
            mean_luma: self.mean_luma,
            std_dev: self.std_dev,
            non_uniform_ratio: self.non_uniform_ratio,
            edge_density: self.edge_density,
            width: stored_u32(self.width, "duplicate hash width")?,
            height: stored_u32(self.height, "duplicate hash height")?,
            low_information: self.low_information,
        })
    }
}

fn stored_duplicate_page_hash(row: &Row<'_>) -> rusqlite::Result<StoredDuplicatePageHash> {
    Ok(StoredDuplicatePageHash {
        entry_id: row.get(0)?,
        gallery_id: row.get(1)?,
        source_page_number: row.get(2)?,
        profile_version: row.get(3)?,
        artifact_sha256: row.get(4)?,
        coarse_d_hash_hex: row.get(5)?,
        detail_d_hash_hex: row.get(6)?,
        p_hash_hex: row.get(7)?,
        mean_luma: row.get(8)?,
        std_dev: row.get(9)?,
        non_uniform_ratio: row.get(10)?,
        edge_density: row.get(11)?,
        width: row.get(12)?,
        height: row.get(13)?,
        low_information: row.get(14)?,
    })
}

fn read_running_duplicate_scan(
    connection: &Connection,
) -> Result<Option<DuplicateScanRun>, RepositoryError> {
    connection
        .query_row(
            r#"
                SELECT run_id, revision, state, total_artifacts,
                       hashed_artifacts, total_pairs, compared_pairs,
                       candidates_found, started_at, updated_at, finished_at,
                       error_code, error_message
                FROM duplicate_scan_runs WHERE state = 'running' LIMIT 1
            "#,
            [],
            stored_duplicate_scan,
        )
        .optional()
        .map_err(map_sqlite_error)?
        .map(StoredDuplicateScan::try_into_domain)
        .transpose()
}

fn read_duplicate_scan(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<DuplicateScanRun>, RepositoryError> {
    connection
        .query_row(
            r#"
                SELECT run_id, revision, state, total_artifacts,
                       hashed_artifacts, total_pairs, compared_pairs,
                       candidates_found, started_at, updated_at, finished_at,
                       error_code, error_message
                FROM duplicate_scan_runs WHERE run_id = ?1
            "#,
            [run_id],
            stored_duplicate_scan,
        )
        .optional()
        .map_err(map_sqlite_error)?
        .map(StoredDuplicateScan::try_into_domain)
        .transpose()
}

fn running_duplicate_profile(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<u32>, RepositoryError> {
    connection
        .query_row(
            "SELECT profile_version FROM duplicate_scan_runs WHERE run_id = ?1 AND state = 'running'",
            [run_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(map_sqlite_error)?
        .map(|value| stored_u32(value, "duplicate scan profile version"))
        .transpose()
}

struct StoredDuplicateScan {
    run_id: String,
    revision: i64,
    state: String,
    total_artifacts: i64,
    hashed_artifacts: i64,
    total_pairs: i64,
    compared_pairs: i64,
    candidates_found: i64,
    started_at: String,
    updated_at: String,
    finished_at: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
}

impl StoredDuplicateScan {
    fn try_into_domain(self) -> Result<DuplicateScanRun, RepositoryError> {
        Ok(DuplicateScanRun {
            run_id: self.run_id,
            revision: stored_u64(self.revision, "duplicate scan revision")?,
            state: DuplicateScanState::from_database(&self.state).ok_or_else(|| {
                RepositoryError::Corrupt(format!(
                    "duplicate scan state {:?} is unsupported",
                    self.state
                ))
            })?,
            total_artifacts: stored_u32(self.total_artifacts, "duplicate scan artifact count")?,
            hashed_artifacts: stored_u32(
                self.hashed_artifacts,
                "duplicate scan hashed artifact count",
            )?,
            total_pairs: stored_u64(self.total_pairs, "duplicate scan total pair count")?,
            compared_pairs: stored_u64(self.compared_pairs, "duplicate scan compared pair count")?,
            candidates_found: stored_u32(self.candidates_found, "duplicate scan candidate count")?,
            started_at: self.started_at,
            updated_at: self.updated_at,
            finished_at: self.finished_at,
            error_code: self.error_code,
            error_message: self.error_message,
        })
    }
}

fn stored_duplicate_scan(row: &Row<'_>) -> rusqlite::Result<StoredDuplicateScan> {
    Ok(StoredDuplicateScan {
        run_id: row.get(0)?,
        revision: row.get(1)?,
        state: row.get(2)?,
        total_artifacts: row.get(3)?,
        hashed_artifacts: row.get(4)?,
        total_pairs: row.get(5)?,
        compared_pairs: row.get(6)?,
        candidates_found: row.get(7)?,
        started_at: row.get(8)?,
        updated_at: row.get(9)?,
        finished_at: row.get(10)?,
        error_code: row.get(11)?,
        error_message: row.get(12)?,
    })
}

fn validate_duplicate_candidate_record(
    record: &DuplicateCandidateRecord,
) -> Result<(), RepositoryError> {
    let candidate = &record.candidate;
    if record.run_id.trim().is_empty()
        || candidate.candidate_id.trim().is_empty()
        || candidate.parent.gallery_id.get() >= candidate.candidate.gallery_id.get()
        || candidate.parent.entry_id.trim().is_empty()
        || candidate.candidate.entry_id.trim().is_empty()
        || candidate.matched_pages == 0
        || !(0.0..=1.0).contains(&candidate.confidence)
        || !(0.0..=1.0).contains(&candidate.parent_coverage)
        || !(0.0..=1.0).contains(&candidate.candidate_coverage)
    {
        return Err(RepositoryError::Other(
            "duplicate candidate record is invalid or not canonically ordered".into(),
        ));
    }
    if record.evidence.iter().any(|evidence| {
        evidence.evidence_id.trim().is_empty()
            || evidence.description.trim().is_empty()
            || !(0.0..=1.0).contains(&evidence.confidence)
    }) || record.page_pairs.iter().any(|pair| {
        pair.parent_source_page == 0
            || pair.candidate_source_page == 0
            || !(0.0..=1.0).contains(&pair.edge_similarity)
            || !(0.0..=1.0).contains(&pair.visual_similarity)
    }) {
        return Err(RepositoryError::Other(
            "duplicate candidate evidence is invalid".into(),
        ));
    }
    Ok(())
}

struct LoadedDuplicateCandidate {
    candidate: DuplicateCandidate,
}

struct StoredDuplicateCandidate {
    candidate_id: String,
    revision: i64,
    parent_gallery_id: i64,
    parent_entry_id: String,
    parent_title: String,
    parent_artist: Option<String>,
    parent_group: Option<String>,
    parent_page_count: i64,
    candidate_gallery_id: i64,
    candidate_entry_id: String,
    candidate_title: String,
    candidate_artist: Option<String>,
    candidate_group: Option<String>,
    candidate_page_count: i64,
    relation: String,
    confidence: f64,
    matched_pages: i64,
    parent_coverage: f64,
    candidate_coverage: f64,
    created_at: String,
    updated_at: String,
}

impl StoredDuplicateCandidate {
    fn try_into_domain(self) -> Result<LoadedDuplicateCandidate, RepositoryError> {
        Ok(LoadedDuplicateCandidate {
            candidate: DuplicateCandidate {
                candidate_id: self.candidate_id,
                revision: stored_u64(self.revision, "duplicate candidate revision")?,
                parent: DuplicateGalleryRef {
                    gallery_id: GalleryId::new(self.parent_gallery_id)
                        .map_err(domain_corruption)?,
                    entry_id: self.parent_entry_id,
                    title: self.parent_title,
                    artist: self.parent_artist,
                    group: self.parent_group,
                    page_count: stored_u32(self.parent_page_count, "duplicate parent page count")?,
                },
                candidate: DuplicateGalleryRef {
                    gallery_id: GalleryId::new(self.candidate_gallery_id)
                        .map_err(domain_corruption)?,
                    entry_id: self.candidate_entry_id,
                    title: self.candidate_title,
                    artist: self.candidate_artist,
                    group: self.candidate_group,
                    page_count: stored_u32(
                        self.candidate_page_count,
                        "duplicate candidate page count",
                    )?,
                },
                relation: DuplicateRelation::from_database(&self.relation).ok_or_else(|| {
                    RepositoryError::Corrupt(format!(
                        "duplicate relation {:?} is unsupported",
                        self.relation
                    ))
                })?,
                confidence: self.confidence,
                matched_pages: stored_u32(self.matched_pages, "duplicate matched page count")?,
                parent_coverage: self.parent_coverage,
                candidate_coverage: self.candidate_coverage,
                created_at: self.created_at,
                updated_at: self.updated_at,
            },
        })
    }
}

const DUPLICATE_CANDIDATE_SELECT: &str = r#"
    SELECT c.candidate_id, c.revision,
           c.parent_gallery_id, c.parent_entry_id,
           pg.title, pg.primary_artist, pg.primary_group, pa.expected_page_count,
           c.candidate_gallery_id, c.candidate_entry_id,
           cg.title, cg.primary_artist, cg.primary_group, ca.expected_page_count,
           c.relation, c.confidence, c.matched_pages,
           c.parent_coverage, c.candidate_coverage, c.created_at, c.updated_at
    FROM duplicate_candidates c
    JOIN galleries pg ON pg.gallery_id = c.parent_gallery_id
    JOIN download_artifacts pa
      ON pa.entry_id = c.parent_entry_id AND pa.gallery_id = c.parent_gallery_id
    JOIN galleries cg ON cg.gallery_id = c.candidate_gallery_id
    JOIN download_artifacts ca
      ON ca.entry_id = c.candidate_entry_id AND ca.gallery_id = c.candidate_gallery_id
"#;

fn stored_duplicate_candidate(row: &Row<'_>) -> rusqlite::Result<StoredDuplicateCandidate> {
    Ok(StoredDuplicateCandidate {
        candidate_id: row.get(0)?,
        revision: row.get(1)?,
        parent_gallery_id: row.get(2)?,
        parent_entry_id: row.get(3)?,
        parent_title: row.get(4)?,
        parent_artist: row.get(5)?,
        parent_group: row.get(6)?,
        parent_page_count: row.get(7)?,
        candidate_gallery_id: row.get(8)?,
        candidate_entry_id: row.get(9)?,
        candidate_title: row.get(10)?,
        candidate_artist: row.get(11)?,
        candidate_group: row.get(12)?,
        candidate_page_count: row.get(13)?,
        relation: row.get(14)?,
        confidence: row.get(15)?,
        matched_pages: row.get(16)?,
        parent_coverage: row.get(17)?,
        candidate_coverage: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
    })
}

fn read_duplicate_candidate(
    connection: &Connection,
    candidate_id: &str,
) -> Result<Option<LoadedDuplicateCandidate>, RepositoryError> {
    let sql = format!("{DUPLICATE_CANDIDATE_SELECT} WHERE c.candidate_id = ?1");
    connection
        .query_row(&sql, [candidate_id], stored_duplicate_candidate)
        .optional()
        .map_err(map_sqlite_error)?
        .map(StoredDuplicateCandidate::try_into_domain)
        .transpose()
}

fn read_duplicate_candidates_for_run(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<DuplicateCandidate>, RepositoryError> {
    let sql = format!(
        r#"
            {DUPLICATE_CANDIDATE_SELECT}
            WHERE c.last_seen_run_id = ?1 AND c.resolved = 0
              AND NOT EXISTS (
                  SELECT 1 FROM duplicate_hidden_galleries hidden
                  WHERE hidden.gallery_id IN (
                      c.parent_gallery_id, c.candidate_gallery_id
                  )
              )
              AND NOT EXISTS (
                  SELECT 1 FROM duplicate_pair_exclusions exclusion
                  WHERE exclusion.parent_gallery_id = c.parent_gallery_id
                    AND exclusion.candidate_gallery_id = c.candidate_gallery_id
              )
            ORDER BY c.confidence DESC, c.updated_at DESC, c.candidate_id ASC
        "#
    );
    let mut statement = connection.prepare(&sql).map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([run_id], stored_duplicate_candidate)
        .map_err(map_sqlite_error)?;
    let mut candidates = Vec::new();
    for row in rows {
        candidates.push(row.map_err(map_sqlite_error)?.try_into_domain()?.candidate);
    }
    Ok(candidates)
}

fn read_duplicate_hash_profile(
    connection: &Connection,
    profile_version: u32,
) -> Result<HashProfile, RepositoryError> {
    let stored = connection
        .query_row(
            r#"
                SELECT profile_version, algorithm_version, d_hash_bits, p_hash_bits,
                       visual_match_threshold, low_information_std_dev_threshold
                FROM duplicate_hash_profiles WHERE profile_version = ?1
            "#,
            [i64::from(profile_version)],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                ))
            },
        )
        .optional()
        .map_err(map_sqlite_error)?
        .ok_or_else(|| {
            RepositoryError::Corrupt(format!(
                "duplicate hash profile {profile_version} is missing"
            ))
        })?;
    Ok(HashProfile {
        profile_version: stored_u32(stored.0, "duplicate hash profile version")?,
        algorithm_version: stored_u32(stored.1, "duplicate hash algorithm version")?,
        d_hash_bits: stored_u32(stored.2, "duplicate dHash bit count")?,
        p_hash_bits: stored_u32(stored.3, "duplicate pHash bit count")?,
        visual_match_threshold: stored.4,
        low_information_std_dev_threshold: stored.5,
    })
}

fn read_duplicate_snapshot(connection: &Connection) -> Result<DuplicateSnapshot, RepositoryError> {
    let latest = connection
        .query_row(
            r#"
                SELECT run_id, profile_version FROM duplicate_scan_runs
                ORDER BY started_at DESC, run_id DESC LIMIT 1
            "#,
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(map_sqlite_error)?;
    let (run, candidates, profile_version) = if let Some((run_id, profile)) = latest {
        let run = read_duplicate_scan(connection, &run_id)?
            .ok_or_else(|| RepositoryError::Corrupt("latest duplicate scan disappeared".into()))?;
        let candidates = read_duplicate_candidates_for_run(connection, &run_id)?;
        (
            Some(run),
            candidates,
            stored_u32(profile, "duplicate scan profile version")?,
        )
    } else {
        (None, Vec::new(), HashProfile::current().profile_version)
    };
    Ok(DuplicateSnapshot {
        profile: read_duplicate_hash_profile(connection, profile_version)?,
        run,
        candidates,
    })
}

fn read_duplicate_review(
    connection: &Connection,
    candidate_id: &str,
) -> Result<Option<DuplicateReview>, RepositoryError> {
    let Some(candidate) = read_duplicate_candidate(connection, candidate_id)? else {
        return Ok(None);
    };
    let mut evidence_statement = connection
        .prepare(
            r#"
                SELECT evidence_id, kind, confidence, matched_pages, description
                FROM duplicate_evidence WHERE candidate_id = ?1
                ORDER BY kind ASC, evidence_id ASC
            "#,
        )
        .map_err(map_sqlite_error)?;
    let evidence_rows = evidence_statement
        .query_map([candidate_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(map_sqlite_error)?;
    let mut evidence = Vec::new();
    for row in evidence_rows {
        let row = row.map_err(map_sqlite_error)?;
        evidence.push(DuplicateEvidence {
            evidence_id: row.0,
            kind: DuplicateEvidenceKind::from_database(&row.1).ok_or_else(|| {
                RepositoryError::Corrupt(format!(
                    "duplicate evidence kind {:?} is unsupported",
                    row.1
                ))
            })?,
            confidence: row.2,
            matched_pages: stored_u32(row.3, "duplicate evidence page count")?,
            description: row.4,
        });
    }

    let mut pair_statement = connection
        .prepare(
            r#"
                SELECT parent_source_page, candidate_source_page, exact_sha256,
                       d_hash_distance, p_hash_distance, detail_hash_distance,
                       edge_similarity, visual_similarity, low_information
                FROM duplicate_page_pairs WHERE candidate_id = ?1
                ORDER BY parent_source_page ASC, candidate_source_page ASC
            "#,
        )
        .map_err(map_sqlite_error)?;
    let pair_rows = pair_statement
        .query_map([candidate_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, f64>(7)?,
                row.get::<_, bool>(8)?,
            ))
        })
        .map_err(map_sqlite_error)?;
    let mut page_pairs = Vec::new();
    for row in pair_rows {
        let row = row.map_err(map_sqlite_error)?;
        page_pairs.push(DuplicatePagePair {
            parent_source_page: stored_u32(row.0, "duplicate parent source page")?,
            candidate_source_page: stored_u32(row.1, "duplicate candidate source page")?,
            exact_sha256: row.2,
            d_hash_distance: stored_u32(row.3, "duplicate dHash distance")?,
            p_hash_distance: stored_u32(row.4, "duplicate pHash distance")?,
            detail_hash_distance: stored_u32(row.5, "duplicate detail hash distance")?,
            edge_similarity: row.6,
            visual_similarity: row.7,
            low_information: row.8,
        });
    }

    let mut decision_statement = connection
        .prepare(
            r#"
                SELECT decision_id, candidate_id, candidate_revision, action,
                       target_gallery_id, series_group_id, created_at
                FROM duplicate_decisions WHERE candidate_id = ?1
                ORDER BY created_at ASC, decision_id ASC
            "#,
        )
        .map_err(map_sqlite_error)?;
    let decision_rows = decision_statement
        .query_map([candidate_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(map_sqlite_error)?;
    let mut decisions = Vec::new();
    for row in decision_rows {
        let row = row.map_err(map_sqlite_error)?;
        decisions.push(DuplicateDecisionHistory {
            decision_id: row.0,
            candidate_id: row.1,
            candidate_revision: stored_u64(row.2, "duplicate decision revision")?,
            action: DuplicateDecisionAction::from_database(&row.3).ok_or_else(|| {
                RepositoryError::Corrupt(format!(
                    "duplicate decision action {:?} is unsupported",
                    row.3
                ))
            })?,
            target_gallery_id: row
                .4
                .map(GalleryId::new)
                .transpose()
                .map_err(domain_corruption)?,
            series_group_id: row.5,
            created_at: row.6,
        });
    }

    Ok(Some(DuplicateReview {
        candidate: candidate.candidate,
        evidence,
        page_pairs,
        decisions,
        series_groups: read_duplicate_series_groups(connection)?,
    }))
}

fn read_duplicate_series_groups(
    connection: &Connection,
) -> Result<Vec<SeriesGroup>, RepositoryError> {
    let groups = {
        let mut statement = connection
            .prepare(
                r#"
                    SELECT series_group_id, name, revision, created_at, updated_at
                    FROM duplicate_series_groups
                    ORDER BY name COLLATE NOCASE ASC, series_group_id ASC
                "#,
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(map_sqlite_error)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(map_sqlite_error)?
    };
    let mut result = Vec::with_capacity(groups.len());
    for (series_group_id, name, revision, created_at, updated_at) in groups {
        let mut statement = connection
            .prepare(
                r#"
                    SELECT m.gallery_id, m.entry_id, g.title,
                           g.primary_artist, g.primary_group, a.expected_page_count
                    FROM duplicate_series_members m
                    JOIN galleries g ON g.gallery_id = m.gallery_id
                    JOIN download_artifacts a
                      ON a.entry_id = m.entry_id AND a.gallery_id = m.gallery_id
                    WHERE m.series_group_id = ?1
                    ORDER BY m.created_at ASC, m.gallery_id ASC
                "#,
            )
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([&series_group_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(map_sqlite_error)?;
        let mut members = Vec::new();
        for row in rows {
            let row = row.map_err(map_sqlite_error)?;
            members.push(DuplicateGalleryRef {
                gallery_id: GalleryId::new(row.0).map_err(domain_corruption)?,
                entry_id: row.1,
                title: row.2,
                artist: row.3,
                group: row.4,
                page_count: stored_u32(row.5, "duplicate series member page count")?,
            });
        }
        result.push(SeriesGroup {
            series_group_id,
            name,
            revision: stored_u64(revision, "duplicate series revision")?,
            members,
            created_at,
            updated_at,
        });
    }
    Ok(result)
}

fn duplicate_decision_target(
    candidate: &DuplicateCandidate,
    requested: Option<i64>,
    required: GalleryId,
) -> Result<GalleryId, RepositoryError> {
    match requested {
        Some(value) => {
            let target = GalleryId::new(value).map_err(domain_corruption)?;
            if target != required {
                return Err(RepositoryError::Other(
                    "duplicate decision target does not match its action".into(),
                ));
            }
            Ok(target)
        }
        None => {
            let _ = candidate;
            Ok(required)
        }
    }
}

fn duplicate_series_target(
    candidate: &DuplicateCandidate,
    requested: Option<i64>,
) -> Result<(GalleryId, &str), RepositoryError> {
    let value = requested.ok_or_else(|| {
        RepositoryError::Other("a series decision requires targetGalleryId".into())
    })?;
    let target = GalleryId::new(value).map_err(domain_corruption)?;
    if target == candidate.parent.gallery_id {
        Ok((target, &candidate.parent.entry_id))
    } else if target == candidate.candidate.gallery_id {
        Ok((target, &candidate.candidate.entry_id))
    } else {
        Err(RepositoryError::Other(
            "series target must be one of the duplicate candidate galleries".into(),
        ))
    }
}

fn validated_series_name(value: Option<&str>) -> Result<Option<String>, RepositoryError> {
    value
        .map(|value| {
            let value = value.trim();
            if value.is_empty() || value.chars().count() > 200 {
                return Err(RepositoryError::Other(
                    "seriesName must contain between 1 and 200 characters".into(),
                ));
            }
            Ok(value.to_owned())
        })
        .transpose()
}

fn validated_series_group_id(value: &str) -> Result<String, RepositoryError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(RepositoryError::Other(
            "seriesGroupId must not be empty".into(),
        ));
    }
    Ok(value.to_owned())
}

fn apply_duplicate_decision_side_effect(
    transaction: &Transaction<'_>,
    candidate: &DuplicateCandidate,
    request: &DuplicateDecisionRequest,
    decision_id: &str,
) -> Result<(Option<i64>, Option<String>), RepositoryError> {
    match request.action {
        DuplicateDecisionAction::HideParent | DuplicateDecisionAction::HideCandidate => {
            if request.series_group_id.is_some() || request.series_name.is_some() {
                return Err(RepositoryError::Other(
                    "hide decisions do not accept series fields".into(),
                ));
            }
            if candidate.relation == DuplicateRelation::Contains
                && candidate.parent.page_count != candidate.candidate.page_count
            {
                let required_action =
                    if candidate.parent.page_count > candidate.candidate.page_count {
                        DuplicateDecisionAction::HideCandidate
                    } else {
                        DuplicateDecisionAction::HideParent
                    };
                if request.action != required_action {
                    return Err(RepositoryError::Other(
                        "contains decisions must keep the gallery with more pages".into(),
                    ));
                }
            }
            let required = if request.action == DuplicateDecisionAction::HideParent {
                candidate.parent.gallery_id
            } else {
                candidate.candidate.gallery_id
            };
            let target = duplicate_decision_target(candidate, request.target_gallery_id, required)?;
            transaction
                .execute(
                    r#"
                        INSERT INTO duplicate_hidden_galleries (
                            gallery_id, decision_id, created_at
                        ) VALUES (
                            ?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        )
                        ON CONFLICT(gallery_id) DO UPDATE SET
                            decision_id = excluded.decision_id,
                            created_at = excluded.created_at
                    "#,
                    params![target.get(), decision_id],
                )
                .map_err(map_sqlite_error)?;
            Ok((Some(target.get()), None))
        }
        DuplicateDecisionAction::ExcludePair => {
            if request.target_gallery_id.is_some()
                || request.series_group_id.is_some()
                || request.series_name.is_some()
            {
                return Err(RepositoryError::Other(
                    "exclude_pair does not accept target or series fields".into(),
                ));
            }
            transaction
                .execute(
                    r#"
                        INSERT INTO duplicate_pair_exclusions (
                            parent_gallery_id, candidate_gallery_id,
                            decision_id, created_at
                        ) VALUES (
                            ?1, ?2, ?3,
                            strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        )
                        ON CONFLICT(parent_gallery_id, candidate_gallery_id) DO UPDATE SET
                            decision_id = excluded.decision_id,
                            created_at = excluded.created_at
                    "#,
                    params![
                        candidate.parent.gallery_id.get(),
                        candidate.candidate.gallery_id.get(),
                        decision_id,
                    ],
                )
                .map_err(map_sqlite_error)?;
            Ok((None, None))
        }
        DuplicateDecisionAction::SeriesLink => {
            if request.target_gallery_id.is_some() {
                return Err(RepositoryError::Other(
                    "series_link atomically links both galleries and does not accept targetGalleryId"
                        .into(),
                ));
            }
            let series_name = validated_series_name(request.series_name.as_deref())?;
            let series_group_id = request
                .series_group_id
                .as_deref()
                .map(validated_series_group_id)
                .transpose()?
                .unwrap_or_else(|| format!("duplicate-series-{}", Uuid::new_v4()));
            let group_exists: bool = transaction
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM duplicate_series_groups WHERE series_group_id = ?1)",
                    [&series_group_id],
                    |row| row.get(0),
                )
                .map_err(map_sqlite_error)?;
            if !group_exists {
                let name = series_name.as_deref().ok_or_else(|| {
                    RepositoryError::Other(
                        "seriesName is required when creating a series group".into(),
                    )
                })?;
                transaction
                    .execute(
                        r#"
                            INSERT INTO duplicate_series_groups (
                                series_group_id, name, revision, created_at, updated_at
                            ) VALUES (
                                ?1, ?2, 0,
                                strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                                strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                            )
                        "#,
                        params![series_group_id, name],
                    )
                    .map_err(map_sqlite_error)?;
            }
            for member in [&candidate.parent, &candidate.candidate] {
                transaction
                    .execute(
                        r#"
                            INSERT OR IGNORE INTO duplicate_series_members (
                                series_group_id, gallery_id, entry_id, created_at
                            ) VALUES (
                                ?1, ?2, ?3,
                                strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                            )
                        "#,
                        params![series_group_id, member.gallery_id.get(), member.entry_id,],
                    )
                    .map_err(map_sqlite_error)?;
            }
            transaction
                .execute(
                    r#"
                        UPDATE duplicate_series_groups
                        SET name = COALESCE(?1, name),
                            revision = revision + 1,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        WHERE series_group_id = ?2
                    "#,
                    params![series_name, series_group_id],
                )
                .map_err(map_sqlite_error)?;
            Ok((None, Some(series_group_id)))
        }
        DuplicateDecisionAction::SeriesUnlink => {
            if request.series_name.is_some() {
                return Err(RepositoryError::Other(
                    "series_unlink does not accept seriesName".into(),
                ));
            }
            let (target, _) = duplicate_series_target(candidate, request.target_gallery_id)?;
            let series_group_id =
                validated_series_group_id(request.series_group_id.as_deref().ok_or_else(
                    || RepositoryError::Other("series_unlink requires seriesGroupId".into()),
                )?)?;
            let removed = transaction
                .execute(
                    r#"
                        DELETE FROM duplicate_series_members
                        WHERE series_group_id = ?1 AND gallery_id = ?2
                    "#,
                    params![series_group_id, target.get()],
                )
                .map_err(map_sqlite_error)?;
            if removed == 0 {
                return Err(RepositoryError::Other(
                    "the gallery is not a member of the requested series group".into(),
                ));
            }
            transaction
                .execute(
                    r#"
                        UPDATE duplicate_series_groups
                        SET revision = revision + 1,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        WHERE series_group_id = ?1
                    "#,
                    [&series_group_id],
                )
                .map_err(map_sqlite_error)?;
            Ok((Some(target.get()), Some(series_group_id)))
        }
    }
}

struct StoredPipelineTarget {
    job_id: String,
    entry_id: String,
    gallery_id: i64,
    job_revision: i64,
    entry_revision: i64,
    state: JobState,
    completed_units: i64,
    total_units: i64,
    attempt: i64,
    progress: f64,
    error_code: Option<String>,
    error_message: Option<String>,
}

impl StoredPipelineTarget {
    fn into_projection(
        self,
        message: Option<&str>,
    ) -> Result<DownloadJobProjection, RepositoryError> {
        Ok(DownloadJobProjection {
            job: JobEvent {
                job_id: self.job_id,
                gallery_id: Some(self.gallery_id),
                revision: stored_u64(self.job_revision, "job revision")?,
                state: self.state,
                completed_units: Some(stored_u64(self.completed_units, "completed units")?),
                total_units: Some(stored_u64(self.total_units, "total units")?),
                message: message.map(str::to_owned),
            },
            download: DownloadChangedEvent {
                entry_id: self.entry_id,
                gallery_id: self.gallery_id,
                revision: stored_u64(self.entry_revision, "download revision")?,
                state: self.state,
                progress: Some(self.progress),
                attempt: Some(stored_u64(self.attempt, "download attempt")?),
                error_code: self.error_code,
                error_message: self.error_message,
                review_kind: None,
                review_id: None,
            },
        })
    }
}

fn read_pipeline_target(
    transaction: &Transaction<'_>,
    descriptor: &DownloadJobDescriptor,
) -> Result<StoredPipelineTarget, RepositoryError> {
    let stored = transaction
        .query_row(
            r#"
                SELECT j.job_id, j.entry_id, j.gallery_id, j.revision,
                       d.revision, j.state, j.completed_units, j.total_units,
                       j.attempt, d.progress, j.last_error_code, j.last_error_message
                FROM download_jobs j
                JOIN download_entries d
                  ON d.entry_id = j.entry_id AND d.gallery_id = j.gallery_id
                WHERE j.job_id = ?1
            "#,
            [&descriptor.job_id],
            |row| {
                Ok(StoredPipelineTarget {
                    job_id: row.get(0)?,
                    entry_id: row.get(1)?,
                    gallery_id: row.get(2)?,
                    job_revision: row.get(3)?,
                    entry_revision: row.get(4)?,
                    state: row
                        .get::<_, String>(5)?
                        .parse::<JobState>()
                        .map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                5,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?,
                    completed_units: row.get(6)?,
                    total_units: row.get(7)?,
                    attempt: row.get(8)?,
                    progress: row.get(9)?,
                    error_code: row.get(10)?,
                    error_message: row.get(11)?,
                })
            },
        )
        .optional()
        .map_err(map_sqlite_error)?
        .ok_or_else(|| RepositoryError::Other("download job no longer exists".into()))?;
    if stored.entry_id != descriptor.entry_id
        || stored.gallery_id != descriptor.gallery_id.get()
        || stored_u64(stored.attempt, "download attempt")? != descriptor.worker_attempt
    {
        return Err(RepositoryError::Other(
            "download worker descriptor is stale".into(),
        ));
    }
    Ok(stored)
}

fn ensure_current_pipeline_attempt(
    connection: &Connection,
    descriptor: &DownloadJobDescriptor,
) -> Result<(), RepositoryError> {
    let current = connection
        .query_row(
            "SELECT entry_id, gallery_id, attempt FROM download_jobs WHERE job_id = ?1",
            [&descriptor.job_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(map_sqlite_error)?;
    if current.is_none_or(|current| {
        current.0 != descriptor.entry_id
            || current.1 != descriptor.gallery_id.get()
            || u64::try_from(current.2).ok() != Some(descriptor.worker_attempt)
    }) {
        return Err(RepositoryError::Other(
            "download worker descriptor is stale".into(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn transition_pipeline_target(
    transaction: &Transaction<'_>,
    target: StoredPipelineTarget,
    next_state: JobState,
    completed_units: Option<u64>,
    total_units: Option<u64>,
    error_code: Option<&str>,
    error_message: Option<&str>,
    message: &'static str,
) -> Result<DownloadJobProjection, RepositoryError> {
    if !target.state.allows_transition_to(next_state) {
        return Err(invalid_pipeline_state(&target, "transition"));
    }
    let job_revision = next_stored_revision(target.job_revision, "job revision")?;
    let entry_revision = next_stored_revision(target.entry_revision, "download revision")?;
    let completed_units = completed_units
        .map(|value| to_sql_integer(value, "completed units"))
        .transpose()?
        .unwrap_or(target.completed_units);
    let total_units = total_units
        .map(|value| to_sql_integer(value, "total units"))
        .transpose()?
        .unwrap_or(target.total_units);
    if total_units <= 0 || completed_units < 0 || completed_units > total_units {
        return Err(RepositoryError::Corrupt(
            "download progress units are inconsistent".into(),
        ));
    }
    let progress = (completed_units as f64 / total_units as f64) * 100.0;
    let terminal = !next_state.is_active();
    let changed_jobs = transaction
        .execute(
            r#"
                UPDATE download_jobs
                SET revision = ?1, state = ?2,
                    completed_units = ?3, total_units = ?4,
                    last_error_code = ?5, last_error_message = ?6,
                    last_error_retryable = CASE
                        WHEN ?5 IS NULL THEN NULL ELSE last_error_retryable END,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                    started_at = CASE
                        WHEN ?2 != 'queued' THEN COALESCE(
                            started_at,
                            strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        ) ELSE started_at END,
                    finished_at = CASE
                        WHEN ?7 THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        ELSE NULL END
                WHERE job_id = ?8 AND revision = ?9 AND attempt = ?10 AND state = ?11
            "#,
            params![
                to_sql_integer(job_revision, "job revision")?,
                next_state.to_string(),
                completed_units,
                total_units,
                error_code,
                error_message,
                terminal,
                target.job_id,
                target.job_revision,
                target.attempt,
                target.state.to_string(),
            ],
        )
        .map_err(map_sqlite_error)?;
    let changed_entries = transaction
        .execute(
            r#"
                UPDATE download_entries
                SET revision = ?1, state = ?2, progress = ?3,
                    review_kind = NULL, review_id = NULL,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                WHERE entry_id = ?4 AND revision = ?5 AND state = ?6
            "#,
            params![
                to_sql_integer(entry_revision, "download revision")?,
                next_state.to_string(),
                progress,
                target.entry_id,
                target.entry_revision,
                target.state.to_string(),
            ],
        )
        .map_err(map_sqlite_error)?;
    if changed_jobs != 1 || changed_entries != 1 {
        return Err(RepositoryError::Other(
            "download pipeline state changed concurrently".into(),
        ));
    }
    StoredPipelineTarget {
        job_revision: to_sql_integer(job_revision, "job revision")?,
        entry_revision: to_sql_integer(entry_revision, "download revision")?,
        state: next_state,
        completed_units,
        total_units,
        progress,
        error_code: error_code.map(str::to_owned),
        error_message: error_message.map(str::to_owned),
        ..target
    }
    .into_projection(Some(message))
}

fn update_pipeline_progress(
    transaction: &Transaction<'_>,
    target: StoredPipelineTarget,
    completed_units: u64,
    message: &'static str,
) -> Result<DownloadJobProjection, RepositoryError> {
    let total_units = stored_u64(target.total_units, "total units")?;
    if completed_units > total_units {
        return Err(RepositoryError::Corrupt(
            "verified page count exceeds the expected page count".into(),
        ));
    }
    let job_revision = next_stored_revision(target.job_revision, "job revision")?;
    let entry_revision = next_stored_revision(target.entry_revision, "download revision")?;
    let progress = if total_units == 0 {
        0.0
    } else {
        (completed_units as f64 / total_units as f64) * 100.0
    };
    let changed_jobs = transaction
        .execute(
            r#"
                UPDATE download_jobs
                SET revision = ?1, completed_units = ?2,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                WHERE job_id = ?3 AND revision = ?4 AND attempt = ?5 AND state = ?6
            "#,
            params![
                to_sql_integer(job_revision, "job revision")?,
                to_sql_integer(completed_units, "completed units")?,
                target.job_id,
                target.job_revision,
                target.attempt,
                target.state.to_string(),
            ],
        )
        .map_err(map_sqlite_error)?;
    let changed_entries = transaction
        .execute(
            r#"
                UPDATE download_entries
                SET revision = ?1, progress = ?2,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                WHERE entry_id = ?3 AND revision = ?4 AND state = ?5
            "#,
            params![
                to_sql_integer(entry_revision, "download revision")?,
                progress,
                target.entry_id,
                target.entry_revision,
                target.state.to_string(),
            ],
        )
        .map_err(map_sqlite_error)?;
    if changed_jobs != 1 || changed_entries != 1 {
        return Err(RepositoryError::Other(
            "download pipeline progress changed concurrently".into(),
        ));
    }
    StoredPipelineTarget {
        job_revision: to_sql_integer(job_revision, "job revision")?,
        entry_revision: to_sql_integer(entry_revision, "download revision")?,
        completed_units: to_sql_integer(completed_units, "completed units")?,
        progress,
        ..target
    }
    .into_projection(Some(message))
}

fn invalid_pipeline_state(target: &StoredPipelineTarget, operation: &str) -> RepositoryError {
    RepositoryError::Other(format!(
        "download job {:?} cannot {operation} from {}",
        target.job_id, target.state
    ))
}

struct StoredDownloadTarget {
    job_id: String,
    entry_id: DownloadEntryId,
    gallery_id: i64,
    job_revision: i64,
    entry_revision: i64,
    state: JobState,
    progress: f64,
    review_kind: Option<String>,
    review_id: Option<String>,
    attempt: i64,
    error_code: Option<String>,
    error_message: Option<String>,
}

impl StoredDownloadTarget {
    fn into_download_entry(self) -> Result<DownloadEntry, RepositoryError> {
        let review_kind = parse_download_review_kind(self.review_kind)?;
        if self.state == JobState::ReviewRequired
            && (review_kind.is_none() || self.review_id.as_deref().is_none_or(str::is_empty))
        {
            return Err(RepositoryError::Corrupt(
                "review_required download entry is missing its review target".into(),
            ));
        }
        Ok(DownloadEntry {
            entry_id: self.entry_id,
            gallery_id: GalleryId::new(self.gallery_id).map_err(domain_corruption)?,
            revision: stored_u64(self.entry_revision, "download revision")?,
            state: self.state,
            progress: Some(self.progress),
            attempt: Some(stored_u64(self.attempt, "download attempt")?),
            error_code: self.error_code,
            error_message: self.error_message,
            error_retryable: None,
            review_kind,
            review_id: self.review_id,
            created_at: None,
            updated_at: None,
        })
    }
}

fn read_download_target(
    transaction: &Transaction<'_>,
    entry_id: &DownloadEntryId,
) -> Result<Option<StoredDownloadTarget>, RepositoryError> {
    let stored = transaction
        .query_row(
            r#"
                SELECT
                    j.job_id, j.entry_id, j.gallery_id,
                    j.revision, d.revision, j.state, d.state,
                    d.progress, d.review_kind, d.review_id, j.attempt,
                    j.last_error_code, j.last_error_message
                FROM download_jobs j
                JOIN download_entries d
                  ON d.entry_id = j.entry_id AND d.gallery_id = j.gallery_id
                WHERE d.entry_id = ?1
            "#,
            [entry_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, f64>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                ))
            },
        )
        .optional()
        .map_err(map_sqlite_error)?;
    let Some((
        job_id,
        stored_entry_id,
        gallery_id,
        job_revision,
        entry_revision,
        stored_job_state,
        stored_entry_state,
        progress,
        review_kind,
        review_id,
        attempt,
        error_code,
        error_message,
    )) = stored
    else {
        return Ok(None);
    };
    let job_state = stored_job_state
        .parse::<JobState>()
        .map_err(domain_corruption)?;
    let entry_state = stored_entry_state
        .parse::<JobState>()
        .map_err(domain_corruption)?;
    if job_state != entry_state {
        return Err(RepositoryError::Corrupt(format!(
            "download job {job_id:?} and entry {stored_entry_id:?} disagree on state"
        )));
    }
    Ok(Some(StoredDownloadTarget {
        job_id,
        entry_id: DownloadEntryId::new(stored_entry_id).map_err(domain_corruption)?,
        gallery_id,
        job_revision,
        entry_revision,
        state: entry_state,
        progress,
        review_kind,
        review_id,
        attempt,
        error_code,
        error_message,
    }))
}

fn active_job_for_gallery(
    transaction: &Transaction<'_>,
    gallery_id: i64,
    excluded_entry_id: &str,
) -> Result<Option<String>, RepositoryError> {
    transaction
        .query_row(
            r#"
                SELECT j.job_id
                FROM download_entries d
                JOIN download_jobs j
                  ON j.entry_id = d.entry_id AND j.gallery_id = d.gallery_id
                WHERE d.gallery_id = ?1
                  AND d.entry_id != ?2
                  AND d.state = j.state
                  AND d.state IN (
                      'queued', 'resolving_metadata', 'downloading',
                      'hashing', 'verifying', 'retry_wait'
                  )
                LIMIT 1
            "#,
            params![gallery_id, excluded_entry_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sqlite_error)
}

fn recover_volatile_downloads(connection: &mut Connection) -> Result<usize, RepositoryError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite_error)?;
    transaction
        .execute(
            r#"
                UPDATE download_attempts
                SET finished_at = COALESCE(
                        finished_at,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    ),
                    outcome_state = 'failed',
                    error_code = COALESCE(
                        NULLIF(trim(error_code), ''),
                        (
                            SELECT COALESCE(
                                NULLIF(trim(j.last_error_code), ''),
                                'ARTIFACT_INTEGRITY_REVIEW'
                            )
                            FROM download_jobs j
                            WHERE j.job_id = download_attempts.job_id
                        )
                    ),
                    error_message = COALESCE(
                        NULLIF(trim(error_message), ''),
                        (
                            SELECT COALESCE(
                                NULLIF(trim(j.last_error_message), ''),
                                'Artifact integrity requires a safe retry or manual recovery'
                            )
                            FROM download_jobs j
                            WHERE j.job_id = download_attempts.job_id
                        )
                    ),
                    error_retryable = 0
                WHERE outcome_state IS NULL
                  AND EXISTS (
                    SELECT 1
                    FROM download_jobs j
                    JOIN download_entries d ON d.entry_id = j.entry_id
                    WHERE j.job_id = download_attempts.job_id
                      AND j.attempt = download_attempts.attempt
                      AND j.state = 'review_required'
                      AND d.state = 'review_required'
                      AND (
                        NULLIF(trim(d.review_kind), '') IS NULL
                        OR NULLIF(trim(d.review_id), '') IS NULL
                        OR (
                          d.review_kind = 'gallery_duplicate'
                          AND NOT EXISTS (
                            SELECT 1
                            FROM download_overlap_reviews overlap_review
                            WHERE overlap_review.review_id = d.review_id
                              AND overlap_review.entry_id = d.entry_id
                              AND overlap_review.state = 'pending'
                          )
                        )
                      )
                  )
            "#,
            [],
        )
        .map_err(map_sqlite_error)?;
    transaction
        .execute(
            r#"
                UPDATE download_jobs
                SET revision = revision + 1,
                    state = 'failed',
                    last_error_code = COALESCE(
                        NULLIF(trim(last_error_code), ''),
                        'ARTIFACT_INTEGRITY_REVIEW'
                    ),
                    last_error_message = COALESCE(
                        NULLIF(trim(last_error_message), ''),
                        'Artifact integrity requires a safe retry or manual recovery'
                    ),
                    last_error_retryable = 0,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                    finished_at = COALESCE(
                        finished_at,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                WHERE state = 'review_required'
                  AND entry_id IN (
                    SELECT d.entry_id
                    FROM download_entries d
                    WHERE d.state = 'review_required'
                      AND (
                        NULLIF(trim(d.review_kind), '') IS NULL
                        OR NULLIF(trim(d.review_id), '') IS NULL
                        OR (
                          d.review_kind = 'gallery_duplicate'
                          AND NOT EXISTS (
                            SELECT 1
                            FROM download_overlap_reviews overlap_review
                            WHERE overlap_review.review_id = d.review_id
                              AND overlap_review.entry_id = d.entry_id
                              AND overlap_review.state = 'pending'
                          )
                        )
                      )
                  )
            "#,
            [],
        )
        .map_err(map_sqlite_error)?;
    let repaired_invalid_reviews = transaction
        .execute(
            r#"
                UPDATE download_entries
                SET revision = revision + 1,
                    state = 'failed',
                    review_kind = NULL,
                    review_id = NULL,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                WHERE state = 'review_required'
                  AND (
                    NULLIF(trim(review_kind), '') IS NULL
                    OR NULLIF(trim(review_id), '') IS NULL
                    OR (
                      review_kind = 'gallery_duplicate'
                      AND NOT EXISTS (
                        SELECT 1
                        FROM download_overlap_reviews overlap_review
                        WHERE overlap_review.review_id = download_entries.review_id
                          AND overlap_review.entry_id = download_entries.entry_id
                          AND overlap_review.state = 'pending'
                      )
                    )
                  )
                  AND EXISTS (
                    SELECT 1
                    FROM download_jobs j
                    WHERE j.entry_id = download_entries.entry_id
                      AND j.state = 'failed'
                  )
            "#,
            [],
        )
        .map_err(map_sqlite_error)?;
    transaction
        .execute(
            r#"
                UPDATE download_attempts
                SET finished_at = COALESCE(
                        finished_at,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    ),
                    outcome_state = 'interrupted',
                    error_code = 'JOB_INTERRUPTED',
                    error_message =
                        'The application stopped before the job reached a terminal state'
                WHERE EXISTS (
                    SELECT 1
                    FROM download_jobs j
                    WHERE j.job_id = download_attempts.job_id
                      AND j.attempt = download_attempts.attempt
                      AND j.state IN (
                          'queued', 'resolving_metadata', 'downloading',
                          'hashing', 'verifying', 'retry_wait'
                      )
                )
            "#,
            [],
        )
        .map_err(map_sqlite_error)?;
    transaction
        .execute(
            r#"
                UPDATE download_jobs
                SET revision = revision + 1,
                    state = 'interrupted',
                    last_error_code = 'JOB_INTERRUPTED',
                    last_error_message =
                        'The application stopped before the job reached a terminal state',
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                    finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                WHERE state IN (
                    'queued', 'resolving_metadata', 'downloading', 'hashing',
                    'verifying', 'retry_wait'
                )
            "#,
            [],
        )
        .map_err(map_sqlite_error)?;
    let recovered_entries = transaction
        .execute(
            r#"
                UPDATE download_entries
                SET revision = revision + 1,
                    state = 'interrupted',
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                WHERE state IN (
                    'queued', 'resolving_metadata', 'downloading', 'hashing',
                    'verifying', 'retry_wait'
                )
            "#,
            [],
        )
        .map_err(map_sqlite_error)?;
    transaction.commit().map_err(map_sqlite_error)?;
    repaired_invalid_reviews
        .checked_add(recovered_entries)
        .ok_or_else(|| RepositoryError::Other("download recovery count overflowed".into()))
}

fn read_request_entries(
    connection: &Connection,
    request_id: &str,
) -> Result<Vec<DownloadEntry>, RepositoryError> {
    let mut statement = connection
        .prepare(
            r#"
                SELECT
                    request_entry.entry_id,
                    request_entry.gallery_id,
                    request_entry.response_revision,
                    request_entry.response_state,
                    request_entry.response_progress,
                    request_entry.response_review_kind,
                    request_entry.response_review_id,
                    NULL AS attempt,
                    NULL AS error_code,
                    NULL AS error_message,
                    NULL AS error_retryable,
                    download.created_at,
                    download.updated_at
                FROM download_queue_request_entries request_entry
                LEFT JOIN download_entries download
                  ON download.entry_id = request_entry.entry_id
                WHERE request_entry.request_id = ?1
                ORDER BY request_entry.position ASC
            "#,
        )
        .map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([request_id], stored_download_entry)
        .map_err(map_sqlite_error)?;
    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.map_err(map_sqlite_error)?.try_into_domain()?);
    }
    Ok(entries)
}

struct StoredDownloadEntry {
    entry_id: String,
    gallery_id: i64,
    revision: i64,
    state: String,
    progress: f64,
    review_kind: Option<String>,
    review_id: Option<String>,
    attempt: Option<i64>,
    error_code: Option<String>,
    error_message: Option<String>,
    error_retryable: Option<i64>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

impl StoredDownloadEntry {
    fn try_into_domain(self) -> Result<DownloadEntry, RepositoryError> {
        let state = self.state.parse::<JobState>().map_err(domain_corruption)?;
        let review_kind = parse_download_review_kind(self.review_kind)?;
        if state == JobState::ReviewRequired
            && (review_kind.is_none() || self.review_id.as_deref().is_none_or(str::is_empty))
        {
            return Err(RepositoryError::Corrupt(
                "review_required download entry is missing its review target".into(),
            ));
        }

        Ok(DownloadEntry {
            entry_id: DownloadEntryId::new(self.entry_id).map_err(domain_corruption)?,
            gallery_id: GalleryId::new(self.gallery_id).map_err(domain_corruption)?,
            revision: stored_u64(self.revision, "download revision")?,
            state,
            progress: Some(self.progress),
            attempt: self
                .attempt
                .map(|attempt| stored_u64(attempt, "download attempt"))
                .transpose()?,
            error_code: self.error_code,
            error_message: self.error_message,
            error_retryable: self.error_retryable.map(|value| value != 0),
            review_kind,
            review_id: self.review_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

fn parse_download_review_kind(
    review_kind: Option<String>,
) -> Result<Option<DownloadReviewKind>, RepositoryError> {
    review_kind
        .map(|kind| match kind.as_str() {
            "gallery_duplicate" => Ok(DownloadReviewKind::GalleryDuplicate),
            "internal_pages" => Ok(DownloadReviewKind::InternalPages),
            _ => Err(RepositoryError::Corrupt(format!(
                "download review kind {kind:?} is unsupported"
            ))),
        })
        .transpose()
}

fn stored_download_entry(row: &Row<'_>) -> rusqlite::Result<StoredDownloadEntry> {
    Ok(StoredDownloadEntry {
        entry_id: row.get(0)?,
        gallery_id: row.get(1)?,
        revision: row.get(2)?,
        state: row.get(3)?,
        progress: row.get(4)?,
        review_kind: row.get(5)?,
        review_id: row.get(6)?,
        attempt: row.get(7)?,
        error_code: row.get(8)?,
        error_message: row.get(9)?,
        error_retryable: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn canonical_overlap_fingerprints<'a>(
    left: &'a str,
    right: &'a str,
) -> Result<(&'a str, &'a str), RepositoryError> {
    if left.len() != 64 || right.len() != 64 {
        return Err(RepositoryError::Other(
            "download overlap fingerprint must contain 64 hexadecimal characters".into(),
        ));
    }
    Ok(if left <= right {
        (left, right)
    } else {
        (right, left)
    })
}

fn read_pending_overlap_candidates(
    transaction: &Transaction<'_>,
    review_id: &str,
) -> Result<Vec<(String, String)>, RepositoryError> {
    let mut statement = transaction
        .prepare(
            r#"
                SELECT candidate_id, existing_fingerprint
                FROM download_overlap_candidates
                WHERE review_id = ?1 AND decision IS NULL
                ORDER BY rank ASC, candidate_id ASC
            "#,
        )
        .map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([review_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(map_sqlite_error)?;
    rows.map(|row| row.map_err(map_sqlite_error)).collect()
}

fn insert_overlap_pair_policy(
    transaction: &Transaction<'_>,
    incoming_fingerprint: &str,
    existing_fingerprint: &str,
    profile_version: u32,
    policy_version: u32,
    decision: DownloadOverlapPairDecision,
) -> Result<(), RepositoryError> {
    let (left, right) = canonical_overlap_fingerprints(incoming_fingerprint, existing_fingerprint)?;
    transaction
        .execute(
            r#"
                INSERT INTO download_overlap_pair_policies (
                    left_fingerprint, right_fingerprint,
                    profile_version, policy_version, decision, created_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5,
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                )
                ON CONFLICT (
                    left_fingerprint, right_fingerprint,
                    profile_version, policy_version
                ) DO UPDATE SET decision=excluded.decision, created_at=excluded.created_at
            "#,
            params![
                left,
                right,
                i64::from(profile_version),
                i64::from(policy_version),
                decision.as_str(),
            ],
        )
        .map_err(map_sqlite_error)?;
    Ok(())
}

fn finish_overlap_review(
    transaction: &Transaction<'_>,
    review_id: &str,
    state: &str,
) -> Result<(), RepositoryError> {
    if !matches!(state, "resolved" | "cancelled" | "stale") {
        return Err(RepositoryError::Other(
            "download overlap review has an unsupported terminal state".into(),
        ));
    }
    let changed = transaction
        .execute(
            r#"
                UPDATE download_overlap_reviews
                SET revision = revision + 1, state = ?1,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                    resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                WHERE review_id = ?2 AND state = 'pending'
            "#,
            params![state, review_id],
        )
        .map_err(map_sqlite_error)?;
    if changed != 1 {
        return Err(RepositoryError::Other(
            "download overlap review changed concurrently".into(),
        ));
    }
    Ok(())
}

fn cancel_chained_overlap_target(
    transaction: &Transaction<'_>,
    target: StoredDownloadTarget,
    current_review_id: &str,
) -> Result<Option<DownloadJobProjection>, RepositoryError> {
    if target.review_kind.as_deref() != Some("gallery_duplicate") {
        return Ok(None);
    }
    let Some(review_id) = target.review_id.as_deref() else {
        return Ok(None);
    };
    if review_id == current_review_id {
        return Ok(None);
    }
    if !incomplete_overlap_artifact_exists(transaction, &target.entry_id)? {
        return Ok(None);
    }
    let review_target = transaction
        .query_row(
            "SELECT entry_id, state FROM download_overlap_reviews WHERE review_id=?1",
            [review_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(map_sqlite_error)?;
    if review_target
        .as_ref()
        .is_none_or(|(entry_id, state)| entry_id != target.entry_id.as_str() || state != "pending")
    {
        return Ok(None);
    }

    finish_overlap_review(transaction, review_id, "cancelled")?;
    let descriptor = DownloadJobDescriptor {
        job_id: target.job_id.clone(),
        entry_id: target.entry_id.to_string(),
        gallery_id: GalleryId::new(target.gallery_id).map_err(domain_corruption)?,
        worker_attempt: stored_u64(target.attempt, "download attempt")?,
    };
    let pipeline_target = read_pipeline_target(transaction, &descriptor)?;
    let projection = transition_pipeline_target(
        transaction,
        pipeline_target,
        JobState::Cancelled,
        None,
        None,
        None,
        None,
        "Chained overlap staging was cancelled after removal from another review",
    )?;
    transaction
        .execute(
            "UPDATE download_attempts SET finished_at=COALESCE(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), outcome_state='cancelled' WHERE job_id=?1 AND attempt=?2",
            params![
                descriptor.job_id,
                to_sql_integer(descriptor.worker_attempt, "download attempt")?
            ],
        )
        .map_err(map_sqlite_error)?;
    Ok(Some(projection))
}

fn cancel_failed_overlap_target(
    transaction: &Transaction<'_>,
    target: StoredDownloadTarget,
) -> Result<Option<DownloadJobProjection>, RepositoryError> {
    if !matches!(target.state, JobState::Failed | JobState::Interrupted)
        || !incomplete_overlap_artifact_exists(transaction, &target.entry_id)?
    {
        return Ok(None);
    }
    let descriptor = DownloadJobDescriptor {
        job_id: target.job_id.clone(),
        entry_id: target.entry_id.to_string(),
        gallery_id: GalleryId::new(target.gallery_id).map_err(domain_corruption)?,
        worker_attempt: stored_u64(target.attempt, "download attempt")?,
    };
    let pipeline_target = read_pipeline_target(transaction, &descriptor)?;
    let error_code = pipeline_target.error_code.clone();
    let error_message = pipeline_target.error_message.clone();
    let projection = transition_pipeline_target(
        transaction,
        pipeline_target,
        JobState::Cancelled,
        None,
        None,
        error_code.as_deref(),
        error_message.as_deref(),
        "Failed overlap staging was cancelled after removal from another review",
    )?;
    transaction
        .execute(
            r#"
                UPDATE download_attempts
                SET finished_at = COALESCE(
                        finished_at,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    ),
                    outcome_state = 'cancelled'
                WHERE job_id = ?1 AND attempt = ?2 AND finished_at IS NULL
            "#,
            params![
                descriptor.job_id,
                to_sql_integer(descriptor.worker_attempt, "download attempt")?
            ],
        )
        .map_err(map_sqlite_error)?;
    Ok(Some(projection))
}

fn incomplete_overlap_artifact_exists(
    transaction: &Transaction<'_>,
    entry_id: &DownloadEntryId,
) -> Result<bool, RepositoryError> {
    let artifact_state = transaction
        .query_row(
            "SELECT state FROM download_artifacts WHERE entry_id=?1",
            [entry_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(map_sqlite_error)?;
    Ok(artifact_state.as_deref() == Some("incomplete"))
}

fn bump_pending_overlap_review(
    transaction: &Transaction<'_>,
    review_id: &str,
) -> Result<(), RepositoryError> {
    let changed = transaction
        .execute(
            r#"
                UPDATE download_overlap_reviews
                SET revision = revision + 1,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                WHERE review_id = ?1 AND state = 'pending'
            "#,
            [review_id],
        )
        .map_err(map_sqlite_error)?;
    if changed != 1 {
        return Err(RepositoryError::Other(
            "download overlap review changed concurrently".into(),
        ));
    }
    Ok(())
}

fn requeue_overlap_target(
    transaction: &Transaction<'_>,
    entry_id: &str,
) -> Result<(DownloadJobProjection, DownloadJobDescriptor), RepositoryError> {
    let entry_id = DownloadEntryId::new(entry_id.to_owned()).map_err(domain_corruption)?;
    let target = read_download_target(transaction, &entry_id)?
        .ok_or_else(|| RepositoryError::Corrupt("overlap download target disappeared".into()))?;
    if target.state != JobState::ReviewRequired {
        return Err(RepositoryError::Other(format!(
            "download overlap target cannot resume from {}",
            target.state
        )));
    }
    let job_revision = next_stored_revision(target.job_revision, "job revision")?;
    let entry_revision = next_stored_revision(target.entry_revision, "download revision")?;
    let attempt = next_stored_revision(target.attempt, "download attempt")?;
    let changed_jobs = transaction
        .execute(
            r#"
                UPDATE download_jobs
                SET revision=?1, state='queued', attempt=?2,
                    completed_units=0, total_units=1,
                    last_error_code=NULL, last_error_message=NULL,
                    last_error_retryable=NULL,
                    updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                    started_at=NULL, finished_at=NULL
                WHERE job_id=?3 AND revision=?4 AND state='review_required'
            "#,
            params![job_revision, attempt, target.job_id, target.job_revision],
        )
        .map_err(map_sqlite_error)?;
    let changed_entries = transaction
        .execute(
            r#"
                UPDATE download_entries
                SET revision=?1, state='queued', progress=0,
                    review_kind=NULL, review_id=NULL,
                    updated_at=strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                WHERE entry_id=?2 AND revision=?3 AND state='review_required'
            "#,
            params![
                entry_revision,
                target.entry_id.as_str(),
                target.entry_revision
            ],
        )
        .map_err(map_sqlite_error)?;
    if changed_jobs != 1 || changed_entries != 1 {
        return Err(RepositoryError::Other(
            "download overlap target changed while resuming".into(),
        ));
    }
    transaction
        .execute(
            r#"
                INSERT INTO download_attempts (job_id, attempt, started_at)
                VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            "#,
            params![target.job_id, attempt],
        )
        .map_err(map_sqlite_error)?;
    let descriptor = DownloadJobDescriptor {
        job_id: target.job_id,
        entry_id: target.entry_id.to_string(),
        gallery_id: GalleryId::new(target.gallery_id).map_err(domain_corruption)?,
        worker_attempt: attempt,
    };
    let projection = read_pipeline_target(transaction, &descriptor)?.into_projection(Some(
        "Overlap review completed; resuming verified staging pages",
    ))?;
    Ok((projection, descriptor))
}

fn read_download_overlap_review(
    connection: &Connection,
    review_id: &str,
) -> Result<Option<DownloadOverlapReview>, RepositoryError> {
    let stored = connection
        .query_row(
            r#"
                SELECT r.review_id, r.entry_id, r.incoming_gallery_id,
                       r.revision, r.state, r.profile_version, r.policy_version,
                       r.incoming_fingerprint, r.created_at, r.updated_at, r.resolved_at,
                       gallery.title, artifact.expected_page_count
                FROM download_overlap_reviews r
                JOIN galleries gallery ON gallery.gallery_id = r.incoming_gallery_id
                JOIN download_artifacts artifact
                  ON artifact.entry_id = r.entry_id
                 AND artifact.gallery_id = r.incoming_gallery_id
                WHERE r.review_id = ?1
            "#,
            [review_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, i64>(12)?,
                ))
            },
        )
        .optional()
        .map_err(map_sqlite_error)?;
    let Some((
        review_id,
        entry_id,
        incoming_gallery_id,
        revision,
        state,
        profile_version,
        policy_version,
        incoming_fingerprint,
        created_at,
        updated_at,
        resolved_at,
        incoming_title,
        incoming_page_count,
    )) = stored
    else {
        return Ok(None);
    };
    let incoming_gallery_id = GalleryId::new(incoming_gallery_id).map_err(domain_corruption)?;
    let incoming = DownloadOverlapGalleryRef {
        entry_id: entry_id.clone(),
        gallery_id: incoming_gallery_id,
        title: incoming_title,
        artists: read_owned_gallery_artists(connection, incoming_gallery_id)?,
        page_count: stored_u32(incoming_page_count, "overlap incoming page count")?,
    };
    let mut statement = connection
        .prepare(
            r#"
                SELECT candidate.candidate_id, candidate.existing_entry_id,
                       candidate.existing_gallery_id, candidate.existing_fingerprint,
                       candidate.relation, candidate.confidence,
                       candidate.matched_pages, candidate.exact_pages,
                       candidate.visual_pages, candidate.existing_coverage,
                       candidate.incoming_coverage, candidate.existing_unique_pages,
                       candidate.incoming_unique_pages, candidate.longest_aligned_run,
                       candidate.rank, candidate.decision,
                       gallery.title, artifact.expected_page_count
                FROM download_overlap_candidates candidate
                JOIN galleries gallery
                  ON gallery.gallery_id = candidate.existing_gallery_id
                JOIN download_artifacts artifact
                  ON artifact.entry_id = candidate.existing_entry_id
                 AND artifact.gallery_id = candidate.existing_gallery_id
                WHERE candidate.review_id = ?1
                ORDER BY candidate.rank ASC, candidate.candidate_id ASC
            "#,
        )
        .map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([review_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, f64>(9)?,
                row.get::<_, f64>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, i64>(13)?,
                row.get::<_, i64>(14)?,
                row.get::<_, Option<String>>(15)?,
                row.get::<_, String>(16)?,
                row.get::<_, i64>(17)?,
            ))
        })
        .map_err(map_sqlite_error)?;
    let mut stored_candidates = Vec::new();
    for row in rows {
        stored_candidates.push(row.map_err(map_sqlite_error)?);
    }
    drop(statement);
    let mut candidates = Vec::with_capacity(stored_candidates.len());
    for candidate in stored_candidates {
        let existing_gallery_id = GalleryId::new(candidate.2).map_err(domain_corruption)?;
        let mut pair_statement = connection
            .prepare(
                r#"
                    SELECT incoming_source_page, existing_source_page, exact_sha256,
                           d_hash_distance, p_hash_distance, detail_hash_distance,
                           edge_similarity, visual_similarity, low_information
                    FROM download_overlap_page_pairs
                    WHERE candidate_id = ?1 ORDER BY pair_index ASC
                "#,
            )
            .map_err(map_sqlite_error)?;
        let pair_rows = pair_statement
            .query_map([candidate.0.as_str()], |row| {
                Ok(DownloadOverlapPagePair {
                    incoming_source_page: u32::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
                    existing_source_page: u32::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                    exact_sha256: row.get::<_, bool>(2)?,
                    d_hash_distance: u32::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                    p_hash_distance: u32::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                    detail_hash_distance: u32::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
                    edge_similarity: row.get(6)?,
                    visual_similarity: row.get(7)?,
                    low_information: row.get(8)?,
                })
            })
            .map_err(map_sqlite_error)?;
        let mut page_pairs = Vec::new();
        for pair in pair_rows {
            let pair = pair.map_err(map_sqlite_error)?;
            if pair.incoming_source_page == 0 || pair.existing_source_page == 0 {
                return Err(RepositoryError::Corrupt(
                    "download overlap page pair contains a zero source page".into(),
                ));
            }
            page_pairs.push(pair);
        }
        candidates.push(DownloadOverlapCandidate {
            candidate_id: candidate.0,
            existing: DownloadOverlapGalleryRef {
                entry_id: candidate.1,
                gallery_id: existing_gallery_id,
                title: candidate.16,
                artists: read_owned_gallery_artists(connection, existing_gallery_id)?,
                page_count: stored_u32(candidate.17, "overlap existing page count")?,
            },
            existing_fingerprint: candidate.3,
            relation: DownloadOverlapRelation::from_database(&candidate.4).ok_or_else(|| {
                RepositoryError::Corrupt(format!(
                    "download overlap relation {:?} is unsupported",
                    candidate.4
                ))
            })?,
            confidence: candidate.5,
            matched_pages: stored_u32(candidate.6, "overlap matched pages")?,
            exact_pages: stored_u32(candidate.7, "overlap exact pages")?,
            visual_pages: stored_u32(candidate.8, "overlap visual pages")?,
            existing_coverage: candidate.9,
            incoming_coverage: candidate.10,
            existing_unique_pages: stored_u32(candidate.11, "overlap existing unique pages")?,
            incoming_unique_pages: stored_u32(candidate.12, "overlap incoming unique pages")?,
            longest_aligned_run: stored_u32(candidate.13, "overlap aligned run")?,
            rank: stored_u32(candidate.14, "overlap candidate rank")?,
            decision: candidate
                .15
                .map(|decision| {
                    DownloadOverlapPairDecision::from_database(&decision).ok_or_else(|| {
                        RepositoryError::Corrupt(format!(
                            "download overlap decision {decision:?} is unsupported"
                        ))
                    })
                })
                .transpose()?,
            page_pairs,
        });
    }
    Ok(Some(DownloadOverlapReview {
        review_id,
        entry_id,
        incoming,
        revision: stored_u64(revision, "download overlap review revision")?,
        state: DownloadOverlapReviewState::from_database(&state).ok_or_else(|| {
            RepositoryError::Corrupt(format!(
                "download overlap review state {state:?} is unsupported"
            ))
        })?,
        profile_version: stored_u32(profile_version, "download overlap profile version")?,
        policy_version: stored_u32(policy_version, "download overlap policy version")?,
        incoming_fingerprint,
        candidates,
        created_at,
        updated_at,
        resolved_at,
    }))
}

struct StoredArtifactBundle {
    gallery_id: i64,
    gallery_revision: i64,
    title: String,
    primary_artist: Option<String>,
    primary_group: Option<String>,
    source_page_count: i64,
    artifact_revision: i64,
    relative_directory: String,
    expected_page_count: i64,
    artifact_state: String,
    manifest_relative_path: Option<String>,
    manifest_schema_version: Option<i64>,
    writer_version: Option<String>,
    hash_profile_version: i64,
    completed_at: Option<String>,
}

fn stored_artifact_bundle(row: &Row<'_>) -> rusqlite::Result<StoredArtifactBundle> {
    Ok(StoredArtifactBundle {
        gallery_id: row.get(0)?,
        gallery_revision: row.get(1)?,
        title: row.get(2)?,
        primary_artist: row.get(3)?,
        primary_group: row.get(4)?,
        source_page_count: row.get(5)?,
        artifact_revision: row.get(6)?,
        relative_directory: row.get(7)?,
        expected_page_count: row.get(8)?,
        artifact_state: row.get(9)?,
        manifest_relative_path: row.get(10)?,
        manifest_schema_version: row.get(11)?,
        writer_version: row.get(12)?,
        hash_profile_version: row.get(13)?,
        completed_at: row.get(14)?,
    })
}

struct StoredPageArtifact {
    gallery_id: i64,
    source_page_number: i64,
    relative_path: String,
    page_state: String,
    byte_length: Option<i64>,
    sha256: Option<String>,
    storage_format: Option<String>,
    source_revision: Option<String>,
    verified_at: Option<String>,
    excluded: bool,
}

fn stored_page_artifact(row: &Row<'_>) -> rusqlite::Result<StoredPageArtifact> {
    Ok(StoredPageArtifact {
        gallery_id: row.get(0)?,
        source_page_number: row.get(1)?,
        relative_path: row.get(2)?,
        page_state: row.get(3)?,
        byte_length: row.get(4)?,
        sha256: row.get(5)?,
        storage_format: row.get(6)?,
        source_revision: row.get(7)?,
        verified_at: row.get(8)?,
        excluded: row.get(9)?,
    })
}

fn read_tag_catalog_status(connection: &Connection) -> Result<TagCatalogStatus, RepositoryError> {
    connection.query_row(
        "SELECT revision, entry_count, neutral_count, female_count, male_count, artist_count, group_count, last_attempt_at, last_success_at, last_error_code, last_error_message FROM tag_catalog_state WHERE singleton = 1",
        [],
        |row| Ok(TagCatalogStatus {
            revision: row.get::<_, i64>(0)? as u64,
            entry_count: row.get::<_, i64>(1)? as u64,
            neutral_count: row.get::<_, i64>(2)? as u64,
            female_count: row.get::<_, i64>(3)? as u64,
            male_count: row.get::<_, i64>(4)? as u64,
            artist_count: row.get::<_, i64>(5)? as u64,
            group_count: row.get::<_, i64>(6)? as u64,
            last_attempt_at: row.get(7)?, last_success_at: row.get(8)?, last_error_code: row.get(9)?, last_error_message: row.get(10)?,
        }),
    ).map_err(map_sqlite_error)
}

fn read_favorites(connection: &Connection) -> Result<Vec<FavoriteRecord>, RepositoryError> {
    let mut statement = connection
        .prepare(
            r#"
                SELECT namespace, value, revision, created_at, updated_at
                FROM favorites
                ORDER BY namespace ASC, value COLLATE NOCASE ASC
            "#,
        )
        .map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([], stored_favorite)
        .map_err(map_sqlite_error)?;
    rows.map(|row| row.map_err(map_sqlite_error)?.try_into_domain())
        .collect()
}

fn read_favorite(
    connection: &Connection,
    key: &FavoriteKey,
) -> Result<Option<FavoriteRecord>, RepositoryError> {
    connection
        .query_row(
            r#"
                SELECT namespace, value, revision, created_at, updated_at
                FROM favorites
                WHERE namespace = ?1 AND value = ?2
            "#,
            params![key.namespace.as_str(), key.value],
            stored_favorite,
        )
        .optional()
        .map_err(map_sqlite_error)?
        .map(StoredFavorite::try_into_domain)
        .transpose()
}

struct StoredFavorite {
    namespace: String,
    value: String,
    revision: i64,
    created_at: String,
    updated_at: String,
}

impl StoredFavorite {
    fn try_into_domain(self) -> Result<FavoriteRecord, RepositoryError> {
        Ok(FavoriteRecord {
            namespace: FavoriteNamespace::from_database(&self.namespace).ok_or_else(|| {
                RepositoryError::Corrupt(format!(
                    "favorite namespace {:?} is unsupported",
                    self.namespace
                ))
            })?,
            value: self.value,
            revision: stored_u64(self.revision, "favorite revision")?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

fn stored_favorite(row: &Row<'_>) -> rusqlite::Result<StoredFavorite> {
    Ok(StoredFavorite {
        namespace: row.get(0)?,
        value: row.get(1)?,
        revision: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn read_search_history_by_fingerprint(
    connection: &Connection,
    fingerprint: &str,
) -> Result<Option<SearchHistoryEntry>, RepositoryError> {
    connection
        .query_row(
            r#"
                SELECT history_id, text, include_tags_json, exclude_tags_json,
                       languages_json, sort, page_size, use_count, last_used_at
                FROM search_history
                WHERE fingerprint = ?1
            "#,
            [fingerprint],
            stored_search_history,
        )
        .optional()
        .map_err(map_sqlite_error)?
        .map(StoredSearchHistory::try_into_domain)
        .transpose()
}

struct StoredSearchHistory {
    history_id: i64,
    text: String,
    include_tags_json: String,
    exclude_tags_json: String,
    languages_json: String,
    sort: String,
    page_size: i64,
    use_count: i64,
    last_used_at: String,
}

impl StoredSearchHistory {
    fn try_into_domain(self) -> Result<SearchHistoryEntry, RepositoryError> {
        Ok(SearchHistoryEntry {
            history_id: self.history_id,
            text: self.text,
            include_tags: serde_json::from_str(&self.include_tags_json)
                .map_err(domain_corruption)?,
            exclude_tags: serde_json::from_str(&self.exclude_tags_json)
                .map_err(domain_corruption)?,
            languages: serde_json::from_str(&self.languages_json).map_err(domain_corruption)?,
            sort: parse_search_sort(&self.sort)?,
            page_size: stored_u32(self.page_size, "search history page size")?,
            use_count: stored_u64(self.use_count, "search history use count")?,
            last_used_at: self.last_used_at,
        })
    }
}

fn stored_search_history(row: &Row<'_>) -> rusqlite::Result<StoredSearchHistory> {
    Ok(StoredSearchHistory {
        history_id: row.get(0)?,
        text: row.get(1)?,
        include_tags_json: row.get(2)?,
        exclude_tags_json: row.get(3)?,
        languages_json: row.get(4)?,
        sort: row.get(5)?,
        page_size: row.get(6)?,
        use_count: row.get(7)?,
        last_used_at: row.get(8)?,
    })
}

fn search_sort_text(sort: SearchSort) -> &'static str {
    match sort {
        SearchSort::Recent => "recent",
        SearchSort::PopularToday => "popular_today",
        SearchSort::PopularWeek => "popular_week",
        SearchSort::PopularMonth => "popular_month",
        SearchSort::PopularYear => "popular_year",
        SearchSort::Random => "random",
    }
}

fn parse_search_sort(value: &str) -> Result<SearchSort, RepositoryError> {
    match value {
        "recent" => Ok(SearchSort::Recent),
        "popular_today" => Ok(SearchSort::PopularToday),
        "popular_week" => Ok(SearchSort::PopularWeek),
        "popular_month" => Ok(SearchSort::PopularMonth),
        "popular_year" => Ok(SearchSort::PopularYear),
        "random" => Ok(SearchSort::Random),
        _ => Err(RepositoryError::Corrupt(format!(
            "search sort {value:?} is unsupported"
        ))),
    }
}

fn language_text(language: Language) -> &'static str {
    match language {
        Language::Korean => "korean",
        Language::Japanese => "japanese",
        Language::Chinese => "chinese",
        Language::English => "english",
    }
}

fn parse_language(value: &str) -> Result<Language, RepositoryError> {
    match value {
        "korean" => Ok(Language::Korean),
        "japanese" => Ok(Language::Japanese),
        "chinese" => Ok(Language::Chinese),
        "english" => Ok(Language::English),
        _ => Err(RepositoryError::Corrupt(format!(
            "gallery language {value:?} is unsupported"
        ))),
    }
}

fn auto_find_run_is_running(
    connection: &Connection,
    run_id: &str,
) -> Result<bool, RepositoryError> {
    connection
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM auto_find_runs WHERE run_id = ?1 AND state = 'running')",
            [run_id],
            |row| row.get(0),
        )
        .map_err(map_sqlite_error)
}

fn read_running_auto_find(connection: &Connection) -> Result<Option<AutoFindRun>, RepositoryError> {
    connection
        .query_row(
            r#"
                SELECT run_id, revision, state, total_favorites,
                       completed_favorites, candidates_found, history_mode,
                       started_at, updated_at, finished_at,
                       error_code, error_message
                FROM auto_find_runs
                WHERE state = 'running'
                LIMIT 1
            "#,
            [],
            stored_auto_find_run,
        )
        .optional()
        .map_err(map_sqlite_error)?
        .map(StoredAutoFindRun::try_into_domain)
        .transpose()
}

fn read_auto_find_run(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<AutoFindRun>, RepositoryError> {
    connection
        .query_row(
            r#"
                SELECT run_id, revision, state, total_favorites,
                       completed_favorites, candidates_found, history_mode,
                       started_at, updated_at, finished_at,
                       error_code, error_message
                FROM auto_find_runs
                WHERE run_id = ?1
            "#,
            [run_id],
            stored_auto_find_run,
        )
        .optional()
        .map_err(map_sqlite_error)?
        .map(StoredAutoFindRun::try_into_domain)
        .transpose()
}

struct StoredAutoFindRun {
    run_id: String,
    revision: i64,
    state: String,
    total_favorites: i64,
    completed_favorites: i64,
    candidates_found: i64,
    history_mode: String,
    started_at: String,
    updated_at: String,
    finished_at: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
}

impl StoredAutoFindRun {
    fn try_into_domain(self) -> Result<AutoFindRun, RepositoryError> {
        Ok(AutoFindRun {
            run_id: self.run_id,
            revision: stored_u64(self.revision, "Auto Find revision")?,
            state: AutoFindRunState::from_database(&self.state).ok_or_else(|| {
                RepositoryError::Corrupt(format!("Auto Find state {:?} is unsupported", self.state))
            })?,
            total_favorites: stored_u32(self.total_favorites, "Auto Find favorite count")?,
            completed_favorites: stored_u32(
                self.completed_favorites,
                "Auto Find completed favorite count",
            )?,
            candidates_found: stored_u32(self.candidates_found, "Auto Find candidate count")?,
            history_mode: AutoFindHistoryMode::from_database(&self.history_mode).ok_or_else(
                || {
                    RepositoryError::Corrupt(format!(
                        "Auto Find history mode {:?} is unsupported",
                        self.history_mode
                    ))
                },
            )?,
            started_at: self.started_at,
            updated_at: self.updated_at,
            finished_at: self.finished_at,
            error_code: self.error_code,
            error_message: self.error_message,
        })
    }
}

fn stored_auto_find_run(row: &Row<'_>) -> rusqlite::Result<StoredAutoFindRun> {
    Ok(StoredAutoFindRun {
        run_id: row.get(0)?,
        revision: row.get(1)?,
        state: row.get(2)?,
        total_favorites: row.get(3)?,
        completed_favorites: row.get(4)?,
        candidates_found: row.get(5)?,
        history_mode: row.get(6)?,
        started_at: row.get(7)?,
        updated_at: row.get(8)?,
        finished_at: row.get(9)?,
        error_code: row.get(10)?,
        error_message: row.get(11)?,
    })
}

fn read_exploration_exclusions(
    connection: &Connection,
) -> Result<Vec<ExplorationExclusion>, RepositoryError> {
    let mut statement = connection
        .prepare(
            r#"
                WITH active_reasons(gallery_id, kind, detail, excluded_at) AS (
                    SELECT gallery_id, 'manual', reason, created_at
                    FROM auto_find_exclusions
                    UNION ALL
                    SELECT hidden.gallery_id, 'duplicate_hidden',
                           '중복 판정에서 숨김', hidden.created_at
                    FROM duplicate_hidden_galleries hidden
                    WHERE NOT EXISTS (
                        SELECT 1 FROM exploration_restored_galleries restored
                        WHERE restored.gallery_id = hidden.gallery_id
                    )
                )
                SELECT reason.gallery_id,
                       COALESCE(
                           (SELECT gallery.title FROM galleries gallery
                            WHERE gallery.gallery_id = reason.gallery_id),
                           (SELECT candidate.title FROM auto_find_candidates candidate
                            WHERE candidate.gallery_id = reason.gallery_id
                            ORDER BY candidate.discovered_at DESC, candidate.run_id DESC LIMIT 1),
                           'Gallery #' || reason.gallery_id
                       ) AS title,
                       COALESCE(
                           NULLIF((SELECT gallery.primary_artist FROM galleries gallery
                                   WHERE gallery.gallery_id = reason.gallery_id), ''),
                           NULLIF((SELECT candidate.artist FROM auto_find_candidates candidate
                                   WHERE candidate.gallery_id = reason.gallery_id
                                   ORDER BY candidate.discovered_at DESC, candidate.run_id DESC LIMIT 1), ''),
                           '알 수 없는 작가'
                       ) AS artist,
                       reason.kind, reason.detail, reason.excluded_at
                FROM active_reasons reason
                ORDER BY reason.gallery_id DESC, reason.excluded_at DESC, reason.kind ASC
            "#,
        )
        .map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(map_sqlite_error)?;
    let mut grouped = BTreeMap::<i64, (String, String, Vec<ExplorationExclusionReason>)>::new();
    for row in rows {
        let (gallery_id, title, artist, kind, detail, excluded_at) =
            row.map_err(map_sqlite_error)?;
        let kind = match kind.as_str() {
            "manual" => ExplorationExclusionKind::Manual,
            "duplicate_hidden" => ExplorationExclusionKind::DuplicateHidden,
            "duplicate_resolved" => ExplorationExclusionKind::DuplicateResolved,
            "duplicate_pair" => ExplorationExclusionKind::DuplicatePair,
            _ => {
                return Err(RepositoryError::Corrupt(format!(
                    "exploration exclusion kind {kind:?} is unsupported"
                )))
            }
        };
        grouped
            .entry(gallery_id)
            .or_insert_with(|| (title, artist, Vec::new()))
            .2
            .push(ExplorationExclusionReason {
                kind,
                detail,
                excluded_at,
            });
    }
    grouped
        .into_iter()
        .rev()
        .map(|(gallery_id, (title, artist, reasons))| {
            Ok(ExplorationExclusion {
                gallery_id: GalleryId::new(gallery_id).map_err(domain_corruption)?,
                title,
                artist,
                reasons,
            })
        })
        .collect()
}

fn read_auto_find_snapshot(connection: &Connection) -> Result<AutoFindSnapshot, RepositoryError> {
    let run = connection
        .query_row(
            r#"
                SELECT run_id, revision, state, total_favorites,
                       completed_favorites, candidates_found, history_mode,
                       started_at, updated_at, finished_at,
                       error_code, error_message
                FROM auto_find_runs
                ORDER BY started_at DESC, run_id DESC
                LIMIT 1
            "#,
            [],
            stored_auto_find_run,
        )
        .optional()
        .map_err(map_sqlite_error)?
        .map(StoredAutoFindRun::try_into_domain)
        .transpose()?;
    let Some(run) = run else {
        return Ok(AutoFindSnapshot {
            run: None,
            candidates: Vec::new(),
            cutoff_evidence: Vec::new(),
            truncations: Vec::new(),
        });
    };
    let mut statement = connection
        .prepare(
            r#"
                SELECT run_id, gallery_id, title, artist, group_name, pages,
                       language, tags_json, series_json, characters_json,
                       published_rank, popularity,
                       thumbnail_key, thumbnail_width, thumbnail_height,
                       favorite_namespace, favorite_value, discovered_at
                FROM auto_find_candidates candidate
                WHERE run_id = ?1
                  AND NOT EXISTS (
                      SELECT 1 FROM auto_find_exclusions exclusion
                      WHERE exclusion.gallery_id = candidate.gallery_id
                  )
                  AND NOT EXISTS (
                      SELECT 1 FROM download_entries download
                      WHERE download.gallery_id = candidate.gallery_id
                  )
                  AND NOT EXISTS (
                      SELECT 1 FROM duplicate_hidden_galleries hidden
                      WHERE hidden.gallery_id = candidate.gallery_id
                        AND NOT EXISTS (
                            SELECT 1 FROM exploration_restored_galleries restored
                            WHERE restored.gallery_id = candidate.gallery_id
                        )
                  )
                ORDER BY published_rank DESC, gallery_id DESC
            "#,
        )
        .map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([run.run_id.as_str()], stored_auto_find_candidate)
        .map_err(map_sqlite_error)?;
    let candidates = rows
        .map(|row| row.map_err(map_sqlite_error)?.try_into_domain())
        .collect::<Result<Vec<_>, _>>()?;
    let cutoff_evidence = read_auto_find_cutoff_evidence(connection, &run.run_id)?;
    let truncations = read_auto_find_truncations(connection, &run.run_id)?;
    Ok(AutoFindSnapshot {
        run: Some(run),
        candidates,
        cutoff_evidence,
        truncations,
    })
}

fn read_auto_find_owned_cutoff(
    connection: &Connection,
    artist: &str,
) -> Result<AutoFindCutoffEvidence, RepositoryError> {
    let (oldest, count): (Option<i64>, i64) = connection
        .query_row(
            r#"
            SELECT MIN(owned.gallery_id), COUNT(DISTINCT owned.gallery_id)
            FROM owned_gallery_artists owned
            JOIN download_entries entry ON entry.gallery_id = owned.gallery_id
            JOIN download_artifacts artifact
              ON artifact.entry_id = entry.entry_id AND artifact.gallery_id = entry.gallery_id
            WHERE owned.artist = ?1
              AND entry.state IN ('completed', 'quarantined')
              AND artifact.state IN ('complete', 'quarantined')
              AND artifact.manifest_relative_path IS NOT NULL
              AND artifact.manifest_schema_version IS NOT NULL
              AND artifact.writer_version IS NOT NULL
              AND artifact.completed_at IS NOT NULL
              AND (SELECT COUNT(*) FROM download_pages page
                   WHERE page.entry_id = artifact.entry_id) = artifact.expected_page_count
              AND EXISTS (
                  SELECT 1 FROM download_pages page
                  WHERE page.entry_id = artifact.entry_id
                    AND page.excluded = 0
                    AND page.state IN ('present', 'quarantined')
                    AND page.byte_length IS NOT NULL AND page.sha256 IS NOT NULL
                    AND page.storage_format IS NOT NULL AND page.source_revision IS NOT NULL
                    AND page.verified_at IS NOT NULL
              )
              AND NOT EXISTS (
                  SELECT 1 FROM download_pages page
                  WHERE page.entry_id = artifact.entry_id
                    AND page.excluded = 0
                    AND (page.state NOT IN ('present', 'quarantined')
                         OR page.byte_length IS NULL OR page.sha256 IS NULL
                         OR page.storage_format IS NULL OR page.source_revision IS NULL
                         OR page.verified_at IS NULL)
              )
        "#,
            [artist],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(map_sqlite_error)?;
    Ok(AutoFindCutoffEvidence {
        artist: artist.to_owned(),
        oldest_owned_gallery_id: oldest
            .map(GalleryId::new)
            .transpose()
            .map_err(domain_corruption)?,
        qualified_owned_count: stored_u32(count, "Auto Find qualified ownership count")?,
        source: "verified_owned_artifact".into(),
        policy_version: 1,
    })
}

fn read_owned_gallery_artists(
    connection: &Connection,
    gallery_id: GalleryId,
) -> Result<Vec<String>, RepositoryError> {
    let mut statement = connection
        .prepare("SELECT artist FROM owned_gallery_artists WHERE gallery_id = ?1 ORDER BY artist COLLATE NOCASE ASC")
        .map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([gallery_id.get()], |row| row.get::<_, String>(0))
        .map_err(map_sqlite_error)?;
    let mut artists = rows
        .map(|row| row.map_err(map_sqlite_error))
        .collect::<Result<Vec<_>, _>>()?;
    if artists.is_empty() {
        let primary_artist = connection
            .query_row(
                "SELECT primary_artist FROM galleries WHERE gallery_id = ?1",
                [gallery_id.get()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(map_sqlite_error)?
            .flatten()
            .map(|artist| artist.trim().to_owned())
            .filter(|artist| !artist.is_empty());
        artists.extend(primary_artist);
    }
    Ok(artists)
}

fn read_auto_find_cutoff_evidence(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<AutoFindCutoffEvidence>, RepositoryError> {
    let mut statement = connection.prepare(
        "SELECT artist, oldest_owned_gallery_id, qualified_owned_count, cutoff_source, policy_version FROM auto_find_run_cutoffs WHERE run_id = ?1 ORDER BY artist COLLATE NOCASE ASC",
    ).map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(map_sqlite_error)?;
    rows.map(|row| {
        let (artist, oldest, count, source, policy_version) = row.map_err(map_sqlite_error)?;
        if source != "verified_owned_artifact" {
            return Err(RepositoryError::Corrupt(format!(
                "Auto Find cutoff source {source:?} is unsupported"
            )));
        }
        Ok(AutoFindCutoffEvidence {
            artist,
            oldest_owned_gallery_id: oldest
                .map(GalleryId::new)
                .transpose()
                .map_err(domain_corruption)?,
            qualified_owned_count: stored_u32(count, "Auto Find qualified ownership count")?,
            source,
            policy_version: stored_u32(policy_version, "Auto Find cutoff policy version")?,
        })
    })
    .collect()
}

fn read_auto_find_truncations(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<AutoFindTruncation>, RepositoryError> {
    let mut statement = connection.prepare(
        "SELECT artist, reason, eligible_count, candidate_limit FROM auto_find_run_truncations WHERE run_id = ?1 ORDER BY artist COLLATE NOCASE ASC",
    ).map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(map_sqlite_error)?;
    rows.map(|row| {
        let (artist, reason, eligible, limit) = row.map_err(map_sqlite_error)?;
        Ok(AutoFindTruncation {
            artist,
            reason,
            eligible_count: stored_u32(eligible, "Auto Find eligible candidate count")?,
            limit: stored_u32(limit, "Auto Find candidate limit")?,
        })
    })
    .collect()
}

struct StoredAutoFindCandidate {
    run_id: String,
    gallery_id: i64,
    title: String,
    artist: String,
    group: Option<String>,
    pages: i64,
    language: String,
    tags_json: String,
    series_json: String,
    characters_json: String,
    published_rank: i64,
    popularity: i64,
    thumbnail_key: Option<String>,
    thumbnail_width: i64,
    thumbnail_height: i64,
    favorite_namespace: String,
    favorite_value: String,
    discovered_at: String,
}

impl StoredAutoFindCandidate {
    fn try_into_domain(self) -> Result<AutoFindCandidate, RepositoryError> {
        Ok(AutoFindCandidate {
            run_id: self.run_id,
            gallery: GallerySummary {
                id: GalleryId::new(self.gallery_id).map_err(domain_corruption)?,
                title: self.title,
                artist: self.artist,
                group: self.group,
                pages: stored_u32(self.pages, "Auto Find page count")?,
                language: parse_language(&self.language)?,
                tags: serde_json::from_str(&self.tags_json).map_err(domain_corruption)?,
                series: serde_json::from_str(&self.series_json).map_err(domain_corruption)?,
                characters: serde_json::from_str(&self.characters_json)
                    .map_err(domain_corruption)?,
                published_rank: stored_u32(self.published_rank, "Auto Find published rank")?,
                popularity: stored_u32(self.popularity, "Auto Find popularity")?,
                thumbnail_key: self.thumbnail_key,
                thumbnail_width: stored_u32(self.thumbnail_width, "Auto Find thumbnail width")?,
                thumbnail_height: stored_u32(self.thumbnail_height, "Auto Find thumbnail height")?,
            },
            matched_favorite: FavoriteKey {
                namespace: FavoriteNamespace::from_database(&self.favorite_namespace).ok_or_else(
                    || {
                        RepositoryError::Corrupt(format!(
                            "Auto Find favorite namespace {:?} is unsupported",
                            self.favorite_namespace
                        ))
                    },
                )?,
                value: self.favorite_value,
            },
            discovered_at: self.discovered_at,
        })
    }
}

fn stored_auto_find_candidate(row: &Row<'_>) -> rusqlite::Result<StoredAutoFindCandidate> {
    Ok(StoredAutoFindCandidate {
        run_id: row.get(0)?,
        gallery_id: row.get(1)?,
        title: row.get(2)?,
        artist: row.get(3)?,
        group: row.get(4)?,
        pages: row.get(5)?,
        language: row.get(6)?,
        tags_json: row.get(7)?,
        series_json: row.get(8)?,
        characters_json: row.get(9)?,
        published_rank: row.get(10)?,
        popularity: row.get(11)?,
        thumbnail_key: row.get(12)?,
        thumbnail_width: row.get(13)?,
        thumbnail_height: row.get(14)?,
        favorite_namespace: row.get(15)?,
        favorite_value: row.get(16)?,
        discovered_at: row.get(17)?,
    })
}

fn domain_corruption(error: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::Corrupt(error.to_string())
}

fn read_settings(connection: &Connection) -> Result<SettingsSnapshot, RepositoryError> {
    let values = connection
        .query_row(
            r#"
                SELECT revision, download_root, folder_name_template, max_columns, preview_width,
                       related_preview_width,
                       cache_limit_gb, concurrent_image_requests,
                       request_start_interval_ms, auto_find_history_mode,
                       auto_find_grouping, downloads_grouping, privacy_mode,
                       collapsed_group_keys_json, search_include_tags_json,
                       search_exclude_tags_json
                FROM settings
                WHERE singleton = 1
            "#,
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, bool>(12)?,
                    row.get::<_, String>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, String>(15)?,
                ))
            },
        )
        .map_err(map_sqlite_error)?;

    Ok(SettingsSnapshot {
        revision: stored_u64(values.0, "settings revision")?,
        download_root: values.1,
        folder_name_template: values.2,
        max_columns: stored_u32(values.3, "max columns")?,
        preview_width: crate::domain::normalize_gallery_preview_width(stored_u32(
            values.4,
            "preview width",
        )?),
        related_preview_width: stored_u32(values.5, "related preview width")?,
        cache_limit_gb: stored_u32(values.6, "cache limit")?,
        concurrent_image_requests: stored_u32(values.7, "concurrent image requests")?,
        request_start_interval_ms: stored_u64(values.8, "request start interval")?,
        auto_find_history_mode: AutoFindHistoryMode::from_database(&values.9).ok_or_else(|| {
            RepositoryError::Corrupt(format!(
                "Auto Find history mode {:?} is unsupported",
                values.9
            ))
        })?,
        auto_find_grouping: GalleryGroupingMode::from_database(&values.10).ok_or_else(|| {
            RepositoryError::Corrupt(format!(
                "Auto Find grouping mode {:?} is unsupported",
                values.10
            ))
        })?,
        downloads_grouping: GalleryGroupingMode::from_database(&values.11).ok_or_else(|| {
            RepositoryError::Corrupt(format!(
                "Downloads grouping mode {:?} is unsupported",
                values.11
            ))
        })?,
        privacy_mode: values.12,
        collapsed_group_keys: crate::domain::normalize_collapsed_group_keys(
            serde_json::from_str(&values.13).map_err(domain_corruption)?,
        )
        .map_err(domain_corruption)?,
        search_include_tags: crate::domain::normalize_search_tags(
            serde_json::from_str(&values.14).map_err(domain_corruption)?,
            "searchIncludeTags",
        )
        .map_err(domain_corruption)?,
        search_exclude_tags: crate::domain::normalize_search_tags(
            serde_json::from_str(&values.15).map_err(domain_corruption)?,
            "searchExcludeTags",
        )
        .map_err(domain_corruption)?,
    })
}

fn read_window_placement(
    connection: &Connection,
) -> Result<WindowPlacementSnapshot, RepositoryError> {
    let values = connection
        .query_row(
            r#"
                SELECT revision, x, y, width, height, maximized
                FROM window_placement
                WHERE singleton = 1
            "#,
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i32>>(1)?,
                    row.get::<_, Option<i32>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, bool>(5)?,
                ))
            },
        )
        .map_err(map_sqlite_error)?;

    Ok(WindowPlacementSnapshot {
        revision: stored_u64(values.0, "window placement revision")?,
        x: values.1,
        y: values.2,
        width: stored_u32(values.3, "window width")?,
        height: stored_u32(values.4, "window height")?,
        maximized: values.5,
    })
}

fn next_stored_revision(value: i64, label: &str) -> Result<u64, RepositoryError> {
    stored_u64(value, label)?
        .checked_add(1)
        .ok_or_else(|| RepositoryError::Corrupt(format!("{label} cannot be incremented")))
}

fn stored_u64(value: i64, label: &str) -> Result<u64, RepositoryError> {
    value
        .try_into()
        .map_err(|_| RepositoryError::Corrupt(format!("{label} is negative")))
}

fn stored_u32(value: i64, label: &str) -> Result<u32, RepositoryError> {
    value
        .try_into()
        .map_err(|_| RepositoryError::Corrupt(format!("{label} is outside the supported range")))
}

fn to_sql_integer(value: u64, label: &str) -> Result<i64, RepositoryError> {
    value.try_into().map_err(|_| {
        RepositoryError::Other(format!("{label} exceeds SQLite's signed integer range"))
    })
}

fn map_sqlite_error(error: rusqlite::Error) -> RepositoryError {
    match &error {
        rusqlite::Error::SqliteFailure(sqlite, _)
            if matches!(
                sqlite.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ) =>
        {
            RepositoryError::Busy(error.to_string())
        }
        rusqlite::Error::SqliteFailure(sqlite, _)
            if matches!(
                sqlite.code,
                ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase
            ) =>
        {
            RepositoryError::Corrupt(error.to_string())
        }
        _ => RepositoryError::Other(error.to_string()),
    }
}

fn map_migration_error(error: MigrationError) -> RepositoryError {
    match error {
        MigrationError::Sqlite(error) => map_sqlite_error(error),
        MigrationError::FutureVersion {
            found,
            latest_supported,
        } => RepositoryError::UnsupportedSchema {
            found,
            latest_supported,
        },
        MigrationError::NonContiguousHistory { .. } | MigrationError::NameMismatch { .. } => {
            RepositoryError::Corrupt(error.to_string())
        }
    }
}

#[cfg(test)]
mod tag_catalog_repository_tests {
    use super::*;

    fn catalog_entry(namespace: TagNamespace, name: &str, gallery_count: u64) -> TagCatalogEntry {
        TagCatalogEntry {
            namespace,
            name: name.to_owned(),
            normalized_name: name.to_owned(),
            canonical_token: format!("{}:{}", namespace.as_str(), name.replace(' ', "_")),
            gallery_count,
        }
    }

    #[test]
    fn artist_and_group_catalog_round_trip_and_namespace_filtering() {
        let repository = SqliteRepository::open_in_memory().expect("open repository");
        let entries = vec![
            catalog_entry(TagNamespace::Artist, "mizuno tooru", 142),
            catalog_entry(TagNamespace::Artist, "mizuryu kei", 938),
            catalog_entry(TagNamespace::Group, "mizuryu kei land", 451),
            catalog_entry(TagNamespace::Tag, "mizugi", 2_000),
            catalog_entry(TagNamespace::Female, "mind control", 810),
        ];

        let status = repository
            .tag_catalog_replace(&entries)
            .expect("replace catalog");
        assert_eq!(status.entry_count, 5);
        assert_eq!(status.artist_count, 2);
        assert_eq!(status.group_count, 1);
        assert_eq!(status.neutral_count, 1);
        assert_eq!(status.female_count, 1);

        let artists = repository
            .tag_suggestions_search(&TagSuggestionRequest {
                query: "miz".to_owned(),
                namespace: Some(TagNamespace::Artist),
                limit: 8,
            })
            .expect("search artists");
        assert_eq!(
            artists
                .iter()
                .map(|entry| entry.token.as_str())
                .collect::<Vec<_>>(),
            vec!["artist:mizuryu_kei", "artist:mizuno_tooru"]
        );

        let mixed = repository
            .tag_suggestions_search(&TagSuggestionRequest {
                query: "mizuryu".to_owned(),
                namespace: None,
                limit: 8,
            })
            .expect("search mixed catalog");
        assert_eq!(
            mixed
                .iter()
                .map(|entry| entry.namespace)
                .collect::<Vec<_>>(),
            vec![TagNamespace::Artist, TagNamespace::Group]
        );
    }

    #[test]
    fn artist_and_group_favorites_use_their_own_namespace_and_sort_first() {
        let repository = SqliteRepository::open_in_memory().expect("open repository");
        repository
            .tag_catalog_replace(&[
                catalog_entry(TagNamespace::Artist, "mizuno tooru", 142),
                catalog_entry(TagNamespace::Artist, "mizuryu kei", 938),
                catalog_entry(TagNamespace::Group, "circle energy", 76),
            ])
            .expect("replace catalog");
        repository
            .favorite_set(
                &FavoriteKey {
                    namespace: FavoriteNamespace::Artist,
                    value: "mizuno tooru".to_owned(),
                },
                true,
            )
            .expect("favorite artist");
        repository
            .favorite_set(
                &FavoriteKey {
                    namespace: FavoriteNamespace::Group,
                    value: "circle energy".to_owned(),
                },
                true,
            )
            .expect("favorite group");

        let artists = repository
            .tag_suggestions_search(&TagSuggestionRequest {
                query: "miz".to_owned(),
                namespace: Some(TagNamespace::Artist),
                limit: 8,
            })
            .expect("search artists");
        assert_eq!(artists[0].token, "artist:mizuno_tooru");
        assert!(artists[0].favorite);
        assert!(!artists[1].favorite);

        let groups = repository
            .tag_suggestions_search(&TagSuggestionRequest {
                query: "circle".to_owned(),
                namespace: Some(TagNamespace::Group),
                limit: 8,
            })
            .expect("search groups");
        assert_eq!(groups[0].token, "group:circle_energy");
        assert!(groups[0].favorite);
    }
}

#[cfg(test)]
mod duplicate_repository_tests {
    use std::{
        sync::{mpsc, Arc, Condvar},
        thread,
        time::Instant,
    };

    use image::{codecs::webp::WebPEncoder, ExtendedColorType, ImageEncoder};

    use super::*;
    use crate::{
        application::{ArtifactStore, DuplicateRelationProvider, DuplicateSupervisor},
        domain::ExternalRelationEvidence,
        infrastructure::FilesystemArtifactStore,
    };

    fn seed_verified_gallery(repository: &SqliteRepository, gallery_id: i64, entry_id: &str) {
        let connection = repository.connection().expect("lock test repository");
        connection
            .execute(
                r#"
                    INSERT INTO galleries (
                        gallery_id, revision, title, primary_artist,
                        source_page_count, primary_group
                    ) VALUES (?1, 0, ?2, ?3, 1, ?4)
                "#,
                params![
                    gallery_id,
                    format!("Gallery {gallery_id}"),
                    format!("Artist {gallery_id}"),
                    format!("Group {gallery_id}"),
                ],
            )
            .expect("insert gallery");
        connection
            .execute(
                r#"
                    INSERT INTO download_entries (
                        entry_id, gallery_id, revision, state, progress,
                        created_at, updated_at
                    ) VALUES (?1, ?2, 0, 'completed', 100.0, ?3, ?3)
                "#,
                params![entry_id, gallery_id, "2026-08-15T00:00:00.000Z"],
            )
            .expect("insert download entry");
        connection
            .execute(
                r#"
                    INSERT INTO download_artifacts (
                        entry_id, gallery_id, revision, relative_directory,
                        expected_page_count, state, manifest_relative_path,
                        manifest_schema_version, writer_version,
                        hash_profile_version, completed_at
                    ) VALUES (
                        ?1, ?2, 1, ?3, 1, 'complete', ?4, 1,
                        'duplicate-repository-test', 1, ?5
                    )
                "#,
                params![
                    entry_id,
                    gallery_id,
                    format!("gallery-{gallery_id}"),
                    format!("gallery-{gallery_id}/manifest.json"),
                    "2026-08-15T00:00:00.000Z",
                ],
            )
            .expect("insert complete artifact");
        connection
            .execute(
                r#"
                    INSERT INTO download_pages (
                        entry_id, gallery_id, source_page_number, relative_path,
                        state, byte_length, sha256, storage_format,
                        source_revision, verified_at, excluded
                    ) VALUES (
                        ?1, ?2, 1, ?3, 'present', 128, ?4, 'webp',
                        'source-v1', ?5, 0
                    )
                "#,
                params![
                    entry_id,
                    gallery_id,
                    format!("gallery-{gallery_id}/page-1.webp"),
                    format!("{gallery_id:064x}"),
                    "2026-08-15T00:00:00.000Z",
                ],
            )
            .expect("insert verified page");
    }

    fn candidate_record(run_id: &str) -> DuplicateCandidateRecord {
        DuplicateCandidateRecord {
            run_id: run_id.to_owned(),
            candidate: DuplicateCandidate {
                candidate_id: "candidate-1-2".into(),
                revision: 0,
                parent: DuplicateGalleryRef {
                    gallery_id: GalleryId::new(1).unwrap(),
                    entry_id: "entry-1".into(),
                    title: "ignored scanner title".into(),
                    artist: None,
                    group: None,
                    page_count: 1,
                },
                candidate: DuplicateGalleryRef {
                    gallery_id: GalleryId::new(2).unwrap(),
                    entry_id: "entry-2".into(),
                    title: "ignored scanner title".into(),
                    artist: None,
                    group: None,
                    page_count: 1,
                },
                relation: DuplicateRelation::Exact,
                confidence: 1.0,
                matched_pages: 1,
                parent_coverage: 1.0,
                candidate_coverage: 1.0,
                created_at: "ignored".into(),
                updated_at: "ignored".into(),
            },
            evidence: vec![DuplicateEvidence {
                evidence_id: "evidence-1-2-sha".into(),
                kind: DuplicateEvidenceKind::ExactSha256,
                confidence: 1.0,
                matched_pages: 1,
                description: "Verified page SHA-256 matches".into(),
            }],
            page_pairs: vec![DuplicatePagePair {
                parent_source_page: 1,
                candidate_source_page: 1,
                exact_sha256: true,
                d_hash_distance: 0,
                p_hash_distance: 0,
                detail_hash_distance: 0,
                edge_similarity: 1.0,
                visual_similarity: 1.0,
                low_information: false,
            }],
        }
    }

    fn auto_candidate(run_id: &str, gallery_id: i64) -> AutoFindCandidateRecord {
        AutoFindCandidateRecord {
            run_id: run_id.into(),
            gallery: GallerySummary {
                id: GalleryId::new(gallery_id).unwrap(),
                title: format!("Auto gallery {gallery_id}"),
                artist: "Artist".into(),
                group: None,
                pages: 10,
                language: Language::English,
                tags: vec!["tag".into()],
                series: Vec::new(),
                characters: Vec::new(),
                published_rank: 1,
                popularity: 1,
                thumbnail_key: None,
                thumbnail_width: 100,
                thumbnail_height: 150,
            },
            matched_favorite: FavoriteKey {
                namespace: FavoriteNamespace::Artist,
                value: "artist".into(),
            },
        }
    }

    #[derive(Default)]
    struct BlockingRelationProvider {
        state: Mutex<BlockingRelationState>,
        changed: Condvar,
    }

    #[derive(Default)]
    struct BlockingRelationState {
        entered: bool,
        released: bool,
        active: usize,
        max_active: usize,
    }

    impl BlockingRelationProvider {
        fn wait_until_entered(&self) {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut state = self.state.lock().unwrap();
            while !state.entered {
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .expect("duplicate relation provider was not reached");
                let (next, timeout) = self.changed.wait_timeout(state, remaining).unwrap();
                state = next;
                assert!(
                    !timeout.timed_out(),
                    "duplicate relation provider was not reached"
                );
            }
        }

        fn release(&self) {
            let mut state = self.state.lock().unwrap();
            state.released = true;
            self.changed.notify_all();
        }

        fn max_active(&self) -> usize {
            self.state.lock().unwrap().max_active
        }
    }

    impl DuplicateRelationProvider for BlockingRelationProvider {
        fn enabled(&self) -> bool {
            true
        }

        fn relation(
            &self,
            _parent_gallery_id: GalleryId,
            _candidate_gallery_id: GalleryId,
        ) -> Result<Option<ExternalRelationEvidence>, RepositoryError> {
            let mut state = self.state.lock().unwrap();
            state.active += 1;
            state.max_active = state.max_active.max(state.active);
            state.entered = true;
            self.changed.notify_all();
            while !state.released {
                state = self.changed.wait(state).unwrap();
            }
            state.active -= 1;
            Ok(None)
        }
    }

    fn materialize_verified_page(
        repository: &SqliteRepository,
        root: &Path,
        gallery_id: i64,
        entry_id: &str,
        bytes: &[u8],
    ) {
        seed_verified_gallery(repository, gallery_id, entry_id);
        let directory = root.join(format!("gallery-{gallery_id}"));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("page-1.webp"), bytes).unwrap();
        repository
            .connection()
            .unwrap()
            .execute(
                r#"
                    UPDATE download_pages
                    SET byte_length = ?1, sha256 = ?2
                    WHERE entry_id = ?3 AND source_page_number = 1
                "#,
                params![
                    i64::try_from(bytes.len()).unwrap(),
                    format!("{:x}", Sha256::digest(bytes)),
                    entry_id,
                ],
            )
            .unwrap();
    }

    fn patterned_webp() -> Vec<u8> {
        let mut rgba = vec![0_u8; 256 * 256 * 4];
        for y in 0..256_usize {
            for x in 0..256_usize {
                let offset = (y * 256 + x) * 4;
                rgba[offset] = ((x * 3 + y) % 256) as u8;
                rgba[offset + 1] = ((x + y * 5) % 256) as u8;
                rgba[offset + 2] = if (x / 24 + y / 31) % 2 == 0 { 30 } else { 220 };
                rgba[offset + 3] = 255;
            }
        }
        let mut bytes = Vec::new();
        WebPEncoder::new_lossless(&mut bytes)
            .write_image(&rgba, 256, 256, ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }

    #[test]
    fn duplicate_hash_scan_review_and_decision_are_transactional() {
        let repository = SqliteRepository::open_in_memory().expect("open repository");
        seed_verified_gallery(&repository, 1, "entry-1");
        seed_verified_gallery(&repository, 2, "entry-2");

        let bundles = repository
            .duplicate_artifact_bundles()
            .expect("enumerate verified artifacts");
        assert_eq!(bundles.len(), 2);

        let page_hash = DuplicatePageHash {
            entry_id: "entry-1".into(),
            gallery_id: GalleryId::new(1).unwrap(),
            source_page_number: SourcePageNumber::new(1).unwrap(),
            profile_version: 1,
            artifact_sha256: ArtifactSha256::new(format!("{:064x}", 1)).unwrap(),
            coarse_d_hash: u64::MAX,
            detail_d_hash_hex: "ab".repeat(128),
            p_hash: 0x0123_4567_89ab_cdef,
            mean_luma: 127.0,
            std_dev: 48.0,
            non_uniform_ratio: 0.9,
            edge_density: 0.4,
            width: 1200,
            height: 1800,
            low_information: false,
        };
        repository
            .duplicate_page_hash_upsert(&page_hash)
            .expect("cache page hash");
        assert_eq!(
            repository
                .duplicate_page_hash_get(
                    "entry-1",
                    SourcePageNumber::new(1).unwrap(),
                    1,
                    page_hash.artifact_sha256.as_str(),
                )
                .expect("load cached page hash"),
            Some(page_hash)
        );

        let run = repository
            .duplicate_scan_start(1, 2, 1)
            .expect("start duplicate scan");
        repository
            .duplicate_scan_progress(&run.run_id, 2, 1)
            .expect("advance duplicate scan");
        repository
            .duplicate_candidate_replace(&candidate_record(&run.run_id))
            .expect("store duplicate candidate")
            .expect("candidate changes run");
        let snapshot = repository.duplicate_snapshot().expect("load snapshot");
        assert_eq!(snapshot.candidates.len(), 1);
        assert_eq!(snapshot.candidates[0].parent.title, "Gallery 1");
        assert_eq!(snapshot.run.as_ref().unwrap().candidates_found, 1);
        let review = repository
            .duplicate_review_get("candidate-1-2")
            .expect("load review")
            .expect("candidate exists");
        assert_eq!(review.evidence.len(), 1);
        assert_eq!(review.page_pairs[0].parent_source_page, 1);

        let expected_revision = review.candidate.revision;
        let applied = repository
            .duplicate_decision_apply(&DuplicateDecisionRequest {
                candidate_id: "candidate-1-2".into(),
                expected_revision,
                action: DuplicateDecisionAction::ExcludePair,
                target_gallery_id: None,
                series_group_id: None,
                series_name: None,
            })
            .expect("apply exclusion decision");
        let DuplicateDecisionApplyOutcome::Applied(decided) = applied else {
            panic!("decision should apply");
        };
        assert_eq!(decided.decisions.len(), 1);
        assert_eq!(
            decided.decisions[0].action,
            DuplicateDecisionAction::ExcludePair
        );
        assert!(repository
            .duplicate_snapshot()
            .expect("load filtered snapshot")
            .candidates
            .is_empty());
        assert!(repository
            .duplicate_candidate_replace(&candidate_record(&run.run_id))
            .expect("excluded pair is ignored")
            .is_none());

        let stale = repository
            .duplicate_decision_apply(&DuplicateDecisionRequest {
                candidate_id: "candidate-1-2".into(),
                expected_revision,
                action: DuplicateDecisionAction::ExcludePair,
                target_gallery_id: None,
                series_group_id: None,
                series_name: None,
            })
            .expect("stale decision has typed outcome");
        assert!(matches!(
            stale,
            DuplicateDecisionApplyOutcome::RevisionConflict {
                actual_revision
            } if actual_revision == expected_revision + 1
        ));
    }

    #[test]
    fn contains_decision_keeps_the_gallery_with_more_pages() {
        for (parent_pages, candidate_pages, rejected_action, accepted_action) in [
            (
                20_u32,
                16_u32,
                DuplicateDecisionAction::HideParent,
                DuplicateDecisionAction::HideCandidate,
            ),
            (
                16_u32,
                20_u32,
                DuplicateDecisionAction::HideCandidate,
                DuplicateDecisionAction::HideParent,
            ),
        ] {
            let repository = SqliteRepository::open_in_memory().expect("open repository");
            seed_verified_gallery(&repository, 1, "entry-1");
            seed_verified_gallery(&repository, 2, "entry-2");
            {
                let connection = repository.connection().expect("lock repository");
                for (gallery_id, entry_id, page_count) in [
                    (1_i64, "entry-1", parent_pages),
                    (2_i64, "entry-2", candidate_pages),
                ] {
                    connection
                        .execute(
                            "UPDATE galleries SET source_page_count=?1 WHERE gallery_id=?2",
                            params![i64::from(page_count), gallery_id],
                        )
                        .expect("update gallery page count");
                    connection
                        .execute(
                            "UPDATE download_artifacts SET expected_page_count=?1 WHERE entry_id=?2",
                            params![i64::from(page_count), entry_id],
                        )
                        .expect("update artifact page count");
                }
            }
            let run = repository
                .duplicate_scan_start(1, 2, 1)
                .expect("start scan");
            let mut record = candidate_record(&run.run_id);
            record.candidate.relation = DuplicateRelation::Contains;
            record.candidate.parent.page_count = parent_pages;
            record.candidate.candidate.page_count = candidate_pages;
            repository
                .duplicate_candidate_replace(&record)
                .expect("store contains candidate")
                .expect("candidate changes run");

            let rejected = repository
                .duplicate_decision_apply(&DuplicateDecisionRequest {
                    candidate_id: "candidate-1-2".into(),
                    expected_revision: 0,
                    action: rejected_action,
                    target_gallery_id: None,
                    series_group_id: None,
                    series_name: None,
                })
                .expect_err("hiding the longer gallery must be rejected");
            assert!(rejected
                .to_string()
                .contains("must keep the gallery with more pages"));
            assert_eq!(
                repository
                    .duplicate_review_get("candidate-1-2")
                    .expect("reload review")
                    .expect("candidate remains")
                    .candidate
                    .revision,
                0
            );

            assert!(matches!(
                repository
                    .duplicate_decision_apply(&DuplicateDecisionRequest {
                        candidate_id: "candidate-1-2".into(),
                        expected_revision: 0,
                        action: accepted_action,
                        target_gallery_id: None,
                        series_group_id: None,
                        series_name: None,
                    })
                    .expect("hide shorter gallery"),
                DuplicateDecisionApplyOutcome::Applied(_)
            ));
        }
    }

    #[test]
    fn hide_decision_excludes_only_the_selected_gallery_from_exploration() {
        let repository = SqliteRepository::open_in_memory().expect("open repository");
        seed_verified_gallery(&repository, 2_201_788, "entry-2201788");
        seed_verified_gallery(&repository, 2_232_736, "entry-2232736");
        let run = repository
            .duplicate_scan_start(1, 2, 1)
            .expect("start duplicate scan");
        let mut record = candidate_record(&run.run_id);
        record.candidate.candidate_id = "duplicate-p1-2201788-2232736".into();
        record.candidate.parent.gallery_id = GalleryId::new(2_201_788).unwrap();
        record.candidate.parent.entry_id = "entry-2201788".into();
        record.candidate.candidate.gallery_id = GalleryId::new(2_232_736).unwrap();
        record.candidate.candidate.entry_id = "entry-2232736".into();
        repository
            .duplicate_candidate_replace(&record)
            .expect("store duplicate candidate")
            .expect("candidate changes run");
        {
            let connection = repository.connection().expect("lock repository");
            for gallery_id in [2_201_788_i64, 2_232_736_i64] {
                connection
                    .execute(
                        "INSERT INTO exploration_restored_galleries (gallery_id, restored_at) VALUES (?1, '2026-08-27T00:00:00Z')",
                        [gallery_id],
                    )
                    .expect("seed prior restoration");
            }
        }

        assert!(matches!(
            repository
                .duplicate_decision_apply(&DuplicateDecisionRequest {
                    candidate_id: "duplicate-p1-2201788-2232736".into(),
                    expected_revision: 0,
                    action: DuplicateDecisionAction::HideCandidate,
                    target_gallery_id: None,
                    series_group_id: None,
                    series_name: None,
                })
                .expect("hide selected candidate"),
            DuplicateDecisionApplyOutcome::Applied(_)
        ));

        let connection = repository.connection().expect("inspect decision state");
        let hidden = connection
            .prepare("SELECT gallery_id FROM duplicate_hidden_galleries ORDER BY gallery_id")
            .unwrap()
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(hidden, vec![2_232_736]);
        let restored = connection
            .prepare("SELECT gallery_id FROM exploration_restored_galleries ORDER BY gallery_id")
            .unwrap()
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(restored, vec![2_201_788]);
        connection
            .execute(
                "DELETE FROM exploration_restored_galleries WHERE gallery_id = 2201788",
                [],
            )
            .unwrap();
        drop(connection);

        let exclusions = repository.exploration_exclusions_list().unwrap();
        assert_eq!(exclusions.len(), 1);
        assert_eq!(exclusions[0].gallery_id, GalleryId::new(2_232_736).unwrap());
        assert_eq!(
            exclusions[0].reasons[0].kind,
            ExplorationExclusionKind::DuplicateHidden
        );
    }

    #[test]
    #[ignore = "local duplicate-scan storage profiler"]
    fn profile_duplicate_database_write_stages() {
        const PAGES_PER_ARTIFACT: u32 = 48;
        const PROGRESS_WRITES: u32 = 24;

        let temporary = tempfile::tempdir().expect("create profiler directory");
        let database_path = temporary.path().join("duplicate-profile.sqlite3");
        let repository = SqliteRepository::open(&database_path).expect("open profiler repository");
        seed_verified_gallery(&repository, 1, "entry-1");
        seed_verified_gallery(&repository, 2, "entry-2");

        {
            let connection = repository.connection().expect("lock profiler repository");
            for (gallery_id, entry_id) in [(1_i64, "entry-1"), (2_i64, "entry-2")] {
                connection
                    .execute(
                        "UPDATE galleries SET source_page_count = ?1 WHERE gallery_id = ?2",
                        params![PAGES_PER_ARTIFACT, gallery_id],
                    )
                    .expect("update profiler gallery page count");
                connection
                    .execute(
                        "UPDATE download_artifacts SET expected_page_count = ?1 WHERE entry_id = ?2",
                        params![PAGES_PER_ARTIFACT, entry_id],
                    )
                    .expect("update profiler artifact page count");
                for source_page in 2..=PAGES_PER_ARTIFACT {
                    connection
                        .execute(
                            r#"
                                INSERT INTO download_pages (
                                    entry_id, gallery_id, source_page_number, relative_path,
                                    state, byte_length, sha256, storage_format,
                                    source_revision, verified_at, excluded
                                ) VALUES (
                                    ?1, ?2, ?3, ?4, 'present', 128, ?5, 'webp',
                                    'source-v1', ?6, 0
                                )
                            "#,
                            params![
                                entry_id,
                                gallery_id,
                                source_page,
                                format!("gallery-{gallery_id}/page-{source_page}.webp"),
                                format!("{gallery_id:016x}{source_page:048x}"),
                                "2026-08-15T00:00:00.000Z",
                            ],
                        )
                        .expect("insert profiler page");
                }
            }
        }

        let hash_write_started = Instant::now();
        for (gallery_id, entry_id) in [(1_i64, "entry-1"), (2_i64, "entry-2")] {
            for source_page in 1..=PAGES_PER_ARTIFACT {
                let page_hash = DuplicatePageHash {
                    entry_id: entry_id.into(),
                    gallery_id: GalleryId::new(gallery_id).unwrap(),
                    source_page_number: SourcePageNumber::new(source_page).unwrap(),
                    profile_version: 1,
                    artifact_sha256: ArtifactSha256::new(format!(
                        "{gallery_id:016x}{source_page:048x}"
                    ))
                    .unwrap(),
                    coarse_d_hash: u64::from(source_page),
                    detail_d_hash_hex: format!("{:02x}", source_page % 256).repeat(128),
                    p_hash: u64::from(source_page).rotate_left(17),
                    mean_luma: 127.0,
                    std_dev: 48.0,
                    non_uniform_ratio: 0.9,
                    edge_density: 0.4,
                    width: 640,
                    height: 960,
                    low_information: false,
                };
                repository
                    .duplicate_page_hash_upsert(&page_hash)
                    .expect("write profiler page hash");
            }
        }
        let hash_cache_write = hash_write_started.elapsed();

        let run = repository
            .duplicate_scan_start(1, 2, 1_378)
            .expect("start profiler scan");
        let progress_write_started = Instant::now();
        for step in 0..PROGRESS_WRITES {
            repository
                .duplicate_scan_progress(
                    &run.run_id,
                    if step == 0 { 1 } else { 2 },
                    u64::from(step).saturating_mul(64).min(1_378),
                )
                .expect("write profiler progress");
        }
        let progress_write = progress_write_started.elapsed();

        let candidate_write_started = Instant::now();
        repository
            .duplicate_candidate_replace(&candidate_record(&run.run_id))
            .expect("write profiler candidate");
        let candidate_write = candidate_write_started.elapsed();

        let finish_write_started = Instant::now();
        repository
            .duplicate_scan_finish(&run.run_id, DuplicateScanState::Completed, None, None)
            .expect("finish profiler scan");
        let finish_write = finish_write_started.elapsed();

        let total_write = hash_cache_write
            .saturating_add(progress_write)
            .saturating_add(candidate_write)
            .saturating_add(finish_write);
        eprintln!(
            "duplicate database profile: hash_rows={} hash_cache_write_us={} progress_writes={} progress_write_us={} candidate_write_us={} finish_write_us={} total_write_us={}",
            PAGES_PER_ARTIFACT * 2,
            hash_cache_write.as_micros(),
            PROGRESS_WRITES,
            progress_write.as_micros(),
            candidate_write.as_micros(),
            finish_write.as_micros(),
            total_write.as_micros(),
        );

        assert_eq!(
            repository
                .duplicate_snapshot()
                .expect("read profiler snapshot")
                .run
                .expect("profiler run")
                .state,
            DuplicateScanState::Completed
        );
    }

    #[test]
    fn duplicate_recovery_is_idempotent_and_only_fails_running_scans() {
        let repository = SqliteRepository::open_in_memory().expect("open repository");
        let run = repository
            .duplicate_scan_start(1, 0, 0)
            .expect("start scan");
        assert!(repository
            .duplicate_scan_is_running(&run.run_id)
            .expect("check running scan"));
        assert_eq!(repository.duplicate_recover_interrupted().unwrap(), 1);
        assert_eq!(repository.duplicate_recover_interrupted().unwrap(), 0);
        let recovered = repository.duplicate_snapshot().unwrap().run.unwrap();
        assert_eq!(recovered.state, DuplicateScanState::Failed);
        assert_eq!(
            recovered.error_code.as_deref(),
            Some("DUPLICATE_SCAN_INTERRUPTED")
        );
    }

    #[test]
    fn series_decisions_are_atomic_and_revision_guarded() {
        let repository = SqliteRepository::open_in_memory().expect("open repository");
        seed_verified_gallery(&repository, 1, "entry-1");
        seed_verified_gallery(&repository, 2, "entry-2");
        let run = repository.duplicate_scan_start(1, 2, 1).unwrap();
        repository
            .duplicate_candidate_replace(&candidate_record(&run.run_id))
            .unwrap();

        let first = repository
            .duplicate_decision_apply(&DuplicateDecisionRequest {
                candidate_id: "candidate-1-2".into(),
                expected_revision: 0,
                action: DuplicateDecisionAction::SeriesLink,
                target_gallery_id: None,
                series_group_id: None,
                series_name: Some("A linked series".into()),
            })
            .unwrap();
        let DuplicateDecisionApplyOutcome::Applied(first) = first else {
            panic!("first series link should apply");
        };
        let group_id = first.series_groups[0].series_group_id.clone();
        assert_eq!(first.series_groups[0].members.len(), 2);
        assert_eq!(first.series_groups[0].revision, 1);
        assert_eq!(first.decisions[0].target_gallery_id, None);
        assert_eq!(repository.duplicate_snapshot().unwrap().candidates.len(), 1);

        let second = repository
            .duplicate_decision_apply(&DuplicateDecisionRequest {
                candidate_id: "candidate-1-2".into(),
                expected_revision: 1,
                action: DuplicateDecisionAction::SeriesUnlink,
                target_gallery_id: Some(1),
                series_group_id: Some(group_id.clone()),
                series_name: None,
            })
            .unwrap();
        let DuplicateDecisionApplyOutcome::Applied(second) = second else {
            panic!("series unlink should apply");
        };
        assert_eq!(second.candidate.revision, 2);
        assert_eq!(second.decisions.len(), 2);
        assert_eq!(second.series_groups[0].members.len(), 1);
        assert_eq!(repository.duplicate_snapshot().unwrap().candidates.len(), 1);

        assert!(matches!(
            repository
                .duplicate_decision_apply(&DuplicateDecisionRequest {
                    candidate_id: "candidate-1-2".into(),
                    expected_revision: 1,
                    action: DuplicateDecisionAction::HideParent,
                    target_gallery_id: None,
                    series_group_id: None,
                    series_name: None,
                })
                .unwrap(),
            DuplicateDecisionApplyOutcome::RevisionConflict { actual_revision: 2 }
        ));
        assert!(matches!(
            repository
                .duplicate_decision_apply(&DuplicateDecisionRequest {
                    candidate_id: "candidate-1-2".into(),
                    expected_revision: 2,
                    action: DuplicateDecisionAction::HideParent,
                    target_gallery_id: None,
                    series_group_id: None,
                    series_name: None,
                })
                .unwrap(),
            DuplicateDecisionApplyOutcome::Applied(_)
        ));
        assert!(repository
            .duplicate_snapshot()
            .unwrap()
            .candidates
            .is_empty());
    }

    #[test]
    fn auto_find_excludes_only_explicitly_hidden_duplicate_galleries() {
        let repository = SqliteRepository::open_in_memory().expect("open repository");
        let run = repository
            .auto_find_start(1, AutoFindHistoryMode::IncludeAllHistory, &[])
            .unwrap();
        {
            let connection = repository.connection().unwrap();
            connection
                .execute(
                    "INSERT INTO duplicate_hidden_galleries (gallery_id, decision_id, created_at) VALUES (10, 'hidden-10', 'now')",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO duplicate_pair_exclusions (parent_gallery_id, candidate_gallery_id, decision_id, created_at) VALUES (11, 12, 'excluded-11-12', 'now')",
                    [],
                )
                .unwrap();
        }
        assert!(repository
            .auto_find_candidate_add(&auto_candidate(&run.run_id, 10))
            .unwrap()
            .is_none());
        assert!(repository
            .auto_find_candidate_add(&auto_candidate(&run.run_id, 11))
            .unwrap()
            .is_some());
        assert!(repository
            .auto_find_candidate_add(&auto_candidate(&run.run_id, 13))
            .unwrap()
            .is_some());
        assert_eq!(repository.auto_find_snapshot().unwrap().candidates.len(), 2);
        repository
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO duplicate_hidden_galleries (gallery_id, decision_id, created_at) VALUES (13, 'hidden-13', 'now')",
                [],
            )
            .unwrap();
        let visible = repository
            .auto_find_snapshot()
            .unwrap()
            .candidates
            .into_iter()
            .map(|candidate| candidate.gallery.id)
            .collect::<Vec<_>>();
        assert_eq!(visible, vec![GalleryId::new(11).unwrap()]);
        let exclusions = repository.exploration_exclusions_list().unwrap();
        assert!(!exclusions
            .iter()
            .any(|item| item.gallery_id == GalleryId::new(11).unwrap()));
        assert!(exclusions.iter().any(|item| {
            item.gallery_id == GalleryId::new(13).unwrap()
                && item
                    .reasons
                    .iter()
                    .any(|reason| reason.kind == ExplorationExclusionKind::DuplicateHidden)
        }));
        repository
            .exploration_exclusions_restore(&[
                GalleryId::new(10).unwrap(),
                GalleryId::new(13).unwrap(),
            ])
            .unwrap();
        assert!(repository
            .auto_find_candidate_add(&auto_candidate(&run.run_id, 10))
            .unwrap()
            .is_some());
        let visible = repository
            .auto_find_snapshot()
            .unwrap()
            .candidates
            .into_iter()
            .map(|candidate| candidate.gallery.id)
            .collect::<Vec<_>>();
        assert!(visible.contains(&GalleryId::new(10).unwrap()));
        assert!(visible.contains(&GalleryId::new(11).unwrap()));
        assert!(visible.contains(&GalleryId::new(13).unwrap()));
        let exclusions = repository.exploration_exclusions_list().unwrap();
        assert!(!exclusions.iter().any(|item| {
            item.gallery_id == GalleryId::new(10).unwrap()
                || item.gallery_id == GalleryId::new(13).unwrap()
        }));
    }

    #[test]
    fn auto_find_cutoff_requires_a_complete_verified_owned_artifact() {
        let repository = SqliteRepository::open_in_memory().expect("open repository");
        let seed = |gallery_id: i64,
                    entry_state: &str,
                    artifact_state: &str,
                    expected: i64,
                    page_count: i64,
                    excluded: bool| {
            let connection = repository.connection().expect("lock repository");
            connection.execute(
                "INSERT INTO galleries (gallery_id, revision, title, primary_artist, source_page_count) VALUES (?1, 0, ?2, 'serein', ?3)",
                params![gallery_id, format!("Gallery {gallery_id}"), expected],
            ).unwrap();
            connection.execute(
                "INSERT INTO download_entries (entry_id, gallery_id, revision, state, progress, created_at, updated_at) VALUES (?1, ?2, 0, ?3, 100.0, 'now', 'now')",
                params![format!("entry-{gallery_id}"), gallery_id, entry_state],
            ).unwrap();
            connection.execute(
                "INSERT INTO download_artifacts (entry_id, gallery_id, revision, relative_directory, expected_page_count, state, manifest_relative_path, manifest_schema_version, writer_version, hash_profile_version, completed_at) VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, 1, 'test', 1, 'now')",
                params![format!("entry-{gallery_id}"), gallery_id, format!("gallery-{gallery_id}"), expected, artifact_state, format!("gallery-{gallery_id}/manifest.json")],
            ).unwrap();
            for page in 1..=page_count {
                connection.execute(
                    "INSERT INTO download_pages (entry_id, gallery_id, source_page_number, relative_path, state, byte_length, sha256, storage_format, source_revision, verified_at, excluded) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 'webp', 'source', 'now', ?7)",
                    params![format!("entry-{gallery_id}"), gallery_id, page, format!("{page}.webp"), if artifact_state == "quarantined" { "quarantined" } else { "present" }, "a".repeat(64), excluded],
                ).unwrap();
            }
            connection
                .execute(
                    "INSERT INTO owned_gallery_artists (gallery_id, artist) VALUES (?1, 'serein')",
                    [gallery_id],
                )
                .unwrap();
        };

        seed(100, "completed", "complete", 2, 1, false); // missing required page
        seed(150, "completed", "complete", 1, 1, true); // all-excluded is not owned evidence
        seed(200, "quarantined", "quarantined", 1, 1, false); // recoverable ownership counts
        seed(50, "failed", "complete", 1, 1, false); // failed work is not owned
        repository.connection().unwrap().execute(
            "INSERT INTO duplicate_hidden_galleries (gallery_id, decision_id, created_at) VALUES (200, 'hidden-200', 'now')",
            [],
        ).unwrap();

        let cutoffs = repository
            .auto_find_owned_cutoffs(&["serein".into()])
            .unwrap();
        assert_eq!(
            cutoffs,
            vec![AutoFindCutoffEvidence {
                artist: "serein".into(),
                oldest_owned_gallery_id: Some(GalleryId::new(200).unwrap()),
                qualified_owned_count: 1,
                source: "verified_owned_artifact".into(),
                policy_version: 1,
            }]
        );
    }

    #[test]
    fn duplicate_cancel_joins_worker_before_immediate_restart() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("downloads");
        std::fs::create_dir_all(&root).unwrap();
        let repository = Arc::new(SqliteRepository::open_in_memory().unwrap());
        let bytes = patterned_webp();
        materialize_verified_page(&repository, &root, 1, "entry-1", &bytes);
        materialize_verified_page(&repository, &root, 2, "entry-2", &bytes);
        repository
            .connection()
            .unwrap()
            .execute(
                "UPDATE galleries SET primary_artist = 'shared artist' WHERE gallery_id IN (1, 2)",
                [],
            )
            .unwrap();
        repository
            .connection()
            .unwrap()
            .execute(
                "UPDATE settings SET download_root = ?1 WHERE singleton = 1",
                [root.to_string_lossy().as_ref()],
            )
            .unwrap();

        let gate = Arc::new(BlockingRelationProvider::default());
        let duplicate_repository: Arc<dyn DuplicateRepository> = repository.clone();
        let settings_repository: Arc<dyn StateRepository> = repository.clone();
        let artifact_store: Arc<dyn ArtifactStore> = Arc::new(FilesystemArtifactStore::new());
        let relations: Arc<dyn DuplicateRelationProvider> = gate.clone();
        let (events, _event_rx) = mpsc::channel();
        let supervisor = DuplicateSupervisor::new(
            duplicate_repository,
            settings_repository,
            artifact_store,
            relations,
            events,
        );
        let first = supervisor.start().expect("start first scan");
        gate.wait_until_entered();

        let cancelling = supervisor.clone();
        let cancel = thread::spawn(move || cancelling.cancel().expect("cancel first scan"));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snapshot = repository.duplicate_snapshot().unwrap();
            if snapshot
                .run
                .as_ref()
                .is_some_and(|run| run.state == DuplicateScanState::Cancelled)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "cancel did not persist before worker join"
            );
            thread::yield_now();
        }
        gate.release();
        assert_eq!(cancel.join().unwrap().state, DuplicateScanState::Cancelled);

        let second = supervisor
            .start()
            .expect("restart after joined cancellation");
        assert_ne!(first.run_id, second.run_id);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let run = repository.duplicate_snapshot().unwrap().run.unwrap();
            if run.run_id == second.run_id && run.state != DuplicateScanState::Running {
                assert_eq!(run.state, DuplicateScanState::Completed);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "replacement scan did not complete"
            );
            thread::yield_now();
        }
        assert_eq!(gate.max_active(), 1);
        supervisor.shutdown_and_wait();
    }
}

#[cfg(test)]
mod migration_backup_tests {
    use super::*;
    use crate::infrastructure::migrations::MIGRATIONS;

    fn create_database_before_latest_migration(path: &Path) -> i64 {
        let connection = Connection::open(path).expect("create pre-migration database");
        connection
            .execute_batch(
                r#"
                    PRAGMA foreign_keys = ON;
                    CREATE TABLE schema_migrations (
                        version INTEGER PRIMARY KEY,
                        name TEXT NOT NULL,
                        applied_at TEXT NOT NULL DEFAULT (
                            strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        )
                    ) STRICT;
                "#,
            )
            .expect("create migration history");
        let migrations_before_latest = &MIGRATIONS[..MIGRATIONS.len() - 1];
        for migration in migrations_before_latest {
            connection
                .execute_batch(migration.sql)
                .expect("apply pre-latest migration");
            connection
                .execute(
                    "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                    params![migration.version, migration.name],
                )
                .expect("record pre-latest migration");
        }
        migrations_before_latest
            .last()
            .expect("at least one pre-latest migration")
            .version
    }

    fn backup_files(directory: &Path) -> Vec<PathBuf> {
        let mut backups = std::fs::read_dir(directory)
            .expect("read database directory")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(".pre-migration-") && name.ends_with(".bak"))
            })
            .collect::<Vec<_>>();
        backups.sort();
        backups
    }

    #[test]
    fn file_database_is_backed_up_once_before_pending_migrations() {
        let temporary = tempfile::tempdir().expect("create temporary directory");
        let database_path = temporary.path().join("atsumi-next.sqlite3");
        let previous_version = create_database_before_latest_migration(&database_path);
        let target_version = MIGRATIONS.last().expect("latest migration").version;

        drop(SqliteRepository::open(&database_path).expect("migrate persistent repository"));

        let backups = backup_files(temporary.path());
        assert_eq!(backups.len(), 1);
        let backup_name = backups[0]
            .file_name()
            .and_then(|name| name.to_str())
            .expect("backup file name is Unicode");
        assert!(backup_name.starts_with(&format!(
            "atsumi-next.sqlite3.pre-migration-v{previous_version}-to-v{target_version}-"
        )));
        assert!(backup_name.ends_with(".bak"));
        let backup = Connection::open(&backups[0]).expect("open recoverable backup");
        let backup_version: i64 = backup
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("read backup schema version");
        assert_eq!(backup_version, previous_version);
        let integrity: String = backup
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .expect("check backup integrity");
        assert_eq!(integrity, "ok");
        drop(backup);

        let migrated = Connection::open(&database_path).expect("open migrated database");
        let migrated_version: i64 = migrated
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("read migrated schema version");
        assert_eq!(migrated_version, target_version);
        drop(migrated);

        drop(SqliteRepository::open(&database_path).expect("reopen current repository"));
        assert_eq!(backup_files(temporary.path()).len(), 1);
    }

    #[test]
    fn backup_names_never_overwrite_an_existing_snapshot() {
        let temporary = tempfile::tempdir().expect("create temporary directory");
        let database_path = temporary.path().join("atsumi-next.sqlite3");
        let created_at = 1_786_780_000;
        let first = next_pre_migration_backup_path(&database_path, 6, 7, created_at)
            .expect("reserve first backup path");
        std::fs::write(&first, b"keep this recovery snapshot")
            .expect("create an existing recovery snapshot");

        let second = next_pre_migration_backup_path(&database_path, 6, 7, created_at)
            .expect("reserve non-overwriting backup path");

        assert_ne!(second, first);
        assert_eq!(
            second.file_name().and_then(|name| name.to_str()),
            Some("atsumi-next.sqlite3.pre-migration-v6-to-v7-1786780000-1.bak")
        );
        assert_eq!(
            std::fs::read(&first).expect("existing snapshot remains readable"),
            b"keep this recovery snapshot"
        );
    }

    #[test]
    fn backup_failure_prevents_pending_migrations_from_running() {
        let temporary = tempfile::tempdir().expect("create temporary directory");
        let missing_database_path = temporary
            .path()
            .join("missing-directory")
            .join("atsumi-next.sqlite3");
        let mut connection = Connection::open_in_memory().expect("open migration test database");
        connection
            .execute_batch(
                r#"
                    CREATE TABLE schema_migrations (
                        version INTEGER PRIMARY KEY,
                        name TEXT NOT NULL,
                        applied_at TEXT NOT NULL DEFAULT (
                            strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        )
                    ) STRICT;
                    INSERT INTO schema_migrations (version, name)
                    VALUES (1, 'settings_and_window_placement');
                "#,
            )
            .expect("seed pending migration history");

        let error = run_migrations_with_backup(&mut connection, Some(&missing_database_path))
            .expect_err("backup failure must abort migration");

        assert!(matches!(
            error,
            RepositoryError::MigrationBackup(message)
                if message.contains("could not create pre-migration backup")
        ));
        let recorded_version: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("read unchanged migration history");
        assert_eq!(recorded_version, 1);
        let migration_two_table_exists: bool = connection
            .query_row(
                r#"
                    SELECT EXISTS (
                        SELECT 1 FROM sqlite_schema
                        WHERE type = 'table' AND name = 'download_entries'
                    )
                "#,
                [],
                |row| row.get(0),
            )
            .expect("check that migration two did not run");
        assert!(!migration_two_table_exists);
    }

    #[test]
    fn persistent_repository_uses_wal_without_exclusive_locking() {
        let temporary = tempfile::tempdir().expect("create temporary directory");
        let database_path = temporary.path().join("atsumi-next.sqlite3");
        let first = SqliteRepository::open(&database_path).expect("open primary repository");
        let second = SqliteRepository::open(&database_path).expect("open concurrent repository");

        let observer = Connection::open(&database_path).expect("open independent observer");
        let journal_mode: String = observer
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("read journal mode");
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");

        drop(observer);
        drop(second);
        drop(first);
    }

    #[test]
    fn future_schema_is_rejected_without_modifying_the_database() {
        let temporary = tempfile::tempdir().expect("create temporary directory");
        let database_path = temporary.path().join("future.sqlite3");
        let future_version = MIGRATIONS.last().expect("latest migration").version + 1;
        let connection = Connection::open(&database_path).expect("create future database");
        connection
            .execute_batch(&format!(
                r#"
                    CREATE TABLE schema_migrations (
                        version INTEGER PRIMARY KEY,
                        name TEXT NOT NULL,
                        applied_at TEXT NOT NULL DEFAULT (
                            strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        )
                    ) STRICT;
                    INSERT INTO schema_migrations (version, name)
                    VALUES ({future_version}, 'future_schema');
                    CREATE TABLE future_sentinel (
                        value TEXT NOT NULL
                    ) STRICT;
                    INSERT INTO future_sentinel (value) VALUES ('preserve-me');
                "#
            ))
            .expect("seed future schema");
        drop(connection);
        let before = std::fs::read(&database_path).expect("snapshot future database");

        let error = match SqliteRepository::open(&database_path) {
            Ok(_) => panic!("older application must reject a future schema"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            RepositoryError::UnsupportedSchema {
                found,
                latest_supported
            } if found == future_version
                && latest_supported == future_version - 1
        ));
        let after = std::fs::read(&database_path).expect("re-read future database");
        assert_eq!(after, before);
        assert!(backup_files(temporary.path()).is_empty());
    }
}
