use super::super::super::{Chunk, Mode, Owner, Runtime, SpoolWriter, Window};
use super::*;
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    sync::{atomic::Ordering as AtomicOrdering, Arc},
};

fn unique(bytes: &[u8], marker: &[u8]) -> usize {
    let found = bytes
        .windows(marker.len())
        .enumerate()
        .filter_map(|(n, b)| (b == marker).then_some(n))
        .collect::<Vec<_>>();
    assert_eq!(found.len(), 1, "authored fixture marker must be unique");
    found[0]
}
// These are container-only authored bytes, not decodable VP9/AAC fixtures.
fn video(ms: u16) -> Vec<u8> {
    let mut bytes = index::authored_pair()[0].clone();
    let at = unique(&bytes, &[0xe7, 0x82, 3, 232]);
    bytes[at + 2..at + 4].copy_from_slice(&ms.to_be_bytes());
    bytes
}
fn audio(tick: u64) -> Vec<u8> {
    let mut bytes = index::authored_pair()[1].clone();
    let at = unique(&bytes, b"tfdt");
    bytes[at + 8..at + 16].copy_from_slice(&tick.to_be_bytes());
    let at = unique(&bytes, b"trex");
    // Three authored structural samples span exactly100ms. No AAC codec claim.
    bytes[at + 16..at + 20].copy_from_slice(&1470u32.to_be_bytes());
    bytes
}
fn pair(count: u16) -> [Vec<u8>; 2] {
    [
        (0..count).flat_map(|n| video(1000 + n * 100)).collect(),
        (0..count)
            .flat_map(|n| audio(44100 + n as u64 * 4410))
            .collect(),
    ]
}
struct Fixture {
    root: tempfile::TempDir,
    id: SpoolIdentity,
    limits: Limits,
}
impl Fixture {
    fn new(bytes: [Vec<u8>; 2], incomplete_audio: bool, seal: Option<SealCause>) -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("YouTube")).unwrap();
        let id = SpoolIdentity {
            job_id: uuid::Uuid::new_v4().to_string(),
            request_id: uuid::Uuid::new_v4().to_string(),
            receiver_id: uuid::Uuid::new_v4().to_string(),
            source_id: uuid::Uuid::new_v4().to_string(),
            document_id: uuid::Uuid::new_v4().to_string(),
            document_epoch: 1,
            source_generation: 1,
            video_id: "abcdefghijk".into(),
            channel_id: format!("UC{}", "a".repeat(22)),
            owner: Owner::Manual,
        };
        let clock = ClockDescriptor {
            video_timestamp_scale_ns: 1_000_000,
            audio_timescale: 44100,
            mode: Mode::Segments,
            append_window: Window::Default,
            video_timestamp_offset: 0,
            audio_timestamp_offset: 0,
        };
        let mut limits = LIMITS;
        if incomplete_audio {
            limits.chunk = bytes[1].len() as u64;
        }
        let mut writer = SpoolWriter::create(
            root.path(),
            id.clone(),
            clock,
            Arc::new(AtomicBool::new(false)),
            limits,
            Runtime {
                elapsed: Some(0),
                available: Some(u64::MAX),
                ..Default::default()
            },
        )
        .unwrap();
        for (track, bytes) in [(Track::Video, &bytes[0]), (Track::Audio, &bytes[1])] {
            let chunks = bytes.chunks(limits.chunk as usize);
            let count = chunks.len();
            for (n, chunk) in chunks.enumerate() {
                writer
                    .append(
                        &id,
                        Chunk {
                            track,
                            append_index: 0,
                            chunk_index: n as u32,
                            final_chunk: n + 1 == count
                                && !(incomplete_audio && track == Track::Audio),
                            bytes: chunk.to_vec(),
                        },
                    )
                    .unwrap();
            }
        }
        if let Some(cause) = seal {
            writer.seal(&id, cause).unwrap();
        }
        drop(writer);
        Self { root, id, limits }
    }
    fn dir(&self) -> std::path::PathBuf {
        self.root.path().join("YouTube").join(&self.id.job_id)
    }
    fn select(&self, cancel: &AtomicBool) -> Result<CompletedSelection> {
        CompletedSelection::prepare_inner(self.root.path(), &self.id, cancel, self.limits)
    }
}
fn success(bytes: [Vec<u8>; 2]) -> (Fixture, CompletedSelection) {
    let f = Fixture::new(bytes, false, Some(SealCause::Stopped));
    let selected = f.select(&AtomicBool::new(false)).unwrap();
    (f, selected)
}

// Packet algebra fixtures: these use the native authored binding but are not
// claims that the arbitrary packet hashes are decodable source media.
use super::super::super::{
    finalize::{aac_selection::{AacRetention, Partition, RetainedComparison}, Fault as FinalFault},
    packet_facts::{PacketFact, Rational, TimeBase, TrackFold},
    packet_tape,
    stream_facts::{ProbeFacts, StreamFacts},
};
use std::{io::Cursor, time::{Duration, Instant}};
fn aac_binding(start: u16, count: u16) -> (Fixture, CompletedSelection) {
    let mut a = audio(44100);
    let at = unique(&a, b"trex") + 16;
    a[at..at + 4].copy_from_slice(&4410u32.to_be_bytes());
    success([(0..count).flat_map(|n| video(start + n * 100)).collect(), a])
}
fn aac_binding_durations(start: u16, count: u16, audio_start: u64, durations: &[i64]) -> (Fixture, CompletedSelection) {
    let durations = durations.iter().map(|&x| u32::try_from(x).unwrap()).collect::<Vec<_>>();
    success([(0..count).flat_map(|n| video(start + n * 100)).collect(),
        index::authored_audio_durations(audio_start, &durations)])
}
fn aac_packets(durations: &[i64], start: i64) -> Vec<PacketFact> {
    let mut pts = start;
    durations.iter().enumerate().map(|(n, duration)| {
        let mut record = [0; 80];
        record[0] = 1; record[1] = 1;
        record[4..12].copy_from_slice(&pts.to_be_bytes());
        record[12..20].copy_from_slice(&pts.to_be_bytes());
        record[20..28].copy_from_slice(&duration.to_be_bytes());
        record[28..36].copy_from_slice(&372u64.to_be_bytes());
        record[36..68].fill((n + 1) as u8);
        pts += duration;
        PacketFact::decode_record(&record).unwrap()
    }).collect()
}
fn packet_change(packet: PacketFact, change: impl FnOnce(&mut [u8; 80])) -> PacketFact {
    let mut record = packet.encode_record().unwrap();
    change(&mut record);
    PacketFact::decode_record(&record).unwrap()
}
fn aac_facts(packets: &[PacketFact], timebase: TimeBase) -> ProbeFacts {
    let mut fold = TrackFold::new(Track::Audio);
    for &packet in packets { fold.push(packet).unwrap(); }
    ProbeFacts { packets: fold.finish().unwrap(), stream: StreamFacts {
        track: Track::Audio, time_base: timebase, width: 0, height: 0, sample_rate: 44100, channels: 2,
    } }
}
fn aac_tape(packets: &[PacketFact]) -> (Vec<u8>, packet_tape::Summary) {
    let mut bytes = Vec::new();
    let mut writer = packet_tape::Writer::new(&mut bytes, Track::Audio).unwrap();
    for &packet in packets { writer.push(packet).unwrap(); }
    let summary = writer.finish(&aac_facts(packets, TimeBase::new(1, 44100).unwrap())).unwrap();
    drop(writer);
    (bytes, summary)
}
fn aac_plan(binding: &SelectionBinding, packets: &[PacketFact], summary: &packet_tape::Summary) -> AacRetention {
    let cancel = AtomicBool::new(false);
    let mut partition = Partition::new(binding, TimeBase::new(1, 44100).unwrap(), &cancel, Instant::now() + Duration::from_secs(60)).unwrap();
    for &packet in packets { partition.push(packet).unwrap(); }
    partition.finish(summary).unwrap()
}
fn compare_aac(
    bytes: &[u8], summary: &packet_tape::Summary, plan: &AacRetention,
    binding: &SelectionBinding, output: &[PacketFact], origin: Rational,
) -> std::result::Result<super::super::super::packet_facts::PairReport, FinalFault> {
    let cancel = AtomicBool::new(false);
    let base = TimeBase::new(1, 44100).unwrap();
    let mut input = Cursor::new(bytes);
    let mut comparison = RetainedComparison::new(&mut input, summary, plan, binding, base, origin, &cancel, Instant::now() + Duration::from_secs(60))?;
    for &packet in output { comparison.push(packet)?; }
    comparison.finish(&aac_facts(output, base))
}

