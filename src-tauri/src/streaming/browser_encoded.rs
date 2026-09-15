//! Approved, pane-local encoded capture. No URLs, cookies, external processes or
//! additional media requests are accepted here. Only the native fMP4 parser's
//! independently playable output reaches the existing durable segment journal.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[path = "browser_fmp4.rs"]
pub(crate) mod fmp4;
use fmp4::{EncodedMuxer, EncodedSegment, EncodedTrackInput};

const MAX_INIT: usize = 64 * 1024;
const MAX_APPEND: usize = 32 * 1024 * 1024;
const CHUNK: usize = 128 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EncodedTrack {
    track_index: u32,
    mime_type: String,
    init: String,
}

struct PartialAppend {
    track: u32,
    index: u64,
    next_chunk: u64,
    bytes: Vec<u8>,
}
#[derive(PartialEq)]
struct Receipt {
    track: u32,
    index: u64,
    chunk: u64,
    final_chunk: bool,
    digest: [u8; 32],
}
pub(super) struct EncodedSession {
    pub(super) recording_id: String,
    request_id: String,
    pub(super) source_id: String,
    fingerprint: [u8; 32],
    generation: u64,
    muxer: EncodedMuxer,
    next: BTreeMap<u32, u64>,
    partial: Option<PartialAppend>,
    previous: Option<Receipt>,
    segment_index: u64,
}
fn invalid() -> StreamError {
    error(
        "BROWSER_ENCODED_INVALID",
        "원본 영상 저장 요청의 형식이나 순서가 올바르지 않습니다.",
    )
}
fn fingerprint(tracks: &[EncodedTrack]) -> Result<[u8; 32], StreamError> {
    Ok(Sha256::digest(serde_json::to_vec(tracks).map_err(|_| invalid())?).into())
}
fn decode_tracks(tracks: &[EncodedTrack]) -> Result<Vec<EncodedTrackInput>, StreamError> {
    if tracks.is_empty() || tracks.len() > 2 {
        return Err(invalid());
    }
    let mut result = Vec::new();
    for track in tracks {
        if track.track_index > 1
            || result
                .iter()
                .any(|t: &EncodedTrackInput| t.track_index == track.track_index)
            || track.mime_type.len() > 120
            || track.init.len() > MAX_INIT.div_ceil(3) * 4
        {
            return Err(invalid());
        }
        let init = STANDARD.decode(&track.init).map_err(|_| invalid())?;
        if init.is_empty() || init.len() > MAX_INIT {
            return Err(invalid());
        }
        result.push(EncodedTrackInput {
            track_index: track.track_index,
            mime_type: track.mime_type.clone(),
            init,
        });
    }
    Ok(result)
}
impl EncodedSession {
    fn accept(
        &mut self,
        track: u32,
        index: u64,
        chunk: u64,
        final_chunk: bool,
        data: String,
    ) -> Result<(bool, Vec<EncodedSegment>), StreamError> {
        if data.len() > CHUNK.div_ceil(3) * 4 {
            return Err(invalid());
        }
        let bytes = STANDARD.decode(data).map_err(|_| invalid())?;
        if bytes.is_empty() || bytes.len() > CHUNK {
            return Err(invalid());
        }
        let receipt = Receipt {
            track,
            index,
            chunk,
            final_chunk,
            digest: Sha256::digest(&bytes).into(),
        };
        if self.previous.as_ref() == Some(&receipt) {
            return Ok((true, Vec::new()));
        }
        if self.next.get(&track) != Some(&index) || index >= 1_000_000 {
            return Err(invalid());
        }
        if self.partial.is_none() {
            if chunk != 0 {
                return Err(invalid());
            }
            self.partial = Some(PartialAppend {
                track,
                index,
                next_chunk: 0,
                bytes: Vec::new(),
            });
        }
        let partial = self.partial.as_mut().ok_or_else(invalid)?;
        if partial.track != track
            || partial.index != index
            || partial.next_chunk != chunk
            || partial
                .bytes
                .len()
                .checked_add(bytes.len())
                .is_none_or(|size| size > MAX_APPEND)
        {
            return Err(invalid());
        }
        partial.bytes.extend_from_slice(&bytes);
        partial.next_chunk += 1;
        let segments = if final_chunk {
            let partial = self.partial.take().ok_or_else(invalid)?;
            let segments = self.muxer.push(track, &partial.bytes)?;
            *self.next.get_mut(&track).ok_or_else(invalid)? += 1;
            segments
        } else {
            Vec::new()
        };
        self.previous = Some(receipt);
        Ok((false, segments))
    }
}
impl OfficialBrowser {
    pub(super) fn process_encoded(
        &self,
        channel: &str,
        message: BrowserMessage,
    ) -> Result<Value, StreamError> {
        match message {
            BrowserMessage::EncodedBegin {
                request_id,
                channel_id,
                title,
                source_id,
                tracks,
            } => {
                if channel != channel_id
                    || title.len() > 2048
                    || uuid::Uuid::parse_str(&source_id).is_err()
                    || source_id.len() != 36
                    || tracks.len() > 2
                {
                    return Err(invalid());
                }
                // Check the native one-use arm before doing expensive binary parsing.
                let state = self.inner.view.lock().map_err(|_| unavailable())?;
                if state.channel.as_deref() != Some(channel)
                    || state.account_busy
                    || self.inner.closing.load(Ordering::Acquire)
                    || self.inner.reserved.load(Ordering::Acquire)
                {
                    return Err(control_stale());
                }
                if let Some(id) = &state.recording {
                    let encoded = self.inner.encoded.lock().map_err(|_| unavailable())?;
                    let previous = encoded.as_ref().ok_or_else(invalid)?;
                    if &previous.recording_id == id
                        && previous.request_id == request_id
                        && previous.source_id == source_id
                        && previous.generation == state.page_generation
                        && previous.fingerprint == fingerprint(&tracks)?
                    {
                        return Ok(
                            json!({"id":id,"captureChat":state.capture_chat,"mode":"encoded","nativeApproved":true}),
                        );
                    }
                    return Err(invalid());
                }
                if !state.arm.as_ref().is_some_and(|arm| {
                    arm.id == request_id
                        && arm.generation == state.page_generation
                        && arm.created.elapsed() <= Duration::from_secs(20)
                }) {
                    return Err(control_stale());
                }
                drop(state);
                let inputs = decode_tracks(&tracks)?;
                let next = inputs.iter().map(|track| (track.track_index, 0)).collect();
                let digest = fingerprint(&tracks)?;
                let muxer = EncodedMuxer::new(inputs).inspect_err(|error| {
                    // Parser messages are fixed local descriptions, never media URLs
                    // or credential-bearing response text. Keep the real reason.
                    tracing::warn!(code = %error.code, reason = %error.message, "original recording init rejected");
                    if let Ok(mut state) = self.inner.view.lock() {
                        state.error = Some(format!("원본 저장 불가: {}", error.message));
                    }
                })?;
                // Reuse the existing arm consumption, output-root policy and chat
                // lifecycle. process() has not acquired writes for encoded input.
                let result = self.process(
                    channel,
                    BrowserMessage::Begin {
                        request_id: request_id.clone(),
                        channel_id,
                        title,
                        mime_type: "video/mp4".into(),
                    },
                )?;
                let id = result
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(invalid)?
                    .to_owned();
                let _write = self.inner.writes.lock().map_err(|_| unavailable())?;
                let state = self.inner.view.lock().map_err(|_| unavailable())?;
                if state.recording.as_deref() != Some(&id)
                    || state.channel.as_deref() != Some(channel)
                {
                    return Err(control_stale());
                }
                *self.inner.encoded.lock().map_err(|_| unavailable())? = Some(EncodedSession {
                    recording_id: id.clone(),
                    request_id,
                    source_id,
                    fingerprint: digest,
                    generation: state.page_generation,
                    muxer,
                    next,
                    partial: None,
                    previous: None,
                    segment_index: 0,
                });
                Ok(
                    json!({"id":id,"captureChat":result["captureChat"],"mode":"encoded","nativeApproved":true}),
                )
            }
            BrowserMessage::EncodedAppend {
                recording_id,
                track_index,
                append_index,
                chunk_index,
                final_chunk,
                data,
            } => {
                let _write = self.inner.writes.lock().map_err(|_| unavailable())?;
                self.validate_encoded_owner(channel, &recording_id)?;
                let mut encoded = self.inner.encoded.lock().map_err(|_| unavailable())?;
                let current = encoded
                    .as_mut()
                    .filter(|entry| entry.recording_id == recording_id)
                    .ok_or_else(invalid)?;
                let (duplicate, segments) =
                    current.accept(track_index, append_index, chunk_index, final_chunk, data)?;
                self.commit_encoded(current, segments)?;
                // An accepted partial append isn't described as durably saved.
                Ok(
                    json!({"accepted":true,"duplicate":duplicate,"committedSegments":current.segment_index,
                    "recordingId":recording_id,"appendIndex":append_index,"chunkIndex":chunk_index}),
                )
            }
            BrowserMessage::EncodedFinish {
                recording_id,
                interrupted,
                reason,
            } => {
                let wrote_any = {
                    let _write = self.inner.writes.lock().map_err(|_| unavailable())?;
                    self.validate_encoded_owner(channel, &recording_id)?;
                    let mut encoded = self.inner.encoded.lock().map_err(|_| unavailable())?;
                    let current = encoded
                        .as_mut()
                        .filter(|entry| entry.recording_id == recording_id)
                        .ok_or_else(invalid)?;
                    if current.partial.is_some() {
                        return Err(invalid());
                    }
                    let segments = current.muxer.finish()?;
                    self.commit_encoded(current, segments)?;
                    let wrote_any = current.segment_index > 0;
                    encoded.take();
                    wrote_any
                };
                self.process(
                    channel,
                    BrowserMessage::Finish {
                        recording_id,
                        interrupted: interrupted || !wrote_any,
                        reason: if !wrote_any {
                            Some("empty_segment".into())
                        } else {
                            reason
                        },
                    },
                )
            }
            _ => Err(invalid()),
        }
    }
    fn validate_encoded_owner(&self, channel: &str, recording: &str) -> Result<(), StreamError> {
        let state = self.inner.view.lock().map_err(|_| unavailable())?;
        if state.recording.as_deref() == Some(recording)
            && state.channel.as_deref() == Some(channel)
        {
            Ok(())
        } else {
            Err(control_stale())
        }
    }
    fn commit_encoded(
        &self,
        current: &mut EncodedSession,
        segments: Vec<EncodedSegment>,
    ) -> Result<(), StreamError> {
        let store = self.inner.store.lock().map_err(|_| unavailable())?;
        for segment in segments {
            if segment.bytes.is_empty() || segment.bytes.len() > 64 * 1024 * 1024 {
                return Err(invalid());
            }
            for (index, bytes) in segment.bytes.chunks(CHUNK).enumerate() {
                store.append(
                    &current.recording_id,
                    current.segment_index,
                    index as u64,
                    bytes,
                )?;
            }
            store.finish_segment_source(
                &current.recording_id,
                current.segment_index,
                segment.duration_seconds,
                Some(segment.source_start_seconds),
                Some(segment.source_end_seconds),
            )?;
            current.segment_index += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fn armed() -> (tempfile::TempDir, OfficialBrowser, String) {
        let dir = tempfile::tempdir().unwrap();
        let host = OfficialBrowser::new(dir.path().to_owned()).unwrap();
        let nonce = uuid::Uuid::new_v4().to_string();
        {
            let mut state = host.inner.view.lock().unwrap();
            state.channel = Some(CHANNEL.into());
            state.ready = true;
            state.arm = Some(Arm {
                id: nonce.clone(),
                root: dir.path().to_owned(),
                capture_chat: false,
                created: Instant::now(),
                generation: state.page_generation,
            });
        }
        (dir, host, nonce)
    }
    fn begin(host: &OfficialBrowser, nonce: &str, source: &str) -> Result<Value, StreamError> {
        host.process(
            CHANNEL,
            BrowserMessage::EncodedBegin {
                request_id: nonce.into(),
                channel_id: CHANNEL.into(),
                title: "synthetic timestamp test".into(),
                source_id: source.into(),
                tracks: fmp4::fixtures::inputs()
                    .into_iter()
                    .map(|t| EncodedTrack {
                        track_index: t.track_index,
                        mime_type: t.mime_type,
                        init: STANDARD.encode(t.init),
                    })
                    .collect(),
            },
        )
    }
    fn append(
        host: &OfficialBrowser,
        id: &str,
        track: u32,
        append_index: u64,
        chunk: u64,
        final_chunk: bool,
        bytes: &[u8],
    ) -> Result<Value, StreamError> {
        host.process(
            CHANNEL,
            BrowserMessage::EncodedAppend {
                recording_id: id.into(),
                track_index: track,
                append_index,
                chunk_index: chunk,
                final_chunk,
                data: STANDARD.encode(bytes),
            },
        )
    }
    #[test]
    fn rejects_oversized_or_duplicate_track_inputs_before_parsing() {
        let track = |index| EncodedTrack {
            track_index: index,
            mime_type: "video/mp4".into(),
            init: STANDARD.encode([0; 8]),
        };
        assert!(decode_tracks(&[]).is_err());
        assert!(decode_tracks(&[track(0), track(0)]).is_err());
        assert!(decode_tracks(&[track(2)]).is_err());
        assert!(decode_tracks(&[EncodedTrack {
            init: STANDARD.encode(vec![0; MAX_INIT + 1]),
            ..track(0)
        }])
        .is_err());
    }
    #[test]
    fn an_unarmed_page_cannot_allocate_an_encoded_recording() {
        let dir = tempfile::tempdir().unwrap();
        let host = OfficialBrowser::new(dir.path().to_owned()).unwrap();
        let channel = "a".repeat(32);
        host.inner.view.lock().unwrap().channel = Some(channel.clone());
        assert!(host
            .process_encoded(
                &channel,
                BrowserMessage::EncodedBegin {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    channel_id: channel.clone(),
                    title: "test".into(),
                    source_id: uuid::Uuid::new_v4().to_string(),
                    tracks: Vec::new(),
                }
            )
            .is_err());
        assert!(host.active_ids().is_empty());
    }
    #[test]
    fn native_mux_output_is_durable_and_recovered_with_original_source_clock() {
        let (dir, host, nonce) = armed();
        let source = uuid::Uuid::new_v4().to_string();
        let ack = begin(&host, &nonce, &source).unwrap();
        let id = ack["id"].as_str().unwrap();
        assert_eq!(ack["nativeApproved"], true);
        assert_eq!(begin(&host, &nonce, &source).unwrap()["id"], id);
        assert!(begin(&host, &nonce, &uuid::Uuid::new_v4().to_string()).is_err());
        for (track, scale) in [(0, 90_000), (1, 48_000)] {
            let bytes = fmp4::fixtures::fragment(track, 4000 * scale, 4, true);
            let middle = bytes.len() / 2;
            append(&host, id, track, 0, 0, false, &bytes[..middle]).unwrap();
            let result = append(&host, id, track, 0, 1, true, &bytes[middle..]).unwrap();
            assert_eq!(result["committedSegments"], 0);
            assert_eq!(
                append(&host, id, track, 0, 1, true, &bytes[middle..]).unwrap()["duplicate"],
                true
            );
        }
        assert!(host
            .process(
                CHANNEL,
                BrowserMessage::Chunk {
                    recording_id: id.into(),
                    segment_index: 0,
                    chunk_index: 0,
                    data: STANDARD.encode(b"unvalidated")
                }
            )
            .is_err());
        let done = host
            .process(
                CHANNEL,
                BrowserMessage::EncodedFinish {
                    recording_id: id.into(),
                    interrupted: false,
                    reason: None,
                },
            )
            .unwrap();
        assert_eq!(done["interrupted"], false);
        let snapshot = host.snapshot().unwrap();
        let saved = snapshot.recordings.iter().find(|r| r.id == id).unwrap();
        assert_eq!(saved.segment_count, 1);
        assert_eq!(saved.duration_seconds, 2.0);
        assert_eq!(saved.segments[0].source_start_seconds, Some(4000.0));
        assert_eq!(saved.segments[0].source_end_seconds, Some(4002.0));
        let bytes =
            std::fs::read(Path::new(&saved.output_dir).join(&saved.segments[0].file)).unwrap();
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(bytes.windows(4).any(|s| s == b"moof"));
        let restored = OfficialBrowser::new(dir.path().to_owned())
            .unwrap()
            .snapshot()
            .unwrap();
        assert_eq!(restored.recordings[0].segments, saved.segments);
    }
    #[test]
    fn pending_append_rejects_reordering_other_recordings_and_changed_retries() {
        let (_dir, host, nonce) = armed();
        let ack = begin(&host, &nonce, &uuid::Uuid::new_v4().to_string()).unwrap();
        let id = ack["id"].as_str().unwrap();
        assert!(append(&host, &"b".repeat(32), 0, 0, 0, false, b"123").is_err());
        assert!(append(&host, id, 0, 1, 0, false, b"123").is_err());
        append(&host, id, 0, 0, 0, false, b"123").unwrap();
        assert!(append(&host, id, 0, 0, 0, false, b"456").is_err());
        assert!(append(&host, id, 1, 0, 1, false, b"123").is_err());
        assert!(host
            .process(
                CHANNEL,
                BrowserMessage::EncodedFinish {
                    recording_id: id.into(),
                    interrupted: false,
                    reason: None
                }
            )
            .is_err());
        host.interrupt_matching("native_rejected", Some(id));
        assert!(host.inner.encoded.lock().unwrap().is_none());
        assert!(host.active_ids().is_empty());
    }
}
