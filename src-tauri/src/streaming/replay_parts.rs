//! Immutable snapshots let replay read completed ranges while capture appends.
use super::*;
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayPart {
    pub index: usize,
    pub start_seconds: f64,
    pub duration_seconds: f64,
}
pub(super) struct PartMedia {
    pub descriptor: ReplayPart,
    pub file: Mutex<File>,
    pub stamp: FileStamp,
}
#[derive(Default)]
pub(super) struct TemporaryFiles(pub Vec<PathBuf>);
impl Drop for TemporaryFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}
pub(super) fn snapshot_bytes(
    root: &Path,
    token: &str,
    kind: &str,
    bytes: &[u8],
    paths: &mut Vec<PathBuf>,
) -> Result<File, StreamError> {
    let name = format!("prefix-{token}-{kind}.jsonl");
    super::super::browser_store::atomic_write(root, &name, bytes)?;
    let path = root.join(name);
    paths.push(path.clone());
    let file = File::open(&path).map_err(|_| storage())?;
    Ok(file)
}
pub(super) fn snapshot_file(
    root: &Path,
    token: &str,
    kind: &str,
    mut source: File,
    paths: &mut Vec<PathBuf>,
) -> Result<File, StreamError> {
    let length = source.metadata().map_err(|_| storage())?.len();
    if length > 512 * 1024 * 1024 {
        return Err(storage());
    }
    source.seek(SeekFrom::Start(0)).map_err(|_| storage())?;
    let path = root.join(format!("prefix-{token}-{kind}.jsonl"));
    let mut target = BufWriter::new(
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|_| storage())?,
    );
    paths.push(path.clone());
    let mut reader = BufReader::new(source.take(length));
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader
            .by_ref()
            .take((MAX_LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|_| storage())?
            == 0
        {
            break;
        }
        if line.len() > MAX_LINE_BYTES {
            return Err(storage());
        }
        if line.last() != Some(&b'\n') {
            break;
        }
        target.write_all(&line).map_err(|_| storage())?;
    }
    target
        .flush()
        .and_then(|()| target.get_ref().sync_all())
        .map_err(|_| storage())?;
    drop(target);
    File::open(path).map_err(|_| storage())
}