#[test]
fn aac_partition_exact_boundaries_and_all_four_exclusion_shapes() {
    let packets = aac_packets(&[4410; 3], 44100);
    let (tape, summary) = aac_tape(&packets);
    for (start, count, ordinals) in [(1000, 3, (0, 3)), (1100, 2, (1, 3)), (1000, 2, (0, 2)), (1100, 1, (1, 2))] {
        let (_fixture, source) = aac_binding(start, count);
        let binding = source.binding(&AtomicBool::new(false)).unwrap();
        let plan = aac_plan(&binding, &packets, &summary);
        assert_eq!(plan.ordinals(), ordinals);
        assert_eq!(plan.leading().map(|f| f.count), (ordinals.0 > 0).then_some(ordinals.0));
        assert_eq!(plan.trailing().map(|f| f.count), (ordinals.1 < 3).then_some(3 - ordinals.1));
        assert_eq!(plan.retained().bytes, 372 * (ordinals.1 - ordinals.0));
        assert_eq!(plan.input(), &summary);
        let output = &packets[ordinals.0 as usize..ordinals.1 as usize];
        let report = compare_aac(&tape, &summary, &plan, &binding, output, Rational::new(0, 1).unwrap()).unwrap();
        assert_eq!(report.count, ordinals.1 - ordinals.0);
    }
}

#[test]
fn aac_one_tick_intersection_is_not_rounded_to_video_nanoseconds() {
    for (durations, expected) in [([4410,4410,4410], (1,2)), ([4411,4409,4410], (0,2)),
        ([4409,4411,4410], (1,2)), ([4410,4411,4409], (1,2)), ([4410,4409,4411], (1,3))] {
        let (_fixture, source) = aac_binding_durations(1100, 1, 44100, &durations);
        let binding = source.binding(&AtomicBool::new(false)).unwrap();
        let packets = aac_packets(&durations, 44100);
        let (bytes, summary) = aac_tape(&packets);
        let plan = aac_plan(&binding, &packets, &summary);
        assert_eq!(plan.ordinals(), expected);
        compare_aac(&bytes, &summary, &plan, &binding, &packets[expected.0 as usize..expected.1 as usize], Rational::new(0,1).unwrap()).unwrap();
    }
}

#[test]
fn aac_packet_ordinals_start_zero_even_when_native_source_unit_starts_nonzero() {
    let (_fixture, source) = success([[video(1100),video(1200)].concat(), pair(4)[1].clone()]);
    let binding = source.binding(&AtomicBool::new(false)).unwrap();
    assert_eq!(binding.range(Track::Audio).first_ordinal, 1);
    let packets = aac_packets(&[1470; 6], 48510);
    let (bytes, summary) = aac_tape(&packets);
    let plan = aac_plan(&binding, &packets, &summary);
    assert_eq!(plan.ordinals(), (0,6));
    let shifted = packets.iter().map(|&packet| packet_change(packet, |record| {
        for at in [4, 12] {
            let pts = i64::from_be_bytes(record[at..at+8].try_into().unwrap()) - 48510;
            record[at..at+8].copy_from_slice(&pts.to_be_bytes());
        }
    })).collect::<Vec<_>>();
    compare_aac(&bytes,&summary,&plan,&binding,&shifted,Rational::new(48510,44100).unwrap()).unwrap();
}

#[test]
fn aac_partition_rejects_invalid_clock_side_data_scope_and_terminal_reuse() {
    let (_fixture, source) = aac_binding(1100, 1);
    let binding = source.binding(&AtomicBool::new(false)).unwrap();
    let packets = aac_packets(&[4410;3],44100);
    let (_, summary) = aac_tape(&packets);
    let cancel = AtomicBool::new(false);
    let deadline = Instant::now()+Duration::from_secs(60);
    let base = TimeBase::new(1,44100).unwrap();
    assert!(Partition::new(&binding,TimeBase::new(1,48000).unwrap(),&cancel,deadline).is_err());
    let mut empty = Partition::new(&binding,base,&cancel,deadline).unwrap();
    assert!(empty.finish(&summary).is_err());
    for field in 0..3 {
        let mut facts = aac_facts(&packets,base);
        match field { 0 => facts.stream.channels=1, 1 => facts.stream.width=1, _ => facts.stream.time_base=TimeBase::new(1,48000).unwrap() }
        let mut data=Vec::new();
        let mut writer=packet_tape::Writer::new(&mut data,Track::Audio).unwrap();
        for &packet in &packets {writer.push(packet).unwrap();}
        let changed=writer.finish(&facts).unwrap();
        let mut partition=Partition::new(&binding,base,&cancel,deadline).unwrap();
        for &packet in &packets {partition.push(packet).unwrap();}
        assert!(partition.finish(&changed).is_err());
    }
    let variants = [
        packet_change(packets[0],|r| r[0]=0),
        packet_change(packets[0],|r| {r[4..12].copy_from_slice(&(-1i64).to_be_bytes());r[12..20].copy_from_slice(&(-1i64).to_be_bytes());}),
        packet_change(packets[0],|r| r[12..20].copy_from_slice(&44101i64.to_be_bytes())),
        packet_change(packets[0],|r| {r[2]=1;r[71]=1;}),
        packet_change(packets[0],|r| {r[2]=1;r[75]=1;}),
        packet_change(packets[0],|r| {r[2]=1;r[76]=1;}),
    ];
    for bad in variants {
        let mut partition = Partition::new(&binding,base,&cancel,deadline).unwrap();
        let error = partition.push(bad).unwrap_err();
        assert_eq!(partition.push(packets[0]),Err(error));
        assert_eq!(partition.finish(&summary).unwrap_err(),error);
    }
    for shift in [-1i64,1] {
        let mut partition=Partition::new(&binding,base,&cancel,deadline).unwrap();
        partition.push(packets[0]).unwrap();
        let changed=packet_change(packets[1],|r| for at in [4,12] {
            r[at..at+8].copy_from_slice(&(48510+shift).to_be_bytes());
        });
        assert!(partition.push(changed).is_err());
    }
    for subset in [&packets[..1], &packets[1..]] {
        let (_,short)=aac_tape(subset);
        let mut partition=Partition::new(&binding,base,&cancel,deadline).unwrap();
        for &packet in subset {partition.push(packet).unwrap();}
        assert!(partition.finish(&short).is_err());
    }
    let mut partition=Partition::new(&binding,base,&cancel,deadline).unwrap();
    for &packet in &packets {partition.push(packet).unwrap();}
    partition.finish(&summary).unwrap();
    assert_eq!(partition.finish(&summary).unwrap_err(),FinalFault::Terminal);
    assert_eq!(partition.push(packets[0]),Err(FinalFault::Terminal));
}

