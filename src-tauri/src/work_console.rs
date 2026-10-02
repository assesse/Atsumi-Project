//! Bounded, read-only queue projection. It does not own download state transitions.
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path, time::Duration};

const ACTIVE: &str =
    "('queued','resolving_metadata','downloading','hashing','verifying','retry_wait')";
// An entry reused by a later request still belongs to its original request.
const OWNER: &str = "WITH owners AS (SELECT x.entry_id, MIN(r.rowid) AS sequence FROM download_queue_request_entries x JOIN download_queue_requests r ON r.request_id=x.request_id GROUP BY x.entry_id)";

#[derive(Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueueQuery {
    pub sequence: Option<i64>,
    #[serde(default)]
    pub after: bool,
    #[serde(default)]
    pub page: u32,
    #[serde(default)]
    pub include_settled: bool,
    #[serde(default)]
    pub cancellation_preview: bool,
    pub observed_since: Option<String>,
    pub offset: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueRow {
    entry_id: String,
    gallery_id: i64,
    title: String,
    artist: String,
    state: String,
    progress: f64,
    sequence: Option<i64>,
    updated_at: String,
    error_code: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueBatch {
    sequence: i64,
    request_id: String,
    requested: u64,
    owned: u64,
    active: u64,
    created_at: Option<String>,
    label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    queried_at: String,
    counts: BTreeMap<String, u64>,
    global_active: u64,
    total_rows: u64,
    page: u32,
    page_size: u32,
    offset: u32,
    items: Vec<QueueRow>,
    batches: Vec<QueueBatch>,
    /// Rate of end-to-end completed jobs during a recent observed window, not CPU utilization.
    eta_seconds: Option<u64>,
    recent_completed: u64,
    last_progress_at: Option<String>,
}

pub fn connect(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    connection
        .busy_timeout(Duration::from_millis(300))
        .map_err(|e| e.to_string())?;
    connection
        .execute_batch("PRAGMA query_only=ON; BEGIN DEFERRED;")
        .map_err(|e| e.to_string())?;
    Ok(connection)
}

pub fn snapshot(path: &Path, query: &QueueQuery) -> Result<QueueSnapshot, String> {
    snapshot_connection(&connect(path)?, query).map_err(|e| e.to_string())
}

fn snapshot_connection(c: &Connection, query: &QueueQuery) -> rusqlite::Result<QueueSnapshot> {
    if query
        .observed_since
        .as_ref()
        .is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_err())
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "observedSince must be RFC3339".into(),
        ));
    }
    let scope = "(?1 IS NULL OR (CASE WHEN ?2 THEN o.sequence > ?1 ELSE o.sequence = ?1 END))";
    let mut counts = BTreeMap::new();
    let mut statement = c.prepare(&format!("{OWNER} SELECT e.state, COUNT(*) FROM download_entries e LEFT JOIN owners o ON o.entry_id=e.entry_id WHERE {scope} AND (?3 IS NULL OR e.state IN {ACTIVE} OR EXISTS (SELECT 1 FROM download_jobs j WHERE j.entry_id=e.entry_id AND julianday(COALESCE(j.finished_at,j.updated_at)) >= julianday(?3))) GROUP BY e.state"))?;
    for result in statement.query_map(
        params![query.sequence, query.after, query.observed_since],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)),
    )? {
        let (state, count) = result?;
        counts.insert(state, count);
    }
    let global_active: u64 = c.query_row(
        &format!("SELECT COUNT(*) FROM download_jobs WHERE state IN {ACTIVE}"),
        [],
        |r| r.get(0),
    )?;
    let where_rows = format!("{scope} AND (?3 OR e.state IN {ACTIVE})");
    let include_settled = query.include_settled && !query.cancellation_preview;
    let total_rows = c.query_row(&format!("{OWNER} SELECT COUNT(*) FROM download_entries e LEFT JOIN owners o ON o.entry_id=e.entry_id WHERE {where_rows}"), params![query.sequence, query.after, include_settled], |r| r.get(0))?;
    let page = if query.cancellation_preview {
        1
    } else {
        query.page.clamp(1, 100_000)
    };
    let page_size = if query.cancellation_preview {
        10_000
    } else {
        100
    };
    let offset = if query.cancellation_preview {
        0
    } else {
        query
            .offset
            .unwrap_or((page - 1) * page_size)
            .min(10_000_000)
    };
    let mut statement = c.prepare(&format!("{OWNER}
      SELECT e.entry_id,e.gallery_id,COALESCE(g.title,'#' || e.gallery_id),COALESCE(g.primary_artist,''),e.state,e.progress,o.sequence,e.updated_at,
        (SELECT j.last_error_code FROM download_jobs j WHERE j.entry_id=e.entry_id ORDER BY j.rowid DESC LIMIT 1)
      FROM download_entries e LEFT JOIN owners o ON o.entry_id=e.entry_id LEFT JOIN galleries g ON g.gallery_id=e.gallery_id
      WHERE {where_rows} ORDER BY CASE WHEN e.state IN {ACTIVE} THEN 0 ELSE 1 END, o.sequence DESC,e.created_at DESC,e.entry_id LIMIT {page_size} OFFSET ?4"))?;
    let items = statement
        .query_map(
            params![query.sequence, query.after, include_settled, offset],
            |r| {
                Ok(QueueRow {
                    entry_id: r.get(0)?,
                    gallery_id: r.get(1)?,
                    title: r.get(2)?,
                    artist: r.get(3)?,
                    state: r.get(4)?,
                    progress: r.get(5)?,
                    sequence: r.get(6)?,
                    updated_at: r.get(7)?,
                    error_code: r.get(8)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let mut statement = c.prepare(&format!("{OWNER}
      SELECT r.rowid,r.request_id,COUNT(DISTINCT x.entry_id),COUNT(DISTINCT CASE WHEN o.sequence=r.rowid THEN x.entry_id END),
        COUNT(DISTINCT CASE WHEN o.sequence=r.rowid AND e.state IN {ACTIVE} THEN e.entry_id END),
        MIN(CASE WHEN o.sequence=r.rowid THEN e.created_at END),COALESCE(MIN(g.primary_artist),'앨범 요청')
      FROM download_queue_requests r JOIN download_queue_request_entries x ON x.request_id=r.request_id
      JOIN download_entries e ON e.entry_id=x.entry_id LEFT JOIN owners o ON o.entry_id=x.entry_id LEFT JOIN galleries g ON g.gallery_id=e.gallery_id
      GROUP BY r.rowid HAVING COUNT(DISTINCT CASE WHEN o.sequence=r.rowid THEN x.entry_id END)>0
      ORDER BY (COUNT(DISTINCT CASE WHEN o.sequence=r.rowid AND e.state IN {ACTIVE} THEN e.entry_id END)>0) DESC,r.rowid DESC LIMIT 100"))?;
    let batches = statement
        .query_map([], |r| {
            Ok(QueueBatch {
                sequence: r.get(0)?,
                request_id: r.get(1)?,
                requested: r.get(2)?,
                owned: r.get(3)?,
                active: r.get(4)?,
                created_at: r.get(5)?,
                label: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let (recent_completed, window_seconds, seconds_since_progress): (u64, f64, Option<f64>) = c.query_row(
      "SELECT COUNT(*), COALESCE((julianday('now')-julianday(MIN(finished_at)))*86400,0), (julianday('now')-julianday(MAX(finished_at)))*86400 FROM download_jobs WHERE state='completed' AND julianday(finished_at)>julianday('now','-30 minutes')", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    let last_progress_at = c.query_row("SELECT MAX(updated_at) FROM download_jobs", [], |r| {
        r.get(0)
    })?;
    let eta_seconds = if global_active == 0 {
        Some(0)
    } else if recent_completed >= 3
        && window_seconds >= 60.
        && seconds_since_progress.is_some_and(|s| s < 300.)
    {
        Some((global_active as f64 * window_seconds / recent_completed as f64).ceil() as u64)
    } else {
        None
    };
    Ok(QueueSnapshot {
        queried_at: chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())
            .to_rfc3339(),
        counts,
        global_active,
        total_rows,
        page,
        page_size,
        offset,
        items,
        batches,
        eta_seconds,
        recent_completed,
        last_progress_at,
    })
}

#[tauri::command]
pub async fn work_queue_snapshot(
    app: tauri::AppHandle,
    query: QueueQuery,
) -> Result<QueueSnapshot, String> {
    use tauri::Manager;
    let path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("atsumi-next.sqlite3");
    tauri::async_runtime::spawn_blocking(move || snapshot(&path, &query))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn later_reused_request_does_not_own_old_work_and_settled_is_not_active() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE download_queue_requests(request_id TEXT); CREATE TABLE download_queue_request_entries(request_id TEXT,entry_id TEXT);
        CREATE TABLE download_entries(entry_id TEXT,gallery_id INTEGER,state TEXT,progress REAL,created_at TEXT,updated_at TEXT);
        CREATE TABLE download_jobs(entry_id TEXT,state TEXT,updated_at TEXT,finished_at TEXT,last_error_code TEXT);
        CREATE TABLE galleries(gallery_id INTEGER,title TEXT,primary_artist TEXT);
        INSERT INTO download_queue_requests VALUES('first'),('later');
        INSERT INTO download_queue_request_entries VALUES('first','old'),('later','old'),('later','new');
        INSERT INTO download_entries VALUES('old',1,'hashing',80,'2026-01-01','2026-01-01'),('new',2,'review_required',90,'2026-01-02','2026-01-02');
        INSERT INTO download_jobs VALUES('old','hashing','2026-01-01',NULL,NULL),('new','review_required','2026-01-02',NULL,NULL);").unwrap();
        let all = snapshot_connection(&c, &QueueQuery::default()).unwrap();
        assert_eq!(all.global_active, 1);
        assert_eq!(all.items.len(), 1);
        let session = snapshot_connection(
            &c,
            &QueueQuery {
                observed_since: Some("2026-01-03T00:00:00Z".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(session.counts.get("hashing"), Some(&1));
        assert!(!session.counts.contains_key("review_required"));
        assert!(snapshot_connection(
            &c,
            &QueueQuery {
                observed_since: Some("invalid".into()),
                ..Default::default()
            }
        )
        .is_err());
        let after = snapshot_connection(
            &c,
            &QueueQuery {
                sequence: Some(1),
                after: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(after.counts.get("review_required"), Some(&1));
        assert_eq!(after.total_rows, 0);
        assert_eq!(after.counts.get("hashing"), None);
        let batch = all.batches.iter().find(|b| b.sequence == 2).unwrap();
        assert_eq!(batch.requested, 2);
        assert_eq!(batch.owned, 1);
        for id in 10..120 {
            let entry = format!("extra-{id}");
            c.execute(
                "INSERT INTO download_entries VALUES(?1,?2,'queued',0,'2026-01-03','2026-01-03')",
                params![entry, id],
            )
            .unwrap();
            c.execute(
                "INSERT INTO download_jobs VALUES(?1,'queued','2026-01-03',NULL,NULL)",
                [&entry],
            )
            .unwrap();
            c.execute(
                "INSERT INTO download_queue_request_entries VALUES('later',?1)",
                [&entry],
            )
            .unwrap();
        }
        let second = snapshot_connection(
            &c,
            &QueueQuery {
                page: 2,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(second.total_rows, 111);
        assert_eq!(second.items.len(), 11);
        let window = snapshot_connection(
            &c,
            &QueueQuery {
                offset: Some(25),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(window.offset, 25);
        assert_eq!(window.items.len(), 86);
        let preview = snapshot_connection(
            &c,
            &QueueQuery {
                page: 2,
                include_settled: true,
                cancellation_preview: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(preview.page, 1);
        assert_eq!(preview.items.len(), 111);
        assert!(preview
            .items
            .iter()
            .all(|row| row.state != "review_required"));
    }
    #[test]
    fn production_connection_cannot_write_or_create_a_database() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queue.sqlite3");
        assert!(connect(&path).is_err());
        assert!(!path.exists());
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE evidence(id INTEGER); INSERT INTO evidence VALUES(1)")
                .unwrap();
        }
        let c = connect(&path).unwrap();
        assert_eq!(
            c.query_row("PRAGMA query_only", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            1
        );
        assert!(c.execute("DELETE FROM evidence", []).is_err());
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM evidence", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            1
        );
    }
}
