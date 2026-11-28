use super::*;
use sha2::Digest;

#[test]
fn sealed_timing_keeps_nonzero_source_origin_positive_cts_and_exact_wire_ranges() {
    let (mut r,mut m)=setup();let mut v=samples(true,120);let mut a=samples(false,188);
    for s in &mut v{s.dts+=900_000;s.cts=3000;}for s in &mut a{s.dts+=480_000;}
    let mut all=feed(&mut r,&mut m,&v,&a).unwrap();let (tail,end)=m.finish(r.finish().unwrap()).unwrap();all.extend(tail);
    assert_eq!(end.origin().ticks*10,end.origin().timescale as u64*100);
    for segment in &all {
        let t=segment.timing();assert_eq!(t.segment_end_90k-t.segment_start_90k,segment.duration_ticks());
        assert_eq!(t.video.min_pts-t.video.first_dts,3000);
        assert_eq!(t.video.min_pts_90k-t.video.first_dts_90k,3000);
        assert_eq!(t.audio.min_pts,t.audio.first_dts);assert_eq!(t.video.first_dts,segment.video.first_dts);
        let pts:Vec<_>=packets(segment.bytes()).filter(|p|pid(p)==VIDEO_PID&&p[1]&0x40!=0).map(|p|unstamp(&payload(p)[9..14])).collect();
        assert_eq!(pts.iter().min().copied(),Some(t.video.min_pts_90k));
        assert_eq!(pts.iter().max().copied().unwrap()+3000,t.video.max_pts_end_90k);
    }
    assert_eq!(all[0].timing.audio.last_dts_end,all[1].timing.audio.first_dts);
}

#[test]
fn durable_ack_ledger_accepts_only_sealed_order_and_matching_successful_end() {
    use crate::platform_media::relay_chat::test_fixture;
    use crate::streaming::browser::encoded::fmp4::{fixtures as canonical_fixtures,
        relay::{EncodedSampleReader as CanonicalReader,ts::TsMuxer as CanonicalMuxer}};
    use std::time::{Duration,Instant};
    // This source file also compiles below platform_media's independent parser.
    // The cross-domain ledger accepts only the canonical relay worker's types.
    let mut r=CanonicalReader::new(canonical_fixtures::inputs()).unwrap();
    let mut m=CanonicalMuxer::new(r.descriptor()).unwrap();
    assert!(m.push(r.push(0,&make_fragment(7,&samples(true,120),0,1).unwrap()).unwrap()).unwrap().is_empty());
    let mut all=m.push(r.push(1,&make_fragment(7,&samples(false,188),0,1).unwrap()).unwrap()).unwrap();
    let(tail,end)=m.finish(r.finish().unwrap()).unwrap();all.extend(tail);
    let(mut lease,_root)=test_fixture();let t=lease.timing();
    for segment in &all {let before=Instant::now();assert!(t.segment_acked(segment,before,Instant::now()));}
    assert!(!t.segment_acked(&all[0],Instant::now(),Instant::now()));
    let before=Instant::now();assert!(t.end_acked(&end,before,Instant::now()));assert!(!t.end_acked(&end,before,Instant::now()));
    lease.begin_finish(false);let deadline=Instant::now()+Duration::from_secs(3);
    let report=loop{if let Some(r)=lease.poll_join().unwrap(){break r;}assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1));};
    let text=std::fs::read_to_string(report.directory.join("relay-timing.jsonl")).unwrap();
    let rows:Vec<serde_json::Value>=text.lines().map(|r|serde_json::from_str(r).unwrap()).collect();
    assert_eq!(rows.len(),all.len()+1);assert_eq!(rows.last().unwrap()["kind"],"end_ack");
    for(i,s)in all.iter().enumerate(){assert_eq!(rows[i]["data"]["sha256"],format!("{:x}",sha2::Sha256::digest(s.bytes())));assert_eq!(rows[i]["data"]["timing"],serde_json::to_value(s.timing()).unwrap());}
    assert_eq!(rows.last().unwrap()["data"]["segments"],all.len());assert!(report.timing_incomplete);
}

