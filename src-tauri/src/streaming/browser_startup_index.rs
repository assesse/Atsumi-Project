//! SSD-local, disposable last-known summaries. Never authorize playback or file
//! mutation from this index: invalidate durably, then recover that recording's
//! authoritative journal before any operation. Pending jobs must recover first.
use super::*;

const NAME: &str = "startup-index.json";
const LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    recordings: Vec<BrowserRecording>,
}

pub(super) fn settled(r: &BrowserRecording) -> bool {
    if r.media_removed_at.is_some() {
        return r.status != BrowserRecordingStatus::Recording && !r.deletion_pending;
    }
    r.status != BrowserRecordingStatus::Recording
        && !r.deletion_pending
        && r.partial.is_none()
        && r.ending.as_ref().is_none_or(|e| e.reason != "checking")
        && r.progressive
            .as_ref()
            .is_none_or(|p| p.segment_count == r.segment_count || p.last_error.is_some())
        && if r.segment_count == 0 {
            r.bytes_written == 0 && r.merge.is_none()
        } else {
            r.merge.as_ref().is_some_and(|m| {
                m.status == BrowserMergeStatus::Complete
                    && m.source_cleanup
                        .as_ref()
                        .is_none_or(|c| c.status == BrowserSourceCleanupStatus::Complete)
            })
        }
}

fn matches_entry(r: &BrowserRecording, e: &CatalogEntry) -> bool {
    !e.deletion_pending
        && r.id == e.id
        && r.channel_id == e.channel_id
        && r.title == e.title
        && r.started_at == e.started_at
        && r.mime_type == e.mime_type
        && r.output_dir == e.output_dir
        && r.media_removed_at == e.media_removed_at
        && r.segment_count <= MAX_SEGMENTS
        && r.segments.len() <= MAX_RECENT
        && r.duration_seconds.is_finite()
        && r.duration_seconds >= 0.0
        && (r.media_removed_at.is_some()
            || valid_merge(r.merge.as_ref(), r.segment_count, &r.mime_type))
        && valid_chat_summary(r.capture_chat, r.chat_status.as_deref(), r.chat_count)
        && r.status != BrowserRecordingStatus::Recording
        && !r.summary_pending
}

fn read(root: &Path) -> Result<Index, StreamError> {
    let index: Index =
        serde_json::from_reader(BufReader::new(open_regular(&root.join(NAME), LIMIT)?))
            .map_err(|_| invalid())?;
    if index.version != 1 || index.recordings.len() > MAX_RECORDINGS {
        return Err(invalid());
    }
    Ok(index)
}

/// First upgrade only: a bounded summary, never the segment/media journal.
pub(super) fn display_summary(entry: &CatalogEntry) -> Option<BrowserRecording> {
    let root = owned_root(&entry.output_dir, &entry.id).ok()?;
    let file = open_regular(&root.join("recording.json"), MAX_METADATA).ok()?;
    let mut r: BrowserRecording = serde_json::from_reader(BufReader::new(file)).ok()?;
    r.media_removed_at = entry.media_removed_at;
    if r.status == BrowserRecordingStatus::Recording {
        r.status = BrowserRecordingStatus::Interrupted;
    }
    if !matches_entry(&r, entry) {
        return None;
    }
    r.storage_check_pending = true;
    r.summary_pending = false;
    Some(r)
}

pub(super) fn load(root: &Path, catalog: &[CatalogEntry]) -> HashMap<String, BrowserRecording> {
    let Ok(index) = read(root) else {
        return HashMap::new();
    };
    index
        .recordings
        .into_iter()
        .filter_map(|mut r| {
            let e = catalog.iter().find(|e| e.id == r.id)?;
            if !matches_entry(&r, e) {
                return None;
            }
            r.storage_check_pending = true;
            Some((r.id.clone(), r))
        })
        .collect()
}

fn write(root: &Path, recordings: Vec<BrowserRecording>) -> Result<(), StreamError> {
    let bytes = serde_json::to_vec(&Index {
        version: 1,
        recordings,
    })
    .map_err(|_| storage())?;
    if bytes.len() as u64 > LIMIT {
        return Err(storage());
    }
    atomic_write(root, NAME, &bytes)
}

