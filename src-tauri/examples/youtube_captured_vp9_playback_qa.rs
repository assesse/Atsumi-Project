//! Fixed captured VP9/AAC fixture compatibility, not production recording proof.
//! Contract: docs/requirements/youtube-captured-vp9-playback-qa-20261015.json.
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions}, io::{Read, Write}, net::{TcpListener, TcpStream}, path::{Path, PathBuf},
    sync::{atomic::{AtomicBool, AtomicI32, Ordering}, Arc, Mutex}, thread, time::{Duration, Instant},
};
use tauri::{WebviewBuilder, WebviewUrl};

const FIXTURE: &str = ".runtime/youtube-webm-raw-pair-qa/0ab34128-d5cf-43b2-8fad-2d2fe9cc89c2/candidate-common-origin.mp4";
const SHA256: &str = "EF625424D091E5994CBC141E7969A29AF98A0D1243D189D6BB1C452A2648E2EA";
const BYTES: usize = 209643;
const HTML: &str = "<!doctype html><meta charset=utf-8><title>Isolated captured VP9 playback</title><video autoplay muted playsinline src='/captured.mp4' width='854' height='480'></video>";
const FACTS: &str = r#"(()=>{
 const v=document.querySelector('video');if(!v)return null;
 if(!window.qaAudio){
  const a=window.qaAudio={failed:false,context:null,analysers:[],buffers:[]};
  try{
   const c=a.context=new AudioContext(),s=c.createMediaElementSource(v),split=c.createChannelSplitter(2),mute=c.createGain();
   mute.gain.value=0;s.connect(split);mute.connect(c.destination);
   for(let i=0;i<2;i++){const n=c.createAnalyser();n.fftSize=2048;split.connect(n,i);n.connect(mute);a.analysers.push(n);a.buffers.push(new Float32Array(2048));}
   v.muted=false;c.resume().catch(()=>{a.failed=true;});
  }catch{a.failed=true;}
 }
 const a=window.qaAudio,rms=[0,0],peak=[0,0];
 if(!a.failed&&a.context?.state==='running'){
  for(let i=0;i<2;i++){a.analysers[i].getFloatTimeDomainData(a.buffers[i]);let sum=0;
   for(const x of a.buffers[i]){sum+=x*x;peak[i]=Math.max(peak[i],Math.abs(x));}rms[i]=Math.sqrt(sum/2048);}
 }
 return {time:Number.isFinite(v.currentTime)?v.currentTime:null,duration:Number.isFinite(v.duration)?v.duration:null,
  ready:v.readyState,paused:v.paused,ended:v.ended,error:v.error?.code??null,frames:v.getVideoPlaybackQuality?.().totalVideoFrames??null,
  width:v.videoWidth,height:v.videoHeight,audio:{state:a.failed?'failed':a.context?.state??'pending',rms,peak}};
})()"#;
static RESULT: AtomicI32 = AtomicI32::new(2);

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Outcome { EofVerified, DebugOnly, InvalidArguments, FixtureRejected, TransportFailed, PlatformFailed, InvalidFacts, MediaError, WrongDimensions, WrongDuration, MissedStart, PlaybackReset, IncompleteEof, AudioUnverified, Deadline, WindowClosed }
type Result<T> = std::result::Result<T, Outcome>;