fn setup() -> (EncodedSampleReader, TsMuxer) {
    let r = EncodedSampleReader::new(fixtures::inputs()).unwrap();
    let m = TsMuxer::new(r.descriptor()).unwrap();
    (r, m)
}
fn samples(video: bool, count: u32) -> Vec<Sample> {
    (0..count)
        .map(|i| {
            let key = video && i % 60 == 0;
            Sample {
                dts: u64::from(i) * if video { 3000 } else { 1024 },
                duration: if video { 3000 } else { 1024 },
                flags: if !video || key {
                    0x02000000
                } else {
                    0x01010000
                },
                cts: 0,
                aac_rounding: None,
                data: if video {
                    vec![0, 0, 0, 2, if key { 0x65 } else { 0x41 }, i as u8]
                } else {
                    vec![0x21, i as u8]
                },
            }
        })
        .collect()
}
fn feed(
    r: &mut EncodedSampleReader,
    m: &mut TsMuxer,
    video: &[Sample],
    audio: &[Sample],
) -> Result<Vec<TsSegment>> {
    assert!(m
        .push(r.push(0, &make_fragment(7, video, 0, 1).unwrap()).unwrap())?
        .is_empty());
    m.push(r.push(1, &make_fragment(7, audio, 0, 1).unwrap()).unwrap())
}
fn packets(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    assert_eq!(bytes.len() % 188, 0);
    bytes.chunks_exact(188)
}
fn pid(p: &[u8]) -> u16 {
    u16::from(p[1] & 31) * 256 + u16::from(p[2])
}
fn payload(p: &[u8]) -> &[u8] {
    let at = if p[3] & 0x20 != 0 {
        5 + usize::from(p[4])
    } else {
        4
    };
    &p[at..]
}
fn unstamp(p: &[u8]) -> u64 {
    u64::from((p[0] >> 1) & 7) << 30
        | u64::from(p[1]) << 22
        | u64::from(p[2] >> 1) << 15
        | u64::from(p[3]) << 7
        | u64::from(p[4] >> 1)
}

#[test]
fn segments_preserve_one_clock_crc_payload_continuity_and_terminal_identity() {
    let (mut r, mut m) = setup();
    let mut all = feed(&mut r, &mut m, &samples(true, 360), &samples(false, 563)).unwrap();
    assert_eq!(all.len(), 5);
    let (tail, end) = m.finish(r.finish().unwrap()).unwrap();
    all.extend(tail);
    assert_eq!(all.len(), 6);
    assert_eq!(end.segments(), 6);
    assert_eq!(
        all.iter().map(|s| s.duration_ticks()).sum::<u64>(),
        563 * 1024 * 90000 / 48000
    );
    let mut cc = std::collections::HashMap::new();
    let mut clocks = [Vec::new(), Vec::new()];
    for (i, s) in all.iter().enumerate() {
        assert_eq!(s.ordinal(), i as u64);
        assert_eq!(s.stream_id(), end.stream_id());
        assert!((1..=5000).contains(&s.duration_ms()));
        assert!(s.bytes().len() < SEGMENT_BYTES);
        let p: Vec<_> = packets(s.bytes()).collect();
        assert_eq!(pid(p[0]), 0);
        assert_eq!(pid(p[1]), PMT_PID);
        for table in &p[..2] {
            let b = payload(table);
            let section = &b[1..];
            let n = 3 + ((usize::from(section[1] & 15) << 8) | usize::from(section[2]));
            assert_eq!(crc(&section[..n]), 0);
        }
        for packet in p {
            assert_eq!(packet[0], 0x47);
            let id = pid(packet);
            if packet[3] & 0x10 == 0 {
                continue;
            }
            let c = packet[3] & 15;
            if let Some(previous) = cc.insert(id, c) {
                assert_eq!(c, (previous + 1) & 15);
            }
            if [VIDEO_PID, AUDIO_PID].contains(&id) && packet[1] & 0x40 != 0 {
                let b = payload(packet);
                assert_eq!(&b[..3], &[0, 0, 1]);
                clocks[usize::from(id == AUDIO_PID)].push(unstamp(&b[9..14]));
            }
        }
    }
    assert_eq!(clocks[0], (0..360).map(|i| i * 3000).collect::<Vec<_>>());
    assert_eq!(
        clocks[1],
        (0..563)
            .map(|i| (i * 1024 * 90000 + 24000) / 48000)
            .collect::<Vec<_>>()
    );
    assert!(m.base.closed);
    assert_eq!(m.rejection(), None);
}

