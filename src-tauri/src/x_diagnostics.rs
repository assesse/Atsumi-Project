//! Process-local, debug-only structure diagnostics. Callers supply only their
//! existing bounded shapes, never credentials, page content or raw IPC values.
use serde_json::Value;

#[cfg(any(debug_assertions, test))]
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::Mutex,
};

#[cfg(any(debug_assertions, test))]
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
#[cfg(any(debug_assertions, test))]
static APPEND_LOCK: Mutex<()> = Mutex::new(());

pub(super) fn record(feed: &str, status: &str, shape: &Value) {
    #[cfg(debug_assertions)]
    {
        #[derive(serde::Serialize)]
        struct Record<'a> {
            at: u128,
            feed: &'a str,
            status: &'a str,
            shape: &'a Value,
        }
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../.runtime/x-cleanroom-observations-{}.jsonl",
            std::process::id()
        ));
        let value = Record {
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            feed: feed.split(':').next().unwrap_or("unknown"),
            status,
            shape,
        };
        // A diagnostics failure must not affect playback or a confirmed action.
        let _ = append_record(&path, &value, MAX_LOG_BYTES);
    }
    #[cfg(not(debug_assertions))]
    let _ = (feed, status, shape);
}

#[cfg(any(debug_assertions, test))]
struct BoundedLine {
    bytes: Vec<u8>,
    limit: usize,
    overflowed: bool,
}

#[cfg(any(debug_assertions, test))]
impl Write for BoundedLine {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .map_or(true, |size| size > self.limit)
        {
            self.overflowed = true;
            return Err(io::Error::new(io::ErrorKind::InvalidData, "diagnostic cap"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(any(debug_assertions, test))]
fn append_record(path: &Path, value: &impl serde::Serialize, cap: u64) -> io::Result<bool> {
    if cap == 0 {
        return Ok(false);
    }
    // Serialize before acquiring the writer guard. Value's Display formatter
    // performs many writes, so writeln!(file, "{value}") is not a record boundary.
    let mut line = BoundedLine {
        bytes: Vec::new(),
        limit: usize::try_from(cap - 1).unwrap_or(usize::MAX),
        overflowed: false,
    };
    if let Err(error) = serde_json::to_writer(&mut line, value) {
        if line.overflowed {
            return Ok(false);
        }
        return Err(io::Error::new(io::ErrorKind::InvalidData, error));
    }
    line.bytes.push(b'\n');

    // The cap check and every partial write_all write belong to the same record.
    // All reader/network callbacks in this process share this exact guard.
    let _guard = APPEND_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let length = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    if length
        .checked_add(line.bytes.len() as u64)
        .map_or(true, |size| size > cap)
    {
        return Ok(false);
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&line.bytes)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{collections::HashSet, sync::Arc, sync::Barrier};

    #[test]
    fn concurrent_records_are_complete_independent_json_lines() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("concurrent.jsonl");
        let start = Arc::new(Barrier::new(9));
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let path = &path;
                let start = Arc::clone(&start);
                scope.spawn(move || {
                    start.wait();
                    for sequence in 0..75 {
                        let row = json!({"worker":worker,"sequence":sequence,"shape":{"ready":true,"counts":[1,2,3]}});
                        assert!(append_record(path, &row, MAX_LOG_BYTES).unwrap());
                    }
                });
            }
            start.wait();
        });
        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.ends_with('\n'));
        let rows: Vec<Value> = contents.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(rows.len(), 600);
        let unique: HashSet<(u64, u64)> = rows.iter().map(|row| (row["worker"].as_u64().unwrap(), row["sequence"].as_u64().unwrap())).collect();
        assert_eq!(unique.len(), 600);
    }

    #[test]
    fn unicode_quotes_and_embedded_newlines_remain_one_physical_line() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("escaped.jsonl");
        let row = json!({"marker":"회귀 \"따옴표\"\n두 줄\\끝"});
        assert!(append_record(&path, &row, MAX_LOG_BYTES).unwrap());
        let contents = fs::read_to_string(path).unwrap();
        assert_eq!(contents.lines().count(), 1);
        assert_eq!(serde_json::from_str::<Value>(&contents).unwrap(), row);
    }

    #[test]
    fn exact_cap_includes_newline_and_never_overwrites_existing_rows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cap.jsonl");
        let row = json!({"count":1});
        let one_line = serde_json::to_vec(&row).unwrap().len() as u64 + 1;
        assert!(append_record(&path, &row, one_line).unwrap());
        let original = fs::read(&path).unwrap();
        assert!(!append_record(&path, &row, one_line).unwrap());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(original.len() as u64, one_line);
    }

    #[test]
    fn concurrent_cap_checks_do_not_overrun_the_shared_budget() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("concurrent-cap.jsonl");
        let row = json!({"count":1});
        let one_line = serde_json::to_vec(&row).unwrap().len() as u64 + 1;
        std::thread::scope(|scope| {
            for _ in 0..16 {
                let path = &path;
                let row = &row;
                scope.spawn(move || { append_record(path, row, 3 * one_line).unwrap(); });
            }
        });
        let contents = fs::read_to_string(path).unwrap();
        assert_eq!(contents.len() as u64, 3 * one_line);
        assert_eq!(contents.lines().count(), 3);
        assert!(contents.lines().all(|line| serde_json::from_str::<Value>(line).is_ok()));
    }

    #[test]
    fn oversized_records_and_old_oversized_logs_are_retained_without_partial_append() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.jsonl");
        let original = b"retained historical diagnostics\n";
        fs::write(&path, original).unwrap();
        assert!(!append_record(&path, &json!({"n":1}), 8).unwrap());
        assert_eq!(fs::read(&path).unwrap(), original);
        let new_path = directory.path().join("new.jsonl");
        assert!(!append_record(&new_path, &json!({"marker":"x".repeat(2048)}), 64).unwrap());
        assert!(!new_path.exists());
        assert!(!append_record(&new_path, &json!({}), 0).unwrap());
    }

    #[test]
    fn unavailable_directory_returns_an_error_without_creating_it() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("absent");
        assert!(append_record(&parent.join("events.jsonl"), &json!({}), MAX_LOG_BYTES).is_err());
        assert!(!parent.exists());
    }
}
