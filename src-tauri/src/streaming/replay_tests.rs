use super::super::browser_store::BrowserMergedOutput;
use super::*;
use std::io::{BufWriter, Write};
use tauri::http::{header, Request, StatusCode};

const CHANNEL: &str = "0123456789abcdef0123456789abcdef";
const WEBM: &[u8] = &[
    0x1a, 0x45, 0xdf, 0xa3, 0x9f, 0x42, 0x86, 0x81, 1, 0, 0, 0, 0, 0, 0, 0,
];
#[test]
#[cfg(windows)]
fn deletion_rejects_active_replay_and_blocks_new_replay_after_reservation() {
    let fixture = fixture(5, 16);
    let opened = fixture.service.open(&fixture.id).unwrap();
    assert_eq!(
        fixture
            .service
            .prepare_recording_delete(&fixture.id)
            .unwrap_err()
            .code,
        "BROWSER_DELETE_REPLAY_BUSY"
    );
    fixture.service.close(&opened.token).unwrap();
    // Reap the synthetic index worker deterministically, including its source handles.
    let workers = std::mem::take(&mut *fixture.service.inner.workers.lock().unwrap());
    for worker in workers {
        worker.handle.join().unwrap();
    }
    let job = fixture
        .service
        .prepare_recording_delete(&fixture.id)
        .unwrap();
    assert!(fixture.service.open(&fixture.id).is_err());
    assert!(fixture
        .store
        .lock()
        .unwrap()
        .merged_file(&fixture.id)
        .is_err());
    super::super::browser_store::deletion::remove_files(&job).unwrap();
    fixture.service.delete_recording_cache(&fixture.id).unwrap();
    fixture
        .store
        .lock()
        .unwrap()
        .finish_delete(&job, Ok(()))
        .unwrap();
    assert!(!fixture.output.exists());
    assert!(fixture.store.lock().unwrap().snapshot().unwrap().is_empty());
}

#[test]
#[cfg(windows)]
fn deletion_removes_saved_chat_index_and_offset_without_other_recordings() {
    let fixture = fixture(2, 16);
    let opened = fixture.service.open(&fixture.id).unwrap();
    fixture.service.set_offset(&opened.token, 12.0).unwrap();
    let workers = std::mem::take(&mut *fixture.service.inner.workers.lock().unwrap());
    for worker in workers {
        worker.handle.join().unwrap();
    }
    let index_path = fixture
        .service
        .session(&opened.token)
        .unwrap()
        .index_path
        .clone();
    assert!(index_path.exists());
    fixture.service.close(&opened.token).unwrap();
    let job = fixture
        .service
        .prepare_recording_delete(&fixture.id)
        .unwrap();
    super::super::browser_store::deletion::remove_files(&job).unwrap();
    fixture.service.delete_recording_cache(&fixture.id).unwrap();
    fixture
        .store
        .lock()
        .unwrap()
        .finish_delete(&job, Ok(()))
        .unwrap();
    assert!(!index_path.exists());
    let root = fixture.service.cache_root().unwrap();
    assert_eq!(
        fixture.service.read_offset(&root, &fixture.id).unwrap(),
        0.0
    );
}
#[test]
fn saved_channel_profile_reaches_replay_without_chat_index_or_network() {
    use base64::Engine;
    let fixture = fixture(0, 16);
    let asset_id = "a".repeat(64);
    let bytes = include_bytes!("../../../public/original-player/assets/default_profile_dark.png");
    let body = base64::engine::general_purpose::STANDARD.encode(bytes);
    fs::create_dir(fixture.output.join("replay-assets")).unwrap();
    fs::write(fixture.output.join("replay-assets").join(format!("{asset_id}.json")), serde_json::to_vec(&serde_json::json!({
        "version":1,"mime":"image/png","body":body,"sha256":format!("{:x}",Sha256::digest(bytes))
    })).unwrap()).unwrap();
    fs::write(
        fixture.output.join("channel-profile.json"),
        serde_json::to_vec(&serde_json::json!({
            "version":1,"channelId":CHANNEL,"name":"저장 채널","imageAssetId":asset_id
        }))
        .unwrap(),
    )
    .unwrap();
    let opened = fixture.service.open(&fixture.id).unwrap();
    assert_eq!(opened.channel_name.as_deref(), Some("저장 채널"));
    assert_eq!(
        opened.channel_profile_image,
        Some(format!("data:image/png;base64,{body}"))
    );
    fixture.service.close(&opened.token).unwrap();
    // A corrupted decoration is optional: it cannot make the video unplayable.
    fs::write(fixture.output.join("channel-profile.json"), b"broken").unwrap();
    let reopened = fixture.service.open(&fixture.id).unwrap();
    assert!(reopened.channel_name.is_none());
    assert!(reopened.channel_profile_image.is_none());
}

