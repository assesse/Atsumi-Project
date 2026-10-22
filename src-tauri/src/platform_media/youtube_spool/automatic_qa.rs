//! Debug-only, read-only evidence for the isolated automatic-recording example.
//! No capture, publication, repair, cleanup, network or process authority.
use super::{
    files::{Directories, GuardedFile}, index, raw_anchor,
    recovery::{selection::{CompletedSelection, SelectionFacts}, PrefixState}, schema, Owner, SpoolIdentity,
    PublicationExclusion, PublicationSourceFacts, PublicationState,
};
use crate::platform_media::{capture_outcome, chat_status, encoded, storage, MediaJob};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path, sync::atomic::{AtomicBool, Ordering}, time::Instant};

const MAX_MEDIA: u64 = 128 * 1024 * 1024 * 1024;
type Result<T> = std::result::Result<T, RawAutomaticQaError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RawAutomaticQaError {
    JobIdentity, RawProof, SourceSelection, FinalFile, ChatStatus, Timeline,
    Bounds, Cancelled, Deadline,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawAutomaticQaEvidence {
    pub job_id: String,
    #[serde(skip)]
    pub group_key: String,
    pub first_tick: i128,
    pub end_tick: i128,
    pub ticks_per_second: u64,
    pub first_ordinal: u64,
    pub last_ordinal: u64,
    pub within_job_boundaries: u64,
    pub source_interrupted: bool,
    pub torn_journal: bool,
    pub incomplete_append: bool,
    pub uncommitted_bytes: [u64; 2],
    pub partial_tail: [bool; 2],
    pub excluded_units: [u64; 2],
    pub excluded_bytes: [u64; 2],
    pub final_file_verified: bool,
    pub final_bytes: u64,
    pub final_sha256: String,
    pub capture_outcome: Value,
    pub recording_complete: bool,
    pub chat_metadata_verified: bool,
    pub chat_status: Option<chat_status::ChatArchiveSummary>,
    pub timeline_verified: bool,
    pub timeline_origin_ms: Option<u64>,
    pub common_numerator: Option<i128>,
    pub common_denominator: Option<i128>,
}

fn checkpoint(cancel: &AtomicBool, deadline: Instant) -> Result<()> {
    if cancel.load(Ordering::Acquire) { return Err(RawAutomaticQaError::Cancelled); }
    if Instant::now() >= deadline { return Err(RawAutomaticQaError::Deadline); }
    Ok(())
}
fn group(identity: &SpoolIdentity) -> Result<String> {
    if !matches!(&identity.owner, Owner::Scheduled { .. }) { return Err(RawAutomaticQaError::JobIdentity); }
    // Per-job/request IDs are excluded. A raw generation is never silently
    // joined to another generation or to an AVC decoder-format continuation.
    let bytes = serde_json::to_vec(&json!(["raw", identity.video_id, identity.channel_id,
        identity.receiver_id, identity.source_id, identity.document_id,
        identity.document_epoch, identity.source_generation, identity.owner]))
        .map_err(|_| RawAutomaticQaError::JobIdentity)?;
    Ok(schema::hash(&bytes))
}
fn file_hash(path: &Path, expected_bytes: u64, expected_hash: &str, cancel: &AtomicBool,
    deadline: Instant) -> Result<(GuardedFile, Directories)> {
    if !(1..=MAX_MEDIA).contains(&expected_bytes) || !schema::digest_valid(expected_hash) {
        return Err(RawAutomaticQaError::FinalFile);
    }
    let dirs = Directories::hold(path.parent().ok_or(RawAutomaticQaError::FinalFile)?)
        .map_err(|_| RawAutomaticQaError::FinalFile)?;
    let mut held = GuardedFile::open(path, false).map_err(|_| RawAutomaticQaError::FinalFile)?;
    if held.check().map_err(|_| RawAutomaticQaError::FinalFile)? != expected_bytes {
        return Err(RawAutomaticQaError::FinalFile);
    }
    let mut buffer = vec![0; 1024 * 1024];
    let mut remaining = expected_bytes;
    let mut digest = Sha256::new();
    while remaining > 0 {
        checkpoint(cancel, deadline)?;
        let count = remaining.min(buffer.len() as u64) as usize;
        held.file.read_exact(&mut buffer[..count]).map_err(|_| RawAutomaticQaError::FinalFile)?;
        digest.update(&buffer[..count]); remaining -= count as u64;
    }
    let mut tail = [0];
    if held.file.read(&mut tail).map_err(|_| RawAutomaticQaError::FinalFile)? != 0
        || format!("{:x}", digest.finalize()) != expected_hash
        || held.check().map_err(|_| RawAutomaticQaError::FinalFile)? != expected_bytes {
        return Err(RawAutomaticQaError::FinalFile);
    }
    dirs.check().map_err(|_| RawAutomaticQaError::FinalFile)?;
    checkpoint(cancel, deadline)?;
    Ok((held, dirs))
}
fn timeline_bytes(directory: &Path) -> Result<Option<(GuardedFile, Vec<u8>)>> {
    let path = directory.join("chat-timeline.json");
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(RawAutomaticQaError::Timeline),
        Ok(_) => (),
    }
    let mut held = GuardedFile::open(&path, false).map_err(|_| RawAutomaticQaError::Timeline)?;
    let length = held.check().map_err(|_| RawAutomaticQaError::Timeline)?;
    if !(1..=4096).contains(&length) { return Err(RawAutomaticQaError::Timeline); }
    let mut bytes = vec![0; length as usize];
    held.file.read_exact(&mut bytes).map_err(|_| RawAutomaticQaError::Timeline)?;
    Ok(Some((held, bytes)))
}
fn capture_projection(facts: &SelectionFacts) -> Result<capture_outcome::CaptureOutcome> {
    let mut value = PublicationSourceFacts { state: match facts.state {
        PrefixState::Sealed => PublicationState::Sealed,
        PrefixState::Interrupted => PublicationState::Interrupted,
        _ => return Err(RawAutomaticQaError::SourceSelection),
    }, seal_cause: facts.seal_cause, selected: true, incomplete_append: facts.incomplete_append_start.is_some(),
        torn_journal: facts.torn_journal, uncommitted_bytes: facts.uncommitted_bytes,
        partial_tail: [false;2], leading: [PublicationExclusion::default();2],
        trailing: [PublicationExclusion::default();2], audio_excluded_packets: [0;2] };
    for (at,track) in facts.tracks.iter().enumerate() {
        value.partial_tail[at] = matches!(track.original_outcome.terminal,index::Terminal::PartialTail {..});
        for (out,input) in [(&mut value.leading[at],track.leading),(&mut value.trailing[at],track.trailing)] {
            if let Some(input) = input { let reasons=input.reasons;
                *out=PublicationExclusion { bytes:input.end.checked_sub(input.start).ok_or(RawAutomaticQaError::Bounds)?,
                    units:input.complete_units,startup_coverage:reasons.startup_coverage,
                    audio_coverage_tail:reasons.audio_coverage_tail,outside_selected_video:reasons.outside_selected_video,
                    incomplete_structure:reasons.incomplete_structure,incomplete_append:reasons.incomplete_append };
            }
        }
    }
    // Packet-only AAC boundary trimming does not change this persisted DTO's
    // state/counts. No packet-exclusion count or packet proof is invented here.
    capture_outcome::from_source(&value).map_err(|_|RawAutomaticQaError::SourceSelection)
}