#[test]
fn parser_proven_aac_rounding_preserves_every_payload_clock_and_source_end() {
    for rounding in [-1i64, 1] {
        let (mut r, mut m) = setup();
        assert!(m.push(r.push(0, &make_fragment(7, &samples(true, 120), 0, 1).unwrap()).unwrap()).unwrap().is_empty());
        let audio = samples(false, 188);
        assert!(m.push(r.push(1, &make_fragment(7, &audio[..47], 0, 1).unwrap()).unwrap()).unwrap().is_empty());
        let mut rest = audio[47..].to_vec();
        for sample in &mut rest { sample.dts = (sample.dts as i64 + rounding) as u64; }
        let batch = r.push(1, &make_fragment(7, &rest, 0, 2).unwrap()).unwrap();
        // The actual reader's bounded fragment-boundary correction, not an
        // invented change to the AAC elementary stream, produces this duration.
        assert_eq!(batch.audio[0].duration, (1024 + rounding) as u32);
        assert_eq!(batch.audio[0].dts, 47 * 1024);
        assert_eq!(batch.audio[0].aac_rounding, Some(AacContainerRounding {
            original_dts:(47*1024i64+rounding) as u64, original_duration:1024, timescale:48000,
        }));
        assert!(batch.audio.iter().skip(1).all(|s|s.aac_rounding.is_none()));
        let expected:Vec<_>=audio[..47].iter().chain(batch.audio.iter())
            .map(|s|(s.dts,s.data.clone())).collect();
        let mut segments=m.push(batch).unwrap();
        segments.extend(m.finish(r.finish().unwrap()).unwrap().0);
        assert_eq!(segments.iter().map(|s|s.audio.samples).sum::<u64>(),188);
        assert_eq!(segments.last().unwrap().audio.last_end,(188*1024i64+rounding) as u64);
        let mut emitted=Vec::new();
        for segment in &segments {
            for packet in packets(segment.bytes()).filter(|p|pid(p)==AUDIO_PID && p[1]&0x40!=0) {
                let pes=payload(packet);
                let end=6+usize::from(u16::from_be_bytes([pes[4],pes[5]]));
                let adts=&pes[9+usize::from(pes[8])..end];
                assert_eq!(adts.len(),9); // Same2-byte encoded AAC fixture payload +7 ADTS bytes.
                emitted.push((unstamp(&pes[9..14]),adts[7..].to_vec()));
            }
        }
        assert_eq!(emitted,expected.into_iter().map(|(dts,bytes)|((dts*90000+24000)/48000,bytes)).collect::<Vec<_>>());
        assert!(m.base.closed && m.base.video.is_empty() && m.base.audio.is_empty());
        assert_eq!(m.rejection(),None);
    }
}

#[test]
fn raw_aac1023_and_non1024_original_normalized_to1024_remain_unsupported() {
    let (mut r,mut m)=setup();
    let mut audio=samples(false,188);
    for (i,s) in audio.iter_mut().enumerate() {s.dts=i as u64*1023;s.duration=1023;}
    assert_eq!(feed(&mut r,&mut m,&samples(true,120),&audio).err().unwrap().code,"ENCODED_UNSUPPORTED");
    let rejected=m.rejection().unwrap().aac_sample.unwrap();
    assert!(rejected.duration_not_1024 && rejected.duration_not_proven_aac1024);

    let (mut r,mut m)=setup();
    assert!(m.push(r.push(0,&make_fragment(7,&samples(true,120),0,1).unwrap()).unwrap()).unwrap().is_empty());
    let audio=samples(false,188);
    assert!(m.push(r.push(1,&make_fragment(7,&audio[..47],0,1).unwrap()).unwrap()).unwrap().is_empty());
    let mut rest=audio[47..].to_vec();rest[0].dts+=1;rest[0].duration=1023;
    let batch=r.push(1,&make_fragment(7,&rest,0,2).unwrap()).unwrap();
    assert_eq!(batch.audio[0].duration,1024);
    assert_eq!(batch.audio[0].aac_rounding.unwrap().original_duration,1023);
    assert!(m.push(batch).is_err());
    let rejected=m.rejection().unwrap().aac_sample.unwrap();
    assert!(!rejected.duration_not_1024 && rejected.duration_not_proven_aac1024);
}

