use std::{
    sync::{Arc, Barrier},
    thread,
};

use rusqlite::{params, Connection};
use serde_json::json;

use crate::{
    application::{
        ApplicationError, ApplicationService, ArtifactRepository, AutomationRepository,
        RepositoryError, StateRepository,
    },
    domain::{
        windows_path_for_display, ArtifactBundle, ArtifactRelativePath, AutoFindHistoryMode,
        AutoFindRunState, DownloadArtifact, DownloadArtifactState, DownloadEntry, DownloadEntryId,
        DownloadListRequest, ExplorationDataResetRequest, FavoriteKey, FavoriteNamespace,
        FixtureDownloadJobStep, Gallery, GalleryGroupingMode, GalleryId, GalleryMetadata, JobRef,
        JobState, Language, PageArtifact, PageArtifactState, SearchRequest, SearchSort,
        SettingsPatch, SettingsSnapshot, SourcePageNumber, WindowPlacement,
        WindowPlacementSnapshot,
    },
    infrastructure::{FixtureSearchRepository, MigrationRunner, SqliteRepository, MIGRATIONS},
    interface::{ApiAction, ApiError, ApiResult},
};

#[test]
fn migrations_are_ordered_and_idempotent() {
    let mut connection = Connection::open_in_memory().expect("open in-memory database");

    let first = MigrationRunner::run(&mut connection).expect("apply migrations");
    assert_eq!(
        first.applied_versions,
        MIGRATIONS
            .iter()
            .map(|migration| migration.version)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        first.current_version,
        MIGRATIONS.last().expect("at least one migration").version
    );

    let second = MigrationRunner::run(&mut connection).expect("re-run migrations");
    assert!(second.applied_versions.is_empty());
    assert_eq!(second.current_version, first.current_version);

    let recorded: i64 = connection
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("count applied migrations");
    assert_eq!(recorded as usize, MIGRATIONS.len());
}

