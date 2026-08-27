use std::collections::BTreeMap;

use rusqlite::{params, Connection};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "settings_and_window_placement",
        sql: r#"
            CREATE TABLE settings (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                download_root TEXT NOT NULL,
                max_columns INTEGER NOT NULL CHECK (max_columns BETWEEN 1 AND 4),
                preview_width INTEGER NOT NULL CHECK (preview_width BETWEEN 160 AND 360),
                cache_limit_gb INTEGER NOT NULL CHECK (cache_limit_gb BETWEEN 1 AND 30),
                concurrent_image_requests INTEGER NOT NULL CHECK (concurrent_image_requests BETWEEN 1 AND 30),
                request_start_interval_ms INTEGER NOT NULL CHECK (request_start_interval_ms BETWEEN 0 AND 5000)
            ) STRICT;

            INSERT INTO settings (
                singleton,
                revision,
                download_root,
                max_columns,
                preview_width,
                cache_limit_gb,
                concurrent_image_requests,
                request_start_interval_ms
            ) VALUES (1, 0, '', 3, 220, 10, 5, 25);

            CREATE TABLE window_placement (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                x INTEGER,
                y INTEGER,
                width INTEGER NOT NULL CHECK (width BETWEEN 1 AND 32768),
                height INTEGER NOT NULL CHECK (height BETWEEN 1 AND 32768),
                maximized INTEGER NOT NULL CHECK (maximized IN (0, 1))
            ) STRICT;

            INSERT INTO window_placement (
                singleton, revision, x, y, width, height, maximized
            ) VALUES (1, 0, NULL, NULL, 1280, 820, 0);
        "#,
    },
    Migration {
        version: 2,
        name: "mock_job_event_foundation",
        sql: r#"
            CREATE TABLE download_entries (
                entry_id TEXT PRIMARY KEY,
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                state TEXT NOT NULL CHECK (state IN (
                    'queued', 'resolving_metadata', 'downloading', 'hashing',
                    'verifying', 'retry_wait', 'review_required', 'interrupted',
                    'failed', 'completed', 'quarantined'
                )),
                progress REAL NOT NULL CHECK (progress BETWEEN 0.0 AND 100.0)
            ) STRICT;

            CREATE TABLE download_jobs (
                job_id TEXT PRIMARY KEY,
                request_id TEXT NOT NULL UNIQUE,
                entry_id TEXT NOT NULL UNIQUE REFERENCES download_entries(entry_id) ON DELETE CASCADE,
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                state TEXT NOT NULL CHECK (state IN (
                    'queued', 'resolving_metadata', 'downloading', 'hashing',
                    'verifying', 'retry_wait', 'review_required', 'interrupted',
                    'failed', 'completed', 'quarantined'
                )),
                completed_units INTEGER NOT NULL CHECK (completed_units >= 0),
                total_units INTEGER NOT NULL CHECK (total_units > 0)
            ) STRICT;

            CREATE INDEX download_jobs_gallery_id_idx ON download_jobs(gallery_id);
        "#,
    },
    Migration {
        version: 3,
        name: "gallery_and_artifact_foundation",
        sql: r#"
            CREATE TABLE settings_v3 (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                download_root TEXT NOT NULL,
                max_columns INTEGER NOT NULL CHECK (max_columns BETWEEN 1 AND 4),
                preview_width INTEGER NOT NULL CHECK (preview_width BETWEEN 160 AND 360),
                cache_limit_gb INTEGER NOT NULL CHECK (cache_limit_gb BETWEEN 1 AND 30),
                concurrent_image_requests INTEGER NOT NULL CHECK (concurrent_image_requests BETWEEN 1 AND 30),
                request_start_interval_ms INTEGER NOT NULL CHECK (request_start_interval_ms BETWEEN 0 AND 5000)
            ) STRICT;

            INSERT INTO settings_v3 (
                singleton, revision, download_root, max_columns, preview_width,
                cache_limit_gb, concurrent_image_requests, request_start_interval_ms
            )
            SELECT
                singleton,
                revision,
                download_root,
                MAX(1, MIN(4, max_columns)),
                MAX(160, MIN(360, preview_width)),
                MAX(1, MIN(30, cache_limit_gb)),
                MAX(1, MIN(30, concurrent_image_requests)),
                MAX(0, MIN(5000, request_start_interval_ms))
            FROM settings;

            DROP TABLE settings;
            ALTER TABLE settings_v3 RENAME TO settings;

            CREATE TABLE galleries (
                gallery_id INTEGER PRIMARY KEY CHECK (gallery_id > 0),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                title TEXT NOT NULL CHECK (length(trim(title)) > 0),
                primary_artist TEXT,
                source_page_count INTEGER NOT NULL CHECK (source_page_count > 0)
            ) STRICT;

            CREATE UNIQUE INDEX download_entries_identity_idx
                ON download_entries(entry_id, gallery_id);

            CREATE TABLE download_artifacts (
                entry_id TEXT PRIMARY KEY,
                gallery_id INTEGER NOT NULL,
                revision INTEGER NOT NULL CHECK (revision >= 0),
                relative_directory TEXT NOT NULL UNIQUE
                    CHECK (length(trim(relative_directory)) > 0),
                expected_page_count INTEGER NOT NULL CHECK (expected_page_count > 0),
                state TEXT NOT NULL CHECK (state IN (
                    'incomplete', 'complete', 'missing_artifacts', 'quarantined'
                )),
                UNIQUE (entry_id, gallery_id),
                FOREIGN KEY (gallery_id)
                    REFERENCES galleries(gallery_id) ON DELETE RESTRICT,
                FOREIGN KEY (entry_id, gallery_id)
                    REFERENCES download_entries(entry_id, gallery_id) ON DELETE CASCADE
            ) STRICT;

            CREATE TABLE download_pages (
                entry_id TEXT NOT NULL,
                gallery_id INTEGER NOT NULL,
                source_page_number INTEGER NOT NULL CHECK (source_page_number > 0),
                relative_path TEXT NOT NULL CHECK (length(trim(relative_path)) > 0),
                state TEXT NOT NULL CHECK (state IN (
                    'pending', 'present', 'missing', 'quarantined'
                )),
                byte_length INTEGER CHECK (byte_length > 0),
                PRIMARY KEY (entry_id, source_page_number),
                UNIQUE (entry_id, relative_path),
                FOREIGN KEY (entry_id, gallery_id)
                    REFERENCES download_artifacts(entry_id, gallery_id) ON DELETE CASCADE
            ) STRICT;

            CREATE INDEX download_pages_gallery_page_idx
                ON download_pages(gallery_id, source_page_number);
        "#,
    },
    Migration {
        version: 4,
        name: "gallery_primary_group",
        sql: r#"
            ALTER TABLE galleries ADD COLUMN primary_group TEXT;
        "#,
    },
    Migration {
        version: 5,
        name: "download_queue_contract",
        sql: r#"
            ALTER TABLE download_entries ADD COLUMN review_kind TEXT
                CHECK (review_kind IS NULL OR review_kind IN (
                    'gallery_duplicate', 'internal_pages'
                ));
            ALTER TABLE download_entries ADD COLUMN review_id TEXT;

            CREATE TABLE download_queue_requests (
                request_id TEXT PRIMARY KEY CHECK (length(trim(request_id)) > 0),
                normalized_galleries TEXT NOT NULL
                    CHECK (length(normalized_galleries) > 0)
            ) STRICT;

            CREATE TABLE download_queue_request_entries (
                request_id TEXT NOT NULL,
                position INTEGER NOT NULL CHECK (position >= 0),
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                entry_id TEXT NOT NULL,
                response_state TEXT NOT NULL CHECK (response_state IN (
                    'queued', 'resolving_metadata', 'downloading', 'hashing',
                    'verifying', 'retry_wait', 'review_required', 'interrupted',
                    'failed', 'completed', 'quarantined'
                )),
                response_progress REAL NOT NULL
                    CHECK (response_progress BETWEEN 0.0 AND 100.0),
                response_review_kind TEXT CHECK (
                    response_review_kind IS NULL OR response_review_kind IN (
                        'gallery_duplicate', 'internal_pages'
                    )
                ),
                response_review_id TEXT,
                PRIMARY KEY (request_id, position),
                UNIQUE (request_id, gallery_id),
                FOREIGN KEY (request_id)
                    REFERENCES download_queue_requests(request_id) ON DELETE CASCADE,
                FOREIGN KEY (entry_id)
                    REFERENCES download_entries(entry_id) ON DELETE RESTRICT
            ) STRICT;

            UPDATE download_jobs
            SET revision = revision + 1,
                state = 'interrupted'
            WHERE entry_id IN (
                SELECT duplicate.entry_id
                FROM download_entries duplicate
                WHERE duplicate.state IN (
                    'queued', 'resolving_metadata', 'downloading', 'hashing',
                    'verifying', 'retry_wait'
                )
                AND EXISTS (
                    SELECT 1
                    FROM download_entries keeper
                    WHERE keeper.gallery_id = duplicate.gallery_id
                      AND keeper.rowid < duplicate.rowid
                      AND keeper.state IN (
                          'queued', 'resolving_metadata', 'downloading', 'hashing',
                          'verifying', 'retry_wait'
                      )
                )
            );
            UPDATE download_entries
            SET revision = revision + 1,
                state = 'interrupted'
            WHERE state IN (
                    'queued', 'resolving_metadata', 'downloading', 'hashing',
                    'verifying', 'retry_wait'
                )
              AND EXISTS (
                    SELECT 1
                    FROM download_entries keeper
                    WHERE keeper.gallery_id = download_entries.gallery_id
                      AND keeper.rowid < download_entries.rowid
                      AND keeper.state IN (
                          'queued', 'resolving_metadata', 'downloading', 'hashing',
                          'verifying', 'retry_wait'
                      )
                );

            CREATE UNIQUE INDEX download_entries_active_gallery_idx
                ON download_entries(gallery_id)
                WHERE state IN (
                    'queued', 'resolving_metadata', 'downloading', 'hashing',
                    'verifying', 'retry_wait'
                );
            CREATE INDEX download_queue_request_entries_entry_idx
                ON download_queue_request_entries(entry_id);
        "#,
    },
    Migration {
        version: 6,
        name: "download_queue_response_revision",
        sql: r#"
            ALTER TABLE download_queue_request_entries
            ADD COLUMN response_revision INTEGER NOT NULL DEFAULT 0
                CHECK (response_revision >= 0);
        "#,
    },
    Migration {
        version: 7,
        name: "download_lifecycle_and_cancelled_state",
        sql: r#"
            PRAGMA defer_foreign_keys = ON;

            CREATE TABLE download_entries_v7 (
                entry_id TEXT PRIMARY KEY,
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                state TEXT NOT NULL CHECK (state IN (
                    'queued', 'resolving_metadata', 'downloading', 'hashing',
                    'verifying', 'retry_wait', 'review_required', 'interrupted',
                    'failed', 'completed', 'quarantined', 'cancelled'
                )),
                progress REAL NOT NULL CHECK (progress BETWEEN 0.0 AND 100.0),
                review_kind TEXT CHECK (review_kind IS NULL OR review_kind IN (
                    'gallery_duplicate', 'internal_pages'
                )),
                review_id TEXT,
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                UNIQUE (entry_id, gallery_id)
            ) STRICT;

            INSERT INTO download_entries_v7 (
                entry_id, gallery_id, revision, state, progress,
                review_kind, review_id, created_at, updated_at
            )
            SELECT
                entry_id, gallery_id, revision, state, progress,
                review_kind, review_id,
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            FROM download_entries;

            CREATE TABLE download_jobs_v7 (
                job_id TEXT PRIMARY KEY,
                request_id TEXT NOT NULL UNIQUE,
                entry_id TEXT NOT NULL
                    REFERENCES download_entries_v7(entry_id) ON DELETE CASCADE,
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                state TEXT NOT NULL CHECK (state IN (
                    'queued', 'resolving_metadata', 'downloading', 'hashing',
                    'verifying', 'retry_wait', 'review_required', 'interrupted',
                    'failed', 'completed', 'quarantined', 'cancelled'
                )),
                completed_units INTEGER NOT NULL CHECK (completed_units >= 0),
                total_units INTEGER NOT NULL CHECK (total_units > 0),
                attempt INTEGER NOT NULL CHECK (attempt > 0),
                last_error_code TEXT,
                last_error_message TEXT,
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                started_at TEXT,
                finished_at TEXT
            ) STRICT;

            INSERT INTO download_jobs_v7 (
                job_id, request_id, entry_id, gallery_id, revision, state,
                completed_units, total_units, attempt,
                last_error_code, last_error_message,
                created_at, updated_at, started_at, finished_at
            )
            SELECT
                job_id, request_id, entry_id, gallery_id, revision, state,
                completed_units, total_units, 1,
                CASE WHEN state = 'interrupted' THEN 'JOB_INTERRUPTED' END,
                CASE WHEN state = 'interrupted'
                    THEN 'The application stopped before the job reached a terminal state'
                END,
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                CASE WHEN state != 'queued'
                    THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                END,
                CASE WHEN state IN (
                    'review_required', 'interrupted', 'failed', 'completed', 'quarantined'
                ) THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END
            FROM download_jobs;

            CREATE TABLE download_queue_request_entries_v7 (
                request_id TEXT NOT NULL,
                position INTEGER NOT NULL CHECK (position >= 0),
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                entry_id TEXT NOT NULL,
                response_state TEXT NOT NULL CHECK (response_state IN (
                    'queued', 'resolving_metadata', 'downloading', 'hashing',
                    'verifying', 'retry_wait', 'review_required', 'interrupted',
                    'failed', 'completed', 'quarantined', 'cancelled'
                )),
                response_progress REAL NOT NULL
                    CHECK (response_progress BETWEEN 0.0 AND 100.0),
                response_review_kind TEXT CHECK (
                    response_review_kind IS NULL OR response_review_kind IN (
                        'gallery_duplicate', 'internal_pages'
                    )
                ),
                response_review_id TEXT,
                response_revision INTEGER NOT NULL CHECK (response_revision >= 0),
                PRIMARY KEY (request_id, position),
                UNIQUE (request_id, gallery_id),
                FOREIGN KEY (request_id)
                    REFERENCES download_queue_requests(request_id) ON DELETE CASCADE,
                FOREIGN KEY (entry_id)
                    REFERENCES download_entries_v7(entry_id) ON DELETE RESTRICT
            ) STRICT;

            INSERT INTO download_queue_request_entries_v7 (
                request_id, position, gallery_id, entry_id,
                response_state, response_progress,
                response_review_kind, response_review_id, response_revision
            )
            SELECT
                request_id, position, gallery_id, entry_id,
                response_state, response_progress,
                response_review_kind, response_review_id, response_revision
            FROM download_queue_request_entries;

            CREATE TABLE download_artifacts_v7 (
                entry_id TEXT PRIMARY KEY,
                gallery_id INTEGER NOT NULL,
                revision INTEGER NOT NULL CHECK (revision >= 0),
                relative_directory TEXT NOT NULL UNIQUE
                    CHECK (length(trim(relative_directory)) > 0),
                expected_page_count INTEGER NOT NULL CHECK (expected_page_count > 0),
                state TEXT NOT NULL CHECK (state IN (
                    'incomplete', 'complete', 'missing_artifacts', 'quarantined'
                )),
                UNIQUE (entry_id, gallery_id),
                FOREIGN KEY (gallery_id)
                    REFERENCES galleries(gallery_id) ON DELETE RESTRICT,
                FOREIGN KEY (entry_id, gallery_id)
                    REFERENCES download_entries_v7(entry_id, gallery_id) ON DELETE CASCADE
            ) STRICT;

            INSERT INTO download_artifacts_v7 (
                entry_id, gallery_id, revision, relative_directory,
                expected_page_count, state
            )
            SELECT
                entry_id, gallery_id, revision, relative_directory,
                expected_page_count, state
            FROM download_artifacts;

            CREATE TABLE download_pages_v7 (
                entry_id TEXT NOT NULL,
                gallery_id INTEGER NOT NULL,
                source_page_number INTEGER NOT NULL CHECK (source_page_number > 0),
                relative_path TEXT NOT NULL CHECK (length(trim(relative_path)) > 0),
                state TEXT NOT NULL CHECK (state IN (
                    'pending', 'present', 'missing', 'quarantined'
                )),
                byte_length INTEGER CHECK (byte_length > 0),
                PRIMARY KEY (entry_id, source_page_number),
                UNIQUE (entry_id, relative_path),
                FOREIGN KEY (entry_id, gallery_id)
                    REFERENCES download_artifacts_v7(entry_id, gallery_id) ON DELETE CASCADE
            ) STRICT;

            INSERT INTO download_pages_v7 (
                entry_id, gallery_id, source_page_number,
                relative_path, state, byte_length
            )
            SELECT
                entry_id, gallery_id, source_page_number,
                relative_path, state, byte_length
            FROM download_pages;

            DROP TABLE download_pages;
            DROP TABLE download_artifacts;
            DROP TABLE download_queue_request_entries;
            DROP TABLE download_jobs;
            DROP TABLE download_entries;

            ALTER TABLE download_entries_v7 RENAME TO download_entries;
            ALTER TABLE download_jobs_v7 RENAME TO download_jobs;
            ALTER TABLE download_queue_request_entries_v7
                RENAME TO download_queue_request_entries;
            ALTER TABLE download_artifacts_v7 RENAME TO download_artifacts;
            ALTER TABLE download_pages_v7 RENAME TO download_pages;

            CREATE INDEX download_jobs_gallery_id_idx ON download_jobs(gallery_id);
            CREATE UNIQUE INDEX download_entries_active_gallery_idx
                ON download_entries(gallery_id)
                WHERE state IN (
                    'queued', 'resolving_metadata', 'downloading', 'hashing',
                    'verifying', 'retry_wait'
                );
            CREATE INDEX download_queue_request_entries_entry_idx
                ON download_queue_request_entries(entry_id);
            CREATE INDEX download_pages_gallery_page_idx
                ON download_pages(gallery_id, source_page_number);

            CREATE TABLE download_attempts (
                job_id TEXT NOT NULL
                    REFERENCES download_jobs(job_id) ON DELETE CASCADE,
                attempt INTEGER NOT NULL CHECK (attempt > 0),
                started_at TEXT NOT NULL CHECK (length(started_at) > 0),
                finished_at TEXT,
                outcome_state TEXT CHECK (
                    outcome_state IS NULL OR outcome_state IN (
                        'queued', 'resolving_metadata', 'downloading', 'hashing',
                        'verifying', 'retry_wait', 'review_required', 'interrupted',
                        'failed', 'completed', 'quarantined', 'cancelled'
                    )
                ),
                error_code TEXT,
                error_message TEXT,
                PRIMARY KEY (job_id, attempt)
            ) STRICT;

            INSERT INTO download_attempts (
                job_id, attempt, started_at, finished_at,
                outcome_state, error_code, error_message
            )
            SELECT
                job_id, attempt, created_at, finished_at,
                CASE WHEN finished_at IS NOT NULL THEN state END,
                last_error_code, last_error_message
            FROM download_jobs;
        "#,
    },
    Migration {
        version: 8,
        name: "verified_artifact_pipeline",
        sql: r#"
            ALTER TABLE download_artifacts
            ADD COLUMN manifest_relative_path TEXT
                CHECK (
                    manifest_relative_path IS NULL
                    OR length(trim(manifest_relative_path)) > 0
                );
            ALTER TABLE download_artifacts
            ADD COLUMN manifest_schema_version INTEGER
                CHECK (manifest_schema_version IS NULL OR manifest_schema_version > 0);
            ALTER TABLE download_artifacts
            ADD COLUMN writer_version TEXT
                CHECK (writer_version IS NULL OR length(trim(writer_version)) > 0);
            ALTER TABLE download_artifacts
            ADD COLUMN hash_profile_version INTEGER NOT NULL DEFAULT 1
                CHECK (hash_profile_version > 0);
            ALTER TABLE download_artifacts
            ADD COLUMN completed_at TEXT;

            ALTER TABLE download_pages
            ADD COLUMN sha256 TEXT
                CHECK (sha256 IS NULL OR length(sha256) = 64);
            ALTER TABLE download_pages
            ADD COLUMN storage_format TEXT
                CHECK (storage_format IS NULL OR storage_format = 'webp');
            ALTER TABLE download_pages
            ADD COLUMN source_revision TEXT
                CHECK (source_revision IS NULL OR length(trim(source_revision)) > 0);
            ALTER TABLE download_pages
            ADD COLUMN verified_at TEXT;
            ALTER TABLE download_pages
            ADD COLUMN excluded INTEGER NOT NULL DEFAULT 0
                CHECK (excluded IN (0, 1));

            CREATE TABLE download_page_attempts (
                job_id TEXT NOT NULL,
                job_attempt INTEGER NOT NULL CHECK (job_attempt > 0),
                source_page_number INTEGER NOT NULL CHECK (source_page_number > 0),
                candidate_index INTEGER NOT NULL CHECK (candidate_index >= 0),
                started_at TEXT NOT NULL CHECK (length(started_at) > 0),
                finished_at TEXT,
                outcome TEXT CHECK (
                    outcome IS NULL OR outcome IN (
                        'succeeded', 'failed', 'cancelled'
                    )
                ),
                error_code TEXT,
                error_message TEXT,
                bytes_received INTEGER CHECK (bytes_received IS NULL OR bytes_received >= 0),
                PRIMARY KEY (
                    job_id, job_attempt, source_page_number, candidate_index
                ),
                FOREIGN KEY (job_id, job_attempt)
                    REFERENCES download_attempts(job_id, attempt) ON DELETE CASCADE
            ) STRICT;

            CREATE INDEX download_page_attempts_page_idx
                ON download_page_attempts(job_id, source_page_number, job_attempt);

            CREATE TABLE quarantine_records (
                record_id TEXT PRIMARY KEY,
                entry_id TEXT NOT NULL
                    REFERENCES download_entries(entry_id) ON DELETE RESTRICT,
                original_relative_path TEXT NOT NULL
                    CHECK (length(trim(original_relative_path)) > 0),
                quarantine_relative_path TEXT NOT NULL UNIQUE
                    CHECK (length(trim(quarantine_relative_path)) > 0),
                reason TEXT NOT NULL CHECK (length(trim(reason)) > 0),
                state TEXT NOT NULL CHECK (state IN ('quarantined', 'restored')),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                restored_at TEXT
            ) STRICT;

            CREATE INDEX quarantine_records_entry_idx
                ON quarantine_records(entry_id, state);
        "#,
    },
    Migration {
        version: 9,
        name: "crash_safe_quarantine_saga",
        sql: r#"
            ALTER TABLE quarantine_records RENAME TO quarantine_records_v8;

            CREATE TABLE quarantine_records (
                record_id TEXT PRIMARY KEY,
                entry_id TEXT NOT NULL
                    REFERENCES download_entries(entry_id) ON DELETE RESTRICT,
                original_relative_path TEXT NOT NULL
                    CHECK (length(trim(original_relative_path)) > 0),
                quarantine_relative_path TEXT NOT NULL UNIQUE
                    CHECK (length(trim(quarantine_relative_path)) > 0),
                reason TEXT NOT NULL CHECK (length(trim(reason)) > 0),
                state TEXT NOT NULL CHECK (state IN (
                    'pending_quarantine', 'quarantined',
                    'pending_restore', 'restored'
                )),
                original_entry_state TEXT NOT NULL DEFAULT 'completed'
                    CHECK (original_entry_state = 'completed'),
                original_artifact_state TEXT NOT NULL DEFAULT 'complete'
                    CHECK (original_artifact_state = 'complete'),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                restored_at TEXT
            ) STRICT;

            INSERT INTO quarantine_records (
                record_id, entry_id, original_relative_path,
                quarantine_relative_path, reason, state,
                original_entry_state, original_artifact_state,
                created_at, restored_at
            )
            SELECT
                record_id, entry_id, original_relative_path,
                quarantine_relative_path, reason, state,
                'completed', 'complete', created_at, restored_at
            FROM quarantine_records_v8;

            DROP TABLE quarantine_records_v8;

            CREATE INDEX quarantine_records_entry_idx
                ON quarantine_records(entry_id, state);
            CREATE UNIQUE INDEX quarantine_records_active_entry_idx
                ON quarantine_records(entry_id)
                WHERE state IN (
                    'pending_quarantine', 'quarantined', 'pending_restore'
                );
        "#,
    },
    Migration {
        version: 10,
        name: "favorites_search_history_and_auto_find",
        sql: r#"
            CREATE TABLE favorites (
                namespace TEXT NOT NULL CHECK (namespace IN (
                    'artist', 'group', 'series', 'character', 'tag'
                )),
                value TEXT NOT NULL COLLATE NOCASE
                    CHECK (length(trim(value)) BETWEEN 1 AND 200),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                PRIMARY KEY (namespace, value)
            ) STRICT;

            CREATE TABLE search_history (
                history_id INTEGER PRIMARY KEY,
                fingerprint TEXT NOT NULL UNIQUE CHECK (length(fingerprint) = 64),
                text TEXT NOT NULL CHECK (length(text) <= 500),
                include_tags_json TEXT NOT NULL CHECK (json_valid(include_tags_json)),
                exclude_tags_json TEXT NOT NULL CHECK (json_valid(exclude_tags_json)),
                languages_json TEXT NOT NULL CHECK (json_valid(languages_json)),
                sort TEXT NOT NULL CHECK (sort IN (
                    'recent', 'popular_today', 'popular_week',
                    'popular_month', 'popular_year', 'random'
                )),
                page_size INTEGER NOT NULL CHECK (page_size BETWEEN 1 AND 200),
                use_count INTEGER NOT NULL CHECK (use_count > 0),
                last_used_at TEXT NOT NULL CHECK (length(last_used_at) > 0)
            ) STRICT;

            CREATE INDEX search_history_recent_idx
                ON search_history(last_used_at DESC, history_id DESC);

            CREATE TABLE auto_find_runs (
                run_id TEXT PRIMARY KEY CHECK (length(trim(run_id)) > 0),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                state TEXT NOT NULL CHECK (state IN (
                    'running', 'completed', 'failed', 'cancelled'
                )),
                total_favorites INTEGER NOT NULL CHECK (total_favorites >= 0),
                completed_favorites INTEGER NOT NULL
                    CHECK (completed_favorites BETWEEN 0 AND total_favorites),
                candidates_found INTEGER NOT NULL CHECK (candidates_found >= 0),
                started_at TEXT NOT NULL CHECK (length(started_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                finished_at TEXT,
                error_code TEXT,
                error_message TEXT
            ) STRICT;

            CREATE UNIQUE INDEX auto_find_one_running_idx
                ON auto_find_runs(state)
                WHERE state = 'running';
            CREATE INDEX auto_find_runs_recent_idx
                ON auto_find_runs(started_at DESC, run_id DESC);

            CREATE TABLE auto_find_candidates (
                run_id TEXT NOT NULL
                    REFERENCES auto_find_runs(run_id) ON DELETE CASCADE,
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                title TEXT NOT NULL CHECK (length(trim(title)) > 0),
                artist TEXT NOT NULL CHECK (length(trim(artist)) > 0),
                group_name TEXT,
                pages INTEGER NOT NULL CHECK (pages > 0),
                language TEXT NOT NULL CHECK (language IN (
                    'korean', 'japanese', 'chinese', 'english'
                )),
                tags_json TEXT NOT NULL CHECK (json_valid(tags_json)),
                published_rank INTEGER NOT NULL CHECK (published_rank >= 0),
                popularity INTEGER NOT NULL CHECK (popularity >= 0),
                thumbnail_key TEXT,
                thumbnail_width INTEGER NOT NULL CHECK (thumbnail_width > 0),
                thumbnail_height INTEGER NOT NULL CHECK (thumbnail_height > 0),
                favorite_namespace TEXT NOT NULL CHECK (favorite_namespace IN (
                    'artist', 'group', 'series', 'character', 'tag'
                )),
                favorite_value TEXT NOT NULL CHECK (length(trim(favorite_value)) > 0),
                discovered_at TEXT NOT NULL CHECK (length(discovered_at) > 0),
                PRIMARY KEY (run_id, gallery_id)
            ) STRICT;

            CREATE INDEX auto_find_candidates_gallery_idx
                ON auto_find_candidates(gallery_id);
            CREATE INDEX auto_find_candidates_group_idx
                ON auto_find_candidates(run_id, favorite_namespace, favorite_value);

            CREATE TABLE auto_find_exclusions (
                gallery_id INTEGER PRIMARY KEY CHECK (gallery_id > 0),
                reason TEXT NOT NULL CHECK (length(trim(reason)) > 0),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0)
            ) STRICT;
        "#,
    },
    Migration {
        version: 11,
        name: "auto_find_visible_metadata",
        sql: r#"
            ALTER TABLE auto_find_candidates
                ADD COLUMN series_json TEXT NOT NULL DEFAULT '[]'
                    CHECK (json_valid(series_json));
            ALTER TABLE auto_find_candidates
                ADD COLUMN characters_json TEXT NOT NULL DEFAULT '[]'
                    CHECK (json_valid(characters_json));
        "#,
    },
    Migration {
        version: 12,
        name: "artifact_duplicate_evidence_and_decisions",
        sql: r#"
            CREATE TABLE duplicate_hash_profiles (
                profile_version INTEGER PRIMARY KEY CHECK (profile_version > 0),
                algorithm_version INTEGER NOT NULL CHECK (algorithm_version > 0),
                d_hash_bits INTEGER NOT NULL CHECK (d_hash_bits >= 64),
                p_hash_bits INTEGER NOT NULL CHECK (p_hash_bits >= 64),
                visual_match_threshold REAL NOT NULL
                    CHECK (visual_match_threshold BETWEEN 0.0 AND 1.0),
                low_information_std_dev_threshold REAL NOT NULL
                    CHECK (low_information_std_dev_threshold >= 0.0),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0)
            ) STRICT;
            INSERT INTO duplicate_hash_profiles (
                profile_version, algorithm_version, d_hash_bits, p_hash_bits,
                visual_match_threshold, low_information_std_dev_threshold,
                created_at
            ) VALUES (
                1, 1, 1024, 64, 0.80, 10.0,
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            );

            CREATE TABLE duplicate_page_hashes (
                entry_id TEXT NOT NULL,
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                source_page_number INTEGER NOT NULL CHECK (source_page_number > 0),
                profile_version INTEGER NOT NULL
                    REFERENCES duplicate_hash_profiles(profile_version),
                artifact_sha256 TEXT NOT NULL CHECK (length(artifact_sha256) = 64),
                coarse_d_hash_hex TEXT NOT NULL CHECK (length(coarse_d_hash_hex) = 16),
                detail_d_hash_hex TEXT NOT NULL CHECK (length(detail_d_hash_hex) = 256),
                p_hash_hex TEXT NOT NULL CHECK (length(p_hash_hex) = 16),
                mean_luma REAL NOT NULL CHECK (mean_luma BETWEEN 0.0 AND 255.0),
                std_dev REAL NOT NULL CHECK (std_dev >= 0.0),
                non_uniform_ratio REAL NOT NULL CHECK (non_uniform_ratio BETWEEN 0.0 AND 1.0),
                edge_density REAL NOT NULL CHECK (edge_density BETWEEN 0.0 AND 1.0),
                width INTEGER NOT NULL CHECK (width > 0),
                height INTEGER NOT NULL CHECK (height > 0),
                low_information INTEGER NOT NULL CHECK (low_information IN (0, 1)),
                computed_at TEXT NOT NULL CHECK (length(computed_at) > 0),
                PRIMARY KEY (entry_id, source_page_number, profile_version),
                FOREIGN KEY (entry_id, source_page_number)
                    REFERENCES download_pages(entry_id, source_page_number)
                    ON DELETE CASCADE
            ) STRICT;
            CREATE INDEX duplicate_page_hashes_gallery_idx
                ON duplicate_page_hashes(gallery_id, profile_version);

            CREATE TABLE duplicate_scan_runs (
                run_id TEXT PRIMARY KEY CHECK (length(trim(run_id)) > 0),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                state TEXT NOT NULL CHECK (state IN (
                    'running', 'completed', 'failed', 'cancelled'
                )),
                profile_version INTEGER NOT NULL
                    REFERENCES duplicate_hash_profiles(profile_version),
                total_artifacts INTEGER NOT NULL CHECK (total_artifacts >= 0),
                hashed_artifacts INTEGER NOT NULL
                    CHECK (hashed_artifacts BETWEEN 0 AND total_artifacts),
                total_pairs INTEGER NOT NULL CHECK (total_pairs >= 0),
                compared_pairs INTEGER NOT NULL
                    CHECK (compared_pairs BETWEEN 0 AND total_pairs),
                candidates_found INTEGER NOT NULL CHECK (candidates_found >= 0),
                started_at TEXT NOT NULL CHECK (length(started_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                finished_at TEXT,
                error_code TEXT,
                error_message TEXT
            ) STRICT;
            CREATE UNIQUE INDEX duplicate_scan_one_running_idx
                ON duplicate_scan_runs(state) WHERE state = 'running';
            CREATE INDEX duplicate_scan_recent_idx
                ON duplicate_scan_runs(started_at DESC, run_id DESC);

            CREATE TABLE duplicate_candidates (
                candidate_id TEXT PRIMARY KEY CHECK (length(trim(candidate_id)) > 0),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                last_seen_run_id TEXT NOT NULL
                    REFERENCES duplicate_scan_runs(run_id) ON DELETE CASCADE,
                profile_version INTEGER NOT NULL
                    REFERENCES duplicate_hash_profiles(profile_version),
                parent_gallery_id INTEGER NOT NULL CHECK (parent_gallery_id > 0),
                parent_entry_id TEXT NOT NULL
                    REFERENCES download_entries(entry_id) ON DELETE CASCADE,
                candidate_gallery_id INTEGER NOT NULL CHECK (candidate_gallery_id > 0),
                candidate_entry_id TEXT NOT NULL
                    REFERENCES download_entries(entry_id) ON DELETE CASCADE,
                relation TEXT NOT NULL CHECK (relation IN (
                    'exact', 'contains', 'partial', 'translation_visual'
                )),
                confidence REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
                matched_pages INTEGER NOT NULL CHECK (matched_pages > 0),
                parent_coverage REAL NOT NULL CHECK (parent_coverage BETWEEN 0.0 AND 1.0),
                candidate_coverage REAL NOT NULL CHECK (candidate_coverage BETWEEN 0.0 AND 1.0),
                resolved INTEGER NOT NULL DEFAULT 0 CHECK (resolved IN (0, 1)),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                CHECK (parent_gallery_id < candidate_gallery_id),
                UNIQUE (profile_version, parent_gallery_id, candidate_gallery_id)
            ) STRICT;
            CREATE INDEX duplicate_candidates_run_idx
                ON duplicate_candidates(last_seen_run_id, resolved, confidence DESC);

            CREATE TABLE duplicate_evidence (
                evidence_id TEXT PRIMARY KEY CHECK (length(trim(evidence_id)) > 0),
                candidate_id TEXT NOT NULL
                    REFERENCES duplicate_candidates(candidate_id) ON DELETE CASCADE,
                kind TEXT NOT NULL CHECK (kind IN (
                    'exact_sha256', 'visual_hash', 'sequence_alignment',
                    'e_hentai_relation'
                )),
                confidence REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
                matched_pages INTEGER NOT NULL CHECK (matched_pages >= 0),
                description TEXT NOT NULL CHECK (length(trim(description)) > 0)
            ) STRICT;
            CREATE INDEX duplicate_evidence_candidate_idx
                ON duplicate_evidence(candidate_id, kind);

            CREATE TABLE duplicate_page_pairs (
                candidate_id TEXT NOT NULL
                    REFERENCES duplicate_candidates(candidate_id) ON DELETE CASCADE,
                parent_source_page INTEGER NOT NULL CHECK (parent_source_page > 0),
                candidate_source_page INTEGER NOT NULL CHECK (candidate_source_page > 0),
                exact_sha256 INTEGER NOT NULL CHECK (exact_sha256 IN (0, 1)),
                d_hash_distance INTEGER NOT NULL CHECK (d_hash_distance >= 0),
                p_hash_distance INTEGER NOT NULL CHECK (p_hash_distance >= 0),
                detail_hash_distance INTEGER NOT NULL CHECK (detail_hash_distance >= 0),
                edge_similarity REAL NOT NULL CHECK (edge_similarity BETWEEN 0.0 AND 1.0),
                visual_similarity REAL NOT NULL CHECK (visual_similarity BETWEEN 0.0 AND 1.0),
                low_information INTEGER NOT NULL CHECK (low_information IN (0, 1)),
                PRIMARY KEY (
                    candidate_id, parent_source_page, candidate_source_page
                )
            ) STRICT;

            CREATE TABLE duplicate_hidden_galleries (
                gallery_id INTEGER PRIMARY KEY CHECK (gallery_id > 0),
                decision_id TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL CHECK (length(created_at) > 0)
            ) STRICT;

            CREATE TABLE duplicate_series_groups (
                series_group_id TEXT PRIMARY KEY CHECK (length(trim(series_group_id)) > 0),
                name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 200),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0)
            ) STRICT;
            CREATE TABLE duplicate_series_members (
                series_group_id TEXT NOT NULL
                    REFERENCES duplicate_series_groups(series_group_id) ON DELETE CASCADE,
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                entry_id TEXT NOT NULL
                    REFERENCES download_entries(entry_id) ON DELETE CASCADE,
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                PRIMARY KEY (series_group_id, gallery_id)
            ) STRICT;
            CREATE INDEX duplicate_series_gallery_idx
                ON duplicate_series_members(gallery_id);

            CREATE TABLE duplicate_pair_exclusions (
                parent_gallery_id INTEGER NOT NULL CHECK (parent_gallery_id > 0),
                candidate_gallery_id INTEGER NOT NULL CHECK (candidate_gallery_id > 0),
                decision_id TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                PRIMARY KEY (parent_gallery_id, candidate_gallery_id),
                CHECK (parent_gallery_id < candidate_gallery_id)
            ) STRICT;

            CREATE TABLE duplicate_decisions (
                decision_id TEXT PRIMARY KEY CHECK (length(trim(decision_id)) > 0),
                candidate_id TEXT NOT NULL
                    REFERENCES duplicate_candidates(candidate_id),
                candidate_revision INTEGER NOT NULL CHECK (candidate_revision > 0),
                action TEXT NOT NULL CHECK (action IN (
                    'hide_parent', 'hide_candidate', 'series_link',
                    'series_unlink', 'exclude_pair'
                )),
                target_gallery_id INTEGER,
                series_group_id TEXT,
                created_at TEXT NOT NULL CHECK (length(created_at) > 0)
            ) STRICT;
            CREATE INDEX duplicate_decisions_candidate_idx
                ON duplicate_decisions(candidate_id, created_at ASC, decision_id ASC);
        "#,
    },
    Migration {
        version: 13,
        name: "internal_scene_review_and_page_quarantine",
        sql: r#"
            CREATE TABLE internal_duplicate_runs (
                run_id TEXT PRIMARY KEY CHECK (length(trim(run_id)) > 0),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                state TEXT NOT NULL CHECK (state IN (
                    'running', 'completed', 'failed', 'cancelled'
                )),
                profile_version INTEGER NOT NULL
                    REFERENCES duplicate_hash_profiles(profile_version),
                total_artifacts INTEGER NOT NULL CHECK (total_artifacts >= 0),
                scanned_artifacts INTEGER NOT NULL
                    CHECK (scanned_artifacts BETWEEN 0 AND total_artifacts),
                total_pages INTEGER NOT NULL CHECK (total_pages >= 0),
                compared_pairs INTEGER NOT NULL CHECK (compared_pairs >= 0),
                groups_found INTEGER NOT NULL CHECK (groups_found >= 0),
                started_at TEXT NOT NULL CHECK (length(started_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                finished_at TEXT,
                error_code TEXT,
                error_message TEXT
            ) STRICT;
            CREATE UNIQUE INDEX internal_duplicate_one_running_idx
                ON internal_duplicate_runs(state) WHERE state = 'running';
            CREATE INDEX internal_duplicate_recent_idx
                ON internal_duplicate_runs(started_at DESC, run_id DESC);

            CREATE TABLE internal_duplicate_groups (
                group_id TEXT PRIMARY KEY CHECK (length(trim(group_id)) > 0),
                block_id TEXT NOT NULL CHECK (length(trim(block_id)) > 0),
                sequence_index INTEGER NOT NULL CHECK (sequence_index >= 0),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                last_seen_run_id TEXT NOT NULL
                    REFERENCES internal_duplicate_runs(run_id) ON DELETE CASCADE,
                entry_id TEXT NOT NULL
                    REFERENCES download_entries(entry_id) ON DELETE CASCADE,
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                relation TEXT NOT NULL CHECK (relation IN (
                    'exact', 'translation_visual'
                )),
                confidence REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
                recommended_keep_source_page INTEGER NOT NULL CHECK (
                    recommended_keep_source_page > 0
                ),
                resolved INTEGER NOT NULL DEFAULT 0 CHECK (resolved IN (0, 1)),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                UNIQUE (entry_id, block_id, sequence_index)
            ) STRICT;
            CREATE INDEX internal_duplicate_groups_entry_idx
                ON internal_duplicate_groups(entry_id, resolved, block_id, sequence_index);

            CREATE TABLE internal_duplicate_group_pages (
                group_id TEXT NOT NULL
                    REFERENCES internal_duplicate_groups(group_id) ON DELETE CASCADE,
                source_page_number INTEGER NOT NULL CHECK (source_page_number > 0),
                exact_sha256 INTEGER NOT NULL CHECK (exact_sha256 IN (0, 1)),
                visual_similarity REAL NOT NULL CHECK (
                    visual_similarity BETWEEN 0.0 AND 1.0
                ),
                detail_hash_distance INTEGER NOT NULL CHECK (detail_hash_distance >= 0),
                low_information INTEGER NOT NULL CHECK (low_information IN (0, 1)),
                PRIMARY KEY (group_id, source_page_number)
            ) STRICT;

            CREATE TABLE internal_removal_plans (
                plan_id TEXT PRIMARY KEY CHECK (length(trim(plan_id)) > 0),
                entry_id TEXT NOT NULL
                    REFERENCES download_entries(entry_id) ON DELETE CASCADE,
                selections_json TEXT NOT NULL CHECK (json_valid(selections_json)),
                files_to_quarantine INTEGER NOT NULL CHECK (files_to_quarantine > 0),
                bytes_to_quarantine INTEGER NOT NULL CHECK (bytes_to_quarantine > 0),
                state TEXT NOT NULL CHECK (state IN (
                    'prepared', 'applying', 'applied', 'cancelled'
                )),
                expires_at TEXT NOT NULL CHECK (length(expires_at) > 0),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0)
            ) STRICT;
            CREATE INDEX internal_removal_plans_entry_idx
                ON internal_removal_plans(entry_id, state, created_at DESC);

            CREATE TABLE page_quarantine_records (
                record_id TEXT PRIMARY KEY CHECK (length(trim(record_id)) > 0),
                plan_id TEXT NOT NULL
                    REFERENCES internal_removal_plans(plan_id),
                entry_id TEXT NOT NULL,
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                source_page_number INTEGER NOT NULL CHECK (source_page_number > 0),
                original_relative_path TEXT NOT NULL
                    CHECK (length(trim(original_relative_path)) > 0),
                quarantine_relative_path TEXT NOT NULL UNIQUE
                    CHECK (length(trim(quarantine_relative_path)) > 0),
                reason TEXT NOT NULL CHECK (length(trim(reason)) BETWEEN 1 AND 500),
                state TEXT NOT NULL CHECK (state IN (
                    'pending_quarantine', 'quarantined',
                    'pending_restore', 'restored'
                )),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                FOREIGN KEY (entry_id, source_page_number)
                    REFERENCES download_pages(entry_id, source_page_number)
            ) STRICT;
            CREATE UNIQUE INDEX page_quarantine_active_page_idx
                ON page_quarantine_records(entry_id, source_page_number)
                WHERE state IN (
                    'pending_quarantine', 'quarantined', 'pending_restore'
                );
            CREATE INDEX page_quarantine_pending_idx
                ON page_quarantine_records(state, created_at);
        "#,
    },
    Migration {
        version: 14,
        name: "classic_read_only_import_and_rollback",
        sql: r#"
            CREATE TABLE classic_import_runs (
                import_id TEXT PRIMARY KEY CHECK (length(trim(import_id)) > 0),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                state TEXT NOT NULL CHECK (state IN (
                    'dry_run', 'applying', 'applied', 'rolling_back',
                    'rolled_back', 'failed'
                )),
                source_schema_version INTEGER NOT NULL CHECK (source_schema_version > 0),
                data_root TEXT NOT NULL CHECK (length(trim(data_root)) > 0),
                download_root TEXT,
                data_root_label TEXT NOT NULL CHECK (length(trim(data_root_label)) > 0),
                download_root_label TEXT,
                source_fingerprint TEXT NOT NULL CHECK (length(source_fingerprint) = 64),
                plan_json TEXT NOT NULL CHECK (json_valid(plan_json)),
                report_json TEXT NOT NULL CHECK (json_valid(report_json)),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                applied_at TEXT,
                rolled_back_at TEXT,
                error_code TEXT,
                error_message TEXT
            ) STRICT;
            CREATE INDEX classic_import_runs_recent_idx
                ON classic_import_runs(created_at DESC, import_id DESC);
            CREATE INDEX classic_import_runs_state_idx
                ON classic_import_runs(state, updated_at);

            CREATE TABLE classic_import_artifact_copies (
                import_id TEXT NOT NULL
                    REFERENCES classic_import_runs(import_id) ON DELETE CASCADE,
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                entry_id TEXT NOT NULL UNIQUE CHECK (length(trim(entry_id)) > 0),
                relative_directory TEXT NOT NULL UNIQUE
                    CHECK (length(trim(relative_directory)) > 0),
                copied_files INTEGER NOT NULL CHECK (copied_files >= 0),
                copied_bytes INTEGER NOT NULL CHECK (copied_bytes >= 0),
                state TEXT NOT NULL CHECK (state IN (
                    'copied', 'registered', 'quarantined'
                )),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                PRIMARY KEY (import_id, gallery_id)
            ) STRICT;

            CREATE TABLE classic_import_changes (
                import_id TEXT NOT NULL
                    REFERENCES classic_import_runs(import_id) ON DELETE CASCADE,
                sequence INTEGER NOT NULL CHECK (sequence >= 0),
                entity_kind TEXT NOT NULL CHECK (entity_kind IN (
                    'favorite', 'search_history', 'auto_find_exclusion',
                    'hidden_gallery', 'pair_exclusion', 'series_group',
                    'download_artifact'
                )),
                entity_key TEXT NOT NULL CHECK (length(trim(entity_key)) > 0),
                after_revision INTEGER,
                PRIMARY KEY (import_id, sequence),
                UNIQUE (import_id, entity_kind, entity_key)
            ) STRICT;

            CREATE TABLE classic_import_legacy_hashes (
                import_id TEXT NOT NULL
                    REFERENCES classic_import_runs(import_id) ON DELETE CASCADE,
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                page_hashes INTEGER NOT NULL CHECK (page_hashes >= 0),
                file_hashes INTEGER NOT NULL CHECK (file_hashes >= 0),
                trusted_for_duplicate_blocking INTEGER NOT NULL DEFAULT 0
                    CHECK (trusted_for_duplicate_blocking = 0),
                PRIMARY KEY (import_id, gallery_id)
            ) STRICT;
        "#,
    },
    Migration {
        version: 15,
        name: "artifact_folder_template_and_immutable_path",
        sql: r#"
            ALTER TABLE settings
            ADD COLUMN folder_name_template TEXT NOT NULL
                DEFAULT '[{artist}] {title} [{group}] {id}'
                CHECK (
                    length(trim(folder_name_template)) > 0
                    AND length(folder_name_template) <= 512
                );

            CREATE TRIGGER download_artifacts_relative_directory_immutable
            BEFORE UPDATE OF relative_directory ON download_artifacts
            FOR EACH ROW
            WHEN NEW.relative_directory <> OLD.relative_directory
            BEGIN
                SELECT RAISE(ABORT, 'download artifact relative_directory is immutable');
            END;
        "#,
    },
    Migration {
        version: 16,
        name: "download_candidate_diagnostics_and_artifact_root_snapshot",
        sql: r#"
            ALTER TABLE download_jobs
            ADD COLUMN last_error_retryable INTEGER
                CHECK (last_error_retryable IS NULL OR last_error_retryable IN (0, 1));
            ALTER TABLE download_attempts
            ADD COLUMN error_retryable INTEGER
                CHECK (error_retryable IS NULL OR error_retryable IN (0, 1));

            ALTER TABLE download_page_attempts
            ADD COLUMN candidate_format TEXT NOT NULL DEFAULT 'unknown'
                CHECK (candidate_format IN ('unknown', 'webp', 'jpeg', 'png', 'avif', 'jxl'));
            ALTER TABLE download_page_attempts
            ADD COLUMN http_status INTEGER
                CHECK (http_status IS NULL OR http_status BETWEEN 100 AND 599);
            ALTER TABLE download_page_attempts
            ADD COLUMN content_type TEXT
                CHECK (content_type IS NULL OR length(content_type) BETWEEN 1 AND 127);
            ALTER TABLE download_page_attempts
            ADD COLUMN retryable INTEGER NOT NULL DEFAULT 0
                CHECK (retryable IN (0, 1));

            ALTER TABLE download_artifacts
            ADD COLUMN root_snapshot TEXT NOT NULL DEFAULT '';
            UPDATE download_artifacts
            SET root_snapshot = (SELECT download_root FROM settings WHERE singleton = 1)
            WHERE root_snapshot = '';

            CREATE TRIGGER download_artifacts_root_snapshot_immutable
            BEFORE UPDATE OF root_snapshot ON download_artifacts
            FOR EACH ROW
            WHEN NEW.root_snapshot <> OLD.root_snapshot
            BEGIN
                SELECT RAISE(ABORT, 'download artifact root_snapshot is immutable');
            END;
        "#,
    },
    Migration {
        version: 17,
        name: "auto_find_history_cutoff_evidence",
        sql: r#"
            ALTER TABLE settings
            ADD COLUMN auto_find_history_mode TEXT NOT NULL
                DEFAULT 'include_all_history'
                CHECK (auto_find_history_mode IN (
                    'include_all_history', 'newer_than_oldest_downloaded'
                ));

            ALTER TABLE auto_find_runs
            ADD COLUMN history_mode TEXT NOT NULL
                DEFAULT 'include_all_history'
                CHECK (history_mode IN (
                    'include_all_history', 'newer_than_oldest_downloaded'
                ));

            CREATE TABLE owned_gallery_artists (
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                artist TEXT NOT NULL COLLATE NOCASE
                    CHECK (length(trim(artist)) BETWEEN 1 AND 200),
                PRIMARY KEY (gallery_id, artist)
            ) STRICT;
            CREATE INDEX owned_gallery_artists_artist_idx
                ON owned_gallery_artists(artist, gallery_id);

            -- Legacy records only retained a primary artist. Keeping that
            -- conservative backfill means unknown co-artists get no cutoff.
            INSERT OR IGNORE INTO owned_gallery_artists (gallery_id, artist)
            SELECT DISTINCT gallery.gallery_id, trim(gallery.primary_artist)
            FROM galleries gallery
            JOIN download_entries entry ON entry.gallery_id = gallery.gallery_id
            JOIN download_artifacts artifact ON artifact.entry_id = entry.entry_id
            WHERE gallery.primary_artist IS NOT NULL
              AND length(trim(gallery.primary_artist)) > 0
              AND entry.state IN ('completed', 'quarantined')
              AND artifact.state IN ('complete', 'quarantined');

            CREATE TABLE auto_find_run_cutoffs (
                run_id TEXT NOT NULL REFERENCES auto_find_runs(run_id) ON DELETE CASCADE,
                artist TEXT NOT NULL COLLATE NOCASE
                    CHECK (length(trim(artist)) BETWEEN 1 AND 200),
                oldest_owned_gallery_id INTEGER CHECK (oldest_owned_gallery_id > 0),
                qualified_owned_count INTEGER NOT NULL CHECK (qualified_owned_count >= 0),
                cutoff_source TEXT NOT NULL
                    CHECK (cutoff_source = 'verified_owned_artifact'),
                policy_version INTEGER NOT NULL CHECK (policy_version = 1),
                PRIMARY KEY (run_id, artist)
            ) STRICT;

            CREATE TABLE auto_find_run_truncations (
                run_id TEXT NOT NULL REFERENCES auto_find_runs(run_id) ON DELETE CASCADE,
                artist TEXT NOT NULL COLLATE NOCASE
                    CHECK (length(trim(artist)) BETWEEN 1 AND 200),
                reason TEXT NOT NULL CHECK (reason = 'candidate_limit_after_cutoff'),
                eligible_count INTEGER NOT NULL CHECK (eligible_count >= 0),
                candidate_limit INTEGER NOT NULL CHECK (candidate_limit > 0),
                PRIMARY KEY (run_id, artist)
            ) STRICT;
        "#,
    },
    Migration {
        version: 18,
        name: "gallery_source_revision_identity",
        sql: r#"
            ALTER TABLE galleries
            ADD COLUMN source_revision TEXT
                CHECK (
                    source_revision IS NULL
                    OR length(trim(source_revision)) BETWEEN 1 AND 512
                );
        "#,
    },
    Migration {
        version: 19,
        name: "related_gallery_preview_preference",
        sql: r#"
            ALTER TABLE settings
            ADD COLUMN related_preview_width INTEGER NOT NULL DEFAULT 240
                CHECK (related_preview_width IN (180, 200, 220, 240, 260, 280, 300, 320));
        "#,
    },
    Migration {
        version: 20,
        name: "tag_catalog",
        sql: r#"
            CREATE TABLE tag_catalog_entries (
                namespace TEXT NOT NULL CHECK (namespace IN ('tag', 'female', 'male')),
                name TEXT NOT NULL COLLATE NOCASE CHECK (length(trim(name)) BETWEEN 1 AND 200),
                normalized_name TEXT NOT NULL COLLATE NOCASE CHECK (length(normalized_name) > 0),
                canonical_token TEXT NOT NULL COLLATE NOCASE CHECK (length(canonical_token) > 0),
                gallery_count INTEGER NOT NULL CHECK (gallery_count >= 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                PRIMARY KEY (namespace, name),
                UNIQUE (canonical_token)
            ) STRICT;
            CREATE INDEX tag_catalog_entries_normalized_name ON tag_catalog_entries(normalized_name);
            CREATE TABLE tag_catalog_state (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                entry_count INTEGER NOT NULL CHECK (entry_count >= 0),
                neutral_count INTEGER NOT NULL CHECK (neutral_count >= 0),
                female_count INTEGER NOT NULL CHECK (female_count >= 0),
                male_count INTEGER NOT NULL CHECK (male_count >= 0),
                last_attempt_at TEXT,
                last_success_at TEXT,
                last_error_code TEXT,
                last_error_message TEXT
            ) STRICT;
            INSERT INTO tag_catalog_state (singleton, revision, entry_count, neutral_count, female_count, male_count)
            VALUES (1, 0, 0, 0, 0, 0);
        "#,
    },
    Migration {
        version: 21,
        name: "internal_duplicate_nway_scene_clustering",
        sql: r#"
            ALTER TABLE internal_duplicate_runs
            ADD COLUMN algorithm_version INTEGER NOT NULL DEFAULT 1
                CHECK (algorithm_version > 0);
            ALTER TABLE internal_duplicate_runs
            ADD COLUMN skipped_artifacts INTEGER NOT NULL DEFAULT 0
                CHECK (skipped_artifacts >= 0);
            ALTER TABLE internal_duplicate_runs
            ADD COLUMN skipped_pages INTEGER NOT NULL DEFAULT 0
                CHECK (skipped_pages >= 0);

            CREATE TABLE internal_duplicate_scan_skips (
                run_id TEXT NOT NULL
                    REFERENCES internal_duplicate_runs(run_id) ON DELETE CASCADE,
                entry_id TEXT NOT NULL
                    REFERENCES download_entries(entry_id) ON DELETE CASCADE,
                gallery_id INTEGER NOT NULL CHECK (gallery_id > 0),
                title TEXT NOT NULL CHECK (length(trim(title)) > 0),
                page_count INTEGER NOT NULL CHECK (page_count >= 500),
                reason TEXT NOT NULL CHECK (reason = 'page_limit'),
                PRIMARY KEY (run_id, entry_id)
            ) STRICT;
            CREATE INDEX internal_duplicate_scan_skips_run_idx
                ON internal_duplicate_scan_skips(run_id, gallery_id);
        "#,
    },
    Migration {
        version: 22,
        name: "internal_duplicate_edition_tracks",
        sql: r#"
            ALTER TABLE internal_duplicate_group_pages
            ADD COLUMN edition_track_id TEXT CHECK (
                edition_track_id IS NULL OR length(trim(edition_track_id)) > 0
            );
            ALTER TABLE internal_duplicate_group_pages
            ADD COLUMN edition_track_ordinal INTEGER CHECK (
                edition_track_ordinal IS NULL OR edition_track_ordinal >= 0
            );
            CREATE INDEX internal_duplicate_group_pages_track_idx
                ON internal_duplicate_group_pages (
                    edition_track_id, edition_track_ordinal, group_id
                );
        "#,
    },
    Migration {
        version: 23,
        name: "preview_privacy_mode",
        sql: r#"
            ALTER TABLE settings
            ADD COLUMN privacy_mode INTEGER NOT NULL DEFAULT 0
                CHECK (privacy_mode IN (0, 1));
        "#,
    },
    Migration {
        version: 24,
        name: "artist_group_autocomplete_catalog",
        sql: r#"
            ALTER TABLE tag_catalog_state
            ADD COLUMN artist_count INTEGER NOT NULL DEFAULT 0
                CHECK (artist_count >= 0);
            ALTER TABLE tag_catalog_state
            ADD COLUMN group_count INTEGER NOT NULL DEFAULT 0
                CHECK (group_count >= 0);

            CREATE TABLE metadata_catalog_entries (
                namespace TEXT NOT NULL CHECK (namespace IN ('artist', 'group')),
                name TEXT NOT NULL COLLATE NOCASE CHECK (length(trim(name)) BETWEEN 1 AND 200),
                normalized_name TEXT NOT NULL COLLATE NOCASE CHECK (length(normalized_name) > 0),
                canonical_token TEXT NOT NULL COLLATE NOCASE CHECK (length(canonical_token) > 0),
                gallery_count INTEGER NOT NULL CHECK (gallery_count >= 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                PRIMARY KEY (namespace, name),
                UNIQUE (canonical_token)
            ) STRICT;
            CREATE INDEX metadata_catalog_entries_normalized_name
                ON metadata_catalog_entries(normalized_name);
        "#,
    },
    Migration {
        version: 25,
        name: "gallery_group_accordion_state",
        sql: r#"
            ALTER TABLE settings
            ADD COLUMN collapsed_group_keys_json TEXT NOT NULL DEFAULT '[]'
                CHECK (length(collapsed_group_keys_json) <= 131072);
        "#,
    },
    Migration {
        version: 26,
        name: "download_overlap_review_gate",
        sql: r#"
            CREATE TABLE download_overlap_reviews (
                review_id TEXT PRIMARY KEY CHECK (length(trim(review_id)) > 0),
                entry_id TEXT NOT NULL
                    REFERENCES download_entries(entry_id) ON DELETE CASCADE,
                incoming_gallery_id INTEGER NOT NULL CHECK (incoming_gallery_id > 0),
                revision INTEGER NOT NULL CHECK (revision >= 0),
                state TEXT NOT NULL CHECK (state IN (
                    'pending', 'resolved', 'cancelled', 'stale'
                )),
                profile_version INTEGER NOT NULL
                    REFERENCES duplicate_hash_profiles(profile_version),
                policy_version INTEGER NOT NULL CHECK (policy_version > 0),
                incoming_fingerprint TEXT NOT NULL CHECK (length(incoming_fingerprint) = 64),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
                resolved_at TEXT
            ) STRICT;
            CREATE UNIQUE INDEX download_overlap_one_pending_per_entry
                ON download_overlap_reviews(entry_id) WHERE state = 'pending';
            CREATE INDEX download_overlap_reviews_entry_idx
                ON download_overlap_reviews(entry_id, created_at DESC, review_id DESC);
            CREATE INDEX download_overlap_reviews_recent_idx
                ON download_overlap_reviews(updated_at DESC, review_id DESC);

            CREATE TABLE download_overlap_candidates (
                candidate_id TEXT PRIMARY KEY CHECK (length(trim(candidate_id)) > 0),
                review_id TEXT NOT NULL
                    REFERENCES download_overlap_reviews(review_id) ON DELETE CASCADE,
                existing_entry_id TEXT NOT NULL
                    REFERENCES download_entries(entry_id) ON DELETE CASCADE,
                existing_gallery_id INTEGER NOT NULL CHECK (existing_gallery_id > 0),
                existing_fingerprint TEXT NOT NULL CHECK (length(existing_fingerprint) = 64),
                relation TEXT NOT NULL CHECK (relation IN (
                    'near_equivalent', 'incoming_contains_existing',
                    'existing_contains_incoming', 'partial_overlap',
                    'translation_edition'
                )),
                confidence REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
                matched_pages INTEGER NOT NULL CHECK (matched_pages > 0),
                exact_pages INTEGER NOT NULL CHECK (exact_pages BETWEEN 0 AND matched_pages),
                visual_pages INTEGER NOT NULL CHECK (visual_pages BETWEEN 0 AND matched_pages),
                existing_coverage REAL NOT NULL CHECK (existing_coverage BETWEEN 0.0 AND 1.0),
                incoming_coverage REAL NOT NULL CHECK (incoming_coverage BETWEEN 0.0 AND 1.0),
                existing_unique_pages INTEGER NOT NULL CHECK (existing_unique_pages >= 0),
                incoming_unique_pages INTEGER NOT NULL CHECK (incoming_unique_pages >= 0),
                longest_aligned_run INTEGER NOT NULL CHECK (longest_aligned_run > 0),
                rank INTEGER NOT NULL CHECK (rank > 0),
                decision TEXT CHECK (decision IS NULL OR decision IN ('keep_both', 'false_positive')),
                UNIQUE (review_id, existing_entry_id),
                CHECK (exact_pages + visual_pages = matched_pages)
            ) STRICT;
            CREATE INDEX download_overlap_candidates_review_idx
                ON download_overlap_candidates(review_id, decision, rank, candidate_id);

            CREATE TABLE download_overlap_page_pairs (
                candidate_id TEXT NOT NULL
                    REFERENCES download_overlap_candidates(candidate_id) ON DELETE CASCADE,
                pair_index INTEGER NOT NULL CHECK (pair_index >= 0),
                incoming_source_page INTEGER NOT NULL CHECK (incoming_source_page > 0),
                existing_source_page INTEGER NOT NULL CHECK (existing_source_page > 0),
                exact_sha256 INTEGER NOT NULL CHECK (exact_sha256 IN (0, 1)),
                d_hash_distance INTEGER NOT NULL CHECK (d_hash_distance >= 0),
                p_hash_distance INTEGER NOT NULL CHECK (p_hash_distance >= 0),
                detail_hash_distance INTEGER NOT NULL CHECK (detail_hash_distance >= 0),
                edge_similarity REAL NOT NULL CHECK (edge_similarity BETWEEN 0.0 AND 1.0),
                visual_similarity REAL NOT NULL CHECK (visual_similarity BETWEEN 0.0 AND 1.0),
                low_information INTEGER NOT NULL CHECK (low_information IN (0, 1)),
                PRIMARY KEY (candidate_id, pair_index)
            ) STRICT;

            CREATE TABLE download_overlap_decisions (
                decision_id TEXT PRIMARY KEY CHECK (length(trim(decision_id)) > 0),
                review_id TEXT NOT NULL
                    REFERENCES download_overlap_reviews(review_id),
                review_revision INTEGER NOT NULL CHECK (review_revision >= 0),
                candidate_id TEXT REFERENCES download_overlap_candidates(candidate_id),
                action TEXT NOT NULL CHECK (action IN (
                    'continue_keep_both', 'false_positive_continue', 'cancel_incoming'
                )),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0)
            ) STRICT;
            CREATE INDEX download_overlap_decisions_review_idx
                ON download_overlap_decisions(review_id, created_at, decision_id);

            CREATE TABLE download_overlap_pair_policies (
                left_fingerprint TEXT NOT NULL CHECK (length(left_fingerprint) = 64),
                right_fingerprint TEXT NOT NULL CHECK (length(right_fingerprint) = 64),
                profile_version INTEGER NOT NULL
                    REFERENCES duplicate_hash_profiles(profile_version),
                policy_version INTEGER NOT NULL CHECK (policy_version > 0),
                decision TEXT NOT NULL CHECK (decision IN ('keep_both', 'false_positive')),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                PRIMARY KEY (
                    left_fingerprint, right_fingerprint,
                    profile_version, policy_version
                ),
                CHECK (left_fingerprint <= right_fingerprint)
            ) STRICT;
        "#,
    },
    Migration {
        version: 27,
        name: "global_search_rules_and_exclusion_management",
        sql: r#"
            ALTER TABLE settings
            ADD COLUMN search_include_tags_json TEXT NOT NULL DEFAULT '[]'
                CHECK (length(search_include_tags_json) <= 65536);
            ALTER TABLE settings
            ADD COLUMN search_exclude_tags_json TEXT NOT NULL DEFAULT '[]'
                CHECK (length(search_exclude_tags_json) <= 65536);

            CREATE TABLE exploration_restored_galleries (
                gallery_id INTEGER PRIMARY KEY CHECK (gallery_id > 0),
                restored_at TEXT NOT NULL CHECK (length(restored_at) > 0)
            ) STRICT;
        "#,
    },
    Migration {
        version: 28,
        name: "gallery_grouping_preferences",
        sql: r#"
            ALTER TABLE settings
            ADD COLUMN auto_find_grouping TEXT NOT NULL DEFAULT 'all'
                CHECK (auto_find_grouping IN ('all', 'day', 'artist'));
            ALTER TABLE settings
            ADD COLUMN downloads_grouping TEXT NOT NULL DEFAULT 'all'
                CHECK (downloads_grouping IN ('all', 'day', 'artist'));
        "#,
    },
    Migration {
        version: 29,
        name: "candidate_scoped_download_overlap_actions",
        sql: r#"
            DROP INDEX download_overlap_candidates_review_idx;
            DROP INDEX download_overlap_decisions_review_idx;

            ALTER TABLE download_overlap_page_pairs
                RENAME TO download_overlap_page_pairs_v29;
            ALTER TABLE download_overlap_decisions
                RENAME TO download_overlap_decisions_v29;
            ALTER TABLE download_overlap_candidates
                RENAME TO download_overlap_candidates_v29;

            CREATE TABLE download_overlap_candidates (
                candidate_id TEXT PRIMARY KEY CHECK (length(trim(candidate_id)) > 0),
                review_id TEXT NOT NULL
                    REFERENCES download_overlap_reviews(review_id) ON DELETE CASCADE,
                existing_entry_id TEXT NOT NULL
                    REFERENCES download_entries(entry_id) ON DELETE CASCADE,
                existing_gallery_id INTEGER NOT NULL CHECK (existing_gallery_id > 0),
                existing_fingerprint TEXT NOT NULL CHECK (length(existing_fingerprint) = 64),
                relation TEXT NOT NULL CHECK (relation IN (
                    'near_equivalent', 'incoming_contains_existing',
                    'existing_contains_incoming', 'partial_overlap',
                    'translation_edition'
                )),
                confidence REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
                matched_pages INTEGER NOT NULL CHECK (matched_pages > 0),
                exact_pages INTEGER NOT NULL CHECK (exact_pages BETWEEN 0 AND matched_pages),
                visual_pages INTEGER NOT NULL CHECK (visual_pages BETWEEN 0 AND matched_pages),
                existing_coverage REAL NOT NULL CHECK (existing_coverage BETWEEN 0.0 AND 1.0),
                incoming_coverage REAL NOT NULL CHECK (incoming_coverage BETWEEN 0.0 AND 1.0),
                existing_unique_pages INTEGER NOT NULL CHECK (existing_unique_pages >= 0),
                incoming_unique_pages INTEGER NOT NULL CHECK (incoming_unique_pages >= 0),
                longest_aligned_run INTEGER NOT NULL CHECK (longest_aligned_run > 0),
                rank INTEGER NOT NULL CHECK (rank > 0),
                decision TEXT CHECK (decision IS NULL OR decision IN (
                    'keep_both', 'false_positive', 'existing_removed'
                )),
                UNIQUE (review_id, existing_entry_id),
                CHECK (exact_pages + visual_pages = matched_pages)
            ) STRICT;
            CREATE INDEX download_overlap_candidates_review_idx
                ON download_overlap_candidates(review_id, decision, rank, candidate_id);

            CREATE TABLE download_overlap_page_pairs (
                candidate_id TEXT NOT NULL
                    REFERENCES download_overlap_candidates(candidate_id) ON DELETE CASCADE,
                pair_index INTEGER NOT NULL CHECK (pair_index >= 0),
                incoming_source_page INTEGER NOT NULL CHECK (incoming_source_page > 0),
                existing_source_page INTEGER NOT NULL CHECK (existing_source_page > 0),
                exact_sha256 INTEGER NOT NULL CHECK (exact_sha256 IN (0, 1)),
                d_hash_distance INTEGER NOT NULL CHECK (d_hash_distance >= 0),
                p_hash_distance INTEGER NOT NULL CHECK (p_hash_distance >= 0),
                detail_hash_distance INTEGER NOT NULL CHECK (detail_hash_distance >= 0),
                edge_similarity REAL NOT NULL CHECK (edge_similarity BETWEEN 0.0 AND 1.0),
                visual_similarity REAL NOT NULL CHECK (visual_similarity BETWEEN 0.0 AND 1.0),
                low_information INTEGER NOT NULL CHECK (low_information IN (0, 1)),
                PRIMARY KEY (candidate_id, pair_index)
            ) STRICT;

            CREATE TABLE download_overlap_decisions (
                decision_id TEXT PRIMARY KEY CHECK (length(trim(decision_id)) > 0),
                review_id TEXT NOT NULL REFERENCES download_overlap_reviews(review_id),
                review_revision INTEGER NOT NULL CHECK (review_revision >= 0),
                candidate_id TEXT REFERENCES download_overlap_candidates(candidate_id),
                action TEXT NOT NULL CHECK (action IN (
                    'continue_keep_both', 'false_positive_continue', 'cancel_incoming',
                    'keep_both_continue', 'remove_existing_continue', 'remove_incoming'
                )),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0)
            ) STRICT;
            CREATE INDEX download_overlap_decisions_review_idx
                ON download_overlap_decisions(review_id, created_at, decision_id);

            INSERT INTO download_overlap_candidates SELECT * FROM download_overlap_candidates_v29;
            INSERT INTO download_overlap_page_pairs SELECT * FROM download_overlap_page_pairs_v29;
            INSERT INTO download_overlap_decisions SELECT * FROM download_overlap_decisions_v29;

            DROP TABLE download_overlap_page_pairs_v29;
            DROP TABLE download_overlap_decisions_v29;
            DROP TABLE download_overlap_candidates_v29;
        "#,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    pub applied_versions: Vec<i64>,
    pub current_version: i64,
}

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("SQLite migration failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "database schema version {found} is newer than the latest supported version {latest_supported}"
    )]
    FutureVersion { found: i64, latest_supported: i64 },
    #[error(
        "migration history is non-contiguous: expected version {expected}, found version {actual}"
    )]
    NonContiguousHistory { expected: i64, actual: i64 },
    #[error("migration {version} was recorded as {actual:?}, expected {expected:?}")]
    NameMismatch {
        version: i64,
        expected: &'static str,
        actual: String,
    },
}

