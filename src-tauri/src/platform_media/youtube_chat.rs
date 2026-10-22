//! One pinned YouTube chat producer, streaming into the existing session sink.
//! No untrusted info-JSON passthrough, separate extractor, account or video request.
use super::{process, storage, MediaRef, PlatformMediaService};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

const IDENTITY_ERROR: &str = "YouTube 채팅의 방송 식별 정보를 확인하지 못했습니다.";
const SOURCE_ERROR: &str = "채팅 임시 조각을 확인하지 못했습니다. 받은 기록은 보존했습니다.";
const FRAGMENT_BUDGET: u64 = 32 * 1024 * 1024;
const SPACE_RESERVE: u64 = 1024 * 1024 * 1024;
const MAX_FILES: usize = 16;

#[derive(Clone)]
pub(super) struct Descriptor {
    video_id: String,
    channel_id: String,
}

fn valid_video(value: &str) -> bool {
    value.len() == 11
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}
fn valid_channel(value: &str) -> bool {
    value.len() == 24
        && value.starts_with("UC")
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}
fn youtube_url(raw: &str, path: &str) -> Option<reqwest::Url> {
    if raw.len() > 512 {
        return None;
    }
    let url = reqwest::Url::parse(raw).ok()?;
    (url.scheme() == "https"
        && url.host_str() == Some("www.youtube.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.path() == path)
        .then_some(url)
}
fn valid_watch(raw: &str, id: &str) -> bool {
    let Some(url) = youtube_url(raw, "/watch") else {
        return false;
    };
    let mut video = false;
    let mut counter = false;
    let mut verified = false;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "v" if !video && value == id => video = true,
            "bpctr" if !counter && value == "9999999999" => counter = true,
            "has_verified" if !verified && value == "1" => verified = true,
            _ => return false,
        }
    }
    video
}

impl Descriptor {
    /// `None` means this verified preflight did not advertise public live chat.
    /// A present but mismatched descriptor is an error, never an alternate target.
    pub(super) fn from_metadata(
        reference: &MediaRef,
        metadata: &Value,
    ) -> Result<Option<Self>, String> {
        if reference.provider != "youtube"
            || reference.kind != "live"
            || !valid_video(&reference.external_id)
            || metadata["id"].as_str() != Some(reference.external_id.as_str())
            || metadata["is_live"].as_bool() != Some(true)
        {
            return Err(IDENTITY_ERROR.into());
        }
        let Some(chats) = metadata.get("subtitles").and_then(|v| v.get("live_chat")) else {
            return Ok(None);
        };
        let chats = chats.as_array().ok_or(IDENTITY_ERROR)?;
        if chats.is_empty() {
            return Ok(None);
        }
        if chats.len() != 1 {
            return Err(IDENTITY_ERROR.into());
        }
        let channel = metadata["channel_id"]
            .as_str()
            .filter(|v| valid_channel(v))
            .ok_or(IDENTITY_ERROR)?;
        if let Some(raw) = metadata.get("channel_url") {
            let raw = raw.as_str().ok_or(IDENTITY_ERROR)?;
            let url = youtube_url(raw, &format!("/channel/{channel}")).ok_or(IDENTITY_ERROR)?;
            if url.query().is_some() {
                return Err(IDENTITY_ERROR.into());
            }
        }
        let chat = &chats[0];
        if chat["video_id"].as_str() != Some(reference.external_id.as_str())
            || chat["protocol"].as_str() != Some("youtube_live_chat")
            || chat["ext"].as_str() != Some("json")
            || !chat["url"]
                .as_str()
                .is_some_and(|url| valid_watch(url, &reference.external_id))
        {
            return Err(IDENTITY_ERROR.into());
        }
        Ok(Some(Self {
            video_id: reference.external_id.clone(),
            channel_id: channel.into(),
        }))
    }

