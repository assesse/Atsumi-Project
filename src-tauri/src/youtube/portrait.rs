//! Optional, anonymous channel portrait bound to the resolved video's owner.
//! The existing resolving worker owns all work; only bounded OS DNS may outlive it.
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde_json::Value;
use std::{
    io::{Cursor, Read},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const HTML_LIMIT: usize = 4 * 1024 * 1024;
const IMAGE_LIMIT: usize = 128 * 1024;
const TOTAL_TIME: Duration = Duration::from_secs(5);
static DNS_ACTIVE: AtomicUsize = AtomicUsize::new(0);

struct Budget<'a> {
    deadline: Instant,
    stop: &'a AtomicBool,
}
impl Budget<'_> {
    fn remaining(&self) -> Option<Duration> {
        if self.stop.load(Ordering::Acquire) {
            return None;
        }
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|value| !value.is_zero())
    }
}

fn valid_video(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn valid_channel(id: &str) -> bool {
    id.len() == 24
        && id.starts_with("UC")
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn portrait_url(input: &str) -> Option<Url> {
    image_url(input,false)
}
fn image_url(input:&str,channel_metadata:bool)->Option<Url>{
    // Exact authority also rejects an explicit :443, which Url normalizes away.
    if input.len() > 1024
        || !(input.starts_with("https://yt3.ggpht.com/")||channel_metadata&&input.starts_with("https://yt3.googleusercontent.com/"))
        || input.contains('\\')
        || input
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
    {
        return None;
    }
    let url = Url::parse(input).ok()?;
    if !(url.host_str()==Some("yt3.ggpht.com")||channel_metadata&&url.host_str()==Some("yt3.googleusercontent.com"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() == "/"
    {
        return None;
    }
    Some(url)
}

fn owner_portrait(html: &[u8], video: &str, channel: &str) -> Option<Url> {
    if html.len() > HTML_LIMIT || !valid_video(video) || !valid_channel(channel) {
        return None;
    }
    let html = std::str::from_utf8(html).ok()?;
    let player = super::search::object_after(html, "var ytInitialPlayerResponse =")
        .or_else(|| super::search::object_after(html, "ytInitialPlayerResponse ="))?;
    let player: Value = serde_json::from_str(player).ok()?;
    if player["videoDetails"]["videoId"].as_str() != Some(video)
        || player["videoDetails"]["channelId"].as_str() != Some(channel)
    {
        return None;
    }
    let initial = super::search::object_after(html, "var ytInitialData =")
        .or_else(|| super::search::object_after(html, "window[\"ytInitialData\"] ="))?;
    let initial: Value = serde_json::from_str(initial).ok()?;
    let contents = initial["contents"]["twoColumnWatchNextResults"]["results"]["results"]
        ["contents"]
        .as_array()?;
    // Only the requested watch page's exact owner slot. Never recursively search
    // recommended videos, comments or collaboration cards for a matching image.
    let mut owners = contents.iter().filter_map(|item| {
        item.get("videoSecondaryInfoRenderer")?
            .get("owner")?
            .get("videoOwnerRenderer")
    });
    let owner = owners.next()?;
    if owners.next().is_some()
        || owner["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str() != Some(channel)
    {
        return None;
    }
    owner["thumbnail"]["thumbnails"]
        .as_array()?
        .iter()
        .filter_map(|entry| {
            let width = entry["width"].as_u64()?;
            let height = entry["height"].as_u64()?;
            if width == 0 || width != height || width > 4096 {
                return None;
            }
            Some((width.abs_diff(96), portrait_url(entry["url"].as_str()?)?))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, url)| url)
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || a == 0
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && matches!(b, 18 | 19)))
        }
        IpAddr::V6(ip) => {
            let p = ip.segments();
            (p[0] & 0xe000) == 0x2000
                && p[0] != 0x2002
                && !(p[0] == 0x2001 && p[1] < 0x0200)
                && !(p[0] == 0x2001 && p[1] == 0x0db8)
                && !(p[0] == 0x3fff && p[1] < 0x1000)
        }
    }
}
struct DnsPermit;
impl DnsPermit {
    fn acquire() -> Option<Self> {
        DNS_ACTIVE
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 4).then_some(n + 1)
            })
            .ok()?;
        Some(Self)
    }
}
impl Drop for DnsPermit {
    fn drop(&mut self) {
        DNS_ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}
fn addresses(host: &str, budget: &Budget<'_>) -> Option<Vec<SocketAddr>> {
    let timeout = budget.remaining()?.min(Duration::from_secs(2));
    let permit = DnsPermit::acquire()?;
    let host = host.to_owned();
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("youtube-portrait-dns".into())
        .spawn(move || {
            // An OS resolver cannot be cancelled. Its permit remains owned until
            // that resolver returns, even when the caller's deadline has expired.
            let _permit = permit;
            let result = (host.as_str(), 443)
                .to_socket_addrs()
                .map(|values| values.take(17).collect::<Vec<_>>());
            let _ = tx.send(result);
        })
        .ok()?;
    let resolved = rx.recv_timeout(timeout).ok()?.ok()?;
    budget.remaining()?;
    if resolved.is_empty() || resolved.len() > 16 || !resolved.iter().all(|a| public_ip(a.ip())) {
        return None;
    }
    Some(resolved)
}

fn get(url: Url, maximum: usize, budget: &Budget<'_>) -> Option<Vec<u8>> {
    let host = url.host_str()?;
    let resolved = addresses(host, budget)?;
    let timeout = budget.remaining()?;
    let client = Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(Policy::none())
        .referer(false)
        .resolve_to_addrs(host, &resolved)
        .connect_timeout(timeout.min(Duration::from_secs(2)))
        .timeout(timeout)
        .user_agent("Atsumi/2.2 YouTube public metadata")
        .build()
        .ok()?;
    let timeout = budget.remaining()?;
    let mut response = client.get(url).timeout(timeout).send().ok()?;
    budget.remaining()?;
    if !accepts_response(response.status(), response.content_length(), maximum) {
        return None;
    }
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        budget.remaining()?;
        let count = response.read(&mut chunk).ok()?;
        if count == 0 {
            break;
        }
        if bytes.len().checked_add(count)? > maximum {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    budget.remaining()?;
    Some(bytes)
}

fn accepts_response(status: reqwest::StatusCode, length: Option<u64>, maximum: usize) -> bool {
    status == reqwest::StatusCode::OK && length.is_none_or(|n| n <= maximum as u64)
}

fn normalize(bytes: &[u8]) -> Option<String> {
    normalize_size(bytes,96)
}
fn normalize_size(bytes:&[u8],size:u32)->Option<String>{
    if bytes.is_empty() || bytes.len() > IMAGE_LIMIT {
        return None;
    }
    crate::streaming::chat_assets::validate_raster(bytes, None).ok()?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    if !matches!(
        reader.format()?,
        image::ImageFormat::Jpeg
            | image::ImageFormat::Png
            | image::ImageFormat::WebP
            | image::ImageFormat::Gif
    ) {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(size,size);
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, image::ImageFormat::Png).ok()?;
    if output.get_ref().len() > IMAGE_LIMIT {
        return None;
    }
    Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(output.into_inner())
    ))
}

