//! Native, source-bound VP9 header subset. Not an entropy decoder, a packet
//! preservation certificate, publication authority, or permission to delete raw.
use super::{
    index::{self, Configuration, Format, Unit},
    recovery::{selection::{self, CompletedSelection, SelectionBinding}, PrefixState, SequentialSource, SpanFacts, UnitSpan},
    ClockDescriptor, Error, SealCause, SpoolIdentity, Track, TrackExtent,
};
use sha2::{Digest, Sha256};
use std::{io::{Read, Seek, SeekFrom}, sync::atomic::{AtomicBool, Ordering}};

const MAX_UNITS: u64 = 100_000;
const MAX_BLOCKS: u64 = 100_000_000;
const MAX_FRAMES: u64 = MAX_BLOCKS * 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fault { Source, Binding, Unsupported, Malformed, Bounds, Io, Cancelled }
type Result<T> = std::result::Result<T, Fault>;
fn checkpoint(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) { Err(Fault::Cancelled) } else { Ok(()) }
}
fn source_error(error: Error) -> Fault {
    match error { Error::Cancelled => Fault::Cancelled, Error::Io => Fault::Io, _ => Fault::Source }
}
fn index_error(error: index::Error) -> Fault {
    match error.kind {
        index::Failure::Cancelled => Fault::Cancelled,
        index::Failure::Io => Fault::Io,
        index::Failure::Limit => Fault::Bounds,
        index::Failure::Unsupported => Fault::Unsupported,
        _ => Fault::Malformed,
    }
}
fn selection_error(error: selection::Fault) -> Fault {
    match error {
        selection::Fault::Source(error) => source_error(error),
        selection::Fault::Index(error) => index_error(error),
        selection::Fault::Overflow => Fault::Bounds,
        _ => Fault::Binding,
    }
}

