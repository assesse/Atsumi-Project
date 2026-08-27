use std::{
    collections::BTreeSet,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use uuid::Uuid;

use crate::{
    application::{InternalDuplicateRepository, InternalPlanPrepareOutcome, RepositoryError},
    domain::{
        GalleryId, InternalDuplicateGroup, InternalDuplicateReview, InternalDuplicateSnapshot,
        InternalGroupRecord, InternalMatchKind, InternalPageEvidence, InternalRemovalPlan,
        InternalRemovalSelection, InternalScanRun, InternalScanSkip, InternalScanState,
        PageQuarantineRecord, PageQuarantineSaga, PageQuarantineState, SourcePageNumber,
    },
};

use super::SqliteRepository;

impl InternalDuplicateRepository for SqliteRepository {
    fn internal_recover_interrupted(&self) -> Result<usize, RepositoryError> {
        let connection = self.connection()?;
        connection
            .execute(
                r#"
                    UPDATE internal_duplicate_runs
                    SET revision = revision + 1,
                        state = 'failed',
                        finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        error_code = 'INTERNAL_SCAN_INTERRUPTED',
                        error_message = 'The previous process ended during internal duplicate scanning'
                    WHERE state = 'running'
                "#,
                [],
            )
            .map_err(repository_sql_error)
    }

    fn internal_scan_start(
        &self,
        profile_version: u32,
        algorithm_version: u32,
        total_artifacts: u32,
        total_pages: u32,
        skips: &[InternalScanSkip],
    ) -> Result<InternalScanRun, RepositoryError> {
        let mut connection = self.connection()?;
        if let Some(run) = read_latest_internal_run(&connection)? {
            if run.state == InternalScanState::Running {
                return Ok(run);
            }
        }
        let run_id = format!("internal-run-{}", Uuid::new_v4());
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(repository_sql_error)?;
        transaction
            .execute(
                r#"
                    INSERT INTO internal_duplicate_runs (
                        run_id, revision, state, profile_version, algorithm_version,
                        total_artifacts, scanned_artifacts, total_pages,
                        compared_pairs, groups_found, skipped_artifacts, skipped_pages,
                        started_at, updated_at
                    ) VALUES (
                        ?1, 0, 'running', ?2, ?3, ?4, 0, ?5, 0, 0, ?6, ?7,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                "#,
                params![
                    run_id,
                    i64::from(profile_version),
                    i64::from(algorithm_version),
                    i64::from(total_artifacts),
                    i64::from(total_pages),
                    i64::try_from(skips.len()).unwrap_or(i64::MAX),
                    skips
                        .iter()
                        .map(|skip| i64::from(skip.page_count))
                        .sum::<i64>()
                ],
            )
            .map_err(repository_sql_error)?;
        for skip in skips {
            transaction.execute(
                "INSERT INTO internal_duplicate_scan_skips (run_id, entry_id, gallery_id, title, page_count, reason) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![run_id, skip.entry_id, skip.gallery_id.get(), skip.title, i64::from(skip.page_count), skip.reason],
            ).map_err(repository_sql_error)?;
        }
        transaction.commit().map_err(repository_sql_error)?;
        read_internal_run(&connection, &run_id)?.ok_or_else(|| {
            RepositoryError::Corrupt("new internal duplicate run disappeared".into())
        })
    }

    fn internal_scan_progress(
        &self,
        run_id: &str,
        scanned_artifacts: u32,
        compared_pairs: u64,
    ) -> Result<Option<InternalScanRun>, RepositoryError> {
        let connection = self.connection()?;
        connection
            .execute(
                r#"
                    UPDATE internal_duplicate_runs
                    SET revision = revision + 1,
                        scanned_artifacts = MIN(total_artifacts, MAX(scanned_artifacts, ?1)),
                        compared_pairs = MAX(compared_pairs, ?2),
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    WHERE run_id = ?3 AND state = 'running'
                "#,
                params![
                    i64::from(scanned_artifacts),
                    sql_u64(compared_pairs)?,
                    run_id
                ],
            )
            .map_err(repository_sql_error)?;
        read_internal_run(&connection, run_id)
    }

    fn internal_group_replace(
        &self,
        record: &InternalGroupRecord,
    ) -> Result<Option<InternalScanRun>, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(repository_sql_error)?;
        let running: bool = transaction
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM internal_duplicate_runs WHERE run_id = ?1 AND state = 'running')",
                [&record.run_id],
                |row| row.get(0),
            )
            .map_err(repository_sql_error)?;
        if !running {
            transaction.commit().map_err(repository_sql_error)?;
            return Ok(None);
        }
        let group = &record.group;
        transaction
            .execute(
                r#"
                    INSERT INTO internal_duplicate_groups (
                        group_id, block_id, sequence_index, revision,
                        last_seen_run_id, entry_id, gallery_id, relation,
                        confidence, recommended_keep_source_page, resolved,
                        created_at, updated_at
                    ) VALUES (
                        ?1, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9, 0,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                    ON CONFLICT(group_id) DO UPDATE SET
                        block_id = excluded.block_id,
                        sequence_index = excluded.sequence_index,
                        revision = internal_duplicate_groups.revision + 1,
                        last_seen_run_id = excluded.last_seen_run_id,
                        entry_id = excluded.entry_id,
                        gallery_id = excluded.gallery_id,
                        relation = excluded.relation,
                        confidence = excluded.confidence,
                        recommended_keep_source_page = excluded.recommended_keep_source_page,
                        resolved = 0,
                        updated_at = excluded.updated_at
                "#,
                params![
                    group.group_id,
                    group.block_id,
                    i64::from(group.sequence_index),
                    record.run_id,
                    group.entry_id,
                    group.gallery_id.get(),
                    group.relation.as_str(),
                    group.confidence,
                    i64::from(group.recommended_keep_source_page),
                ],
            )
            .map_err(repository_sql_error)?;
        transaction
            .execute(
                "DELETE FROM internal_duplicate_group_pages WHERE group_id = ?1",
                [&group.group_id],
            )
            .map_err(repository_sql_error)?;
        for page in &group.pages {
            if page.edition_track_id.is_some() != page.edition_track_ordinal.is_some()
                || page
                    .edition_track_id
                    .as_deref()
                    .is_some_and(|track_id| track_id.trim().is_empty())
            {
                return Err(RepositoryError::Corrupt(
                    "internal duplicate page has an incomplete edition track".into(),
                ));
            }
            transaction
                .execute(
                    r#"
                        INSERT INTO internal_duplicate_group_pages (
                            group_id, source_page_number, exact_sha256,
                            visual_similarity, detail_hash_distance, low_information,
                            edition_track_id, edition_track_ordinal
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    "#,
                    params![
                        group.group_id,
                        i64::from(page.source_page),
                        page.exact_sha256,
                        page.visual_similarity,
                        i64::from(page.detail_hash_distance),
                        page.low_information,
                        page.edition_track_id,
                        page.edition_track_ordinal.map(i64::from),
                    ],
                )
                .map_err(repository_sql_error)?;
        }
        transaction
            .execute(
                r#"
                    UPDATE internal_duplicate_runs
                    SET revision = revision + 1,
                        groups_found = (
                            SELECT COUNT(*) FROM internal_duplicate_groups
                            WHERE last_seen_run_id = ?1 AND resolved = 0
                        ),
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    WHERE run_id = ?1 AND state = 'running'
                "#,
                [&record.run_id],
            )
            .map_err(repository_sql_error)?;
        transaction.commit().map_err(repository_sql_error)?;
        read_internal_run(&connection, &record.run_id)
    }

    fn internal_scan_finish(
        &self,
        run_id: &str,
        state: InternalScanState,
        error_code: Option<&str>,
        error_message: Option<&str>,
        completed_gallery_ids: &[GalleryId],
    ) -> Result<Option<InternalScanRun>, RepositoryError> {
        if state == InternalScanState::Running {
            return Err(RepositoryError::Other(
                "internal scan finish requires a terminal state".into(),
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(repository_sql_error)?;
        let changed = transaction
            .execute(
                r#"
                    UPDATE internal_duplicate_runs
                    SET revision = revision + 1,
                        state = ?1,
                        finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        error_code = ?2,
                        error_message = ?3
                    WHERE run_id = ?4 AND state = 'running'
                "#,
                params![state.as_str(), error_code, error_message, run_id],
            )
            .map_err(repository_sql_error)?;
        if changed == 1 && state == InternalScanState::Completed {
            for gallery_id in completed_gallery_ids {
                transaction
                    .execute(
                        r#"
                            UPDATE internal_duplicate_groups
                            SET revision = revision + 1, resolved = 1,
                                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                            WHERE gallery_id = ?1 AND last_seen_run_id != ?2 AND resolved = 0
                        "#,
                        params![gallery_id.get(), run_id],
                    )
                    .map_err(repository_sql_error)?;
            }
        }
        transaction.commit().map_err(repository_sql_error)?;
        read_internal_run(&connection, run_id)
    }

    fn internal_scan_is_running(&self, run_id: &str) -> Result<bool, RepositoryError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM internal_duplicate_runs WHERE run_id = ?1 AND state = 'running')",
                [run_id],
                |row| row.get(0),
            )
            .map_err(repository_sql_error)
    }

    fn internal_snapshot(&self) -> Result<InternalDuplicateSnapshot, RepositoryError> {
        let connection = self.connection()?;
        Ok(InternalDuplicateSnapshot {
            run: read_latest_internal_run(&connection)?,
            groups: read_internal_groups(&connection, None)?,
            quarantine_records: read_page_records(&connection, None, false)?,
            skips: read_latest_internal_skips(&connection)?,
        })
    }

    fn internal_review_get(
        &self,
        entry_id: &str,
    ) -> Result<Option<InternalDuplicateReview>, RepositoryError> {
        let connection = self.connection()?;
        let gallery = connection
            .query_row(
                r#"
                    SELECT e.gallery_id, g.title
                    FROM download_entries e
                    JOIN galleries g ON g.gallery_id = e.gallery_id
                    WHERE e.entry_id = ?1
                "#,
                [entry_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(repository_sql_error)?;
        let Some((gallery_id, title)) = gallery else {
            return Ok(None);
        };
        Ok(Some(InternalDuplicateReview {
            entry_id: entry_id.to_owned(),
            gallery_id: valid_gallery_id(gallery_id)?,
            title,
            groups: read_internal_groups(&connection, Some(entry_id))?,
            quarantine_records: read_page_records(&connection, Some(entry_id), true)?,
        }))
    }

    fn internal_plan_prepare(
        &self,
        plan: &InternalRemovalPlan,
    ) -> Result<InternalPlanPrepareOutcome, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(repository_sql_error)?;
        let entry_exists: bool = transaction
            .query_row(
                r#"
                    SELECT EXISTS (
                        SELECT 1 FROM download_entries e
                        JOIN download_artifacts a ON a.entry_id = e.entry_id
                        WHERE e.entry_id = ?1 AND e.state = 'completed'
                          AND a.state = 'complete'
                    )
                "#,
                [&plan.entry_id],
                |row| row.get(0),
            )
            .map_err(repository_sql_error)?;
        if !entry_exists {
            transaction.commit().map_err(repository_sql_error)?;
            return Ok(InternalPlanPrepareOutcome::EntryNotFound);
        }
        let mut removals = BTreeSet::new();
        let mut bytes = 0_u64;
        for selection in &plan.selections {
            let stored = transaction
                .query_row(
                    r#"
                        SELECT revision, resolved FROM internal_duplicate_groups
                        WHERE group_id = ?1 AND entry_id = ?2
                    "#,
                    params![selection.group_id, plan.entry_id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?)),
                )
                .optional()
                .map_err(repository_sql_error)?;
            let Some((revision, resolved)) = stored else {
                transaction.commit().map_err(repository_sql_error)?;
                return Ok(InternalPlanPrepareOutcome::InvalidSelection(
                    "The internal duplicate group no longer exists".into(),
                ));
            };
            let revision = stored_u64(revision, "internal group revision")?;
            if revision != selection.expected_revision {
                transaction.commit().map_err(repository_sql_error)?;
                return Ok(InternalPlanPrepareOutcome::RevisionConflict {
                    group_id: selection.group_id.clone(),
                    actual_revision: revision,
                });
            }
            if resolved || selection.remove_source_pages.is_empty() {
                transaction.commit().map_err(repository_sql_error)?;
                return Ok(InternalPlanPrepareOutcome::InvalidSelection(
                    "The internal duplicate group is already resolved or has no removal".into(),
                ));
            }
            let group_pages = internal_group_page_numbers(&transaction, &selection.group_id)?;
            if !group_pages.contains(&selection.keep_source_page)
                || selection
                    .remove_source_pages
                    .iter()
                    .any(|page| *page == selection.keep_source_page || !group_pages.contains(page))
            {
                transaction.commit().map_err(repository_sql_error)?;
                return Ok(InternalPlanPrepareOutcome::InvalidSelection(
                    "The keep/remove pages do not match the synchronized review row".into(),
                ));
            }
            for source_page in &selection.remove_source_pages {
                if !removals.insert(*source_page) {
                    transaction.commit().map_err(repository_sql_error)?;
                    return Ok(InternalPlanPrepareOutcome::InvalidSelection(
                        "A source page appears in more than one removal selection".into(),
                    ));
                }
                let length = transaction
                    .query_row(
                        r#"
                            SELECT byte_length FROM download_pages
                            WHERE entry_id = ?1 AND source_page_number = ?2
                              AND state = 'present' AND excluded = 0
                        "#,
                        params![plan.entry_id, i64::from(*source_page)],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()
                    .map_err(repository_sql_error)?;
                let Some(length) = length else {
                    transaction.commit().map_err(repository_sql_error)?;
                    return Ok(InternalPlanPrepareOutcome::InvalidSelection(
                        "A selected source page is no longer present and verified".into(),
                    ));
                };
                bytes = bytes.saturating_add(stored_u64(length, "page byte length")?);
            }
        }
        if removals.is_empty()
            || u32::try_from(removals.len()).unwrap_or(u32::MAX) != plan.files_to_quarantine
            || bytes != plan.bytes_to_quarantine
        {
            transaction.commit().map_err(repository_sql_error)?;
            return Ok(InternalPlanPrepareOutcome::InvalidSelection(
                "The removal plan summary does not match current artifact pages".into(),
            ));
        }
        let selections_json = serde_json::to_string(&plan.selections).map_err(|error| {
            RepositoryError::Other(format!("could not encode removal plan: {error}"))
        })?;
        transaction
            .execute(
                r#"
                    INSERT INTO internal_removal_plans (
                        plan_id, entry_id, selections_json, files_to_quarantine,
                        bytes_to_quarantine, state, expires_at, created_at, updated_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, 'prepared', ?6,
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                "#,
                params![
                    plan.plan_id,
                    plan.entry_id,
                    selections_json,
                    i64::from(plan.files_to_quarantine),
                    sql_u64(plan.bytes_to_quarantine)?,
                    plan.expires_at,
                ],
            )
            .map_err(repository_sql_error)?;
        transaction.commit().map_err(repository_sql_error)?;
        Ok(InternalPlanPrepareOutcome::Prepared(plan.clone()))
    }

    fn internal_removal_begin(
        &self,
        plan_id: &str,
        reason: &str,
    ) -> Result<Vec<PageQuarantineSaga>, RepositoryError> {
        let reason = reason.trim();
        if reason.is_empty() || reason.len() > 500 {
            return Err(RepositoryError::Other(
                "page quarantine reason must be 1..=500 bytes".into(),
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(repository_sql_error)?;
        let stored = transaction
            .query_row(
                "SELECT entry_id, selections_json, state, expires_at FROM internal_removal_plans WHERE plan_id = ?1",
                [plan_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
            )
            .optional()
            .map_err(repository_sql_error)?
            .ok_or_else(|| RepositoryError::Other("internal removal plan was not found".into()))?;
        if stored.2 != "prepared" || stored.3.parse::<u128>().unwrap_or(0) < unix_ms() {
            return Err(RepositoryError::Other(
                "internal removal plan expired or was already applied".into(),
            ));
        }
        let selections: Vec<InternalRemovalSelection> =
            serde_json::from_str(&stored.1).map_err(|_| {
                RepositoryError::Corrupt("stored internal removal plan is invalid".into())
            })?;
        for selection in &selections {
            let actual = transaction
                .query_row(
                    "SELECT revision FROM internal_duplicate_groups WHERE group_id = ?1 AND entry_id = ?2 AND resolved = 0",
                    params![selection.group_id, stored.0],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(repository_sql_error)?;
            if actual
                .map(|value| stored_u64(value, "internal group revision"))
                .transpose()?
                != Some(selection.expected_revision)
            {
                return Err(RepositoryError::Other(
                    "internal duplicate group changed after plan preview".into(),
                ));
            }
        }
        let changed = transaction
            .execute(
                "UPDATE internal_removal_plans SET state = 'applying', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE plan_id = ?1 AND state = 'prepared'",
                [plan_id],
            )
            .map_err(repository_sql_error)?;
        if changed != 1 {
            return Err(RepositoryError::Other(
                "internal removal plan changed".into(),
            ));
        }
        let mut sagas = Vec::new();
        for selection in &selections {
            for source_page in &selection.remove_source_pages {
                let row = transaction
                    .query_row(
                        r#"
                            SELECT e.gallery_id, p.relative_path, a.relative_directory
                            FROM download_pages p
                            JOIN download_entries e ON e.entry_id = p.entry_id
                            JOIN download_artifacts a ON a.entry_id = p.entry_id
                            WHERE p.entry_id = ?1 AND p.source_page_number = ?2
                              AND p.state = 'present' AND p.excluded = 0
                              AND a.state = 'complete'
                        "#,
                        params![stored.0, i64::from(*source_page)],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(repository_sql_error)?
                    .ok_or_else(|| {
                        RepositoryError::Other("selected page is no longer present".into())
                    })?;
                let record_id = format!("page-quarantine-{}", Uuid::new_v4());
                let quarantine_relative_path = format!(
                    "{}/.atsumi-page-quarantine/{}/{:04}.webp",
                    row.2, plan_id, source_page
                );
                transaction
                    .execute(
                        r#"
                            INSERT INTO page_quarantine_records (
                                record_id, plan_id, entry_id, gallery_id,
                                source_page_number, original_relative_path,
                                quarantine_relative_path, reason, state,
                                created_at, updated_at
                            ) VALUES (
                                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                                'pending_quarantine',
                                strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                                strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                            )
                        "#,
                        params![
                            record_id,
                            plan_id,
                            stored.0,
                            row.0,
                            i64::from(*source_page),
                            row.1,
                            quarantine_relative_path,
                            reason
                        ],
                    )
                    .map_err(repository_sql_error)?;
                sagas.push(PageQuarantineSaga {
                    record_id,
                    plan_id: plan_id.to_owned(),
                    entry_id: stored.0.clone(),
                    gallery_id: valid_gallery_id(row.0)?,
                    source_page: SourcePageNumber::new(*source_page)
                        .map_err(|error| RepositoryError::Other(error.to_string()))?,
                    original_relative_path: row.1,
                    quarantine_relative_path,
                    reason: reason.to_owned(),
                    state: PageQuarantineState::PendingQuarantine,
                });
            }
        }
        transaction.commit().map_err(repository_sql_error)?;
        Ok(sagas)
    }

    fn internal_removal_complete(
        &self,
        plan_id: &str,
    ) -> Result<Vec<PageQuarantineRecord>, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(repository_sql_error)?;
        let (entry_id, selections_json, state) = transaction
            .query_row(
                "SELECT entry_id, selections_json, state FROM internal_removal_plans WHERE plan_id = ?1",
                [plan_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
            )
            .optional()
            .map_err(repository_sql_error)?
            .ok_or_else(|| RepositoryError::Other("internal removal plan was not found".into()))?;
        if state != "applying" {
            return Err(RepositoryError::Other(
                "internal removal plan is not applying".into(),
            ));
        }
        let selections: Vec<InternalRemovalSelection> = serde_json::from_str(&selections_json)
            .map_err(|_| {
                RepositoryError::Corrupt("stored internal removal plan is invalid".into())
            })?;
        let sagas = read_page_sagas_for_plan(
            &transaction,
            plan_id,
            PageQuarantineState::PendingQuarantine,
        )?;
        if sagas.is_empty() {
            return Err(RepositoryError::Corrupt(
                "applying plan has no pending pages".into(),
            ));
        }
        for saga in &sagas {
            let changed = transaction
                .execute(
                    r#"
                        UPDATE download_pages
                        SET relative_path = ?1, state = 'quarantined', excluded = 1
                        WHERE entry_id = ?2 AND source_page_number = ?3
                          AND state = 'present' AND excluded = 0
                    "#,
                    params![
                        saga.quarantine_relative_path,
                        saga.entry_id,
                        i64::from(saga.source_page.get())
                    ],
                )
                .map_err(repository_sql_error)?;
            if changed != 1 {
                return Err(RepositoryError::Other(
                    "page changed before quarantine commit".into(),
                ));
            }
            transaction
                .execute(
                    "UPDATE page_quarantine_records SET state = 'quarantined', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE record_id = ?1 AND state = 'pending_quarantine'",
                    [&saga.record_id],
                )
                .map_err(repository_sql_error)?;
        }
        for selection in &selections {
            let changed = transaction
                .execute(
                    r#"
                        UPDATE internal_duplicate_groups
                        SET revision = revision + 1, resolved = 1,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                        WHERE group_id = ?1 AND revision = ?2 AND resolved = 0
                    "#,
                    params![selection.group_id, sql_u64(selection.expected_revision)?],
                )
                .map_err(repository_sql_error)?;
            if changed != 1 {
                return Err(RepositoryError::Other(
                    "internal group changed before quarantine commit".into(),
                ));
            }
        }
        transaction
            .execute(
                "UPDATE download_artifacts SET revision = revision + 1 WHERE entry_id = ?1 AND state = 'complete'",
                [&entry_id],
            )
            .map_err(repository_sql_error)?;
        transaction
            .execute(
                "UPDATE internal_removal_plans SET state = 'applied', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE plan_id = ?1 AND state = 'applying'",
                [plan_id],
            )
            .map_err(repository_sql_error)?;
        transaction.commit().map_err(repository_sql_error)?;
        read_page_records(&connection, Some(&entry_id), true).map(|records| {
            records
                .into_iter()
                .filter(|record| record.plan_id == plan_id)
                .collect()
        })
    }

    fn internal_restore_begin(
        &self,
        record_ids: &[String],
    ) -> Result<Vec<PageQuarantineSaga>, RepositoryError> {
        let ids = normalized_ids(record_ids)?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(repository_sql_error)?;
        let mut sagas = Vec::new();
        for record_id in ids {
            let saga = read_page_saga(&transaction, &record_id)?
                .filter(|saga| saga.state == PageQuarantineState::Quarantined)
                .ok_or_else(|| {
                    RepositoryError::Other("page quarantine record is not restorable".into())
                })?;
            let changed = transaction
                .execute(
                    "UPDATE page_quarantine_records SET state = 'pending_restore', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE record_id = ?1 AND state = 'quarantined'",
                    [record_id],
                )
                .map_err(repository_sql_error)?;
            if changed != 1 {
                return Err(RepositoryError::Other(
                    "page quarantine record changed".into(),
                ));
            }
            sagas.push(PageQuarantineSaga {
                state: PageQuarantineState::PendingRestore,
                ..saga
            });
        }
        transaction.commit().map_err(repository_sql_error)?;
        Ok(sagas)
    }

    fn internal_restore_complete(
        &self,
        record_ids: &[String],
    ) -> Result<Vec<PageQuarantineRecord>, RepositoryError> {
        let ids = normalized_ids(record_ids)?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(repository_sql_error)?;
        let mut entries = BTreeSet::new();
        let mut plans = BTreeSet::new();
        for record_id in &ids {
            let saga = read_page_saga(&transaction, record_id)?
                .filter(|saga| saga.state == PageQuarantineState::PendingRestore)
                .ok_or_else(|| RepositoryError::Other("page restore is not pending".into()))?;
            let changed = transaction
                .execute(
                    r#"
                        UPDATE download_pages
                        SET relative_path = ?1, state = 'present', excluded = 0
                        WHERE entry_id = ?2 AND source_page_number = ?3
                          AND state = 'quarantined' AND excluded = 1
                    "#,
                    params![
                        saga.original_relative_path,
                        saga.entry_id,
                        i64::from(saga.source_page.get())
                    ],
                )
                .map_err(repository_sql_error)?;
            if changed != 1 {
                return Err(RepositoryError::Other(
                    "page changed before restore commit".into(),
                ));
            }
            transaction
                .execute(
                    "UPDATE page_quarantine_records SET state = 'restored', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE record_id = ?1 AND state = 'pending_restore'",
                    [record_id],
                )
                .map_err(repository_sql_error)?;
            entries.insert(saga.entry_id);
            plans.insert(saga.plan_id);
        }
        for entry_id in &entries {
            transaction
                .execute(
                    "UPDATE download_artifacts SET revision = revision + 1 WHERE entry_id = ?1 AND state = 'complete'",
                    [entry_id],
                )
                .map_err(repository_sql_error)?;
        }
        for plan_id in plans {
            let active: bool = transaction
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM page_quarantine_records WHERE plan_id = ?1 AND state IN ('pending_quarantine', 'quarantined', 'pending_restore'))",
                    [&plan_id],
                    |row| row.get(0),
                )
                .map_err(repository_sql_error)?;
            if !active {
                for selection in read_plan_selections(&transaction, &plan_id)? {
                    transaction
                        .execute(
                            r#"
                                UPDATE internal_duplicate_groups
                                SET revision = revision + 1, resolved = 0,
                                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                                WHERE group_id = ?1
                            "#,
                            [&selection.group_id],
                        )
                        .map_err(repository_sql_error)?;
                }
            }
        }
        transaction.commit().map_err(repository_sql_error)?;
        let all = read_page_records(&connection, None, true)?;
        Ok(all
            .into_iter()
            .filter(|record| ids.contains(&record.record_id))
            .collect())
    }

    fn internal_pending_page_sagas(&self) -> Result<Vec<PageQuarantineSaga>, RepositoryError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                r#"
                    SELECT record_id, plan_id, entry_id, gallery_id,
                           source_page_number, original_relative_path,
                           quarantine_relative_path, reason, state
                    FROM page_quarantine_records
                    WHERE state IN ('pending_quarantine', 'pending_restore')
                    ORDER BY created_at, record_id
                "#,
            )
            .map_err(repository_sql_error)?;
        let sagas = statement
            .query_map([], row_page_saga)
            .map_err(repository_sql_error)?
            .map(|row| row.map_err(repository_sql_error).and_then(stored_saga))
            .collect();
        sagas
    }

    fn internal_plan_selections(
        &self,
        plan_id: &str,
    ) -> Result<Vec<InternalRemovalSelection>, RepositoryError> {
        let connection = self.connection()?;
        read_plan_selections(&connection, plan_id)
    }
}

fn read_latest_internal_run(
    connection: &Connection,
) -> Result<Option<InternalScanRun>, RepositoryError> {
    connection
        .query_row(
            r#"
                SELECT run_id, revision, state, total_artifacts, scanned_artifacts,
                       total_pages, compared_pairs, groups_found, algorithm_version,
                       skipped_artifacts, skipped_pages, started_at, updated_at,
                       finished_at, error_code, error_message
                FROM internal_duplicate_runs
                ORDER BY started_at DESC, run_id DESC LIMIT 1
            "#,
            [],
            row_internal_run,
        )
        .optional()
        .map_err(repository_sql_error)?
        .map(stored_run)
        .transpose()
}

fn read_internal_run(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<InternalScanRun>, RepositoryError> {
    connection
        .query_row(
            r#"
                SELECT run_id, revision, state, total_artifacts, scanned_artifacts,
                       total_pages, compared_pairs, groups_found, algorithm_version,
                       skipped_artifacts, skipped_pages, started_at, updated_at,
                       finished_at, error_code, error_message
                FROM internal_duplicate_runs WHERE run_id = ?1
            "#,
            [run_id],
            row_internal_run,
        )
        .optional()
        .map_err(repository_sql_error)?
        .map(stored_run)
        .transpose()
}

type StoredRun = (
    String,
    i64,
    String,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn row_internal_run(row: &Row<'_>) -> rusqlite::Result<StoredRun> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
        row.get(15)?,
    ))
}

fn stored_run(row: StoredRun) -> Result<InternalScanRun, RepositoryError> {
    Ok(InternalScanRun {
        run_id: row.0,
        revision: stored_u64(row.1, "internal run revision")?,
        state: InternalScanState::from_database(&row.2)
            .ok_or_else(|| RepositoryError::Corrupt("invalid internal scan state".into()))?,
        total_artifacts: stored_u32(row.3, "internal total artifacts")?,
        scanned_artifacts: stored_u32(row.4, "internal scanned artifacts")?,
        total_pages: stored_u32(row.5, "internal total pages")?,
        compared_pairs: stored_u64(row.6, "internal compared pairs")?,
        groups_found: stored_u32(row.7, "internal groups found")?,
        algorithm_version: stored_u32(row.8, "internal algorithm version")?,
        skipped_artifacts: stored_u32(row.9, "internal skipped artifacts")?,
        skipped_pages: stored_u32(row.10, "internal skipped pages")?,
        started_at: row.11,
        updated_at: row.12,
        finished_at: row.13,
        error_code: row.14,
        error_message: row.15,
    })
}

fn read_latest_internal_skips(
    connection: &Connection,
) -> Result<Vec<InternalScanSkip>, RepositoryError> {
    let Some(run) = read_latest_internal_run(connection)? else {
        return Ok(Vec::new());
    };
    let mut statement = connection.prepare(
        "SELECT entry_id, gallery_id, title, page_count, reason FROM internal_duplicate_scan_skips WHERE run_id = ?1 ORDER BY gallery_id, entry_id"
    ).map_err(repository_sql_error)?;
    let rows = statement
        .query_map([run.run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(repository_sql_error)?
        .map(|row| {
            row.map_err(repository_sql_error).and_then(
                |(entry_id, gallery_id, title, page_count, reason)| {
                    Ok(InternalScanSkip {
                        entry_id,
                        gallery_id: valid_gallery_id(gallery_id)?,
                        title,
                        page_count: stored_u32(page_count, "internal skip page count")?,
                        reason,
                    })
                },
            )
        })
        .collect();
    rows
}

fn read_internal_groups(
    connection: &Connection,
    entry_id: Option<&str>,
) -> Result<Vec<InternalDuplicateGroup>, RepositoryError> {
    let sql = if entry_id.is_some() {
        "SELECT group_id, block_id, sequence_index, revision, entry_id, gallery_id, relation, confidence, recommended_keep_source_page, resolved, created_at, updated_at FROM internal_duplicate_groups WHERE entry_id = ?1 AND resolved = 0 ORDER BY block_id, sequence_index, group_id"
    } else {
        "SELECT group_id, block_id, sequence_index, revision, entry_id, gallery_id, relation, confidence, recommended_keep_source_page, resolved, created_at, updated_at FROM internal_duplicate_groups WHERE resolved = 0 ORDER BY gallery_id, block_id, sequence_index, group_id"
    };
    let mut statement = connection.prepare(sql).map_err(repository_sql_error)?;
    let mut groups = Vec::new();
    if let Some(entry_id) = entry_id {
        let rows = statement
            .query_map([entry_id], row_internal_group)
            .map_err(repository_sql_error)?;
        for row in rows {
            groups.push(stored_group(
                connection,
                row.map_err(repository_sql_error)?,
            )?);
        }
    } else {
        let rows = statement
            .query_map([], row_internal_group)
            .map_err(repository_sql_error)?;
        for row in rows {
            groups.push(stored_group(
                connection,
                row.map_err(repository_sql_error)?,
            )?);
        }
    }
    Ok(groups)
}

type StoredGroup = (
    String,
    String,
    i64,
    i64,
    String,
    i64,
    String,
    f64,
    i64,
    bool,
    String,
    String,
);
fn row_internal_group(row: &Row<'_>) -> rusqlite::Result<StoredGroup> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
    ))
}
fn stored_group(
    connection: &Connection,
    row: StoredGroup,
) -> Result<InternalDuplicateGroup, RepositoryError> {
    let mut statement = connection.prepare("SELECT source_page_number, exact_sha256, visual_similarity, detail_hash_distance, low_information, edition_track_id, edition_track_ordinal FROM internal_duplicate_group_pages WHERE group_id = ?1 ORDER BY source_page_number").map_err(repository_sql_error)?;
    let pages = statement
        .query_map([&row.0], |page| {
            Ok((
                page.get::<_, i64>(0)?,
                page.get::<_, bool>(1)?,
                page.get::<_, f64>(2)?,
                page.get::<_, i64>(3)?,
                page.get::<_, bool>(4)?,
                page.get::<_, Option<String>>(5)?,
                page.get::<_, Option<i64>>(6)?,
            ))
        })
        .map_err(repository_sql_error)?
        .map(|page| {
            let page = page.map_err(repository_sql_error)?;
            let (edition_track_id, edition_track_ordinal) = match (page.5, page.6) {
                (None, None) => (None, None),
                (Some(track_id), Some(ordinal)) if !track_id.trim().is_empty() => (
                    Some(track_id),
                    Some(stored_u32(ordinal, "edition track ordinal")?),
                ),
                _ => {
                    return Err(RepositoryError::Corrupt(
                        "internal duplicate page has an incomplete edition track".into(),
                    ))
                }
            };
            Ok(InternalPageEvidence {
                source_page: stored_u32(page.0, "internal source page")?,
                exact_sha256: page.1,
                visual_similarity: page.2,
                detail_hash_distance: stored_u32(page.3, "detail hash distance")?,
                low_information: page.4,
                edition_track_id,
                edition_track_ordinal,
            })
        })
        .collect::<Result<Vec<_>, RepositoryError>>()?;
    Ok(InternalDuplicateGroup {
        group_id: row.0,
        block_id: row.1,
        sequence_index: stored_u32(row.2, "internal sequence index")?,
        revision: stored_u64(row.3, "internal group revision")?,
        entry_id: row.4,
        gallery_id: valid_gallery_id(row.5)?,
        relation: InternalMatchKind::from_database(&row.6)
            .ok_or_else(|| RepositoryError::Corrupt("invalid internal match relation".into()))?,
        confidence: row.7,
        recommended_keep_source_page: stored_u32(row.8, "recommended keep page")?,
        pages,
        resolved: row.9,
        created_at: row.10,
        updated_at: row.11,
    })
}

fn internal_group_page_numbers(
    connection: &Connection,
    group_id: &str,
) -> Result<BTreeSet<u32>, RepositoryError> {
    let mut statement = connection
        .prepare(
            "SELECT source_page_number FROM internal_duplicate_group_pages WHERE group_id = ?1",
        )
        .map_err(repository_sql_error)?;
    let pages = statement
        .query_map([group_id], |row| row.get::<_, i64>(0))
        .map_err(repository_sql_error)?
        .map(|row| stored_u32(row.map_err(repository_sql_error)?, "internal source page"))
        .collect();
    pages
}

fn read_page_records(
    connection: &Connection,
    entry_id: Option<&str>,
    include_restored: bool,
) -> Result<Vec<PageQuarantineRecord>, RepositoryError> {
    let mut sql = String::from("SELECT record_id, plan_id, entry_id, gallery_id, source_page_number, original_relative_path, quarantine_relative_path, reason, state, created_at, updated_at FROM page_quarantine_records WHERE 1 = 1");
    if entry_id.is_some() {
        sql.push_str(" AND entry_id = ?1");
    }
    if !include_restored {
        sql.push_str(" AND state != 'restored'");
    }
    sql.push_str(" ORDER BY created_at DESC, record_id DESC");
    let mut statement = connection.prepare(&sql).map_err(repository_sql_error)?;
    let convert = |row: Result<StoredPageRecord, rusqlite::Error>| {
        row.map_err(repository_sql_error)
            .and_then(stored_page_record)
    };
    if let Some(entry_id) = entry_id {
        statement
            .query_map([entry_id], row_page_record)
            .map_err(repository_sql_error)?
            .map(convert)
            .collect()
    } else {
        statement
            .query_map([], row_page_record)
            .map_err(repository_sql_error)?
            .map(convert)
            .collect()
    }
}

type StoredPageRecord = (
    String,
    String,
    String,
    i64,
    i64,
    String,
    String,
    String,
    String,
    String,
    String,
);
fn row_page_record(row: &Row<'_>) -> rusqlite::Result<StoredPageRecord> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
    ))
}
fn stored_page_record(row: StoredPageRecord) -> Result<PageQuarantineRecord, RepositoryError> {
    Ok(PageQuarantineRecord {
        record_id: row.0,
        plan_id: row.1,
        entry_id: row.2,
        gallery_id: valid_gallery_id(row.3)?,
        source_page: stored_u32(row.4, "page quarantine source page")?,
        original_relative_path: row.5,
        quarantine_relative_path: row.6,
        reason: row.7,
        state: PageQuarantineState::from_database(&row.8)
            .ok_or_else(|| RepositoryError::Corrupt("invalid page quarantine state".into()))?,
        created_at: row.9,
        updated_at: row.10,
    })
}

type StoredSaga = (
    String,
    String,
    String,
    i64,
    i64,
    String,
    String,
    String,
    String,
);
fn row_page_saga(row: &Row<'_>) -> rusqlite::Result<StoredSaga> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}
fn stored_saga(row: StoredSaga) -> Result<PageQuarantineSaga, RepositoryError> {
    let source_page = stored_u32(row.4, "page quarantine source page")?;
    Ok(PageQuarantineSaga {
        record_id: row.0,
        plan_id: row.1,
        entry_id: row.2,
        gallery_id: valid_gallery_id(row.3)?,
        source_page: SourcePageNumber::new(source_page)
            .map_err(|error| RepositoryError::Other(error.to_string()))?,
        original_relative_path: row.5,
        quarantine_relative_path: row.6,
        reason: row.7,
        state: PageQuarantineState::from_database(&row.8)
            .ok_or_else(|| RepositoryError::Corrupt("invalid page quarantine state".into()))?,
    })
}
fn read_page_saga(
    connection: &Connection,
    record_id: &str,
) -> Result<Option<PageQuarantineSaga>, RepositoryError> {
    connection.query_row("SELECT record_id, plan_id, entry_id, gallery_id, source_page_number, original_relative_path, quarantine_relative_path, reason, state FROM page_quarantine_records WHERE record_id = ?1", [record_id], row_page_saga).optional().map_err(repository_sql_error)?.map(stored_saga).transpose()
}
fn read_page_sagas_for_plan(
    connection: &Connection,
    plan_id: &str,
    state: PageQuarantineState,
) -> Result<Vec<PageQuarantineSaga>, RepositoryError> {
    let mut statement = connection.prepare("SELECT record_id, plan_id, entry_id, gallery_id, source_page_number, original_relative_path, quarantine_relative_path, reason, state FROM page_quarantine_records WHERE plan_id = ?1 AND state = ?2 ORDER BY source_page_number").map_err(repository_sql_error)?;
    let sagas = statement
        .query_map(params![plan_id, state.as_str()], row_page_saga)
        .map_err(repository_sql_error)?
        .map(|row| row.map_err(repository_sql_error).and_then(stored_saga))
        .collect();
    sagas
}

fn read_plan_selections(
    connection: &Connection,
    plan_id: &str,
) -> Result<Vec<InternalRemovalSelection>, RepositoryError> {
    let json = connection
        .query_row(
            "SELECT selections_json FROM internal_removal_plans WHERE plan_id = ?1",
            [plan_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(repository_sql_error)?
        .ok_or_else(|| RepositoryError::Other("internal removal plan was not found".into()))?;
    serde_json::from_str(&json)
        .map_err(|_| RepositoryError::Corrupt("stored internal removal plan is invalid".into()))
}

fn normalized_ids(ids: &[String]) -> Result<Vec<String>, RepositoryError> {
    let ids = ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if ids.is_empty() || ids.len() > 200 {
        return Err(RepositoryError::Other(
            "recordIds must contain 1..=200 unique values".into(),
        ));
    }
    Ok(ids)
}
fn valid_gallery_id(value: i64) -> Result<GalleryId, RepositoryError> {
    GalleryId::new(value).map_err(|error| RepositoryError::Corrupt(error.to_string()))
}
fn stored_u64(value: i64, label: &str) -> Result<u64, RepositoryError> {
    u64::try_from(value).map_err(|_| RepositoryError::Corrupt(format!("{label} is negative")))
}
fn stored_u32(value: i64, label: &str) -> Result<u32, RepositoryError> {
    u32::try_from(value).map_err(|_| RepositoryError::Corrupt(format!("{label} is outside u32")))
}
fn sql_u64(value: u64) -> Result<i64, RepositoryError> {
    i64::try_from(value)
        .map_err(|_| RepositoryError::Other("numeric value exceeds SQLite range".into()))
}
fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}
fn repository_sql_error(error: rusqlite::Error) -> RepositoryError {
    RepositoryError::Other(format!(
        "SQLite internal duplicate operation failed: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use crate::{
        application::InternalDuplicateRepository,
        domain::{GalleryId, InternalScanState},
        infrastructure::SqliteRepository,
    };

    #[test]
    fn selected_scan_completion_resolves_only_the_scanned_gallery() {
        let repository = SqliteRepository::open_in_memory().expect("open repository");
        let connection = repository.connection().expect("lock repository");
        for gallery_id in [101_i64, 202_i64] {
            connection
                .execute(
                    "INSERT INTO galleries (gallery_id, revision, title, source_page_count) VALUES (?1, 0, ?2, 2)",
                    params![gallery_id, format!("Gallery {gallery_id}")],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO download_entries (entry_id, gallery_id, revision, state, progress, created_at, updated_at) VALUES (?1, ?2, 0, 'completed', 100, 'now', 'now')",
                    params![format!("entry-{gallery_id}"), gallery_id],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO internal_duplicate_runs (run_id, revision, state, profile_version, total_artifacts, scanned_artifacts, total_pages, compared_pairs, groups_found, started_at, updated_at, finished_at) VALUES (?1, 0, 'completed', 1, 1, 1, 2, 1, 1, 'now', 'now', 'now')",
                    [format!("old-run-{gallery_id}")],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO internal_duplicate_groups (group_id, block_id, sequence_index, revision, last_seen_run_id, entry_id, gallery_id, relation, confidence, recommended_keep_source_page, resolved, created_at, updated_at) VALUES (?1, ?2, 0, 0, ?3, ?4, ?5, 'exact', 1, 1, 0, 'now', 'now')",
                    params![
                        format!("group-{gallery_id}"),
                        format!("block-{gallery_id}"),
                        format!("old-run-{gallery_id}"),
                        format!("entry-{gallery_id}"),
                        gallery_id,
                    ],
                )
                .unwrap();
        }
        drop(connection);

        let run = repository
            .internal_scan_start(1, 3, 1, 2, &[])
            .expect("start selected run");
        repository
            .internal_scan_finish(
                &run.run_id,
                InternalScanState::Completed,
                None,
                None,
                &[GalleryId::new(101).unwrap()],
            )
            .expect("finish selected run");

        let connection = repository.connection().expect("lock repository");
        let selected_resolved: bool = connection
            .query_row(
                "SELECT resolved FROM internal_duplicate_groups WHERE group_id = 'group-101'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let unselected_resolved: bool = connection
            .query_row(
                "SELECT resolved FROM internal_duplicate_groups WHERE group_id = 'group-202'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(selected_resolved);
        assert!(!unselected_resolved);
    }
}