#[test]
fn aac_partition_keeps_present_zero_side_data_distinct() {
    let (_fixture, source)=aac_binding(1100,1);
    let binding=source.binding(&AtomicBool::new(false)).unwrap();
    let mut packets=aac_packets(&[4410;3],44100);
    packets[1]=packet_change(packets[1],|r|r[2]=1);
    let (bytes,summary)=aac_tape(&packets);
    let plan=aac_plan(&binding,&packets,&summary);
    compare_aac(&bytes,&summary,&plan,&binding,&packets[1..2],Rational::new(0,1).unwrap()).unwrap();
    let absent=packet_change(packets[1],|r|r[2]=0);
    assert!(compare_aac(&bytes,&summary,&plan,&binding,&[absent],Rational::new(0,1).unwrap()).is_err());
}

#[test]
fn aac_comparison_rejects_candidate_changes_missing_extra_and_bad_probe_facts() {
    let (_fixture,source)=aac_binding_durations(1100,1,44100,&[2205;6]);
    let binding=source.binding(&AtomicBool::new(false)).unwrap();
    let packets=aac_packets(&[2205;6],44100);
    let (bytes,summary)=aac_tape(&packets);
    let plan=aac_plan(&binding,&packets,&summary);
    assert_eq!(plan.ordinals(),(2,4));
    let output=packets[2..4].to_vec();
    let mut bad=Vec::new();
    bad.push(vec![output[0]]);
    bad.push(vec![output[0],output[1],output[1]]);
    bad.push(vec![output[1],output[0]]);
    bad.push(vec![output[0],output[0]]);
    for changed in [
        packet_change(output[0],|r|r[36]^=1),
        packet_change(output[0],|r|r[1]=0),
        packet_change(output[0],|r|r[2]=1),
        packet_change(output[0],|r| for at in [4,12] {r[at..at+8].copy_from_slice(&48513i64.to_be_bytes());}),
    ] {bad.push(vec![changed,output[1]]);}
    for changed in bad {assert!(compare_aac(&bytes,&summary,&plan,&binding,&changed,Rational::new(0,1).unwrap()).is_err());}
    let cancel=AtomicBool::new(false);
    let deadline=Instant::now()+Duration::from_secs(60);
    let base=TimeBase::new(1,44100).unwrap();
    let facts=aac_facts(&output,base);
    let mut bad=facts.clone();bad.packets.digest[0]^=1;
    let mut wrong=facts.clone();wrong.stream.channels=1;
    for changed in [bad,wrong] {
        let mut input=Cursor::new(&bytes);
        let mut comparison=RetainedComparison::new(&mut input,&summary,&plan,&binding,base,Rational::new(0,1).unwrap(),&cancel,deadline).unwrap();
        for &packet in &output {comparison.push(packet).unwrap();}
        let error=comparison.finish(&changed).unwrap_err();
        assert_eq!(comparison.finish(&facts).unwrap_err(),error);
        assert_eq!(comparison.push(output[0]),Err(error));
    }
}

#[test]
fn aac_plan_matches_private_tape_hash_not_only_identical_fold_facts() {
    let (_fixture,source)=aac_binding_durations(1100,1,44100,&[1024,3072,2048,2048,1024,3072,942]);
    let binding=source.binding(&AtomicBool::new(false)).unwrap();
    let first=aac_packets(&[1024,3072,2048,2048,1024,3072,942],44100);
    let changed=aac_packets(&[1024,3072,2049,2047,1024,3072,942],44100);
    let (_,summary)=aac_tape(&first);
    let (other_bytes,other_summary)=aac_tape(&changed);
    assert_eq!(summary.facts(),other_summary.facts());
    assert_ne!(summary,other_summary);
    let plan=aac_plan(&binding,&first,&summary);
    let cancel=AtomicBool::new(false);
    let deadline=Instant::now()+Duration::from_secs(60);
    let mut input=Cursor::new(&other_bytes);
    assert!(RetainedComparison::new(&mut input,&other_summary,&plan,&binding,TimeBase::new(1,44100).unwrap(),Rational::new(0,1).unwrap(),&cancel,deadline).is_err());
    // Even lying about the summary cannot turn altered encoded rows into EOF.
    let (start,end)=plan.ordinals();
    assert!(compare_aac(&other_bytes,&summary,&plan,&binding,&changed[start as usize..end as usize],Rational::new(0,1).unwrap()).is_err());
    let (_other,other)=aac_binding(1100,1);
    assert_eq!(plan.check_binding(&other.binding(&cancel).unwrap()),Err(FinalFault::Identity));
    let mut changed_binding=binding.clone();
    changed_binding.facts.tracks[0].selected.first_tick+=1;
    assert_eq!(plan.check_binding(&changed_binding),Err(FinalFault::Identity));
}

#[test]
fn aac_comparison_checks_exclusions_and_physical_eof_not_only_retained_packets() {
    let (_fixture,source)=aac_binding(1100,1);
    let binding=source.binding(&AtomicBool::new(false)).unwrap();
    let packets=aac_packets(&[4410;3],44100);
    let (bytes,summary)=aac_tape(&packets);
    let plan=aac_plan(&binding,&packets,&summary);
    for end in [7,8,40,87,88,167,168,247] {
        assert!(compare_aac(&bytes[..end],&summary,&plan,&binding,&packets[1..2],Rational::new(0,1).unwrap()).is_err());
    }
    for row in 0..3 {
        let mut changed=bytes.clone();changed[8+row*80+36]^=1;
        assert!(compare_aac(&changed,&summary,&plan,&binding,&packets[1..2],Rational::new(0,1).unwrap()).is_err());
    }
    let mut extra=bytes.clone();extra.push(0);
    assert!(compare_aac(&extra,&summary,&plan,&binding,&packets[1..2],Rational::new(0,1).unwrap()).is_err());
    struct ErrorAtEof(Cursor<Vec<u8>>);
    impl Read for ErrorAtEof {
        fn read(&mut self, out:&mut[u8])->std::io::Result<usize> {
            if self.0.position()==self.0.get_ref().len() as u64 {return Err(std::io::Error::other("fixture EOF error"));}
            self.0.read(out)
        }
    }
    let cancel=AtomicBool::new(false);
    let base=TimeBase::new(1,44100).unwrap();
    let mut input=ErrorAtEof(Cursor::new(bytes));
    let mut comparison=RetainedComparison::new(&mut input,&summary,&plan,&binding,base,Rational::new(0,1).unwrap(),&cancel,Instant::now()+Duration::from_secs(60)).unwrap();
    comparison.push(packets[1]).unwrap();
    assert!(comparison.finish(&aac_facts(&packets[1..2],base)).is_err());
}