    fn input(&self) -> Result<Vec<u8>, String> {
        // In particular, no webpage_url: yt-dlp otherwise re-extracts on failure.
        // Neither the descriptor URL nor remote headers/request data are copied.
        let bytes = serde_json::to_vec(&json!({
            "_type": "video", "id": self.video_id, "video_id": self.video_id,
            "channel_id": self.channel_id, "title": "Live chat", "is_live": true,
            "ext": "json", "protocol": "youtube_live_chat", "format_id": "chat",
            "extractor": "youtube", "extractor_key": "Youtube", "vcodec": "none", "acodec": "none",
            "url": format!("https://www.youtube.com/watch?v={}&bpctr=9999999999&has_verified=1", self.video_id),
        })).map_err(|_| IDENTITY_ERROR)?;
        if bytes.len() > 8192 {
            return Err(IDENTITY_ERROR.into());
        }
        Ok(bytes)
    }
}

fn arguments() -> Vec<String> {
    // Fresh immutable flags. Never copy the synthetic probe's local proxy or
    // certificate bypass, a user config, URL argument, exec hook or extractor data.
    [
        "--ignore-config",
        "--no-plugin-dirs",
        "--no-cache-dir",
        "--no-remote-components",
        "--no-js-runtimes",
        "--no-mark-watched",
        "--no-playlist",
        "--no-warnings",
        "--no-progress",
        "--socket-timeout",
        "15",
        "--retries",
        "2",
        "--extractor-retries",
        "0",
        "--fragment-retries",
        "2",
        "--concurrent-fragments",
        "1",
        "--load-info-json",
        "-",
        "--output",
        "-",
        "--format",
        "chat",
        "--downloader",
        "native",
        "--no-simulate",
        "--no-write-subs",
        "--no-write-auto-subs",
        "--no-write-info-json",
        "--no-write-thumbnail",
        "--fixup",
        "never",
        "--no-continue",
        "--abort-on-unavailable-fragments",
        "--max-filesize",
        "32M",
        "--buffer-size",
        "16K",
        "--no-resize-buffer",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

pub(super) fn owned_source_name(name: &str) -> bool {
    // Pinned FragmentFD uses "%s-Frag%d". Its stdout tmpfilename is "-",
    // so the actual owned leaf starts with TWO hyphens, not one.
    let Some(number) = name.strip_prefix("--Frag") else {
        return false;
    };
    let number = number.strip_suffix(".part").unwrap_or(number);
    !number.is_empty() && number.len() <= 20 && number.bytes().all(|c| c.is_ascii_digit())
}

fn pin_parents(directory: &Path) -> Result<Vec<File>, String> {
    storage::safe_path(directory)?;
    let mut pins = Vec::new();
    for ancestor in directory.ancestors() {
        if pins.len() >= 64 {
            return Err(SOURCE_ERROR.into());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(3).custom_flags(0x02000000 | 0x00200000);
        }
        let file = options.open(ancestor).map_err(|_| SOURCE_ERROR)?;
        let metadata = file.metadata().map_err(|_| SOURCE_ERROR)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(SOURCE_ERROR.into());
            }
        }
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(SOURCE_ERROR.into());
        }
        pins.push(file);
    }
    Ok(pins)
}

fn fragment_size(path: &Path) -> Result<Option<u64>, String> {
    Ok(fragment_file(path)?.map(|(_, size)| size))
}

fn fragment_file(path: &Path) -> Result<Option<(File, u64)>, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // A downloader may rename/remove its OWN active part. Do not prevent it.
        options.share_mode(7).custom_flags(0x00200000);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(SOURCE_ERROR.into()),
    };
    let metadata = file.metadata().map_err(|_| SOURCE_ERROR)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(SOURCE_ERROR.into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                .map_err(|_| SOURCE_ERROR)?;
        }
        if info.dwFileAttributes & 0x400 != 0 || info.nNumberOfLinks != 1 {
            return Err(SOURCE_ERROR.into());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(SOURCE_ERROR.into());
        }
    }
    Ok(Some((file, metadata.len())))
}

