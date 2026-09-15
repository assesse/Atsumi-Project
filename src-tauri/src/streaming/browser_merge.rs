//! Offline derivatives of completed official-browser recordings. Source cleanup
//! requires durable verified output; no network, shell or remote paths are used.
//! FFmpeg concat requires equal stream parameters and correct per-file duration:
//! https://ffmpeg.org/ffmpeg-formats.html#concat-1
use super::{
    browser_store::{
        self, BrowserCaptureStore, BrowserMergeJob, BrowserMergeStatus, BrowserMergedOutput,
        BrowserSegment,
    },
    model::StreamError,
};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const RESERVE: u64 = 512 * 1024 * 1024;
const MAX_INFO: usize = 256 * 1024;
const MAX_TIMELINE: u64 = 128 * 1024 * 1024;
const MAX_SEGMENT: u64 = 64 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(40);

/// The host supplies verified, application-managed executables. Never resolve
/// executables through PATH or the media directory. Missing tools are retryable.
#[derive(Clone, Debug)]
pub struct MediaTools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

struct WorkerState {
    wake: bool,
}
struct Shared {
    store: Arc<Mutex<BrowserCaptureStore>>,
    tools: Option<MediaTools>,
    cancel: AtomicBool,
    state: Mutex<WorkerState>,
    changed: Condvar,
}
#[derive(Clone)]
pub struct BrowserMergeWorker {
    shared: Arc<Shared>,
    handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}
impl BrowserMergeWorker {
    pub fn start(
        store: Arc<Mutex<BrowserCaptureStore>>,
        tools: Option<MediaTools>,
    ) -> Result<Self, StreamError> {
        let shared = Arc::new(Shared {
            store,
            tools,
            cancel: AtomicBool::new(false),
            state: Mutex::new(WorkerState { wake: true }),
            changed: Condvar::new(),
        });
        let run = shared.clone();
        let handle = thread::Builder::new()
            .name("browser-merge".into())
            .spawn(move || worker(run))
            .map_err(|_| failure("병합 작업자를 시작하지 못했습니다."))?;
        Ok(Self {
            shared,
            handle: Arc::new(Mutex::new(Some(handle))),
        })
    }
    pub fn wake(&self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.wake = true;
            self.shared.changed.notify_one();
        }
    }
    pub fn retry(&self, recording_id: Option<&str>) -> Result<usize, StreamError> {
        if self.shared.cancel.load(Ordering::Acquire) {
            return Err(cancelled());
        }
        let count = self
            .shared
            .store
            .lock()
            .map_err(|_| failure("녹화 목록을 확인하지 못했습니다."))?
            .retry_merges(recording_id)?;
        self.wake();
        Ok(count)
    }
    /// Kill/wait the owned tool first, then join. Never hold a host/store mutex
    /// while calling this method. Unverified sources survive cancellation;
    /// already-committed cleanup is not rolled back.
    pub fn shutdown_and_wait(&self) {
        self.shared.cancel.store(true, Ordering::Release);
        self.shared.changed.notify_all();
        if let Some(handle) = self.handle.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = handle.join();
        }
    }
}
impl Drop for BrowserMergeWorker {
    fn drop(&mut self) {
        // Thread owns Shared, not this handle Arc; the last host handle owns exit.
        if Arc::strong_count(&self.handle) == 1 {
            self.shutdown_and_wait();
        }
    }
}
fn worker(shared: Arc<Shared>) {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
    // Startup retry only touches the bounded catalog and small metadata files.
    if let Ok(store) = shared.store.lock() {
        let _ = store.retry_merges(None);
    }
    loop {
        if shared.cancel.load(Ordering::Acquire) {
            break;
        }
        let cleanup_job = shared
            .store
            .lock()
            .ok()
            .and_then(|store| store.take_cleanup_job().ok())
            .flatten();
        if let Some(job) = cleanup_job {
            // Media hashing/deletion never holds either shared store mutex.
            let result = browser_store::cleanup::remove_verified_sources(&job, &shared.cancel);
            if let Ok(store) = shared.store.lock() {
                let _ = store.finish_cleanup(&job, result);
            }
            continue;
        }
        let job = shared
            .store
            .lock()
            .ok()
            .and_then(|store| store.take_merge_job().ok())
            .flatten();
        if let Some(job) = job {
            let result = merge_recording(&job, shared.tools.as_ref(), &shared.cancel);
            if let Ok(store) = shared.store.lock() {
                match result {
                    Ok(output) => {
                        if store.complete_merge(&job, output).is_err() {
                            let _ = store.fail_merge(
                                &job,
                                BrowserMergeStatus::Failed,
                                "병합 결과를 확정하지 못했습니다. 원본과 생성된 파일은 보존됩니다.",
                            );
                        }
                    }
                    Err(error) => {
                        let status = if error.code == "BROWSER_MERGE_CANCELLED" {
                            BrowserMergeStatus::Queued
                        } else if matches!(
                            error.code.as_str(),
                            "BROWSER_MERGE_TOOL_MISSING" | "BROWSER_MERGE_SPACE"
                        ) {
                            BrowserMergeStatus::Blocked
                        } else {
                            BrowserMergeStatus::Failed
                        };
                        let _ = store.fail_merge(&job, status, &error.message);
                    }
                }
            }
            continue;
        }
        let state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
        let mut state = shared
            .changed
            .wait_timeout_while(state, Duration::from_secs(30), |s| {
                !s.wake && !shared.cancel.load(Ordering::Acquire)
            })
            .unwrap_or_else(|p| p.into_inner())
            .0;
        state.wake = false;
    }
}
fn failure(message: &str) -> StreamError {
    StreamError::new("BROWSER_MERGE_FAILED", message, true)
}
fn cancelled() -> StreamError {
    StreamError::new(
        "BROWSER_MERGE_CANCELLED",
        "병합이 중단되어 다음 실행에서 다시 시도합니다. 원본 조각은 보존됩니다.",
        true,
    )
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), StreamError> {
    if cancel.load(Ordering::Acquire) {
        Err(cancelled())
    } else {
        Ok(())
    }
}