fn read_with(
    video: &str,
    channel: &str,
    budget: &Budget<'_>,
    mut fetch: impl FnMut(Url, usize, &Budget<'_>) -> Option<Vec<u8>>,
) -> Option<String> {
    if !valid_video(video) || !valid_channel(channel) {
        return None;
    }
    budget.remaining()?;
    let watch = Url::parse(&format!("https://www.youtube.com/watch?v={video}")).ok()?;
    let html = fetch(watch, HTML_LIMIT, budget)?;
    budget.remaining()?;
    let url = owner_portrait(&html, video, channel)?;
    budget.remaining()?;
    let bytes = fetch(url, IMAGE_LIMIT, budget)?;
    budget.remaining()?;
    let data = normalize(&bytes)?;
    budget.remaining()?;
    Some(data)
}

pub(crate) fn read(video: &str, channel: &str, stop: &AtomicBool) -> Option<String> {
    read_with(
        video,
        channel,
        &Budget {
            deadline: Instant::now() + TOTAL_TIME,
            stop,
        },
        get,
    )
}

fn video_owner(html: &[u8], video: &str) -> Option<String> {
    if html.len() > HTML_LIMIT || !valid_video(video) { return None; }
    let html = std::str::from_utf8(html).ok()?;
    let player = super::search::object_after(html, "var ytInitialPlayerResponse =")
        .or_else(|| super::search::object_after(html, "ytInitialPlayerResponse ="))?;
    let player: Value = serde_json::from_str(player).ok()?;
    let channel = player["videoDetails"]["channelId"].as_str()?;
    (player["videoDetails"]["videoId"].as_str() == Some(video) && valid_channel(channel))
        .then(|| channel.to_owned())
}

fn read_video_with(video: &str, budget: &Budget<'_>, mut fetch: impl FnMut(Url, usize, &Budget<'_>) -> Option<Vec<u8>>) -> Option<String> {
    if !valid_video(video) { return None; }
    budget.remaining()?;
    let html = fetch(Url::parse(&format!("https://www.youtube.com/watch?v={video}")).ok()?, HTML_LIMIT, budget)?;
    budget.remaining()?;
    let channel = video_owner(&html, video)?;
    // Same exact requested-video / player-channel / owner-slot proof as recordings.
    let image = owner_portrait(&html, video, &channel)?;
    let bytes = fetch(image, IMAGE_LIMIT, budget)?;
    budget.remaining()?;
    let image = normalize(&bytes)?;
    budget.remaining()?;
    Some(image)
}

