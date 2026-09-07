use std::{
    collections::{BTreeSet, HashMap},
    fs::{self, Metadata, OpenOptions},
    io::{self, Read, Write},
    path::{Component, PathBuf},
    sync::{Mutex, MutexGuard},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::thumbnail::{CancellationToken, ResolvedThumbnail, ThumbnailKey};

const MAGIC: &[u8; 8] = b"ATSUTHM1";
const PREFIX: &str = "atsumi-thumb-v1-";
const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_PAYLOAD_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 50_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThumbnailDiskCacheUsage {
    pub entries: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThumbnailDiskCacheClear {
    pub entries_removed: usize,
    pub bytes_removed: u64,
}

/// A ticket precedes metadata reads and expensive resolution. Clearing or
/// invalidating the cache fences all older writers without retaining tombstones.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CacheTicket(u64);

#[derive(Clone)]
struct Entry {
    bytes: u64,
    touched: SystemTime,
}

struct State {
    loaded: bool,
    generation: u64,
    max_bytes: u64,
    bytes: u64,
    entries: HashMap<String, Entry>,
    lru: BTreeSet<(SystemTime, String)>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    identity: String,
    content_type: String,
    width: u32,
    height: u32,
    source_revision: Option<String>,
    payload_bytes: u64,
}

/// Disposable derived images, scoped to one application data directory. Cache
/// errors never prevent source resolution. Only strictly named, regular cache
/// files are read/deleted; directories, links and foreign files are preserved.
/// The lock covers cache I/O only, never source reads, image codecs or networking.
pub struct ThumbnailDiskCache {
    root: PathBuf,
    state: Mutex<State>,
}

impl ThumbnailDiskCache {
    pub fn new(path: impl Into<PathBuf>, max_bytes: u64) -> Self {
        Self {
            root: path.into(),
            state: Mutex::new(State {
                loaded: false,
                generation: 0,
                max_bytes,
                bytes: 0,
                entries: HashMap::new(),
                lru: BTreeSet::new(),
            }),
        }
    }

    pub fn usage(&self) -> ThumbnailDiskCacheUsage {
        let mut state = self.lock();
        // Initialization is deliberately deferred from application setup to a
        // thumbnail worker or an explicit usage/maintenance request.
        let _ = self.initialize(&mut state);
        ThumbnailDiskCacheUsage {
            entries: state.entries.len(),
            bytes: state.bytes,
        }
    }

    pub fn set_max_bytes(&self, max_bytes: u64) -> io::Result<()> {
        let mut state = self.lock();
        state.max_bytes = max_bytes;
        self.initialize(&mut state)?;
        self.evict(&mut state, 0, 0)
    }

    pub fn clear(&self) -> io::Result<ThumbnailDiskCacheClear> {
        let mut state = self.lock();
        state.generation = state.generation.wrapping_add(1);
        self.initialize(&mut state)?;
        let mut removed = ThumbnailDiskCacheClear::default();
        let mut first_error = None;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_cache_name(&name) && !is_temp_name(&name) {
                continue;
            }
            match self.remove_owned_file(&name) {
                Ok(Some(bytes)) => {
                    if is_cache_name(&name) {
                        removed.entries_removed += 1;
                    }
                    removed.bytes_removed = removed.bytes_removed.saturating_add(bytes);
                    forget(&mut state, &name);
                }
                Ok(None) => {
                    forget(&mut state, &name);
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(removed),
        }
    }

    /// Invalidate every SHA/profile variant of the same logical thumbnail.
    pub fn invalidate(&self, key: &ThumbnailKey) -> io::Result<bool> {
        let mut state = self.lock();
        state.generation = state.generation.wrapping_add(1);
        self.initialize(&mut state)?;
        let prefix = format!("{PREFIX}{}-", digest(key.cache_id().as_bytes()));
        let names = state
            .entries
            .keys()
            .filter(|name| name.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>();
        let mut removed = false;
        let mut first_error = None;
        for name in names {
            match self.remove_owned_file(&name) {
                Ok(bytes) => {
                    removed |= bytes.is_some();
                    forget(&mut state, &name);
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(removed),
        }
    }

    pub(crate) fn ticket(&self) -> CacheTicket {
        CacheTicket(self.lock().generation)
    }

    pub(crate) fn get(
        &self,
        key: &ThumbnailKey,
        profile: &str,
        ticket: CacheTicket,
    ) -> Option<ResolvedThumbnail> {
        let mut state = self.lock();
        if ticket.0 != state.generation || self.initialize(&mut state).is_err() {
            return None;
        }
        let name = cache_name(key, profile);
        if !state.entries.contains_key(&name) {
            return None;
        }
        match self.read(&name) {
            Ok(thumbnail) => {
                if let Some(entry) = state.entries.get(&name).cloned() {
                    state.lru.remove(&(entry.touched, name.clone()));
                    let touched = SystemTime::now();
                    state.lru.insert((touched, name.clone()));
                    state.entries.get_mut(&name).unwrap().touched = touched;
                }
                Some(thumbnail)
            }
            Err(_) => {
                // A truncated, stale-format or corrupt file is disposable.
                if self.remove_owned_file(&name).is_ok() {
                    forget(&mut state, &name);
                }
                None
            }
        }
    }

    pub(crate) fn put(
        &self,
        key: &ThumbnailKey,
        profile: &str,
        ticket: CacheTicket,
        thumbnail: &ResolvedThumbnail,
        cancellation: &CancellationToken,
    ) -> io::Result<()> {
        if cancellation.is_cancelled()
            || thumbnail.validate().is_err()
            || thumbnail.bytes.len() as u64 > MAX_PAYLOAD_BYTES
        {
            return Ok(());
        }
        let name = cache_name(key, profile);
        let header = serde_json::to_vec(&Header {
            identity: name.clone(),
            content_type: thumbnail.content_type.clone(),
            width: thumbnail.width,
            height: thumbnail.height,
            source_revision: thumbnail.source_revision.clone(),
            payload_bytes: thumbnail.bytes.len() as u64,
        })?;
        if header.len() > MAX_HEADER_BYTES {
            return Ok(());
        }
        let header_len = (header.len() as u32).to_le_bytes();
        let mut checksum = Sha256::new();
        checksum.update(MAGIC);
        checksum.update(header_len);
        checksum.update(&header);
        checksum.update(&thumbnail.bytes);
        let checksum = checksum.finalize();
        let bytes = (MAGIC.len() + 4 + header.len() + thumbnail.bytes.len() + 32) as u64;
        let mut state = self.lock();
        if ticket.0 != state.generation || cancellation.is_cancelled() || bytes > state.max_bytes {
            return Ok(());
        }
        self.initialize(&mut state)?;
        if state.entries.contains_key(&name) {
            // Another resolver already published the same source identity.
            return Ok(());
        }
        self.evict(&mut state, bytes, 1)?;
        let temp_name = format!("{PREFIX}{}.tmp", uuid::Uuid::new_v4());
        let temp = self.root.join(&temp_name);
        let destination = self.root.join(&name);
        let result = (|| -> io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(MAGIC)?;
            file.write_all(&header_len)?;
            file.write_all(&header)?;
            file.write_all(&thumbnail.bytes)?;
            file.write_all(&checksum)?;
            file.sync_all()?;
            drop(file);
            if cancellation.is_cancelled() {
                return Ok(());
            }
            self.validate_root()?;
            match fs::symlink_metadata(&destination) {
                Ok(metadata) if regular_file(&metadata) => {
                    // Can occur after an earlier failed index load or external
                    // restoration. Removing a regular owned leaf cannot follow it.
                    fs::remove_file(&destination)?;
                }
                Ok(_) => return Err(unsafe_path()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            fs::rename(&temp, &destination)?;
            let touched = SystemTime::now();
            state.entries.insert(name.clone(), Entry { bytes, touched });
            state.lru.insert((touched, name));
            state.bytes = state.bytes.saturating_add(bytes);
            Ok(())
        })();
        let _ = self.remove_owned_file(&temp_name);
        result
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn initialize(&self, state: &mut State) -> io::Result<()> {
        self.validate_root()?;
        if state.loaded {
            return Ok(());
        }
        fs::create_dir_all(&self.root)?;
        self.validate_root()?;
        state.entries.clear();
        state.lru.clear();
        state.bytes = 0;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_temp_name(&name) {
                let _ = self.remove_owned_file(&name);
                continue;
            }
            if !is_cache_name(&name) {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            if !regular_file(&metadata) {
                continue;
            }
            let bytes = metadata.len();
            let touched = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            state.entries.insert(name.clone(), Entry { bytes, touched });
            state.lru.insert((touched, name));
            state.bytes = state.bytes.saturating_add(bytes);
            // Bound memory while scanning a cache created with older limits.
            self.evict(state, 0, 0)?;
        }
        state.loaded = true;
        Ok(())
    }

    fn evict(&self, state: &mut State, extra_bytes: u64, extra_entries: usize) -> io::Result<()> {
        while state.bytes.saturating_add(extra_bytes) > state.max_bytes
            || state.entries.len().saturating_add(extra_entries) > MAX_ENTRIES
        {
            let Some((_, name)) = state.lru.first().cloned() else {
                break;
            };
            self.remove_owned_file(&name)?;
            forget(state, &name);
        }
        Ok(())
    }

    fn read(&self, name: &str) -> io::Result<ResolvedThumbnail> {
        self.validate_root()?;
        let path = self.root.join(name);
        if !regular_file(&fs::symlink_metadata(&path)?) {
            return Err(unsafe_path());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        #[cfg(windows)]
        let mut file = {
            use std::os::windows::fs::OpenOptionsExt;
            let mut touch_options = options.clone();
            // GENERIC_READ | FILE_WRITE_ATTRIBUTES: permit mtime-only recency
            // updates without opening the payload for modification.
            touch_options.access_mode(0x8000_0100);
            touch_options.open(&path).or_else(|_| options.open(&path))?
        };
        #[cfg(not(windows))]
        let mut file = options.open(&path)?;
        let metadata = file.metadata()?;
        if !regular_file(&metadata)
            || metadata.len() > MAX_PAYLOAD_BYTES + MAX_HEADER_BYTES as u64 + 44
        {
            return Err(invalid_data());
        }
        let mut prefix = [0u8; 12];
        file.read_exact(&mut prefix)?;
        if &prefix[..8] != MAGIC {
            return Err(invalid_data());
        }
        let header_len = u32::from_le_bytes(prefix[8..].try_into().unwrap()) as usize;
        if header_len > MAX_HEADER_BYTES {
            return Err(invalid_data());
        }
        let mut header_bytes = vec![0; header_len];
        file.read_exact(&mut header_bytes)?;
        let header: Header = serde_json::from_slice(&header_bytes).map_err(|_| invalid_data())?;
        if header.identity != name
            || header.payload_bytes == 0
            || header.payload_bytes > MAX_PAYLOAD_BYTES
            || metadata.len() != 12 + header_len as u64 + header.payload_bytes + 32
        {
            return Err(invalid_data());
        }
        let mut bytes = vec![0; header.payload_bytes as usize];
        file.read_exact(&mut bytes)?;
        let mut expected = [0u8; 32];
        file.read_exact(&mut expected)?;
        let mut checksum = Sha256::new();
        checksum.update(prefix);
        checksum.update(header_bytes);
        checksum.update(&bytes);
        if checksum.finalize().as_slice() != expected {
            return Err(invalid_data());
        }
        let thumbnail = ResolvedThumbnail {
            bytes,
            content_type: header.content_type,
            width: header.width,
            height: header.height,
            source_revision: header.source_revision,
        };
        thumbnail.validate().map_err(|_| invalid_data())?;
        // Persist LRU recency after checksum validation through this already
        // opened regular-file handle. Read-only caches still serve valid hits.
        let _ = file.set_modified(SystemTime::now());
        Ok(thumbnail)
    }

    fn validate_root(&self) -> io::Result<()> {
        if !self.root.is_absolute()
            || self.root.file_name().is_none()
            || self
                .root
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(unsafe_path());
        }
        for ancestor in self.root.ancestors() {
            match fs::symlink_metadata(ancestor) {
                Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {}
                Ok(_) => return Err(unsafe_path()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn remove_owned_file(&self, name: &str) -> io::Result<Option<u64>> {
        if !is_cache_name(name) && !is_temp_name(name) {
            return Err(unsafe_path());
        }
        self.validate_root()?;
        let path = self.root.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if regular_file(&metadata) => {
                fs::remove_file(path)?;
                Ok(Some(metadata.len()))
            }
            // Never traverse or delete user directories, junctions or links.
            Ok(_) => Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn forget(state: &mut State, name: &str) {
    if let Some(entry) = state.entries.remove(name) {
        state.bytes = state.bytes.saturating_sub(entry.bytes);
        state.lru.remove(&(entry.touched, name.to_owned()));
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn cache_name(key: &ThumbnailKey, profile: &str) -> String {
    format!(
        "{PREFIX}{}-{}.bin",
        digest(key.cache_id().as_bytes()),
        digest(profile.as_bytes()),
    )
}

fn is_cache_name(name: &str) -> bool {
    let Some(body) = name
        .strip_prefix(PREFIX)
        .and_then(|name| name.strip_suffix(".bin"))
    else {
        return false;
    };
    body.len() == 129
        && body.as_bytes()[64] == b'-'
        && body.bytes().enumerate().all(|(index, byte)| {
            index == 64 || byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
        })
}

fn is_temp_name(name: &str) -> bool {
    name.strip_prefix(PREFIX)
        .and_then(|name| name.strip_suffix(".tmp"))
        .is_some_and(|name| name.len() == 36 && uuid::Uuid::parse_str(name).is_ok())
}

fn regular_file(metadata: &Metadata) -> bool {
    metadata.is_file() && !is_link(metadata)
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT includes junctions as well as symlinks.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn unsafe_path() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "thumbnail cache path is not a regular managed path",
    )
}

fn invalid_data() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "thumbnail cache entry is invalid",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thumbnail(value: u8) -> ResolvedThumbnail {
        ResolvedThumbnail {
            content_type: "image/webp".into(),
            bytes: vec![value; 128],
            width: 32,
            height: 24,
            source_revision: Some("verified-source".into()),
        }
    }

    fn put(cache: &ThumbnailDiskCache, key: &ThumbnailKey, profile: &str, value: u8) {
        cache
            .put(
                key,
                profile,
                cache.ticket(),
                &thumbnail(value),
                &CancellationToken::new(),
            )
            .unwrap();
    }

    #[test]
    fn constructor_defers_filesystem_access_until_worker_or_explicit_usage() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        let cache = ThumbnailDiskCache::new(&root, 4096);
        assert!(!root.exists());
        assert_eq!(cache.usage().entries, 0);
        assert!(root.is_dir());
    }

    #[test]
    fn restart_eviction_preserves_the_most_recently_read_entry() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        let cache = ThumbnailDiskCache::new(&root, 8192);
        let first = ThumbnailKey::gallery_cover(1).unwrap();
        let second = ThumbnailKey::gallery_cover(2).unwrap();
        put(&cache, &first, "remote-v1", 1);
        let entry_bytes = cache.usage().bytes;
        put(&cache, &second, "remote-v1", 2);
        for (seconds, key) in [(1, &first), (2, &second)] {
            OpenOptions::new()
                .write(true)
                .open(root.join(cache_name(key, "remote-v1")))
                .unwrap()
                .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
                .unwrap();
        }
        assert!(cache.get(&first, "remote-v1", cache.ticket()).is_some());
        drop(cache);
        let cache = ThumbnailDiskCache::new(&root, entry_bytes);
        assert!(cache.get(&first, "remote-v1", cache.ticket()).is_some());
        assert!(cache.get(&second, "remote-v1", cache.ticket()).is_none());
        assert_eq!(cache.usage().entries, 1);
    }

    #[test]
    fn restart_retains_bytes_dimensions_and_revision_but_sha_changes_miss() {
        let temp = tempfile::tempdir().unwrap();
        let key = ThumbnailKey::artifact_page("entry", 1).unwrap();
        let root = temp.path().join("cache");
        let cache = ThumbnailDiskCache::new(&root, 4096);
        put(&cache, &key, "local-v1:sha-a", 42);
        drop(cache);
        let cache = ThumbnailDiskCache::new(&root, 4096);
        assert_eq!(
            cache.get(&key, "local-v1:sha-a", cache.ticket()),
            Some(thumbnail(42))
        );
        assert_eq!(cache.get(&key, "local-v1:sha-b", cache.ticket()), None);
    }

    #[test]
    fn corruption_or_truncation_is_a_miss_and_can_be_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let cache = ThumbnailDiskCache::new(temp.path().join("cache"), 4096);
        let key = ThumbnailKey::gallery_cover(100).unwrap();
        for truncated in [false, true] {
            put(&cache, &key, "remote-v1", 1);
            let path = cache.root.join(cache_name(&key, "remote-v1"));
            let mut bytes = fs::read(&path).unwrap();
            if truncated {
                bytes.truncate(10);
            } else {
                *bytes.last_mut().unwrap() ^= 1;
            }
            fs::write(&path, bytes).unwrap();
            assert_eq!(cache.get(&key, "remote-v1", cache.ticket()), None);
            put(&cache, &key, "remote-v1", 2);
            assert_eq!(
                cache.get(&key, "remote-v1", cache.ticket()),
                Some(thumbnail(2))
            );
            cache.clear().unwrap();
        }
    }

    #[test]
    fn lru_limit_updates_and_clear_preserve_foreign_files_and_directories() {
        let temp = tempfile::tempdir().unwrap();
        let cache = ThumbnailDiskCache::new(temp.path().join("cache"), 8192);
        let keys = (1..=3)
            .map(|id| ThumbnailKey::gallery_cover(id).unwrap())
            .collect::<Vec<_>>();
        put(&cache, &keys[0], "remote-v1", 1);
        let entry_bytes = cache.usage().bytes;
        cache.set_max_bytes(entry_bytes * 2).unwrap();
        put(&cache, &keys[1], "remote-v1", 2);
        assert!(cache.get(&keys[0], "remote-v1", cache.ticket()).is_some());
        put(&cache, &keys[2], "remote-v1", 3);
        assert!(cache.get(&keys[1], "remote-v1", cache.ticket()).is_none());
        assert_eq!(cache.usage().entries, 2);
        assert!(cache.usage().bytes <= entry_bytes * 2);
        cache.set_max_bytes(entry_bytes).unwrap();
        assert_eq!(cache.usage().entries, 1);
        fs::write(cache.root.join("notes.txt"), b"user file").unwrap();
        let directory = cache.root.join(cache_name(&keys[1], "remote-v1"));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("keep.txt"), b"user directory").unwrap();
        let removed = cache.clear().unwrap();
        assert_eq!(removed.entries_removed, 1);
        assert_eq!(cache.usage().bytes, 0);
        assert_eq!(
            fs::read(cache.root.join("notes.txt")).unwrap(),
            b"user file"
        );
        assert!(directory.join("keep.txt").exists());
    }

    #[test]
    fn invalidate_covers_sha_variants_and_fences_old_writers_and_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let cache = ThumbnailDiskCache::new(temp.path().join("cache"), 8192);
        let key = ThumbnailKey::artifact_page("entry", 3).unwrap();
        put(&cache, &key, "local-v1:sha-a", 1);
        put(&cache, &key, "local-v1:sha-b", 2);
        let old = cache.ticket();
        assert!(cache.invalidate(&key).unwrap());
        assert_eq!(cache.usage().entries, 0);
        cache
            .put(
                &key,
                "local-v1:sha-a",
                old,
                &thumbnail(1),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(cache.usage().entries, 0);
        let old = cache.ticket();
        cache.clear().unwrap();
        cache
            .put(
                &key,
                "local-v1:sha-a",
                old,
                &thumbnail(1),
                &CancellationToken::new(),
            )
            .unwrap();
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        cache
            .put(
                &key,
                "local-v1:sha-a",
                cache.ticket(),
                &thumbnail(1),
                &cancelled,
            )
            .unwrap();
        assert_eq!(cache.usage().entries, 0);
    }

    #[test]
    fn unavailable_or_parent_traversing_cache_is_a_miss() {
        let temp = tempfile::tempdir().unwrap();
        let occupied = temp.path().join("occupied");
        fs::write(&occupied, b"preserve").unwrap();
        let key = ThumbnailKey::gallery_cover(1).unwrap();
        for path in [occupied.clone(), temp.path().join("nested/../unsafe")] {
            let cache = ThumbnailDiskCache::new(path, 8192);
            assert!(cache.get(&key, "remote-v1", cache.ticket()).is_none());
            assert!(cache
                .put(
                    &key,
                    "remote-v1",
                    cache.ticket(),
                    &thumbnail(1),
                    &CancellationToken::new()
                )
                .is_err());
        }
        assert_eq!(fs::read(occupied).unwrap(), b"preserve");
    }

    #[cfg(windows)]
    #[test]
    fn symlink_cache_root_and_owned_leaf_never_modify_the_target() {
        use std::os::windows::fs::{symlink_dir, symlink_file};
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let linked_root = temp.path().join("linked");
        if symlink_dir(outside.path(), &linked_root).is_err() {
            return;
        }
        let cache = ThumbnailDiskCache::new(linked_root, 8192);
        assert!(cache.clear().is_err());
        let key = ThumbnailKey::gallery_cover(1).unwrap();
        assert!(cache
            .put(
                &key,
                "remote-v1",
                cache.ticket(),
                &thumbnail(1),
                &CancellationToken::new()
            )
            .is_err());
        let target = outside.path().join("private-file");
        fs::write(&target, b"preserve").unwrap();
        let cache = ThumbnailDiskCache::new(temp.path().join("cache"), 8192);
        let link = cache.root.join(cache_name(&key, "remote-v1"));
        fs::create_dir(&cache.root).unwrap();
        symlink_file(&target, &link).unwrap();
        cache.clear().unwrap();
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(target).unwrap(), b"preserve");
    }
}
