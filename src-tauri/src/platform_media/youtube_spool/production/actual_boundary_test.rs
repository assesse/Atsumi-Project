use super::*;
use crate::platform_media::youtube_spool::{index, recovery::inspect_and_index, Track};
use sha2::{Digest, Sha256};
use std::io::{Seek, SeekFrom};
use std::time::Duration;

fn check_hash(file: &GuardedFile, length: u64, expected: &str) -> bool {
    if length == 0 || length >= 4 * 1024 * 1024 || file.check().ok() != Some(length) {
        return false;
    }
    let mut reader = &file.file;
    let Ok(saved) = reader.stream_position() else { return false; };
    let result = (|| -> std::io::Result<bool> {
        reader.seek(SeekFrom::Start(0))?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 131072];
        let mut left = length;
        while left != 0 {
            let size = left.min(buffer.len() as u64) as usize;
            reader.read_exact(&mut buffer[..size])?;
            hash.update(&buffer[..size]);
            left -= size as u64;
        }
        Ok(reader.read(&mut buffer[..1])? == 0 && format!("{:x}", hash.finalize()) == expected)
    })();
    let restored = reader.seek(SeekFrom::Start(saved)).is_ok();
    let unchanged = file.check().ok() == Some(length);
    result.unwrap_or(false) && restored && unchanged
}

#[test]
#[ignore = "Root-only isolated actual raw sources; pinned inputs unchanged, new finalizer attempts only, no publication/UI"]
fn actual_complete_raw_boundaries_preserve_video_through_production_finalizer() {
    let cases = [
        ("manual", ".runtime/youtube-original-qa/89c5a86b-833f-43d4-966a-6b891fa16215/media",
         "47ac1001-18c9-4e8d-962c-931de53e87a5", [
            ("proof.json",805u64,"4461f8bbdf376320d5ad0c7f40f2e535488c6c60ec2793e797ddb4d6842409bb"),
            ("journal.jsonl",11969,"143a747aeb3a57270ddff2f49e600f7fc5995aeb3d16479371e8171b11fe36b8"),
            ("video.raw",351859,"5a5d034282dc82dbc6146edcf241cf5461946f3b98307bbb0c87940dd4e3daa8"),
            ("audio.raw",327619,"0cb1e1a7fdbc59526c2ecb43e6842451770600cfd0bab12cd9818488f94cf951")]),
        ("automatic", ".runtime/youtube-auto-record-qa/local.atsumi.youtubeautoqa.aeb71d4637694e9bb3aa16f2ce0605fb/media",
         "3e5fee79-6417-4c82-9e85-295a72a97675", [
            ("proof.json",942u64,"4c56a1470d38dc13a2b7c235915069f94114117357346132369e89a949251c07"),
            ("journal.jsonl",89029,"2c0fbff2566fe6281d861dd91e9c87f744f0d49ae7afc90d8845d2ddc17a561d"),
            ("video.raw",1380681,"3bb0b865a31c196591e8f6f1055006059fa1a26a00817e4b3eb9a96dbe391919"),
            ("audio.raw",573806,"74430cceecb478bbf2a5d6e39a85d96cfb809a16a2a8199438b132de671f208a")]),
    ];
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let bin = workspace.join(".runtime/media-tools").join(managed_probe::PACKAGE).join("bin");
    let tools = PlatformMediaTools {
        yt_dlp: Default::default(), deno: Default::default(),
        ffmpeg: bin.join("ffmpeg.exe"), ffprobe: bin.join("ffprobe.exe"),
    };
    let cancel = AtomicBool::new(false);
    for (label, relative, job, specs) in cases {
        let root = workspace.join(relative);
        let directories = Directories::hold(&root.join("YouTube").join(job)).unwrap();
        let held: Vec<_> = specs.iter().map(|(name, length, digest)| {
            let file = GuardedFile::open(&root.join("YouTube").join(job).join(name), false).unwrap();
            assert!(check_hash(&file, *length, digest), "fixed source prehash failed");
            file
        }).collect();
        // Includes assertion panics in the index/finalizer/verified-path callback:
        // all held source posthashes must still run before failure propagates.
        let report = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let identity = recovery_identity(&root, job, &cancel).unwrap();
        let mut full_video: Option<(i128, i128, u64, u64, u64)> = None;
        let (_prefix, outcomes) = inspect_and_index(&root, &identity, &cancel, &mut |track, unit| {
            if unit.ordinal >= 16 { return Err(()); }
            if track == Track::Video {
                let clock = unit.facts.clock;
                if let Some(value) = full_video.as_mut() {
                    assert_eq!(value.1, clock.first);
                    assert_eq!(value.2, clock.ticks_per_second);
                    value.1 = clock.end;
                    value.3 += unit.facts.items;
                    value.4 += 1;
                } else {
                    full_video = Some((clock.first, clock.end, clock.ticks_per_second, unit.facts.items, 1));
                }
            }
            Ok(())
        }).unwrap();
        assert!(outcomes.iter().all(|v| v.unwrap().terminal == index::Terminal::Complete));
        directories.check().unwrap();
        let (first, end, scale, packets, units) = full_video.unwrap();
        let deadline = Instant::now() + Duration::from_secs(240);
        let candidate = prepare_recording(&root, &identity, &tools, &cancel, deadline)
            .expect("actual complete source must pass strict production finalization");
        candidate.with_verified_path(&cancel, deadline, |path, facts| {
            let selected = facts.selected.expect("selected native provenance");
            let video = selected.facts().tracks[0];
            assert_eq!(video.selected.start, 0);
            assert_eq!(video.selected.end, specs[2].1);
            assert_eq!((video.selected.first_tick, video.selected.end_tick, video.selected.ticks_per_second), (first, end, scale));
            assert_eq!(video.selected.last_ordinal + 1, units);
            assert!(video.leading.is_none() && video.trailing.is_none());
            assert_eq!(facts.comparisons[0].count, packets);
            assert_eq!(facts.comparisons[1].count, selected.audio().retained().count);
            assert!(!facts.incomplete_append && !facts.torn_journal);
            assert_eq!(facts.uncommitted_bytes, [0,0]);
            Ok(serde_json::json!({"case":label,"candidate":path,"bytes":facts.bytes,
                "sha256":facts.sha256.iter().map(|b|format!("{b:02x}")).collect::<String>(),
                "videoPackets":facts.comparisons[0].count,"audioPackets":facts.comparisons[1].count,
                "videoFirst":first.to_string(),"videoEnd":end.to_string(),"videoScale":scale,
                "videoUnits":units,"originalsUnchanged":true,"published":false,"actualPlayerVerified":false}))
        }).unwrap()
        }));
        let mut originals_unchanged = true;
        for (file, (_, length, digest)) in held.iter().zip(specs) {
            // &= deliberately evaluates every fixed file, even after failure.
            originals_unchanged &= check_hash(file, length, digest);
        }
        originals_unchanged &= directories.check().is_ok();
        assert!(originals_unchanged, "fixed source posthash/guard failed");
        let report = match report {
            Ok(value) => value,
            Err(panic) => std::panic::resume_unwind(panic),
        };
        // with_verified_path has completed its candidate/source postchecks,
        // and every held original has now passed its final pin check.
        println!("{report}");
    }
}