struct Fixture {
    service: ReplayService,
    id: String,
    output: PathBuf,
    media: PathBuf,
    store: Arc<Mutex<BrowserCaptureStore>>,
    _directory: tempfile::TempDir,
}
fn fixture(rows: u64, media_bytes: u64) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let store = BrowserCaptureStore::new(directory.path()).unwrap();
    let recording = store
        .begin(
            directory.path(),
            CHANNEL,
            "합성 다시보기",
            "video/webm;codecs=vp8,opus",
        )
        .unwrap();
    store.append(&recording.id, 0, 0, WEBM).unwrap();
    store.finish_segment(&recording.id, 0, 120.0).unwrap();
    store
        .finish_with_chat(&recording.id, false, None, true, "partial", rows)
        .unwrap();
    let output = PathBuf::from(&recording.output_dir);
    let mut chat = BufWriter::new(File::create(output.join("chat.jsonl")).unwrap());
    for sequence in 1..=rows {
        writeln!(chat,"{}",serde_json::json!({"sequence":sequence,"sender":"닉네임","text":format!("채팅 {sequence}"),"serverTime":null,"receivedAt":sequence,"offsetSeconds":sequence as f64 / rows.max(1) as f64 * 100.0,"broadcastOffsetSeconds":null})).unwrap();
    }
    chat.flush().unwrap();
    drop(chat);
    let job = store.take_merge_job().unwrap().unwrap();
    let media_name = format!("merged-{}.webm", job.token);
    let media = output.join(&media_name);
    let mut media_file = File::create(&media).unwrap();
    media_file.write_all(WEBM).unwrap();
    media_file
        .set_len(media_bytes.max(WEBM.len() as u64))
        .unwrap();
    drop(media_file);
    let timeline_name = format!("merged-{}.timeline.jsonl", job.token);
    fs::write(output.join(&timeline_name),format!("{}\n",serde_json::json!({"segmentIndex":0,"sourceFile":"segment-000000000000.webm","mergedStartSeconds":0.0,"mergedDurationSeconds":120.0,"sourceStartSeconds":100.0,"sourceEndSeconds":220.0,"chatRewritten":false}))).unwrap();
    store
        .complete_merge(
            &job,
            BrowserMergedOutput {
                file: media_name,
                timeline_file: timeline_name,
                bytes: media_bytes.max(WEBM.len() as u64),
                duration_seconds: 120.0,
                cleanup: None,
            },
        )
        .unwrap();
    let store = Arc::new(Mutex::new(store));
    let service = ReplayService::new(directory.path(), store.clone());
    Fixture {
        service,
        id: recording.id,
        output,
        media,
        store,
        _directory: directory,
    }
}
fn open_ready(fixture: &Fixture) -> ReplaySession {
    let opened = fixture.service.open(&fixture.id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let descriptor = fixture
            .service
            .session(&opened.token)
            .unwrap()
            .descriptor()
            .unwrap();
        if descriptor.index_state == "ready" {
            return descriptor;
        }
        assert_ne!(
            descriptor.index_state, "failed",
            "{:?}",
            descriptor.warnings
        );
        assert!(Instant::now() < deadline, "index worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn request(token: &str, range: Option<&str>) -> Request<Vec<u8>> {
    let mut builder = Request::builder().uri(format!("http://atsumi-replay.localhost/{token}"));
    if let Some(range) = range {
        builder = builder.header(header::RANGE, range);
    }
    builder.body(vec![]).unwrap()
}

#[test]
fn replay_saved_metadata_and_literal_search_cover_all_messages_with_scoped_cursors() {
    let fixture = fixture(450, 16);
    let opened = open_ready(&fixture);
    assert_eq!(opened.title, "합성 다시보기");
    assert!(opened.recorded_at > 0);
    let first = fixture
        .service
        .chat_search(&opened.token, "채팅", "body", None, 1, Some(200))
        .unwrap();
    assert_eq!(first.items.len(), 200);
    assert_eq!(first.items[0].message.sequence, 251);
    let cursor = first.previous_cursor.as_deref().unwrap();
    let second = fixture
        .service
        .chat_search(&opened.token, "채팅", "body", Some(cursor), 2, Some(200))
        .unwrap();
    assert_eq!(second.items.len(), 200);
    assert_eq!(second.items[0].message.sequence, 51);
    let third = fixture
        .service
        .chat_search(
            &opened.token,
            "채팅",
            "body",
            second.previous_cursor.as_deref(),
            3,
            Some(200),
        )
        .unwrap();
    assert_eq!(third.items.len(), 50);
    assert!(third.previous_cursor.is_none());
    assert!(fixture
        .service
        .chat_search(&opened.token, "다른 검색", "body", Some(cursor), 4, None)
        .is_err());
    assert!(fixture
        .service
        .chat_page(&opened.token, Some(cursor), 5, None)
        .is_err());
    let names = fixture
        .service
        .chat_search(&opened.token, "닉네임", "nickname", None, 6, Some(10))
        .unwrap();
    assert_eq!(names.items.len(), 10);
    assert!(fixture
        .service
        .chat_search(&opened.token, "닉네임", "body", None, 7, None)
        .unwrap()
        .items
        .is_empty());
    assert!(fixture
        .service
        .chat_search(&opened.token, "%_' OR 1=1 --", "all", None, 8, None)
        .unwrap()
        .items
        .is_empty());
    assert_eq!(index::normalize_search("  ＡＢＣ\t가  "), "abc 가");
    assert!(fixture
        .service
        .chat_search(&opened.token, &"가".repeat(257), "all", None, 9, None)
        .is_err());
    assert!(fixture
        .service
        .chat_search(&opened.token, "", "unsupported", None, 9, None)
        .is_err());
    assert!(fixture
        .service
        .chat_search(&opened.token, "채팅", "body", None, 1, None)
        .is_err());
}

#[test]
fn public_viewer_api_cadence_covers_ten_seconds_and_bounds_stale_and_corrupt_cache() {
    let fixture = fixture(0, 16);
    let rows = [
        (0.0, Some(10)),
        (10.0, Some(0)),
        (20.0, None),
        (30.0, Some(40)),
    ]
    .into_iter()
    .map(|(at, count)| {
        serde_json::json!({
            "version":1,"source":"chzzk_live_status_api_v1","receivedAt":1000+(at*1000.0) as u64,
            "offsetSeconds":at,"viewerCount":count,
            "replayClock":{"version":1,"receivedAtMs":1000+(at*1000.0) as u64,
                "observedMonotonicMs":at*1000.0,"sourceGeneration":1,"clock":"player_observation",
                "playbackRate":2.0}
        })
        .to_string()
    })
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    fs::write(fixture.output.join("viewer-metrics.jsonl"), rows).unwrap();
    let opened = open_ready(&fixture);
    let timeline = fixture.service.timeline(&opened.token, Some(2.0)).unwrap();
    assert_eq!(timeline.viewer_metric_status, "partial");
    assert!(timeline.buckets[..5]
        .iter()
        .all(|bucket| bucket.viewer_count == Some(10) && bucket.viewer_coverage_seconds == 2.0));
    assert!(timeline.buckets[5..10]
        .iter()
        .all(|bucket| bucket.viewer_count == Some(0) && bucket.viewer_coverage_seconds == 2.0));
    assert!(timeline.buckets[10..15]
        .iter()
        .all(|bucket| bucket.viewer_count.is_none()));
    assert_eq!(timeline.buckets[22].viewer_count, Some(40));
    assert_eq!(timeline.buckets[22].viewer_coverage_seconds, 1.0); // 45s, not a fabricated 60s at 2x without a source mapping.
    assert_eq!(timeline.buckets[23].viewer_count, None);
    assert_eq!(
        timeline
            .buckets
            .iter()
            .map(|bucket| bucket.viewer_sample_count)
            .sum::<u64>(),
        3
    );
    let session = fixture.service.session(&opened.token).unwrap();
    let connection = rusqlite::Connection::open(&session.index_path).unwrap();
    connection
        .execute("UPDATE viewer_samples SET hold_seconds=1e300", [])
        .unwrap();
    assert!(fixture.service.timeline(&opened.token, Some(2.0)).is_err());
    drop(connection);
    fixture.service.shutdown_and_wait();
}

#[test]
fn viewer_samples_map_to_media_and_keep_unknown_gaps_and_saved_profiles() {
    let fixture = fixture(1, 16);
    let profile = format!("https://chzzk.naver.com/{CHANNEL}");
    fs::write(fixture.output.join("chat.jsonl"),format!("{}\n",serde_json::json!({"sequence":1,"sender":"합성","text":"test","serverTime":null,"receivedAt":1000,"offsetSeconds":0.0,"rich":{"nicknameColor":"#123456","textColor":"#abcdef","profileUrl":profile,"badges":[],"emojis":[]}}))).unwrap();
    let rows=[(0.0,Some(0)),(1.0,Some(100)),(2.0,None),(4.0,Some(60))].into_iter().map(|(at,count)|serde_json::json!({
        "version":1,"source":"chzzk_video_info_dom_v1","receivedAt":1000+(at*1000.0) as u64,"offsetSeconds":at+50.0,"viewerCount":count,
        "replayClock":{"version":1,"receivedAtMs":1000+(at*1000.0) as u64,"observedMonotonicMs":at*1000.0,"sourceGeneration":1,"clock":"mse_presentation_v1","mediaTimeSeconds":100.0+at,"sourceTimeSeconds":100.0+at,"sourceId":"40000000-0000-4000-8000-000000000001","playbackRate":1.0}
    }).to_string()).collect::<Vec<_>>().join("\n")+"\n";
    fs::write(fixture.output.join("viewer-metrics.jsonl"), rows).unwrap();
    let session = open_ready(&fixture);
    let timeline = fixture.service.timeline(&session.token, Some(2.0)).unwrap();
    assert_eq!(timeline.viewer_metric_status, "partial");
    assert_eq!(timeline.buckets[0].viewer_count, Some(50));
    assert_eq!(timeline.buckets[0].viewer_sample_count, 2);
    assert_eq!(timeline.buckets[0].viewer_coverage_seconds, 2.0);
    assert_eq!(timeline.buckets[1].viewer_count, None);
    assert_eq!(timeline.buckets[2].viewer_count, Some(60));
    assert_eq!(timeline.buckets[5].viewer_count, None);
    assert_eq!(
        fixture.service.profile_url(&session.token, 1).unwrap(),
        profile
    );
    assert!(fixture.service.profile_url(&session.token, 2).is_err());
    assert!(fixture.service.profile_url("../bad", 1).is_err());
    let page = fixture
        .service
        .chat_at(&session.token, 0.0, 1, None)
        .unwrap();
    assert_eq!(
        page.items[0]
            .message
            .rich
            .as_ref()
            .unwrap()
            .text_color
            .as_deref(),
        Some("#abcdef")
    );
    fs::write(fixture.output.join("viewer-metrics.jsonl"), "changed").unwrap();
    assert!(fixture.service.timeline(&session.token, Some(2.0)).is_err());
    fixture.service.shutdown_and_wait();
}

#[test]
fn profile_links_are_optional_strict_and_do_not_require_legacy_backfill() {
    use super::super::model::public_profile_url;
    let good = format!("https://chzzk.naver.com/{CHANNEL}");
    assert_eq!(public_profile_url(&good), Some(good));
    for value in [
        "javascript:alert(1)",
        "https://chzzk.naver.com.evil.test/0123456789abcdef0123456789abcdef",
        "https://chzzk.naver.com/0123456789abcdef0123456789abcdef?token=secret",
        "https://chzzk.naver.com/live/0123456789abcdef0123456789abcdef",
        "https://chzzk.naver.com/@name",
        "https://user:pass@chzzk.naver.com/0123456789abcdef0123456789abcdef",
    ] {
        assert!(public_profile_url(value).is_none());
    }
    let fixture = fixture(1, 16);
    let session = open_ready(&fixture);
    assert!(fixture.service.profile_url(&session.token, 1).is_err());
    assert_eq!(
        fixture
            .service
            .timeline(&session.token, Some(2.0))
            .unwrap()
            .viewer_metric_status,
        "not_recorded"
    );
    fixture.service.shutdown_and_wait();
}

#[test]
fn catalog_authorization_rejects_arbitrary_ids_and_unfinished_recordings() {
    let fixture = fixture(0, 16);
    for id in [
        "../escape",
        "https://example.com/media.webm",
        "C:\\private.webm",
        "",
        CHANNEL,
    ] {
        assert!(fixture.service.open(id).is_err());
    }
    let active = fixture
        .store
        .lock()
        .unwrap()
        .begin(fixture._directory.path(), CHANNEL, "active", "video/webm")
        .unwrap();
    assert!(fixture
        .store
        .lock()
        .unwrap()
        .replay_source(&active.id)
        .is_err());
    fixture.service.shutdown_and_wait();
}

#[test]
fn local_protocol_is_bounded_seekable_and_token_authorized() {
    let fixture = fixture(0, 3 * 1024 * 1024);
    let opened = open_ready(&fixture);
    let full = fixture
        .service
        .media_response(&request(&opened.token, None), "main");
    assert_eq!(full.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        full.headers()[header::CONTENT_LENGTH],
        full.body().len().to_string()
    );
    let part = fixture
        .service
        .media_response(&request(&opened.token, Some("bytes=0-")), "main");
    assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.body().len(), 1024 * 1024);
    assert_eq!(&part.body()[..WEBM.len()], WEBM);
    assert_eq!(
        part.headers()[header::CONTENT_RANGE],
        "bytes 0-1048575/3145728"
    );
    let seek = fixture
        .service
        .media_response(&request(&opened.token, Some("bytes=2097152-")), "main");
    assert_eq!(seek.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        seek.headers()[header::CONTENT_RANGE],
        "bytes 2097152-3145727/3145728"
    );
    let head = Request::builder()
        .method("HEAD")
        .uri(format!("atsumi-replay://localhost/{}", opened.token))
        .body(vec![])
        .unwrap();
    let head = fixture.service.media_response(&head, "main");
    assert_eq!(head.status(), StatusCode::OK);
    assert!(head.body().is_empty());
    assert_eq!(head.headers()[header::CONTENT_LENGTH], "3145728");
    let bad = fixture
        .service
        .media_response(&request(&opened.token, Some("bytes=9000000-")), "main");
    assert_eq!(bad.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(bad.headers()[header::CONTENT_RANGE], "bytes */3145728");
    assert_eq!(
        fixture
            .service
            .media_response(&request(&opened.token, Some("bytes=0-1")), "chzzk-official")
            .status(),
        StatusCode::FORBIDDEN
    );
    let remote = Request::builder()
        .uri(format!("atsumi-replay://localhost/{}", opened.token))
        .header(header::ORIGIN, "https://chzzk.naver.com")
        .body(vec![])
        .unwrap();
    assert_eq!(
        fixture.service.media_response(&remote, "main").status(),
        StatusCode::FORBIDDEN
    );
    for path in [
        format!("{}/../private", opened.token),
        format!("{}?path=C:/private", opened.token),
        format!("%2e%2e/{}", opened.token),
    ] {
        let request = Request::builder()
            .uri(format!("atsumi-replay://localhost/{path}"))
            .body(vec![])
            .unwrap();
        assert_eq!(
            fixture.service.media_response(&request, "main").status(),
            StatusCode::BAD_REQUEST
        );
    }
    fixture.service.close(&opened.token).unwrap();
    assert_eq!(
        fixture
            .service
            .media_response(&request(&opened.token, Some("bytes=0-1")), "main")
            .status(),
        StatusCode::NOT_FOUND
    );
    fixture.service.shutdown_and_wait();
}

#[test]
fn seeks_offsets_log_cursors_and_generations_are_independent_of_wall_time() {
    let fixture = fixture(1000, 16);
    let original = fs::read(fixture.output.join("chat.jsonl")).unwrap();
    let opened = open_ready(&fixture);
    let page = fixture
        .service
        .chat_at(&opened.token, 80.0, 2, None)
        .unwrap();
    assert_eq!(page.items.len(), 200);
    assert!(page
        .items
        .iter()
        .all(|item| item.media_time_seconds <= 80.0));
    assert_eq!(page.items.last().unwrap().message.sequence, 800);
    let older = fixture
        .service
        .chat_page(&opened.token, page.previous_cursor.as_deref(), 2, None)
        .unwrap();
    assert_eq!(older.items.last().unwrap().message.sequence, 600);
    let backward = fixture
        .service
        .chat_at(&opened.token, 1.0, 3, None)
        .unwrap();
    assert_eq!(backward.items.len(), 10);
    assert!(fixture
        .service
        .chat_at(&opened.token, 80.0, 2, None)
        .is_err());
    fixture.service.set_offset(&opened.token, 5.0).unwrap();
    let corrected = fixture
        .service
        .chat_at(&opened.token, 6.0, 4, None)
        .unwrap();
    assert_eq!(corrected.items.len(), 10);
    assert_eq!(corrected.items.last().unwrap().media_time_seconds, 6.0);
    assert!(fixture.service.set_offset(&opened.token, f64::NAN).is_err());
    assert!(fixture.service.set_offset(&opened.token, 3600.1).is_err());
    let first_path = fixture
        .service
        .session(&opened.token)
        .unwrap()
        .index_path
        .clone();
    fixture.service.close(&opened.token).unwrap();
    let reopened = open_ready(&fixture);
    assert_eq!(reopened.manual_offset_seconds, 5.0);
    assert_eq!(
        fixture.service.session(&reopened.token).unwrap().index_path,
        first_path
    );
    assert!(fixture
        .service
        .chat_page(&reopened.token, page.previous_cursor.as_deref(), 0, None)
        .is_err());
    assert_eq!(
        fs::read(fixture.output.join("chat.jsonl")).unwrap(),
        original
    );
    fixture.service.shutdown_and_wait();
}

#[test]
fn large_log_index_and_pages_remain_bounded_and_complete() {
    let fixture = fixture(200_000, 16);
    let opened = open_ready(&fixture);
    for (generation, time) in [0.1, 50.0, 99.0, 2.0].into_iter().enumerate() {
        let page = fixture
            .service
            .chat_at(&opened.token, time, generation as u64, Some(usize::MAX))
            .unwrap();
        assert!(page.items.len() <= 200);
        assert!(serde_json::to_vec(&page).unwrap().len() <= MAX_PAGE_BYTES);
        assert!(page
            .items
            .iter()
            .all(|item| item.media_time_seconds <= time));
    }
    let timeline = fixture.service.timeline(&opened.token, Some(10.0)).unwrap();
    assert_eq!(
        timeline
            .buckets
            .iter()
            .map(|bucket| bucket.chat_count)
            .sum::<u64>(),
        200_000
    );
    assert!(timeline
        .buckets
        .iter()
        .all(|bucket| bucket.viewer_count.is_none() && bucket.unique_sender_count.is_none()));
    assert_eq!(timeline.viewer_metric_status, "not_recorded");
    let page = fixture
        .service
        .chat_page(&opened.token, None, 4, Some(200))
        .unwrap();
    assert_eq!(page.items.last().unwrap().message.sequence, 200_000);
    fixture.service.shutdown_and_wait();
}

#[test]
fn damaged_rows_clock_regressions_and_truncated_tail_preserve_sources() {
    let fixture = fixture(0, 16);
    let row = |sequence: u64, time: f64| {
        format!(
            "{}\n",
            serde_json::json!({"sequence":sequence,"sender":"x","text":"line","receivedAt":0,"serverTime":null,"offsetSeconds":time})
        )
    };
    let raw = format!(
        "{}{}{}{}\n{{bad\n{}",
        row(1, 10.0),
        row(2, 5.0),
        row(2, 6.0),
        "x".repeat(MAX_LINE_BYTES + 100),
        row(3, 8.0).trim_end()
    );
    fs::write(fixture.output.join("chat.jsonl"), raw.as_bytes()).unwrap();
    let opened = open_ready(&fixture);
    let page = fixture
        .service
        .chat_at(&opened.token, 20.0, 0, None)
        .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|item| item.message.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(page.warnings.len() >= 4);
    assert_eq!(
        fs::read_to_string(fixture.output.join("chat.jsonl")).unwrap(),
        raw
    );
    fixture.service.shutdown_and_wait();
}

#[test]
fn validated_observed_presentation_time_maps_through_merge_and_unknown_stays_approximate() {
    let fixture = fixture(0, 16);
    let row = serde_json::json!({"sequence":1,"sender":"x","text":"observed","receivedAt":1,"serverTime":null,"offsetSeconds":99.0,"senderKey":format!("sha256:{}","a".repeat(64)),"replayClock":{"version":1,"clock":"mse_presentation_v1","receivedAtMs":1,"observedMonotonicMs":1000.0,"sourceGeneration":1,"mediaTimeSeconds":102.5,"playbackRate":1.0,"sourceId":uuid::Uuid::new_v4().to_string(),"sourceTimeSeconds":102.5}});
    fs::write(fixture.output.join("chat.jsonl"), format!("{row}\n")).unwrap();
    let opened = open_ready(&fixture);
    let page = fixture
        .service
        .chat_at(&opened.token, 3.0, 0, None)
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].media_time_seconds, 2.5);
    assert_eq!(page.items[0].sync_quality, "observed_media");
    let timeline = fixture.service.timeline(&opened.token, Some(10.0)).unwrap();
    assert_eq!(timeline.buckets[0].unique_sender_count, Some(1));
    fixture.service.shutdown_and_wait();
}

