//! Disposable versioned SQLite derivative. One bounded line is decoded at a
//! time; media-time and ordinal indexes support seeks and complete log paging.
use super::*;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{BufRead, BufReader, Seek, SeekFrom},
    sync::atomic::Ordering,
};

const INDEX_VERSION: u32 = 1;
const MAX_TIMELINE_ROWS: u64 = 262_144;

pub(super) fn build(session: &Session) -> Result<IndexStatus, StreamError> {
    session.valid()?;
    if let Ok(status) = cached_status(session) {
        return Ok(status);
    }
    // A failed/cancelled derivative is disposable. Never remove a source file.
    if let Ok(metadata) = std::fs::symlink_metadata(&session.index_path) {
        if !regular_metadata(&metadata) {
            return Err(storage());
        }
        std::fs::remove_file(&session.index_path).map_err(|_| storage())?;
    }
    let _ = std::fs::remove_file(session.index_path.with_extension("sqlite-journal"));
    let root = session.index_path.parent().ok_or_else(storage)?;
    let used = std::fs::read_dir(root)
        .map_err(|_| storage())?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .try_fold(0u64, |sum, len| sum.checked_add(len))
        .ok_or_else(storage)?;
    if used > 2 * 1024 * 1024 * 1024 {
        return Err(failure(
            "REPLAY_INDEX_FULL",
            "채팅 인덱스 저장 공간 한도에 도달했습니다.",
        ));
    }
    let result = build_fresh(session);
    if result.is_err() {
        let _ = std::fs::remove_file(&session.index_path);
        let _ = std::fs::remove_file(session.index_path.with_extension("sqlite-journal"));
    }
    result
}
fn cached_status(session: &Session) -> Result<IndexStatus, StreamError> {
    let connection = read_connection(session)?;
    let check = connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get::<_, String>(0))
        .map_err(|_| storage())?;
    if check != "ok" {
        return Err(storage());
    }
    let status = connection
        .query_row(
            "SELECT observed_rows,approximate_rows,warnings FROM metadata",
            [],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|_| storage())?;
    if status.2.len() > 8192 {
        return Err(storage());
    }
    let warnings: Vec<String> = serde_json::from_str(&status.2).map_err(|_| storage())?;
    if warnings.len() > 16 || warnings.iter().any(|warning| warning.len() > 1024) {
        return Err(storage());
    }
    // Exercise both lookup indexes before accepting a previously cached file.
    connection.query_row("SELECT COUNT(*) FROM (SELECT ordinal FROM messages ORDER BY media_time DESC,ordinal DESC LIMIT 1)", [], |row| row.get::<_,u64>(0)).map_err(|_| storage())?;
    connection
        .query_row(
            "SELECT COUNT(*) FROM (SELECT id FROM assets LIMIT 1)",
            [],
            |row| row.get::<_, u64>(0),
        )
        .map_err(|_| storage())?;
    session.valid()?;
    Ok(IndexStatus {
        state: "ready".into(),
        warnings,
        observed_rows: status.0,
        approximate_rows: status.1,
    })
}
fn build_fresh(session: &Session) -> Result<IndexStatus, StreamError> {
    // UUID filename and create_new prevent following a pre-existing cache link.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&session.index_path)
        .map_err(|_| storage())?;
    let mut connection = Connection::open(&session.index_path).map_err(|_| storage())?;
    connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA journal_mode=DELETE; PRAGMA synchronous=NORMAL; PRAGMA cache_size=-2048; PRAGMA temp_store=FILE; PRAGMA max_page_count=262144;
        CREATE TABLE metadata(version INTEGER NOT NULL, source TEXT NOT NULL, complete INTEGER NOT NULL, observed_rows INTEGER NOT NULL DEFAULT 0, approximate_rows INTEGER NOT NULL DEFAULT 0, warnings TEXT NOT NULL DEFAULT '[]');
        CREATE TABLE segments(ordinal INTEGER PRIMARY KEY, source_start REAL, source_end REAL, merged_start REAL NOT NULL, duration REAL NOT NULL);
        CREATE INDEX segments_source ON segments(source_start);
        CREATE TABLE messages(ordinal INTEGER PRIMARY KEY, sequence INTEGER NOT NULL UNIQUE, media_time REAL NOT NULL, quality TEXT NOT NULL, sender_key TEXT, sender TEXT NOT NULL, text TEXT NOT NULL, payload BLOB NOT NULL);
        CREATE INDEX messages_time ON messages(media_time DESC, ordinal DESC);
        CREATE INDEX messages_sender_time ON messages(sender_key,media_time);
        CREATE TABLE assets(id TEXT PRIMARY KEY);").map_err(|_| storage())?;
    connection
        .execute(
            "INSERT INTO metadata(version,source,complete) VALUES (?1,?2,0)",
            params![INDEX_VERSION, session.fingerprint],
        )
        .map_err(|_| storage())?;
    load_timeline(session, &mut connection)?;
    let mut status = IndexStatus::default();
    let Some(chat) = &session.chat else {
        status.state = "ready".into();
        connection
            .execute("UPDATE metadata SET complete=1", [])
            .map_err(|_| storage())?;
        return Ok(status);
    };
    let mut source = chat.try_clone().map_err(|_| storage())?;
    source.seek(SeekFrom::Start(0)).map_err(|_| storage())?;
    let mut reader = BufReader::with_capacity(MAX_LINE_BYTES, source);
    let mut ordinal = 0u64;
    let mut previous_sequence = 0u64;
    let mut previous_time = 0f64;
    let mut malformed = 0u64;
    let mut duplicate = 0u64;
    let mut backward = 0u64;
    let mut tail = false;
    loop {
        let transaction = connection.transaction().map_err(|_| storage())?;
        let mut end = false;
        for _ in 0..512 {
            if session.cancel.load(Ordering::Acquire) {
                return Err(stale());
            }
            let Some(line) = bounded_line(&mut reader, &session.cancel)? else {
                end = true;
                break;
            };
            ordinal += 1;
            if !line.terminated {
                tail = true;
                end = true;
                break;
            }
            if line.oversized {
                malformed += 1;
                continue;
            }
            let Ok(raw) = serde_json::from_slice::<Value>(&line.bytes) else {
                malformed += 1;
                continue;
            };
            let Ok(message) = serde_json::from_value::<ChatMessage>(raw.clone()) else {
                malformed += 1;
                continue;
            };
            if !valid_message(&message) {
                malformed += 1;
                continue;
            }
            let observed_time = observed_media_time(&transaction, &raw);
            let media_time = observed_time.unwrap_or(message.offset_seconds);
            let quality = if observed_time.is_some() {
                "observed_media"
            } else {
                "receive_time_approximate"
            };
            let sender_key = raw
                .get("senderKey")
                .and_then(Value::as_str)
                .filter(|value| valid_sender_key(value));
            let payload = serde_json::to_vec(&message).map_err(|_| storage())?;
            if payload.len() > MAX_LINE_BYTES {
                malformed += 1;
                continue;
            }
            let changed = transaction.execute("INSERT OR IGNORE INTO messages(ordinal,sequence,media_time,quality,sender_key,sender,text,payload) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", params![ordinal, message.sequence, media_time, quality, sender_key, message.sender, message.text, payload]).map_err(|_| storage())?;
            if changed == 0 {
                duplicate += 1;
                continue;
            }
            if let Some(rich) = &message.rich {
                for url in rich
                    .badges
                    .iter()
                    .map(|badge| &badge.image_url)
                    .chain(rich.emojis.iter().map(|emoji| &emoji.image_url))
                {
                    if let Some(id) = super::super::replay_assets::asset_id(url) {
                        transaction
                            .execute("INSERT OR IGNORE INTO assets(id) VALUES (?1)", [id])
                            .map_err(|_| storage())?;
                    }
                }
            }
            if message.sequence <= previous_sequence || media_time < previous_time {
                backward += 1;
            }
            previous_sequence = message.sequence;
            previous_time = media_time;
            if quality == "observed_media" {
                status.observed_rows += 1;
            } else {
                status.approximate_rows += 1;
            }
        }
        transaction.commit().map_err(|_| storage())?;
        session.valid()?;
        if end {
            break;
        }
    }
    if malformed > 0 {
        status.warnings.push(format!(
            "손상되거나 너무 큰 채팅 {malformed}행은 건너뛰었습니다. 원본은 보존됩니다."
        ));
    }
    if duplicate > 0 {
        status.warnings.push(format!(
            "중복된 채팅 번호 {duplicate}행은 한 번만 표시합니다."
        ));
    }
    if backward > 0 {
        status.warnings.push(
            "시각 또는 순서가 역행한 기록이 있어 시간 조회와 원본 로그 순서가 다를 수 있습니다."
                .into(),
        );
    }
    if tail {
        status
            .warnings
            .push("완료되지 않은 채팅 마지막 줄은 표시하지 않습니다. 원본은 보존됩니다.".into());
    }
    if status.approximate_rows > 0 {
        status.warnings.push(
            "수신 시각 기준의 근사 동기화가 포함됩니다. 채팅 보정값으로 조정할 수 있습니다.".into(),
        );
    }
    session.valid()?;
    connection
        .execute(
            "UPDATE metadata SET complete=1,observed_rows=?1,approximate_rows=?2,warnings=?3",
            params![
                status.observed_rows,
                status.approximate_rows,
                serde_json::to_string(&status.warnings).map_err(|_| storage())?
            ],
        )
        .map_err(|_| storage())?;
    status.state = "ready".into();
    Ok(status)
}