fn inspect_handle(file: &File, directory: bool) -> Result<()> {
    let metadata=file.metadata().map_err(|_| Outcome::FixtureRejected)?;
    if metadata.is_dir()!=directory || (!directory && !metadata.is_file()) { return Err(Outcome::FixtureRejected); }
    #[cfg(windows)] {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION}};
        let mut info=BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()),&mut info) }.map_err(|_| Outcome::FixtureRejected)?;
        if info.dwFileAttributes&0x400!=0 || (!directory && info.nNumberOfLinks!=1) { return Err(Outcome::FixtureRejected); }
    }
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        if !directory && metadata.nlink()!=1 { return Err(Outcome::FixtureRejected); }
    }
    Ok(())
}
fn safe_ancestors(path: &Path) -> Result<()> {
    if !path.is_absolute() { return Err(Outcome::FixtureRejected); }
    for ancestor in path.ancestors() {
        let metadata=fs::symlink_metadata(ancestor).map_err(|_| Outcome::FixtureRejected)?;
        if metadata.file_type().is_symlink() { return Err(Outcome::FixtureRejected); }
        #[cfg(windows)] {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes()&0x400!=0 { return Err(Outcome::FixtureRejected); }
        }
    }
    Ok(())
}
fn directory_guard(path: &Path) -> Result<File> {
    safe_ancestors(path)?;
    let mut options=OpenOptions::new(); options.read(true);
    #[cfg(windows)] {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(3).custom_flags(0x02000000|0x00200000);
    }
    let file=options.open(path).map_err(|_| Outcome::FixtureRejected)?;
    inspect_handle(&file,true)?; Ok(file)
}
fn verify_bytes(bytes: &[u8]) -> Result<()> {
    if bytes.len()!=BYTES || format!("{:X}",Sha256::digest(bytes))!=SHA256 { return Err(Outcome::FixtureRejected); }
    Ok(())
}
struct Fixture { bytes: Arc<Vec<u8>>, _file: File, _directories: Vec<File> }
impl Fixture {
    fn open() -> Result<Self> {
        let workspace=PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().ok_or(Outcome::FixtureRejected)?.to_owned();
        let path=workspace.join(FIXTURE); safe_ancestors(&path)?;
        let mut directories=Vec::new();
        for directory in path.parent().ok_or(Outcome::FixtureRejected)?.ancestors() {
            directories.push(directory_guard(directory)?);
            if directory==workspace { break; }
        }
        let mut options=OpenOptions::new(); options.read(true);
        #[cfg(windows)] {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1).custom_flags(0x00200000);
        }
        let mut file=options.open(&path).map_err(|_| Outcome::FixtureRejected)?;
        inspect_handle(&file,false)?;
        if file.metadata().map_err(|_| Outcome::FixtureRejected)?.len()!=BYTES as u64 { return Err(Outcome::FixtureRejected); }
        let mut bytes=Vec::with_capacity(BYTES);
        (&mut file).take(BYTES as u64+1).read_to_end(&mut bytes).map_err(|_| Outcome::FixtureRejected)?;
        verify_bytes(&bytes)?; inspect_handle(&file,false)?;
        Ok(Self {bytes:Arc::new(bytes),_file:file,_directories:directories})
    }
}

