use rusqlite::{params, Row, TransactionBehavior};

use crate::application::{DownloadTuningProfile, DownloadTuningStore, RepositoryError};

use super::SqliteRepository;

const MAX_PROFILES: usize = 128;

fn db_error(error: rusqlite::Error) -> RepositoryError {
    RepositoryError::Other(error.to_string())
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.bytes().any(|byte| byte.is_ascii_lowercase())
        && host.split('.').all(|label| {
            let bytes = label.as_bytes();
            !bytes.is_empty()
                && bytes.len() <= 63
                && bytes[0].is_ascii_alphanumeric()
                && bytes[bytes.len() - 1].is_ascii_alphanumeric()
                && bytes
                    .iter()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
        })
}

fn valid_profile(profile: &DownloadTuningProfile) -> bool {
    valid_host(&profile.host)
        && (1..=8).contains(&profile.stable_limit)
        && profile.baseline_bytes_per_second.is_finite()
        && profile.baseline_bytes_per_second >= 0.0
        && profile.updated_at_unix_ms <= i64::MAX as u64
        && profile.cooldown_until_unix_ms <= i64::MAX as u64
        && profile.blocked_until_unix_ms <= i64::MAX as u64
}

fn stored_profile(row: &Row<'_>) -> Option<DownloadTuningProfile> {
    let profile = DownloadTuningProfile {
        host: row.get(0).ok()?,
        algorithm_version: u32::try_from(row.get::<_, i64>(1).ok()?).ok()?,
        stable_limit: usize::try_from(row.get::<_, i64>(2).ok()?).ok()?,
        baseline_bytes_per_second: row.get(3).ok()?,
        updated_at_unix_ms: u64::try_from(row.get::<_, i64>(4).ok()?).ok()?,
        cooldown_until_unix_ms: u64::try_from(row.get::<_, i64>(5).ok()?).ok()?,
        blocked_until_unix_ms: u64::try_from(row.get::<_, i64>(6).ok()?).ok()?,
    };
    valid_profile(&profile).then_some(profile)
}