pub struct MigrationRunner;

impl MigrationRunner {
    pub fn run(connection: &mut Connection) -> Result<MigrationReport, MigrationError> {
        connection.execute_batch(
            r#"
                PRAGMA foreign_keys = ON;
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_at TEXT NOT NULL DEFAULT (
                        strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                    )
                ) STRICT;
            "#,
        )?;

        let applied = Self::applied_migrations(connection)?;
        Self::validate_applied_migrations(&applied)?;

        let mut applied_versions = Vec::new();
        for migration in MIGRATIONS {
            if applied.contains_key(&migration.version) {
                continue;
            }

            let transaction = connection.transaction()?;
            transaction.execute_batch(migration.sql)?;
            transaction.execute(
                "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                params![migration.version, migration.name],
            )?;
            transaction.commit()?;
            applied_versions.push(migration.version);
        }

        let current_version = connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;

        Ok(MigrationReport {
            applied_versions,
            current_version,
        })
    }

    pub(crate) fn pending_versions(connection: &Connection) -> Result<Vec<i64>, MigrationError> {
        let migration_table_exists = connection.query_row(
            r#"
                SELECT EXISTS (
                    SELECT 1
                    FROM sqlite_schema
                    WHERE type = 'table' AND name = 'schema_migrations'
                )
            "#,
            [],
            |row| row.get::<_, bool>(0),
        )?;
        let applied = if migration_table_exists {
            Self::applied_migrations(connection)?
        } else {
            BTreeMap::new()
        };
        Self::validate_applied_migrations(&applied)?;
        Ok(MIGRATIONS
            .iter()
            .skip(applied.len())
            .map(|migration| migration.version)
            .collect())
    }

    fn validate_applied_migrations(applied: &BTreeMap<i64, String>) -> Result<(), MigrationError> {
        let latest_supported = MIGRATIONS
            .last()
            .map(|migration| migration.version)
            .unwrap_or(0);
        if let Some(found) = applied.keys().next_back().copied() {
            if found > latest_supported {
                return Err(MigrationError::FutureVersion {
                    found,
                    latest_supported,
                });
            }
        }

        for (index, (actual_version, actual_name)) in applied.iter().enumerate() {
            let Some(expected) = MIGRATIONS.get(index) else {
                return Err(MigrationError::FutureVersion {
                    found: *actual_version,
                    latest_supported,
                });
            };
            if *actual_version != expected.version {
                return Err(MigrationError::NonContiguousHistory {
                    expected: expected.version,
                    actual: *actual_version,
                });
            }
            if actual_name != expected.name {
                return Err(MigrationError::NameMismatch {
                    version: expected.version,
                    expected: expected.name,
                    actual: actual_name.clone(),
                });
            }
        }

        Ok(())
    }

    fn applied_migrations(
        connection: &Connection,
    ) -> Result<BTreeMap<i64, String>, rusqlite::Error> {
        let mut statement = connection
            .prepare("SELECT version, name FROM schema_migrations ORDER BY version ASC")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn migration_history(entries: &[(i64, &str)]) -> Connection {
        let connection = Connection::open_in_memory().expect("open migration test database");
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
                "#,
            )
            .expect("create migration history");
        for (version, name) in entries {
            connection
                .execute(
                    "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                    params![version, name],
                )
                .expect("record migration history");
        }
        connection
    }

    #[test]
    fn rejects_a_schema_version_newer_than_this_binary_supports() {
        let future_version = MIGRATIONS.last().expect("known migration").version + 1;
        let mut connection = migration_history(&[(future_version, "future_migration")]);

        let error = MigrationRunner::run(&mut connection).expect_err("reject future schema");

        assert!(matches!(
            error,
            MigrationError::FutureVersion {
                found,
                latest_supported
            } if found == future_version
                && latest_supported == MIGRATIONS.last().expect("known migration").version
        ));
    }

    #[test]
    fn rejects_a_gap_in_recorded_migration_history() {
        let mut connection = migration_history(&[
            (MIGRATIONS[0].version, MIGRATIONS[0].name),
            (MIGRATIONS[2].version, MIGRATIONS[2].name),
        ]);

        let error = MigrationRunner::run(&mut connection).expect_err("reject migration gap");

        assert!(matches!(
            error,
            MigrationError::NonContiguousHistory {
                expected: 2,
                actual: 3
            }
        ));
    }

    #[test]
    fn rejects_a_recorded_migration_name_mismatch() {
        let mut connection = migration_history(&[(MIGRATIONS[0].version, "renamed")]);

        let error = MigrationRunner::run(&mut connection).expect_err("reject renamed migration");

        assert!(matches!(
            error,
            MigrationError::NameMismatch {
                version: 1,
                expected: "settings_and_window_placement",
                actual
            } if actual == "renamed"
        ));
    }

    #[test]
    fn folder_template_migration_defaults_settings_and_locks_existing_artifact_paths() {
        let mut connection = Connection::open_in_memory().expect("open v14 migration database");
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
            .unwrap();
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= 14)
        {
            connection.execute_batch(migration.sql).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                    params![migration.version, migration.name],
                )
                .unwrap();
        }
        connection
            .execute_batch(
                r#"
                    INSERT INTO galleries (
                        gallery_id, revision, title, source_page_count
                    ) VALUES (42, 0, 'Legacy title', 1);
                    UPDATE settings SET download_root = 'C:\legacy-root' WHERE singleton = 1;
                    INSERT INTO download_entries (
                        entry_id, gallery_id, revision, state, progress,
                        created_at, updated_at
                    ) VALUES (
                        'legacy-entry', 42, 0, 'completed', 100.0,
                        '2026-08-20T00:00:00Z', '2026-08-20T00:00:00Z'
                    );
                    INSERT INTO download_artifacts (
                        entry_id, gallery_id, revision, relative_directory,
                        expected_page_count, state
                    ) VALUES (
                        'legacy-entry', 42, 0, 'gallery-42', 1, 'complete'
                    );
                "#,
            )
            .unwrap();

        let report = MigrationRunner::run(&mut connection).expect("migrate v14 to v15");
        assert_eq!(
            report.applied_versions,
            vec![15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29]
        );
        let historical_import_tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'classic_import_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(historical_import_tables, 4);
        let template: String = connection
            .query_row("SELECT folder_name_template FROM settings", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(template, "[{artist}] {title} [{group}] {id}");
        let stored_path: String = connection
            .query_row(
                "SELECT relative_directory FROM download_artifacts WHERE entry_id='legacy-entry'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_path, "gallery-42");
        let root_snapshot: String = connection
            .query_row(
                "SELECT root_snapshot FROM download_artifacts WHERE entry_id='legacy-entry'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(root_snapshot, r"C:\legacy-root");
        let gallery_source_revision: Option<String> = connection
            .query_row(
                "SELECT source_revision FROM galleries WHERE gallery_id=42",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(gallery_source_revision, None);
        assert!(connection
            .execute(
                "UPDATE download_artifacts SET relative_directory='renamed-42' WHERE entry_id='legacy-entry'",
                [],
            )
            .is_err());
        assert!(connection
            .execute(
                r"UPDATE download_artifacts SET root_snapshot='C:\new-root' WHERE entry_id='legacy-entry'",
                [],
            )
            .is_err());
    }

    #[test]
    fn duplicate_evidence_migration_is_additive_from_v11() {
        let mut connection = Connection::open_in_memory().expect("open v11 migration database");
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
            .unwrap();
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= 11)
        {
            connection.execute_batch(migration.sql).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                    params![migration.version, migration.name],
                )
                .unwrap();
        }
        connection
            .execute(
                r#"
                    INSERT INTO favorites (
                        namespace, value, revision, created_at, updated_at
                    ) VALUES ('artist', 'preserved', 0, 'before-v12', 'before-v12')
                "#,
                [],
            )
            .unwrap();

        let report = MigrationRunner::run(&mut connection).expect("migrate v11 to v12");
        assert_eq!(
            report.applied_versions,
            vec![12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29]
        );
        assert_eq!(report.current_version, 29);
        let favorite: String = connection
            .query_row(
                "SELECT value FROM favorites WHERE namespace = 'artist'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(favorite, "preserved");
        let profile: (i64, i64, i64) = connection
            .query_row(
                r#"
                    SELECT profile_version, d_hash_bits, p_hash_bits
                    FROM duplicate_hash_profiles WHERE profile_version = 1
                "#,
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(profile, (1, 1_024, 64));
    }

    #[test]
    fn edition_track_migration_is_additive_from_v21() {
        let mut connection = Connection::open_in_memory().expect("open v21 migration database");
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
            .unwrap();
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= 21)
        {
            connection.execute_batch(migration.sql).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                    params![migration.version, migration.name],
                )
                .unwrap();
        }

        let report = MigrationRunner::run(&mut connection).expect("migrate v21 to v22");
        assert_eq!(
            report.applied_versions,
            vec![22, 23, 24, 25, 26, 27, 28, 29]
        );
        let columns = connection
            .prepare(
                "SELECT name FROM pragma_table_info('internal_duplicate_group_pages') ORDER BY cid",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(columns.contains(&"edition_track_id".to_string()));
        assert!(columns.contains(&"edition_track_ordinal".to_string()));
    }

    #[test]
    fn privacy_mode_migration_is_additive_from_v22() {
        let mut connection = Connection::open_in_memory().expect("open v22 migration database");
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
            .unwrap();
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= 22)
        {
            connection.execute_batch(migration.sql).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                    params![migration.version, migration.name],
                )
                .unwrap();
        }
        connection
            .execute(
                "UPDATE settings SET max_columns = 4 WHERE singleton = 1",
                [],
            )
            .unwrap();

        let report = MigrationRunner::run(&mut connection).expect("migrate v22 to v23");
        assert_eq!(report.applied_versions, vec![23, 24, 25, 26, 27, 28, 29]);
        assert_eq!(report.current_version, 29);
        let settings: (i64, i64) = connection
            .query_row(
                "SELECT max_columns, privacy_mode FROM settings WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(settings, (4, 0));
        assert_eq!(
            connection
                .execute(
                    "UPDATE settings SET privacy_mode = 1 WHERE singleton = 1",
                    [],
                )
                .unwrap(),
            1
        );
        assert!(connection
            .execute(
                "UPDATE settings SET privacy_mode = 2 WHERE singleton = 1",
                [],
            )
            .is_err());
    }

    #[test]
    fn artist_group_catalog_migration_is_additive_from_v23() {
        let mut connection = Connection::open_in_memory().expect("open v23 migration database");
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
            .unwrap();
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= 23)
        {
            connection.execute_batch(migration.sql).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                    params![migration.version, migration.name],
                )
                .unwrap();
        }
        connection
            .execute_batch(
                r#"
                    INSERT INTO tag_catalog_entries (
                        namespace, name, normalized_name, canonical_token,
                        gallery_count, updated_at
                    ) VALUES (
                        'tag', 'webtoon', 'webtoon', 'tag:webtoon', 42,
                        '2026-08-24T00:00:00Z'
                    );
                    UPDATE tag_catalog_state
                       SET revision = 7, entry_count = 1, neutral_count = 1;
                "#,
            )
            .unwrap();

        let report = MigrationRunner::run(&mut connection).expect("migrate v23 to v24");
        assert_eq!(report.applied_versions, vec![24, 25, 26, 27, 28, 29]);
        assert_eq!(report.current_version, 29);
        let preserved: (String, i64, i64, i64) = connection
            .query_row(
                r#"SELECT e.canonical_token, s.revision, s.artist_count, s.group_count
                     FROM tag_catalog_entries e CROSS JOIN tag_catalog_state s
                    WHERE e.canonical_token = 'tag:webtoon'"#,
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(preserved, ("tag:webtoon".to_owned(), 7, 0, 0));
        let collapsed_group_keys: String = connection
            .query_row(
                "SELECT collapsed_group_keys_json FROM settings WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(collapsed_group_keys, "[]");
        assert_eq!(
            connection
                .execute(
                    r#"INSERT INTO metadata_catalog_entries (
                         namespace, name, normalized_name, canonical_token,
                         gallery_count, updated_at
                       ) VALUES ('artist', 'mizuno tooru', 'mizuno tooru',
                         'artist:mizuno_tooru', 142, '2026-08-24T00:00:00Z')"#,
                    [],
                )
                .unwrap(),
            1
        );
        assert!(connection
            .execute(
                r#"INSERT INTO metadata_catalog_entries (
                     namespace, name, normalized_name, canonical_token,
                     gallery_count, updated_at
                   ) VALUES ('tag', 'invalid', 'invalid', 'tag:invalid', 1,
                     '2026-08-24T00:00:00Z')"#,
                [],
            )
            .is_err());
    }

    #[test]
    fn download_overlap_migration_is_additive_from_v25() {
        let mut connection = Connection::open_in_memory().expect("open v25 migration database");
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
            .unwrap();
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= 25)
        {
            connection.execute_batch(migration.sql).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                    params![migration.version, migration.name],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO galleries (gallery_id, revision, title, source_page_count) VALUES (42, 0, 'Preserved', 1)",
                [],
            )
            .unwrap();

        let report = MigrationRunner::run(&mut connection).expect("migrate v25 to v26");
        assert_eq!(report.applied_versions, vec![26, 27, 28, 29]);
        assert_eq!(report.current_version, 29);
        let preserved: String = connection
            .query_row(
                "SELECT title FROM galleries WHERE gallery_id=42",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(preserved, "Preserved");
        for table in [
            "download_overlap_reviews",
            "download_overlap_candidates",
            "download_overlap_page_pairs",
            "download_overlap_decisions",
            "download_overlap_pair_policies",
        ] {
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "missing {table}");
        }
    }

    #[test]
    fn search_rule_migration_is_additive_from_v26() {
        let mut connection = Connection::open_in_memory().expect("open v26 migration database");
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
            .unwrap();
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= 26)
        {
            connection.execute_batch(migration.sql).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                    params![migration.version, migration.name],
                )
                .unwrap();
        }
        connection
            .execute(
                "UPDATE settings SET max_columns = 4 WHERE singleton = 1",
                [],
            )
            .unwrap();

        let report = MigrationRunner::run(&mut connection).expect("migrate v26 to current");
        assert_eq!(report.applied_versions, vec![27, 28, 29]);
        assert_eq!(report.current_version, 29);
        let settings: (i64, String, String) = connection
            .query_row(
                "SELECT max_columns, search_include_tags_json, search_exclude_tags_json FROM settings WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(settings, (4, "[]".into(), "[]".into()));
        assert_eq!(
            connection
                .execute(
                    "INSERT INTO exploration_restored_galleries (gallery_id, restored_at) VALUES (42, 'now')",
                    [],
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn gallery_grouping_preference_migration_is_additive_from_v27() {
        let mut connection = Connection::open_in_memory().expect("open v27 migration database");
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
            .unwrap();
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= 27)
        {
            connection.execute_batch(migration.sql).unwrap();
            connection
                .execute(
                    "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                    params![migration.version, migration.name],
                )
                .unwrap();
        }
        connection
            .execute(
                "UPDATE settings SET max_columns = 4 WHERE singleton = 1",
                [],
            )
            .unwrap();

        let report = MigrationRunner::run(&mut connection).expect("migrate v27 to v28");
        assert_eq!(report.applied_versions, vec![28, 29]);
        assert_eq!(report.current_version, 29);
        let settings: (i64, String, String) = connection
            .query_row(
                "SELECT max_columns, auto_find_grouping, downloads_grouping FROM settings WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(settings, (4, "all".into(), "all".into()));
        connection
            .execute(
                "UPDATE settings SET auto_find_grouping = 'artist', downloads_grouping = 'day'",
                [],
            )
            .unwrap();
        assert!(connection
            .execute("UPDATE settings SET downloads_grouping = 'invalid'", [],)
            .is_err());
    }
}