/// Current public channel picture for a library card; never a saved-time proof.
pub(crate) fn read_video(video: &str, stop: &AtomicBool) -> Option<String> {
    read_video_with(video, &Budget { deadline: Instant::now() + TOTAL_TIME, stop }, get)
}

fn channel_identity(html:&[u8],target:&super::YouTubeTarget)->Option<(String,Option<Url>)>{
    if html.len()>HTML_LIMIT||target.kind!="channel"{return None;}
    let html=std::str::from_utf8(html).ok()?;
    let raw=super::search::object_after(html,"var ytInitialData =")
        .or_else(||super::search::object_after(html,"window[\"ytInitialData\"] ="))?;
    let data:Value=serde_json::from_str(raw).ok()?;
    let metadata=&data["metadata"]["channelMetadataRenderer"];
    let id=metadata["externalId"].as_str()?;
    if !valid_channel(id)||target.id.strip_prefix("channel/").is_some_and(|requested|requested!=id){return None;}
    let canonical=super::parse_target(metadata["channelUrl"].as_str()?).ok()?;
    if canonical.kind!="channel"||canonical.id!=format!("channel/{id}"){return None;}
    let title=metadata["title"].as_str()?.chars().filter(|ch|!ch.is_control()).take(160).collect::<String>();
    if title.trim().is_empty(){return None;}
    let image=metadata["avatar"]["thumbnails"].as_array().into_iter().flatten().take(16).filter_map(|entry|{
        let width=entry["width"].as_u64()?;let height=entry["height"].as_u64()?;
        if width==0||width!=height||width>4096{return None;}
        Some((width.abs_diff(192),image_url(entry["url"].as_str()?,true)?))
    }).min_by_key(|(distance,_)|*distance).map(|(_,url)|url);
    Some((title,image))
}