#[test]
fn aac_rounding_rejects_mismatched_provenance_or_correction_outside_existing_bound() {
    for variant in 0..5 {
        let (mut r,mut m)=setup();
        assert!(m.push(r.push(0,&make_fragment(7,&samples(true,120),0,1).unwrap()).unwrap()).unwrap().is_empty());
        let mut batch=r.push(1,&make_fragment(7,&samples(false,188),0,1).unwrap()).unwrap();
        let sample=&mut batch.audio[3];sample.duration=1023;
        let mut proof=AacContainerRounding{original_dts:3071,original_duration:1024,timescale:48000};
        match variant {
            0=>proof.original_duration=1023,
            1=>proof.timescale=44100,
            2=>proof.original_dts=3070,
            3=>{proof.original_dts=3023;sample.duration=975;}, //49ticks exceeds48kHz1ms.
            _=>{proof.original_dts=3072;sample.duration=1024;}, //No correction cannot mint provenance.
        }
        sample.aac_rounding=Some(proof);
        assert!(m.push(batch).is_err());
        assert!(m.rejection().unwrap().aac_sample.unwrap().duration_not_proven_aac1024);
        assert!(m.base.closed && m.segments==0);
    }
    for delta in [-49i64,49] {
        let (mut r,_)=setup();let audio=samples(false,188);
        r.push(1,&make_fragment(7,&audio[..47],0,1).unwrap()).unwrap();
        let mut rest=audio[47..].to_vec();rest[0].dts=(rest[0].dts as i64+delta) as u64;
        assert!(r.push(1,&make_fragment(7,&rest,0,2).unwrap()).is_err());
    }
}

#[test]
fn rejection_copies_first_aac_cts_or_payload_predicate_before_poison() {
    for variant in 0..3 {
        let (mut r, mut m) = setup();
        assert!(m.push(r.push(0, &make_fragment(7, &samples(true, 120), 0, 1).unwrap()).unwrap()).unwrap().is_empty());
        let mut batch = r.push(1, &make_fragment(7, &samples(false, 188), 0, 1).unwrap()).unwrap();
        // Test-private mutation of a valid sealed batch exercises mux defenses;
        // production reader would reject the CTS/empty cases before this point.
        match variant {
            0 => batch.audio[3].cts = 1,
            1 => batch.audio[3].data.clear(),
            _ => batch.audio[3].data = vec![0x21; 8191 - 6],
        }
        batch.audio[4].duration = 1023; // A later failure must not replace index3.
        assert_eq!(m.push(batch).err().unwrap().code, "ENCODED_UNSUPPORTED");
        let rejection=m.rejection().unwrap();
        let s=rejection.aac_sample.unwrap();
        assert_eq!(rejection.reason,TsRejectReason::AacFrame);
        assert_eq!((s.pending_index,s.normalized_dts,s.normalized_duration),(3,3072,1024));
        assert_eq!((s.nonzero_cts,s.empty_payload,s.payload_too_large),(variant==0,variant==1,variant==2));
        assert!(!s.duration_not_1024);
        assert_eq!(s.payload_bytes,match variant {1=>0,2=>8185,_=>2});
        assert!(m.base.closed && m.base.audio.is_empty());
        assert_eq!(m.segments,0);
        assert!(m.finish(r.finish().unwrap()).is_err());
        assert_eq!(m.rejection(),Some(rejection));
    }
}

#[test]
fn rejection_preserves_video_first_failure_without_inventing_aac_cause() {
    let (mut r,mut m)=setup();
    let mut video=samples(true,120);
    video[1].cts=180_000;
    assert!(m.push(r.push(0,&make_fragment(7,&video,0,1).unwrap()).unwrap()).unwrap().is_empty());
    let mut batch=r.push(1,&make_fragment(7,&samples(false,188),0,1).unwrap()).unwrap();
    batch.audio[3].duration=1023;
    let error=m.push(batch).err().unwrap();
    assert_eq!(error.message,"영상 표시 시각이 IDR 조각 경계를 벗어납니다.");
    assert_eq!(m.rejection(),Some(TsRejection {reason:TsRejectReason::VideoPresentationBoundary,aac_sample:None}));
    assert!(m.base.closed);
    assert_eq!(m.segments,0);
}

