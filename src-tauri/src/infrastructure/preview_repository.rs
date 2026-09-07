use std::collections::BTreeSet;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::{
    application::{
        ApplicationError, ArtifactRepository, ArtistPreview, GalleryPreview,
        GalleryPreviewRepository, GalleryPreviewSetRequest, PreviewCandidate, RepositoryError,
        GALLERY_PREVIEW_ALGORITHM_VERSION,
    },
    domain::{ArtifactBundle, DownloadEntryId, GalleryId, ValidationError},
};

use super::SqliteRepository;

type StoredGalleryPreview = (String, Option<u32>, Option<String>, String, u32, String);

fn db_error(error: rusqlite::Error) -> RepositoryError {
    RepositoryError::Other(error.to_string())
}
fn json_error(error: serde_json::Error) -> RepositoryError {
    RepositoryError::Corrupt(format!("invalid persisted gallery preview: {error}"))
}

fn active_entry(
    connection: &Connection,
    gallery_id: GalleryId,
) -> Result<Option<String>, RepositoryError> {
    connection.query_row(
        "SELECT artifact.entry_id FROM download_artifacts artifact JOIN download_entries entry ON entry.entry_id = artifact.entry_id
         WHERE artifact.gallery_id = ?1 AND artifact.state = 'complete' AND entry.state = 'completed'
           AND NOT EXISTS (SELECT 1 FROM duplicate_hidden_galleries hidden WHERE hidden.gallery_id = artifact.gallery_id)
         ORDER BY entry.created_at DESC, entry.entry_id DESC LIMIT 1",
        [gallery_id.get()], |row| row.get(0)).optional().map_err(db_error)
}

fn valid_pages(connection: &Connection, entry_id: &str) -> Result<Vec<u32>, RepositoryError> {
    let mut query = connection.prepare(
        "SELECT source_page_number FROM download_pages WHERE entry_id = ?1 AND excluded = 0 AND state = 'present'
         AND verified_at IS NOT NULL AND sha256 IS NOT NULL AND byte_length > 0
         AND storage_format IS NOT NULL AND source_revision IS NOT NULL ORDER BY source_page_number"
    ).map_err(db_error)?;
    let pages = query
        .query_map([entry_id], |row| row.get(0))
        .map_err(db_error)?;
    pages.collect::<Result<Vec<u32>, _>>().map_err(db_error)
}

