//! Native HLS delivery. No account, viewer, page IPC, filesystem or process API.
//! Call only from an owned background worker. A complete HTTP acknowledgement
//! is required for each object; mux/process completion is not delivery success.
use super::browser::encoded::fmp4::relay::ts::{TsEnd, TsSegment};
use crate::cloud_recording::live_fingerprint::AckSink;
use reqwest::{
    blocking::{Body, Client},
    Url,
};
use std::{
    collections::VecDeque,
    io::{Cursor, Read},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

const MAX_SEGMENT: usize = 16 * 1024 * 1024;
const MAX_RESPONSE: u64 = 8192;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const ATTEMPTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeliveryFault {
    Target,
    Bounds,
    Sequence,
    Empty,
    Closed,
    Cancelled,
    Network,
    Response,
    Rejected(u16),
    UnfinishedInput,
}
type Result<T> = std::result::Result<T, DeliveryFault>;

/// Secret-bearing template obtained by the native Live API owner. Deliberately
/// has no Debug/Serialize/Display implementation and never reaches argv or IPC.
pub(crate) struct HlsTarget(Url);
impl HlsTarget {
    #[cfg(test)]
    pub(crate) fn loopback_test(address: std::net::SocketAddr) -> Result<Self> {
        if address.ip() != std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
            || address.port() == 0
        {
            return Err(DeliveryFault::Target);
        }
        Ok(Self(Url::parse(&format!(
            "http://{address}/http_upload_hls?cid=fixture&copy=0&file="
        )).map_err(|_| DeliveryFault::Target)?))
    }

    pub(crate) fn from_api(value: &str) -> Result<Self> {
        if value.len() > 2048 {
            return Err(DeliveryFault::Target);
        }
        let url = Url::parse(value).map_err(|_| DeliveryFault::Target)?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some_and(|p| p != 443)
            || url.fragment().is_some()
            || !matches!(
                url.host_str(),
                Some("a.upload.youtube.com" | "b.upload.youtube.com")
            )
            || url.path() != "/http_upload_hls"
        {
            return Err(DeliveryFault::Target);
        }
        let pairs: Vec<_> = url.query_pairs().collect();
        if pairs.len() != 3 {
            return Err(DeliveryFault::Target);
        }
        let exactly = |key: &str| pairs.iter().filter(|(k, _)| k == key).count() == 1;
        if !exactly("cid") || !exactly("copy") || !exactly("file") {
            return Err(DeliveryFault::Target);
        }
        for (key, value) in &pairs {
            let valid = match key.as_ref() {
                "cid" => {
                    !value.is_empty()
                        && value.len() <= 1024
                        && value
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                }
                "copy" => {
                    (url.host_str() == Some("a.upload.youtube.com") && value == "0")
                        || (url.host_str() == Some("b.upload.youtube.com") && value == "1")
                }
                "file" => value.is_empty(),
                _ => false,
            };
            if !valid {
                return Err(DeliveryFault::Target);
            }
        }
        Ok(Self(url))
    }
    fn object(&self, name: &str) -> Url {
        let mut url = self.0.clone();
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(key, _)| key != "file")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        url.set_query(None);
        url.query_pairs_mut()
            .extend_pairs(pairs)
            .append_pair("file", name);
        url
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DeliveryProgress {
    pub segment_acks: u64,
    pub playlist_acks: u64,
    pub requests: u64,
    pub acknowledged_bytes: u64,
    /// Only true after a sealed mux end and a complete final playlist response.
    /// This is NOT proof of a playable/processed YouTube archive.
    pub end_list_acked: bool,
}
struct Entry {
    ordinal: u64,
    ticks: u64,
    name: String,
}

pub(crate) struct HlsDelivery {
    target: HlsTarget,
    client: Client,
    cancel: Arc<AtomicBool>,
    run: Uuid,
    stream: Option<Uuid>,
    recent: VecDeque<Entry>,
    progress: DeliveryProgress,
    fault: Option<DeliveryFault>,
    frame_sampler: Option<AckSink>,
}
impl HlsDelivery {
    pub(crate) fn new(target: HlsTarget, cancel: Arc<AtomicBool>) -> Result<Self> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(REQUEST_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("Atsumi / Desktop / ", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| DeliveryFault::Network)?;
        Ok(Self {
            target,
            client,
            cancel,
            run: Uuid::new_v4(),
            stream: None,
            recent: VecDeque::new(),
            progress: DeliveryProgress::default(),
            fault: None,
            frame_sampler: None,
        })
    }
    /// Optional native-owned observer; the caller joins its worker before
    /// sealing the sidecar. Delivery never waits for this bounded queue.
    pub(crate) fn with_frame_sampler(mut self, sampler: AckSink) -> Self {
        self.frame_sampler = Some(sampler);
        self
    }
    pub(crate) fn progress(&self) -> DeliveryProgress {
        self.progress
    }
    pub(crate) fn deliver(&mut self, segment: &TsSegment) -> Result<DeliveryProgress> {
        let progress = self.deliver_ticks(
            segment.stream_id(),
            segment.ordinal(),
            segment.duration_ticks(),
            segment.bytes_arc(),
        )?;
        // Only sealed native segments reach this path, and only after both
        // complete responses succeeded. No decode/hash/file work occurs here.
        if let Some(sampler) = &self.frame_sampler {
            sampler.observe(segment);
        }
        Ok(progress)
    }
    fn check_open(&self) -> Result<()> {
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        if self.progress.end_list_acked {
            return Err(DeliveryFault::Closed);
        }
        if self.cancel.load(Ordering::Acquire) {
            return Err(DeliveryFault::Cancelled);
        }
        Ok(())
    }
    #[cfg(test)]
    fn deliver_parts(
        &mut self,
        stream: Uuid,
        ordinal: u64,
        milliseconds: u64,
        bytes: Arc<[u8]>,
    ) -> Result<DeliveryProgress> {
        self.deliver_ticks(
            stream,
            ordinal,
            milliseconds.checked_mul(90).ok_or(DeliveryFault::Bounds)?,
            bytes,
        )
    }
    fn deliver_ticks(
        &mut self,
        stream: Uuid,
        ordinal: u64,
        ticks: u64,
        bytes: Arc<[u8]>,
    ) -> Result<DeliveryProgress> {
        let result = self.deliver_inner(stream, ordinal, ticks, bytes);
        if let Err(fault) = result {
            self.fault.get_or_insert(fault);
        }
        result
    }
    fn deliver_inner(
        &mut self,
        stream: Uuid,
        ordinal: u64,
        ticks: u64,
        bytes: Arc<[u8]>,
    ) -> Result<DeliveryProgress> {
        self.check_open()?;
        if stream.is_nil()
            || self.stream.is_some_and(|id| id != stream)
            || ordinal != self.progress.segment_acks
            || ordinal == u64::MAX
        {
            return Err(DeliveryFault::Sequence);
        }
        if ticks == 0
            || ticks > 450_000
            || bytes.is_empty()
            || bytes.len() > MAX_SEGMENT
            || bytes.len() % 188 != 0
        {
            return Err(DeliveryFault::Bounds);
        }
        // Share the sealed immutable bytes through all retries. The caller's
        // bounded segment owner must remain accounted until delivery returns.
        let byte_count = bytes.len() as u64;
        self.stream = Some(stream);
        let name = format!("{}-{}.ts", self.run.simple(), ordinal);
        self.recent.push_back(Entry {
            ordinal,
            ticks,
            name: name.clone(),
        });
        let playlist = self.playlist(false)?;
        self.send("index.m3u8", playlist.into(), false)?;
        self.progress.playlist_acks += 1;
        self.send(&name, bytes, true)?;
        self.progress.segment_acks += 1;
        self.progress.acknowledged_bytes = self
            .progress
            .acknowledged_bytes
            .checked_add(byte_count)
            .ok_or(DeliveryFault::Bounds)?;
        // Keep two acknowledged entries, plus at most one pending next entry.
        while self.recent.len() > 2 {
            self.recent.pop_front();
        }
        Ok(self.progress)
    }
    pub(crate) fn finish(&mut self, end: &TsEnd) -> Result<DeliveryProgress> {
        self.finish_parts(end.stream_id(), end.segments())
    }
    fn finish_parts(&mut self, stream: Uuid, segments: u64) -> Result<DeliveryProgress> {
        if self.progress.end_list_acked
            && self.stream == Some(stream)
            && segments == self.progress.segment_acks
        {
            return Ok(self.progress);
        }
        let result = (|| {
            self.check_open()?;
            if self.progress.segment_acks == 0 {
                return Err(DeliveryFault::Empty);
            }
            if self.stream != Some(stream) || segments != self.progress.segment_acks {
                return Err(DeliveryFault::UnfinishedInput);
            }
            self.send("index.m3u8", self.playlist(true)?.into(), false)?;
            self.progress.playlist_acks += 1;
            self.progress.end_list_acked = true;
            Ok(self.progress)
        })();
        if let Err(fault) = result {
            self.fault.get_or_insert(fault);
        }
        result
    }
    fn playlist(&self, final_list: bool) -> Result<Vec<u8>> {
        let first = self.recent.front().ok_or(DeliveryFault::Empty)?;
        if self.recent.len() > 3 {
            return Err(DeliveryFault::Bounds);
        }
        let mut text = format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:5\n#EXT-X-MEDIA-SEQUENCE:{}\n",
            first.ordinal
        );
        for entry in &self.recent {
            text.push_str(&format!(
                "#EXTINF:{}.{:09},\n{}\n",
                entry.ticks / 90_000,
                ((entry.ticks % 90_000) * 1_000_000_000 + 45_000) / 90_000,
                entry.name
            ));
        }
        if final_list {
            text.push_str("#EXT-X-ENDLIST\n");
        }
        Ok(text.into_bytes())
    }
    fn send(&mut self, name: &str, body: Arc<[u8]>, segment: bool) -> Result<()> {
        let url = self.target.object(name);
        for attempt in 0..ATTEMPTS {
            self.check_open()?;
            self.progress.requests = self
                .progress
                .requests
                .checked_add(1)
                .ok_or(DeliveryFault::Bounds)?;
            let result = self
                .client
                .put(url.clone())
                // Blocking Client timeout alone is renewed by individual body
                // reads. Request timeout also bounds the async response body,
                // including a peer that keeps trickling bytes before each read.
                .timeout(REQUEST_TIMEOUT)
                .header(
                    reqwest::header::CONTENT_TYPE,
                    if segment {
                        "video/mp2t"
                    } else {
                        "application/vnd.apple.mpegurl"
                    },
                )
                .body(Body::sized(Cursor::new(body.clone()), body.len() as u64))
                .send();
            let outcome = match result {
                Ok(response) => {
                    let status = response.status().as_u16();
                    // A successful header with a truncated/oversized body is
                    // not a complete response. Drain only a bounded payload.
                    let mut drained = Vec::new();
                    let complete = response.take(MAX_RESPONSE + 1).read_to_end(&mut drained);
                    if complete.is_err() || drained.len() as u64 > MAX_RESPONSE {
                        Err(DeliveryFault::Response)
                    } else if status == 200 || (segment && status == 202) {
                        Ok(())
                    } else {
                        Err(DeliveryFault::Rejected(status))
                    }
                }
                Err(_) => Err(DeliveryFault::Network),
            };
            if self.cancel.load(Ordering::Acquire) {
                return Err(DeliveryFault::Cancelled);
            }
            match outcome {
                Ok(()) => return Ok(()),
                Err(fault) => {
                    let retry = matches!(
                        fault,
                        DeliveryFault::Network
                            | DeliveryFault::Response
                            | DeliveryFault::Rejected(500..=599)
                    );
                    if !retry || attempt + 1 == ATTEMPTS {
                        return Err(fault);
                    }
                    let until =
                        Instant::now() + Duration::from_millis(100 * 4u64.pow(attempt as u32));
                    while Instant::now() < until {
                        if self.cancel.load(Ordering::Acquire) {
                            return Err(DeliveryFault::Cancelled);
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        }
        Err(DeliveryFault::Network)
    }
}

#[cfg(test)]
#[path = "relay_hls_tests.rs"]
mod tests;