fn valid_message(message: &ChatMessage) -> bool {
    message.sequence > 0
        && message.sequence <= 9_007_199_254_740_991
        && message.sender.chars().count() <= 128
        && message.text.chars().count() <= 4096
        && message.offset_seconds.is_finite()
        && (0.0..=31_536_000.0).contains(&message.offset_seconds)
        && message
            .broadcast_offset_seconds
            .is_none_or(|value| value.is_finite() && value >= 0.0 && value <= 31_536_000.0)
}
fn valid_sender_key(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn load_timeline(session: &Session, connection: &mut Connection) -> Result<(), StreamError> {
    let mut file = session.timeline.try_clone().map_err(|_| storage())?;
    file.seek(SeekFrom::Start(0)).map_err(|_| storage())?;
    let mut reader = BufReader::with_capacity(MAX_LINE_BYTES, file);
    let transaction = connection.transaction().map_err(|_| storage())?;
    let mut ordinal = 0u64;
    let mut previous_end = 0.0;
    while let Some(line) = bounded_line(&mut reader, &session.cancel)? {
        if line.oversized || !line.terminated || ordinal >= MAX_TIMELINE_ROWS {
            return Err(storage());
        }
        let row: Value = serde_json::from_slice(&line.bytes).map_err(|_| storage())?;
        let start = row
            .get("mergedStartSeconds")
            .and_then(Value::as_f64)
            .ok_or_else(storage)?;
        let duration = row
            .get("mergedDurationSeconds")
            .and_then(Value::as_f64)
            .ok_or_else(storage)?;
        let source_start = row.get("sourceStartSeconds").and_then(Value::as_f64);
        let source_end = row.get("sourceEndSeconds").and_then(Value::as_f64);
        if !start.is_finite()
            || !duration.is_finite()
            || duration <= 0.0
            || duration > 122.0
            || (start - previous_end).abs() > 0.1
            || start < 0.0
            || row.get("chatRewritten").and_then(Value::as_bool) != Some(false)
            || row.get("segmentIndex").and_then(Value::as_u64) != Some(ordinal)
        {
            return Err(storage());
        }
        match (source_start, source_end) {
            (Some(a), Some(b))
                if a.is_finite()
                    && b.is_finite()
                    && a >= 0.0
                    && b > a
                    && duration <= b - a + 0.1 => {}
            (None, None) => {}
            _ => return Err(storage()),
        }
        transaction
            .execute(
                "INSERT INTO segments VALUES (?1,?2,?3,?4,?5)",
                params![ordinal, source_start, source_end, start, duration],
            )
            .map_err(|_| storage())?;
        ordinal += 1;
        previous_end = start + duration;
    }
    if ordinal == 0 || (previous_end - session.duration).abs() > (session.duration * 0.005).max(2.0)
    {
        return Err(storage());
    }
    transaction.commit().map_err(|_| storage())?;
    Ok(())
}

fn observed_clock(raw: &Value) -> Option<f64> {
    let clock = serde_json::from_value::<super::super::model::ChatReplayClock>(
        raw.get("replayClock")?.clone(),
    )
    .ok()?
    .bounded()?;
    if clock.clock != "mse_presentation_v1" {
        return None;
    }
    clock.source_time_seconds
}
fn observed_media_time(connection: &Connection, raw: &Value) -> Option<f64> {
    let source = observed_clock(raw)?;
    connection.query_row("SELECT merged_start + (?1-source_start) FROM segments WHERE source_start <= ?1 AND ?1 < source_start+duration ORDER BY source_start DESC LIMIT 1", [source], |row| row.get::<_,f64>(0)).optional().ok().flatten()
}

struct Line {
    bytes: Vec<u8>,
    oversized: bool,
    terminated: bool,
}
fn bounded_line<R: BufRead>(
    reader: &mut R,
    cancel: &AtomicBool,
) -> Result<Option<Line>, StreamError> {
    let mut line = Line {
        bytes: Vec::new(),
        oversized: false,
        terminated: false,
    };
    let mut seen = false;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(stale());
        }
        let buffer = reader.fill_buf().map_err(|_| storage())?;
        if buffer.is_empty() {
            return Ok(if seen { Some(line) } else { None });
        }
        seen = true;
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(buffer.len(), |index| index + 1);
        if !line.oversized {
            if line.bytes.len() + count > MAX_LINE_BYTES {
                line.oversized = true;
                line.bytes.clear();
            } else {
                line.bytes.extend_from_slice(&buffer[..count]);
            }
        }
        reader.consume(count);
        if newline.is_some() {
            line.terminated = true;
            return Ok(Some(line));
        }
    }
}

