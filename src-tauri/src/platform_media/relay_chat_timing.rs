//! Fixed numeric timing evidence; never a claim of source-event or archive sync.
use super::*;
use crate::streaming::browser::encoded::fmp4::relay::ts::{TsEnd, TsSegment};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

#[path="relay_chat_event_clock.rs"]
mod event_clock;
pub(crate) use event_clock::EventClock;

const ITEMS:usize=8;
const BYTES:usize=32*1024;
const RECORD_BYTES:usize=4096;
const PROBE_TIMEOUT:Duration=Duration::from_secs(3);

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(crate) struct ClockSample {
    source_time_seconds:f64,
    performance_now_ms:f64,
    timestamp_offset_seconds:f64,
    current_time_seconds:f64,
    playback_rate:f64,
    paused:bool,
    seeking:bool,
    ready_state:u8,
}
impl ClockSample {
    fn valid(&self)->bool {
        self.source_time_seconds.is_finite()&&(0.0..=1e9).contains(&self.source_time_seconds)
            &&self.current_time_seconds.is_finite()&&(0.0..=1e9).contains(&self.current_time_seconds)
            &&self.timestamp_offset_seconds.is_finite()&&self.timestamp_offset_seconds.abs()<=1e9
            &&self.performance_now_ms.is_finite()&&(0.0..=1e12).contains(&self.performance_now_ms)
            &&self.playback_rate.is_finite()&&(0.0625..=16.0).contains(&self.playback_rate)
            &&self.ready_state<=4
            &&(self.current_time_seconds-self.timestamp_offset_seconds-self.source_time_seconds).abs()<=1e-6
    }
    fn value(&self)->Value {json!({"sourceTimeSeconds":self.source_time_seconds,"performanceNowMs":self.performance_now_ms,
        "timestampOffsetSeconds":self.timestamp_offset_seconds,"currentTimeSeconds":self.current_time_seconds,
        "playbackRate":self.playback_rate,"paused":self.paused,"seeking":self.seeking,"readyState":self.ready_state})}
}
struct State {
    binding:Binding,
    sequence:u64,
    pending:Option<(Uuid,Instant)>,
    last_probe:Option<Instant>,
    last_performance:Option<f64>,
    stream:Option<Uuid>,
    segments:u64,
    ended:bool,
    events:event_clock::EventState,
}
#[derive(Clone)]
pub(crate) struct RelayTimingSink {core:Arc<Core>,state:Arc<Mutex<State>>}
impl RelayTimingSink {
    pub(super) fn new(core:Arc<Core>,binding:Binding)->Self {
        Self{core,state:Arc::new(Mutex::new(State{binding,sequence:0,pending:None,last_probe:None,
            last_performance:None,stream:None,segments:0,ended:false,events:event_clock::EventState::default()}))}
    }
    fn ns(&self,at:Instant)->Option<u64>{at.checked_duration_since(self.core.started).and_then(|d|u64::try_from(d.as_nanos()).ok())}
    fn incomplete(&self){self.core.timing_incomplete.store(true,Ordering::Release);}
    fn record(&self,state:&mut State,kind:&str,data:Value)->bool {
        let Some(next)=state.sequence.checked_add(1) else {self.incomplete();return false;};
        let row=json!({"version":1,"sequence":state.sequence,"kind":kind,"binding":state.binding,"data":data});
        let Ok(bytes)=serde_json::to_vec(&row) else {self.incomplete();return false;};
        let mut q=self.core.state.lock().unwrap_or_else(|p|p.into_inner());
        if q.finish||q.sealed{return false;}
        if bytes.len()>RECORD_BYTES||q.timing_items>=ITEMS||q.timing_bytes+bytes.len()>BYTES {
            self.incomplete();return false;
        }
        state.sequence=next;q.timing_items+=1;q.timing_bytes+=bytes.len();
        q.records.push_back(Record{bytes,raw:false,display_control:false,timing:true,event_clock:false,
            receipt:Receipt{sequence:0,monotonic_ns:None}});
        self.core.wake.notify_one();true
    }
    /// Called on the existing manager loop, not an independent timer/thread.
    pub(crate) fn probe(&self,now:Instant)->Option<Uuid> {
        let mut s=self.state.lock().unwrap_or_else(|p|p.into_inner());
        if s.ended||self.core.state.lock().unwrap_or_else(|p|p.into_inner()).finish{return None;}
        if let Some((nonce,at))=s.pending {
            if now.saturating_duration_since(at)<PROBE_TIMEOUT{return None;}
            s.pending=None;self.incomplete();
            self.record(&mut s,"observation_missing",json!({"nonce":nonce,"nativeBeforeNs":self.ns(at),"nativeAfterNs":self.ns(now),"reason":"timeout"}));
        }
        if s.last_probe.is_some_and(|at|now.saturating_duration_since(at)<Duration::from_millis(1500)){return None;}
        let nonce=Uuid::new_v4();s.pending=Some((nonce,now));s.last_probe=Some(now);Some(nonce)
    }
    pub(crate) fn observe(&self,nonce:Uuid,sample:Option<ClockSample>,now:Instant)->bool {
        let mut s=self.state.lock().unwrap_or_else(|p|p.into_inner());
        let Some((wanted,before))=s.pending else{return false;};
        if s.ended||wanted!=nonce{return false;}
        s.pending=None;
        let bounded=now.checked_duration_since(before).is_some_and(|d|d<=PROBE_TIMEOUT)
            &&self.ns(before).is_some()&&self.ns(now).is_some();
        let sample=sample.filter(|v|bounded&&v.valid()&&s.last_performance.is_none_or(|p|v.performance_now_ms>p));
        if let Some(v)=&sample{s.last_performance=Some(v.performance_now_ms);}else{self.incomplete();}
        let usable=sample.as_ref().is_some_and(|v|!v.paused&&!v.seeking&&v.ready_state>=2);
        self.record(&mut s,"source_observation",json!({"nonce":nonce,"nativeBeforeNs":self.ns(before),"nativeAfterNs":self.ns(now),
            "sample":sample.as_ref().map(ClockSample::value),"usable":usable,"quality":"presentation-observation-bracket","archiveMapping":null}))
    }
    /// Only call after delivery returned a fully read successful response.
    pub(crate) fn segment_acked(&self,segment:&TsSegment,before:Instant,after:Instant)->bool {
        let mut s=self.state.lock().unwrap_or_else(|p|p.into_inner());
        if s.ended||segment.ordinal()!=s.segments||s.stream.is_some_and(|v|v!=segment.stream_id())
            ||self.ns(before).is_none()||after.checked_duration_since(before).is_none(){self.incomplete();return false;}
        let recorded=self.record(&mut s,"segment_ack",json!({"streamId":segment.stream_id(),"ordinal":segment.ordinal(),
            "nativeBeforeNs":self.ns(before),"nativeAfterNs":self.ns(after),"bytes":segment.bytes().len(),
            "sha256":format!("{:x}",Sha256::digest(segment.bytes())),"timing":segment.timing(),"archiveMapping":null}));
        s.stream=Some(segment.stream_id());s.segments+=1;recorded
    }
    pub(crate) fn end_acked(&self,end:&TsEnd,before:Instant,after:Instant)->bool {
        let mut s=self.state.lock().unwrap_or_else(|p|p.into_inner());
        if s.ended||s.stream!=Some(end.stream_id())||s.segments!=end.segments()
            ||self.ns(before).is_none()||after.checked_duration_since(before).is_none(){self.incomplete();return false;}
        let o=end.origin();let p=end.preroll();
        if let Some((nonce,at))=s.pending.take() {
            self.incomplete();
            self.record(&mut s,"observation_missing",json!({"nonce":nonce,"nativeBeforeNs":self.ns(at),"nativeAfterNs":self.ns(after),"reason":"input_ended"}));
        }
        let ok=self.record(&mut s,"end_ack",json!({"streamId":end.stream_id(),"segments":end.segments(),
            "nativeBeforeNs":self.ns(before),"nativeAfterNs":self.ns(after),
            "origin":{"ticks":o.ticks,"timescale":o.timescale,"videoBase":o.video_base,"audioBase":o.audio_base,"videoRemainder":o.video_remainder,"audioRemainder":o.audio_remainder},
            "preroll":{"videoSamples":p.video_samples,"audioSamples":p.audio_samples,"bytes":p.bytes},"archiveMapping":null}));
        s.ended=true;s.pending=None;ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn finish(lease:&mut RelayChatLease)->RelayChatSummary {
        lease.begin_finish(false);let deadline=Instant::now()+Duration::from_secs(3);
        loop {if let Some(r)=lease.poll_join().unwrap(){return r;}assert!(Instant::now()<deadline);thread::sleep(Duration::from_millis(1));}
    }
    fn sample(at:f64)->ClockSample {serde_json::from_value(json!({"sourceTimeSeconds":12.25,"performanceNowMs":at,
        "timestampOffsetSeconds":2.0,"currentTimeSeconds":14.25,"playbackRate":1.0,"paused":false,"seeking":false,"readyState":4})).unwrap()}
    fn lines(root:&Path)->Vec<Value>{fs::read_to_string(root.join("relay-timing.jsonl")).unwrap().lines().map(|s|serde_json::from_str(s).unwrap()).collect()}
    #[test]
    fn clock_brackets_share_receipt_origin_and_foreign_duplicate_nonce_cannot_write() {
        let (mut lease,_root)=test_fixture();let t=lease.timing();let at=t.core.started+Duration::from_millis(20);
        let nonce=t.probe(at).unwrap();
        assert!(!t.observe(Uuid::new_v4(),Some(sample(100.0)),at+Duration::from_millis(2)));
        assert!(t.observe(nonce,Some(sample(100.0)),at+Duration::from_millis(3)));
        assert!(!t.observe(nonce,Some(sample(101.0)),at+Duration::from_millis(4)));
        let receipt=Sink(t.core.clone()).raw(b"{\"fixture\":1}",at+Duration::from_millis(1)).unwrap();
        assert_eq!(receipt.monotonic_ns,Some(21_000_000));
        let report=finish(&mut lease);let rows=lines(&report.directory);
        assert_eq!(rows.len(),1);assert_eq!(rows[0]["data"]["nativeBeforeNs"],20_000_000);
        assert_eq!(rows[0]["data"]["nativeAfterNs"],23_000_000);assert_eq!(rows[0]["data"]["usable"],true);
        assert_eq!(report.clock_quality,"receive-monotonic-only");assert_eq!(report.timing_records,1);
        assert!(!t.observe(nonce,Some(sample(102.0)),at+Duration::from_millis(5)));
    }
    #[test]
    fn timeout_nonfinite_and_paused_seek_keep_unknown_or_unusable_instead_of_zero() {
        let (mut lease,_root)=test_fixture();let t=lease.timing();let at=t.core.started;
        let old=t.probe(at).unwrap();let next=t.probe(at+Duration::from_secs(4)).unwrap();
        assert!(!t.observe(old,Some(sample(100.0)),at+Duration::from_secs(4)));
        let mut invalid=sample(101.0);invalid.source_time_seconds=f64::NAN;
        assert!(t.observe(next,Some(invalid),at+Duration::from_millis(4001)));
        let nonce=t.probe(at+Duration::from_secs(6)).unwrap();let mut paused=sample(102.0);paused.paused=true;paused.seeking=true;
        assert!(t.observe(nonce,Some(paused),at+Duration::from_millis(6001)));
        let report=finish(&mut lease);let rows=lines(&report.directory);
        assert_eq!(rows[0]["kind"],"observation_missing");assert!(rows[1]["data"]["sample"].is_null());
        assert_eq!(rows[2]["data"]["usable"],false);assert_eq!(rows[2]["data"]["sample"]["sourceTimeSeconds"],12.25);
        assert!(report.timing_incomplete);
    }
    #[test]
    fn invalid_shape_and_backward_page_clock_are_qualified_without_replacing_chat() {
        assert!(serde_json::from_value::<ClockSample>(json!({"url":"not-allowed"})).is_err());
        let (mut lease,_root)=test_fixture();let t=lease.timing();let at=t.core.started;
        for (n,page) in [(0,200.0),(2,199.0)] {let now=at+Duration::from_secs(n);let nonce=t.probe(now).unwrap();assert!(t.observe(nonce,Some(sample(page)),now+Duration::from_millis(1)));}
        let report=finish(&mut lease);let rows=lines(&report.directory);assert!(rows[1]["data"]["sample"].is_null());
        assert_eq!(report.raw_rows,0);assert_eq!(report.event_rows,0);assert!(report.timing_incomplete);
    }
    #[test]
    fn timing_credit_exhaustion_leaves_chat_capacity_and_normalized_bytes_intact() {
        let (mut lease,_root)=test_fixture();let t=lease.timing();
        // Occupy only timing credits; the writer cannot release these synthetic
        // reservations. No fake record is inserted into durable history.
        {let mut q=t.core.state.lock().unwrap();q.timing_items=ITEMS;q.timing_bytes=BYTES;}
        let at=t.core.started;let nonce=t.probe(at).unwrap();assert!(!t.observe(nonce,Some(sample(1.0)),at+Duration::from_millis(1)));
        let sink=Sink(t.core.clone());let receipt=sink.raw(b"{\"kept\":true}",at).unwrap();
        assert!(sink.event(b"{\"normalized\":true}",receipt));
        {let mut q=t.core.state.lock().unwrap();q.timing_items=0;q.timing_bytes=0;}
        let report=finish(&mut lease);assert_eq!((report.raw_rows,report.event_rows),(1,1));assert!(report.timing_incomplete);assert!(report.failure.is_none());
    }
}
