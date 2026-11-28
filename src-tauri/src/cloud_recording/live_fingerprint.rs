//! Lightweight references from acknowledged native TS, never an archive clock.
//! Delivery only tries a bounded queue; decoding and persistence are off-lane.
#[path = "live_fingerprint_decode.rs"]
mod decode;
#[path = "live_fingerprint_process.rs"]
mod process;
use crate::streaming::{browser::encoded::fmp4::relay::ts::{TsSegment, TsTiming}, browser_merge::MediaTools};
use serde::{Deserialize, Serialize};
use std::{fs::{self, OpenOptions}, io::{Read, Write}, path::{Path, PathBuf}, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}, mpsc::{self, SyncSender, TrySendError}}, thread::{self, JoinHandle}, time::Duration};
use uuid::Uuid;

pub(crate) const FILE_NAME: &str = "relay-frame-reference.json";
const MAX_FILE: usize = 256 * 1024;
const QUEUE: usize = 1;
const MAX_WINDOWS: usize = 16;
pub(crate) const WIDTH: usize = 32;
pub(crate) const HEIGHT: usize = 18;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct FrameFeature { pub gradient: [u32; 4], pub mean: u8, pub spread: u8 }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct FrameReference { pub pts90k: u64, pub feature: FrameFeature }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct ReferenceWindow {
    pub ordinal: u64, pub segment_sha256: String, pub source_origin_ticks: u64,
    pub source_origin_timescale: u32, pub source_video_timescale: u32,
    pub frames: Vec<FrameReference>,
}
impl ReferenceWindow {
    fn center(&self) -> u64 { self.frames.first().zip(self.frames.last()).map(|(a,b)| a.pts90k+(b.pts90k-a.pts90k)/2).unwrap_or(0) }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct ReferenceSet {
    pub version: u8, pub run_id: Uuid, pub source_id: Uuid, pub stream_id: Option<Uuid>,
    pub windows: Vec<ReferenceWindow>, pub acknowledged_segments: u64, pub sampled_segments: u64,
    pub queue_dropped: u64, pub decode_failures: u64, pub static_windows: u64,
    pub cancelled: bool, pub incomplete: bool, pub remote_anchor: String,
}
pub(super) struct Segment { stream: Uuid, ordinal: u64, bytes: Arc<[u8]>, timing: TsTiming }
struct Schedule { elapsed: u64, next: u64, stream: Option<Uuid>, ordinal: u64 }
struct Shared {
    closed: AtomicBool, cancel: Arc<AtomicBool>, dropped: AtomicU64, acks: AtomicU64,
    schedule: Mutex<Schedule>,
}
#[derive(Clone)]
pub(crate) struct AckSink { sender: SyncSender<Segment>, shared: Arc<Shared> }
impl AckSink {
    /// Must be called only after the complete segment response succeeded.
    /// No wait, decode, hashing, filesystem access or media-byte copy occurs here.
    pub(crate) fn observe(&self, segment: &TsSegment) {
        if self.shared.closed.load(Ordering::Acquire) { return; }
        let Ok(mut schedule) = self.shared.schedule.try_lock() else { self.shared.dropped.fetch_add(1, Ordering::Relaxed); return; };
        if schedule.stream.is_some_and(|id| id != segment.stream_id()) || segment.ordinal() != schedule.ordinal {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed); return;
        }
        schedule.stream = Some(segment.stream_id()); schedule.ordinal += 1;
        self.shared.acks.store(schedule.ordinal, Ordering::Release);
        let Some(end) = schedule.elapsed.checked_add(segment.duration_ticks()) else { self.shared.dropped.fetch_add(1, Ordering::Relaxed); return; };
        schedule.elapsed = end;
        if end < schedule.next { return; }
        // Early short recordings receive several candidates. Long streams
        // sample at most once/minute, retaining only compact reference windows.
        let spacing = (end / 12).clamp(5*90_000, 60*90_000);
        schedule.next = end.saturating_add(spacing);
        let candidate = Segment { stream:segment.stream_id(), ordinal:segment.ordinal(), bytes:segment.bytes_arc(), timing:segment.timing() };
        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) = self.sender.try_send(candidate) { self.shared.dropped.fetch_add(1, Ordering::Relaxed); }
    }
}

