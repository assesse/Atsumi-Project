//! App-local CONNECT transport: DoH + first ClientHello record fragmentation.
//! No driver, OS proxy/DNS change, root CA, TLS termination, or traffic logging.
use base64::Engine;
use reqwest::blocking::{Client, ClientBuilder};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

const HOSTS: [&str; 2] = ["danbooru.donmai.us", "cdn.donmai.us"];
const LIMIT: usize = 16;
struct Gateway {
    address: SocketAddr,
    secret: String,
}
static GATEWAY: OnceLock<Result<Gateway, String>> = OnceLock::new();
type DnsCache = Arc<Mutex<HashMap<String, (Instant, Vec<Ipv4Addr>)>>>;
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "unsupported Danbooru tunnel",
    )
}
fn allowed(host: &str) -> bool {
    HOSTS.contains(&host)
}
fn public(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    !address.is_private()
        && !address.is_loopback()
        && !address.is_link_local()
        && !address.is_multicast()
        && !address.is_unspecified()
        && !address.is_broadcast()
        && address.octets()[0] != 0
        && address.octets()[0] < 224
        && !(a == 100 && (64..=127).contains(&b))
        && !(a == 192 && b == 0 && (c == 0 || c == 2))
        && !(a == 198 && (b == 18 || b == 19 || b == 51 && c == 100))
        && !(a == 203 && b == 0 && c == 113)
}