const BOOTSTRAP_LIMIT: usize = 2 * 1024 * 1024;
/// Validate a completed first server reply, not a DOM, message count or .part file.
/// A quiet live room with zero messages can still have a valid continuation.
fn bootstrap_actions(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    if bytes.is_empty() || bytes.len() > BOOTSTRAP_LIMIT { return Err(SOURCE_ERROR.into()); }
    let text = std::str::from_utf8(bytes).map_err(|_| SOURCE_ERROR)?;
    let payload = if text.trim_start().starts_with('{') { text.trim_start() } else {
        ["ytInitialData = ", "ytInitialData=", "window[\"ytInitialData\"] = "]
            .iter().find_map(|marker| text.find(marker).map(|index| &text[index + marker.len()..]))
            .ok_or(SOURCE_ERROR)?
    };
    let data = serde_json::Deserializer::from_str(payload).into_iter::<Value>().next().ok_or(SOURCE_ERROR)?.map_err(|_| SOURCE_ERROR)?;
    let continuation = data.pointer("/continuationContents/liveChatContinuation").filter(|v| v.is_object()).ok_or(SOURCE_ERROR)?;
    let continuations = continuation["continuations"].as_array().filter(|rows| !rows.is_empty() && rows.len() <= 8).ok_or(SOURCE_ERROR)?;
    if !continuations.iter().any(|row| ["invalidationContinuationData", "timedContinuationData"].iter().any(|kind| {
        row[*kind]["continuation"].as_str().is_some_and(|s| !s.is_empty() && s.len() <= 32768)
    })) { return Err(SOURCE_ERROR.into()); }
    let mut rows = Vec::new();
    if let Some(actions) = continuation.get("actions") {
        let actions = actions.as_array().filter(|v| v.len() <= 500).ok_or(SOURCE_ERROR)?;
        let mut total = 0;
        for action in actions {
            if !action.is_object() { return Err(SOURCE_ERROR.into()); }
            let row = serde_json::to_vec(&json!({"replayChatItemAction":{"actions":[action]}, "isLive":true})).map_err(|_| SOURCE_ERROR)?;
            total += row.len();
            if row.len() > 512 * 1024 || total > 1024 * 1024 { return Err(SOURCE_ERROR.into()); }
            rows.push(row);
        }
    }
    Ok(rows)
}
fn bootstrap(directory: &Path) -> Result<Option<Vec<Vec<u8>>>, String> {
    let Some((mut file, length)) = fragment_file(&directory.join("--Frag1"))? else { return Ok(None); };
    if length > BOOTSTRAP_LIMIT as u64 { return Err(SOURCE_ERROR.into()); }
    let mut bytes = Vec::new();
    (&mut file).take((BOOTSTRAP_LIMIT + 1) as u64).read_to_end(&mut bytes).map_err(|_| SOURCE_ERROR)?;
    if bytes.len() as u64 != length || file.metadata().map_err(|_| SOURCE_ERROR)?.len() != length { return Err(SOURCE_ERROR.into()); }
    bootstrap_actions(&bytes).map(Some)
}

fn inspect(directory: &Path, available: u64) -> Result<(), String> {
    if available < SPACE_RESERVE {
        return Err("채팅 저장 여유 공간이 부족합니다. 받은 기록은 보존했습니다.".into());
    }
    let mut total = 0u64;
    for (count, entry) in fs::read_dir(directory)
        .map_err(|_| SOURCE_ERROR)?
        .enumerate()
    {
        if count >= MAX_FILES {
            return Err(SOURCE_ERROR.into());
        }
        let entry = entry.map_err(|_| SOURCE_ERROR)?;
        if !entry.file_name().to_str().is_some_and(owned_source_name) {
            return Err(SOURCE_ERROR.into());
        }
        total = total.saturating_add(fragment_size(&entry.path())?.unwrap_or(0));
        if total > FRAGMENT_BUDGET {
            return Err("채팅 임시 조각 한도에 도달했습니다. 받은 기록은 보존했습니다.".into());
        }
    }
    Ok(())
}