#[test]
fn aac_partition_and_comparison_final_cancel_deadline_are_sticky() {
    let (_fixture,source)=aac_binding(1100,1);
    let binding=source.binding(&AtomicBool::new(false)).unwrap();
    let packets=aac_packets(&[4410;3],44100);
    let (bytes,summary)=aac_tape(&packets);
    let plan=aac_plan(&binding,&packets,&summary);
    let base=TimeBase::new(1,44100).unwrap();
    let cancel=AtomicBool::new(false);
    let deadline=Instant::now()+Duration::from_secs(60);
    assert!(Partition::new(&binding,base,&cancel,Instant::now()).is_err());
    let mut partition=Partition::new(&binding,base,&cancel,deadline).unwrap();
    for &packet in &packets {partition.push(packet).unwrap();}
    cancel.store(true,AtomicOrdering::Release);
    assert_eq!(partition.finish(&summary).unwrap_err(),FinalFault::Cancelled);
    cancel.store(false,AtomicOrdering::Release);
    assert_eq!(partition.finish(&summary).unwrap_err(),FinalFault::Cancelled);
    let mut input=Cursor::new(&bytes);
    assert!(RetainedComparison::new(&mut input,&summary,&plan,&binding,base,Rational::new(0,1).unwrap(),&cancel,Instant::now()).is_err());
    let mut comparison=RetainedComparison::new(&mut input,&summary,&plan,&binding,base,Rational::new(0,1).unwrap(),&cancel,deadline).unwrap();
    comparison.push(packets[1]).unwrap();
    cancel.store(true,AtomicOrdering::Release);
    assert_eq!(comparison.finish(&aac_facts(&packets[1..2],base)).unwrap_err(),FinalFault::Cancelled);
    cancel.store(false,AtomicOrdering::Release);
    assert_eq!(comparison.finish(&aac_facts(&packets[1..2],base)).unwrap_err(),FinalFault::Cancelled);
}

#[test]
fn binding_keeps_original_coordinates_and_rejects_outside_or_foreign_spans() {
    use super::super::SequentialSource;
    let bytes = [pair(4)[0].clone(), [audio(48510), audio(52920)].concat()];
    let (fixture, mut selected) = success(bytes.clone());
    let cancel = AtomicBool::new(false);
    let binding = selected.binding(&cancel).unwrap();
    assert_eq!(binding.range(Track::Video).first_ordinal, 1);
    assert_eq!(binding.range(Track::Video).last_ordinal, 2);
    assert_eq!(binding.range(Track::Video).start, video(1000).len() as u64);
    for track in [Track::Video, Track::Audio] {
        selected.visit(track, &cancel, |span| binding.check_span(&span)).unwrap();
    }
    selected.check(&cancel).unwrap();
    assert_eq!(binding, selected.binding(&cancel).unwrap());
    let mut whole = SequentialSource::prepare(fixture.root.path(), &fixture.id, &cancel).unwrap();
    whole.visit(Track::Video, &cancel, |span| {
        let inside = (1..=2).contains(&span.facts().unit.ordinal);
        assert_eq!(binding.check_span(&span).is_ok(), inside);
        Ok(())
    }).unwrap();
    let (_other, mut other) = success(bytes);
    other.visit(Track::Video, &cancel, |span| {
        assert_eq!(binding.check_span(&span), Err(Error::Identity));
        Ok(())
    }).unwrap();
    assert_ne!(binding, other.binding(&cancel).unwrap());
}

#[test]
fn binding_rejects_changed_hash_clock_flags_and_exact_range() {
    let (_fixture, mut selected) = success(pair(2));
    let cancel = AtomicBool::new(false);
    let binding = selected.binding(&cancel).unwrap();
    // Only this private-module test may mutate a binding; production callers
    // cannot manufacture one from copied SelectionFacts or arbitrary ranges.
    let mut bad = Vec::new();
    let mut changed = binding.clone();
    changed.tracks[0].sha256 = "0".repeat(64);
    bad.push(changed);
    let mut changed = binding.clone();
    changed.facts.clock.video_timestamp_scale_ns += 1;
    bad.push(changed);
    let mut changed = binding.clone();
    changed.facts.torn_journal = true;
    bad.push(changed);
    let mut changed = binding.clone();
    changed.facts.tracks[0].selected.start += 1;
    bad.push(changed);
    let mut changed = binding.clone();
    changed.facts.tracks[0].selected.first_tick += 1;
    bad.push(changed);
    let mut changed = binding.clone();
    changed.facts.tracks[0].physical_bytes += 1;
    bad.push(changed);
    selected.visit(Track::Video, &cancel, |span| {
        if span.facts().unit.ordinal == 0 {
            for changed in &bad { assert!(changed.check_span(&span).is_err()); }
        }
        binding.check_span(&span)
    }).unwrap();
    assert_eq!(binding, selected.binding(&cancel).unwrap());
}

#[test]
fn binding_check_preserves_partial_tail_and_cannot_revive_poison_or_cancel() {
    let mut bytes = pair(2);
    bytes[0].extend([0x1a, 0x45]);
    let (_fixture, mut selected) = success(bytes);
    let cancel = AtomicBool::new(false);
    let binding = selected.binding(&cancel).unwrap();
    assert!(matches!(binding.scans[0].outcome.terminal, index::Terminal::PartialTail { .. }));
    selected.check(&cancel).unwrap();
    selected.visit(Track::Video, &cancel, |span| binding.check_span(&span)).unwrap();
    assert_eq!(binding, selected.binding(&cancel).unwrap());
    assert_eq!(selected.check(&AtomicBool::new(true)), Err(Fault::Source(Error::Cancelled)));
    assert!(selected.binding(&AtomicBool::new(true)).is_err());
    assert!(selected.visit(Track::Audio, &cancel, |_| Err(Error::Proof)).is_err());
    assert_eq!(selected.check(&cancel), Err(Fault::Poisoned));
    assert_eq!(selected.binding(&cancel), Err(Fault::Poisoned));
}

#[test]
fn selected_builder_uses_nonzero_original_video_or_audio_positions_without_rewriting_source() {
    use super::super::super::reconstruct::TrackBuilder;
    for bytes in [
        [pair(4)[0].clone(), [audio(48510), audio(52920)].concat()],
        [[video(1100), video(1200)].concat(), pair(4)[1].clone()],
    ] {
        let (_fixture, mut selected) = success(bytes.clone());
        let cancel = AtomicBool::new(false);
        let binding = selected.binding(&cancel).unwrap();
        assert!(binding.range(Track::Video).first_ordinal > 0 || binding.range(Track::Audio).first_ordinal > 0);
        for track in [Track::Video, Track::Audio] {
            let range = binding.range(track);
            let mut output = Vec::new();
            let mut expected = Vec::new();
            let mut count = 0u32;
            let mut builder = TrackBuilder::for_selection(track, &mut output, &binding);
            selected.visit(track, &cancel, |span| {
                let unit = &span.facts().unit.facts;
                let start = if count == 0 { unit.start } else { unit.init_end };
                let mut copy = bytes[track.index()][start as usize..unit.end as usize].to_vec();
                count += 1;
                if track == Track::Audio {
                    let at = unique(&copy, b"mfhd") + 8;
                    copy[at..at + 4].copy_from_slice(&count.to_be_bytes());
                }
                expected.extend(copy);
                builder.push(span, &cancel)
            }).unwrap();
            let summary = builder.finish(&cancel).unwrap();
            assert_eq!(summary.units, range.last_ordinal - range.first_ordinal + 1);
            assert_eq!(summary.clock.first, range.first_tick);
            assert_eq!(summary.clock.end, range.end_tick);
            drop(builder);
            assert_eq!(output, expected);
            selected.check(&cancel).unwrap();
        }
    }
}

