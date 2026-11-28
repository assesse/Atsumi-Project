//! Disposable, lease-owned timing derivative. No original file is ever written.
//! Every admitted ledger row is scanned before either lookup table is exposed.
use super::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

// Same 512 MiB derivative budget as the existing SOOP replay SQLite reader.
// One binary UUID row is <= 64 payload bytes, one ACK row <= 80, both much
// smaller than their authenticated JSON records. This is a disk budget, not
// a retained-prefix row cap: FULL fails the entire disposable build explicitly.
const MAX_PAGES: u32 = 131_072;
const SQL_DEADLINE: Duration = Duration::from_secs(2);
const TRANSACTION_ROWS: u64 = 256;
const EVENT_INSERT: &str = "INSERT INTO events(hash,ticks,service,sequence,ambiguous) VALUES(?1,?2,?3,?4,0) ON CONFLICT(hash) DO UPDATE SET ambiguous=1";
const ACK_INSERT: &str = "INSERT INTO acks(transport_min,ordinal,source_min,source_end,transport_end,sha) VALUES(?1,?2,?3,?4,?5,?6)";
const ACK_QUERY: &str = "SELECT transport_min,ordinal,source_min,source_end,transport_end,sha FROM (SELECT * FROM acks WHERE transport_min<=?1 ORDER BY transport_min DESC LIMIT 2) UNION ALL SELECT transport_min,ordinal,source_min,source_end,transport_end,sha FROM (SELECT * FROM acks WHERE transport_min>?1 ORDER BY transport_min ASC LIMIT 1)";

