use super::*;
use crate::streaming::browser::encoded::fmp4::fixtures;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::AtomicUsize,
};

#[derive(Clone, Copy)]
enum Response {
    Ack,
    RejectSegment,
    HoldEnd,
    HoldFirst,
}
struct Server {
    address: std::net::SocketAddr,
    reached: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    worker: Option<JoinHandle<()>>,
}
impl Server {
    fn new(response: Response) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let reached = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let (s, r, l, n) = (
            stop.clone(),
            reached.clone(),
            release.clone(),
            requests.clone(),
        );
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(12);
            while !s.load(Ordering::Acquire) && Instant::now() < deadline {
                let mut stream = match listener.accept() {
                    Ok((stream, peer)) => {
                        assert!(peer.ip().is_loopback());
                        stream
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(_) => break,
                };
                stream
                    .set_read_timeout(Some(Duration::from_millis(50)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_millis(100)))
                    .unwrap();
                let Some((segment, end)) = request(&mut stream, &s, deadline) else {
                    continue;
                };
                let count = n.fetch_add(1, Ordering::AcqRel);
                if matches!(response, Response::HoldEnd) && end
                    || matches!(response, Response::HoldFirst) && count == 0
                {
                    // Complete headers are insufficient for ACK: hold the last
                    // response body bytes while owner/join/cancel are checked.
                    if stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nx",
                        )
                        .is_err()
                    {
                        continue;
                    }
                    r.store(true, Ordering::Release);
                    while !l.load(Ordering::Acquire)
                        && !s.load(Ordering::Acquire)
                        && Instant::now() < deadline
                    {
                        thread::sleep(Duration::from_millis(1));
                    }
                    let _ = stream.write_all(b"y");
                } else {
                    let code = if segment && matches!(response, Response::RejectSegment) {
                        401
                    } else {
                        200
                    };
                    let _ = stream.write_all(format!("HTTP/1.1 {code} Result\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
                }
            }
        });
        Self {
            address,
            reached,
            release,
            stop,
            requests,
            worker: Some(worker),
        }
    }
    fn wait_reached(&self) {
        let end = Instant::now() + Duration::from_secs(3);
        while !self.reached.load(Ordering::Acquire) {
            assert!(
                Instant::now() < end,
                "bounded HTTP response was not reached"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.release.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
fn request(stream: &mut TcpStream, stop: &AtomicBool, deadline: Instant) -> Option<(bool, bool)> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut header_end = None;
    while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue
            }
            Err(_) => return None,
        }
        if header_end.is_none() {
            header_end = bytes
                .windows(4)
                .position(|b| b == b"\r\n\r\n")
                .map(|at| at + 4);
        }
        let Some(at) = header_end else {
            assert!(bytes.len() <= 8192);
            continue;
        };
        let header = std::str::from_utf8(&bytes[..at]).unwrap();
        let length = header
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                    .map(|(_, v)| v.trim().parse::<usize>().unwrap())
            })
            .expect("bounded known body");
        assert!(length <= 16 * 1024 * 1024);
        if bytes.len() >= at + length {
            let segment = header.lines().next().unwrap().contains(".ts");
            let end = bytes[at..at + length]
                .windows(b"#EXT-X-ENDLIST\n".len())
                .any(|b| b == b"#EXT-X-ENDLIST\n");
            return Some((segment, end));
        }
    }
    None
}
struct Harness {
    slot: Slot,
    context: Context,
    source: Uuid,
}
impl Harness {
    fn new(server: &Server) -> Self {
        let h = Self {
            slot: Slot::default(),
            context: Context {
                owner: Uuid::new_v4(),
                run: Uuid::new_v4(),
                epoch: 17,
                channel: "fixture".into(),
                broadcast: "123456789".into(),
            },
            source: Uuid::new_v4(),
        };
        install_manager(
            &h.slot,
            h.context.clone(),
            Replies::default(),
            None,
            ManagerMode::Hls {
                target: HlsTarget::loopback_test(server.address).unwrap(),
                receiver: None,
            },
        )
        .unwrap();
        h
    }
    fn call(&self, kind: &str, fields: Value) -> Option<Result<Value, String>> {
        dispatch_fields(
            &self.slot,
            &self.context.channel,
            &self.context.broadcast,
            self.context.epoch,
            false,
            Envelope {
                intent: None,
                id: Uuid::new_v4().to_string(),
                epoch: self.context.epoch,
                kind: kind.into(),
                fields,
            },
        )
    }
    fn begin(&self) {
        let tracks:Vec<_>=fixtures::inputs().into_iter().map(|t|json!({"trackIndex":t.track_index,"mimeType":t.mime_type,"init":STANDARD.encode(t.init)})).collect();
        assert!(self.call("relay_begin",json!({"requestId":self.context.run.to_string(),"sourceId":self.source.to_string(),"channelId":self.context.channel,"actualChannel":self.context.channel,"actualBroadcast":self.context.broadcast,"tracks":tracks})).is_none());
        let end = Instant::now() + Duration::from_secs(3);
        loop {
            if lock(&self.slot.owner)
                .as_ref()
                .is_some_and(|o| lock(&o.shared).admitted)
            {
                break;
            }
            assert!(Instant::now() < end, "codec admission deadline");
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn append(&self, track: u32, append: u64) {
        let (count, duration) = if track == 0 {
            (60u32, 3000u32)
        } else {
            (94, 1024)
        };
        let mut bytes = fixtures::fragment(
            track,
            append * u64::from(count) * u64::from(duration),
            count,
            true,
        );
        let trun = bytes.windows(4).position(|b| b == b"trun").unwrap();
        for n in 0..count as usize {
            let at = trun + 16 + n * 16;
            bytes[at..at + 4].copy_from_slice(&duration.to_be_bytes());
        }
        assert!(self.call("relay_append",json!({"runId":self.context.run.to_string(),"sourceId":self.source.to_string(),"actualChannel":self.context.channel,"actualBroadcast":self.context.broadcast,"trackIndex":track,"appendIndex":append,"chunkIndex":0,"finalChunk":true,"data":STANDARD.encode(bytes)})).unwrap().is_ok());
    }
    fn finish(&self, interrupted: bool) -> Value {
        self.call("relay_finish",json!({"runId":self.context.run.to_string(),"sourceId":self.source.to_string(),"actualChannel":self.context.channel,"actualBroadcast":self.context.broadcast,"interrupted":interrupted,"reason":"fixture_stop"})).unwrap().unwrap()
    }
    fn source(&self) {
        self.begin();
        for a in 0..2 {
            for t in 0..2 {
                self.append(t, a);
            }
        }
    }
    fn result(&self) -> Inspection {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            self.slot.reap();
            if let Some(result) = lock(&self.slot.last).clone() {
                return result;
            }
            assert!(Instant::now() < end, "manager actual join deadline");
            thread::sleep(Duration::from_millis(1));
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.slot.cancel();
        let end = Instant::now() + Duration::from_secs(5);
        while self.slot.active() && Instant::now() < end {
            self.slot.reap();
            thread::sleep(Duration::from_millis(1));
        }
        assert!(!self.slot.active(), "owned manager was not joined");
    }
}

#[test]
fn normal_finish_requires_final_response_body_and_actual_manager_join() {
    let server = Server::new(Response::HoldEnd);
    let h = Harness::new(&server);
    h.source();
    let ack = h.finish(false);
    assert_eq!(ack["stopRequested"], true);
    assert!(ack.get("joined").is_none());
    server.wait_reached();
    h.slot.reap();
    assert!(h.slot.active());
    let snapshot = lock(&lock(&h.slot.owner).as_ref().unwrap().progress).clone();
    assert!(!snapshot.manager_joined);
    assert!(!snapshot.delivery.unwrap().end_list_acked);
    server.release.store(true, Ordering::Release);
    let result = h.result();
    assert!(
        result.input_ended && result.worker_joined && result.manager_joined && !result.interrupted
    );
    let d = result.delivery.unwrap();
    assert!(d.end_list_acked);
    assert_eq!((d.segment_acks, d.playlist_acks, d.requests), (2, 3, 5));
    assert_eq!(result.validated_appends, 4);
    assert_eq!(result.output_chunks, 2);
    assert!(result.sidecar.is_none());
}
#[test]
fn delivery_rejection_survives_clean_worker_end() {
    let server = Server::new(Response::RejectSegment);
    let h = Harness::new(&server);
    h.source();
    h.finish(false);
    let result = h.result();
    let d = result.delivery.unwrap();
    assert!(result.manager_joined && result.worker_joined && result.interrupted);
    assert_eq!(d.fault.as_deref(), Some("Rejected(401)"));
    assert_eq!(result.fault.as_deref(), Some("Delivery:Rejected(401)"));
    assert_eq!(d.segment_acks, 0);
    assert_eq!(d.playlist_acks, 1);
    assert!(!d.end_list_acked);
}
#[test]
fn immediate_cancel_interrupts_inflight_body_but_does_not_invent_ack() {
    let server = Server::new(Response::HoldFirst);
    let h = Harness::new(&server);
    h.source();
    h.finish(false);
    server.wait_reached();
    let at = Instant::now();
    h.slot.cancel();
    assert!(h.slot.active());
    let result = h.result();
    assert!(at.elapsed() < Duration::from_secs(4));
    assert!(result.manager_joined && result.worker_joined && result.interrupted);
    let d = result.delivery.unwrap();
    assert!(!d.end_list_acked);
    assert_eq!(d.playlist_acks, 0);
    assert_eq!(d.segment_acks, 0);
    assert_eq!(d.fault.as_deref(), Some("Cancelled"));
}
#[test]
fn normal_stop_deadline_without_page_finish_cancels_only_current_owner() {
    let server = Server::new(Response::Ack);
    let h = Harness::new(&server);
    h.begin();
    {
        let owner = lock(&h.slot.owner);
        let mut state = lock(&owner.as_ref().unwrap().shared);
        state.stop_deadline = Some(Instant::now());
    }
    let result = h.result();
    assert!(result.interrupted && result.manager_joined && result.worker_joined);
    assert_eq!(server.requests.load(Ordering::Acquire), 0);
}
#[test]
fn stale_finish_cannot_close_source_bound_delivery() {
    let server = Server::new(Response::Ack);
    let h = Harness::new(&server);
    h.source();
    assert!(h.call("relay_finish",json!({"runId":h.context.run.to_string(),"sourceId":Uuid::new_v4().to_string(),"actualChannel":h.context.channel,"actualBroadcast":h.context.broadcast,"interrupted":true,"reason":"stale"})).unwrap().is_err());
    h.finish(false);
    let result = h.result();
    assert!(!result.interrupted);
    assert!(result.delivery.unwrap().end_list_acked);
}

#[test]
fn unwind_joins_actual_chat_writer_and_common_worker_before_cleanup_proof() {
    let (sidecar, root) = crate::platform_media::relay_chat::test_fixture();
    let shared = Arc::new(Mutex::new(Admission::default()));
    let proof = Arc::new(AtomicBool::new(true));
    let binding = RelayBinding::new(NativeIdentity {
        provider: RelayProvider::Soop,
        owner: Uuid::new_v4(),
        run: Uuid::new_v4(),
        source: Uuid::new_v4(),
        document_epoch: 1,
    })
    .unwrap();
    let (worker, input, output) = RelayRun::start_ts(binding.clone(), fixtures::inputs()).unwrap();
    lock(&shared).input = Some((input, binding));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut resources = Resources::new(shared.clone(), proof.clone(), None);
        resources.sidecar = Some(sidecar);
        resources.worker = Some(worker);
        assert!(!proof.load(Ordering::Acquire));
        panic!("controlled manager unwind");
    }));
    assert!(outcome.is_err());
    assert!(proof.load(Ordering::Acquire));
    assert!(lock(&shared).input.is_none());
    assert!(matches!(output.try_recv(), Err(TryRecvError::Disconnected)));
    let directories: Vec<_> = std::fs::read_dir(root.path().join("SOOP/Relays"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(directories.len(), 1);
    let summary: Value =
        serde_json::from_slice(&std::fs::read(directories[0].join("relay-summary.json")).unwrap())
            .unwrap();
    assert_eq!(summary["interrupted"], true);
    // The writer's original receipt is preserved. No MediaJob/video is made.
    assert_eq!(summary["rawRows"], 0);
    assert_eq!(summary["eventRows"], 0);
    use std::os::windows::fs::OpenOptionsExt;
    let _closed = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(directories[0].join("relay-receive.jsonl"))
        .unwrap();
}

#[test]
fn native_sidecar_path_is_not_serialized_and_text_coverage_stays_independent() {
    let mut report = Inspection::default();
    report.sidecar(RelayChatSummary {
        sidecar_id: Uuid::new_v4().to_string(),
        directory: std::path::PathBuf::from("native-only"),
        raw_rows: 1,
        event_rows: 1,
        bytes: 20,
        interrupted: false,
        assets_incomplete: true,
        failure: None,
        text_state: "partial".into(),
        joined: true,
        clock_quality: "receive-monotonic-only",
        timing_records: 0,
        timing_incomplete: false,
    });
    let body = serde_json::to_value(&report).unwrap();
    assert!(body.get("sidecarDirectory").is_none());
    assert!(body["sidecar"].get("directory").is_none());
    assert_eq!(body["sidecar"]["textState"], "partial");
    assert_eq!(body["sidecar"]["assetsIncomplete"], true);
    assert_eq!(body["sidecar"]["clockQuality"], "receive-monotonic-only");
    assert!(report.delivery.is_none());
    assert!(!report.input_ended);
}
