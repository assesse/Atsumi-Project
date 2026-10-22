//! Isolated public WebM shape observation, never a production capture admission.
//! Contract: docs/requirements/youtube-webm-shape-probe-20261015.json.
//! Root owns execution. No chat, account profile, media file or arbitrary output.
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{atomic::{AtomicBool, AtomicI32, Ordering}, Arc},
    thread,
    time::{Duration, Instant},
};
use tauri::{Webview, WebviewBuilder, WebviewUrl};

const MAX_SNAPSHOT: usize = 32 * 1024;
const MAX_SAMPLES: usize = 8;
const MAX_NUMBER: f64 = 1_000_000_000_000.0;
const BOOL_STATS: &[&str] = &["completeInit", "partialHeader", "openUnknownCluster", "partialBlock"];
const NULLABLE_STATS: &[&str] = &[
    "timestampScale", "trackNumber", "defaultDurationNs", "width", "height", "trackScale",
    "codecDelayNs", "flagLacing", "vp9Profile", "firstPtsSeconds", "lastPtsSeconds",
    "minCadenceSeconds", "maxCadenceSeconds", "audioTimescale", "firstDtsSeconds", "lastEndSeconds",
    "vp9Width", "vp9Height", "vp9BitDepth", "vp9ColorSpace", "vp9FullRange",
];
const COUNT_STATS: &[&str] = &[
    "ebmlHeaders", "segments", "segmentUnknown", "infoCount", "tracksCount", "trackEntries",
    "videoTracks", "codecVp9", "codecOther", "codecPrivate", "contentEncodings", "compression",
    "encryption", "clusters", "unknownClusters", "closedClusters", "simpleBlocks", "blockGroups",
    "blocks", "lacingNone", "lacingXiph", "lacingFixed", "lacingEbml", "lacedFrames",
    "referenceBlocks", "blockDurations", "containerKeys", "otherTrackBlocks", "vp9Keys",
    "vp9Inter", "vp9Invisible", "vp9ShowExisting", "vp9MarkerErrors", "vp9HeaderUnknown",
    "superframes", "superframeFrames", "superframeInvalid", "mp4Boxes", "moofs", "mdats", "audioSamples",
];
static RESULT: AtomicI32 = AtomicI32::new(2);

