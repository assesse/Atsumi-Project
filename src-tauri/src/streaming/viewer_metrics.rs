//! Optional, bounded public live-count observations. Legacy DOM samples remain
//! readable; new recordings query only official public metadata, never media.
use super::model::{ChatReplayClock, StreamError};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};

pub(crate) const FILE_NAME: &str = "viewer-metrics.jsonl";
pub(crate) const MAX_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const SOURCE: &str = "chzzk_video_info_dom_v1";
pub(crate) const API_SOURCE: &str = "chzzk_live_status_api_v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ViewerSample {
    pub version: u8,
    pub source: String,
    pub received_at: u64,
    pub offset_seconds: f64,
    pub viewer_count: Option<u64>,
    pub replay_clock: ChatReplayClock,
}
impl ViewerSample {
    pub fn valid(&self) -> bool {
        self.version == 1
            && matches!(self.source.as_str(), SOURCE | API_SOURCE)
            && self.received_at <= 9_007_199_254_740_991
            && self.offset_seconds.is_finite()
            && (0.0..=1e9).contains(&self.offset_seconds)
            && self.viewer_count.is_none_or(|count| count <= 100_000_000)
            && self.replay_clock.clone().bounded().is_some()
            && self.received_at == self.replay_clock.received_at_ms
    }

    /// A bounded sample-and-hold, not an assertion of continuous observations.
    /// 10s API polls get 5s scheduling grace; legacy 2s DOM samples retain 6s.
    pub fn hold_seconds(&self, mapped_to_media: bool) -> f64 {
        if self.source == API_SOURCE {
            15.0 * if mapped_to_media {
                self.replay_clock.playback_rate.unwrap_or(1.0)
            } else {
                1.0
            }
        } else {
            6.0
        }
    }

    pub fn from_page(
        value: &serde_json::Value,
        clock: ChatReplayClock,
        recording_started_at: u64,
        broadcast_started_at: Option<u64>,
        channel: &str,
    ) -> Option<Self> {
        let source = match value.get("viewerSource") {
            None => SOURCE,
            Some(value) => value.as_str()?,
        };
        if !matches!(source, SOURCE | API_SOURCE) {
            return None;
        }
        let supplied = value.get("viewerCount")?;
        let mut count = if supplied.is_null() {
            None
        } else {
            Some(supplied.as_u64()?)
        };
        if count.is_some_and(|count| count > 100_000_000) {
            return None;
        }
        if source == API_SOURCE {
            if value
                .get("viewerChannelId")
                .and_then(serde_json::Value::as_str)
                != Some(channel)
            {
                return None;
            }
            let opened = value
                .get("viewerBroadcastStartedAt")
                .and_then(serde_json::Value::as_u64);
            if !opened.is_some_and(|opened| {
                opened > 0
                    && opened <= clock.received_at_ms
                    && broadcast_started_at.is_none_or(|expected| expected == opened)
            }) {
                // A different/unknown broadcast cannot extend an old count's
                // coverage. Persist an explicit unknown, not the claimed value.
                count = None;
            }
        }
        let sample = Self {
            version: 1,
            source: source.into(),
            received_at: clock.received_at_ms,
            offset_seconds: clock.received_at_ms.saturating_sub(recording_started_at) as f64
                / 1000.0,
            viewer_count: count,
            replay_clock: clock,
        };
        sample.valid().then_some(sample)
    }
}
fn storage() -> StreamError {
    StreamError::new(
        "VIEWER_METRIC_STORAGE",
        "시청자 수 관측 기록을 저장하거나 읽지 못했습니다.",
        false,
    )
}
pub(crate) struct ViewerLog {
    file: File,
    bytes: u64,
    last: Option<u64>,
    dirty: bool,
}
impl ViewerLog {
    pub fn create(root: &Path) -> Result<Self, StreamError> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(FILE_NAME))
            .map_err(|_| storage())?;
        Ok(Self {
            file,
            bytes: 0,
            last: None,
            dirty: true,
        })
    }
    pub fn append(&mut self, sample: &ViewerSample) -> Result<(), StreamError> {
        if !sample.valid()
            || self
                .last
                .is_some_and(|last| sample.received_at < last.saturating_add(1000))
        {
            return Ok(());
        }
        let mut data = serde_json::to_vec(sample).map_err(|_| storage())?;
        if data.len() > 2048 || self.bytes.saturating_add(data.len() as u64 + 1) > MAX_BYTES {
            return Err(storage());
        }
        data.push(b'\n');
        self.dirty = true;
        self.file.write_all(&data).map_err(|_| storage())?;
        self.bytes += data.len() as u64;
        self.last = Some(sample.received_at);
        Ok(())
    }
    pub fn sync(&mut self) -> Result<(), StreamError> {
        if self.dirty {
            self.file.sync_all().map_err(|_| storage())?;
            self.dirty = false;
        }
        Ok(())
    }
}