#[test]
fn source_mutation_and_cancelled_worker_invalidate_access() {
    let cancelled = fixture(10000, 16);
    let opened = cancelled.service.open(&cancelled.id).unwrap();
    cancelled.service.close(&opened.token).unwrap();
    assert!(cancelled
        .service
        .chat_at(&opened.token, 0.0, 0, None)
        .is_err());
    cancelled.service.shutdown_and_wait();
    assert!(cancelled.service.inner.workers.lock().unwrap().is_empty());
    assert!(cancelled.service.open(&cancelled.id).is_err());
    let fixture = fixture(1, 16);
    let opened = open_ready(&fixture);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&fixture.media)
        .unwrap()
        .write_all(b"changed")
        .unwrap();
    assert!(fixture
        .service
        .chat_at(&opened.token, 0.0, 0, None)
        .is_err());
    assert_eq!(
        fixture
            .service
            .media_response(&request(&opened.token, Some("bytes=0-1")), "main")
            .status(),
        StatusCode::NOT_FOUND
    );
    fixture.service.shutdown_and_wait();
}

#[test]
fn corrupt_derived_index_is_rebuilt_without_rewriting_chat() {
    let fixture = fixture(20, 16);
    let opened = open_ready(&fixture);
    let path = fixture
        .service
        .session(&opened.token)
        .unwrap()
        .index_path
        .clone();
    fixture.service.close(&opened.token).unwrap();
    fs::write(path, b"broken sqlite derivative").unwrap();
    let reopened = open_ready(&fixture);
    assert_eq!(
        fixture
            .service
            .chat_page(&reopened.token, None, 0, None)
            .unwrap()
            .items
            .len(),
        20
    );
    fixture.service.shutdown_and_wait();
}