fn read_connection(session: &Session) -> Result<Connection, StreamError> {
    let metadata = std::fs::symlink_metadata(&session.index_path).map_err(|_| storage())?;
    if !regular_metadata(&metadata) {
        return Err(storage());
    }
    let connection = Connection::open_with_flags(
        &session.index_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| storage())?;
    connection
        .execute_batch(
            "PRAGMA trusted_schema=OFF; PRAGMA cache_size=-1024; PRAGMA temp_store=FILE;",
        )
        .map_err(|_| storage())?;
    let valid = connection
        .query_row("SELECT version,source,complete FROM metadata", [], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u32>(2)?,
            ))
        })
        .map_err(|_| storage())?;
    if valid != (INDEX_VERSION, session.fingerprint.clone(), 1) {
        return Err(stale());
    }
    Ok(connection)
}

pub(super) fn asset_allowed(session: &Session, id: &str) -> bool {
    read_connection(session)
        .and_then(|connection| {
            connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM assets WHERE id=?1)",
                    [id],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|_| storage())
        })
        .unwrap_or(false)
}

#[derive(Clone)]
struct Entry {
    ordinal: u64,
    sequence: u64,
    time: f64,
    item: ReplayChatMessage,
}
pub(super) fn page(
    session: &Session,
    media_time: Option<f64>,
    cursor: Option<&str>,
    generation: u64,
    limit: usize,
) -> Result<ReplayChatPage, StreamError> {
    let mut page = session.empty_page(generation)?;
    if page.index_state != "ready" {
        return Ok(page);
    }
    let connection = read_connection(session)?;
    let offset = *session.offset.lock().map_err(|_| storage())?;
    let (direction, anchor) = if let Some(cursor) = cursor {
        decode_cursor(&connection, session, cursor)?
    } else {
        ('b', i64::MAX as u64)
    };
    let (sql, bound) = if let Some(time) = media_time {
        ("SELECT ordinal,sequence,media_time,substr(quality,1,32),substr(payload,1,65537) FROM messages WHERE media_time <= ?1 ORDER BY media_time DESC,ordinal DESC LIMIT ?2", time - offset)
    } else if direction == 'a' {
        ("SELECT ordinal,sequence,media_time,substr(quality,1,32),substr(payload,1,65537) FROM messages WHERE ordinal > ?1 ORDER BY ordinal ASC LIMIT ?2", anchor as f64)
    } else {
        ("SELECT ordinal,sequence,media_time,substr(quality,1,32),substr(payload,1,65537) FROM messages WHERE ordinal < ?1 ORDER BY ordinal DESC LIMIT ?2", anchor as f64)
    };
    let mut query = connection.prepare(sql).map_err(|_| storage())?;
    let mut rows = query.query(params![bound, limit]).map_err(|_| storage())?;
    let mut entries = Vec::new();
    let mut bytes = serde_json::to_vec(&page).map_err(|_| storage())?.len() + 1024;
    while let Some(row) = rows.next().map_err(|_| storage())? {
        session.check_generation(generation)?;
        let payload: Vec<u8> = row.get(4).map_err(|_| storage())?;
        if payload.len() > MAX_LINE_BYTES {
            return Err(storage());
        }
        let message: ChatMessage = serde_json::from_slice(&payload).map_err(|_| storage())?;
        let base_time: f64 = row.get(2).map_err(|_| storage())?;
        let quality: String = row.get(3).map_err(|_| storage())?;
        if !valid_message(&message)
            || !base_time.is_finite()
            || base_time < 0.0
            || !matches!(
                quality.as_str(),
                "observed_media" | "receive_time_approximate"
            )
            || row.get::<_, u64>(1).map_err(|_| storage())? != message.sequence
        {
            return Err(storage());
        }
        let asset_ids = message
            .rich
            .as_ref()
            .map(|rich| {
                rich.badges
                    .iter()
                    .map(|badge| &badge.image_url)
                    .chain(rich.emojis.iter().map(|emoji| &emoji.image_url))
                    .filter_map(|url| {
                        super::super::replay_assets::asset_id(url).map(|id| (url.clone(), id))
                    })
                    .collect()
            })
            .unwrap_or_else(BTreeMap::new);
        let item = ReplayChatMessage {
            message,
            media_time_seconds: base_time + offset,
            sync_quality: quality,
            asset_ids,
        };
        let item_bytes = serde_json::to_vec(&item).map_err(|_| storage())?.len();
        if bytes + item_bytes > MAX_PAGE_BYTES {
            break;
        }
        bytes += item_bytes + 1;
        entries.push(Entry {
            ordinal: row.get(0).map_err(|_| storage())?,
            sequence: row.get(1).map_err(|_| storage())?,
            time: base_time,
            item,
        });
    }
    entries.sort_by_key(|entry| entry.ordinal);
    if let Some(first) = entries.first() {
        if connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE ordinal < ?1)",
                [first.ordinal],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|_| storage())?
        {
            page.previous_cursor = Some(encode_cursor(session, 'b', first));
        }
    }
    if let Some(last) = entries.last() {
        if connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE ordinal > ?1)",
                [last.ordinal],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|_| storage())?
        {
            page.next_cursor = Some(encode_cursor(session, 'a', last));
        }
    }
    page.items = entries.into_iter().map(|entry| entry.item).collect();
    Ok(page)
}
fn encode_cursor(session: &Session, direction: char, entry: &Entry) -> String {
    format!(
        "{}:{direction}:{}:{}:{:x}",
        session.token,
        entry.ordinal,
        entry.sequence,
        entry.time.to_bits()
    )
}
fn decode_cursor(
    connection: &Connection,
    session: &Session,
    cursor: &str,
) -> Result<(char, u64), StreamError> {
    if cursor.len() > 128 {
        return Err(invalid());
    }
    let fields = cursor.split(':').collect::<Vec<_>>();
    if fields.len() != 5 || fields[0] != session.token || !matches!(fields[1], "a" | "b") {
        return Err(stale());
    }
    let ordinal = fields[2].parse::<u64>().map_err(|_| invalid())?;
    let sequence = fields[3].parse::<u64>().map_err(|_| invalid())?;
    let bits = u64::from_str_radix(fields[4], 16).map_err(|_| invalid())?;
    let actual = connection
        .query_row(
            "SELECT sequence,media_time FROM messages WHERE ordinal=?1",
            [ordinal],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, f64>(1)?)),
        )
        .optional()
        .map_err(|_| storage())?
        .ok_or_else(stale)?;
    if actual.0 != sequence || actual.1.to_bits() != bits {
        return Err(stale());
    }
    Ok((fields[1].chars().next().ok_or_else(invalid)?, ordinal))
}