#[test]
fn selected_builder_does_not_accept_whole_constructor_or_incomplete_selected_run() {
    use super::super::super::reconstruct::TrackBuilder;
    let bytes = [pair(4)[0].clone(), [audio(48510), audio(52920)].concat()];
    let (_fixture, mut selected) = success(bytes);
    let cancel = AtomicBool::new(false);
    let mut output = Vec::new();
    let mut builder = TrackBuilder::new(Track::Video, &mut output);
    assert!(selected.visit(Track::Video, &cancel, |span| builder.push(span, &cancel)).is_err());
    assert!(builder.finish(&cancel).is_err());
    drop(builder);
    assert!(output.is_empty());
    let bytes = [pair(4)[0].clone(), [audio(48510), audio(52920)].concat()];
    let (_fixture, mut selected) = success(bytes);
    let binding = selected.binding(&cancel).unwrap();
    let mut output = Vec::new();
    let mut builder = TrackBuilder::for_selection(Track::Video, &mut output, &binding);
    let mut count = 0;
    selected.visit(Track::Video, &cancel, |span| {
        count += 1;
        if count == 1 { builder.push(span, &cancel)?; }
        Ok(())
    }).unwrap();
    assert_eq!(builder.finish(&cancel).unwrap_err(), Error::Proof);
    assert_eq!(builder.finish(&cancel).unwrap_err(), Error::Proof);
}

#[test]
fn selected_builder_rejects_foreign_binding_and_cancel_without_output() {
    use super::super::super::reconstruct::TrackBuilder;
    let (_fixture, source) = success(pair(2));
    let cancel = AtomicBool::new(false);
    let binding = source.binding(&cancel).unwrap();
    let (_other, mut other) = success(pair(2));
    let mut output = Vec::new();
    let mut builder = TrackBuilder::for_selection(Track::Video, &mut output, &binding);
    assert!(other.visit(Track::Video, &cancel, |span| builder.push(span, &cancel)).is_err());
    assert_eq!(builder.finish(&cancel).unwrap_err(), Error::Identity);
    drop(builder);
    assert!(output.is_empty());
    let mut output = Vec::new();
    let mut builder = TrackBuilder::for_selection(Track::Video, &mut output, &binding);
    assert_eq!(builder.finish(&AtomicBool::new(true)).unwrap_err(), Error::Cancelled);
    assert_eq!(builder.finish(&cancel).unwrap_err(), Error::Cancelled);
}

#[test]
fn complete_pair_selects_all_units_without_packet_or_codec_authority() {
    let (_f, mut selected) = success(pair(3));
    assert!(selected.facts().packet_selection_pending && selected.facts().codec_validation_pending);
    for track in [Track::Video, Track::Audio] {
        let p = selected.facts().tracks[track.index()];
        assert_eq!((p.selected.first_ordinal, p.selected.last_ordinal), (0, 2));
        assert_eq!(p.leading, None);
        assert_eq!(p.trailing, None);
        let mut ordinals = Vec::new();
        selected
            .visit(track, &AtomicBool::new(false), |span| {
                ordinals.push(span.facts().unit.ordinal);
                Ok(())
            })
            .unwrap();
        assert_eq!(ordinals, [0, 1, 2]);
    }
}

#[test]
fn complete_prefix_survives_partial_successor_on_either_or_both_tracks() {
    for clipped in [[true, false], [false, true], [true, true]] {
        let mut bytes = pair(3);
        for at in 0..2 {
            if clipped[at] {
                bytes[at].pop();
            }
        }
        let (_f, selected) = success(bytes);
        for at in 0..2 {
            assert_eq!(selected.facts().tracks[at].selected.last_ordinal, 1);
            let trailing = selected.facts().tracks[at].trailing.unwrap();
            assert_eq!(trailing.reasons.incomplete_structure, clipped[at]);
            assert_eq!(trailing.complete_units, u64::from(!clipped[at]));
        }
    }
}

#[test]
fn only_complete_successor_marker_can_prove_unknown_unit_boundary() {
    let mut v = video(1000);
    v.extend_from_slice(&[0x1a, 0x45]);
    let f = Fixture::new([v, audio(44100)], false, Some(SealCause::Stopped));
    assert!(matches!(
        f.select(&AtomicBool::new(false)),
        Err(Fault::NoCompleteUnits)
    ));
    let next = video(1100);
    let segment = unique(&next, &[0x18, 0x53, 0x80, 0x67]);
    let mut v = video(1000);
    v.extend_from_slice(&next[..segment]);
    let (_f, selected) = success([v, audio(44100)]);
    assert_eq!(selected.facts().tracks[0].selected.last_ordinal, 0);
    assert!(
        selected.facts().tracks[0]
            .trailing
            .unwrap()
            .reasons
            .incomplete_structure
    );
}

#[test]
fn audio_coverage_excludes_whole_start_end_video_units_and_is_not_output_trim() {
    for (start, end) in [(1, 3), (0, 2), (1, 2)] {
        let audio = (start..end).flat_map(|n| audio(44100 + n * 4410)).collect();
        let (_f, selected) = success([pair(3)[0].clone(), audio]);
        let v = selected.facts().tracks[0];
        assert_eq!(
            (v.selected.first_ordinal, v.selected.last_ordinal),
            (start, end - 1)
        );
        assert_eq!(v.leading.is_some(), start > 0);
        assert_eq!(v.trailing.is_some(), end < 3);
        if let Some(e) = v.leading {
            assert!(e.reasons.startup_coverage);
        }
        if let Some(e) = v.trailing {
            assert!(e.reasons.audio_coverage_tail);
        }
        assert!(selected.facts().packet_selection_pending);
    }
}

#[test]
fn rational_single_tick_and_audio_excess_are_not_rounded_away() {
    // Superseded strict outer containment: one native sample tick of phase
    // preserves the full original video, without moving either source clock.
    for start in [44101, 44099] {
        let (_f, selected) = success([video(1000), audio(start)]);
        assert_eq!(selected.facts().tracks[0].selected.first_tick, 1_000_000_000);
        assert_eq!(selected.facts().tracks[1].selected.first_tick, i128::from(start));
    }
    let (_f, selected) = success([video(1100), pair(3)[1].clone()]);
    let a = selected.facts().tracks[1];
    assert_eq!((a.selected.first_ordinal, a.selected.last_ordinal), (1, 1));
    assert!(a.leading.unwrap().reasons.outside_selected_video);
    assert!(a.trailing.unwrap().reasons.outside_selected_video);
    assert_eq!(compare(i128::MAX, 1, 0, 2), Err(Fault::Overflow));
    assert_eq!(compare(1, 3, 2, 6), Ok(Ordering::Equal));
}