/// Lazy discovery identity only: an exact public channel document and its avatar.
/// This never reads the account profile or fetches any video/recommended portrait.
pub(super) fn read_channel(input:&str)->Option<(String,Option<String>)>{
    let target=super::parse_target(input).ok()?;
    if target.kind!="channel"{return None;}
    let stop=AtomicBool::new(false);let budget=Budget{deadline:Instant::now()+TOTAL_TIME,stop:&stop};
    let html=get(Url::parse(&target.url).ok()?,HTML_LIMIT,&budget)?;
    let (title,image)=channel_identity(&html,&target)?;
    let image=image.and_then(|url|get(url,IMAGE_LIMIT,&budget)).and_then(|bytes|normalize_size(&bytes,192));
    Some((title,image))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const VIDEO: &str = "jNQXAC9IVRw";
    const CHANNEL: &str = "UC4QobU6STFB0P71PMvOGN5A";
    #[test]
    fn current_video_portrait_requires_same_requested_video_and_unique_owner() {
        let valid = html(VIDEO, CHANNEL, CHANNEL);
        assert_eq!(video_owner(&valid, VIDEO).as_deref(), Some(CHANNEL));
        let stop = AtomicBool::new(false);
        let budget = Budget { deadline: Instant::now() + TOTAL_TIME, stop: &stop };
        for data in [html("abcdefghijk", CHANNEL, CHANNEL), html(VIDEO, CHANNEL, "UCaaaaaaaaaaaaaaaaaaaaaa")] {
            let mut requests = 0;
            assert!(read_video_with(VIDEO, &budget, |_,_,_| { requests += 1; Some(data.clone()) }).is_none());
            assert_eq!(requests, 1, "identity failure must not fetch an image");
        }
        assert!(video_owner(&valid, "invalid").is_none());
    }
    #[test]
    fn discovery_portrait_uses_exact_channel_metadata_not_recommendations(){
        let target=super::super::parse_target(&format!("https://www.youtube.com/channel/{CHANNEL}")).unwrap();
        let make=|id:&str,canonical:&str,url:&str|format!("var ytInitialData = {};",json!({"metadata":{"channelMetadataRenderer":{"externalId":id,"channelUrl":canonical,"title":"Public channel","avatar":{"thumbnails":[{"width":900,"height":900,"url":url}]}}}})).into_bytes();
        let canonical=format!("https://www.youtube.com/channel/{CHANNEL}");
        let valid=make(CHANNEL,&canonical,"https://yt3.googleusercontent.com/public=s900");
        let (title,image)=channel_identity(&valid,&target).unwrap();assert_eq!(title,"Public channel");assert!(image.is_some());
        assert!(channel_identity(&make("UCrNicqyCSAUuxSzUi6VChvw",&canonical,"https://yt3.googleusercontent.com/public=s900"),&target).is_none());
        assert!(channel_identity(&make(CHANNEL,"https://evil.invalid/channel/x","https://yt3.googleusercontent.com/public=s900"),&target).is_none());
        for url in ["https://yt3.googleusercontent.com.evil.invalid/x","https://yt3.googleusercontent.com:443/x","https://user@yt3.googleusercontent.com/x","http://yt3.googleusercontent.com/x","https://yt3.googleusercontent.com/x?token=private"]{
            assert!(channel_identity(&make(CHANNEL,&canonical,url),&target).unwrap().1.is_none());
        }
    }
    #[test]
    #[ignore="Explicit anonymous channel metadata/avatar; no account or media download"]
    fn public_channel_discovery_portrait(){
        let (title,image)=read_channel("https://www.youtube.com/@NASA").unwrap();assert_eq!(title,"NASA");
        let image=image.expect("actual channel avatar");let bytes=STANDARD.decode(image.strip_prefix("data:image/png;base64,").unwrap()).unwrap();
        let raster=image::load_from_memory(&bytes).unwrap();assert_eq!(raster.width(),192);assert_eq!(raster.height(),192);
        eprintln!("public channel portrait:192x192,{}bytes; video0;account0",bytes.len());
    }
    #[test]
    #[ignore="Explicit anonymous exact Lofi channel/avatar; no account, video, app or writes"]
    fn public_lofi_channel_discovery_portrait() {
        let started = Instant::now();
        let (title, image) = read_channel("https://www.youtube.com/channel/UCSJ4gkVC6NrvII8umztf0Ow")
            .expect("fixed public channel metadata was unavailable");
        assert_eq!(title, "Lofi Girl");
        let image = image.expect("fixed public channel avatar was unavailable");
        let bytes = STANDARD.decode(image.strip_prefix("data:image/png;base64,").expect("normalized raster prefix")).unwrap();
        let raster = image::load_from_memory(&bytes).expect("normalized raster");
        assert_eq!((raster.width(), raster.height()), (192, 192));
        assert!(bytes.len() <= IMAGE_LIMIT);
        eprintln!("fixed public channel portrait:192x192,{}bytes,{}ms; video0;account0;writes0", bytes.len(), started.elapsed().as_millis());
    }
    fn html(video: &str, player_channel: &str, owner_channel: &str) -> Vec<u8> {
        let player = json!({"videoDetails":{"videoId":video,"channelId":player_channel}});
        let owner = json!({"videoSecondaryInfoRenderer":{"owner":{"videoOwnerRenderer":{
            "navigationEndpoint":{"browseEndpoint":{"browseId":owner_channel}},
            "thumbnail":{"thumbnails":[{"url":"https://yt3.ggpht.com/owner=s88","width":88,"height":88}]}
        }}}});
        format!("var ytInitialPlayerResponse = {player}; var ytInitialData = {};", json!({"contents":{"twoColumnWatchNextResults":{"results":{"results":{"contents":[owner]}}}}})).into_bytes()
    }
    fn image() -> Vec<u8> {
        let mut data = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(180, 180)
            .write_to(&mut data, image::ImageFormat::Png)
            .unwrap();
        data.into_inner()
    }
    #[test]
    fn exact_video_and_resolved_channel_and_watch_owner_must_agree() {
        assert!(owner_portrait(&html(VIDEO, CHANNEL, CHANNEL), VIDEO, CHANNEL).is_some());
        for bytes in [
            html("M7lc1UVf-VE", CHANNEL, CHANNEL),
            html(VIDEO, "UCrNicqyCSAUuxSzUi6VChvw", CHANNEL),
            html(VIDEO, CHANNEL, "UCrNicqyCSAUuxSzUi6VChvw"),
            b"var ytInitialPlayerResponse = {}; var ytInitialData = {};".to_vec(),
            b"var ytInitialPlayerResponse = {broken".to_vec(),
            vec![b'x'; HTML_LIMIT + 1],
        ] {
            assert!(owner_portrait(&bytes, VIDEO, CHANNEL).is_none());
        }
        assert!(owner_portrait(&html(VIDEO, CHANNEL, CHANNEL), VIDEO, "@same_name").is_none());
        let unrelated = String::from_utf8(html(VIDEO, CHANNEL, CHANNEL))
            .unwrap()
            .replace("videoSecondaryInfoRenderer", "recommendedVideoRenderer");
        assert!(owner_portrait(unrelated.as_bytes(), VIDEO, CHANNEL).is_none());
    }
    #[test]
    fn only_exact_public_image_authority_is_accepted() {
        assert!(portrait_url("https://yt3.ggpht.com/owner=s88-c-k-c0x00ffffff-no-rj").is_some());
        for url in [
            "http://yt3.ggpht.com/a",
            "https://yt3.ggpht.com:443/a",
            "https://yt3.ggpht.com:444/a",
            "https://yt3.ggpht.com.evil.invalid/a",
            "https://a@yt3.ggpht.com/a",
            "https://yt3.ggpht.com/",
            "https://i.ytimg.com/vi/a.jpg",
            "https://yt3.googleusercontent.com/a",
            "https://yt3.ggpht.com/a?q=1",
            "https://yt3.ggpht.com/a#x",
            "https://yt3.ggpht.com/\\a",
            "https://127.0.0.1/a",
        ] {
            assert!(portrait_url(url).is_none(), "unexpected allowed source");
        }
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "224.0.0.1",
            "::1",
            "::ffff:8.8.8.8",
            "fc00::1",
            "2001:db8::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()));
        }
        assert!(public_ip("8.8.8.8".parse().unwrap()));
    }
    #[test]
    fn raster_is_bounded_normalized_and_never_html() {
        let input = image();
        let output = normalize(&input).unwrap();
        let bytes = STANDARD
            .decode(output.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 96);
        assert_eq!(image::load_from_memory(&bytes).unwrap().height(), 96);
        assert!(bytes.len() <= IMAGE_LIMIT);
        assert!(normalize(b"<svg></svg>").is_none());
        assert!(normalize(&vec![0; IMAGE_LIMIT + 1]).is_none());
        assert!(normalize(&input[..input.len() / 2]).is_none());
    }
    #[test]
    fn only_two_requests_and_no_subsequent_phase_after_stop_or_expiry() {
        let stop = AtomicBool::new(false);
        let budget = Budget {
            deadline: Instant::now() + TOTAL_TIME,
            stop: &stop,
        };
        let mut count = 0;
        let data = read_with(VIDEO, CHANNEL, &budget, |url, limit, _| {
            count += 1;
            if count == 1 {
                assert_eq!(url.host_str(), Some("www.youtube.com"));
                assert_eq!(limit, HTML_LIMIT);
                Some(html(VIDEO, CHANNEL, CHANNEL))
            } else {
                assert_eq!(count, 2);
                assert_eq!(url.host_str(), Some("yt3.ggpht.com"));
                assert_eq!(limit, IMAGE_LIMIT);
                Some(image())
            }
        });
        assert!(data.is_some());
        assert_eq!(count, 2);
        let mut count = 0;
        assert!(read_with(VIDEO, CHANNEL, &budget, |_, _, _| {
            count += 1;
            stop.store(true, Ordering::Release);
            Some(html(VIDEO, CHANNEL, CHANNEL))
        })
        .is_none());
        assert_eq!(count, 1);
        assert!(read_with(VIDEO, CHANNEL, &budget, |_, _, _| panic!(
            "cancelled request"
        ))
        .is_none());
        stop.store(false, Ordering::Release);
        let expired = Budget {
            deadline: Instant::now() - Duration::from_millis(1),
            stop: &stop,
        };
        assert!(read_with(VIDEO, CHANNEL, &expired, |_, _, _| panic!(
            "expired request"
        ))
        .is_none());
        let expiring = Budget {
            deadline: Instant::now() + Duration::from_millis(5),
            stop: &stop,
        };
        let mut count = 0;
        assert!(read_with(VIDEO, CHANNEL, &expiring, |_, _, _| {
            count += 1;
            std::thread::sleep(Duration::from_millis(10));
            Some(html(VIDEO, CHANNEL, CHANNEL))
        })
        .is_none());
        assert_eq!(count, 1);
        let mut count = 0;
        assert!(read_with(VIDEO, CHANNEL, &budget, |_, _, _| {
            count += 1;
            None
        })
        .is_none());
        assert_eq!(count, 1);
    }
    #[test]
    fn redirects_partial_errors_and_large_declared_bodies_are_rejected() {
        assert!(accepts_response(reqwest::StatusCode::OK, None, IMAGE_LIMIT));
        assert!(accepts_response(
            reqwest::StatusCode::OK,
            Some(IMAGE_LIMIT as u64),
            IMAGE_LIMIT
        ));
        for code in [206, 301, 302, 303, 307, 308, 401, 403, 404, 500] {
            assert!(!accepts_response(
                reqwest::StatusCode::from_u16(code).unwrap(),
                None,
                IMAGE_LIMIT
            ));
        }
        assert!(!accepts_response(
            reqwest::StatusCode::OK,
            Some(IMAGE_LIMIT as u64 + 1),
            IMAGE_LIMIT
        ));
    }
    #[test]
    fn dns_helpers_keep_their_own_bounded_lifetime() {
        let permits: Vec<_> = (0..4).map(|_| DnsPermit::acquire().unwrap()).collect();
        assert!(DnsPermit::acquire().is_none());
        drop(permits);
        assert!(DnsPermit::acquire().is_some());
    }
    #[test]
    #[ignore = "Explicit anonymous bounded owner portraits only; no video, account, writes or app"]
    fn public_vod_and_live_owner_portraits() {
        for (video, channel) in [
            (VIDEO, CHANNEL),
            ("cswFQYam8Ic", "UCrNicqyCSAUuxSzUi6VChvw"),
        ] {
            let started = Instant::now();
            let stop = AtomicBool::new(false);
            let budget = Budget {
                deadline: started + TOTAL_TIME,
                stop: &stop,
            };
            let mut requests = 0;
            let data = read_with(video, channel, &budget, |url, maximum, budget| {
                requests += 1;
                get(url, maximum, budget)
            })
            .expect("verified public owner portrait");
            let bytes = STANDARD
                .decode(data.strip_prefix("data:image/png;base64,").unwrap())
                .unwrap();
            let image = image::load_from_memory(&bytes).unwrap();
            assert!(image.width() <= 96 && image.height() <= 96);
            assert!(bytes.len() <= IMAGE_LIMIT);
            assert_eq!(requests, 2);
            eprintln!("public portrait: {}x{}, {}bytes, {}ms; requests={requests}; video=0; account=0; writes=0", image.width(), image.height(), bytes.len(), started.elapsed().as_millis());
        }
    }
}
