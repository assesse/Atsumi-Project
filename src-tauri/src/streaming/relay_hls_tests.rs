use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    thread::JoinHandle,
};

#[test]
fn local_test_factory_is_exact_loopback_and_does_not_relax_production_target() {
    for value in ["127.0.0.1:0", "127.0.0.2:4321", "192.0.2.1:4321", "[::1]:4321"] {
        assert!(matches!(HlsTarget::loopback_test(value.parse().unwrap()), Err(DeliveryFault::Target)));
    }
    assert!(HlsTarget::loopback_test("127.0.0.1:4321".parse().unwrap()).is_ok());
    assert!(matches!(HlsTarget::from_api("http://127.0.0.1:4321/http_upload_hls?cid=fixture&copy=0&file="), Err(DeliveryFault::Target)));
}

#[derive(Clone, Copy)]
enum Reply {
    Status(u16),
    Truncated,
    Stall,
    Drip,
}
#[derive(Clone)]
struct Seen {
    name: String,
    body: Vec<u8>,
    connection: usize,
}
struct Server {
    target: Option<HlsTarget>,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    task: Option<JoinHandle<()>>,
}
impl Server {
    fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let target = HlsTarget(
            Url::parse(&format!(
                "http://{address}/http_upload_hls?cid=fixture&copy=0&file="
            ))
            .unwrap(),
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let output = seen.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let task = std::thread::spawn(move || {
            let end = Instant::now() + Duration::from_secs(15);
            let mut connection = 0;
            let mut index = 0;
            while !stopping.load(Ordering::Acquire) && Instant::now() < end && index < replies.len()
            {
                let socket = match listener.accept() {
                    Ok((socket, _)) => socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("local listener: {error}"),
                };
                connection += 1;
                socket
                    .set_read_timeout(Some(Duration::from_millis(100)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut socket = BufReader::new(socket);
                while !stopping.load(Ordering::Acquire)
                    && Instant::now() < end
                    && index < replies.len()
                {
                    let Some((name, body)) = request(&mut socket, &stopping, end) else {
                        break;
                    };
                    output.lock().unwrap().push(Seen {
                        name,
                        body,
                        connection,
                    });
                    let reply = replies[index];
                    index += 1;
                    match reply {
                        Reply::Status(status) => {
                            write!(socket.get_mut(), "HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n").unwrap();
                            socket.get_mut().flush().unwrap();
                        }
                        Reply::Truncated => {
                            socket
                                .get_mut()
                                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nx")
                                .unwrap();
                            break;
                        }
                        Reply::Stall => {
                            while !stopping.load(Ordering::Acquire) && Instant::now() < end {
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            break;
                        }
                        Reply::Drip => {
                            socket
                                .get_mut()
                                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2048\r\n\r\n")
                                .unwrap();
                            while !stopping.load(Ordering::Acquire) && Instant::now() < end {
                                if socket.get_mut().write_all(b"x").is_err() {
                                    break;
                                }
                                std::thread::sleep(Duration::from_millis(100));
                            }
                            break;
                        }
                    }
                }
            }
        });
        Self {
            target: Some(target),
            seen,
            stop,
            task: Some(task),
        }
    }
    fn delivery(&mut self, cancel: Arc<AtomicBool>) -> HlsDelivery {
        HlsDelivery::new(self.target.take().unwrap(), cancel).unwrap()
    }
    fn rows(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let joined = self.task.take().unwrap().join();
        if !std::thread::panicking() {
            assert!(joined.is_ok(), "local server thread failed");
        }
    }
}
fn request(
    socket: &mut BufReader<TcpStream>,
    stop: &AtomicBool,
    end: Instant,
) -> Option<(String, Vec<u8>)> {
    let mut first = String::new();
    loop {
        if stop.load(Ordering::Acquire) || Instant::now() >= end {
            return None;
        }
        match socket.read_line(&mut first) {
            Ok(0) => return None,
            Ok(_) => break,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(_) => return None,
        }
    }
    assert!(first.len() < 2048 && first.starts_with("PUT "));
    let path = first.split_whitespace().nth(1).unwrap();
    let url = Url::parse(&format!("http://localhost{path}")).unwrap();
    let name = url
        .query_pairs()
        .find(|(key, _)| key == "file")
        .unwrap()
        .1
        .into_owned();
    let mut length = None;
    let mut total = first.len();
    loop {
        let mut line = String::new();
        match socket.read_line(&mut line) {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
        total += line.len();
        assert!(total < 8192);
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            assert!(length.is_none());
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let length = length.unwrap();
    assert!(length <= MAX_SEGMENT);
    let mut body = vec![0; length];
    let mut received = 0;
    while received < length {
        if stop.load(Ordering::Acquire) || Instant::now() >= end {
            return None;
        }
        match socket.read(&mut body[received..]) {
            Ok(0) => return None,
            Ok(bytes) => received += bytes,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(_) => return None,
        }
    }
    Some((name, body))
}
fn packet() -> Arc<[u8]> {
    let mut data = vec![0; 188];
    data[0] = 0x47;
    data.into()
}
fn cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[test]
fn endpoint_template_rejects_other_hosts_credentials_queries_and_transports() {
    for value in [
        "https://a.upload.youtube.com/http_upload_hls?cid=test-key&copy=0&file=",
        "https://b.upload.youtube.com/http_upload_hls?file=&copy=1&cid=test_key",
    ] {
        assert!(HlsTarget::from_api(value).is_ok());
    }
    for value in [
        "http://a.upload.youtube.com/http_upload_hls?cid=x&copy=0&file=",
        "https://a.upload.youtube.com.evil.test/http_upload_hls?cid=x&copy=0&file=",
        "https://user@a.upload.youtube.com/http_upload_hls?cid=x&copy=0&file=",
        "https://a.upload.youtube.com/http_upload_hls?cid=x&copy=1&file=",
        "https://a.upload.youtube.com/http_upload_hls?cid=x&cid=y&file=",
        "https://a.upload.youtube.com/http_upload_hls?cid=x&copy=0&file=foo.ts",
        "https://a.upload.youtube.com/http_upload_hls?cid=x&copy=0&file=&extra=1",
        "https://a.upload.youtube.com/http_upload_hls?cid=x&copy=0&file=#fragment",
    ] {
        assert!(HlsTarget::from_api(value).is_err(), "{value}");
    }
}

#[test]
fn real_http_orders_bounded_playlists_segments_and_final_ack_on_one_connection() {
    let mut server = Server::new(vec![Reply::Status(200); 11]);
    let mut delivery = server.delivery(cancel());
    let stream = Uuid::new_v4();
    for ordinal in 0..5 {
        delivery
            .deliver_parts(stream, ordinal, 2001, packet())
            .unwrap();
    }
    assert!(!delivery.progress().end_list_acked);
    let end = delivery.finish_parts(stream, 5).unwrap();
    assert_eq!(end.segment_acks, 5);
    assert_eq!(end.playlist_acks, 6);
    assert!(end.end_list_acked);
    assert_eq!(end.acknowledged_bytes, 5 * 188);
    assert_eq!(delivery.finish_parts(stream, 5).unwrap(), end);
    let rows = server.rows();
    assert_eq!(rows.len(), 11);
    assert!(rows.iter().all(|row| row.connection == 1));
    for ordinal in 0usize..5 {
        let playlist = std::str::from_utf8(&rows[ordinal * 2].body).unwrap();
        let segment = &rows[ordinal * 2 + 1];
        assert_eq!(rows[ordinal * 2].name, "index.m3u8");
        assert!(segment.name.ends_with(&format!("-{ordinal}.ts")));
        assert!(playlist.contains(&segment.name));
        assert!(playlist.contains(&format!(
            "#EXT-X-MEDIA-SEQUENCE:{}\n",
            ordinal.saturating_sub(2)
        )));
        assert_eq!(playlist.matches("#EXTINF:").count(), (ordinal + 1).min(3));
        assert!(!playlist.contains("ENDLIST"));
        assert_eq!(&segment.body[..], packet().as_ref());
    }
    let final_list = std::str::from_utf8(&rows[10].body).unwrap();
    assert!(final_list.ends_with("#EXT-X-ENDLIST\n"));
    assert!(final_list.contains("#EXT-X-MEDIA-SEQUENCE:3\n"));
}

#[test]
fn retry_reuses_exact_segment_name_bytes_and_accepts_202() {
    let mut server = Server::new(vec![
        Reply::Status(200),
        Reply::Status(500),
        Reply::Status(202),
        Reply::Status(200),
    ]);
    let mut delivery = server.delivery(cancel());
    let stream = Uuid::new_v4();
    delivery.deliver_parts(stream, 0, 2000, packet()).unwrap();
    delivery.finish_parts(stream, 1).unwrap();
    let rows = server.rows();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[1].name, rows[2].name);
    assert_eq!(rows[1].body, rows[2].body);
    assert_eq!(delivery.progress().segment_acks, 1);
}

#[test]
fn playlist_keeps_native_clock_precision_instead_of_rounding_each_segment_to_milliseconds() {
    let mut server = Server::new(vec![Reply::Status(200); 3]);
    let mut delivery = server.delivery(cancel());
    let stream = Uuid::new_v4();
    delivery
        .deliver_ticks(stream, 0, 180_001, packet())
        .unwrap();
    delivery.finish_parts(stream, 1).unwrap();
    let rows = server.rows();
    assert!(std::str::from_utf8(&rows[0].body)
        .unwrap()
        .contains("#EXTINF:2.000011111,\n"));
}

#[test]
fn truncated_final_response_is_not_completion_and_preserves_segment_receipt() {
    let mut server = Server::new(vec![
        Reply::Status(200),
        Reply::Status(200),
        Reply::Truncated,
        Reply::Truncated,
        Reply::Truncated,
    ]);
    let mut delivery = server.delivery(cancel());
    let stream = Uuid::new_v4();
    delivery.deliver_parts(stream, 0, 2000, packet()).unwrap();
    assert_eq!(
        delivery.finish_parts(stream, 1),
        Err(DeliveryFault::Response)
    );
    assert_eq!(delivery.progress().segment_acks, 1);
    assert!(!delivery.progress().end_list_acked);
    assert_eq!(
        delivery.finish_parts(stream, 1),
        Err(DeliveryFault::Response)
    );
    let rows = server.rows();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[2].body, rows[3].body);
    assert_eq!(rows[3].body, rows[4].body);
}

#[test]
fn rejected_segment_never_skips_or_sends_endlist() {
    let mut server = Server::new(vec![Reply::Status(200), Reply::Status(401)]);
    let mut delivery = server.delivery(cancel());
    let stream = Uuid::new_v4();
    assert_eq!(
        delivery.deliver_parts(stream, 0, 2000, packet()),
        Err(DeliveryFault::Rejected(401))
    );
    assert_eq!(
        delivery.finish_parts(stream, 1),
        Err(DeliveryFault::Rejected(401))
    );
    assert_eq!(delivery.progress().segment_acks, 0);
    assert_eq!(server.rows().len(), 2);
}

#[test]
fn cross_source_or_missing_end_proof_cannot_finish_or_send_new_object() {
    let mut server = Server::new(vec![Reply::Status(200), Reply::Status(200)]);
    let mut delivery = server.delivery(cancel());
    let stream = Uuid::new_v4();
    delivery.deliver_parts(stream, 0, 2000, packet()).unwrap();
    assert_eq!(
        delivery.finish_parts(stream, 2),
        Err(DeliveryFault::UnfinishedInput)
    );
    assert_eq!(server.rows().len(), 2);
    let mut other_server = Server::new(vec![Reply::Status(200), Reply::Status(200)]);
    let mut other = other_server.delivery(cancel());
    other.deliver_parts(stream, 0, 2000, packet()).unwrap();
    assert_eq!(
        other.deliver_parts(Uuid::new_v4(), 1, 2000, packet()),
        Err(DeliveryFault::Sequence)
    );
    assert_eq!(other_server.rows().len(), 2);
}

#[test]
fn wrong_order_oversized_duration_and_cancellation_do_not_open_connection() {
    for (ordinal, duration, cancelled, expected) in [
        (1, 2000, false, DeliveryFault::Sequence),
        (0, 5001, false, DeliveryFault::Bounds),
        (0, 2000, true, DeliveryFault::Cancelled),
    ] {
        let mut server = Server::new(vec![]);
        let cancellation = cancel();
        cancellation.store(cancelled, Ordering::Release);
        let mut delivery = server.delivery(cancellation);
        assert_eq!(
            delivery.deliver_parts(Uuid::new_v4(), ordinal, duration, packet()),
            Err(expected)
        );
        assert_eq!(delivery.progress().requests, 0);
        assert!(server.rows().is_empty());
    }
}

#[test]
fn stalled_response_cancellation_has_bounded_wait_and_never_reports_success() {
    let mut server = Server::new(vec![Reply::Stall]);
    let cancellation = cancel();
    let trigger = cancellation.clone();
    let observed = server.seen.clone();
    let cancel_task = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while observed.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        trigger.store(true, Ordering::Release);
    });
    let mut delivery = server.delivery(cancellation);
    let start = Instant::now();
    assert_eq!(
        delivery.deliver_parts(Uuid::new_v4(), 0, 2000, packet()),
        Err(DeliveryFault::Cancelled)
    );
    assert!(start.elapsed() < Duration::from_secs(4));
    assert_eq!(delivery.progress().requests, 1);
    assert_eq!(delivery.progress().segment_acks, 0);
    cancel_task.join().unwrap();
}

#[test]
fn trickling_success_response_cannot_renew_deadline_or_delay_cancel_indefinitely() {
    let mut server = Server::new(vec![Reply::Drip]);
    let cancellation = cancel();
    let trigger = cancellation.clone();
    let observed = server.seen.clone();
    let task = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while observed.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        trigger.store(true, Ordering::Release);
    });
    let mut delivery = server.delivery(cancellation);
    let start = Instant::now();
    assert_eq!(
        delivery.deliver_parts(Uuid::new_v4(), 0, 2000, packet()),
        Err(DeliveryFault::Cancelled)
    );
    assert!(start.elapsed() < Duration::from_secs(4));
    assert_eq!(delivery.progress().playlist_acks, 0);
    assert_eq!(delivery.progress().segment_acks, 0);
    task.join().unwrap();
}