#[test]
fn selected_phase_uses_distinct_actual_first_and_last_durations_strictly() {
    for (offset, accepted) in [(0i64,true),(99,true),(100,false),(101,false),(-199,true),(-200,false),(-201,false)] {
        let a = index::authored_audio_durations((44100 + offset) as u64, &[100,4110,200]);
        let fixture = Fixture::new([video(1000),a], false, Some(SealCause::Stopped));
        let selected = fixture.select(&AtomicBool::new(false));
        assert_eq!(selected.is_ok(), accepted, "offset {offset}");
        if let Ok(selected) = selected {
            assert_eq!(selected.facts().audio_boundary, AudioBoundary { first:100,last:200 });
            assert_eq!(selected.facts().tracks[0].selected.first_ordinal, 0);
            assert!(selected.facts().tracks[0].leading.is_none());
            assert!(selected.facts().tracks[0].trailing.is_none());
        }
    }
    let range = |first_tick,end_tick,ticks_per_second| Range { first_ordinal:0,last_ordinal:0,start:0,end:1,first_tick,end_tick,ticks_per_second };
    let bounds = AudioBoundary { first:2,last:3 };
    assert!(bounds.allows(range(1,2,3),range(3,4,6)).unwrap()); // 1/6 late, less than 2/6
    assert!(!bounds.allows(range(1,2,3),range(4,5,6)).unwrap()); // no overlap at the edge
    assert!(!AudioBoundary { first:1000,last:1000 }.allows(range(0,1,1),range(2,3,1)).unwrap());
    assert!(AudioBoundary { first:0,last:1 }.allows(range(0,1,1),range(0,1,1)).is_err());
    assert_eq!(bounds.allows(range(i128::MAX-1,i128::MAX,1),range(0,1,2)),Err(Fault::Overflow));
}

#[test]
fn selected_boundary_is_bound_to_native_observations_and_withheld_eof() {
    let first = index::authored_audio_durations(44100, &[100,4150,100]);
    let second = index::authored_audio_durations(48450, &[100,3750,500]);
    let fixture = Fixture::new([video(1000),[first,second].concat()],true,Some(SealCause::Interrupted));
    let mut selected = fixture.select(&AtomicBool::new(false)).unwrap();
    assert_eq!(selected.facts().audio_boundary, AudioBoundary {first:100,last:100});
    assert_eq!(selected.facts().tracks[1].selected.last_ordinal,0);
    let cancel = AtomicBool::new(false);
    let binding = selected.binding(&cancel).unwrap();
    selected.visit(Track::Audio,&cancel,|span| {
        for first_edge in [true,false] {
            let mut changed = binding.clone();
            if first_edge { changed.facts.audio_boundary.first += 1; } else { changed.facts.audio_boundary.last += 1; }
            assert_eq!(changed.check_span(&span),Err(Error::Proof));
        }
        let original = span.facts().unit;
        let mut changed = (*original).clone();
        let index::ReconstructionFacts::Aac(ref mut fact) = changed.facts.reconstruction else {panic!("AAC expected")};
        fact.first_sample_duration += 1;
        let mut a = ObservationHash::new(Track::Audio); a.push(original).unwrap();
        let mut b = ObservationHash::new(Track::Audio); b.push(&changed).unwrap();
        assert_ne!(a.finish(),b.finish());
        binding.check_span(&span)
    }).unwrap();
    // The withheld final unit's longer packet must not approve the prior tail.
    let first = index::authored_audio_durations(44100, &[100,4200,50]);
    let second = index::authored_audio_durations(48450, &[100,3750,500]);
    let fixture = Fixture::new([video(1000),[first,second].concat()],true,Some(SealCause::Interrupted));
    assert!(matches!(fixture.select(&cancel),Err(Fault::NoCoveredInterval)));
}

#[test]
fn aac_phase_preserves_all_packets_and_checks_native_duration_proof() {
    // 64 ticks late and 46 ticks early: both smaller than their different
    // observed boundary packets. Every original packet remains represented.
    let durations = [100,4000,200];
    let (_fixture,source) = aac_binding_durations(1000,1,44164,&durations);
    let cancel = AtomicBool::new(false);
    let binding = source.binding(&cancel).unwrap();
    let packets = aac_packets(&durations,44164);
    let (bytes,summary) = aac_tape(&packets);
    let plan = aac_plan(&binding,&packets,&summary);
    assert_eq!(plan.ordinals(),(0,3));
    assert!(plan.leading().is_none() && plan.trailing().is_none());
    compare_aac(&bytes,&summary,&plan,&binding,&packets,Rational::new(0,1).unwrap()).unwrap();
    for durations in [[101,3999,200],[100,3999,201]] {
        let changed = aac_packets(&durations,44164);
        let (_,summary) = aac_tape(&changed);
        let mut partition = Partition::new(&binding,TimeBase::new(1,44100).unwrap(),&cancel,Instant::now()+Duration::from_secs(60)).unwrap();
        for packet in changed {partition.push(packet).unwrap();}
        assert!(partition.finish(&summary).is_err());
    }
    // Missing excluded records is not repaired by the outer phase allowance.
    let (_fixture,source) = aac_binding_durations(1100,1,44100,&[4410,4410,4410]);
    let binding = source.binding(&cancel).unwrap();
    let packets = aac_packets(&[4410;3],44100);
    let (_,summary) = aac_tape(&packets[1..]);
    let mut partition = Partition::new(&binding,TimeBase::new(1,44100).unwrap(),&cancel,Instant::now()+Duration::from_secs(60)).unwrap();
    for &packet in &packets[1..] {partition.push(packet).unwrap();}
    assert!(partition.finish(&summary).is_err());
}

#[test]
fn config_clock_or_malformed_suffix_never_becomes_partial_success() {
    let first = video(1000);
    for next in [video(1200), video(1000)] {
        let f = Fixture::new(
            [[first.clone(), next].concat(), pair(3)[1].clone()],
            false,
            Some(SealCause::Stopped),
        );
        assert!(matches!(
            f.select(&AtomicBool::new(false)),
            Err(Fault::Discontinuity)
        ));
    }
    let mut next = video(1100);
    let at = unique(&next, &[0xb0, 0x82, 3, 86]);
    next[at + 3] += 1;
    let f = Fixture::new(
        [[first.clone(), next].concat(), pair(3)[1].clone()],
        false,
        Some(SealCause::Stopped),
    );
    assert!(matches!(
        f.select(&AtomicBool::new(false)),
        Err(Fault::Discontinuity)
    ));
    let mut next = video(1100);
    next.extend_from_slice(&[0x81, 0x81, 0]);
    let f = Fixture::new(
        [[first, next].concat(), pair(3)[1].clone()],
        false,
        Some(SealCause::Stopped),
    );
    assert!(matches!(
        f.select(&AtomicBool::new(false)),
        Err(Fault::Index(_))
    ));
}

#[test]
fn incomplete_logical_append_withholds_only_its_eof_unit() {
    let f = Fixture::new(pair(2), true, Some(SealCause::Interrupted));
    let selected = f.select(&AtomicBool::new(false)).unwrap();
    assert_eq!(
        selected.facts().incomplete_append_start,
        Some((Track::Audio, 0))
    );
    assert_eq!(selected.facts().state, PrefixState::Interrupted);
    assert!(
        selected.facts().tracks[1]
            .trailing
            .unwrap()
            .reasons
            .incomplete_append
    );
    assert!(
        !selected.facts().tracks[0]
            .trailing
            .unwrap()
            .reasons
            .incomplete_append
    );
    assert_eq!(selected.facts().tracks[0].selected.last_ordinal, 0);
    let f = Fixture::new(pair(1), true, Some(SealCause::Interrupted));
    assert!(matches!(
        f.select(&AtomicBool::new(false)),
        Err(Fault::NoCompleteUnits)
    ));
}