#[test]
fn rapid_open_close_cannot_exceed_live_index_worker_slots() {
    let fixture = fixture(0, 16);
    let source = fixture
        .store
        .lock()
        .unwrap()
        .replay_source(&fixture.id)
        .unwrap();
    let probe = fixture
        .service
        .make_session(source, &fixture.service.cache_root().unwrap())
        .unwrap();
    let builder_lock = Arc::new(Mutex::new(()));
    let held = builder_lock.lock().unwrap();
    fixture
        .service
        .inner
        .index_locks
        .lock()
        .unwrap()
        .insert(probe.fingerprint, Arc::downgrade(&builder_lock));
    for _ in 0..MAX_SESSIONS {
        let opened = fixture.service.open(&fixture.id).unwrap();
        fixture.service.close(&opened.token).unwrap();
    }
    assert_eq!(
        fixture.service.inner.workers.lock().unwrap().len(),
        MAX_SESSIONS
    );
    assert_eq!(
        fixture.service.open(&fixture.id).unwrap_err().code,
        "REPLAY_INDEX_BUSY"
    );
    drop(held);
    fixture.service.shutdown_and_wait();
    assert!(fixture.service.inner.workers.lock().unwrap().is_empty());
}

#[test]
fn corrupt_cache_payload_cannot_create_an_unbounded_ipc_page() {
    let fixture = fixture(1, 16);
    let opened = open_ready(&fixture);
    let session = fixture.service.session(&opened.token).unwrap();
    let connection = rusqlite::Connection::open(&session.index_path).unwrap();
    connection
        .execute("UPDATE messages SET payload=zeroblob(2000000)", [])
        .unwrap();
    drop(connection);
    assert!(fixture
        .service
        .chat_at(&opened.token, 100.0, 0, None)
        .is_err());
    fixture.service.shutdown_and_wait();
}