fn read_preview(
    connection: &Connection,
    gallery_id: GalleryId,
) -> Result<Option<GalleryPreview>, RepositoryError> {
    let stored: Option<StoredGalleryPreview> = connection.query_row(
        "SELECT mode, manual_source_page, entry_id, candidates_json, algorithm_version, updated_at FROM gallery_previews WHERE gallery_id = ?1",
        [gallery_id.get()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
    ).optional().map_err(db_error)?;
    let Some((mode, manual_source_page, stored_entry_id, json, algorithm_version, updated_at)) =
        stored
    else {
        return Ok(None);
    };
    let candidates: Vec<PreviewCandidate> = serde_json::from_str(&json).map_err(json_error)?;
    let entry_id = active_entry(connection, gallery_id)?;
    let valid = entry_id
        .as_deref()
        .map(|id| valid_pages(connection, id))
        .transpose()?
        .unwrap_or_default();
    let applicable = candidates
        .iter()
        .filter(|candidate| stored_entry_id == entry_id && valid.contains(&candidate.source_page))
        .collect::<Vec<_>>();
    let source_page = manual_source_page
        .filter(|page| valid.contains(page))
        .or_else(|| applicable.first().map(|candidate| candidate.source_page))
        .or_else(|| valid.first().copied());
    let dimensions = source_page
        .and_then(|page| {
            applicable
                .iter()
                .find(|candidate| candidate.source_page == page)
        })
        .map(|candidate| (candidate.width, candidate.height));
    let dimensions = match (dimensions, entry_id.as_deref(), source_page) {
        (Some((Some(width), Some(height))), _, _) => Some((Some(width), Some(height))),
        (_, Some(id), Some(page)) => connection.query_row(
            "SELECT hash.width, hash.height FROM duplicate_page_hashes hash JOIN download_pages page ON page.entry_id = hash.entry_id AND page.source_page_number = hash.source_page_number AND page.sha256 = hash.artifact_sha256
             WHERE hash.entry_id = ?1 AND hash.source_page_number = ?2 ORDER BY hash.profile_version DESC LIMIT 1",
            params![id, page], |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(db_error)?,
        _ => None,
    };
    Ok(Some(GalleryPreview {
        gallery_id,
        mode,
        source_page,
        manual_source_page,
        entry_id,
        width: dimensions.and_then(|value| value.0),
        height: dimensions.and_then(|value| value.1),
        candidates: applicable
            .into_iter()
            .map(|candidate| candidate.source_page)
            .collect(),
        algorithm_version,
        updated_at,
    }))
}

fn read_artist(
    connection: &Connection,
    artist: &str,
) -> Result<Option<ArtistPreview>, RepositoryError> {
    let stored: Option<(String, String, String)> = connection.query_row(
        "SELECT artist, gallery_ids_json, updated_at FROM artist_previews WHERE artist = ?1 COLLATE NOCASE",
        [artist], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional().map_err(db_error)?;
    stored
        .map(|(artist, json, updated_at)| {
            let ids: Vec<i64> = serde_json::from_str(&json).map_err(json_error)?;
            let gallery_ids = ids
                .into_iter()
                .map(|id| {
                    GalleryId::new(id).map_err(|error| RepositoryError::Corrupt(error.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ArtistPreview {
                artist,
                gallery_ids,
                updated_at,
            })
        })
        .transpose()
}

impl GalleryPreviewRepository for SqliteRepository {
    fn gallery_preview_list(
        &self,
        gallery_ids: &[GalleryId],
    ) -> Result<Vec<GalleryPreview>, RepositoryError> {
        let connection = self.connection()?;
        let mut result = Vec::new();
        for gallery_id in gallery_ids.iter().copied().collect::<BTreeSet<_>>() {
            if let Some(preview) = read_preview(&connection, gallery_id)? {
                result.push(preview);
            }
        }
        Ok(result)
    }

    fn gallery_preview_set(
        &self,
        request: &GalleryPreviewSetRequest,
    ) -> Result<GalleryPreview, ApplicationError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let entry_id = active_entry(&transaction, request.gallery_id)?
            .ok_or(ApplicationError::GalleryNotFound(request.gallery_id))?;
        let pages = valid_pages(&transaction, &entry_id)?;
        if request
            .source_page
            .is_some_and(|page| !pages.contains(&page))
        {
            return Err(ValidationError::new(
                "sourcePage",
                "must identify a verified, available local page",
            )
            .into());
        }
        let fallback = pages
            .into_iter()
            .take(12)
            .map(|source_page| PreviewCandidate {
                source_page,
                width: None,
                height: None,
            })
            .collect::<Vec<_>>();
        transaction.execute(
            "INSERT INTO gallery_previews (gallery_id, mode, manual_source_page, entry_id, candidates_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             ON CONFLICT(gallery_id) DO UPDATE SET mode = excluded.mode, manual_source_page = excluded.manual_source_page, updated_at = excluded.updated_at",
            params![request.gallery_id.get(), if request.source_page.is_some() { "manual" } else { "automatic" }, request.source_page, entry_id, serde_json::to_string(&fallback).map_err(json_error)?]
        ).map_err(db_error)?;
        let preview = read_preview(&transaction, request.gallery_id)?
            .ok_or(ApplicationError::GalleryNotFound(request.gallery_id))?;
        transaction.commit().map_err(db_error)?;
        Ok(preview)
    }

    fn gallery_preview_pending(&self) -> Result<Vec<GalleryId>, RepositoryError> {
        let connection = self.connection()?;
        let mut query = connection.prepare(
            "SELECT DISTINCT artifact.gallery_id FROM download_artifacts artifact JOIN download_entries entry ON entry.entry_id = artifact.entry_id
             LEFT JOIN gallery_previews preview ON preview.gallery_id = artifact.gallery_id
             WHERE artifact.state = 'complete' AND entry.state = 'completed' AND preview.analysis_attempted_at IS NULL
               AND NOT EXISTS (SELECT 1 FROM duplicate_hidden_galleries hidden WHERE hidden.gallery_id = artifact.gallery_id)
             ORDER BY entry.created_at DESC, artifact.gallery_id DESC"
        ).map_err(db_error)?;
        let rows = query
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(db_error)?;
        rows.map(|value| {
            GalleryId::new(value.map_err(db_error)?)
                .map_err(|error| RepositoryError::Corrupt(error.to_string()))
        })
        .collect()
    }

    fn gallery_preview_bundle(
        &self,
        gallery_id: GalleryId,
    ) -> Result<Option<ArtifactBundle>, RepositoryError> {
        let entry_id = {
            let connection = self.connection()?;
            active_entry(&connection, gallery_id)?
        };
        match entry_id {
            Some(id) => self.artifact_bundle_get(
                &DownloadEntryId::new(id)
                    .map_err(|error| RepositoryError::Corrupt(error.to_string()))?,
            ),
            None => Ok(None),
        }
    }

    fn gallery_preview_store_automatic(
        &self,
        gallery_id: GalleryId,
        entry_id: &str,
        candidates: &[PreviewCandidate],
        analyzed: bool,
    ) -> Result<(), RepositoryError> {
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO gallery_previews (gallery_id, entry_id, candidates_json, algorithm_version, analysis_attempted_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             ON CONFLICT(gallery_id) DO UPDATE SET entry_id = excluded.entry_id,
               candidates_json = CASE WHEN ?5 OR gallery_previews.entry_id IS NOT excluded.entry_id OR gallery_previews.algorithm_version = 0 THEN excluded.candidates_json ELSE gallery_previews.candidates_json END,
               algorithm_version = CASE WHEN ?5 THEN excluded.algorithm_version ELSE gallery_previews.algorithm_version END,
               analysis_attempted_at = excluded.analysis_attempted_at, updated_at = excluded.updated_at",
            params![gallery_id.get(), entry_id, serde_json::to_string(candidates).map_err(json_error)?, if analyzed { GALLERY_PREVIEW_ALGORITHM_VERSION } else { 0 }, analyzed]
        ).map_err(db_error)?;
        Ok(())
    }

    fn gallery_preview_root(
        &self,
        entry_id: &str,
    ) -> Result<Option<std::path::PathBuf>, RepositoryError> {
        let connection = self.connection()?;
        let stored: String = connection
            .query_row(
                "SELECT root_snapshot FROM download_artifacts WHERE entry_id = ?1",
                [entry_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        Ok((!stored.trim().is_empty()).then(|| std::path::PathBuf::from(stored)))
    }

    fn artist_preview_list(
        &self,
        artists: &[String],
    ) -> Result<Vec<ArtistPreview>, RepositoryError> {
        let connection = self.connection()?;
        artists
            .iter()
            .map(|artist| read_artist(&connection, artist))
            .collect::<Result<Vec<_>, _>>()
            .map(|rows| rows.into_iter().flatten().collect())
    }

    fn artist_preview_refresh(
        &self,
        gallery_id: GalleryId,
    ) -> Result<Vec<ArtistPreview>, RepositoryError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let artists = {
            let mut query = transaction
                .prepare("SELECT artist FROM owned_gallery_artists WHERE gallery_id = ?1")
                .map_err(db_error)?;
            let rows = query
                .query_map([gallery_id.get()], |row| row.get::<_, String>(0))
                .map_err(db_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?
        };
        let mut result = Vec::new();
        for artist in artists {
            let ids = {
                let mut query = transaction.prepare(
                    "SELECT entry.gallery_id FROM owned_gallery_artists owned
                     JOIN download_entries entry ON entry.gallery_id = owned.gallery_id JOIN download_artifacts artifact ON artifact.entry_id = entry.entry_id
                     WHERE owned.artist = ?1 COLLATE NOCASE AND entry.state = 'completed' AND artifact.state = 'complete'
                       AND NOT EXISTS (SELECT 1 FROM duplicate_hidden_galleries hidden WHERE hidden.gallery_id = entry.gallery_id)
                     GROUP BY entry.gallery_id ORDER BY max(entry.created_at) DESC, entry.gallery_id DESC LIMIT 5"
                ).map_err(db_error)?;
                let rows = query
                    .query_map([&artist], |row| row.get::<_, i64>(0))
                    .map_err(db_error)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?
            };
            transaction.execute("INSERT INTO artist_previews (artist, gallery_ids_json, updated_at) VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) ON CONFLICT(artist) DO UPDATE SET gallery_ids_json = excluded.gallery_ids_json, updated_at = excluded.updated_at",
                params![artist, serde_json::to_string(&ids).map_err(json_error)?]).map_err(db_error)?;
            if let Some(preview) = read_artist(&transaction, &artist)? {
                result.push(preview);
            }
        }
        transaction.commit().map_err(db_error)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(repository: &SqliteRepository, id: i64) {
        let connection = repository.connection().unwrap();
        let entry = format!("entry-{id}");
        connection.execute("INSERT INTO galleries (gallery_id, revision, title, primary_artist, source_page_count) VALUES (?1, 0, 'Gallery', 'Artist', 3)", [id]).unwrap();
        connection
            .execute(
                "INSERT INTO owned_gallery_artists (gallery_id, artist) VALUES (?1, 'Artist')",
                [id],
            )
            .unwrap();
        connection.execute("INSERT INTO download_entries (entry_id, gallery_id, revision, state, progress, created_at, updated_at) VALUES (?1, ?2, 0, 'completed', 100, '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')", params![entry, id]).unwrap();
        connection.execute("INSERT INTO download_artifacts (entry_id, gallery_id, revision, relative_directory, expected_page_count, state, manifest_relative_path, manifest_schema_version, writer_version, hash_profile_version, completed_at) VALUES (?1, ?2, 1, ?1, 3, 'complete', ?3, 1, 'preview-test', 1, '2026-09-01T00:00:00Z')", params![entry, id, format!("{entry}/manifest.json")]).unwrap();
        for page in 1..=3 {
            connection.execute("INSERT INTO download_pages (entry_id, gallery_id, source_page_number, relative_path, state, byte_length, sha256, storage_format, source_revision, verified_at, excluded) VALUES (?1, ?2, ?3, ?4, 'present', 128, ?5, 'webp', 'v1', '2026-09-01T00:00:00Z', 0)", params![entry, id, page, format!("{entry}/{page}.webp"), "a".repeat(64)]).unwrap();
        }
    }

    fn candidates() -> Vec<PreviewCandidate> {
        [3, 2, 1]
            .into_iter()
            .map(|source_page| PreviewCandidate {
                source_page,
                width: Some(800),
                height: Some(1200),
            })
            .collect()
    }

    #[test]
    fn gallery_preview_manual_choice_survives_refresh_restart_and_page_exclusion() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preview.sqlite3");
        let id = GalleryId::new(1).unwrap();
        {
            let repository = SqliteRepository::open(&path).unwrap();
            seed(&repository, 1);
            repository
                .gallery_preview_store_automatic(id, "entry-1", &candidates(), true)
                .unwrap();
            assert_eq!(
                repository.gallery_preview_list(&[id]).unwrap()[0].source_page,
                Some(3)
            );
            repository
                .gallery_preview_set(&GalleryPreviewSetRequest {
                    gallery_id: id,
                    source_page: Some(2),
                })
                .unwrap();
            repository
                .gallery_preview_store_automatic(id, "entry-1", &candidates(), true)
                .unwrap();
        }
        let repository = SqliteRepository::open(&path).unwrap();
        let chosen = &repository.gallery_preview_list(&[id]).unwrap()[0];
        assert_eq!(
            (chosen.mode.as_str(), chosen.source_page),
            ("manual", Some(2))
        );
        repository.connection().unwrap().execute("UPDATE download_pages SET excluded = 1 WHERE entry_id = 'entry-1' AND source_page_number = 2", []).unwrap();
        let fallback = &repository.gallery_preview_list(&[id]).unwrap()[0];
        assert_eq!(
            (fallback.manual_source_page, fallback.source_page),
            (Some(2), Some(3))
        );
        assert_eq!(fallback.candidates, vec![3, 1]);
        let automatic = repository
            .gallery_preview_set(&GalleryPreviewSetRequest {
                gallery_id: id,
                source_page: None,
            })
            .unwrap();
        assert_eq!(
            (
                automatic.mode.as_str(),
                automatic.manual_source_page,
                automatic.source_page
            ),
            ("automatic", None, Some(3))
        );
    }

    #[test]
    fn gallery_preview_backfill_attempt_is_durable_and_invalid_manual_page_is_rejected() {
        let repository = SqliteRepository::open_in_memory().unwrap();
        seed(&repository, 1);
        let id = GalleryId::new(1).unwrap();
        assert_eq!(repository.gallery_preview_pending().unwrap(), vec![id]);
        assert!(repository
            .gallery_preview_set(&GalleryPreviewSetRequest {
                gallery_id: id,
                source_page: Some(4)
            })
            .is_err());
        assert!(repository.gallery_preview_list(&[id]).unwrap().is_empty());
        repository
            .gallery_preview_store_automatic(id, "entry-1", &candidates(), false)
            .unwrap();
        assert!(repository.gallery_preview_pending().unwrap().is_empty());
        assert_eq!(
            repository.gallery_preview_list(&[id]).unwrap()[0].algorithm_version,
            0
        );
    }

    #[test]
    fn artist_preview_persists_latest_five_and_refreshes_after_exclusion() {
        let repository = SqliteRepository::open_in_memory().unwrap();
        for id in 1..=7 {
            seed(&repository, id);
        }
        assert!(repository
            .artist_preview_list(&["Artist".into()])
            .unwrap()
            .is_empty());
        let preview = repository
            .artist_preview_refresh(GalleryId::new(7).unwrap())
            .unwrap();
        assert_eq!(
            preview[0]
                .gallery_ids
                .iter()
                .map(|id| id.get())
                .collect::<Vec<_>>(),
            vec![7, 6, 5, 4, 3]
        );
        repository.connection().unwrap().execute("INSERT INTO duplicate_hidden_galleries (gallery_id, decision_id, created_at) VALUES (7, 'test-hidden', 'now')", []).unwrap();
        repository
            .artist_preview_refresh(GalleryId::new(7).unwrap())
            .unwrap();
        let stored = repository.artist_preview_list(&["artist".into()]).unwrap();
        assert_eq!(
            stored[0]
                .gallery_ids
                .iter()
                .map(|id| id.get())
                .collect::<Vec<_>>(),
            vec![6, 5, 4, 3, 2]
        );
    }
}