#[test]
fn corruption_cancelled_and_foreign_identity_deny_selection() {
    let f = Fixture::new(pair(2), false, Some(SealCause::Stopped));
    assert!(matches!(
        f.select(&AtomicBool::new(true)),
        Err(Fault::Source(Error::Cancelled))
    ));
    let mut foreign = f.id.clone();
    foreign.source_id = uuid::Uuid::new_v4().to_string();
    assert!(CompletedSelection::prepare(f.root.path(), &foreign, &AtomicBool::new(false)).is_err());
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(f.dir().join("video.raw"))
        .unwrap();
    file.write_all(&[0]).unwrap();
    drop(file);
    assert!(matches!(
        f.select(&AtomicBool::new(false)),
        Err(Fault::Source(Error::Corrupt))
    ));
    let f = Fixture::new(pair(2), false, Some(SealCause::Cancelled));
    assert!(matches!(
        f.select(&AtomicBool::new(false)),
        Err(Fault::Source(Error::Cancelled))
    ));
}

#[test]
fn interrupted_torn_journal_and_uncommitted_bytes_remain_visible_but_unread() {
    let f = Fixture::new(pair(2), false, None);
    fs::OpenOptions::new()
        .append(true)
        .open(f.dir().join("journal.jsonl"))
        .unwrap()
        .write_all(b"{\"record\":")
        .unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(f.dir().join("video.raw"))
        .unwrap()
        .write_all(&[255; 31])
        .unwrap();
    let selected = f.select(&AtomicBool::new(false)).unwrap();
    assert_eq!(selected.facts().state, PrefixState::Interrupted);
    assert!(selected.facts().torn_journal);
    assert_eq!(selected.facts().uncommitted_bytes, [31, 0]);
    assert_eq!(
        selected.facts().tracks[0].physical_bytes,
        selected.facts().tracks[0].committed_bytes + 31
    );
}

#[test]
fn callback_cursor_error_unwind_and_final_cancel_preserve_revocation() {
    let (_f, mut selected) = success(pair(2));
    let cancel = AtomicBool::new(false);
    let initial = selected.prefix.raw[0].file.stream_position().unwrap();
    assert!(matches!(
        selected.visit(Track::Video, &cancel, |mut span| {
            span.take_reader()?.seek(SeekFrom::Start(1)).unwrap();
            Err(Error::Order)
        }),
        Err(Fault::Source(Error::Order))
    ));
    assert_eq!(
        selected.prefix.raw[0].file.stream_position().unwrap(),
        initial
    );
    assert_eq!(
        selected.visit(Track::Video, &cancel, |_| Ok(())),
        Err(Fault::Poisoned)
    );
    let (_f, mut selected) = success(pair(2));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = selected.visit(Track::Audio, &cancel, |_| panic!("test callback"));
    }))
    .is_err());
    assert_eq!(
        selected.visit(Track::Audio, &cancel, |_| Ok(())),
        Err(Fault::Poisoned)
    );
    let (_f, mut selected) = success(pair(2));
    assert!(matches!(
        selected.visit(Track::Video, &cancel, |_| {
            cancel.store(true, AtomicOrdering::Release);
            Ok(())
        }),
        Err(Fault::Source(Error::Cancelled))
    ));
    assert_eq!(
        selected.visit(Track::Video, &AtomicBool::new(false), |_| Ok(())),
        Err(Fault::Poisoned)
    );
}

#[test]
fn empty_and_no_complete_prefix_fail_without_media_authority() {
    let f = Fixture::new([Vec::new(), Vec::new()], false, Some(SealCause::Stopped));
    assert!(matches!(
        f.select(&AtomicBool::new(false)),
        Err(Fault::NoCompleteUnits)
    ));
    let f = Fixture::new(
        [video(1000)[..10].to_vec(), audio(44100)],
        false,
        Some(SealCause::Stopped),
    );
    assert!(matches!(
        f.select(&AtomicBool::new(false)),
        Err(Fault::NoCompleteUnits)
    ));
}

#[test]
fn many_repeated_units_keep_fixed_summaries_and_visit_each_selected_unit_once() {
    let (_f, mut selected) = success(pair(100));
    for track in [Track::Video, Track::Audio] {
        let mut count = 0;
        selected
            .visit(track, &AtomicBool::new(false), |span| {
                assert_eq!(span.facts().unit.ordinal, count);
                count += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(count, 100);
    }
}

#[test]
#[ignore = "Root-only fixed immutable actual V3 source: copied into owned temporary spools for structural partial-tail checks; no media execution"]
fn actual_v3_partial_tail_and_two_track_coverage() {
    use super::super::super::files::{Directories, GuardedFile};
    use sha2::{Digest, Sha256};
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".runtime/youtube-webm-raw-pair-qa/c26c9147-735f-4ed3-821b-31e92cef9afd");
    let dirs = Directories::hold(&directory).unwrap();
    let specs = [
        (
            "video-1.webm",
            95635u64,
            "9be82861467e27ed66fa8220db1a7c4f1e2872996755257cbeafe149035070a4",
        ),
        (
            "video-2.webm",
            76753,
            "7bb5f6e6f3fcd435042dc5b7227ca023799014f7f3e7886e4a99bc732451a4b6",
        ),
        (
            "audio-1.mp4",
            81810,
            "c349dc8672a8ade3d961d19c7d0b30f09473b08b96c7eb58641cbaceb385779a",
        ),
        (
            "audio-2.mp4",
            82185,
            "b9fa4f91f7dc48bee2a9f7128188f27b3b041b6faf16f91e19b7a3e17daf7457",
        ),
    ];
    let mut manifest = GuardedFile::open(&directory.join("manifest.json"), false).unwrap();
    let manifest_len = manifest.check().unwrap();
    assert!(manifest_len <= 16384);
    let mut metadata = Vec::new();
    manifest.file.read_to_end(&mut metadata).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&metadata)),
        "c4a92fbff4e9c27870b817e1d2015cb66990eaf9ec47568ead953026c15508ad"
    );
    let mut held = Vec::new();
    let mut bytes = Vec::new();
    for (name, size, digest) in specs {
        let mut file = GuardedFile::open(&directory.join(name), false).unwrap();
        assert_eq!(file.check().unwrap(), size);
        let mut data = Vec::with_capacity(size as usize);
        file.file.read_to_end(&mut data).unwrap();
        assert_eq!(data.len() as u64, size);
        assert_eq!(format!("{:x}", Sha256::digest(&data)), digest);
        held.push(file);
        bytes.push(data);
    }
    // Actual second video body is cut in our private copy only. Both complete
    // audio units remain, so this must retain one fully covered video unit.
    let mut video_tail = [bytes[0].clone(), bytes[1].clone()].concat();
    video_tail.pop();
    let audio_all = [bytes[2].clone(), bytes[3].clone()].concat();
    let (_f, selected) = success([video_tail, audio_all]);
    assert_eq!(selected.facts().tracks[0].selected.last_ordinal, 0);
    assert!(
        selected.facts().tracks[0]
            .trailing
            .unwrap()
            .reasons
            .incomplete_structure
    );
    assert!(selected.facts().packet_selection_pending && selected.facts().codec_validation_pending);
    for (file, (_, size, digest)) in held.iter_mut().zip(specs) {
        assert_eq!(file.check().unwrap(), size);
        file.file.seek(SeekFrom::Start(0)).unwrap();
        let mut again = Vec::with_capacity(size as usize);
        file.file.read_to_end(&mut again).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(again)), digest);
    }
    manifest.file.seek(SeekFrom::Start(0)).unwrap();
    let mut again = Vec::new();
    manifest.file.read_to_end(&mut again).unwrap();
    assert_eq!(again, metadata);
    assert_eq!(manifest.check().unwrap(), manifest_len);
    dirs.check().unwrap();
}