#[test]
fn corrupt_large_profile_payload_is_rejected_by_bounded_lookup() {
    let fixture = fixture(1, 16);
    let opened = open_ready(&fixture);
    let session = fixture.service.session(&opened.token).unwrap();
    let connection = rusqlite::Connection::open(&session.index_path).unwrap();
    connection
        .execute("UPDATE messages SET payload=zeroblob(4000000)", [])
        .unwrap();
    drop(connection);
    assert_eq!(
        fixture
            .service
            .profile_url(&opened.token, 1)
            .unwrap_err()
            .code,
        "REPLAY_STORAGE"
    );
    fixture.service.shutdown_and_wait();
}

#[test]
fn oversized_cache_metadata_is_rebuilt_without_changing_recording() {
    for update in [
        "UPDATE metadata SET warnings='[\"' || hex(zeroblob(1000000)) || '\"]'",
        "UPDATE metadata SET source=hex(zeroblob(1000000))",
    ] {
        let fixture = fixture(1, 16);
        let original = fs::read(fixture.output.join("chat.jsonl")).unwrap();
        let opened = open_ready(&fixture);
        let session = fixture.service.session(&opened.token).unwrap();
        let connection = rusqlite::Connection::open(&session.index_path).unwrap();
        connection.execute(update, []).unwrap();
        drop(connection);
        fixture.service.close(&opened.token).unwrap();
        let reopened = open_ready(&fixture);
        assert_eq!(
            fixture
                .service
                .chat_page(&reopened.token, None, 0, None)
                .unwrap()
                .items
                .len(),
            1
        );
        assert_eq!(
            fs::read(fixture.output.join("chat.jsonl")).unwrap(),
            original
        );
        fixture.service.shutdown_and_wait();
    }
}