fn range(header: &str, length: usize) -> std::result::Result<Option<(usize,usize)>,()> {
    let mut values=header.lines().filter_map(|line|line.split_once(':')).filter(|(key,_)|key.eq_ignore_ascii_case("Range"));
    let Some((_,value))=values.next() else {return Ok(None)};
    if values.next().is_some() || length==0 {return Err(())}
    let (start,end)=value.trim().strip_prefix("bytes=").ok_or(())?.split_once('-').ok_or(())?;
    if start.is_empty() || !start.bytes().all(|b|b.is_ascii_digit()) || !end.bytes().all(|b|b.is_ascii_digit()) {return Err(())}
    let start:usize=start.parse().map_err(|_|())?;
    let end=if end.is_empty(){length-1}else{end.parse::<usize>().map_err(|_|())?.min(length-1)};
    if start>end || start>=length {return Err(())} Ok(Some((start,end)))
}
#[derive(Debug,PartialEq)]
enum Request { Page, Media(Option<(usize,usize)>) }
fn request(header: &str, host: &str) -> std::result::Result<Request,()> {
    if header.len()>8192 || !header.ends_with("\r\n\r\n") || !header.is_ascii() {return Err(())}
    let mut lines=header.split("\r\n"); let first=lines.next().ok_or(())?;
    let mut hosts=0;
    for line in lines {
        if line.is_empty(){continue}
        let (key,value)=line.split_once(':').ok_or(())?;
        if key.is_empty() || key.trim()!=key {return Err(())}
        if key.eq_ignore_ascii_case("Host") {hosts+=1;if value.trim()!=host{return Err(())}}
        if key.eq_ignore_ascii_case("Transfer-Encoding") || key.eq_ignore_ascii_case("Content-Length") {return Err(())}
    }
    if hosts!=1{return Err(())}
    match first {
        "GET / HTTP/1.1" if range(header,BYTES)?.is_none()=>Ok(Request::Page),
        "GET /captured.mp4 HTTP/1.1"=>Ok(Request::Media(range(header,BYTES)?)),
        _=>Err(()),
    }
}
fn write_bounded(socket: &mut TcpStream, bytes: &[u8], deadline: Instant, done: &AtomicBool) -> std::io::Result<()> {
    let mut remaining=bytes;
    while !remaining.is_empty() && !done.load(Ordering::Acquire) && Instant::now()<deadline {
        let amount=remaining.len().min(16384);
        let count=socket.write(&remaining[..amount])?;
        if count==0 {break} remaining=&remaining[count..];
    }
    if remaining.is_empty(){Ok(())}else{Err(std::io::ErrorKind::TimedOut.into())}
}
fn serve(listener: TcpListener, media: Arc<Vec<u8>>, host: String, done: Arc<AtomicBool>, deadline: Instant) {
    let mut count=0; let mut sent=0usize;
    while !done.load(Ordering::Acquire) && Instant::now()<deadline && count<32 {
        let (mut socket,_)=match listener.accept(){Ok(v)=>v,Err(e)if e.kind()==std::io::ErrorKind::WouldBlock=>{thread::sleep(Duration::from_millis(20));continue},Err(_)=>return};
        count+=1;
        let _=socket.set_read_timeout(Some(Duration::from_millis(200))); let _=socket.set_write_timeout(Some(Duration::from_millis(200)));
        let mut bytes=Vec::new(); let mut buffer=[0u8;1024];
        while bytes.len()<8192 && !bytes.windows(4).any(|v|v==b"\r\n\r\n") && !done.load(Ordering::Acquire) && Instant::now()<deadline {
            let length=(8192-bytes.len()).min(buffer.len());
            match socket.read(&mut buffer[..length]) {Ok(0)|Err(_)=>break,Ok(n)=>bytes.extend_from_slice(&buffer[..n])}
        }
        let Ok(header)=std::str::from_utf8(&bytes)else{continue};
        let Ok(request)=request(header,&host)else{continue};
        let (status,mime,body,extra):(&str,&str,&[u8],String)=match request {
            Request::Page=>("200 OK","text/html; charset=utf-8",HTML.as_bytes(),String::new()),
            Request::Media(Some((start,end)))=>("206 Partial Content","video/mp4",&media[start..=end],format!("Content-Range: bytes {start}-{end}/{BYTES}\r\n")),
            Request::Media(None)=>("200 OK","video/mp4",&media,String::new()),
        };
        if sent+body.len()>8*1024*1024 {return} sent+=body.len();
        let response=format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n{extra}Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; media-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'\r\nConnection: close\r\n\r\n",body.len());
        if write_bounded(&mut socket,response.as_bytes(),deadline,&done).is_ok(){let _=write_bounded(&mut socket,body,deadline,&done);}
    }
}