pub(super) fn buckets(session: &Session, seconds: f64) -> Result<ReplayTimeline, StreamError> {
    let status = session.index.lock().map_err(|_| storage())?.state.clone();
    let mut result = ReplayTimeline {
        bucket_seconds: seconds,
        buckets: vec![],
        viewer_metric_status: "not_recorded".into(),
        index_state: status.clone(),
    };
    if status != "ready" {
        return Ok(result);
    }
    let connection = read_connection(session)?;
    let offset = *session.offset.lock().map_err(|_| storage())?;
    let count = ((session.duration / seconds).ceil() as usize).clamp(1, 2000);
    result.buckets = (0..count)
        .map(|index| ReplayTimelineBucket {
            start_seconds: index as f64 * seconds,
            chat_count: 0,
            unique_sender_count: None,
            viewer_count: None,
        })
        .collect();
    let mut statement = connection.prepare("SELECT CAST((media_time+?1)/?2 AS INTEGER),COUNT(*),COUNT(DISTINCT sender_key),SUM(sender_key IS NULL) FROM messages WHERE media_time+?1 >= 0 AND media_time+?1 <= ?3 GROUP BY 1 ORDER BY 1 LIMIT 2001").map_err(|_| storage())?;
    let mut rows = statement
        .query(params![offset, seconds, session.duration])
        .map_err(|_| storage())?;
    while let Some(row) = rows.next().map_err(|_| storage())? {
        if session.cancel.load(Ordering::Acquire) {
            return Err(stale());
        }
        let bucket: usize = row.get(0).map_err(|_| storage())?;
        if let Some(item) = result.buckets.get_mut(bucket.min(count - 1)) {
            item.chat_count += row.get::<_, u64>(1).map_err(|_| storage())?;
            item.unique_sender_count = if row.get::<_, u64>(3).map_err(|_| storage())? == 0 {
                Some(row.get(2).map_err(|_| storage())?)
            } else {
                None
            };
        }
    }
    Ok(result)
}
