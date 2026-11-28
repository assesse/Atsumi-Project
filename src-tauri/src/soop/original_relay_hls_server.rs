//! Bounded, test-only local receiver. No external address or disk media output.
use crate::streaming::relay_hls::HlsTarget;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const OBJECT_BYTES: usize = 16 * 1024 * 1024;
const OBJECTS: usize = 96;
#[derive(Default)]
struct State {
    objects: Vec<Value>,
    retained: Vec<Arc<Vec<u8>>>,
    error: Option<String>,
    connections: u64,
}
pub(super) struct Server {
    address: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Server {
    pub(super) fn start() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "qa_listener")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "qa_listener_mode")?;
        let address = listener.local_addr().map_err(|_| "qa_listener_address")?;
        let state = Arc::new(Mutex::new(State::default()));
        let output = state.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            let result = serve(listener, &output, &stopping);
            if let Err(error) = result {
                output.lock().unwrap_or_else(|e| e.into_inner()).error = Some(error);
            }
        });
        Ok(Self {
            address,
            state,
            stop,
            worker: Some(worker),
        })
    }
    pub(super) fn target(&self) -> HlsTarget {
        HlsTarget::loopback_test(self.address).expect("fixed QA localhost")
    }
    pub(super) fn snapshot(&self) -> Value {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        json!({"objects":state.objects,"connections":state.connections,"error":state.error,
            "retainedSegments":state.retained.len(), "externalDestination":false})
    }
    pub(super) fn segments(&self) -> Vec<Arc<Vec<u8>>> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retained
            .clone()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                assert!(result.is_ok(), "QA server join");
            }
        }
    }
}
fn expired(stop: &AtomicBool, end: Instant) -> bool {
    stop.load(Ordering::Acquire) || Instant::now() >= end
}
fn read_exact_bounded(
    reader: &mut BufReader<TcpStream>,
    bytes: &mut [u8],
    stop: &AtomicBool,
    end: Instant,
) -> Result<bool, String> {
    let mut offset = 0;
    while offset < bytes.len() {
        if expired(stop, end) {
            return Ok(false);
        }
        match reader.read(&mut bytes[offset..]) {
            Ok(0) => return Ok(false),
            Ok(size) => offset += size,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return Ok(false),
        }
    }
    Ok(true)
}
fn line(
    reader: &mut BufReader<TcpStream>,
    stop: &AtomicBool,
    end: Instant,
) -> Result<Option<String>, String> {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0];
        if !read_exact_bounded(reader, &mut byte, stop, end)? {
            return Ok(None);
        }
        if bytes.len() >= 2048 {
            return Err("qa_header_bound".into());
        }
        bytes.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "qa_header_utf8".into())
}
fn serve(listener: TcpListener, state: &Mutex<State>, stop: &AtomicBool) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs(600);
    while !expired(stop, end) {
        let socket = match listener.accept() {
            Ok((socket, address)) if address.ip().is_loopback() => socket,
            Ok(_) => return Err("qa_peer".into()),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(_) => return Err("qa_accept".into()),
        };
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(|_| "qa_read_timeout")?;
        socket
            .set_write_timeout(Some(Duration::from_secs(1)))
            .map_err(|_| "qa_write_timeout")?;
        let connection = {
            let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
            s.connections += 1;
            if s.connections > 8 {
                return Err("qa_connection_bound".into());
            }
            s.connections
        };
        let mut reader = BufReader::new(socket);
        while !expired(stop, end) {
            let Some(first) = line(&mut reader, stop, end)? else {
                break;
            };
            let fields = first.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 3 || fields[0] != "PUT" {
                return Err("qa_method".into());
            }
            let url = reqwest::Url::parse(&format!("http://localhost{}", fields[1]))
                .map_err(|_| "qa_request_url")?;
            if url.path() != "/http_upload_hls" {
                return Err("qa_request_path".into());
            }
            let pairs = url.query_pairs().collect::<Vec<_>>();
            if pairs.len() != 3
                || pairs.iter().filter(|(k, _)| k == "file").count() != 1
                || !pairs.iter().any(|(k, v)| k == "cid" && v == "fixture")
                || !pairs.iter().any(|(k, v)| k == "copy" && v == "0")
            {
                return Err("qa_request_query".into());
            }
            let name = pairs
                .iter()
                .find(|(k, _)| k == "file")
                .unwrap()
                .1
                .to_string();
            if name.len() > 128
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            {
                return Err("qa_object_name".into());
            }
            let mut length = None;
            let mut total = first.len();
            loop {
                let Some(header) = line(&mut reader, stop, end)? else {
                    return Ok(());
                };
                total += header.len();
                if total > 8192 {
                    return Err("qa_headers_bound".into());
                }
                if header == "\r\n" {
                    break;
                }
                if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                    if length.is_some() {
                        return Err("qa_duplicate_length".into());
                    }
                    length = Some(value.trim().parse::<usize>().map_err(|_| "qa_length")?);
                }
            }
            let length = length
                .filter(|n| *n > 0 && *n <= OBJECT_BYTES)
                .ok_or("qa_body_bound")?;
            let mut body = vec![0; length];
            if !read_exact_bounded(&mut reader, &mut body, stop, end)? {
                break;
            }
            let playlist = name.ends_with(".m3u8");
            let end_list;
            if playlist {
                if body.len() > 8192 {
                    return Err("qa_playlist_bound".into());
                }
                let text = std::str::from_utf8(&body).map_err(|_| "qa_playlist_utf8")?;
                if !text.starts_with("#EXTM3U\n") {
                    return Err("qa_playlist_shape".into());
                }
                end_list = text.contains("#EXT-X-ENDLIST");
            } else {
                if !name.ends_with(".ts")
                    || body.len() < 376
                    || body.len() % 188 != 0
                    || !body.chunks_exact(188).all(|p| p[0] == 0x47)
                    || body[1] & 0x1f != 0
                    || body[2] != 0
                {
                    return Err("qa_ts_packets".into());
                }
                end_list = false;
            }
            let row = json!({"name":name,"bytes":length,"sha256":format!("{:x}",Sha256::digest(&body)),
                "playlist":playlist,"endList":end_list,"connection":connection});
            {
                let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                if s.objects.len() >= OBJECTS {
                    return Err("qa_objects_bound".into());
                }
                s.objects.push(row);
                if !playlist && s.retained.len() < 2 {
                    s.retained.push(Arc::new(body));
                }
            }
            reader
                .get_mut()
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n",
                )
                .map_err(|_| "qa_ack_write")?;
            reader.get_mut().flush().map_err(|_| "qa_ack_flush")?;
        }
    }
    if !stop.load(Ordering::Acquire) {
        return Err("qa_server_deadline".into());
    }
    Ok(())
}
