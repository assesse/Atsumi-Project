//! Browser-owned gateway. Never installs system routes, DNS or proxy settings.
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Direct,
    Blocked,
    Vpn(u16),
}
struct Policy {
    route: Route,
    fragment: bool,
    generation: u64,
}
pub struct Gateway {
    pub port: u16,
    policy: Mutex<Policy>,
    sockets: Mutex<HashMap<u64, TcpStream>>,
    sequence: AtomicU64,
    stopping: AtomicBool,
}
impl Gateway {
    pub fn start() -> io::Result<Arc<Self>> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let this = Arc::new(Self {
            port: listener.local_addr()?.port(),
            policy: Mutex::new(Policy {
                route: Route::Direct,
                fragment: false,
                generation: 0,
            }),
            sockets: Mutex::default(),
            sequence: AtomicU64::new(0),
            stopping: AtomicBool::new(false),
        });
        let copy = this.clone();
        thread::Builder::new()
            .name("browser-gateway".into())
            .spawn(move || {
                while !copy.stopping.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            // Winsock inherits the listener's nonblocking mode. The per-connection
                            // workers use blocking Read/Write; leaving it set ends TLS mid-handshake.
                            if stream.set_nonblocking(false).is_err() {
                                continue;
                            }
                            if copy.sockets.lock().unwrap().len() >= 128 {
                                let _ = stream.shutdown(Shutdown::Both);
                                continue;
                            }
                            let id = copy.sequence.fetch_add(1, Ordering::Relaxed);
                            let Ok(clone) = stream.try_clone() else {
                                continue;
                            };
                            copy.sockets.lock().unwrap().insert(id, clone);
                            let gateway = copy.clone();
                            thread::spawn(move || {
                                if let Err(error) = gateway.serve(stream, id) {
                                    #[cfg(debug_assertions)]
                                    eprintln!(
                                        "Browser gateway connection ended: {:?}",
                                        error.kind()
                                    );
                                }
                                gateway.sockets.lock().unwrap().remove(&id);
                                gateway.sockets.lock().unwrap().remove(&(id | 1 << 63));
                            });
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(25))
                        }
                        Err(_) => break,
                    }
                }
            })?;
        Ok(this)
    }
    pub fn change(&self, route: Route, fragment: bool) {
        let mut policy = self.policy.lock().unwrap();
        policy.generation += 1;
        policy.route = route;
        policy.fragment = fragment;
        for stream in self.sockets.lock().unwrap().values() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
    pub fn is_stopped(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        self.change(Route::Blocked, false);
    }
    fn serve(&self, mut client: TcpStream, id: u64) -> io::Result<()> {
        client.set_read_timeout(Some(Duration::from_secs(20)))?;
        client.set_write_timeout(Some(Duration::from_secs(20)))?;
        let (route, fragment, generation) = {
            let p = self.policy.lock().unwrap();
            (p.route, p.fragment, p.generation)
        };
        let mut bytes = Vec::new();
        let mut one = [0];
        while !bytes.ends_with(b"\r\n\r\n") {
            client.read_exact(&mut one)?;
            bytes.push(one[0]);
            if bytes.len() > 32768 {
                return Err(invalid());
            }
        }
        let request = parse_request(&bytes)?;
        let connected = match route {
            Route::Blocked => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "network blocked",
            )),
            Route::Direct => connect_public(&request.host, request.port),
            Route::Vpn(port) => socks_connect(port, &request.host, request.port),
        };
        let mut upstream = match connected {
            Ok(s) => s,
            Err(e) => {
                let _ = client.write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                );
                return Err(e);
            }
        };
        upstream.set_nodelay(true)?;
        upstream.set_read_timeout(Some(Duration::from_secs(90)))?;
        upstream.set_write_timeout(Some(Duration::from_secs(30)))?;
        {
            let p = self.policy.lock().unwrap();
            if generation != p.generation {
                return Err(invalid());
            }
            self.sockets
                .lock()
                .unwrap()
                .insert(id | 1 << 63, upstream.try_clone()?);
        }
        if request.connect {
            client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
            if fragment {
                let mut head = [0; 5];
                client.read_exact(&mut head)?;
                let len = u16::from_be_bytes([head[3], head[4]]) as usize;
                if head[0] == 22 && head[1] == 3 && len <= 18432 {
                    let mut record = head.to_vec();
                    record.resize(5 + len, 0);
                    client.read_exact(&mut record[5..])?;
                    if let Some(split) = sni_split(&record) {
                        upstream.write_all(&record[..split])?;
                        thread::sleep(Duration::from_millis(8));
                        upstream.write_all(&record[split..])?;
                    } else {
                        upstream.write_all(&record)?;
                    }
                } else {
                    upstream.write_all(&head)?;
                }
            }
        } else {
            upstream.write_all(&request.forward)?;
        }
        client.set_read_timeout(Some(Duration::from_secs(90)))?;
        let mut input = client.try_clone()?;
        let mut output = upstream.try_clone()?;
        let forward = thread::spawn(move || {
            let _ = io::copy(&mut input, &mut output);
            let _ = output.shutdown(Shutdown::Write);
        });
        let _ = io::copy(&mut upstream, &mut client);
        let _ = client.shutdown(Shutdown::Both);
        let _ = upstream.shutdown(Shutdown::Both);
        let _ = forward.join();
        Ok(())
    }
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid browser proxy request")
}
struct Request {
    host: String,
    port: u16,
    connect: bool,
    forward: Vec<u8>,
}
fn parse_request(bytes: &[u8]) -> io::Result<Request> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let mut lines = text.split("\r\n");
    let line = lines.next().ok_or_else(invalid)?;
    let parts: Vec<_> = line.split_whitespace().collect();
    if parts.len() != 3 || !parts[2].starts_with("HTTP/1.") {
        return Err(invalid());
    }
    let connect = parts[0] == "CONNECT";
    let url = tauri::Url::parse(&if connect {
        format!("https://{}", parts[1])
    } else {
        parts[1].to_owned()
    })
    .map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || !connect && url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid());
    }
    let host = url
        .host_str()
        .ok_or_else(invalid)?
        .trim_matches(['[', ']'])
        .to_string();
    if !public_host(&host) {
        return Err(invalid());
    }
    let port = url.port_or_known_default().ok_or_else(invalid)?;
    let mut forward = format!(
        "{} {}{} HTTP/1.1\r\n",
        parts[0],
        url.path(),
        url.query().map(|q| format!("?{q}")).unwrap_or_default()
    );
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (key, _) = line.split_once(':').ok_or_else(invalid)?;
        if !matches!(
            key.to_ascii_lowercase().as_str(),
            "proxy-connection" | "proxy-authorization" | "connection"
        ) {
            forward.push_str(line);
            forward.push_str("\r\n");
        }
    }
    forward.push_str("Connection: close\r\n\r\n");
    Ok(Request {
        host,
        port,
        connect,
        forward: forward.into_bytes(),
    })
}
pub fn public_host(host: &str) -> bool {
    let host = host
        .trim_matches(['[', ']'])
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || !host.contains('.') && !host.contains(':')
    {
        return false;
    }
    host.parse::<IpAddr>().map(public_ip).unwrap_or(true)
}
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let o = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && o[0] != 0
                && o[0] < 224
                && !(o[0] == 100 && (64..=127).contains(&o[1]))
                && o != [255; 4]
        }
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(|v| public_ip(IpAddr::V4(v)))
            .unwrap_or(
                !ip.is_loopback()
                    && !ip.is_unspecified()
                    && !ip.is_multicast()
                    && (ip.segments()[0] & 0xfe00) != 0xfc00
                    && (ip.segments()[0] & 0xffc0) != 0xfe80,
            ),
    }
}
fn connect_public(host: &str, port: u16) -> io::Result<TcpStream> {
    let addresses: Vec<_> = (host, port).to_socket_addrs()?.collect();
    if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
        return Err(invalid());
    }
    for address in addresses.into_iter().take(4) {
        if let Ok(s) = TcpStream::connect_timeout(&address, Duration::from_secs(5)) {
            return Ok(s);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "browser connection failed",
    ))
}
pub fn socks_connect(port: u16, host: &str, target: u16) -> io::Result<TcpStream> {
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_secs(3),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(12)))?;
    stream.set_write_timeout(Some(Duration::from_secs(12)))?;
    stream.write_all(&[5, 1, 0])?;
    let mut response = [0; 2];
    stream.read_exact(&mut response)?;
    if response != [5, 0] {
        return Err(invalid());
    }
    let len = u8::try_from(host.len()).map_err(|_| invalid())?;
    let mut request = vec![5, 1, 0, 3, len];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&target.to_be_bytes());
    stream.write_all(&request)?;
    let mut reply = [0; 4];
    stream.read_exact(&mut reply)?;
    if reply[0] != 5 || reply[1] != 0 {
        return Err(invalid());
    }
    let count = match reply[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut n = [0];
            stream.read_exact(&mut n)?;
            n[0] as usize
        }
        _ => return Err(invalid()),
    };
    let mut discard = vec![0; count + 2];
    stream.read_exact(&mut discard)?;
    Ok(stream)
}
/// Split inside the SNI hostname, without modifying any TLS byte or certificate.
pub fn sni_split(record: &[u8]) -> Option<usize> {
    fn word(b: &[u8], i: usize) -> Option<usize> {
        Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as usize)
    }
    if record.first() != Some(&22) || record.get(5) != Some(&1) {
        return None;
    }
    let end = 5 + word(record, 3)?;
    if end > record.len() {
        return None;
    }
    let mut i = 43;
    i += 1 + *record.get(i)? as usize;
    i += 2 + word(record, i)?;
    i += 1 + *record.get(i)? as usize;
    let extensions_end = i + 2 + word(record, i)?;
    i += 2;
    if extensions_end > end {
        return None;
    }
    while i + 4 <= extensions_end {
        let kind = word(record, i)?;
        let size = word(record, i + 2)?;
        let start = i + 4;
        let next = start.checked_add(size)?;
        if next > extensions_end {
            return None;
        }
        if kind == 0 {
            if size < 6 || record.get(start + 2) != Some(&0) {
                return None;
            }
            let n = word(record, start + 3)?;
            if n < 2 || start + 5 + n > next {
                return None;
            }
            return Some(start + 5 + n / 2);
        }
        i = next;
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "explicit public HTTPS connectivity check"]
    fn live_https_survives_sni_fragmentation_and_blocks_when_disabled() {
        let gateway = Gateway::start().unwrap();
        for fragment in [false, true] {
            gateway.change(Route::Direct, fragment);
            let client = reqwest::blocking::Client::builder()
                .no_proxy()
                .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", gateway.port)).unwrap())
                .timeout(Duration::from_secs(25))
                .build()
                .unwrap();
            let response = client
                .get("https://example.com/")
                .send()
                .unwrap_or_else(|error| panic!("fragment={fragment}: {error:?}"));
            assert!(response.status().is_success());
            assert!(response.text().unwrap().contains("Example Domain"));
            gateway.change(Route::Blocked, fragment);
            assert!(client.get("https://example.com/").send().is_err());
        }
        gateway.stop();
    }
    #[test]
    fn request_rejects_local_and_privileged_destinations() {
        for uri in [
            "http://127.0.0.1/a",
            "http://tauri.localhost/",
            "file:///C:/secret",
            "http://10.0.0.1/",
            "http://[::1]/",
            "http://user:pass@example.com/",
        ] {
            assert!(
                parse_request(format!("GET {uri} HTTP/1.1\r\nHost: ignored\r\n\r\n").as_bytes())
                    .is_err(),
                "{uri}"
            );
        }
    }
    #[test]
    fn rewrite_removes_proxy_credentials_preserves_path() {
        let r=parse_request(b"GET http://example.com/a?q=b HTTP/1.1\r\nHost: example.com\r\nProxy-Authorization: secret\r\n\r\n").unwrap();
        let s = String::from_utf8(r.forward).unwrap();
        assert!(s.starts_with("GET /a?q=b HTTP/1.1"));
        assert!(!s.contains("secret"));
    }
    #[test]
    fn blocked_vpn_has_no_direct_fallback() {
        let g = Gateway::start().unwrap();
        for route in [Route::Blocked, Route::Vpn(1)] {
            g.change(route, false);
            let mut s = TcpStream::connect(("127.0.0.1", g.port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            s.write_all(b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com\r\n\r\n")
                .unwrap();
            let mut b = [0; 64];
            let n = s.read(&mut b).unwrap();
            assert!(std::str::from_utf8(&b[..n])
                .unwrap()
                .starts_with("HTTP/1.1 502"));
        }
        g.stop();
    }
    #[test]
    fn mode_change_terminates_existing_socket() {
        let g = Gateway::start().unwrap();
        let mut s = TcpStream::connect(("127.0.0.1", g.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        thread::sleep(Duration::from_millis(75));
        g.change(Route::Blocked, false);
        assert!(matches!(s.read(&mut [0; 1]), Ok(0) | Err(_)));
        g.stop();
    }
    #[test]
    fn sni_parser_handles_complete_and_malformed_client_hello() {
        let host = b"example.com";
        let mut b = vec![22, 3, 1, 0, 0, 1, 0, 0, 0, 3, 3];
        b.extend_from_slice(&[0; 32]);
        b.extend_from_slice(&[0, 0, 2, 0x13, 1, 1, 0]);
        let ext_len = 9 + host.len();
        b.extend_from_slice(&(ext_len as u16).to_be_bytes());
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(&((5 + host.len()) as u16).to_be_bytes());
        b.extend_from_slice(&((3 + host.len()) as u16).to_be_bytes());
        b.push(0);
        b.extend_from_slice(&(host.len() as u16).to_be_bytes());
        let start = b.len();
        b.extend_from_slice(host);
        let length = (b.len() - 5) as u16;
        b[3..5].copy_from_slice(&length.to_be_bytes());
        assert_eq!(sni_split(&b), Some(start + 5));
        for len in 0..b.len() {
            assert_eq!(sni_split(&b[..len]), None);
        }
    }
}