#[derive(PartialEq, Eq)]
struct Scope {
    identity: SpoolIdentity,
    clock: ClockDescriptor,
    tracks: [TrackExtent; 2],
    receipts: u64,
    state: PrefixState,
    seal: Option<SealCause>,
    incomplete: bool,
    torn: bool,
    uncommitted: [u64; 2],
}
impl Scope {
    fn of(f: &SpanFacts<'_>) -> Self {
        Self { identity: f.identity.clone(), clock: f.clock.clone(), tracks: (*f.tracks).clone(),
            receipts: f.receipts, state: f.state, seal: f.seal_cause,
            incomplete: f.incomplete_append, torn: f.torn_journal, uncommitted: f.uncommitted_bytes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const PICTURE: Picture = Picture { width: 854, height: 480 };
    #[derive(Default)]
    struct BitWriter(Vec<bool>);
    impl BitWriter {
        fn put(&mut self, value: u32, n: usize) {
            for bit in (0..n).rev() { self.0.push((value >> bit) & 1 != 0); }
        }
        fn size(&mut self, p: Picture) { self.put(p.width - 1, 16); self.put(p.height - 1, 16); }
        fn bytes(self) -> Vec<u8> {
            let mut out = vec![0; self.0.len().div_ceil(8)];
            for (i, bit) in self.0.into_iter().enumerate() { out[i / 8] |= u8::from(bit) << (7 - i % 8); }
            out.push(0x55); // Opaque fixture data, not a decodable media claim.
            out
        }
    }
    fn initial(w: &mut BitWriter, existing: bool) {
        w.put(2, 2); w.put(0, 2); w.put(u32::from(existing), 1);
    }
    fn key(picture: Picture, render: Option<Picture>, color: u32, range: u32) -> Vec<u8> {
        let mut w = BitWriter::default(); initial(&mut w, false);
        w.put(0, 1); w.put(1, 1); w.put(0, 1); w.put(0x498342, 24);
        w.put(color, 3); w.put(range, 1); w.size(picture);
        w.put(u32::from(render.is_some()), 1); if let Some(p) = render { w.size(p); }
        w.bytes()
    }
    fn good_key() -> Vec<u8> { key(PICTURE, None, 2, 0) }
    fn inter(show: bool, explicit: Option<Picture>, render: Option<Picture>) -> Vec<u8> {
        let mut w = BitWriter::default(); initial(&mut w, false);
        w.put(1, 1); w.put(u32::from(show), 1); w.put(0, 1);
        if !show { w.put(0, 1); } w.put(0, 2); w.put(1, 8);
        for reference in 0..3 { w.put(reference, 3); w.put(0, 1); }
        if let Some(p) = explicit { w.put(0, 3); w.size(p); } else { w.put(1, 1); }
        w.put(u32::from(render.is_some()), 1); if let Some(p) = render { w.size(p); }
        w.bytes()
    }
    fn existing(index: u32) -> Vec<u8> {
        let mut w = BitWriter::default(); initial(&mut w, true); w.put(index, 3); w.bytes()
    }
    fn superframe(parts: &[Vec<u8>], magnitude: usize) -> Vec<u8> {
        let marker = 0xc0 | (((magnitude - 1) as u8) << 3) | (parts.len() - 1) as u8;
        let mut bytes = parts.concat(); bytes.push(marker);
        for part in parts {
            bytes.extend_from_slice(&(part.len() as u32).to_le_bytes()[..magnitude]);
        }
        bytes.push(marker); bytes
    }
    fn element(id: u32, body: &[u8]) -> Vec<u8> {
        let bytes = id.to_be_bytes();
        let mut out = bytes[bytes.iter().position(|b| *b != 0).unwrap()..].to_vec();
        let n = (1..=8).find(|n| (body.len() as u64) < (1u64 << (7*n)) - 1).unwrap();
        let size = (body.len() as u64 | 1u64 << (7*n)).to_be_bytes();
        out.extend_from_slice(&size[8-n..]); out.extend_from_slice(body); out
    }
    fn integer(id: u32, value: u64) -> Vec<u8> {
        let bytes = value.to_be_bytes();
        element(id, &bytes[bytes.iter().position(|b| *b != 0).unwrap_or(7)..])
    }
    fn video(packets: &[(Vec<u8>, bool)], base: u64, group: bool) -> Vec<u8> {
        let header = element(0x1a45dfa3, &element(0x4282, b"webm"));
        let info = element(0x1549a966, &integer(0x2ad7b1, 1_000_000));
        let entry = [integer(0xd7, 1), integer(0x83, 1), element(0x86, b"V_VP9"),
            integer(0x23e383, 33_000_000), element(0xe0, &[integer(0xb0, 854), integer(0xba, 480)].concat())].concat();
        let tracks = element(0x1654ae6b, &element(0xae, &entry));
        let mut cluster = integer(0xe7, base);
        for (i, (bytes, rap)) in packets.iter().enumerate() {
            let mut block = vec![0x81]; block.extend_from_slice(&((i as i16)*33).to_be_bytes());
            block.push(if *rap && !group { 0x80 } else { 0 }); block.extend_from_slice(bytes);
            if group {
                let mut body = element(0xa1, &block);
                if !*rap { body.extend_from_slice(&element(0xfb, &[0xff])); }
                cluster.extend_from_slice(&element(0xa0, &body));
            } else { cluster.extend_from_slice(&element(0xa3, &block)); }
        }
        [header, vec![0x18, 0x53, 0x80, 0x67, 0xff], info, tracks, element(0x1f43b675, &cluster)].concat()
    }
    fn units(bytes: &[u8]) -> Vec<Unit> {
        let mut units = Vec::new();
        let result = index::scan(Format::Vp9Webm, &mut Cursor::new(bytes), bytes.len() as u64,
            1_000_000, &AtomicBool::new(false), &mut |unit| { units.push(unit); Ok(()) });
        assert_eq!(result.terminal, index::Terminal::Complete); units
    }
    fn inspect(bytes: &[u8], state: &mut Frames) -> index::Result<()> {
        let indexed = units(bytes);
        for unit in indexed { index::inspect_vp9_unit(&mut Cursor::new(bytes), &unit, &AtomicBool::new(false), state)?; }
        Ok(())
    }

    #[test]
    fn independent_key_and_every_explicit_or_reference_dimension() {
        let mut f = Frames::default();
        f.frame(&good_key(), PICTURE, true).unwrap();
        f.frame(&inter(true, None, None), PICTURE, false).unwrap();
        f.frame(&inter(false, Some(PICTURE), Some(PICTURE)), PICTURE, false).unwrap();
        f.frame(&existing(7), PICTURE, false).unwrap();
        assert_eq!((f.coded, f.displayed), (4, 3));
        let changed = Picture { width: 853, ..PICTURE };
        for bad in [inter(true, Some(changed), None), inter(true, None, Some(changed)),
            key(changed, None, 2, 0), key(PICTURE, Some(changed), 2, 0)] {
            assert_eq!(f.clone().frame(&bad, PICTURE, false), Err(Fault::Unsupported));
        }
    }
    #[test]
    fn first_container_key_cannot_substitute_coded_key_or_reference() {
        for payload in [inter(true, None, None), existing(0)] {
            assert_eq!(Frames::default().frame(&payload, PICTURE, true), Err(Fault::Unsupported));
        }
        assert_eq!(Frames::default().frame(&good_key(), PICTURE, false), Err(Fault::Unsupported));
        let mut f = Frames::default(); f.frame(&good_key(), PICTURE, true).unwrap();
        f.references[0] = None;
        assert_eq!(f.clone().frame(&existing(0), PICTURE, false), Err(Fault::Unsupported));
        assert_eq!(f.frame(&inter(true, None, None), PICTURE, false), Err(Fault::Unsupported));
    }
    #[test]
    fn marker_sync_profile_color_and_prefix_truncation_are_closed() {
        let encoded = good_key();
        for length in 0..8 { assert!(Frames::default().frame(&encoded[..length], PICTURE, true).is_err()); }
        for byte in [0usize, 1] {
            let mut bad = encoded.clone(); bad[byte] ^= 0x80;
            assert!(Frames::default().frame(&bad, PICTURE, true).is_err());
        }
        let mut profile = encoded.clone(); profile[0] |= 0x20;
        assert_eq!(Frames::default().frame(&profile, PICTURE, true), Err(Fault::Unsupported));
        for (color, range) in [(1,0), (7,0), (2,1)] {
            assert_eq!(Frames::default().frame(&key(PICTURE,None,color,range),PICTURE,true),Err(Fault::Unsupported));
        }
    }
    #[test]
    fn profile_zero_intra_only_does_not_inherit_bt709() {
        let mut w = BitWriter::default(); initial(&mut w, false);
        w.put(1,1); w.put(0,1); w.put(0,1); w.put(1,1); w.put(0,2);
        let mut f = Frames::default(); f.frame(&good_key(), PICTURE, true).unwrap();
        assert_eq!(f.frame(&w.bytes(),PICTURE,false),Err(Fault::Unsupported));
    }
    #[test]
    fn superframe_index_all_widths_counts_and_exact_sum() {
        for magnitude in 1..=4 { for count in 1..=8 {
            let encoded = superframe(&vec![vec![0x80,0x55];count],magnitude);
            let tail = &encoded[encoded.len().saturating_sub(34)..];
            let (ranges,n) = spans(encoded.len() as u64,tail).unwrap();
            assert_eq!(n,count); assert_eq!(ranges[n-1],((2*(count-1)) as u64,(2*count) as u64));
            let mut broken = encoded.clone(); broken[2*count] ^= 1;
            assert!(spans(broken.len() as u64,&broken[broken.len().saturating_sub(34)..]).is_err());
            let mut zero = encoded.clone(); zero[2*count+1..2*count+1+magnitude].fill(0);
            assert!(spans(zero.len() as u64,&zero[zero.len().saturating_sub(34)..]).is_err());
        } }
        assert!(spans(1,&[0xc0]).is_err());
        assert!(spans(4,&[0x55,0xc0,2,0xc0]).is_err());
    }
    #[test]
    fn all_subframes_are_inspected_and_packet_state_is_atomic() {
        let packet = superframe(&[good_key(), inter(false,None,None), existing(0)],1);
        let bytes = video(&[(packet,true)],1000,false);
        let mut f=Frames::default(); inspect(&bytes,&mut f).unwrap();
        assert_eq!((f.blocks,f.coded,f.displayed),(1,3,2));
        for bad in [key(PICTURE,None,1,0), vec![0], inter(true,Some(Picture{width:1,height:1}),None)] {
            let bytes=video(&[(superframe(&[good_key(),bad],1),true)],1000,false);
            let mut f=Frames::default(); assert!(inspect(&bytes,&mut f).is_err());
            assert_eq!((f.blocks,f.coded),(0,0));
        }
    }
    #[test]
    fn ordinary_index_stays_opaque_and_group_rap_is_final_before_inspection() {
        let invalid=video(&[(vec![0],true)],1000,false);
        assert_eq!(units(&invalid).len(),1);
        assert!(inspect(&invalid,&mut Frames::default()).is_err());
        let valid=video(&[(good_key(),true)],1000,true);
        inspect(&valid,&mut Frames::default()).unwrap();
        let dependent=video(&[(good_key(),false)],1000,true);
        assert!(inspect(&dependent,&mut Frames::default()).is_err());
    }
    #[test]
    fn repeated_init_keeps_reference_state_without_reading_successor_in_span() {
        let bytes=[video(&[(good_key(),true)],1000,false),video(&[(inter(true,None,None),false)],1033,false)].concat();
        let indexed=units(&bytes); assert_eq!(indexed.len(),2);
        assert_eq!(indexed[0].facts.boundary,index::Boundary::NextInitialization);
        let mut f=Frames::default(); inspect(&bytes,&mut f).unwrap();
        assert_eq!((f.blocks,f.coded),(2,2));
        let mut reader=Cursor::new(&bytes);
        assert!(index::inspect_vp9_unit(&mut reader,&indexed[1],&AtomicBool::new(false),&mut Frames::default()).is_err());
    }
    #[test]
    fn opt_in_cancel_and_promised_read_failure_cannot_pass() {
        let bytes=video(&[(good_key(),true)],1000,false); let unit=units(&bytes).remove(0);
        assert_eq!(index::inspect_vp9_unit(&mut Cursor::new(&bytes),&unit,&AtomicBool::new(true),&mut Frames::default()).unwrap_err().kind,index::Failure::Cancelled);
        assert_eq!(index::inspect_vp9_unit(&mut Cursor::new(&bytes[..bytes.len()-1]),&unit,&AtomicBool::new(false),&mut Frames::default()).unwrap_err().kind,index::Failure::Io);
    }

    #[cfg(windows)]
    fn stored_pair(video: Vec<u8>, audio: Vec<u8>) -> (tempfile::TempDir, SpoolIdentity) {
        use super::super::{Chunk, Mode, Owner, Runtime, SpoolWriter, Window, LIMITS};
        use std::sync::Arc;
        let directory=tempfile::tempdir().unwrap(); std::fs::create_dir(directory.path().join("youtube")).unwrap();
        let id=SpoolIdentity { job_id:uuid::Uuid::new_v4().to_string(),request_id:uuid::Uuid::new_v4().to_string(),receiver_id:uuid::Uuid::new_v4().to_string(),source_id:uuid::Uuid::new_v4().to_string(),document_id:uuid::Uuid::new_v4().to_string(),document_epoch:1,source_generation:1,video_id:"abcdefghijk".into(),channel_id:format!("UC{}","a".repeat(22)),owner:Owner::Manual };
        let clock=ClockDescriptor{video_timestamp_scale_ns:1_000_000,audio_timescale:44_100,mode:Mode::Segments,append_window:Window::Default,video_timestamp_offset:0,audio_timestamp_offset:0};
        let mut writer=SpoolWriter::create(directory.path(),id.clone(),clock,Arc::new(AtomicBool::new(false)),LIMITS,Runtime{available:Some(u64::MAX),elapsed:Some(0),..Runtime::default()}).unwrap();
        for (track,bytes) in [(Track::Video,video),(Track::Audio,audio)] {
            writer.append(&id,Chunk{track,append_index:0,chunk_index:0,final_chunk:true,bytes}).unwrap();
        }
        writer.seal(&id,SealCause::Stopped).unwrap(); drop(writer);
        (directory,id)
    }
    #[cfg(windows)]
    fn stored(video: Vec<u8>) -> (tempfile::TempDir, SequentialSource) {
        let (directory,id)=stored_pair(video,index::authored_pair()[1].clone());
        let source=SequentialSource::prepare(directory.path(),&id,&AtomicBool::new(false)).unwrap();
        (directory,source)
    }
    #[cfg(windows)]
    fn selected_source(second: Vec<u8>, third: Vec<u8>) -> (tempfile::TempDir, CompletedSelection) {
        let video=[video(&[(good_key(),true)],1000,false),second,third].concat();
        let mut audio=index::authored_pair()[1].clone();
        let at=audio.windows(4).position(|b| b==b"tfdt").unwrap()+8;
        // Just before1.033s through1.102s: excludes only original video unit0.
        audio[at..at+8].copy_from_slice(&45555u64.to_be_bytes());
        let (directory,id)=stored_pair(video,audio);
        let source=CompletedSelection::prepare(directory.path(),&id,&AtomicBool::new(false)).unwrap();
        (directory,source)
    }
    #[cfg(windows)]
    #[test]
    fn real_held_source_admission_matches_exact_both_tracks_and_rejects_other_job() {
        let (_dir,mut source)=stored(video(&[(good_key(),true)],1000,false));
        let cancel=AtomicBool::new(false); let admission=admit(&mut source,&cancel).unwrap();
        for track in [Track::Video,Track::Audio] {
            source.visit(track,&cancel,|span| { admission.check_span(&span).unwrap(); Ok(()) }).unwrap();
        }
        assert_eq!(admission.tracks[0].units,1); assert_ne!(admission.tracks[0].observations,[0;32]);
        let (_other,mut other)=stored(video(&[(good_key(),true)],1000,false));
        other.visit(Track::Video,&cancel,|span| { assert_eq!(admission.check_span(&span),Err(Fault::Binding)); Ok(()) }).unwrap();
        cancel.store(true,Ordering::Release);
        assert!(matches!(admit(&mut source,&cancel),Err(Fault::Cancelled)));
    }
    #[cfg(windows)]
    #[test]
    fn late_bad_unit_or_gap_never_constructs_source_admission() {
        for second in [video(&[(vec![0],false)],1033,false),video(&[(inter(true,None,None),false)],1034,false)] {
            let (_dir,mut source)=stored([video(&[(good_key(),true)],1000,false),second].concat());
            assert!(admit(&mut source,&AtomicBool::new(false)).is_err());
        }
    }
    #[cfg(windows)]
    #[test]
    fn selected_admission_starts_at_real_original_key_and_binds_both_tracks() {
        let (directory,mut source)=selected_source(video(&[(good_key(),true)],1033,false),video(&[(inter(true,None,None),false)],1066,false));
        let cancel=AtomicBool::new(false);
        let binding=source.binding(&cancel).unwrap();
        assert_eq!((binding.range(Track::Video).first_ordinal,binding.range(Track::Video).last_ordinal),(1,2));
        let admission=admit_selected(&mut source,&cancel).unwrap();
        admission.check_source(&source,&cancel).unwrap();
        assert_eq!(admission.tracks[0].units,2);
        assert_eq!(admission.tracks[0].first,1_033_000_000);
        for track in [Track::Video,Track::Audio] {
            source.visit(track,&cancel,|span| { admission.check_span(&span).unwrap(); Ok(()) }).unwrap();
        }
        let mut whole=SequentialSource::prepare(directory.path(),&source.facts().identity,&cancel).unwrap();
        whole.visit(Track::Video,&cancel,|span| {
            assert_eq!(admission.check_span(&span).is_ok(),span.facts().unit.ordinal!=0); Ok(())
        }).unwrap();
        let (_other,other)=selected_source(video(&[(good_key(),true)],1033,false),video(&[(inter(true,None,None),false)],1066,false));
        assert_eq!(admission.check_source(&other,&cancel),Err(Fault::Binding));
        assert_eq!(admission.check_source(&source,&AtomicBool::new(true)),Err(Fault::Cancelled));
        assert!(source.visit(Track::Audio,&cancel,|_| Err(Error::Proof)).is_err());
        assert_eq!(admission.check_source(&source,&cancel),Err(Fault::Binding));
    }
    #[cfg(windows)]
    #[test]
    fn selected_admission_cannot_borrow_excluded_references_or_container_key_claim() {
        for second in [inter(true,None,None),existing(0)] {
            let (directory,mut selected)=selected_source(video(&[(second,true)],1033,false),video(&[(inter(true,None,None),false)],1066,false));
            let cancel=AtomicBool::new(false);
            let mut whole=SequentialSource::prepare(directory.path(),&selected.facts().identity,&cancel).unwrap();
            // Existing Whole admission can use the earlier actual key, while a
            // selected run must not inherit those discarded reference frames.
            admit(&mut whole,&cancel).unwrap();
            assert!(matches!(admit_selected(&mut selected,&cancel),Err(Fault::Unsupported)));
            assert!(selected.check(&cancel).is_err());
        }
    }
    #[cfg(windows)]
    #[test]
    fn late_bad_selected_payload_and_pre_cancel_do_not_create_admission() {
        let (_dir,mut source)=selected_source(video(&[(good_key(),true)],1033,false),video(&[(vec![0],false)],1066,false));
        assert!(admit_selected(&mut source,&AtomicBool::new(false)).is_err());
        assert!(source.binding(&AtomicBool::new(false)).is_err());
        let (_dir,mut source)=selected_source(video(&[(good_key(),true)],1033,false),video(&[(inter(true,None,None),false)],1066,false));
        assert!(matches!(admit_selected(&mut source,&AtomicBool::new(true)),Err(Fault::Cancelled)));
    }
}
struct TrackProof {
    configuration: Configuration,
    units: u64,
    end: u64,
    first: i128,
    last_end: i128,
    clock_scale: u64,
    observations: [u8; 32],
}

/// Private constructor, non-Clone and non-serializable. Raw hashes bind the
/// entire admitted unit sequence; a caller's copied Unit/SpanFacts is not a lease.
pub(super) struct SourceAdmission {
    scope: Scope,
    tracks: [TrackProof; 2],
}
impl SourceAdmission {
    pub(super) fn check_span(&self, span: &UnitSpan<'_>) -> Result<()> {
        span.check().map_err(source_error)?;
        let facts = span.facts();
        let proof = &self.tracks[facts.track.index()];
        let unit = facts.unit;
        if Scope::of(&facts) != self.scope || unit.ordinal >= proof.units
            || unit.facts.configuration != proof.configuration
            || unit.facts.start >= unit.facts.init_end || unit.facts.init_end >= unit.facts.end
            || unit.facts.end > proof.end || unit.facts.clock.ticks_per_second != proof.clock_scale
            || unit.facts.clock.first < proof.first || unit.facts.clock.end > proof.last_end
            || !unit.codec_packet_validation_pending
        { return Err(Fault::Binding); }
        span.check().map_err(source_error)
    }
}

struct Accumulator {
    first_ordinal: u64, count: u64, end: u64, first: i128, last_end: i128, scale: u64,
    configuration: Option<Configuration>, hash: Sha256,
}
impl Accumulator {
    fn new(track: Track) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"atsumi-vp9-source-units-v1\0");
        hash.update([track.index() as u8]);
        Self { first_ordinal: 0, count: 0, end: 0, first: 0, last_end: 0, scale: 0, configuration: None, hash }
    }
    fn selected(track: Track, binding: &SelectionBinding) -> Self {
        let mut accumulator = Self::new(track);
        let range = binding.range(track);
        accumulator.first_ordinal = range.first_ordinal;
        accumulator.end = range.start;
        accumulator
    }
    fn push(&mut self, scope: &Scope, track: Track, unit: &Unit) -> Result<()> {
        let f = &unit.facts;
        let (format, timebase, scale) = match track {
            Track::Video => (Format::Vp9Webm, scope.clock.video_timestamp_scale_ns, 1_000_000_000),
            Track::Audio => (Format::AacMp4, scope.clock.audio_timescale, scope.clock.audio_timescale),
        };
        if self.count >= MAX_UNITS || Some(unit.ordinal) != self.first_ordinal.checked_add(self.count) || f.start != self.end
            || f.start >= f.init_end || f.init_end >= f.end || f.end > scope.tracks[track.index()].bytes
            || f.configuration.format != format || f.configuration.timebase != timebase
            || f.clock.ticks_per_second != scale || f.clock.first > f.clock.last
            || f.clock.last >= f.clock.end || f.items == 0 || f.fragments == 0
            || !unit.codec_packet_validation_pending
            || self.configuration.as_ref().is_some_and(|c| c != &f.configuration)
            || self.count != 0 && self.last_end != f.clock.first
        { return Err(Fault::Binding); }
        if self.count == 0 { self.first = f.clock.first; self.scale = scale; }
        self.configuration.get_or_insert_with(|| f.configuration.clone());
        for n in [unit.ordinal, f.start, f.init_end, f.end, f.items, f.fragments, f.references, f.clock.ticks_per_second] {
            self.hash.update(n.to_be_bytes());
        }
        self.hash.update(f.init_sha256); self.hash.update(unit.configuration_sha256);
        for n in [f.clock.first, f.clock.last, f.clock.end] { self.hash.update(n.to_be_bytes()); }
        self.hash.update([u8::from(f.rap_candidate)]);
        self.end = f.end; self.last_end = f.clock.end; self.count += 1;
        Ok(())
    }
    fn finish(self, bytes: u64) -> Result<TrackProof> {
        if self.count == 0 || self.end != bytes { return Err(Fault::Binding); }
        Ok(TrackProof { configuration: self.configuration.ok_or(Fault::Binding)?, units: self.count,
            end: self.end, first: self.first, last_end: self.last_end, clock_scale: self.scale,
            observations: self.hash.finalize().into() })
    }
}