pub(super) struct DiskIndex {
    // Declaration order closes the connection before removing its owned TempDir.
    connection: Mutex<Connection>,
    _directory: tempfile::TempDir,
}
fn sql_error(error: rusqlite::Error, cancel: &AtomicBool) -> TimingReadError {
    if cancel.load(Ordering::Acquire) { return TimingReadError::Cancelled; }
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DiskFull | rusqlite::ErrorCode::TooBig) => TimingReadError::Limit,
        // An internal VM deadline is an I/O/preparation failure, not a user
        // cancellation. It never yields a successful partially built index.
        _ => TimingReadError::Io,
    }
}
fn sql<T>(db: &Connection, cancel: &AtomicBool, work: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
    cancelled(cancel)?;
    let deadline = Instant::now() + SQL_DEADLINE;
    db.progress_handler(1000, Some(move || Instant::now() >= deadline));
    let result = work(db);
    db.progress_handler(0, None::<fn() -> bool>);
    cancelled(cancel)?;
    result.map_err(|e| sql_error(e, cancel))
}
pub(super) fn lock<'a, T>(mutex: &'a Mutex<T>, cancel: &AtomicBool) -> Result<MutexGuard<'a, T>> {
    loop {
        cancelled(cancel)?;
        match mutex.try_lock() {
            Ok(guard) => { cancelled(cancel)?; return Ok(guard); }
            Err(TryLockError::Poisoned(_)) => return Err(TimingReadError::Io),
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}
fn number(bytes: Vec<u8>) -> Result<u64> {
    Ok(u64::from_be_bytes(bytes.try_into().map_err(|_| TimingReadError::Schema)?))
}
impl DiskIndex {
    fn create(max_pages: u32, cancel: &AtomicBool) -> Result<Self> {
        cancelled(cancel)?;
        require((3..=MAX_PAGES).contains(&max_pages), TimingReadError::Limit)?;
        let directory = tempfile::Builder::new().prefix("atsumi-relay-timing-").tempdir().map_err(|_| TimingReadError::Io)?;
        let path = directory.path().join("index.sqlite");
        // The private fresh TempDir and create_new prohibit opening an existing
        // profile/cache/source. No caller controls this database path.
        fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|_| TimingReadError::Io)?;
        let db = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| sql_error(e, cancel))?;
        db.busy_timeout(Duration::ZERO).map_err(|e| sql_error(e, cancel))?;
        sql(&db, cancel, |db| db.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA page_size=4096; PRAGMA cache_size=-2048; PRAGMA temp_store=FILE; PRAGMA mmap_size=0; PRAGMA journal_mode=DELETE; PRAGMA synchronous=OFF;"))?;
        sql(&db, cancel, |db| db.pragma_update(None, "max_page_count", max_pages))?;
        sql(&db, cancel, |db| db.execute_batch("CREATE TABLE events(hash BLOB PRIMARY KEY CHECK(length(hash)=32),ticks BLOB CHECK(ticks IS NULL OR length(ticks)=8),service INTEGER NOT NULL CHECK(service BETWEEN 0 AND 65535),sequence BLOB NOT NULL CHECK(length(sequence)=8),ambiguous INTEGER NOT NULL CHECK(ambiguous IN(0,1))) WITHOUT ROWID;
            CREATE TABLE acks(transport_min BLOB PRIMARY KEY CHECK(length(transport_min)=8),ordinal BLOB NOT NULL CHECK(length(ordinal)=8),source_min BLOB NOT NULL CHECK(length(source_min)=8),source_end BLOB NOT NULL CHECK(length(source_end)=8),transport_end BLOB NOT NULL CHECK(length(transport_end)=8),sha BLOB NOT NULL CHECK(length(sha)=32)) WITHOUT ROWID;"))?;
        Ok(Self { connection: Mutex::new(db), _directory: directory })
    }
    fn insert_event(db: &Connection, event: &soop_events::EventRow, cancel: &AtomicBool) -> Result<()> {
        let (hash, ticks, service, sequence) = event.index_parts()?;
        let ticks = ticks.map(u64::to_be_bytes);
        sql(db, cancel, |db| db.execute(EVENT_INSERT, params![&hash[..], ticks.as_ref().map(|v| &v[..]),
            i64::from(service), &sequence.to_be_bytes()[..]]))?;
        Ok(())
    }
    fn insert_ack(db: &Connection, ordinal: u64, row: &points::VideoAckRange, cancel: &AtomicBool) -> Result<()> {
        sql(db, cancel, |db| db.execute(ACK_INSERT, params![&row.transport_min.to_be_bytes()[..],
            &ordinal.to_be_bytes()[..], &row.source_min.to_be_bytes()[..], &row.source_end.to_be_bytes()[..],
            &row.transport_end.to_be_bytes()[..], &row.sha256[..]]))?;
        Ok(())
    }
    fn build<R: BufRead>(reader: &mut R, owner: &ExpectedOwner, durable: Durable, native_acks: u64,
        native_end: bool, expected_summary: &RelayTimingSummary, cancel: &AtomicBool, max_pages: u32) -> Result<Self> {
        cancelled(cancel)?;
        let index = Self::create(max_pages, cancel)?;
        let db = lock(&index.connection, cancel)?;
        let mut state = State::new(owner.dialect(), durable.incomplete, native_acks, native_end);
        state.report.sidecar_incomplete = durable.sidecar_incomplete;
        let mut line = Vec::with_capacity(LINE_BYTES + 1);
        let mut bytes = 0u64;
        let mut inserted = 0u64;
        let mut prefix = 0u64;
        let mut previous: Option<(u64, u64)> = None;
        sql(&db, cancel, |db| db.execute_batch("BEGIN IMMEDIATE"))?;
        loop {
            cancelled(cancel)?;
            line.clear();
            let count = (&mut *reader).take((LINE_BYTES + 2) as u64).read_until(b'\n', &mut line).map_err(|_| TimingReadError::Io)?;
            if count == 0 { break; }
            bytes = bytes.checked_add(count as u64).ok_or(TimingReadError::Limit)?;
            let line_limit = if owner.dialect() == Dialect::Soop { LINE_BYTES + 1 } else { LINE_BYTES };
            require(bytes <= owner.dialect().bytes() && count <= line_limit && state.report.records < owner.dialect().bytes()/64, TimingReadError::Limit)?;
            require(count > 1 && line.last() == Some(&b'\n'), TimingReadError::Schema)?;
            let raw = &line[..count - 1];
            // Native authority and all row/order/clock checks are reapplied;
            // the original native ACK count is not reconstructed from a partial summary.
            state.row(raw, owner)?;
            let segment = match owner.dialect() {
                Dialect::Soop => {
                    let row: SoopRow = parse(raw)?;
                    match (row.kind.as_str(), row.data) {
                        ("soop_event_clock", SoopData::Event(event)) => {
                            Self::insert_event(&db, &event, cancel)?; inserted += 1; None
                        }
                        ("segment_ack", SoopData::Segment(s)) => Some((s.ordinal, s.sha256, s.timing)),
                        _ => None,
                    }
                }
                Dialect::Chzzk => match parse::<ChzzkRow>(raw)? {
                    ChzzkRow::Segment { ordinal, sha256, source, .. } => Some((ordinal, sha256, source)),
                    _ => None,
                },
            };
            if let Some((ordinal, sha, timing)) = segment {
                // A native incomplete/gapped ledger retains only the verified
                // contiguous transport prefix for mapping, exactly as before.
                if ordinal < expected_summary.prefix_segments {
                    require(ordinal == prefix, TimingReadError::Order)?;
                    let row = points::VideoAckRange { source_min: timing.video.min_pts, source_end: timing.video.max_pts_end,
                        transport_min: timing.video.min_pts90k, transport_end: timing.video.max_pts_end90k, sha256: points::decode_sha(&sha)? };
                    require(row.source_min < row.source_end && row.transport_min < row.transport_end, TimingReadError::Clock)?;
                    if let Some((min, end)) = previous {
                        require(row.transport_min > min && row.transport_min >= end.saturating_sub(1), TimingReadError::Order)?;
                    }
                    previous = Some((row.transport_min, row.transport_end));
                    Self::insert_ack(&db, ordinal, &row, cancel)?;
                    prefix += 1; inserted += 1;
                }
            }
            if inserted >= TRANSACTION_ROWS {
                sql(&db, cancel, |db| db.execute_batch("COMMIT; BEGIN IMMEDIATE"))?;
                inserted = 0;
            }
        }
        require(state.report.records == durable.records && state.report.acknowledged_segments <= native_acks, TimingReadError::Order)?;
        if !state.report.transport_complete { state.report.timing_incomplete = true; }
        require(state.report == *expected_summary && prefix == expected_summary.prefix_segments, TimingReadError::Identity)?;
        cancelled(cancel)?;
        sql(&db, cancel, |db| db.execute_batch("COMMIT; PRAGMA query_only=ON;"))?;
        drop(db);
        cancelled(cancel)?;
        // No reference to this DB escapes before the final EOF and summary checks.
        Ok(index)
    }
    pub(super) fn events(&self, ids: &[Option<&str>], cancel: &AtomicBool) -> Result<Vec<EventPosition>> {
        let db = lock(&self.connection, cancel)?;
        ids.iter().map(|id| {
            cancelled(cancel)?;
            let Some(id) = id else { return Ok(EventPosition::Unknown("missing_event_identity")); };
            let Ok(hash) = soop_events::hash(id) else { return Ok(EventPosition::Unknown("invalid_event_identity")); };
            let row = sql(&db, cancel, |db| db.query_row("SELECT ticks,service,sequence,ambiguous FROM events WHERE hash=?1", [&hash[..]],
                |r| Ok((r.get::<_, Option<Vec<u8>>>(0)?, r.get::<_, u16>(1)?, r.get::<_, Vec<u8>>(2)?, r.get::<_, bool>(3)?))).optional())?;
            let Some((ticks, service, sequence, ambiguous)) = row else { return Ok(EventPosition::Unknown("missing_event_source_clock")); };
            if ambiguous { return Ok(EventPosition::Unknown("ambiguous_event_source_clock")); }
            Ok(match ticks {
                Some(ticks) => EventPosition::Source { ticks90k: number(ticks)?, service, observation_sequence: number(sequence)? },
                None => EventPosition::Unknown("inactive_or_hidden_source_clock"),
            })
        }).collect()
    }
    pub(super) fn points(&self, summary: &RelayTimingSummary, points90k: &[u64], cancel: &AtomicBool) -> Result<Vec<Option<points::AcknowledgedSourcePoint>>> {
        let db = lock(&self.connection, cancel)?;
        let (Some(origin), Some(video)) = (summary.origin, summary.video) else { return Err(TimingReadError::Clock); };
        points90k.iter().map(|source| {
            cancelled(cancel)?;
            if u128::from(*source)*u128::from(origin.timescale) < u128::from(origin.ticks)*90_000 { return Ok(None); }
            let transport = to90k(*source, 90_000, origin)?;
            let mut rows = sql(&db, cancel, |db| {
                let mut statement = db.prepare(ACK_QUERY)?;
                let rows = statement.query_map([&transport.to_be_bytes()[..]], |r| {
                    Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Vec<u8>>(2)?,
                        r.get::<_, Vec<u8>>(3)?, r.get::<_, Vec<u8>>(4)?, r.get::<_, Vec<u8>>(5)?))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })?;
            require(rows.len() <= 3, TimingReadError::Limit)?;
            rows.sort_unstable_by(|a, b| a.1.cmp(&b.1));
            for (min, ordinal, source_min, source_end, end, sha) in rows {
                cancelled(cancel)?;
                let row = points::VideoAckRange { transport_min: number(min)?, source_min: number(source_min)?, source_end: number(source_end)?,
                    transport_end: number(end)?, sha256: sha.try_into().map_err(|_| TimingReadError::Schema)? };
                if let Some(mapped) = points::point_in_range(&row, video.timescale, number(ordinal)?, *source, transport) { return Ok(Some(mapped)); }
            }
            Ok(None)
        }).collect()
    }
}
impl RelayTimingLease {
    pub(super) fn ensure_index(&self, cancel: &AtomicBool) -> Result<&DiskIndex> {
        cancelled(cancel)?;
        if self.disk_index.get().is_none() {
            let _read = lock(&self.reference_read, cancel)?;
            if self.disk_index.get().is_none() {
                let mut file = self._files[2].try_clone().map_err(|_| TimingReadError::Io)?;
                file.seek(SeekFrom::Start(0)).map_err(|_| TimingReadError::Io)?;
                let built = DiskIndex::build(&mut BufReader::with_capacity(8192, file), &self.owner, self.durable,
                    self.native_acks, self.native_end, &self.summary, cancel, MAX_PAGES)?;
                cancelled(cancel)?;
                self.disk_index.set(built).map_err(|_| TimingReadError::Order)?;
            }
        }
        cancelled(cancel)?;
        self.disk_index.get().ok_or(TimingReadError::Order)
    }
}

#[cfg(test)]
#[path="relay_timing_disk_index_tests.rs"]
mod tests;