/// The root comes only from the capture store's already-owned recording, never
/// IPC. Revalidate every ancestor and the final file; optional absence is legacy.
pub(crate) fn open_recording(root: &Path) -> Result<Option<File>, StreamError> {
    if !root.is_absolute() {
        return Err(storage());
    }
    let path = root.join(FILE_NAME);
    for ancestor in path.ancestors() {
        let metadata = match fs::symlink_metadata(ancestor) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && ancestor == path => {
                return Ok(None)
            }
            Err(_) => return Err(storage()),
        };
        if metadata.file_type().is_symlink() {
            return Err(storage());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(storage());
            }
        }
    }
    let file = File::open(&path).map_err(|_| storage())?;
    let metadata = file.metadata().map_err(|_| storage())?;
    if !metadata.is_file()
        || metadata.len() > MAX_BYTES
        || fs::canonicalize(&path).map_err(|_| storage())? != path
    {
        return Err(storage());
    }
    Ok(Some(file))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(time: u64, value: Option<u64>) -> ViewerSample {
        ViewerSample {
            version: 1,
            source: SOURCE.into(),
            received_at: time,
            offset_seconds: time as f64 / 1000.0,
            viewer_count: value,
            replay_clock: ChatReplayClock {
                version: 1,
                received_at_ms: time,
                observed_monotonic_ms: time as f64,
                source_generation: 1,
                clock: "player_observation".into(),
                media_time_seconds: None,
                playback_rate: None,
                source_id: None,
                source_time_seconds: None,
            },
        }
    }
    #[test]
    fn public_api_samples_are_generation_checked_and_legacy_rows_stay_readable() {
        use serde_json::json;
        let clock = sample(20_000, Some(0)).replay_clock;
        let mut event = json!({"viewerSource":API_SOURCE,"viewerChannelId":"channel",
            "viewerBroadcastStartedAt":1000,"viewerCount":0});
        let ingest = |value: &serde_json::Value| {
            ViewerSample::from_page(value, clock.clone(), 10_000, Some(1000), "channel")
        };
        let first = ingest(&event).unwrap();
        assert_eq!(first.viewer_count, Some(0));
        assert_eq!(first.offset_seconds, 10.0);
        assert_eq!(first.source, API_SOURCE);
        let raw = serde_json::to_string(&first).unwrap();
        assert!(!raw.contains("viewerBroadcastStartedAt"));
        assert!(!raw.contains("viewerChannelId"));
        assert!(serde_json::from_str::<ViewerSample>(&raw).unwrap().valid());
        event["viewerBroadcastStartedAt"] = json!(2000);
        assert_eq!(ingest(&event).unwrap().viewer_count, None);
        event["viewerBroadcastStartedAt"] = json!(30_000);
        assert_eq!(ingest(&event).unwrap().viewer_count, None);
        event["viewerBroadcastStartedAt"] = serde_json::Value::Null;
        assert_eq!(ingest(&event).unwrap().viewer_count, None);
        event["viewerChannelId"] = json!("other");
        assert!(ingest(&event).is_none());
        event["viewerSource"] = json!("untrusted_estimate");
        assert!(ingest(&event).is_none());
        for count in [json!(-1), json!(1.5), json!("10"), json!(100_000_001)] {
            assert!(ingest(&json!({"viewerCount":count})).is_none());
        }
        let legacy = ingest(&json!({"viewerCount":42})).unwrap();
        assert_eq!(legacy.source, SOURCE);
        assert!(
            serde_json::from_str::<ViewerSample>(&serde_json::to_string(&legacy).unwrap())
                .unwrap()
                .valid()
        );
    }

    #[test]
    fn sample_hold_uses_source_cadence_and_only_scales_an_observed_media_mapping() {
        let mut value = sample(1000, Some(10));
        value.replay_clock.playback_rate = Some(2.0);
        assert_eq!(value.hold_seconds(true), 6.0);
        value.source = API_SOURCE.into();
        assert_eq!(value.hold_seconds(false), 15.0);
        assert_eq!(value.hold_seconds(true), 30.0);
        value.replay_clock.playback_rate = Some(4.0);
        assert_eq!(value.hold_seconds(true), 60.0);
        value.replay_clock.playback_rate = Some(4.01);
        // An out-of-range rate invalidates the clock/sample rather than
        // permitting an unbounded hold at ingestion.
        assert!(!value.valid());
    }

    #[test]
    fn viewer_journal_preserves_zero_and_unknown_and_bounds_sampling() {
        let dir = tempfile::tempdir().unwrap();
        // BrowserCaptureStore supplies a canonical root (including Windows'
        // verbatim path prefix), not an arbitrary caller's temp path spelling.
        let root = fs::canonicalize(dir.path()).unwrap();
        assert!(open_recording(&root).unwrap().is_none());
        let mut log = ViewerLog::create(&root).unwrap();
        for item in [
            sample(1000, Some(0)),
            sample(1100, Some(3)),
            sample(3000, None),
            sample(5000, Some(42)),
        ] {
            log.append(&item).unwrap();
        }
        log.sync().unwrap();
        let raw = fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        let samples = raw
            .lines()
            .map(|line| serde_json::from_str::<ViewerSample>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0].viewer_count, Some(0));
        assert_eq!(samples[1].viewer_count, None);
        assert!(open_recording(&root).unwrap().is_some());
        assert!(!sample(6000, Some(100_000_001)).valid());
        let mut bad = sample(7000, Some(1));
        bad.source = "chat_participants".into();
        assert!(!bad.valid());
        bad = sample(7000, None);
        bad.replay_clock.received_at_ms = 1;
        assert!(!bad.valid());
        assert!(open_recording(Path::new("relative")).is_err());
    }
}