impl BrowserCaptureStore {
    /// Serialize cache publication/invalidation; never hold the main store lock
    /// across archive I/O. Repeated concurrent requests recover an ID only once.
    pub(super) fn ensure_recovered(&self, id: &str) -> Result<(), StreamError> {
        let _gate = self.index_gate.lock().map_err(|_| storage())?;
        let (entry, recover) = {
            let state = self.lock()?;
            if !state.indexed.contains(id) && !state.unverified.contains(id) {
                return Ok(());
            }
            (
                state
                    .catalog
                    .iter()
                    .find(|e| e.id == id)
                    .cloned()
                    .ok_or_else(invalid)?,
                state.unverified.contains(id),
            )
        };
        // A crash after the next operation must not resurrect an older summary.
        if self.lock()?.indexed.contains(id) {
            let mut index = read(&self.catalog_root).unwrap_or(Index {
                version: 1,
                recordings: vec![],
            });
            index.recordings.retain(|r| r.id != id);
            write(&self.catalog_root, index.recordings)?;
        }
        let recording = recover.then(|| recover_recording(&entry));
        let mut state = self.lock()?;
        state.indexed.remove(id);
        state.recovery_pending.retain(|pending| pending != id);
        if state.unverified.remove(id) {
            let target = state
                .recordings
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or_else(invalid)?;
            *target = recording.ok_or_else(invalid)?;
        }
        Ok(())
    }

    pub(super) fn save_startup_index(&self) -> Result<(), StreamError> {
        let _gate = self.index_gate.lock().map_err(|_| storage())?;
        let recordings: Vec<_> = {
            let state = self.lock()?;
            state
                .recordings
                .iter()
                .filter(|r| {
                    !state.active.contains_key(&r.id)
                        && (!state.unverified.contains(&r.id) || state.indexed.contains(&r.id))
                        && !state.deleting.contains_key(&r.id)
                        && !state.merge_job.as_ref().is_some_and(|(id, _)| id == &r.id)
                        && state.catalog.iter().any(|e| matches_entry(r, e))
                })
                .cloned()
                .collect()
        };
        let indexed = recordings.iter().map(|r| r.id.clone()).collect();
        write(&self.catalog_root, recordings)?;
        self.lock()?.indexed = indexed;
        Ok(())
    }