#[test]
fn primary_group_migration_preserves_existing_gallery_rows() {
    let mut connection = Connection::open_in_memory().expect("open v3 database");
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
    for migration in MIGRATIONS.iter().take(3) {
        connection
            .execute_batch(migration.sql)
            .expect("apply pre-v4 migration");
        connection
            .execute(
                "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                params![migration.version, migration.name],
            )
            .expect("record pre-v4 migration");
    }
    connection
        .execute(
            r#"
                INSERT INTO galleries (
                    gallery_id, revision, title, primary_artist, source_page_count
                ) VALUES (44, 2, 'Existing gallery', 'artist', 12)
            "#,
            [],
        )
        .expect("insert v3 gallery");

    let report = MigrationRunner::run(&mut connection).expect("apply v4 migration");
    assert_eq!(
        report.applied_versions,
        vec![
            4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
            27, 28, 29, 30, 31
        ]
    );
    let stored: (String, Option<String>) = connection
        .query_row(
            "SELECT title, primary_group FROM galleries WHERE gallery_id = 44",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load migrated gallery");
    assert_eq!(stored, ("Existing gallery".into(), None));
}

#[test]
fn lifecycle_migration_preserves_v6_download_graph_and_enables_cancelled() {
    let mut connection = Connection::open_in_memory().expect("open v6 database");
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
    for migration in MIGRATIONS.iter().take(6) {
        connection
            .execute_batch(migration.sql)
            .expect("apply through v6");
        connection
            .execute(
                "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                params![migration.version, migration.name],
            )
            .expect("record migration");
    }
    connection
        .execute_batch(
            r#"
                INSERT INTO galleries (
                    gallery_id, revision, title, primary_artist,
                    source_page_count, primary_group
                ) VALUES (42, 1, 'Migrated gallery', 'artist', 2, 'group');
                INSERT INTO download_entries (
                    entry_id, gallery_id, revision, state, progress
                ) VALUES ('entry-v6', 42, 2, 'interrupted', 37.0);
                INSERT INTO download_jobs (
                    job_id, request_id, entry_id, gallery_id,
                    revision, state, completed_units, total_units
                ) VALUES (
                    'job-v6', 'job-request-v6', 'entry-v6', 42,
                    2, 'interrupted', 37, 100
                );
                INSERT INTO download_queue_requests (
                    request_id, normalized_galleries
                ) VALUES ('queue-v6', '[42]');
                INSERT INTO download_queue_request_entries (
                    request_id, position, gallery_id, entry_id,
                    response_state, response_progress, response_revision
                ) VALUES ('queue-v6', 0, 42, 'entry-v6', 'queued', 0.0, 0);
                INSERT INTO download_artifacts (
                    entry_id, gallery_id, revision, relative_directory,
                    expected_page_count, state
                ) VALUES ('entry-v6', 42, 1, 'migrated-gallery', 2, 'incomplete');
                INSERT INTO download_pages (
                    entry_id, gallery_id, source_page_number,
                    relative_path, state, byte_length
                ) VALUES (
                    'entry-v6', 42, 1,
                    'migrated-gallery/001.webp', 'present', 1024
                );
            "#,
        )
        .expect("seed v6 download graph");

    let report = MigrationRunner::run(&mut connection).expect("apply lifecycle migration");
    assert_eq!(
        report.applied_versions,
        vec![
            7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
            29, 30, 31
        ]
    );
    let lifecycle: (i64, String, Option<String>, i64) = connection
        .query_row(
            r#"
                SELECT j.attempt, j.created_at, j.last_error_code,
                       (SELECT COUNT(*) FROM download_attempts a WHERE a.job_id = j.job_id)
                FROM download_jobs j
                WHERE j.job_id = 'job-v6'
            "#,
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read migrated lifecycle metadata");
    assert_eq!(lifecycle.0, 1);
    assert!(!lifecycle.1.is_empty());
    assert_eq!(lifecycle.2.as_deref(), Some("JOB_INTERRUPTED"));
    assert_eq!(lifecycle.3, 1);
    let preserved: (i64, i64, i64) = connection
        .query_row(
            r#"
                SELECT
                    (SELECT COUNT(*) FROM download_queue_request_entries),
                    (SELECT COUNT(*) FROM download_artifacts),
                    (SELECT COUNT(*) FROM download_pages)
            "#,
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("count preserved graph");
    assert_eq!(preserved, (1, 1, 1));
    let mut foreign_keys = connection
        .prepare("PRAGMA foreign_key_check")
        .expect("prepare foreign key check");
    assert!(foreign_keys
        .query([])
        .expect("run foreign key check")
        .next()
        .expect("read foreign key check")
        .is_none());
    drop(foreign_keys);

    connection
        .execute("UPDATE download_jobs SET state = 'cancelled'", [])
        .expect("job CHECK accepts cancelled");
    connection
        .execute("UPDATE download_entries SET state = 'cancelled'", [])
        .expect("entry CHECK accepts cancelled");
}

#[test]
fn visible_metadata_migration_defaults_existing_auto_find_candidates() {
    let mut connection = Connection::open_in_memory().expect("open v10 database");
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
    for migration in MIGRATIONS.iter().take(10) {
        connection
            .execute_batch(migration.sql)
            .expect("apply through v10 migration");
        connection
            .execute(
                "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                params![migration.version, migration.name],
            )
            .expect("record v10 migration");
    }
    connection
        .execute_batch(
            r#"
                INSERT INTO auto_find_runs (
                    run_id, revision, state, total_favorites,
                    completed_favorites, candidates_found,
                    started_at, updated_at, finished_at
                ) VALUES (
                    'v10-run', 1, 'completed', 1, 1, 1,
                    '2026-08-15T00:00:00Z', '2026-08-15T00:00:01Z',
                    '2026-08-15T00:00:01Z'
                );
                INSERT INTO auto_find_candidates (
                    run_id, gallery_id, title, artist, group_name, pages,
                    language, tags_json, published_rank, popularity,
                    thumbnail_key, thumbnail_width, thumbnail_height,
                    favorite_namespace, favorite_value, discovered_at
                ) VALUES (
                    'v10-run', 424242, 'Legacy candidate', 'artist', NULL, 12,
                    'english', '[]', 20260815, 0,
                    NULL, 512, 512, 'artist', 'artist',
                    '2026-08-15T00:00:00Z'
                );
            "#,
        )
        .expect("seed v10 Auto Find candidate");

    let report = MigrationRunner::run(&mut connection).expect("apply visible metadata migration");
    assert_eq!(
        report.applied_versions,
        vec![11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31]
    );
    let metadata: (String, String) = connection
        .query_row(
            r#"
                SELECT series_json, characters_json
                FROM auto_find_candidates
                WHERE run_id = 'v10-run' AND gallery_id = 424242
            "#,
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load compatible metadata defaults");
    assert_eq!(metadata, ("[]".into(), "[]".into()));
}

#[test]
fn settings_constraint_migration_clamps_legacy_values() {
    let mut connection = Connection::open_in_memory().expect("open legacy database");
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

                CREATE TABLE settings (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    revision INTEGER NOT NULL CHECK (revision >= 0),
                    download_root TEXT NOT NULL,
                    max_columns INTEGER NOT NULL CHECK (max_columns BETWEEN 1 AND 12),
                    preview_width INTEGER NOT NULL CHECK (preview_width BETWEEN 120 AND 1000),
                    cache_limit_gb INTEGER NOT NULL CHECK (cache_limit_gb BETWEEN 1 AND 1000),
                    concurrent_image_requests INTEGER NOT NULL CHECK (concurrent_image_requests BETWEEN 1 AND 30),
                    request_start_interval_ms INTEGER NOT NULL CHECK (request_start_interval_ms BETWEEN 0 AND 60000)
                ) STRICT;
                INSERT INTO settings VALUES (1, 7, '', 12, 1000, 1000, 30, 60000);

                CREATE TABLE window_placement (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    revision INTEGER NOT NULL CHECK (revision >= 0),
                    x INTEGER,
                    y INTEGER,
                    width INTEGER NOT NULL CHECK (width BETWEEN 1 AND 32768),
                    height INTEGER NOT NULL CHECK (height BETWEEN 1 AND 32768),
                    maximized INTEGER NOT NULL CHECK (maximized IN (0, 1))
                ) STRICT;
                INSERT INTO window_placement VALUES (1, 0, NULL, NULL, 1280, 820, 0);
            "#,
        )
        .expect("create legacy schema");

    let report = MigrationRunner::run(&mut connection).expect("upgrade legacy schema");
    assert_eq!(
        report.applied_versions,
        vec![
            2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
            26, 27, 28, 29, 30, 31
        ]
    );
    let tightened: (i64, i64, i64, i64, i64, i64, i64) = connection
        .query_row(
            r#"
                SELECT revision, max_columns, preview_width, cache_limit_gb,
                       concurrent_image_requests, request_start_interval_ms, privacy_mode
                FROM settings
            "#,
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .expect("read tightened settings");
    assert_eq!(tightened, (7, 4, 360, 30, 30, 5_000, 0));
    assert!(connection
        .execute("UPDATE settings SET max_columns = 5", [])
        .is_err());
}

#[test]
fn default_settings_match_the_approved_foundation_values() {
    let repository = SqliteRepository::open_in_memory().expect("create repository");
    let settings = repository.settings_get().expect("load settings");

    assert_eq!(settings, SettingsSnapshot::default());
    assert_eq!(
        serde_json::to_value(settings).expect("serialize settings"),
        json!({
            "revision": 0,
            "downloadRoot": "",
            "folderNameTemplate": "[{artist}] {title} [{group}] {id}",
            "maxColumns": 3,
            "previewWidth": 220,
            "relatedPreviewWidth": 240,
            "privacyMode": false,
            "cacheLimitGb": 10,
            "concurrentImageRequests": 5,
            "requestStartIntervalMs": 25,
            "autoFindHistoryMode": "include_all_history",
            "autoFindGrouping": "all",
            "downloadsGrouping": "all",
            "collapsedGroupKeys": [],
            "searchIncludeTags": [],
            "searchExcludeTags": []
        })
    );
}

#[test]
fn persisted_legacy_preview_width_is_normalized_on_load() {
    let temporary = tempfile::tempdir().expect("create temporary directory");
    let database_path = temporary.path().join("legacy-preview.sqlite3");
    drop(SqliteRepository::open(&database_path).expect("create repository"));
    let connection = Connection::open(&database_path).expect("open raw repository");
    connection
        .execute(
            "UPDATE settings SET preview_width=305 WHERE singleton=1",
            [],
        )
        .expect("seed legacy arbitrary preview width");
    drop(connection);

    let repository = SqliteRepository::open(&database_path).expect("reopen repository");
    assert_eq!(repository.settings_get().unwrap().preview_width, 280);
}

#[test]
fn folder_name_preview_uses_the_production_artifact_planner() {
    let repository = Arc::new(SqliteRepository::open_in_memory().expect("create repository"));
    let service = ApplicationService::new(repository);

    assert_eq!(
        service
            .folder_name_template_preview("[{artist}] {title} [{group}] {id}")
            .expect("preview default template"),
        "[작가] 작품 제목 [그룹] 4113714"
    );
    assert_eq!(
        service
            .folder_name_template_preview("{title}:<{artist}>* [{group}] {id}")
            .expect("preview sanitized template"),
        "작품 제목__작가__ [그룹] 4113714"
    );
    assert!(service.folder_name_template_preview("{title}").is_err());
}

#[cfg(windows)]
#[test]
fn settings_display_normalization_does_not_move_or_rewrite_existing_artifacts() {
    let temporary = tempfile::tempdir().expect("create temporary directory");
    let download_root = temporary.path().join("downloads");
    let artifact_directory = download_root.join("gallery-77");
    std::fs::create_dir_all(&artifact_directory).expect("create artifact fixture");
    let sentinel = artifact_directory.join("001.webp");
    std::fs::write(&sentinel, b"keep existing artifact bytes").expect("write artifact fixture");
    let canonical_root = download_root
        .canonicalize()
        .expect("canonicalize Windows download root");
    let canonical_text = canonical_root.to_string_lossy().into_owned();
    assert!(canonical_text.starts_with(r"\\?\"));

    let repository = Arc::new(SqliteRepository::open_in_memory().expect("create repository"));
    {
        let connection = repository.connection().expect("lock repository");
        connection
            .execute(
                "UPDATE settings SET download_root = ?1 WHERE singleton = 1",
                [&canonical_text],
            )
            .expect("seed legacy verbatim setting");
        connection
            .execute(
                "INSERT INTO galleries (gallery_id, revision, title, source_page_count) VALUES (77, 0, 'Existing artifact', 1)",
                [],
            )
            .expect("insert gallery");
        connection
            .execute(
                "INSERT INTO download_entries (entry_id, gallery_id, revision, state, progress, created_at, updated_at) VALUES ('entry-77', 77, 0, 'completed', 100, 'now', 'now')",
                [],
            )
            .expect("insert download entry");
        connection
            .execute(
                "INSERT INTO download_artifacts (entry_id, gallery_id, revision, relative_directory, expected_page_count, state, manifest_relative_path, hash_profile_version, root_snapshot) VALUES ('entry-77', 77, 0, 'gallery-77', 1, 'complete', 'gallery-77/manifest.json', 1, ?1)",
                [&canonical_text],
            )
            .expect("insert artifact root snapshot");
    }

    let service = ApplicationService::new(repository.clone());
    let expected_display = windows_path_for_display(&canonical_text);
    let displayed = service.settings_get().expect("load displayed settings");
    assert_eq!(displayed.download_root, expected_display);
    let updated = service
        .settings_update(
            SettingsPatch {
                max_columns: Some(4),
                ..SettingsPatch::default()
            },
            displayed.revision,
        )
        .expect("save unrelated setting");
    assert_eq!(updated.download_root, expected_display);

    let connection = repository.connection().expect("lock repository");
    let stored_setting: String = connection
        .query_row(
            "SELECT download_root FROM settings WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .expect("read normalized setting");
    let stored_artifact_root: String = connection
        .query_row(
            "SELECT root_snapshot FROM download_artifacts WHERE entry_id = 'entry-77'",
            [],
            |row| row.get(0),
        )
        .expect("read immutable artifact root");
    assert_eq!(stored_setting, expected_display);
    assert_eq!(
        std::path::Path::new(&stored_setting)
            .canonicalize()
            .expect("display root resolves to the same directory"),
        canonical_root
    );
    assert_eq!(stored_artifact_root, canonical_text);
    assert_eq!(
        std::fs::read(&sentinel).expect("existing artifact remains in place"),
        b"keep existing artifact bytes"
    );
}

#[test]
fn settings_validation_matches_the_approved_ui_ranges() {
    let limits = SettingsSnapshot {
        revision: 0,
        download_root: String::new(),
        folder_name_template: "[{artist}] {title} [{group}] {id}".into(),
        max_columns: 4,
        preview_width: 360,
        related_preview_width: 320,
        privacy_mode: false,
        cache_limit_gb: 30,
        concurrent_image_requests: 30,
        request_start_interval_ms: 5_000,
        auto_find_history_mode: AutoFindHistoryMode::IncludeAllHistory,
        auto_find_grouping: GalleryGroupingMode::All,
        downloads_grouping: GalleryGroupingMode::All,
        collapsed_group_keys: Vec::new(),
        search_include_tags: Vec::new(),
        search_exclude_tags: Vec::new(),
    };
    assert!(limits.validate().is_ok());
    for width in [160, 190, 220, 250, 280, 320, 360] {
        let mut preset = limits.clone();
        preset.preview_width = width;
        assert!(preset.validate().is_ok(), "preset {width}");
    }

    let mut invalid = limits.clone();
    invalid.max_columns = 5;
    assert!(invalid.validate().is_err());
    invalid = limits.clone();
    invalid.preview_width = 159;
    assert!(invalid.validate().is_err());
    invalid = limits.clone();
    invalid.preview_width = 361;
    assert!(invalid.validate().is_err());
    invalid = limits.clone();
    invalid.preview_width = 305;
    assert!(invalid.validate().is_err());
    invalid = limits.clone();
    invalid.related_preview_width = 170;
    assert!(invalid.validate().is_err());
    invalid = limits.clone();
    invalid.cache_limit_gb = 31;
    assert!(invalid.validate().is_err());
    for template in ["{title}", "{unknown} {id}", "{title {id}", "x\ny {id}"] {
        invalid = limits.clone();
        invalid.folder_name_template = template.into();
        assert!(invalid.validate().is_err(), "{template:?}");
    }
    invalid = limits;
    invalid.request_start_interval_ms = 5_001;
    assert!(invalid.validate().is_err());
}

#[test]
fn gallery_metadata_normalizes_blank_optional_group() {
    let metadata = GalleryMetadata::new(
        "  Gallery title  ",
        Some("  artist  ".into()),
        Some("   ".into()),
        12,
    )
    .expect("valid gallery metadata");

    assert_eq!(metadata.title, "Gallery title");
    assert_eq!(metadata.primary_artist.as_deref(), Some("artist"));
    assert_eq!(metadata.primary_group, None);

    let grouped = GalleryMetadata::new("Gallery title", None, Some("  paper studio  ".into()), 12)
        .expect("valid grouped gallery metadata");
    assert_eq!(grouped.primary_group.as_deref(), Some("paper studio"));
}

#[test]
fn settings_update_rejects_a_stale_revision() {
    let repository = Arc::new(SqliteRepository::open_in_memory().expect("create repository"));
    let service = ApplicationService::new(repository);

    let updated = service
        .settings_update(
            SettingsPatch {
                max_columns: Some(4),
                privacy_mode: Some(true),
                ..SettingsPatch::default()
            },
            0,
        )
        .expect("update current settings");
    assert_eq!(updated.revision, 1);
    assert_eq!(updated.max_columns, 4);
    assert!(updated.privacy_mode);

    let error = service
        .settings_update(
            SettingsPatch {
                preview_width: Some(260),
                ..SettingsPatch::default()
            },
            0,
        )
        .expect_err("stale update must fail");

    assert!(matches!(
        error,
        ApplicationError::RevisionConflict {
            resource: "settings",
            expected: 0,
            actual: 1,
        }
    ));
    let persisted = service.settings_get().expect("reload settings");
    assert_eq!(persisted.preview_width, 220);
    assert!(persisted.privacy_mode);
}

#[test]
fn window_placement_round_trips_through_sqlite() {
    let temporary = tempfile::tempdir().expect("create temporary directory");
    let database_path = temporary.path().join("atsumi-next.sqlite3");
    let expected = WindowPlacementSnapshot {
        revision: 1,
        x: Some(-1_920),
        y: Some(24),
        width: 1_600,
        height: 900,
        maximized: true,
    };

    {
        let repository =
            Arc::new(SqliteRepository::open(&database_path).expect("create persistent repository"));
        let service = ApplicationService::new(repository);
        assert_eq!(
            service
                .window_placement_get()
                .expect("load default placement"),
            WindowPlacementSnapshot::default()
        );
        let stored = service
            .window_placement_update(
                WindowPlacement {
                    x: expected.x,
                    y: expected.y,
                    width: expected.width,
                    height: expected.height,
                    maximized: expected.maximized,
                },
                0,
            )
            .expect("store window placement");
        assert_eq!(stored, expected);
    }

    let reopened = SqliteRepository::open(&database_path).expect("reopen repository");
    assert_eq!(
        reopened
            .window_placement_get()
            .expect("load persisted placement"),
        expected
    );
}

#[test]
fn repository_open_is_non_exclusive_and_recovery_is_an_explicit_app_operation() {
    let temporary = tempfile::tempdir().expect("create temporary directory");
    let database_path = temporary.path().join("exclusive-owner.sqlite3");
    let first_repository = Arc::new(
        SqliteRepository::open(&database_path).expect("open first application repository"),
    );
    let first_service = ApplicationService::new(first_repository.clone())
        .with_download_repository(first_repository.clone());
    let queued = first_service
        .download_queue_add(vec![4_051_038], "exclusive-owner-batch".into())
        .expect("queue a live entry");
    let entry_id = queued.entries[0].entry_id.clone();

    let second_repository = Arc::new(
        SqliteRepository::open(&database_path).expect("WAL permits a concurrent repository"),
    );
    let second_service = ApplicationService::new(second_repository.clone())
        .with_download_repository(second_repository.clone());

    let still_queued = first_service
        .download_entries_list(DownloadListRequest {
            state: Some(JobState::Queued),
            query: None,
            page: 1,
            page_size: 20,
        })
        .expect("the first instance remains authoritative");
    assert_eq!(still_queued.entries[0].entry_id, entry_id);
    assert_eq!(still_queued.entries[0].revision, 0);

    let observed_by_second = second_service
        .download_entries_list(DownloadListRequest {
            state: Some(JobState::Queued),
            query: None,
            page: 1,
            page_size: 20,
        })
        .expect("opening a repository does not mutate active jobs");
    assert_eq!(observed_by_second.entries[0].entry_id, entry_id);
    assert_eq!(observed_by_second.entries[0].revision, 0);

    drop(second_service);
    drop(second_repository);
    drop(first_service);
    drop(first_repository);
    let reopened =
        Arc::new(SqliteRepository::open(&database_path).expect("open after the first owner exits"));
    let reopened_service =
        ApplicationService::new(reopened.clone()).with_download_repository(reopened);
    assert_eq!(
        reopened_service
            .download_recover_interrupted()
            .expect("the single app owner recovers abandoned work"),
        1
    );
    let interrupted = reopened_service
        .download_entries_list(DownloadListRequest {
            state: Some(JobState::Interrupted),
            query: None,
            page: 1,
            page_size: 20,
        })
        .expect("recover abandoned active work only after ownership is released");
    assert_eq!(interrupted.entries[0].entry_id, entry_id);
    assert_eq!(interrupted.entries[0].revision, 1);
}

#[test]
fn duplicate_hidden_downloads_are_omitted_until_the_user_restores_them() {
    let repository = Arc::new(SqliteRepository::open_in_memory().expect("create repository"));
    let service = ApplicationService::new(repository.clone())
        .with_download_repository(repository.clone())
        .with_automation_repository(repository.clone());
    service
        .download_queue_add(
            vec![2_232_736, 4_051_038],
            "duplicate-hidden-download-list".into(),
        )
        .expect("seed downloads");
    repository
        .connection()
        .expect("open repository connection")
        .execute(
            "INSERT INTO duplicate_hidden_galleries (gallery_id, decision_id, created_at) VALUES (2232736, 'decision-hide-2232736', '2026-08-27T00:00:00Z')",
            [],
        )
        .expect("hide the duplicate gallery");

    let hidden = service
        .download_entries_list(DownloadListRequest {
            state: None,
            query: None,
            page: 1,
            page_size: 20,
        })
        .expect("list visible downloads");
    assert_eq!(hidden.total_items, 1);
    assert_eq!(hidden.entries[0].gallery_id.get(), 4_051_038);

    service
        .exploration_exclusions_restore(vec![2_232_736])
        .expect("restore the hidden gallery");
    let restored = service
        .download_entries_list(DownloadListRequest {
            state: None,
            query: None,
            page: 1,
            page_size: 20,
        })
        .expect("list downloads after restoration");
    assert_eq!(restored.total_items, 2);
    assert!(restored
        .entries
        .iter()
        .any(|entry| entry.gallery_id.get() == 2_232_736));
}

#[test]
fn api_result_serializes_the_exact_discriminated_envelope() {
    let success = ApiResult::success(json!({ "jobId": "job-1", "reused": false }));
    assert_eq!(
        serde_json::to_value(success).expect("serialize success"),
        json!({
            "ok": true,
            "data": { "jobId": "job-1", "reused": false }
        })
    );

    let failure: ApiResult<()> = ApiResult::failure(ApiError {
        code: "REVISION_CONFLICT".into(),
        message: "settings changed".into(),
        retryable: false,
        action: Some(ApiAction::Review),
        details: None,
    });
    assert_eq!(
        serde_json::to_value(failure).expect("serialize failure"),
        json!({
            "ok": false,
            "error": {
                "code": "REVISION_CONFLICT",
                "message": "settings changed",
                "retryable": false,
                "action": "review"
            }
        })
    );
}

#[test]
fn download_state_transitions_are_centralized_and_do_not_fake_cancellation() {
    assert!(JobState::Queued.allows_transition_to(JobState::ResolvingMetadata));
    assert!(JobState::Downloading.allows_transition_to(JobState::Cancelled));
    assert!(JobState::Interrupted.allows_transition_to(JobState::Queued));
    assert!(JobState::Interrupted.allows_transition_to(JobState::Failed));
    assert!(JobState::Cancelled.allows_transition_to(JobState::Queued));
    assert!(!JobState::Completed.allows_transition_to(JobState::Cancelled));
    assert!(!JobState::Cancelled.allows_transition_to(JobState::Interrupted));
    assert!(JobState::Queued.is_active());
    assert!(!JobState::Cancelled.is_active());
    assert_eq!(JobState::Cancelled.to_string(), "cancelled");
    assert_eq!(
        "cancelled".parse::<JobState>().expect("parse cancelled"),
        JobState::Cancelled
    );
}

#[test]
fn queued_fixture_job_stops_honestly_without_creating_artifacts() {
    let repository = Arc::new(SqliteRepository::open_in_memory().expect("create repository"));
    let service =
        ApplicationService::new(repository.clone()).with_download_repository(repository.clone());

    let first = service
        .download_queue_add(vec![42], "request-42".into())
        .expect("queue fixture job");
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.entries[0].revision, 0);
    assert_eq!(first.entries[0].state, JobState::Queued);
    let descriptor = first
        .jobs
        .first()
        .expect("new queue entry should schedule one fixture job")
        .clone();

    let replay = service
        .download_queue_add(vec![42], "request-42".into())
        .expect("replay queue request");
    assert_eq!(replay.entries, first.entries);
    assert!(replay.jobs.is_empty());

    let invalid = service
        .fixture_download_job_advance(
            &descriptor.job_id,
            descriptor.worker_attempt,
            FixtureDownloadJobStep::FoundationUnavailable,
        )
        .expect_err("fixture runner cannot skip its validating state");
    assert!(matches!(invalid, ApplicationError::Repository(_)));

    let resolving = service
        .fixture_download_job_advance(
            &descriptor.job_id,
            descriptor.worker_attempt,
            FixtureDownloadJobStep::ResolvingMetadata,
        )
        .expect("advance fixture validation");
    assert_eq!(resolving.job.revision, 1);
    assert_eq!(resolving.download.revision, 1);
    assert_eq!(resolving.job.state, JobState::ResolvingMetadata);

    let interrupted = service
        .fixture_download_job_advance(
            &descriptor.job_id,
            descriptor.worker_attempt,
            FixtureDownloadJobStep::FoundationUnavailable,
        )
        .expect("stop before an unavailable artifact pipeline");
    assert_eq!(interrupted.job.revision, 2);
    assert_eq!(interrupted.download.revision, 2);
    assert_eq!(interrupted.job.state, JobState::Interrupted);
    assert!(interrupted
        .job
        .message
        .as_deref()
        .is_some_and(|message| message.contains("artifact pipeline is not implemented")));

    let page = service
        .download_entries_list(DownloadListRequest {
            state: None,
            query: None,
            page: 1,
            page_size: 20,
        })
        .expect("read final fixture state");
    assert_eq!(page.entries[0].revision, 2);
    assert_eq!(page.entries[0].state, JobState::Interrupted);
    assert_eq!(page.entries[0].attempt, Some(1));
    assert_eq!(
        page.entries[0].error_code.as_deref(),
        Some("DOWNLOAD_FOUNDATION_UNAVAILABLE")
    );
    assert!(page.entries[0]
        .error_message
        .as_deref()
        .is_some_and(|message| message.contains("artifact pipeline is not implemented")));
    assert!(repository
        .artifact_bundle_get(&page.entries[0].entry_id)
        .expect("read artifact state")
        .is_none());
}

#[test]
fn download_cancel_is_atomic_and_idempotent() {
    let repository = Arc::new(SqliteRepository::open_in_memory().expect("create repository"));
    let service =
        ApplicationService::new(repository.clone()).with_download_repository(repository.clone());
    let queued = service
        .download_queue_add(vec![42, 43], "cancel-batch".into())
        .expect("queue downloads")
        .entries;
    let entry_ids = queued
        .iter()
        .map(|entry| entry.entry_id.to_string())
        .collect::<Vec<_>>();

    let cancelled = service
        .download_cancel(entry_ids.clone())
        .expect("cancel active downloads");
    assert!(cancelled
        .iter()
        .all(|entry| entry.state == JobState::Cancelled && entry.revision == 1));
    assert_eq!(
        service
            .download_active_count()
            .expect("count active downloads"),
        0
    );
    let visible_cancelled = service
        .download_entries_list(DownloadListRequest {
            state: Some(JobState::Cancelled),
            query: None,
            page: 1,
            page_size: 20,
        })
        .expect("list ordinary cancelled downloads");
    assert_eq!(visible_cancelled.total_items, 2);
    assert_eq!(visible_cancelled.entries.len(), 2);

    let replay = service
        .download_cancel(entry_ids)
        .expect("repeated cancellation is idempotent");
    assert_eq!(replay, cancelled);
}

#[test]
fn concurrent_retry_reuses_one_job_and_increments_one_attempt() {
    let temporary = tempfile::tempdir().expect("create temporary directory");
    let database_path = temporary.path().join("concurrent-retry.sqlite3");
    let repository =
        Arc::new(SqliteRepository::open(&database_path).expect("create persistent repository"));
    let service =
        ApplicationService::new(repository.clone()).with_download_repository(repository.clone());
    let launch = service
        .download_queue_add(vec![42], "retry-concurrency".into())
        .expect("queue fixture download");
    let descriptor = launch.jobs[0].clone();
    service
        .fixture_download_job_advance(
            &descriptor.job_id,
            descriptor.worker_attempt,
            FixtureDownloadJobStep::ResolvingMetadata,
        )
        .expect("start fixture attempt");
    service
        .fixture_download_job_advance(
            &descriptor.job_id,
            descriptor.worker_attempt,
            FixtureDownloadJobStep::FoundationUnavailable,
        )
        .expect("interrupt fixture attempt");

    let barrier = Arc::new(Barrier::new(3));
    let handles = (0..2)
        .map(|_| {
            let service = service.clone();
            let barrier = Arc::clone(&barrier);
            let entry_id = descriptor.entry_id.clone();
            thread::spawn(move || {
                barrier.wait();
                service
                    .download_retry(vec![entry_id])
                    .expect("retry download")
                    .remove(0)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let mut refs = handles
        .into_iter()
        .map(|handle| handle.join().expect("join retry thread"))
        .collect::<Vec<_>>();
    refs.sort_by_key(|job_ref| job_ref.reused);
    assert_eq!(refs[0].job_id, descriptor.job_id);
    assert_eq!(refs[1].job_id, descriptor.job_id);
    assert!(!refs[0].reused);
    assert!(refs[1].reused);

    let queued = service
        .download_entries_list(DownloadListRequest {
            state: Some(JobState::Queued),
            query: None,
            page: 1,
            page_size: 20,
        })
        .expect("read retried entry");
    assert_eq!(queued.entries[0].revision, 3);
    assert_eq!(queued.entries[0].progress, Some(0.0));
    assert_eq!(queued.entries[0].attempt, Some(2));
    assert!(queued.entries[0].error_code.is_none());
    assert!(queued.entries[0].error_message.is_none());

    drop(service);
    drop(repository);
    let connection = Connection::open(&database_path).expect("inspect retry metadata");
    let lifecycle: (i64, i64, i64) = connection
        .query_row(
            r#"
                SELECT
                    j.attempt,
                    (SELECT COUNT(*) FROM download_attempts a WHERE a.job_id = j.job_id),
                    j.revision
                FROM download_jobs j
            "#,
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read retry lifecycle");
    assert_eq!(lifecycle, (2, 2, 3));
}

#[test]
fn cancelling_an_interrupted_download_preserves_attempt_failure_evidence() {
    let temporary = tempfile::tempdir().expect("create temporary directory");
    let database_path = temporary.path().join("cancelled-evidence.sqlite3");
    let repository =
        Arc::new(SqliteRepository::open(&database_path).expect("create persistent repository"));
    let service =
        ApplicationService::new(repository.clone()).with_download_repository(repository.clone());
    let launch = service
        .download_queue_add(vec![42], "cancelled-evidence".into())
        .expect("queue fixture download");
    let descriptor = launch.jobs[0].clone();
    service
        .fixture_download_job_advance(
            &descriptor.job_id,
            descriptor.worker_attempt,
            FixtureDownloadJobStep::ResolvingMetadata,
        )
        .expect("start fixture attempt");
    service
        .fixture_download_job_advance(
            &descriptor.job_id,
            descriptor.worker_attempt,
            FixtureDownloadJobStep::FoundationUnavailable,
        )
        .expect("record fixture failure");

    let cancelled = service
        .download_cancel(vec![descriptor.entry_id.clone()])
        .expect("cancel interrupted entry");
    assert_eq!(cancelled[0].state, JobState::Cancelled);
    assert_eq!(cancelled[0].attempt, Some(1));
    assert_eq!(
        cancelled[0].error_code.as_deref(),
        Some("DOWNLOAD_FOUNDATION_UNAVAILABLE")
    );

    drop(service);
    drop(repository);
    let connection = Connection::open(&database_path).expect("inspect failure evidence");
    let evidence: (Option<String>, Option<String>, Option<String>) = connection
        .query_row(
            r#"
                SELECT j.last_error_code, a.outcome_state, a.error_code
                FROM download_jobs j
                JOIN download_attempts a
                  ON a.job_id = j.job_id AND a.attempt = j.attempt
            "#,
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read preserved failure evidence");
    assert_eq!(
        evidence,
        (
            Some("DOWNLOAD_FOUNDATION_UNAVAILABLE".into()),
            Some("interrupted".into()),
            Some("DOWNLOAD_FOUNDATION_UNAVAILABLE".into()),
        )
    );
}

#[test]
fn stale_fixture_worker_cannot_advance_a_retried_attempt() {
    let temporary = tempfile::tempdir().expect("create temporary directory");
    let repository = Arc::new(
        SqliteRepository::open(temporary.path().join("stale-worker.sqlite3"))
            .expect("create repository"),
    );
    let service = ApplicationService::new(repository.clone()).with_download_repository(repository);
    let launch = service
        .download_queue_add(vec![42], "stale-worker".into())
        .expect("queue fixture download");
    let descriptor = launch.jobs[0].clone();
    service
        .fixture_download_job_advance(
            &descriptor.job_id,
            descriptor.worker_attempt,
            FixtureDownloadJobStep::ResolvingMetadata,
        )
        .expect("start first attempt");
    service
        .fixture_download_job_advance(
            &descriptor.job_id,
            descriptor.worker_attempt,
            FixtureDownloadJobStep::FoundationUnavailable,
        )
        .expect("interrupt first attempt");

    let retry = service
        .download_retry(vec![descriptor.entry_id.clone()])
        .expect("retry download")
        .remove(0);
    assert_eq!(descriptor.worker_attempt, 1);
    assert_eq!(retry.worker_attempt, 2);

    service
        .fixture_download_job_advance(
            &descriptor.job_id,
            descriptor.worker_attempt,
            FixtureDownloadJobStep::ResolvingMetadata,
        )
        .expect_err("the first worker must not own the retried attempt");
    let queued = service
        .download_entries_list(DownloadListRequest {
            state: Some(JobState::Queued),
            query: None,
            page: 1,
            page_size: 20,
        })
        .expect("stale worker leaves the retry queued");
    assert_eq!(queued.entries.len(), 1);

    service
        .fixture_download_job_advance(
            &retry.job_id,
            retry.worker_attempt,
            FixtureDownloadJobStep::ResolvingMetadata,
        )
        .expect("the current worker owns the retried attempt");
    service
        .fixture_download_job_advance(
            &descriptor.job_id,
            descriptor.worker_attempt,
            FixtureDownloadJobStep::FoundationUnavailable,
        )
        .expect_err("a stale completion cannot interrupt the current attempt");
    let resolving = service
        .download_entries_list(DownloadListRequest {
            state: Some(JobState::ResolvingMetadata),
            query: None,
            page: 1,
            page_size: 20,
        })
        .expect("current attempt remains active");
    assert_eq!(resolving.entries.len(), 1);
}

#[test]
fn download_mutation_errors_are_stable_and_batches_roll_back() {
    let temporary = tempfile::tempdir().expect("create temporary directory");
    let database_path = temporary.path().join("download-mutation-errors.sqlite3");
    let (first_id, completed_id) = {
        let repository =
            Arc::new(SqliteRepository::open(&database_path).expect("create persistent repository"));
        let service = ApplicationService::new(repository.clone())
            .with_download_repository(repository.clone());
        let entries = service
            .download_queue_add(vec![42, 43], "mutation-errors".into())
            .expect("queue downloads")
            .entries;
        (
            entries[0].entry_id.to_string(),
            entries[1].entry_id.to_string(),
        )
    };
    let connection = Connection::open(&database_path).expect("open raw database");
    connection
        .execute(
            "UPDATE download_entries SET state = 'completed' WHERE entry_id = ?1",
            [&completed_id],
        )
        .expect("mark entry completed");
    connection
        .execute(
            "UPDATE download_jobs SET state = 'completed' WHERE entry_id = ?1",
            [&completed_id],
        )
        .expect("mark job completed");
    drop(connection);

    let repository =
        Arc::new(SqliteRepository::open(&database_path).expect("reopen and recover active entry"));
    let service =
        ApplicationService::new(repository.clone()).with_download_repository(repository.clone());
    assert_eq!(
        service
            .download_recover_interrupted()
            .expect("recover the abandoned active entry"),
        1
    );
    let invalid = service
        .download_cancel(vec![first_id.clone(), completed_id.clone()])
        .expect_err("completed entry makes the whole batch invalid");
    assert!(matches!(
        invalid,
        ApplicationError::InvalidDownloadState {
            ref entry_id,
            state: JobState::Completed,
            operation: "cancel"
        } if entry_id.as_str() == completed_id
    ));
    let invalid_api = ApiError::from(invalid);
    assert_eq!(invalid_api.code, "INVALID_DOWNLOAD_STATE");
    assert_eq!(invalid_api.action, Some(ApiAction::Review));

    let first = service
        .download_entries_list(DownloadListRequest {
            state: Some(JobState::Interrupted),
            query: Some(first_id.clone()),
            page: 1,
            page_size: 20,
        })
        .expect("batch rollback preserves first entry");
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.entries[0].revision, 1);

    let missing = service
        .download_retry(vec!["missing-entry".into()])
        .expect_err("missing retry target fails");
    let missing_api = ApiError::from(missing);
    assert_eq!(missing_api.code, "DOWNLOAD_ENTRY_NOT_FOUND");
    assert_eq!(missing_api.action, Some(ApiAction::None));
}

#[test]
fn download_queue_is_batch_idempotent_and_reuses_active_gallery_entries() {
    let repository = Arc::new(SqliteRepository::open_in_memory().expect("create repository"));
    let service =
        ApplicationService::new(repository.clone()).with_download_repository(repository.clone());

    let first = service
        .download_queue_add(vec![42, 7, 42], "queue-batch-1".into())
        .expect("queue normalized gallery batch")
        .entries;
    assert_eq!(
        first
            .iter()
            .map(|entry| entry.gallery_id.get())
            .collect::<Vec<_>>(),
        vec![7, 42]
    );
    assert!(first.iter().all(|entry| entry.state == JobState::Queued));
    assert_eq!(
        service
            .download_active_count()
            .expect("count active downloads"),
        2
    );
    let active_entry_ids = service
        .download_active_entry_ids()
        .expect("list active download identities");
    let mut expected_active_entry_ids = first
        .iter()
        .map(|entry| entry.entry_id.clone())
        .collect::<Vec<_>>();
    expected_active_entry_ids.sort();
    assert_eq!(active_entry_ids, expected_active_entry_ids);

    let replay = service
        .download_queue_add(vec![42, 7], " queue-batch-1 ".into())
        .expect("replay equivalent normalized batch")
        .entries;
    assert_eq!(replay, first);

    let conflict = service
        .download_queue_add(vec![7], "queue-batch-1".into())
        .expect_err("request ID cannot be reused for a different batch");
    assert!(matches!(
        conflict,
        ApplicationError::IdempotencyConflict { ref request_id }
            if request_id == "queue-batch-1"
    ));
    let conflict = ApiError::from(conflict);
    assert_eq!(conflict.code, "IDEMPOTENCY_CONFLICT");
    assert!(!conflict.retryable);
    assert_eq!(conflict.action, Some(ApiAction::Review));

    let active_reuse = service
        .download_queue_add(vec![42], "queue-batch-2".into())
        .expect("reuse active gallery entry for a new request")
        .entries;
    let gallery_42 = first
        .iter()
        .find(|entry| entry.gallery_id.get() == 42)
        .expect("gallery 42 entry");
    assert_eq!(active_reuse, vec![gallery_42.clone()]);

    let first_page = service
        .download_entries_list(DownloadListRequest {
            state: Some(JobState::Queued),
            query: None,
            page: 1,
            page_size: 1,
        })
        .expect("list first queued page");
    assert_eq!(first_page.total_items, 2);
    assert_eq!(first_page.entries.len(), 1);
    assert_eq!(first_page.entries[0].gallery_id.get(), 7);
    let second_page = service
        .download_entries_list(DownloadListRequest {
            state: Some(JobState::Queued),
            query: None,
            page: 2,
            page_size: 1,
        })
        .expect("list second queued page");
    assert_eq!(second_page.total_items, 2);
    assert_eq!(second_page.entries.len(), 1);
    assert_eq!(second_page.entries[0].gallery_id.get(), 42);

    let queried = service
        .download_entries_list(DownloadListRequest {
            state: None,
            query: Some(" 42 ".into()),
            page: 1,
            page_size: 20,
        })
        .expect("query downloads by gallery ID");
    assert!(queried.total_items >= 1);
    assert!(queried
        .entries
        .iter()
        .any(|entry| entry.entry_id == gallery_42.entry_id));

    let literal_wildcard = service
        .download_entries_list(DownloadListRequest {
            state: None,
            query: Some("%".into()),
            page: 1,
            page_size: 20,
        })
        .expect("treat query metacharacters as literal text");
    assert_eq!(literal_wildcard.total_items, 0);

    let too_many = service
        .download_queue_add(vec![99; 201], "too-many-galleries".into())
        .expect_err("raw gallery list is capped before normalization");
    assert!(matches!(
        too_many,
        ApplicationError::Validation(ref error) if error.field == "galleries"
    ));
    let invalid_gallery = service
        .download_queue_add(vec![0], "invalid-gallery".into())
        .expect_err("gallery IDs must be positive");
    assert!(matches!(
        invalid_gallery,
        ApplicationError::Validation(ref error)
            if error.field == "galleries"
                && error.message == "gallery IDs must be positive integers"
    ));

    let oversized_query = service
        .download_entries_list(DownloadListRequest {
            state: None,
            query: Some("한".repeat(167)),
            page: 1,
            page_size: 20,
        })
        .expect_err("download query is limited by UTF-8 byte length");
    assert!(matches!(
        oversized_query,
        ApplicationError::Validation(ref error)
            if error.field == "query" && error.message == "must be at most 500 bytes"
    ));
}

#[test]
fn active_download_identities_use_the_exact_interruptible_state_set() {
    let repository = Arc::new(SqliteRepository::open_in_memory().expect("create repository"));
    let service =
        ApplicationService::new(repository.clone()).with_download_repository(repository.clone());
    let entries = service
        .download_queue_add((1..=11).collect(), "active-state-coverage".into())
        .expect("queue state coverage fixtures")
        .entries;
    let states = [
        "queued",
        "resolving_metadata",
        "downloading",
        "hashing",
        "verifying",
        "retry_wait",
        "completed",
        "failed",
        "cancelled",
        "interrupted",
        "review_required",
    ];
    {
        let connection = repository.connection().expect("open fixture connection");
        for (entry, state) in entries.iter().zip(states) {
            connection
                .execute(
                    "UPDATE download_entries SET state = ?1 WHERE entry_id = ?2",
                    params![state, entry.entry_id.as_str()],
                )
                .expect("seed canonical download state");
        }
    }

    let mut expected = entries
        .iter()
        .take(6)
        .map(|entry| entry.entry_id.clone())
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(
        service
            .download_active_entry_ids()
            .expect("read active download identities"),
        expected
    );
    assert_eq!(
        service
            .download_active_count()
            .expect("count active downloads"),
        6
    );
}

#[test]
fn exploration_reset_is_explicit_atomic_and_preserves_download_records() {
    let repository = Arc::new(SqliteRepository::open_in_memory().expect("create repository"));
    let service = ApplicationService::new(repository.clone())
        .with_download_repository(repository.clone())
        .with_automation_repository(repository.clone());
    service
        .favorite_set(
            FavoriteKey {
                namespace: FavoriteNamespace::Artist,
                value: "fixture artist".into(),
            },
            true,
        )
        .unwrap();
    repository
        .search_history_record(&SearchRequest {
            text: "fixture".into(),
            include_tags: vec![],
            exclude_tags: vec![],
            languages: vec![Language::Korean],
            sort: SearchSort::Recent,
            page_size: 20,
        })
        .unwrap();
    let run = repository
        .auto_find_start(0, AutoFindHistoryMode::IncludeAllHistory, &[])
        .unwrap();
    repository
        .auto_find_finish(&run.run_id, AutoFindRunState::Completed, None, None)
        .unwrap();
    repository
        .auto_find_exclude(&[GalleryId::new(998).unwrap()], "fixture exclusion")
        .unwrap();
    service
        .download_queue_add(vec![997], "reset-preserves-download".into())
        .unwrap();

    let reset = service
        .exploration_data_reset(ExplorationDataResetRequest {
            confirmation: "RESET_EXPLORATION_DATA".into(),
        })
        .unwrap();
    assert_eq!(reset.favorites_removed, 1);
    assert_eq!(reset.search_history_removed, 1);
    assert_eq!(reset.auto_find_runs_removed, 1);
    assert_eq!(reset.auto_find_candidates_removed, 0);
    assert_eq!(reset.auto_find_exclusions_removed, 1);
    assert!(service.favorites_list().unwrap().is_empty());
    assert!(service.search_history_list(20).unwrap().is_empty());
    assert_eq!(
        service
            .download_entries_list(DownloadListRequest {
                state: None,
                query: None,
                page: 1,
                page_size: 20,
            })
            .unwrap()
            .total_items,
        1
    );
}

#[test]
fn explicit_exploration_exclusions_are_listed_and_can_be_restored() {
    let repository = Arc::new(SqliteRepository::open_in_memory().expect("create repository"));
    let service =
        ApplicationService::new(repository.clone()).with_automation_repository(repository.clone());
    let gallery_id = GalleryId::new(998).unwrap();
    repository
        .auto_find_exclude(&[gallery_id], "사용자 제외")
        .expect("exclude gallery");

    let exclusions = service
        .exploration_exclusions_list()
        .expect("list exclusions");
    assert_eq!(exclusions.len(), 1);
    assert_eq!(exclusions[0].gallery_id, gallery_id);
    assert_eq!(exclusions[0].title, "Gallery #998");
    assert_eq!(exclusions[0].reasons[0].detail, "사용자 제외");

    let restored = service
        .exploration_exclusions_restore(vec![998])
        .expect("restore gallery");
    assert_eq!(restored.restored_gallery_ids, vec![gallery_id]);
    assert!(service.exploration_exclusions_list().unwrap().is_empty());

    repository
        .auto_find_exclude(&[gallery_id], "다시 제외")
        .expect("exclude restored gallery again");
    assert_eq!(service.exploration_exclusions_list().unwrap().len(), 1);
}

#[test]
fn exploration_reset_rejects_bad_confirmation_and_running_auto_find() {
    let repository = Arc::new(SqliteRepository::open_in_memory().expect("create repository"));
    let service =
        ApplicationService::new(repository.clone()).with_automation_repository(repository.clone());
    assert!(matches!(
        service.exploration_data_reset(ExplorationDataResetRequest {
            confirmation: "RESET".into(),
        }),
        Err(ApplicationError::Validation(_))
    ));
    repository
        .auto_find_start(0, AutoFindHistoryMode::IncludeAllHistory, &[])
        .unwrap();
    assert!(matches!(
        service.exploration_data_reset(ExplorationDataResetRequest {
            confirmation: "RESET_EXPLORATION_DATA".into(),
        }),
        Err(ApplicationError::Repository(
            RepositoryError::OperationActive(_)
        ))
    ));
}

#[test]
fn volatile_download_state_recovers_as_interrupted_after_reopen() {
    let temporary = tempfile::tempdir().expect("create temporary directory");
    let database_path = temporary.path().join("download-recovery.sqlite3");
    let original_entry_id = {
        let repository =
            Arc::new(SqliteRepository::open(&database_path).expect("create persistent repository"));
        let service =
            ApplicationService::new(repository.clone()).with_download_repository(repository);
        service
            .download_queue_add(vec![4_051_038], "recovery-batch".into())
            .expect("queue persistent gallery")
            .entries
            .remove(0)
            .entry_id
    };

    let connection = Connection::open(&database_path).expect("open persisted database directly");
    connection
        .execute(
            "UPDATE download_entries SET state = 'downloading', progress = 37.0",
            [],
        )
        .expect("simulate volatile download checkpoint");
    connection
        .execute(
            "UPDATE download_jobs SET state = 'downloading', completed_units = 37, total_units = 100",
            [],
        )
        .expect("simulate volatile job checkpoint");
    drop(connection);

    let repository =
        Arc::new(SqliteRepository::open(&database_path).expect("reopen and recover repository"));
    let service =
        ApplicationService::new(repository.clone()).with_download_repository(repository.clone());
    assert_eq!(
        service
            .download_recover_interrupted()
            .expect("the application owner performs startup recovery"),
        1
    );
    let interrupted = service
        .download_entries_list(DownloadListRequest {
            state: Some(JobState::Interrupted),
            query: None,
            page: 1,
            page_size: 20,
        })
        .expect("reconstruct interrupted state from SQLite");
    assert_eq!(interrupted.total_items, 1);
    assert_eq!(interrupted.entries[0].entry_id, original_entry_id);
    assert_eq!(interrupted.entries[0].revision, 1);
    assert_eq!(interrupted.entries[0].progress, Some(37.0));
    assert_eq!(
        service
            .download_active_count()
            .expect("recovered jobs are not active"),
        0
    );

    let replay = service
        .download_queue_add(vec![4_051_038], "recovery-batch".into())
        .expect("idempotent replay returns its original response snapshot")
        .entries;
    assert_eq!(replay[0].entry_id, original_entry_id);
    assert_eq!(replay[0].revision, 0);
    assert_eq!(replay[0].state, JobState::Queued);
    assert_eq!(replay[0].progress, Some(0.0));

    let replacement = service
        .download_queue_add(vec![4_051_038], "post-recovery-batch".into())
        .expect("an interrupted entry is not active and is not auto-resumed")
        .entries;
    assert_ne!(replacement[0].entry_id, original_entry_id);
    assert_eq!(replacement[0].state, JobState::Queued);

    drop(service);
    drop(repository);
    let connection = Connection::open(&database_path).expect("inspect recovered job state");
    let interrupted_jobs: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM download_jobs WHERE state = 'interrupted'",
            [],
            |row| row.get(0),
        )
        .expect("count interrupted jobs");
    assert_eq!(interrupted_jobs, 1);
}

#[test]
fn targetless_legacy_artifact_review_recovers_as_a_listable_failure() {
    let temporary = tempfile::tempdir().expect("create temporary directory");
    let database_path = temporary.path().join("artifact-review-recovery.sqlite3");
    let (original_entry_id, valid_review_entry_id) = {
        let repository =
            Arc::new(SqliteRepository::open(&database_path).expect("create persistent repository"));
        let service =
            ApplicationService::new(repository.clone()).with_download_repository(repository);
        let entries = service
            .download_queue_add(
                vec![4_136_275, 4_113_714],
                "artifact-review-recovery".into(),
            )
            .expect("queue persistent gallery")
            .entries;
        (entries[0].entry_id.clone(), entries[1].entry_id.clone())
    };

    let connection = Connection::open(&database_path).expect("open persisted database directly");
    connection
        .execute(
            r#"
                UPDATE download_entries
                SET state = 'review_required', review_kind = NULL, review_id = NULL
                WHERE entry_id = ?1
            "#,
            [original_entry_id.as_str()],
        )
        .expect("simulate legacy targetless artifact review entry");
    connection
        .execute(
            r#"
                UPDATE download_jobs
                SET state = 'review_required',
                    last_error_code = 'RECOVERY_CONFLICT',
                    last_error_message = 'Ambiguous page files were moved aside for review',
                    last_error_retryable = NULL
                WHERE entry_id = ?1
            "#,
            [original_entry_id.as_str()],
        )
        .expect("simulate legacy targetless artifact review job");
    connection
        .execute(
            r#"
                UPDATE download_entries
                SET state = 'review_required',
                    review_kind = 'internal_pages',
                    review_id = 'candidate-preserved'
                WHERE entry_id = ?1
            "#,
            [valid_review_entry_id.as_str()],
        )
        .expect("seed a valid user review entry");
    connection
        .execute(
            "UPDATE download_jobs SET state = 'review_required' WHERE entry_id = ?1",
            [valid_review_entry_id.as_str()],
        )
        .expect("seed a valid user review job");
    drop(connection);

    let repository = Arc::new(SqliteRepository::open(&database_path).expect("reopen repository"));
    let service =
        ApplicationService::new(repository.clone()).with_download_repository(repository.clone());
    assert!(matches!(
        service.download_entries_list(DownloadListRequest {
            state: None,
            query: None,
            page: 1,
            page_size: 20,
        }),
        Err(ApplicationError::Repository(RepositoryError::Corrupt(_)))
    ));

    assert_eq!(
        service
            .download_recover_interrupted()
            .expect("repair targetless artifact review during startup recovery"),
        1
    );
    let page = service
        .download_entries_list(DownloadListRequest {
            state: None,
            query: None,
            page: 1,
            page_size: 20,
        })
        .expect("the repaired entry remains visible");
    assert_eq!(page.total_items, 2);
    let repaired = page
        .entries
        .iter()
        .find(|entry| entry.entry_id == original_entry_id)
        .expect("repaired entry remains visible");
    assert_eq!(repaired.entry_id, original_entry_id);
    assert_eq!(repaired.state, JobState::Failed);
    assert_eq!(repaired.error_code.as_deref(), Some("RECOVERY_CONFLICT"));
    assert_eq!(repaired.error_retryable, Some(false));
    assert_eq!(repaired.review_kind, None);
    assert_eq!(repaired.review_id, None);
    let preserved = page
        .entries
        .iter()
        .find(|entry| entry.entry_id == valid_review_entry_id)
        .expect("valid user review remains visible");
    assert_eq!(preserved.state, JobState::ReviewRequired);
    assert_eq!(
        preserved.review_kind,
        Some(crate::domain::DownloadReviewKind::InternalPages)
    );
    assert_eq!(preserved.review_id.as_deref(), Some("candidate-preserved"));
    assert_eq!(
        service
            .download_recover_interrupted()
            .expect("repeated recovery is idempotent"),
        0
    );

    drop(service);
    drop(repository);
    let connection = Connection::open(&database_path).expect("inspect repaired database");
    let attempt: (String, Option<String>, Option<i64>) = connection
        .query_row(
            r#"
                SELECT outcome_state, error_code, error_retryable
                FROM download_attempts
                WHERE job_id = (
                    SELECT job_id FROM download_jobs WHERE entry_id = ?1
                )
            "#,
            [original_entry_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("inspect repaired attempt");
    assert_eq!(
        attempt,
        ("failed".into(), Some("RECOVERY_CONFLICT".into()), Some(0))
    );
    let quick_check: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .expect("quick check repaired database");
    assert_eq!(quick_check, "ok");
}

#[test]
fn download_command_payloads_and_results_match_typescript_contracts() {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct QueuePayload {
        galleries: Vec<i64>,
        request_id: String,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct ListPayload {
        request: DownloadListRequest,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct MutationPayload {
        entry_ids: Vec<String>,
    }

    let queue: QueuePayload = serde_json::from_value(json!({
        "galleries": [4051038, 4051027],
        "requestId": "selection-1"
    }))
    .expect("deserialize download_queue_add payload");
    assert_eq!(queue.galleries, vec![4_051_038, 4_051_027]);
    assert_eq!(queue.request_id, "selection-1");

    let list: ListPayload = serde_json::from_value(json!({
        "request": {
            "state": "interrupted",
            "query": "serein",
            "page": 2,
            "pageSize": 50
        }
    }))
    .expect("deserialize download_entries_list payload");
    assert_eq!(list.request.state, Some(JobState::Interrupted));
    assert_eq!(list.request.page_size, 50);

    let mutation: MutationPayload = serde_json::from_value(json!({
        "entryIds": ["entry-1", "entry-2"]
    }))
    .expect("deserialize retry/cancel payload");
    assert_eq!(mutation.entry_ids, vec!["entry-1", "entry-2"]);
    assert_eq!(
        serde_json::to_value(ApiResult::success(vec![JobRef {
            job_id: "job-contract".into(),
            reused: false,
            worker_attempt: 7,
        }]))
        .expect("serialize download retry result"),
        json!({
            "ok": true,
            "data": [{ "jobId": "job-contract", "reused": false }]
        })
    );

    let entry = DownloadEntry {
        entry_id: DownloadEntryId::new("entry-contract").expect("valid entry ID"),
        gallery_id: GalleryId::new(4_051_038).expect("valid gallery ID"),
        revision: 7,
        state: JobState::Queued,
        progress: Some(0.0),
        attempt: None,
        error_code: None,
        error_message: None,
        error_retryable: None,
        review_kind: None,
        review_id: None,
        created_at: None,
        updated_at: None,
    };
    assert_eq!(
        serde_json::to_value(ApiResult::success(vec![entry]))
            .expect("serialize download queue result"),
        json!({
            "ok": true,
            "data": [{
                "entryId": "entry-contract",
                "galleryId": 4051038,
                "revision": 7,
                "state": "queued",
                "progress": 0.0
            }]
        })
    );
}

#[test]
fn gallery_and_artifacts_round_trip_with_original_page_numbers() {
    let temporary = tempfile::tempdir().expect("create temporary directory");
    let database_path = temporary.path().join("artifacts.sqlite3");

    let expected = {
        let repository =
            Arc::new(SqliteRepository::open(&database_path).expect("create artifact repository"));
        let service = ApplicationService::new(repository.clone())
            .with_download_repository(repository.clone());
        let launch = service
            .download_queue_add(vec![3_005_910], "artifact-round-trip".into())
            .expect("create owning download entry");
        let descriptor = launch
            .jobs
            .first()
            .expect("new fixture download entry")
            .clone();
        let entry_id = DownloadEntryId::new(descriptor.entry_id).expect("valid entry id");
        let gallery_id = GalleryId::new(3_005_910).expect("valid gallery id");
        let directory = ArtifactRelativePath::new("[artist] immutable pages")
            .expect("valid artifact directory");
        let gallery = Gallery::new(
            gallery_id,
            2,
            GalleryMetadata::new(
                "Immutable source page example",
                Some("artist".into()),
                Some("paper studio".into()),
                3,
            )
            .expect("valid gallery metadata"),
        );
        let artifact = DownloadArtifact::new(
            entry_id.clone(),
            gallery_id,
            4,
            directory,
            3,
            DownloadArtifactState::Incomplete,
        )
        .expect("valid download artifact");
        let pages = [1_u32, 3]
            .into_iter()
            .map(|number| {
                PageArtifact::new(
                    entry_id.clone(),
                    gallery_id,
                    SourcePageNumber::new(number).expect("one-based page number"),
                    ArtifactRelativePath::new(format!("[artist] immutable pages/{number:03}.webp"))
                        .expect("valid page path"),
                    PageArtifactState::Present,
                    Some(u64::from(number) * 1_024),
                )
                .expect("valid page artifact")
            })
            .collect();
        let bundle = ArtifactBundle::new(gallery, artifact, pages).expect("valid artifact bundle");

        repository
            .artifact_bundle_replace(&bundle)
            .expect("store artifact bundle");
        assert_eq!(
            repository
                .artifact_bundle_get(&entry_id)
                .expect("load artifact bundle"),
            Some(bundle.clone())
        );
        bundle
    };

    let reopened = SqliteRepository::open(&database_path).expect("reopen artifact repository");
    assert_eq!(
        reopened
            .artifact_bundle_get(&expected.artifact.entry_id)
            .expect("load persisted artifact bundle"),
        Some(expected)
    );
}

#[test]
fn artifact_paths_cannot_escape_the_download_root() {
    assert!(ArtifactRelativePath::new("gallery/001.webp").is_ok());
    assert!(ArtifactRelativePath::new("../outside.webp").is_err());
    assert!(ArtifactRelativePath::new("C:\\outside.webp").is_err());
}

#[test]
fn fixture_search_supports_recent_pagination_and_stable_query_keys() {
    let service = fixture_search_service();
    let request = SearchRequest {
        text: "  ".into(),
        include_tags: Vec::new(),
        exclude_tags: Vec::new(),
        languages: vec![Language::Korean],
        sort: SearchSort::Recent,
        page_size: 2,
    };

    let first = service
        .search_submit(request.clone())
        .expect("submit fixture search");
    assert_eq!(
        first
            .first_page
            .items
            .iter()
            .map(|gallery| gallery.id.get())
            .collect::<Vec<_>>(),
        vec![4_051_038, 4_051_027]
    );
    assert_eq!(first.first_page.total_pages, 2);

    let second_page = service
        .search_page_get(first.query_id.clone(), 2)
        .expect("load second fixture page");
    assert_eq!(
        second_page
            .items
            .iter()
            .map(|gallery| gallery.id.get())
            .collect::<Vec<_>>(),
        vec![4_050_754]
    );

    let repeated = service
        .search_submit(request)
        .expect("repeat equivalent fixture search");
    assert_eq!(repeated.query_id, first.query_id);
}

#[test]
fn fixture_search_applies_tag_language_and_group_clauses() {
    let service = fixture_search_service();
    let tag_result = service
        .search_submit(SearchRequest {
            text: String::new(),
            include_tags: vec![" FEMALE:SWIMSUIT ".into()],
            exclude_tags: vec!["sports".into()],
            languages: vec![Language::Korean, Language::Korean],
            sort: SearchSort::Recent,
            page_size: 20,
        })
        .expect("search fixture tags");
    assert_eq!(tag_result.first_page.items.len(), 1);
    assert_eq!(tag_result.first_page.items[0].id.get(), 4_051_027);

    let group_result = service
        .search_submit(SearchRequest {
            text: "GROUP:Paper Studio".into(),
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            languages: Vec::new(),
            sort: SearchSort::Recent,
            page_size: 20,
        })
        .expect("search fixture group");
    assert_eq!(group_result.first_page.items.len(), 1);
    assert_eq!(
        group_result.first_page.items[0].group.as_deref(),
        Some("paper studio")
    );

    let series_result = service
        .search_submit(SearchRequest {
            text: "series:rain_archives".into(),
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            languages: Vec::new(),
            sort: SearchSort::Recent,
            page_size: 20,
        })
        .expect("search fixture series chip token");
    assert_eq!(
        series_result
            .first_page
            .items
            .iter()
            .map(|gallery| gallery.id.get())
            .collect::<Vec<_>>(),
        vec![4_051_038, 4_050_754]
    );

    let character_result = service
        .search_submit(SearchRequest {
            text: "character:mira_lane".into(),
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            languages: Vec::new(),
            sort: SearchSort::Recent,
            page_size: 20,
        })
        .expect("search fixture character chip token");
    assert_eq!(
        character_result
            .first_page
            .items
            .iter()
            .map(|gallery| gallery.id.get())
            .collect::<Vec<_>>(),
        vec![4_051_038, 4_050_754]
    );
}

#[test]
fn fixture_gallery_detail_matches_the_typescript_projection() {
    let service = fixture_search_service();
    let detail = service
        .gallery_detail_get(4_051_038)
        .expect("load fixture gallery detail");
    assert_eq!(detail.summary.id.get(), 4_051_038);
    assert_eq!(detail.summary.artist, "serein");
    assert_eq!(detail.summary.group.as_deref(), Some("nocturne circle"));
    assert_eq!(detail.summary.series, vec!["rain archives"]);
    assert_eq!(detail.summary.characters, vec!["mira lane", "ren kujo"]);
    assert!(detail.summary.tags.contains(&"female:glasses".into()));
    assert_eq!(detail.related.len(), 2);

    let mut serialized =
        serde_json::to_value(ApiResult::success(detail)).expect("serialize gallery detail");
    let dimensions = serialized["data"]["pageDimensions"]
        .as_array()
        .expect("page dimensions");
    assert_eq!(dimensions.len(), 64);
    assert_eq!(
        dimensions.first(),
        Some(&json!({ "sourcePage": 1, "width": 512, "height": 512 }))
    );
    assert_eq!(
        dimensions.last(),
        Some(&json!({ "sourcePage": 64, "width": 512, "height": 512 }))
    );
    serialized["data"]
        .as_object_mut()
        .expect("detail data")
        .remove("pageDimensions");
    assert_eq!(
        serialized,
        json!({
            "ok": true,
            "data": {
                "id": 4051038,
                "title": "Archive of Rain",
                "artist": "serein",
                "group": "nocturne circle",
                "series": ["rain archives"],
                "characters": ["mira lane", "ren kujo"],
                "pages": 64,
                "language": "korean",
                "tags": ["female:glasses", "female:long_hair", "full_color", "mystery"],
                "publishedRank": 20260812,
                "popularity": 98,
                "thumbnailKey": "fixture-gallery-4051038-cover",
                "thumbnailWidth": 512,
                "thumbnailHeight": 512,
                "related": [
                    {
                        "id": 4050754,
                        "title": "The Last Tram",
                        "artist": "serein",
                        "group": "nocturne circle",
                        "series": ["rain archives"],
                        "characters": ["mira lane"],
                        "pages": 76,
                        "language": "korean",
                        "tags": ["drama", "female:coat", "male:suit", "night", "rain"],
                        "publishedRank": 20260809,
                        "popularity": 83,
                        "thumbnailKey": "fixture-gallery-4050754-cover",
                        "thumbnailWidth": 512,
                        "thumbnailHeight": 512
                    },
                    {
                        "id": 4050974,
                        "title": "The Green Window",
                        "artist": "paperlane",
                        "group": "paper studio",
                        "series": ["paper city"],
                        "characters": ["hana ito"],
                        "pages": 42,
                        "language": "english",
                        "tags": ["female:schoolgirl_uniform", "female:short_hair", "library", "mystery"],
                        "publishedRank": 20260811,
                        "popularity": 71,
                        "thumbnailKey": "fixture-gallery-4050974-cover",
                        "thumbnailWidth": 512,
                        "thumbnailHeight": 512
                    }
                ]
            }
        })
    );
}

#[test]
fn fixture_search_reports_validation_and_not_found_errors() {
    let service = fixture_search_service();
    let invalid = service
        .search_submit(SearchRequest {
            text: String::new(),
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            languages: vec![Language::Korean],
            sort: SearchSort::Recent,
            page_size: 0,
        })
        .expect_err("zero page size must fail");
    assert!(matches!(invalid, ApplicationError::Validation(_)));

    let missing_gallery = ApiError::from(
        service
            .gallery_detail_get(9_999_999)
            .expect_err("unknown gallery must fail"),
    );
    assert_eq!(missing_gallery.code, "SOURCE_NOT_FOUND");

    let missing_query = ApiError::from(
        service
            .search_page_get("fixture-missing".into(), 1)
            .expect_err("unknown query must fail"),
    );
    assert_eq!(missing_query.code, "QUERY_NOT_FOUND");

    let submission = service
        .search_submit(SearchRequest {
            text: String::new(),
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            languages: Vec::new(),
            sort: SearchSort::Recent,
            page_size: 200,
        })
        .expect("submit one-page fixture search");
    assert_eq!(submission.first_page.total_pages, 1);

    let out_of_range = service
        .search_page_get(submission.query_id, 2)
        .expect_err("page beyond the search result must fail");
    assert!(matches!(
        out_of_range,
        ApplicationError::Validation(ref error) if error.field == "page"
    ));
}

#[test]
fn search_submit_command_payload_deserializes_request_wrapper() {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct SearchSubmitPayload {
        request: SearchRequest,
    }

    let payload: SearchSubmitPayload = serde_json::from_value(json!({
        "request": {
            "text": "artist:serein",
            "includeTags": ["female:glasses"],
            "excludeTags": [],
            "languages": ["korean"],
            "sort": "popular_week",
            "pageSize": 40
        }
    }))
    .expect("deserialize the Tauri search_submit argument envelope");

    assert_eq!(
        payload.request,
        SearchRequest {
            text: "artist:serein".into(),
            include_tags: vec!["female:glasses".into()],
            exclude_tags: Vec::new(),
            languages: vec![Language::Korean],
            sort: SearchSort::PopularWeek,
            page_size: 40,
        }
    );
}

fn fixture_search_service() -> ApplicationService {
    let repository = Arc::new(SqliteRepository::open_in_memory().expect("create state repository"));
    let search_repository =
        Arc::new(FixtureSearchRepository::new().expect("create fixture search repository"));
    ApplicationService::new(repository).with_search_repository(search_repository)
}