#[cfg(windows)]
#[test]
#[ignore = "Pinned original read-only; native TS to actual loopback HTTP; no external request/media file"]
fn actual_source_mux_to_http_end_ack_preserves_every_object_and_original() {
    use super::super::browser::encoded::fmp4::EncodedTrackInput;
    use super::super::relay_ingress::{
        InputEnd, JoinResult, NativeIdentity, RelayBinding, RelayProvider, RelayRun,
    };
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        io::{Seek, SeekFrom},
        path::Path,
    };
    assert_eq!(
        std::env::var("ATSUMI_TS_HLS_QA").as_deref(),
        Ok("exact-source-loopback")
    );
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let source = workspace.join(".runtime/soop-original-qa/baa5338478ee4c8cb1cdda57d278d918/media/SOOP/8fd00700-81d4-401f-a083-0d13fcf286ea/e00000008.mp4");
    use std::os::windows::fs::MetadataExt;
    for part in source.ancestors() {
        let metadata = fs::symlink_metadata(part).unwrap();
        assert!(!metadata.file_type().is_symlink() && metadata.file_attributes() & 0x400 == 0);
    }
    let source = fs::canonicalize(source).unwrap();
    let _directories =
        crate::streaming::browser_store::cleanup::pin_directories(source.parent().unwrap())
            .unwrap();
    let mut original =
        crate::streaming::browser_store::cleanup::verification_guard(&source).unwrap();
    assert_eq!(original.metadata().unwrap().len(), 1_789_078);
    let mut bytes = Vec::new();
    (&mut original)
        .take(2 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .unwrap();
    let hash = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
    let source_hash = "861f08cf68605ff4e7393e652a149d137552aaef4093b981637dc6e43911d088";
    assert_eq!(hash(&bytes), source_hash);
    let mut split = 0;
    for kind in [b"ftyp", b"moov"] {
        let length = u32::from_be_bytes(bytes[split..split + 4].try_into().unwrap()) as usize;
        assert!(length >= 8 && split + length <= bytes.len());
        assert_eq!(&bytes[split + 4..split + 8], kind);
        split += length;
    }
    let tracks = vec![EncodedTrackInput {
        track_index: 0,
        mime_type: "video/mp4;codecs=avc1.640028,mp4a.40.2".into(),
        init: bytes[..split].to_vec(),
    }];
    let binding = RelayBinding::new(NativeIdentity {
        provider: RelayProvider::Soop,
        owner: Uuid::new_v4(),
        run: Uuid::new_v4(),
        source: Uuid::new_v4(),
        document_epoch: 1,
    })
    .unwrap();
    let (mut worker, input, output) = RelayRun::start_ts(binding.clone(), tracks).unwrap();
    let input_end = Instant::now() + Duration::from_secs(5);
    while !input.codec_admitted() {
        assert!(Instant::now() < input_end);
        std::thread::sleep(Duration::from_millis(2));
    }
    let chunks = bytes[split..].chunks(128 * 1024);
    let count = chunks.len();
    for (index, chunk) in chunks.enumerate() {
        input
            .try_enqueue(&binding, 0, 0, index as u32, index + 1 == count, chunk)
            .unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
    input.finish();
    let mut server = Server::new(vec![Reply::Status(200); 25]);
    let mut delivery = server.delivery(cancel());
    let mut segments = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    let report = loop {
        while let Ok(guard) = output.try_recv() {
            assert_eq!(guard.binding().identity(), binding.identity());
            let segment = guard.ts_segment().unwrap();
            assert!(output.held_bytes() >= segment.bytes().len());
            delivery.deliver(segment).unwrap();
            // Guard still holds queue credit after the complete HTTP response.
            assert!(output.held_bytes() >= segment.bytes().len());
            segments.push(serde_json::json!({"ordinal":segment.ordinal(), "milliseconds":segment.duration_ms(),
                "bytes":segment.bytes().len(), "sha256":hash(segment.bytes())}));
            drop(guard);
        }
        match worker.join(Duration::from_millis(0)) {
            JoinResult::Joined(report) => {
                if segments.len() as u64 == report.counts.output_chunks {
                    break report;
                }
            }
            JoinResult::Pending => {}
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    };
    let InputEnd::TsEnded(end) = report.input_end else {
        panic!("input did not finish: {:?}", report.input_end);
    };
    assert!(!segments.is_empty() && segments.len() <= 12);
    assert_eq!(segments.len() as u64, end.segments());
    assert_eq!(output.held_bytes(), 0);
    let finished = delivery.finish(&end).unwrap();
    assert!(finished.end_list_acked);
    let rows = server.rows();
    assert_eq!(rows.len(), segments.len() * 2 + 1);
    for (index, segment) in segments.iter().enumerate() {
        assert_eq!(
            hash(&rows[index * 2 + 1].body),
            segment["sha256"].as_str().unwrap()
        );
        assert_eq!(segment["ordinal"].as_u64().unwrap(), index as u64);
        let playlist = std::str::from_utf8(&rows[index * 2].body).unwrap();
        assert!(playlist.contains(&rows[index * 2 + 1].name));
    }
    assert!(std::str::from_utf8(&rows.last().unwrap().body)
        .unwrap()
        .ends_with("#EXT-X-ENDLIST\n"));
    original.seek(SeekFrom::Start(0)).unwrap();
    let mut after = Vec::new();
    original.read_to_end(&mut after).unwrap();
    assert_eq!(hash(&after), source_hash);
    let directory = workspace
        .join(".runtime/requirements-audit/20261117-cloud-goal-resume")
        .join(format!("ts-http-actual-{}", Uuid::new_v4().simple()));
    fs::create_dir(&directory).unwrap();
    let report = serde_json::json!({"passed":true,"sourceSha256":source_hash,"sourceBytes":bytes.len(),
        "segments":segments,"commonWorkerJoined":true,"queueCreditHeldThroughAck":true,"finalOutputHeldBytes":output.held_bytes(),
        "objects":rows.len(),"connections":rows.iter().map(|r|r.connection).max(),
        "segmentAcks":finished.segment_acks,"playlistAcks":finished.playlist_acks,"endListAcked":finished.end_list_acked,
        "originalUnchanged":true,"allReceivedSegmentBodiesEqual":true,"mediaFiles":0,"externalRequests":0,
        "actualYouTubeIngestion":false,"liveAppIntegration":false,"codecDecodeClaim":false});
    fs::write(
        directory.join("result.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!(
        "actual_ts_http_result={}",
        directory.join("result.json").display()
    );
}