fn inspect_span(
    span: &mut UnitSpan<'_>, unit: &Unit, cancel: &AtomicBool, frames: &mut Frames,
) -> Result<()> {
    let mut reader = span.take_reader().map_err(source_error)?;
    let position = reader.stream_position().map_err(|_| Fault::Io)?;
    let inspected = index::inspect_vp9_unit(&mut reader, unit, cancel, frames).map_err(index_error);
    let restored = reader.seek(SeekFrom::Start(position)).map_err(|_| Fault::Io);
    span.check().map_err(source_error)?;
    restored?;
    inspected
}

pub(super) fn admit(source: &mut SequentialSource, cancel: &AtomicBool) -> Result<SourceAdmission> {
    checkpoint(cancel)?;
    let mut scope: Option<Scope> = None;
    let mut frames = Frames::default();
    let mut accumulators = [Accumulator::new(Track::Video), Accumulator::new(Track::Audio)];
    for track in [Track::Video, Track::Audio] {
        let mut failure = None;
        let visited = source.visit(track, cancel, |mut span| {
            let worked = (|| {
                checkpoint(cancel)?; span.check().map_err(source_error)?;
                let facts = span.facts();
                let candidate = Scope::of(&facts);
                candidate.identity.validate().map_err(source_error)?;
                candidate.clock.validate().map_err(source_error)?;
                if matches!(candidate.state, PrefixState::Cancelled | PrefixState::Corrupt)
                    || candidate.seal == Some(SealCause::Cancelled)
                    || scope.as_ref().is_some_and(|known| known != &candidate)
                { return Err(Fault::Binding); }
                let unit = facts.unit.clone();
                accumulators[track.index()].push(&candidate, track, &unit)?;
                scope.get_or_insert(candidate);
                if track == Track::Video {
                    inspect_span(&mut span, &unit, cancel, &mut frames)?;
                }
                checkpoint(cancel)?; span.check().map_err(source_error)
            })();
            if let Err(error) = worked { failure = Some(error); Err(Error::Proof) } else { Ok(()) }
        });
        if let Some(error) = failure { return Err(error); }
        visited.map_err(source_error)?;
    }
    checkpoint(cancel)?;
    if frames.blocks == 0 || frames.displayed == 0 { return Err(Fault::Unsupported); }
    let scope = scope.ok_or(Fault::Binding)?;
    let [video, audio] = accumulators;
    let tracks = [video.finish(scope.tracks[0].bytes)?, audio.finish(scope.tracks[1].bytes)?];
    source.check(cancel).map_err(source_error)?;
    checkpoint(cancel)?;
    Ok(SourceAdmission { scope, tracks })
}