fn verify_full_decode(
    tools: &MediaTools,
    file: &Path,
    duration: f64,
    cancel: &AtomicBool,
) -> Result<(), StreamError> {
    let mut command = Command::new(&tools.ffmpeg);
    command
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-xerror",
            "-err_detect",
            "explode",
            "-threads",
            "1",
            "-filter_threads",
            "1",
            "-max_alloc",
            "67108864",
            "-protocol_whitelist",
            "file",
            "-i",
        ])
        .arg(file)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0",
            "-progress",
            "pipe:1",
            "-stats_period",
            "60",
            "-f",
            "null",
            "-",
        ]);
    let timeout = Duration::from_secs(
        (duration.ceil() as u64)
            .saturating_mul(2)
            .saturating_add(60)
            .clamp(60, 7200),
    );
    let ToolOutput::Bytes(bytes) = run_tool(command, cancel, timeout, false, None)? else {
        unreachable!()
    };
    let progress = std::str::from_utf8(&bytes)
        .map_err(|_| failure("병합 영상의 전체 재생 검증 결과를 읽지 못했습니다."))?;
    let decoded = progress
        .lines()
        .filter_map(|line| line.strip_prefix("out_time_us=")?.parse::<f64>().ok())
        .next_back()
        .unwrap_or(0.0)
        / 1_000_000.0;
    if !progress.lines().any(|line| line == "progress=end")
        || !decoded.is_finite()
        || (decoded - duration).abs() > 0.5
    {
        return Err(failure(
            "병합 영상의 전체 재생 길이를 검증하지 못했습니다. 원본 조각은 보존됩니다.",
        ));
    }
    Ok(())
}
fn regular(path: &Path, max: u64) -> Result<fs::Metadata, StreamError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| failure("병합에 필요한 파일을 확인하지 못했습니다."))?;
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let linked = metadata.file_type().is_symlink();
    if !metadata.is_file() || linked || metadata.len() > max {
        return Err(failure("병합 파일의 형식이나 크기가 올바르지 않습니다."));
    }
    Ok(metadata)
}
fn tools_available(tools: Option<&MediaTools>) -> Result<&MediaTools, StreamError> {
    tools.filter(|t| [&t.ffmpeg, &t.ffprobe].iter().all(|p| p.is_absolute() && regular(p, 512 * 1024 * 1024).is_ok()))
        .ok_or_else(|| StreamError::new("BROWSER_MERGE_TOOL_MISSING", "병합 도구를 찾지 못했습니다. FFmpeg와 ffprobe를 준비한 뒤 다시 시도해 주세요. 원본 조각은 보존됩니다.", true))
}
fn space(root: &Path, extra: u64) -> Result<(), StreamError> {
    if fs2::available_space(root)
        .map_err(|_| failure("병합 디스크의 여유 공간을 확인하지 못했습니다."))?
        < RESERVE.saturating_add(extra)
    {
        Err(StreamError::new(
            "BROWSER_MERGE_SPACE",
            "병합할 여유 공간이 부족합니다. 원본 크기만큼의 추가 공간과 512MiB 여유가 필요합니다.",
            true,
        ))
    } else {
        Ok(())
    }
}
#[derive(Default)]
struct PacketClock {
    first: Option<f64>,
    end: f64,
    count: u64,
    invalid: bool,
}
impl PacketClock {
    fn line(&mut self, line: &str) {
        let mut pts = None;
        let mut dts = None;
        let mut duration = None;
        for field in line.trim().split('|') {
            let Some((key, value)) = field.split_once('=') else {
                continue;
            };
            let value = value.parse::<f64>().ok().filter(|v| v.is_finite());
            match key {
                "pts_time" => pts = value,
                "dts_time" => dts = value,
                "duration_time" => duration = value,
                _ => {}
            }
        }
        if let Some(at) = pts.or(dts) {
            let duration = duration.unwrap_or(0.0);
            if at.abs() > 1e9 || !(0.0..=120.0).contains(&duration) {
                self.invalid = true;
                return;
            }
            self.first = Some(self.first.map_or(at, |v| v.min(at)));
            self.end = self.end.max(at + duration);
            self.count += 1;
        }
    }
    fn duration(&self) -> Result<f64, StreamError> {
        let duration = self.end - self.first.unwrap_or(self.end);
        if self.invalid || self.count == 0 || !duration.is_finite() || duration <= 0.0 {
            Err(failure("미디어 시간축을 확인하지 못했습니다."))
        } else {
            Ok(duration)
        }
    }
}
enum ToolOutput {
    Bytes(Vec<u8>),
    Packets(PacketClock),
}
/// Drain both pipes even when their retained data reaches its bound. There is
/// exactly one child and two short-lived readers, and every exit joins all three.
fn run_tool(
    mut command: Command,
    cancel: &AtomicBool,
    timeout: Duration,
    packets: bool,
    output_limit: Option<(&Path, u64)>,
) -> Result<ToolOutput, StreamError> {
    check_cancel(cancel)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00004000); // hidden, BELOW_NORMAL_PRIORITY_CLASS
    }
    let mut child = command
        .spawn()
        .map_err(|_| failure("병합 도구를 실행하지 못했습니다."))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let read_out = thread::spawn(move || -> Result<ToolOutput, ()> {
        let mut reader = BufReader::new(stdout);
        if packets {
            let mut stats = PacketClock::default();
            loop {
                let mut line = Vec::new();
                let n = (&mut reader)
                    .take(4097)
                    .read_until(b'\n', &mut line)
                    .map_err(|_| ())?;
                if n == 0 {
                    break;
                }
                if n > 4096 {
                    stats.invalid = true;
                } else {
                    stats.line(std::str::from_utf8(&line).map_err(|_| ())?);
                }
            }
            Ok(ToolOutput::Packets(stats))
        } else {
            let mut bytes = Vec::new();
            let mut buffer = [0; 8192];
            let mut oversized = false;
            loop {
                let n = reader.read(&mut buffer).map_err(|_| ())?;
                if n == 0 {
                    break;
                }
                if bytes.len() + n <= MAX_INFO {
                    bytes.extend_from_slice(&buffer[..n]);
                } else {
                    oversized = true;
                }
            }
            if oversized {
                Err(())
            } else {
                Ok(ToolOutput::Bytes(bytes))
            }
        }
    });
    let read_err = thread::spawn(move || {
        let mut stderr = stderr;
        let mut buffer = [0; 8192];
        #[cfg(test)]
        let mut diagnostic = Vec::new();
        loop {
            let n = stderr.read(&mut buffer).unwrap_or(0);
            if n == 0 {
                break;
            }
            #[cfg(test)]
            if diagnostic.len() + n <= 16 * 1024 {
                diagnostic.extend_from_slice(&buffer[..n]);
            }
        }
        #[cfg(test)]
        {
            diagnostic
        }
    });
    let started = Instant::now();
    let outcome = loop {
        let guard = check_cancel(cancel).and_then(|()| {
            if started.elapsed() > timeout {
                return Err(failure(
                    "병합 도구의 처리 시간이 초과되었습니다. 원본 조각은 보존됩니다.",
                ));
            }
            if let Some((path, limit)) = output_limit {
                if path.exists() && regular(path, limit).is_err() {
                    return Err(failure("병합 결과의 크기 제한을 초과했습니다."));
                }
                space(
                    path.parent()
                        .ok_or_else(|| failure("병합 경로가 올바르지 않습니다."))?,
                    0,
                )?;
            }
            Ok(())
        });
        if let Err(error) = guard {
            let _ = child.kill();
            let _ = child.wait();
            break Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(failure(
                        "미디어 조각을 병합하거나 검증하지 못했습니다. 원본 조각은 보존됩니다.",
                    ))
                }
            }
            Ok(None) => thread::sleep(POLL),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(failure("병합 도구 상태를 확인하지 못했습니다."));
            }
        }
    };
    let output = read_out
        .join()
        .map_err(|_| failure("병합 도구 응답을 확인하지 못했습니다."));
    let _diagnostic = read_err.join();
    // Enabled only by the explicitly invoked temporary-media integration tests.
    // Production always drains/discards tool stderr and returns generic errors.
    #[cfg(test)]
    if outcome.is_err() && std::env::var_os("ATSUMI_MERGE_TEST_TOOLS_DIR").is_some() {
        eprintln!(
            "synthetic media tool: {}",
            String::from_utf8_lossy(&_diagnostic.unwrap_or_default())
        );
    }
    outcome?;
    output?.map_err(|_| failure("병합 도구 응답이 제한을 초과했거나 올바르지 않습니다."))
}
#[derive(Clone)]
struct MediaInfo {
    signature: Value,
    duration: f64,
}
fn numeric(value: &Value) -> Option<f64> {
    value
        .as_str()
        .and_then(|v| v.parse().ok())
        .or_else(|| value.as_f64())
        .filter(|v| v.is_finite())
}
fn parse_info(value: &Value) -> Result<(Value, Option<f64>), StreamError> {
    let streams = value["streams"]
        .as_array()
        .ok_or_else(|| failure("영상·음성 트랙을 확인하지 못했습니다."))?;
    if streams.len() != 2 {
        return Err(failure(
            "영상 한 개와 음성 한 개가 있는 녹화만 자동 병합할 수 있습니다.",
        ));
    }
    let mut signature = Vec::new();
    for (index, expected) in ["video", "audio"].iter().enumerate() {
        let s = streams
            .iter()
            .find(|s| s["codec_type"] == *expected)
            .ok_or_else(|| failure("녹화의 영상·음성 트랙 구성이 다릅니다."))?;
        let codec = s["codec_name"].as_str().unwrap_or("");
        if !(if index == 0 {
            matches!(codec, "h264" | "vp8" | "vp9")
        } else {
            matches!(codec, "aac" | "opus")
        }) || s["time_base"].as_str().is_none()
        {
            return Err(failure("자동 병합이 지원하지 않는 코덱입니다."));
        }
        signature.push(json!({"index":s["index"],"type":s["codec_type"],"codec":s["codec_name"],"profile":s["profile"],
            "timeBase":s["time_base"],"width":s["width"],"height":s["height"],"sampleRate":s["sample_rate"],
            "channels":s["channels"],"channelLayout":s["channel_layout"],"extra":s["extradata_hash"]}));
    }
    Ok((
        Value::Array(signature),
        numeric(&value["format"]["duration"]).filter(|v| *v > 0.0),
    ))
}
fn probe(
    tools: &MediaTools,
    file: &Path,
    mime: &str,
    cancel: &AtomicBool,
    max_duration: f64,
    packet_clock: bool,
) -> Result<MediaInfo, StreamError> {
    let format = if mime.starts_with("video/webm") {
        "matroska"
    } else {
        "mov"
    };
    let mut cmd = Command::new(&tools.ffprobe);
    cmd.args(["-v","error","-max_alloc","67108864","-protocol_whitelist","file","-f",format,"-show_data_hash","sha256","-show_entries",
        "stream=index,codec_type,codec_name,profile,time_base,width,height,sample_rate,channels,channel_layout,extradata_hash:format=duration","-of","json"]).arg(file);
    let ToolOutput::Bytes(bytes) = run_tool(cmd, cancel, Duration::from_secs(20), false, None)?
    else {
        unreachable!()
    };
    let (signature, duration) = parse_info(
        &serde_json::from_slice::<Value>(&bytes)
            .map_err(|_| failure("미디어 검사 응답이 올바르지 않습니다."))?,
    )?;
    // MediaRecorder WebM often has no Duration header. Even when ffprobe
    // estimates a container duration, derive source boundaries from packets.
    let duration = if let Some(duration) = duration.filter(|_| !packet_clock) {
        duration
    } else {
        let mut cmd = Command::new(&tools.ffprobe);
        cmd.args([
            "-v",
            "error",
            "-max_alloc",
            "67108864",
            "-protocol_whitelist",
            "file",
            "-f",
            format,
            "-show_packets",
            "-show_entries",
            "packet=pts_time,dts_time,duration_time",
            "-of",
            "compact=p=0:nk=0",
        ])
        .arg(file);
        let ToolOutput::Packets(clock) =
            run_tool(cmd, cancel, Duration::from_secs(30), true, None)?
        else {
            unreachable!()
        };
        clock.duration()?
    };
    if duration > max_duration || duration <= 0.0 {
        return Err(failure("녹화 미디어의 재생 시간이 올바르지 않습니다."));
    }
    Ok(MediaInfo {
        signature,
        duration,
    })
}
fn timeline_duration(
    segments: &[BrowserSegment],
    index: usize,
    media_duration: f64,
) -> Result<f64, StreamError> {
    let segment = &segments[index];
    if (media_duration - segment.duration_seconds).abs()
        > 1.0_f64.max(segment.duration_seconds * 0.05)
    {
        return Err(failure(
            "저장 기록과 실제 조각의 재생 시간이 다릅니다. 원본을 보존했습니다.",
        ));
    }
    match (segment.source_start_seconds, segment.source_end_seconds) {
        (Some(start), Some(end)) => {
            let next = segments
                .get(index + 1)
                .map(|s| s.source_start_seconds)
                .unwrap_or(Some(end))
                .ok_or_else(|| failure("원본 시간축이 서로 다른 조각은 병합하지 않습니다."))?;
            let duration = next - start;
            // AAC can cross the video cut by one frame. Use source cut points,
            // not the max(A/V) segment duration, which accumulates audio drift.
            if duration <= 0.0 || (next - end).abs() > 0.25 {
                return Err(failure("조각 사이의 원본 시간축이 연속적이지 않습니다."));
            }
            Ok(duration)
        }
        (None, None)
            if segments.get(index + 1).is_none_or(|s| {
                s.source_start_seconds.is_none() && s.source_end_seconds.is_none()
            }) =>
        {
            Ok(media_duration)
        }
        _ => Err(failure("원본 시간축이 서로 다른 조각은 병합하지 않습니다.")),
    }
}
fn new_file(path: &Path) -> Result<File, StreamError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| failure("새 병합 파일을 만들지 못했습니다. 기존 파일은 덮어쓰지 않습니다."))
}
fn rename_new(source: &Path, target: &Path) -> Result<(), StreamError> {
    if fs::symlink_metadata(target).is_ok() {
        return Err(failure("같은 이름의 병합 파일이 이미 있습니다."));
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH},
        };
        let from = source
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let to = target
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|_| failure("병합 결과를 확정하지 못했습니다."))?;
    }
    #[cfg(not(windows))]
    {
        fs::hard_link(source, target).map_err(|_| failure("병합 결과를 확정하지 못했습니다."))?;
    }
    Ok(())
}
fn merge_recording(
    job: &BrowserMergeJob,
    tools: Option<&MediaTools>,
    cancel: &AtomicBool,
) -> Result<BrowserMergedOutput, StreamError> {
    let started = Instant::now();
    check_cancel(cancel)?;
    let tools = tools_available(tools)?;
    let root = browser_store::validate_merge_generation(job)?;
    let segments = browser_store::read_merge_segments(job)?;
    if segments.is_empty() {
        return Err(failure("확정된 녹화 조각이 없습니다."));
    }
    let encoded = segments[0].source_start_seconds.is_some();
    if segments.iter().any(|s| {
        s.source_start_seconds.is_some() != encoded || s.source_end_seconds.is_some() != encoded
    }) {
        return Err(failure("원본 시간축이 서로 다른 조각은 병합하지 않습니다."));
    }
    let extra = job.recording.bytes_written / 20 + 64 * 1024 * 1024;
    let max_output = job
        .recording
        .bytes_written
        .checked_add(extra)
        .ok_or_else(|| failure("병합 크기 한도를 초과했습니다."))?;
    space(&root, max_output)?;
    let list_name = format!("merge-{}.ffconcat", job.token);
    let timeline_name = format!("merged-{}.timeline.jsonl", job.token);
    let ext = if job.recording.mime_type.starts_with("video/webm") {
        "webm"
    } else {
        "mp4"
    };
    let name = format!("merged-{}.{}", job.token, ext);
    let partial = root.join(format!("{name}.partial"));
    let timeline_partial = root.join(format!("{timeline_name}.partial"));
    let mut manifest = new_file(&root.join(&list_name))?;
    let mut timeline = new_file(&timeline_partial)?;
    manifest
        .write_all(b"ffconcat version 1.0\n")
        .map_err(|_| failure("병합 목록을 저장하지 못했습니다."))?;
    let mut first_signature = None;
    let mut source_hashes = Vec::with_capacity(segments.len());
    let mut offset = 0.0;
    for (index, segment) in segments.iter().enumerate() {
        check_cancel(cancel)?;
        if started.elapsed() > Duration::from_secs(7200) {
            return Err(failure(
                "병합 사전 검사의 제한 시간을 초과했습니다. 원본 조각은 보존됩니다.",
            ));
        }
        let path = root.join(&segment.file);
        if regular(&path, MAX_SEGMENT)?.len() != segment.bytes {
            return Err(failure("원본 조각의 크기가 저장 기록과 다릅니다."));
        }
        source_hashes.push(browser_store::cleanup::source_hash(
            &path,
            segment.bytes,
            cancel,
        )?);
        let info = probe(
            tools,
            &path,
            &job.recording.mime_type,
            cancel,
            122.0,
            job.recording.mime_type.starts_with("video/webm"),
        )?;
        if first_signature
            .as_ref()
            .is_some_and(|s| *s != info.signature)
        {
            return Err(failure("조각의 코덱·해상도·음성 형식이 달라 무손실 자동 병합을 중단했습니다. 원본 조각은 보존됩니다."));
        }
        if first_signature.is_none() {
            first_signature = Some(info.signature);
        }
        let duration = timeline_duration(&segments, index, info.duration)?;
        // Only fixed native-generated basenames enter the safe concat grammar.
        writeln!(
            manifest,
            "file '{}'\nduration {:.9}",
            segment.file, duration
        )
        .map_err(|_| failure("병합 목록을 저장하지 못했습니다."))?;
        let row = json!({"segmentIndex":segment.index,"sourceFile":segment.file,"mergedStartSeconds":offset,"mergedDurationSeconds":duration,
            "mediaDurationSeconds":info.duration,"recordedDurationSeconds":segment.duration_seconds,
            "sourceStartSeconds":segment.source_start_seconds,"sourceEndSeconds":segment.source_end_seconds,
            "clock":if segment.source_start_seconds.is_some(){"encoded_source"}else{"media_duration"},
            "chatClock":"original_recording_receive_time","chatRewritten":false});
        serde_json::to_writer(&mut timeline, &row)
            .map_err(|_| failure("병합 시간표를 저장하지 못했습니다."))?;
        timeline
            .write_all(b"\n")
            .map_err(|_| failure("병합 시간표를 저장하지 못했습니다."))?;
        if timeline
            .metadata()
            .map_err(|_| failure("병합 시간표를 확인하지 못했습니다."))?
            .len()
            > MAX_TIMELINE
        {
            return Err(failure("병합 시간표의 크기 제한을 초과했습니다."));
        }
        offset += duration;
    }
    manifest
        .sync_all()
        .and_then(|()| timeline.sync_all())
        .map_err(|_| failure("병합 목록을 확정하지 못했습니다."))?;
    drop(manifest);
    drop(timeline);
    browser_store::validate_merge_generation(job)?;
    check_cancel(cancel)?;
    // Reserve a unique, native-generated derivative first. A killed child
    // leaves a clearly named .partial; finalized/source names are never reused.
    let output = new_file(&partial)?;
    drop(output);
    let mut cmd = Command::new(&tools.ffmpeg);
    cmd.current_dir(&root)
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-max_alloc",
            "67108864",
            "-xerror",
            "-protocol_whitelist",
            "file",
            "-format_whitelist",
            "concat,mov,matroska,webm",
            "-f",
            "concat",
            "-safe",
            "1",
            "-i",
        ])
        .arg(&list_name)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0",
            "-map_metadata",
            "-1",
            "-map_chapters",
            "-1",
            "-c",
            "copy",
        ]);
    if ext == "mp4" {
        cmd.args(["-movflags", "+faststart", "-f", "mp4"]);
    } else {
        cmd.args(["-f", "webm"]);
    }
    // This one exclusive, just-created empty derivative is the only overwrite.
    cmd.arg("-y").arg(&partial);
    let timeout = Duration::from_secs(
        (max_output / (1024 * 1024))
            .saturating_add(60)
            .clamp(60, 7200),
    );
    run_tool(cmd, cancel, timeout, false, Some((&partial, max_output)))?;
    browser_store::validate_merge_generation(job)?;
    let bytes = regular(&partial, max_output)?.len();
    if bytes == 0 {
        return Err(failure("병합 결과가 비어 있습니다."));
    }
    let merged = probe(
        tools,
        &partial,
        &job.recording.mime_type,
        cancel,
        offset + 2.0,
        false,
    )?;
    // Muxers can normalize track order/timebase, so final verification compares
    // codecs and media dimensions separately from the strict input signature.
    if (merged.duration - offset).abs() > 0.5 {
        return Err(failure(
            "병합 결과의 재생 시간이 전체 조각과 일치하지 않습니다.",
        ));
    }
    let input = first_signature.unwrap();
    for index in 0..2 {
        for key in ["codec", "width", "height", "sampleRate", "channels"] {
            if merged.signature[index][key] != input[index][key] {
                return Err(failure("병합 후 영상·음성 형식을 검증하지 못했습니다."));
            }
        }
    }
    OpenOptions::new()
        .write(true)
        .open(&partial)
        .and_then(|file| file.sync_all())
        .map_err(|_| failure("병합 영상을 디스크에 확정하지 못했습니다."))?;
    let verification_guard = browser_store::cleanup::verification_guard(&partial)?;
    verify_full_decode(tools, &partial, offset, cancel)?;
    let cleanup = browser_store::cleanup::write_proof(
        job,
        &source_hashes,
        &partial,
        &timeline_partial,
        cancel,
    )?;
    drop(verification_guard);
    check_cancel(cancel)?;
    browser_store::validate_merge_generation(job)?;
    rename_new(&timeline_partial, &root.join(&timeline_name))?;
    rename_new(&partial, &root.join(&name))?;
    Ok(BrowserMergedOutput {
        file: name,
        timeline_file: timeline_name,
        bytes,
        duration_seconds: merged.duration,
        cleanup: Some(cleanup),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn segment(index: u64, start: Option<f64>, end: Option<f64>, duration: f64) -> BrowserSegment {
        BrowserSegment {
            index,
            file: format!("segment-{index:012}.mp4"),
            bytes: 20,
            duration_seconds: duration,
            source_start_seconds: start,
            source_end_seconds: end,
        }
    }
    #[test]
    fn encoded_clock_uses_cut_points_not_crossing_aac_frame_duration() {
        let segments = vec![
            segment(0, Some(100.0), Some(112.02), 12.02),
            segment(1, Some(112.0), Some(124.0), 12.0),
        ];
        assert_eq!(timeline_duration(&segments, 0, 12.02).unwrap(), 12.0);
        assert_eq!(timeline_duration(&segments, 1, 12.0).unwrap(), 12.0);
        let mut gap = segments;
        gap[1].source_start_seconds = Some(113.0);
        assert!(timeline_duration(&gap, 0, 12.02).is_err());
    }
    #[test]
    fn legacy_uses_media_clock_and_mixed_or_implausible_clocks_fail() {
        let mut segments = vec![segment(0, None, None, 15.01), segment(1, None, None, 15.02)];
        assert_eq!(timeline_duration(&segments, 0, 14.9).unwrap(), 14.9);
        assert!(timeline_duration(&segments, 0, 3.0).is_err());
        segments[1].source_start_seconds = Some(10.0);
        assert!(timeline_duration(&segments, 0, 15.0).is_err());
    }
    #[test]
    fn packet_clock_handles_unknown_container_duration_without_collecting_packets() {
        let mut clock = PacketClock::default();
        clock.line("pts_time=-0.007000|dts_time=N/A|duration_time=0.020000");
        clock.line("pts_time=14.973000|dts_time=14.973000|duration_time=0.020000");
        assert!((clock.duration().unwrap() - 15.0).abs() < 1e-8);
        clock.line("pts_time=nan|dts_time=N/A|duration_time=0.02");
        assert_eq!(clock.count, 2);
    }
    #[test]
    fn missing_or_relative_tools_never_execute() {
        assert_eq!(
            tools_available(None).unwrap_err().code,
            "BROWSER_MERGE_TOOL_MISSING"
        );
        assert!(tools_available(Some(&MediaTools {
            ffmpeg: "ffmpeg.exe".into(),
            ffprobe: "ffprobe.exe".into()
        }))
        .is_err());
    }
    #[test]
    fn cancelled_job_never_creates_derivatives_and_existing_targets_survive() {
        assert_eq!(
            check_cancel(&AtomicBool::new(true)).unwrap_err().code,
            "BROWSER_MERGE_CANCELLED"
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing");
        fs::write(&path, b"original").unwrap();
        assert!(new_file(&path).is_err());
        assert!(rename_new(&dir.path().join("absent"), &path).is_err());
        assert_eq!(fs::read(path).unwrap(), b"original");
    }

    #[test]
    fn process_fixture_child() {
        match std::env::var("ATSUMI_MERGE_TEST_CHILD").as_deref() {
            Ok("wait") => thread::sleep(Duration::from_secs(30)),
            Ok("large") => {
                let block = "x".repeat(8192);
                for _ in 0..64 {
                    println!("{block}");
                }
            }
            _ => {}
        }
    }
    fn child_fixture(mode: &str) -> Command {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "streaming::browser_merge::tests::process_fixture_child",
            "--nocapture",
        ])
        .env("ATSUMI_MERGE_TEST_CHILD", mode);
        cmd
    }
    #[test]
    fn tool_cancellation_kills_waits_and_joins_pipes_without_holding_store() {
        let cancel = Arc::new(AtomicBool::new(false));
        let trigger = cancel.clone();
        let notifier = thread::spawn(move || {
            thread::sleep(Duration::from_millis(250));
            trigger.store(true, Ordering::Release);
        });
        let started = Instant::now();
        let result = run_tool(
            child_fixture("wait"),
            &cancel,
            Duration::from_secs(10),
            false,
            None,
        );
        notifier.join().unwrap();
        assert_eq!(result.err().unwrap().code, "BROWSER_MERGE_CANCELLED");
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    #[test]
    fn oversized_tool_stdout_is_drained_but_never_retained_unbounded() {
        assert!(run_tool(
            child_fixture("large"),
            &AtomicBool::new(false),
            Duration::from_secs(10),
            false,
            None
        )
        .is_err());
    }
    #[test]
    fn missing_tools_background_job_preserves_terminal_video_and_can_be_retried() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let recording = store
            .begin(
                dir.path(),
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "synthetic",
                "video/webm",
            )
            .unwrap();
        let bytes = [0x1a, 0x45, 0xdf, 0xa3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        store.append(&recording.id, 0, 0, &bytes).unwrap();
        store.finish_segment(&recording.id, 0, 15.0).unwrap();
        store.finish(&recording.id, false, None).unwrap();
        let store = Arc::new(Mutex::new(store));
        let worker = BrowserMergeWorker::start(store.clone(), None).unwrap();
        let started = Instant::now();
        loop {
            let recording = store.lock().unwrap().snapshot().unwrap().remove(0);
            if recording
                .merge
                .is_some_and(|m| m.status == BrowserMergeStatus::Blocked)
            {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(worker.retry(Some(&recording.id)).unwrap(), 1);
        worker.shutdown_and_wait();
        let snapshot = store.lock().unwrap().snapshot().unwrap().remove(0);
        assert_eq!(
            snapshot.status,
            super::super::browser_store::BrowserRecordingStatus::Stopped
        );
        assert_eq!(
            fs::read(Path::new(&recording.output_dir).join("segment-000000000000.webm")).unwrap(),
            bytes
        );
        assert!(worker.retry(None).is_err());
    }

    fn e2e_tools() -> MediaTools {
        let bin = PathBuf::from(
            std::env::var_os("ATSUMI_MERGE_TEST_TOOLS_DIR")
                .expect("explicit synthetic-only media-tools directory is required"),
        );
        let tools = MediaTools {
            ffmpeg: bin.join("ffmpeg.exe"),
            ffprobe: bin.join("ffprobe.exe"),
        };
        tools_available(Some(&tools)).unwrap();
        tools
    }
    fn synthetic_webm(tools: &MediaTools) -> Vec<u8> {
        let mut cmd = Command::new(&tools.ffmpeg);
        cmd.args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=160x90:r=25",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "1",
            "-c:v",
            "libvpx",
            "-threads",
            "1",
            "-deadline",
            "realtime",
            "-cpu-used",
            "8",
            "-b:v",
            "200k",
            "-c:a",
            "libopus",
            "-b:a",
            "48k",
            "-live",
            "1",
            "-f",
            "webm",
            "pipe:1",
        ]);
        let ToolOutput::Bytes(bytes) = run_tool(
            cmd,
            &AtomicBool::new(false),
            Duration::from_secs(20),
            false,
            None,
        )
        .unwrap() else {
            unreachable!()
        };
        assert!(bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]));
        bytes
    }
    fn verify_decode(tools: &MediaTools, file: &Path) {
        let mut cmd = Command::new(&tools.ffmpeg);
        cmd.args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-xerror",
            "-protocol_whitelist",
            "file",
            "-i",
        ])
        .arg(file)
        .args(["-map", "0:v:0", "-map", "0:a:0", "-f", "null", "-"]);
        run_tool(
            cmd,
            &AtomicBool::new(false),
            Duration::from_secs(20),
            false,
            None,
        )
        .unwrap();
    }

    fn synthetic_mp4(tools: &MediaTools) -> Vec<u8> {
        let mut cmd = Command::new(&tools.ffmpeg);
        cmd.args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=160x90:r=25:duration=15",
            "-f",
            "lavfi",
            "-i",
            // Supply complete audio coverage beyond the last video frame. A
            // shared output cutoff can truncate AAC before the video's end.
            "sine=frequency=440:sample_rate=48000:duration=15.1",
            "-c:v",
            "libopenh264",
            "-threads",
            "1",
            "-g",
            "25",
            "-b:v",
            "200k",
            "-c:a",
            "aac",
            "-b:a",
            "64k",
            "-movflags",
            "+frag_keyframe+empty_moov+default_base_moof",
            "-f",
            "mp4",
            "pipe:1",
        ]);
        let ToolOutput::Bytes(bytes) = run_tool(
            cmd,
            &AtomicBool::new(false),
            Duration::from_secs(20),
            false,
            None,
        )
        .unwrap() else {
            unreachable!()
        };
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(bytes.windows(4).any(|window| window == b"moof"));
        bytes
    }

    fn fixture_atoms(mut bytes: &[u8]) -> Vec<(&[u8], &[u8])> {
        let mut result = Vec::new();
        while !bytes.is_empty() {
            assert!(bytes.len() >= 8);
            let size = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
            assert!((8..=bytes.len()).contains(&size));
            result.push((&bytes[4..8], &bytes[..size]));
            bytes = &bytes[size..];
        }
        result
    }
    fn native_encoded_fixture(
        media: &[u8],
    ) -> Vec<super::super::browser::encoded::fmp4::EncodedSegment> {
        use super::super::browser::encoded::fmp4::{EncodedMuxer, EncodedTrackInput};
        let mut init = Vec::new();
        let mut fragments = Vec::new();
        for (kind, atom) in fixture_atoms(media) {
            match kind {
                b"ftyp" => init.extend_from_slice(atom),
                b"moov" => {
                    // FFmpeg's encoder-name metadata is not part of the site's
                    // supported init subset. Strip only that synthetic udta box;
                    // compressed samples and timestamps are untouched.
                    let children = fixture_atoms(&atom[8..])
                        .into_iter()
                        .filter(|(kind, _)| *kind != b"udta")
                        .flat_map(|(_, bytes)| bytes.iter().copied())
                        .collect::<Vec<_>>();
                    init.extend_from_slice(&((children.len() + 8) as u32).to_be_bytes());
                    init.extend_from_slice(b"moov");
                    init.extend(children);
                }
                b"moof" | b"mdat" => fragments.push(atom),
                b"mfra" | b"free" => {}
                _ => panic!("unexpected synthetic top-level atom"),
            }
        }
        let mut muxer = EncodedMuxer::new(vec![EncodedTrackInput {
            track_index: 0,
            mime_type: "video/mp4;codecs=avc1.42E01E,mp4a.40.2".into(),
            init,
        }])
        .unwrap();
        let mut segments = Vec::new();
        for fragment in fragments {
            segments.extend(muxer.push(0, fragment).unwrap());
        }
        segments.extend(muxer.finish().unwrap());
        assert_eq!(segments.len(), 2);
        segments
    }

    #[test]
    #[ignore = "explicit verified media tools; temporary encoded-clock AVC/AAC concat and decode"]
    fn synthetic_ffmpeg_fragmented_mp4_uses_source_cut_points_then_cleans_verified_originals() {
        let tools = e2e_tools();
        let media = synthetic_mp4(&tools);
        let segments = native_encoded_fixture(&media);
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let recording = store
            .begin(
                dir.path(),
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "synthetic encoded clock",
                "video/mp4",
            )
            .unwrap();
        for (index, segment) in segments.iter().enumerate() {
            store
                .append(&recording.id, index as u64, 0, &segment.bytes)
                .unwrap();
            store
                .finish_segment_source(
                    &recording.id,
                    index as u64,
                    segment.duration_seconds,
                    Some(segment.source_start_seconds),
                    Some(segment.source_end_seconds),
                )
                .unwrap();
        }
        store.finish(&recording.id, false, None).unwrap();
        let output_root = Path::new(&recording.output_dir);
        let journal = fs::read(output_root.join("segments.jsonl")).unwrap();
        let job = store.take_merge_job().unwrap().unwrap();
        let result = merge_recording(&job, Some(&tools), &AtomicBool::new(false)).unwrap();
        let timeline = fs::read_to_string(output_root.join(&result.timeline_file)).unwrap();
        let rows = timeline
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        let cut = segments[1].source_start_seconds - segments[0].source_start_seconds;
        assert_eq!(rows[0]["mergedDurationSeconds"], cut);
        assert_eq!(rows[1]["mergedStartSeconds"], cut);
        assert_eq!(rows[0]["clock"], "encoded_source");
        verify_decode(&tools, &output_root.join(&result.file));
        store.complete_merge(&job, result).unwrap();
        for (index, segment) in segments.iter().enumerate() {
            assert_eq!(
                fs::read(output_root.join(format!("segment-{index:012}.mp4"))).unwrap(),
                segment.bytes
            );
        }
        assert_eq!(
            fs::read(output_root.join("segments.jsonl")).unwrap(),
            journal
        );
        let replay = store.replay_source(&recording.id).unwrap();
        let cleanup_job = store.take_cleanup_job().unwrap().unwrap();
        let result =
            browser_store::cleanup::remove_verified_sources(&cleanup_job, &AtomicBool::new(false));
        assert!(result.complete);
        assert_eq!(result.deleted, 2);
        store.finish_cleanup(&cleanup_job, result).unwrap();
        for index in 0..2 {
            assert!(!output_root
                .join(format!("segment-{index:012}.mp4"))
                .exists());
        }
        assert!(replay.media.metadata().unwrap().len() > 0);
        let reopened = BrowserCaptureStore::new(dir.path())
            .unwrap()
            .snapshot()
            .unwrap()
            .remove(0);
        assert_eq!(reopened.merge.unwrap().status, BrowserMergeStatus::Complete);
    }

    #[test]
    #[ignore = "explicit verified media tools; generates and merges temporary synthetic media only"]
    fn synthetic_ffmpeg_webm_without_duration_korean_path_single_and_multiple_readonly_sources() {
        let tools = e2e_tools();
        let media = synthetic_webm(&tools);
        let first_cluster = media
            .windows(4)
            .position(|w| w == [0x1f, 0x43, 0xb6, 0x75])
            .unwrap();
        assert!(
            !media[..first_cluster].windows(2).any(|w| w == [0x44, 0x89]),
            "pipe fixture has no EBML Duration element"
        );
        for count in [1u64, 2] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("한국어 녹화 병합 검사");
            fs::create_dir(&root).unwrap();
            let store = BrowserCaptureStore::new(&root).unwrap();
            let recording = store
                .begin(
                    &root,
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "synthetic only",
                    "video/webm;codecs=vp8,opus",
                )
                .unwrap();
            for index in 0..count {
                store.append(&recording.id, index, 0, &media).unwrap();
                store.finish_segment(&recording.id, index, 1.0).unwrap();
            }
            store
                .finish_with_chat(&recording.id, false, None, true, "partial", 2)
                .unwrap();
            let output_root = Path::new(&recording.output_dir);
            fs::write(
                output_root.join("chat.jsonl"),
                b"synthetic chat preserved\n",
            )
            .unwrap();
            let first = output_root.join("segment-000000000000.webm");
            let original_permissions = fs::metadata(&first).unwrap().permissions();
            let mut readonly = original_permissions.clone();
            readonly.set_readonly(true);
            fs::set_permissions(&first, readonly).unwrap();
            let job = store.take_merge_job().unwrap().unwrap();
            let result = merge_recording(&job, Some(&tools), &AtomicBool::new(false));
            // Restore only this temporary fixture's flag so cleanup remains portable.
            fs::set_permissions(&first, original_permissions).unwrap();
            let merged = result.unwrap();
            assert!((merged.duration_seconds - count as f64).abs() < 0.15);
            assert_eq!(fs::read(&first).unwrap(), media);
            assert_eq!(
                fs::read(output_root.join("chat.jsonl")).unwrap(),
                b"synthetic chat preserved\n"
            );
            let timeline = fs::read_to_string(output_root.join(&merged.timeline_file)).unwrap();
            assert_eq!(timeline.lines().count(), count as usize);
            for line in timeline.lines() {
                assert_eq!(
                    serde_json::from_str::<Value>(line).unwrap()["chatRewritten"],
                    false
                );
            }
            let merged_path = output_root.join(&merged.file);
            verify_decode(&tools, &merged_path);
            store.complete_merge(&job, merged).unwrap();
            assert_eq!(store.merged_file(&recording.id).unwrap(), merged_path);
            let cleanup_job = store.take_cleanup_job().unwrap().unwrap();
            let result = browser_store::cleanup::remove_verified_sources(
                &cleanup_job,
                &AtomicBool::new(false),
            );
            assert!(result.complete);
            assert_eq!(result.deleted, count);
            store.finish_cleanup(&cleanup_job, result).unwrap();
            for index in 0..count {
                assert!(!output_root
                    .join(format!("segment-{index:012}.webm"))
                    .exists());
            }
            assert_eq!(
                fs::read(output_root.join("chat.jsonl")).unwrap(),
                b"synthetic chat preserved\n"
            );
            let reopened = BrowserCaptureStore::new(&root)
                .unwrap()
                .snapshot()
                .unwrap()
                .remove(0);
            assert_eq!(reopened.merge.unwrap().status, BrowserMergeStatus::Complete);
            assert_eq!(reopened.chat_status.as_deref(), Some("partial"));
        }
    }

    #[test]
    #[ignore = "explicit verified media tools; corrupts only a middle sample of temporary synthetic media"]
    fn synthetic_ffmpeg_full_decode_rejects_corrupt_middle_with_intact_container_metadata() {
        let tools = e2e_tools();
        let mut media = synthetic_mp4(&tools);
        let mut offset = 0;
        let mut regions = Vec::new();
        for (kind, atom) in fixture_atoms(&media) {
            if kind == b"mdat" {
                regions.push((offset + 8, offset + atom.len()));
            }
            offset += atom.len();
        }
        let (start, end) = regions[regions.len() / 2];
        media[start..end].fill(0xff);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt-middle-only.mp4");
        fs::write(&path, media).unwrap();
        let cancel = AtomicBool::new(false);
        assert!(probe(&tools, &path, "video/mp4", &cancel, 20.0, false).is_ok());
        assert!(verify_full_decode(&tools, &path, 15.0, &cancel).is_err());
        assert!(path.is_file());
    }

    #[test]
    #[ignore = "explicit verified media tools; malformed temporary containers only"]
    fn synthetic_ffmpeg_container_mismatch_and_cancel_never_publish_success() {
        let tools = e2e_tools();
        let dir = tempfile::tempdir().unwrap();
        let store = BrowserCaptureStore::new(dir.path()).unwrap();
        let recording = store
            .begin(
                dir.path(),
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "synthetic malformed",
                "video/mp4",
            )
            .unwrap();
        let bytes = b"\0\0\0\x18ftypisom\0\0\0\0not-a-valid-movie";
        store.append(&recording.id, 0, 0, bytes).unwrap();
        store.finish_segment(&recording.id, 0, 1.0).unwrap();
        store.finish(&recording.id, false, None).unwrap();
        let job = store.take_merge_job().unwrap().unwrap();
        assert!(merge_recording(&job, Some(&tools), &AtomicBool::new(false)).is_err());
        assert_eq!(
            merge_recording(&job, Some(&tools), &AtomicBool::new(true))
                .err()
                .unwrap()
                .code,
            "BROWSER_MERGE_CANCELLED"
        );
        store
            .fail_merge(
                &job,
                BrowserMergeStatus::Failed,
                "synthetic invalid container",
            )
            .unwrap();
        assert!(store.merged_file(&recording.id).is_err());
        assert_eq!(
            fs::read(Path::new(&recording.output_dir).join("segment-000000000000.mp4")).unwrap(),
            bytes
        );
        assert_eq!(
            store.snapshot().unwrap()[0].status,
            super::super::browser_store::BrowserRecordingStatus::Stopped
        );
    }
}