pub fn inspect_automatic_raw_qa(media_root: &Path, job: &MediaJob, cancel: &AtomicBool,
    deadline: Instant) -> Result<RawAutomaticQaEvidence> {
    checkpoint(cancel, deadline)?;
    if job.download_root != media_root || job.reference.provider != "youtube"
        || job.reference.kind != "live" || job.state != "completed" {
        return Err(RawAutomaticQaError::JobIdentity);
    }
    let item = job.library.as_ref().ok_or(RawAutomaticQaError::JobIdentity)?;
    if item.library_id != job.job_id || item.reference != job.reference
        || item.broadcast_id != job.broadcast_id || item.revision != job.revision {
        return Err(RawAutomaticQaError::JobIdentity);
    }
    let directory = storage::job_dir(media_root, "youtube", &job.job_id)
        .map_err(|_| RawAutomaticQaError::JobIdentity)?;
    let identity = encoded::recover_youtube_raw_identity(job, &directory, cancel)
        .map_err(|_| RawAutomaticQaError::RawProof)?;
    let group_key = group(&identity)?;
    let source = CompletedSelection::prepare(media_root, &identity, cancel)
        .map_err(|_| RawAutomaticQaError::SourceSelection)?;
    checkpoint(cancel, deadline)?;
    let facts = source.facts();
    let projected_outcome = capture_projection(facts)?;
    if item.capture_outcome.as_ref() != Some(&projected_outcome) { return Err(RawAutomaticQaError::SourceSelection); }
    let range = facts.tracks[0].selected;
    if range.first_tick < 0 || range.end_tick <= range.first_tick || range.ticks_per_second == 0 {
        return Err(RawAutomaticQaError::SourceSelection);
    }
    let within = range.last_ordinal.checked_sub(range.first_ordinal).ok_or(RawAutomaticQaError::Bounds)?;
    let mut partial_tail = [false; 2];
    let mut excluded_units = [0u64; 2];
    let mut excluded_bytes = [0u64; 2];
    for (at, track) in facts.tracks.iter().enumerate() {
        partial_tail[at] = matches!(track.original_outcome.terminal, index::Terminal::PartialTail { .. });
        for excluded in [track.leading, track.trailing].into_iter().flatten() {
            excluded_units[at] = excluded_units[at].checked_add(excluded.complete_units).ok_or(RawAutomaticQaError::Bounds)?;
            excluded_bytes[at] = excluded_bytes[at].checked_add(excluded.end.checked_sub(excluded.start)
                .ok_or(RawAutomaticQaError::Bounds)?).ok_or(RawAutomaticQaError::Bounds)?;
        }
    }
    let final_path = storage::owned_file(job, item).map_err(|_| RawAutomaticQaError::FinalFile)?;
    let (final_file, final_dirs) = file_hash(&final_path, item.bytes, &item.sha256, cancel, deadline)?;
    let chat = chat_status::inspect(&directory, "youtube", &job.job_id)
        .map_err(|_| RawAutomaticQaError::ChatStatus)?;
    if chat != item.chat_archive { return Err(RawAutomaticQaError::ChatStatus); }
    let timeline = timeline_bytes(&directory)?;
    let mut origin = None; let mut common_numerator = None; let mut common_denominator = None;
    if let Some((held, bytes)) = timeline.as_ref() {
        let (common, saved) = raw_anchor::inspect_qa_timeline(bytes, &directory, &job.job_id)
            .map_err(|_| RawAutomaticQaError::Timeline)?;
        let reconstructed = source.raw_timeline(common, cancel).ok_or(RawAutomaticQaError::Timeline)?;
        if !reconstructed.qa_matches(&saved) || held.check().map_err(|_| RawAutomaticQaError::Timeline)? != bytes.len() as u64 {
            return Err(RawAutomaticQaError::Timeline);
        }
        origin = Some(saved.origin_ms()); common_numerator = Some(common.numerator); common_denominator = Some(common.denominator);
    }
    source.check(cancel).map_err(|_| RawAutomaticQaError::SourceSelection)?;
    if final_file.check().map_err(|_| RawAutomaticQaError::FinalFile)? != item.bytes { return Err(RawAutomaticQaError::FinalFile); }
    final_dirs.check().map_err(|_| RawAutomaticQaError::FinalFile)?;
    checkpoint(cancel, deadline)?;
    let capture_outcome = serde_json::to_value(&item.capture_outcome).map_err(|_| RawAutomaticQaError::JobIdentity)?;
    let recording_complete = capture_outcome["state"] == "complete";
    Ok(RawAutomaticQaEvidence { job_id: job.job_id.clone(), group_key,
        first_tick: range.first_tick, end_tick: range.end_tick, ticks_per_second: range.ticks_per_second,
        first_ordinal: range.first_ordinal, last_ordinal: range.last_ordinal, within_job_boundaries: within,
        source_interrupted: facts.state == PrefixState::Interrupted, torn_journal: facts.torn_journal,
        incomplete_append: facts.incomplete_append_start.is_some(), uncommitted_bytes: facts.uncommitted_bytes,
        partial_tail, excluded_units, excluded_bytes, final_file_verified: true,
        final_bytes: item.bytes, final_sha256: item.sha256.clone(), capture_outcome, recording_complete,
        chat_metadata_verified: chat.is_some(), chat_status: chat, timeline_verified: origin.is_some(),
        timeline_origin_ms: origin, common_numerator, common_denominator })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn deadline() -> Instant { Instant::now() + Duration::from_secs(10) }
    // Container-only fixture and a fixed fake final hash: this checks the QA
    // observer, never mints a verified candidate or claims playable media.
    fn fixture(partial: bool) -> (tempfile::TempDir,MediaJob,SpoolIdentity) {
        use super::super::{Chunk,ClockDescriptor,Mode,SealCause,SpoolWriter,Track,Window};
        use crate::platform_media::{LibraryItem,MediaRef,live::RecordingOwner};
        use std::sync::Arc;
        let root=tempfile::tempdir().unwrap();std::fs::create_dir(root.path().join("YouTube")).unwrap();
        let channel=format!("UC{}","a".repeat(22));let run=uuid::Uuid::new_v4().to_string();
        let id=SpoolIdentity {job_id:uuid::Uuid::new_v4().to_string(),request_id:uuid::Uuid::new_v4().to_string(),
            receiver_id:uuid::Uuid::new_v4().to_string(),source_id:uuid::Uuid::new_v4().to_string(),
            document_id:uuid::Uuid::new_v4().to_string(),document_epoch:1,source_generation:1,
            video_id:"abcdefghijk".into(),channel_id:channel.clone(),owner:Owner::Scheduled {
                schedule_id:channel.clone(),run_id:run.clone(),broadcast_key:"abcdefghijk".into(),generation:1}};
        let owner=RecordingOwner::Scheduled {schedule_id:channel,run_id:run,broadcast_key:id.video_id.clone(),generation:1};
        let clock=ClockDescriptor {video_timestamp_scale_ns:1_000_000,audio_timescale:44100,mode:Mode::Segments,
            append_window:Window::Default,video_timestamp_offset:0,audio_timestamp_offset:0};
        let mut writer=SpoolWriter::create_new(root.path(),id.clone(),clock,Arc::new(AtomicBool::new(false))).unwrap();
        let directory=root.path().join("YouTube").join(&id.job_id);
        let marker_at=|bytes:&[u8],marker:&[u8]|bytes.windows(marker.len()).position(|part|part==marker).unwrap();
        let mut first_video_len=0;
        for n in 0..2u64 {
            let mut pair=index::authored_pair();
            let at=marker_at(&pair[0],&[0xe7,0x82,3,232]);pair[0][at+2..at+4].copy_from_slice(&(1000+n as u16*100).to_be_bytes());
            let at=marker_at(&pair[1],b"tfdt");pair[1][at+8..at+16].copy_from_slice(&(44100+n*4410).to_be_bytes());
            let at=marker_at(&pair[1],b"trex");pair[1][at+16..at+20].copy_from_slice(&1470u32.to_be_bytes());
            for (at,bytes) in pair.into_iter().enumerate() {
                if at==0 && n==0 {first_video_len=bytes.len() as u64;}
                let chunk=Chunk {track:if at==0 {Track::Video}else{Track::Audio},append_index:n,chunk_index:0,final_chunk:true,bytes};
                let observation=if at==0 && n==1 {Some(raw_anchor::BoundaryObservation::capture(Instant::now(),first_video_len,&chunk).unwrap())}else{None};
                let ack=writer.append(&id,chunk).unwrap();
                if let Some(observation)=observation {observation.persist(&directory,&id,&ack,&AtomicBool::new(false)).unwrap();}
            }
        }
        if partial {writer.append(&id,Chunk {track:Track::Video,append_index:2,chunk_index:0,final_chunk:true,bytes:vec![0x1a]}).unwrap();}
        writer.seal(&id,SealCause::Stopped).unwrap();drop(writer);
        let reference=MediaRef {provider:"youtube".into(),kind:"live".into(),external_id:id.video_id.clone()};
        std::fs::write(directory.join("encoded-owner.json"),serde_json::to_vec(&json!({"version":1,"reference":reference,
            "broadcastId":id.video_id,"receiverId":id.receiver_id,"sourceId":id.source_id,"generation":id.document_epoch,
            "requestId":id.request_id,"jobId":id.job_id,"owner":owner})).unwrap()).unwrap();
        std::fs::write(directory.join("recording-owner.json"),serde_json::to_vec(&owner).unwrap()).unwrap();
        let cancel=AtomicBool::new(false);let selected=CompletedSelection::prepare(root.path(),&id,&cancel).unwrap();
        let capture=capture_projection(selected.facts()).unwrap();
        let marker=chat_status::begin(&directory,"youtube",&id.job_id).unwrap();
        std::fs::write(directory.join("chat.events.jsonl"),b"").unwrap();
        let chat=chat_status::finish(&directory,&marker,None,false).unwrap();
        selected.raw_timeline(super::super::packet_facts::Rational::new(1,1).unwrap(),&cancel).unwrap()
            .publish(&directory,&marker,&cancel).unwrap();drop(selected);
        let bytes=b"not a media certificate";std::fs::write(directory.join("media.mp4"),bytes).unwrap();
        let job=MediaJob {job_id:id.job_id.clone(),request_id:id.request_id.clone(),reference:reference.clone(),
            broadcast_id:Some(id.video_id.clone()),revision:2,state:"completed".into(),title:String::new(),channel_name:String::new(),
            created_at:0,received_bytes:0,total_bytes:None,error:None,download_root:root.path().to_owned(),
            library:Some(LibraryItem {library_id:id.job_id.clone(),revision:2,reference,broadcast_id:Some(id.video_id.clone()),
                title:String::new(),channel_name:String::new(),bytes:bytes.len() as u64,duration:0.2,sha256:schema::hash(bytes),
                completed_at:0,modified_millis:0,chat_archive:Some(chat),capture_outcome:Some(capture)})};
        (root,job,id)
    }
    #[test]
    fn native_raw_range_v2_and_final_hash_are_read_without_changing_evidence() {
        let (root,job,_)=fixture(false);let dir=root.path().join("YouTube").join(&job.job_id);
        let names=["video.raw","audio.raw","proof.json","journal.jsonl","chat-timeline.json","chat-status.final.json","media.mp4"];
        let before=names.map(|name|std::fs::read(dir.join(name)).unwrap());
        let result=inspect_automatic_raw_qa(root.path(),&job,&AtomicBool::new(false),deadline()).unwrap();
        assert!(result.final_file_verified&&result.chat_metadata_verified&&result.timeline_verified&&result.recording_complete);
        assert_eq!((result.first_tick,result.end_tick,result.ticks_per_second),(1_000_000_000,1_200_000_000,1_000_000_000));
        assert_eq!(result.within_job_boundaries,1);
        assert_eq!(names.map(|name|std::fs::read(dir.join(name)).unwrap()),before);
    }
    #[test]
    fn partial_source_is_not_complete_and_failed_job_is_not_salvaged() {
        let (root,mut job,_)=fixture(true);
        // The authored Segment and Cluster have unknown size. A lone 0x1a
        // cannot establish the successor EBML header, so the native parser
        // withholds the preceding unit as well as this incomplete header.
        let authored_unit_bytes=index::authored_pair()[0].len() as u64;
        let unfinished_header_bytes=1;
        let raw=root.path().join("YouTube").join(&job.job_id).join("video.raw");
        assert_eq!(std::fs::metadata(raw).unwrap().len(),2*authored_unit_bytes+unfinished_header_bytes);
        let result=inspect_automatic_raw_qa(root.path(),&job,&AtomicBool::new(false),deadline()).unwrap();
        assert!(result.final_file_verified&&result.partial_tail[0]);assert!(!result.recording_complete);
        assert_eq!(result.excluded_bytes[0],authored_unit_bytes+unfinished_header_bytes);
        assert_eq!((result.first_ordinal,result.last_ordinal,result.within_job_boundaries),(0,0,0));
        assert_eq!((result.first_tick,result.end_tick,result.ticks_per_second),(1_000_000_000,1_100_000_000,1_000_000_000));
        job.state="failed".into();assert!(matches!(inspect_automatic_raw_qa(root.path(),&job,&AtomicBool::new(false),deadline()),Err(RawAutomaticQaError::JobIdentity)));
    }
    #[test]
    fn native_owner_root_fatal_and_malformed_timeline_do_not_fall_back() {
        let (root,job,_)=fixture(false);let dir=root.path().join("YouTube").join(&job.job_id);let cancel=AtomicBool::new(false);
        assert!(matches!(inspect_automatic_raw_qa(&root.path().join("other"),&job,&cancel,deadline()),Err(RawAutomaticQaError::JobIdentity)));
        let owner_path=dir.join("recording-owner.json");let owner=std::fs::read(&owner_path).unwrap();
        std::fs::write(&owner_path,b"{}").unwrap();
        assert!(matches!(inspect_automatic_raw_qa(root.path(),&job,&cancel,deadline()),Err(RawAutomaticQaError::RawProof)));
        std::fs::write(owner_path,owner).unwrap();
        let timeline=dir.join("chat-timeline.json");let original=std::fs::read(&timeline).unwrap();
        std::fs::write(&timeline,b"{\"version\":1}").unwrap();
        assert!(matches!(inspect_automatic_raw_qa(root.path(),&job,&cancel,deadline()),Err(RawAutomaticQaError::Timeline)));
        std::fs::write(timeline,original).unwrap();
        std::fs::write(dir.join("youtube-raw-fatal.json"),b"{}").unwrap();
        assert!(matches!(inspect_automatic_raw_qa(root.path(),&job,&cancel,deadline()),Err(RawAutomaticQaError::RawProof)));
    }
    #[test]
    fn distinct_source_generation_document_and_broadcast_never_group() {
        let (_root,_job,id)=fixture(false);let first=group(&id).unwrap();
        for case in 0..5 {let mut other=id.clone();match case {
            0=>other.source_generation+=1,1=>other.document_id=uuid::Uuid::new_v4().to_string(),
            2=>other.source_id=uuid::Uuid::new_v4().to_string(),3=>other.video_id="T4XZmMPQ9Kw".into(),
            _=>other.receiver_id=uuid::Uuid::new_v4().to_string()};assert_ne!(group(&other).unwrap(),first);}
        let mut same=id.clone();same.job_id=uuid::Uuid::new_v4().to_string();same.request_id=uuid::Uuid::new_v4().to_string();assert_eq!(group(&same).unwrap(),first);
    }
    #[test]
    fn held_hash_checks_full_bytes_and_preserves_source() {
        let root = tempfile::tempdir().unwrap(); let path = root.path().join("media.mp4");
        let bytes = vec![19; 1024 * 1024 + 7]; std::fs::write(&path, &bytes).unwrap();
        let cancel = AtomicBool::new(false); let hash = schema::hash(&bytes);
        let guards = file_hash(&path, bytes.len() as u64, &hash, &cancel, deadline()).unwrap();
        drop(guards);
        assert!(matches!(file_hash(&path, bytes.len() as u64 - 1, &hash, &cancel, deadline()), Err(RawAutomaticQaError::FinalFile)));
        assert!(matches!(file_hash(&path, bytes.len() as u64, &"0".repeat(64), &cancel, deadline()), Err(RawAutomaticQaError::FinalFile)));
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn fixed_timeline_missing_is_not_verified_and_directory_is_not_missing() {
        let root = tempfile::tempdir().unwrap(); assert!(timeline_bytes(root.path()).unwrap().is_none());
        std::fs::create_dir(root.path().join("chat-timeline.json")).unwrap();
        assert!(matches!(timeline_bytes(root.path()), Err(RawAutomaticQaError::Timeline)));
    }
    #[test]
    fn hash_cancel_deadline_and_link_reject_without_writing() {
        let root = tempfile::tempdir().unwrap(); let path = root.path().join("media.mp4");
        std::fs::write(&path, b"abcd").unwrap(); let hash = schema::hash(b"abcd");
        assert!(matches!(file_hash(&path, 4, &hash, &AtomicBool::new(true), deadline()), Err(RawAutomaticQaError::Cancelled)));
        assert!(matches!(file_hash(&path, 4, &hash, &AtomicBool::new(false), Instant::now()), Err(RawAutomaticQaError::Deadline)));
        std::fs::hard_link(&path, root.path().join("alias.mp4")).unwrap();
        assert!(matches!(file_hash(&path, 4, &hash, &AtomicBool::new(false), deadline()), Err(RawAutomaticQaError::FinalFile)));
    }
}