#[test]
fn keyed_chat_empty_buckets_are_zero_but_missing_identities_remain_unknown() {
    let fixture = fixture(0, 16);
    let mut raw = String::new();
    for (sequence, time, key) in [(1, 0.0, true), (2, 4.0, true), (3, 6.0, false)] {
        raw.push_str(&format!("{}\n", serde_json::json!({"sequence":sequence,"sender":"fixture","text":"chat","receivedAt":1,"offsetSeconds":time,"senderKey":key.then(||format!("sha256:{}","a".repeat(64)))})));
    }
    fs::write(fixture.output.join("chat.jsonl"), raw).unwrap();
    let opened = open_ready(&fixture);
    let timeline = fixture.service.timeline(&opened.token, Some(2.0)).unwrap();
    assert_eq!(timeline.buckets[0].unique_sender_count, Some(1));
    assert_eq!(timeline.buckets[1].chat_count, 0);
    assert_eq!(timeline.buckets[1].unique_sender_count, Some(0));
    assert_eq!(timeline.buckets[2].unique_sender_count, Some(1));
    assert_eq!(timeline.buckets[3].chat_count, 1);
    assert_eq!(timeline.buckets[3].unique_sender_count, None);
    assert_eq!(timeline.buckets[4].unique_sender_count, Some(0));
    fixture.service.shutdown_and_wait();
}