pub(super) fn configure(builder: ClientBuilder) -> Result<ClientBuilder, String> {
    if std::env::var("ATSUMI_DANBOORU_NETWORK").as_deref() == Ok("direct") {
        return Ok(builder);
    }
    let gateway = GATEWAY.get_or_init(start).as_ref().map_err(Clone::clone)?;
    let address = format!("http://{}", gateway.address);
    let proxy = reqwest::Proxy::custom(move |url| {
        (url.scheme() == "https" && allowed(url.host_str().unwrap_or_default()))
            .then(|| address.clone())
    })
    .basic_auth("atsumi", &gateway.secret);
    Ok(builder.no_proxy().proxy(proxy).https_only(true).redirect(
        reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 5
                && attempt.url().scheme() == "https"
                && allowed(attempt.url().host_str().unwrap_or_default())
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }),
    ))
}
fn start() -> Result<Gateway, String> {
    let listener =
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|_| "local transport unavailable")?;
    let address = listener
        .local_addr()
        .map_err(|_| "local transport unavailable")?;
    let secret = uuid::Uuid::new_v4().simple().to_string();
    let authentication = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("atsumi:{secret}"))
    );
    let dns = Client::builder()
        .no_proxy()
        .https_only(true)
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(6))
        .resolve("cloudflare-dns.com", SocketAddr::from(([1, 1, 1, 1], 443)))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "secure DNS unavailable")?;
    let cache: DnsCache = Arc::new(Mutex::new(HashMap::new()));
    let active = Arc::new(AtomicUsize::new(0));
    std::thread::Builder::new()
        .name("danbooru-transport".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                if active.fetch_add(1, Ordering::AcqRel) >= LIMIT {
                    active.fetch_sub(1, Ordering::AcqRel);
                    continue;
                }
                let active = active.clone();
                let auth = authentication.clone();
                let dns = dns.clone();
                let cache = cache.clone();
                std::thread::spawn(move || {
                    let _ = tunnel(stream, &auth, &dns, &cache);
                    active.fetch_sub(1, Ordering::AcqRel);
                });
            }
        })
        .map_err(|_| "local transport unavailable")?;
    Ok(Gateway { address, secret })
}
fn destination(header: &str, authentication: &str) -> io::Result<String> {
    let mut lines = header.split("\r\n");
    let words = lines
        .next()
        .ok_or_else(invalid)?
        .split_whitespace()
        .collect::<Vec<_>>();
    if words.len() != 3 || words[0] != "CONNECT" || words[2] != "HTTP/1.1" {
        return Err(invalid());
    }
    let host = words[1]
        .strip_suffix(":443")
        .filter(|host| allowed(host))
        .ok_or_else(invalid)?;
    let auth = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("proxy-authorization"))
        .map(|(_, value)| value.trim())
        .collect::<Vec<_>>();
    if auth != [authentication] {
        return Err(invalid());
    }
    Ok(host.into())
}
fn resolve(host: &str, dns: &Client, cache: &DnsCache) -> io::Result<Vec<Ipv4Addr>> {
    if !allowed(host) {
        return Err(invalid());
    }
    if let Some((until, addresses)) = cache.lock().map_err(|_| invalid())?.get(host) {
        if *until > Instant::now() {
            return Ok(addresses.clone());
        }
    }
    let response = dns
        .get("https://cloudflare-dns.com/dns-query")
        .query(&[("name", host), ("type", "A")])
        .header("Accept", "application/dns-json")
        .send()
        .map_err(|_| invalid())?
        .error_for_status()
        .map_err(|_| invalid())?;
    let mut bytes = Vec::new();
    response.take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(invalid());
    }
    let data: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if data["Status"].as_u64() != Some(0) {
        return Err(invalid());
    }
    let addresses = data["Answer"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .filter(|row| row["type"].as_u64() == Some(1))
        .filter_map(|row| row["data"].as_str()?.parse::<Ipv4Addr>().ok())
        .filter(|ip| public(*ip))
        .take(8)
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(invalid());
    }
    cache.lock().map_err(|_| invalid())?.insert(
        host.into(),
        (Instant::now() + Duration::from_secs(60), addresses.clone()),
    );
    Ok(addresses)
}
// Reframe only the unencrypted first handshake record. Every byte of the
// handshake stays identical, including SNI. Certificate verification remains
// in reqwest/rustls end-to-end; application records are relayed untouched.
fn fragmented_hello(header: [u8; 5], payload: &[u8]) -> io::Result<Vec<u8>> {
    if header[0] != 22
        || header[1] != 3
        || !matches!(header[2], 1..=3)
        || payload.first() != Some(&1)
        || payload.len() != u16::from_be_bytes([header[3], header[4]]) as usize
        || payload.len() > 16384
    {
        return Err(invalid());
    }
    let mut output = Vec::with_capacity(payload.len() + payload.len() / 16 * 5 + 5);
    for part in payload.chunks(16) {
        output.extend_from_slice(&[22, header[1], header[2], 0, part.len() as u8]);
        output.extend_from_slice(part);
    }
    Ok(output)
}
fn tunnel(mut client: TcpStream, auth: &str, dns: &Client, cache: &DnsCache) -> io::Result<()> {
    client.set_read_timeout(Some(Duration::from_secs(8)))?;
    client.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 4096 {
            return Err(invalid());
        }
        let mut byte = [0];
        client.read_exact(&mut byte)?;
        header.push(byte[0]);
    }
    let host = destination(std::str::from_utf8(&header).map_err(|_| invalid())?, auth)?;
    let addresses = resolve(&host, dns, cache)?;
    let mut upstream = addresses
        .into_iter()
        .find_map(|ip| {
            TcpStream::connect_timeout(&SocketAddr::from((ip, 443)), Duration::from_secs(3)).ok()
        })
        .ok_or_else(invalid)?;
    upstream.set_nodelay(true)?;
    upstream.set_read_timeout(Some(Duration::from_secs(45)))?;
    upstream.set_write_timeout(Some(Duration::from_secs(30)))?;
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    let mut tls = [0; 5];
    client.read_exact(&mut tls)?;
    let length = u16::from_be_bytes([tls[3], tls[4]]) as usize;
    if !(4..=16384).contains(&length) {
        return Err(invalid());
    }
    let mut payload = vec![0; length];
    client.read_exact(&mut payload)?;
    upstream.write_all(&fragmented_hello(tls, &payload)?)?;
    client.set_read_timeout(Some(Duration::from_secs(45)))?;
    let mut outbound = upstream.try_clone()?;
    let mut input = client.try_clone()?;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let _ = io::copy(&mut input, &mut outbound);
            let _ = outbound.shutdown(Shutdown::Write);
        });
        let _ = io::copy(&mut upstream, &mut client);
        let _ = client.shutdown(Shutdown::Both);
        let _ = upstream.shutdown(Shutdown::Both);
    });
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_hosts_and_per_process_auth_only() {
        assert_eq!(
            destination(
                "CONNECT cdn.donmai.us:443 HTTP/1.1\r\nProxy-Authorization: Basic test\r\n\r\n",
                "Basic test"
            )
            .unwrap(),
            "cdn.donmai.us"
        );
        for host in [
            "127.0.0.1:443",
            "danbooru.donmai.us.evil:443",
            "danbooru.donmai.us:80",
            "user@cdn.donmai.us:443",
        ] {
            assert!(destination(
                &format!("CONNECT {host} HTTP/1.1\r\nProxy-Authorization: Basic test\r\n\r\n"),
                "Basic test"
            )
            .is_err());
        }
        assert!(destination("CONNECT cdn.donmai.us:443 HTTP/1.1\r\n\r\n", "Basic test").is_err());
        for address in [
            "127.0.0.1",
            "10.1.1.1",
            "192.168.0.1",
            "169.254.1.1",
            "0.0.0.0",
            "224.1.1.1",
        ] {
            assert!(!public(address.parse().unwrap()));
        }
    }
    #[test]
    fn fragmented_client_hello_is_byte_identical_after_reassembly() {
        let mut payload = vec![1; 43];
        payload.extend_from_slice(b"danbooru.donmai.us");
        payload.extend_from_slice(&[3; 34]);
        let length = (payload.len() as u16).to_be_bytes();
        let encoded = fragmented_hello([22, 3, 1, length[0], length[1]], &payload).unwrap();
        let mut decoded = Vec::new();
        let mut offset = 0;
        while offset < encoded.len() {
            let size = encoded[offset + 4] as usize;
            assert!(size <= 16);
            decoded.extend_from_slice(&encoded[offset + 5..offset + 5 + size]);
            offset += 5 + size;
        }
        assert_eq!(decoded, payload);
        assert!(!encoded
            .windows(17)
            .any(|part| part == b"danbooru.donmai.us"));
        assert!(fragmented_hello([23, 3, 3, 0, 3], &[1, 2, 3]).is_err());
    }
}