impl DownloadTuningStore for SqliteRepository {
    fn load_all(&self) -> Result<Vec<DownloadTuningProfile>, RepositoryError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT host, algorithm_version, stable_limit, baseline_bytes_per_second,
                        updated_at_unix_ms, cooldown_until_unix_ms, blocked_until_unix_ms
                 FROM download_tuning_profiles
                 ORDER BY updated_at_unix_ms DESC, host ASC LIMIT ?1",
            )
            .map_err(db_error)?;
        let mut rows = statement.query([MAX_PROFILES as i64]).map_err(db_error)?;
        let mut profiles = Vec::new();
        while let Some(row) = rows.next().map_err(db_error)? {
            if let Some(profile) = stored_profile(row) {
                profiles.push(profile);
            }
        }
        Ok(profiles)
    }

    fn save(&self, profile: &DownloadTuningProfile) -> Result<(), RepositoryError> {
        if !valid_profile(profile) {
            return Err(RepositoryError::Other(
                "invalid adaptive download tuning profile".to_owned(),
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        transaction
            .execute(
                "INSERT INTO download_tuning_profiles (
                    host, algorithm_version, stable_limit, baseline_bytes_per_second,
                    updated_at_unix_ms, cooldown_until_unix_ms, blocked_until_unix_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(host) DO UPDATE SET
                    algorithm_version = excluded.algorithm_version,
                    stable_limit = excluded.stable_limit,
                    baseline_bytes_per_second = excluded.baseline_bytes_per_second,
                    updated_at_unix_ms = excluded.updated_at_unix_ms,
                    cooldown_until_unix_ms = excluded.cooldown_until_unix_ms,
                    blocked_until_unix_ms = excluded.blocked_until_unix_ms
                 WHERE excluded.updated_at_unix_ms >= download_tuning_profiles.updated_at_unix_ms",
                params![
                    profile.host,
                    i64::from(profile.algorithm_version),
                    profile.stable_limit as i64,
                    profile.baseline_bytes_per_second,
                    profile.updated_at_unix_ms as i64,
                    profile.cooldown_until_unix_ms as i64,
                    profile.blocked_until_unix_ms as i64,
                ],
            )
            .map_err(db_error)?;
        transaction
            .execute(
                "DELETE FROM download_tuning_profiles WHERE host NOT IN (
                    SELECT host FROM download_tuning_profiles
                    ORDER BY updated_at_unix_ms DESC, host ASC LIMIT ?1
                 )",
                [MAX_PROFILES as i64],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(host: &str, updated_at_unix_ms: u64) -> DownloadTuningProfile {
        DownloadTuningProfile {
            host: host.to_owned(),
            algorithm_version: 1,
            stable_limit: 4,
            baseline_bytes_per_second: 12_345_678.5,
            updated_at_unix_ms,
            cooldown_until_unix_ms: updated_at_unix_ms + 2_000,
            blocked_until_unix_ms: updated_at_unix_ms + 5_000,
        }
    }

    #[test]
    fn download_tuning_profiles_round_trip_in_memory() {
        let repository = SqliteRepository::open_in_memory().unwrap();
        assert!(repository.load_all().unwrap().is_empty());
        let mut expected = profile("cdn.example.org", 1_800_000_000_000);
        repository.save(&expected).unwrap();
        assert_eq!(repository.load_all().unwrap(), vec![expected.clone()]);

        expected.algorithm_version = u32::MAX;
        expected.stable_limit = 8;
        expected.baseline_bytes_per_second = 0.0;
        expected.updated_at_unix_ms = i64::MAX as u64;
        expected.cooldown_until_unix_ms = i64::MAX as u64;
        expected.blocked_until_unix_ms = i64::MAX as u64;
        repository.save(&expected).unwrap();
        assert_eq!(repository.load_all().unwrap(), vec![expected]);
    }

    #[test]
    fn download_tuning_profiles_survive_database_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("download-tuning.sqlite3");
        let expected = profile("images.cdn.example", 1_800_000_000_000);
        {
            let repository = SqliteRepository::open(&path).unwrap();
            repository.save(&expected).unwrap();
        }
        let reopened = SqliteRepository::open(&path).unwrap();
        assert_eq!(reopened.load_all().unwrap(), vec![expected]);
    }

    #[test]
    fn download_tuning_rejects_older_saves_from_another_connection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("download-tuning-stale.sqlite3");
        let first = SqliteRepository::open(&path).unwrap();
        let second = SqliteRepository::open(&path).unwrap();
        let older = profile("cdn.example.org", 1_800_000_000_000);
        first.save(&older).unwrap();
        let newer = DownloadTuningProfile {
            stable_limit: 2,
            baseline_bytes_per_second: 3_000_000.0,
            updated_at_unix_ms: older.updated_at_unix_ms + 10,
            blocked_until_unix_ms: older.blocked_until_unix_ms + 60_000,
            ..older.clone()
        };
        second.save(&newer).unwrap();
        first.save(&older).unwrap();
        assert_eq!(first.load_all().unwrap(), vec![newer.clone()]);

        let same_timestamp = DownloadTuningProfile {
            stable_limit: 3,
            ..newer
        };
        first.save(&same_timestamp).unwrap();
        assert_eq!(second.load_all().unwrap(), vec![same_timestamp]);
    }

    #[test]
    fn download_tuning_saves_only_lowercase_domain_hosts() {
        let repository = SqliteRepository::open_in_memory().unwrap();
        let overlong_label = format!("{}.example", "a".repeat(64));
        let overlong_host = format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(62),
        );
        for host in [
            "",
            "CDN.example.org",
            "https://cdn.example.org/image?gallery=123",
            "cdn.example.org/path",
            "cdn.example.org:443",
            "user@cdn.example.org",
            "cdn.example.org.",
            ".cdn.example.org",
            "cdn..example.org",
            "-cdn.example.org",
            "cdn-.example.org",
            "cdn_name.example.org",
            "cdn.example.org\n",
            "이미지.example.org",
            "127.0.0.1",
            "[::1]",
            "4051038",
            overlong_label.as_str(),
            overlong_host.as_str(),
        ] {
            assert!(repository.save(&profile(host, 1_000)).is_err());
        }
        assert!(repository.load_all().unwrap().is_empty());

        let longest_host = format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61),
        );
        assert_eq!(longest_host.len(), 253);
        for host in ["cdn-01.example.org", "xn--bcher-kva.example", &longest_host] {
            repository.save(&profile(host, 1_000)).unwrap();
        }
        assert_eq!(repository.load_all().unwrap().len(), 3);
    }

    #[test]
    fn download_tuning_rejects_invalid_rates_limits_and_timestamps() {
        let repository = SqliteRepository::open_in_memory().unwrap();
        let base = profile("cdn.example.org", 1_000);
        for rate in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert!(repository
                .save(&DownloadTuningProfile {
                    baseline_bytes_per_second: rate,
                    ..base.clone()
                })
                .is_err());
        }
        for stable_limit in [0, 9, usize::MAX] {
            assert!(repository
                .save(&DownloadTuningProfile {
                    stable_limit,
                    ..base.clone()
                })
                .is_err());
        }
        for invalid in [
            DownloadTuningProfile {
                updated_at_unix_ms: i64::MAX as u64 + 1,
                ..base.clone()
            },
            DownloadTuningProfile {
                cooldown_until_unix_ms: i64::MAX as u64 + 1,
                ..base.clone()
            },
            DownloadTuningProfile {
                blocked_until_unix_ms: i64::MAX as u64 + 1,
                ..base
            },
        ] {
            assert!(repository.save(&invalid).is_err());
        }
        assert!(repository.load_all().unwrap().is_empty());
    }

    #[test]
    fn download_tuning_skips_corrupt_rows_after_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("download-tuning-corrupt.sqlite3");
        let expected = profile("healthy.example.org", 1_800_000_000_000);
        {
            let repository = SqliteRepository::open(&path).unwrap();
            repository.save(&expected).unwrap();
            for (index, assignment) in [
                "algorithm_version = -1",
                "algorithm_version = 4294967296",
                "stable_limit = 0",
                "stable_limit = 9",
                "baseline_bytes_per_second = -1",
                "baseline_bytes_per_second = 1e999",
                "baseline_bytes_per_second = 'not-a-rate'",
                "updated_at_unix_ms = -1",
                "cooldown_until_unix_ms = -1",
                "blocked_until_unix_ms = -1",
                "updated_at_unix_ms = 'not-a-timestamp'",
                "host = 'UPPER.example.org'",
                "host = 'https://cdn.example.org/private-path'",
                "host = '127.0.0.1'",
            ]
            .into_iter()
            .enumerate()
            {
                let host = format!("corrupt-{index}.example.org");
                repository
                    .save(&profile(&host, 2_000 + index as u64))
                    .unwrap();
                repository
                    .connection()
                    .unwrap()
                    .execute(
                        &format!(
                            "UPDATE download_tuning_profiles SET {assignment} WHERE host = ?1"
                        ),
                        [host],
                    )
                    .unwrap();
            }
            assert_eq!(repository.load_all().unwrap(), vec![expected.clone()]);
        }
        let reopened = SqliteRepository::open(&path).unwrap();
        assert_eq!(reopened.load_all().unwrap(), vec![expected]);
    }

    #[test]
    fn download_tuning_saves_prune_to_the_128_most_recent_profiles() {
        let repository = SqliteRepository::open_in_memory().unwrap();
        for index in 0..130 {
            repository
                .save(&profile(&format!("cdn-{index}.example.org"), 1_000 + index))
                .unwrap();
        }
        let profiles = repository.load_all().unwrap();
        assert_eq!(profiles.len(), MAX_PROFILES);
        assert_eq!(profiles[0].host, "cdn-129.example.org");
        assert_eq!(profiles[MAX_PROFILES - 1].host, "cdn-2.example.org");
        repository
            .save(&profile("very-old.example.org", 1))
            .unwrap();
        assert_eq!(repository.load_all().unwrap(), profiles);
        let count: i64 = repository
            .connection()
            .unwrap()
            .query_row("SELECT count(*) FROM download_tuning_profiles", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, MAX_PROFILES as i64);
    }

    #[test]
    fn download_tuning_load_remains_bounded_for_an_oversized_existing_table() {
        let repository = SqliteRepository::open_in_memory().unwrap();
        {
            let connection = repository.connection().unwrap();
            for index in 0..130 {
                connection
                    .execute(
                        "INSERT INTO download_tuning_profiles VALUES (?1, 1, 4, 1000.0, ?2, 0, 0)",
                        params![format!("cdn-{index}.example.org"), 1_000 + index],
                    )
                    .unwrap();
            }
        }
        let profiles = repository.load_all().unwrap();
        assert_eq!(profiles.len(), MAX_PROFILES);
        assert_eq!(profiles[0].host, "cdn-129.example.org");
        assert_eq!(profiles[MAX_PROFILES - 1].host, "cdn-2.example.org");
    }
}