fn nullable<'de,D,T>(d:D)->std::result::Result<Option<T>,D::Error> where D:Deserializer<'de>,T:Deserialize<'de>{Option::<T>::deserialize(d)}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all="lowercase")]
enum AudioState { Pending, Suspended, Running, Closed, Interrupted, Failed }
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AudioFacts { state:AudioState, rms:[f64;2], peak:[f64;2] }
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Facts {
    #[serde(deserialize_with="nullable")] time:Option<f64>,
    #[serde(deserialize_with="nullable")] duration:Option<f64>,
    ready:u8, paused:bool, ended:bool,
    #[serde(deserialize_with="nullable")] error:Option<u8>,
    #[serde(deserialize_with="nullable")] frames:Option<u64>, width:u32, height:u32, audio:AudioFacts,
}
fn parse_facts(raw:&str)->Result<Option<Facts>> {
    if raw.len()>2048 {return Err(Outcome::InvalidFacts)}
    if raw=="null" {return Ok(None)}
    let facts:Facts=serde_json::from_str(raw).map_err(|_|Outcome::InvalidFacts)?;
    if facts.time.is_some_and(|v|!v.is_finite()||!(0.0..=60.0).contains(&v))
        || facts.duration.is_some_and(|v|!v.is_finite()||!(0.0..=60.0).contains(&v))
        || facts.ready>4 || facts.error.is_some_and(|v|!(1..=4).contains(&v))
        || facts.frames.is_some_and(|v|v>10000) || facts.width>16384 || facts.height>16384
        || facts.audio.rms.iter().chain(facts.audio.peak.iter()).any(|v|!v.is_finite()||!(0.0..=4.0).contains(v))
        || (0..2).any(|i|facts.audio.rms[i]>facts.audio.peak[i]+1e-12) {return Err(Outcome::InvalidFacts)}
    Ok(Some(facts))
}
#[derive(Clone, Default)]
struct Progress { samples:u16, first:Option<(f64,u64)>, last:Option<(f64,u64)>, facts:Option<Facts>, audio_samples:u16, audio_first:Option<f64>, audio_last:Option<f64> }
impl Progress {
    fn audio_verified(&self)->bool {self.audio_samples>=3 && self.audio_first.zip(self.audio_last).is_some_and(|(first,last)|last-first>=0.5)}
    fn observe(&mut self,facts:Facts)->Result<bool> {
        self.samples+=1; self.facts=Some(facts.clone());
        if facts.error.is_some(){return Err(Outcome::MediaError)}
        if (facts.width!=0 && facts.width!=854)||(facts.height!=0 && facts.height!=480){return Err(Outcome::WrongDimensions)}
        if facts.duration.is_some_and(|duration|!(5.010..=5.020).contains(&duration)){return Err(Outcome::WrongDuration)}
        if let (Some(time),Some(frames))=(facts.time,facts.frames) {
            if self.last.is_some_and(|(previous,count)|time+0.001<previous||frames<count){return Err(Outcome::PlaybackReset)}
            self.last=Some((time,frames));
            if !facts.paused && facts.ready>=2 && facts.audio.state==AudioState::Running
                && (0..2).any(|i|facts.audio.rms[i]>1e-6&&facts.audio.peak[i]>1e-5)
                && self.audio_last.is_none_or(|previous|time>previous) {
                self.audio_samples+=1;self.audio_first.get_or_insert(time);self.audio_last=Some(time);
            }
            if !facts.paused && facts.ready>=2 && facts.width==854 && facts.height==480 && facts.duration.is_some() && self.first.is_none(){
                if time>1.0{return Err(Outcome::MissedStart)} self.first=Some((time,frames));
            }
            if facts.ended {
                let complete=self.first.is_some_and(|(first,count)|time-first>=4.0 && frames>=count+100)
                    && facts.duration.is_some_and(|duration|(time-duration).abs()<=0.050)
                    && facts.width==854 && facts.height==480 && facts.ready>=2;
                return if !complete{Err(Outcome::IncompleteEof)}else if !self.audio_verified(){Err(Outcome::AudioUnverified)}else{Ok(true)};
            }
        } else if facts.ended {return Err(Outcome::IncompleteEof)}
        Ok(false)
    }
}
fn summary(outcome:Outcome,progress:&Progress,hash_matched:bool)->serde_json::Value {
    let delta=progress.first.zip(progress.last).map(|((t,f),(last,count))|((last-t).max(0.0),count.saturating_sub(f)));
    json!({"kind":"youtube_captured_vp9_playback_qa","outcome":outcome,"playbackVerified":outcome==Outcome::EofVerified,
        "isolated":true,"synthetic":false,"fileHashMatched":hash_matched,"productionRecordingVerified":false,
        "libraryFlowVerified":false,"nativeAudioDecodeVerified":outcome==Outcome::EofVerified&&progress.audio_verified(),
        "audiblePlaybackVerified":false,"audioSignalObservations":progress.audio_samples,"samples":progress.samples,
        "timeProgressSeconds":delta.map(|value|value.0),"frameProgress":delta.map(|value|value.1),"facts":progress.facts})
}
fn finish(app:&tauri::AppHandle,done:&AtomicBool,progress:&Mutex<Progress>,outcome:Outcome) {
    if done.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return}
    let code=if outcome==Outcome::EofVerified{0}else{2}; RESULT.store(code,Ordering::Release);
    let progress=progress.lock().unwrap_or_else(|e|e.into_inner()).clone();
    println!("{}",summary(outcome,&progress,true)); app.exit(code);
}
fn run()->Result<()> {
    let deadline=Instant::now()+Duration::from_secs(30);
    let fixture=Fixture::open()?;
    let profile=tempfile::Builder::new().prefix("atsumi-captured-vp9-").tempdir().map_err(|_|Outcome::PlatformFailed)?;
    let listener=TcpListener::bind(("127.0.0.1",0)).map_err(|_|Outcome::TransportFailed)?;
    listener.set_nonblocking(true).map_err(|_|Outcome::TransportFailed)?;
    let host=format!("127.0.0.1:{}",listener.local_addr().map_err(|_|Outcome::TransportFailed)?.port());
    let origin=format!("http://{host}/"); let navigation=origin.clone();
    let done=Arc::new(AtomicBool::new(false)); let worker_done=done.clone();
    let server_done=done.clone(); let media=fixture.bytes.clone();
    let server=thread::spawn(move||serve(listener,media,host,server_done,deadline));
    let progress=Arc::new(Mutex::new(Progress::default())); let final_progress=progress.clone();
    let profile_path=profile.path().join("webview");
    let mut context=tauri::generate_context!();
    context.config_mut().identifier=format!("local.atsumi.capturedvp9qa.{}",uuid::Uuid::new_v4().simple());
    context.config_mut().app.windows.clear(); context.config_mut().app.tray_icon=None;
    let app=tauri::Builder::default().setup(move|app|{
        let window=tauri::window::WindowBuilder::new(app,"captured-vp9").title("Isolated captured VP9 playback QA")
            .inner_size(854.0,500.0).position(-16000.0,-16000.0).focused(false).skip_taskbar(true).build()?;
        let view=window.add_child(WebviewBuilder::new("captured-video",WebviewUrl::External(origin.parse()?))
            .data_directory(profile_path).focused(false)
            .additional_browser_args("--autoplay-policy=no-user-gesture-required --mute-audio --no-first-run --disable-background-networking --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows")
            .on_navigation(move|url|url.as_str()==navigation).on_new_window(|_,_|tauri::webview::NewWindowResponse::Deny).on_download(|_,_|false),
            tauri::LogicalPosition::new(0.0,0.0),tauri::LogicalSize::new(854.0,500.0))?;
        let handle=app.handle().clone(); let timeout_handle=handle.clone();
        let timeout_done=worker_done.clone(); let timeout_progress=progress.clone();
        thread::spawn(move||{thread::sleep(deadline.saturating_duration_since(Instant::now()));finish(&timeout_handle,&timeout_done,&timeout_progress,Outcome::Deadline);});
        thread::spawn(move||{
            let mut result=Outcome::Deadline;
            for _ in 0..100 {
                if worker_done.load(Ordering::Acquire)||Instant::now()>=deadline{break}
                let (tx,rx)=std::sync::mpsc::sync_channel(1);
                if view.eval_with_callback(FACTS,move|raw|{let _=tx.send((raw.len()<=2048).then_some(raw));}).is_err(){result=Outcome::PlatformFailed;break}
                let wait=deadline.saturating_duration_since(Instant::now()).min(Duration::from_secs(1));
                let raw=match rx.recv_timeout(wait){Ok(Some(raw))=>raw,Ok(None)=>{result=Outcome::InvalidFacts;break},Err(_)=>{result=Outcome::PlatformFailed;break}};
                if worker_done.load(Ordering::Acquire)||Instant::now()>=deadline{break}
                match parse_facts(&raw){
                    Ok(Some(facts))=>match progress.lock().unwrap_or_else(|e|e.into_inner()).observe(facts){Ok(true)=>{result=Outcome::EofVerified;break},Err(error)=>{result=error;break},Ok(false)=>()},
                    Ok(None)=>(),Err(error)=>{result=error;break},
                }
                thread::sleep(deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(250)));
            }
            finish(&handle,&worker_done,&progress,result);
        }); Ok(())
    }).build(context);
    let app=match app{Ok(app)=>app,Err(_)=>{done.store(true,Ordering::Release);let _=server.join();return Err(Outcome::PlatformFailed)}};
    app.run_return(|_,_|{});
    if done.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_ok(){
        println!("{}",summary(Outcome::WindowClosed,&final_progress.lock().unwrap_or_else(|e|e.into_inner()),true));
    }
    let _=server.join(); drop(profile); drop(fixture); Ok(())
}
fn main(){
    let result=if !cfg!(debug_assertions){Err(Outcome::DebugOnly)}else if std::env::args_os().len()!=1{Err(Outcome::InvalidArguments)}else{run()};
    if let Err(error)=result{println!("{}",summary(error,&Progress::default(),false));std::process::exit(2)}
    std::process::exit(RESULT.load(Ordering::Acquire));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn facts(time:f64,frames:u64,ended:bool)->Facts {Facts{time:Some(time),duration:Some(5.015511),ready:4,paused:ended,ended,error:None,frames:Some(frames),width:854,height:480,audio:AudioFacts{state:AudioState::Running,rms:[0.1,0.1],peak:[0.2,0.2]}}}
    #[test]
    fn fixed_media_requires_exact_size_hash_and_regular_single_link_handle(){
        assert!(verify_bytes(&vec![0u8;BYTES]).is_err()); assert!(verify_bytes(&vec![0u8;BYTES-1]).is_err());
        let directory=tempfile::tempdir().unwrap(); let path=directory.path().join("fixture");
        let mut file=OpenOptions::new().create_new(true).write(true).open(&path).unwrap();file.write_all(b"unit").unwrap();drop(file);
        let file=File::open(&path).unwrap();assert!(inspect_handle(&file,false).is_ok());drop(file);
        fs::hard_link(&path,directory.path().join("linked")).unwrap();let file=File::open(&path).unwrap();assert!(inspect_handle(&file,false).is_err());
        assert!(safe_ancestors(Path::new("relative")).is_err());
    }
    #[test]
    fn requests_are_fixed_local_gets_with_single_bounded_ranges(){
        let host="127.0.0.1:12345";
        assert_eq!(request("GET / HTTP/1.1\r\nHost: 127.0.0.1:12345\r\n\r\n",host),Ok(Request::Page));
        assert_eq!(request("GET /captured.mp4 HTTP/1.1\r\nHost: 127.0.0.1:12345\r\nRange: bytes=1-3\r\n\r\n",host),Ok(Request::Media(Some((1,3)))));
        for invalid in ["bytes=-4","bytes=9-3","bytes=209643-","bytes=0-1,3-4","bytes=+1-3"]{assert!(range(&format!("Range: {invalid}\r\n"),BYTES).is_err());}
        for header in ["GET /captured.mp4 HTTP/1.1\r\nHost: other\r\n\r\n","GET /../captured.mp4 HTTP/1.1\r\nHost: 127.0.0.1:12345\r\n\r\n","GET / HTTP/1.1\r\nHost: 127.0.0.1:12345\r\nHost: 127.0.0.1:12345\r\n\r\n","POST / HTTP/1.1\r\nHost: 127.0.0.1:12345\r\n\r\n"]{assert!(request(header,host).is_err());}
        assert!(range("Range: bytes=0-1\r\nrange: bytes=2-3\r\n",BYTES).is_err());
    }
    #[test]
    fn scalar_facts_reject_missing_nullable_extra_fields_bad_numbers_and_oversize(){
        let valid=serde_json::to_value(facts(0.1,3,false)).unwrap();assert!(parse_facts(&valid.to_string()).unwrap().is_some());
        for (key,value) in [("url",json!("private")),("time",json!(-1)),("duration",json!(61)),("ready",json!(5)),("frames",json!(10001)),("error",json!(0))]{let mut bad=valid.clone();bad[key]=value;assert!(parse_facts(&bad.to_string()).is_err());}
        let mut missing=valid.clone();missing.as_object_mut().unwrap().remove("error");assert!(parse_facts(&missing.to_string()).is_err());
        assert!(parse_facts(&" ".repeat(2049)).is_err());assert!(parse_facts("null").unwrap().is_none());
    }
    #[test]
    fn progressing_video_is_not_a_pass_until_real_eof(){
        let mut p=Progress::default();assert!(!p.observe(facts(0.1,3,false)).unwrap());
        assert!(!p.observe(facts(2.0,60,false)).unwrap());
        assert!(!p.observe(facts(4.8,145,false)).unwrap());assert!(p.observe(facts(5.015511,150,true)).unwrap());
        for mut bad in [facts(5.015511,150,true),facts(0.1,3,false)]{
            let mut p=Progress::default();bad.error=Some(3);assert_eq!(p.observe(bad).unwrap_err(),Outcome::MediaError);
        }
        let mut p=Progress::default();assert_eq!(p.observe(facts(5.015511,150,true)).unwrap_err(),Outcome::IncompleteEof);
    }
    #[test]
    fn reset_wrong_shape_duration_and_missed_start_fail(){
        let mut p=Progress::default();p.observe(facts(0.1,3,false)).unwrap();p.observe(facts(2.0,60,false)).unwrap();
        assert_eq!(p.observe(facts(1.0,60,false)).unwrap_err(),Outcome::PlaybackReset);
        let mut bad=facts(0.1,3,false);bad.width=1280;assert_eq!(Progress::default().observe(bad).unwrap_err(),Outcome::WrongDimensions);
        let mut bad=facts(0.1,3,false);bad.duration=Some(10.0);assert_eq!(Progress::default().observe(bad).unwrap_err(),Outcome::WrongDuration);
        assert_eq!(Progress::default().observe(facts(1.1,33,false)).unwrap_err(),Outcome::MissedStart);
    }
    #[test]
    fn summary_is_fixed_and_qualifies_recording_library_and_zero_gain_audio(){
        let mut progress=Progress::default();for (time,frames) in [(0.1,3),(2.0,60),(4.8,145)]{progress.observe(facts(time,frames,false)).unwrap();}progress.observe(facts(5.015511,150,true)).unwrap();
        let value=summary(Outcome::EofVerified,&progress,true);assert_eq!(value["playbackVerified"],true);assert_eq!(value["synthetic"],false);
        assert_eq!(value["nativeAudioDecodeVerified"],true);
        for key in ["productionRecordingVerified","libraryFlowVerified","audiblePlaybackVerified"]{assert_eq!(value[key],false);}
        let text=value.to_string();for private in ["http","cookie","profile","captureId","candidate-common-origin",FIXTURE]{assert!(!text.contains(private));}assert!(text.len()<2048);
    }
    #[test]
    fn silent_suspended_and_short_signal_cannot_claim_native_audio_decode(){
        for variant in 0..3{
            let mut progress=Progress::default();
            for (time,frames) in [(0.1,3),(2.0,60),(4.8,145)]{
                let mut f=facts(time,frames,false);
                match variant{0=>{f.audio.rms=[0.0;2];f.audio.peak=[0.0;2];},1=>f.audio.state=AudioState::Suspended,_=>if time<4.0{f.audio.state=AudioState::Failed}}
                progress.observe(f).unwrap();
            }
            assert_eq!(progress.observe(facts(5.015511,150,true)).unwrap_err(),Outcome::AudioUnverified);
            assert_eq!(summary(Outcome::AudioUnverified,&progress,true)["nativeAudioDecodeVerified"],false);
        }
        for audio in [json!({"state":"unknown","rms":[0,0],"peak":[0,0]}),json!({"state":"running","rms":[1,0],"peak":[0,0]}),json!({"state":"running","rms":[0,0],"peak":[0,5]}),json!({"state":"running","rms":[0,0],"peak":[0,0],"pcm":[]})]{let mut bad=serde_json::to_value(facts(0.1,3,false)).unwrap();bad["audio"]=audio;assert!(parse_facts(&bad.to_string()).is_err());}
    }
}
