use rusqlite::{params, OptionalExtension};

use crate::{
    application::{GallerySummaryCache, RepositoryError},
    domain::{GalleryId, GallerySummary},
};

use super::SqliteRepository;

const SUMMARY_PROFILE: &str = "hitomi-gallery-summary-v1";
const SUMMARY_SCHEMA_VERSION: u32 = 1;
const MAX_SUMMARY_BYTES: usize = 2 * 1024 * 1024;

pub(super) const GALLERY_SUMMARY_CACHE_SCHEMA: &str = r#"
    CREATE TABLE gallery_summary_cache (
        gallery_id INTEGER PRIMARY KEY CHECK (gallery_id > 0),
        profile TEXT NOT NULL,
        schema_version INTEGER NOT NULL CHECK (schema_version > 0),
        summary_json TEXT NOT NULL,
        updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    ) STRICT;
"#;

fn db_error(error: rusqlite::Error) -> RepositoryError {
    RepositoryError::Other(error.to_string())
}

fn valid_summary(summary: &GallerySummary, gallery_id: GalleryId) -> bool {
    summary.id == gallery_id && !summary.title.trim().is_empty() && summary.pages > 0
}

pub(super) fn decode_persisted_summary(
    gallery_id: GalleryId,
    profile: &str,
    version: u32,
    json: &str,
) -> Option<GallerySummary> {
    (profile == SUMMARY_PROFILE
        && version == SUMMARY_SCHEMA_VERSION
        && json.len() <= MAX_SUMMARY_BYTES)
        .then(|| serde_json::from_str::<GallerySummary>(json).ok())
        .flatten()
        .filter(|summary| valid_summary(summary, gallery_id))
}

impl GallerySummaryCache for SqliteRepository {
    fn gallery_summary_cache_get(
        &self,
        gallery_id: GalleryId,
    ) -> Result<Option<GallerySummary>, RepositoryError> {
        let connection = self.connection()?;
        let stored: Option<(String, u32, String)> = connection
            .query_row(
                "SELECT profile, schema_version, summary_json FROM gallery_summary_cache WHERE gallery_id = ?1",
                [gallery_id.get()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(db_error)?;
        let Some((profile, version, json)) = stored else {
            return Ok(None);
        };
        // No wall-clock expiry: once fetched, tags survive app restarts until
        // new main-gallery metadata replaces them or the user clears caches.
        let summary = decode_persisted_summary(gallery_id, &profile, version, &json);
        if summary.is_none() {
            tracing::debug!(
                gallery_id = gallery_id.get(),
                "ignoring invalid persisted gallery summary"
            );
            connection
                .execute(
                    "DELETE FROM gallery_summary_cache WHERE gallery_id = ?1",
                    [gallery_id.get()],
                )
                .map_err(db_error)?;
        }
        Ok(summary)
    }

    fn gallery_summary_cache_put(&self, summary: &GallerySummary) -> Result<(), RepositoryError> {
        if !valid_summary(summary, summary.id) {
            return Err(RepositoryError::Corrupt(
                "invalid gallery summary cache payload".into(),
            ));
        }
        let json = serde_json::to_string(summary)
            .map_err(|error| RepositoryError::Other(error.to_string()))?;
        if json.len() > MAX_SUMMARY_BYTES {
            return Err(RepositoryError::Other(
                "gallery summary cache payload is too large".into(),
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(db_error)?;
        transaction
            .execute(
                "INSERT INTO gallery_summary_cache (gallery_id, profile, schema_version, summary_json)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(gallery_id) DO UPDATE SET profile = excluded.profile,
                     schema_version = excluded.schema_version, summary_json = excluded.summary_json,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![summary.id.get(), SUMMARY_PROFILE, SUMMARY_SCHEMA_VERSION, json],
            )
            .map_err(db_error)?;
        if !summary.artists.is_empty() {
            let artists = serde_json::to_string(&summary.artists)
                .map_err(|error| RepositoryError::Other(error.to_string()))?;
            // A normal metadata fetch gradually enriches legacy Auto Find
            // cards, without re-scanning every candidate or changing their
            // original first artist, discovery favorite, or run counters.
            transaction
                .execute(
                    "UPDATE auto_find_candidates SET artists_json = ?1
                     WHERE gallery_id = ?2 AND artists_json != ?1",
                    params![artists, summary.id.get()],
                )
                .map_err(db_error)?;
        }
        transaction.commit().map_err(db_error)?;
        Ok(())
    }

    fn gallery_summary_cache_clear(&self) -> Result<u64, RepositoryError> {
        self.connection()?
            .execute("DELETE FROM gallery_summary_cache", [])
            .map(|deleted| deleted as u64)
            .map_err(db_error)
    }
}
