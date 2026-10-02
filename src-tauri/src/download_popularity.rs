//! Period-specific source ranks. No gallery metadata, artifact or job state is changed.
use crate::infrastructure::HitomiLiveAdapter;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
    sync::Mutex,
    time::{Duration, SystemTime},
};

static REFRESH: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    Today,
    Week,
    Month,
    Year,
}
impl Period {
    pub fn key(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PopularitySnapshot {
    pub period: Period,
    pub fetched_at: String,
    pub ranks: BTreeMap<i64, Option<u32>>,
    pub warning: Option<String>,
}

pub fn snapshot(
    path: &Path,
    source: &HitomiLiveAdapter,
    period: Period,
) -> Result<PopularitySnapshot, String> {
    // Reject duplicate refreshes instead of building a queue of network/SQLite work.
    let _guard = REFRESH
        .try_lock()
        .map_err(|_| "인기순 동기화가 이미 진행 중입니다.".to_owned())?;
    let mut c = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    c.busy_timeout(Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    let saved: Option<String> = c
        .query_row(
            "SELECT fetched_at FROM hitomi_popularity_snapshots WHERE period=?1",
            [period.key()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let now = chrono::DateTime::<chrono::Utc>::from(SystemTime::now());
    let fresh = saved
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .is_some_and(|date| {
            let age = now.signed_duration_since(date).num_seconds();
            (0..86400).contains(&age)
        });
    let mut warning = None;
    if !fresh {
        // No transaction or application gate is held while waiting for the network.
        match source.popularity_ids(period.key()) {
            Ok(ids) if !ids.is_empty() => {
                let bytes: Vec<u8> = ids
                    .into_iter()
                    .flat_map(|id| (id as u32).to_be_bytes())
                    .collect();
                store_snapshot(&mut c, period.key(), &bytes, &now.to_rfc3339())
                    .map_err(|e| e.to_string())?;
            }
            result => {
                if saved.is_none() {
                    return Err(match result {
                        Err(e) => e.to_string(),
                        _ => "빈 인기순 응답은 저장하지 않습니다.".into(),
                    });
                }
                warning = Some("인기순 갱신 지연 · 저장된 순위 사용 중".into());
            }
        }
    }
    // Assign newcomers from the stored index, not one HTTP request per album.
    backfill_missing(&mut c, period.key()).map_err(|e| e.to_string())?;
    read_snapshot(&c, period, warning).map_err(|e| e.to_string())
}

fn library_ids(c: &Connection, period: &str) -> rusqlite::Result<HashSet<i64>> {
    c.prepare("SELECT DISTINCT e.gallery_id FROM download_entries e WHERE NOT EXISTS (SELECT 1 FROM download_popularity_ranks r WHERE r.period=?1 AND r.gallery_id=e.gallery_id)")?
        .query_map([period], |r| r.get(0))?.collect()
}

fn assign_ranks(bytes: &[u8], mut missing: HashSet<i64>) -> BTreeMap<i64, Option<u32>> {
    let mut ranks = BTreeMap::new();
    for (index, bytes) in bytes.chunks_exact(4).enumerate() {
        let id = i64::from(u32::from_be_bytes(
            bytes.try_into().expect("four-byte chunk"),
        ));
        if missing.remove(&id) {
            ranks.insert(id, Some(index as u32 + 1));
        }
        if missing.is_empty() {
            break;
        }
    }
    for id in missing {
        ranks.insert(id, None);
    }
    ranks
}

fn insert_ranks(
    c: &Connection,
    period: &str,
    ranks: BTreeMap<i64, Option<u32>>,
) -> rusqlite::Result<()> {
    let mut insert = c.prepare(
        "INSERT OR REPLACE INTO download_popularity_ranks(period,gallery_id,rank) VALUES(?1,?2,?3)",
    )?;
    for (id, rank) in ranks {
        insert.execute(params![period, id, rank])?;
    }
    Ok(())
}

fn store_snapshot(
    c: &mut Connection,
    period: &str,
    bytes: &[u8],
    date: &str,
) -> rusqlite::Result<()> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return Err(rusqlite::Error::InvalidParameterName(
            "invalid popularity index".into(),
        ));
    }
    let ids = c
        .prepare("SELECT DISTINCT gallery_id FROM download_entries")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<HashSet<i64>>>()?;
    // Scan the remote index before taking the SQLite writer lock.
    let ranks = assign_ranks(bytes, ids);
    let tx = c.transaction()?;
    tx.execute("INSERT INTO hitomi_popularity_snapshots(period,ordered_ids,fetched_at) VALUES(?1,?2,?3) ON CONFLICT(period) DO UPDATE SET ordered_ids=excluded.ordered_ids,fetched_at=excluded.fetched_at", params![period, bytes, date])?;
    tx.execute(
        "DELETE FROM download_popularity_ranks WHERE period=?1",
        [period],
    )?;
    insert_ranks(&tx, period, ranks)?;
    tx.commit()
}

fn backfill_missing(c: &mut Connection, period: &str) -> rusqlite::Result<()> {
    let ids = library_ids(c, period)?;
    if ids.is_empty() {
        return Ok(());
    }
    let bytes: Vec<u8> = c.query_row(
        "SELECT ordered_ids FROM hitomi_popularity_snapshots WHERE period=?1",
        [period],
        |r| r.get(0),
    )?;
    let ranks = assign_ranks(&bytes, ids);
    let tx = c.transaction()?;
    insert_ranks(&tx, period, ranks)?;
    tx.commit()
}

fn read_snapshot(
    c: &Connection,
    period: Period,
    warning: Option<String>,
) -> rusqlite::Result<PopularitySnapshot> {
    let fetched_at = c.query_row(
        "SELECT fetched_at FROM hitomi_popularity_snapshots WHERE period=?1",
        [period.key()],
        |r| r.get(0),
    )?;
    let ranks = c.prepare("SELECT r.gallery_id,r.rank FROM download_popularity_ranks r WHERE r.period=?1 AND EXISTS (SELECT 1 FROM download_entries e WHERE e.gallery_id=r.gallery_id)")?
        .query_map([period.key()], |r| Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(PopularitySnapshot {
        period,
        fetched_at,
        ranks,
        warning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranks_are_period_specific_unknown_last_and_newcomers_use_saved_index() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE download_entries(gallery_id INTEGER,state TEXT); CREATE TABLE hitomi_popularity_snapshots(period TEXT PRIMARY KEY,ordered_ids BLOB,fetched_at TEXT); CREATE TABLE download_popularity_ranks(period TEXT,gallery_id INTEGER,rank INTEGER,PRIMARY KEY(period,gallery_id)); INSERT INTO download_entries VALUES(1,'completed'),(2,'review_required'),(7,'failed');").unwrap();
        let bytes = [2_u32, 3, 1]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect::<Vec<_>>();
        store_snapshot(&mut c, "year", &bytes, "2026-09-30T00:00:00Z").unwrap();
        let year = read_snapshot(&c, Period::Year, None).unwrap();
        assert_eq!(
            year.ranks,
            BTreeMap::from([(1, Some(3)), (2, Some(1)), (7, None)])
        );
        c.execute("INSERT INTO download_entries VALUES(3,'queued')", [])
            .unwrap();
        backfill_missing(&mut c, "year").unwrap();
        assert_eq!(
            read_snapshot(&c, Period::Year, None).unwrap().ranks[&3],
            Some(2)
        );
        store_snapshot(
            &mut c,
            "month",
            &1_u32.to_be_bytes(),
            "2026-09-30T00:00:00Z",
        )
        .unwrap();
        assert_eq!(
            read_snapshot(&c, Period::Month, None).unwrap().ranks[&1],
            Some(1)
        );
        assert_eq!(
            read_snapshot(&c, Period::Year, None).unwrap().ranks[&1],
            Some(3)
        );
        assert!(store_snapshot(&mut c, "year", &[0, 1, 2], "bad").is_err());
        let state: String = c
            .query_row(
                "SELECT state FROM download_entries WHERE gallery_id=2",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "review_required");
    }
}
