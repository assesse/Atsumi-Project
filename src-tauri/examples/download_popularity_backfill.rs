//! Explicit one-time backfill. Updates only derived popularity caches and the schema.
use atsumi_lib::{
    download_popularity::{snapshot, Period},
    infrastructure::{HitomiLiveAdapter, HitomiLiveConfig, SqliteRepository},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("explicit database path required")?,
    );
    if !path.is_file() {
        return Err("database does not exist".into());
    }
    let c =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    c.execute_batch("PRAGMA query_only=ON;")?;
    let active:i64=c.query_row("SELECT COUNT(*) FROM download_jobs WHERE state IN ('queued','resolving_metadata','downloading','hashing','verifying','retry_wait')",[],|r|r.get(0))?;
    if active != 0 {
        return Err("downloads are still active; backfill deferred".into());
    }
    drop(c);
    // Repository creates a non-overwriting pre-migration backup before migration.
    let _repository = SqliteRepository::open(&path)?;
    let source = HitomiLiveAdapter::new(HitomiLiveConfig::default())?;
    for period in [Period::Today, Period::Week, Period::Month, Period::Year] {
        let result = snapshot(&path, &source, period)?;
        println!(
            "{}: stored={} ranked={} unranked={} fetched_at={} warning={:?}",
            period.key(),
            result.ranks.len(),
            result.ranks.values().filter(|r| r.is_some()).count(),
            result.ranks.values().filter(|r| r.is_none()).count(),
            result.fetched_at,
            result.warning
        );
    }
    Ok(())
}