#[test]
#[ignore = "Root-only fixed actual raw recordings: read-only native clocks/selection, no files created or media execution"]
fn actual_raw_qa_boundary_clocks_read_only() {
    use super::super::super::files::{Directories, GuardedFile};
    use sha2::{Digest, Sha256};
    use serde_json::json;

    fn checked_hash(file: &GuardedFile, length: u64, expected: &str) {
        assert!(length > 0 && length <= 4 * 1024 * 1024);
        assert_eq!(file.check().unwrap(), length);
        let mut reader = &file.file;
        let saved = reader.stream_position().unwrap();
        reader.seek(SeekFrom::Start(0)).unwrap();
        let mut scratch = [0; 128 * 1024];
        let mut left = length;
        let mut digest = Sha256::new();
        while left != 0 {
            let size = left.min(scratch.len() as u64) as usize;
            reader.read_exact(&mut scratch[..size]).unwrap();
            digest.update(&scratch[..size]);
            left -= size as u64;
        }
        assert_eq!(reader.read(&mut scratch[..1]).unwrap(), 0);
        reader.seek(SeekFrom::Start(saved)).unwrap();
        assert_eq!(file.check().unwrap(), length);
        assert_eq!(format!("{:x}", digest.finalize()), expected);
    }
    fn range(value: Range) -> serde_json::Value {
        json!({"firstOrdinal":value.first_ordinal,"lastOrdinal":value.last_ordinal,
            "start":value.start,"end":value.end,"firstTick":value.first_tick,
            "endTick":value.end_tick,"ticksPerSecond":value.ticks_per_second})
    }
    fn exclusion(value: Option<Excluded>) -> serde_json::Value {
        value.map_or(serde_json::Value::Null, |value| json!({
            "start":value.start,"end":value.end,"completeUnits":value.complete_units,
            "startupCoverage":value.reasons.startup_coverage,
            "audioCoverageTail":value.reasons.audio_coverage_tail,
            "outsideSelectedVideo":value.reasons.outside_selected_video,
            "incompleteStructure":value.reasons.incomplete_structure,
            "incompleteAppend":value.reasons.incomplete_append}))
    }
    let cases = [
        ("manual", ".runtime/youtube-original-qa/89c5a86b-833f-43d4-966a-6b891fa16215/media",
            "47ac1001-18c9-4e8d-962c-931de53e87a5", [
                ("proof.json",805u64,"4461f8bbdf376320d5ad0c7f40f2e535488c6c60ec2793e797ddb4d6842409bb"),
                ("journal.jsonl",11969,"143a747aeb3a57270ddff2f49e600f7fc5995aeb3d16479371e8171b11fe36b8"),
                ("video.raw",351859,"5a5d034282dc82dbc6146edcf241cf5461946f3b98307bbb0c87940dd4e3daa8"),
                ("audio.raw",327619,"0cb1e1a7fdbc59526c2ecb43e6842451770600cfd0bab12cd9818488f94cf951")]),
        ("automatic", ".runtime/youtube-auto-record-qa/local.atsumi.youtubeautoqa.6f1dcbb6282d4018af5288a2c0b82883/media",
            "0a2af8b4-9511-4415-9f6c-6cf83d8e16be", [
                ("proof.json",942u64,"9050fa63276b0cdd9d86f492c18652e0346a23cd1857b9028381392508ee5f53"),
                ("journal.jsonl",97372,"c590ce993e06212293c76ed182c6f39a5f6ef7c232095cd2eb51efb6f77dde70"),
                ("video.raw",1374780,"8723e0b77f21ff951c7dd3ec61b7103f3e3f40491676eecd4c50d2943a4ff341"),
                ("audio.raw",573433,"cc9488b8b7c5fb7eb33b77fcd76f268da3cbd2c9925719c3130130f168064895")]),
    ];
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let cancel = AtomicBool::new(false);
    for (label, relative, job, specs) in cases {
        let root = workspace.join(relative);
        let directory = root.join("YouTube").join(job);
        let directories = Directories::hold(&directory).unwrap();
        let held: Vec<_> = specs.iter().map(|(name, length, digest)| {
            let file = GuardedFile::open(&directory.join(name), false).unwrap();
            checked_hash(&file, *length, digest);
            file
        }).collect();
        directories.check().unwrap();
        let identity = super::super::super::production::recovery_identity(&root, job, &cancel).unwrap();
        let mut units: [Vec<serde_json::Value>; 2] = std::array::from_fn(|_| Vec::new());
        let (prefix, outcomes) = super::super::inspect_and_index(&root, &identity, &cancel, &mut |track, unit| {
            let rows = &mut units[track.index()];
            if rows.len() >= 16 { return Err(()); }
            let facts = unit.facts;
            let first_duration = match facts.reconstruction {
                index::ReconstructionFacts::Aac(value) => Some(value.first_sample_duration),
                _ => None,
            };
            rows.push(json!({"ordinal":unit.ordinal,"start":facts.start,"end":facts.end,
                "initEnd":facts.init_end,"items":facts.items,"fragments":facts.fragments,
                "firstAacSampleDuration":first_duration,
                "boundary":format!("{:?}",facts.boundary),
                "clock":{"first":facts.clock.first,"last":facts.clock.last,"end":facts.clock.end,
                    "ticksPerSecond":facts.clock.ticks_per_second}}));
            Ok(())
        }).unwrap();
        for at in 0..2 {
            let outcome = outcomes[at].unwrap();
            assert!(matches!(outcome.terminal, index::Terminal::Complete | index::Terminal::PartialTail { .. }));
            assert_eq!(outcome.complete_units as usize, units[at].len());
            assert!(!units[at].is_empty());
        }
        let selected = CompletedSelection::prepare(&root, &identity, &cancel).unwrap();
        selected.check(&cancel).unwrap();
        let selected_tracks = selected.facts().tracks.map(|plan| json!({
            "selected":range(plan.selected),"leading":exclusion(plan.leading),
            "trailing":exclusion(plan.trailing),"committedBytes":plan.committed_bytes,
            "physicalBytes":plan.physical_bytes}));
        prefix.index_guards(&cancel).unwrap();
        for (file, (_, length, digest)) in held.iter().zip(specs) {
            checked_hash(file, length, digest);
        }
        directories.check().unwrap();
        selected.check(&cancel).unwrap();
        println!("{}", json!({"kind":"youtube_raw_boundary_clocks","case":label,
            "originalsUnchanged":true,"state":format!("{:?}",prefix.state),
            "sealCause":format!("{:?}",prefix.seal_cause),"receipts":prefix.receipts,
            "incompleteAppend":prefix.incomplete_append,"tornJournal":prefix.torn_journal,
            "uncommittedBytes":prefix.uncommitted_bytes,
            "terminal":outcomes.map(|value|format!("{:?}",value.unwrap().terminal)),
            "units":units,"selection":selected_tracks,"firstAacDurationKnown":true}));
    }
}