// deserialize_with deliberately makes nullable fields required, not optional keys.
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where D: Deserializer<'de>, T: Deserialize<'de> {
    Option::<T>::deserialize(deserializer)
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Reason { Observing, Done, Limit, Malformed, Unsupported, Excluded, Detached, Closed, SourceChanged, ObserverError }
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Kind { Webm, Mp4, Other }
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Mode { Segments, Sequence, Other }
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum AppendWindow { Default, Other }
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Clock {
    at_ms: f64,
    start_seconds: f64,
    #[serde(deserialize_with = "nullable")]
    end_seconds: Option<f64>,
    #[serde(deserialize_with = "nullable")]
    offset_seconds: Option<f64>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Track {
    track: u8,
    generation: u8,
    mime: String,
    kind: Kind,
    reason: Reason,
    detached: bool,
    appends: u64,
    bytes: u64,
    mode: Mode,
    #[serde(deserialize_with = "nullable")]
    timestamp_offset: Option<f64>,
    offset_changed: bool,
    append_window: AppendWindow,
    stats: BTreeMap<String, Value>,
    clock: Vec<Clock>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Source { source: u8, selected: bool, closed: bool, tracks: Vec<Track> }
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Video {
    #[serde(deserialize_with = "nullable")]
    current_time: Option<f64>,
    #[serde(deserialize_with = "nullable")]
    decoded_frames: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    ready_state: Option<u8>,
    paused: bool,
    ended: bool,
    protected: bool,
    source_object: bool,
    #[serde(deserialize_with = "nullable")]
    width: Option<u32>,
    #[serde(deserialize_with = "nullable")]
    height: Option<u32>,
    advertisement: bool,
    #[serde(deserialize_with = "nullable")]
    selected_source: Option<u8>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Snapshot {
    version: u8,
    elapsed_ms: f64,
    stopped: bool,
    observer_error: bool,
    total_bytes: u64,
    sources: Vec<Source>,
    #[serde(deserialize_with = "nullable")]
    video: Option<Video>,
}
fn finite(value: f64, minimum: f64, maximum: f64) -> bool {
    value.is_finite() && value >= minimum && value <= maximum
}
fn safe_mime(mime: &str) -> bool {
    if matches!(mime, "other" | "video/webm;codecs=vp9" | "audio/mp4;codecs=mp4a.40.2") { return true; }
    if let Some(codec) = mime.strip_prefix("video/webm;codecs=vp09.") {
        let parts: Vec<_> = codec.split('.').collect();
        return (3..=8).contains(&parts.len())
            && parts.iter().all(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_digit()));
    }
    ["video/mp4;codecs=avc1.", "video/mp4;codecs=avc3."].iter().any(|prefix| {
        mime.strip_prefix(*prefix).is_some_and(|codec| codec.len() == 6 && codec.bytes().all(|byte| byte.is_ascii_hexdigit()))
    })
}
fn safe_stats(stats: &BTreeMap<String, Value>) -> bool {
    stats.len() == BOOL_STATS.len() + NULLABLE_STATS.len() + COUNT_STATS.len()
        && BOOL_STATS.iter().all(|key| stats.get(*key).is_some_and(Value::is_boolean))
        && NULLABLE_STATS.iter().all(|key| stats.get(*key).is_some_and(|value| {
            value.is_null() || value.as_f64().is_some_and(|number| finite(number, -MAX_NUMBER, MAX_NUMBER))
        }))
        && COUNT_STATS.iter().all(|key| stats.get(*key).and_then(Value::as_u64).is_some_and(|number| number <= MAX_NUMBER as u64))
}
fn sanitize(raw: &str) -> Option<Snapshot> {
    if raw.len() > MAX_SNAPSHOT { return None; }
    let value: Snapshot = serde_json::from_str(raw).ok()?;
    if value.version != 1 || !finite(value.elapsed_ms, 0.0, 70_000.0)
        || value.total_bytes > 67_108_864 || value.sources.len() > 8 { return None; }
    let mut sources = HashSet::new();
    let mut selected = None;
    for source in &value.sources {
        if !(1..=8).contains(&source.source) || !sources.insert(source.source) || source.tracks.len() > 2 { return None; }
        if source.selected {
            if selected.replace(source.source).is_some() { return None; }
        }
        let mut tracks = HashSet::new();
        for track in &source.tracks {
            if !(1..=2).contains(&track.track) || !tracks.insert(track.track)
                || !(1..=8).contains(&track.generation) || !safe_mime(&track.mime)
                || track.appends > 128 || track.bytes > 33_554_432 || track.clock.len() > 8
                || !track.timestamp_offset.is_none_or(|number| finite(number, -1_000_000_000.0, 1_000_000_000.0))
                || !safe_stats(&track.stats) { return None; }
            if track.clock.iter().any(|clock| !finite(clock.at_ms, 0.0, 70_000.0)
                || !finite(clock.start_seconds, 0.0, 1_000_000_000.0)
                || !clock.end_seconds.is_none_or(|number| finite(number, clock.start_seconds, 1_000_000_000.0))
                || !clock.offset_seconds.is_none_or(|number| finite(number, -1_000_000_000.0, 1_000_000_000.0))) { return None; }
        }
    }
    if let Some(video) = &value.video {
        if !video.current_time.is_none_or(|number| finite(number, 0.0, 1_000_000_000.0))
            || video.decoded_frames.is_some_and(|number| number > MAX_NUMBER as u64)
            || video.ready_state.is_some_and(|number| number > 4)
            || video.width.is_some_and(|number| number > 65_535)
            || video.height.is_some_and(|number| number > 65_535)
            || video.selected_source != selected { return None; }
    } else if selected.is_some() { return None; }
    // Serialize only this closed typed projection, never the input Value/error.
    (serde_json::to_vec(&value).ok()?.len() <= MAX_SNAPSHOT).then_some(value)
}
fn valid_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}
fn approved(url: &tauri::Url, id: &str) -> bool {
    if url.scheme() != "https" || url.host_str() != Some("www.youtube.com") || url.path() != "/watch"
        || url.port().is_some() || !url.username().is_empty() || url.password().is_some()
        || url.fragment().is_some() { return false; }
    let videos: Vec<_> = url.query_pairs().filter(|(key, _)| key == "v").collect();
    videos.len() == 1 && videos[0].1 == id
}
fn read(view: &Webview, id: &str) -> Option<Snapshot> {
    if !approved(&view.url().ok()?, id) { return None; }
    let script = format!("(() => {{ if(window!==window.top||location.protocol!=='https:'||location.hostname!=='www.youtube.com'||location.pathname!=='/watch'||new URLSearchParams(location.search).getAll('v').length!==1||new URLSearchParams(location.search).get('v')!=={})return null;return window.__atsumiYouTubeWebmShapeProbe?.snapshot()??null; }})()", serde_json::to_string(id).ok()?);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    view.eval_with_callback(&script, move |raw| {
        let bounded = (raw.len() <= MAX_SNAPSHOT).then_some(raw);
        let _ = tx.send(bounded);
    }).ok()?;
    let raw = rx.recv_timeout(Duration::from_secs(3)).ok()??;
    if !approved(&view.url().ok()?, id) { return None; }
    sanitize(&raw)
}
fn run() -> Result<(), ()> {
    let id = std::env::var("ATSUMI_YOUTUBE_WEBM_PROBE_ID").unwrap_or_else(|_| "T4XZmMPQ9Kw".into());
    if !valid_id(&id) { return Err(()); }
    let profile = tempfile::Builder::new().prefix("atsumi-youtube-webm-shape-").tempdir().map_err(|_| ())?;
    let profile_path = profile.path().join("webview-profile");
    let done = Arc::new(AtomicBool::new(false));
    let final_done = done.clone();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().app.tray_icon = None;
    let app = tauri::Builder::default().setup(move |app| {
        let handle = app.handle().clone();
        let deadline_handle = handle.clone();
        let deadline_done = done.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(70));
            if !deadline_done.load(Ordering::Acquire) {
                println!("{}", json!({"kind":"youtube_webm_shape_probe","outcome":"deadline","recordingTested":false}));
                deadline_handle.exit(3);
            }
        });
        let window = tauri::window::WindowBuilder::new(app, "youtube-webm-shape-probe")
            .title("Isolated YouTube WebM shape probe").inner_size(1280.0, 720.0)
            .position(-16000.0, -16000.0).skip_taskbar(true).focused(false).visible(true).build()?;
        let approved_id = id.clone();
        let script = format!("window.__atsumiYouTubeWebmShapeProbeId={};\n{}", serde_json::to_string(&id)?, include_str!("youtube_webm_shape_probe.js"));
        let view = window.add_child(WebviewBuilder::new("youtube-webm-shape-watch", WebviewUrl::External(format!("https://www.youtube.com/watch?v={id}&autoplay=1&mute=1").parse()?))
            .focused(false).data_directory(profile_path).initialization_script(script)
            .additional_browser_args("--autoplay-policy=no-user-gesture-required --mute-audio --no-first-run --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows")
            .on_navigation(move |url| approved(url, &approved_id))
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny),
            tauri::LogicalPosition::new(0.0, 0.0), tauri::LogicalSize::new(1280.0, 720.0))?;
        thread::spawn(move || {
            let started = Instant::now();
            let mut snapshots = Vec::new();
            let mut rejected = 0usize;
            let mut first_time = None;
            let mut first_frames = None;
            let mut progress_source = None;
            let mut progressed = false;
            let mut decoded_progress = false;
            for index in 0..MAX_SAMPLES {
                let target = Duration::from_secs(index as u64 * 7);
                if let Some(delay) = target.checked_sub(started.elapsed()) { thread::sleep(delay); }
                if started.elapsed() >= Duration::from_secs(55) { break; }
                match read(&view, &id) {
                    Some(snapshot) => {
                        if let Some(video) = &snapshot.video {
                            if video.selected_source.is_some() && !video.advertisement && !video.protected
                                && !video.source_object && !video.paused && video.ready_state.is_some_and(|number| number >= 2) {
                                if progress_source != video.selected_source {
                                    progress_source = video.selected_source;
                                    first_time = None;
                                    first_frames = None;
                                }
                                if let Some(time) = video.current_time {
                                    let first = *first_time.get_or_insert(time);
                                    progressed |= time > first + 2.0;
                                }
                                if let Some(frames) = video.decoded_frames {
                                    let first = *first_frames.get_or_insert(frames);
                                    decoded_progress |= frames > first;
                                }
                            } else {
                                progress_source = None;
                                first_time = None;
                                first_frames = None;
                            }
                        }
                        snapshots.push(snapshot);
                    },
                    None => rejected += 1,
                }
            }
            if let Some(delay) = Duration::from_secs(55).checked_sub(started.elapsed()) { thread::sleep(delay); }
            let observed = !snapshots.is_empty();
            println!("{}", json!({"kind":"youtube_webm_shape_probe","outcome":if observed {"observed"} else {"no_valid_snapshot"},"isolatedProfile":true,"defaultUserAgent":true,"loginTested":false,"recordingTested":false,"playbackProgressed":progressed,"decodedFramesProgressed":decoded_progress,"rejectedSnapshots":rejected,"snapshots":snapshots}));
            let code = if observed { 0 } else { 2 };
            RESULT.store(code, Ordering::Release);
            done.store(true, Ordering::Release);
            handle.exit(code);
        });
        Ok(())
    }).build(context).map_err(|_| ())?;
    app.run_return(|_, _| {});
    final_done.store(true, Ordering::Release);
    drop(profile);
    (RESULT.load(Ordering::Acquire) == 0).then_some(()).ok_or(())
}
fn main() {
    if !cfg!(debug_assertions) {
        println!("{}", json!({"kind":"youtube_webm_shape_probe","outcome":"debug_only"}));
        std::process::exit(2);
    }
    if run().is_err() {
        println!("{}", json!({"kind":"youtube_webm_shape_probe","outcome":"not_verified","recordingTested":false}));
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Value {
        let mut stats = serde_json::Map::new();
        for key in BOOL_STATS { stats.insert((*key).into(), json!(false)); }
        for key in NULLABLE_STATS { stats.insert((*key).into(), Value::Null); }
        for key in COUNT_STATS { stats.insert((*key).into(), json!(0)); }
        json!({"version":1,"elapsedMs":5000,"stopped":false,"observerError":false,"totalBytes":100,
            "sources":[{"source":1,"selected":true,"closed":false,"tracks":[{
                "track":1,"generation":1,"mime":"video/webm;codecs=vp09.00.51.08.01.01.01.01.00",
                "kind":"webm","reason":"observing","detached":false,"appends":1,"bytes":100,
                "mode":"segments","timestampOffset":0,"offsetChanged":false,"appendWindow":"default",
                "stats":stats,"clock":[{"atMs":5,"startSeconds":100,"endSeconds":null,"offsetSeconds":0}]}]}],
            "video":{"currentTime":100,"decodedFrames":2,"readyState":4,"paused":false,"ended":false,
                "protected":false,"sourceObject":false,"width":1920,"height":1080,"advertisement":false,"selectedSource":1}})
    }
    fn accepted(value: &Value) -> bool { sanitize(&serde_json::to_string(value).unwrap()).is_some() }
    #[test]
    fn only_closed_bounded_projection_is_serialized() {
        let value = fixture();
        assert!(accepted(&value));
        let clean = sanitize(&value.to_string()).unwrap();
        let projected = serde_json::to_value(clean).unwrap();
        assert!(accepted(&projected));
        assert_eq!(projected["sources"][0]["tracks"][0]["stats"], value["sources"][0]["tracks"][0]["stats"]);
        for path in ["url", "cookie", "title", "headers", "raw"] {
            let mut value = fixture(); value[path] = json!("not_allowed"); assert!(!accepted(&value));
            let mut value = fixture(); value["sources"][0]["tracks"][0]["stats"][path] = json!(1); assert!(!accepted(&value));
        }
    }
    #[test]
    fn enums_nullable_presence_and_numeric_limits_are_strict() {
        for (key, bad) in [("reason", json!("https://secret.invalid")), ("kind", json!("anything")),
            ("bytes", json!(33_554_433u64)), ("appends", json!(129)), ("generation", json!(9)),
            ("timestampOffset", json!(1_000_000_001.0))] {
            let mut value = fixture(); value["sources"][0]["tracks"][0][key] = bad; assert!(!accepted(&value));
        }
        let mut value = fixture(); value["video"].as_object_mut().unwrap().remove("currentTime"); assert!(!accepted(&value));
        let mut value = fixture();
        for key in ["readyState", "width", "height"] { value["video"][key] = Value::Null; }
        value["sources"][0]["tracks"][0]["clock"][0]["offsetSeconds"] = Value::Null;
        assert!(accepted(&value));
        let mut value = fixture(); value["sources"][0]["tracks"][0]["stats"]["blocks"] = json!(-1); assert!(!accepted(&value));
        let mut value = fixture(); value["sources"][0]["tracks"][0]["stats"]["width"] = json!("1920"); assert!(!accepted(&value));
        let mut value = fixture(); value["sources"][0]["tracks"][0]["stats"]["completeInit"] = json!(1); assert!(!accepted(&value));
        assert!(sanitize(&" ".repeat(MAX_SNAPSHOT + 1)).is_none());
    }
    #[test]
    fn selected_source_counts_and_clock_bounds_cannot_be_forged() {
        let mut value = fixture(); value["video"]["selectedSource"] = json!(2); assert!(!accepted(&value));
        let mut value = fixture(); let source = value["sources"][0].clone(); value["sources"].as_array_mut().unwrap().push(source); assert!(!accepted(&value));
        let mut value = fixture(); let track = value["sources"][0]["tracks"][0].clone(); value["sources"][0]["tracks"].as_array_mut().unwrap().push(track); assert!(!accepted(&value));
        let mut value = fixture(); let clock = value["sources"][0]["tracks"][0]["clock"][0].clone(); value["sources"][0]["tracks"][0]["clock"] = json!(vec![clock; 9]); assert!(!accepted(&value));
        let mut value = fixture(); value["sources"][0]["tracks"][0]["clock"][0]["endSeconds"] = json!(99); assert!(!accepted(&value));
        let mut value = fixture(); value["sources"][0]["tracks"][0]["clock"][0]["offsetSeconds"] = json!(-1_000_000_001.0); assert!(!accepted(&value));
    }
    #[test]
    fn mime_accepts_only_fixed_safe_codec_grammar() {
        for mime in ["other", "video/webm;codecs=vp9", "video/webm;codecs=vp09.00.51.08",
            "video/webm;codecs=vp09.00.51.08.01.01.01.01.00", "audio/mp4;codecs=mp4a.40.2", "video/mp4;codecs=avc1.4D401F"] {
            assert!(safe_mime(mime));
        }
        for mime in ["video/webm;codecs=vp09.secret", "video/mp4;codecs=avc1.abcdef;token=secret", "https://secret.invalid", "video/webm\n"] { assert!(!safe_mime(mime)); }
    }
    #[test]
    fn navigation_is_only_the_exact_selected_public_watch() {
        let id = "T4XZmMPQ9Kw";
        assert!(approved(&format!("https://www.youtube.com/watch?v={id}&autoplay=1&mute=1").parse().unwrap(), id));
        for url in ["https://www.youtube.com/watch?v=cswFQYam8Ic", "https://www.youtube.com/live_chat?v=T4XZmMPQ9Kw",
            "https://accounts.google.com/", "https://youtube.com/watch?v=T4XZmMPQ9Kw", "http://www.youtube.com/watch?v=T4XZmMPQ9Kw",
            "https://www.youtube.com/watch?v=T4XZmMPQ9Kw&v=T4XZmMPQ9Kw", "https://www.youtube.com/watch?v=T4XZmMPQ9Kw#x"] {
            assert!(!approved(&url.parse().unwrap(), id));
        }
        assert!(!valid_id("bad\nvalue"));
    }
}