    /// One archive at a time, outside both the host's and the store's state lock.
    /// Failed validation rotates the entry so an unavailable disk cannot starve
    /// the rest. Shutdown joins the owning worker before store replacement.
    pub(crate) fn recover_next_pending(&self) -> Result<Option<String>, StreamError> {
        let id = {
            let mut state = self.lock()?;
            if state.closing {
                return Ok(None);
            }
            let Some(id) = state.recovery_pending.pop_front() else {
                return Ok(None);
            };
            state.recovery_pending.push_back(id.clone());
            id
        };
        self.ensure_recovered(&id)?;
        Ok(Some(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "0123456789abcdef0123456789abcdef";
    const WEBM: &[u8] = &[0x1a, 0x45, 0xdf, 0xa3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

    fn fixture(segments: bool) -> (tempfile::TempDir, BrowserCaptureStore, BrowserRecording) {
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let r = store
            .begin(dir.path(), CHANNEL, "recording", "video/webm")
            .unwrap();
        if segments {
            store.append(&r.id, 0, 0, WEBM).unwrap();
            store.finish_segment(&r.id, 0, 15.0).unwrap();
        }
        store.finish(&r.id, false, None).unwrap();
        (dir, store, r)
    }

    #[test]
    fn cached_history_does_not_touch_archive_but_playback_revalidates_it() {
        let (dir, store, r) = fixture(true);
        let job = store.take_merge_job().unwrap().unwrap();
        let root = Path::new(&r.output_dir);
        let file = format!("merged-{}.webm", job.token);
        let timeline_file = format!("merged-{}.timeline.jsonl", job.token);
        fs::write(root.join(&file), WEBM).unwrap();
        fs::write(root.join(&timeline_file), b"{}\n").unwrap();
        store
            .complete_merge(
                &job,
                BrowserMergedOutput {
                    file,
                    timeline_file,
                    bytes: WEBM.len() as u64,
                    duration_seconds: 15.0,
                    cleanup: None,
                },
            )
            .unwrap();
        store.shutdown().unwrap();
        // An unavailable disk must not be probed merely to display history.
        fs::rename(root, root.with_extension("offline")).unwrap();
        let reopened = BrowserCaptureStore::new(dir.path()).unwrap();
        let snapshot = reopened.snapshot().unwrap();
        assert!(snapshot[0].storage_check_pending);
        assert_eq!(snapshot[0].status, BrowserRecordingStatus::Stopped);
        assert!(reopened.take_merge_job().unwrap().is_none());
        assert!(reopened.take_part_job().unwrap().is_none());
        assert!(reopened.take_cleanup_job().unwrap().is_none());
        assert!(reopened.merged_file(&r.id).is_err());
        assert!(!reopened.snapshot().unwrap()[0].storage_check_pending);
        assert!(read(&reopened.catalog_root).unwrap().recordings.is_empty());
    }

    #[test]
    fn mutation_invalidates_the_old_snapshot_before_a_crash() {
        let (dir, store, r) = fixture(false);
        store.shutdown().unwrap();
        let reopened = BrowserCaptureStore::new(dir.path()).unwrap();
        assert!(reopened.snapshot().unwrap()[0].storage_check_pending);
        reopened.record_end(&r.id, "user_stopped", "test").unwrap();
        drop(reopened); // No graceful shutdown/cache save.
        let after_crash = BrowserCaptureStore::new(dir.path()).unwrap();
        let snapshot = after_crash.snapshot().unwrap();
        assert!(!snapshot[0].storage_check_pending);
        assert_eq!(snapshot[0].ending.as_ref().unwrap().reason, "user_stopped");
    }

    #[test]
    fn post_shutdown_metadata_update_cannot_leave_a_stale_index() {
        let (dir, store, r) = fixture(false);
        store.shutdown().unwrap();
        store
            .record_end(&r.id, "app_shutdown", "late-finalizer")
            .unwrap();
        let reopened = BrowserCaptureStore::new(dir.path()).unwrap();
        assert!(!reopened.snapshot().unwrap()[0].storage_check_pending);
        assert_eq!(
            reopened.snapshot().unwrap()[0]
                .ending
                .as_ref()
                .unwrap()
                .trigger,
            "late-finalizer"
        );
    }

    #[test]
    fn journal_recovery_is_kept_for_pending_merges_and_partials() {
        let (dir, store, r) = fixture(true);
        store.shutdown().unwrap();
        let reopened = BrowserCaptureStore::new(dir.path()).unwrap();
        assert!(!reopened.snapshot().unwrap()[0].storage_check_pending);
        assert_eq!(
            reopened.take_merge_job().unwrap().unwrap().recording.id,
            r.id
        );
        let other = tempfile::tempdir().unwrap();
        let active = BrowserCaptureStore::new(other.path()).unwrap();
        let partial = active
            .begin(other.path(), CHANNEL, "partial", "video/webm")
            .unwrap();
        active.append(&partial.id, 0, 0, WEBM).unwrap();
        active.shutdown().unwrap();
        let recovered = BrowserCaptureStore::new(other.path())
            .unwrap()
            .snapshot()
            .unwrap();
        assert!(!recovered[0].storage_check_pending);
        assert!(recovered[0].partial.is_some());
    }

    #[test]
    fn corrupted_or_wrong_identity_index_falls_back_to_the_journal() {
        let (dir, store, r) = fixture(false);
        store.shutdown().unwrap();
        let mut cache = read(&store.catalog_root).unwrap();
        cache.recordings[0].output_dir = dir.path().to_string_lossy().into_owned();
        write(&store.catalog_root, cache.recordings).unwrap();
        let actual = BrowserCaptureStore::new(dir.path())
            .unwrap()
            .snapshot()
            .unwrap();
        assert_eq!(actual[0].output_dir, r.output_dir);
        assert!(!actual[0].storage_check_pending);
        fs::write(store.catalog_root.join(NAME), b"not-json").unwrap();
        let actual = BrowserCaptureStore::new(dir.path())
            .unwrap()
            .snapshot()
            .unwrap();
        assert_eq!(actual[0].status, BrowserRecordingStatus::Stopped);
        assert!(!actual[0].storage_check_pending);
    }

    #[test]
    fn failed_durable_invalidation_does_not_modify_metadata() {
        let (dir, store, r) = fixture(false);
        store.shutdown().unwrap();
        let reopened = BrowserCaptureStore::new(dir.path()).unwrap();
        let before = fs::read(Path::new(&r.output_dir).join("recording.json")).unwrap();
        let path = reopened.catalog_root.join(NAME);
        fs::rename(&path, path.with_extension("backup")).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(reopened
            .record_end(&r.id, "user_stopped", "must-not-write")
            .is_err());
        assert_eq!(
            before,
            fs::read(Path::new(&r.output_dir).join("recording.json")).unwrap()
        );
        assert!(reopened.snapshot().unwrap()[0].storage_check_pending);
    }

    #[test]
    fn deferred_pending_history_is_recovered_before_background_merge() {
        let (dir, store, r) = fixture(true);
        store.shutdown().unwrap();
        let reopened = BrowserCaptureStore::new_deferred(dir.path()).unwrap();
        assert!(reopened.snapshot().unwrap()[0].storage_check_pending);
        assert_eq!(reopened.retry_loaded_merges(None).unwrap(), 0);
        assert!(reopened.take_merge_job().unwrap().is_none());
        assert_eq!(reopened.recover_next_pending().unwrap(), Some(r.id.clone()));
        assert!(!reopened.snapshot().unwrap()[0].storage_check_pending);
        assert!(reopened.recover_next_pending().unwrap().is_none());
        assert_eq!(
            reopened.take_merge_job().unwrap().unwrap().recording.id,
            r.id
        );
    }

    #[test]
    fn first_upgrade_reads_only_summary_and_on_demand_action_checks_the_journal() {
        let (dir, store, r) = fixture(true);
        drop(store); // Legacy catalog with no SSD index.
        let journal = Path::new(&r.output_dir).join("segments.jsonl");
        fs::write(&journal, b"broken journal").unwrap();
        let reopened = BrowserCaptureStore::new_deferred(dir.path()).unwrap();
        let before = reopened.snapshot().unwrap();
        assert_eq!(before[0].segment_count, 1); // Last-known metadata, not a replay authorization.
        assert!(before[0].storage_check_pending);
        assert!(reopened.merged_file(&r.id).is_err());
        let after = reopened.snapshot().unwrap();
        assert!(!after[0].storage_check_pending);
        assert_eq!(after[0].status, BrowserRecordingStatus::Failed);
    }

    #[test]
    fn unavailable_summary_is_not_persisted_as_empty_recording_or_lost_on_early_quit() {
        let (dir, store, r) = fixture(true);
        drop(store);
        let root = Path::new(&r.output_dir);
        let offline = root.with_extension("offline");
        fs::rename(root, &offline).unwrap();
        let unopened = BrowserCaptureStore::new_deferred(dir.path()).unwrap();
        assert!(unopened.snapshot().unwrap()[0].summary_pending);
        unopened.shutdown().unwrap();
        assert!(read(&unopened.catalog_root).unwrap().recordings.is_empty());
        fs::rename(&offline, root).unwrap();
        let restored = BrowserCaptureStore::new_deferred(dir.path()).unwrap();
        assert!(!restored.snapshot().unwrap()[0].summary_pending);
        assert_eq!(restored.snapshot().unwrap()[0].segment_count, 1);
    }

    #[test]
    fn new_capture_does_not_wait_for_historical_recovery() {
        let (dir, store, _) = fixture(true);
        store.shutdown().unwrap();
        let reopened = BrowserCaptureStore::new_deferred(dir.path()).unwrap();
        let new = reopened
            .begin(dir.path(), CHANNEL, "new", "video/webm")
            .unwrap();
        reopened.append(&new.id, 0, 0, WEBM).unwrap();
        reopened.finish_segment(&new.id, 0, 15.0).unwrap();
        reopened.finish(&new.id, false, None).unwrap();
        assert_eq!(
            reopened.take_merge_job().unwrap().unwrap().recording.id,
            new.id
        );
    }
}