/// Distinct from Whole-Complete admission. This never grants publication or
/// changes a selected range into a complete original recording.
pub(super) struct SelectedAdmission {
    binding: SelectionBinding,
    tracks: [TrackProof; 2],
}
impl SelectedAdmission {
    pub(super) fn check_source(&self, source: &CompletedSelection, cancel: &AtomicBool) -> Result<()> {
        if source.binding(cancel).map_err(selection_error)? != self.binding {
            return Err(Fault::Binding);
        }
        Ok(())
    }
    pub(super) fn check_span(&self, span: &UnitSpan<'_>) -> Result<()> {
        self.binding.check_span(span).map_err(source_error)?;
        let facts = span.facts();
        let unit = facts.unit;
        let proof = &self.tracks[facts.track.index()];
        let range = self.binding.range(facts.track);
        if unit.ordinal.checked_sub(range.first_ordinal).is_none_or(|n| n >= proof.units)
            || unit.facts.configuration != proof.configuration
            || unit.facts.end > proof.end
            || unit.facts.clock.ticks_per_second != proof.clock_scale
            || unit.facts.clock.first < proof.first || unit.facts.clock.end > proof.last_end
        { return Err(Fault::Binding); }
        span.check().map_err(source_error)
    }
}

pub(super) fn admit_selected(source: &mut CompletedSelection, cancel: &AtomicBool) -> Result<SelectedAdmission> {
    checkpoint(cancel)?;
    let binding = source.binding(cancel).map_err(selection_error)?;
    let mut accumulators = [Accumulator::selected(Track::Video, &binding), Accumulator::selected(Track::Audio, &binding)];
    let mut frames = Frames::default();
    for track in [Track::Video, Track::Audio] {
        let mut failure = None;
        let visited = source.visit(track, cancel, |mut span| {
            let worked = (|| {
                checkpoint(cancel)?;
                binding.check_span(&span).map_err(source_error)?;
                let facts = span.facts();
                let scope = Scope::of(&facts);
                let unit = facts.unit.clone();
                accumulators[track.index()].push(&scope, track, &unit)?;
                if track == Track::Video { inspect_span(&mut span, &unit, cancel, &mut frames)?; }
                checkpoint(cancel)?;
                span.check().map_err(source_error)
            })();
            if let Err(error) = worked { failure = Some(error); Err(Error::Proof) } else { Ok(()) }
        });
        if let Some(error) = failure { return Err(error); }
        visited.map_err(selection_error)?;
    }
    checkpoint(cancel)?;
    if frames.blocks == 0 || frames.displayed == 0 { return Err(Fault::Unsupported); }
    let [video, audio] = accumulators;
    let tracks = [video.finish(binding.range(Track::Video).end)?, audio.finish(binding.range(Track::Audio).end)?];
    for track in [Track::Video, Track::Audio] {
        let range = binding.range(track);
        let proof = &tracks[track.index()];
        let count = range.last_ordinal.checked_sub(range.first_ordinal)
            .and_then(|n| n.checked_add(1)).ok_or(Fault::Bounds)?;
        if proof.units != count || proof.first != range.first_tick || proof.last_end != range.end_tick
            || proof.clock_scale != range.ticks_per_second
        { return Err(Fault::Binding); }
    }
    let admission = SelectedAdmission { binding, tracks };
    admission.check_source(source, cancel)?;
    checkpoint(cancel)?;
    Ok(admission)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Picture { width: u32, height: u32 }
#[derive(Clone, Default)]
pub(super) struct Frames {
    references: [Option<Picture>; 8],
    picture: Option<Picture>,
    blocks: u64,
    coded: u64,
    displayed: u64,
}
struct Bits<'a> { bytes: &'a [u8], bit: usize }
impl Bits<'_> {
    fn read(&mut self, n: usize) -> Result<u32> {
        if n > 32 || self.bit.checked_add(n).is_none_or(|end| end > self.bytes.len() * 8) {
            return Err(Fault::Malformed);
        }
        let mut value = 0;
        for _ in 0..n {
            value = (value << 1) | u32::from((self.bytes[self.bit / 8] >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Ok(value)
    }
    fn size(&mut self) -> Result<Picture> { Ok(Picture { width: self.read(16)? + 1, height: self.read(16)? + 1 }) }
}
fn spans(length: u64, tail: &[u8]) -> Result<([(u64, u64); 8], usize)> {
    if length == 0 || length > index::MAX_ITEM || tail.len() != length.min(34) as usize {
        return Err(Fault::Bounds);
    }
    let mut out = [(0, 0); 8];
    let marker = *tail.last().ok_or(Fault::Malformed)?;
    if marker & 0xe0 != 0xc0 { out[0] = (0, length); return Ok((out, 1)); }
    let count = usize::from(marker & 7) + 1;
    let magnitude = usize::from((marker >> 3) & 3) + 1;
    let size = 2 + count * magnitude;
    if size > tail.len() || tail[tail.len() - size] != marker { return Err(Fault::Malformed); }
    let mut cursor = tail.len() - size + 1;
    let mut offset = 0u64;
    for slot in out.iter_mut().take(count) {
        let mut bytes = 0u64;
        for shift in 0..magnitude { bytes |= u64::from(tail[cursor]) << (8 * shift); cursor += 1; }
        let end = offset.checked_add(bytes).filter(|end| *end <= length - size as u64).ok_or(Fault::Malformed)?;
        if bytes == 0 { return Err(Fault::Malformed); }
        *slot = (offset, end); offset = end;
    }
    if offset != length - size as u64 { return Err(Fault::Malformed); }
    Ok((out, count))
}
impl Frames {
    fn frame(&mut self, prefix: &[u8], expected: Picture, container_rap: bool) -> Result<()> {
        if prefix.len() > 32 || self.coded >= MAX_FRAMES { return Err(Fault::Bounds); }
        if self.picture.is_some_and(|known| known != expected) { return Err(Fault::Binding); }
        let mut bits = Bits { bytes: prefix, bit: 0 };
        if bits.read(2)? != 2 { return Err(Fault::Malformed); }
        let profile = bits.read(1)? | (bits.read(1)? << 1);
        if profile != 0 { return Err(Fault::Unsupported); }
        if bits.read(1)? != 0 {
            let index = bits.read(3)? as usize;
            if self.coded == 0 || self.references[index] != Some(expected) { return Err(Fault::Unsupported); }
            self.coded += 1; self.displayed += 1;
            return Ok(());
        }
        let key = bits.read(1)? == 0;
        let show = bits.read(1)? != 0;
        let resilient = bits.read(1)? != 0;
        if self.coded == 0 && (!key || !container_rap) { return Err(Fault::Unsupported); }
        let refresh;
        let picture;
        if key {
            if bits.read(24)? != 0x498342 { return Err(Fault::Malformed); }
            // Profile0 implies 8-bit 4:2:0. This initial supported subset is
            // exactly the existing codec metadata gate: BT.709/studio range.
            if bits.read(3)? != 2 || bits.read(1)? != 0 { return Err(Fault::Unsupported); }
            picture = bits.size()?;
            refresh = 0xff;
        } else {
            let intra_only = !show && bits.read(1)? != 0;
            if !resilient { let _ = bits.read(2)?; }
            // Profile0 intra-only normatively introduces BT.601, not the prior
            // key's color. It cannot silently inherit this BT.709 admission.
            if intra_only { return Err(Fault::Unsupported); }
            refresh = bits.read(8)? as u8;
            let mut references = [expected; 3];
            for slot in &mut references {
                let index = bits.read(3)? as usize;
                *slot = self.references[index].ok_or(Fault::Unsupported)?;
                let _sign_bias = bits.read(1)?;
                if *slot != expected { return Err(Fault::Unsupported); }
            }
            let mut selected = None;
            for reference in references {
                if bits.read(1)? != 0 { selected = Some(reference); break; }
            }
            picture = match selected { Some(picture) => picture, None => bits.size()? };
        }
        if picture != expected { return Err(Fault::Unsupported); }
        let render = if bits.read(1)? != 0 { bits.size()? } else { picture };
        if render != expected { return Err(Fault::Unsupported); }
        for (i, slot) in self.references.iter_mut().enumerate() {
            if refresh & (1 << i) != 0 { *slot = Some(picture); }
        }
        self.picture = Some(picture); self.coded += 1;
        self.displayed += u64::from(show);
        Ok(())
    }

    pub(super) fn packet<R: Read + Seek>(
        &mut self, input: &mut index::Input<'_, R>, config: &Configuration,
        start: u64, end: u64, rap: bool,
    ) -> index::Result<()> {
        use index::{error, Failure};
        if start >= end || end > input.len || self.blocks >= MAX_BLOCKS
            || config.format != Format::Vp9Webm || config.width == 0 || config.height == 0
            || config.width > 16384 || config.height > 16384
        { return Err(error(Failure::Limit, start)); }
        let size = end - start;
        let tail = input.bytes(end - size.min(34), size.min(34) as usize)?;
        let map_fault = |fault| error(match fault {
            Fault::Bounds => Failure::Limit, Fault::Io => Failure::Io,
            Fault::Cancelled => Failure::Cancelled, Fault::Malformed => Failure::Malformed,
            _ => Failure::Unsupported,
        }, start);
        let (ranges, count) = spans(size, &tail).map_err(map_fault)?;
        let mut working = self.clone();
        for (begin, finish) in ranges.into_iter().take(count) {
            input.checkpoint(start + begin)?;
            let prefix = input.bytes(start + begin, (finish - begin).min(32) as usize)?;
            working.frame(&prefix, Picture { width: config.width, height: config.height }, rap).map_err(map_fault)?;
        }
        input.checkpoint(end)?;
        working.blocks += 1; *self = working;
        Ok(())
    }
}