pub(crate) struct FingerprintWorker {
    shared: Arc<Shared>, sink: AckSink, handle: Option<JoinHandle<ReferenceSet>>,
    // Joining consumes the thread handle, not its evidence. A failed durable
    // publication must remain retryable with the same run/source/stream proof.
    result: Option<ReferenceSet>, published_root: Option<PathBuf>,
}
impl FingerprintWorker {
    /// Tools come from the existing managed startup resolver, never PATH or IPC.
    pub(crate) fn start(tools: MediaTools, run_id: Uuid, source_id: Uuid) -> Result<Self, String> {
        if run_id.is_nil() || source_id.is_nil() || !tools.ffmpeg.is_absolute() || !tools.ffmpeg.is_file() { return Err("frame_reference_tools".into()); }
        let (sender, receiver) = mpsc::sync_channel::<Segment>(QUEUE);
        let shared = Arc::new(Shared { closed:AtomicBool::new(false), cancel:Arc::new(AtomicBool::new(false)), dropped:AtomicU64::new(0), acks:AtomicU64::new(0), schedule:Mutex::new(Schedule {elapsed:0,next:0,stream:None,ordinal:0}) });
        let worker = shared.clone();
        let handle = thread::Builder::new().name("live-frame-reference".into()).spawn(move || {
            let mut result = ReferenceSet {version:1,run_id,source_id,stream_id:None,windows:Vec::new(),acknowledged_segments:0,sampled_segments:0,queue_dropped:0,decode_failures:0,static_windows:0,cancelled:false,incomplete:true,remote_anchor:"unknown".into()};
            loop {
                if worker.cancel.load(Ordering::Acquire) { result.cancelled=true; break; }
                let segment = match receiver.recv_timeout(Duration::from_millis(30)) {
                    Ok(value) => value,
                    Err(mpsc::RecvTimeoutError::Timeout) if !worker.closed.load(Ordering::Acquire) => continue,
                    Err(_) => break,
                };
                result.stream_id = Some(segment.stream); result.sampled_segments += 1;
                match decode::window(&tools.ffmpeg, &segment, &worker.cancel) {
                    Ok(window) if dynamic(&window) => retain(&mut result.windows,window),
                    Ok(_) => result.static_windows += 1,
                    Err(_) => result.decode_failures += 1,
                }
            }
            result.windows = choose(&result.windows);
            result.acknowledged_segments = worker.acks.load(Ordering::Acquire);
            result.queue_dropped = worker.dropped.load(Ordering::Acquire);
            result.cancelled |= worker.cancel.load(Ordering::Acquire);
            result.incomplete = result.cancelled || result.queue_dropped>0 || result.decode_failures>0 || result.windows.len()<3;
            result
        }).map_err(|_| "frame_reference_worker")?;
        Ok(Self {sink:AckSink {sender,shared:shared.clone()},shared,handle:Some(handle),result:None,published_root:None})
    }
    pub(crate) fn sink(&self) -> AckSink { self.sink.clone() }
    pub(crate) fn begin_finish(&self, cancelled: bool) {
        if cancelled { self.shared.cancel.store(true,Ordering::Release); }
        self.shared.closed.store(true,Ordering::Release);
    }
    /// Poll from the existing manager's joined-drain loop, before sidecar seal.
    /// None means the owned decoder is still running; it is never detached.
    pub(crate) fn poll_finish_into(&mut self, root: &Path) -> Result<Option<ReferenceSet>, String> {
        if !self.shared.closed.load(Ordering::Acquire) { return Err("frame_reference_not_finishing".into()); }
        if self.published_root.as_ref().is_some_and(|published|published != root) { return Err("frame_reference_root_changed".into()); }
        if self.result.is_none() {
            if self.handle.is_none() { return Err("frame_reference_finish_failed".into()); }
            if !self.handle.as_ref().is_some_and(|handle|handle.is_finished()) { return Ok(None); }
            self.result = Some(self.handle.take().ok_or("frame_reference_join")?.join().map_err(|_| "frame_reference_panic")?);
        }
        let result = self.result.as_ref().ok_or("frame_reference_finish_failed")?;
        if self.published_root.is_none() {
            persist(root,result)?;
            self.published_root = Some(root.to_owned());
        }
        Ok(Some(result.clone()))
    }
}
impl Drop for FingerprintWorker {
    fn drop(&mut self) {
        self.begin_finish(true);
        // The contained decoder observes cancel every10ms and closes pipes
        // before joining. No media child or Rust worker survives its owner.
        if let Some(handle) = self.handle.take() { let _ = handle.join(); }
    }
}
// This path names only a newly create_new-owned compact reference. It never
// points to existing media, another pending generation, or a caller path.
struct Pending(PathBuf);
impl Drop for Pending { fn drop(&mut self) { let _ = fs::remove_file(&self.0); } }
fn publish_new(from:&Path,to:&Path)->Result<(),String> {
    #[cfg(windows)] {
        use std::os::windows::ffi::OsStrExt;
        use windows::{core::PCWSTR, Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH}};
        let from:Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to:Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        // Do not set REPLACE_EXISTING: even a prior failed run is immutable.
        unsafe { MoveFileExW(PCWSTR(from.as_ptr()),PCWSTR(to.as_ptr()),MOVEFILE_WRITE_THROUGH) }
            .map_err(|_|"frame_reference_publish".into())
    }
    #[cfg(not(windows))] {
        // The owned pending link is removed before final-file verification.
        fs::hard_link(from,to).map_err(|_|"frame_reference_publish".into())
    }
}
fn same_published(path:&Path,bytes:&[u8])->Result<(),String> {
    let mut file = crate::streaming::browser_store::cleanup::verification_guard(path).map_err(|_|"frame_reference_existing")?;
    if file.metadata().map_err(|_|"frame_reference_existing")?.len() != bytes.len() as u64 { return Err("frame_reference_conflict".into()); }
    let mut actual = Vec::with_capacity(bytes.len());
    (&mut file).take((MAX_FILE+1) as u64).read_to_end(&mut actual).map_err(|_|"frame_reference_existing")?;
    if actual != bytes { return Err("frame_reference_conflict".into()); }
    Ok(())
}
fn persist(root:&Path,result:&ReferenceSet)->Result<(),String> {
    if !root.is_absolute() || fs::canonicalize(root).map_err(|_|"frame_reference_root")? != root { return Err("frame_reference_root".into()); }
    let _guard = crate::streaming::browser_store::cleanup::pin_directories(root).map_err(|_|"frame_reference_root")?;
    let bytes=serde_json::to_vec(result).map_err(|_|"frame_reference_encode")?;
    if bytes.len()>MAX_FILE { return Err("frame_reference_size".into()); }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)] {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let path = root.join(format!(".relay-frame-reference-{}.pending",Uuid::new_v4()));
    let mut file = options.open(&path).map_err(|_|"frame_reference_create")?;
    let pending = Pending(path);
    let written = file.write_all(&bytes).and_then(|_|file.sync_all());
    drop(file);
    written.map_err(|_|"frame_reference_write")?;
    let published = publish_new(&pending.0,&root.join(FILE_NAME));
    drop(pending);
    match published {
        Ok(()) => Ok(()),
        // A retry after an uncertain publish may accept only identical bytes,
        // including all native ownership and exact source/transport fields.
        Err(_) if root.join(FILE_NAME).exists() => same_published(&root.join(FILE_NAME),&bytes),
        Err(error) => Err(error),
    }
}
fn dynamic(window:&ReferenceWindow)->bool {
    if window.frames.len()<8 { return false; }
    let mut distinct=Vec::new(); let mut transitions=0;
    for frame in &window.frames { if !distinct.contains(&frame.feature.gradient) { distinct.push(frame.feature.gradient); } }
    for pair in window.frames.windows(2) { transitions += pair[0].feature.gradient.iter().zip(&pair[1].feature.gradient).map(|(a,b)|(a^b).count_ones()).sum::<u32>(); }
    distinct.len()>=4 && transitions>=16 && window.frames.iter().any(|f|f.feature.spread>=8)
}
fn retain(windows:&mut Vec<ReferenceWindow>,window:ReferenceWindow) {
    if windows.iter().any(|old| old.frames.iter().map(|f|&f.feature).eq(window.frames.iter().map(|f|&f.feature))) { return; }
    windows.push(window);
    if windows.len()>MAX_WINDOWS {
        // Preserve beginning and current tail. Remove the most redundant
        // interior temporal point; this is bounded, not a growing frame log.
        let index=(1..windows.len()-1).min_by_key(|&i|windows[i+1].center()-windows[i-1].center()).unwrap();
        windows.remove(index);
    }
}
fn choose(windows:&[ReferenceWindow])->Vec<ReferenceWindow> {
    if windows.len()<=3 { return windows.to_vec(); }
    let first=&windows[0]; let last=&windows[windows.len()-1]; let midpoint=first.center()+(last.center()-first.center())/2;
    let middle=windows[1..windows.len()-1].iter().min_by_key(|w|w.center().abs_diff(midpoint)).unwrap();
    vec![first.clone(),middle.clone(),last.clone()]
}
pub(crate) fn feature(pixels:&[u8])->Result<FrameFeature,String> {
    if pixels.len()!=WIDTH*HEIGHT { return Err("frame_reference_pixels".into()); }
    let average=|column:usize,row:usize,columns:usize,rows:usize|->u32 {
        let (x0,x1,y0,y1)=(column*WIDTH/columns,(column+1)*WIDTH/columns,row*HEIGHT/rows,(row+1)*HEIGHT/rows);
        let mut sum=0u32; for y in y0..y1 { for x in x0..x1 { sum+=u32::from(pixels[y*WIDTH+x]); } }
        sum/((x1-x0)*(y1-y0)) as u32
    };
    let mut gradient=[0u32;4];
    for y in 0..8 { for x in 0..8 { let bit=y*8+x;
        if average(x,y,9,8)>average(x+1,y,9,8) {gradient[bit/32]|=1<<(bit%32);}
        if average(x,y,8,9)>average(x,y+1,8,9) {gradient[2+bit/32]|=1<<(bit%32);}
    } }
    let mean=(pixels.iter().map(|p|u32::from(*p)).sum::<u32>()/pixels.len() as u32) as u8;
    let spread=(pixels.iter().map(|p|u32::from(p.abs_diff(mean))).sum::<u32>()/pixels.len() as u32) as u8;
    Ok(FrameFeature {gradient,mean,spread})
}
#[cfg(test)]
#[path = "live_fingerprint_tests.rs"]
mod tests;
