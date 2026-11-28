//! Optional SOOP public-event evidence. Does not write or replace chat rows.
use super::*;

const EVENT_ITEMS:usize=64;
const EVENT_BYTES:usize=64*1024;
const EVENT_ROW_BYTES:usize=1536;
const EVENT_RATE:u16=120;

#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(crate) struct EventClock {
    version:u8,run_id:String,source_id:String,document_epoch:u64,sequence:u64,
    service:u16,message_id:String,sample:ClockSample,live_visible:Option<bool>,
    phase:String,presentation_uncertainty:String,
}
#[derive(Default)]
pub(super) struct EventState {
    sequence:Option<u64>,performance:Option<f64>,received:Option<u64>,
    window:Option<Instant>,count:u16,
}
fn public(service:u16)->bool {matches!(service,5|18|33|91|93|105|109|121)}
fn uuid(value:&str)->bool {Uuid::parse_str(value).is_ok_and(|v|!v.is_nil()&&v.to_string()==value)}
impl EventClock {
    fn valid(&self,binding:&Binding)->bool {
        self.version==1&&self.run_id==binding.run_id&&self.source_id==binding.source_id
            &&self.document_epoch==binding.document_epoch&&self.document_epoch>0
            &&uuid(&self.message_id)&&public(self.service)&&self.sequence<=9_007_199_254_740_991
            &&self.sample.valid()&&self.phase=="public-receiver-dispatch"&&self.presentation_uncertainty=="unknown"
    }
}
impl RelayTimingSink {
    /// Caller has already passed exact native URL, WebView, run and source gates.
    /// Revalidate against this sink's native binding; page JSON cannot select a sink.
    pub(crate) fn event_clock(&self,input:EventClock,now:Instant)->bool {
        let mut s=self.state.lock().unwrap_or_else(|p|p.into_inner());
        if s.ended||!input.valid(&s.binding){return false;}
        let Some(received)=self.ns(now) else {self.incomplete();return false;};
        if s.events.sequence.is_some_and(|last|input.sequence<=last)
            ||s.events.performance.is_some_and(|last|input.sample.performance_now_ms<last)
            ||s.events.received.is_some_and(|last|received<last){self.incomplete();return false;}
        if s.events.sequence.map_or(input.sequence!=0,|last|last.checked_add(1)!=Some(input.sequence)){self.incomplete();}
        s.events.sequence=Some(input.sequence);s.events.performance=Some(input.sample.performance_now_ms);s.events.received=Some(received);
        if s.events.window.is_none_or(|at|now.saturating_duration_since(at)>=Duration::from_secs(1)) {
            s.events.window=Some(now);s.events.count=0;
        }
        s.events.count=s.events.count.saturating_add(1);
        if s.events.count>EVENT_RATE{self.incomplete();return false;}
        let usable=input.live_visible==Some(true)&&!input.sample.paused&&!input.sample.seeking&&input.sample.ready_state>=2;
        // A hash is enough for exact matching to existing normalized messageId.
        // The original UUID and packet body are not copied into this new stream.
        let data=json!({"messageIdSha256":format!("{:x}",Sha256::digest(input.message_id.as_bytes())),
            "service":input.service,"observationSequence":input.sequence,"receivedMonotonicNs":received,
            "sample":input.sample.value(),"liveVisible":input.live_visible,"usable":usable,
            "quality":"public-receiver-dispatch","presentationUncertainty":"unknown","archiveMapping":null});
        let Some(next)=s.sequence.checked_add(1) else {self.incomplete();return false;};
        let Ok(bytes)=serde_json::to_vec(&json!({"version":1,"sequence":s.sequence,"kind":"soop_event_clock","binding":s.binding,"data":data}))
            else {self.incomplete();return false;};
        let mut q=self.core.state.lock().unwrap_or_else(|p|p.into_inner());
        if q.finish||q.sealed{return false;}
        if bytes.len()>EVENT_ROW_BYTES||q.event_clock_items>=EVENT_ITEMS||q.event_clock_bytes+bytes.len()>EVENT_BYTES {
            self.incomplete();return false;
        }
        s.sequence=next;q.event_clock_items+=1;q.event_clock_bytes+=bytes.len();
        q.records.push_back(Record {bytes,raw:false,display_control:false,timing:true,event_clock:true,
            receipt:Receipt {sequence:0,monotonic_ns:None}});
        self.core.wake.notify_one();true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(t:&RelayTimingSink,n:u64)->EventClock {
        let s=t.state.lock().unwrap();
        serde_json::from_value(json!({"version":1,"runId":s.binding.run_id,"sourceId":s.binding.source_id,
            "documentEpoch":s.binding.document_epoch,"sequence":n,"service":5,
            "messageId":"33333333-3333-4333-8333-333333333333","sample":{"sourceTimeSeconds":12.,"currentTimeSeconds":14.,
            "timestampOffsetSeconds":2.,"performanceNowMs":1000.+n as f64,"playbackRate":1.,"paused":false,"seeking":false,"readyState":4},
            "liveVisible":true,"phase":"public-receiver-dispatch","presentationUncertainty":"unknown"})).unwrap()
    }
    fn finish(lease:&mut RelayChatLease)->RelayChatSummary {
        lease.begin_finish(false);let deadline=Instant::now()+Duration::from_secs(3);
        loop{if let Some(r)=lease.poll_join().unwrap(){return r;}assert!(Instant::now()<deadline);thread::sleep(Duration::from_millis(1));}
    }
    #[test]
    fn exact_binding_and_public_uuid_only_no_raw_chat_mutation() {
        let (mut lease,_root)=test_fixture();let t=lease.timing();let at=t.core.started;
        let mut bad=event(&t,0);bad.source_id=Uuid::new_v4().to_string();assert!(!t.event_clock(bad,at));
        let mut bad=event(&t,0);bad.service=9;assert!(!t.event_clock(bad,at));
        let mut bad=event(&t,0);bad.message_id=Uuid::nil().to_string();assert!(!t.event_clock(bad,at));
        assert!(t.event_clock(event(&t,0),at));assert!(!t.event_clock(event(&t,0),at));
        let report=finish(&mut lease);let text=fs::read_to_string(report.directory.join("relay-timing.jsonl")).unwrap();
        let row:Value=serde_json::from_str(text.trim()).unwrap();assert_eq!(row["kind"],"soop_event_clock");
        assert_eq!(row["data"]["messageIdSha256"],format!("{:x}",Sha256::digest(b"33333333-3333-4333-8333-333333333333")));
        assert!(row["data"].get("messageId").is_none());assert_eq!((report.raw_rows,report.event_rows),(0,0));
    }
    #[test]
    fn optional_event_credits_never_consume_ack_or_chat_capacity() {
        let (mut lease,_root)=test_fixture();let t=lease.timing();let at=t.core.started;
        {let mut q=t.core.state.lock().unwrap();q.event_clock_items=EVENT_ITEMS;q.event_clock_bytes=EVENT_BYTES;}
        assert!(!t.event_clock(event(&t,0),at));let nonce=t.probe(at).unwrap();
        assert!(t.observe(nonce,None,at+Duration::from_millis(1)));
        let sink=Sink(t.core.clone());let receipt=sink.raw(b"{\"unchanged\":true}",at).unwrap();
        assert!(sink.event(b"{\"normalized\":true}",receipt));
        {let mut q=t.core.state.lock().unwrap();q.event_clock_items=0;q.event_clock_bytes=0;}
        let report=finish(&mut lease);assert_eq!((report.raw_rows,report.event_rows),(1,1));assert!(report.failure.is_none());assert!(report.timing_incomplete);
    }
    #[test]
    fn pause_seek_and_unknown_visibility_are_persisted_unusable() {
        let (mut lease,_root)=test_fixture();let t=lease.timing();let at=t.core.started;
        for n in 0..3 {let mut v=event(&t,n);match n{0=>v.sample.paused=true,1=>v.sample.seeking=true,_=>v.live_visible=None};assert!(t.event_clock(v,at));}
        let report=finish(&mut lease);let text=fs::read_to_string(report.directory.join("relay-timing.jsonl")).unwrap();
        for line in text.lines(){let row:Value=serde_json::from_str(line).unwrap();assert_eq!(row["data"]["usable"],false);}
    }
}