pub(super) fn receive(
    service: &PlatformMediaService,
    descriptor: &Descriptor,
    directory: &Path,
    stop: &Arc<AtomicBool>,
    on_line: impl FnMut(&[u8]) -> Result<(), String> + Send,
    on_bootstrap: impl Fn(Vec<Vec<u8>>) -> Result<(), String>,
    on_failure: impl Fn(&str) + Sync,
) -> Result<(), String> {
    let input = descriptor.input()?;
    let _parents = pin_parents(directory)?;
    let directory = directory.join("chat-source");
    storage::safe_path(&directory)?;
    // One fresh producer-owned directory; never adopt an interrupted prior source.
    fs::create_dir(&directory).map_err(|_| "채팅 임시 폴더를 준비하지 못했습니다.")?;
    let _pins = pin_parents(&directory)?;
    let mut ready = false;
    process::stream_lines(
        &service.inner.tools.yt_dlp,
        &arguments(),
        &directory,
        &input,
        stop,
        Duration::from_secs(24 * 3600),
        on_line,
        || {
            inspect(
                &directory,
                fs2::available_space(&directory).map_err(|_| SOURCE_ERROR)?,
            )?;
            if !ready {
                if let Some(rows) = bootstrap(&directory)? { on_bootstrap(rows)?; ready = true; }
            }
            Ok(())
        },
        on_failure,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bootstrap_requires_server_continuation_not_a_message_or_dom() {
        let quiet=json!({"continuationContents":{"liveChatContinuation":{"actions":[],"continuations":[{"timedContinuationData":{"continuation":"fixture","timeoutMs":10000}}]}}});
        let bytes=serde_json::to_vec(&quiet).unwrap();
        assert!(bootstrap_actions(&bytes).unwrap().is_empty());
        let html=format!("<script>var ytInitialData = {}; </script>",quiet);
        assert!(bootstrap_actions(html.as_bytes()).unwrap().is_empty());
        for bad in [b"<div id=chat>loaded</div>".as_slice(), b"{\"continuationContents\":", b"{\"continuationContents\":{\"liveChatContinuation\":{\"actions\":[]}}}"] {assert!(bootstrap_actions(bad).is_err());}
        let root=tempfile::tempdir().unwrap(); fs::write(root.path().join("--Frag1.part"),bytes).unwrap();
        assert!(bootstrap(root.path()).unwrap().is_none());
    }
    fn reference() -> MediaRef {
        MediaRef {
            provider: "youtube".into(),
            kind: "live".into(),
            external_id: "Fixture0001".into(),
        }
    }
    fn metadata() -> Value {
        json!({ "id": "Fixture0001", "is_live": true, "channel_id": "UC0123456789012345678901",
            "channel_url": "https://www.youtube.com/channel/UC0123456789012345678901",
            "subtitles": { "live_chat": [{ "video_id": "Fixture0001", "protocol": "youtube_live_chat", "ext": "json",
                "url": "https://www.youtube.com/watch?v=Fixture0001&bpctr=9999999999&has_verified=1" }] } })
    }
    #[test]
    fn descriptor_is_bound_to_the_verified_live_video_and_channel() {
        assert!(Descriptor::from_metadata(&reference(), &metadata())
            .unwrap()
            .is_some());
        for (key, value) in [
            ("id", json!("Fixture0002")),
            ("is_live", json!(false)),
            ("channel_id", json!("UC0123456789012345678902")),
            ("channel_id", json!("../bad")),
        ] {
            let mut data = metadata();
            data[key] = value;
            assert!(Descriptor::from_metadata(&reference(), &data).is_err());
        }
        let mut data = metadata();
        data["subtitles"] = json!({});
        assert!(Descriptor::from_metadata(&reference(), &data)
            .unwrap()
            .is_none());
    }
    #[test]
    fn descriptor_rejects_alternate_target_protocol_host_query_and_duplicates() {
        for (key, value) in [
            ("video_id", json!("Fixture0002")),
            ("protocol", json!("https")),
            ("protocol", json!("youtube_live_chat_replay")),
            ("ext", json!("mp4")),
            ("url", json!("https://127.0.0.1/watch?v=Fixture0001")),
            ("url", json!("https://www.youtube.com/watch?v=Fixture0002")),
            (
                "url",
                json!("https://www.youtube.com/watch?v=Fixture0001&v=Fixture0001"),
            ),
            (
                "url",
                json!("https://www.youtube.com/watch?v=Fixture0001&other=1"),
            ),
            (
                "url",
                json!("https://www.youtube.com/watch?v=Fixture0001#fragment"),
            ),
            (
                "url",
                json!("https://user@www.youtube.com/watch?v=Fixture0001"),
            ),
        ] {
            let mut data = metadata();
            data["subtitles"]["live_chat"][0][key] = value;
            assert!(Descriptor::from_metadata(&reference(), &data).is_err());
        }
        let mut data = metadata();
        let entry = data["subtitles"]["live_chat"][0].clone();
        data["subtitles"]["live_chat"] = json!([entry.clone(), entry]);
        assert!(Descriptor::from_metadata(&reference(), &data).is_err());
    }
    #[test]
    fn fresh_input_and_flags_cannot_copy_reextract_accounts_or_fixture_tls_settings() {
        let mut data = metadata();
        data["webpage_url"] = json!("https://invalid.example/");
        data["http_headers"] = json!({"Cookie": "fixture-secret"});
        data["subtitles"]["live_chat"][0]["request_data"] = json!("fixture-secret");
        let descriptor = Descriptor::from_metadata(&reference(), &data)
            .unwrap()
            .unwrap();
        let input = descriptor.input().unwrap();
        assert!(input.len() < 8192);
        let input: Value = serde_json::from_slice(&input).unwrap();
        for name in [
            "webpage_url",
            "original_url",
            "entries",
            "formats",
            "subtitles",
            "http_headers",
            "cookies",
            "request_data",
        ] {
            assert!(input.get(name).is_none());
        }
        assert_eq!(input["protocol"], "youtube_live_chat");
        let args = arguments();
        for name in [
            "--proxy",
            "--no-check-certificates",
            "--cookies",
            "--cookies-from-browser",
            "--exec",
            "--skip-download",
        ] {
            assert!(!args.iter().any(|arg| arg == name));
        }
        for pair in [
            ["--load-info-json", "-"],
            ["--output", "-"],
            ["--format", "chat"],
            ["--downloader", "native"],
        ] {
            assert!(args
                .windows(2)
                .any(|values| values[0] == pair[0] && values[1] == pair[1]));
        }
    }
    #[test]
    fn sampled_fragment_budget_rejects_unknown_links_and_low_space_without_removing_files() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("--Frag0.part");
        File::create(&file)
            .unwrap()
            .set_len(FRAGMENT_BUDGET + 1)
            .unwrap();
        assert!(inspect(directory.path(), SPACE_RESERVE).is_err());
        assert_eq!(fs::metadata(&file).unwrap().len(), FRAGMENT_BUDGET + 1);
        File::create(&file).unwrap();
        assert!(inspect(directory.path(), SPACE_RESERVE).is_ok());
        assert!(inspect(directory.path(), SPACE_RESERVE - 1).is_err());
        let other = directory.path().join("unknown");
        fs::write(&other, b"fixture").unwrap();
        assert!(inspect(directory.path(), SPACE_RESERVE).is_err());
        assert_eq!(fs::read(&other).unwrap(), b"fixture");
        fs::remove_file(&other).unwrap();
        fs::hard_link(&file, directory.path().join("--Frag1")).unwrap();
        assert!(inspect(directory.path(), SPACE_RESERVE).is_err());
    }
    #[test]
    fn temporary_fragment_names_are_exact_and_not_archive_or_path_patterns() {
        let pinned_fragment = format!("{}-Frag{}.part", "-", 0);
        assert_eq!(pinned_fragment, "--Frag0.part");
        assert!(owned_source_name(&pinned_fragment));
        for name in ["--Frag0", "--Frag123.part"] {
            assert!(owned_source_name(name));
        }
        for name in [
            "-Frag0",
            "-Frag123.part",
            "--Frag",
            "--Frag1.part.more",
            "--Frag../1",
            "chat.live_chat.json",
            "--Frag0/child",
            "--Frag١",
        ] {
            assert!(!owned_source_name(name));
        }
    }
}