#[test]
fn rejection_reason_is_closed_and_never_copies_unknown_error_text() {
    let error=StreamError::new("untrusted-code","https://secret.invalid/private-message",false);
    let detail=TsRejection {reason:rejection_reason(&error),aac_sample:None};
    assert_eq!(detail.reason,TsRejectReason::Other);
    assert_eq!(serde_json::to_string(&detail).unwrap(),r#"{"reason":"other","aacSample":null}"#);
}

#[test]
fn foreign_descriptor_reordered_and_duplicate_batches_poison_without_output() {
    let (mut r, mut m) = setup();
    let (mut foreign, _) = setup();
    let b = foreign
        .push(0, &make_fragment(7, &samples(true, 60), 0, 1).unwrap())
        .unwrap();
    assert!(m.push(b).is_err());
    assert!(m.push(r.push(0, &boxed(b"free", &[])).unwrap()).is_err());
    let (mut r, mut m) = setup();
    let _zero = r.push(0, &boxed(b"free", &[])).unwrap();
    assert!(m.push(r.push(0, &boxed(b"free", &[])).unwrap()).is_err());
}

#[test]
fn empty_batches_keep_ordinals_and_wrong_end_never_mints_proof() {
    let (mut r, mut m) = setup();
    assert!(m
        .push(r.push(0, &boxed(b"free", &[])).unwrap())
        .unwrap()
        .is_empty());
    feed(&mut r, &mut m, &samples(true, 120), &samples(false, 188)).unwrap();
    let (mut other, _) = setup();
    assert!(m.finish(other.finish().unwrap()).is_err());
}

#[test]
fn no_future_idr_over_five_seconds_poison_without_partial_return() {
    let (mut r, mut m) = setup();
    let mut v = samples(true, 181);
    for s in v.iter_mut().skip(1) {
        s.flags = 0x01010000;
        s.data[4] = 0x41;
    }
    assert!(feed(&mut r, &mut m, &v, &samples(false, 283)).is_err());
    assert!(m.finish(r.finish().unwrap()).is_err());
}

#[test]
fn bad_second_segment_never_returns_first_from_failed_call() {
    let (mut r, mut m) = setup();
    let mut v = samples(true, 180);
    v[65].data[4] = 0x02;
    assert!(feed(&mut r, &mut m, &v, &samples(false, 282)).is_err());
    assert!(m.base.closed);
}

#[test]
fn parameter_change_mixed_idr_and_bad_aac_rejected() {
    for mode in 0..4 {
        let (mut r, mut m) = setup();
        let mut v = samples(true, 120);
        let mut a = samples(false, 188);
        match mode {
            0 => v[0].data.extend([0, 0, 0, 2, 0x41, 0]),
            1 => v[1].data.extend([0, 0, 0, 2, 0x67, 0]),
            2 => a[0].duration = 960,
            _ => a[0].data = vec![0; 8191],
        };
        // Test-private valid epoch batch mutation isolates mux validation; parser
        // continuity and NAL restrictions remain independently unchanged.
        assert!(m
            .push(
                r.push(0, &make_fragment(7, &samples(true, 120), 0, 1).unwrap())
                    .unwrap()
            )
            .unwrap()
            .is_empty());
        let mut b = r
            .push(1, &make_fragment(7, &samples(false, 188), 0, 1).unwrap())
            .unwrap();
        m.base.video = v.into();
        b.audio = a.into();
        assert!(m.push(b).is_err());
    }
}

#[test]
fn audio_cut_is_owned_once_and_final_clock_keeps_fractional_source_end() {
    let (mut r, mut m) = setup();
    let mut s = feed(&mut r, &mut m, &samples(true, 120), &samples(false, 188)).unwrap();
    let (tail, end) = m.finish(r.finish().unwrap()).unwrap();
    s.extend(tail);
    assert_eq!(s.iter().map(|s| s.audio.samples).sum::<u64>(), 188);
    assert_eq!(s[0].audio.last_end, s[1].audio.first_dts);
    assert_eq!(s[1].audio.last_end, 188 * 1024);
    assert_eq!(end.preroll, PrerollCounts::default());
}

#[test]
fn nonnegative_reordered_pts_keep_source_dts_and_boundary_crossing_fails() {
    let (mut r, mut m) = setup();
    let mut v = samples(true, 120);
    for sample in &mut v {
        sample.cts = 3000;
    }
    v[1].cts = 6000;
    v[2].cts = 0;
    let mut out = feed(&mut r, &mut m, &v, &samples(false, 188)).unwrap();
    out.extend(m.finish(r.finish().unwrap()).unwrap().0);
    let p: Vec<_> = packets(out[0].bytes())
        .filter(|p| pid(p) == VIDEO_PID && p[1] & 0x40 != 0)
        .map(|p| unstamp(&payload(p)[9..14]))
        .collect();
    assert_eq!(&p[..4], &[3000, 9000, 6000, 12000]);
    let (mut r, mut m) = setup();
    let mut v = samples(true, 120);
    v[59].cts = 6000;
    assert!(feed(&mut r, &mut m, &v, &samples(false, 188)).is_err());
}

#[test]
fn negative_composition_clock_is_explicitly_unsupported_and_cannot_mint_end() {
    for (index, cts) in [(2, -3000), (2, -1), (65, -1)] {
        let (mut r, mut m) = setup();
        let mut video = samples(true, 180);
        video[index].cts = cts;
        let error = feed(&mut r, &mut m, &video, &samples(false, 282))
            .err()
            .expect("no output returned");
        assert_eq!(error.code, "ENCODED_TS_CLOCK_UNSUPPORTED");
        assert!(m.base.closed);
        assert!(m.finish(r.finish().unwrap()).is_err());
    }
    // Exact known counterexample, plus an independently stale PCR boundary.
    for (dts, pts, pcr) in [(6000, 3000, 3600), (6000, 9000, 6001)] {
        assert_eq!(
            transport_clock(dts, pts, pcr).unwrap_err().code,
            "ENCODED_TS_CLOCK_UNSUPPORTED"
        );
    }
    assert!(transport_clock(6000, 6000, 6000).is_ok());
    assert!(transport_clock(6000, 9000, 3600).is_ok());
}

#[test]
fn shared_start_waits_for_overlapping_aac_and_records_preroll() {
    let (mut r, mut m) = setup();
    let v = samples(true, 180);
    let mut a = samples(false, 281);
    for s in &mut a {
        s.dts += 240;
    }
    let mut all = feed(&mut r, &mut m, &v, &a).unwrap();
    all.extend(m.finish(r.finish().unwrap()).unwrap().0);
    assert_eq!(m.base.preroll.video_samples, 60);
    assert_eq!(m.base.preroll.audio_samples, 93);
    assert_eq!(all[0].origin.ticks, 95472);
    assert_eq!(all[0].video.first_dts, 180000);
}

#[test]
fn timestamp_rounding_is_single_rational_with_wrap_only_at_wire() {
    let origin = SourceOrigin {
        ticks: 1,
        timescale: 48000,
        video_base: 1,
        audio_base: 1,
        video_remainder: 42000,
        audio_remainder: 0,
    };
    assert_eq!(clock(2, 48000, origin).unwrap(), 2);
    assert!(clock(0, 48000, origin).is_err());
    assert_eq!(unstamp(&stamp((1 << 33) + 90, 0x20)), 90);
    assert_eq!(crc(b"123456789"), 0x0376e6e7);
}

#[test]
fn pending_credit_and_serialized_segment_caps_poison_without_output() {
    let (mut r, mut m) = setup();
    let mut batch = r.push(0, &boxed(b"free", &[])).unwrap();
    let mut sample = samples(true, 1).remove(0);
    sample.data = vec![0; MAX_PENDING / 2];
    batch.video.push_back(sample.clone());
    batch.video.push_back(sample);
    assert!(m.push(batch).is_err());
    assert!(m.base.closed);

    let (mut r, mut m) = setup();
    assert!(m
        .push(
            r.push(0, &make_fragment(7, &samples(true, 120), 0, 1).unwrap())
                .unwrap()
        )
        .unwrap()
        .is_empty());
    let n = SEGMENT_BYTES - 4096;
    let mut large = vec![0x01; n];
    large[..4].copy_from_slice(&((n - 4) as u32).to_be_bytes());
    large[4] = 0x65;
    m.base.video[0].data = large;
    let batch = r
        .push(1, &make_fragment(7, &samples(false, 188), 0, 1).unwrap())
        .unwrap();
    assert!(m.push(batch).is_err()); // TS framing overhead exceeds16MiB.
    assert!(m.base.closed);
}

#[test]
fn final_reordered_max_presentation_end_is_distinct_from_reader_decode_end() {
    let (mut r, mut m) = setup();
    let mut video = samples(true, 120);
    for sample in &mut video {
        sample.cts = 6000;
    }
    video[117].cts = 12000;
    video[119].cts = 0;
    let mut segments = feed(&mut r, &mut m, &video, &samples(false, 188)).unwrap();
    let (tail, end) = m.finish(r.finish().unwrap()).unwrap();
    segments.extend(tail);
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1].video.last_end, 360000); // Exact reader DTS end.
    assert_eq!(m.segment_start, 366000); // Max source PTS+duration, not last decoded PTS.
    assert_eq!(segments[1].duration_ms(), 2000);
    assert_eq!(end.segments(), 2);
}
